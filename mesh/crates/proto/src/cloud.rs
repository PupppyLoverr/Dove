//! Cloud / GitHub agent mode — wire types shared by engine, UI, and RPC.
//!
//! Cosmos does not host the VM. A [`CloudSeatBinding`] attaches a chat to a
//! user-supplied computer (a Box token, an SSH alias to a Docker host, or a
//! GitHub repo workspace); the desktop drives it over HTTP / SSH / the GitHub
//! API. Keys live in the engine's `cloud-keys.json` — never in the workspace
//! doc and never in `ui-settings.json`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The computer backend a cloud seat binds to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloudSeatKind {
    /// Railway's free trial VM (`ssh railway.new`): identified by the user's
    /// SSH key — never a shared Cosmos account. Sixty-minute build window,
    /// then a 24h claim grace; the seat carries `expires_at` + `claim_url`.
    Railway,
    /// A Boat (boat.dev) sandbox: the user's `boat_…` API token drives the
    /// hosted box's lifecycle (create/stop/resume/destroy) and exec
    /// (`/commands`), files (`/files`), prompts (`/prompt`), and desktop
    /// preview (`/desktop`) over the provider's HTTP API.
    Box,
    /// BYO VPS: an `~/.ssh/config` alias on this desktop to an x86_64 Linux
    /// host running Docker; workspaces are `cosmos-cloud-<workspace>`
    /// containers.
    Ssh,
    /// A GitHub repository + PAT: file/PR data rides the REST API; agent
    /// execution rides `workflow_dispatch` on the repo's agent workflow.
    GitHub,
}

impl CloudSeatKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Railway => "Railway trial",
            Self::Box => "Boat",
            Self::Ssh => "VPS (SSH)",
            Self::GitHub => "GitHub workspace",
        }
    }

    /// Short seat-chip prefix: `Cloud · Railway` / `Cloud · Boat` / `VPS` /
    /// `GitHub agent`.
    pub fn chip(self) -> &'static str {
        match self {
            Self::Railway => "Cloud · Railway",
            Self::Box => "Cloud · Boat",
            Self::Ssh => "Cloud · VPS",
            Self::GitHub => "GitHub agent",
        }
    }
}

/// The seat a chat runs on. Stored on the synced `Chat` row so every device
/// renders the same binding; carries no secrets (tokens stay in the engine's
/// keys file, keyed by `seat_id`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudSeatBinding {
    /// `SeatConfig::id` in the binding engine's `cloud.json`.
    pub seat_id: String,
    pub kind: CloudSeatKind,
    /// Display name (defaults to the seat's label).
    pub label: String,
    /// `owner/name` for GitHub seats; also stamped for SSH seats bound to a
    /// cloned repo so the Pull Requests pane knows what to query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Working directory on the seat (container path / clone root). None =
    /// the seat's configured default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    /// Device running the headless Cosmos runner for this seat. None keeps
    /// the legacy laptop-driven seat behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_device_id: Option<String>,
    /// The repo branch the chat's checkout starts on (the composer's branch
    /// pick). None = the repo's default branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Model label for the seat chip, e.g. the BYOK provider's default model
    /// or the seat CLI's own login ("seat CLI login").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_label: Option<String>,
    /// Railway trial deadline — the header chip counts down to it; every
    /// device sees the same bound hour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

/// The right-rail pane kinds for a cloud workspace session. One pane is
/// visible at a time; the shell persists the last pick per chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloudPane {
    Computer,
    /// An emulator endpoint on the seat (Android on Linux seats). Disabled
    /// unless the seat advertises one — never faked.
    Simulator,
    Files,
    Changes,
    Shell,
    Progress,
    Context,
    Tasks,
    PullRequests,
}

impl CloudPane {
    pub const ALL: [CloudPane; 9] = [
        Self::Computer,
        Self::Simulator,
        Self::Files,
        Self::Changes,
        Self::Shell,
        Self::Progress,
        Self::Context,
        Self::Tasks,
        Self::PullRequests,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Computer => "Computer",
            Self::Simulator => "iOS Simulator",
            Self::Files => "Files",
            Self::Changes => "Changes",
            Self::Shell => "Shell",
            Self::Progress => "Progress",
            Self::Context => "Context",
            Self::Tasks => "Tasks",
            Self::PullRequests => "Pull requests",
        }
    }
}

/// BYOK model providers for cloud sessions — the same families the `zdev`
/// CLI drives (`Cli/zdev/src/providers.ts`): OpenAI chat completions /
/// Responses, ChatGPT subscription, Anthropic Messages, plus the
/// OpenAI-compatible vendors and Devin's cloud models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ByokProvider {
    Anthropic,
    OpenAi,
    Xai,
    OpenRouter,
    DeepSeek,
    Xiaomi,
    /// ChatGPT subscription (OAuth access token; Responses on chatgpt.com).
    ChatgptSub,
    /// Devin subscription — Devin's cloud model catalog, not a harness key.
    DevinSub,
    /// Any OpenAI-compatible endpoint (the key + a base URL the user types).
    Custom,
}

impl ByokProvider {
    pub const ALL: [ByokProvider; 9] = [
        Self::Anthropic,
        Self::OpenAi,
        Self::Xai,
        Self::OpenRouter,
        Self::DeepSeek,
        Self::Xiaomi,
        Self::ChatgptSub,
        Self::DevinSub,
        Self::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic",
            Self::OpenAi => "OpenAI",
            Self::Xai => "xAI / Grok",
            Self::OpenRouter => "OpenRouter",
            Self::DeepSeek => "DeepSeek",
            Self::Xiaomi => "Xiaomi MiMo",
            Self::ChatgptSub => "ChatGPT subscription",
            Self::DevinSub => "Devin subscription",
            Self::Custom => "Custom base URL",
        }
    }

    /// Stable string id used as the key in `cloud-keys.json`.
    pub fn id(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::Xai => "xai",
            Self::OpenRouter => "openrouter",
            Self::DeepSeek => "deepseek",
            Self::Xiaomi => "xiaomi",
            Self::ChatgptSub => "chatgpt-sub",
            Self::DevinSub => "devin-sub",
            Self::Custom => "custom",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    /// Default API base when the row leaves Base URL empty.
    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Anthropic => "https://api.anthropic.com",
            Self::OpenAi => "https://api.openai.com/v1",
            Self::Xai => "https://api.x.ai/v1",
            Self::OpenRouter => "https://openrouter.ai/api/v1",
            Self::DeepSeek => "https://api.deepseek.com/v1",
            Self::Xiaomi => "https://api.xiaomimimo.com/v1",
            Self::ChatgptSub => "https://chatgpt.com/backend-api",
            Self::DevinSub => "https://api.devin.ai",
            Self::Custom => "",
        }
    }

    /// Subscription rows take an access token rather than a raw API key.
    pub fn is_subscription(self) -> bool {
        matches!(self, Self::ChatgptSub | Self::DevinSub)
    }
}

/// Where a run stands for Kanban. Derived from session status for local
/// chats and from the cloud run ledger for seat runs — never faked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloudRunStatus {
    Queued,
    Working,
    WaitingForUser,
    /// A tool call on the seat is parked on user approval.
    NeedsApproval,
    /// User-stopped (or paused) — resumable.
    Stopped,
    Done,
    Error,
}

impl CloudRunStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Working => "Working",
            Self::WaitingForUser => "Waiting for user",
            Self::NeedsApproval => "Needs approval",
            Self::Stopped => "Stopped",
            Self::Done => "Done",
            Self::Error => "Error",
        }
    }

    /// Kanban column order.
    pub const COLUMNS: [CloudRunStatus; 7] = [
        Self::Queued,
        Self::Working,
        Self::WaitingForUser,
        Self::NeedsApproval,
        Self::Stopped,
        Self::Done,
        Self::Error,
    ];
}

/// One row of the cloud run ledger — a run dispatched to a seat. The ledger
/// is engine-local (`cloud-runs.json`), survives restarts, and backs the
/// Kanban cards, the bot bar, and the Progress pane for seats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRun {
    pub id: String,
    pub chat_id: String,
    pub seat_id: String,
    /// The seat's harness command line (for audit/display).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub status: CloudRunStatus,
    /// Remote log path the run writes on the seat (detached runs tail it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    /// Remote marker pid/kill hint (e.g. the docker exec label).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_ref: Option<String>,
    /// Bytes of the remote log already mirrored into the transcript.
    #[serde(default)]
    pub mirrored_bytes: u64,
    /// Boat runs: the last `/events` cursor mirrored, so a reopened app
    /// resumes the stream where it left off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_cursor: Option<String>,
    /// The transcript entry this run streams into — a reattach reopens it
    /// instead of starting a second message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Seat working directory this run executed in (the chat's workspace).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<crate::HarnessId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The harness-native conversation this run continued or started
    /// (`claude --session-id`), so the next turn resumes it instead of
    /// starting cold. Stamped only once a turn completes with output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_session: Option<String>,
    /// First line of the prompt that started the run — the Kanban card's
    /// subtitle before a title is generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

/// An automation: a prompt on a schedule against a workspace or seat.
/// Persisted next to the seat config (`cloud.json`) — "persistence next to
/// existing settings". Fired by the engine's scheduler; a fire on an
/// unreachable seat marks `last_status: Missed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudAutomation {
    pub id: String,
    pub name: String,
    pub prompt: String,
    /// `hourly` | `daily` | `weekly` | a 5-field cron expression.
    pub schedule: String,
    /// Cloud seat id; `None` = run on this computer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seat_id: Option<String>,
    /// Local workspace (space id) the run lands in for local automations —
    /// and the project context for seat runs when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
    /// Local harness for runs on this computer (`None` = the app default).
    /// Seat runs use the seat's own harness command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<crate::HarnessId>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<DateTime<Utc>>,
    /// `ok` | `missed` | `error:<message>` — for Kanban surfacing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
    /// Chat the last fire produced (click-through from the list).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_chat_id: Option<String>,
}

fn default_enabled() -> bool {
    true
}

/// A Kanban task saved as a draft: its chat exists (so the card opens a
/// real session), the prompt waits here until the user starts it — "Start"
/// or a drag into In progress queues the run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDraft {
    pub chat_id: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<crate::HarnessId>,
    /// Where the run executes (seat workspace or local project path).
    pub cwd: String,
    pub created_at: DateTime<Utc>,
}

/// A workspace snapshot pulled from a seat down to this computer — how a
/// cloud workspace's progress survives the seat (a Railway trial dying, a
/// Boat box destroyed). Restored onto whichever seat next runs the chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBackup {
    pub chat_id: String,
    pub seat_id: String,
    /// Seat path the snapshot was taken from.
    pub workspace: String,
    /// Local `.tar.gz` path under the engine data dir.
    pub path: String,
    pub bytes: u64,
    pub taken_at: DateTime<Utc>,
    /// Seat the snapshot was last restored onto (restores are once per seat).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored_to: Option<String>,
}

/// Pointer to a workspace snapshot held in one device's profile store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudBackupPointer {
    pub seat_id: String,
    pub device_id: String,
    /// Relative to the device's profile store root.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    pub created_at: DateTime<Utc>,
}

/// Seat health as the last probe reported it — drives the bot bar's dot and
/// the settings "Health" block.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatProbe {
    pub seat_id: String,
    pub online: bool,
    /// Human detail line: docker server version, GitHub repo name, error
    /// text — verbatim, never secret-bearing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Running `cosmos-cloud-*` containers (SSH seats) or the repo's default
    /// branch (GitHub seats) — small inventory for the status line.
    #[serde(default)]
    pub containers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_digest: Option<String>,
    /// Whether the seat can drive a desktop (a CUA endpoint is configured) —
    /// the "Take control" gate.
    #[serde(default)]
    pub controllable: bool,
    /// Whether the seat can start a harness that survives this app closing
    /// (detached `docker exec`/`nohup` or `workflow_dispatch`). False → the
    /// honest "runs only while the desktop app is open" banner.
    #[serde(default)]
    pub keeps_running: bool,
    /// The seat is alive but stopped — Boat's snapshot-paused ("Sleeping")
    /// state: billing off, resumable. Renders as the amber dot.
    #[serde(default)]
    pub sleeping: bool,
    pub probed_at: DateTime<Utc>,
}

/// What one cloud seat can do — the honest capability surface the UI gates
/// every pane and button on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeatCapabilities {
    /// Remote command execution (ssh exec / docker exec / gh api).
    #[serde(default)]
    pub exec: bool,
    /// Interactive remote shell (ssh alias → PTY via local terminal).
    #[serde(default)]
    pub shell: bool,
    /// Files + Changes via exec or GitHub contents/compare APIs.
    #[serde(default)]
    pub files: bool,
    #[serde(default)]
    pub git: bool,
    /// Pull requests pane (a repo is bound).
    #[serde(default)]
    pub pull_requests: bool,
    /// A CUA remote endpoint is stamped — the Computer pane can drive.
    #[serde(default)]
    pub computer: bool,
    /// An emulator endpoint on the seat.
    #[serde(default)]
    pub simulator: bool,
    /// A harness run can outlive this app (detached exec / workflow).
    #[serde(default)]
    pub keeps_running: bool,
}

#[cfg(test)]
mod tests {
    use super::{CloudRun, CloudRunStatus};

    #[test]
    fn cloud_run_defaults_harness_and_model_for_older_rows() {
        let run: CloudRun = serde_json::from_value(serde_json::json!({
            "id": "run-1",
            "chatId": "chat-1",
            "seatId": "seat-1",
            "status": "working",
            "startedAt": "2025-01-01T00:00:00Z",
            "updatedAt": "2025-01-01T00:00:00Z"
        }))
        .unwrap();

        assert!(run.harness.is_none());
        assert!(run.model.is_none());
        assert_eq!(run.status, CloudRunStatus::Working);
    }
}
