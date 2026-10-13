//! Drives coding-agent CLIs and turns their output into [`AgentEvent`]s.
//!
//! Every adapter has the same shape: [`spawn`] starts the child on the tokio
//! runtime and returns a [`SessionHandle`]. Events come out of a `flume`
//! channel, which any executor can await, so GPUI's foreground executor reads
//! them without touching tokio. Commands go in through a plain sender.

pub mod acp;
pub mod catalog;
pub mod detect;
mod external;
pub mod models;
pub mod process;
mod rpc;
pub mod tui;

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

/// The tool server for a session in `cwd` (which tells Dove which thread a
/// call comes from).
pub fn tool_server(cwd: &std::path::Path) -> Option<ToolServer> {
    let mut server = TOOL_SERVER.read().ok()?.clone()?;
    server.env.push(("DOVE_CWD".into(), cwd.display().to_string()));
    Some(server)
}

/// Whether a tool name is one of Dove's own browser tools, however the
/// harness spells MCP names (`mcp__dove__browser_click`, `dove/browser_click`,
/// `dove.browser_click`). These drive Dove's sandboxed in-app browser and
/// never need the user's approval.
pub fn is_dove_tool(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    let n = n.trim_start_matches("approve").trim();
    ["mcp__dove__", "dove/", "dove.", "dove__", "dove:"]
        .iter()
        .any(|p| n.starts_with(p) && (n[p.len()..].starts_with("browser_") || &n[p.len()..] == "show_ui"))
}

/// ACP `mcpServers` for a session in `cwd`.
pub fn acp_mcp_servers(cwd: &std::path::Path) -> serde_json::Value {
    match tool_server(cwd) {
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
pub fn spawn(rt: &tokio::runtime::Handle, spec: SessionSpec) -> Result<SessionHandle> {
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
        assert!(!is_dove_tool("mcp__other__browser_click"));
        assert!(!is_dove_tool("mcp__dove__something_else"));
        assert!(!is_dove_tool("Bash"));
    }

    #[test]
    fn tool_server_carries_cwd() {
        set_tool_server(Some(ToolServer { name: "dove".into(), command: "dove".into(), args: vec![], env: vec![] }));
        let s = tool_server(std::path::Path::new("/tmp/x")).unwrap();
        assert!(s.env.iter().any(|(k, _)| k == "DOVE_CWD"));
        set_tool_server(None);
    }
}
