//! Dove's skills library: the bundled skills ([`crate::office_skills`]) plus
//! any skill the user installs from a GitHub repository, kept in one place
//! and mirrored into every harness.
//!
//! `$DOVE_HOME/skills/<name>` holds the canonical copy of each skill and
//! `$DOVE_HOME/skills/registry.json` records what is installed, where it came
//! from and whether it is on. [`sync`] copies the skills that are on into the
//! folders harnesses read ([`harness_dirs`]) and removes the ones that are
//! off. Every folder Dove writes carries a `.dove-skill` marker with a content
//! stamp, so a folder the user made is never overwritten or removed, and an
//! unchanged copy is never rewritten. Agents that read no skill folders get
//! the same text from the `dove_skills` MCP tool ([`describe`]).
//!
//! Discover lists skills from curated repositories ([`CURATED`]) and any
//! repository the user adds, through the GitHub API (`gh auth token` raises
//! the anonymous 60 requests an hour) and `raw.githubusercontent.com`.
//! Listings are cached in `$DOVE_HOME/skills/cache/` for a day.

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::office_skills::{self, Skill};

/// Written into every folder Dove installs, so later runs know they may
/// update or remove it.
pub const MARKER: &str = ".dove-skill";
const REGISTRY: &str = "registry.json";
const CACHE: &str = "cache";
/// How long a repository listing is reused before it is fetched again.
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Limits for one skill download.
const MAX_FILES: usize = 400;
const MAX_FILE_BYTES: u64 = 8 << 20;
const MAX_SKILL_BYTES: u64 = 40 << 20;
/// Most skills one repository listing shows.
const MAX_LISTED: usize = 300;

static ENABLED: AtomicBool = AtomicBool::new(true);
static INSTALLED: OnceLock<Option<PathBuf>> = OnceLock::new();
/// Serializes registry read-modify-write within the process.
static LOCK: Mutex<()> = Mutex::new(());

/// The legacy `agent_office_skills` setting: whether the bundled skills
/// start on when the registry is first created. After that the registry
/// decides, per skill.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// `$DOVE_HOME/skills`, the canonical copy agents can run scripts from.
pub fn root() -> PathBuf {
    std::env::var_os("DOVE_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".dove")))
        .unwrap_or_else(|| PathBuf::from(".dove"))
        .join("skills")
}

/// Where harnesses read user-level skills: `~/.claude/skills` (Claude Code,
/// OpenCode), `~/.agents/skills` (Codex, OpenCode, Cursor, Gemini, Pi and
/// other Agent Skills readers), and the own folders of OpenCode, Gemini CLI
/// and Pi when those are set up here (Dove never creates their config
/// folders).
pub fn harness_dirs() -> Vec<PathBuf> {
    let Some(home) = home() else { return Vec::new() };
    let claude = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home.join(".claude"));
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let mut dirs = vec![claude.join("skills"), home.join(".agents").join("skills")];
    for parent in [config.join("opencode"), home.join(".gemini"), home.join(".pi").join("agent")] {
        if parent.is_dir() {
            dirs.push(parent.join("skills"));
        }
    }
    dirs
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Where an installed skill came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Source {
    /// Compiled into Dove.
    Bundled,
    /// A folder of a GitHub repository; `sha` is the folder's tree hash, so
    /// an update is a changed hash.
    Git { repo: String, path: String, sha: String },
    /// A folder on this machine the user approved: a draft from memory's
    /// consolidation, or a skill they wrote themselves.
    Local { path: String },
}

impl Source {
    /// A short label: "Dove" or `owner/repo`.
    pub fn label(&self) -> &str {
        match self {
            Source::Bundled => "Keplorayx",
            Source::Git { repo, .. } => repo,
            Source::Local { .. } => "You",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installed {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub source: Source,
    #[serde(default = "on")]
    pub enabled: bool,
}

fn on() -> bool {
    true
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub skills: Vec<Installed>,
    /// Repositories the user added to Discover, as `owner/repo`.
    #[serde(default)]
    pub repos: Vec<String>,
}

impl Registry {
    pub fn get(&self, name: &str) -> Option<&Installed> {
        self.skills.iter().find(|s| s.name == name)
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut Installed> {
        self.skills.iter_mut().find(|s| s.name == name)
    }
}

/// Read the registry under `root`, adding bundled skills it does not list
/// yet and dropping bundled skills Dove no longer ships.
fn load_in(root: &Path) -> Registry {
    let path = root.join(REGISTRY);
    let existed = path.is_file();
    let mut reg: Registry = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    reg.skills.retain(|s| s.source != Source::Bundled || office_skills::find(&s.name).is_some());
    let mut bundled = Vec::new();
    for b in office_skills::SKILLS {
        match reg.skills.iter().position(|s| s.name == b.name) {
            Some(i) => {
                let mut s = reg.skills.remove(i);
                if s.source == Source::Bundled {
                    s.description = b.description.into();
                }
                bundled.push(s);
            }
            // The first registry inherits the old on/off setting; a skill
            // bundled later starts on.
            None => bundled.push(Installed {
                name: b.name.into(),
                description: b.description.into(),
                source: Source::Bundled,
                enabled: if existed { true } else { enabled() },
            }),
        }
    }
    bundled.append(&mut reg.skills);
    reg.skills = bundled;
    reg
}

fn save_in(root: &Path, reg: &Registry) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    let tmp = root.join(format!("{REGISTRY}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec_pretty(reg).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, root.join(REGISTRY))
}

/// The registry, with the bundled skills reconciled.
pub fn load() -> Registry {
    load_in(&root())
}

/// Change the registry under the process lock, save it and mirror the
/// result into the harness folders.
fn update<T>(f: impl FnOnce(&mut Registry) -> Result<T, String>) -> Result<T, String> {
    let root = root();
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut reg = load_in(&root);
    let out = f(&mut reg)?;
    save_in(&root, &reg).map_err(|e| format!("could not save {}: {e}", root.join(REGISTRY).display()))?;
    for problem in sync_in(&root, &reg, &harness_dirs()) {
        tracing::info!("skills: {problem}");
    }
    Ok(out)
}

/// Turn one installed skill on or off for every harness.
pub fn set_skill_enabled(name: &str, on: bool) -> Result<(), String> {
    update(|reg| {
        let s = reg.get_mut(name).ok_or_else(|| format!("{name} is not installed"))?;
        s.enabled = on;
        Ok(())
    })
}

/// Uninstall a skill from GitHub: its canonical copy and every Dove copy in
/// the harness folders. Bundled skills can only be turned off.
pub fn remove(name: &str) -> Result<(), String> {
    update(|reg| {
        let s = reg.get(name).ok_or_else(|| format!("{name} is not installed"))?;
        if s.source == Source::Bundled {
            return Err(format!("{name} comes with Keplorayx; turn it off instead"));
        }
        reg.skills.retain(|s| s.name != name);
        for dir in harness_dirs() {
            let _ = unmirror(&dir, name);
        }
        let _ = unmirror(&root(), name);
        Ok(())
    })
}

/// Add a repository to Discover. Accepts `owner/repo` or a GitHub URL;
/// returns the normalized `owner/repo`.
pub fn add_repo(input: &str) -> Result<String, String> {
    let repo = parse_repo(input).ok_or_else(|| format!("{input:?} is not a GitHub repository (owner/repo or URL)"))?;
    update(|reg| {
        if !CURATED.iter().any(|c| c.repo.eq_ignore_ascii_case(&repo))
            && !reg.repos.iter().any(|r| r.eq_ignore_ascii_case(&repo))
        {
            reg.repos.push(repo.clone());
        }
        Ok(repo.clone())
    })
}

/// Drop a repository the user added from Discover (its installed skills
/// stay).
pub fn remove_repo(repo: &str) -> Result<(), String> {
    update(|reg| {
        reg.repos.retain(|r| !r.eq_ignore_ascii_case(repo));
        Ok(())
    })
}

/// `owner/repo` from `owner/repo`, `github.com/owner/repo`, an https or
/// SSH clone URL, or a link into the repository.
pub fn parse_repo(input: &str) -> Option<String> {
    let s = input.trim().trim_end_matches('/');
    let s =
        s.strip_prefix("git@github.com:").or_else(|| s.split_once("github.com/").map(|(_, rest)| rest)).unwrap_or(s);
    let mut parts = s.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?.trim_end_matches(".git");
    let ok = |p: &str| {
        !p.is_empty()
            && p.len() <= 100
            && p != "."
            && p != ".."
            && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (ok(owner) && ok(repo)).then(|| format!("{owner}/{repo}"))
}

// ---------------------------------------------------------------------------
// SKILL.md frontmatter
// ---------------------------------------------------------------------------

/// A skill's `name` and `description` from its frontmatter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    pub name: String,
    pub description: String,
}

/// The `key: value` pairs of a SKILL.md's YAML frontmatter, enough of YAML
/// for skill headers: plain, quoted and block (`>`, `|`) scalars.
fn frontmatter(md: &str) -> Option<BTreeMap<String, String>> {
    let md = md.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let body = md.strip_prefix("---\n")?;
    let end = body.find("\n---").map(|i| i + 1).or_else(|| body.starts_with("---").then_some(0))?;
    let mut out = BTreeMap::new();
    let mut lines = body[..end].lines().peekable();
    while let Some(line) = lines.next() {
        if line.starts_with([' ', '\t', '#']) || line.trim().is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        let mut more = Vec::new();
        while let Some(next) = lines.peek() {
            if next.starts_with([' ', '\t']) || next.trim().is_empty() {
                more.push(next.trim());
                lines.next();
            } else {
                break;
            }
        }
        while more.last().is_some_and(|l| l.is_empty()) {
            more.pop();
        }
        let value = if value.starts_with('|') {
            more.join("\n")
        } else if value.starts_with('>') {
            more.join(" ")
        } else {
            let mut v = std::iter::once(value).chain(more.iter().copied()).collect::<Vec<_>>().join(" ");
            for q in ['"', '\''] {
                if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
                    v = v[1..v.len() - 1].to_string();
                    if q == '"' {
                        v = v.replace("\\\"", "\"").replace("\\\\", "\\");
                    } else {
                        v = v.replace("''", "'");
                    }
                }
            }
            v
        };
        out.insert(key.trim().to_string(), value.trim().to_string());
    }
    Some(out)
}

/// Check a SKILL.md against the Agent Skills rules for the folder `folder`:
/// frontmatter with a `name` of 1–64 `[a-z0-9-]` equal to the folder name,
/// and a `description` of at most 1024 characters.
pub fn validate(folder: &str, md: &str) -> Result<Meta, String> {
    let meta = parse_meta(md)?;
    if meta.name != folder {
        return Err(format!("skill name {:?} does not match its folder {folder:?}", meta.name));
    }
    Ok(meta)
}

/// [`validate`] without the folder rule. Many libraries keep a skill in a
/// folder named differently from the skill; Dove installs it under its own
/// name, which satisfies the rule in every harness folder.
pub fn parse_meta(md: &str) -> Result<Meta, String> {
    let fm = frontmatter(md).ok_or("SKILL.md has no frontmatter (--- name/description ---)")?;
    let name = fm.get("name").map(String::as_str).unwrap_or_default();
    if name.is_empty() {
        return Err("SKILL.md frontmatter has no name".into());
    }
    if name.len() > 64 || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(format!("skill name {name:?} must be 1-64 lowercase letters, digits and hyphens"));
    }
    let description = fm.get("description").map(String::as_str).unwrap_or_default();
    if description.is_empty() {
        return Err("SKILL.md frontmatter has no description".into());
    }
    if description.chars().count() > 1024 {
        return Err("the skill description is longer than 1024 characters".into());
    }
    Ok(Meta { name: name.into(), description: description.into() })
}

// ---------------------------------------------------------------------------
// Writing and mirroring folders
// ---------------------------------------------------------------------------

/// FNV-1a over `(path, bytes)` pairs: stable across builds and platforms.
fn fnv<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (path, bytes) in files {
        for b in path.bytes().chain(bytes.iter().copied()) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("dove {} {h:016x}\n", env!("CARGO_PKG_VERSION"))
}

/// Every file under `dir` except the marker, as sorted `/`-joined relative
/// paths.
fn dir_files(dir: &Path) -> std::io::Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((path, rel)) = stack.pop() {
        for entry in std::fs::read_dir(&path)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let kind = entry.file_type()?;
            if kind.is_dir() {
                stack.push((entry.path(), rel));
            } else if kind.is_file() && rel != MARKER {
                out.push((rel, entry.path()));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The content stamp of a folder's files.
fn dir_stamp(dir: &Path) -> std::io::Result<String> {
    let files = dir_files(dir)?
        .into_iter()
        .map(|(rel, path)| Ok((rel, std::fs::read(path)?)))
        .collect::<std::io::Result<Vec<_>>>()?;
    Ok(fnv(files.iter().map(|(r, b)| (r.as_str(), b.as_slice()))))
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// What [`mirror`] did with one folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mirror {
    Written,
    /// Dove's copy was already current.
    Current,
    /// A folder Dove did not write is there; it was left alone.
    Foreign,
}

/// Whether `dest` holds a folder Dove wrote: a real directory (not a link)
/// with the marker.
fn ours(dest: &Path) -> bool {
    std::fs::symlink_metadata(dest).is_ok_and(|m| m.is_dir()) && dest.join(MARKER).is_file()
}

/// Write `files` as `dir/<name>` unless a folder Dove did not create is
/// already there.
fn write_files(dir: &Path, name: &str, files: &[(&str, &[u8])], stamp: &str) -> std::io::Result<Mirror> {
    let dest = dir.join(name);
    if std::fs::symlink_metadata(&dest).is_ok() {
        if !ours(&dest) {
            return Ok(Mirror::Foreign);
        }
        if std::fs::read_to_string(dest.join(MARKER)).is_ok_and(|s| s == stamp) {
            return Ok(Mirror::Current);
        }
        std::fs::remove_dir_all(&dest)?;
    }
    for (rel, bytes) in files {
        let path = dest.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, bytes)?;
        if rel.ends_with(".py") || rel.ends_with(".sh") {
            make_executable(&path);
        }
    }
    std::fs::write(dest.join(MARKER), stamp)?;
    Ok(Mirror::Written)
}

/// Write a bundled skill's canonical copy into `root`.
fn write_bundled(skill: &Skill, root: &Path) -> std::io::Result<Mirror> {
    let files: Vec<(&str, &[u8])> = skill.files.iter().map(|(p, t)| (*p, t.as_bytes())).collect();
    write_files(root, skill.name, &files, &fnv(sorted(&files)))
}

fn sorted<'a>(files: &[(&'a str, &'a [u8])]) -> Vec<(&'a str, &'a [u8])> {
    let mut v = files.to_vec();
    v.sort_by_key(|(p, _)| *p);
    v
}

/// Copy the skill folder `src` into `dir/<name>`, protected by the marker.
fn mirror(src: &Path, dir: &Path, name: &str) -> std::io::Result<Mirror> {
    let files = dir_files(src)?
        .into_iter()
        .map(|(rel, path)| Ok((rel, std::fs::read(path)?)))
        .collect::<std::io::Result<Vec<_>>>()?;
    let files: Vec<(&str, &[u8])> = files.iter().map(|(r, b)| (r.as_str(), b.as_slice())).collect();
    write_files(dir, name, &files, &fnv(files.iter().copied()))
}

/// Remove `dir/<name>` if Dove wrote it. Returns whether it did.
fn unmirror(dir: &Path, name: &str) -> std::io::Result<bool> {
    let dest = dir.join(name);
    if !ours(&dest) {
        return Ok(false);
    }
    std::fs::remove_dir_all(dest)?;
    Ok(true)
}

/// Mirror every skill that is on from `root` into `dirs` and remove Dove's
/// copies of the ones that are off. Returns what it could not do.
fn sync_in(root: &Path, reg: &Registry, dirs: &[PathBuf]) -> Vec<String> {
    let mut problems = Vec::new();
    for dir in dirs {
        for s in &reg.skills {
            let src = root.join(&s.name);
            let result = if s.enabled && src.join("SKILL.md").is_file() {
                mirror(&src, dir, &s.name).map(|m| {
                    if m == Mirror::Foreign {
                        problems.push(format!("kept the existing {} skill in {} (not Keplorayx's)", s.name, dir.display()));
                    }
                })
            } else {
                unmirror(dir, &s.name).map(|_| ())
            };
            if let Err(e) = result {
                problems.push(format!("could not update the {} skill in {}: {e}", s.name, dir.display()));
            }
        }
    }
    problems
}

/// Mirror the registry into every harness folder now. Returns what it could
/// not do (a foreign folder in the way, an unwritable folder).
pub fn sync() -> Vec<String> {
    let root = root();
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    sync_in(&root, &load_in(&root), &harness_dirs())
}

/// Install the bundled skills and mirror the registry (once per process).
/// Returns `$DOVE_HOME/skills` when the canonical copies are in place.
pub fn ensure_installed() -> Option<PathBuf> {
    INSTALLED
        .get_or_init(|| {
            let root = root();
            let mut ok = true;
            for skill in office_skills::SKILLS {
                match write_bundled(skill, &root) {
                    Ok(Mirror::Foreign) => {
                        tracing::info!("kept the existing {} folder in {} (not Dove's)", skill.name, root.display())
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("could not write the {} skill to {}: {e}", skill.name, root.display());
                        ok = false;
                    }
                }
            }
            let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let reg = load_in(&root);
            if let Err(e) = save_in(&root, &reg) {
                tracing::warn!("could not save the skills registry in {}: {e}", root.display());
            }
            for problem in sync_in(&root, &reg, &harness_dirs()) {
                tracing::info!("skills: {problem}");
            }
            ok.then_some(root)
        })
        .clone()
}

// ---------------------------------------------------------------------------
// The dove_skills tool
// ---------------------------------------------------------------------------

/// The `dove_skills` tool's answer: the catalog, or one skill's full text.
pub fn describe(name: Option<&str>) -> Result<String, String> {
    describe_in(&root(), &load(), name)
}

fn describe_in(root: &Path, reg: &Registry, name: Option<&str>) -> Result<String, String> {
    let on: Vec<&Installed> = reg.skills.iter().filter(|s| s.enabled).collect();
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        None => {
            let mut out = String::from(
                "Skills Keplorayx installed for agents. Call dove_skills with {\"name\": ...} for the full instructions.\n\n",
            );
            if on.is_empty() {
                out.push_str("(none are turned on; the user manages them in Keplorayx → Settings → Plugins → Skills)\n");
            }
            for s in on {
                out.push_str(&format!("- {}: {} (folder: {})\n", s.name, s.description, root.join(&s.name).display()));
            }
            Ok(out)
        }
        Some(n) => {
            let n = n.to_ascii_lowercase();
            let skill = on.iter().find(|s| s.name == n).ok_or_else(|| {
                format!(
                    "No skill named {n:?} is on. Available: {}",
                    on.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")
                )
            })?;
            let dir = root.join(&skill.name);
            let body = std::fs::read_to_string(dir.join("SKILL.md"))
                .ok()
                .or_else(|| {
                    office_skills::find(&skill.name)
                        .and_then(|b| b.files.iter().find(|(p, _)| *p == "SKILL.md"))
                        .map(|(_, t)| t.to_string())
                })
                .ok_or_else(|| format!("the {n} skill's SKILL.md is missing from {}", dir.display()))?;
            let mut head = format!("Skill folder: {}\n", dir.display());
            if dir.join("scripts").is_dir() {
                head.push_str(
                    "Run its scripts from there; Python scripts with `uv run <script>` (or python3 after installing \
                     their dependencies). ",
                );
            }
            head.push_str("Relative paths below are inside the skill folder.\n\n");
            Ok(head + &body)
        }
    }
}

// ---------------------------------------------------------------------------
// GitHub
// ---------------------------------------------------------------------------

/// A repository Discover offers out of the box.
pub struct Curated {
    pub repo: &'static str,
    pub label: &'static str,
    /// `Some(dir)`: only skill folders directly inside `dir` ("" for the
    /// repository root). `None`: any `SKILL.md` folder.
    pub under: Option<&'static str>,
    /// Skill names not offered (Dove bundles its own).
    pub skip: &'static [&'static str],
    /// The category of every skill here, when the library is about one
    /// thing; otherwise each skill is sorted by its words ([`category`]).
    pub category: Option<&'static str>,
}

const fn lib(repo: &'static str, label: &'static str, under: Option<&'static str>) -> Curated {
    Curated { repo, label, under, skip: &[], category: None }
}

/// The libraries Discover offers: the most used skill collections on GitHub,
/// chosen to cover every category, official ones first.
pub const CURATED: &[Curated] = &[
    lib("obra/superpowers", "Superpowers", Some("skills")),
    Curated { skip: &["docx", "pdf", "pptx", "xlsx"], ..lib("anthropics/skills", "Anthropic", Some("skills")) },
    lib("openai/skills", "OpenAI", Some("skills/.curated")),
    lib("addyosmani/agent-skills", "Addy Osmani", Some("skills")),
    lib("vercel-labs/agent-skills", "Vercel", Some("skills")),
    Curated { category: Some("Design"), ..lib("Leonxlnx/taste-skill", "Taste", Some("skills")) },
    Curated { category: Some("Design"), ..lib("cathrynlavery/diagram-design", "Diagram Design", Some("skills")) },
    Curated { category: Some("Design"), ..lib("google-labs-code/stitch-skills", "Google Stitch", None) },
    Curated { category: Some("Writing"), ..lib("blader/humanizer", "Humanizer", None) },
    Curated { category: Some("Marketing"), ..lib("coreyhaines31/marketingskills", "Marketing", Some("skills")) },
    Curated { category: Some("Finance"), ..lib("anthropics/financial-services", "Financial services", None) },
    Curated {
        category: Some("Data & science"),
        ..lib("K-Dense-AI/scientific-agent-skills", "Scientific", Some("skills"))
    },
    // Only the skills in the repository itself: the rest of its list links
    // elsewhere, `composio-skills/` is hundreds of generated toolkits, and
    // connect/connect-apps set up MCP connections (Settings › Plugins › Apps).
    Curated { skip: &["connect", "connect-apps"], ..lib("ComposioHQ/awesome-claude-skills", "Composio", Some("")) },
    Curated { category: Some("Web & browser"), ..lib("CitroLabs/ego-lite", "ego(lite)", Some("skills")) },
];

/// Skills the `#` menu suggests until they are installed:
/// `(repo, folder, skill name, what it does)`. Picking one installs it.
pub const RECOMMENDED: &[(&str, &str, &str, &str)] = &[
    ("obra/superpowers", "skills/brainstorming", "brainstorming", "Turn a rough idea into a design before coding"),
    (
        "obra/superpowers",
        "skills/systematic-debugging",
        "systematic-debugging",
        "Find the root cause before fixing a bug",
    ),
    ("obra/superpowers", "skills/test-driven-development", "test-driven-development", "Write the failing test first"),
    ("obra/superpowers", "skills/writing-plans", "writing-plans", "Write a step-by-step implementation plan"),
    ("anthropics/skills", "skills/frontend-design", "frontend-design", "Distinctive, production-grade interfaces"),
    ("anthropics/skills", "skills/webapp-testing", "webapp-testing", "Test web apps with Playwright"),
    ("anthropics/skills", "skills/skill-creator", "skill-creator", "Create and improve skills"),
    (
        "Leonxlnx/taste-skill",
        "skills/taste-skill",
        "design-taste-frontend",
        "Design with taste: layout, type and colour",
    ),
    ("cathrynlavery/diagram-design", "skills/diagram-design", "diagram-design", "Clear, good-looking diagrams"),
    ("blader/humanizer", "", "humanizer", "Make AI-written text read naturally"),
    ("coreyhaines31/marketingskills", "skills/copywriting", "copywriting", "Marketing copy that converts"),
    (
        "anthropics/financial-services",
        "plugins/agent-plugins/market-researcher/skills/comps-analysis",
        "comps-analysis",
        "Comparable-company analysis",
    ),
    ("CitroLabs/ego-lite", "skills/ego-browser", "ego-browser", "Use websites in your own browser"),
];

/// Skill categories, in the order filters show them.
pub const CATEGORIES: &[&str] = &[
    "Engineering",
    "Design",
    "Work",
    "Writing",
    "Documents",
    "Data & science",
    "Finance",
    "Marketing",
    "Web & browser",
    "Other",
];

/// Words that place a skill in a category, checked in this order. A
/// trailing `*` matches any word starting with the stem.
const CATEGORY_WORDS: &[(&str, &[&str])] = &[
    ("Documents", &["docx", "pdf", "pptx", "xlsx", "word", "powerpoint", "excel", "spreadsheet*", "slide*", "deck*"]),
    (
        "Finance",
        &[
            "financ*",
            "invoice*",
            "accounting",
            "budget*",
            "stock*",
            "trading",
            "tax*",
            "expense*",
            "earnings",
            "valuation",
            "portfolio*",
            "kyc",
            "ledger",
        ],
    ),
    (
        "Design",
        &[
            "design*",
            "ui",
            "ux",
            "figma",
            "canvas",
            "art",
            "algorithmic",
            "brand*",
            "theme*",
            "visual*",
            "css",
            "frontend",
            "image*",
            "gif",
            "color*",
            "illustrat*",
            "diagram*",
            "taste",
            "typography",
            "animation*",
        ],
    ),
    ("Web & browser", &["browser*", "website*", "scrap*", "playwright", "chrome", "crawl*", "devtools", "web"]),
    (
        "Marketing",
        &[
            "marketing",
            "seo",
            "ads",
            "ad",
            "social",
            "twitter",
            "lead*",
            "campaign*",
            "growth",
            "competitive",
            "pricing",
            "launch*",
            "copywriting",
            "cro",
            "email*",
        ],
    ),
    (
        "Writing",
        &[
            "writ*",
            "blog*",
            "content",
            "copy",
            "comms",
            "newsletter*",
            "resume",
            "coauthor*",
            "humaniz*",
            "editing",
            "prose",
            "article*",
        ],
    ),
    (
        "Data & science",
        &[
            "data",
            "dataset*",
            "analytics",
            "sql",
            "csv",
            "chart*",
            "research*",
            "analysis",
            "scientific",
            "science",
            "statistic*",
            "bio*",
            "chem*",
            "genom*",
            "protein*",
        ],
    ),
    (
        "Engineering",
        &[
            "code",
            "coding",
            "debug*",
            "test*",
            "tdd",
            "git",
            "branch*",
            "review*",
            "plan*",
            "mcp",
            "api*",
            "deploy*",
            "refactor*",
            "develop*",
            "react",
            "performance",
            "ci",
            "cli",
            "build*",
            "server*",
            "subagent*",
            "security",
            "architecture",
            "aspnet",
            "cloudflare",
            "vercel",
        ],
    ),
    (
        "Work",
        &[
            "meeting*",
            "calendar",
            "slack",
            "task*",
            "organiz*",
            "file*",
            "project*",
            "productivity",
            "notes",
            "brainstorm*",
            "workflow*",
            "automation*",
            "invoice*",
            "career",
            "internal",
        ],
    ),
];

/// The category of a skill: its library's, else the first whose words
/// appear in the name or description.
pub fn category(repo: Option<&str>, name: &str, description: &str) -> &'static str {
    if let Some(c) = repo.and_then(|r| CURATED.iter().find(|c| c.repo.eq_ignore_ascii_case(r))).and_then(|c| c.category)
    {
        return c;
    }
    let text = format!("{name} {description}").to_ascii_lowercase();
    let words: Vec<&str> = text.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    // The name counts first: "pdf" decides before a passing "design".
    let name = name.to_ascii_lowercase();
    let name_words: Vec<&str> = name.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let hit = |list: &[&str], ws: &[&str]| {
        list.iter().any(|k| match k.strip_suffix('*') {
            Some(stem) => ws.iter().any(|w| w.starts_with(stem)),
            None => ws.contains(k),
        })
    };
    CATEGORY_WORDS
        .iter()
        .find(|(_, list)| hit(list, &name_words))
        .or_else(|| CATEGORY_WORDS.iter().find(|(_, list)| hit(list, &words)))
        .map(|(c, _)| *c)
        .unwrap_or("Other")
}

/// The category of an installed skill (bundled ones are documents).
pub fn installed_category(s: &Installed) -> &'static str {
    match &s.source {
        Source::Bundled => category(None, &s.name, &s.description),
        Source::Git { repo, .. } => category(Some(repo), &s.name, &s.description),
        Source::Local { .. } => category(None, &s.name, &s.description),
    }
}

/// A skill a repository offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Available {
    pub name: String,
    pub description: String,
    /// The skill folder inside the repository.
    pub path: String,
    /// The folder's tree hash.
    pub sha: String,
    /// Why it cannot be installed (invalid frontmatter), if so.
    #[serde(default)]
    pub problem: Option<String>,
}

/// One repository's skills, as cached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    pub repo: String,
    /// Unix seconds.
    pub fetched_at: u64,
    pub skills: Vec<Available>,
}

#[derive(Clone, Debug, Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    sha: String,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
struct Tree {
    sha: String,
    tree: Vec<TreeEntry>,
}

fn client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .user_agent(concat!("dove/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default()
    })
}

/// A GitHub token: `GH_TOKEN`/`GITHUB_TOKEN`, else the GitHub CLI's.
fn token() -> Option<String> {
    static TOKEN: OnceLock<Option<String>> = OnceLock::new();
    TOKEN
        .get_or_init(|| {
            for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
                if let Ok(t) = std::env::var(var)
                    && !t.trim().is_empty()
                {
                    return Some(t.trim().to_string());
                }
            }
            let gh = which::which("gh").ok()?;
            let mut cmd = std::process::Command::new(gh);
            cmd.args(["auth", "token"]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }
            let out = cmd.output().ok()?;
            let t = String::from_utf8(out.stdout).ok()?.trim().to_string();
            (out.status.success() && !t.is_empty()).then_some(t)
        })
        .clone()
}

fn get(url: &str, api: bool) -> Result<Vec<u8>, String> {
    let send = |auth: bool| {
        let mut req = client().get(url);
        if api {
            req = req.header("Accept", "application/vnd.github+json").header("X-GitHub-Api-Version", "2022-11-28");
        }
        if auth && let Some(t) = token() {
            req = req.bearer_auth(t);
        }
        req.send().map_err(|e| format!("could not reach GitHub: {e}"))
    };
    let mut resp = send(true)?;
    // A stale token: try once without it (public repositories still work).
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED && token().is_some() {
        resp = send(false)?;
    }
    let status = resp.status();
    if status.is_success() {
        return resp.bytes().map(|b| b.to_vec()).map_err(|e| format!("GitHub download failed: {e}"));
    }
    let limited = resp.headers().get("x-ratelimit-remaining").is_some_and(|v| v == "0");
    Err(match status.as_u16() {
        403 | 429 if limited => {
            "GitHub's rate limit is reached. Sign in with the GitHub CLI (gh auth login) to raise it, or try later."
                .into()
        }
        404 => format!("not found on GitHub: {url}"),
        _ => format!("GitHub answered {status} for {url}"),
    })
}

/// Percent-encode one path segment for a URL.
fn encode(segment: &str) -> String {
    let mut out = String::new();
    for b in segment.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn raw(repo: &str, path: &str) -> Result<Vec<u8>, String> {
    let path = path.split('/').map(encode).collect::<Vec<_>>().join("/");
    get(&format!("https://raw.githubusercontent.com/{repo}/HEAD/{path}"), false)
}

fn fetch_tree(repo: &str) -> Result<Tree, String> {
    let bytes = get(&format!("https://api.github.com/repos/{repo}/git/trees/HEAD?recursive=1"), true)?;
    serde_json::from_slice(&bytes).map_err(|e| format!("unexpected answer from GitHub for {repo}: {e}"))
}

/// The skill folder name for `path` (the repository's name at its root).
fn folder_name<'a>(repo: &'a str, path: &'a str) -> &'a str {
    if path.is_empty() { repo.rsplit('/').next().unwrap_or(repo) } else { path.rsplit('/').next().unwrap_or(path) }
}

/// The skill folders of a repository tree: `(path, tree hash)`.
fn skill_folders(repo: &str, tree: &Tree, rule: Option<&Curated>) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = tree
        .tree
        .iter()
        .filter(|e| e.kind == "blob" && (e.path == "SKILL.md" || e.path.ends_with("/SKILL.md")))
        .map(|e| e.path.strip_suffix("SKILL.md").unwrap_or("").trim_end_matches('/').to_string())
        .filter(|folder| {
            let below = match rule.and_then(|r| r.under) {
                Some(dir) if !dir.is_empty() => folder.strip_prefix(dir).unwrap_or(folder),
                _ => folder,
            };
            !below.split('/').any(|seg| seg.starts_with('.'))
        })
        .filter(|folder| match rule.and_then(|r| r.under) {
            Some("") => !folder.is_empty() && !folder.contains('/'),
            Some(dir) => folder
                .strip_prefix(dir)
                .and_then(|rest| rest.strip_prefix('/'))
                .is_some_and(|name| !name.is_empty() && !name.contains('/')),
            None => true,
        })
        .filter(|folder| rule.is_none_or(|r| !r.skip.contains(&folder_name(repo, folder))))
        .filter_map(|folder| {
            let sha = if folder.is_empty() {
                tree.sha.clone()
            } else {
                tree.tree.iter().find(|e| e.kind == "tree" && e.path == folder)?.sha.clone()
            };
            Some((folder, sha))
        })
        .collect();
    found.sort();
    // A SKILL.md inside another skill (an example, a template) is not its own
    // skill.
    let all: HashSet<String> = found.iter().map(|(f, _)| f.clone()).collect();
    found.retain(|(f, _)| {
        let mut p = f.as_str();
        while let Some((parent, _)) = p.rsplit_once('/') {
            if all.contains(parent) {
                return false;
            }
            p = parent;
        }
        f.is_empty() || !all.contains("")
    });
    // The same skill copied into several plugins: list it once.
    let mut names = HashSet::new();
    found.retain(|(f, _)| names.insert(folder_name(repo, f).to_string()));
    found.truncate(MAX_LISTED);
    found
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn cache_path(root: &Path, repo: &str) -> PathBuf {
    root.join(CACHE).join(format!("{}.json", repo.replace('/', "__")))
}

/// The cached listing of `repo`, however old.
pub fn cached_listing(repo: &str) -> Option<Listing> {
    std::fs::read(cache_path(&root(), repo)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// The skills `repo` offers: from the day-old cache unless `refresh`, else
/// from GitHub (falling back to a stale cache when GitHub is unreachable).
pub fn listing(repo: &str, refresh: bool) -> Result<Listing, String> {
    let cached = cached_listing(repo);
    if !refresh
        && let Some(l) = &cached
        && now().saturating_sub(l.fetched_at) < CACHE_TTL.as_secs()
    {
        return Ok(l.clone());
    }
    match fetch_listing(repo) {
        Ok(l) => {
            let path = cache_path(&root(), repo);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(bytes) = serde_json::to_vec(&l) {
                let _ = std::fs::write(path, bytes);
            }
            Ok(l)
        }
        Err(e) => cached.ok_or(e),
    }
}

fn fetch_listing(repo: &str) -> Result<Listing, String> {
    let rule = CURATED.iter().find(|c| c.repo.eq_ignore_ascii_case(repo));
    let tree = fetch_tree(repo)?;
    let folders = skill_folders(repo, &tree, rule);
    // Read every SKILL.md for its description, a few at a time.
    let mut skills: Vec<Available> = Vec::with_capacity(folders.len());
    for chunk in folders.chunks(8) {
        let read: Vec<Available> = std::thread::scope(|s| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|(path, sha)| {
                    s.spawn(move || {
                        let md_path = if path.is_empty() { "SKILL.md".to_string() } else { format!("{path}/SKILL.md") };
                        let checked = raw(repo, &md_path)
                            .and_then(|b| String::from_utf8(b).map_err(|_| "SKILL.md is not UTF-8".to_string()))
                            .and_then(|md| parse_meta(&md));
                        let (name, description, problem) = match checked {
                            Ok(meta) => (meta.name, meta.description, None),
                            Err(e) => (folder_name(repo, path).to_string(), String::new(), Some(e)),
                        };
                        Available { name, description, path: path.clone(), sha: sha.clone(), problem }
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        skills.extend(read);
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Listing { repo: repo.to_string(), fetched_at: now(), skills })
}

/// Download the skill folder `path` of `repo` (described by `tree`) into the
/// canonical root and record it. Keeps the skill's on/off state when it is
/// already installed.
fn install_from(repo: &str, tree: &Tree, path: &str) -> Result<Installed, String> {
    let path = path.trim_matches('/');
    let prefix = if path.is_empty() { String::new() } else { format!("{path}/") };
    let md = raw(repo, &format!("{prefix}SKILL.md"))?;
    let meta = parse_meta(&String::from_utf8_lossy(&md))?;
    let name = meta.name.clone();
    if office_skills::find(&name).is_some() || name == CACHE {
        return Err(format!("{name} is a name Keplorayx reserves for its own skills"));
    }
    let sha = if path.is_empty() {
        tree.sha.clone()
    } else {
        tree.tree
            .iter()
            .find(|e| e.kind == "tree" && e.path == path)
            .map(|e| e.sha.clone())
            .ok_or_else(|| format!("{repo} has no folder {path}"))?
    };
    if let Some(existing) = load().get(&name)
        && !matches!(&existing.source, Source::Git { repo: r, path: p, .. } if r == repo && p == path)
    {
        return Err(format!("a skill named {name} is already installed from {}", existing.source.label()));
    }
    let files: Vec<&TreeEntry> = tree
        .tree
        .iter()
        .filter(|e| e.kind == "blob" && e.path.starts_with(&prefix))
        .filter(|e| !e.path[prefix.len()..].split('/').any(|seg| seg == ".git" || seg == MARKER))
        .collect();
    if files.len() > MAX_FILES {
        return Err(format!("{name} has more than {MAX_FILES} files"));
    }
    let total: u64 = files.iter().map(|e| e.size.unwrap_or(0)).sum();
    if total > MAX_SKILL_BYTES || files.iter().any(|e| e.size.unwrap_or(0) > MAX_FILE_BYTES) {
        return Err(format!("{name} is too large to install"));
    }

    let root = root();
    let staging = root.join(format!(".staging-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    let written = (|| -> Result<(), String> {
        for e in &files {
            let rel = &e.path[prefix.len()..];
            let rel_path = Path::new(rel);
            if !rel_path.components().all(|c| matches!(c, Component::Normal(_))) {
                return Err(format!("{name} has an unsafe path {rel:?}"));
            }
            let bytes = if rel == "SKILL.md" { md.clone() } else { raw(repo, &e.path)? };
            let dest = staging.join(rel_path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&dest, bytes).map_err(|e| e.to_string())?;
            if rel.ends_with(".sh") || rel.ends_with(".py") || rel.starts_with("scripts/") {
                make_executable(&dest);
            }
        }
        let stamp = dir_stamp(&staging).map_err(|e| e.to_string())?;
        std::fs::write(staging.join(MARKER), stamp).map_err(|e| e.to_string())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("could not download {name}: {e}"));
    }

    update(|reg| {
        let dest = root.join(&name);
        if std::fs::symlink_metadata(&dest).is_ok() {
            if !ours(&dest) {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(format!("{} already has a {name} folder Keplorayx did not make", root.display()));
            }
            std::fs::remove_dir_all(&dest).map_err(|e| format!("could not replace {}: {e}", dest.display()))?;
        }
        std::fs::rename(&staging, &dest).map_err(|e| format!("could not install {name}: {e}"))?;
        let installed = Installed {
            name: name.clone(),
            description: meta.description.clone(),
            source: Source::Git { repo: repo.to_string(), path: path.to_string(), sha: sha.clone() },
            enabled: reg.get(&name).is_none_or(|s| s.enabled),
        };
        match reg.get_mut(&name) {
            Some(s) => *s = installed.clone(),
            None => reg.skills.push(installed.clone()),
        }
        Ok(installed)
    })
}

/// Install the skill folder `dir` from this machine (a suggested draft, or
/// a skill the user wrote) and turn it on for every harness. The folder is
/// copied, so later edits to the draft need another install.
pub fn install_local(dir: &Path) -> Result<Installed, String> {
    let md = std::fs::read(dir.join("SKILL.md")).map_err(|e| format!("{} has no SKILL.md: {e}", dir.display()))?;
    let meta = parse_meta(&String::from_utf8_lossy(&md))?;
    let name = meta.name.clone();
    if office_skills::find(&name).is_some() || name == CACHE {
        return Err(format!("{name} is a name Keplorayx reserves for its own skills"));
    }
    if let Some(existing) = load().get(&name)
        && !matches!(&existing.source, Source::Local { .. })
    {
        return Err(format!("a skill named {name} is already installed from {}", existing.source.label()));
    }
    let mut files = Vec::new();
    collect_files(dir, dir, &mut files).map_err(|e| format!("could not read {}: {e}", dir.display()))?;
    if files.len() > MAX_FILES {
        return Err(format!("{name} has more than {MAX_FILES} files"));
    }
    let total: u64 = files.iter().filter_map(|f| std::fs::metadata(dir.join(f)).ok()).map(|m| m.len()).sum();
    if total > MAX_SKILL_BYTES {
        return Err(format!("{name} is too large to install"));
    }

    let root = root();
    let staging = root.join(format!(".staging-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    let written = (|| -> std::io::Result<()> {
        for rel in &files {
            let dest = staging.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(dir.join(rel), &dest)?;
            let rel = rel.to_string_lossy();
            if rel.ends_with(".sh") || rel.ends_with(".py") || rel.starts_with("scripts") {
                make_executable(&dest);
            }
        }
        let stamp = dir_stamp(&staging)?;
        std::fs::write(staging.join(MARKER), stamp)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("could not copy {name}: {e}"));
    }

    update(|reg| {
        let dest = root.join(&name);
        if std::fs::symlink_metadata(&dest).is_ok() {
            if !ours(&dest) {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(format!("{} already has a {name} folder Keplorayx did not make", root.display()));
            }
            std::fs::remove_dir_all(&dest).map_err(|e| format!("could not replace {}: {e}", dest.display()))?;
        }
        std::fs::rename(&staging, &dest).map_err(|e| format!("could not install {name}: {e}"))?;
        let installed = Installed {
            name: name.clone(),
            description: meta.description.clone(),
            source: Source::Local { path: dir.display().to_string() },
            enabled: reg.get(&name).is_none_or(|s| s.enabled),
        };
        match reg.get_mut(&name) {
            Some(s) => *s = installed.clone(),
            None => reg.skills.push(installed.clone()),
        }
        Ok(installed)
    })
}

/// Every regular file under `dir`, relative to `base`, skipping `.git` and
/// Dove's marker; symlinks are not followed.
fn collect_files(base: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" || name == MARKER {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_files(base, &entry.path(), out)?;
        } else if kind.is_file() {
            out.push(entry.path().strip_prefix(base).map(Path::to_path_buf).unwrap_or_default());
        }
    }
    Ok(())
}

/// Install the skill in folder `path` of the GitHub repository `repo`
/// (`owner/repo`) and turn it on for every harness.
pub fn install(repo: &str, path: &str) -> Result<Installed, String> {
    let repo = parse_repo(repo).ok_or_else(|| format!("{repo:?} is not a GitHub repository"))?;
    let tree = fetch_tree(&repo)?;
    install_from(&repo, &tree, path)
}

/// Reinstall every skill from GitHub whose folder changed upstream. Returns
/// the names updated; skills that could not be checked are reported in the
/// error, after the others are done.
pub fn update_all() -> Result<Vec<String>, String> {
    let mut by_repo: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
    for s in load().skills {
        if let Source::Git { repo, path, sha } = s.source {
            by_repo.entry(repo).or_default().push((s.name, path, sha));
        }
    }
    let mut updated = Vec::new();
    let mut problems = Vec::new();
    for (repo, skills) in by_repo {
        let tree = match fetch_tree(&repo) {
            Ok(t) => t,
            Err(e) => {
                problems.push(e);
                continue;
            }
        };
        for (name, path, sha) in skills {
            let remote = if path.is_empty() {
                Some(tree.sha.as_str())
            } else {
                tree.tree.iter().find(|e| e.kind == "tree" && e.path == path).map(|e| e.sha.as_str())
            };
            match remote {
                None => problems.push(format!("{name} is no longer in {repo}")),
                Some(r) if r == sha => {}
                Some(_) => match install_from(&repo, &tree, &path) {
                    Ok(_) => updated.push(name),
                    Err(e) => problems.push(e),
                },
            }
        }
    }
    if problems.is_empty() { Ok(updated) } else { Err(problems.join("\n")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_skill_files_without_git_or_marker() {
        let dir = std::env::temp_dir().join(format!("dove-local-skill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: x\ndescription: y\n---\n").unwrap();
        std::fs::write(dir.join("scripts/run.py"), "print(1)").unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref").unwrap();
        std::fs::write(dir.join(MARKER), "stamp").unwrap();
        let mut files = Vec::new();
        collect_files(&dir, &dir, &mut files).unwrap();
        files.sort();
        assert_eq!(files, vec![PathBuf::from("SKILL.md"), PathBuf::from("scripts").join("run.py")]);
        assert_eq!(Source::Local { path: "x".into() }.label(), "You");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dove-skills-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn validates_frontmatter() {
        let ok = "---\nname: my-skill\ndescription: Does a thing.\n---\n# Body\n";
        assert_eq!(validate("my-skill", ok).unwrap().description, "Does a thing.");
        let crlf = "\u{feff}---\r\nname: my-skill\r\ndescription: \"Quoted: yes\"\r\n---\r\n";
        assert_eq!(validate("my-skill", crlf).unwrap().description, "Quoted: yes");
        let folded = "---\nname: x1\ndescription: >\n  Line one\n  line two\nlicense: MIT\n---\n";
        assert_eq!(validate("x1", folded).unwrap().description, "Line one line two");
        let plain = "---\nname: x1\ndescription: starts here\n  and continues\n---\n";
        assert_eq!(validate("x1", plain).unwrap().description, "starts here and continues");

        assert!(validate("my-skill", "# no frontmatter").is_err());
        assert!(validate("other", ok).unwrap_err().contains("does not match"));
        assert_eq!(parse_meta(ok).unwrap().name, "my-skill");
        assert!(validate("My_Skill", "---\nname: My_Skill\ndescription: d\n---\n").is_err());
        assert!(validate("a", "---\nname: a\n---\n").unwrap_err().contains("description"));
        let long = format!("---\nname: a\ndescription: {}\n---\n", "x".repeat(1025));
        assert!(validate("a", &long).unwrap_err().contains("1024"));
        let name65 = "a".repeat(65);
        assert!(validate(&name65, &format!("---\nname: {name65}\ndescription: d\n---\n")).is_err());
    }

    #[test]
    fn parses_repositories() {
        assert_eq!(parse_repo("obra/superpowers").as_deref(), Some("obra/superpowers"));
        assert_eq!(parse_repo("https://github.com/obra/superpowers.git").as_deref(), Some("obra/superpowers"));
        assert_eq!(parse_repo("github.com/a/b/tree/main/skills/x").as_deref(), Some("a/b"));
        assert_eq!(parse_repo("git@github.com:a/b.git").as_deref(), Some("a/b"));
        assert_eq!(parse_repo("nope"), None);
        assert_eq!(parse_repo("a/b c"), None);
    }

    #[test]
    fn registry_round_trips_and_reconciles_bundled() {
        let root = scratch("registry");
        let mut reg = load_in(&root);
        assert!(office_skills::SKILLS.iter().all(|b| reg.get(b.name).is_some_and(|s| s.source == Source::Bundled)));
        reg.skills.push(Installed {
            name: "brainstorming".into(),
            description: "Ideas.".into(),
            source: Source::Git {
                repo: "obra/superpowers".into(),
                path: "skills/brainstorming".into(),
                sha: "abc".into(),
            },
            enabled: false,
        });
        reg.get_mut("pdf").unwrap().enabled = false;
        reg.repos.push("a/b".into());
        save_in(&root, &reg).unwrap();
        assert_eq!(load_in(&root), reg);
        // A bundled skill Dove no longer ships drops out.
        reg.skills.push(Installed {
            name: "gone".into(),
            description: String::new(),
            source: Source::Bundled,
            enabled: true,
        });
        save_in(&root, &reg).unwrap();
        assert!(load_in(&root).get("gone").is_none());
        let json = std::fs::read_to_string(root.join(REGISTRY)).unwrap();
        assert!(json.contains("\"kind\": \"git\""));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sync_protects_foreign_folders() {
        let base = scratch("sync");
        let root = base.join("skills");
        let harness = base.join("harness");
        for b in office_skills::SKILLS {
            assert_eq!(write_bundled(b, &root).unwrap(), Mirror::Written);
            assert_eq!(write_bundled(b, &root).unwrap(), Mirror::Current);
        }
        // The user's own docx skill in the harness folder.
        std::fs::create_dir_all(harness.join("docx")).unwrap();
        std::fs::write(harness.join("docx/SKILL.md"), "mine").unwrap();

        let mut reg = load_in(&root);
        for s in &mut reg.skills {
            s.enabled = true;
        }
        let problems = sync_in(&root, &reg, std::slice::from_ref(&harness));
        assert!(problems.iter().any(|p| p.contains("docx")), "{problems:?}");
        assert!(harness.join("pdf/SKILL.md").is_file());
        assert!(harness.join("pdf/scripts/pdf_tool.py").is_file());
        assert!(harness.join("pdf").join(MARKER).is_file());
        assert_eq!(std::fs::read_to_string(harness.join("docx/SKILL.md")).unwrap(), "mine");
        assert_eq!(mirror(&root.join("pdf"), &harness, "pdf").unwrap(), Mirror::Current);

        // Off: Dove's copy goes, the user's folder stays.
        for s in &mut reg.skills {
            s.enabled = false;
        }
        assert!(sync_in(&root, &reg, std::slice::from_ref(&harness)).is_empty());
        assert!(!harness.join("pdf").exists());
        assert_eq!(std::fs::read_to_string(harness.join("docx/SKILL.md")).unwrap(), "mine");

        // A changed canonical copy rewrites Dove's mirror.
        reg.get_mut("pdf").unwrap().enabled = true;
        sync_in(&root, &reg, std::slice::from_ref(&harness));
        std::fs::write(root.join("pdf/extra.md"), "new").unwrap();
        assert_eq!(mirror(&root.join("pdf"), &harness, "pdf").unwrap(), Mirror::Written);
        assert!(harness.join("pdf/extra.md").is_file());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn describes_catalog_and_skill() {
        let root = scratch("describe");
        for b in office_skills::SKILLS {
            write_bundled(b, &root).unwrap();
        }
        let mut reg = load_in(&root);
        for s in &mut reg.skills {
            s.enabled = s.name != "docx";
        }
        let catalog = describe_in(&root, &reg, None).unwrap();
        assert!(catalog.contains("pptx") && !catalog.contains("- docx"));
        let xlsx = describe_in(&root, &reg, Some("XLSX")).unwrap();
        assert!(xlsx.contains("name: xlsx") && xlsx.contains("uv run"));
        assert!(describe_in(&root, &reg, Some("docx")).is_err());
        assert!(describe_in(&root, &reg, Some("nope")).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Lists, installs, updates and removes a real skill from GitHub inside a
    /// scratch home. Network; run with `cargo test -p dove-harness live_github -- --ignored`.
    #[test]
    #[ignore]
    fn live_github() {
        let base = scratch("live");
        // SAFETY: this ignored test runs on its own; nothing else reads the
        // environment concurrently.
        unsafe {
            std::env::set_var("DOVE_HOME", base.join("dove"));
            std::env::set_var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, base.join("home"));
            std::env::remove_var("CLAUDE_CONFIG_DIR");
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        for c in CURATED {
            let l = listing(c.repo, true).unwrap_or_else(|e| panic!("{}: {e}", c.repo));
            assert!(!l.skills.is_empty(), "{} lists no skills", c.repo);
            assert!(l.skills.iter().all(|a| !c.skip.contains(&a.name.as_str())));
            eprintln!(
                "{}: {} skills, {} invalid",
                c.repo,
                l.skills.len(),
                l.skills.iter().filter(|a| a.problem.is_some()).count()
            );
        }
        assert!(cached_listing("obra/superpowers").is_some());
        let l = listing("obra/superpowers", false).unwrap();
        let pick = l.skills.iter().find(|a| a.name == "brainstorming").unwrap();
        let s = install("https://github.com/obra/superpowers", &pick.path).unwrap();
        let claude = base.join("home/.claude/skills/brainstorming");
        assert!(claude.join("SKILL.md").is_file() && claude.join(MARKER).is_file());
        assert!(base.join("home/.agents/skills/brainstorming/SKILL.md").is_file());
        assert!(matches!(s.source, Source::Git { .. }));
        assert_eq!(update_all().unwrap(), Vec::<String>::new());
        set_skill_enabled("brainstorming", false).unwrap();
        assert!(!claude.exists());
        remove("brainstorming").unwrap();
        assert!(!base.join("dove/skills/brainstorming").exists());
        assert!(load().get("brainstorming").is_none());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn finds_skill_folders_by_rule() {
        let entry = |path: &str, kind: &str| TreeEntry {
            path: path.into(),
            kind: kind.into(),
            sha: format!("sha:{path}"),
            size: Some(1),
        };
        let tree = Tree {
            sha: "root".into(),
            tree: vec![
                entry("skills", "tree"),
                entry("skills/a", "tree"),
                entry("skills/a/SKILL.md", "blob"),
                entry("skills/a/examples", "tree"),
                entry("skills/a/examples/SKILL.md", "blob"),
                entry("skills/docx", "tree"),
                entry("skills/docx/SKILL.md", "blob"),
                entry(".github/x", "tree"),
                entry(".github/x/SKILL.md", "blob"),
                entry("b", "tree"),
                entry("b/SKILL.md", "blob"),
            ],
        };
        let rule = Curated { skip: &["docx"], ..lib("o/r", "", Some("skills")) };
        let found = skill_folders("o/r", &tree, Some(&rule));
        assert_eq!(found, vec![("skills/a".to_string(), "sha:skills/a".to_string())]);
        let top = lib("o/r", "", Some(""));
        assert_eq!(skill_folders("o/r", &tree, Some(&top)).len(), 1);
        let any: Vec<String> = skill_folders("o/r", &tree, None).into_iter().map(|(p, _)| p).collect();
        assert_eq!(any, vec!["b", "skills/a", "skills/docx"]);

        let dotted = Tree {
            sha: "root".into(),
            tree: vec![
                entry("skills/.curated/x", "tree"),
                entry("skills/.curated/x/SKILL.md", "blob"),
                entry("plugins/p/skills/x", "tree"),
                entry("plugins/p/skills/x/SKILL.md", "blob"),
                entry("plugins/q/skills/x", "tree"),
                entry("plugins/q/skills/x/SKILL.md", "blob"),
            ],
        };
        let curated = lib("o/r", "", Some("skills/.curated"));
        assert_eq!(skill_folders("o/r", &dotted, Some(&curated)).len(), 1);
        // Copies of one skill in several plugins list once.
        assert_eq!(skill_folders("o/r", &dotted, Some(&lib("o/r", "", None))).len(), 1);
    }

    #[test]
    fn categorizes_skills() {
        assert_eq!(category(None, "pdf", "Create PDFs, merge and split"), "Documents");
        assert_eq!(category(None, "frontend-design", "Build distinctive interfaces"), "Design");
        assert_eq!(category(None, "systematic-debugging", "Use when a test fails"), "Engineering");
        assert_eq!(category(None, "ego-browser", "Operate websites"), "Web & browser");
        assert_eq!(category(Some("coreyhaines31/marketingskills"), "ab-testing", "Run tests"), "Marketing");
        assert_eq!(category(None, "zzz", "nothing known"), "Other");
        assert!(CATEGORIES.contains(&category(None, "meeting-insights-analyzer", "Analyze meeting transcripts")));
        for c in CURATED {
            assert!(c.category.is_none_or(|c| CATEGORIES.contains(&c)), "{}", c.repo);
        }
        for (cat, _) in CATEGORY_WORDS {
            assert!(CATEGORIES.contains(cat));
        }
    }
}
