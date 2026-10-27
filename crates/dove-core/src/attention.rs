//! The attention model: what each live task is doing and what it needs,
//! kept by the engine from the same event stream it records.
//!
//! The sidebar's Needs you, Voice Mode and the notch all read this instead
//! of a thread's view, so they agree with each other and keep working when
//! no view is open (a hibernated thread, a window that's closed). Changes
//! are announced on [`Attention::subscribe`]; nothing needs to poll.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use dove_proto::{AgentEvent, Capabilities, PermissionRequest, TaskId, TaskStatus, TodoItem, ToolKind};
use parking_lot::{Mutex, RwLock};

/// The reply tail keeps this many characters.
const REPLY_MAX: usize = 4000;

/// One task as far as attention goes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskState {
    pub status: TaskStatus,
    /// A session is recording right now.
    pub live: bool,
    pub capabilities: Capabilities,
    /// Undecided permission requests and questions, oldest first.
    pub pending: Vec<PermissionRequest>,
    pub todos: Vec<TodoItem>,
    /// What it's doing now ("Writing main.rs"); cleared between turns.
    pub activity: Option<String>,
    pub last_error: Option<String>,
    /// The latest reply: the assistant's text since the last user message,
    /// its last [`REPLY_MAX`] characters.
    pub reply: String,
    /// When the current (or last) turn started, in store milliseconds.
    pub turn_started_ms: Option<i64>,
    /// When it ended; `None` while it runs.
    pub turn_ended_ms: Option<i64>,
    /// What the current turn has done so far.
    pub steps: Steps,
    /// The last finished turn's cost and tokens, when the agent reported them.
    pub turn_cost: Option<f64>,
    pub turn_tokens: u64,
}

/// The tool work of one turn, for one-line summaries ("Edited 3 files · ran 2
/// commands").
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Steps {
    pub edits: u32,
    pub commands: u32,
    pub reads: u32,
    pub searches: u32,
    pub other: u32,
    pub failed: u32,
    /// Files changed this turn, in order, at most [`FILES_MAX`].
    pub files: Vec<String>,
}

/// Steps keep this many changed file names.
const FILES_MAX: usize = 20;

impl Steps {
    pub fn total(&self) -> u32 {
        self.edits + self.commands + self.reads + self.searches + self.other
    }

    /// "Edited 3 files · ran 2 commands · read 5" (empty when nothing ran).
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        let files = self.files.len().max(self.edits.min(1) as usize);
        if self.edits > 0 {
            parts.push(if files == 1 { "Edited 1 file".to_string() } else { format!("Edited {files} files") });
        }
        if self.commands > 0 {
            parts.push(if self.commands == 1 { "ran 1 command".into() } else { format!("ran {} commands", self.commands) });
        }
        if self.reads + self.searches > 0 {
            parts.push(format!("read {}", self.reads + self.searches));
        }
        if self.failed > 0 {
            parts.push(format!("{} failed", self.failed));
        }
        let mut s = parts.join(" · ");
        if let Some(first) = s.get(..1) {
            let upper = first.to_uppercase();
            s.replace_range(..1, &upper);
        }
        s
    }
}

impl TaskState {
    /// Something here wants the user: an approval, a question, or a stop
    /// the agent itself flagged.
    pub fn needs_you(&self) -> bool {
        !self.pending.is_empty() || self.status == TaskStatus::NeedsInput
    }

    /// Applies one event. Pure, so it's tested without an engine.
    pub fn observe(&mut self, ev: &AgentEvent) {
        match ev {
            AgentEvent::UserMessage { .. } => {
                self.activity = None;
                self.last_error = None;
                self.reply.clear();
                self.steps = Steps::default();
                self.turn_started_ms = Some(dove_store::now_ms());
                self.turn_ended_ms = None;
            }
            AgentEvent::ToolCall { kind, title, .. } => {
                match kind {
                    ToolKind::Edit => self.steps.edits += 1,
                    ToolKind::Execute => self.steps.commands += 1,
                    ToolKind::Read => self.steps.reads += 1,
                    ToolKind::Search | ToolKind::Fetch => self.steps.searches += 1,
                    _ => self.steps.other += 1,
                }
                if !title.is_empty() {
                    self.activity = Some(title.to_string());
                }
            }
            AgentEvent::ToolResult { is_error: true, .. } => self.steps.failed += 1,
            AgentEvent::ToolDiff { path, .. } => {
                if !self.steps.files.iter().any(|f| f == path) && self.steps.files.len() < FILES_MAX {
                    self.steps.files.push(path.clone());
                }
            }
            AgentEvent::TextDelta { text } => {
                self.reply.push_str(text);
                if self.reply.len() > REPLY_MAX * 2 {
                    let cut = self.reply.len() - REPLY_MAX;
                    let cut = (cut..self.reply.len()).find(|&i| self.reply.is_char_boundary(i)).unwrap_or(0);
                    self.reply.drain(..cut);
                }
            }
            AgentEvent::BlockEnd => {
                if !self.reply.is_empty() && !self.reply.ends_with("\n\n") {
                    self.reply.push_str("\n\n");
                }
            }
            AgentEvent::Status { text } => self.activity = Some(text.to_string()),
            AgentEvent::PermissionRequested(req) => {
                if !self.pending.iter().any(|p| p.request_id == req.request_id) {
                    self.pending.push(req.clone());
                }
            }
            AgentEvent::PermissionResolved { request_id, .. } => self.pending.retain(|p| &p.request_id != request_id),
            AgentEvent::Todos { items } => self.todos = items.clone(),
            AgentEvent::Error { message } => self.last_error = Some(message.to_string()),
            // A turn that ended (or a process that's gone) can't still be
            // waiting on an answer.
            AgentEvent::TurnEnded { usage, .. } => {
                self.activity = None;
                self.pending.clear();
                self.turn_ended_ms = Some(dove_store::now_ms());
                if let Some(u) = usage {
                    self.turn_cost = u.cost_usd;
                    self.turn_tokens = u.total_tokens();
                }
            }
            AgentEvent::Exited { .. } => {
                self.activity = None;
                self.pending.clear();
                self.live = false;
                if self.turn_ended_ms.is_none() && self.turn_started_ms.is_some() {
                    self.turn_ended_ms = Some(dove_store::now_ms());
                }
                if self.status == TaskStatus::Running {
                    self.status = TaskStatus::Failed;
                }
            }
            _ => {}
        }
        if let Some(s) = crate::next_status(ev) {
            self.status = s;
        }
    }

    /// The reply trimmed to its last `max` characters, for small surfaces.
    pub fn reply_tail(&self, max: usize) -> String {
        let text = self.reply.trim();
        match text.char_indices().rev().nth(max.saturating_sub(1)) {
            Some((i, _)) if i > 0 => format!("…{}", &text[i..]),
            _ => text.to_string(),
        }
    }
}

/// Every task's [`TaskState`], with change notification.
#[derive(Default)]
pub struct Attention {
    tasks: RwLock<HashMap<TaskId, TaskState>>,
    version: AtomicU64,
    watchers: Mutex<Vec<flume::Sender<u64>>>,
}

impl Attention {
    pub fn get(&self, task: TaskId) -> Option<TaskState> {
        self.tasks.read().get(&task).cloned()
    }

    pub fn all(&self) -> Vec<(TaskId, TaskState)> {
        self.tasks.read().iter().map(|(id, s)| (*id, s.clone())).collect()
    }

    /// The oldest undecided request of `task`.
    pub fn pending(&self, task: TaskId) -> Option<PermissionRequest> {
        self.tasks.read().get(&task).and_then(|s| s.pending.first().cloned())
    }

    /// Bumped on every change.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// Pinged (with the new version) after changes. Pings coalesce: a slow
    /// reader sees one, then reads the current state.
    pub fn subscribe(&self) -> flume::Receiver<u64> {
        let (tx, rx) = flume::bounded(1);
        self.watchers.lock().push(tx);
        rx
    }

    /// Changes one task's state, announcing it if anything changed.
    pub fn update(&self, task: TaskId, f: impl FnOnce(&mut TaskState)) {
        let changed = {
            let mut tasks = self.tasks.write();
            let state = tasks.entry(task).or_default();
            let before = state.clone();
            f(state);
            *state != before
        };
        if changed {
            let v = self.version.fetch_add(1, Ordering::AcqRel) + 1;
            self.watchers.lock().retain(|w| !matches!(w.try_send(v), Err(flume::TrySendError::Disconnected(_))));
        }
    }

    pub fn observe(&self, task: TaskId, ev: &AgentEvent) {
        // Text streams in many small deltas; they change only the reply, and
        // announcing each would wake every reader per token. They're
        // announced with the next event instead (a block or tool, at most).
        if let AgentEvent::TextDelta { .. } = ev {
            if let Some(state) = self.tasks.write().get_mut(&task) {
                state.observe(ev);
            }
            return;
        }
        self.update(task, |s| s.observe(ev));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dove_proto::{PermissionDecision, TodoStatus};
    use std::sync::Arc;

    fn req(id: &str) -> PermissionRequest {
        serde_json::from_value(serde_json::json!({
            "request_id": id,
            "tool": "Bash",
            "summary": "rm -rf build",
            "input": {}
        }))
        .expect("a permission request")
    }

    fn text(t: &str) -> AgentEvent {
        AgentEvent::TextDelta { text: Arc::from(t) }
    }

    #[test]
    fn approvals_come_and_go() {
        let mut s = TaskState { live: true, ..Default::default() };
        s.observe(&AgentEvent::UserMessage { text: Arc::from("go") });
        assert_eq!(s.status, TaskStatus::Running);
        s.observe(&AgentEvent::PermissionRequested(req("a")));
        s.observe(&AgentEvent::PermissionRequested(req("b")));
        assert!(s.needs_you());
        assert_eq!(s.status, TaskStatus::NeedsInput);
        s.observe(&AgentEvent::PermissionResolved { request_id: "a".into(), decision: PermissionDecision::AllowOnce });
        assert_eq!(s.pending.len(), 1);
        s.observe(&AgentEvent::TurnEnded { usage: None, is_error: false });
        assert!(s.pending.is_empty());
        assert_eq!(s.status, TaskStatus::Done);
    }

    #[test]
    fn the_reply_is_the_text_since_the_last_message() {
        let mut s = TaskState::default();
        s.observe(&text("old"));
        s.observe(&AgentEvent::UserMessage { text: Arc::from("again") });
        s.observe(&text("Hello"));
        s.observe(&text(" there"));
        s.observe(&AgentEvent::BlockEnd);
        s.observe(&text("Done."));
        assert_eq!(s.reply_tail(100), "Hello there\n\nDone.");
        assert_eq!(s.reply_tail(5), "…Done.");
    }

    #[test]
    fn the_reply_tail_stays_bounded_on_char_boundaries() {
        let mut s = TaskState::default();
        for _ in 0..5000 {
            s.observe(&text("é…"));
        }
        assert!(s.reply.len() <= REPLY_MAX * 2 + 8);
        assert!(s.reply.is_char_boundary(0));
    }

    #[test]
    fn activity_follows_status_and_tools_and_clears_between_turns() {
        let mut s = TaskState::default();
        s.observe(&AgentEvent::Status { text: Arc::from("Writing main.rs") });
        assert_eq!(s.activity.as_deref(), Some("Writing main.rs"));
        s.observe(&AgentEvent::Todos {
            items: vec![serde_json::from_value(serde_json::json!({"text": "a", "status": "completed"})).unwrap()],
        });
        assert_eq!(s.todos[0].status, TodoStatus::Completed);
        s.observe(&AgentEvent::TurnEnded { usage: None, is_error: true });
        assert_eq!(s.activity, None);
        assert_eq!(s.status, TaskStatus::Failed);
    }

    #[test]
    fn steps_count_the_turn_and_reset_with_the_next() {
        let mut s = TaskState::default();
        s.observe(&AgentEvent::UserMessage { text: Arc::from("go") });
        assert!(s.turn_started_ms.is_some());
        for (kind, title) in [(ToolKind::Edit, "Edit a.rs"), (ToolKind::Edit, "Edit b.rs"), (ToolKind::Execute, "cargo test")] {
            s.observe(&AgentEvent::ToolCall {
                call_id: title.into(),
                name: "x".into(),
                kind,
                title: Arc::from(title),
                input: serde_json::Value::Null,
            });
        }
        for path in ["a.rs", "b.rs", "a.rs"] {
            s.observe(&AgentEvent::ToolDiff {
                call_id: "c".into(),
                path: path.into(),
                old_text: None,
                new_text: Arc::from(""),
            });
        }
        s.observe(&AgentEvent::ToolResult { call_id: "x".into(), output: Arc::from(""), is_error: true });
        assert_eq!(s.activity.as_deref(), Some("cargo test"));
        assert_eq!(s.steps.summary(), "Edited 2 files · ran 1 command · 1 failed");
        s.observe(&AgentEvent::TurnEnded { usage: None, is_error: false });
        assert!(s.turn_ended_ms.is_some());
        s.observe(&AgentEvent::UserMessage { text: Arc::from("again") });
        assert_eq!(s.steps, Steps::default());
    }

    #[test]
    fn exiting_mid_turn_fails_and_goes_offline() {
        let mut s = TaskState { live: true, status: TaskStatus::Running, ..Default::default() };
        s.observe(&AgentEvent::Exited { code: Some(1) });
        assert!(!s.live);
        assert_eq!(s.status, TaskStatus::Failed);
    }

    #[test]
    fn readers_are_pinged_on_change_but_not_per_token() {
        let a = Attention::default();
        let id = TaskId::new();
        let rx = a.subscribe();
        a.update(id, |s| s.live = true);
        assert_eq!(rx.try_recv().ok(), Some(1));
        a.observe(id, &text("streaming"));
        assert!(rx.try_recv().is_err());
        assert_eq!(a.get(id).unwrap().reply, "streaming");
        a.observe(id, &AgentEvent::BlockEnd);
        assert_eq!(rx.try_recv().ok(), Some(2));
    }
}
