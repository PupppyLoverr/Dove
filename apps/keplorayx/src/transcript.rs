//! A thread's transcript as the interface shows it, folded from the engine's
//! `AgentEvent`s: what was recorded (replayed from the store) and what
//! streams in from a live session, through the same fold.
//!
//! The text being written stays out of the entries, in `draft`, until its
//! block ends: a streamed word then costs a small value's write in the
//! window, not the whole list's. Anything else (a tool, a new turn, the turn
//! ending) closes it into an entry and changes the list.
//!
//! No zgx here: `host` turns entries into the scripts' values.

use dove_proto::AgentEvent;

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    User,
    Assistant,
    Thinking,
    Tool,
    Notice,
    Error,
}

impl Kind {
    pub fn id(&self) -> &'static str {
        match self {
            Kind::User => "user",
            Kind::Assistant => "assistant",
            Kind::Thinking => "thinking",
            Kind::Tool => "tool",
            Kind::Notice => "notice",
            Kind::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolState {
    Running,
    Done,
    Failed,
}

impl ToolState {
    pub fn id(&self) -> &'static str {
        match self {
            ToolState::Running => "running",
            ToolState::Done => "done",
            ToolState::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub call_id: String,
    pub name: String,
    pub state: ToolState,
    /// The last lines of what it returned, enough for a row's peek.
    pub output: String,
    /// Files it changed: (path, lines added, lines removed).
    pub changes: Vec<(String, usize, usize)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub kind: Kind,
    pub text: String,
    pub tool: Option<Tool>,
}

/// The block being written: its entry-to-be's id, kind and text so far.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub id: String,
    pub kind: Kind,
    pub text: String,
}

/// What the agent is waiting on you for: a tool to run, or questions.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub request_id: String,
    pub tool: String,
    pub summary: String,
    /// Each question's text and its options' labels (none for a tool).
    pub questions: Vec<(String, Vec<String>)>,
}

/// What an event changed, so the host publishes no more than that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    None,
    /// Only the draft.
    Draft,
    /// The entries (and the draft, which may have closed into them).
    Entries,
}

impl Change {
    pub fn and(self, other: Change) -> Change {
        match (self, other) {
            (Change::Entries, _) | (_, Change::Entries) => Change::Entries,
            (Change::Draft, _) | (_, Change::Draft) => Change::Draft,
            _ => Change::None,
        }
    }
}

/// How many characters of a tool's output a row keeps.
const OUTPUT_TAIL: usize = 2000;

#[derive(Debug, Default)]
pub struct Fold {
    pub entries: Vec<Entry>,
    pub draft: Option<Draft>,
    /// A turn is running (from the user's message until it ends).
    pub running: bool,
    /// The harness's status line while it works.
    pub status: String,
    /// What it waits on you for, if anything.
    pub pending: Option<Pending>,
    next: u64,
}

impl Fold {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every event of a recorded transcript.
    pub fn replay(events: impl IntoIterator<Item = AgentEvent>) -> Self {
        let mut fold = Self::new();
        for event in events {
            fold.apply(&event);
        }
        fold.close();
        // A replay is history: whatever was running when it was recorded
        // isn't now (a live session says so again if it is).
        fold.running = false;
        fold.status.clear();
        fold.pending = None;
        fold
    }

    fn id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    /// Closes the draft into an entry; whether there was one.
    fn close(&mut self) -> bool {
        match self.draft.take() {
            Some(draft) if !draft.text.trim().is_empty() => {
                self.entries.push(Entry { id: draft.id, kind: draft.kind, text: draft.text, tool: None });
                true
            }
            Some(_) => true,
            None => false,
        }
    }

    fn stream(&mut self, kind: Kind, text: &str) -> Change {
        match &mut self.draft {
            Some(draft) if draft.kind == kind => {
                draft.text.push_str(text);
                Change::Draft
            }
            _ => {
                let closed = self.close();
                let prefix = if kind == Kind::Thinking { "k" } else { "a" };
                let id = self.id(prefix);
                self.draft = Some(Draft { id, kind, text: text.to_string() });
                if closed { Change::Entries } else { Change::Draft }
            }
        }
    }

    fn push(&mut self, kind: Kind, text: String, tool: Option<Tool>) -> Change {
        self.close();
        let prefix = match kind {
            Kind::User => "u",
            Kind::Tool => "t",
            _ => "n",
        };
        let id = self.id(prefix);
        self.entries.push(Entry { id, kind, text, tool });
        Change::Entries
    }

    fn tool_mut(&mut self, call_id: &str) -> Option<&mut Tool> {
        self.entries
            .iter_mut()
            .rev()
            .filter_map(|e| e.tool.as_mut())
            .find(|t| t.call_id == call_id)
    }

    pub fn apply(&mut self, event: &AgentEvent) -> Change {
        match event {
            AgentEvent::UserMessage { text } => {
                self.running = true;
                let text = dove_core::strip_persona(text).to_string();
                self.push(Kind::User, text, None)
            }
            AgentEvent::TextDelta { text } => self.stream(Kind::Assistant, text),
            AgentEvent::ThinkingDelta { text } => self.stream(Kind::Thinking, text),
            AgentEvent::BlockEnd => {
                if self.close() {
                    Change::Entries
                } else {
                    Change::None
                }
            }
            AgentEvent::ToolCall { call_id, name, title, .. } => {
                let tool = Tool {
                    call_id: call_id.clone(),
                    name: name.clone(),
                    state: ToolState::Running,
                    output: String::new(),
                    changes: Vec::new(),
                };
                self.push(Kind::Tool, title.to_string(), Some(tool))
            }
            AgentEvent::ToolTitle { call_id, title } => {
                let title = title.to_string();
                match self.entries.iter_mut().rev().find(|e| e.tool.as_ref().is_some_and(|t| &t.call_id == call_id)) {
                    Some(entry) => {
                        entry.text = title;
                        Change::Entries
                    }
                    None => Change::None,
                }
            }
            AgentEvent::ToolResult { call_id, output, is_error } => match self.tool_mut(call_id) {
                Some(tool) => {
                    tool.state = if *is_error { ToolState::Failed } else { ToolState::Done };
                    tool.output = tail(output, OUTPUT_TAIL);
                    Change::Entries
                }
                None => Change::None,
            },
            AgentEvent::ToolDiff { call_id, path, old_text, new_text } => {
                let (added, removed) = line_counts(old_text.as_deref().unwrap_or(""), new_text);
                match self.tool_mut(call_id) {
                    Some(tool) => {
                        tool.changes.push((path.clone(), added, removed));
                        Change::Entries
                    }
                    None => Change::None,
                }
            }
            AgentEvent::PermissionRequested(request) => {
                self.close();
                self.pending = Some(Pending {
                    request_id: request.request_id.clone(),
                    tool: request.tool.clone(),
                    summary: request.summary.to_string(),
                    questions: request
                        .questions
                        .iter()
                        .map(|q| (q.question.clone(), q.options.clone()))
                        .collect(),
                });
                Change::Entries
            }
            AgentEvent::PermissionResolved { request_id, .. } => {
                if self.pending.as_ref().is_some_and(|p| &p.request_id == request_id) {
                    self.pending = None;
                    Change::Entries
                } else {
                    Change::None
                }
            }
            AgentEvent::Notice { text } => self.push(Kind::Notice, text.to_string(), None),
            AgentEvent::Error { message } => self.push(Kind::Error, message.to_string(), None),
            AgentEvent::Status { text } => {
                self.status = text.to_string();
                Change::Entries
            }
            AgentEvent::TurnEnded { .. } | AgentEvent::Exited { .. } => {
                self.running = false;
                self.status.clear();
                self.pending = None;
                self.close();
                Change::Entries
            }
            _ => Change::None,
        }
    }
}

/// The last `max` characters of `text`, from a line's start where it can.
fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let start = text.char_indices().nth(count - max).map(|(i, _)| i).unwrap_or(0);
    let cut = &text[start..];
    match cut.find('\n') {
        Some(nl) if nl + 1 < cut.len() => cut[nl + 1..].to_string(),
        _ => cut.to_string(),
    }
}

/// Lines added and removed between two texts, as a diff's +/− counts.
fn line_counts(old: &str, new: &str) -> (usize, usize) {
    use std::collections::HashMap;
    let mut seen: HashMap<&str, isize> = HashMap::new();
    for line in old.lines() {
        *seen.entry(line).or_default() += 1;
    }
    let mut added = 0;
    for line in new.lines() {
        match seen.get_mut(line) {
            Some(n) if *n > 0 => *n -= 1,
            _ => added += 1,
        }
    }
    let removed = seen.values().filter(|n| **n > 0).map(|n| *n as usize).sum();
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn text(s: &str) -> AgentEvent {
        AgentEvent::TextDelta { text: Arc::from(s) }
    }

    #[test]
    fn a_permission_waits_until_it_is_answered_or_the_turn_ends() {
        let mut fold = Fold::new();
        let ask = |id: &str| {
            AgentEvent::PermissionRequested(dove_proto::PermissionRequest {
                request_id: id.into(),
                tool: "Bash".into(),
                summary: Arc::from("cargo test"),
                input: serde_json::Value::Null,
                questions: Vec::new(),
            })
        };
        fold.apply(&AgentEvent::UserMessage { text: Arc::from("go") });
        assert_eq!(fold.apply(&ask("r1")), Change::Entries);
        assert_eq!(fold.pending.as_ref().map(|p| p.summary.as_str()), Some("cargo test"));
        // Another request's answer leaves this one waiting.
        let other = AgentEvent::PermissionResolved {
            request_id: "r0".into(),
            decision: dove_proto::PermissionDecision::AllowOnce,
        };
        assert_eq!(fold.apply(&other), Change::None);
        assert!(fold.pending.is_some());
        let answered = AgentEvent::PermissionResolved {
            request_id: "r1".into(),
            decision: dove_proto::PermissionDecision::AllowOnce,
        };
        assert_eq!(fold.apply(&answered), Change::Entries);
        assert!(fold.pending.is_none());
        fold.apply(&ask("r2"));
        fold.apply(&AgentEvent::TurnEnded { usage: None, is_error: false });
        assert!(fold.pending.is_none());
    }

    #[test]
    fn streamed_text_stays_in_the_draft_until_its_block_ends() {
        let mut fold = Fold::new();
        assert_eq!(fold.apply(&AgentEvent::UserMessage { text: Arc::from("hi") }), Change::Entries);
        assert_eq!(fold.apply(&text("Hel")), Change::Draft);
        assert_eq!(fold.apply(&text("lo")), Change::Draft);
        assert_eq!(fold.entries.len(), 1);
        assert_eq!(fold.draft.as_ref().unwrap().text, "Hello");
        assert_eq!(fold.apply(&AgentEvent::BlockEnd), Change::Entries);
        assert!(fold.draft.is_none());
        assert_eq!(fold.entries[1].text, "Hello");
        assert_eq!(fold.entries[1].kind, Kind::Assistant);
    }

    #[test]
    fn a_tool_closes_the_text_before_it_and_its_result_updates_its_row() {
        let mut fold = Fold::new();
        fold.apply(&text("Reading"));
        fold.apply(&AgentEvent::ToolCall {
            call_id: "c1".into(),
            name: "Read".into(),
            kind: dove_proto::ToolKind::Read,
            title: Arc::from("Read main.rs"),
            input: serde_json::Value::Null,
        });
        assert_eq!(fold.entries.len(), 2);
        let id = fold.entries[1].id.clone();
        fold.apply(&AgentEvent::ToolResult { call_id: "c1".into(), output: Arc::from("fn main() {}"), is_error: false });
        let tool = fold.entries[1].tool.as_ref().unwrap();
        assert_eq!(tool.state, ToolState::Done);
        assert_eq!(fold.entries[1].id, id, "a row keeps its id as it updates");
    }

    #[test]
    fn a_replay_is_not_running() {
        let fold = Fold::replay([AgentEvent::UserMessage { text: Arc::from("go") }, text("partial")]);
        assert!(!fold.running);
        assert_eq!(fold.entries.len(), 2);
        assert!(fold.draft.is_none());
    }

    #[test]
    fn line_counts_count_changed_lines() {
        assert_eq!(line_counts("a\nb\nc", "a\nB\nc\nd"), (2, 1));
        assert_eq!(line_counts("", "x\ny"), (2, 0));
    }
}
