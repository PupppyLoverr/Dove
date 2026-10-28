//! How the island looks (docs/NOTCH-V2.md §4–§5.4, calmed in §11): frosted
//! glass, white type, one accent per meaning, one filled control per screen.
//! Secondary actions show on hover, rare ones in a single `…` menu, and all
//! of them in the command bar. Every size and colour comes from
//! [`super::tokens`].

use std::f32::consts::TAU;
use std::ops::Range;
use std::time::{Duration, Instant};

use dove_core::proto::{HarnessKind, PermissionDecision, TaskId, TaskStatus};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, AnyView, App, AppContext as _, ClickEvent, ClipboardItem, Context, Div,
    ExternalPaths, Focusable as _, FontStyle, FontWeight, HighlightStyle, Hsla, InteractiveElement as _,
    InteractiveText, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit,
    ParentElement as _, Render, ScrollDelta, ScrollWheelEvent, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, StyledImage as _, StyledText, UnderlineStyle, Window, canvas, div, img, linear_color_stop,
    linear_gradient, point, px,
};
use gpui_base::input::{Input, Textarea};

use super::bridge::{Agent, Pending, Preview, TurnView};
use super::data::{ClipBody, ClipKind};
use super::tokens as tk;
use super::{
    FlashKind, Geo, HeroKind, Hit, Mark, MenuAction, Notch, Page, Popup, Shot, ShotTool, State, Tab, TrayFilter,
};
use crate::assets::{icon, icons};
use crate::island::ipc::ToEngine;

// ---------------------------------------------------------------------------
// Small parts
// ---------------------------------------------------------------------------

/// What the slim island can show, in the order it tries them.
#[derive(Clone, Debug, PartialEq)]
pub enum SlimItem {
    /// Agents with something going on: each kind, how many, whether one needs
    /// you; plus overall todo progress.
    Agents {
        groups: Vec<(HarnessKind, usize, bool)>,
        progress: Option<f32>,
    },
    /// A pull request whose checks are running or failed.
    Pr {
        checks: String,
    },
    Music,
    Timer,
    Note(String),
}

/// The tab a page belongs to (the agent view and the new-agent form live
/// under Agents; Notes, Timer and Dev servers are pages without a tab).
pub fn tab_of(page: &Page) -> Tab {
    match page {
        Page::Tab(t) => *t,
        Page::Agent(_) | Page::NewAgent => Tab::Agents,
    }
}

/// Where the music page draws the album art (canvas x, y, size), for the
/// art that flies there from the slim island.
pub fn music_art_rect() -> (f32, f32, f32) {
    let x = (tk::CANVAS_W - tk::EXPANDED_W) / 2.0 + tk::PAD + 12.0;
    (x, tk::TOP_GAP + tk::HEADER_H + 12.0, 88.0)
}

/// Where the timer page draws its ring.
pub fn timer_ring_rect() -> (f32, f32, f32) {
    let x = (tk::CANVAS_W - tk::EXPANDED_W) / 2.0 + tk::PAD + (tk::EXPANDED_W - 2.0 * tk::PAD - 120.0) / 2.0;
    (x, tk::TOP_GAP + tk::HEADER_H + 8.0, 120.0)
}

fn glyph(t: Tab) -> &'static str {
    match t {
        Tab::Home => icons::HOME,
        Tab::Agents => icons::BOT,
        Tab::Music => icons::MUSIC_NOTE,
        Tab::Tray => icons::CLIPBOARD_LIST,
        Tab::Notes => icons::NOTE_TEXT,
        Tab::Timer => icons::TIMER,
        Tab::Alerts => icons::BELL,
        Tab::Dev => icons::SERVER,
    }
}

fn mins_secs(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn elapsed(secs: i64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        _ => format!("{}h {}m", secs / 3600, (secs / 60) % 60),
    }
}

fn ago(ms: i64) -> String {
    let secs = ((dove_core::store::now_ms() - ms) / 1000).max(0);
    match secs {
        0..45 => "now".into(),
        45..3600 => format!("{}m ago", (secs / 60).max(1)),
        3600..86400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}

/// "in 2h 10m" until a moment in store milliseconds.
fn until(ms: i64) -> String {
    let secs = ((ms - dove_core::store::now_ms()) / 1000).max(0);
    match secs {
        0..60 => "in under a minute".into(),
        60..3600 => format!("in {}m", secs / 60),
        3600..86400 => format!("in {}h {}m", secs / 3600, (secs / 60) % 60),
        _ => format!("in {}d", secs / 86400),
    }
}

fn tint(kind: HarnessKind) -> Hsla {
    if kind == HarnessKind::ClaudeCode { tk::CLAUDE } else { tk::TEXT }
}

fn mark(kind: HarnessKind, size: f32) -> AnyElement {
    icon(crate::ui::harness_icon(kind)).size(px(size)).text_color(tint(kind)).into_any_element()
}

/// An agent's mark on a glass plate, with an optional ring around it.
fn avatar(kind: HarnessKind, size: f32, ring: Option<(f32, Hsla)>) -> AnyElement {
    let plate = div()
        .size(px(size))
        .flex_none()
        .rounded_full()
        .bg(tk::WASH)
        .flex()
        .items_center()
        .justify_center()
        .child(mark(kind, size * 0.52));
    match ring {
        Some((p, color)) => progress_ring(p, size + 6.0, 2.0, color, plate.into_any_element()),
        None => plate.into_any_element(),
    }
}

/// A circular progress ring (`p` 0–1; negative draws a short spinning arc
/// for "working, no steps"), with `inner` centered in it.
fn progress_ring(p: f32, size: f32, stroke: f32, color: Hsla, inner: AnyElement) -> AnyElement {
    let spin = p < 0.0;
    let phase = if spin { now_secs() % 1.4 / 1.4 } else { 0.0 };
    div()
        .relative()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let c = bounds.center();
                    let r = f32::from(bounds.size.width) / 2.0 - stroke;
                    let arc = |from: f32, sweep: f32| {
                        let steps = ((sweep.abs() / TAU) * 64.0).ceil().max(2.0) as usize;
                        let mut pb = gpui::PathBuilder::stroke(px(stroke));
                        for i in 0..=steps {
                            let a = from + sweep * i as f32 / steps as f32;
                            let pt = point(c.x + px(r * a.cos()), c.y + px(r * a.sin()));
                            if i == 0 {
                                pb.move_to(pt);
                            } else {
                                pb.line_to(pt);
                            }
                        }
                        pb.build().ok()
                    };
                    if let Some(track) = arc(0.0, TAU) {
                        window.paint_path(track, tk::WASH_HOVER);
                    }
                    let (from, sweep) = if spin {
                        (phase * TAU - TAU / 4.0, TAU * 0.28)
                    } else {
                        (-TAU / 4.0, TAU * p.clamp(0.0, 1.0))
                    };
                    if sweep > 0.01
                        && let Some(path) = arc(from, sweep)
                    {
                        window.paint_path(path, color);
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size(px(size)),
        )
        .child(inner)
        .into_any_element()
}

/// Seconds since the epoch modulo a day, for phase-based animation that
/// survives the window being recreated.
fn now_secs() -> f32 {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (d.as_millis() % 86_400_000) as f32 / 1000.0
}

fn breathe(t: f32) -> f32 {
    0.5 + 0.5 * (t * TAU / tk::BREATHE_S).sin().abs()
}

/// A tooltip: the control's name (and its key).
struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(8.0))
            .bg(tk::FLOAT)
            .border_1()
            .border_color(tk::RIM)
            .shadow_md()
            .text_size(px(tk::T11))
            .text_color(tk::TEXT)
            .child(self.0.clone())
    }
}

fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
}

/// An icon-only button, named by its tooltip.
fn ibtn(
    id: impl Into<gpui::ElementId>,
    path: &'static str,
    size: f32,
    color: Hsla,
    name: &'static str,
) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(size))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(tk::WASH_HOVER))
        .active(|s| s.bg(tk::WASH_ON))
        .tooltip(tip(name))
        .child(icon(path).size(px(if size >= 30.0 { tk::ICON_HEADER } else { tk::ICON_ROW })).text_color(color))
}

/// A secondary control: a quiet pill.
fn pill(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(tk::CONTROL_H))
        .px(px(12.0))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_size(px(tk::T12))
        .cursor_pointer()
        .text_color(if on { tk::TEXT } else { tk::DIM })
        .bg(if on { tk::WASH_ON } else { tk::WASH })
        .hover(|s| s.bg(tk::WASH_HOVER).text_color(tk::TEXT))
        .active(|s| s.bg(tk::WASH_ON))
        .child(label.into())
}

/// A text-only action ("Not now", "Open").
fn link(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .px(px(6.0))
        .h(px(tk::CONTROL_H))
        .flex()
        .items_center()
        .text_size(px(tk::T12))
        .text_color(tk::FAINT)
        .cursor_pointer()
        .hover(|s| s.text_color(tk::TEXT))
        .child(label.into())
}

/// The one primary control on a screen: solid white, dark text.
fn primary(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(tk::CONTROL_H))
        .px(px(14.0))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(tk::T12))
        .font_weight(FontWeight::SEMIBOLD)
        .cursor_pointer()
        .bg(tk::WHITE)
        .text_color(tk::INK)
        .hover(|s| s.bg(tk::WHITE_HOVER))
        .active(|s| s.opacity(0.85))
        .child(label.into())
}

fn section(label: impl Into<SharedString>) -> Div {
    div().text_size(px(tk::T11)).font_weight(FontWeight::MEDIUM).text_color(tk::FAINT).child(label.into())
}

/// A card: a lighter pane on the glass with a faint edge.
fn card() -> Div {
    div().p(px(12.0)).rounded(px(tk::R_CARD)).bg(tk::CARD).border_1().border_color(tk::CARD_EDGE)
}

fn bar(p: f32, color: Hsla, height: f32) -> Div {
    div()
        .h(px(height))
        .w_full()
        .rounded_full()
        .bg(tk::WASH_HOVER)
        .child(div().h_full().rounded_full().bg(color).w(gpui::relative(p.clamp(0.0, 1.0))))
}

fn status_of(a: &Agent) -> (&'static str, Hsla) {
    if a.needs_you() {
        ("Needs you", tk::AMBER)
    } else {
        match a.status {
            TaskStatus::Running => ("Working", tk::BLUE),
            TaskStatus::NeedsInput => ("Needs you", tk::AMBER),
            TaskStatus::Done => ("Done", tk::GREEN),
            TaskStatus::Failed => ("Failed", tk::RED),
            _ => ("Idle", tk::FAINT),
        }
    }
}

fn agent_progress(a: &Agent) -> Option<(f32, Hsla)> {
    let (_, color) = status_of(a);
    match (a.todos, a.status) {
        (Some((done, total)), _) if total > 0 => Some((done as f32 / total as f32, color)),
        (_, TaskStatus::Running) => Some((-1.0, color)),
        _ if a.needs_you() => Some((1.0, color)),
        _ => None,
    }
}

/// Digits that roll into place when they change (tabular figures).
fn roll(text: &str, key: &str, size: f32, color: Hsla, weight: FontWeight) -> Div {
    let mut row = div().flex().items_center().text_size(px(size)).text_color(color).font_weight(weight);
    for (i, ch) in text.chars().enumerate() {
        if ch.is_ascii_digit() && !crate::motion::reduced_now() {
            let id = SharedString::from(format!("roll-{key}-{i}-{ch}"));
            row = row.child(div().min_w(px(size * 0.6)).flex().justify_center().child(ch.to_string()).with_animation(
                id,
                Animation::new(Duration::from_millis(160)),
                move |el, t| {
                    let e = 1.0 - (1.0 - t).powi(3);
                    el.relative().top(px((1.0 - e) * -size * 0.35)).opacity(e)
                },
            ));
        } else {
            row = row.child(div().child(ch.to_string()));
        }
    }
    row
}

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

type Styles = Vec<(Range<usize>, HighlightStyle)>;
type Links = Vec<(Range<usize>, String)>;

enum Block {
    Para { text: String, styles: Styles, links: Links, heading: bool },
    Item { lead: String, depth: usize, text: String, styles: Styles, links: Links },
    Code { lang: String, text: String },
    Rule,
}

/// The text being collected for the next paragraph, heading or list item.
#[derive(Default)]
struct Run {
    text: String,
    styles: Styles,
    links: Links,
}

impl Run {
    fn flush(&mut self, out: &mut Vec<Block>, heading: bool, item: &Option<(String, usize)>) {
        let Run { text, styles, links } = std::mem::take(self);
        if text.trim().is_empty() {
            return;
        }
        match item {
            Some((lead, depth)) => out.push(Block::Item { lead: lead.clone(), depth: *depth, text, styles, links }),
            None => out.push(Block::Para { text, styles, links, heading }),
        }
    }
}

/// Parses a reply into blocks: paragraphs and headings with inline bold,
/// italic, code and links as styled runs of one text (so a sentence wraps
/// as a sentence), list items, fenced code and rules.
fn blocks(src: &str) -> Vec<Block> {
    use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
    let mut out = Vec::new();
    let mut run = Run::default();
    let (mut bold, mut italic): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
    let mut link: Option<(usize, String)> = None;
    let mut heading = false;
    let mut item: Option<(String, usize)> = None;
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut code: Option<(String, String)> = None;
    for ev in Parser::new(src) {
        if let Some((_, body)) = &mut code {
            match ev {
                Event::Text(t) => body.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, body) = code.take().unwrap_or_default();
                    out.push(Block::Code { lang, text: body.trim_end().to_string() });
                }
                _ => {}
            }
            continue;
        }
        match ev {
            Event::Start(Tag::Heading { .. }) => {
                run.flush(&mut out, heading, &item);
                heading = true;
            }
            Event::End(TagEnd::Heading(_)) => {
                run.flush(&mut out, true, &None);
                heading = false;
            }
            Event::Start(Tag::Paragraph) => {
                if item.is_none() {
                    run.flush(&mut out, heading, &item);
                } else if !run.text.is_empty() {
                    run.text.push(' ');
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if item.is_none() {
                    run.flush(&mut out, false, &None);
                }
            }
            Event::Start(Tag::List(start)) => {
                run.flush(&mut out, heading, &item);
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                run.flush(&mut out, heading, &item);
                let depth = lists.len().saturating_sub(1);
                let lead = match lists.last_mut() {
                    Some(Some(n)) => {
                        let l = format!("{n}.");
                        *n += 1;
                        l
                    }
                    _ => "•".to_string(),
                };
                item = Some((lead, depth));
            }
            Event::End(TagEnd::Item) => {
                run.flush(&mut out, false, &item);
                item = None;
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                run.flush(&mut out, heading, &item);
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.split(',').next().unwrap_or("").trim().to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                code = Some((lang, String::new()));
            }
            Event::Start(Tag::Strong) => bold.push(run.text.len()),
            Event::End(TagEnd::Strong) => {
                if let Some(s) = bold.pop() {
                    run.styles.push((
                        s..run.text.len(),
                        HighlightStyle {
                            font_weight: Some(FontWeight::SEMIBOLD),
                            color: Some(tk::TEXT),
                            ..Default::default()
                        },
                    ));
                }
            }
            Event::Start(Tag::Emphasis) => italic.push(run.text.len()),
            Event::End(TagEnd::Emphasis) => {
                if let Some(s) = italic.pop() {
                    run.styles.push((
                        s..run.text.len(),
                        HighlightStyle { font_style: Some(FontStyle::Italic), ..Default::default() },
                    ));
                }
            }
            Event::Start(Tag::Link { dest_url, .. }) => link = Some((run.text.len(), dest_url.to_string())),
            Event::End(TagEnd::Link) => {
                if let Some((s, url)) = link.take() {
                    let r = s..run.text.len();
                    run.styles.push((
                        r.clone(),
                        HighlightStyle {
                            color: Some(tk::BLUE),
                            underline: Some(UnderlineStyle {
                                thickness: px(1.0),
                                color: Some(tk::BLUE.opacity(0.5)),
                                wavy: false,
                            }),
                            ..Default::default()
                        },
                    ));
                    run.links.push((r, url));
                }
            }
            Event::Code(c) => {
                let s = run.text.len();
                run.text.push_str(&c);
                run.styles.push((
                    s..run.text.len(),
                    HighlightStyle {
                        color: Some(tk::TEXT),
                        background_color: Some(tk::WASH_HOVER),
                        ..Default::default()
                    },
                ));
            }
            Event::Text(t) => run.text.push_str(&t),
            Event::SoftBreak => run.text.push(' '),
            Event::HardBreak => run.text.push('\n'),
            Event::Rule => {
                run.flush(&mut out, heading, &item);
                out.push(Block::Rule);
            }
            _ => {}
        }
    }
    if let Some((lang, body)) = code {
        out.push(Block::Code { lang, text: body.trim_end().to_string() });
    }
    run.flush(&mut out, heading, &item);
    out
}

/// One paragraph's text with its styles, and links that open on click.
fn rich(id: SharedString, text: String, mut styles: Styles, links: Links) -> AnyElement {
    styles.sort_by_key(|(r, _)| r.start);
    // Overlapping runs (bold inside a link) keep the outer one.
    let mut clean: Styles = Vec::new();
    for (r, s) in styles {
        if r.start < r.end && clean.last().is_none_or(|(p, _)| r.start >= p.end) {
            clean.push((r, s));
        }
    }
    let styled = StyledText::new(text).with_highlights(clean);
    if links.is_empty() {
        return styled.into_any_element();
    }
    let (ranges, urls): (Vec<_>, Vec<_>) = links.into_iter().unzip();
    InteractiveText::new(id, styled)
        .on_click(ranges, move |ix, _, cx| {
            if let Some(url) = urls.get(ix) {
                cx.open_url(url);
            }
        })
        .into_any_element()
}

/// A reply, rendered.
fn markdown(src: &str, key: &str) -> AnyElement {
    let mut col = div().flex().flex_col().gap(px(6.0)).text_size(px(tk::T12)).text_color(tk::DIM);
    for (i, b) in blocks(src).into_iter().enumerate() {
        let id = SharedString::from(format!("md-{key}-{i}"));
        col = col.child(match b {
            Block::Para { text, styles, links, heading } => div()
                .when(heading, |d| {
                    d.text_color(tk::TEXT).font_weight(FontWeight::SEMIBOLD).text_size(px(tk::T13)).pt(px(2.0))
                })
                .child(rich(id, text, styles, links))
                .into_any_element(),
            Block::Item { lead, depth, text, styles, links } => div()
                .flex()
                .gap(px(6.0))
                .pl(px(depth as f32 * 12.0))
                .child(div().flex_none().min_w(px(12.0)).text_color(tk::FAINT).child(lead))
                .child(div().flex_1().min_w_0().child(rich(id, text, styles, links)))
                .into_any_element(),
            Block::Code { lang, text } => {
                let copy = text.clone();
                let group = SharedString::from(format!("code-{key}-{i}"));
                div()
                    .id(id)
                    .group(group.clone())
                    .relative()
                    .rounded(px(10.0))
                    .bg(gpui::hsla(0.0, 0.0, 0.0, 0.24))
                    .px(px(10.0))
                    .py(px(8.0))
                    .font_family(tk::MONO)
                    .text_size(px(tk::T11))
                    .text_color(tk::TEXT)
                    .child(text)
                    .child(
                        div()
                            .absolute()
                            .top(px(4.0))
                            .right(px(4.0))
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .invisible()
                            .group_hover(group, |s| s.visible())
                            .when(!lang.is_empty(), |d| {
                                d.child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(lang))
                            })
                            .child(
                                div()
                                    .id("copy-code")
                                    .size(px(22.0))
                                    .rounded(px(6.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(tk::WASH_HOVER))
                                    .tooltip(tip("Copy"))
                                    .child(icon(icons::COPY).size(px(13.0)).text_color(tk::DIM))
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                                    }),
                            ),
                    )
                    .into_any_element()
            }
            Block::Rule => div().h(px(1.0)).bg(tk::HAIRLINE).into_any_element(),
        });
    }
    col.into_any_element()
}

/// The preview of what an approval would do.
fn preview(p: &Preview, lines: usize) -> AnyElement {
    match p {
        Preview::Diff { file, added, removed, lines: diff } => {
            let name = file.rsplit(['/', '\\']).next().unwrap_or(file).to_string();
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(tk::T11))
                        .child(icon(icons::FILE_CODE).size(px(13.0)).text_color(tk::DIM))
                        .child(
                            div().flex_1().min_w_0().truncate().font_family(tk::MONO).text_color(tk::TEXT).child(name),
                        )
                        .child(div().text_color(tk::GREEN).child(format!("+{added}")))
                        .child(div().text_color(tk::RED).child(format!("−{removed}"))),
                )
                .when(lines > 0 && !diff.is_empty(), |d| {
                    d.child(
                        div()
                            .rounded(px(10.0))
                            .bg(gpui::hsla(0.0, 0.0, 0.0, 0.22))
                            .py(px(4.0))
                            .flex()
                            .flex_col()
                            .children(diff.iter().take(lines).map(|(tag, text)| {
                                let (bg, fg) = match tag {
                                    '+' => (tk::GREEN.opacity(0.12), tk::GREEN),
                                    '-' => (tk::RED.opacity(0.12), tk::RED),
                                    _ => (gpui::transparent_black(), tk::DIM),
                                };
                                div()
                                    .flex()
                                    .bg(bg)
                                    .font_family(tk::MONO)
                                    .text_size(px(tk::T11))
                                    .child(
                                        div().w(px(16.0)).flex_none().text_color(fg).pl(px(6.0)).child(tag.to_string()),
                                    )
                                    .child(div().flex_1().min_w_0().truncate().text_color(tk::TEXT).child(text.clone()))
                            })),
                    )
                })
                .into_any_element()
        }
        Preview::Command(c) => div()
            .px(px(8.0))
            .py(px(5.0))
            .rounded(px(10.0))
            .bg(gpui::hsla(0.0, 0.0, 0.0, 0.22))
            .font_family(tk::MONO)
            .text_size(px(tk::T11))
            .text_color(tk::TEXT)
            .line_clamp(2)
            .child(format!("$ {c}"))
            .into_any_element(),
        Preview::File(f) => div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(px(tk::T11))
            .child(icon(icons::DOCUMENT).size(px(13.0)).text_color(tk::DIM))
            .child(div().truncate().font_family(tk::MONO).text_color(tk::TEXT).child(f.clone()))
            .into_any_element(),
    }
}

/// A row or card rising into place as a page opens: the first four, one
/// after another; the rest come with the fourth.
fn rise(el: impl IntoElement + 'static, key: (u64, usize), index: usize) -> AnyElement {
    if crate::motion::reduced_now() {
        return el.into_any_element();
    }
    let delay = (index.min(tk::RISE_ROWS - 1) as u64) * tk::STAGGER_MS;
    let dur = Duration::from_millis(tk::RISE_MS + delay);
    let id = SharedString::from(format!("rise-{}-{}", key.0, key.1));
    let lead = delay as f32 / dur.as_millis() as f32;
    div()
        .child(el)
        .with_animation(
            id,
            Animation::new(dur).with_easing(move |t| ((t - lead) / (1.0 - lead)).clamp(0.0, 1.0)),
            |el, t| {
                let e = 1.0 - (1.0 - t).powi(3);
                el.opacity(e).relative().top(px((1.0 - e) * tk::RISE_PX))
            },
        )
        .into_any_element()
}

/// A message bubble (yours), with a note under it while it's on its way.
fn bubble(text: &str, note: Option<&'static str>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_end()
        .gap(px(2.0))
        .child(
            div()
                .max_w(gpui::relative(0.82))
                .px(px(12.0))
                .py(px(7.0))
                .rounded(px(16.0))
                .bg(tk::WASH_ON)
                .text_size(px(tk::T12))
                .text_color(tk::TEXT)
                .when(note.is_some(), |d| d.opacity(0.7))
                .child(text.trim().to_string()),
        )
        .when_some(note, |d, n| d.child(div().text_size(px(tk::T11)).text_color(tk::GHOST).child(n)))
        .into_any_element()
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

/// A small menu drawn inside the island, next to what opened it.
#[derive(Clone, Debug)]
pub struct Menu {
    /// Top-left, relative to the island.
    pub x: f32,
    pub y: f32,
    pub items: Vec<MenuItem>,
    pub hovered: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct MenuItem {
    pub label: String,
    pub icon: Option<&'static str>,
    /// Shows a check (on) or a blank (off) before the label.
    pub checked: Option<bool>,
    pub danger: bool,
    /// None: a section label.
    pub action: Option<MenuAction>,
}

impl MenuItem {
    fn act(label: impl Into<String>, icon: &'static str, action: MenuAction) -> Self {
        MenuItem { label: label.into(), icon: Some(icon), checked: None, danger: false, action: Some(action) }
    }

    fn check(label: impl Into<String>, on: bool, action: MenuAction) -> Self {
        MenuItem { label: label.into(), icon: None, checked: Some(on), danger: false, action: Some(action) }
    }

    fn danger(label: impl Into<String>, icon: &'static str, action: MenuAction) -> Self {
        MenuItem { label: label.into(), icon: Some(icon), checked: None, danger: true, action: Some(action) }
    }

    fn head(label: impl Into<String>) -> Self {
        MenuItem { label: label.into(), icon: None, checked: None, danger: false, action: None }
    }
}

const MENU_W: f32 = 216.0;
const MENU_ROW: f32 = 30.0;
const MENU_HEAD: f32 = 24.0;

impl Menu {
    fn height(&self) -> f32 {
        12.0 + self.items.iter().map(|i| if i.action.is_some() { MENU_ROW } else { MENU_HEAD }).sum::<f32>()
    }

    /// The island-relative bottom, so the island can grow to fit it.
    pub fn bottom(&self) -> f32 {
        self.y + self.height()
    }
}

/// A card on Home.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HomeCard {
    Offline,
    Approval,
    Working,
    Music,
    Timer,
    Dev,
    Prs,
    Automations,
    Note,
    Clips,
}

impl HomeCard {
    fn label(self) -> &'static str {
        match self {
            HomeCard::Offline => "Dove",
            HomeCard::Approval => "Needs you",
            HomeCard::Working => "Agents",
            HomeCard::Music => "Music",
            HomeCard::Timer => "Timer",
            HomeCard::Dev => "Dev servers",
            HomeCard::Prs => "Pull requests",
            HomeCard::Automations => "Automations",
            HomeCard::Note => "Notes",
            HomeCard::Clips => "Clipboard",
        }
    }

    /// Where its "Also" pill goes.
    fn page(self, n: &Notch) -> Page {
        match self {
            HomeCard::Music => Page::Tab(Tab::Music),
            HomeCard::Timer => Page::Tab(Tab::Timer),
            HomeCard::Dev => Page::Tab(Tab::Dev),
            HomeCard::Note => Page::Tab(Tab::Notes),
            HomeCard::Clips => Page::Tab(Tab::Tray),
            HomeCard::Approval => n.approval().map(|(a, _)| Page::Agent(a.id)).unwrap_or(Page::Tab(Tab::Agents)),
            _ => Page::Tab(Tab::Agents),
        }
    }

    /// Its height, for sizing the island before it draws.
    fn height(self, n: &Notch) -> f32 {
        match self {
            HomeCard::Offline => 62.0,
            HomeCard::Approval => {
                let preview = n.approval().and_then(|(_, p)| p.preview.as_ref()).map(|p| match p {
                    Preview::Diff { lines, .. } => 22.0 + 14.0 * lines.len().min(3) as f32,
                    Preview::Command(_) => 32.0,
                    Preview::File(_) => 20.0,
                });
                24.0 + 8.0 + 36.0 + 8.0 + tk::CONTROL_H + preview.map(|p| p + 8.0).unwrap_or(0.0)
            }
            HomeCard::Working => {
                n.snap.agents.iter().filter(|a| a.active() && !a.needs_you()).take(3).count() as f32 * (tk::ROW_H + 2.0)
            }
            HomeCard::Music => 68.0,
            HomeCard::Timer => 64.0,
            HomeCard::Dev => 24.0 + n.ports.len().min(3) as f32 * 22.0,
            HomeCard::Prs => 24.0 + n.snap.prs.iter().filter(|p| p.state == "open").take(3).count() as f32 * 40.0,
            HomeCard::Automations => {
                24.0 + n.snap.automations.iter().filter(|a| a.enabled).take(2).count() as f32 * 38.0
            }
            HomeCard::Note => 76.0,
            HomeCard::Clips => 24.0 + n.clips.len().min(3) as f32 * 26.0,
        }
    }
}

/// A tray tile: a shelved file or a clipboard entry (indices into the
/// notch's lists).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrayItem {
    File(usize),
    Clip(usize),
}

fn human_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.0} KB", bytes as f64 / 1024.0),
        1_048_576..1_073_741_824 => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
        _ => format!("{:.1} GB", bytes as f64 / 1_073_741_824.0),
    }
}

mod canvas;
mod menus;
mod pages;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_well() {
        assert_eq!(mins_secs(Duration::from_secs(65)), "1:05");
        assert_eq!(mins_secs(Duration::from_secs(3725)), "1:02:05");
        assert_eq!(elapsed(42), "42s");
        assert_eq!(elapsed(125), "2m");
        assert_eq!(elapsed(3900), "1h 5m");
        assert_eq!(human_size(2048), "2 KB");
    }

    #[test]
    fn pages_map_to_their_tab() {
        assert_eq!(tab_of(&Page::NewAgent), Tab::Agents);
        assert_eq!(tab_of(&Page::Tab(Tab::Music)), Tab::Music);
        assert!(!tab_of(&Page::Tab(Tab::Notes)).in_bar());
    }

    #[test]
    fn replies_parse_into_blocks_that_keep_sentences_whole() {
        let b = blocks(
            "# Done\n\nI fixed **the bug** in `auth.rs` and [opened a PR](https://x.dev/1).\n\n- one\n- two\n\n```rust\nfn main() {}\n```",
        );
        assert!(matches!(&b[0], Block::Para { heading: true, text, .. } if text == "Done"));
        match &b[1] {
            Block::Para { text, styles, links, heading: false } => {
                assert_eq!(text, "I fixed the bug in auth.rs and opened a PR.");
                assert_eq!(styles.len(), 3, "bold, code and the link are runs of one text");
                assert_eq!(links[0].1, "https://x.dev/1");
                assert_eq!(&text[links[0].0.clone()], "opened a PR");
            }
            _ => panic!("a paragraph"),
        }
        assert!(matches!(&b[2], Block::Item { lead, text, .. } if lead == "•" && text == "one"));
        assert!(matches!(&b[4], Block::Code { lang, text } if lang == "rust" && text == "fn main() {}"));
    }

    #[test]
    fn numbered_lists_count() {
        let b = blocks("1. first\n2. second");
        assert!(matches!(&b[0], Block::Item { lead, .. } if lead == "1."));
        assert!(matches!(&b[1], Block::Item { lead, .. } if lead == "2."));
    }

    #[test]
    fn hero_targets_sit_inside_the_expanded_island() {
        let (x, y, s) = music_art_rect();
        let left = (tk::CANVAS_W - tk::EXPANDED_W) / 2.0;
        assert!(x > left && x + s < left + tk::EXPANDED_W && y > tk::TOP_GAP);
        let (x, _, s) = timer_ring_rect();
        assert!((x + s / 2.0 - tk::CANVAS_W / 2.0).abs() < 1.0, "the ring is centered");
    }
}
