//! Drives coding-agent CLIs and turns their output into [`AgentEvent`]s.
//!
//! Every adapter has the same shape: [`spawn`] starts the child on the tokio
//! runtime and returns a [`SessionHandle`]. Events come out of a `flume`
//! channel, which any executor can await, so GPUI's foreground executor reads
//! them without touching tokio. Commands go in through a plain sender.

pub mod acp;
mod agy;
pub mod catalog;
pub mod detect;
mod external;
pub mod host;
pub use external::defers_prompt;
pub mod models;
pub mod office_skills;
pub mod process;
mod rpc;
pub mod skills;
pub mod teach;
pub mod title;
pub mod tui;
pub mod walkthrough;

use std::path::PathBuf;

use anyhow::{Result, bail};
use dove_proto::{AgentEvent, Capabilities, HarnessKind, PermissionDecision, PermissionMode, ReasoningLevel};

/// Everything needed to start (or resume) one agent session.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub kind: HarnessKind,
    /// Resolved binary. Use [`detect::find`] to fill it.
    pub program: PathBuf,
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub reasoning: Option<ReasoningLevel>,
    /// Selected model traits, as `(option id, choice id)`.
    pub model_options: Vec<(String, String)>,
    pub permission_mode: PermissionMode,
    /// Harness-native session id to resume.
    pub resume: Option<String>,
    /// The first user turn, sent as soon as the session is up.
    pub prompt: Option<String>,
    /// With `resume`: branch a new session from it instead of continuing it.
    pub fork: bool,
    /// Dove's key for this session, handed to its tool server as
    /// `DOVE_SESSION`: how a tool call says which thread it comes from (many
    /// threads can share one folder). Empty: no key.
    pub session: String,
    /// Where the agent runs (this PC, WSL, SSH, a Cua Space).
    pub host: host::Host,
}

/// A fresh session key for [`SessionSpec::session`].
pub fn new_session_key() -> String {
    uuid::Uuid::now_v7().simple().to_string()
}

/// An MCP server Dove offers every agent it starts (its in-app browser).
#[derive(Debug, Clone)]
pub struct ToolServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

static TOOL_SERVER: std::sync::RwLock<Option<ToolServer>> = std::sync::RwLock::new(None);

/// Offer (or stop offering) `server` to sessions started from now on.
pub fn set_tool_server(server: Option<ToolServer>) {
    if let Ok(mut s) = TOOL_SERVER.write() {
        *s = server;
    }
}

/// The tool server for a session in `cwd` with key `session` (together they
/// tell Dove which thread a call comes from).
pub fn tool_server(cwd: &std::path::Path, session: &str) -> Option<ToolServer> {
    let mut server = TOOL_SERVER.read().ok()?.clone()?;
    server.env.push(("DOVE_CWD".into(), cwd.display().to_string()));
    if !session.is_empty() {
        server.env.push(("DOVE_SESSION".into(), session.to_string()));
    }
    Some(server)
}

static ALLOW_EGO_BROWSER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Let sessions started from now on run ego lite's `ego-browser` command
/// without a prompt (Settings → Plugins → ego lite). Each harness
/// applies it through its own allow rules, which parse the shell command
/// themselves; Dove never matches shell text. Harnesses with no safe
/// launch-time rule keep asking.
pub fn set_allow_ego_browser(on: bool) {
    ALLOW_EGO_BROWSER.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn allow_ego_browser() -> bool {
    ALLOW_EGO_BROWSER.load(std::sync::atomic::Ordering::Relaxed)
}

/// Claude Code's allow rule for `ego-browser`. Claude splits compound
/// commands itself, so `ego-browser x; rm -rf ~` still asks.
pub(crate) const CLAUDE_EGO_RULE: &str = "Bash(ego-browser:*)";

/// Whether a tool name is one of Dove's own browser tools, however the
/// harness spells MCP names (`mcp__dove__browser_click`, `dove/browser_click`,
/// `dove.browser_click`). These drive Dove's sandboxed in-app browser and
/// never need the user's approval.
pub fn is_dove_tool(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    let n = n.trim_start_matches("approve").trim();
    ["mcp__dove__", "dove/", "dove.", "dove__", "dove:"].iter().any(|p| {
        n.starts_with(p)
            && (n[p.len()..].starts_with("browser_")
                || matches!(&n[p.len()..], "show_ui" | "show_canvas" | "dove_skills")
                || is_memory_read(&n[p.len()..])
                || is_team_read(&n[p.len()..]))
    })
}

/// Dove's team tools that only look: who is on the team, what threads say,
/// how delegated work stands. Starting, messaging and stopping work still
/// goes through the harness's own approvals.
fn is_team_read(tool: &str) -> bool {
    matches!(tool, "team_capabilities" | "thread_list" | "thread_read" | "thread_wait" | "task_status")
}

/// Supermemory's read-only tools, proxied through Dove's server: recalling
/// memories never needs a prompt (saving and forgetting still ask).
fn is_memory_read(tool: &str) -> bool {
    tool.strip_prefix("supermemory__").is_some_and(|t| {
        matches!(
            t,
            "search_memory"
                | "get_profile"
                | "list_memories"
                | "list_documents"
                | "get_document"
                | "list_spaces"
                | "listspaces"
                | "who_am_i"
                | "whoami"
        )
    })
}

/// ACP `mcpServers` for a session in `cwd` with key `session`.
pub fn acp_mcp_servers(cwd: &std::path::Path, session: &str) -> serde_json::Value {
    match tool_server(cwd, session) {
        Some(s) => serde_json::json!([{
            "name": s.name,
            "command": s.command,
            "args": s.args,
            "env": s.env.iter().map(|(k, v)| serde_json::json!({"name": k, "value": v})).collect::<Vec<_>>(),
        }]),
        None => serde_json::json!([]),
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    Send(String),
    Interrupt,
    Permission {
        request_id: String,
        decision: PermissionDecision,
    },
    /// Answers to a question request: per question, the chosen labels (or
    /// free text).
    Answer {
        request_id: String,
        answers: Vec<Vec<String>>,
    },
    Kill,
}

pub struct SessionHandle {
    pub events: flume::Receiver<AgentEvent>,
    commands: flume::Sender<Command>,
    pub capabilities: Capabilities,
}

impl SessionHandle {
    pub fn send(&self, cmd: Command) -> bool {
        self.commands.send(cmd).is_ok()
    }

    pub fn is_alive(&self) -> bool {
        !self.commands.is_disconnected()
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Kill);
    }
}

/// Whether Dove can drive `kind` with structured events.
pub fn is_supported(kind: HarnessKind) -> bool {
    external::drives(kind) || kind == HarnessKind::Amp || acp::launch_args(kind).is_some()
}

/// Start a session on `rt`. Returns immediately; failures to launch arrive as
/// an [`AgentEvent::Error`] followed by [`AgentEvent::Exited`].
pub fn spawn(rt: &tokio::runtime::Handle, mut spec: SessionSpec) -> Result<SessionHandle> {
    skills::ensure_installed();
    // A remote host runs through a launcher named like the agent, so every
    // driver below starts it exactly as it would the local binary.
    if !spec.host.is_local() {
        spec.program = host::launcher(&spec.host, &spec.program)?;
    }
    let (ev_tx, ev_rx) = flume::unbounded();
    let (cmd_tx, cmd_rx) = flume::unbounded();
    // Shared agents (Claude, Codex, Cursor, OpenCode, Pi, Devin, Grok,
    // Hermes, Antigravity) run on Zeron's drivers. The rest stay on Dove's.
    let capabilities = if external::drives(spec.kind) {
        rt.spawn(external::run(spec, ev_tx, cmd_rx));
        external::CAPABILITIES
    } else {
        match spec.kind {
            other if other == HarnessKind::Amp || acp::launch_args(other).is_some() => {
                rt.spawn(acp::run(spec, ev_tx, cmd_rx));
                acp::CAPABILITIES
            }
            other => bail!("{} is not supported yet", other.label()),
        }
    };
    Ok(SessionHandle { events: ev_rx, commands: cmd_tx, capabilities })
}

#[cfg(test)]
mod tool_server_tests {
    use super::*;

    #[test]
    fn recognizes_dove_tools() {
        assert!(is_dove_tool("mcp__dove__browser_click"));
        assert!(is_dove_tool("Approve dove/browser_navigate"));
        assert!(is_dove_tool("dove.browser_type"));
        assert!(is_dove_tool("mcp__dove__show_ui"));
        assert!(is_dove_tool("mcp__dove__dove_skills"));
        assert!(!is_dove_tool("mcp__other__browser_click"));
        assert!(!is_dove_tool("mcp__dove__something_else"));
        assert!(!is_dove_tool("Bash"));
        assert!(is_dove_tool("mcp__dove__supermemory__search_memory"));
        assert!(!is_dove_tool("mcp__dove__supermemory__add_memory"));
        assert!(!is_dove_tool("mcp__dove__linear__list_issues"));
    }

    #[test]
    fn tool_server_carries_cwd_and_session() {
        set_tool_server(Some(ToolServer { name: "dove".into(), command: "dove".into(), args: vec![], env: vec![] }));
        let s = tool_server(std::path::Path::new("/tmp/x"), "abc").unwrap();
        assert!(s.env.iter().any(|(k, _)| k == "DOVE_CWD"));
        assert!(s.env.iter().any(|(k, v)| k == "DOVE_SESSION" && v == "abc"));
        let s = tool_server(std::path::Path::new("/tmp/x"), "").unwrap();
        assert!(!s.env.iter().any(|(k, _)| k == "DOVE_SESSION"), "no key, no variable");
        set_tool_server(None);
    }

    #[test]
    fn read_only_team_tools_need_no_approval() {
        assert!(is_dove_tool("mcp__dove__team_capabilities"));
        assert!(is_dove_tool("dove/thread_read"));
        assert!(is_dove_tool("mcp__dove__task_status"));
        assert!(!is_dove_tool("mcp__dove__delegate_task"));
        assert!(!is_dove_tool("mcp__dove__team_message"));
        assert!(!is_dove_tool("mcp__dove__thread_send"));
        assert!(!is_dove_tool("mcp__dove__task_cancel"));
    }
}
