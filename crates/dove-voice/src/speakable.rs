//! Turning an agent's streamed reply into short spoken clauses.
//!
//! Speech starts at the first clause, not the end of the reply. Code blocks,
//! diffs and tables are never read aloud: each becomes one short note that
//! the detail is in the chat. Markdown punctuation, URLs and file paths are
//! reduced to what a listener needs.

/// Accumulates streamed text and yields clauses ready to speak.
#[derive(Debug, Default)]
pub struct Chunker {
    /// The current line as streamed so far (not yet newline-terminated).
    raw: String,
    /// Spoken text waiting for a clause boundary.
    buf: String,
    in_code: bool,
    in_diff: bool,
    noted_code: bool,
    emitted: usize,
}

/// The first clause may go early (at a comma) so the voice starts sooner.
const FIRST_MIN: usize = 24;
const MIN: usize = 20;
const MAX: usize = 220;

impl Chunker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a delta; returns the clauses it completed.
    pub fn push(&mut self, delta: &str) -> Vec<String> {
        let mut out = Vec::new();
        for piece in split_keep_newlines(delta) {
            self.raw.push_str(piece);
            if self.raw.ends_with('\n') {
                let line = std::mem::take(&mut self.raw);
                self.line(line.trim_end_matches(['\n', '\r']), &mut out);
            } else {
                self.speak_partial_line(&mut out);
            }
        }
        out
    }

    /// Speak finished sentences of a line that is still streaming, unless the
    /// line may turn out to be code, a table or a diff.
    fn speak_partial_line(&mut self, out: &mut Vec<String>) {
        let t = self.raw.trim_start();
        if self.in_code
            || self.in_diff
            || t.len() < 3
            || t.starts_with(['`', '~', '|', '@', '+', '-'])
            || t.starts_with("diff")
        {
            return;
        }
        let Some(cut) = last_sentence_end(&self.raw) else { return };
        let done: String = self.raw.drain(..cut).collect();
        let spoken = speakable_line(&done);
        if !spoken.is_empty() {
            self.buf.push_str(&spoken);
            self.buf.push(' ');
        }
        self.flush_sentences(out, false);
    }

    /// The reply ended: speak whatever is left.
    pub fn finish(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        let rest = std::mem::take(&mut self.raw);
        if !rest.trim().is_empty() {
            self.line(&rest, &mut out);
        }
        self.flush_sentences(&mut out, true);
        out
    }

    fn line(&mut self, line: &str, out: &mut Vec<String>) {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            self.in_code = !self.in_code;
            if self.in_code {
                self.flush_sentences(out, true);
                if !self.noted_code {
                    self.noted_code = true;
                    out.push("I've put the code in the chat.".into());
                }
            }
            return;
        }
        if self.in_code {
            return;
        }
        if self.in_diff {
            if line.starts_with(['+', '-', ' ', '@', '\\']) || line.starts_with("index ") {
                return;
            }
            self.in_diff = false;
        }
        // Tables and diff hunks are for the screen.
        let diff =
            t.starts_with("@@ ") || t.starts_with("+++ ") || t.starts_with("--- ") || t.starts_with("diff --git");
        if diff {
            self.in_diff = true;
        }
        if t.starts_with('|') || diff {
            if !self.noted_code {
                self.noted_code = true;
                self.flush_sentences(out, true);
                out.push("The details are in the chat.".into());
            }
            return;
        }
        let spoken = speakable_line(t);
        if !spoken.is_empty() {
            self.buf.push_str(&spoken);
            self.buf.push(' ');
        }
        self.flush_sentences(out, false);
    }

    fn flush_sentences(&mut self, out: &mut Vec<String>, all: bool) {
        loop {
            let min = if self.emitted == 0 { FIRST_MIN } else { MIN };
            let cut = boundary(&self.buf, min, self.emitted == 0);
            match cut {
                Some(i) => {
                    let clause: String = self.buf.drain(..i).collect();
                    let clause = clean(&clause);
                    if !clause.is_empty() {
                        self.emitted += 1;
                        out.push(clause);
                    }
                }
                None if self.buf.len() > MAX => {
                    // A run-on line: cut at the last space.
                    let at = self.buf[..MAX].rfind(' ').unwrap_or(MAX);
                    let clause: String = self.buf.drain(..at).collect();
                    let clause = clean(&clause);
                    if !clause.is_empty() {
                        self.emitted += 1;
                        out.push(clause);
                    }
                }
                None => break,
            }
        }
        if all {
            let clause = clean(&std::mem::take(&mut self.buf));
            if !clause.is_empty() {
                self.emitted += 1;
                out.push(clause);
            }
        }
    }
}

fn split_keep_newlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if c == '\n' {
            out.push(&s[start..=i]);
            start = i + 1;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// Byte index just past a sentence end at or after `min` chars.
fn boundary(s: &str, min: usize, first: bool) -> Option<usize> {
    let bytes = s.as_bytes();
    for (i, c) in s.char_indices() {
        if i < min {
            continue;
        }
        let next_is_space = bytes.get(i + 1).is_none_or(|b| b.is_ascii_whitespace());
        if matches!(c, '.' | '!' | '?' | ';') && next_is_space && bytes.get(i + 1).is_some() {
            // "e.g." and decimals are not sentence ends.
            let prev = s[..i].chars().last();
            if c == '.' && prev.is_some_and(|p| p.is_ascii_digit()) {
                continue;
            }
            return Some(i + 1);
        }
        if first && c == ',' && next_is_space && bytes.get(i + 1).is_some() {
            return Some(i + 1);
        }
    }
    None
}

/// Byte index just past the last sentence end followed by a space.
fn last_sentence_end(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(1))
        .rev()
        .find(|&i| {
            matches!(b[i], b'.' | b'!' | b'?' | b';')
                && b[i + 1] == b' '
                && !(b[i] == b'.' && i > 0 && b[i - 1].is_ascii_digit())
        })
        .map(|i| i + 1)
}

fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One markdown line as something to say.
pub fn speakable_line(line: &str) -> String {
    let mut t = line.trim();
    // Headings, bullets, quotes, numbered items.
    t = t.trim_start_matches('#').trim_start();
    for p in ["- [ ] ", "- [x] ", "* ", "- ", "+ ", "> "] {
        if let Some(r) = t.strip_prefix(p) {
            t = r;
        }
    }
    if let Some((num, rest)) = t.split_once(". ")
        && num.chars().all(|c| c.is_ascii_digit())
        && !num.is_empty()
    {
        t = rest;
    }
    let mut out = String::with_capacity(t.len());
    let mut rest = t;
    while !rest.is_empty() {
        // [text](url) -> text
        if let Some(stripped) = rest.strip_prefix('[')
            && let Some(close) = stripped.find("](")
            && let Some(end) = stripped[close + 2..].find(')')
        {
            out.push_str(&stripped[..close]);
            rest = &stripped[close + 2 + end + 1..];
            continue;
        }
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            out.push_str("a link");
            rest = &rest[end..];
            continue;
        }
        let c = rest.chars().next().unwrap();
        match c {
            '*' | '_' | '`' | '~' => {}
            _ => out.push(c),
        }
        rest = &rest[c.len_utf8()..];
    }
    // Long paths: say the file name.
    out.split_whitespace()
        .map(|w| {
            let bare = w.trim_end_matches([',', '.', ':', ';', ')']);
            if bare.matches(['/', '\\']).count() >= 2 {
                let name = bare.rsplit(['/', '\\']).next().unwrap_or(bare);
                format!("{name}{}", &w[bare.len()..])
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(deltas: &[&str]) -> Vec<String> {
        let mut c = Chunker::new();
        let mut out = Vec::new();
        for d in deltas {
            out.extend(c.push(d));
        }
        out.extend(c.finish());
        out
    }

    #[test]
    fn first_clause_is_ready_before_the_reply_ends() {
        let mut c = Chunker::new();
        let first = c.push("I fixed the token refresh, and the tests now pass on Windows. ");
        assert_eq!(
            first,
            vec!["I fixed the token refresh,".to_string(), "and the tests now pass on Windows.".to_string()]
        );
    }

    #[test]
    fn code_and_diffs_are_not_read_aloud() {
        let out = all(&[
            "Here is the change:\n```rust\nfn main() {\n    rm_rf();\n}\n```\n",
            "It also updates ",
            "the docs.\n",
        ]);
        assert_eq!(out, vec!["Here is the change:", "I've put the code in the chat.", "It also updates the docs."]);
        assert!(!out.join(" ").contains("rm_rf"));
        let diff = all(&["Done.\ndiff --git a/x b/x\n@@ -1 +1 @@\n-old\n+new\nAll good.\n"]);
        assert_eq!(diff, vec!["Done.", "The details are in the chat.", "All good."]);
    }

    #[test]
    fn markdown_links_and_paths_are_spoken_plainly() {
        assert_eq!(speakable_line("## **Summary**"), "Summary");
        assert_eq!(
            speakable_line("- See [the docs](https://x.dev/a) or https://example.com/page"),
            "See the docs or a link"
        );
        assert_eq!(speakable_line("1. Edited `crates/dove-voice/src/lib.rs`."), "Edited lib.rs.");
    }

    #[test]
    fn decimals_do_not_split_sentences() {
        let out = all(&["The p50 latency dropped to 3.5 milliseconds after the change. Nice."]);
        assert_eq!(out[0], "The p50 latency dropped to 3.5 milliseconds after the change.");
    }

    #[test]
    fn run_on_text_is_cut_at_a_space() {
        let long = "word ".repeat(100);
        let out = all(&[&long]);
        assert!(out.iter().all(|c| c.len() <= MAX));
        assert_eq!(out.join(" ").split(' ').count(), 100);
    }
}
