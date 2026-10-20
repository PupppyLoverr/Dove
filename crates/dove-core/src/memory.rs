//! Memory for every harness, backed by Supermemory.
//!
//! Dove sits between the user and every agent, so it recalls and captures
//! memories itself instead of relying on per-harness plugins:
//!
//! - **Recall.** Before a prompt reaches the agent (first turn, resumed turn
//!   or a live send), Dove searches memory with the prompt (plus the profile
//!   on a session's first turn), drops what this session already saw, and
//!   prepends a fenced [`OPEN`] block within [`BUDGET_CHARS`]. The chat shows
//!   a "Recalled N memories" chip instead of the block ([`strip`],
//!   [`recalled`]).
//! - **Capture.** When a turn ends, the turn delta (prompt, final answer,
//!   edited files) is redacted ([`redact`]) and saved. Saves that fail go to
//!   `$DOVE_HOME/memory/outbox.jsonl` ([`Outbox`]) and are retried.
//! - **Corrections.** A prompt that corrects the agent, or a denied
//!   permission, is saved as a `feedback` memory so no agent repeats it.
//!
//! Scoping follows Supermemory's own Claude/Codex/OpenCode plugins
//! ([`project_tag`]), so their memories and Dove's are the same; the `user`
//! scope is the account's default space and each teammate gets `team_<id>`.
//!
//! The network side is a [`Backend`] the app registers ([`set_backend`]):
//! Supermemory's MCP tools over the user's OAuth token, so no separate key is
//! needed. With no backend registered every entry point is a no-op.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{Hash as _, Hasher as _};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use dove_proto::{AgentEvent, HarnessKind, PermissionDecision, ProjectId, TaskId};
use dove_store::Store;
use parking_lot::{Mutex, RwLock};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;

// ------------------------------------------------------------------ backend

/// One memory as a backend returns it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    pub id: Option<String>,
    pub text: String,
    pub score: Option<f32>,
    /// When it was saved (RFC 3339), so newer facts can win over older ones.
    #[serde(default)]
    pub at: Option<String>,
}

impl Memory {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), ..Default::default() }
    }
}

/// Where a memory lives.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// One repository, shared with Supermemory's own agent plugins.
    Project(String),
    /// The user's personal memories: the account's default space.
    User,
    /// One teammate (persona).
    Team(String),
}

impl Scope {
    /// The container tag to pass, `None` for the account default.
    pub fn container(&self) -> Option<&str> {
        match self {
            Scope::Project(t) | Scope::Team(t) => Some(t),
            Scope::User => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Scope::Project(_) => "project",
            Scope::User => "user",
            Scope::Team(_) => "teammate",
        }
    }
}

/// A memory to save.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    pub content: String,
    pub scope: Scope,
    /// `{source: "dove", harness, project, task, kind}`.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    /// Makes retries idempotent where the backend supports it.
    #[serde(default)]
    pub custom_id: Option<String>,
    /// Unix ms when it was made.
    #[serde(default)]
    pub at: i64,
}

impl Doc {
    /// The content with its metadata as a leading line, for backends that
    /// only take text (Supermemory's `add_memory`).
    pub fn text_with_metadata(&self) -> String {
        if self.metadata.is_empty() {
            return self.content.clone();
        }
        let meta = self.metadata.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(" · ");
        format!("[{meta}]\n{}", self.content)
    }
}

/// The memory service. Calls block (run them off the UI thread and outside
/// async tasks: [`blocking`]).
pub trait Backend: Send + Sync {
    /// Hybrid search within one scope.
    fn search(&self, query: &str, scope: &Scope, limit: usize) -> Result<Vec<Memory>, String>;
    /// The scope's standing rules (saved corrections and stated rules or
    /// preferences), newest first, whatever the prompt is about.
    fn standing(&self, scope: &Scope, query: &str, limit: usize) -> Result<Vec<Memory>, String> {
        let _ = (scope, query, limit);
        Ok(Vec::new())
    }
    /// The profile summary for a scope (static + dynamic facts).
    fn profile(&self, scope: &Scope) -> Result<Vec<Memory>, String>;
    fn add(&self, doc: &Doc) -> Result<(), String>;
    /// One page (from 1) of a scope's memories, newest first.
    fn list(&self, scope: &Scope, page: usize, limit: usize) -> Result<Vec<Memory>, String>;
    fn forget(&self, memory: &Memory, scope: &Scope) -> Result<(), String>;
    /// Who is signed in, for Settings.
    fn who_am_i(&self) -> Result<String, String>;
}

static BACKEND: RwLock<Option<Arc<dyn Backend>>> = RwLock::new(None);

/// Register (or clear) the memory service. The app does this when the user
/// signs in to Supermemory, and clears it when they sign out.
pub fn set_backend(backend: Option<Arc<dyn Backend>>) {
    *BACKEND.write() = backend;
}

pub fn backend() -> Option<Arc<dyn Backend>> {
    BACKEND.read().clone()
}

/// Run blocking backend work: on tokio's blocking pool inside a runtime,
/// else on a thread of its own.
pub fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> tokio::sync::oneshot::Receiver<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let job = move || {
        let _ = tx.send(f());
    };
    match tokio::runtime::Handle::try_current() {
        Ok(rt) => {
            rt.spawn_blocking(job);
        }
        Err(_) => {
            let _ = std::thread::Builder::new().name("dove-memory".into()).spawn(job);
        }
    }
    rx
}

// ---------------------------------------------------------------- settings

/// Settings keys (all flags default on once a backend is registered).
pub const ENABLED: &str = "memory_enabled";
pub const RECALL: &str = "memory_recall";
pub const CAPTURE: &str = "memory_capture";
pub const CORRECTIONS: &str = "memory_corrections";
/// Newline-separated path globs never sent to memory.
pub const NEVER_SEND: &str = "memory_never_send";
/// When the weekly consolidation last ran (unix ms).
pub const CONSOLIDATED_AT: &str = "memory_consolidated_at";
/// Set once the first-sign-in backfill was offered (or done).
pub const BACKFILL_OFFERED: &str = "memory_backfill_offered";

pub const DEFAULT_NEVER_SEND: &str = ".env*\n**/.env*\nsecrets/**\n**/secrets/**\n*.pem\n*.key\n*.p12\nid_rsa*\nid_ed25519*";

/// The per-project switch (`"false"` turns memory off for that project).
pub fn project_key(project: ProjectId) -> String {
    format!("memory_project.{project}")
}

/// What memory may do for one project right now.
#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    pub enabled: bool,
    pub recall: bool,
    pub capture: bool,
    pub corrections: bool,
    pub never_send: Vec<String>,
}

impl Prefs {
    pub fn load(store: &Store, project: Option<ProjectId>) -> Self {
        let on = store.flag(ENABLED, true) && project.is_none_or(|p| store.flag(&project_key(p), true));
        Self {
            enabled: on,
            recall: on && store.flag(RECALL, true),
            capture: on && store.flag(CAPTURE, true),
            corrections: on && store.flag(CORRECTIONS, true),
            never_send: store
                .setting(NEVER_SEND)
                .unwrap_or_else(|| DEFAULT_NEVER_SEND.to_string())
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_string)
                .collect(),
        }
    }
}

/// Whether prompts for `project` go through recall (a backend is signed in
/// and the switches allow it). Callers then hand the first prompt to the
/// session as a send, so recall can run without blocking the caller.
pub fn recalls(store: &Store, project: ProjectId) -> bool {
    backend().is_some() && Prefs::load(store, Some(project)).recall
}

// ------------------------------------------------------------- container tags

/// The first 16 hex digits of SHA-256, as Supermemory's plugins hash.
pub fn sha16(input: &str) -> String {
    let digest = sha2::Sha256::digest(input.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// A git remote reduced to `host/owner/repo`, so HTTPS and SSH clones of one
/// repository agree (Supermemory's `normalizeGitRemote`).
pub fn normalize_remote(remote: &str) -> Option<String> {
    let raw = remote.trim();
    if raw.is_empty() {
        return None;
    }
    static SCHEME: OnceLock<Regex> = OnceLock::new();
    static SCP: OnceLock<Regex> = OnceLock::new();
    let scheme = SCHEME.get_or_init(|| Regex::new(r"(?i)^[a-z][a-z\d+.\-]*://").unwrap());
    let scp = SCP.get_or_init(|| Regex::new(r"^(?:[^@/]+@)?([^:]+):(.+)$").unwrap());
    let normalized = if let Some(m) = scheme.find(raw) {
        let proto = raw[..m.end() - 3].to_ascii_lowercase();
        let rest = &raw[m.end()..];
        if proto == "file" {
            format!("file:{}", percent_decode(rest.find('/').map_or("/", |i| &rest[i..])))
        } else {
            let (authority, path) = rest.find('/').map_or((rest, ""), |i| (&rest[..i], &rest[i..]));
            let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
            let (host, port) = match host_port.rsplit_once(':') {
                Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => (h, p),
                _ => (host_port, ""),
            };
            let default_port = matches!(
                (proto.as_str(), port),
                ("http", "80") | ("https", "443") | ("ws", "80") | ("wss", "443") | ("ftp", "21")
            );
            let port = if port.is_empty() || default_port { String::new() } else { format!(":{port}") };
            format!("{}{port}/{}", host.to_ascii_lowercase(), path.trim_start_matches('/'))
        }
    } else if let Some(c) = scp.captures(raw) {
        format!("{}/{}", c[1].to_ascii_lowercase(), &c[2])
    } else {
        let p = Path::new(raw);
        let abs = if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::env::current_dir().map(|d| d.join(p)).unwrap_or_else(|_| p.to_path_buf())
        };
        format!("file:{}", abs.display())
    };
    let mut s = normalized;
    if let Some(i) = s.find(['?', '#']) {
        s.truncate(i);
    }
    let mut s = s.trim_end_matches('/').to_string();
    if s.to_ascii_lowercase().ends_with(".git") {
        s.truncate(s.len() - 4);
    }
    while s.contains("//") {
        s = s.replace("//", "/");
    }
    Some(s.to_lowercase())
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(b) = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A repository name as a tag segment (Supermemory's `sanitizeRepoName`).
pub fn sanitize_repo_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        let c = if c.is_ascii_lowercase() || c.is_ascii_digit() { c } else { '_' };
        if !(c == '_' && out.ends_with('_')) {
            out.push(c);
        }
    }
    let out = out.trim_matches('_');
    let out: String = out.chars().take(95).collect();
    let out = out.trim_end_matches('_');
    if out.is_empty() { "unknown".into() } else { out.to_string() }
}

/// `repo_<name>__<sha16(identity)>`, where identity is the normalized remote
/// or `path:<real path>`.
pub fn generated_tag(repo_name: &str, identity: &str) -> String {
    let short: String = sanitize_repo_name(repo_name).chars().take(72).collect();
    let short = short.trim_end_matches('_');
    let short = if short.is_empty() { "unknown" } else { short };
    format!("repo_{short}__{}", sha16(identity))
}

/// A repository's name from its remote URL (the last path segment).
pub fn remote_name(remote: &str) -> Option<String> {
    let display = remote.trim().trim_end_matches('/');
    let display = if display.to_ascii_lowercase().ends_with(".git") { &display[..display.len() - 4] } else { display };
    let cut = display.rfind(['/', ':']).map_or(0, |i| i + 1);
    Some(display[cut..].to_string()).filter(|s| !s.is_empty())
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args).current_dir(dir).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

/// The repository root every worktree of it shares (its main checkout), or
/// the folder itself outside git.
pub fn project_base(dir: &Path) -> PathBuf {
    let isolate = std::env::var("SUPERMEMORY_ISOLATE_WORKTREES").as_deref() == Ok("true");
    let toplevel = || git(dir, &["rev-parse", "--show-toplevel"]).map(PathBuf::from);
    let root = if isolate {
        toplevel()
    } else {
        match git(dir, &["rev-parse", "--git-common-dir"]) {
            Some(common) if common == ".git" => toplevel(),
            Some(common) => {
                let resolved = dir.join(&common);
                let sep = std::path::MAIN_SEPARATOR;
                let inside = resolved.display().to_string().contains(&format!("{sep}.git{sep}"));
                if resolved.file_name().is_some_and(|n| n == ".git") && !inside {
                    resolved.parent().map(Path::to_path_buf)
                } else {
                    toplevel()
                }
            }
            None => None,
        }
    };
    root.unwrap_or_else(|| dir.to_path_buf())
}

/// The project container for a folder, matching Supermemory's plugins: the
/// repo's `.claude/.supermemory-claude/config.json` `repoContainerTag`, then
/// `SUPERMEMORY_REPO_TAG`, then the generated `repo_<name>__<hash>`.
pub fn project_tag(dir: &Path) -> String {
    let base = project_base(dir);
    let config = base.join(".claude").join(".supermemory-claude").join("config.json");
    if let Some(tag) = std::fs::read_to_string(config)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("repoContainerTag")?.as_str().map(str::to_string))
        .filter(|t| !t.trim().is_empty())
    {
        return tag;
    }
    if let Ok(tag) = std::env::var("SUPERMEMORY_REPO_TAG")
        && !tag.trim().is_empty()
    {
        return tag;
    }
    let remote = git(&base, &["remote", "get-url", "origin"]);
    let normalized = remote.as_deref().and_then(normalize_remote);
    let isolate = std::env::var("SUPERMEMORY_ISOLATE_WORKTREES").as_deref() == Ok("true");
    let identity = match normalized {
        Some(n) if !isolate => n,
        _ => {
            let real = std::fs::canonicalize(&base).unwrap_or_else(|_| base.clone());
            let real = real.display().to_string();
            format!("path:{}", real.strip_prefix(r"\\?\").unwrap_or(&real))
        }
    };
    let name = remote
        .as_deref()
        .and_then(remote_name)
        .or_else(|| base.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "unknown".into());
    generated_tag(&name, &identity)
}

/// A teammate's own container.
pub fn team_tag(persona: &str) -> String {
    format!("team_{}", sanitize_repo_name(persona))
}

// ---------------------------------------------------------------- redaction

/// Credential-shaped strings replaced by `[REDACTED]` before anything leaves
/// the machine.
pub fn redact(text: &str) -> String {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        [
            (r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----", "[REDACTED]"),
            (r"(?is)<private>.*?</private>", "[REDACTED]"),
            // Credentials inside URLs.
            (r"(?i)\b([a-z][a-z0-9+.\-]*://)([^/\s:@]+):([^/\s@]+)@", "${1}${2}:[REDACTED]@"),
            // Provider keys.
            (r"\b(?:sk|pk|rk)-[A-Za-z0-9_\-]{16,}", "[REDACTED]"),
            (r"\b(?:sk|rk|pk)_(?:live|test)_[A-Za-z0-9]{12,}", "[REDACTED]"),
            (r"\bgh[pousr]_[A-Za-z0-9]{30,}", "[REDACTED]"),
            (r"\bgithub_pat_[A-Za-z0-9_]{30,}", "[REDACTED]"),
            (r"\bglpat-[A-Za-z0-9_\-]{20,}", "[REDACTED]"),
            (r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", "[REDACTED]"),
            (r"\bxox[abprs]-[A-Za-z0-9\-]{10,}", "[REDACTED]"),
            (r"\bAIza[0-9A-Za-z_\-]{35}", "[REDACTED]"),
            (r"\bsm_[A-Za-z0-9_\-]{20,}", "[REDACTED]"),
            (r"\bnpm_[A-Za-z0-9]{30,}", "[REDACTED]"),
            (r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}", "[REDACTED]"),
            (r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._\-~+/]{16,}=*", "${1} [REDACTED]"),
            // `API_KEY=…`, `"password": "…"`, `client_secret: …`.
            (
                r#"(?i)\b([A-Za-z0-9_\-]*(?:api[_\-]?key|secret|token|password|passwd|pwd|access[_\-]?key|private[_\-]?key|credential)[A-Za-z0-9_\-]*)(["']?\s*[:=]\s*)(["']?)([^\s"',;]{6,})"#,
                "${1}${2}${3}[REDACTED]",
            ),
        ]
        .into_iter()
        .map(|(re, with)| (Regex::new(re).expect("redaction rule"), with))
        .collect()
    });
    let mut out = text.to_string();
    for (re, with) in rules {
        if re.is_match(&out) {
            out = re.replace_all(&out, *with).into_owned();
        }
    }
    out
}

/// Whether `path` matches a never-send glob (`*`, `?`, `**`). A pattern
/// without `/` also matches the file name alone.
pub fn never_send(path: &str, globs: &[String]) -> bool {
    let path = path.replace('\\', "/");
    let path = path.trim_start_matches("./");
    let name = path.rsplit('/').next().unwrap_or(path);
    globs.iter().any(|g| {
        let g = g.replace('\\', "/");
        glob_match(g.as_bytes(), path.as_bytes()) || (!g.contains('/') && glob_match(g.as_bytes(), name.as_bytes()))
    })
}

fn glob_match(p: &[u8], s: &[u8]) -> bool {
    match p.first() {
        None => s.is_empty(),
        Some(b'*') if p.get(1) == Some(&b'*') => {
            // `**/` matches zero or more whole segments; `**` anything.
            let rest = if p.get(2) == Some(&b'/') { &p[3..] } else { &p[2..] };
            (0..=s.len()).any(|i| (i == 0 || s[i - 1] == b'/' || p.get(2) != Some(&b'/')) && glob_match(rest, &s[i..]))
        }
        Some(b'*') => (0..=s.len()).take_while(|&i| i == 0 || s[i - 1] != b'/').any(|i| glob_match(&p[1..], &s[i..])),
        Some(b'?') => !s.is_empty() && s[0] != b'/' && glob_match(&p[1..], &s[1..]),
        Some(c) => !s.is_empty() && s[0].eq_ignore_ascii_case(c) && glob_match(&p[1..], &s[1..]),
    }
}

// ---------------------------------------------------------------- recall block

/// The fence around recalled memories in a prompt.
pub const OPEN: &str = "<memory source=\"dove-recall\"";
pub const CLOSE: &str = "</memory>";
/// About 1.5k tokens.
pub const BUDGET_CHARS: usize = 6000;
/// One memory's longest line in the block.
const ITEM_CHARS: usize = 600;
/// How long a prompt may wait for recall before going without.
pub const RECALL_TIMEOUT: Duration = Duration::from_secs(4);

fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        format!("{}…", flat.chars().take(max - 1).collect::<String>())
    }
}

/// The fenced block for `profile` and `found`, within `budget` characters.
/// `None` when there is nothing to add.
pub fn render_block(profile: &[Memory], found: &[Memory], budget: usize) -> Option<(String, usize)> {
    render_block_with(&[], profile, found, budget)
}

/// `[2026-10-20 19:14] ` from an RFC 3339 time, or nothing.
fn stamp(at: Option<&str>) -> String {
    match at {
        Some(t) if t.len() >= 16 && t.as_bytes()[10] == b'T' => format!("[{} {}] ", &t[..10], &t[11..16]),
        _ => String::new(),
    }
}

/// [`render_block`] with standing rules first.
pub fn render_block_with(
    standing: &[Memory],
    profile: &[Memory],
    found: &[Memory],
    budget: usize,
) -> Option<(String, usize)> {
    let head = "Memories Dove recalled for this task from earlier sessions (any agent). Standing rules are the user's own instructions for this project: follow them. Use the rest where it helps. When two memories disagree, the newer one wins; the user's current request wins over all of them.";
    let mut body = String::new();
    let mut count = 0;
    let mut used = head.len() + OPEN.len() + CLOSE.len() + 40;
    for (title, list) in [("Standing rules", standing), ("Profile", profile), ("Relevant", found)] {
        let mut section = String::new();
        for m in list {
            let line = format!("- {}{}\n", stamp(m.at.as_deref()), one_line(&m.text, ITEM_CHARS));
            if line.trim().len() <= 2 || used + section.len() + line.len() > budget {
                continue;
            }
            section.push_str(&line);
            count += 1;
        }
        if !section.is_empty() {
            used += section.len() + title.len() + 2;
            body.push_str(&format!("{title}:\n{section}"));
        }
    }
    (count > 0).then(|| (format!("{OPEN} count=\"{count}\">\n{head}\n{body}{CLOSE}\n\n"), count))
}

/// `prompt` with `block` in front.
pub fn augment(prompt: &str, block: &str) -> String {
    format!("{block}{prompt}")
}

/// A prompt without its recalled-memory block.
pub fn strip(text: &str) -> &str {
    let trimmed = text.trim_start();
    if trimmed.starts_with(OPEN)
        && let Some(end) = trimmed.find(CLOSE)
    {
        return trimmed[end + CLOSE.len()..].trim_start();
    }
    text
}

/// The memories a prompt's block carried, for the chat's chip.
pub fn recalled(text: &str) -> Option<Vec<String>> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with(OPEN) {
        return None;
    }
    let end = trimmed.find(CLOSE)?;
    let items: Vec<String> = trimmed[..end]
        .lines()
        .filter_map(|l| l.strip_prefix("- "))
        .map(|l| {
            let l = l.trim();
            // Drop a leading `[date time] ` stamp.
            match l.strip_prefix('[').and_then(|r| r.split_once("] ")) {
                Some((d, rest)) if d.len() == 16 && d.as_bytes()[4] == b'-' => rest.to_string(),
                _ => l.to_string(),
            }
        })
        .collect();
    (!items.is_empty()).then_some(items)
}

/// Drops memories a session has already been shown.
#[derive(Debug, Default)]
pub struct Dedupe {
    seen: HashSet<u64>,
}

impl Dedupe {
    fn key(m: &Memory) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let norm = m.text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        norm.hash(&mut h);
        h.finish()
    }

    /// The memories in `list` not seen before (by text; ids differ across
    /// scopes for the same fact), marking them seen.
    pub fn fresh(&mut self, list: Vec<Memory>) -> Vec<Memory> {
        list.into_iter().filter(|m| !m.text.trim().is_empty() && self.seen.insert(Self::key(m))).collect()
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

// ------------------------------------------------------------------ outbox

/// Saves that could not be sent, one JSON [`Doc`] per line.
pub struct Outbox {
    path: PathBuf,
}

/// Oldest entries are dropped past this.
const OUTBOX_CAP: usize = 2000;

/// One writer at a time, across sessions.
fn outbox_lock() -> &'static Mutex<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    &LOCK
}

impl Outbox {
    /// `<root>/memory/outbox.jsonl` (`root` is `$DOVE_HOME`'s data folder).
    pub fn at(root: &Path) -> Self {
        Self { path: root.join("memory").join("outbox.jsonl") }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> Vec<Doc> {
        std::fs::read_to_string(&self.path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn write(&self, docs: &[Doc]) -> std::io::Result<()> {
        if docs.is_empty() {
            return match std::fs::remove_file(&self.path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("jsonl.tmp");
        let mut f = std::fs::File::create(&tmp)?;
        for d in docs {
            writeln!(f, "{}", serde_json::to_string(d).unwrap_or_default())?;
        }
        f.flush()?;
        drop(f);
        std::fs::rename(tmp, &self.path)
    }

    pub fn push(&self, doc: &Doc) -> std::io::Result<()> {
        let _g = outbox_lock().lock();
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        writeln!(f, "{}", serde_json::to_string(doc).map_err(std::io::Error::other)?)?;
        drop(f);
        let docs = self.read();
        if docs.len() > OUTBOX_CAP {
            self.write(&docs[docs.len() - OUTBOX_CAP..])?;
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Send queued docs in order, stopping at the first failure (offline).
    /// Returns `(sent, left)`.
    pub fn drain(&self, mut send: impl FnMut(&Doc) -> Result<(), String>) -> (usize, usize) {
        let _g = outbox_lock().lock();
        let docs = self.read();
        let mut sent = 0;
        for d in &docs {
            if send(d).is_err() {
                break;
            }
            sent += 1;
        }
        if sent > 0 {
            let _ = self.write(&docs[sent..]);
        }
        (sent, docs.len() - sent)
    }
}

/// Save `doc` now, or queue it. Also flushes anything already queued once a
/// save gets through.
pub fn save_or_queue(root: &Path, doc: Doc) {
    let Some(b) = backend() else { return };
    let root = root.to_path_buf();
    drop(blocking(move || {
        let outbox = Outbox::at(&root);
        match b.add(&doc) {
            Ok(()) => {
                if !outbox.is_empty() {
                    outbox.drain(|d| b.add(d));
                }
            }
            Err(e) => {
                tracing::info!("memory save queued: {e}");
                let _ = outbox.push(&doc);
            }
        }
    }));
}

/// Retry queued saves (app start, sign-in).
pub fn flush_outbox(root: &Path) {
    let Some(b) = backend() else { return };
    let root = root.to_path_buf();
    drop(blocking(move || {
        let (sent, left) = Outbox::at(&root).drain(|d| b.add(d));
        if sent > 0 || left > 0 {
            tracing::info!("memory outbox: {sent} sent, {left} waiting");
        }
    }));
}

// ------------------------------------------------------------- corrections

/// The user correcting or redirecting the agent, as a memory worth keeping:
/// `prompt` follows `prev_answer` (the agent's last reply to `prev_prompt`).
pub fn correction(prompt: &str, prev_prompt: &str, prev_answer: &str) -> Option<String> {
    let p = prompt.trim();
    if p.len() < 6 || prev_answer.trim().is_empty() {
        return None;
    }
    let lower = p.to_lowercase();
    let lead = [
        "no,", "no.", "no!", "no ", "nope", "don't", "dont ", "do not", "stop", "wrong", "that's wrong", "thats wrong",
        "that's not", "that is not", "not what i", "i said", "i told you", "actually,", "instead,", "never ",
        "please don't", "please do not", "why did you", "you should have", "you shouldn't", "revert", "undo",
    ];
    let anywhere = [
        "not what i asked", "i told you", "i already said", "you should have", "please don't", "stop doing",
        "don't ever", "never do", "that's wrong", "that is wrong", "revert that", "undo that", "use it instead",
        "instead of that", "you ignored", "you forgot",
    ];
    let redirect = lead.iter().any(|l| lower.starts_with(l)) || anywhere.iter().any(|a| lower.contains(a));
    let reasked = !prev_prompt.trim().is_empty() && similarity(p, prev_prompt) >= 0.75;
    if !redirect && !reasked {
        return None;
    }
    let what = if reasked { "The user had to ask again" } else { "The user corrected the agent" };
    Some(format!(
        "{what}. Their words: \"{}\". The agent had answered: \"{}\". Avoid repeating this.",
        one_line(p, 500),
        one_line(prev_answer, 300)
    ))
}

/// Whether a prompt states a standing rule or preference ("always…",
/// "never…", "my preference…", "from now on…") or corrects what was known
/// before ("no, that's out of date…", "X changed from A to B"), saved as
/// `kind: rule` so every later session gets it whatever it is about.
pub fn is_rule(prompt: &str) -> bool {
    let p = prompt.trim().to_lowercase();
    let states = [
        "always ", "never ", "my preference", "i prefer", "from now on", "every answer", "every time", "convention",
        "rule:", "rule for this", "must be", "must always", "don't ever", "do not ever", "make sure to", "remember to",
        "remember that", "remember this",
    ];
    let corrects = ["out of date", "no longer", "is retired", "was deleted", "changed from", "is now `", "moved from"];
    let leads = ["no,", "no.", "no —", "no -", "no:", "correction", "actually,", "update:"];
    states.iter().chain(corrects.iter()).any(|k| p.contains(k)) || leads.iter().any(|l| p.starts_with(l))
}

/// Word-set overlap (Jaccard) of two texts, 0–1.
fn similarity(a: &str, b: &str) -> f32 {
    let words = |s: &str| -> HashSet<String> {
        s.split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 2).map(str::to_lowercase).collect()
    };
    let (a, b) = (words(a), words(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    a.intersection(&b).count() as f32 / a.union(&b).count() as f32
}

// ------------------------------------------------------------------ sessions

/// What memory tracks for one live session.
pub struct Session {
    store: Arc<Store>,
    pub task: TaskId,
    pub project: ProjectId,
    pub harness: HarnessKind,
    cwd: PathBuf,
    persona: Option<String>,
    /// Container tags, worked out on first use (it runs git).
    scopes: OnceLock<Vec<Scope>>,
    dedupe: Dedupe,
    profiled: bool,
    turns: u32,
    /// The turn in progress.
    prompt: String,
    answer: String,
    files: BTreeSet<String>,
    /// The last finished turn, for the corrections detector.
    last_prompt: String,
    last_answer: String,
    /// Open permission requests: id → what was asked.
    asks: HashMap<String, String>,
}

pub type Shared = Arc<Mutex<Session>>;

/// Longest prompt / answer kept per captured turn.
const PROMPT_CHARS: usize = 2000;
const ANSWER_CHARS: usize = 4000;

impl Session {
    pub fn new(store: Arc<Store>, task: TaskId, project: ProjectId, harness: HarnessKind, cwd: PathBuf) -> Shared {
        let persona =
            store.persona_assignments().ok().and_then(|a| a.into_iter().find(|(t, _)| *t == task).map(|(_, p)| p));
        Arc::new(Mutex::new(Self {
            store,
            task,
            project,
            harness,
            cwd,
            persona,
            scopes: OnceLock::new(),
            dedupe: Dedupe::default(),
            profiled: false,
            turns: 0,
            prompt: String::new(),
            answer: String::new(),
            files: BTreeSet::new(),
            last_prompt: String::new(),
            last_answer: String::new(),
            asks: HashMap::new(),
        }))
    }

    fn prefs(&self) -> Prefs {
        Prefs::load(&self.store, Some(self.project))
    }

    /// Project, user and (for a teammate) team scopes. Blocking (git).
    fn scopes(&self) -> Vec<Scope> {
        self.scopes
            .get_or_init(|| {
                let mut s = vec![Scope::Project(project_tag(&self.cwd)), Scope::User];
                if let Some(p) = &self.persona {
                    s.push(Scope::Team(team_tag(p)));
                }
                s
            })
            .clone()
    }

    fn metadata(&self, kind: &str) -> BTreeMap<String, String> {
        let project = self
            .store
            .projects()
            .ok()
            .and_then(|ps| ps.into_iter().find(|p| p.id == self.project).map(|p| p.name))
            .unwrap_or_default();
        let mut m = BTreeMap::new();
        m.insert("source".into(), "dove".into());
        m.insert("harness".into(), self.harness.label().to_string());
        m.insert("project".into(), project);
        m.insert("task".into(), self.task.to_string());
        m.insert("kind".into(), kind.into());
        m
    }

    /// Follow one recorded event: turn tracking, capture, open asks.
    pub fn observe(&mut self, ev: &AgentEvent) {
        match ev {
            AgentEvent::UserMessage { text } => {
                self.prompt = crate::strip_persona(text).to_string();
                self.answer.clear();
                self.files.clear();
            }
            AgentEvent::TextDelta { text } => {
                if self.answer.len() < ANSWER_CHARS * 4 {
                    self.answer.push_str(text);
                }
            }
            // The final answer is the text after the last tool call.
            AgentEvent::ToolCall { input, kind, .. } => {
                self.answer.clear();
                if *kind == dove_proto::ToolKind::Edit
                    && let Some(p) = ["file_path", "path", "filePath"].iter().find_map(|k| input.get(*k)?.as_str())
                {
                    self.files.insert(self.relative(p));
                }
            }
            AgentEvent::ToolDiff { path, .. } => {
                self.files.insert(self.relative(path));
            }
            AgentEvent::PermissionRequested(r) => {
                self.asks.insert(r.request_id.clone(), one_line(&format!("{}: {}", r.tool, r.summary), 300));
            }
            AgentEvent::TurnEnded { is_error, .. } => {
                let prompt = std::mem::take(&mut self.prompt);
                let answer = std::mem::take(&mut self.answer);
                let files = std::mem::take(&mut self.files);
                if !*is_error {
                    self.turns += 1;
                    self.capture(&prompt, &answer, &files);
                }
                self.last_prompt = prompt;
                self.last_answer = answer;
            }
            _ => {}
        }
    }

    fn relative(&self, path: &str) -> String {
        let p = Path::new(path);
        p.strip_prefix(&self.cwd).unwrap_or(p).display().to_string().replace('\\', "/")
    }

    /// The memory a finished turn becomes, if any.
    pub fn turn_doc(&self, prompt: &str, answer: &str, files: &BTreeSet<String>, prefs: &Prefs) -> Option<Doc> {
        let prompt = prompt.trim();
        let answer = answer.trim();
        if prompt.len() < 4 || prompt.starts_with('/') || (answer.is_empty() && files.is_empty()) {
            return None;
        }
        let files: Vec<&str> =
            files.iter().map(String::as_str).filter(|f| !never_send(f, &prefs.never_send)).take(40).collect();
        let mut content = format!("User: {}\n", clip(prompt, PROMPT_CHARS));
        if !answer.is_empty() {
            content.push_str(&format!("Agent: {}\n", clip(answer, ANSWER_CHARS)));
        }
        if !files.is_empty() {
            content.push_str(&format!("Files edited: {}\n", files.join(", ")));
        }
        Some(Doc {
            content: redact(&content),
            scope: Scope::Project(String::new()),
            metadata: self.metadata(if is_rule(prompt) { "rule" } else { "turn" }),
            custom_id: Some(format!("dove_{}_{}", self.task, self.turns)),
            at: dove_store::now_ms(),
        })
    }

    fn capture(&self, prompt: &str, answer: &str, files: &BTreeSet<String>) {
        let prefs = self.prefs();
        if !prefs.capture || backend().is_none() {
            return;
        }
        let Some(doc) = self.turn_doc(prompt, answer, files, &prefs) else { return };
        self.save(doc);
    }

    /// Save `doc` in this session's project scope (resolved off-thread).
    fn save(&self, doc: Doc) {
        let root = self.store.root().to_path_buf();
        let cwd = self.cwd.clone();
        let tags = self.scopes.get().cloned();
        drop(blocking(move || {
            let project = tags
                .and_then(|t| t.into_iter().find(|s| matches!(s, Scope::Project(_))))
                .unwrap_or_else(|| Scope::Project(project_tag(&cwd)));
            let mut doc = doc;
            if matches!(&doc.scope, Scope::Project(t) if t.is_empty()) {
                doc.scope = project;
            }
            save_or_queue(&root, doc);
        }));
    }

    /// The user answered a permission prompt.
    pub fn decided(&mut self, request_id: &str, decision: PermissionDecision) {
        let ask = self.asks.remove(request_id);
        if decision != PermissionDecision::Deny || !self.prefs().corrections || backend().is_none() {
            return;
        }
        let Some(ask) = ask else { return };
        let content = redact(&format!(
            "The user denied this agent action: {ask}. While working on: \"{}\". Ask before doing this again.",
            one_line(&self.prompt, 300)
        ));
        let mut meta = self.metadata("feedback");
        meta.insert("priority".into(), "high".into());
        self.save(Doc { content, scope: Scope::Project(String::new()), metadata: meta, custom_id: None, at: dove_store::now_ms() });
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max - 1).collect::<String>())
    }
}

/// A prompt on its way to the agent: save it as feedback if it corrects the
/// last turn, then prepend what memory recalls for it. Waits at most
/// [`RECALL_TIMEOUT`]; without a backend, or with recall off, the prompt
/// passes unchanged.
pub async fn before_send(session: &Shared, prompt: String) -> String {
    let Some(b) = backend() else { return prompt };
    let (prefs, query, first) = {
        let s = session.lock();
        let prefs = s.prefs();
        let visible = crate::strip_persona(&prompt).trim().to_string();
        if prefs.corrections
            && let Some(note) = correction(&visible, &s.last_prompt, &s.last_answer)
        {
            let mut meta = s.metadata("feedback");
            meta.insert("priority".into(), "high".into());
            s.save(Doc {
                content: redact(&note),
                scope: Scope::Project(String::new()),
                metadata: meta,
                custom_id: None,
                at: dove_store::now_ms(),
            });
        }
        let first = !s.profiled;
        (prefs, visible, first)
    };
    if !prefs.recall || query.is_empty() || query.starts_with('/') || prompt.trim_start().starts_with(OPEN) {
        return prompt;
    }
    let shared = session.clone();
    let query = redact(&clip(&query, 1000));
    let started = std::time::Instant::now();
    let job = blocking(move || {
        let scopes = shared.lock().scopes();
        // The profile and every scope's search at once: recall has a few
        // seconds at most.
        let (standing, profile, mut found) = std::thread::scope(|t| {
            let profile = t.spawn(|| match scopes.first().filter(|_| first).map(|p| b.profile(p)) {
                Some(Ok(p)) => p,
                Some(Err(e)) => {
                    tracing::debug!("memory profile: {e}");
                    Vec::new()
                }
                None => Vec::new(),
            });
            // Standing rules once per session, from every scope.
            let standing: Vec<_> = scopes
                .iter()
                .filter(|_| first)
                .map(|scope| {
                    let (b, query) = (&b, &query);
                    t.spawn(move || b.standing(scope, query, 12).unwrap_or_default())
                })
                .collect();
            let searches: Vec<_> = scopes
                .iter()
                .map(|scope| {
                    let (b, query) = (&b, &query);
                    t.spawn(move || match b.search(query, scope, 6) {
                        Ok(list) => list,
                        Err(e) => {
                            tracing::debug!("memory search ({}): {e}", scope.label());
                            Vec::new()
                        }
                    })
                })
                .collect();
            let found: Vec<Memory> = searches.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect();
            let mut standing: Vec<Memory> = standing.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect();
            standing.sort_by(|a, b| b.at.cmp(&a.at));
            (standing, profile.join().unwrap_or_default(), found)
        });
        found.sort_by(|a, b| b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)));
        (standing, profile, found)
    });
    let Ok(Ok((standing, profile, found))) = tokio::time::timeout(RECALL_TIMEOUT, job).await else {
        tracing::info!("memory recall skipped after {} ms (slow or failed)", started.elapsed().as_millis());
        return prompt;
    };
    let mut s = session.lock();
    s.profiled = true;
    let counts = (standing.len(), profile.len(), found.len());
    let standing = s.dedupe.fresh(standing);
    let profile = s.dedupe.fresh(profile);
    let found = s.dedupe.fresh(found);
    let block = render_block_with(&standing, &profile, &found, BUDGET_CHARS);
    tracing::info!(
        "memory recall for {} in {} ms: {} standing, {} profile, {} found; {} new, {} sent",
        s.task,
        started.elapsed().as_millis(),
        counts.0,
        counts.1,
        counts.2,
        standing.len() + profile.len() + found.len(),
        block.as_ref().map_or(0, |(_, n)| *n)
    );
    match block {
        Some((block, _)) => augment(&prompt, &block),
        None => prompt,
    }
}

// ------------------------------------------------------------ consolidation

/// Memories that say the same thing: groups of two or more (by word overlap
/// of at least `threshold`), the first being the one to keep.
pub fn duplicates(list: &[Memory], threshold: f32) -> Vec<Vec<usize>> {
    let mut used = vec![false; list.len()];
    let mut groups = Vec::new();
    for i in 0..list.len() {
        if used[i] {
            continue;
        }
        let mut group = vec![i];
        for j in i + 1..list.len() {
            if !used[j] && similarity(&list[i].text, &list[j].text) >= threshold {
                used[j] = true;
                group.push(j);
            }
        }
        if group.len() > 1 {
            used[i] = true;
            groups.push(group);
        }
    }
    groups
}

/// A workflow that keeps coming back, worth a skill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillProposal {
    pub name: String,
    pub description: String,
    /// The commands seen, most frequent first.
    pub steps: Vec<String>,
    /// How many memories mention it.
    pub seen: usize,
}

impl SkillProposal {
    /// A SKILL.md draft for the user to review before installing.
    pub fn skill_md(&self) -> String {
        let steps = self.steps.iter().enumerate().map(|(i, s)| format!("{}. `{s}`", i + 1)).collect::<Vec<_>>().join("\n");
        format!(
            "---\nname: {}\ndescription: {}\n---\n\n# {}\n\nDove proposed this skill after seeing the same workflow in {} memories. Review and edit before installing.\n\n## Steps\n\n{steps}\n",
            self.name, self.description, self.name, self.seen
        )
    }
}

/// Repeated command workflows across memories: shell commands in backticks
/// (or after `$ `) that appear in at least `min` memories, grouped by their
/// leading tool (`cargo`, `npm`, `pnpm`, `gh`, …).
pub fn propose_skills(list: &[Memory], min: usize) -> Vec<SkillProposal> {
    static CMD: OnceLock<Regex> = OnceLock::new();
    let re = CMD.get_or_init(|| Regex::new(r"(?:`([^`\n]{4,120})`|\$ ([^\n]{4,120}))").unwrap());
    const TOOLS: &[&str] = &[
        "cargo", "npm", "pnpm", "yarn", "bun", "npx", "gh", "git", "make", "docker", "pytest", "uv", "python", "go",
        "just", "dotnet", "mvn", "gradle", "kubectl", "terraform",
    ];
    // tool → command → memories mentioning it
    let mut by_tool: BTreeMap<String, BTreeMap<String, HashSet<usize>>> = BTreeMap::new();
    for (i, m) in list.iter().enumerate() {
        for c in re.captures_iter(&m.text) {
            let cmd = c.get(1).or(c.get(2)).map(|m| m.as_str().trim()).unwrap_or_default();
            let tool = cmd.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
            if TOOLS.contains(&tool.as_str()) {
                by_tool.entry(tool).or_default().entry(cmd.to_string()).or_default().insert(i);
            }
        }
    }
    let mut out = Vec::new();
    for (tool, cmds) in by_tool {
        let seen: HashSet<usize> = cmds.values().flatten().copied().collect();
        if seen.len() < min {
            continue;
        }
        let mut steps: Vec<(String, usize)> = cmds.into_iter().map(|(c, m)| (c, m.len())).collect();
        steps.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out.push(SkillProposal {
            name: format!("project-{tool}-workflow"),
            description: format!("How this project runs its {tool} workflow (build, test, release), learned from memory."),
            steps: steps.into_iter().take(8).map(|(c, _)| c).collect(),
            seen: seen.len(),
        });
    }
    out.sort_by(|a, b| b.seen.cmp(&a.seen));
    out
}

/// What a consolidation run found, saved to `memory/consolidation.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub at: i64,
    /// Per scope: the memory to keep and the copies of it.
    pub duplicates: Vec<(Scope, Memory, Vec<Memory>)>,
    pub skills: Vec<SkillProposal>,
    /// Memories looked at.
    pub scanned: usize,
}

impl Report {
    pub fn path(root: &Path) -> PathBuf {
        root.join("memory").join("consolidation.json")
    }

    pub fn load(root: &Path) -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(Self::path(root)).ok()?).ok()
    }

    pub fn duplicate_count(&self) -> usize {
        self.duplicates.iter().map(|(_, _, d)| d.len()).sum()
    }
}

/// Where proposed skills are drafted for the user to review.
pub fn suggested_skills_dir(root: &Path) -> PathBuf {
    root.join("memory").join("suggested-skills")
}

/// Weekly consolidation: read every project's memories (and the user's),
/// find duplicates and repeated workflows, draft a SKILL.md per workflow
/// and save the report. Nothing is deleted here; the Memory page offers to
/// forget the duplicates. Blocking.
pub fn consolidate(store: &Store) -> Result<Report, String> {
    let b = backend().ok_or("Supermemory is not signed in")?;
    let mut scopes: Vec<Scope> = store
        .projects()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| store.flag(&project_key(p.id), true) && p.path.is_dir())
        .map(|p| Scope::Project(project_tag(&p.path)))
        .collect();
    scopes.push(Scope::User);
    scopes.dedup();
    let mut report = Report { at: dove_store::now_ms(), ..Default::default() };
    let mut all = Vec::new();
    for scope in scopes {
        let mut list = Vec::new();
        for page in 1..=4 {
            match b.list(&scope, page, 50) {
                Ok(p) if !p.is_empty() => {
                    let short = p.len() < 50;
                    list.extend(p);
                    if short {
                        break;
                    }
                }
                Ok(_) => break,
                Err(e) => {
                    tracing::info!("consolidation: listing {} memories: {e}", scope.label());
                    break;
                }
            }
        }
        report.scanned += list.len();
        for group in duplicates(&list, 0.85) {
            let keep = list[group[0]].clone();
            let copies = group[1..].iter().map(|&i| list[i].clone()).collect();
            report.duplicates.push((scope.clone(), keep, copies));
        }
        all.extend(list);
    }
    report.skills = propose_skills(&all, 3);
    let root = store.root();
    let dir = suggested_skills_dir(root);
    for s in &report.skills {
        let d = dir.join(&s.name);
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::write(d.join("SKILL.md"), s.skill_md());
    }
    if let Some(parent) = Report::path(root).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(Report::path(root), serde_json::to_string_pretty(&report).unwrap_or_default());
    let _ = store.set_setting(CONSOLIDATED_AT, &report.at.to_string());
    Ok(report)
}

/// Whether the weekly consolidation is due.
pub fn consolidation_due(store: &Store) -> bool {
    const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
    let last: i64 = store.setting(CONSOLIDATED_AT).and_then(|s| s.parse().ok()).unwrap_or(0);
    dove_store::now_ms() - last > WEEK_MS
}

// ------------------------------------------------------------------ backfill

/// Project documents worth starting memory from.
const BACKFILL_DOCS: &[&str] = &["README.md", "ARCHITECTURE.md", "CLAUDE.md", "AGENTS.md", "CONTRIBUTING.md"];

/// Everything a first sign-in can warm memory with: each project's docs,
/// recent threads (first prompt and last answer), and merged PR
/// descriptions (via `gh`, when installed). Redacted. Blocking.
pub fn backfill_docs(store: &Store, with_prs: bool) -> Vec<Doc> {
    let prefs = Prefs::load(store, None);
    let mut out = Vec::new();
    for project in store.projects().unwrap_or_default() {
        if !store.flag(&project_key(project.id), true) || !project.path.is_dir() {
            continue;
        }
        let scope = Scope::Project(project_tag(&project.path));
        let meta = |kind: &str| -> BTreeMap<String, String> {
            [("source", "dove"), ("project", project.name.as_str()), ("kind", kind)]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        for name in BACKFILL_DOCS {
            if never_send(name, &prefs.never_send) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(project.path.join(name)) else { continue };
            if text.trim().is_empty() {
                continue;
            }
            out.push(Doc {
                content: redact(&format!("{name} of {}:\n{}", project.name, clip(&text, 8000))),
                scope: scope.clone(),
                metadata: meta("doc"),
                custom_id: Some(format!("dove_doc_{}_{}", sha16(&scope_key(&scope)), sanitize_repo_name(name))),
                at: dove_store::now_ms(),
            });
        }
        let mut tasks = store.tasks(project.id).unwrap_or_default();
        tasks.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        for task in tasks.into_iter().take(30) {
            let events = store.read_transcript(task.id);
            let prompt = events.iter().find_map(|e| match e {
                AgentEvent::UserMessage { text } => Some(crate::strip_persona(text).trim().to_string()),
                _ => None,
            });
            let Some(prompt) = prompt.filter(|p| p.len() >= 8) else { continue };
            // The last text block before the final turn end.
            let mut answer = String::new();
            for e in &events {
                match e {
                    AgentEvent::UserMessage { .. } | AgentEvent::ToolCall { .. } => answer.clear(),
                    AgentEvent::TextDelta { text } if answer.len() < ANSWER_CHARS * 2 => answer.push_str(text),
                    _ => {}
                }
            }
            out.push(Doc {
                content: redact(&format!(
                    "Earlier Dove thread \"{}\" ({}):\nUser: {}\nAgent: {}",
                    task.title,
                    task.harness.label(),
                    clip(&prompt, PROMPT_CHARS),
                    clip(answer.trim(), ANSWER_CHARS)
                )),
                scope: scope.clone(),
                metadata: {
                    let mut m = meta("transcript");
                    m.insert("harness".into(), task.harness.label().to_string());
                    m.insert("task".into(), task.id.to_string());
                    m
                },
                custom_id: Some(format!("dove_thread_{}", task.id)),
                at: dove_store::now_ms(),
            });
        }
        if with_prs && let Some(prs) = merged_prs(&project.path) {
            for (number, title, body) in prs {
                out.push(Doc {
                    content: redact(&format!("Merged PR #{number} in {}: {title}\n{}", project.name, clip(&body, 3000))),
                    scope: scope.clone(),
                    metadata: meta("pull_request"),
                    custom_id: Some(format!("dove_pr_{}_{number}", sha16(&scope_key(&scope)))),
                    at: dove_store::now_ms(),
                });
            }
        }
    }
    out
}

fn scope_key(scope: &Scope) -> String {
    scope.container().unwrap_or("user").to_string()
}

/// The last 20 merged PRs' `(number, title, body)`, via `gh`.
fn merged_prs(dir: &Path) -> Option<Vec<(u64, String, String)>> {
    let mut cmd = std::process::Command::new("gh");
    cmd.args(["pr", "list", "--state", "merged", "--limit", "20", "--json", "number,title,body"])
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok().filter(|o| o.status.success())?;
    let list: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).ok()?;
    Some(
        list.into_iter()
            .filter_map(|v| {
                Some((
                    v.get("number")?.as_u64()?,
                    v.get("title")?.as_str()?.to_string(),
                    v.get("body").and_then(|b| b.as_str()).unwrap_or_default().to_string(),
                ))
            })
            .collect(),
    )
}

/// Queue `docs` and send what can be sent now. Returns `(sent, waiting)`.
pub fn ingest(root: &Path, docs: &[Doc]) -> (usize, usize) {
    let outbox = Outbox::at(root);
    for d in docs {
        let _ = outbox.push(d);
    }
    match backend() {
        Some(b) => outbox.drain(|d| b.add(d)),
        None => (0, outbox.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes_normalize_like_supermemory() {
        assert_eq!(normalize_remote("https://github.com/SupermemoryAI/mono.git").as_deref(), Some("github.com/supermemoryai/mono"));
        assert_eq!(normalize_remote("git@github.com:SupermemoryAI/mono.git").as_deref(), Some("github.com/supermemoryai/mono"));
        assert_eq!(normalize_remote("ssh://git@github.com/a/b.git/").as_deref(), Some("github.com/a/b"));
        assert_eq!(normalize_remote("https://user:pw@GitLab.com:443/G/R?x=1").as_deref(), Some("gitlab.com/g/r"));
        assert_eq!(normalize_remote("https://host:8443/x/y").as_deref(), Some("host:8443/x/y"));
        assert_eq!(normalize_remote("   "), None);
    }

    #[test]
    fn container_tags_match_supermemory() {
        // sha256("github.com/supermemoryai/mono")[..16], as Node computes it.
        let id = "github.com/supermemoryai/mono";
        assert_eq!(sha16(id).len(), 16);
        assert_eq!(sha16("abc"), "ba7816bf8f01cfea");
        assert_eq!(generated_tag("mono", id), format!("repo_mono__{}", sha16(id)));
        assert_eq!(sanitize_repo_name("Cursor Supermemory.js"), "cursor_supermemory_js");
        assert_eq!(sanitize_repo_name("__"), "unknown");
        assert_eq!(remote_name("git@github.com:SupermemoryAI/mono.git").as_deref(), Some("mono"));
        assert_eq!(remote_name("https://github.com/a/Dove/").as_deref(), Some("Dove"));
        assert_eq!(team_tag("Ava Reviewer"), "team_ava_reviewer");
        let long = "x".repeat(200);
        assert!(generated_tag(&long, id).len() <= "repo_".len() + 72 + 2 + 16);
    }

    #[test]
    fn project_tag_uses_the_shared_remote() {
        let dir = tempfile::tempdir().unwrap();
        let ok = std::process::Command::new("git").arg("init").arg("-q").current_dir(dir.path()).status();
        if !ok.is_ok_and(|s| s.success()) {
            return; // no git here
        }
        let _ = std::process::Command::new("git")
            .args(["remote", "add", "origin", "git@github.com:Acme/Widget.git"])
            .current_dir(dir.path())
            .status();
        let tag = project_tag(dir.path());
        assert_eq!(tag, format!("repo_widget__{}", sha16("github.com/acme/widget")));
    }

    #[test]
    fn redacts_credentials() {
        let text = "key sk-ant-api03-abcdefghijklmnopqrstuv and ghp_abcdefghijklmnopqrstuvwxyz0123456789 \
                    AKIAABCDEFGHIJKLMNOP OPENAI_API_KEY=supersecretvalue \"password\": \"hunter2hunter\" \
                    Authorization: Bearer abcdefghijklmnopqrstuvwxyz https://me:pa55w0rd@example.com/x \
                    sm_abcdefghijklmnopqrstuvwx eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4";
        let out = redact(text);
        for secret in [
            "sk-ant-api03",
            "ghp_abc",
            "AKIAABCD",
            "supersecretvalue",
            "hunter2hunter",
            "abcdefghijklmnopqrstuvwxyz ",
            "pa55w0rd",
            "sm_abc",
            "eyJhbGci",
        ] {
            assert!(!out.contains(secret), "{secret} leaked: {out}");
        }
        assert!(out.contains("OPENAI_API_KEY=[REDACTED]"));
        assert!(out.contains("https://me:[REDACTED]@example.com/x"));
        assert!(out.contains("Bearer [REDACTED]"));
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----";
        assert_eq!(redact(pem), "[REDACTED]");
        assert_eq!(redact("a <private>my ssn</private> b"), "a [REDACTED] b");
        // Ordinary prose survives.
        let plain = "Run cargo test -p dove-core, then open the token picker.";
        assert_eq!(redact(plain), plain);
    }

    #[test]
    fn never_send_globs() {
        let g: Vec<String> = DEFAULT_NEVER_SEND.lines().map(str::to_string).collect();
        assert!(never_send(".env", &g));
        assert!(never_send("apps/web/.env.local", &g));
        assert!(never_send("secrets/prod/db.json", &g));
        assert!(never_send("config\\secrets\\a.txt", &g));
        assert!(never_send("certs/server.pem", &g));
        assert!(!never_send("src/main.rs", &g));
        assert!(!never_send("docs/environment.md", &g));
        assert!(never_send("a/b/c.rs", &["a/**".into()]));
        assert!(!never_send("x/a/b.rs", &["a/*".into()]));
    }

    #[test]
    fn block_round_trips_and_respects_budget() {
        let profile = vec![Memory::new("Prefers pnpm over npm")];
        let found = vec![Memory::new("Run tests with\n cargo test -j 2"), Memory::new("   ")];
        let (block, n) = render_block(&profile, &found, BUDGET_CHARS).unwrap();
        assert_eq!(n, 2);
        assert!(block.starts_with(OPEN) && block.contains("count=\"2\""));
        let sent = augment("Fix the build", &block);
        assert_eq!(strip(&sent), "Fix the build");
        assert_eq!(recalled(&sent).unwrap(), vec!["Prefers pnpm over npm", "Run tests with cargo test -j 2"]);
        assert_eq!(strip("plain"), "plain");
        assert!(recalled("plain").is_none());
        assert!(render_block(&[], &[], BUDGET_CHARS).is_none());
        let many: Vec<Memory> = (0..100).map(|i| Memory::new(format!("{i} {}", "word ".repeat(60)))).collect();
        let (block, n) = render_block(&[], &many, BUDGET_CHARS).unwrap();
        assert!(block.len() <= BUDGET_CHARS, "{}", block.len());
        assert!(n > 3 && n < 100);
        // Persona and memory both stripped.
        let p = format!("{block}<dove-persona>\nYou are Ava\n</dove-persona>\n\nHi");
        assert_eq!(crate::strip_persona(&p), "Hi");
    }

    #[test]
    fn standing_rules_lead_with_dates() {
        let rule = Memory { text: "Start answers with ORB:".into(), at: Some("2026-10-20T19:14:05.000Z".into()), ..Default::default() };
        let (block, n) = render_block_with(&[rule], &[], &[Memory::new("Port is 4317")], BUDGET_CHARS).unwrap();
        assert_eq!(n, 2);
        assert!(block.contains("Standing rules:\n- [2026-10-20 19:14] Start answers with ORB:"));
        assert!(block.find("Standing rules").unwrap() < block.find("Relevant").unwrap());
        assert_eq!(recalled(&block).unwrap(), vec!["Start answers with ORB:", "Port is 4317"]);
        assert!(is_rule("My preference: always use tabs"));
        assert!(is_rule("From now on run cargo fmt first"));
        assert!(!is_rule("What is a mutex?"));
        assert!(is_rule("No, that's out of date: the dev server port changed from 4100 to 4317."));
        assert!(is_rule("Correction: staging is orbit-stg now"));
        assert!(!is_rule("Nothing to add, thanks"));
        assert!(!is_rule("Add the dayjs package"));
    }

    #[test]
    fn dedupe_per_session() {
        let mut d = Dedupe::default();
        let first = d.fresh(vec![Memory::new("Uses pnpm"), Memory::new("uses   PNPM"), Memory::new("Tabs not spaces")]);
        assert_eq!(first.len(), 2);
        let again = d.fresh(vec![Memory::new("Tabs not spaces"), Memory::new("New fact")]);
        assert_eq!(again, vec![Memory::new("New fact")]);
        assert_eq!(d.len(), 3);
    }

    fn doc(n: usize) -> Doc {
        Doc { content: format!("m{n}"), scope: Scope::User, metadata: BTreeMap::new(), custom_id: None, at: n as i64 }
    }

    #[test]
    fn outbox_queues_and_drains_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let ob = Outbox::at(dir.path());
        assert!(ob.is_empty());
        for n in 0..5 {
            ob.push(&doc(n)).unwrap();
        }
        assert_eq!(ob.len(), 5);
        // Offline after two sends: those leave, the rest stay in order.
        let mut sent = Vec::new();
        let (ok, left) = ob.drain(|d| {
            if sent.len() < 2 {
                sent.push(d.content.clone());
                Ok(())
            } else {
                Err("offline".into())
            }
        });
        assert_eq!((ok, left), (2, 3));
        assert_eq!(sent, ["m0", "m1"]);
        let mut rest = Vec::new();
        assert_eq!(
            ob.drain(|d| {
                rest.push(d.content.clone());
                Ok(())
            }),
            (3, 0)
        );
        assert_eq!(rest, ["m2", "m3", "m4"]);
        assert!(!ob.path().exists());
        // Corrupt lines are skipped, not fatal.
        std::fs::create_dir_all(ob.path().parent().unwrap()).unwrap();
        std::fs::write(ob.path(), "not json\n").unwrap();
        ob.push(&doc(9)).unwrap();
        assert_eq!(ob.len(), 1);
    }

    #[test]
    fn detects_corrections() {
        let ans = "I used npm install.";
        assert!(correction("No, use pnpm in this repo", "set up deps", ans).is_some());
        assert!(correction("Please don't touch the lockfile", "update", ans).is_some());
        assert!(correction("set up the deps please", "set up the deps", ans).is_some()); // re-asked
        assert!(correction("Great, now add a test", "set up deps", ans).is_none());
        assert!(correction("No, use pnpm", "set up deps", "").is_none()); // nothing to correct
    }

    #[test]
    fn turn_docs_redact_and_filter() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path()).unwrap());
        let s = Session::new(store.clone(), TaskId::new(), ProjectId::new(), HarnessKind::Codex, dir.path().into());
        let s = s.lock();
        let prefs = Prefs::load(&store, None);
        let files: BTreeSet<String> = [".env".to_string(), "src/lib.rs".to_string()].into();
        let d = s.turn_doc("Use key sk-abcdefghijklmnopqrstu please", "Done.", &files, &prefs).unwrap();
        assert!(!d.content.contains("sk-abc"));
        assert!(d.content.contains("src/lib.rs") && !d.content.contains(".env"));
        assert_eq!(d.metadata["source"], "dove");
        assert_eq!(d.metadata["harness"], "Codex");
        assert!(s.turn_doc("/compact", "ok", &files, &prefs).is_none());
        assert!(s.turn_doc("hello there", "", &BTreeSet::new(), &prefs).is_none());
    }

    #[test]
    fn consolidation_finds_duplicates_and_skills() {
        let list = vec![
            Memory::new("Run the tests with `cargo test -j 2` before pushing"),
            Memory::new("before pushing run tests with `cargo test -j 2`"),
            Memory::new("Release: `cargo build --release` then `gh release create`"),
            Memory::new("Lint with `cargo clippy --all-targets`"),
            Memory::new("Prefers dark mode"),
        ];
        assert_eq!(duplicates(&list, 0.8), vec![vec![0, 1]]);
        let skills = propose_skills(&list, 3);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "project-cargo-workflow");
        assert_eq!(skills[0].steps[0], "cargo test -j 2");
        assert!(skills[0].skill_md().starts_with("---\nname: project-cargo-workflow\n"));
    }

    #[test]
    fn prefs_follow_switches() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let p = ProjectId::new();
        assert!(Prefs::load(&store, Some(p)).recall);
        store.set_setting(&project_key(p), "false").unwrap();
        let off = Prefs::load(&store, Some(p));
        assert!(!off.enabled && !off.recall && !off.capture);
        assert!(Prefs::load(&store, None).capture);
        store.set_setting(CAPTURE, "false").unwrap();
        assert!(!Prefs::load(&store, None).capture);
        assert!(!recalls(&store, ProjectId::new()), "no backend registered");
    }
}
