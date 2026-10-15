//! Random access into a JSONL transcript.
//!
//! A transcript is append-only, one [`AgentEvent`] per line. Long threads
//! grow to many megabytes, so readers avoid parsing all of it:
//!
//! - [`Index`] records each line's byte offset and a coarse [`Kind`] read off
//!   the line's `"type"` prefix, with no JSON parsing. It is what a thread
//!   opens with: the tail is chosen from it ([`Index::tail_start`]) and only
//!   that part is parsed; older lines are read later, on demand.
//! - A tool's full output stays on disk; the UI keeps a head and a tail and
//!   fetches the rest by offset ([`read_event_at`]) or call id
//!   ([`find_tool_output`]) when asked.
//! - [`read_from`] reads only the complete lines appended since a byte
//!   offset, for consumers that follow a transcript incrementally.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use dove_proto::AgentEvent;

/// What a line holds, read off its `{"type":"…"` prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    UserMessage,
    TextDelta,
    ThinkingDelta,
    ToolCall,
    /// A `show_canvas` call: a design the canvas gallery lists.
    CanvasCall,
    Todos,
    ContextUsage,
    RateLimits,
    TurnEnded,
    /// Another event that becomes a transcript row on its own (a permission
    /// request, a plan, a notice, an error).
    Entry,
    /// Everything else (results, titles, diffs, usage, subagent events,
    /// unknown or torn lines).
    Other,
}

impl Kind {
    /// Classify one serialized line. Serde writes the internal `type` tag
    /// first, so the prefix is enough.
    pub fn of(line: &[u8]) -> Kind {
        const PREFIX: &[u8] = b"{\"type\":\"";
        let Some(rest) = line.strip_prefix(PREFIX) else { return Kind::Other };
        let end = rest.iter().position(|b| *b == b'"').unwrap_or(rest.len());
        match &rest[..end] {
            b"user_message" => Kind::UserMessage,
            b"text_delta" => Kind::TextDelta,
            b"thinking_delta" => Kind::ThinkingDelta,
            b"tool_call" if contains(line, b"show_canvas") => Kind::CanvasCall,
            b"tool_call" => Kind::ToolCall,
            b"todos" => Kind::Todos,
            b"context_usage" => Kind::ContextUsage,
            b"rate_limits" => Kind::RateLimits,
            b"turn_ended" => Kind::TurnEnded,
            b"permission_requested" | b"plan" | b"notice" | b"error" => Kind::Entry,
            _ => Kind::Other,
        }
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

/// Line offsets and kinds of one transcript.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    /// Byte offset where each line starts.
    pub offsets: Vec<u64>,
    pub kinds: Vec<Kind>,
    /// Byte length of the indexed part (the end of the last complete line).
    pub len: u64,
}

impl Index {
    /// Scan a transcript file. A torn final line (no newline yet) is left out.
    pub fn build(path: &Path) -> Index {
        let Ok(f) = File::open(path) else { return Index::default() };
        Self::scan(BufReader::with_capacity(64 * 1024, f))
    }

    fn scan(mut r: impl BufRead) -> Index {
        let mut index = Index::default();
        let mut line = Vec::with_capacity(4096);
        let mut at = 0u64;
        loop {
            line.clear();
            let n = match r.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if line.last() != Some(&b'\n') {
                break;
            }
            index.offsets.push(at);
            index.kinds.push(Kind::of(&line));
            at += n as u64;
        }
        index.len = at;
        index
    }

    pub fn lines(&self) -> usize {
        self.offsets.len()
    }

    /// Byte range of lines `range`.
    pub fn span(&self, range: Range<usize>) -> Range<u64> {
        let start = self.offsets.get(range.start).copied().unwrap_or(self.len);
        let end = self.offsets.get(range.end).copied().unwrap_or(self.len);
        start..end.max(start)
    }

    /// Where to start reading so that lines `start..upto` hold at least
    /// `min_entries` entries: the latest user message at or before that point,
    /// so the loaded part begins on a turn boundary. An entry is a line that
    /// becomes a transcript row (a user message, a tool call, a permission
    /// request, a plan, a notice, an error, a turn end), with each run of
    /// text or thinking deltas counted once. Returns `0` (load everything)
    /// when the prefix is short or has no earlier user message.
    pub fn tail_start(&self, upto: usize, min_entries: usize) -> usize {
        let upto = upto.min(self.lines());
        let mut entries = 0usize;
        let mut prev: Option<Kind> = None;
        let mut i = upto;
        while i > 0 {
            i -= 1;
            let k = self.kinds[i];
            let counts = match k {
                Kind::TextDelta | Kind::ThinkingDelta => prev != Some(k),
                Kind::UserMessage | Kind::ToolCall | Kind::CanvasCall | Kind::TurnEnded | Kind::Entry => true,
                _ => false,
            };
            if !matches!(k, Kind::Other | Kind::Todos | Kind::ContextUsage | Kind::RateLimits) {
                prev = Some(k);
            }
            if counts {
                entries += 1;
            }
            if entries >= min_entries && k == Kind::UserMessage {
                return i;
            }
        }
        0
    }

    /// Lines of `kind` within `range`.
    pub fn of_kind(&self, range: Range<usize>, kind: Kind) -> impl Iterator<Item = usize> + '_ {
        let end = range.end.min(self.lines());
        (range.start.min(end)..end).filter(move |&i| self.kinds[i] == kind)
    }
}

/// Parse lines `range` of the indexed file, with each event's byte offset.
/// Lines that fail to parse are skipped.
pub fn read_lines(path: &Path, index: &Index, range: Range<usize>) -> Vec<(u64, AgentEvent)> {
    let span = index.span(range);
    if span.is_empty() {
        return Vec::new();
    }
    let Ok(mut f) = File::open(path) else { return Vec::new() };
    if f.seek(SeekFrom::Start(span.start)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::with_capacity((span.end - span.start) as usize);
    if f.take(span.end - span.start).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut at = span.start;
    for line in buf.split_inclusive(|b| *b == b'\n') {
        if let Ok(ev) = serde_json::from_slice::<AgentEvent>(line) {
            out.push((at, ev));
        }
        at += line.len() as u64;
    }
    out
}

/// Parse the given lines (ascending line numbers) with one file handle,
/// each with its line number. Lines that fail to parse are skipped.
pub fn read_picked(path: &Path, index: &Index, lines: &[usize]) -> Vec<(usize, AgentEvent)> {
    if lines.is_empty() {
        return Vec::new();
    }
    let Ok(f) = File::open(path) else { return Vec::new() };
    let mut r = BufReader::new(f);
    let mut out = Vec::with_capacity(lines.len());
    let mut buf = Vec::new();
    for &i in lines {
        let Some(&at) = index.offsets.get(i) else { break };
        if r.seek(SeekFrom::Start(at)).is_err() {
            break;
        }
        buf.clear();
        if r.read_until(b'\n', &mut buf).is_err() {
            break;
        }
        if let Ok(ev) = serde_json::from_slice::<AgentEvent>(&buf) {
            out.push((i, ev));
        }
    }
    out
}

/// The event on the line starting at `offset`.
pub fn read_event_at(path: &Path, offset: u64) -> Option<AgentEvent> {
    let mut f = File::open(path).ok()?;
    f.seek(SeekFrom::Start(offset)).ok()?;
    let mut line = Vec::new();
    BufReader::new(f).read_until(b'\n', &mut line).ok()?;
    serde_json::from_slice(&line).ok()
}

/// Complete lines appended at or after byte `from`, parsed, and the offset
/// just past the last complete line (where the next read starts). A torn
/// final line is left for the next read.
pub fn read_from(path: &Path, from: u64) -> (Vec<AgentEvent>, u64) {
    let Ok(mut f) = File::open(path) else { return (Vec::new(), from) };
    if f.seek(SeekFrom::Start(from)).is_err() {
        return (Vec::new(), from);
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return (Vec::new(), from);
    }
    let complete = buf.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let events = buf[..complete]
        .split_inclusive(|b| *b == b'\n')
        .filter_map(|l| serde_json::from_slice::<AgentEvent>(l).ok())
        .collect();
    (events, from + complete as u64)
}

/// The full output of tool call `call_id`, wherever it ran: the thread's
/// own calls and its subagents' (newest match wins).
pub fn find_tool_output(path: &Path, call_id: &str) -> Option<Arc<str>> {
    let f = File::open(path).ok()?;
    let needle = serde_json::to_string(call_id).ok()?;
    let mut found = None;
    for line in BufReader::new(f).split(b'\n').map_while(Result::ok) {
        if !contains(&line, b"tool_result") || !contains(&line, needle.as_bytes()) {
            continue;
        }
        let Ok(ev) = serde_json::from_slice::<AgentEvent>(&line) else { continue };
        if let Some(out) = tool_output_of(&ev, call_id) {
            found = Some(out);
        }
    }
    found
}

/// `ev`'s output when it is the result of `call_id` (also nested in a
/// subagent's events).
pub fn tool_output_of(ev: &AgentEvent, call_id: &str) -> Option<Arc<str>> {
    match ev {
        AgentEvent::ToolResult { call_id: id, output, .. } if id == call_id => Some(output.clone()),
        AgentEvent::Subagent { event, .. } => tool_output_of(event, call_id),
        _ => None,
    }
}

/// Every event of the subagent spawned by `parent`, in order.
pub fn subagent_events(path: &Path, parent: &str) -> Vec<AgentEvent> {
    let Ok(f) = File::open(path) else { return Vec::new() };
    let Ok(needle) = serde_json::to_string(parent) else { return Vec::new() };
    let mut out = Vec::new();
    for line in BufReader::new(f).split(b'\n').map_while(Result::ok) {
        if !line.starts_with(b"{\"type\":\"subagent\"") || !contains(&line, needle.as_bytes()) {
            continue;
        }
        if let Ok(AgentEvent::Subagent { parent: p, event }) = serde_json::from_slice::<AgentEvent>(&line)
            && p == parent
        {
            out.push(*event);
        }
    }
    out
}

/// Every event of the thread's own transcript within byte range `span`.
pub fn read_span(path: &Path, span: Range<u64>) -> Vec<AgentEvent> {
    let Ok(mut f) = File::open(path) else { return Vec::new() };
    if span.is_empty() || f.seek(SeekFrom::Start(span.start)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if f.take(span.end - span.start).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    buf.split_inclusive(|b| *b == b'\n').filter_map(|l| serde_json::from_slice::<AgentEvent>(l).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(path: &Path, events: &[AgentEvent]) {
        let mut f = std::fs::File::create(path).unwrap();
        for e in events {
            serde_json::to_writer(&mut f, e).unwrap();
            f.write_all(b"\n").unwrap();
        }
    }

    fn user(s: &str) -> AgentEvent {
        AgentEvent::UserMessage { text: s.into() }
    }
    fn text(s: &str) -> AgentEvent {
        AgentEvent::TextDelta { text: s.into() }
    }
    fn end() -> AgentEvent {
        AgentEvent::TurnEnded { usage: None, is_error: false }
    }

    #[test]
    fn kinds_come_from_the_type_prefix() {
        let line = |e: &AgentEvent| serde_json::to_vec(e).unwrap();
        assert_eq!(Kind::of(&line(&user("x"))), Kind::UserMessage);
        assert_eq!(Kind::of(&line(&text("x"))), Kind::TextDelta);
        assert_eq!(Kind::of(&line(&end())), Kind::TurnEnded);
        let canvas = AgentEvent::ToolCall {
            call_id: "c".into(),
            name: "mcp__dove__show_canvas".into(),
            kind: dove_proto::ToolKind::Other,
            title: "t".into(),
            input: serde_json::Value::Null,
        };
        assert_eq!(Kind::of(&line(&canvas)), Kind::CanvasCall);
        assert_eq!(Kind::of(b"{\"type\":\"text_de"), Kind::Other);
        assert_eq!(Kind::of(b"garbage"), Kind::Other);
    }

    #[test]
    fn index_skips_a_torn_tail_and_reads_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        write(&path, &[user("a"), text("b"), end()]);
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{\"type\":\"te").unwrap();
        let index = Index::build(&path);
        assert_eq!(index.lines(), 3);
        assert_eq!(index.kinds, vec![Kind::UserMessage, Kind::TextDelta, Kind::TurnEnded]);
        let got = read_lines(&path, &index, 1..3);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, index.offsets[1]);
        assert_eq!(got[0].1, text("b"));
        assert_eq!(read_event_at(&path, index.offsets[2]), Some(end()));
        assert_eq!(read_picked(&path, &index, &[0, 2]), vec![(0, user("a")), (2, end())]);
        // Incremental reads stop before the torn line, then pick it up.
        let (evs, next) = read_from(&path, 0);
        assert_eq!(evs.len(), 3);
        assert_eq!(next, index.len);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"xt_delta\",\"text\":\"c\"}\n")
            .unwrap();
        let (evs, after) = read_from(&path, next);
        assert_eq!(evs, vec![text("c")]);
        assert!(after > next);
        assert_eq!(read_from(&path, after).0, Vec::new());
    }

    #[test]
    fn tail_starts_on_a_user_message_with_enough_entries() {
        let mut events = Vec::new();
        for i in 0..10 {
            events.push(user(&format!("q{i}")));
            // A run of deltas is one entry.
            events.extend((0..20).map(|_| text("x")));
            events.push(end());
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        write(&path, &events);
        let index = Index::build(&path);
        // Each turn is 3 entries: 4 entries need the last 2 turns.
        let start = index.tail_start(index.lines(), 4);
        assert_eq!(index.kinds[start], Kind::UserMessage);
        assert_eq!(start, 8 * 22);
        // The next older chunk ends where the tail starts.
        let older = index.tail_start(start, 4);
        assert_eq!(older, 6 * 22);
        // More than the whole transcript: load everything.
        assert_eq!(index.tail_start(index.lines(), 1000), 0);
        assert_eq!(index.tail_start(0, 4), 0);
    }

    #[test]
    fn tool_outputs_and_subagent_events_are_found_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let result =
            |id: &str, out: &str| AgentEvent::ToolResult { call_id: id.into(), output: out.into(), is_error: false };
        write(
            &path,
            &[
                user("a"),
                result("c1", "first"),
                AgentEvent::Subagent { parent: "p".into(), event: Box::new(result("c2", "nested")) },
                AgentEvent::Subagent { parent: "q".into(), event: Box::new(text("other")) },
                AgentEvent::Subagent { parent: "p".into(), event: Box::new(text("mine")) },
            ],
        );
        assert_eq!(find_tool_output(&path, "c1").as_deref(), Some("first"));
        assert_eq!(find_tool_output(&path, "c2").as_deref(), Some("nested"));
        assert_eq!(find_tool_output(&path, "c3"), None);
        assert_eq!(subagent_events(&path, "p"), vec![result("c2", "nested"), text("mine")]);
        let index = Index::build(&path);
        assert_eq!(read_span(&path, index.span(0..2)), vec![user("a"), result("c1", "first")]);
    }
}
