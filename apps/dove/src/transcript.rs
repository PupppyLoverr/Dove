//! One thread's conversation: a virtualized transcript, the agent's todo
//! list, the question panel and a composer bound to the thread's agent.
//!
//! The transcript model ([`Chat`]) is plain data with no GPUI types, so a
//! saved transcript is rebuilt (parsed and syntax-highlighted) on the
//! background executor and the UI thread only swaps it in.
//!
//! Live events arrive from the engine on a channel. The pump wakes on the
//! first event, waits one 120 Hz frame for more, applies the batch and
//! notifies once. Assistant text is parsed incrementally (see
//! [`markdown::Stream`]): finished top-level blocks become their own rows and
//! only the open tail row is re-laid out per frame.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dove_core::proto::{
    AgentEvent, PermissionDecision, PermissionRequest, RateWindow, TaskStatus, TodoItem, TodoStatus, ToolKind, Usage,
};
use dove_core::store::{Project, Task};
use dove_core::{Command, Engine, LiveSession, git};
use gpui::prelude::FluentBuilder as _;
use gpui::{Anchor, anchored, deferred};
use gpui::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter,
    FocusHandle, Focusable, FollowMode, InteractiveElement as _, IntoElement, KeyDownEvent, ListAlignment, ListOffset,
    ListState, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    WeakEntity, Window, canvas, div, list, px,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::composer::{self, Composer, ComposerEvent};
use crate::kit;
use crate::markdown::{self, Block, Stream};
use crate::theme::Theme;
use crate::ui;

const FRAME: Duration = Duration::from_millis(8);
const DIFF_PREVIEW: usize = 8;
const DIFF_MAX: usize = 600;
/// Steps of a live activity shown before the rest fold away.
const LIVE_STEPS: usize = 6;
/// Output lines an open chip shows before its counted "more" row.
const OUTPUT_KEEP: usize = 24;
/// Output lines once the user asks for all of it.
const OUTPUT_FULL: usize = 400;
/// A one-line call this short reads whole in its chip header.
const INVOCATION_INLINE: usize = 56;
pub const COLUMN_MAX: f32 = 760.0;
/// The composer dock's height before its first measurement: a one-line
/// pill and its meta row.
const DOCK_GUESS: f32 = 90.0;
/// The composer's meta row under the pill, with its gap.
const COMPOSER_META: f32 = 34.0;

pub enum TaskViewEvent {
    Status(TaskStatus),
    Changed,
    /// The native session id became known (resume / fork need it).
    Native(String),
    /// Fork this thread (from its message actions).
    Fork,
    /// Show the side panel's Changes tab.
    ShowChanges,
    /// The thread's first answer arrived; a better title is available.
    Title(String),
    /// From the read-only banner: hand the session back to the chat.
    SwitchToChat,
    /// From the read-only banner: show the running terminal.
    ShowTerminal,
    /// Drive the in-app browser the way a person would.
    DriveBrowser(crate::browser::Drive),
    /// Show a design in the side panel. `true` when the user asked (a
    /// card's Open); `false` when it just arrived from the agent.
    OpenCanvas(crate::canvas::Spec, bool),
}

// ------------------------------------------------------------------ model

/// A tool call inside an activity fold.
#[derive(Clone)]
pub struct Tool {
    /// The harness's call id (it also keys a subagent's own events).
    pub id: String,
    pub title: SharedString,
    pub kind: ToolKind,
    pub output: Option<SharedString>,
    pub error: bool,
    pub diff: Option<Arc<MiniDiff>>,
    /// The user's fold choice; `None` follows [`Tool::opens_by_default`].
    pub open: Option<bool>,
    /// Show the whole output (or diff), not its first screen.
    pub full: bool,
    /// The harness's tool name (`Bash`, `mcp__server__tool`, ...).
    pub name: SharedString,
    pub input: Arc<serde_json::Value>,
    started: Option<Instant>,
    /// Wall time from call to result, for live calls.
    pub took: Option<Duration>,
}

/// A subagent a thread started (Claude Code's Task tool and its kin).
#[derive(Debug, Clone, PartialEq)]
pub struct Subagent {
    pub title: String,
    /// The subagent type, when the harness names one ("Explore", "Plan").
    pub kind: String,
    pub prompt: String,
    pub output: Option<String>,
    pub error: bool,
    pub running: bool,
    pub took: Option<Duration>,
    /// The transcript row holding the call.
    pub row: usize,
    /// The spawning call's id, which keys the subagent's own events.
    pub id: String,
    /// Tool calls the subagent has made so far.
    pub steps: usize,
    /// What it is doing right now, while it runs.
    pub doing: Option<String>,
}

/// An edit's diff in the Changes pane's model, as Zeron's inline tool
/// diffs: hunks with 3 lines of context, dual line numbers, and syntax
/// spans computed once here, never per paint. Capped for display.
#[derive(Debug, Clone, PartialEq)]
pub struct MiniDiff {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub file: git::FileDiff,
    /// Per hunk, per line; empty when the language is unknown.
    pub spans: Vec<Vec<Vec<zeron_syntax::HighlightSpan>>>,
    /// Lines kept across all hunks (hunk headers not counted).
    pub lines: usize,
    pub truncated: bool,
}

impl MiniDiff {
    pub fn new(path: &str, old: &str, new: &str) -> Self {
        use git::{DiffLine, FileDiff, FileStatus, Hunk, LineKind};
        let diff = similar::TextDiff::from_lines(old, new);
        let (mut added, mut removed, mut kept) = (0, 0, 0usize);
        let mut truncated = false;
        let mut hunks = Vec::new();
        for group in diff.grouped_ops(3) {
            let (Some(first), Some(last)) = (group.first(), group.last()) else { continue };
            let (o, n) = (first.old_range().start..last.old_range().end, first.new_range().start..last.new_range().end);
            let header = format!("@@ -{},{} +{},{} @@", o.start + 1, o.len(), n.start + 1, n.len());
            let mut lines = Vec::new();
            for op in &group {
                for change in diff.iter_changes(op) {
                    let kind = match change.tag() {
                        similar::ChangeTag::Insert => {
                            added += 1;
                            LineKind::Added
                        }
                        similar::ChangeTag::Delete => {
                            removed += 1;
                            LineKind::Removed
                        }
                        similar::ChangeTag::Equal => LineKind::Context,
                    };
                    if kept >= DIFF_MAX {
                        truncated = true;
                        continue;
                    }
                    kept += 1;
                    let text = change.value().trim_end_matches(['\n', '\r']).replace('\t', "    ");
                    lines.push(DiffLine {
                        kind,
                        old_no: change.old_index().map(|i| i as u32 + 1),
                        new_no: change.new_index().map(|i| i as u32 + 1),
                        text: text.into(),
                    });
                }
            }
            if !lines.is_empty() {
                hunks.push(Hunk { header: header.into(), lines });
            }
        }
        let file = FileDiff {
            path: path.to_string(),
            old_path: None,
            status: if old.is_empty() { FileStatus::Added } else { FileStatus::Modified },
            binary: false,
            additions: added as u32,
            deletions: removed as u32,
            hunks,
        };
        let spans = crate::changes::highlight_file(&file);
        Self { path: path.to_string(), added, removed, file, spans, lines: kept, truncated }
    }
}

/// One stretch of reasoning inside an activity, parsed as it streams.
#[derive(Default, Clone)]
pub struct Thought {
    /// How many tool calls preceded it, which places it on the rail.
    pub at: usize,
    pub text: String,
    /// Zeron's flattened thought detail: the mended display tree while it
    /// streams, the canonical tree once it settles.
    pub detail: Arc<crate::tool_calls::ThoughtDetail>,
    pub live: bool,
    /// The user's fold choice; `None` is open while it streams.
    pub open: Option<bool>,
    started: Option<Instant>,
}

#[derive(Default, Clone)]
pub struct Activity {
    /// All reasoning, concatenated (for summaries and the status strip).
    pub thinking: String,
    /// The same reasoning, segmented by the tool calls between it.
    pub thoughts: Vec<Thought>,
    pub tools: Vec<Tool>,
    /// The user's choice; `None` follows the default (open while live).
    pub pinned: Option<bool>,
    pub live: bool,
    started: Option<Instant>,
    thought: Option<Duration>,
}

impl Activity {
    /// Zeron's group summary ("Thought for 4s · ran 3 commands · edited 2
    /// files · 1 failed"), with Dove's thinking time.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.thinking.trim().is_empty() {
            parts.push(match self.thought {
                Some(d) if d.as_secs() >= 1 => format!("Thought for {}", duration(d)),
                _ => "Thought process".into(),
            });
        }
        let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let (mut ran, mut read, mut searched, mut fetched, mut todos, mut other) = (0, 0, 0, 0, 0, 0);
        let mut edited: Vec<String> = Vec::new();
        for t in &self.tools {
            match t.kind {
                _ if todo_items(&t.name, &t.input).is_some() => todos += 1,
                ToolKind::Execute => ran += 1,
                ToolKind::Edit => {
                    let path = t.diff.as_ref().map(|d| d.path.clone()).or_else(|| tool_path(t)).unwrap_or_default();
                    if !edited.contains(&path) {
                        edited.push(path);
                    }
                }
                ToolKind::Read => read += 1,
                ToolKind::Search => searched += 1,
                ToolKind::Fetch if t.name.to_ascii_lowercase().contains("search") => searched += 1,
                ToolKind::Fetch => fetched += 1,
                // Spawned subagents are cards outside the fold, not steps in it.
                ToolKind::Agent => {}
                ToolKind::Other | ToolKind::Think => other += 1,
            }
        }
        if ran > 0 {
            parts.push(format!("ran {}", plural(ran, "command", "commands")));
        }
        if !edited.is_empty() {
            parts.push(format!("edited {}", plural(edited.len(), "file", "files")));
        }
        if read > 0 {
            parts.push(format!("read {}", plural(read, "file", "files")));
        }
        if searched > 0 {
            parts.push(format!("searched {}", plural(searched, "time", "times")));
        }
        if fetched > 0 {
            parts.push(format!("fetched {}", plural(fetched, "page", "pages")));
        }
        if todos > 0 {
            parts.push("updated todos".into());
        }
        if other > 0 {
            parts.push(format!("called {}", plural(other, "tool", "tools")));
        }
        if parts.is_empty() {
            return "Working".into();
        }
        let failed = self.failed();
        if failed > 0 {
            parts.push(format!("{failed} failed"));
        }
        let mut s = parts.join(" · ");
        if let Some(first) = s.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        s
    }

    fn failed(&self) -> usize {
        self.tools.iter().filter(|t| t.error).count()
    }
}

#[derive(Clone)]
pub enum Row {
    User {
        text: SharedString,
        at: i64,
    },
    Activity(Activity),
    /// A finished top-level markdown block of assistant message `msg`.
    Block {
        block: Arc<Block>,
        msg: usize,
    },
    /// The open tail of the streaming message.
    Live {
        blocks: Vec<Arc<Block>>,
    },
    Permission {
        req: PermissionRequest,
        decided: Option<PermissionDecision>,
    },
    Plan(Arc<Vec<Block>>),
    Notice(SharedString),
    Error(SharedString),
    /// The end of a turn: message actions and how long it took.
    TurnEnd {
        msgs: Range<usize>,
        prompt: Option<SharedString>,
        elapsed: Option<Duration>,
        failed: bool,
    },
}

struct Open {
    stream: Stream,
    msg: usize,
    row: usize,
}

/// What applying an event changed outside the rows.
#[derive(Default)]
pub struct Effects {
    pub status: Option<TaskStatus>,
    pub native: Option<String>,
    /// A live user message: glide it to the top as the answer streams in.
    pub runway: Option<usize>,
    pub first_answer: bool,
    /// A design arrived (or changed): show it.
    pub canvas: Option<crate::canvas::Spec>,
    /// Assistant text to speak, when Jarvis is on and this was a live turn.
    pub speak: Option<String>,
}

/// A thread's transcript as plain data.
#[derive(Default)]
pub struct Chat {
    pub rows: Vec<Row>,
    /// Source text of each assistant message, for copy.
    pub messages: Vec<String>,
    open: Option<Open>,
    open_activity: Option<usize>,
    /// The streaming thought: its activity row and Zeron's incremental parser.
    thought: Option<(usize, zeron_markdown::IncrementalParser)>,
    tools: HashMap<String, (usize, usize)>,
    pub todos: Vec<TodoItem>,
    pub running_agents: Vec<SharedString>,
    pub status_text: Option<SharedString>,
    pub usage: Usage,
    /// Latest context occupancy: (tokens, window).
    pub context: (Option<u64>, Option<u64>),
    pub rate_limits: Vec<RateWindow>,
    pub turn_started: Option<Instant>,
    turn_first_msg: usize,
    turn_prompt: Option<SharedString>,
    /// Lowest row index changed since the last take (rows past the old
    /// length are new, not changed).
    dirty: Option<usize>,
    /// Finished code blocks still waiting for syntax highlighting.
    pub to_highlight: Vec<usize>,
    answered: bool,
    /// Each subagent's own events, by the spawning call's id, for the
    /// subagent view to replay.
    pub subs: HashMap<String, Vec<AgentEvent>>,
}

impl Chat {
    /// Rebuild a saved transcript (call off the UI thread).
    pub fn replay(events: Vec<AgentEvent>) -> Self {
        let mut chat = Chat::default();
        for ev in events {
            chat.apply(ev, false);
        }
        chat.close_all();
        for i in std::mem::take(&mut chat.to_highlight) {
            if let Some(Row::Block { block, .. }) = chat.rows.get_mut(i) {
                *block = Arc::new(markdown::highlight_block((**block).clone()));
            }
        }
        chat.dirty = None;
        chat
    }

    fn touch(&mut self, i: usize) {
        self.dirty = Some(self.dirty.map_or(i, |d| d.min(i)));
    }

    fn push(&mut self, row: Row) -> usize {
        self.rows.push(row);
        self.rows.len() - 1
    }

    /// End the streaming message: its tail becomes final rows.
    fn close_text(&mut self) {
        let Some(mut open) = self.open.take() else { return };
        let finished = open.stream.finish();
        // The live row is always last while a message streams.
        debug_assert_eq!(open.row + 1, self.rows.len());
        self.rows.truncate(open.row);
        self.touch(open.row);
        for b in finished {
            let wants = b.wants_highlight();
            let i = self.push(Row::Block { block: b, msg: open.msg });
            if wants {
                self.to_highlight.push(i);
            }
        }
    }

    /// The current thought is over: its tail parses as final.
    fn close_thought(&mut self) {
        let Some((i, parser)) = self.thought.take() else { return };
        if let Some(Row::Activity(a)) = self.rows.get_mut(i)
            && let Some(th) = a.thoughts.last_mut()
        {
            // The honest settle: the canonical tree, not the mended display.
            th.detail = Arc::new(crate::tool_calls::thought_detail(parser.tree(), false));
            th.live = false;
        }
        self.touch(i);
    }

    fn close_activity(&mut self) {
        self.close_thought();
        if let Some(i) = self.open_activity.take() {
            if let Some(Row::Activity(a)) = self.rows.get_mut(i) {
                a.live = false;
                if a.thought.is_none() {
                    a.thought = a.started.map(|s| s.elapsed());
                }
            }
            self.touch(i);
        }
    }

    fn close_all(&mut self) {
        self.close_text();
        self.close_activity();
    }

    fn activity(&mut self) -> usize {
        self.close_text();
        if let Some(i) = self.open_activity {
            return i;
        }
        self.push(Row::Activity(Activity { live: true, started: Some(Instant::now()), ..Default::default() }));
        let i = self.rows.len() - 1;
        self.open_activity = Some(i);
        i
    }

    fn text(&mut self, delta: &str) {
        if self.open_activity.is_some() {
            self.close_activity();
        }
        if self.open.is_none() {
            let msg = self.messages.len();
            self.messages.push(String::new());
            let row = self.push(Row::Live { blocks: Vec::new() });
            self.open = Some(Open { stream: Stream::default(), msg, row });
        }
        let open = self.open.as_mut().expect("opened above");
        self.messages[open.msg].push_str(delta);
        let up = open.stream.push(delta);
        let (msg, row) = (open.msg, open.row);
        if up.finished.is_empty() {
            self.rows[row] = Row::Live { blocks: up.live };
            self.touch(row);
            return;
        }
        self.rows.truncate(row);
        self.touch(row);
        for b in up.finished {
            let wants = b.wants_highlight();
            let i = self.push(Row::Block { block: b, msg });
            if wants {
                self.to_highlight.push(i);
            }
        }
        let live = self.push(Row::Live { blocks: up.live });
        if let Some(open) = self.open.as_mut() {
            open.row = live;
        }
    }

    pub fn apply(&mut self, ev: AgentEvent, live: bool) -> Effects {
        let mut fx = Effects::default();
        if live {
            fx.status = dove_core::next_status(&ev);
        }
        match ev {
            AgentEvent::SessionStarted { native_session_id, .. } => fx.native = native_session_id,
            AgentEvent::UserMessage { text } => {
                self.close_all();
                self.status_text = None;
                // Transcripts carry no timestamps; a replayed message shows none
                // rather than the time it was reloaded.
                let at = if live { dove_core::store::now_ms() } else { 0 };
                let text: SharedString = dove_core::strip_persona(&text).to_string().into();
                self.turn_prompt = Some(text.clone());
                self.turn_started = live.then(Instant::now);
                self.turn_first_msg = self.messages.len();
                let i = self.push(Row::User { text, at });
                if live {
                    fx.runway = Some(i);
                }
            }
            AgentEvent::TextDelta { text } => {
                if !self.answered && live && !text.trim().is_empty() {
                    self.answered = true;
                    fx.first_answer = true;
                }
                self.text(&text);
            }
            AgentEvent::ThinkingDelta { text } => {
                let i = self.activity();
                let fresh = match &self.rows[i] {
                    Row::Activity(a) => {
                        a.thoughts.last().is_none_or(|th| !th.live || th.at != a.tools.len())
                            || self.thought.as_ref().is_none_or(|(r, _)| *r != i)
                    }
                    _ => false,
                };
                if fresh {
                    self.close_thought();
                }
                if let Row::Activity(a) = &mut self.rows[i] {
                    a.thinking.push_str(&text);
                    if fresh {
                        a.thoughts.push(Thought {
                            at: a.tools.len(),
                            live: true,
                            started: live.then(Instant::now),
                            ..Default::default()
                        });
                        self.thought = Some((i, zeron_markdown::IncrementalParser::new()));
                    }
                    if let (Some(th), Some((_, parser))) = (a.thoughts.last_mut(), self.thought.as_mut()) {
                        th.text.push_str(&text);
                        // Zeron's streamdown: append reparses only from the
                        // last stable block; the display tree is mended so
                        // half-typed markers never flash raw.
                        parser.append(&text);
                        th.detail = Arc::new(crate::tool_calls::thought_detail(&parser.display_tree(), true));
                    }
                }
                self.touch(i);
            }
            AgentEvent::BlockEnd => self.close_text(),
            // A live design from `show_canvas`: a card that opens the canvas.
            AgentEvent::ToolCall { name, input, .. } if is_show_canvas(&name) => {
                self.close_all();
                if let Some(spec) = crate::canvas::Spec::from_value(input.clone()) {
                    let json = spec.to_json();
                    let existing = self.rows.iter().rposition(|r| match r {
                        Row::Block { block, .. } => matches!(&**block, Block::Canvas(src)
                            if crate::canvas::Spec::parse(src).is_some_and(|s| s.id == spec.id)),
                        _ => false,
                    });
                    match existing {
                        Some(i) => {
                            if let Row::Block { block, .. } = &mut self.rows[i] {
                                *block = Arc::new(Block::Canvas(json));
                            }
                            self.touch(i);
                        }
                        None => {
                            let msg = self.messages.len();
                            self.messages.push(String::new());
                            self.push(Row::Block { block: Arc::new(Block::Canvas(json)), msg });
                        }
                    }
                    if live {
                        fx.canvas = Some(spec);
                    }
                }
            }
            // Generative UI from Dove's `show_ui` tool: a card, not a step.
            AgentEvent::ToolCall { name, input, .. } if is_show_ui(&name) => {
                self.close_all();
                if let Some(spec) = crate::widget::Spec::from_value(input.clone()) {
                    // Same id as an earlier card: update that card in place
                    // (live dashboards, progress), instead of stacking copies.
                    let existing = (!spec.id.is_empty())
                        .then(|| {
                            self.rows.iter().rposition(|r| match r {
                                Row::Block { block, .. } => matches!(&**block, Block::Widget(src)
                                    if crate::widget::Spec::parse(src).is_some_and(|s| s.id == spec.id)),
                                _ => false,
                            })
                        })
                        .flatten();
                    match existing {
                        Some(i) => {
                            if let Row::Block { block, .. } = &mut self.rows[i] {
                                *block = Arc::new(Block::Widget(input.to_string()));
                            }
                            self.touch(i);
                        }
                        None => {
                            let msg = self.messages.len();
                            self.messages.push(String::new());
                            self.push(Row::Block { block: Arc::new(Block::Widget(input.to_string())), msg });
                        }
                    }
                }
            }
            AgentEvent::ToolCall { call_id, name, kind, title, input } => {
                let i = self.activity();
                self.close_thought();
                if let Row::Activity(a) = &mut self.rows[i] {
                    if a.thought.is_none() && !a.thinking.is_empty() {
                        a.thought = a.started.map(|s| s.elapsed());
                    }
                    if kind == ToolKind::Agent {
                        self.running_agents.push(title.to_string().into());
                    }
                    self.tools.insert(call_id.clone(), (i, a.tools.len()));
                    a.tools.push(Tool {
                        id: call_id,
                        title: title.to_string().into(),
                        kind,
                        output: None,
                        error: false,
                        diff: diff_from_input(&title, &input).map(Arc::new),
                        open: None,
                        full: false,
                        name: name.into(),
                        input: Arc::new(input),
                        started: live.then(Instant::now),
                        took: None,
                    });
                }
                self.touch(i);
            }
            AgentEvent::ToolTitle { call_id, title } => {
                if let Some(&(i, j)) = self.tools.get(&call_id) {
                    if let Some(Row::Activity(a)) = self.rows.get_mut(i) {
                        a.tools[j].title = title.to_string().into();
                    }
                    self.touch(i);
                }
            }
            AgentEvent::ToolDiff { call_id, path, old_text, new_text } => {
                if let Some(&(i, j)) = self.tools.get(&call_id) {
                    if let Some(Row::Activity(a)) = self.rows.get_mut(i) {
                        let old = old_text.as_deref().unwrap_or("");
                        a.tools[j].diff = Some(Arc::new(MiniDiff::new(&path, old, &new_text)));
                    }
                    self.touch(i);
                }
            }
            AgentEvent::ToolResult { call_id, output, is_error } => {
                if let Some(&(i, j)) = self.tools.get(&call_id) {
                    if let Some(Row::Activity(a)) = self.rows.get_mut(i) {
                        let t = &mut a.tools[j];
                        if t.kind == ToolKind::Agent {
                            let title = t.title.clone();
                            if let Some(p) = self.running_agents.iter().position(|x| *x == title) {
                                self.running_agents.remove(p);
                            }
                        }
                        t.output = Some(output.to_string().into());
                        t.error = is_error;
                        t.took = t.started.map(|s| s.elapsed());
                    }
                    self.touch(i);
                }
            }
            AgentEvent::PermissionRequested(req) => {
                self.close_all();
                let decided = if live { None } else { Some(PermissionDecision::Deny) };
                self.push(Row::Permission { req, decided });
            }
            AgentEvent::PermissionResolved { request_id, decision } => {
                if let Some(i) = self
                    .rows
                    .iter()
                    .rposition(|r| matches!(r, Row::Permission { req, .. } if req.request_id == request_id))
                {
                    if let Row::Permission { decided, .. } = &mut self.rows[i] {
                        *decided = Some(decision);
                    }
                    self.touch(i);
                }
            }
            AgentEvent::Plan { text } => {
                self.close_all();
                self.push(Row::Plan(markdown::parse(&text)));
            }
            AgentEvent::Todos { items } => self.todos = items,
            AgentEvent::Status { text } => self.status_text = Some(text.to_string().into()),
            AgentEvent::ContextUsage { tokens, window } => {
                // Missing fields keep the previous measurement.
                if tokens.is_some() {
                    self.context.0 = tokens;
                }
                if window.is_some() {
                    self.context.1 = window;
                }
            }
            AgentEvent::RateLimits { windows } => {
                for w in windows {
                    match self.rate_limits.iter_mut().find(|x| x.kind == w.kind) {
                        Some(x) => *x = w,
                        None => self.rate_limits.push(w),
                    }
                }
            }
            AgentEvent::Commands { .. } => {}
            AgentEvent::Notice { text } => {
                self.close_text();
                self.push(Row::Notice(text.to_string().into()));
            }
            AgentEvent::Error { message } => {
                self.close_all();
                self.push(Row::Error(message.to_string().into()));
            }
            AgentEvent::TurnEnded { usage, is_error } => {
                self.close_all();
                self.status_text = None;
                self.running_agents.clear();
                if live && !is_error {
                    let raw = self.messages.get(self.turn_first_msg..).map(|m| m.join("\n\n")).unwrap_or_default();
                    let spoken = crate::jarvis::for_speech(&raw);
                    if !spoken.is_empty() {
                        fx.speak = Some(spoken);
                    }
                }
                if live {
                    self.usage += usage.unwrap_or_default();
                }
                let msgs = self.turn_first_msg..self.messages.len();
                let elapsed = self.turn_started.take().map(|s| s.elapsed());
                self.push(Row::TurnEnd { msgs, prompt: self.turn_prompt.clone(), elapsed, failed: is_error });
                self.turn_first_msg = self.messages.len();
            }
            AgentEvent::Exited { .. } => {
                self.close_all();
                self.turn_started = None;
            }
            AgentEvent::Subagent { parent, event } => self.subs.entry(parent).or_default().push(*event),
        }
        fx
    }

    /// The pending question or approval, if the agent is waiting on one.
    pub fn pending(&self) -> Option<(usize, &PermissionRequest)> {
        self.rows.iter().enumerate().rev().find_map(|(i, r)| match r {
            Row::Permission { req, decided: None } => Some((i, req)),
            _ => None,
        })
    }

    /// Text of each user message, for the session rail.
    pub fn user_messages(&self) -> Vec<(usize, SharedString)> {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| match r {
                Row::User { text, .. } => Some((i, text.clone())),
                _ => None,
            })
            .collect()
    }

    /// The step the agent is on right now, for the status strip.
    pub fn current_step(&self) -> Option<SharedString> {
        if let Some(s) = &self.status_text {
            return Some(s.clone());
        }
        let i = self.open_activity?;
        let Row::Activity(a) = &self.rows[i] else { return None };
        match a.tools.iter().rev().find(|t| t.output.is_none()) {
            Some(t) => Some(step_label(t).into()),
            None if !a.thinking.is_empty() => Some("Thinking".into()),
            None => None,
        }
    }
}

/// A running subagent's latest step, read off its event log.
fn sub_doing(events: &[AgentEvent]) -> Option<String> {
    events.iter().rev().find_map(|e| match e {
        AgentEvent::ToolCall { title, .. } => Some(title.to_string()),
        AgentEvent::Status { text } => Some(text.to_string()),
        AgentEvent::ThinkingDelta { .. } => Some("Thinking".into()),
        AgentEvent::TextDelta { .. } => Some("Writing".into()),
        _ => None,
    })
}

fn step_label(t: &Tool) -> String {
    let verb = match t.kind {
        ToolKind::Execute => "Running",
        ToolKind::Edit => "Editing",
        ToolKind::Read => "Reading",
        ToolKind::Search => "Searching",
        ToolKind::Fetch => "Fetching",
        ToolKind::Agent => "Subagent",
        _ => "Calling",
    };
    format!("{verb} {}", t.title)
}

/// Edit tools carry old/new text in their input (see dove-harness).
fn diff_from_input(title: &str, input: &serde_json::Value) -> Option<MiniDiff> {
    let s = |k: &str| input.get(k).and_then(|v| v.as_str());
    let path = s("path").or_else(|| s("file_path")).unwrap_or(title);
    match (s("old_string"), s("new_string"), s("content")) {
        (Some(old), Some(new), _) => Some(MiniDiff::new(path, old, new)),
        (None, Some(new), _) => Some(MiniDiff::new(path, "", new)),
        (_, None, Some(content)) => Some(MiniDiff::new(path, "", content)),
        _ => None,
    }
}

/// Zeron's working words; one turns in every [`FLAVOUR_ROTATE_SECS`].
const FLAVOUR_WORDS: [&str; 21] = [
    "Cooing",
    "Thinking",
    "Pondering",
    "Scheming",
    "Brewing",
    "Weaving",
    "Tinkering",
    "Musing",
    "Composing",
    "Sifting",
    "Untangling",
    "Distilling",
    "Sketching",
    "Plotting",
    "Riffing",
    "Combobulating",
    "Percolating",
    "Marinating",
    "Noodling",
    "Puzzling",
    "Conjuring",
];
const FLAVOUR_ROTATE_SECS: u64 = 7;

/// Compact elapsed time, at most two units: "8s", "2m 5s", "1h 4m".
fn format_elapsed(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3_600 => format!("{}m {}s", secs / 60, secs % 60),
        3_600..86_400 => format!("{}h {}m", secs / 3_600, (secs % 3_600) / 60),
        _ => format!("{}d {}h", secs / 86_400, (secs % 86_400) / 3_600),
    }
}

pub fn duration(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    }
}

/// Words that make a command worth a second look.
pub fn risk_hints(req: &PermissionRequest, root: Option<&std::path::Path>) -> Vec<&'static str> {
    let text = req.summary.to_ascii_lowercase();
    let mut out = Vec::new();
    let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
    if has(&["rm ", "rm -", "rmdir", "del ", "remove-item", "git clean", "git reset --hard", "drop table"]) {
        out.push("Deletes files");
    }
    if has(&["curl ", "wget ", "invoke-webrequest", "http://", "https://", "npm publish", "git push"]) {
        out.push("Network");
    }
    if has(&["sudo ", "chmod ", "chown ", "reg add", "set-executionpolicy"]) {
        out.push("System change");
    }
    if let Some(root) = root {
        let root = root.to_string_lossy().to_ascii_lowercase().replace('\\', "/");
        let outside = req.summary.split_whitespace().any(|w| {
            let w = w.trim_matches(|c| c == '"' || c == '\'').to_ascii_lowercase().replace('\\', "/");
            let absolute = w.starts_with('/') || w.get(1..3) == Some(":/");
            absolute && !w.starts_with(&root) && !w.starts_with("/dev/null")
        });
        if outside {
            out.push("Outside the worktree");
        }
    }
    out
}

// ------------------------------------------------------------------ view

/// The question panel's state while the agent waits on answers.
struct Questions {
    request_id: String,
    page: usize,
    answers: Vec<Vec<String>>,
    other: Entity<InputState>,
}

/// What a subagent view follows: one spawn in a parent thread's chat.
struct Mirror {
    source: WeakEntity<TaskView>,
    call: String,
    /// Events of the subagent's log already applied.
    applied: usize,
    /// The opening brief was already shown. The parent repaints while the
    /// subagent works, and the brief is not itself an event.
    briefed: bool,
    /// The spawn finished and the rows were closed.
    settled: bool,
    pub info: Option<Subagent>,
    _observe: Subscription,
}

pub struct TaskView {
    engine: Arc<Engine>,
    pub task: Task,
    pub project: Project,
    live: Option<LiveSession>,
    chat: Chat,
    loading: bool,
    list: ListState,
    following: std::rc::Rc<std::cell::Cell<bool>>,
    /// Hovered rail mark.
    rail_hover: Option<usize>,
    /// The pointer is over the conversation (shows the scroll thumb).
    chat_hovered: bool,
    /// A user message just jumped to, briefly highlighted.
    flash: Option<usize>,
    glide: Option<gpui::Task<()>>,
    pub composer: Entity<Composer>,
    /// Height of the composer dock floating over the transcript's bottom
    /// edge, measured each frame; the list pads itself by it so the last
    /// message can scroll clear of the glass.
    dock_height: std::rc::Rc<std::cell::Cell<f32>>,
    /// The teammate this thread runs as, for its replies' header.
    persona: Option<dove_core::store::Persona>,
    pub status: TaskStatus,
    widgets: HashMap<String, crate::widget::State>,
    todos_open: bool,
    todos_hidden: bool,
    questions: Option<Questions>,
    /// The agent's TUI owns the session; the chat is read-only.
    pub terminal_banner: bool,
    /// Set when this view shows one subagent's transcript (read-only).
    mirror: Option<Mirror>,
    /// Fold tweens and measured heights of the tool groups on screen.
    folds: crate::tool_calls::Folds,
    focus: FocusHandle,
    _ticker: Option<gpui::Task<()>>,
    _pump: Option<gpui::Task<()>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<TaskViewEvent> for TaskView {}

impl Focusable for TaskView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TaskView {
    pub fn new(
        engine: Arc<Engine>,
        task: Task,
        project: Project,
        live: Option<LiveSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::build(engine, task, project, window, cx);
        this.load_transcript(live, cx);
        this
    }

    /// A read-only view of the subagent spawned by `call` in `source`'s
    /// chat: its brief as the opening message, then its own steps and
    /// replies, following along while it runs.
    pub fn subagent(source: &Entity<TaskView>, call: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (engine, task, project) = {
            let s = source.read(cx);
            (s.engine.clone(), s.task.clone(), s.project.clone())
        };
        let mut this = Self::build(engine, task, project, window, cx);
        this.loading = false;
        this.chat = Chat::default();
        let observe = cx.observe(source, |this: &mut Self, _, cx| this.sync_mirror(cx));
        this.mirror = Some(Mirror {
            source: source.downgrade(),
            call,
            applied: 0,
            briefed: false,
            settled: false,
            info: None,
            _observe: observe,
        });
        this.sync_mirror(cx);
        this
    }

    /// The spawn this view follows, when it is a subagent view.
    pub fn mirrored(&self) -> Option<(&str, Option<&Subagent>)> {
        self.mirror.as_ref().map(|m| (m.call.as_str(), m.info.as_ref()))
    }

    /// Pull the subagent's new events from the parent chat.
    fn sync_mirror(&mut self, cx: &mut Context<Self>) {
        let Some(m) = &self.mirror else { return };
        let Some(source) = m.source.upgrade() else { return };
        let (info, fresh) = {
            let src = source.read(cx);
            let info = src.subagents().into_iter().find(|s| s.id == m.call);
            let log = src.chat.subs.get(&m.call).map(Vec::as_slice).unwrap_or(&[]);
            (info, log[m.applied.min(log.len())..].to_vec())
        };
        let Some(m) = self.mirror.as_mut() else { return };
        let running = info.as_ref().is_some_and(|s| s.running);
        let brief = info.as_ref().map(|s| s.prompt.clone()).filter(|p| !p.trim().is_empty());
        let mut events = Vec::with_capacity(fresh.len() + 2);
        // Drivers that do not echo the brief still open on it, once.
        if !m.briefed
            && m.applied == 0
            && !matches!(fresh.first(), Some(AgentEvent::UserMessage { .. }))
            && let Some(brief) = brief.clone()
        {
            m.briefed = true;
            events.push(AgentEvent::UserMessage { text: brief.into() });
        }
        m.applied += fresh.len();
        // A driver that later echoes the same brief would show it twice.
        let mut fresh = fresh;
        if m.briefed
            && let Some(brief) = brief.as_deref().map(str::trim).filter(|b| !b.is_empty())
        {
            let shown = self.chat.rows.iter().any(|r| matches!(r, Row::User { text, .. } if text.trim() == brief));
            if shown
                && let Some(AgentEvent::UserMessage { text }) = fresh.first()
                && text.trim() == brief
            {
                fresh.remove(0);
            }
        }
        events.extend(fresh);
        let finished = info.as_ref().is_some_and(|s| !s.running);
        if finished && !m.settled {
            m.settled = true;
            events.push(AgentEvent::Exited { code: None });
        }
        let changed = m.info != info;
        m.info = info;
        if events.is_empty() {
            if changed {
                cx.notify();
            }
            return;
        }
        let before = self.chat.rows.len();
        for ev in events {
            self.chat.apply(ev, running);
        }
        let after = self.chat.rows.len();
        let from = self.chat.dirty.take().unwrap_or(before).min(before);
        if from < before || after > before {
            self.list.splice(from..before, after - from);
        }
        self.highlight_pending(cx);
        cx.emit(TaskViewEvent::Changed);
        cx.notify();
    }

    fn build(engine: Arc<Engine>, task: Task, project: Project, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| Composer::for_task(engine.clone(), &task, &project, window, cx));
        let sub = cx.subscribe_in(&composer, window, |this, _, ev: &ComposerEvent, window, cx| match ev {
            ComposerEvent::Submit(text) => this.submit(text.clone(), window, cx),
            ComposerEvent::Stop => this.interrupt(cx),
            _ => {}
        });
        let list = ListState::new(0, ListAlignment::Top, px(320.0));
        list.set_follow_mode(FollowMode::Tail);
        let following = std::rc::Rc::new(std::cell::Cell::new(true));
        let weak = cx.weak_entity();
        let flag = following.clone();
        let top_row = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let top = top_row;
        list.set_scroll_handler(move |ev, _, cx| {
            let follow_changed = flag.get() != ev.is_following_tail;
            let top_changed = top.get() != ev.visible_range.start;
            flag.set(ev.is_following_tail);
            top.set(ev.visible_range.start);
            if follow_changed || top_changed {
                let _ = weak.update(cx, |_, cx| cx.notify());
            }
        });
        let chat = Chat {
            usage: engine.store.task_usage(task.id).unwrap_or_default(),
            rate_limits: engine.rate_limits(task.harness),
            ..Default::default()
        };
        let engine_persona = engine
            .store
            .persona_assignments()
            .unwrap_or_default()
            .into_iter()
            .find(|(t, _)| *t == task.id)
            .and_then(|(_, pid)| engine.store.personas().ok()?.into_iter().find(|p| p.id == pid));
        Self {
            engine,
            status: task.status,
            task,
            project,
            live: None,
            chat,
            loading: true,
            list,
            following,
            rail_hover: None,
            chat_hovered: false,
            flash: None,
            glide: None,
            composer,
            dock_height: std::rc::Rc::new(std::cell::Cell::new(DOCK_GUESS)),
            persona: engine_persona.clone(),
            widgets: HashMap::new(),
            todos_open: true,
            todos_hidden: false,
            questions: None,
            terminal_banner: false,
            mirror: None,
            folds: Default::default(),
            focus: cx.focus_handle(),
            _ticker: None,
            _pump: None,
            _subs: vec![sub],
        }
    }

    pub fn usage(&self) -> Usage {
        self.chat.usage
    }

    pub fn todos(&self) -> &[TodoItem] {
        &self.chat.todos
    }

    pub fn context(&self) -> (Option<u64>, Option<u64>) {
        self.chat.context
    }

    /// Every design this thread made, newest first (one per id).
    pub fn designs(&self) -> Vec<crate::canvas::Spec> {
        let mut out: Vec<crate::canvas::Spec> = Vec::new();
        for r in self.chat.rows.iter().rev() {
            if let Row::Block { block, .. } = r
                && let Block::Canvas(src) = &**block
                && let Some(spec) = crate::canvas::Spec::parse(src)
                && !out.iter().any(|s| s.id == spec.id)
            {
                out.push(spec);
            }
        }
        out
    }

    /// Every subagent this thread started, oldest first, with where it sits
    /// in the transcript.
    pub fn subagents(&self) -> Vec<Subagent> {
        let mut out = Vec::new();
        for (row, r) in self.chat.rows.iter().enumerate() {
            let Row::Activity(a) = r else { continue };
            for tool in a.tools.iter().filter(|t| t.kind == ToolKind::Agent) {
                let input = tool.input.as_ref();
                let prompt = str_field(input, &["prompt", "task"]).unwrap_or_default();
                let title = str_field(input, &["description", "subagent_type", "name"])
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| tool.title.to_string());
                out.push(Subagent {
                    title,
                    kind: str_field(input, &["subagent_type", "agent"]).unwrap_or_default(),
                    prompt,
                    output: tool.output.as_ref().map(|o| o.to_string()),
                    error: tool.error,
                    running: tool.output.is_none() && a.live,
                    took: tool.took,
                    row,
                    id: tool.id.clone(),
                    steps: self
                        .chat
                        .subs
                        .get(&tool.id)
                        .map_or(0, |evs| evs.iter().filter(|e| matches!(e, AgentEvent::ToolCall { .. })).count()),
                    doing: (tool.output.is_none() && a.live)
                        .then(|| self.chat.subs.get(&tool.id).and_then(|evs| sub_doing(evs)))
                        .flatten(),
                });
            }
        }
        out
    }

    pub fn is_working(&self) -> bool {
        self.live.is_some() && self.status == TaskStatus::Running
    }

    /// The pending approval or question summary, for "Needs you".
    pub fn pending(&self) -> Option<PermissionRequest> {
        self.chat.pending().map(|(_, r)| r.clone())
    }

    pub fn current_step(&self) -> Option<SharedString> {
        self.chat.current_step()
    }

    /// Detach from the live session (the engine keeps recording). Used when
    /// the terminal takes over the session.
    pub fn take_live(&mut self, cx: &mut Context<Self>) -> Option<LiveSession> {
        self._pump = None;
        let live = self.live.take();
        self.composer.update(cx, |c, cx| c.set_running(false, cx));
        cx.notify();
        live
    }

    /// Replay the saved transcript off-thread, then attach the live session.
    fn load_transcript(&mut self, live: Option<LiveSession>, cx: &mut Context<Self>) {
        let store = self.engine.store.clone();
        let id = self.task.id;
        let build = cx.background_spawn(async move { Chat::replay(store.read_transcript(id)) });
        cx.spawn(async move |this, cx| {
            let mut chat = build.await;
            let _ = this.update(cx, |v, cx| {
                chat.usage = v.chat.usage;
                if chat.rate_limits.is_empty() {
                    chat.rate_limits = std::mem::take(&mut v.chat.rate_limits);
                }
                // `DOVE_EXPAND_TOOLS=1`: every tool group and chip opens;
                // `=group`: groups only (QA).
                if let Some(how) = std::env::var_os("DOVE_EXPAND_TOOLS") {
                    let chips = how != "group";
                    for r in &mut chat.rows {
                        if let Row::Activity(a) = r {
                            a.pinned = Some(true);
                            if chips {
                                a.thoughts.iter_mut().for_each(|th| th.open = Some(true));
                                a.tools.iter_mut().for_each(|t| t.open = Some(true));
                            }
                        }
                    }
                }
                v.chat = chat;
                v.folds.clear();
                v.loading = false;
                v.list.reset(v.chat.rows.len());
                v.list.set_follow_mode(FollowMode::Tail);
                v.sync_composer(cx);
                if let Some(l) = live {
                    v.attach(l, cx);
                }
                cx.emit(TaskViewEvent::Changed);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn attach(&mut self, live: LiveSession, cx: &mut Context<Self>) {
        let rx = live.events.clone();
        self.live = Some(live);
        self.composer.update(cx, |c, cx| c.set_running(true, cx));
        self.start_ticker(cx);
        self._pump = Some(cx.spawn(async move |this, cx| {
            while let Ok(first) = rx.recv_async().await {
                cx.background_executor().timer(FRAME).await;
                let mut batch = vec![first];
                batch.extend(rx.try_iter());
                let exited = batch.iter().any(|e| matches!(e, AgentEvent::Exited { .. }));
                if this.update(cx, |v, cx| v.apply_batch(batch, cx)).is_err() {
                    break;
                }
                if exited {
                    let _ = this.update(cx, |v, cx| {
                        v.live = None;
                        v.composer.update(cx, |c, cx| c.set_running(false, cx));
                        cx.notify();
                    });
                    break;
                }
            }
        }));
    }

    /// Re-render once a second while the agent works, for the elapsed clock.
    fn start_ticker(&mut self, cx: &mut Context<Self>) {
        self._ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let alive = this
                    .update(cx, |v, cx| {
                        if v.is_working() {
                            cx.notify();
                        }
                        v.live.is_some()
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        }));
    }

    pub(crate) fn apply_batch(&mut self, events: Vec<AgentEvent>, cx: &mut Context<Self>) {
        if let Some(remote) = crate::remote::Remote::global(cx) {
            let task = self.task.id;
            remote.update(cx, |r, _| r.observe(task, &events));
        }
        let before = self.chat.rows.len();
        let mut status = None;
        let mut runway = None;
        // Adjacent text deltas parse once.
        let mut merged: Vec<AgentEvent> = Vec::with_capacity(events.len());
        for ev in events {
            if let (AgentEvent::TextDelta { text: next }, Some(AgentEvent::TextDelta { text: prev })) =
                (&ev, merged.last_mut())
            {
                *prev = format!("{prev}{next}").into();
                continue;
            }
            merged.push(ev);
        }
        for ev in merged {
            if let AgentEvent::ToolCall { name, input, .. } = &ev {
                // Dove's own browser tools already act on the page.
                if dove_core::is_dove_tool(name) {
                } else if let Some(d) = crate::browser::Drive::from_tool(name, input) {
                    cx.emit(TaskViewEvent::DriveBrowser(d));
                }
            }
            let fx = self.chat.apply(ev, true);
            if let Some(s) = fx.status {
                status = Some(s);
            }
            if let Some(id) = fx.native {
                self.task.native_session_id = Some(id.clone());
                cx.emit(TaskViewEvent::Native(id));
            }
            if fx.runway.is_some() {
                runway = fx.runway;
            }
            if fx.first_answer
                && let Some(t) = self.auto_title()
            {
                cx.emit(TaskViewEvent::Title(t));
            }
            if let Some(spec) = fx.canvas {
                cx.emit(TaskViewEvent::OpenCanvas(spec, false));
            }
            if let Some(text) = fx.speak
                && self.mirror.is_none()
            {
                crate::jarvis::maybe_speak(&self.engine, text);
            }
        }
        let after = self.chat.rows.len();
        let from = self.chat.dirty.take().unwrap_or(before).min(before);
        if from < before || after > before {
            self.list.splice(from..before, after - from);
        }
        if let Some(row) = runway {
            // The sent message glides to the top; the answer fills below it.
            self.list.set_tail_reservation(Some((row, px(24.0))));
            self.list.scroll_to(ListOffset { item_ix: row, offset_in_item: px(0.0) });
        }
        self.highlight_pending(cx);
        if let Some(s) = status
            && s != self.status
        {
            self.status = s;
            let running = s == TaskStatus::Running;
            self.composer.update(cx, |c, cx| c.set_running(running && self.live.is_some(), cx));
            cx.emit(TaskViewEvent::Status(s));
        }
        // A queued message goes out as soon as the turn ends.
        if matches!(status, Some(TaskStatus::Done | TaskStatus::Failed))
            && let Some(next) = self.composer.update(cx, |c, cx| c.take_queued(cx))
            && let Some(l) = &self.live
        {
            l.send(Command::Send(next));
        }
        self.sync_questions(cx);
        self.sync_composer(cx);
        cx.emit(TaskViewEvent::Changed);
        cx.notify();
    }

    /// The chat shows a banner while the agent's terminal UI drives the
    /// session (Chat ⇄ Terminal).
    pub fn set_terminal_banner(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.terminal_banner != on {
            self.terminal_banner = on;
            cx.notify();
        }
    }

    /// Push meters and run state into the composer.
    fn sync_composer(&mut self, cx: &mut Context<Self>) {
        let context = self.chat.context;
        let limits = self.chat.rate_limits.clone();
        self.composer.update(cx, |c, cx| c.set_meters(context, limits, cx));
    }

    /// A short title from the first prompt, used until the user renames.
    fn auto_title(&self) -> Option<String> {
        let prompt = self.chat.user_messages().into_iter().next()?.1;
        let t = ui::auto_title(&prompt);
        (t != self.task.title).then_some(t)
    }

    /// Highlight finished code blocks off the UI thread.
    fn highlight_pending(&mut self, cx: &mut Context<Self>) {
        let jobs: Vec<(usize, Arc<Block>)> = std::mem::take(&mut self.chat.to_highlight)
            .into_iter()
            .filter_map(|i| match self.chat.rows.get(i) {
                Some(Row::Block { block, .. }) => Some((i, block.clone())),
                _ => None,
            })
            .collect();
        if jobs.is_empty() {
            return;
        }
        let work = cx.background_spawn(async move {
            jobs.into_iter()
                .map(|(i, b)| (i, b.clone(), Arc::new(markdown::highlight_block((*b).clone()))))
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let done = work.await;
            let _ = this.update(cx, |v, cx| {
                for (i, old, new) in done {
                    // Only if the row still holds the block we highlighted.
                    if let Some(Row::Block { block, .. }) = v.chat.rows.get_mut(i)
                        && Arc::ptr_eq(block, &old)
                    {
                        *block = new;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn submit(&mut self, text: String, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(live) = &self.live {
            live.send(Command::Send(text));
            return;
        }
        let run = self.composer.read(cx).run_settings(cx);
        self.task.model = run.model.clone().or(self.task.model.clone());
        match self.engine.resume_task(&self.task, &self.project, Some(text), run) {
            Ok(live) => {
                self.attach(live, cx);
                self.status = TaskStatus::Running;
                cx.emit(TaskViewEvent::Status(TaskStatus::Running));
            }
            Err(e) => {
                let at = self.chat.rows.len();
                self.chat.rows.push(Row::Error(format!("{e:#}").into()));
                self.list.splice(at..at, 1);
            }
        }
        cx.notify();
    }

    pub fn interrupt(&mut self, cx: &mut Context<Self>) {
        if let Some(l) = &self.live {
            l.send(Command::Interrupt);
        }
        cx.notify();
    }

    /// Answer an approval from outside the chat (the sidebar's Needs you).
    pub fn decide_from_outside(&mut self, request_id: String, decision: PermissionDecision, cx: &mut Context<Self>) {
        self.decide(request_id, decision, cx);
    }

    fn decide(&mut self, request_id: String, decision: PermissionDecision, cx: &mut Context<Self>) {
        if let Some(l) = &self.live {
            l.send(Command::Permission { request_id, decision });
        }
        cx.notify();
    }

    fn toggle_activity(&mut self, i: usize, cx: &mut Context<Self>) {
        if let Some(Row::Activity(a)) = self.chat.rows.get_mut(i) {
            // Fold whatever is on screen: groups open while they work (and
            // always in a subagent chat), so the default is not "closed".
            let open = a.pinned.unwrap_or(a.live || self.mirror.is_some());
            a.pinned = Some(!open);
            self.list.remeasure_items(i..i + 1);
            cx.notify();
        }
    }

    fn toggle_thought(&mut self, i: usize, k: usize, cx: &mut Context<Self>) {
        if let Some(Row::Activity(a)) = self.chat.rows.get_mut(i)
            && let Some(th) = a.thoughts.get_mut(k)
        {
            th.open = Some(!th.open.unwrap_or(th.live));
            cx.notify();
        }
    }

    /// A chip's "N more lines": its whole output, or back to the first screen.
    fn toggle_full(&mut self, i: usize, j: usize, cx: &mut Context<Self>) {
        if let Some(Row::Activity(a)) = self.chat.rows.get_mut(i)
            && let Some(t) = a.tools.get_mut(j)
        {
            t.full = !t.full;
            cx.notify();
        }
    }

    /// A working group's "N earlier steps": pin the whole tree open.
    fn show_all_steps(&mut self, i: usize, cx: &mut Context<Self>) {
        if let Some(Row::Activity(a)) = self.chat.rows.get_mut(i) {
            a.pinned = Some(true);
            cx.notify();
        }
    }

    fn toggle_tool(&mut self, i: usize, j: usize, cx: &mut Context<Self>) {
        if let Some(Row::Activity(a)) = self.chat.rows.get_mut(i) {
            if let Some(t) = a.tools.get_mut(j) {
                t.open = Some(!t.open.unwrap_or_else(|| t.opens_by_default()));
            }
            self.list.remeasure_items(i..i + 1);
            cx.notify();
        }
    }

    /// Glide so row `target` sits near the top (a short eased scroll when
    /// it is measured; a jump plus a settle when it is far off screen),
    /// then flash it.
    pub fn glide_to_row(&mut self, target: usize, cx: &mut Context<Self>) {
        self.list.set_tail_reservation(None);
        self.list.set_follow_mode(FollowMode::Normal);
        self.following.set(false);
        self.flash = Some(target);
        const INSET: f32 = 12.0;
        let measured =
            self.list.bounds_for_item(target).map(|b| f32::from(b.top() - self.list.viewport_bounds().top()) - INSET);
        let Some(total) = measured.filter(|d| d.abs() < 4000.0) else {
            self.list.scroll_to(ListOffset { item_ix: target, offset_in_item: px(0.0) });
            self.list.scroll_by(px(-INSET));
            cx.notify();
            self.glide = Some(self.clear_flash_later(cx));
            return;
        };
        self.glide = Some(cx.spawn(async move |this, cx| {
            let frames = 18;
            let mut done = 0.0f32;
            for n in 1..=frames {
                cx.background_executor().timer(Duration::from_millis(14)).await;
                let p = n as f32 / frames as f32;
                // Ease-out cubic.
                let eased = 1.0 - (1.0 - p).powi(3);
                let step = total * eased - done;
                done += step;
                if this
                    .update(cx, |v, cx| {
                        v.list.scroll_by(px(step));
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
            cx.background_executor().timer(Duration::from_millis(900)).await;
            let _ = this.update(cx, |v, cx| {
                v.flash = None;
                cx.notify();
            });
        }));
    }

    fn clear_flash_later(&self, cx: &mut Context<Self>) -> gpui::Task<()> {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(1100)).await;
            let _ = this.update(cx, |v, cx| {
                v.flash = None;
                cx.notify();
            });
        })
    }

    /// Jump to the previous (`-1`) or next (`+1`) prompt from the one being read.
    pub fn step_prompt(&mut self, dir: i32, cx: &mut Context<Self>) {
        let rows: Vec<usize> = self.chat.user_messages().into_iter().map(|(r, _)| r).collect();
        if rows.is_empty() {
            return;
        }
        let top = self.list.logical_scroll_top().item_ix;
        let target = if dir < 0 {
            rows.iter().rev().find(|&&r| r < top).or(rows.first())
        } else {
            rows.iter().find(|&&r| r > top)
        };
        match target {
            Some(&r) => self.glide_to_row(r, cx),
            None if dir > 0 => self.jump_to_latest(cx),
            None => {}
        }
    }

    pub fn scroll_page(&mut self, dir: f32, cx: &mut Context<Self>) {
        let h = f32::from(self.list.viewport_bounds().size.height).max(200.0);
        self.list.set_follow_mode(FollowMode::Normal);
        self.list.scroll_by(px(dir * h * 0.85));
        cx.notify();
    }

    pub fn scroll_to_top(&mut self, cx: &mut Context<Self>) {
        self.list.set_follow_mode(FollowMode::Normal);
        self.list.scroll_to(ListOffset { item_ix: 0, offset_in_item: px(0.0) });
        cx.notify();
    }

    pub fn jump_to_latest(&mut self, cx: &mut Context<Self>) {
        self.list.set_tail_reservation(None);
        self.list.set_follow_mode(FollowMode::Tail);
        self.following.set(true);
        cx.notify();
    }

    fn retry(&mut self, prompt: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_working() {
            return;
        }
        self.submit(prompt.to_string(), window, cx);
    }

    fn edit_message(&mut self, text: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        composer::set_text(&self.composer, &text, window, cx);
        composer::focus(&self.composer, window, cx);
    }

    // ------------------------------------------------------------ questions

    /// Drop a question panel whose request is gone or replaced; render opens
    /// the panel for a new one (building its input needs the window).
    fn sync_questions(&mut self, cx: &mut Context<Self>) {
        let pending = self.chat.pending().filter(|(_, r)| r.is_question()).map(|(_, r)| r.request_id.clone());
        if self.questions.as_ref().is_some_and(|q| Some(&q.request_id) != pending.as_ref()) {
            self.questions = None;
            cx.notify();
        }
    }

    fn open_questions(&mut self, req: &PermissionRequest, window: &mut Window, cx: &mut Context<Self>) {
        let theme = Theme::of(cx).clone();
        let other = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Or type your own answer…");
            s.set_editor_style(ui::input_style(&theme));
            s
        });
        let sub = cx.subscribe_in(&other, window, |this, _, ev: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                this.answer_other(window, cx);
            }
        });
        self._subs.push(sub);
        self.questions = Some(Questions {
            request_id: req.request_id.clone(),
            page: 0,
            answers: vec![Vec::new(); req.questions.len()],
            other,
        });
    }

    fn pick_option(&mut self, label: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, req)) = self.chat.pending() else { return };
        let req = req.clone();
        let Some(q) = self.questions.as_mut() else { return };
        let Some(question) = req.questions.get(q.page) else { return };
        let answer = &mut q.answers[q.page];
        if question.multi_select {
            match answer.iter().position(|a| *a == label) {
                Some(p) => {
                    answer.remove(p);
                }
                None => answer.push(label),
            }
            cx.notify();
            return;
        }
        *answer = vec![label];
        self.next_question(window, cx);
    }

    fn answer_other(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(q) = self.questions.as_mut() else { return };
        let text = q.other.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        q.answers[q.page] = vec![text];
        q.other.update(cx, |s, cx| s.set_value("", window, cx));
        self.next_question(window, cx);
    }

    fn next_question(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(q) = self.questions.as_mut() else { return };
        if q.page + 1 < q.answers.len() {
            q.page += 1;
            cx.notify();
            return;
        }
        let (request_id, answers) = (q.request_id.clone(), q.answers.clone());
        if let Some(l) = &self.live {
            l.send(Command::Answer { request_id, answers });
        }
        self.questions = None;
        cx.notify();
    }

    fn skip_questions(&mut self, cx: &mut Context<Self>) {
        let Some(q) = self.questions.take() else { return };
        if let Some(l) = &self.live {
            l.send(Command::Answer { request_id: q.request_id, answers: Vec::new() });
        }
        cx.notify();
    }

    /// A/S/D answer an approval; 1-9 pick a question option. Only while the
    /// composer is empty, so typing is never hijacked.
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        // Reading navigation: Alt+↑/↓ steps between your prompts; PageUp /
        // PageDown scroll; Ctrl+Home / Ctrl+End jump to either end.
        let m = &k.modifiers;
        let nav = match (k.key.as_str(), m.alt, m.control || m.platform, m.shift) {
            ("up", true, false, false) => Some(0),
            ("down", true, false, false) => Some(1),
            ("pageup", false, false, false) => Some(2),
            ("pagedown", false, false, false) => Some(3),
            ("home", false, true, false) => Some(4),
            ("end", false, true, false) => Some(5),
            _ => None,
        };
        if let Some(n) = nav {
            match n {
                0 => self.step_prompt(-1, cx),
                1 => self.step_prompt(1, cx),
                2 => self.scroll_page(-1.0, cx),
                3 => self.scroll_page(1.0, cx),
                4 => self.scroll_to_top(cx),
                _ => self.jump_to_latest(cx),
            }
            cx.stop_propagation();
            return;
        }
        if k.modifiers.control || k.modifiers.alt || k.modifiers.platform || k.modifiers.shift {
            return;
        }
        let Some((_, req)) = self.chat.pending() else { return };
        let req = req.clone();
        if req.is_question() {
            let Some(q) = &self.questions else { return };
            if q.other.read(cx).focus_handle(cx).is_focused(window) {
                return;
            }
            if let Some(n) = k.key.parse::<usize>().ok().filter(|n| (1..=9).contains(n))
                && let Some(label) = req.questions.get(q.page).and_then(|qq| qq.options.get(n - 1)).cloned()
            {
                self.pick_option(label, window, cx);
                cx.stop_propagation();
            }
            return;
        }
        if !composer::is_empty(&self.composer, cx) {
            return;
        }
        let decision = match k.key.as_str() {
            "a" => PermissionDecision::AllowOnce,
            "s" => PermissionDecision::AllowAlways,
            "d" => PermissionDecision::Deny,
            _ => return,
        };
        self.decide(req.request_id, decision, cx);
        cx.stop_propagation();
    }

    // ------------------------------------------------------------ render

    fn render_row(&self, i: usize, weak: &WeakEntity<Self>, window: &Window, cx: &App) -> AnyElement {
        let t = Theme::of(cx);
        let body: AnyElement = match &self.chat.rows[i] {
            Row::User { text, at } => self.render_user(i, text, *at, weak, t),
            Row::Activity(a) => self.render_activity(i, a, weak, t),
            Row::Block { block, msg } => {
                let first = i == 0 || !matches!(&self.chat.rows[i - 1], Row::Block { msg: m, .. } if m == msg);
                // Code blocks carry their own copy button; other blocks get one on hover.
                let copyable =
                    !matches!(&**block, Block::Code { .. } | Block::Rule | Block::Widget(_) | Block::Canvas(_));
                let group = SharedString::from(format!("blk-{i}"));
                div()
                    .group(group.clone())
                    .relative()
                    .when(!first, |d| d.pt(px(2.0)))
                    .child(self.render_md(&format!("b{i}"), block, weak, t, window))
                    .when(copyable, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(px(-2.0))
                                .right(px(-30.0))
                                .invisible()
                                .group_hover(group, |s| s.visible())
                                .child(copy_button(SharedString::from(format!("copy-blk-{i}")), block.plain(), t)),
                        )
                    })
                    .into_any_element()
            }
            Row::Live { blocks, .. } => div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .children(
                    blocks.iter().enumerate().map(|(j, b)| self.render_md(&format!("l{i}-{j}"), b, weak, t, window)),
                )
                .into_any_element(),
            Row::Permission { req, decided } => self.render_permission(i, req, *decided, weak, t),
            Row::Plan(blocks) => div()
                .p(px(14.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(t.border)
                .bg(t.wash(0.025))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(12.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(t.text_muted)
                        .child(icon(icons::CHECKLIST).size(px(14.0)))
                        .child("Plan"),
                )
                .child(markdown::render(SharedString::from(format!("plan-{i}")), blocks, t, window))
                .into_any_element(),
            Row::Notice(text) => div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_size(px(12.0))
                .text_color(t.text_faint)
                .child(icon(icons::INFO_CIRCLE).size(px(13.0)))
                .child(text.clone())
                .into_any_element(),
            Row::Error(text) => div()
                .flex()
                .items_start()
                .gap(px(8.0))
                .p(px(10.0))
                .rounded(px(10.0))
                .bg(t.danger.opacity(0.08))
                .border_1()
                .border_color(t.danger.opacity(0.25))
                .text_color(t.danger)
                .text_size(px(13.0))
                .child(icon(icons::DANGER_TRIANGLE).size(px(15.0)).mt(px(1.0)))
                .child(div().flex_1().min_w_0().child(text.clone()))
                .into_any_element(),
            Row::TurnEnd { msgs, prompt, elapsed, failed } => {
                self.render_turn_end(i, msgs.clone(), prompt.clone(), *elapsed, *failed, weak, t)
            }
        };
        // Each reply opens with who is answering: the teammate, or the agent.
        let opens_reply = i > 0
            && matches!(self.chat.rows[i - 1], Row::User { .. })
            && matches!(self.chat.rows[i], Row::Activity(_) | Row::Block { .. } | Row::Live { .. });
        let body = if opens_reply {
            div().flex().flex_col().gap(px(8.0)).child(self.reply_header(t)).child(body).into_any_element()
        } else {
            body
        };
        // Zeron's working trailer rides under the last row, in the flow of
        // the reply, and scrolls away with it.
        let body = if i + 1 == self.chat.rows.len() && self.is_working() {
            div().flex().flex_col().child(body).child(self.render_working(weak, t)).into_any_element()
        } else {
            body
        };
        let gap = match &self.chat.rows[i] {
            Row::Block { .. } => px(5.0),
            Row::TurnEnd { .. } => px(0.0),
            _ => px(7.0),
        };
        div()
            .w_full()
            .flex()
            .justify_center()
            .px(px(28.0))
            .py(gap)
            .child(div().w_full().max_w(px(COLUMN_MAX)).child(body))
            .into_any_element()
    }

    /// The working loader: the gradient spinner, a flavour word that turns
    /// every 7s and rolls as it turns, and the turn's timer rolling its
    /// digits in place.
    fn render_working(&self, weak: &WeakEntity<Self>, t: &Theme) -> AnyElement {
        let secs = self.chat.turn_started.map_or(0, |s| s.elapsed().as_secs());
        let seed = format!("{:?}", self.task.id)
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
        let word = FLAVOUR_WORDS[(seed.wrapping_add(secs / FLAVOUR_ROTATE_SECS) % FLAVOUR_WORDS.len() as u64) as usize];
        let key = weak.entity_id().as_u64();
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .pt(px(16.0))
            .child(crate::loaders::gradient_spinner(SharedString::from(format!("working-indicator-{key}")), 2.5))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(t.text_muted)
                    .child(crate::roll_text::roll_text(format!("working-word-{key}"), format!("{word}…").into())),
            )
            .child(
                div()
                    .relative()
                    .top(px(1.0))
                    .text_size(px(11.0))
                    .text_color(t.text_faint)
                    .child(crate::roll_text::roll_text(format!("working-timer-{key}"), format_elapsed(secs).into())),
            )
            .into_any_element()
    }

    /// "Ava · Claude Code · opus" over a reply.
    fn reply_header(&self, t: &Theme) -> AnyElement {
        let kind = self.task.harness;
        let model = self.task.model.clone().filter(|m| !m.is_empty());
        let (mark, name, detail): (AnyElement, String, String) = match &self.persona {
            Some(p) => (
                crate::team::avatar(p, 20.0, t).into_any_element(),
                p.name.clone(),
                match &model {
                    Some(m) => format!("{} · {m}", kind.label()),
                    None => kind.label().to_string(),
                },
            ),
            None => (
                div()
                    .size(px(20.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .bg(t.wash(0.06))
                    .child(icon(ui::harness_icon(kind)).size(px(12.0)).text_color(ui::harness_tint(
                        kind,
                        t,
                        t.text_muted,
                    )))
                    .into_any_element(),
                kind.label().to_string(),
                model.unwrap_or_default(),
            ),
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(px(12.5))
            .child(mark)
            .child(div().font_weight(gpui::FontWeight::MEDIUM).text_color(t.text).child(name))
            .when(!detail.is_empty(), |d| d.child(div().text_color(t.text_faint).child(detail)))
            .into_any_element()
    }

    fn render_user(&self, i: usize, text: &SharedString, at: i64, weak: &WeakEntity<Self>, t: &Theme) -> AnyElement {
        let copy = text.to_string();
        let when = if at > 0 { ui::relative_time(at) } else { String::new() };
        let edit = {
            let weak = weak.clone();
            let text = text.clone();
            action_button(SharedString::from(format!("edit-{i}")), icons::PEN, "Edit & resend", t).on_click(
                move |_, window, cx| {
                    let _ = weak.update(cx, |v, cx| v.edit_message(text.clone(), window, cx));
                },
            )
        };
        let sent = self.mirror.is_some() && self.chat.rows[..i].iter().all(|r| !matches!(r, Row::User { .. }));
        div()
            .group("user-msg")
            .w_full()
            .flex()
            .flex_col()
            .items_end()
            .gap(px(2.0))
            .pt(px(10.0))
            .when(sent, |d| {
                d.child(
                    div()
                        .pr(px(4.0))
                        .pb(px(2.0))
                        .text_size(px(11.5))
                        .text_color(t.text_faint)
                        .child("Sent to this subagent"),
                )
            })
            .child(
                div()
                    .max_w(px(580.0))
                    .px(px(14.0))
                    .py(px(9.0))
                    .rounded(px(Theme::BUBBLE_RADIUS))
                    .bg(t.surface_raised)
                    .border_1()
                    .border_color(if self.flash == Some(i) { t.accent.opacity(0.7) } else { gpui::transparent_black() })
                    .text_color(t.text)
                    .child(text.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .h(px(24.0))
                    .invisible()
                    .group_hover("user-msg", |s| s.visible())
                    .text_size(px(12.0))
                    .text_color(t.text_faint)
                    .child(div().mr(px(4.0)).child(when))
                    .when(self.mirror.is_none(), |d| d.child(edit))
                    .child(copy_button(SharedString::from(format!("copy-user-{i}")), copy, t)),
            )
            .into_any_element()
    }

    fn render_md(&self, id: &str, block: &Block, weak: &WeakEntity<Self>, t: &Theme, window: &Window) -> AnyElement {
        if let Block::Canvas(src) = block
            && let Some(spec) = crate::canvas::Spec::parse(src)
        {
            let weak = weak.clone();
            let open = spec.clone();
            return crate::canvas::card(&spec, t, move |_, cx| {
                let _ = weak.update(cx, |_, cx| cx.emit(TaskViewEvent::OpenCanvas(open.clone(), true)));
            });
        }
        // A finished ```html / ```jsx / ```svg block can open as a live design.
        if let Block::Code { lang: Some(lang), text, .. } = block
            && let Some(spec) = crate::canvas::Spec::from_fence(lang, text)
        {
            let weak = weak.clone();
            let code = markdown::render_block(id, block, t, window);
            return div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(code)
                .child(
                    div()
                        .id(SharedString::from(format!("{id}-preview")))
                        .h(px(26.0))
                        .px(px(9.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .self_start()
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(t.border)
                        .text_size(px(12.0))
                        .text_color(t.text_muted)
                        .cursor_pointer()
                        .hover(|s| s.bg(t.wash(0.06)).text_color(t.text))
                        .child(icon(icons::ACTION_PLAY).size(px(12.0)))
                        .child("Preview as canvas")
                        .on_click(move |_, _, cx| {
                            let spec = spec.clone();
                            let _ = weak.update(cx, |_, cx| cx.emit(TaskViewEvent::OpenCanvas(spec, true)));
                        }),
                )
                .into_any_element();
        }
        let Block::Widget(src) = block else {
            return markdown::render_block(id, block, t, window);
        };
        // A card still streaming renders what has arrived, inert until whole.
        let (spec, partial) = match crate::widget::Spec::parse(src) {
            Some(spec) => (spec, false),
            None => match crate::widget::Spec::parse_partial(src) {
                Some(spec) => (spec, true),
                None => return markdown::render_block(id, block, t, window),
            },
        };
        let key = id.to_string();
        let state = match partial {
            true => crate::widget::State::new(spec.clone()),
            false => self.widgets.get(&key).cloned().unwrap_or_else(|| crate::widget::State::new(spec.clone())),
        };
        crate::widget::render(key.clone(), &state, t, weak.clone(), move |this, ev, window, cx| {
            if partial {
                return;
            }
            let entry = this.widgets.entry(key.clone()).or_insert_with(|| crate::widget::State::new(spec.clone()));
            match ev {
                crate::widget::Event::Act { action, message } => {
                    entry.sent = Some(action);
                    this.submit(message, window, cx);
                }
                crate::widget::Event::Set { id, value } => entry.set(&id, &value),
                crate::widget::Event::Edit(prefill) => {
                    composer::set_text(&this.composer, &prefill, window, cx);
                    composer::focus(&this.composer, window, cx);
                }
            }
            cx.notify();
        })
    }

    /// One activity: Zeron's collapsible task group. A quiet summary (with a
    /// light sweeping across it while the agent works) folds a task tree of
    /// thoughts and tool calls; subagent spawn cards stay outside the fold.
    fn render_activity(&self, i: usize, a: &Activity, weak: &WeakEntity<Self>, t: &Theme) -> AnyElement {
        use crate::tool_calls as tc;
        let now = Instant::now();
        let mirror = self.mirror.is_some();
        let agents: Vec<usize> = if mirror {
            Vec::new()
        } else {
            a.tools.iter().enumerate().filter(|(_, t)| t.kind == ToolKind::Agent).map(|(j, _)| j).collect()
        };
        // Thoughts and calls in arrival order.
        let mut items: Vec<Step> = Vec::new();
        let thoughts_at = |at: usize| {
            a.thoughts
                .iter()
                .enumerate()
                .filter(move |(_, th)| th.at == at && !th.text.trim().is_empty())
                .map(|(k, _)| Step::Thought(k))
        };
        for j in 0..a.tools.len() {
            items.extend(thoughts_at(j));
            if mirror || a.tools[j].kind != ToolKind::Agent {
                items.push(Step::Tool(j));
            }
        }
        items.extend(
            a.thoughts
                .iter()
                .enumerate()
                .filter(|(_, th)| th.at >= a.tools.len() && !th.text.trim().is_empty())
                .map(|(k, _)| Step::Thought(k)),
        );
        let chips: Vec<AnyElement> = agents.iter().map(|&j| self.render_subagent_chip(i, j, &a.tools[j], t)).collect();
        // A turn that only spawned helpers is the cards, not a "ran 1 subagent" fold.
        if items.is_empty() {
            return div().flex().flex_col().children(chips).into_any_element();
        }

        let open = a.pinned.unwrap_or(a.live || mirror);
        let frame = self.folds.step((i, tc::PART_GROUP, 0), open, now);
        let mut moving = frame.animating;
        let shimmer = a.live.then(|| tc::shimmer_phase(a.started.unwrap_or(now), now));
        moving |= shimmer.is_some();

        let view = weak.entity_id();
        let header = {
            let weak = weak.clone();
            div()
                .id(("activity", i))
                .h(px(tc::HEADER_HEIGHT))
                .pr(px(4.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .cursor_pointer()
                .text_size(px(tc::LABEL_SIZE))
                .line_height(px(tc::LINE_HEIGHT))
                // Quiet even when steps failed: probes fail mid-work all the
                // time. Failures tint their own rows and count in the summary.
                .text_color(t.text_muted)
                .hover(|s| s.text_color(t.text))
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |v, cx| v.toggle_activity(i, cx));
                })
                .child(tc::chevron(frame.disclosure, t))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .h(px(tc::LINE_HEIGHT))
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .child({
                            // The summary rolls as the group grows ("Ran 2
                            // commands" → "Ran 3 commands"), then shimmers on.
                            let summary: SharedString = a.summary().into();
                            crate::roll_text::rolling(
                                format!("tool-group-{}-{i}", view.as_u64()),
                                summary.clone(),
                            )
                            .unwrap_or_else(|| tc::title(summary, shimmer, t))
                        }),
                )
        };
        // A new group's header grows into place, like its rows.
        let header_reveal = tc::progress(a.started.filter(|_| a.live), now, tc::ROW_REVEAL, tc::EASE_OUT_EXPO);
        moving |= header_reveal < 1.0;
        let header: AnyElement = if header_reveal < 1.0 {
            div()
                .w_full()
                .h(px(tc::HEADER_HEIGHT * header_reveal))
                .flex_none()
                .overflow_hidden()
                .child(header)
                .into_any_element()
        } else {
            header.into_any_element()
        };

        let body = frame.mounted.then(|| {
            // While it works (and the user has not chosen), only the newest
            // steps show; the rest wait behind one row.
            let hidden =
                if a.live && a.pinned.is_none() && !mirror { items.len().saturating_sub(LIVE_STEPS) } else { 0 };
            let shown = &items[hidden..];
            let start_of = |s: &Step| {
                match s {
                    Step::Thought(k) => a.thoughts[*k].started,
                    Step::Tool(j) => a.tools[*j].started,
                }
                .filter(|_| a.live)
            };
            let connectors: Vec<f32> = shown
                .iter()
                .map(|s| tc::progress(start_of(s), now, tc::CONNECTOR_REVEAL, tc::EASE_OUT_QUINT))
                .collect();
            let mut rows: Vec<AnyElement> = Vec::with_capacity(shown.len() + 1);
            if hidden > 0 {
                rows.push(self.render_earlier_steps(i, hidden, weak, t));
            }
            for (n, step) in shown.iter().enumerate() {
                let rail = Rail {
                    after: n > 0 || hidden > 0,
                    continues: n + 1 < shown.len(),
                    connector: connectors[n],
                    continuation: tc::continuation(connectors.get(n + 1).copied()),
                };
                let reveal = tc::progress(start_of(step), now, tc::ROW_REVEAL, tc::EASE_OUT_EXPO);
                moving |= reveal < 1.0 || rail.connector < 1.0;
                let (row, key) = match step {
                    Step::Thought(k) => (
                        self.render_thought(i, *k, &a.thoughts[*k], rail, weak, t, &mut moving),
                        (i, tc::PART_REVEAL_THOUGHT, *k),
                    ),
                    Step::Tool(j) => (
                        self.render_tool(i, *j, &a.tools[*j], a.live, rail, weak, t, &mut moving),
                        (i, tc::PART_REVEAL_TOOL, *j),
                    ),
                };
                // A new arrival grows into place; settled rows lay out freely.
                rows.push(if reveal < 1.0 {
                    let cell = self.folds.cell(key);
                    let h = cell.get() * reveal;
                    div()
                        .w_full()
                        .h(px(h))
                        .flex_none()
                        .overflow_hidden()
                        .child(tc::measured(row, cell))
                        .into_any_element()
                } else {
                    row
                });
            }
            tc::folding(div().w_full().pt(px(2.0)).flex().flex_col().children(rows), &frame)
        });

        div()
            .relative()
            .flex()
            .flex_col()
            .child(header)
            .children(body)
            .when(!chips.is_empty(), |d| d.child(div().mt(px(2.0)).flex().flex_col().children(chips)))
            .when(moving, |d| d.child(tc::frame_pump(weak.entity_id())))
            .into_any_element()
    }

    /// The fold's first row while a long activity works: how many steps
    /// scrolled off the tree. Clicking pins the whole tree open.
    fn render_earlier_steps(&self, i: usize, hidden: usize, weak: &WeakEntity<Self>, t: &Theme) -> AnyElement {
        use crate::tool_calls as tc;
        let weak = weak.clone();
        div()
            .id(("earlier-steps", i))
            .w_full()
            .flex()
            .flex_row()
            .cursor_pointer()
            .text_color(t.text_faint)
            .hover(|s| s.text_color(t.text_muted))
            .on_click(move |_, _, cx| {
                let _ = weak.update(cx, |v, cx| v.show_all_steps(i, cx));
            })
            .child(tc::rail(tc::Glyph::Icon(icons::MORE_HORIZONTAL, t.text_faint), false, true, 1.0, 1.0, t))
            .child(
                div()
                    .ml(px(tc::TEXT_GAP))
                    .h(px(tc::ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .text_size(px(tc::LABEL_SIZE))
                    .child(format!("{hidden} earlier {}", if hidden == 1 { "step" } else { "steps" })),
            )
            .into_any_element()
    }

    /// One branch of the tree: rail gutter, header strip, folding body.
    fn rail_row(
        &self,
        glyph: crate::tool_calls::Glyph,
        rail: Rail,
        header: AnyElement,
        body: Option<AnyElement>,
        t: &Theme,
    ) -> AnyElement {
        use crate::tool_calls as tc;
        let content = tc::connector_parts(rail.connector, rail.after).1;
        div()
            .w_full()
            .flex()
            .flex_row()
            .child(tc::rail(glyph, rail.after, rail.continues, rail.connector, rail.continuation, t))
            .child(
                div()
                    .ml(px(tc::TEXT_GAP))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .when(content < 1.0, |d| d.relative().top(px(4.0 * (1.0 - content))).opacity(content))
                    .child(header)
                    .children(body),
            )
            .into_any_element()
    }

    /// A stretch of reasoning on the tree. It streams open (the live
    /// thinking is the point), then settles shut; its markdown is parsed
    /// incrementally and mended, so it never shows raw markers.
    #[allow(clippy::too_many_arguments)]
    fn render_thought(
        &self,
        i: usize,
        k: usize,
        th: &Thought,
        rail: Rail,
        weak: &WeakEntity<Self>,
        t: &Theme,
        moving: &mut bool,
    ) -> AnyElement {
        use crate::tool_calls as tc;
        let open = th.open.unwrap_or(th.live);
        let frame = self.folds.step((i, tc::PART_THOUGHT, k), open, Instant::now());
        *moving |= frame.animating;
        let toggle = {
            let weak = weak.clone();
            move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |v, cx| v.toggle_thought(i, k, cx));
            }
        };
        let header = chip_header(
            SharedString::from(format!("thought-{i}-{k}")),
            "Thought process".into(),
            div().into_any_element(),
            Vec::new(),
            Some((open, Box::new(toggle))),
            false,
            t,
        );
        let body = frame.mounted.then(|| tc::folding(tc::thought_body(&th.detail, t), &frame));
        self.rail_row(tc::Glyph::Icon(icons::CHAT_ROUND_LINE, t.text_muted), rail, header, body, t)
    }

    /// Zeron's spawn link: the whole card opens the subagent's chat. One
    /// quiet line (icon tile, "Agent", the task, the model) whose lifecycle
    /// shows only in its own language: the glyph spinner while it runs, the
    /// danger tint when it failed. No live text rewriting the header.
    fn render_subagent_chip(&self, i: usize, j: usize, tool: &Tool, t: &Theme) -> AnyElement {
        use crate::tool_calls as tc;
        let input = tool.input.as_ref();
        let running = tool.output.is_none();
        let failed = tool.error && !running;
        let call = tool.id.clone();
        let (label, detail) = chip_content(tool, &ToolView::of(tool));
        let tag = str_field(input, &["model"])
            .or_else(|| str_field(input, &["subagent_type", "agent"]))
            .filter(|k| !k.trim().is_empty());
        let tint = if failed { t.danger } else { t.text_muted };
        let tile = |glyph: &'static str, size: f32, bg: f32| {
            div()
                .size(px(18.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .bg(crate::theme::ink(bg))
                .child(icon(glyph).size(px(size)).text_color(t.text_muted.opacity(0.9)))
        };
        div()
            .h(px(38.0))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .child(
                div()
                    .id(SharedString::from(format!("subagent-chip-{i}-{j}")))
                    .h(px(tc::CARD_HEIGHT))
                    .min_w_0()
                    .flex_1()
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .overflow_hidden()
                    .rounded(px(9.0))
                    .border_1()
                    .border_color(crate::theme::ink(0.07))
                    .bg(crate::theme::ink(0.03))
                    .cursor_pointer()
                    .hover(|s| s.bg(crate::theme::ink(0.05)))
                    .text_size(px(tc::LABEL_SIZE))
                    .line_height(px(tc::LINE_HEIGHT))
                    .tooltip(ui::tip(if running { "Watch this subagent" } else { "Open this subagent" }))
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(crate::actions::OpenSubagent { call: call.clone() }), cx);
                    })
                    .child(tile(icons::BOT, 12.0, 0.08))
                    .child(div().flex_none().font_weight(gpui::FontWeight::MEDIUM).text_color(tint).child(label))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(if failed { t.danger } else { t.text.opacity(0.85) })
                            .child(detail),
                    )
                    .when_some(tag, |d, tag| {
                        d.child(div().flex_none().text_size(px(11.0)).text_color(t.text_faint).child(tag))
                    })
                    .when(running, |d| {
                        d.child(crate::loaders::glyph_spinner(
                            SharedString::from(format!("sub-chip-{i}-{j}")),
                            2.0,
                            crate::loaders::glyph_rows(t),
                        ))
                    })
                    .child(tile(icons::ARROW_UP_RIGHT, 11.0, 0.06)),
            )
            .into_any_element()
    }

    /// One call on the tree: Zeron's chip (label, detail, file badge) that
    /// unfolds into the full invocation and what came back. Edits unfold
    /// by default into Dove's inline diff.
    #[allow(clippy::too_many_arguments)]
    fn render_tool(
        &self,
        i: usize,
        j: usize,
        tool: &Tool,
        live: bool,
        rail: Rail,
        weak: &WeakEntity<Self>,
        t: &Theme,
        moving: &mut bool,
    ) -> AnyElement {
        use crate::tool_calls as tc;
        let view = ToolView::of(tool);
        let running = live && tool.output.is_none();
        let failed = tool.error && !running;
        let (label, detail) = chip_content(tool, &view);
        // The full call, unless the header already shows all of it. A file
        // badge shows only the name, so a path always unfolds.
        let invocation = invocation(tool, &view).filter(|call| {
            matches!(view, ToolView::File) && tc::file_name(call) != call
                || call.contains('\n')
                || tc::single_line(call) != detail
                || call.chars().count() > INVOCATION_INLINE
        });
        let expandable = tool.diff.is_some()
            || matches!(view, ToolView::Todo(_))
            || invocation.is_some()
            || tool.output.as_deref().is_some_and(|o| !o.trim().is_empty())
            || matches!(&view, ToolView::Agent { prompt } if !prompt.trim().is_empty());
        let open = expandable && tool.open.unwrap_or_else(|| tool.opens_by_default());
        let frame = self.folds.step((i, tc::PART_TOOL, j), open, Instant::now());
        *moving |= frame.animating;

        // Zeron keeps the tool's glyph at the tip while it runs: the group's
        // shimmering summary and the working trailer carry "busy".
        let glyph = tc::Glyph::Icon(tool_glyph(tool, &view), if failed { t.danger } else { t.text_muted });
        let detail: AnyElement = match tool_path(tool).filter(|_| matches!(view, ToolView::File)) {
            Some(path) => tc::file_badge(SharedString::from(format!("tool-file-{i}-{j}")), &path, failed, t),
            None => div()
                .min_w_0()
                .truncate()
                .when(matches!(view, ToolView::Shell { .. } | ToolView::Search), |d| {
                    d.font_family(t.font_mono.clone()).text_size(px(tc::LABEL_SIZE - 0.5))
                })
                .child(detail)
                .into_any_element(),
        };
        let mut meta: Vec<AnyElement> = Vec::new();
        if let Some(diff) = &tool.diff {
            meta.push(
                div()
                    .flex()
                    .flex_none()
                    .gap(px(4.0))
                    .font_family(t.font_mono.clone())
                    .text_size(px(11.0))
                    .child(div().text_color(t.success).child(format!("+{}", diff.added)))
                    .child(div().text_color(t.danger).child(format!("−{}", diff.removed)))
                    .into_any_element(),
            );
        }
        if let Some(took) = tool.took.filter(|d| d.as_secs() >= 1) {
            meta.push(
                div()
                    .flex_none()
                    .font_family(t.font_mono.clone())
                    .text_size(px(11.0))
                    .text_color(t.text_faint)
                    .child(short_duration(took))
                    .into_any_element(),
            );
        }
        let toggle = expandable.then(|| {
            let weak = weak.clone();
            let f: ChipToggle = Box::new(move |_, _, cx| {
                let _ = weak.update(cx, |v, cx| v.toggle_tool(i, j, cx));
            });
            (open, f)
        });
        let header = chip_header(
            SharedString::from(format!("tool-head-{i}-{j}")),
            label.into(),
            detail,
            meta,
            toggle,
            failed,
            t,
        );
        let body = frame.mounted.then(|| tc::folding(self.tool_body(i, j, tool, &view, invocation, weak, t), &frame));
        self.rail_row(glyph, rail, header, body, t)
    }

    /// What an open chip shows: the full call, then what came back.
    #[allow(clippy::too_many_arguments)]
    fn tool_body(
        &self,
        i: usize,
        j: usize,
        tool: &Tool,
        view: &ToolView,
        invocation: Option<String>,
        weak: &WeakEntity<Self>,
        t: &Theme,
    ) -> AnyElement {
        use crate::tool_calls as tc;
        let full = tool.full;
        let keep = if full { OUTPUT_FULL } else { OUTPUT_KEEP };
        let more = |tail: bool| {
            let weak = weak.clone();
            move |cut: usize| -> AnyElement {
                let weak = weak.clone();
                div()
                    .id(SharedString::from(format!("tool-more-{i}-{j}")))
                    .font_family(t.font_sans.clone())
                    .text_color(t.text_faint)
                    .cursor_pointer()
                    .hover(|s| s.text_color(t.text_muted))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        let _ = weak.update(cx, |v, cx| v.toggle_full(i, j, cx));
                    })
                    .child(if tail { format!("… {cut} earlier lines") } else { format!("… {cut} more lines") })
                    .into_any_element()
            }
        };
        let out = tool.output.as_deref().map(str::trim_end).filter(|o| !o.trim().is_empty());
        // Faint even when the call failed: the chip carries the red, and a
        // wall of red output reads as the whole turn breaking.
        let out_color = t.text_faint;
        let mut parts: Vec<AnyElement> = Vec::new();
        if let Some(call) = &invocation {
            parts.push(tc::lines_block(call, keep, false, t.text_muted, more(false), t));
        }
        let result: Option<AnyElement> = match view {
            _ if tool.kind == ToolKind::Edit && tool.diff.is_some() => {
                tool.diff.as_ref().map(|diff| self.render_mini_diff(i, j, diff, full, weak, t))
            }
            ToolView::Todo(items) => Some(todo_list(items, t)),
            ToolView::Agent { prompt } => {
                let mut prose = Vec::new();
                if !prompt.trim().is_empty() {
                    prose.push(tc::thought_body(&tc::prose_detail(prompt), t));
                }
                if let Some(o) = out {
                    prose.push(
                        div()
                            .pl(px(10.0))
                            .border_l_1()
                            .border_color(t.border)
                            .child(tc::thought_body(&tc::prose_detail(o), t))
                            .into_any_element(),
                    );
                }
                (!prose.is_empty()).then(|| div().flex().flex_col().gap(px(6.0)).children(prose).into_any_element())
            }
            ToolView::Search if !tool.error && out.is_some_and(|o| !path_lines(o).is_empty()) => {
                let paths = path_lines(out.unwrap_or_default());
                let keep = if full { 200 } else { 8 };
                let more = paths.len().saturating_sub(keep);
                Some(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.0))
                        .children(
                            paths
                                .into_iter()
                                .take(keep)
                                .enumerate()
                                .map(|(k, p)| path_chip(("hit", i * 10_000 + j, k), p, t)),
                        )
                        .when(more > 0, |d| d.child(more_chip(i, j, format!("+{more} more"), weak, t)))
                        .into_any_element(),
                )
            }
            ToolView::Web { .. } if !tool.error && out.is_some_and(|o| !urls_in(o).is_empty()) => {
                let urls = urls_in(out.unwrap_or_default());
                let keep = if full { 40 } else { 5 };
                let more = urls.len().saturating_sub(keep);
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .children(
                            urls.into_iter()
                                .take(keep)
                                .enumerate()
                                .map(|(k, u)| web_row(("web", i * 10_000 + j, k), u, t)),
                        )
                        .when(more > 0, |d| d.child(more_chip(i, j, format!("+{more} more results"), weak, t)))
                        .into_any_element(),
                )
            }
            _ => out.map(|o| {
                // JSON results read better indented.
                let text = serde_json::from_str::<serde_json::Value>(o)
                    .ok()
                    .filter(|v| v.is_object() || v.is_array())
                    .map(|v| pretty_json(&v))
                    .unwrap_or_else(|| o.to_string());
                let tail = matches!(view, ToolView::Shell { .. });
                tc::lines_block(&text, keep, tail, out_color, more(tail), t)
            }),
        };
        parts.extend(result);
        let cuttable = out.is_some_and(|o| tc::overflow(o, OUTPUT_KEEP) > 0)
            || invocation.as_deref().is_some_and(|c| tc::overflow(c, OUTPUT_KEEP) > 0);
        let less = (full && cuttable && tool.diff.is_none()).then(|| {
            let weak = weak.clone();
            div()
                .id(SharedString::from(format!("tool-less-{i}-{j}")))
                .text_size(px(tc::LABEL_SIZE))
                .text_color(t.text_faint)
                .cursor_pointer()
                .hover(|s| s.text_color(t.text_muted))
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |v, cx| v.toggle_full(i, j, cx));
                })
                .child("Show less")
        });
        div()
            .w_full()
            .min_w_0()
            .pt(px(2.0))
            .pb(px(8.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .children(parts)
            .children(less)
            .into_any_element()
    }

    fn render_mini_diff(
        &self,
        i: usize,
        j: usize,
        diff: &MiniDiff,
        open: bool,
        weak: &WeakEntity<Self>,
        t: &Theme,
    ) -> AnyElement {
        let take = if open { diff.lines } else { DIFF_PREVIEW };
        let more = diff.lines.saturating_sub(take) + if diff.truncated { 1 } else { 0 };
        let toggle = weak.clone();
        let changes = weak.clone();
        // Zeron's inline diff (the Changes pane's own rows) inside Dove's
        // card, with Dove's footer: unfold the rest, or jump to Changes.
        div()
            .rounded(px(8.0))
            .border_1()
            .border_color(t.border)
            .overflow_hidden()
            .flex()
            .flex_col()
            .children(crate::changes::inline_rows(&diff.file, &diff.spans, take, t))
            .child(
                div()
                    .h(px(26.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .border_t_1()
                    .border_color(t.border)
                    .bg(t.wash(0.02))
                    .font_family(t.font_sans.clone())
                    .text_size(px(12.0))
                    .text_color(t.text_faint)
                    .when(more > 0 || open, |d| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("diff-more-{i}-{j}")))
                                .cursor_pointer()
                                .hover(|s| s.text_color(t.text))
                                .on_click(move |_, _, cx| {
                                    let _ = toggle.update(cx, |v, cx| v.toggle_full(i, j, cx));
                                })
                                .child(if open { "Show less".to_string() } else { format!("{more} more lines") }),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        div()
                            .id(SharedString::from(format!("diff-view-{i}-{j}")))
                            .cursor_pointer()
                            .hover(|s| s.text_color(t.text))
                            .on_click(move |_, _, cx| {
                                let _ = changes.update(cx, |_, cx| cx.emit(TaskViewEvent::ShowChanges));
                            })
                            .child("View in Changes"),
                    ),
            )
            .into_any_element()
    }

    fn render_permission(
        &self,
        i: usize,
        req: &PermissionRequest,
        decided: Option<PermissionDecision>,
        weak: &WeakEntity<Self>,
        t: &Theme,
    ) -> AnyElement {
        if req.is_question() {
            let asked = req.questions.iter().map(|q| q.question.clone()).collect::<Vec<_>>().join(" · ");
            return div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_size(px(13.0))
                .text_color(t.text_faint)
                .child(icon(icons::CHAT_ROUND_LINE).size(px(14.0)).text_color(if decided.is_none() {
                    t.warning
                } else {
                    t.text_faint
                }))
                .child(div().flex_1().min_w_0().truncate().child(format!("Asked: {asked}")))
                .child(if decided.is_none() { "Waiting for your answer" } else { "Answered" })
                .into_any_element();
        }
        let root = self.task.worktree.clone().unwrap_or_else(|| self.project.path.clone());
        let hints = risk_hints(req, Some(&root));
        let mut card = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .p(px(14.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(if decided.is_none() { t.warning.opacity(0.55) } else { t.border })
            .bg(t.wash(0.03))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(icon(icons::DANGER_TRIANGLE).text_color(if decided.is_none() {
                        t.warning
                    } else {
                        t.text_faint
                    }))
                    .child(div().font_weight(gpui::FontWeight::MEDIUM).child(format!("Allow {}?", req.tool)))
                    .children(hints.into_iter().map(|h| {
                        div()
                            .px(px(6.0))
                            .rounded(px(5.0))
                            .bg(t.warning.opacity(0.12))
                            .text_color(t.warning)
                            .text_size(px(11.0))
                            .child(h)
                    })),
            )
            .child(
                div()
                    .px(px(10.0))
                    .py(px(7.0))
                    .rounded(px(8.0))
                    .bg(t.wash(0.04))
                    .font_family(t.font_mono.clone())
                    .text_size(px(12.5))
                    .text_color(t.text_muted)
                    .child(req.summary.to_string()),
            );
        match decided {
            None => {
                let b =
                    |id: &'static str, label: &'static str, key: &'static str, d: PermissionDecision, primary: bool| {
                        let weak = weak.clone();
                        let rid = req.request_id.clone();
                        let el = if primary {
                            ui::button_primary(SharedString::from(format!("{id}-{i}")), label, t)
                        } else {
                            ui::button_ghost(SharedString::from(format!("{id}-{i}")), label, t)
                        };
                        el.child(kit::kbd(key, t)).on_click(move |_, _, cx| {
                            let _ = weak.update(cx, |v, cx| v.decide(rid.clone(), d, cx));
                        })
                    };
                card = card.child(
                    div()
                        .flex()
                        .gap(px(6.0))
                        .child(b("allow", "Allow", "A", PermissionDecision::AllowOnce, true))
                        .child(b("always", "Allow for session", "S", PermissionDecision::AllowAlways, false))
                        .child(b("deny", "Deny", "D", PermissionDecision::Deny, false)),
                );
            }
            Some(d) => {
                card = card.child(div().text_size(px(12.0)).text_color(t.text_faint).child(match d {
                    PermissionDecision::Deny => "Denied",
                    PermissionDecision::AllowAlways => "Allowed for this session",
                    PermissionDecision::AllowOnce => "Allowed",
                }));
            }
        }
        card.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_turn_end(
        &self,
        i: usize,
        msgs: Range<usize>,
        prompt: Option<SharedString>,
        elapsed: Option<Duration>,
        failed: bool,
        weak: &WeakEntity<Self>,
        t: &Theme,
    ) -> AnyElement {
        let copy: String = self.chat.messages
            [msgs.start.min(self.chat.messages.len())..msgs.end.min(self.chat.messages.len())]
            .iter()
            .map(|m| m.trim())
            .filter(|m| !m.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let last = !self.chat.rows[i + 1..].iter().any(|r| matches!(r, Row::TurnEnd { .. }));
        let can_fork = last && self.mirror.is_none() && Engine::can_fork(&self.task);
        let idle = !self.is_working() && self.mirror.is_none();
        div()
            .group("turn-end")
            .h(px(28.0))
            .flex()
            .items_center()
            .gap(px(2.0))
            .text_size(px(12.0))
            .text_color(t.text_faint)
            .when(!copy.is_empty(), |d| d.child(copy_button(SharedString::from(format!("copy-turn-{i}")), copy, t)))
            .when_some(prompt.filter(|_| idle), |d, p| {
                let weak = weak.clone();
                d.child(action_button(SharedString::from(format!("retry-{i}")), icons::RESTART, "Retry", t).on_click(
                    move |_, window, cx| {
                        let _ = weak.update(cx, |v, cx| v.retry(p.clone(), window, cx));
                    },
                ))
            })
            .when(can_fork, |d| {
                let weak = weak.clone();
                d.child(action_button(SharedString::from(format!("fork-{i}")), icons::FORK, "Fork", t).on_click(
                    move |_, _, cx| {
                        let _ = weak.update(cx, |_, cx| cx.emit(TaskViewEvent::Fork));
                    },
                ))
            })
            .when_some(elapsed, |d, e| {
                d.child(div().ml(px(6.0)).child(if failed {
                    format!("Failed after {}", duration(e))
                } else {
                    format!("Worked for {}", duration(e))
                }))
            })
            .when(failed && elapsed.is_none(), |d| d.child(div().ml(px(6.0)).text_color(t.danger).child("Turn failed")))
            .into_any_element()
    }

    fn render_todos(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let todos = &self.chat.todos;
        if todos.is_empty() || self.todos_hidden {
            return None;
        }
        let t = Theme::of(cx).clone();
        let done = todos.iter().filter(|x| x.status == TodoStatus::Completed).count();
        let current = todos
            .iter()
            .find(|x| x.status == TodoStatus::InProgress)
            .or_else(|| todos.iter().find(|x| x.status == TodoStatus::Pending))
            .map(|x| x.text.clone())
            .unwrap_or_default();
        let header = div()
            .h(px(36.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(icon(icons::CHECKLIST).text_color(t.text_muted))
            .child(div().font_weight(gpui::FontWeight::MEDIUM).child("Todo"))
            .child(div().text_color(t.text_faint).child(format!("{done}/{}", todos.len())))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(t.text_muted)
                    .when(!self.todos_open, |d| d.child(current)),
            )
            .child(
                div()
                    .id("todo-toggle")
                    .cursor_pointer()
                    .child(
                        icon(if self.todos_open { icons::ALT_ARROW_DOWN } else { icons::ALT_ARROW_UP })
                            .size(px(14.0))
                            .text_color(t.text_muted),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.todos_open = !this.todos_open;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("todo-close")
                    .cursor_pointer()
                    .child(icon(icons::CLOSE).size(px(14.0)).text_color(t.text_muted))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.todos_hidden = true;
                        cx.notify();
                    })),
            );
        let items = self.todos_open.then(|| {
            div().px(px(12.0)).pb(px(10.0)).flex().flex_col().gap(px(6.0)).children(todos.iter().map(|item| {
                let (glyph, color, strike) = match item.status {
                    TodoStatus::Completed => (Some(icons::CHECK), t.success, true),
                    TodoStatus::InProgress => (None, t.busy, false),
                    TodoStatus::Pending => (None, t.text_faint, false),
                };
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.0))
                    .text_size(px(13.0))
                    .child(
                        div().mt(px(3.0)).size(px(13.0)).flex_none().flex().items_center().justify_center().child(
                            match glyph {
                                Some(g) => icon(g).size(px(13.0)).text_color(color).into_any_element(),
                                None => div()
                                    .size(px(11.0))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(color)
                                    .when(item.status == TodoStatus::InProgress, |d| d.bg(color.opacity(0.3)))
                                    .into_any_element(),
                            },
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_color(if strike { t.text_faint } else { t.text })
                            .when(strike, |d| d.line_through())
                            .child(item.text.clone()),
                    )
            }))
        });
        Some(
            ui::frosted(
                12.0,
                20.0,
                div()
                    .w_full()
                    .rounded(px(12.0))
                    .bg(t.glass_overlay())
                    .border_1()
                    .border_color(t.border)
                    .text_size(px(13.0))
                    .child(header)
                    .children(items),
            )
            .into_any_element(),
        )
    }

    /// The reply that followed the user message at `row`, for previews.
    fn reply_after(&self, row: usize) -> Option<String> {
        self.chat.rows[row + 1..].iter().take_while(|r| !matches!(r, Row::User { .. })).find_map(|r| match r {
            Row::Block { block, .. } => Some(block.plain()).filter(|t| !t.trim().is_empty()),
            _ => None,
        })
    }

    /// The message rail: a compact minimap of your prompts along the left
    /// edge. The prompt you are reading lights up; hover shows the prompt
    /// and the start of its reply; click glides there. Long threads fold
    /// into evenly sized buckets so the rail never grows past 14 marks.
    fn render_rail(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        const MAX_MARKS: usize = 14;
        let messages = self.chat.user_messages();
        if messages.len() < 2 {
            return None;
        }
        let t = Theme::of(cx).clone();
        let weak = cx.weak_entity();
        let top = self.list.logical_scroll_top().item_ix;
        let active = messages.iter().rposition(|(row, _)| *row <= top + 1).unwrap_or(0);
        let n = messages.len();
        let cap = n.min(MAX_MARKS);
        let buckets: Vec<(usize, usize)> = (0..cap).map(|k| (k * n / cap, (k + 1) * n / cap)).collect();
        let mut rail = div()
            .absolute()
            .left(px(10.0))
            .top_0()
            .bottom_0()
            .w(px(30.0))
            .flex()
            .flex_col()
            .justify_center()
            .gap(px(2.0));
        for (ix, (start, end)) in buckets.into_iter().enumerate() {
            let is_active = active >= start && active < end;
            let rep = if is_active { active } else { start };
            let (row, text) = messages[rep].clone();
            let hovered = self.rail_hover == Some(ix);
            let reply = hovered.then(|| self.reply_after(row)).flatten();
            let w_hover = weak.clone();
            let w_click = weak.clone();
            let mark = div()
                .id(SharedString::from(format!("rail-{ix}")))
                .relative()
                .h(px(12.0))
                .w_full()
                .flex()
                .items_center()
                .cursor_pointer()
                .on_hover(move |on, _, cx| {
                    let _ = w_hover.update(cx, |v, cx| {
                        let next = if *on {
                            Some(ix)
                        } else if v.rail_hover == Some(ix) {
                            None
                        } else {
                            v.rail_hover
                        };
                        if v.rail_hover != next {
                            v.rail_hover = next;
                            cx.notify();
                        }
                    });
                })
                .on_click(move |_, _, cx| {
                    let _ = w_click.update(cx, |v, cx| v.glide_to_row(row, cx));
                })
                .child(
                    div()
                        .h(px(if is_active { 3.0 } else { 2.0 }))
                        .w(px(if hovered {
                            22.0
                        } else if is_active {
                            16.0
                        } else {
                            11.0
                        }))
                        .rounded_full()
                        .bg(if is_active || hovered { t.text.opacity(0.85) } else { t.text.opacity(0.18) }),
                )
                .when(hovered, |d| {
                    let prompt: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    let prompt: String = if prompt.chars().count() > 160 {
                        format!("{}…", prompt.chars().take(159).collect::<String>())
                    } else {
                        prompt
                    };
                    let reply = reply.map(|r| {
                        let r = r.split_whitespace().collect::<Vec<_>>().join(" ");
                        if r.chars().count() > 200 {
                            format!("{}…", r.chars().take(199).collect::<String>())
                        } else {
                            r
                        }
                    });
                    let span = end - start;
                    d.child(
                        deferred(
                            anchored().anchor(Anchor::LeftCenter).offset(gpui::point(px(34.0), px(0.0))).child(
                                div()
                                    .w(px(300.0))
                                    .p(px(12.0))
                                    .flex()
                                    .flex_col()
                                    .gap(px(6.0))
                                    .rounded(px(12.0))
                                    .border_1()
                                    .border_color(t.border)
                                    .bg(t.surface_overlay)
                                    .shadow_lg()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(6.0))
                                            .text_size(px(11.0))
                                            .text_color(t.text_faint)
                                            .child(format!("Prompt {} of {n}", rep + 1))
                                            .when(span > 1, |d| d.child(format!("· {span} here"))),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.0))
                                            .line_height(px(18.0))
                                            .text_color(t.text)
                                            .child(prompt),
                                    )
                                    .when_some(reply, |d, r| {
                                        d.child(
                                            div()
                                                .pt(px(6.0))
                                                .border_t_1()
                                                .border_color(t.border)
                                                .text_size(px(12.0))
                                                .line_height(px(17.0))
                                                .text_color(t.text_muted)
                                                .child(r),
                                        )
                                    }),
                            ),
                        )
                        .with_priority(3),
                    )
                });
            rail = rail.child(mark);
        }
        Some(rail.into_any_element())
    }

    /// The reserved strip under the composer: what the agent is doing now.
    fn render_status_strip(&self, t: &Theme) -> impl IntoElement {
        let working = self.is_working();
        let waiting = self.status == TaskStatus::NeedsInput && self.live.is_some();
        let step = self.chat.current_step();
        let todos = &self.chat.todos;
        let done = todos.iter().filter(|x| x.status == TodoStatus::Completed).count();
        div()
            .h(px(22.0))
            .px(px(4.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(px(12.0))
            .text_color(t.text_faint)
            // The spinner and timer live in the transcript's working trailer;
            // the strip keeps what the agent is doing right now.
            .when(working, |d| d.child(div().flex_1().min_w_0().truncate().when_some(step, |d, step| d.child(step))))
            .when(waiting, |d| {
                d.child(ui::dot(t.warning, 6.0)).child(div().flex_1().child(if self.questions.is_some() {
                    "Waiting for your answer"
                } else {
                    "Waiting for your approval · A allow · S session · D deny"
                }))
            })
            .when(!working && !waiting, |d| d.child(div().flex_1()))
            .when(!todos.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .child(icon(icons::CHECKLIST).size(px(12.0)))
                        .child(format!("{done}/{}", todos.len())),
                )
            })
    }

    fn render_questions(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let q = self.questions.as_ref()?;
        let (_, req) = self.chat.pending()?;
        let question = req.questions.get(q.page)?;
        let t = Theme::of(cx).clone();
        let total = req.questions.len();
        let chosen = &q.answers[q.page];
        let _ = window;
        Some(
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .p(px(14.0))
                .rounded(px(16.0))
                .border_1()
                .border_color(t.border_strong)
                .bg(t.surface_raised)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(px(12.0))
                        .text_color(t.text_faint)
                        .child(icon(icons::CHAT_ROUND_LINE).size(px(14.0)).text_color(t.accent))
                        .child(question.header.clone())
                        .child(div().flex_1())
                        .when(total > 1, |d| d.child(format!("{}/{}", q.page + 1, total)))
                        .child(
                            div()
                                .id("q-skip")
                                .cursor_pointer()
                                .hover(|s| s.text_color(t.text))
                                .child("Skip")
                                .on_click(cx.listener(|this, _, _, cx| this.skip_questions(cx))),
                        ),
                )
                .child(div().text_size(px(14.0)).text_color(t.text).child(question.question.clone()))
                .child(div().flex().flex_col().gap(px(4.0)).children(question.options.iter().enumerate().map(
                    |(n, label)| {
                        let selected = chosen.contains(label);
                        let label2 = label.clone();
                        div()
                            .id(SharedString::from(format!("q-opt-{n}")))
                            .h(px(34.0))
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(if selected { t.accent.opacity(0.6) } else { t.border })
                            .when(selected, |d| d.bg(t.accent.opacity(0.08)))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.glass_hover()))
                            .on_click(
                                cx.listener(move |this, _, window, cx| this.pick_option(label2.clone(), window, cx)),
                            )
                            .when(n < 9, |d| d.child(kit::kbd(&(n + 1).to_string(), &t)))
                            .child(div().flex_1().child(label.clone()))
                            .when(selected, |d| d.child(icon(icons::CHECK).size(px(14.0)).text_color(t.accent)))
                    },
                )))
                .child(div().flex().items_center().gap(px(8.0)).child(div().flex_1().child(Input::new(&q.other))).when(
                    question.multi_select,
                    |d| {
                        d.child(
                            ui::button_primary("q-next", if q.page + 1 < total { "Next" } else { "Send" }, &t)
                                .on_click(cx.listener(|this, _, window, cx| this.next_question(window, cx))),
                        )
                    },
                ))
                .into_any_element(),
        )
    }
}


fn action_button(id: SharedString, glyph: &'static str, tip: &'static str, t: &Theme) -> gpui::Stateful<gpui::Div> {
    ui::row_button(id, glyph, tip, t)
}

fn copy_button(id: SharedString, text: String, t: &Theme) -> impl IntoElement {
    action_button(id, icons::COPY, "Copy", t)
        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone())))
}

impl Render for TaskView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The question panel needs a window to build its input.
        if self.questions.is_none()
            && let Some((_, req)) = self.chat.pending().filter(|(_, r)| r.is_question() && self.live.is_some())
        {
            let req = req.clone();
            self.open_questions(&req, window, cx);
        }
        let t = Theme::of(cx).clone();
        let weak = cx.weak_entity();
        let empty = self.chat.rows.is_empty();
        let mirror = self.mirror.is_some();
        // A subagent view has no composer dock.
        let dock_h = if mirror { 0.0 } else { self.dock_height.get() };
        // Replies dissolve as they slide under the pill and are gone by the
        // composer's meta row, which has no plate of its own.
        let under_meta = if self.terminal_banner || self.questions.is_some() { 0.0 } else { COMPOSER_META };

        let transcript: AnyElement = if self.loading {
            div().flex_1().into_any_element()
        } else if empty {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.text_faint)
                .child(if mirror {
                    "Waiting for the subagent…"
                } else if self.live.is_some() {
                    "Starting agent…"
                } else {
                    "No messages yet"
                })
                .into_any_element()
        } else {
            let renderer = weak.clone();
            let rows = list(self.list.clone(), move |i, window, cx| {
                let Some(view) = renderer.upgrade() else { return div().into_any_element() };
                let v = view.read(cx);
                if i >= v.chat.rows.len() {
                    return div().into_any_element();
                }
                v.render_row(i, &renderer, window, cx)
            })
            .flex_1()
            .size_full()
            .pt(px(if mirror { 8.0 } else { 20.0 }))
            .pb(px(dock_h + 12.0));
            if mirror {
                rows.into_any_element()
            } else {
                ui::bottom_faded(under_meta, (dock_h - under_meta).max(24.0), rows).into_any_element()
            }
        };

        let jump = (!self.following.get() && !empty).then(|| {
            div().absolute().bottom(px(dock_h + 10.0)).left_0().right_0().flex().justify_center().child(
                div()
                    .id("jump-latest")
                    .h(px(30.0))
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .rounded_full()
                    .border_1()
                    .border_color(t.border)
                    .bg(t.surface_raised)
                    .shadow_md()
                    .text_size(px(12.0))
                    .text_color(t.text_muted)
                    .cursor_pointer()
                    .hover(|s| s.text_color(t.text))
                    .child(icon(icons::ARROW_DOWN).size(px(13.0)))
                    .child(if self.is_working() { "Jump to latest" } else { "Scroll to bottom" })
                    .on_click(cx.listener(|this, _, _, cx| this.jump_to_latest(cx))),
            )
        });

        let input: AnyElement = if self.terminal_banner {
            div()
                .w_full()
                .h(px(52.0))
                .px(px(16.0))
                .flex()
                .items_center()
                .gap(px(10.0))
                .rounded(px(Theme::COMPOSER_RADIUS))
                .border_1()
                .border_color(t.border_strong)
                .bg(t.wash(0.04))
                .child(icon(icons::TERMINAL).size(px(16.0)).text_color(t.accent))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(13.5)).text_color(t.text).child("Running in the terminal"))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(t.text_faint)
                                .child("Read-only here; this chat stays in sync"),
                        ),
                )
                .child(
                    ui::button_ghost("banner-terminal", "Show terminal", &t)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(TaskViewEvent::ShowTerminal))),
                )
                .child(
                    ui::button_primary("banner-back", "Switch back to chat", &t)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(TaskViewEvent::SwitchToChat))),
                )
                .into_any_element()
        } else {
            match self.render_questions(window, cx) {
                Some(panel) => panel,
                None => self.composer.clone().into_any_element(),
            }
        };
        // The composer floats over the transcript's bottom edge, Zeron
        // style: replies scroll up beneath the glass. Its measured height
        // pads the list (and lifts the jump button) on the next frame.
        let measured = self.dock_height.clone();
        let view = cx.weak_entity();
        let dock = div().absolute().left_0().right_0().bottom_0().flex().justify_center().px(px(24.0)).child(
            div()
                .relative()
                .w_full()
                .max_w(px(COLUMN_MAX + 8.0))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .children(self.render_todos(cx))
                .child(input)
                .child(
                    canvas(
                        move |b, _, cx| {
                            let h = f32::from(b.size.height);
                            if (measured.get() - h).abs() > 0.5 {
                                measured.set(h);
                                let view = view.clone();
                                cx.defer(move |cx| {
                                    let _ = view.update(cx, |_, cx| cx.notify());
                                });
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                ),
        );
        let bottom = div()
            .w_full()
            .flex()
            .justify_center()
            .px(px(24.0))
            .pt(px(4.0))
            .pb(px(6.0))
            .child(div().w_full().max_w(px(COLUMN_MAX + 8.0)).child(self.render_status_strip(&t)));

        div()
            .id("task-view")
            .key_context("TaskView")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("chat-area")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        if this.chat_hovered != *hovered {
                            this.chat_hovered = *hovered;
                            cx.notify();
                        }
                    }))
                    .child(transcript)
                    .when(!empty && !self.loading, |d| {
                        d.child(
                            // Just a rounded thumb, no track: hidden at rest,
                            // shown while scrolling or while the pointer is
                            // over the conversation, then it fades.
                            gpui_base::Scrollbar::vertical(&self.list)
                                .id("chat-scrollbar")
                                .mode(if self.chat_hovered {
                                    gpui_base::ScrollbarMode::Always
                                } else {
                                    gpui_base::ScrollbarMode::Scrolling
                                })
                                .styles(|s| {
                                    let clear = gpui::transparent_black();
                                    s.track(|tr| tr.bg(clear).border_color(clear))
                                        .track_hover(|tr| tr.bg(clear).border_color(clear))
                                        .track_active(|tr| tr.bg(clear).border_color(clear))
                                        .thumb(|th| {
                                            th.bg(t.text.opacity(0.20))
                                                .width(px(5.0))
                                                .radius(px(3.0))
                                                .min_length(px(40.0))
                                        })
                                        .thumb_hover(|th| {
                                            th.bg(t.text.opacity(0.38))
                                                .width(px(7.0))
                                                .radius(px(4.0))
                                                .min_length(px(40.0))
                                        })
                                        .thumb_active(|th| {
                                            th.bg(t.text.opacity(0.50))
                                                .width(px(7.0))
                                                .radius(px(4.0))
                                                .min_length(px(40.0))
                                        })
                                }),
                        )
                    })
                    .when(!mirror, |d| d.children(self.render_rail(cx)))
                    .children(jump)
                    .when(!mirror, |d| d.child(dock)),
            )
            .when(!mirror, |d| d.child(bottom))
    }
}

/// Dove's canvas tool, however the harness spells MCP names.
fn is_show_canvas(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("dove") && (n.ends_with("__show_canvas") || n.ends_with("/show_canvas") || n.ends_with(".show_canvas"))
}

/// Dove's generative-UI tool, however the harness spells MCP names.
fn is_show_ui(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("dove") && (n.ends_with("__show_ui") || n.ends_with("/show_ui") || n.ends_with(".show_ui"))
}

// ------------------------------------------------------------ tool views

/// How a tool call is drawn, recognised from its kind, name and input.
enum ToolView {
    Shell { cmd: String },
    Search,
    Web { query: String },
    Browser { action: String, target: String },
    Mcp { server: String, tool: String },
    Agent { prompt: String },
    Todo(Vec<(String, TodoStatus)>),
    File,
    Generic,
}

impl ToolView {
    fn of(tool: &Tool) -> Self {
        let input = tool.input.as_ref();
        let (server, name) = mcp_parts(&tool.name);
        if let Some(action) = name.strip_prefix("browser_") {
            let target = str_field(input, &["url", "selector", "ref", "element", "text", "key", "direction", "script"])
                .unwrap_or_default();
            let mut action = action.replace('_', " ");
            if let Some(c) = action.get_mut(0..1) {
                c.make_ascii_uppercase();
            }
            return Self::Browser { action, target: first_line(&target) };
        }
        if let Some(items) = todo_items(&name, input) {
            return Self::Todo(items);
        }
        if let Some(server) = server {
            return Self::Mcp { server, tool: name };
        }
        match tool.kind {
            ToolKind::Execute => {
                let cmd = match input.get("command").or_else(|| input.get("cmd")) {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(serde_json::Value::Array(a)) => {
                        let parts: Vec<&str> = a.iter().filter_map(|v| v.as_str()).collect();
                        // `["bash", "-lc", "..."]` shows just the script.
                        match parts.as_slice() {
                            [sh, flag, script] if sh.ends_with("sh") && flag.starts_with('-') => script.to_string(),
                            _ => parts.join(" "),
                        }
                    }
                    _ => tool.title.to_string(),
                };
                Self::Shell { cmd: if cmd.trim().is_empty() { tool.title.to_string() } else { cmd } }
            }
            ToolKind::Search => Self::Search,
            ToolKind::Fetch => Self::Web {
                query: str_field(input, &["query", "url", "q"])
                    .map(|q| first_line(&q))
                    .unwrap_or_else(|| tool.title.to_string()),
            },
            ToolKind::Agent => {
                Self::Agent { prompt: str_field(input, &["prompt", "description", "task"]).unwrap_or_default() }
            }
            ToolKind::Read | ToolKind::Edit => Self::File,
            _ => Self::Generic,
        }
    }
}

/// `(server, tool)` for MCP spellings (`mcp__server__tool`, `server/tool`);
/// the bare name otherwise.
fn mcp_parts(name: &str) -> (Option<String>, String) {
    let n = name.trim();
    if let Some((server, tool)) = n.strip_prefix("mcp__").and_then(|r| r.split_once("__")) {
        return (Some(server.to_string()), tool.to_string());
    }
    if let Some((server, tool)) = n.split_once('/').filter(|(s, t)| !s.is_empty() && !t.is_empty() && !t.contains('/'))
    {
        return (Some(server.to_string()), tool.to_string());
    }
    (None, n.to_string())
}

fn str_field(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| v.get(*k).and_then(|x| x.as_str()).filter(|s| !s.trim().is_empty()).map(str::to_string))
}

/// Checklist items from a todo/plan tool (`TodoWrite`, `update_plan`, ...).
fn todo_items(name: &str, input: &serde_json::Value) -> Option<Vec<(String, TodoStatus)>> {
    let n = name.to_ascii_lowercase();
    if !(n.contains("todo") || n.contains("plan")) {
        return None;
    }
    let list = ["todos", "plan", "items", "steps"].iter().find_map(|k| input.get(*k).and_then(|v| v.as_array()))?;
    let items: Vec<(String, TodoStatus)> = list
        .iter()
        .filter_map(|it| {
            let text = str_field(it, &["content", "step", "text", "title", "activeForm"])?;
            let status = match it.get("status").and_then(|s| s.as_str()).unwrap_or("") {
                "completed" | "done" | "complete" => TodoStatus::Completed,
                "in_progress" | "in-progress" | "active" | "running" => TodoStatus::InProgress,
                _ => TodoStatus::Pending,
            };
            Some((text, status))
        })
        .collect();
    (!items.is_empty()).then_some(items)
}

fn first_line(s: &str) -> String {
    let line = s.trim().lines().next().unwrap_or("").trim();
    if s.trim().lines().nth(1).is_some() { format!("{line} …") } else { line.to_string() }
}

/// `0.4s`, `12.3s`, `1m 12s`.
fn short_duration(d: Duration) -> String {
    if d.as_secs() < 60 { format!("{:.1}s", d.as_secs_f64()) } else { duration(d) }
}

/// Lines of a search result that name a file (`path`, `path:12`, `path:12:text`).
fn path_lines(out: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    out.lines()
        .filter_map(|l| {
            let l = l.trim();
            let head = match l.split_once(':') {
                Some((p, rest)) if rest.split(':').next().is_some_and(|n| n.chars().all(|c| c.is_ascii_digit())) => {
                    let line = rest.split(':').next().unwrap_or("");
                    if line.is_empty() { p.to_string() } else { format!("{p}:{line}") }
                }
                _ => l.to_string(),
            };
            let path = head.split(':').next().unwrap_or("");
            let looks = !path.is_empty()
                && !path.contains(char::is_whitespace)
                && (path.contains('/') || path.contains('.'))
                && !path.starts_with("http");
            (looks && seen.insert(head.clone())).then_some(head)
        })
        .collect()
}

/// Every distinct `http(s)://` URL in `out`, in order.
fn urls_in(out: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut urls = Vec::new();
    let mut rest = out;
    while let Some(at) = rest.find("http") {
        let s = &rest[at..];
        if s.starts_with("https://") || s.starts_with("http://") {
            let end = s
                .find(|c: char| c.is_whitespace() || matches!(c, ')' | ']' | '>' | '"' | '\'' | '`' | ',' | '<'))
                .unwrap_or(s.len());
            let url = s[..end].trim_end_matches(['.', ';', ':']).to_string();
            if url.len() > 10 && seen.insert(url.clone()) {
                urls.push(url);
            }
            rest = &s[end..];
        } else {
            rest = &s[4..];
        }
    }
    urls
}

fn path_chip(id: (&'static str, usize, usize), path: String, t: &Theme) -> AnyElement {
    let (name, line) = markdown::split_path_line(&path);
    let label = match line {
        Some(n) => format!("{}:{n}", short_path(&name)),
        None => short_path(&name),
    };
    div()
        .id(SharedString::from(format!("{}-{}-{}", id.0, id.1, id.2)))
        .h(px(22.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(5.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(t.wash(0.08))
        .bg(t.wash(0.03))
        .font_family(t.font_mono.clone())
        .text_size(px(11.5))
        .text_color(t.text_muted)
        .cursor_pointer()
        .hover(|s| s.bg(t.wash(0.07)).text_color(t.text))
        .tooltip(ui::tip(path.clone()))
        .child(crate::files_panel::file_type_icon(&name, false, t).size(px(12.0)))
        .child(label)
        .on_click(move |_, window, cx| {
            let (path, line) = markdown::split_path_line(&path);
            window.dispatch_action(Box::new(crate::actions::OpenPath { path, line }), cx);
        })
        .into_any_element()
}

/// The last two components of a path.
fn short_path(p: &str) -> String {
    let parts: Vec<&str> = p.trim_end_matches('/').rsplitn(3, '/').collect();
    match parts.as_slice() {
        [file, dir, _] => format!("{dir}/{file}"),
        _ => p.to_string(),
    }
}

/// A result link: globe, domain, path; opens in the browser.
fn web_row(id: (&'static str, usize, usize), url: String, t: &Theme) -> AnyElement {
    let after = url.split_once("://").map(|(_, r)| r).unwrap_or(&url);
    let (domain, path) = after.split_once('/').map(|(d, p)| (d, format!("/{p}"))).unwrap_or((after, String::new()));
    let domain = domain.trim_start_matches("www.").to_string();
    div()
        .id(SharedString::from(format!("{}-{}-{}", id.0, id.1, id.2)))
        .h(px(24.0))
        .px(px(6.0))
        .mx(px(-6.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .rounded(px(6.0))
        .text_size(px(12.5))
        .cursor_pointer()
        .hover(|s| s.bg(t.wash(0.05)))
        .child(icon(icons::GLOBE).size(px(12.0)).text_color(t.text_faint))
        .child(div().flex_none().text_color(t.text_muted).child(domain))
        .child(div().min_w_0().truncate().text_color(t.text_faint).child(path))
        .on_click(move |_, _, cx| cx.open_url(&url))
        .into_any_element()
}

fn pretty_json(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Object(m) if m.is_empty() => String::new(),
        _ => serde_json::to_string_pretty(v).unwrap_or_default(),
    }
}

fn todo_list(items: &[(String, TodoStatus)], t: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .text_size(px(13.0))
        .children(items.iter().map(|(text, status)| {
            let mark: AnyElement = match status {
                TodoStatus::Completed => icon(icons::CHECK).size(px(13.0)).text_color(t.text_faint).into_any_element(),
                TodoStatus::InProgress => div()
                    .size(px(11.0))
                    .rounded_full()
                    .border_1()
                    .border_color(t.text_muted)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size(px(5.0)).rounded_full().bg(t.text_muted))
                    .into_any_element(),
                TodoStatus::Pending => {
                    div().size(px(11.0)).rounded_full().border_1().border_color(t.border_strong).into_any_element()
                }
            };
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(div().flex_none().size(px(14.0)).flex().items_center().justify_center().child(mark))
                .child(
                    div()
                        .min_w_0()
                        .text_color(match status {
                            TodoStatus::Completed => t.text_faint,
                            TodoStatus::InProgress => t.text,
                            TodoStatus::Pending => t.text_muted,
                        })
                        .when(*status == TodoStatus::Completed, |d| d.line_through())
                        .child(text.clone()),
                )
        }))
        .into_any_element()
}

// ------------------------------------------------------------ tool chips

/// A step on an activity's tree.
#[derive(Clone, Copy)]
enum Step {
    Thought(usize),
    Tool(usize),
}

/// Where a row sits on the tree and how far its connector has drawn.
#[derive(Clone, Copy)]
struct Rail {
    /// A row above it on the tree (its trunk comes from there).
    after: bool,
    /// A row below it (its trunk continues down).
    continues: bool,
    connector: f32,
    continuation: f32,
}

type ChipToggle = Box<dyn Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static>;

impl Tool {
    /// Edits unfold into their diff; everything else waits to be asked.
    pub fn opens_by_default(&self) -> bool {
        self.kind == ToolKind::Edit && self.diff.is_some()
    }
}

/// Zeron's chip header: label, one-line detail, trailing meta, and a
/// chevron that surfaces on hover when the chip unfolds.
fn chip_header(
    id: SharedString,
    label: SharedString,
    detail: AnyElement,
    meta: Vec<AnyElement>,
    toggle: Option<(bool, ChipToggle)>,
    failed: bool,
    t: &Theme,
) -> AnyElement {
    use crate::tool_calls as tc;
    let open = toggle.as_ref().map(|(open, _)| *open);
    div()
        .id(id)
        .group("tool-header")
        .h(px(tc::CARD_HEIGHT))
        .my(px((tc::ROW_HEIGHT - tc::CARD_HEIGHT) / 2.0))
        .w_full()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(8.0))
        .text_size(px(tc::LABEL_SIZE))
        .line_height(px(tc::LINE_HEIGHT))
        .text_color(if failed { t.danger } else { t.text_muted })
        .when(!failed, |d| d.hover(|s| s.text_color(t.text)))
        .when_some(toggle, |d, (_, f)| d.cursor_pointer().on_click(f))
        .child(div().flex_none().child(label))
        .child(div().min_w_0().flex().items_center().child(detail))
        .children(meta)
        .when_some(open, |d, open| {
            d.child(
                div()
                    .size(px(18.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.0)
                    .group_hover("tool-header", |s| s.opacity(1.0))
                    .child(
                        icon(if open { icons::ALT_ARROW_DOWN } else { icons::ALT_ARROW_RIGHT })
                            .size(px(12.0))
                            .text_color(if failed { t.danger } else { t.text_faint }),
                    ),
            )
        })
        .into_any_element()
}

/// A chip's label and one-line detail, named the way Zeron names them
/// ("Run", "Read", "Edit", "Search", "Web", "MCP" ...).
fn chip_content(tool: &Tool, view: &ToolView) -> (String, String) {
    use crate::tool_calls::single_line;
    let name = tool.name.to_ascii_lowercase();
    let input = tool.input.as_ref();
    let (label, detail): (String, String) = match view {
        ToolView::Browser { action, target } => (action.clone(), target.clone()),
        ToolView::Todo(items) => {
            let done = items.iter().filter(|(_, s)| *s == TodoStatus::Completed).count();
            ("Todo".into(), format!("{done}/{} done", items.len()))
        }
        ToolView::Mcp { server, tool } => ("MCP".into(), format!("{server} · {tool}")),
        ToolView::Shell { cmd } => ("Run".into(), cmd.clone()),
        ToolView::Search => {
            let pattern =
                str_field(input, &["pattern", "query", "regex", "glob", "q"]).unwrap_or_else(|| tool.title.to_string());
            let label =
                if name.contains("glob") || name.contains("find") || name.contains("list") { "Glob" } else { "Search" };
            let detail = match str_field(input, &["path", "include", "dir"]) {
                Some(path) => format!("{pattern} in {path}"),
                None => pattern,
            };
            (label.into(), detail)
        }
        ToolView::Web { query } => (if name.contains("search") { "Web" } else { "Fetch" }.into(), query.clone()),
        ToolView::Agent { .. } => {
            let what = str_field(input, &["description"]).unwrap_or_else(|| tool.title.to_string());
            ("Agent".into(), what)
        }
        ToolView::File => {
            let path = tool_path(tool).unwrap_or_else(|| tool.title.to_string());
            let label = match tool.kind {
                ToolKind::Read => "Read",
                _ if name.contains("write") || name.contains("create") => "Write",
                _ if name.contains("patch") => "Patch",
                _ => "Edit",
            };
            (label.into(), path)
        }
        ToolView::Generic => {
            (if tool.kind == ToolKind::Think { "Think" } else { "Tool" }.into(), tool.title.to_string())
        }
    };
    (label, single_line(&detail))
}

/// The complete call the header cuts to one line, when there is more to it.
fn invocation(tool: &Tool, view: &ToolView) -> Option<String> {
    let input = tool.input.as_ref();
    let text = match view {
        ToolView::Shell { cmd } => cmd.trim().to_string(),
        ToolView::Search => {
            let pattern = str_field(input, &["pattern", "query", "regex", "glob", "q"])?;
            match str_field(input, &["path", "include", "dir"]) {
                Some(path) => format!("{pattern}\nin {path}"),
                None => pattern,
            }
        }
        ToolView::Web { .. } => {
            let target = str_field(input, &["url", "query", "q"])?;
            match str_field(input, &["prompt"]) {
                Some(prompt) => format!("{target}\n{prompt}"),
                None => target,
            }
        }
        ToolView::File if tool.kind == ToolKind::Edit && tool.diff.is_some() => return None,
        ToolView::File => {
            let path = tool_path(tool)?;
            let range =
                match (input.get("offset").and_then(|v| v.as_u64()), input.get("limit").and_then(|v| v.as_u64())) {
                    (Some(o), Some(l)) => format!(" (lines {}–{})", o.max(1), o.max(1) + l.saturating_sub(1)),
                    (None, Some(l)) => format!(" (first {l} lines)"),
                    _ => String::new(),
                };
            format!("{path}{range}")
        }
        ToolView::Mcp { .. } | ToolView::Generic | ToolView::Browser { .. } => pretty_json(input),
        ToolView::Agent { .. } | ToolView::Todo(_) => return None,
    };
    (!text.trim().is_empty()).then_some(text)
}

/// Zeron's glyph for a tool, at the branch tip.
fn tool_glyph(tool: &Tool, view: &ToolView) -> &'static str {
    let name = tool.name.to_ascii_lowercase();
    match view {
        ToolView::Browser { .. } => icons::MONITOR,
        ToolView::Mcp { .. } => icons::WIDGET,
        ToolView::Todo(_) => icons::CHECKLIST,
        _ => match tool.kind {
            ToolKind::Execute => icons::TERMINAL,
            ToolKind::Edit if name.contains("write") || name.contains("create") => icons::DOCUMENT_ADD,
            ToolKind::Edit => icons::PEN,
            ToolKind::Read => icons::DOCUMENT,
            ToolKind::Search if name.contains("glob") || name.contains("find") || name.contains("list") => {
                icons::FOLDER_WITH_FILES
            }
            ToolKind::Search => icons::MAGNIFER,
            ToolKind::Fetch => icons::GLOBAL,
            ToolKind::Agent => icons::BOT,
            ToolKind::Think | ToolKind::Other => icons::WIDGET,
        },
    }
}

/// The file a read or edit touched.
fn tool_path(tool: &Tool) -> Option<String> {
    str_field(tool.input.as_ref(), &["file_path", "path", "filePath", "notebook_path", "target_file", "filename"])
        .or_else(|| tool.diff.as_ref().map(|d| d.path.clone()))
        .or_else(|| {
            let title = tool.title.trim();
            (matches!(tool.kind, ToolKind::Read | ToolKind::Edit) && markdown::looks_like_path(title))
                .then(|| title.to_string())
        })
}

/// "+12 more": unfolds the whole list.
fn more_chip(i: usize, j: usize, label: String, weak: &WeakEntity<TaskView>, t: &Theme) -> AnyElement {
    let weak = weak.clone();
    div()
        .id(SharedString::from(format!("tool-more-list-{i}-{j}")))
        .h(px(22.0))
        .px(px(6.0))
        .flex()
        .items_center()
        .text_size(px(12.0))
        .text_color(t.text_faint)
        .cursor_pointer()
        .hover(|s| s.text_color(t.text_muted))
        .on_click(move |_, _, cx| {
            let _ = weak.update(cx, |v, cx| v.toggle_full(i, j, cx));
        })
        .child(label)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dove_core::proto::Question;

    fn text(s: &str) -> AgentEvent {
        AgentEvent::TextDelta { text: s.into() }
    }

    #[test]
    fn activity_summaries_read_naturally() {
        let mut a = Activity { thinking: "hmm".into(), ..Default::default() };
        let tool = |kind| Tool {
            id: String::new(),
            title: "".into(),
            kind,
            output: None,
            error: false,
            diff: None,
            open: None,
            full: false,
            name: "".into(),
            input: Arc::new(serde_json::Value::Null),
            started: None,
            took: None,
        };
        a.tools.push(tool(ToolKind::Execute));
        a.tools.push(tool(ToolKind::Execute));
        a.tools.push(tool(ToolKind::Execute));
        a.tools.push(tool(ToolKind::Other));
        assert_eq!(a.summary(), "Thought process · ran 3 commands · called 1 tool");
        let b = Activity { tools: vec![tool(ToolKind::Edit)], ..Default::default() };
        assert_eq!(b.summary(), "Edited 1 file");
        a.thought = Some(Duration::from_secs(12));
        assert!(a.summary().starts_with("Thought for 12s"));
        a.tools[0].error = true;
        assert!(a.summary().ends_with(" · 1 failed"));
        // Two edits of one file are one edited file.
        let mut edit = tool(ToolKind::Edit);
        edit.input = Arc::new(serde_json::json!({ "file_path": "src/a.rs" }));
        let c = Activity { tools: vec![edit.clone(), edit, tool(ToolKind::Read)], ..Default::default() };
        assert_eq!(c.summary(), "Edited 1 file · read 1 file");
    }

    fn line(th: &Thought, n: usize) -> String {
        th.detail.lines[n].iter().map(|r| r.text.as_str()).collect::<String>().trim().to_string()
    }

    #[test]
    fn thoughts_stream_as_markdown_and_split_at_tool_calls() {
        let mut chat = Chat::default();
        chat.apply(AgentEvent::UserMessage { text: "hi".into() }, true);
        let think = |s: &str| AgentEvent::ThinkingDelta { text: s.into() };
        chat.apply(think("**Plan"), true);
        let Some(Row::Activity(a)) = chat.rows.last() else { panic!("no activity") };
        assert_eq!(a.thoughts.len(), 1);
        assert!(a.thoughts[0].live);
        // The half-typed bold is mended: no raw markers reach the row.
        assert_eq!(line(&a.thoughts[0], 0), "Plan");
        assert!(a.thoughts[0].detail.lines[0].iter().any(|r| r.style.bold));
        chat.apply(think("ning**\n\nNext I read the file."), true);
        chat.apply(
            AgentEvent::ToolCall {
                call_id: "c1".into(),
                name: "Read".into(),
                kind: ToolKind::Read,
                title: "src/main.rs".into(),
                input: serde_json::json!({ "file_path": "src/main.rs" }),
            },
            true,
        );
        chat.apply(think("Now edit."), true);
        let Some(Row::Activity(a)) = chat.rows.last() else { panic!("no activity") };
        assert_eq!(a.thoughts.len(), 2, "a tool call ends a thought");
        assert!(!a.thoughts[0].live);
        // Two blocks: a line each with a blank separator between.
        assert_eq!(a.thoughts[0].detail.lines.len(), 3);
        assert_eq!(line(&a.thoughts[0], 0), "Planning");
        assert_eq!(line(&a.thoughts[0], 2), "Next I read the file.");
        assert_eq!((a.thoughts[0].at, a.thoughts[1].at), (0, 1));
        assert_eq!(a.thinking, "**Planning**\n\nNext I read the file.Now edit.");
        chat.apply(text("Done."), true);
        let Some(Row::Activity(a)) = chat.rows.iter().find(|r| matches!(r, Row::Activity(_))) else {
            panic!("no activity")
        };
        assert!(a.thoughts.iter().all(|th| !th.live), "the answer closes the thought");
    }

    #[test]
    fn chips_name_tools_like_zeron() {
        let tool = |kind, name: &str, input: serde_json::Value| Tool {
            id: String::new(),
            title: "t".into(),
            kind,
            output: None,
            error: false,
            diff: None,
            open: None,
            full: false,
            name: name.to_string().into(),
            input: Arc::new(input),
            started: None,
            took: None,
        };
        let run = tool(ToolKind::Execute, "Bash", serde_json::json!({ "command": "cargo  test\n--all" }));
        assert_eq!(chip_content(&run, &ToolView::of(&run)), ("Run".into(), "cargo test --all".into()));
        let read = tool(ToolKind::Read, "Read", serde_json::json!({ "file_path": "a/b.rs", "offset": 10, "limit": 5 }));
        assert_eq!(chip_content(&read, &ToolView::of(&read)).0, "Read");
        assert_eq!(invocation(&read, &ToolView::of(&read)).as_deref(), Some("a/b.rs (lines 10–14)"));
        let glob = tool(ToolKind::Search, "Glob", serde_json::json!({ "pattern": "**/*.rs" }));
        assert_eq!(chip_content(&glob, &ToolView::of(&glob)), ("Glob".into(), "**/*.rs".into()));
        let mut edit = tool(ToolKind::Edit, "Edit", serde_json::json!({ "file_path": "x.rs" }));
        assert!(!edit.opens_by_default());
        edit.diff = Some(Arc::new(MiniDiff::new("x.rs", "a\n", "b\n")));
        assert!(edit.opens_by_default(), "edits unfold into their diff");
        assert_eq!(invocation(&edit, &ToolView::of(&edit)), None);
    }

    #[test]
    fn streamed_answer_becomes_one_row_per_block() {
        let mut chat = Chat::default();
        chat.apply(AgentEvent::UserMessage { text: "hi".into() }, true);
        for chunk in ["# Title\n\nFirst para", "graph.\n\n- a\n- b\n\n", "Last"] {
            chat.apply(text(chunk), true);
        }
        // Heading + paragraph finished; the list is the open tail's
        // predecessor once "Last" started a new block.
        let finished = chat.rows.iter().filter(|r| matches!(r, Row::Block { .. })).count();
        assert_eq!(finished, 3);
        assert!(matches!(chat.rows.last(), Some(Row::Live { .. })));
        chat.apply(AgentEvent::TurnEnded { usage: None, is_error: false }, true);
        let blocks = chat.rows.iter().filter(|r| matches!(r, Row::Block { .. })).count();
        assert_eq!(blocks, 4);
        assert!(!chat.rows.iter().any(|r| matches!(r, Row::Live { .. })));
        assert!(matches!(chat.rows.last(), Some(Row::TurnEnd { msgs, .. }) if *msgs == (0..1)));
        assert_eq!(chat.messages[0], "# Title\n\nFirst paragraph.\n\n- a\n- b\n\nLast");
    }

    #[test]
    fn replay_matches_live_rows_and_highlights() {
        let events = vec![
            AgentEvent::UserMessage { text: "q".into() },
            text("Here:\n\n```rust\nfn main() {}\n```\n\nDone."),
            AgentEvent::ToolCall {
                call_id: "c1".into(),
                name: "edit".into(),
                kind: ToolKind::Edit,
                title: "src/a.rs".into(),
                input: serde_json::json!({"path": "src/a.rs", "old_string": "a\nb\n", "new_string": "a\nc\n"}),
            },
            AgentEvent::ToolResult { call_id: "c1".into(), output: "ok".into(), is_error: false },
            AgentEvent::TurnEnded { usage: None, is_error: false },
        ];
        let chat = Chat::replay(events);
        let code = chat.rows.iter().find_map(|r| match r {
            Row::Block { block, .. } => match &**block {
                Block::Code { highlight, .. } => Some(highlight.is_some()),
                _ => None,
            },
            _ => None,
        });
        assert_eq!(code, Some(true));
        let Some(Row::Activity(a)) = chat.rows.iter().find(|r| matches!(r, Row::Activity(_))) else {
            panic!("activity")
        };
        let diff = a.tools[0].diff.as_ref().expect("diff");
        assert_eq!((diff.added, diff.removed), (1, 1));
        assert!(chat.dirty.is_none());
    }

    #[test]
    fn dirty_tracks_the_lowest_changed_row() {
        let mut chat = Chat::default();
        chat.apply(AgentEvent::UserMessage { text: "q".into() }, true);
        chat.apply(text("a"), true);
        chat.dirty = None;
        chat.apply(text("b"), true);
        assert_eq!(chat.dirty, Some(1));
    }

    #[test]
    fn context_usage_keeps_previous_fields() {
        let mut chat = Chat::default();
        chat.apply(AgentEvent::ContextUsage { tokens: Some(10), window: Some(200) }, true);
        chat.apply(AgentEvent::ContextUsage { tokens: Some(50), window: None }, true);
        assert_eq!(chat.context, (Some(50), Some(200)));
    }

    #[test]
    fn pending_question_is_found_until_resolved() {
        let mut chat = Chat::default();
        let req = PermissionRequest {
            request_id: "r".into(),
            tool: "Question".into(),
            summary: "Pick".into(),
            input: serde_json::Value::Null,
            questions: vec![Question {
                id: "q".into(),
                header: "Approach".into(),
                question: "Which?".into(),
                options: vec!["A".into(), "B".into()],
                multi_select: false,
            }],
        };
        chat.apply(AgentEvent::PermissionRequested(req), true);
        assert!(chat.pending().is_some_and(|(_, r)| r.is_question()));
        chat.apply(
            AgentEvent::PermissionResolved { request_id: "r".into(), decision: PermissionDecision::AllowOnce },
            true,
        );
        assert!(chat.pending().is_none());
    }

    #[test]
    fn risky_commands_are_flagged() {
        let req = |s: &str| PermissionRequest {
            request_id: String::new(),
            tool: "Bash".into(),
            summary: s.into(),
            input: serde_json::Value::Null,
            questions: Vec::new(),
        };
        let root = std::path::Path::new("C:/work/repo");
        assert_eq!(risk_hints(&req("rm -rf build"), Some(root)), vec!["Deletes files"]);
        assert_eq!(risk_hints(&req("curl https://x.dev"), Some(root)), vec!["Network"]);
        assert_eq!(risk_hints(&req("cat C:/other/file.txt"), Some(root)), vec!["Outside the worktree"]);
        assert!(risk_hints(&req("cat C:/work/repo/src/a.rs"), Some(root)).is_empty());
    }

    #[test]
    fn durations_are_short() {
        assert_eq!(duration(Duration::from_secs(5)), "5s");
        assert_eq!(duration(Duration::from_secs(72)), "1m 12s");
        assert_eq!(duration(Duration::from_secs(3720)), "1h 2m");
    }
}
