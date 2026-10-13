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
use dove_core::{Command, Engine, LiveSession};
use gpui::prelude::FluentBuilder as _;
use gpui::{Anchor, anchored, deferred};
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter,
    FocusHandle, Focusable, FollowMode, InteractiveElement as _, IntoElement, KeyDownEvent, ListAlignment, ListOffset,
    ListState, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    WeakEntity, Window, div, list, px,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::composer::{self, Composer, ComposerEvent};
use crate::kit;
use crate::markdown::{self, Block, Stream};
use crate::theme::Theme;
use crate::ui;

const FRAME: Duration = Duration::from_millis(8);
const OUTPUT_TAIL: usize = 3;
const OUTPUT_OPEN: usize = 40;
const DIFF_PREVIEW: usize = 8;
const DIFF_MAX: usize = 120;
/// Steps of a live activity shown before the rest fold away.
const LIVE_STEPS: usize = 4;
pub const COLUMN_MAX: f32 = 760.0;

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
}

// ------------------------------------------------------------------ model

/// A tool call inside an activity fold.
#[derive(Clone)]
pub struct Tool {
    /// The harness's call id (tests and debugging).
    #[allow(dead_code)]
    pub id: String,
    pub title: SharedString,
    pub kind: ToolKind,
    pub output: Option<SharedString>,
    pub error: bool,
    pub diff: Option<Arc<MiniDiff>>,
    pub open: bool,
    /// The harness's tool name (`Bash`, `mcp__server__tool`, ...).
    pub name: SharedString,
    pub input: Arc<serde_json::Value>,
    started: Option<Instant>,
    /// Wall time from call to result, for live calls.
    pub took: Option<Duration>,
}

/// A compact line diff for an edit, capped for display.
#[derive(Debug, Clone, PartialEq)]
pub struct MiniDiff {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    /// `('+'|'-'|' ', text)`.
    pub lines: Vec<(char, String)>,
    pub truncated: bool,
}

impl MiniDiff {
    pub fn new(path: &str, old: &str, new: &str) -> Self {
        let diff = similar::TextDiff::from_lines(old, new);
        let (mut added, mut removed, mut lines) = (0, 0, Vec::new());
        let mut truncated = false;
        for group in diff.grouped_ops(1) {
            for op in group {
                for change in diff.iter_changes(&op) {
                    let tag = match change.tag() {
                        similar::ChangeTag::Insert => {
                            added += 1;
                            '+'
                        }
                        similar::ChangeTag::Delete => {
                            removed += 1;
                            '-'
                        }
                        similar::ChangeTag::Equal => ' ',
                    };
                    if lines.len() < DIFF_MAX {
                        lines.push((tag, change.value().trim_end_matches(['\n', '\r']).to_string()));
                    } else {
                        truncated = true;
                    }
                }
            }
        }
        Self { path: path.to_string(), added, removed, lines, truncated }
    }
}

#[derive(Default, Clone)]
pub struct Activity {
    pub thinking: String,
    pub tools: Vec<Tool>,
    /// The user's choice; `None` follows the default (open while live).
    pub pinned: Option<bool>,
    pub live: bool,
    started: Option<Instant>,
    thought: Option<Duration>,
}

impl Activity {
    fn expanded(&self) -> bool {
        self.pinned.unwrap_or(false)
    }

    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.thinking.trim().is_empty() {
            parts.push(match self.thought {
                Some(d) if d.as_secs() >= 1 => format!("Thought for {}", duration(d)),
                _ => "Thought process".into(),
            });
        }
        let count = |k: &[ToolKind]| self.tools.iter().filter(|t| k.contains(&t.kind)).count();
        let plural = |n: usize, one: &str, many: &str| if n == 1 { one.to_string() } else { many.to_string() };
        let ran = count(&[ToolKind::Execute]);
        if ran > 0 {
            parts.push(format!("Ran {ran} {}", plural(ran, "command", "commands")));
        }
        let edited = count(&[ToolKind::Edit]);
        if edited > 0 {
            parts.push(format!("edited {edited} {}", plural(edited, "file", "files")));
        }
        let read = count(&[ToolKind::Read, ToolKind::Search]);
        if read > 0 {
            parts.push(format!("explored {read} {}", plural(read, "item", "items")));
        }
        let fetched = count(&[ToolKind::Fetch]);
        if fetched > 0 {
            parts.push(format!("searched the web {fetched}×"));
        }
        let agents = count(&[ToolKind::Agent]);
        if agents > 0 {
            parts.push(format!("ran {agents} {}", plural(agents, "subagent", "subagents")));
        }
        let other = count(&[ToolKind::Other, ToolKind::Think]);
        if other > 0 {
            parts.push(format!("called {other} {}", plural(other, "tool", "tools")));
        }
        if parts.is_empty() {
            return "Working".into();
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
}

/// A thread's transcript as plain data.
#[derive(Default)]
pub struct Chat {
    pub rows: Vec<Row>,
    /// Source text of each assistant message, for copy.
    pub messages: Vec<String>,
    open: Option<Open>,
    open_activity: Option<usize>,
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

    fn close_activity(&mut self) {
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
                let text: SharedString = text.to_string().into();
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
                if let Row::Activity(a) = &mut self.rows[i] {
                    a.thinking.push_str(&text);
                }
                self.touch(i);
            }
            AgentEvent::BlockEnd => self.close_text(),
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
                        open: false,
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

pub struct TaskView {
    engine: Arc<Engine>,
    pub task: Task,
    pub project: Project,
    live: Option<LiveSession>,
    chat: Chat,
    loading: bool,
    list: ListState,
    following: std::rc::Rc<std::cell::Cell<bool>>,
    /// First visible row, kept by the scroll handler (drives the rail).
    top_row: std::rc::Rc<std::cell::Cell<usize>>,
    /// Hovered rail mark.
    rail_hover: Option<usize>,
    /// The pointer is over the conversation (shows the scroll thumb).
    chat_hovered: bool,
    /// A user message just jumped to, briefly highlighted.
    flash: Option<usize>,
    glide: Option<gpui::Task<()>>,
    pub composer: Entity<Composer>,
    pub status: TaskStatus,
    widgets: HashMap<String, crate::widget::State>,
    todos_open: bool,
    todos_hidden: bool,
    questions: Option<Questions>,
    /// The agent's TUI owns the session; the chat is read-only.
    pub terminal_banner: bool,
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
        let top = top_row.clone();
        list.set_scroll_handler(move |ev, _, cx| {
            let follow_changed = flag.get() != ev.is_following_tail;
            let top_changed = top.get() != ev.visible_range.start;
            flag.set(ev.is_following_tail);
            top.set(ev.visible_range.start);
            if follow_changed || top_changed {
                let _ = weak.update(cx, |_, cx| cx.notify());
            }
        });
        let mut chat = Chat::default();
        chat.usage = engine.store.task_usage(task.id).unwrap_or_default();
        chat.rate_limits = engine.rate_limits(task.harness);
        let mut this = Self {
            engine,
            status: task.status,
            task,
            project,
            live: None,
            chat,
            loading: true,
            list,
            following,
            top_row,
            rail_hover: None,
            chat_hovered: false,
            flash: None,
            glide: None,
            composer,
            widgets: HashMap::new(),
            todos_open: true,
            todos_hidden: false,
            questions: None,
            terminal_banner: false,
            focus: cx.focus_handle(),
            _ticker: None,
            _pump: None,
            _subs: vec![sub],
        };
        this.load_transcript(live, cx);
        this
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
                v.chat = chat;
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
            if fx.first_answer {
                if let Some(t) = self.auto_title() {
                    cx.emit(TaskViewEvent::Title(t));
                }
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
        if let Some(s) = status {
            if s != self.status {
                self.status = s;
                let running = s == TaskStatus::Running;
                self.composer.update(cx, |c, cx| c.set_running(running && self.live.is_some(), cx));
                cx.emit(TaskViewEvent::Status(s));
            }
        }
        // A queued message goes out as soon as the turn ends.
        if matches!(status, Some(TaskStatus::Done | TaskStatus::Failed)) {
            if let Some(next) = self.composer.update(cx, |c, cx| c.take_queued(cx)) {
                if let Some(l) = &self.live {
                    l.send(Command::Send(next));
                }
            }
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
                    if let Some(Row::Block { block, .. }) = v.chat.rows.get_mut(i) {
                        if Arc::ptr_eq(block, &old) {
                            *block = new;
                        }
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
            a.pinned = Some(!a.expanded());
            self.list.remeasure_items(i..i + 1);
            cx.notify();
        }
    }

    fn toggle_tool(&mut self, i: usize, j: usize, cx: &mut Context<Self>) {
        if let Some(Row::Activity(a)) = self.chat.rows.get_mut(i) {
            if let Some(t) = a.tools.get_mut(j) {
                t.open = !t.open;
            }
            self.list.remeasure_items(i..i + 1);
            cx.notify();
        }
    }

    pub fn scroll_to_row(&mut self, i: usize, cx: &mut Context<Self>) {
        self.list.set_tail_reservation(None);
        self.list.scroll_to_reveal_item(i);
        cx.notify();
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
            if let Some(n) = k.key.parse::<usize>().ok().filter(|n| (1..=9).contains(n)) {
                if let Some(label) = req.questions.get(q.page).and_then(|qq| qq.options.get(n - 1)).cloned() {
                    self.pick_option(label, window, cx);
                    cx.stop_propagation();
                }
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
                let copyable = !matches!(&**block, Block::Code { .. } | Block::Rule | Block::Widget(_));
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
        div()
            .group("user-msg")
            .w_full()
            .flex()
            .flex_col()
            .items_end()
            .gap(px(2.0))
            .pt(px(10.0))
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
                    .child(edit)
                    .child(copy_button(SharedString::from(format!("copy-user-{i}")), copy, t)),
            )
            .into_any_element()
    }

    fn render_md(&self, id: &str, block: &Block, weak: &WeakEntity<Self>, t: &Theme, window: &Window) -> AnyElement {
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

    fn render_activity(&self, i: usize, a: &Activity, weak: &WeakEntity<Self>, t: &Theme) -> AnyElement {
        let label = a.summary();
        let failed = a.failed();
        let expanded = a.expanded();
        // While live the last few steps show; once done everything folds.
        let shown: Vec<usize> = if expanded {
            (0..a.tools.len()).collect()
        } else if a.live {
            (a.tools.len().saturating_sub(LIVE_STEPS)..a.tools.len()).collect()
        } else {
            Vec::new()
        };
        let hidden = if a.live && !expanded { a.tools.len().saturating_sub(LIVE_STEPS) } else { 0 };
        let header = {
            let weak = weak.clone();
            div()
                .id(("activity", i))
                .h(px(28.0))
                .px(px(8.0))
                .mx(px(-8.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .rounded(px(8.0))
                .text_size(px(13.0))
                .text_color(t.text_faint)
                .cursor_pointer()
                .hover(|s| s.bg(t.wash(0.06)).text_color(t.text_muted))
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |v, cx| v.toggle_activity(i, cx));
                })
                .child(
                    icon(if expanded { icons::ALT_ARROW_DOWN } else { icons::ALT_ARROW_RIGHT })
                        .size(px(12.0))
                        .text_color(t.text_faint),
                )
                .when(a.live, |d| d.child(pulse(("act-pulse", i), t.busy)))
                .child(label)
                .when(failed > 0, |d| {
                    d.child(
                        div()
                            .px(px(6.0))
                            .rounded(px(5.0))
                            .bg(t.danger.opacity(0.10))
                            .text_color(t.danger)
                            .text_size(px(11.0))
                            .child(format!("{failed} failed")),
                    )
                })
        };
        let thinking = (!a.thinking.trim().is_empty() && (expanded || (a.live && a.tools.is_empty()))).then(|| {
            let text = a.thinking.trim();
            let text: String = if expanded { text.to_string() } else { last_lines(text, 3) };
            div().text_size(px(13.0)).italic().text_color(t.text_faint).child(text)
        });
        let steps = (!shown.is_empty() || thinking.is_some()).then(|| {
            div()
                .ml(px(5.0))
                .pl(px(13.0))
                .border_l_1()
                .border_color(if failed > 0 { t.danger.opacity(0.35) } else { t.border })
                .flex()
                .flex_col()
                .gap(px(8.0))
                .children(thinking)
                .when(hidden > 0, |d| {
                    d.child(div().text_size(px(12.0)).text_color(t.text_faint).child(format!("{hidden} earlier steps")))
                })
                .children(shown.into_iter().map(|j| self.render_tool(i, j, &a.tools[j], weak, t)))
        });
        div().flex().flex_col().gap(px(8.0)).child(header).children(steps).into_any_element()
    }

    fn render_tool(&self, i: usize, j: usize, tool: &Tool, weak: &WeakEntity<Self>, t: &Theme) -> AnyElement {
        let view = ToolView::of(tool);
        let running = tool.output.is_none();
        let glyph = match &view {
            ToolView::Browser { .. } => icons::MONITOR,
            ToolView::Mcp { .. } => icons::WIDGET,
            ToolView::Todo(_) => icons::CHECKLIST,
            _ => match tool.kind {
                ToolKind::Execute => icons::TERMINAL,
                ToolKind::Edit => icons::PEN,
                ToolKind::Read => icons::DOCUMENT,
                ToolKind::Search => icons::MAGNIFER,
                ToolKind::Fetch => icons::GLOBE,
                ToolKind::Agent => icons::BOT,
                ToolKind::Think => icons::BRAIN,
                ToolKind::Other => icons::SETTINGS_MINIMALISTIC,
            },
        };
        let status: AnyElement = if running {
            kit::spinner(SharedString::from(format!("spin-{i}-{j}")), 13.0, t.text_muted).into_any_element()
        } else if tool.error {
            icon(icons::CLOSE_CIRCLE).size(px(14.0)).text_color(t.danger).into_any_element()
        } else {
            icon(glyph).size(px(14.0)).text_color(t.text_faint).into_any_element()
        };
        let toggle = {
            let weak = weak.clone();
            move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |v, cx| v.toggle_tool(i, j, cx));
            }
        };
        let path_like = matches!(tool.kind, ToolKind::Read | ToolKind::Edit) && !tool.title.is_empty();
        let mono = matches!(view, ToolView::Shell { .. } | ToolView::Search)
            || matches!(tool.kind, ToolKind::Read | ToolKind::Edit);
        let subject: AnyElement = match &view {
            ToolView::Browser { action, target } => div()
                .flex_1()
                .min_w_0()
                .flex()
                .gap(px(6.0))
                .child(div().flex_none().text_color(t.text_muted).child(action.clone()))
                .child(div().min_w_0().truncate().text_color(t.text_faint).child(target.clone()))
                .into_any_element(),
            ToolView::Mcp { server, tool: name } => div()
                .flex_1()
                .min_w_0()
                .flex()
                .gap(px(6.0))
                .child(div().flex_none().text_color(t.text_faint).child(server.clone()))
                .child(div().flex_none().text_color(t.text_faint).child("·"))
                .child(div().min_w_0().truncate().text_color(t.text_muted).child(name.clone()))
                .into_any_element(),
            ToolView::Web { query } => {
                div().flex_1().min_w_0().truncate().text_color(t.text_muted).child(query.clone()).into_any_element()
            }
            _ => {
                let el = div()
                    .id(SharedString::from(format!("tool-{i}-{j}")))
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(t.text_muted)
                    .when(mono, |d| d.font_family(t.font_mono.clone()).text_size(px(12.5)))
                    .child(match &view {
                        ToolView::Shell { cmd } => SharedString::from(first_line(cmd)),
                        _ => tool.title.clone(),
                    });
                if path_like {
                    let path = tool.title.to_string();
                    el.cursor_pointer()
                        .hover(|s| s.text_color(t.text).underline())
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            let (path, line) = markdown::split_path_line(&path);
                            window.dispatch_action(Box::new(crate::actions::OpenPath { path, line }), cx);
                        })
                        .into_any_element()
                } else {
                    el.into_any_element()
                }
            }
        };
        let expandable = tool_expandable(tool, &view);
        let head = div()
            .id(SharedString::from(format!("tool-head-{i}-{j}")))
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(px(13.0))
            .when(expandable, |d| d.cursor_pointer().on_click(toggle.clone()))
            .child(div().flex_none().size(px(14.0)).flex().items_center().justify_center().child(status))
            .child(subject)
            .when_some(tool.diff.as_ref(), |d, diff| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .gap(px(4.0))
                        .text_size(px(11.5))
                        .font_family(t.font_mono.clone())
                        .child(div().text_color(t.text_muted).child(format!("+{}", diff.added)))
                        .child(div().text_color(t.text_faint).child(format!("−{}", diff.removed))),
                )
            })
            .when(tool.error && !running, |d| {
                d.child(div().flex_none().text_size(px(11.5)).text_color(t.danger).child("failed"))
            })
            .when_some(tool.took.filter(|d| d.as_millis() >= 100), |d, took| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(px(11.5))
                        .font_family(t.font_mono.clone())
                        .text_color(t.text_faint)
                        .child(short_duration(took)),
                )
            })
            .when(expandable, |d| {
                d.child(
                    icon(if tool.open { icons::ALT_ARROW_DOWN } else { icons::ALT_ARROW_RIGHT })
                        .size(px(11.0))
                        .text_color(t.text_faint),
                )
            });

        let body: Option<AnyElement> = match &view {
            _ if tool.kind == ToolKind::Edit && tool.diff.is_some() => {
                tool.diff.as_ref().map(|diff| self.render_mini_diff(i, j, diff, tool.open, weak, t))
            }
            ToolView::Todo(items) => Some(todo_list(items, t)),
            ToolView::Shell { cmd } => {
                let out = tool.output.as_deref().unwrap_or("");
                let multi = cmd.contains('\n') || cmd.chars().count() > 72;
                (tool.open || !out.trim().is_empty() || multi).then(|| {
                    let mut text = String::new();
                    if tool.open || multi {
                        text.push_str("$ ");
                        text.push_str(cmd.trim());
                        if !out.trim().is_empty() {
                            text.push('\n');
                        }
                    }
                    let (shown, more) = tail(out, if tool.open { OUTPUT_OPEN } else { OUTPUT_TAIL });
                    text.push_str(&shown);
                    output_box(("out", i, j), text, more, tool, toggle.clone(), t)
                })
            }
            ToolView::Search => tool.output.as_deref().filter(|o| !o.trim().is_empty()).map(|out| {
                let paths = path_lines(out);
                if paths.is_empty() || tool.error {
                    let (shown, more) = tail(out, if tool.open { OUTPUT_OPEN } else { OUTPUT_TAIL });
                    return output_box(("out", i, j), shown, more, tool, toggle.clone(), t);
                }
                let keep = if tool.open { 40 } else { 6 };
                let more = paths.len().saturating_sub(keep);
                div()
                    .ml(px(22.0))
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
                    .when(more > 0, |d| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("hits-more-{i}-{j}")))
                                .h(px(22.0))
                                .px(px(6.0))
                                .flex()
                                .items_center()
                                .text_size(px(12.0))
                                .text_color(t.text_faint)
                                .cursor_pointer()
                                .hover(|s| s.text_color(t.text_muted))
                                .on_click(toggle.clone())
                                .child(format!("+{more} more")),
                        )
                    })
                    .into_any_element()
            }),
            ToolView::Web { .. } => tool.output.as_deref().filter(|o| !o.trim().is_empty()).map(|out| {
                let urls = urls_in(out);
                if urls.is_empty() || tool.error {
                    let (shown, more) = tail(out, if tool.open { OUTPUT_OPEN } else { OUTPUT_TAIL });
                    return output_box(("out", i, j), shown, more, tool, toggle.clone(), t);
                }
                let keep = if tool.open { 20 } else { 4 };
                let more = urls.len().saturating_sub(keep);
                div()
                    .ml(px(22.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .children(
                        urls.into_iter().take(keep).enumerate().map(|(k, u)| web_row(("web", i * 10_000 + j, k), u, t)),
                    )
                    .when(more > 0, |d| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("web-more-{i}-{j}")))
                                .px(px(6.0))
                                .text_size(px(12.0))
                                .text_color(t.text_faint)
                                .cursor_pointer()
                                .hover(|s| s.text_color(t.text_muted))
                                .on_click(toggle.clone())
                                .child(format!("+{more} more results")),
                        )
                    })
                    .into_any_element()
            }),
            ToolView::Mcp { .. } => tool.open.then(|| {
                let args = pretty_json(&tool.input);
                let result = tool.output.as_deref().map(|o| {
                    serde_json::from_str::<serde_json::Value>(o)
                        .map(|v| pretty_json(&v))
                        .unwrap_or_else(|_| o.to_string())
                });
                div()
                    .ml(px(22.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .when(!args.is_empty(), |d| d.child(json_box("Arguments", args, false, t)))
                    .when_some(result.filter(|r| !r.trim().is_empty()), |d, r| {
                        d.child(json_box("Result", r, tool.error, t))
                    })
                    .into_any_element()
            }),
            ToolView::Agent { prompt } => {
                let out = tool.output.as_deref().unwrap_or("").trim();
                (!prompt.is_empty() || !out.is_empty()).then(|| {
                    let keep = if tool.open { OUTPUT_OPEN } else { 2 };
                    let (p, _) = head_lines(prompt, keep);
                    let (o, more) = tail(out, if tool.open { OUTPUT_OPEN } else { OUTPUT_TAIL });
                    div()
                        .id(SharedString::from(format!("agent-{i}-{j}")))
                        .ml(px(22.0))
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .text_size(px(12.5))
                        .cursor_pointer()
                        .on_click(toggle.clone())
                        .when(!p.is_empty(), |d| d.child(div().italic().text_color(t.text_faint).child(p)))
                        .when(!o.is_empty(), |d| {
                            d.child(
                                div()
                                    .pl(px(10.0))
                                    .border_l_1()
                                    .border_color(t.border)
                                    .text_color(if tool.error { t.danger } else { t.text_muted })
                                    .when(more > 0 && !tool.open, |d| {
                                        d.child(div().text_color(t.text_faint).child(format!("… {more} more lines")))
                                    })
                                    .child(o),
                            )
                        })
                        .into_any_element()
                })
            }
            ToolView::Browser { .. } | ToolView::Generic | ToolView::File => {
                let out = tool.output.as_deref().unwrap_or("");
                let show = tool.open || tool.error || matches!(view, ToolView::Generic) && tool.kind != ToolKind::Read;
                (show && !out.trim().is_empty()).then(|| {
                    let (shown, more) = tail(out, if tool.open { OUTPUT_OPEN } else { OUTPUT_TAIL });
                    output_box(("out", i, j), shown, more, tool, toggle.clone(), t)
                })
            }
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(8.0))
            .hover(|s| s.bg(t.wash(0.03)))
            .child(head)
            .children(body)
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
        let take = if open { diff.lines.len() } else { DIFF_PREVIEW };
        let more = diff.lines.len().saturating_sub(take) + if diff.truncated { 1 } else { 0 };
        let toggle = weak.clone();
        let changes = weak.clone();
        div()
            .ml(px(22.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(t.border)
            .overflow_hidden()
            .font_family(t.font_mono.clone())
            .text_size(px(12.0))
            .line_height(px(18.0))
            .children(diff.lines.iter().take(take).map(|(tag, text)| {
                let (bg, fg) = match tag {
                    '+' => (t.success.opacity(0.10), t.text),
                    '-' => (t.danger.opacity(0.10), t.text_muted),
                    _ => (gpui::transparent_black(), t.text_faint),
                };
                div()
                    .flex()
                    .bg(bg)
                    .text_color(fg)
                    .child(div().w(px(18.0)).flex_none().text_center().text_color(t.text_faint).child(tag.to_string()))
                    .child(div().flex_1().min_w_0().truncate().pr(px(8.0)).child(if text.is_empty() {
                        " ".to_string()
                    } else {
                        text.replace('\t', "    ")
                    }))
            }))
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
                                    let _ = toggle.update(cx, |v, cx| v.toggle_tool(i, j, cx));
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
        let can_fork = last && Engine::can_fork(&self.task);
        let idle = !self.is_working();
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
        let elapsed = self.chat.turn_started.map(|s| duration(s.elapsed()));
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
            .when(working, |d| {
                d.child(pulse("strip-pulse", t.busy))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(t.text_muted)
                            .child(step.unwrap_or_else(|| "Working".into())),
                    )
                    .when_some(elapsed, |d, e| d.child(e))
            })
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

fn last_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// A softly pulsing dot: the "working" glyph.
fn pulse(id: impl Into<gpui::ElementId>, color: gpui::Hsla) -> impl IntoElement {
    ui::dot(color, 6.0).with_animation(id, Animation::new(Duration::from_millis(1200)).repeat(), move |el, delta| {
        let wave = 0.5 - 0.5 * (delta * std::f32::consts::TAU).cos();
        el.opacity(0.35 + 0.65 * wave)
    })
}

fn action_button(id: SharedString, glyph: &'static str, tip: &'static str, t: &Theme) -> gpui::Stateful<gpui::Div> {
    let hover = t.glass_hover();
    div()
        .id(id)
        .size(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(ui::tip(tip))
        .child(icon(glyph).size(px(14.0)).text_color(t.text_faint))
}

fn copy_button(id: SharedString, text: String, t: &Theme) -> impl IntoElement {
    action_button(id, icons::COPY, "Copy", t)
        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone())))
}

impl Render for TaskView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The question panel needs a window to build its input.
        if self.questions.is_none() {
            if let Some((_, req)) = self.chat.pending().filter(|(_, r)| r.is_question() && self.live.is_some()) {
                let req = req.clone();
                self.open_questions(&req, window, cx);
            }
        }
        let t = Theme::of(cx).clone();
        let weak = cx.weak_entity();
        let empty = self.chat.rows.is_empty();

        let transcript: AnyElement = if self.loading {
            div().flex_1().into_any_element()
        } else if empty {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.text_faint)
                .child(if self.live.is_some() { "Starting agent…" } else { "No messages yet" })
                .into_any_element()
        } else {
            let renderer = weak.clone();
            list(self.list.clone(), move |i, window, cx| {
                let Some(view) = renderer.upgrade() else { return div().into_any_element() };
                let v = view.read(cx);
                if i >= v.chat.rows.len() {
                    return div().into_any_element();
                }
                v.render_row(i, &renderer, window, cx)
            })
            .flex_1()
            .size_full()
            .into_any_element()
        };

        let jump = (!self.following.get() && !empty).then(|| {
            div().absolute().bottom(px(10.0)).left_0().right_0().flex().justify_center().child(
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
        let bottom = div().w_full().flex().justify_center().px(px(24.0)).pb(px(8.0)).child(
            div()
                .w_full()
                .max_w(px(COLUMN_MAX + 8.0))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .children(self.render_todos(cx))
                .child(input)
                .child(self.render_status_strip(&t)),
        );

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
                                        .thumb(|th| th.bg(t.text.opacity(0.20)).width(px(5.0)).radius(px(3.0)).min_length(px(40.0)))
                                        .thumb_hover(|th| th.bg(t.text.opacity(0.38)).width(px(7.0)).radius(px(4.0)).min_length(px(40.0)))
                                        .thumb_active(|th| th.bg(t.text.opacity(0.50)).width(px(7.0)).radius(px(4.0)).min_length(px(40.0)))
                                }),
                        )
                    })
                    .children(self.render_rail(cx))
                    .children(jump),
            )
            .child(bottom)
    }
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
            open: false,
            name: "".into(),
            input: Arc::new(serde_json::Value::Null),
            started: None,
            took: None,
        };
        a.tools.push(tool(ToolKind::Execute));
        a.tools.push(tool(ToolKind::Execute));
        a.tools.push(tool(ToolKind::Execute));
        a.tools.push(tool(ToolKind::Other));
        assert_eq!(a.summary(), "Thought process · Ran 3 commands · called 1 tool");
        let b = Activity { tools: vec![tool(ToolKind::Edit)], ..Default::default() };
        assert_eq!(b.summary(), "Edited 1 file");
        a.thought = Some(Duration::from_secs(12));
        assert!(a.summary().starts_with("Thought for 12s"));
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

fn tool_expandable(tool: &Tool, view: &ToolView) -> bool {
    let out = tool.output.as_deref().unwrap_or("");
    match view {
        ToolView::Todo(_) => false,
        ToolView::Mcp { .. } => true,
        ToolView::File if tool.kind == ToolKind::Edit => {
            tool.diff.as_ref().is_some_and(|d| d.lines.len() > DIFF_PREVIEW || d.truncated)
        }
        ToolView::File => false,
        ToolView::Browser { .. } => !out.trim().is_empty(),
        ToolView::Shell { cmd } => out.lines().count() > OUTPUT_TAIL || !cmd.trim().is_empty() && !out.is_empty(),
        _ => out.lines().count() > OUTPUT_TAIL,
    }
}

fn first_line(s: &str) -> String {
    let line = s.trim().lines().next().unwrap_or("").trim();
    if s.trim().lines().nth(1).is_some() { format!("{line} …") } else { line.to_string() }
}

/// `0.4s`, `12.3s`, `1m 12s`.
fn short_duration(d: Duration) -> String {
    if d.as_secs() < 60 { format!("{:.1}s", d.as_secs_f64()) } else { duration(d) }
}

/// The last `keep` lines and how many were dropped.
fn tail(s: &str, keep: usize) -> (String, usize) {
    let lines: Vec<&str> = s.trim_end().lines().collect();
    let more = lines.len().saturating_sub(keep);
    (lines[more..].join("\n"), more)
}

fn head_lines(s: &str, keep: usize) -> (String, usize) {
    let lines: Vec<&str> = s.trim().lines().collect();
    let more = lines.len().saturating_sub(keep);
    let mut out = lines[..lines.len() - more].join("\n");
    if more > 0 {
        out.push_str(" …");
    }
    (out, more)
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
        .child(icon(icons::FILE_CODE).size(px(11.0)).text_color(t.text_faint))
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

fn json_box(label: &'static str, text: String, error: bool, t: &Theme) -> AnyElement {
    let lines: Vec<&str> = text.lines().collect();
    let more = lines.len().saturating_sub(OUTPUT_OPEN);
    let shown = lines[..lines.len() - more].join("\n");
    div()
        .rounded(px(8.0))
        .border_1()
        .border_color(t.border)
        .bg(t.wash(0.03))
        .overflow_hidden()
        .child(div().px(px(10.0)).pt(px(6.0)).text_size(px(11.0)).text_color(t.text_faint).child(label))
        .child(
            div()
                .px(px(10.0))
                .pt(px(2.0))
                .pb(px(8.0))
                .font_family(t.font_mono.clone())
                .text_size(px(12.0))
                .text_color(if error { t.danger } else { t.text_muted })
                .child(shown)
                .when(more > 0, |d| d.child(div().text_color(t.text_faint).child(format!("… {more} more lines")))),
        )
        .into_any_element()
}

fn output_box(
    id: (&'static str, usize, usize),
    text: String,
    more: usize,
    tool: &Tool,
    toggle: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    t: &Theme,
) -> AnyElement {
    div()
        .id(SharedString::from(format!("{}-{}-{}", id.0, id.1, id.2)))
        .ml(px(22.0))
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(8.0))
        .bg(t.wash(0.035))
        .font_family(t.font_mono.clone())
        .text_size(px(12.0))
        .text_color(if tool.error { t.danger } else { t.text_muted })
        .cursor_pointer()
        .on_click(toggle)
        .when(more > 0, |d| {
            d.child(div().text_color(t.text_faint).child(if tool.open {
                format!("… {more} earlier lines")
            } else {
                format!("… {more} more lines · click to expand")
            }))
        })
        .child(text)
        .into_any_element()
}

fn todo_list(items: &[(String, TodoStatus)], t: &Theme) -> AnyElement {
    div()
        .ml(px(22.0))
        .flex()
        .flex_col()
        .gap(px(3.0))
        .text_size(px(13.0))
        .children(items.iter().map(|(text, status)| {
            let mark: AnyElement = match status {
                TodoStatus::Completed => {
                    icon(icons::CIRCLE_CHECK).size(px(13.0)).text_color(t.text_faint).into_any_element()
                }
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
