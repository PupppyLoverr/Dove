//! The notch's own things: clipboard history, the file shelf, notes and the
//! timer. Pinned clips, the shelf and notes are saved in Dove's settings
//! store; the rest of the clipboard history lives only while Dove runs.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dove_core::store::Store;
use serde::{Deserialize, Serialize};

const CLIPS_KEY: &str = "notch_clips";
const SHELF_KEY: &str = "notch_shelf";
const NOTES_KEY: &str = "notch_notes";
/// Clipboard history keeps this many unpinned items.
const MAX_CLIPS: usize = 30;

#[derive(Clone, Debug)]
pub enum ClipBody {
    Text(String),
    Image(Arc<gpui::Image>),
}

#[derive(Clone, Debug)]
pub struct Clip {
    pub body: ClipBody,
    pub pinned: bool,
    pub at_ms: i64,
}

impl Clip {
    pub fn text(text: String) -> Self {
        Clip { body: ClipBody::Text(text), pinned: false, at_ms: dove_core::store::now_ms() }
    }

    pub fn image(image: Arc<gpui::Image>) -> Self {
        Clip { body: ClipBody::Image(image), pinned: false, at_ms: dove_core::store::now_ms() }
    }

    pub fn as_text(&self) -> Option<&str> {
        match &self.body {
            ClipBody::Text(t) => Some(t),
            ClipBody::Image(_) => None,
        }
    }

    /// What the text is, for its tile.
    pub fn kind(&self) -> ClipKind {
        match &self.body {
            ClipBody::Image(_) => ClipKind::Image,
            ClipBody::Text(t) => classify(t),
        }
    }

    fn same(&self, other: &Clip) -> bool {
        match (&self.body, &other.body) {
            (ClipBody::Text(a), ClipBody::Text(b)) => a == b,
            (ClipBody::Image(a), ClipBody::Image(b)) => a.bytes() == b.bytes(),
            _ => false,
        }
    }
}

/// Adds a copy to the front; a repeat moves up (keeping its pin), and the
/// oldest unpinned items fall off.
pub fn push_clip(clips: &mut Vec<Clip>, clip: Clip) {
    let pinned = match clips.iter().position(|c| c.same(&clip)) {
        Some(i) => clips.remove(i).pinned,
        None => false,
    };
    clips.insert(0, Clip { pinned, ..clip });
    let mut unpinned = 0;
    clips.retain(|c| {
        if c.pinned {
            return true;
        }
        unpinned += 1;
        unpinned <= MAX_CLIPS
    });
}

/// What a clipboard text is, so its tile can preview it well.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipKind {
    Text,
    Image,
    /// A web address, and its host.
    Link(String),
    /// A colour (#rrggbb, rgb(), hsl()), as RGB.
    Color(u8, u8, u8),
    /// Code, and a guess at its language.
    Code(&'static str),
}

/// Reads a copied text: a link, a colour, code, or plain text.
pub fn classify(text: &str) -> ClipKind {
    let t = text.trim();
    if !t.contains(char::is_whitespace)
        && let Ok(url) = url::Url::parse(t)
        && matches!(url.scheme(), "http" | "https")
    {
        return ClipKind::Link(url.host_str().unwrap_or_default().trim_start_matches("www.").to_string());
    }
    if let Some((r, g, b)) = parse_color(t) {
        return ClipKind::Color(r, g, b);
    }
    if let Some(lang) = code_language(t) {
        return ClipKind::Code(lang);
    }
    ClipKind::Text
}

fn parse_color(t: &str) -> Option<(u8, u8, u8)> {
    let t = t.trim().to_ascii_lowercase();
    if let Some(hex) = t.strip_prefix('#') {
        let hex: String = hex.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        if hex.len() != t.len() - 1 {
            return None;
        }
        let v = |i: usize, n: usize| {
            let s = &hex[i..i + n];
            u8::from_str_radix(&if n == 1 { s.repeat(2) } else { s.to_string() }, 16).ok()
        };
        return match hex.len() {
            3 | 4 => Some((v(0, 1)?, v(1, 1)?, v(2, 1)?)),
            6 | 8 => Some((v(0, 2)?, v(2, 2)?, v(4, 2)?)),
            _ => None,
        };
    }
    let inner = t.strip_prefix("rgb(").or_else(|| t.strip_prefix("rgba("))?.strip_suffix(')')?;
    let parts: Vec<u8> = inner
        .split([',', ' ', '/'])
        .filter(|p| !p.is_empty())
        .take(3)
        .map(|p| p.trim().parse::<f32>().ok().map(|v| v.clamp(0.0, 255.0) as u8))
        .collect::<Option<Vec<_>>>()?;
    (parts.len() == 3).then(|| (parts[0], parts[1], parts[2]))
}

/// Copied text that reads like an error: a stack trace, a compiler error,
/// failing tests or a crashed command. The island offers to have an agent
/// fix it.
pub fn looks_like_error(text: &str) -> bool {
    let t = text.trim();
    if t.len() < 24 || t.len() > 40_000 {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    const MARKERS: [&str; 14] = [
        "traceback (most recent call last)",
        "panicked at",
        "error[e",
        "uncaught exception",
        "unhandled exception",
        "unhandledpromiserejection",
        "npm err!",
        "exception in thread",
        "segmentation fault",
        "fatal error:",
        "syntaxerror:",
        "typeerror:",
        "referenceerror:",
        "build failed",
    ];
    if MARKERS.iter().any(|m| lower.contains(m)) {
        return true;
    }
    // A stack: several "at f (file:line:col)" / "file.py", line N" lines.
    let frames = t
        .lines()
        .filter(|l| {
            let l = l.trim_start();
            (l.starts_with("at ") && l.contains(':')) || (l.starts_with("File \"") && l.contains(", line "))
        })
        .count();
    if frames >= 2 {
        return true;
    }
    // "error: …" / "Error: …" at a line start, or failing tests.
    t.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("error:") || l.starts_with("Error:") || l.starts_with("ERROR ") || l.starts_with("FAILED ")
    }) || lower.contains(" failed;")
        || lower.contains("tests failed")
}

/// A cheap language guess for copied code (multi-line, with code-ish lines).
fn code_language(t: &str) -> Option<&'static str> {
    let lines: Vec<&str> = t.lines().filter(|l| !l.trim().is_empty()).collect();
    let codeish = lines
        .iter()
        .filter(|l| {
            let l = l.trim_end();
            l.ends_with(';')
                || l.ends_with('{')
                || l.ends_with('}')
                || l.ends_with(':')
                || l.starts_with("    ")
                || l.starts_with('\t')
        })
        .count();
    if lines.len() < 2 || codeish * 2 < lines.len() {
        return None;
    }
    let has = |k: &str| t.contains(k);
    Some(if has("fn ") && (has("let ") || has("->") || has("impl ")) {
        "Rust"
    } else if has("def ") || (has("import ") && has(":\n")) {
        "Python"
    } else if has("interface ") || has(": string") || has(": number") {
        "TypeScript"
    } else if has("function ") || has("const ") || has("=>") {
        "JavaScript"
    } else if has("func ") && has("package ") {
        "Go"
    } else if has("#include") {
        "C"
    } else if has("<div") || has("</") {
        "HTML"
    } else {
        "Code"
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub id: i64,
    pub text: String,
    /// Shown in the slim island.
    #[serde(default)]
    pub pinned: bool,
}

impl Note {
    pub fn new(text: String) -> Self {
        Note { id: dove_core::store::now_ms(), text, pinned: false }
    }

    /// The first line, without Markdown's heading marks.
    pub fn title(&self) -> String {
        let first = self.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        first.trim().trim_start_matches('#').trim_start_matches("- [ ]").trim_start_matches("- [x]").trim().to_string()
    }

    /// Ticks or unticks the checklist item on `line` (`- [ ]` / `- [x]`).
    pub fn toggle(&mut self, line: usize) {
        let mut lines: Vec<String> = self.text.lines().map(str::to_string).collect();
        if let Some(l) = lines.get_mut(line) {
            if let Some(rest) = l.trim_start().strip_prefix("- [ ]") {
                let indent = &l[..l.len() - l.trim_start().len()];
                *l = format!("{indent}- [x]{rest}");
            } else if let Some(rest) = l.trim_start().strip_prefix("- [x]") {
                let indent = &l[..l.len() - l.trim_start().len()];
                *l = format!("{indent}- [ ]{rest}");
            }
        }
        self.text = lines.join("\n");
    }

    /// Checklist progress (done, total), when the note has one.
    pub fn checklist(&self) -> Option<(usize, usize)> {
        let done = self.text.lines().filter(|l| l.trim_start().starts_with("- [x]")).count();
        let open = self.text.lines().filter(|l| l.trim_start().starts_with("- [ ]")).count();
        (done + open > 0).then_some((done, done + open))
    }
}

/// Something removed that a toast can still bring back.
#[derive(Clone, Debug)]
pub enum Undo {
    Note(usize, Note),
    Clips(Vec<Clip>),
    Shelf(usize, PathBuf),
}

/// A countdown that can pause.
#[derive(Clone, Debug)]
pub struct Timer {
    pub total: Duration,
    left: Duration,
    since: Option<Instant>,
}

impl Timer {
    pub fn new(total: Duration) -> Self {
        Timer { total, left: total, since: None }
    }

    pub fn remaining(&self) -> Duration {
        match self.since {
            Some(t) => self.left.saturating_sub(t.elapsed()),
            None => self.left,
        }
    }

    pub fn running(&self) -> bool {
        self.since.is_some()
    }

    /// Started and not back at its full length: worth showing.
    pub fn active(&self) -> bool {
        self.running() || self.left != self.total
    }

    pub fn progress(&self) -> f32 {
        if self.total.is_zero() { 0.0 } else { 1.0 - self.remaining().as_secs_f32() / self.total.as_secs_f32() }
    }

    pub fn finished(&self) -> bool {
        self.running() && self.remaining().is_zero()
    }

    pub fn toggle(&mut self) {
        match self.since.take() {
            Some(t) => self.left = self.left.saturating_sub(t.elapsed()),
            None => {
                if self.left.is_zero() {
                    self.left = self.total;
                }
                self.since = Some(Instant::now());
            }
        }
    }

    pub fn reset(&mut self) {
        self.since = None;
        self.left = self.total;
    }

    /// Sets a new length (a preset), stopped.
    pub fn set(&mut self, total: Duration) {
        self.total = total;
        self.reset();
    }

    /// Adds or takes a minute, running or not.
    pub fn nudge(&mut self, minutes: i64) {
        let d = Duration::from_secs(minutes.unsigned_abs() * 60);
        if minutes >= 0 {
            self.left += d;
            self.total += d;
        } else {
            self.left = self.left.saturating_sub(d).max(Duration::from_secs(1));
            self.total = self.total.saturating_sub(d).max(Duration::from_secs(60));
        }
    }
}

/// What's kept between launches.
pub struct Saved {
    pub clips: Vec<Clip>,
    pub shelf: Vec<PathBuf>,
    pub notes: Vec<Note>,
}

#[derive(Serialize, Deserialize)]
struct SavedClip {
    text: String,
    at_ms: i64,
}

impl Saved {
    pub fn load(store: &Store) -> Saved {
        let read = |key: &str| store.setting(key).unwrap_or_default();
        let clips = serde_json::from_str::<Vec<SavedClip>>(&read(CLIPS_KEY))
            .unwrap_or_default()
            .into_iter()
            .map(|c| Clip { body: ClipBody::Text(c.text), pinned: true, at_ms: c.at_ms })
            .collect();
        let shelf = serde_json::from_str::<Vec<PathBuf>>(&read(SHELF_KEY))
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p.exists())
            .collect();
        let notes = serde_json::from_str(&read(NOTES_KEY)).unwrap_or_default();
        Saved { clips, shelf, notes }
    }

    pub fn store(store: &Store, clips: &[Clip], shelf: &[PathBuf], notes: &[Note]) {
        let pinned: Vec<SavedClip> = clips
            .iter()
            .filter(|c| c.pinned)
            .filter_map(|c| Some(SavedClip { text: c.as_text()?.to_string(), at_ms: c.at_ms }))
            .collect();
        for (key, value) in [
            (CLIPS_KEY, serde_json::to_string(&pinned)),
            (SHELF_KEY, serde_json::to_string(shelf)),
            (NOTES_KEY, serde_json::to_string(notes)),
        ] {
            if let Ok(v) = value {
                let _ = store.set_setting(key, &v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_recognized_and_prose_is_not() {
        assert!(looks_like_error(
            "thread 'main' panicked at src/main.rs:3:5:
boom"
        ));
        assert!(looks_like_error(
            "Traceback (most recent call last):
  File \"a.py\", line 1, in <module>
KeyError: 'x'"
        ));
        assert!(looks_like_error(
            "TypeError: cannot read properties of undefined
    at f (app.js:10:3)
    at g (app.js:20:1)"
        ));
        assert!(looks_like_error(
            "error[E0308]: mismatched types
 --> src/lib.rs:4:5"
        ));
        assert!(!looks_like_error("Remember to buy milk and eggs on the way home tonight."));
        assert!(!looks_like_error("https://example.com/some/long/path?q=1"));
    }

    #[test]
    fn repeats_move_up_and_keep_their_pin() {
        let mut clips = Vec::new();
        push_clip(&mut clips, Clip::text("a".into()));
        push_clip(&mut clips, Clip::text("b".into()));
        clips[1].pinned = true;
        push_clip(&mut clips, Clip::text("a".into()));
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].as_text(), Some("a"));
        assert!(clips[0].pinned);
    }

    #[test]
    fn history_drops_the_oldest_unpinned() {
        let mut clips = Vec::new();
        push_clip(&mut clips, Clip::text("keep".into()));
        clips[0].pinned = true;
        for i in 0..MAX_CLIPS + 5 {
            push_clip(&mut clips, Clip::text(i.to_string()));
        }
        assert_eq!(clips.len(), MAX_CLIPS + 1);
        assert!(clips.iter().any(|c| c.as_text() == Some("keep")));
    }

    #[test]
    fn copies_are_classified() {
        assert_eq!(classify("https://www.github.com/dove"), ClipKind::Link("github.com".into()));
        assert_eq!(classify("#ff8800"), ClipKind::Color(255, 136, 0));
        assert_eq!(classify("#fff"), ClipKind::Color(255, 255, 255));
        assert_eq!(classify("rgb(10, 20, 30)"), ClipKind::Color(10, 20, 30));
        assert_eq!(classify("fn main() {\n    let a = 1;\n}"), ClipKind::Code("Rust"));
        assert_eq!(classify("just some words"), ClipKind::Text);
    }

    #[test]
    fn checklists_toggle_and_count() {
        let mut n = Note::new("# Groceries\n- [ ] milk\n- [x] eggs".into());
        assert_eq!(n.title(), "Groceries");
        assert_eq!(n.checklist(), Some((1, 2)));
        n.toggle(1);
        assert_eq!(n.checklist(), Some((2, 2)));
        assert!(n.text.contains("- [x] milk"));
    }

    #[test]
    fn timer_pauses_and_nudges() {
        let mut t = Timer::new(Duration::from_secs(300));
        assert!(!t.active());
        t.toggle();
        assert!(t.running());
        t.toggle();
        assert!(!t.running());
        t.nudge(1);
        assert!(t.remaining() > Duration::from_secs(350));
        t.reset();
        assert_eq!(t.remaining(), t.total);
    }
}
