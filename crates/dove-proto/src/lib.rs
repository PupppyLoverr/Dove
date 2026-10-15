//! Types every other Dove crate speaks.
//!
//! The engine normalizes each harness's native protocol (Claude stream-json,
//! Codex app-server, ACP, raw PTY) into [`AgentEvent`]s. The UI, the store,
//! the usage ledger and the CLI socket only ever see these types, so adding a
//! harness never touches them.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn parse(s: &str) -> Option<Self> {
                Uuid::parse_str(s.trim()).ok().map(Self)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_type!(ProjectId);
id_type!(TaskId);

/// Which CLI drives a session. The `id()` strings are persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessKind {
    ClaudeCode,
    Codex,
    Cursor,
    Devin,
    Grok,
    Gemini,
    OpenCode,
    Pi,
    Antigravity,
    Copilot,
    Amp,
    Droid,
    Goose,
    Qwen,
    Kilo,
    Cline,
    Hermes,
    /// Moonshot's Kimi Code CLI (`kimi acp`). Not Kilo Code.
    Kimi,
    /// Any other CLI run in a bare terminal.
    Shell,
}

impl HarnessKind {
    pub const ALL: &'static [HarnessKind] = &[
        Self::ClaudeCode,
        Self::Codex,
        Self::Cursor,
        Self::Devin,
        Self::Grok,
        Self::Gemini,
        Self::OpenCode,
        Self::Pi,
        Self::Antigravity,
        Self::Copilot,
        Self::Amp,
        Self::Droid,
        Self::Goose,
        Self::Qwen,
        Self::Kilo,
        Self::Cline,
        Self::Hermes,
        Self::Kimi,
        Self::Shell,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::Cursor => "Cursor",
            Self::Devin => "Devin",
            Self::Grok => "Grok Build",
            Self::Gemini => "Gemini CLI",
            Self::OpenCode => "OpenCode",
            Self::Pi => "Pi",
            Self::Antigravity => "Antigravity",
            Self::Copilot => "Copilot",
            Self::Amp => "Amp",
            Self::Droid => "Factory Droid",
            Self::Goose => "goose",
            Self::Qwen => "Qwen Code",
            Self::Kilo => "Kilo Code",
            Self::Cline => "Cline",
            Self::Hermes => "Hermes",
            Self::Kimi => "Kimi Code",
            Self::Shell => "Shell",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
            Self::Devin => "devin",
            Self::Grok => "grok",
            Self::Gemini => "gemini",
            Self::OpenCode => "open_code",
            Self::Pi => "pi",
            Self::Antigravity => "antigravity",
            Self::Copilot => "copilot",
            Self::Amp => "amp",
            Self::Droid => "droid",
            Self::Goose => "goose",
            Self::Qwen => "qwen",
            Self::Kilo => "kilo",
            Self::Cline => "cline",
            Self::Hermes => "hermes",
            Self::Kimi => "kimi",
            Self::Shell => "shell",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.id() == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Idle,
    /// Not started: a Kanban card with a prompt waiting to run.
    Draft,
    Running,
    NeedsInput,
    Done,
    Failed,
    Archived,
}

impl TaskStatus {
    pub fn id(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Draft => "draft",
            Self::Running => "running",
            Self::NeedsInput => "needs_input",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Archived => "archived",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "draft" => Self::Draft,
            "running" => Self::Running,
            "needs_input" => Self::NeedsInput,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "archived" => Self::Archived,
            _ => Self::Idle,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Ask before edits and commands.
    #[default]
    Default,
    AcceptEdits,
    Plan,
    /// No prompts. Only sensible inside an isolated worktree.
    Bypass,
}

/// How hard a model should think. Harnesses clamp this to the levels they
/// actually accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningLevel {
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
    Ultra,
    /// xhigh plus a harness-specific ultracode setting.
    Ultracode,
    /// Prompt-prefix driven (Claude).
    Ultrathink,
}

impl ReasoningLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Minimal => "Minimal",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::XHigh => "Extra high",
            Self::Max => "Max",
            Self::Ultra => "Ultra",
            Self::Ultracode => "Ultracode",
            Self::Ultrathink => "Ultrathink",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
            Self::Ultra => "ultra",
            Self::Ultracode => "ultracode",
            Self::Ultrathink => "ultrathink",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "minimal" | "none" => Self::Minimal,
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "xhigh" | "x-high" => Self::XHigh,
            "max" => Self::Max,
            "ultra" => Self::Ultra,
            "ultracode" => Self::Ultracode,
            "ultrathink" => Self::Ultrathink,
            _ => return None,
        })
    }
}

/// One choice inside a model trait (context window, speed, fast mode).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelOptionChoice {
    pub id: String,
    pub label: String,
}

/// A per-model setting the picker can change: context window, speed, and so on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub choices: Vec<ModelOptionChoice>,
    pub default_choice: String,
}

/// A model a harness can run, with the effort ladder and traits it offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub reasoning_levels: Vec<ReasoningLevel>,
    pub options: Vec<ModelOption>,
}

impl Model {
    pub fn option(&self, id: &str) -> Option<&ModelOption> {
        self.options.iter().find(|o| o.id == id)
    }
}

/// Token counts as reported by the harness, never estimated unless `estimated`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Cost as reported by the harness itself, when it reports one.
    pub cost_usd: Option<f64>,
    pub estimated: bool,
}

impl Usage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, o: Self) {
        self.input_tokens += o.input_tokens;
        self.output_tokens += o.output_tokens;
        self.cache_read_tokens += o.cache_read_tokens;
        self.cache_write_tokens += o.cache_write_tokens;
        self.cost_usd = match (self.cost_usd, o.cost_usd) {
            (Some(a), Some(b)) => Some(a + b),
            (a, b) => a.or(b),
        };
        self.estimated |= o.estimated;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionRequest {
    /// Harness-native request id, echoed back in the reply.
    pub request_id: String,
    pub tool: String,
    pub summary: Arc<str>,
    pub input: serde_json::Value,
    /// Set when the agent is asking the user something (AskUserQuestion,
    /// requestInput) rather than asking to run a tool.
    #[serde(default)]
    pub questions: Vec<Question>,
}

impl PermissionRequest {
    pub fn is_question(&self) -> bool {
        !self.questions.is_empty()
    }
}

/// One question the agent asked, with its choices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<String>,
    pub multi_select: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    AllowOnce,
    AllowAlways,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Execute,
    Search,
    Fetch,
    Think,
    Agent,
    Other,
}

impl ToolKind {
    /// Best-effort mapping from a harness tool name.
    pub fn classify(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "read" | "view" | "read_file" | "notebookread" => Self::Read,
            "edit" | "write" | "multiedit" | "notebookedit" | "apply_patch" | "write_file" | "str_replace"
            | "filechange" | "file_change" => Self::Edit,
            "bash" | "shell" | "exec" | "exec_command" | "local_shell" | "run_terminal_cmd" | "commandexecution"
            | "command_execution" | "execute" => Self::Execute,
            "grep" | "glob" | "search" | "ls" | "find" | "list_dir" => Self::Search,
            "webfetch" | "websearch" | "fetch" | "web_search" | "web_fetch" | "browser" | "browser_navigate"
            | "browser_click" | "browser_type" | "browser_press" | "browser_scroll" | "open_browser" | "navigate" => {
                Self::Fetch
            }
            "task" | "agent" => Self::Agent,
            "think" => Self::Think,
            _ => Self::Other,
        }
    }
}

/// One normalized thing that happened in a session.
///
/// Text is `Arc<str>` so fan-out to UI, store and ledger never copies bodies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    SessionStarted {
        native_session_id: Option<String>,
        model: Option<String>,
    },
    /// The user's own turn, recorded so a transcript replays in full.
    UserMessage {
        text: Arc<str>,
    },
    /// Streaming assistant text. Consecutive deltas append to one block.
    TextDelta {
        text: Arc<str>,
    },
    ThinkingDelta {
        text: Arc<str>,
    },
    /// Closes the current streamed text block.
    BlockEnd,
    ToolCall {
        call_id: String,
        name: String,
        kind: ToolKind,
        title: Arc<str>,
        input: serde_json::Value,
    },
    /// A better title for a call already shown (ACP sends the real one later).
    ToolTitle {
        call_id: String,
        title: Arc<str>,
    },
    ToolResult {
        call_id: String,
        output: Arc<str>,
        is_error: bool,
    },
    /// A file change a tool made (old text `None` = new file).
    ToolDiff {
        call_id: String,
        path: String,
        old_text: Option<Arc<str>>,
        new_text: Arc<str>,
    },
    PermissionRequested(PermissionRequest),
    PermissionResolved {
        request_id: String,
        decision: PermissionDecision,
    },
    Plan {
        text: Arc<str>,
    },
    /// The agent's current checklist, replacing any earlier one.
    Todos {
        items: Vec<TodoItem>,
    },
    /// Status line the harness reports while working ("Writing main.rs").
    Status {
        text: Arc<str>,
    },
    Notice {
        text: Arc<str>,
    },
    Error {
        message: Arc<str>,
    },
    /// How full the model's context window is (latest call, not cumulative).
    ContextUsage {
        tokens: Option<u64>,
        window: Option<u64>,
    },
    /// The account's plan usage windows (5-hour, weekly, ...).
    RateLimits {
        windows: Vec<RateWindow>,
    },
    /// The agent's slash commands, for the composer's `/` menu.
    Commands {
        commands: Vec<SlashCommand>,
    },
    /// The current turn ended; the session can take more input.
    TurnEnded {
        usage: Option<Usage>,
        is_error: bool,
    },
    /// The process exited.
    Exited {
        code: Option<i32>,
    },
    /// Something that happened inside a subagent's own transcript, keyed by
    /// the parent's spawning tool call. The parent chat keeps only the
    /// spawn's tool row; the subagent view replays these as its own chat.
    Subagent {
        parent: String,
        event: Box<AgentEvent>,
    },
}

/// One plan usage window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateWindow {
    /// `five_hour`, `seven_day`, `primary`, `secondary`, ...
    pub kind: String,
    /// 0-100.
    pub used_percent: Option<f64>,
    /// Unix seconds.
    pub resets_at: Option<i64>,
    pub window_minutes: Option<u64>,
    pub blocked: bool,
}

impl RateWindow {
    pub fn label(&self) -> String {
        match (self.kind.as_str(), self.window_minutes) {
            ("five_hour", _) | (_, Some(300)) => "5-hour limit".into(),
            ("seven_day_opus", _) => "Weekly (Opus)".into(),
            ("seven_day_sonnet", _) => "Weekly (Sonnet)".into(),
            (k, _) if k.starts_with("seven_day") => "Weekly limit".into(),
            (_, Some(10080)) => "Weekly limit".into(),
            (_, Some(m)) if m % 1440 == 0 => format!("{}-day limit", m / 1440),
            (_, Some(m)) if m % 60 == 0 => format!("{}-hour limit", m / 60),
            ("primary", _) => "Primary limit".into(),
            ("secondary", _) => "Secondary limit".into(),
            (k, _) => k.replace('_', " "),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

impl TodoStatus {
    /// Accepts the spellings Claude (`in_progress`), Codex (`inProgress`) and
    /// ACP use.
    pub fn parse(s: &str) -> Self {
        match s {
            "completed" | "done" => Self::Completed,
            "in_progress" | "inProgress" | "in-progress" | "active" => Self::InProgress,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub text: String,
    pub status: TodoStatus,
}

/// Input from the user to a running session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserTurn {
    pub text: String,
}

/// What a harness supports, so the UI can hide controls that would do nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Capabilities {
    pub streaming: bool,
    pub permissions: bool,
    pub resume: bool,
    pub interrupt: bool,
    pub usage: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_ids_round_trip() {
        for k in HarnessKind::ALL {
            assert_eq!(HarnessKind::from_id(k.id()), Some(*k));
        }
    }

    #[test]
    fn usage_adds() {
        let mut a = Usage { input_tokens: 1, cost_usd: Some(0.5), ..Default::default() };
        a += Usage { output_tokens: 2, cost_usd: Some(0.25), ..Default::default() };
        assert_eq!(a.total_tokens(), 3);
        assert_eq!(a.cost_usd, Some(0.75));
    }

    #[test]
    fn event_serializes_tagged() {
        let e = AgentEvent::TextDelta { text: "hi".into() };
        assert_eq!(serde_json::to_string(&e).unwrap(), r#"{"type":"text_delta","text":"hi"}"#);
    }
}
