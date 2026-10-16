//! Markdown to GPUI elements via pulldown-cmark.
//!
//! Streaming is incremental (the technique Zeron uses): [`Stream`] keeps the
//! source and the byte offset where its last *open* top-level block starts.
//! Each delta re-parses only that tail. Every top-level block before the last
//! one is final: it is parsed (and its code syntax-highlighted) once and
//! handed out as its own transcript row, so a long answer costs a few cheap
//! rows instead of one giant row re-parsed per token.
//!
//! The open tail is "mended" before parsing (unclosed `**` and backticks are
//! closed) so text does not flicker between styles mid-stream.

use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, ClipboardItem, FontStyle, FontWeight, InteractiveElement as _, InteractiveText, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, StrikethroughStyle, Styled as _,
    StyledImage as _, StyledText, TextRun, UnderlineStyle, Window, div, px,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use zeron_syntax::{HighlightRequest, HighlightedDocument};

#[path = "markdown_mend.rs"]
mod mend;

use crate::assets::{icon, icons};
use crate::design::ty;
use crate::markdown_extra::{self, Diagram};
use crate::theme::Theme;
use crate::ui;
use crate::widget;

const OPTS: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_TASKLISTS)
    .union(Options::ENABLE_MATH);

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Style {
    bold: bool,
    italic: bool,
    code: bool,
    strike: bool,
    link: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Inline {
    text: String,
    spans: Vec<(Range<usize>, Style)>,
    links: Vec<(Range<usize>, String)>,
    images: Vec<(String, String)>,
    /// `$$…$$` met inside the paragraph; rendered as math blocks after it.
    display: Vec<String>,
}

impl Inline {
    fn push(&mut self, s: &str, style: Style) {
        let start = self.text.len();
        self.text.push_str(s);
        if let Some((r, last)) = self.spans.last_mut()
            && *last == style
            && r.end == start
        {
            r.end = self.text.len();
            return;
        }
        self.spans.push((start..self.text.len(), style));
    }

    fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Paragraph(Inline),
    Heading(u8, Inline),
    /// `highlight` is filled for finished blocks with a known language.
    Code {
        lang: Option<String>,
        text: String,
        highlight: Option<Arc<HighlightedDocument>>,
    },
    Mermaid(String),
    Math(String),
    Widget(String),
    /// A live design (`show_canvas`), as its JSON spec.
    Canvas(String),
    /// A task list item's checkbox (first block of the item).
    Check(bool),
    Image {
        alt: String,
        url: String,
    },
    List {
        ordered: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Quote(Vec<Block>),
    Table(Vec<Vec<Inline>>),
    Rule,
}

impl Block {
    /// Plain text, for copy actions.
    pub fn plain(&self) -> String {
        match self {
            Block::Paragraph(i) | Block::Heading(_, i) => i.text.clone(),
            Block::Code { text, .. } | Block::Mermaid(text) | Block::Math(text) | Block::Widget(text) => text.clone(),
            Block::Canvas(src) => crate::canvas::Spec::parse(src).map(|s| s.code).unwrap_or_default(),
            Block::Check(done) => {
                if *done {
                    "[x]".into()
                } else {
                    "[ ]".into()
                }
            }
            Block::Image { alt, url } => {
                if alt.is_empty() {
                    url.clone()
                } else {
                    alt.clone()
                }
            }
            Block::List { items, .. } => items
                .iter()
                .map(|item| item.iter().map(Block::plain).collect::<Vec<_>>().join("\n"))
                .collect::<Vec<_>>()
                .join("\n"),
            Block::Quote(inner) => inner.iter().map(Block::plain).collect::<Vec<_>>().join("\n"),
            Block::Table(rows) => rows
                .iter()
                .map(|r| r.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\t"))
                .collect::<Vec<_>>()
                .join("\n"),
            Block::Rule => "---".into(),
        }
    }
}

/// Parse a complete document (no highlighting; used for plans and tests).
pub fn parse(src: &str) -> Arc<Vec<Block>> {
    let mut parser = Parser::new_ext(src, OPTS).peekable();
    Arc::new(parse_blocks(&mut parser, None))
}

/// Parse finished markdown into rows, highlighting code blocks.
#[cfg(test)]
pub fn parse_final(src: &str) -> Vec<Arc<Block>> {
    let mut parser = Parser::new_ext(src, OPTS).peekable();
    parse_blocks(&mut parser, None).into_iter().map(|b| Arc::new(highlight_block(b))).collect()
}

/// Parse finished markdown into rows without highlighting (the UI thread
/// path; highlighting follows off-thread).
pub fn parse_final_plain(src: &str) -> Vec<Arc<Block>> {
    let mut parser = Parser::new_ext(src, OPTS).peekable();
    parse_blocks(&mut parser, None).into_iter().map(Arc::new).collect()
}

impl Block {
    /// Whether highlighting would change this block.
    pub fn wants_highlight(&self) -> bool {
        matches!(self, Block::Code { lang: Some(_), highlight: None, .. })
    }
}

pub fn highlight_block(b: Block) -> Block {
    match b {
        Block::Code { lang, text, .. } => {
            let highlight = lang.as_deref().and_then(|l| {
                zeron_syntax::highlight(HighlightRequest { source: &text, path: None, fence_tag: Some(l) })
                    .ok()
                    .map(Arc::new)
            });
            Block::Code { lang, text, highlight }
        }
        other => other,
    }
}

/// Each top-level block of `src`: its start offset and whether it is a list.
fn top_level_starts(src: &str) -> Vec<(usize, bool)> {
    let mut depth = 0usize;
    let mut out = Vec::new();
    for (ev, r) in Parser::new_ext(src, OPTS).into_offset_iter() {
        match ev {
            Event::Start(tag) => {
                if depth == 0 {
                    out.push((r.start, matches!(tag, Tag::List(_))));
                }
                depth += 1;
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Rule | Event::Html(_) if depth == 0 => out.push((r.start, false)),
            _ => {}
        }
    }
    out
}

/// The last offset at which `tail` can be cut so everything before it is
/// final: the next block must follow a blank line (a table, paragraph or
/// setext heading can still grow otherwise), and a list only ends at an
/// unindented non-list block.
fn stable_cut(tail: &str) -> Option<usize> {
    let starts = top_level_starts(tail);
    (1..starts.len()).rev().find_map(|j| {
        let (start, is_list) = starts[j];
        let prev_list = starts[j - 1].1;
        let before = tail[..start].trim_end_matches([' ', '\t']);
        let blank = before.ends_with("\n\n") || before.ends_with("\r\n\r\n");
        let indented = tail[start..].starts_with([' ', '\t']);
        let ok = blank && (!prev_list || (!is_list && !indented));
        ok.then_some(start)
    })
}

/// Close dangling inline markers in an open tail so it renders stably
/// (Zeron's `close_hanging`, see [`mend`]).
fn mend_tail(tail: &str) -> std::borrow::Cow<'_, str> {
    match mend::close_hanging(tail) {
        Some(fixed) => fixed.into(),
        None => tail.into(),
    }
}

/// A fence opener line at column 0: ```` ```lang ```` or `~~~lang`.
fn fence_opener(line: &str) -> Option<(u8, usize)> {
    let b = line.as_bytes();
    let ch = *b.first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let len = b.iter().take_while(|&&c| c == ch).count();
    // A backtick fence's info string may not contain backticks.
    (len >= 3 && !(ch == b'`' && line[len..].contains('`'))).then_some((ch, len))
}

/// Whether `line` closes a fence opened with `len` × `ch`.
fn fence_closer(line: &str, ch: u8, len: usize) -> bool {
    let t = line.trim_start_matches(' ');
    if line.len() - t.len() > 3 {
        return false;
    }
    let run = t.bytes().take_while(|&c| c == ch).count();
    run >= len && t[run..].trim().is_empty()
}

/// An assistant message arriving token by token.
///
/// Per delta the work is O(open tail), never O(message): finished blocks are
/// cut off and parsed once, and the cut search itself only runs when a blank
/// line followed by new content arrived (a top-level block can only start
/// there). An open fenced code block at the tail skips the markdown parser
/// entirely, so a 30 KB code listing streams as cheaply as a sentence.
#[derive(Default)]
pub struct Stream {
    source: String,
    /// Start of the open (last) top-level block.
    stable_end: usize,
    /// Complete lines of the tail up to here have been scanned.
    scanned: usize,
    /// The fence the scan is inside: (marker, run length, opener offset).
    fence: Option<(u8, usize, usize)>,
    /// A blank line was seen and no content has followed it yet.
    saw_blank: bool,
    /// Content followed a blank line: a cut may now exist.
    check: bool,
}

pub struct StreamUpdate {
    /// Blocks that just became final, in order.
    pub finished: Vec<Arc<Block>>,
    /// The open tail, re-parsed (usually one block, sometimes none).
    pub live: Vec<Arc<Block>>,
}

impl Stream {
    pub fn push(&mut self, delta: &str) -> StreamUpdate {
        let finished = self.ingest(delta);
        StreamUpdate { finished, live: self.live_blocks() }
    }

    /// Append `delta` and cut any top-level block that just closed.
    ///
    /// Does not re-parse the open tail. A hidden thread calls this on every
    /// coalesced batch, then [`Self::live_blocks`] once when the thread is
    /// shown. Re-parsing that tail on every token is most of the cost of a
    /// background stream.
    pub fn ingest(&mut self, delta: &str) -> Vec<Arc<Block>> {
        self.source.push_str(delta);
        self.scan();
        let mut finished = Vec::new();
        if std::mem::take(&mut self.check) {
            let tail = &self.source[self.stable_end..];
            if let Some(cut) = stable_cut(tail) {
                finished = parse_final_plain(&tail[..cut]);
                self.stable_end += cut;
            }
        }
        finished
    }

    /// The open tail, parsed for display.
    pub fn live_blocks(&mut self) -> Vec<Arc<Block>> {
        match self.open_code() {
            Some(b) => vec![Arc::new(b)],
            None => {
                let mended = mend_tail(&self.source[self.stable_end..]);
                let mut parser = Parser::new_ext(&mended, OPTS).peekable();
                parse_blocks(&mut parser, None).into_iter().map(Arc::new).collect()
            }
        }
    }

    /// Track fences and blank lines over the newly completed lines.
    fn scan(&mut self) {
        let from = self.scanned.max(self.stable_end);
        let Some(nl) = self.source[from..].rfind('\n') else {
            self.note_partial(from);
            return;
        };
        let upto = from + nl + 1;
        let mut at = from;
        for line in self.source[from..upto].split_inclusive('\n') {
            let text = line.trim_end_matches(['\n', '\r']);
            match self.fence {
                Some((ch, len, _)) => {
                    if fence_closer(text, ch, len) {
                        self.fence = None;
                    }
                }
                None if text.trim().is_empty() => self.saw_blank = true,
                None => {
                    if std::mem::take(&mut self.saw_blank) {
                        self.check = true;
                    }
                    if let Some((ch, len)) = fence_opener(text) {
                        self.fence = Some((ch, len, at));
                    }
                }
            }
            at += line.len();
        }
        self.scanned = upto;
        self.note_partial(upto);
    }

    /// New content on an unfinished line after a blank line.
    fn note_partial(&mut self, from: usize) {
        if self.fence.is_none() && self.saw_blank && !self.source[from..].trim().is_empty() {
            self.saw_blank = false;
            self.check = true;
        }
    }

    /// The tail is exactly one open fenced code block: build it directly.
    fn open_code(&self) -> Option<Block> {
        let (_, _, opener) = self.fence?;
        if opener != self.stable_end {
            return None;
        }
        let tail = &self.source[self.stable_end..];
        let (head, body) = tail.split_once('\n')?;
        let head = head.trim_end_matches('\r');
        let info = head.trim_start_matches(['`', '~']).trim();
        let lang = info.split_whitespace().next().filter(|l| !l.is_empty()).map(str::to_string);
        let text = body.strip_suffix('\n').unwrap_or(body).replace("\r\n", "\n");
        Some(match lang.as_deref().map(str::to_ascii_lowercase).as_deref() {
            // A diagram renders once whole; while it streams it is source.
            Some("math" | "latex" | "tex") => Block::Math(text),
            Some("ui" | "widget") => Block::Widget(text),
            _ => Block::Code { lang, text, highlight: None },
        })
    }

    /// The message ended: everything left is final.
    pub fn finish(&mut self) -> Vec<Arc<Block>> {
        let out = parse_final_plain(&self.source[self.stable_end..]);
        self.stable_end = self.source.len();
        out
    }
}

type P<'a> = std::iter::Peekable<Parser<'a>>;

fn parse_blocks(p: &mut P, until: Option<TagEnd>) -> Vec<Block> {
    let mut blocks = Vec::new();
    // Tight list items emit inline text with no Paragraph wrapper.
    let mut loose = Inline::default();
    let mut style = Style::default();
    while let Some(ev) = p.next() {
        match ev {
            Event::End(end) if Some(end) == until => break,
            Event::Start(Tag::Paragraph) => {
                flush(&mut loose, &mut blocks);
                let mut inl = parse_inline(p, TagEnd::Paragraph);
                let display = std::mem::take(&mut inl.display);
                if inl.images.len() == 1 && inl.images[0].0 == inl.text {
                    let (alt, url) = inl.images.into_iter().next().unwrap();
                    blocks.push(Block::Image { alt, url });
                } else if !inl.text.trim().is_empty() {
                    blocks.push(Block::Paragraph(inl));
                }
                blocks.extend(display.into_iter().map(Block::Math));
            }
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut loose, &mut blocks);
                let n = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    _ => 4,
                };
                blocks.push(Block::Heading(n, parse_inline(p, TagEnd::Heading(level))));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut loose, &mut blocks);
                let lang = match kind {
                    CodeBlockKind::Fenced(l) if !l.is_empty() => {
                        Some(l.split_whitespace().next().unwrap_or_default().to_string())
                    }
                    _ => None,
                };
                let mut text = String::new();
                for ev in p.by_ref() {
                    match ev {
                        Event::Text(t) => text.push_str(&t),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                if text.ends_with('\n') {
                    text.pop();
                }
                let kind = lang.as_deref().map(|s| s.to_ascii_lowercase());
                blocks.push(match kind.as_deref() {
                    Some("mermaid") => Block::Mermaid(text),
                    Some("math" | "latex" | "tex") => Block::Math(text),
                    Some("ui" | "widget" | "dove-ui") => Block::Widget(text),
                    Some("canvas" | "dove-canvas") => Block::Canvas(text),
                    _ => Block::Code { lang, text, highlight: None },
                });
            }
            Event::Start(Tag::List(start)) => {
                flush(&mut loose, &mut blocks);
                let mut items = Vec::new();
                loop {
                    match p.next() {
                        Some(Event::Start(Tag::Item)) => items.push(parse_blocks(p, Some(TagEnd::Item))),
                        Some(Event::End(TagEnd::List(_))) | None => break,
                        _ => {}
                    }
                }
                blocks.push(Block::List { ordered: start, items });
            }
            Event::Start(Tag::BlockQuote(_)) => {
                flush(&mut loose, &mut blocks);
                blocks.push(Block::Quote(parse_blocks(p, Some(TagEnd::BlockQuote(None)))));
            }
            Event::Start(Tag::Table(_)) => {
                flush(&mut loose, &mut blocks);
                blocks.push(Block::Table(parse_table(p)));
            }
            Event::Rule => {
                flush(&mut loose, &mut blocks);
                blocks.push(Block::Rule);
            }
            // Inline content directly in a container (tight list items).
            Event::Text(t) => loose.push(&t, style),
            Event::Code(t) => {
                let start = loose.text.len();
                loose.push(&t, Style { code: true, ..style });
                if looks_like_path(&t) {
                    loose.links.push((start..loose.text.len(), format!("{FILE_SCHEME}{t}")));
                }
            }
            Event::SoftBreak => loose.push(" ", style),
            Event::HardBreak => loose.push("\n", style),
            Event::TaskListMarker(done) => {
                flush(&mut loose, &mut blocks);
                blocks.push(Block::Check(done));
            }
            Event::InlineMath(t) => loose.push(&crate::math::inline(&t), Style { italic: true, ..style }),
            Event::DisplayMath(t) => {
                flush(&mut loose, &mut blocks);
                blocks.push(Block::Math(t.to_string()));
            }
            Event::Start(Tag::Strong) => style.bold = true,
            Event::End(TagEnd::Strong) => style.bold = false,
            Event::Start(Tag::Emphasis) => style.italic = true,
            Event::End(TagEnd::Emphasis) => style.italic = false,
            Event::Start(Tag::Strikethrough) => style.strike = true,
            Event::End(TagEnd::Strikethrough) => style.strike = false,
            Event::End(e)
                if matches!(e, TagEnd::BlockQuote(_)) && until.is_some_and(|u| matches!(u, TagEnd::BlockQuote(_))) =>
            {
                break;
            }
            _ => {}
        }
    }
    flush(&mut loose, &mut blocks);
    blocks
}

fn flush(loose: &mut Inline, blocks: &mut Vec<Block>) {
    if !loose.is_empty() {
        blocks.push(Block::Paragraph(std::mem::take(loose)));
    } else {
        *loose = Inline::default();
    }
}

fn parse_inline(p: &mut P, end: TagEnd) -> Inline {
    let mut out = Inline::default();
    let mut style = Style::default();
    let mut link: Option<(usize, String)> = None;
    while let Some(ev) = p.next() {
        match ev {
            Event::End(e) if e == end => break,
            Event::Text(t) => out.push(&t, style),
            Event::Code(t) => {
                let start = out.text.len();
                out.push(&t, Style { code: true, ..style });
                if link.is_none() && looks_like_path(&t) {
                    out.links.push((start..out.text.len(), format!("{FILE_SCHEME}{t}")));
                }
            }
            Event::SoftBreak => out.push(" ", style),
            Event::HardBreak => out.push("\n", style),
            Event::Start(Tag::Strong) => style.bold = true,
            Event::End(TagEnd::Strong) => style.bold = false,
            Event::Start(Tag::Emphasis) => style.italic = true,
            Event::End(TagEnd::Emphasis) => style.italic = false,
            Event::Start(Tag::Strikethrough) => style.strike = true,
            Event::End(TagEnd::Strikethrough) => style.strike = false,
            Event::InlineMath(t) => out.push(&crate::math::inline(&t), Style { italic: true, ..style }),
            Event::DisplayMath(t) => out.display.push(t.to_string()),
            Event::TaskListMarker(done) => out.push(if done { "☑ " } else { "☐ " }, style),
            Event::Start(Tag::Image { dest_url, .. }) => {
                let start = out.text.len();
                let mut alt = String::new();
                for ev in p.by_ref() {
                    match ev {
                        Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                        Event::End(TagEnd::Image) => break,
                        _ => {}
                    }
                }
                let label = if alt.is_empty() { dest_url.to_string() } else { alt };
                out.images.push((label.clone(), dest_url.to_string()));
                out.push(&label, Style { link: true, ..style });
                out.links.push((start..out.text.len(), dest_url.to_string()));
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                style.link = true;
                link = Some((out.text.len(), dest_url.to_string()));
            }
            Event::End(TagEnd::Link) => {
                style.link = false;
                if let Some((start, url)) = link.take() {
                    out.links.push((start..out.text.len(), url));
                }
            }
            Event::Html(h) | Event::InlineHtml(h) => out.push(&h, style),
            _ => {}
        }
    }
    out
}

/// Links to files in the worktree use this scheme; clicks open the preview.
pub const FILE_SCHEME: &str = "dove-file:";

/// `src/main.rs`, `main.rs:42`, `crates/x/lib.rs:10:5`; not `foo()` or `a b`.
pub fn looks_like_path(s: &str) -> bool {
    if s.is_empty() || s.len() > 240 || s.contains(char::is_whitespace) || s.contains("://") {
        return false;
    }
    let (path, _) = split_path_line(s);
    let file = path.rsplit(['/', '\\']).next().unwrap_or(&path);
    let Some((stem, ext)) = file.rsplit_once('.') else { return false };
    !stem.is_empty()
        && (1..=6).contains(&ext.len())
        && ext.chars().all(|c| c.is_ascii_alphanumeric())
        && ext.chars().any(|c| c.is_ascii_alphabetic())
        && path.chars().all(|c| c.is_alphanumeric() || "/\\._-@+:".contains(c))
}

/// Split `path:line[:col]` into its parts. A Windows drive prefix stays part
/// of the path.
pub fn split_path_line(s: &str) -> (String, Option<u32>) {
    let (drive, rest) = match s.as_bytes() {
        [d, b':', b'\\' | b'/', ..] if d.is_ascii_alphabetic() => s.split_at(2),
        _ => ("", s),
    };
    let mut parts = rest.splitn(3, ':');
    let path = format!("{drive}{}", parts.next().unwrap_or(rest));
    let line = parts.next().and_then(|l| l.parse().ok());
    (path, line)
}

fn parse_table(p: &mut P) -> Vec<Vec<Inline>> {
    let mut rows = Vec::new();
    let mut row: Vec<Inline> = Vec::new();
    loop {
        match p.next() {
            Some(Event::Start(Tag::TableCell)) => row.push(parse_inline(p, TagEnd::TableCell)),
            Some(Event::End(TagEnd::TableRow)) | Some(Event::End(TagEnd::TableHead)) => {
                rows.push(std::mem::take(&mut row));
            }
            Some(Event::End(TagEnd::Table)) | None => break,
            _ => {}
        }
    }
    rows
}

// ------------------------------------------------------------------ render

/// Several blocks stacked (plans, small documents).
pub fn render(id: impl Into<SharedString>, blocks: &[Block], theme: &Theme, window: &Window) -> AnyElement {
    let id: SharedString = id.into();
    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .children(blocks.iter().enumerate().map(|(i, b)| render_block(&format!("{id}-{i}"), b, theme, window)))
        .into_any_element()
}

/// One top-level block (one transcript row).
pub fn render_block(id: &str, b: &Block, theme: &Theme, window: &Window) -> AnyElement {
    match b {
        Block::Paragraph(inl) => inline(id, inl, theme, window).into_any_element(),
        Block::Heading(level, inl) => {
            let size = match level {
                1 => 20.0,
                2 => 17.0,
                3 => 15.5,
                _ => 14.5,
            };
            div()
                .mt(px(if *level <= 2 { 6.0 } else { 2.0 }))
                .text_size(px(size))
                .font_weight(FontWeight::SEMIBOLD)
                .child(inline(id, inl, theme, window))
                .into_any_element()
        }
        Block::Code { lang, text, highlight } => code_block(id, lang.as_deref(), text, highlight.as_deref(), theme),
        Block::Mermaid(src) => mermaid_block(id, src, theme),
        Block::Math(src) => math_block(src, theme),
        Block::Check(done) => checkbox(*done, theme),
        Block::Widget(src) => widget_preview(src, theme),
        Block::Canvas(src) => match crate::canvas::Spec::parse(src) {
            Some(spec) => crate::canvas::card(&spec, theme, |_, _| {}),
            None => div().into_any_element(),
        },
        Block::Image { alt, url } => image_block(id, alt, url, theme),
        Block::List { ordered, items } => div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .children(items.iter().enumerate().map(|(i, item)| {
                // Task items show their checkbox in place of the bullet.
                let check = match item.first() {
                    Some(Block::Check(done)) => Some(*done),
                    _ => None,
                };
                let marker = match (check, ordered) {
                    (Some(done), _) => div().pt(px(3.0)).child(checkbox(done, theme)).into_any_element(),
                    (None, Some(start)) => {
                        div().text_color(theme.text_muted).child(format!("{}.", start + i as u64)).into_any_element()
                    }
                    (None, None) => div().text_color(theme.text_muted).child("•").into_any_element(),
                };
                let done = check == Some(true);
                div().flex().gap(px(8.0)).child(div().flex_none().min_w(px(14.0)).child(marker)).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .when(done, |d| d.text_color(theme.text_muted))
                        .children(
                            item.iter()
                                .enumerate()
                                .filter(|(_, b)| !matches!(b, Block::Check(_)))
                                .map(|(j, b)| render_block(&format!("{id}-{i}-{j}"), b, theme, window)),
                        ),
                )
            }))
            .into_any_element(),
        Block::Quote(inner) => div()
            .pl(px(12.0))
            .border_l_2()
            .border_color(theme.border_strong)
            .text_color(theme.text_muted)
            .flex()
            .flex_col()
            .gap(px(6.0))
            .children(inner.iter().enumerate().map(|(j, b)| render_block(&format!("{id}-q{j}"), b, theme, window)))
            .into_any_element(),
        Block::Table(rows) => div()
            .rounded(px(8.0))
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .children(rows.iter().enumerate().map(|(r, row)| {
                div()
                    .flex()
                    .when(r == 0, |d| d.bg(theme.wash(0.05)).font_weight(FontWeight::MEDIUM))
                    .when(r > 0, |d| d.border_t_1().border_color(theme.border))
                    .children(row.iter().enumerate().map(|(c, cell)| {
                        div().flex_1().min_w_0().px(px(10.0)).py(px(6.0)).text_size(ty::BODY).child(inline(
                            &format!("{id}-t{r}-{c}"),
                            cell,
                            theme,
                            window,
                        ))
                    }))
            }))
            .into_any_element(),
        Block::Rule => div().h(px(1.0)).bg(theme.border).into_any_element(),
    }
}

fn code_line(line: &str, spans: Option<&[zeron_syntax::HighlightSpan]>, theme: &Theme, window: &Window) -> AnyElement {
    if line.is_empty() {
        return div().child(" ").into_any_element();
    }
    let Some(spans) = spans.filter(|s| !s.is_empty()) else {
        return div().child(line.to_string()).into_any_element();
    };
    let mut font = window.text_style().font();
    font.family = theme.font_mono.clone();
    let run = |len: usize, color| TextRun {
        len,
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let mut runs = Vec::with_capacity(spans.len() * 2 + 1);
    let mut at = 0usize;
    for s in spans {
        let (start, end) = (s.range.start.min(line.len()), s.range.end.min(line.len()));
        if start < at || start >= end {
            continue;
        }
        if start > at {
            runs.push(run(start - at, theme.text));
        }
        runs.push(run(end - start, theme.syntax(s.kind)));
        at = end;
    }
    if at < line.len() {
        runs.push(run(line.len() - at, theme.text));
    }
    StyledText::new(SharedString::from(line.to_string())).with_runs(runs).into_any_element()
}

fn mermaid_block(id: &str, src: &str, theme: &Theme) -> AnyElement {
    let body = match markdown_extra::mermaid(src, theme) {
        Diagram::Ready(img, (w, h)) => {
            // Fit the width, keep the aspect, never taller than 460px.
            let height = h.min(460.0).min(h * (720.0 / w.max(1.0)).min(1.0)).max(40.0);
            div()
                .w_full()
                .h(px(height))
                .flex()
                .justify_center()
                .overflow_hidden()
                .child(gpui::img(img).size_full().object_fit(gpui::ObjectFit::Contain))
                .into_any_element()
        }
        Diagram::Failed(err) => div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(div().text_size(ty::META).text_color(theme.danger).child(err))
            .child(
                div()
                    .font_family(theme.font_mono.clone())
                    .text_size(ty::META)
                    .text_color(theme.text_muted)
                    .child(src.to_string()),
            )
            .into_any_element(),
    };
    div()
        .id(SharedString::from(format!("{id}-mermaid")))
        .rounded(px(12.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.wash(0.03))
        .p(px(12.0))
        .child(div().pb(px(8.0)).text_size(ty::CAPTION).text_color(theme.text_faint).child("Diagram"))
        .child(body)
        .into_any_element()
}

fn math_block(src: &str, theme: &Theme) -> AnyElement {
    div()
        .id(SharedString::from(format!("math-{}", src.len())))
        .w_full()
        .py(px(14.0))
        .px(px(14.0))
        .rounded(px(12.0))
        .bg(theme.wash(0.035))
        .overflow_x_scroll()
        .tooltip(crate::ui::tip(src.trim().to_string()))
        .child(crate::math::block(src, 19.0, theme.text))
        .into_any_element()
}

fn checkbox(done: bool, theme: &Theme) -> AnyElement {
    div()
        .size(px(15.0))
        .flex_none()
        .rounded(px(4.5))
        .border_1()
        .flex()
        .items_center()
        .justify_center()
        .when(done, |d| d.bg(theme.accent).border_color(theme.accent))
        .when(!done, |d| d.border_color(theme.border_strong))
        .when(done, |d| d.child(icon(icons::CHECK).size(px(10.0)).text_color(gpui::white())))
        .into_any_element()
}

fn widget_preview(src: &str, theme: &Theme) -> AnyElement {
    let spec = widget::Spec::parse(src).or_else(|| widget::Spec::parse_partial(src));
    let title = spec.as_ref().map(|s| s.title.clone()).filter(|s| !s.is_empty());
    let body = spec.map(|s| s.body).filter(|s| !s.is_empty());
    div()
        .p(px(14.0))
        .rounded(px(14.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.wash(0.03))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .text_size(ty::BODY)
                .text_color(theme.text_muted)
                .child(icon(icons::WIDGET).size(px(13.0)).text_color(theme.text_faint))
                .child(title.unwrap_or_else(|| "Building card…".into())),
        )
        .when_some(body, |d, body| d.child(div().text_size(ty::BODY).text_color(theme.text).child(body)))
        .into_any_element()
}

/// Images the user clicked to see at full size.
static ZOOMED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);

const IMAGE_FOLDED: f32 = 360.0;

fn image_block(id: &str, alt: &str, url: &str, theme: &Theme) -> AnyElement {
    let key = format!("{id}\u{0}{url}");
    let zoomed = ZOOMED.lock().map(|z| z.contains(&key)).unwrap_or(false);
    let fit = |el: gpui::Img| {
        el.w_full().when(!zoomed, |d| d.max_h(px(IMAGE_FOLDED))).object_fit(gpui::ObjectFit::Contain).into_any_element()
    };
    let picture = markdown_extra::preview_image(url)
        .map(|img| fit(gpui::img(img)))
        .unwrap_or_else(|| fit(gpui::img(SharedString::from(url.to_string()))));
    let href = url.to_string();
    let hover = theme.glass_hover();
    div()
        .id(SharedString::from(format!("{id}-img")))
        .group(SharedString::from(format!("{id}-img")))
        .relative()
        .rounded(px(14.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.wash(0.02))
        .overflow_hidden()
        .flex()
        .flex_col()
        .cursor_pointer()
        .tooltip(ui::tip(if zoomed { "Click to fit" } else { "Click to zoom" }))
        .child(picture)
        .when(!alt.is_empty(), |d| {
            d.child(
                div().px(px(10.0)).py(px(6.0)).text_size(ty::META).text_color(theme.text_faint).child(alt.to_string()),
            )
        })
        .when(href.starts_with("http"), |d| {
            let href = href.clone();
            d.child(
                div()
                    .id(SharedString::from(format!("{id}-img-open")))
                    .absolute()
                    .top(px(8.0))
                    .right(px(8.0))
                    .size(px(ui::BUTTON))
                    .rounded(px(ui::BUTTON_RADIUS))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.surface_overlay)
                    .border_1()
                    .border_color(theme.border)
                    .invisible()
                    .group_hover(SharedString::from(format!("{id}-img")), |s| s.visible())
                    .hover(move |s| s.bg(hover))
                    .tooltip(ui::tip("Open image"))
                    .child(icon(icons::ARROW_UP_RIGHT).size(px(ui::ICON_SM)).text_color(theme.text_muted))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        cx.open_url(&href);
                    }),
            )
        })
        .on_click(move |_, window, _| {
            if let Ok(mut z) = ZOOMED.lock()
                && !z.remove(&key)
            {
                z.insert(key.clone());
            }
            window.refresh();
        })
        .into_any_element()
}

/// Fenced code in the conversation: Zeron's 12.5 / 18 at the default code
/// size, tracking Settings › Appearance › Code & diff font size.
fn code_block_size() -> f32 {
    crate::design::code() + 0.5
}

fn code_block_line() -> f32 {
    (code_block_size() * 18.0 / 12.5).round()
}

fn code_block(id: &str, lang: Option<&str>, text: &str, hl: Option<&HighlightedDocument>, theme: &Theme) -> AnyElement {
    let copy = text.to_string();
    let theme2 = theme.clone();
    div()
        .rounded(px(10.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.wash(0.035))
        .overflow_hidden()
        .child(
            div()
                .h(px(30.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(theme.border)
                .text_size(ty::META)
                .text_color(theme.text_faint)
                .child(lang.unwrap_or("").to_string())
                .child(
                    ui::row_button(format!("{id}-copy"), icons::COPY, "Copy code", theme)
                        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))),
                ),
        )
        .child(
            div()
                .id(SharedString::from(format!("{id}-code")))
                .px(px(12.0))
                .py(px(10.0))
                .overflow_x_scroll()
                .font_family(theme.font_mono.clone())
                .text_size(px(code_block_size()))
                .line_height(px(code_block_line()))
                .whitespace_nowrap()
                .flex()
                .flex_col()
                .children(text.lines().enumerate().map(move |(i, l)| LineEl {
                    line: l.to_string(),
                    spans: hl.and_then(|h| h.lines.get(i)).cloned(),
                    theme: theme2.clone(),
                })),
        )
        .into_any_element()
}

/// A code line that resolves the window font at render time.
#[derive(IntoElement)]
struct LineEl {
    line: String,
    spans: Option<Vec<zeron_syntax::HighlightSpan>>,
    theme: Theme,
}

impl gpui::RenderOnce for LineEl {
    fn render(self, window: &mut Window, _: &mut gpui::App) -> impl IntoElement {
        code_line(&self.line, self.spans.as_deref(), &self.theme, window)
    }
}

fn inline(id: &str, inl: &Inline, theme: &Theme, window: &Window) -> AnyElement {
    let base = window.text_style();
    let mut runs = Vec::with_capacity(inl.spans.len());
    for (range, s) in &inl.spans {
        let mut font = base.font();
        if s.bold {
            font.weight = FontWeight::SEMIBOLD;
        }
        if s.italic {
            font.style = FontStyle::Italic;
        }
        if s.code {
            font.family = theme.font_mono.clone();
        }
        // Inline code stays monochrome on a neutral wash: calm in long answers.
        let color = if s.link && !s.code { theme.accent } else { base.color };
        runs.push(TextRun {
            len: range.len(),
            font,
            color,
            background_color: s.code.then_some(theme.wash(0.09)),
            underline: s.link.then(|| UnderlineStyle {
                thickness: px(1.0),
                color: Some(theme.accent.opacity(0.5)),
                wavy: false,
            }),
            strikethrough: s.strike.then(|| StrikethroughStyle { thickness: px(1.0), color: Some(base.color) }),
        });
    }
    let styled = StyledText::new(SharedString::from(inl.text.clone())).with_runs(runs);
    if inl.links.is_empty() {
        return styled.into_any_element();
    }
    let ranges: Vec<Range<usize>> = inl.links.iter().map(|(r, _)| r.clone()).collect();
    let urls: Vec<String> = inl.links.iter().map(|(_, u)| u.clone()).collect();
    InteractiveText::new(SharedString::from(format!("{id}-links")), styled)
        .on_click(ranges, move |ix, window, cx| {
            let Some(u) = urls.get(ix) else { return };
            match u.strip_prefix(FILE_SCHEME) {
                Some(target) => {
                    let (path, line) = split_path_line(target);
                    window.dispatch_action(Box::new(crate::actions::OpenPath { path, line }), cx);
                }
                None => cx.open_url(u),
            }
        })
        .into_any_element()
}

/// Plain text of the source with markup stripped, for copy actions.
#[cfg(test)]
pub fn plain(src: &str) -> String {
    let mut out = String::new();
    for ev in Parser::new(src) {
        match ev {
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak | Event::End(TagEnd::Paragraph) => out.push('\n'),
            _ => {}
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_blocks_use_zerons_metrics_at_the_default_size() {
        if (crate::design::code() - crate::design::CODE_DEFAULT).abs() < f32::EPSILON {
            assert!((code_block_size() - 12.5).abs() < f32::EPSILON);
            assert!((code_block_line() - 18.0).abs() < f32::EPSILON);
        }
    }

    /// A ~50 KB agent reply: prose, lists, a table, a long code block.
    pub(crate) fn sample_reply() -> String {
        let mut s = String::from("# Refactor plan\n\n");
        for i in 0..60 {
            s.push_str(&format!(
                "Step {i}: the **parser** keeps a `stable_end` offset and only re-reads the open *tail* block, \
                 so a long answer costs a few cheap rows. See [the docs](https://example.com/{i}) for more.\n\n"
            ));
            if i % 15 == 0 {
                for j in 0..8 {
                    s.push_str(&format!("- item {j} with `code` and **bold** text\n"));
                }
                s.push_str("\n| a | b | c |\n|---|---|---|\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |\n\n");
            }
        }
        s.push_str("```rust\n");
        for i in 0..700 {
            s.push_str(&format!("fn step_{i}(x: u32) -> u32 {{ x.wrapping_mul({i}) + 1 }}\n"));
        }
        s.push_str("```\n\nDone.\n");
        s
    }

    fn bench_one(label: &str, doc: &str, step: usize, naive: bool) {
        let mut s = Stream::default();
        let (mut total, mut worst) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        let (mut pushes, mut i) = (0, 0);
        while i < doc.len() {
            let mut j = (i + step).min(doc.len());
            while !doc.is_char_boundary(j) {
                j += 1;
            }
            let t = std::time::Instant::now();
            if naive {
                std::hint::black_box(parse(&doc[..j]));
            } else {
                std::hint::black_box(s.push(&doc[i..j]).live);
            }
            let d = t.elapsed();
            total += d;
            worst = worst.max(d);
            pushes += 1;
            i = j;
        }
        println!(
            "stream_bench {label:<28} {:>6} B, {step:>2}-byte deltas: {pushes:>5} pushes, total {:>8.1} ms, mean {:>7.1} us, worst {:>6.2} ms",
            doc.len(),
            total.as_secs_f64() * 1e3,
            total.as_secs_f64() * 1e6 / pushes as f64,
            worst.as_secs_f64() * 1e3
        );
    }

    /// Streaming cost of 50 KB replies (run with
    /// `cargo test -p dove stream_bench -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn stream_bench() {
        let mixed = sample_reply();
        let mut code = String::from("Here is the file:\n\n```rust\n");
        while code.len() < 50_000 {
            code.push_str(&format!("fn step_{}(x: u32) -> u32 {{ x.wrapping_mul(3) + 1 }}\n", code.len()));
        }
        code.push_str("```\n");
        for step in [4usize, 16, 64] {
            bench_one("mixed / incremental", &mixed, step, false);
            bench_one("50KB code block / incremental", &code, step, false);
        }
        bench_one("mixed / full re-parse", &mixed, 16, true);
        bench_one("50KB code block / full re-parse", &code, 16, true);
    }

    #[test]
    fn ingest_then_live_blocks_matches_push() {
        let mut pushed = Stream::default();
        let mut held = Stream::default();
        let up = pushed.push("Hello **world**");
        let finished = held.ingest("Hello **world**");
        assert_eq!(finished, up.finished);
        assert_eq!(held.live_blocks(), up.live);
    }

    #[test]
    fn paragraphs_headings_and_inline_styles() {
        let b = parse("# Title\n\nSome **bold** and `code` with [a link](https://x.dev).");
        assert!(matches!(&b[0], Block::Heading(1, i) if i.text == "Title"));
        let Block::Paragraph(p) = &b[1] else { panic!("paragraph") };
        assert_eq!(p.text, "Some bold and code with a link.");
        assert!(p.spans.iter().any(|(r, s)| s.bold && &p.text[r.clone()] == "bold"));
        assert!(p.spans.iter().any(|(r, s)| s.code && &p.text[r.clone()] == "code"));
        assert_eq!(p.links[0].1, "https://x.dev");
    }

    #[test]
    fn tight_and_loose_lists_and_code() {
        let b = parse("- one\n- two `x`\n\n1. a\n2. b\n\n```rust\nfn main() {}\n```");
        let Block::List { ordered: None, items } = &b[0] else { panic!("bullet list") };
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[1][0], Block::Paragraph(i) if i.text == "two x"));
        assert!(matches!(&b[1], Block::List { ordered: Some(1), items } if items.len() == 2));
        assert!(matches!(&b[2], Block::Code { lang: Some(l), text, .. } if l == "rust" && text == "fn main() {}"));
    }

    #[test]
    fn tables_and_quotes() {
        let b = parse("| a | b |\n|---|---|\n| 1 | 2 |\n\n> quoted");
        let Block::Table(rows) = &b[0] else { panic!("table") };
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][1].text, "2");
        assert!(matches!(&b[1], Block::Quote(q) if matches!(&q[0], Block::Paragraph(i) if i.text == "quoted")));
    }

    #[test]
    fn partial_streams_parse() {
        let b = parse("Here:\n\n```py\nprint(1");
        assert!(matches!(&b[1], Block::Code { text, .. } if text.contains("print(1")));
    }

    #[test]
    fn plain_strips_markup() {
        assert_eq!(plain("**hi** `x`"), "hi x");
    }

    /// Streaming any prefix split must end with the same blocks as a full
    /// parse (ignoring highlight data, which only final blocks carry).
    #[test]
    fn stream_matches_full_parse_for_every_split() {
        let doc = "# Plan\n\nFirst **bold** para with `code`.\n\n- a\n- b\n\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n\n| x | y |\n|---|---|\n| 1 | 2 |\n\n> quote\n\nLast line.";
        let strip = |b: &Block| match b {
            Block::Code { lang, text, .. } => Block::Code { lang: lang.clone(), text: text.clone(), highlight: None },
            other => other.clone(),
        };
        let full: Vec<Block> = parse(doc).iter().map(strip).collect();
        for step in [1usize, 3, 7, 16, 64] {
            let mut s = Stream::default();
            let mut done: Vec<Block> = Vec::new();
            let chars: Vec<char> = doc.chars().collect();
            for chunk in chars.chunks(step) {
                let delta: String = chunk.iter().collect();
                let up = s.push(&delta);
                done.extend(up.finished.iter().map(|b| strip(b)));
            }
            done.extend(s.finish().iter().map(|b| strip(b)));
            assert_eq!(done, full, "step {step}");
        }
    }

    /// The work per token stays bounded: the open tail never grows past the
    /// block being written, however long the answer gets.
    #[test]
    fn stream_parses_only_the_tail() {
        let mut s = Stream::default();
        let para = "Some words in a paragraph with `code` and **bold** text.\n\n";
        let mut max_tail = 0;
        for _ in 0..400 {
            for chunk in para.as_bytes().chunks(5) {
                s.push(std::str::from_utf8(chunk).unwrap());
                max_tail = max_tail.max(s.source.len() - s.stable_end);
            }
        }
        assert!(s.source.len() > 20_000);
        assert!(max_tail < 2 * para.len(), "tail grew to {max_tail}");
    }

    #[test]
    fn finished_code_is_highlighted() {
        let blocks = parse_final("```rust\nfn main() {}\n```");
        assert!(matches!(&*blocks[0], Block::Code { highlight: Some(h), .. } if !h.lines.is_empty()));
    }

    #[test]
    fn paths_in_inline_code_become_file_links() {
        assert!(looks_like_path("src/main.rs"));
        assert!(looks_like_path("main.rs:42"));
        assert!(looks_like_path("crates/dove-core/src/lib.rs:10:5"));
        assert!(!looks_like_path("foo()"));
        assert!(!looks_like_path("cargo build"));
        assert!(!looks_like_path("v1.2"));
        assert!(!looks_like_path("https://x.dev/a.js"));
        assert_eq!(split_path_line("a/b.rs:12:3"), ("a/b.rs".into(), Some(12)));
        assert_eq!(split_path_line("C:\\x\\y.rs:4"), ("C:\\x\\y.rs".into(), Some(4)));
        let b = parse("See `src/lib.rs:7` and `fn x()`.");
        let Block::Paragraph(p) = &b[0] else { panic!("paragraph") };
        assert_eq!(p.links.len(), 1);
        assert_eq!(p.links[0].1, "dove-file:src/lib.rs:7");
    }

    #[test]
    fn mend_closes_dangling_markers() {
        assert_eq!(mend_tail("a **b"), "a **b**");
        assert_eq!(mend_tail("run `ls"), "run `ls`");
    }
}
