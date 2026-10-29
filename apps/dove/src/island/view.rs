//! What the engine and the notch share beyond the snapshot (docs/notch-v3-plan.md):
//!
//! - **Views** ([`NotchView`]): a small typed vocabulary of blocks an agent
//!   (K22 above all) composes with the `notch_view` tool, and the notch draws
//!   natively. The built-in lenses (status, needs you, today, one agent) are
//!   views too, made in the notch from the snapshot. Actions are references to
//!   checked commands, never code, and only ever run from a click.
//! - **Plans** ([`PendingPlan`]): an agent's plan waiting on the user's
//!   approval, parsed into steps the notch lets you edit, reorder and remove.
//! - **Calls** ([`CallInfo`]): a Voice Mode call's state as the notch shows it.

use std::sync::{Mutex, OnceLock};

use dove_core::attention::TaskState;
use dove_core::proto::{HarnessKind, PermissionRequest, TaskId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

/// Where a view came from.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewSource {
    /// Composed by an agent (`notch_view`).
    #[default]
    Agent,
    /// A built-in lens, by id (`status`, `needs`, `today`, `agent`).
    Lens(String),
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct NotchView {
    /// Stable per view: a newer view with the same id replaces it (that's
    /// how a view streams in, skeleton first).
    #[serde(default)]
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub source: ViewSource,
    /// The agent that composed it.
    #[serde(default)]
    pub task: Option<TaskId>,
    #[serde(default)]
    pub kind: Option<HarnessKind>,
    /// Still being composed (the notch shows a shimmer under it).
    #[serde(default)]
    pub pending: bool,
}

/// Colour by meaning only. Reads leniently: anything else is neutral.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    #[default]
    Neutral,
    Good,
    Warn,
    Bad,
    Info,
}

impl<'de> Deserialize<'de> for Tone {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = Option::<String>::deserialize(d)?.unwrap_or_default();
        Ok(match s.to_ascii_lowercase().as_str() {
            "good" | "success" | "ok" | "green" => Tone::Good,
            "warn" | "warning" | "amber" => Tone::Warn,
            "bad" | "error" | "danger" | "red" | "fail" | "failed" => Tone::Bad,
            "info" | "blue" => Tone::Info,
            _ => Tone::Neutral,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatItem {
    pub value: String,
    pub label: String,
    #[serde(default)]
    pub trend: Option<String>,
    #[serde(default)]
    pub tone: Tone,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListItem {
    pub title: String,
    #[serde(default)]
    pub meta: Option<String>,
    /// An icon name from the app's set (`check`, `bell`, `server`, …).
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub tone: Tone,
    #[serde(default)]
    pub action: Option<ViewAction>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckItem {
    pub text: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimelineItem {
    #[serde(default)]
    pub time: String,
    pub text: String,
    #[serde(default)]
    pub tone: Tone,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Button {
    pub label: String,
    pub action: ViewAction,
    #[serde(default)]
    pub primary: bool,
}

/// One block of a view. Unknown blocks are dropped, so an older notch
/// reads a newer agent's views.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Heading {
        text: String,
    },
    /// Markdown: paragraphs, lists, **bold**, `code`.
    Text {
        text: String,
    },
    Stat {
        value: String,
        label: String,
        #[serde(default)]
        trend: Option<String>,
        #[serde(default)]
        tone: Tone,
    },
    Stats {
        items: Vec<StatItem>,
    },
    Progress {
        label: String,
        /// 0–1.
        value: f32,
        #[serde(default)]
        tone: Tone,
    },
    List {
        items: Vec<ListItem>,
    },
    Checklist {
        items: Vec<CheckItem>,
    },
    Timeline {
        items: Vec<TimelineItem>,
    },
    Diff {
        file: String,
        #[serde(default)]
        added: u32,
        #[serde(default)]
        removed: u32,
        /// `+line` / `-line` / ` line`.
        #[serde(default)]
        lines: Vec<String>,
    },
    /// A live agent card: it updates itself from the snapshot.
    Agent {
        task: TaskId,
    },
    /// That agent's plan, live: its pending plan or its checklist.
    Plan {
        task: TaskId,
    },
    Actions {
        items: Vec<Button>,
    },
    Callout {
        text: String,
        #[serde(default)]
        tone: Tone,
    },
    Split {
        left: Vec<Block>,
        right: Vec<Block>,
    },
    Section {
        title: String,
        blocks: Vec<Block>,
    },
    #[serde(other)]
    Unknown,
}

/// What a button or row does: a checked command, never code. Every one runs
/// only from the user's click (none runs when a view arrives), and an
/// approval the notch can't make safely goes to the app instead.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum ViewAction {
    Reply { task: TaskId, text: String },
    Approve { task: TaskId, request: String },
    Deny { task: TaskId, request: String },
    Stop { task: TaskId },
    Open { task: TaskId },
    /// Start an agent (`agent` and `project` by id or name; the defaults
    /// otherwise).
    Start {
        prompt: String,
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        project: Option<String>,
    },
    Merge { task: TaskId, pr: u64 },
    RunAutomation { id: String },
    Timer { mins: u64 },
    /// Ask K22 a follow-up.
    Ask { text: String },
    /// Review an agent's plan.
    Plan { task: TaskId },
    /// Talk to K22 (or about `task`).
    Call {
        #[serde(default)]
        task: Option<TaskId>,
    },
    #[serde(other)]
    None,
}

impl ViewAction {
    /// Changes something an agent does (worth a confirming look).
    pub fn acts(&self) -> bool {
        !matches!(self, ViewAction::Open { .. } | ViewAction::Plan { .. } | ViewAction::None)
    }
}

/// Limits that keep a view small, quick and readable.
const MAX_BLOCKS: usize = 40;
const MAX_ITEMS: usize = 24;
const MAX_TEXT: usize = 2400;
const MAX_LINE: usize = 200;

fn clip(s: &mut String, max: usize) {
    if s.chars().count() > max {
        *s = s.chars().take(max.saturating_sub(1)).collect::<String>() + "…";
    }
}

impl NotchView {
    /// Reads a `notch_view` call's arguments: `{title, subtitle?, blocks}`.
    pub fn from_args(args: &Value) -> Result<NotchView, String> {
        let title = args.get("title").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty());
        let Some(title) = title else { return Err("notch_view needs a title.".into()) };
        let blocks = match args.get("blocks") {
            Some(Value::Array(_)) => serde_json::from_value::<Vec<Block>>(args["blocks"].clone())
                .map_err(|e| format!("notch_view: a block didn't parse: {e}"))?,
            Some(Value::String(s)) => {
                serde_json::from_str::<Vec<Block>>(s).map_err(|e| format!("notch_view: blocks didn't parse: {e}"))?
            }
            _ => return Err("notch_view needs blocks: an array of {\"type\": …} objects.".into()),
        };
        let mut view = NotchView {
            title: title.to_string(),
            subtitle: args.get("subtitle").and_then(Value::as_str).map(str::to_string),
            blocks,
            ..Default::default()
        };
        view.sanitize();
        if view.blocks.is_empty() {
            return Err("notch_view: none of the blocks are ones the notch knows. See the tool's description.".into());
        }
        Ok(view)
    }

    /// Drops unknown blocks, nests at most one level, and trims everything to
    /// the notch's limits.
    pub fn sanitize(&mut self) {
        clip(&mut self.title, 80);
        if let Some(s) = self.subtitle.as_mut() {
            clip(s, 140);
        }
        let mut budget = MAX_BLOCKS;
        self.blocks = sanitize_blocks(std::mem::take(&mut self.blocks), 0, &mut budget);
    }

    /// Every action anywhere in the view, for checks and tests.
    #[cfg(test)]
    pub fn actions(&self) -> Vec<&ViewAction> {
        fn walk<'a>(blocks: &'a [Block], out: &mut Vec<&'a ViewAction>) {
            for b in blocks {
                match b {
                    Block::List { items } => out.extend(items.iter().filter_map(|i| i.action.as_ref())),
                    Block::Actions { items } => out.extend(items.iter().map(|b| &b.action)),
                    Block::Split { left, right } => {
                        walk(left, out);
                        walk(right, out);
                    }
                    Block::Section { blocks, .. } => walk(blocks, out),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.blocks, &mut out);
        out
    }
}

fn sanitize_blocks(blocks: Vec<Block>, depth: usize, budget: &mut usize) -> Vec<Block> {
    let mut out = Vec::new();
    for b in blocks {
        if *budget == 0 {
            break;
        }
        let b = match b {
            Block::Unknown => continue,
            Block::Split { .. } | Block::Section { .. } if depth >= 1 => continue,
            Block::Split { left, right } => Block::Split {
                left: sanitize_blocks(left, depth + 1, budget),
                right: sanitize_blocks(right, depth + 1, budget),
            },
            Block::Section { mut title, blocks } => {
                clip(&mut title, 80);
                Block::Section { title, blocks: sanitize_blocks(blocks, depth + 1, budget) }
            }
            Block::Heading { mut text } => {
                clip(&mut text, 100);
                Block::Heading { text }
            }
            Block::Text { mut text } => {
                clip(&mut text, MAX_TEXT);
                Block::Text { text }
            }
            Block::Callout { mut text, tone } => {
                clip(&mut text, 400);
                Block::Callout { text, tone }
            }
            Block::Stat { mut value, mut label, trend, tone } => {
                clip(&mut value, 24);
                clip(&mut label, 60);
                Block::Stat { value, label, trend, tone }
            }
            Block::Stats { mut items } => {
                items.truncate(6);
                for i in &mut items {
                    clip(&mut i.value, 24);
                    clip(&mut i.label, 60);
                }
                Block::Stats { items }
            }
            Block::Progress { mut label, value, tone } => {
                clip(&mut label, 80);
                Block::Progress { label, value: if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 }, tone }
            }
            Block::List { mut items } => {
                items.truncate(MAX_ITEMS);
                for i in &mut items {
                    clip(&mut i.title, MAX_LINE);
                    if let Some(m) = i.meta.as_mut() {
                        clip(m, MAX_LINE);
                    }
                }
                Block::List { items }
            }
            Block::Checklist { mut items } => {
                items.truncate(MAX_ITEMS);
                for i in &mut items {
                    clip(&mut i.text, MAX_LINE);
                }
                Block::Checklist { items }
            }
            Block::Timeline { mut items } => {
                items.truncate(MAX_ITEMS);
                for i in &mut items {
                    clip(&mut i.text, MAX_LINE);
                    clip(&mut i.time, 24);
                }
                Block::Timeline { items }
            }
            Block::Diff { mut file, added, removed, mut lines } => {
                clip(&mut file, 160);
                lines.truncate(16);
                for l in &mut lines {
                    clip(l, MAX_LINE);
                }
                Block::Diff { file, added, removed, lines }
            }
            Block::Actions { mut items } => {
                items.retain(|b| b.action != ViewAction::None);
                items.truncate(4);
                for b in &mut items {
                    clip(&mut b.label, 32);
                }
                if items.is_empty() {
                    continue;
                }
                Block::Actions { items }
            }
            b @ (Block::Agent { .. } | Block::Plan { .. }) => b,
        };
        *budget -= 1;
        out.push(b);
    }
    out
}

/// The `notch_view` tool, as an MCP `tools/list` entry.
pub fn tool() -> Value {
    json!({
        "name": "notch_view",
        "description": NOTCH_VIEW,
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": {"type": "string", "description": "A short title (a few words)."},
                "subtitle": {"type": "string", "description": "One line under the title."},
                "blocks": {"type": "array", "items": {"type": "object"}, "description": "The view's blocks, top to bottom."}
            },
            "required": ["title", "blocks"]
        }
    })
}

const NOTCH_VIEW: &str = "\
Show a live view in the user's notch (the island at the top of their screen). The notch unfolds and draws it \
natively. Use it to answer with structure: status across agents, what needs the user, a plan, a comparison, \
progress, next steps. Keep it scannable: a heading, a few stats or rows, and actions. Call it again with the same \
title to update it. The user sees it, so your text reply should be one or two sentences that don't restate it.\n\
Blocks (each an object with \"type\"):\n\
- {\"type\":\"heading\",\"text\":\"…\"}\n\
- {\"type\":\"text\",\"text\":\"markdown\"}\n\
- {\"type\":\"stats\",\"items\":[{\"value\":\"3\",\"label\":\"running\",\"tone\":\"good\"}]} (or one \"stat\")\n\
- {\"type\":\"progress\",\"label\":\"Tests\",\"value\":0.7}\n\
- {\"type\":\"list\",\"items\":[{\"title\":\"…\",\"meta\":\"…\",\"icon\":\"check\",\"tone\":\"warn\",\"action\":{…}}]}\n\
- {\"type\":\"checklist\",\"items\":[{\"text\":\"…\",\"done\":true}]}\n\
- {\"type\":\"timeline\",\"items\":[{\"time\":\"10:42\",\"text\":\"…\"}]}\n\
- {\"type\":\"diff\",\"file\":\"src/a.rs\",\"added\":3,\"removed\":1,\"lines\":[\"+new\",\"-old\"]}\n\
- {\"type\":\"agent\",\"task\":\"<thread id>\"} (a live card that updates itself)\n\
- {\"type\":\"plan\",\"task\":\"<thread id>\"} (that thread's plan or checklist, live)\n\
- {\"type\":\"callout\",\"text\":\"…\",\"tone\":\"bad\"}\n\
- {\"type\":\"actions\",\"items\":[{\"label\":\"Approve\",\"primary\":true,\"action\":{…}}]}\n\
- {\"type\":\"section\",\"title\":\"…\",\"blocks\":[…]} and {\"type\":\"split\",\"left\":[…],\"right\":[…]} (one level deep)\n\
Tones: neutral, good, warn, bad, info. Actions run only when the user clicks: \
{\"do\":\"reply\",\"task\":id,\"text\":\"…\"}, {\"do\":\"approve\",\"task\":id,\"request\":request_id}, \
{\"do\":\"deny\",…}, {\"do\":\"stop\",\"task\":id}, {\"do\":\"open\",\"task\":id}, \
{\"do\":\"start\",\"prompt\":\"…\",\"agent\":\"codex\",\"project\":\"dove\"}, {\"do\":\"merge\",\"task\":id,\"pr\":12}, \
{\"do\":\"run_automation\",\"id\":\"…\"}, {\"do\":\"timer\",\"mins\":25}, {\"do\":\"ask\",\"text\":\"a follow-up\"}, \
{\"do\":\"plan\",\"task\":id}, {\"do\":\"call\"}. Thread ids come from the team tools (thread_list).";

/// Something to open in the notch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum ShowWhat {
    Call,
    Plan { task: TaskId },
    Agent { task: TaskId },
    Lens { id: String },
    View { id: u64 },
}

/// K22's agent and model, and the models it could use, for the call page.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct VoiceInfo {
    pub agent: Option<HarnessKind>,
    pub model: Option<String>,
    /// (id, label) for `agent`.
    #[serde(default)]
    pub models: Vec<(String, String)>,
}

// ---------------------------------------------------------------------------
// notch_* tool calls from agents (via `browser::control`)
// ---------------------------------------------------------------------------

/// One `notch_*` call from an agent, with where to send the answer.
pub struct ToolCall {
    pub tool: String,
    pub args: Value,
    /// The calling session (`DOVE_SESSION`) and folder.
    pub session: String,
    pub cwd: Option<String>,
    pub reply: tokio::sync::oneshot::Sender<Result<String, String>>,
}

fn calls() -> &'static (flume::Sender<ToolCall>, Mutex<Option<flume::Receiver<ToolCall>>>) {
    static CH: OnceLock<(flume::Sender<ToolCall>, Mutex<Option<flume::Receiver<ToolCall>>>)> = OnceLock::new();
    CH.get_or_init(|| {
        let (tx, rx) = flume::unbounded();
        (tx, Mutex::new(Some(rx)))
    })
}

/// Hands a call to the island host; the answer comes back on the oneshot.
pub async fn submit(tool: String, args: Value, session: String, cwd: Option<String>) -> Result<String, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    calls()
        .0
        .send(ToolCall { tool, args, session, cwd, reply: tx })
        .map_err(|_| "The notch isn't available.".to_string())?;
    rx.await.unwrap_or_else(|_| Err("The notch didn't answer.".into()))
}

/// The call stream, once (the island host owns it).
pub fn take_calls() -> Option<flume::Receiver<ToolCall>> {
    calls().1.lock().ok()?.take()
}

// ---------------------------------------------------------------------------
// Plans
// ---------------------------------------------------------------------------

/// An agent's plan, waiting on the user (`request_id`), or just proposed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingPlan {
    pub task: TaskId,
    /// The permission request that approves it (plan mode's exit), if any.
    pub request_id: Option<String>,
    pub kind: HarnessKind,
    pub title: String,
    pub project: String,
    pub text: String,
    pub steps: Vec<PlanStep>,
    /// Files the plan names.
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlanStep {
    pub text: String,
    /// Indented detail under the step.
    #[serde(default)]
    pub detail: Vec<String>,
}

/// How the user answered a plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "v", rename_all = "snake_case")]
pub enum PlanVerdict {
    Approve,
    /// Approve, with the user's edited plan sent along.
    Edited { text: String },
    /// Keep planning, with why.
    Reject { reason: String },
}

/// The request that gates a plan (Claude Code's `ExitPlanMode`, ACP's
/// switch out of plan mode), recognised by name or by a `plan` input.
pub fn is_plan_request(req: &PermissionRequest) -> bool {
    if req.is_question() {
        return false;
    }
    let tool = req.tool.to_ascii_lowercase().replace(['_', '-', ' '], "");
    tool.contains("exitplan")
        || tool == "plan"
        || tool.contains("switchmode") && req.input.to_string().contains("plan")
        || req.input.get("plan").is_some_and(|p| p.as_str().is_some_and(|s| !s.trim().is_empty()))
}

/// A plan request's text: its `plan` input, else the plan the agent last
/// showed, else what it said this turn.
pub fn plan_text(req: Option<&PermissionRequest>, state: &TaskState) -> Option<String> {
    let from_req = req.and_then(|r| r.input.get("plan").and_then(Value::as_str)).map(str::trim).filter(|s| !s.is_empty());
    from_req
        .map(str::to_string)
        .or_else(|| state.plan.clone().filter(|p| !p.trim().is_empty()))
        .or_else(|| req.map(|_| state.reply.trim().to_string()).filter(|r| !r.is_empty()))
}

/// The plans in `state`: one per plan request, or the proposed plan.
pub fn plans_of(task: TaskId, kind: HarnessKind, title: &str, project: &str, state: &TaskState) -> Vec<PendingPlan> {
    let make = |request_id: Option<String>, text: String| {
        let steps = plan_steps(&text);
        let files = plan_files(&text);
        PendingPlan { task, request_id, kind, title: title.to_string(), project: project.to_string(), text, steps, files }
    };
    let mut out: Vec<PendingPlan> = state
        .pending
        .iter()
        .filter(|r| is_plan_request(r))
        .filter_map(|r| plan_text(Some(r), state).map(|t| make(Some(r.request_id.clone()), t)))
        .collect();
    if out.is_empty()
        && state.live
        && let Some(text) = state.plan.clone().filter(|p| !p.trim().is_empty())
    {
        out.push(make(None, text));
    }
    out
}

/// Steps from a plan's markdown: its top-level list items with what's
/// indented under them; else its headings; else its paragraphs.
pub fn plan_steps(md: &str) -> Vec<PlanStep> {
    let mut steps: Vec<PlanStep> = Vec::new();
    let mut base: Option<usize> = None;
    let mut in_code = false;
    for raw in md.lines() {
        if raw.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code || raw.trim().is_empty() {
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        let line = raw.trim();
        let item = list_item(line);
        match (item, base) {
            (Some(text), None) => {
                base = Some(indent);
                steps.push(PlanStep { text, detail: Vec::new() });
            }
            (Some(text), Some(b)) if indent <= b + 1 => steps.push(PlanStep { text, detail: Vec::new() }),
            (Some(text), Some(_)) => {
                if let Some(s) = steps.last_mut() {
                    s.detail.push(text);
                }
            }
            (None, Some(b)) if indent > b && !line.starts_with('#') => {
                if let Some(s) = steps.last_mut() {
                    s.detail.push(inline(line));
                }
            }
            _ => {}
        }
    }
    if !steps.is_empty() {
        return steps;
    }
    let heads: Vec<PlanStep> = md
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("##") || (l.starts_with('#') && !l.starts_with("# ")))
        .map(|l| PlanStep { text: inline(l.trim_start_matches('#').trim()), detail: Vec::new() })
        .filter(|s| !s.text.is_empty())
        .collect();
    if heads.len() >= 2 {
        return heads;
    }
    md.split("\n\n")
        .map(|p| p.lines().map(str::trim).filter(|l| !l.starts_with('#')).collect::<Vec<_>>().join(" "))
        .map(|p| inline(&p))
        .filter(|p| !p.is_empty())
        .map(|text| PlanStep { text, detail: Vec::new() })
        .take(MAX_ITEMS)
        .collect()
}

/// `1. foo` / `- foo` / `* [ ] foo` → `foo`.
fn list_item(line: &str) -> Option<String> {
    let rest = if let Some(r) = line.strip_prefix("- ").or(line.strip_prefix("* ")).or(line.strip_prefix("+ ")) {
        r
    } else {
        let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 || digits > 3 {
            return None;
        }
        let after = &line[digits..];
        after.strip_prefix(". ").or(after.strip_prefix(") "))?
    };
    let rest = rest.trim_start_matches("[ ] ").trim_start_matches("[x] ").trim_start_matches("[X] ");
    let text = inline(rest);
    (!text.is_empty()).then_some(text)
}

/// Light inline markdown cleanup for a one-line step.
fn inline(s: &str) -> String {
    s.replace("**", "").replace("__", "").trim().trim_end_matches(':').trim().to_string()
}

/// File paths a plan names (`src/auth.rs`, `apps/x/y.tsx`).
pub fn plan_files(md: &str) -> Vec<String> {
    const EXT: &[&str] = &[
        "rs", "ts", "tsx", "js", "jsx", "py", "go", "md", "toml", "json", "yaml", "yml", "css", "html", "swift", "kt",
        "java", "c", "h", "cpp", "rb", "sh", "sql", "svg", "lock",
    ];
    let mut out: Vec<String> = Vec::new();
    for word in md.split(|c: char| c.is_whitespace() || matches!(c, '`' | '(' | ')' | ',' | '"' | '\'' | '[' | ']')) {
        let w = word.trim_end_matches(['.', ':', ';']);
        let Some((stem, ext)) = w.rsplit_once('.') else { continue };
        if stem.is_empty() || !EXT.contains(&ext) || w.contains("://") {
            continue;
        }
        if (w.contains('/') || w.contains('\\') || ext != "md") && !out.iter().any(|o| o == w) {
            out.push(w.to_string());
        }
        if out.len() >= 12 {
            break;
        }
    }
    out
}

/// A plan as markdown again (after edits).
pub fn plan_markdown(steps: &[PlanStep]) -> String {
    let mut out = String::new();
    for (i, s) in steps.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, s.text));
        for d in &s.detail {
            out.push_str(&format!("   - {d}\n"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallPhase {
    #[default]
    Loading,
    Listening,
    /// The user is talking.
    Hearing,
    /// A turn was sent; waiting on the agent.
    Thinking,
    Speaking,
    Muted,
}

impl<'de> Deserialize<'de> for CallPhase {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(d)?.as_str() {
            "listening" => CallPhase::Listening,
            "hearing" => CallPhase::Hearing,
            "thinking" => CallPhase::Thinking,
            "speaking" => CallPhase::Speaking,
            "muted" => CallPhase::Muted,
            _ => CallPhase::Loading,
        })
    }
}

impl CallPhase {
    pub fn of(s: dove_voice::pipeline::State) -> Self {
        use dove_voice::pipeline::State;
        match s {
            State::Loading => CallPhase::Loading,
            State::Listening => CallPhase::Listening,
            State::Hearing => CallPhase::Hearing,
            State::Thinking => CallPhase::Thinking,
            State::Speaking => CallPhase::Speaking,
            State::Muted => CallPhase::Muted,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            CallPhase::Loading => "Connecting",
            CallPhase::Listening => "Listening",
            CallPhase::Hearing => "Hearing you",
            CallPhase::Thinking => "Thinking",
            CallPhase::Speaking => "Speaking",
            CallPhase::Muted => "Muted",
        }
    }
}

/// A call as the notch shows it.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct CallInfo {
    pub live: bool,
    #[serde(default)]
    pub phase: CallPhase,
    /// Who it talks to ("K22", or a thread's title and agent).
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub target_task: Option<TaskId>,
    #[serde(default)]
    pub target_kind: Option<HarnessKind>,
    /// The last line heard or said, and the words so far while hearing.
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    pub partial: Option<String>,
    /// The conversation so far (you?, text), oldest first.
    #[serde(default)]
    pub lines: Vec<(bool, String)>,
    /// Mic or speaker level, 0–1, stepped.
    #[serde(default)]
    pub level: f32,
    #[serde(default)]
    pub elapsed_s: u32,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub muted: bool,
    /// A destructive request confirmed by voice: it needs a click in the app.
    #[serde(default)]
    pub awaiting_click: Option<(TaskId, String)>,
    /// K22's agent and model for this call.
    #[serde(default)]
    pub agent: Option<HarnessKind>,
    #[serde(default)]
    pub model: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_parse_and_unknown_blocks_drop() {
        let args = json!({
            "title": "Release status",
            "blocks": [
                {"type": "heading", "text": "Today"},
                {"type": "hologram", "text": "from the future"},
                {"type": "stats", "items": [{"value": "3", "label": "running", "tone": "good"}, {"value": "1", "label": "failed", "tone": "sparkly"}]},
                {"type": "actions", "items": [{"label": "Timer", "action": {"do": "timer", "mins": 25}}, {"label": "?", "action": {"do": "format_disk"}}]}
            ]
        });
        let v = NotchView::from_args(&args).unwrap();
        assert_eq!(v.title, "Release status");
        assert_eq!(v.blocks.len(), 3, "the unknown block is dropped");
        let Block::Stats { items } = &v.blocks[1] else { panic!("stats") };
        assert_eq!(items[1].tone, Tone::Neutral, "an unknown tone reads as neutral");
        let Block::Actions { items } = &v.blocks[2] else { panic!("actions") };
        assert_eq!(items.len(), 1, "an unknown action is dropped, never run");
        assert_eq!(items[0].action, ViewAction::Timer { mins: 25 });
    }

    #[test]
    fn views_nest_one_level_and_stay_small() {
        let deep = json!({"type": "section", "title": "in", "blocks": [{"type": "text", "text": "x"}]});
        let args = json!({
            "title": "t",
            "blocks": [{"type": "section", "title": "out", "blocks": [deep, {"type": "text", "text": "kept"}]}]
        });
        let v = NotchView::from_args(&args).unwrap();
        let Block::Section { blocks, .. } = &v.blocks[0] else { panic!("section") };
        assert_eq!(blocks.len(), 1, "a section inside a section is dropped");
        let many: Vec<Value> = (0..100).map(|i| json!({"type": "text", "text": format!("{i}")})).collect();
        let v = NotchView::from_args(&json!({"title": "t", "blocks": many})).unwrap();
        assert_eq!(v.blocks.len(), MAX_BLOCKS);
        assert!(NotchView::from_args(&json!({"title": "t", "blocks": [{"type": "nope"}]})).is_err());
        assert!(NotchView::from_args(&json!({"blocks": []})).is_err());
    }

    #[test]
    fn a_view_lists_its_actions_and_none_runs_by_itself() {
        let task = TaskId::new();
        let args = json!({
            "title": "t",
            "blocks": [
                {"type": "list", "items": [{"title": "a", "action": {"do": "open", "task": task}}]},
                {"type": "split", "left": [{"type": "actions", "items": [{"label": "Stop", "action": {"do": "stop", "task": task}}]}], "right": []}
            ]
        });
        let v = NotchView::from_args(&args).unwrap();
        let acts = v.actions();
        assert_eq!(acts.len(), 2);
        assert!(!acts[0].acts(), "opening changes nothing");
        assert!(acts[1].acts());
    }

    #[test]
    fn plan_steps_come_from_lists_headings_or_paragraphs() {
        let md = "# Plan\n\n1. Add a `login` route in `src/auth.rs`\n   - validate the token\n   - return 401 on failure\n2. **Write tests**:\n\n```rust\nfn x() {}\n- not a step\n```\n3. Update docs/AUTH.md\n";
        let steps = plan_steps(md);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].text, "Add a `login` route in `src/auth.rs`");
        assert_eq!(steps[0].detail, vec!["validate the token", "return 401 on failure"]);
        assert_eq!(steps[1].text, "Write tests");
        assert_eq!(plan_files(md), vec!["src/auth.rs", "docs/AUTH.md"]);

        let heads = plan_steps("## Investigate\nlook around\n## Fix\nchange it\n");
        assert_eq!(heads.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["Investigate", "Fix"]);

        let paras = plan_steps("First, read the code.\n\nThen fix the bug.");
        assert_eq!(paras.len(), 2);

        let back = plan_markdown(&steps);
        assert!(back.starts_with("1. Add a `login` route"));
        assert_eq!(plan_steps(&back).len(), 3, "an edited plan reads back the same");
    }

    #[test]
    fn plan_requests_are_recognised() {
        let req = |tool: &str, input: Value| PermissionRequest {
            request_id: "r1".into(),
            tool: tool.into(),
            summary: "s".into(),
            input,
            questions: Vec::new(),
        };
        assert!(is_plan_request(&req("ExitPlanMode", json!({"plan": "1. a"}))));
        assert!(is_plan_request(&req("exit_plan_mode", json!({}))));
        assert!(is_plan_request(&req("switch_mode", json!({"mode": "code", "from": "plan"}))));
        assert!(!is_plan_request(&req("Bash", json!({"command": "ls"}))));
        let state = TaskState { plan: Some("1. step".into()), live: true, ..Default::default() };
        let r = req("ExitPlanMode", json!({"plan": "1. from the request\n2. two"}));
        assert_eq!(plan_text(Some(&r), &state).as_deref(), Some("1. from the request\n2. two"));
        let s2 = TaskState { pending: vec![r], ..state.clone() };
        let plans = plans_of(TaskId::new(), HarnessKind::ClaudeCode, "t", "p", &s2);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].request_id.as_deref(), Some("r1"));
        assert_eq!(plans[0].steps.len(), 2);
        let proposed = plans_of(TaskId::new(), HarnessKind::Codex, "t", "p", &state);
        assert_eq!(proposed[0].request_id, None, "a plan without a gate is shown, not approved");
    }

    #[test]
    fn call_info_reads_from_older_engines() {
        let old: CallInfo = serde_json::from_str(r#"{"live":true}"#).unwrap();
        assert!(old.live);
        assert_eq!(old.phase, CallPhase::Loading);
        let odd: CallInfo = serde_json::from_str(r#"{"live":true,"phase":"teleporting"}"#).unwrap();
        assert_eq!(odd.phase, CallPhase::Loading);
    }
}
