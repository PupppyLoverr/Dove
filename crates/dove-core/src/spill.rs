//! Caps on tool output (Hermes's `tool_result_storage`: 50 KB, 2000 lines,
//! 2000-character lines).
//!
//! An agent can return megabytes from one tool call: a build log, a minified
//! bundle, a `cat` of a lockfile. The recorder stores and forwards a preview
//! instead (the head and the tail, every line at most 2000 characters), and
//! writes the whole text to `<data>/spill/<task>-<call>.txt`. The preview ends
//! with one marker line that [`split`] reads back, so the transcript can say
//! "Output truncated" and open the full file.

use std::path::{Path, PathBuf};

use dove_proto::{AgentEvent, TaskId};

/// Output larger than this is capped.
pub const MAX_BYTES: usize = 50 * 1024;
/// Output with more lines than this is capped.
pub const MAX_LINES: usize = 2000;
/// Output with a line longer than this (in characters) is capped, and every
/// line of a preview is cut to it.
pub const MAX_LINE_CHARS: usize = 2000;

/// What a preview keeps from the start and the end of the output.
const HEAD_LINES: usize = 100;
const HEAD_BYTES: usize = 12 * 1024;
const TAIL_LINES: usize = 40;
const TAIL_BYTES: usize = 6 * 1024;

const MARK_OPEN: &str = "[dove: output truncated · ";
const MARK_PATH: &str = " · full output: ";

/// A capped output: what is kept, and the size of the whole.
#[derive(Debug, Clone, PartialEq)]
pub struct Cut {
    pub preview: String,
    pub bytes: usize,
    pub lines: usize,
}

/// Where the whole of a capped output went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spilled {
    pub path: PathBuf,
    pub bytes: u64,
    pub lines: u64,
}

impl Spilled {
    /// "52 KB, 4,000 lines".
    pub fn size_label(&self) -> String {
        let size = if self.bytes >= 1024 * 1024 {
            format!("{:.1} MB", self.bytes as f64 / (1024.0 * 1024.0))
        } else {
            format!("{} KB", self.bytes.div_ceil(1024))
        };
        let lines = group_digits(self.lines);
        format!("{size}, {lines} {}", if self.lines == 1 { "line" } else { "lines" })
    }
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The preview of `text`, or `None` when it is within every cap.
pub fn cap(text: &str) -> Option<Cut> {
    let lines = text.lines().count();
    let long_line = text.lines().any(|l| l.len() > MAX_LINE_CHARS && l.chars().count() > MAX_LINE_CHARS);
    if text.len() <= MAX_BYTES && lines <= MAX_LINES && !long_line {
        return None;
    }
    let all: Vec<&str> = text.lines().collect();
    // The head: whole lines up to its line and byte budget, the first line
    // always (cut to its character cap, a line is at most ~8 KB, under the
    // head's budget).
    let mut head = Vec::new();
    let mut used = 0;
    for line in &all {
        let line = cut_line(line);
        if head.len() >= HEAD_LINES || (!head.is_empty() && used + line.len() + 1 > HEAD_BYTES) {
            break;
        }
        used += line.len() + 1;
        head.push(line);
    }
    // The tail: from the end, never overlapping the head.
    let mut tail = Vec::new();
    let mut used = 0;
    for line in all[head.len()..].iter().rev() {
        let line = cut_line(line);
        if tail.len() >= TAIL_LINES || used + line.len() + 1 > TAIL_BYTES {
            break;
        }
        used += line.len() + 1;
        tail.push(line);
    }
    tail.reverse();
    let omitted = all.len() - head.len() - tail.len();
    let mut preview = head.join("\n");
    if omitted > 0 {
        let s = if omitted == 1 { "" } else { "s" };
        preview.push_str(&format!("\n… {} line{s} omitted …", group_digits(omitted as u64)));
    }
    if !tail.is_empty() {
        preview.push('\n');
        preview.push_str(&tail.join("\n"));
    }
    Some(Cut { preview, bytes: text.len(), lines })
}

/// `line` with at most [`MAX_LINE_CHARS`] characters, marked when cut.
fn cut_line(line: &str) -> String {
    if line.len() <= MAX_LINE_CHARS {
        return line.to_string();
    }
    match line.char_indices().nth(MAX_LINE_CHARS) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_string(),
    }
}

/// The last line of a capped output: what was cut and where it went.
pub fn marker(s: &Spilled) -> String {
    format!("{MARK_OPEN}{} bytes · {} lines{MARK_PATH}{}]", s.bytes, s.lines, s.path.display())
}

/// A recorded output without its spill marker, and the marker's contents.
pub fn split(output: &str) -> (&str, Option<Spilled>) {
    let trimmed = output.trim_end();
    let (body, last) = match trimmed.rsplit_once('\n') {
        Some((body, last)) => (body.trim_end(), last),
        None => ("", trimmed),
    };
    let parsed = (|| {
        let rest = last.strip_prefix(MARK_OPEN)?.strip_suffix(']')?;
        let (sizes, path) = rest.split_once(MARK_PATH)?;
        let (bytes, lines) = sizes.split_once(" · ")?;
        let bytes = bytes.strip_suffix(" bytes")?.parse().ok()?;
        let lines = lines.strip_suffix(" lines")?.parse().ok()?;
        (!path.is_empty()).then(|| Spilled { path: PathBuf::from(path), bytes, lines })
    })();
    match parsed {
        Some(s) => (body, Some(s)),
        None => (output, None),
    }
}

/// `<root>/spill`, where whole outputs are kept.
pub fn dir(root: &Path) -> PathBuf {
    root.join("spill")
}

/// The file `task`'s call `call_id` spills into.
pub fn file_for(root: &Path, task: TaskId, call_id: &str) -> PathBuf {
    let mut safe: String =
        call_id.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).take(80).collect();
    if safe.is_empty() {
        safe = format!("t{}", dove_store::now_ms());
    }
    dir(root).join(format!("{task}-{safe}.txt"))
}

/// `ev` with an oversized tool result capped: the whole text is written to
/// the spill folder and the event carries the preview and its marker. Any
/// other event, or a write that fails, passes through unchanged (nothing is
/// lost).
pub fn cap_event(root: &Path, task: TaskId, ev: AgentEvent) -> AgentEvent {
    match ev {
        AgentEvent::ToolResult { call_id, output, is_error } => {
            let output = match cap(&output) {
                Some(cut) => {
                    let path = file_for(root, task, &call_id);
                    let written =
                        std::fs::create_dir_all(dir(root)).and_then(|_| std::fs::write(&path, output.as_bytes()));
                    match written {
                        Ok(()) => {
                            let s = Spilled { path, bytes: cut.bytes as u64, lines: cut.lines as u64 };
                            format!("{}\n\n{}", cut.preview, marker(&s)).into()
                        }
                        Err(e) => {
                            tracing::warn!("keeping a large tool result whole; spilling it failed: {e}");
                            output
                        }
                    }
                }
                None => output,
            };
            AgentEvent::ToolResult { call_id, output, is_error }
        }
        AgentEvent::Subagent { parent, event } => {
            AgentEvent::Subagent { parent, event: Box::new(cap_event(root, task, *event)) }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_output_is_left_alone() {
        assert_eq!(cap("hello\nworld\n"), None);
        assert_eq!(cap(&"x\n".repeat(MAX_LINES)), None);
        assert_eq!(cap(&"y".repeat(MAX_LINE_CHARS)), None);
    }

    #[test]
    fn too_many_lines_keeps_head_and_tail() {
        let text: String = (0..5000).map(|i| format!("line {i}\n")).collect();
        let cut = cap(&text).expect("capped");
        assert_eq!(cut.lines, 5000);
        assert_eq!(cut.bytes, text.len());
        assert!(cut.preview.starts_with("line 0\nline 1\n"));
        assert!(cut.preview.ends_with("line 4999"));
        assert!(cut.preview.contains(&format!("… {} lines omitted …", group_digits(5000 - 140))));
        assert!(cut.preview.lines().count() <= HEAD_LINES + TAIL_LINES + 1);
    }

    #[test]
    fn too_many_bytes_is_capped_within_the_line_limit() {
        // 1000 lines of 100 bytes: under the line cap, over the byte cap.
        let text: String = (0..1000).map(|i| format!("{i:>99}\n")).collect();
        assert!(text.lines().count() <= MAX_LINES && text.len() > MAX_BYTES);
        let cut = cap(&text).expect("capped");
        assert!(cut.preview.len() <= HEAD_BYTES + TAIL_BYTES + 64);
        assert!(cut.preview.contains("lines omitted"));
    }

    #[test]
    fn long_lines_are_cut_to_the_character_cap() {
        let text = format!("short\n{}\nend", "a".repeat(MAX_LINE_CHARS + 500));
        let cut = cap(&text).expect("capped");
        let long = cut.preview.lines().nth(1).unwrap();
        assert_eq!(long.chars().count(), MAX_LINE_CHARS + 1);
        assert!(long.ends_with('…'));
        assert!(cut.preview.ends_with("end"));
    }

    #[test]
    fn cuts_land_on_char_boundaries() {
        // Multi-byte characters straddle every byte budget.
        let line = "日本語のテキスト🙂é".repeat(400);
        let cut = cap(&line).expect("capped");
        assert!(cut.preview.chars().count() <= MAX_LINE_CHARS + 1);
        assert!(cut.preview.ends_with('…'));
        // One giant line of 3-byte characters, cut by the character cap.
        let wide = "語".repeat(60_000);
        let cut = cap(&wide).expect("capped");
        assert!(cut.preview.len() <= HEAD_BYTES + "…".len());
        // Many lines of 4-byte characters around the tail budget.
        let text: String = (0..3000).map(|_| "🙂🙂🙂🙂🙂\n").collect();
        let cut = cap(&text).expect("capped");
        assert!(cut.preview.ends_with("🙂🙂🙂🙂🙂"));
    }

    #[test]
    fn marker_round_trips() {
        let s = Spilled { path: PathBuf::from("/tmp/dove spill/x · y.txt"), bytes: 123_456, lines: 4000 };
        let out = format!("preview line\nmore\n\n{}", marker(&s));
        let (body, got) = split(&out);
        assert_eq!(body, "preview line\nmore");
        assert_eq!(got, Some(s.clone()));
        assert_eq!(s.size_label(), "121 KB, 4,000 lines");
        // Ordinary output, and output that only mentions the words.
        assert_eq!(split("all fine\n"), ("all fine\n", None));
        assert_eq!(split("[dove: output truncated · nonsense]").1, None);
    }

    #[test]
    fn oversized_results_spill_to_disk() {
        let root = tempfile::tempdir().unwrap();
        let task = TaskId::new();
        let text: String = (0..3000).map(|i| format!("row {i}\n")).collect();
        let ev = AgentEvent::Subagent {
            parent: "p".into(),
            event: Box::new(AgentEvent::ToolResult {
                call_id: "call/1".into(),
                output: text.clone().into(),
                is_error: false,
            }),
        };
        let AgentEvent::Subagent { event, .. } = cap_event(root.path(), task, ev) else { panic!() };
        let AgentEvent::ToolResult { output, .. } = *event else { panic!() };
        let (body, spilled) = split(&output);
        let spilled = spilled.expect("marker");
        assert!(body.starts_with("row 0\n"));
        assert_eq!(spilled.lines, 3000);
        assert_eq!(spilled.path, root.path().join("spill").join(format!("{task}-call1.txt")));
        assert_eq!(std::fs::read_to_string(&spilled.path).unwrap(), text);
        // Small results pass through untouched.
        let small = AgentEvent::ToolResult { call_id: "c".into(), output: "ok".into(), is_error: false };
        assert_eq!(cap_event(root.path(), task, small.clone()), small);
    }
}
