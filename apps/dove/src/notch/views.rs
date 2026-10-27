//! How the island looks (docs/NOTCH-V2.md §4–§5.4): frosted glass, white
//! type, one accent per meaning, springy and calm. Every size and colour
//! comes from [`super::tokens`].

use std::f32::consts::TAU;
use std::time::{Duration, Instant};

use dove_core::proto::{HarnessKind, PermissionDecision, TaskId, TaskStatus};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, ClickEvent, ClipboardItem, Context, Div, ExternalPaths, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ObjectFit, ParentElement as _, ScrollDelta, ScrollWheelEvent, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Window, canvas, div, img, linear_color_stop,
    linear_gradient, point, px,
};
use gpui::Focusable as _;
use gpui_base::input::{Input, Textarea};

use super::bridge::{Agent, Pending, Preview};
use super::data::{ClipBody, ClipKind};
use super::tokens as tk;
use super::{FlashKind, Geo, Hit, Notch, Page, State, Tab, TrayFilter};
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
    Agents { groups: Vec<(HarnessKind, usize, bool)>, progress: Option<f32> },
    Music,
    Timer,
    Note(String),
}

/// The tab a page belongs to (the agent view and the new-agent form live
/// under Agents).
pub fn tab_of(page: &Page) -> Tab {
    match page {
        Page::Tab(t) => *t,
        Page::Agent(_) | Page::NewAgent => Tab::Agents,
    }
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
        Tab::Dev => icons::TERMINAL,
    }
}

fn mins_secs(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
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
    let phase = if spin { (Instant::now().elapsed().as_secs_f32() + now_secs()) % 1.4 / 1.4 } else { 0.0 };
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
                    let (from, sweep) =
                        if spin { (phase * TAU - TAU / 4.0, TAU * 0.28) } else { (-TAU / 4.0, TAU * p.clamp(0.0, 1.0)) };
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

fn ibtn(id: impl Into<gpui::ElementId>, path: &'static str, size: f32, color: Hsla) -> Stateful<Div> {
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
        .child(icon(path).size(px((size * 0.5).round())).text_color(color))
}

fn pill(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.0))
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
        .hover(|s| s.bg(tk::WASH_HOVER))
        .active(|s| s.bg(tk::WASH_ON))
        .child(label.into())
}

/// The primary button: solid white, dark text.
fn primary(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.0))
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
        .child(label.into())
}

fn section(label: impl Into<SharedString>) -> Div {
    div().text_size(px(tk::T11)).font_weight(FontWeight::MEDIUM).text_color(tk::FAINT).child(label.into())
}

fn card() -> Div {
    div().p(px(12.0)).rounded(px(tk::R_CARD)).bg(tk::CARD).border_1().border_color(tk::HAIRLINE)
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

/// Light Markdown for replies: `**bold**`, `` `code` `` and bullet lines.
fn rich_text(text: &str) -> AnyElement {
    let mut col = div().flex().flex_col().gap(px(4.0));
    for line in text.lines().take(60) {
        let trimmed = line.trim_start();
        let (bullet, body) = match trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            Some(rest) => (true, rest),
            None => (false, trimmed.trim_start_matches('#').trim_start()),
        };
        if body.is_empty() {
            col = col.child(div().h(px(4.0)));
            continue;
        }
        let mut row = div().flex().flex_wrap().items_baseline();
        if bullet {
            row = row.child(div().w(px(12.0)).text_color(tk::FAINT).child("•"));
        }
        let heading = trimmed.starts_with('#');
        for (i, chunk) in body.split('`').enumerate() {
            if chunk.is_empty() {
                continue;
            }
            if i % 2 == 1 {
                row = row.child(
                    div()
                        .px(px(4.0))
                        .rounded(px(5.0))
                        .bg(tk::WASH)
                        .font_family(tk::MONO)
                        .text_size(px(tk::T11))
                        .text_color(tk::TEXT)
                        .child(chunk.to_string()),
                );
                continue;
            }
            for (j, part) in chunk.split("**").enumerate() {
                if part.is_empty() {
                    continue;
                }
                let strong = j % 2 == 1 || heading;
                row = row.child(
                    div()
                        .text_color(if strong { tk::TEXT } else { tk::DIM })
                        .when(strong, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .child(part.to_string()),
                );
            }
        }
        col = col.child(row);
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
                        .child(icon(icons::FILE_CODE).size(px(12.0)).text_color(tk::DIM))
                        .child(div().flex_1().min_w_0().truncate().font_family(tk::MONO).text_color(tk::TEXT).child(name))
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
                                    .child(div().w(px(16.0)).flex_none().text_color(fg).pl(px(6.0)).child(tag.to_string()))
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
            .child(icon(icons::DOCUMENT).size(px(12.0)).text_color(tk::DIM))
            .child(div().truncate().font_family(tk::MONO).text_color(tk::TEXT).child(f.clone()))
            .into_any_element(),
    }
}

/// A row or card rising into place, one after another when a page opens.
fn rise(el: impl IntoElement + 'static, key: (u64, usize), index: usize) -> AnyElement {
    if crate::motion::reduced_now() {
        return el.into_any_element();
    }
    let delay = (index.min(12) as u64) * tk::STAGGER_MS;
    let dur = Duration::from_millis(220 + delay);
    let id = SharedString::from(format!("rise-{}-{}", key.0, key.1));
    let lead = delay as f32 / dur.as_millis() as f32;
    div()
        .child(el)
        .with_animation(id, Animation::new(dur).with_easing(move |t| ((t - lead) / (1.0 - lead)).clamp(0.0, 1.0)), |el, t| {
            let e = 1.0 - (1.0 - t).powi(3);
            el.opacity(e).relative().top(px((1.0 - e) * tk::RISE_PX))
        })
        .into_any_element()
}

impl Notch {
    fn t(&self) -> f32 {
        self.epoch.elapsed().as_secs_f32()
    }

    fn key(&self) -> u64 {
        self.page_at.duration_since(self.epoch).as_millis() as u64
    }

    fn dot(&self, color: Hsla, size: f32, breathing: bool) -> Div {
        let a = if breathing { breathe(self.t()) } else { 1.0 };
        div().size(px(size)).flex_none().rounded_full().bg(color.opacity(a))
    }

    /// Four bars that follow the real output level (when Windows reports
    /// one), else a gentle rhythm while playing.
    fn eq(&self, height: f32) -> Div {
        let playing = self.media.as_ref().is_some_and(|m| m.playing);
        let peak = self.audio_state.peak;
        let t = self.t();
        let mut row = div().h(px(height)).flex().items_end().gap(px(2.0)).flex_none();
        for (speed, phase) in [(7.1f32, 0.0f32), (9.3, 1.3), (6.2, 2.1), (8.4, 0.7)] {
            let wobble = 0.5 + 0.5 * (t * speed + phase).sin();
            let k = if !playing {
                0.0
            } else if peak > 0.001 {
                (peak.sqrt() * (0.55 + 0.45 * wobble)).clamp(0.0, 1.0)
            } else {
                0.25 + 0.6 * wobble
            };
            let h = (3.0 + (height - 3.0) * k).max(3.0);
            row = row.child(div().w(px(2.5)).h(px(h)).rounded_full().bg(tk::GREEN));
        }
        row
    }

    fn art(&self, size: f32, radius: f32) -> AnyElement {
        match self.media.as_ref().and_then(|m| m.art.clone()) {
            Some(a) => img(a).size(px(size)).flex_none().rounded(px(radius)).object_fit(ObjectFit::Cover).into_any_element(),
            None => div()
                .size(px(size))
                .flex_none()
                .rounded(px(radius))
                .bg(tk::WASH_HOVER)
                .flex()
                .items_center()
                .justify_center()
                .child(icon(icons::MUSIC_NOTE).size(px((size * 0.5).round())).text_color(tk::DIM))
                .into_any_element(),
        }
    }

    // ----- slim -----------------------------------------------------------------

    pub(super) fn slim_items(&self) -> Vec<SlimItem> {
        let mut items = Vec::new();
        if self.prefs.slim_agents {
            let mut groups: Vec<(HarnessKind, usize, bool)> = Vec::new();
            let mut add = |kind: HarnessKind, needs: bool| match groups.iter_mut().find(|g| g.0 == kind) {
                Some(g) => {
                    g.1 += 1;
                    g.2 |= needs;
                }
                None => groups.push((kind, 1, needs)),
            };
            let (mut done, mut total) = (0u32, 0u32);
            for a in self.snap.agents.iter().filter(|a| a.active()) {
                add(a.kind, a.needs_you());
                if let Some((d, t)) = a.todos {
                    done += d;
                    total += t;
                }
            }
            for f in self.snap.flock.iter().filter(|f| f.working || f.needs_input) {
                add(f.kind, f.needs_input);
            }
            if !groups.is_empty() {
                let progress = (total > 0).then(|| done as f32 / total as f32);
                items.push(SlimItem::Agents { groups, progress });
            }
        }
        if self.prefs.slim_music && self.media.as_ref().is_some_and(|m| m.playing) {
            items.push(SlimItem::Music);
        }
        if self.prefs.slim_timer && self.timer.active() {
            items.push(SlimItem::Timer);
        }
        if self.prefs.slim_note
            && let Some(n) = self.notes.iter().find(|n| n.pinned)
        {
            items.push(SlimItem::Note(n.title()));
        }
        // Needs you always leads; otherwise scrolling picks the primary.
        let needs = matches!(items.first(), Some(SlimItem::Agents { groups, .. }) if groups.iter().any(|g| g.2));
        if !needs && !items.is_empty() {
            let k = self.primary % items.len();
            items.rotate_left(k);
        }
        items
    }

    /// The slim width (one of three) and how many bubbles.
    pub(super) fn slim_size(&self) -> (f32, usize) {
        let items = self.slim_items();
        let width = match items.first() {
            None => tk::SLIM_SINGLE,
            Some(SlimItem::Agents { groups, .. }) => {
                let marks = groups.len().min(3);
                if marks >= 3 || (marks == 2 && groups.iter().any(|g| g.2)) { tk::SLIM_WIDE } else { tk::SLIM_DUAL }
            }
            Some(SlimItem::Music) => tk::SLIM_DUAL,
            Some(SlimItem::Timer) => tk::SLIM_SINGLE,
            Some(SlimItem::Note(_)) => tk::SLIM_DUAL,
        };
        let width = if self.context_menu { tk::SLIM_WIDE } else { width };
        (width, items.len().saturating_sub(1).min(2))
    }

    fn slim_leading(&self, item: &SlimItem) -> AnyElement {
        match item {
            SlimItem::Agents { groups, progress } => {
                let needs = groups.iter().any(|g| g.2);
                let color = if needs { tk::AMBER } else { tk::BLUE };
                let extra: usize = groups.iter().skip(3).map(|g| g.1).sum::<usize>()
                    + groups.iter().take(3).map(|g| g.1.saturating_sub(1)).sum::<usize>();
                let mut stack = div().flex().items_center();
                for (i, (kind, _, n)) in groups.iter().take(3).enumerate() {
                    let face = div()
                        .relative()
                        .when(i > 0, |d| d.ml(px(-6.0)))
                        .child(
                            div()
                                .size(px(22.0))
                                .rounded_full()
                                .bg(gpui::hsla(240.0 / 360.0, 0.06, 0.16, 1.0))
                                .border_1()
                                .border_color(tk::HAIRLINE)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(mark(*kind, 12.0)),
                        )
                        .when(*n, |d| d.child(self.dot(tk::AMBER, 6.0, true).absolute().top(px(-1.0)).right(px(-1.0))));
                    stack = stack.child(face);
                }
                let lead = match progress {
                    Some(p) => progress_ring(*p, 30.0, 2.0, color, stack.into_any_element()),
                    None => stack.into_any_element(),
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(lead)
                    .when(extra > 0, |d| {
                        d.child(
                            div()
                                .px(px(6.0))
                                .h(px(18.0))
                                .rounded_full()
                                .bg(tk::WASH_HOVER)
                                .flex()
                                .items_center()
                                .text_size(px(tk::T11))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(tk::DIM)
                                .child(format!("+{extra}")),
                        )
                    })
                    .into_any_element()
            }
            SlimItem::Music => self.art(22.0, 7.0),
            SlimItem::Timer => progress_ring(
                self.timer.progress(),
                24.0,
                2.0,
                tk::AMBER,
                icon(icons::TIMER).size(px(11.0)).text_color(tk::AMBER).into_any_element(),
            ),
            SlimItem::Note(_) => icon(icons::NOTE_TEXT).size(px(15.0)).text_color(tk::TEXT).into_any_element(),
        }
    }

    fn slim_trailing(&self, item: &SlimItem) -> AnyElement {
        if let Some((text, _)) = &self.moment {
            return div()
                .flex()
                .items_center()
                .gap(px(5.0))
                .text_size(px(tk::T12))
                .font_weight(FontWeight::MEDIUM)
                .text_color(tk::GREEN)
                .child(icon(icons::CHECK).size(px(13.0)).text_color(tk::GREEN))
                .child(text.clone())
                .into_any_element();
        }
        let phrase = |text: String, color: Hsla| {
            div().text_size(px(tk::T12)).font_weight(FontWeight::MEDIUM).text_color(color).truncate().child(text).into_any_element()
        };
        match item {
            SlimItem::Agents { groups, .. } => {
                let n: usize = groups.iter().map(|g| g.1).sum();
                if groups.iter().any(|g| g.2) {
                    phrase("Needs you".into(), tk::AMBER)
                } else {
                    phrase(if n == 1 { "Working".into() } else { format!("{n} working") }, tk::BLUE)
                }
            }
            SlimItem::Music => self.eq(14.0).into_any_element(),
            SlimItem::Timer => phrase(mins_secs(self.timer.remaining()), if self.timer.running() { tk::TEXT } else { tk::DIM }),
            SlimItem::Note(title) => phrase(title.clone(), tk::DIM),
        }
    }

    fn render_slim(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.context_menu {
            return div()
                .size_full()
                .px(px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .gap(px(6.0))
                .child(pill("slim-hide", "Hide for 1 hour", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.hide_for_an_hour(cx))))
                .child(pill("slim-settings", "Settings", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.context_menu = false;
                    this.command(ToEngine::OpenSettings);
                    cx.notify();
                })))
                .into_any_element();
        }
        let items = self.slim_items();
        let (left, right) = match items.first() {
            None => (
                icon(icons::DOVE_LOGO).size(px(15.0)).text_color(tk::TEXT.opacity(0.6)).into_any_element(),
                div()
                    .text_size(px(tk::T12))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(tk::DIM.opacity(0.6))
                    .child(super::clock_text())
                    .into_any_element(),
            ),
            Some(first) => (self.slim_leading(first), self.slim_trailing(first)),
        };
        div()
            .size_full()
            .pl(px(8.0))
            .pr(px(14.0))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(10.0))
            .child(div().flex_none().child(left))
            .child(div().min_w_0().flex().justify_end().child(right))
            .into_any_element()
    }

    fn render_bubble(&self, index: usize) -> AnyElement {
        let items = self.slim_items();
        let Some(item) = items.get(index + 1) else { return div().into_any_element() };
        let inner = match item {
            SlimItem::Music => self.art(22.0, 11.0),
            SlimItem::Timer => progress_ring(
                self.timer.progress(),
                24.0,
                2.0,
                tk::AMBER,
                icon(icons::TIMER).size(px(11.0)).text_color(tk::AMBER).into_any_element(),
            ),
            SlimItem::Agents { groups, progress } => {
                let needs = groups.iter().any(|g| g.2);
                let m = groups.first().map(|(k, _, _)| mark(*k, 13.0)).unwrap_or_else(|| div().into_any_element());
                progress_ring(progress.unwrap_or(-1.0), 26.0, 2.0, if needs { tk::AMBER } else { tk::BLUE }, m)
            }
            SlimItem::Note(_) => icon(icons::NOTE_TEXT).size(px(14.0)).text_color(tk::TEXT).into_any_element(),
        };
        div().size_full().flex().items_center().justify_center().child(inner).into_any_element()
    }

    // ----- banner ---------------------------------------------------------------

    pub(super) fn banner_height(&self) -> f32 {
        match self.approval() {
            Some((_, p)) => match &p.preview {
                Some(Preview::Diff { lines, .. }) if !lines.is_empty() => 96.0 + 14.0 * lines.len().min(3) as f32 + 12.0,
                Some(Preview::Command(_)) => 96.0 + 28.0,
                _ => tk::BANNER_H,
            },
            None => tk::FLASH_H,
        }
    }

    fn render_alert(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some((agent, req)) = self.approval() {
            let (id, req) = (agent.id, req.clone());
            let kind = agent.kind;
            return div()
                .size_full()
                .px(px(14.0))
                .pt(px(12.0))
                .pb(px(12.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .size(px(32.0))
                                .rounded(px(10.0))
                                .bg(if req.safe { tk::WASH } else { tk::AMBER.opacity(0.16) })
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(if req.safe { mark(kind, 16.0) } else {
                                    icon(icons::DANGER_TRIANGLE).size(px(16.0)).text_color(tk::AMBER).into_any_element()
                                }),
                        )
                        .child(
                            div()
                                .id("banner-body")
                                .flex_1()
                                .min_w_0()
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.open_page(Page::Agent(id), Some(window), cx)))
                                .child(div().text_size(px(tk::T13)).font_weight(FontWeight::SEMIBOLD).text_color(tk::TEXT).truncate().child(agent.title.clone()))
                                .child(div().text_size(px(tk::T12)).text_color(tk::DIM).truncate().child(if req.question {
                                    format!("{} asks: {}", kind.label(), req.summary)
                                } else {
                                    format!("{} wants to {}", kind.label(), req.tool.to_lowercase())
                                })),
                        )
                        .child(ibtn("banner-dismiss", icons::CLOSE, 24.0, tk::FAINT).on_click(cx.listener({
                            let rid = req.request_id.clone();
                            move |this, _: &ClickEvent, _, cx| {
                                this.dismissed.insert(rid.clone());
                                this.retarget(cx);
                                cx.notify();
                            }
                        }))),
                )
                .when_some(req.preview.clone(), |d, p| d.child(preview(&p, 3)))
                .child(div().flex_1())
                .child(self.decision_row(id, &req, "banner", cx))
                .into_any_element();
        }
        let Some(flash) = &self.flash else { return div().into_any_element() };
        let (glyph, color): (AnyElement, Hsla) = match (flash.kind, flash.agent) {
            (k, Some(kind)) => (avatar(kind, 28.0, None), if k == FlashKind::Failed { tk::RED } else { tk::GREEN }),
            (FlashKind::Timer, None) => (icon(icons::TIMER).size(px(17.0)).text_color(tk::AMBER).into_any_element(), tk::AMBER),
            (FlashKind::Failed, None) => (icon(icons::DANGER_TRIANGLE).size(px(17.0)).text_color(tk::RED).into_any_element(), tk::RED),
            _ => (icon(icons::CHECK).size(px(17.0)).text_color(tk::GREEN).into_any_element(), tk::GREEN),
        };
        let task = flash.task;
        let left = 1.0 - flash.at.elapsed().as_secs_f32() / flash.until.duration_since(flash.at).as_secs_f32().max(0.1);
        div()
            .id("flash")
            .relative()
            .size_full()
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.flash = None;
                match task {
                    Some(id) => this.open_page(Page::Agent(id), Some(window), cx),
                    None => cx.notify(),
                }
            }))
            .child(glyph)
            .child(div().flex_1().min_w_0().truncate().text_size(px(tk::T13)).font_weight(FontWeight::MEDIUM).text_color(tk::TEXT).child(flash.text.clone()))
            .child(self.dot(color, 7.0, false))
            // The countdown along the bottom.
            .child(div().absolute().bottom_0().left(px(18.0)).right(px(18.0)).h(px(2.0)).child(bar(left, color.opacity(0.7), 2.0)))
            .into_any_element()
    }

    /// Deny / Always / Allow, or Review in Dove for what the island won't
    /// approve (deletions, force pushes, work outside the folder) and for
    /// questions, which need their options.
    fn decision_row(&self, id: TaskId, req: &Pending, key: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let open = cx.listener(move |this, _: &ClickEvent, _, _| this.command(ToEngine::Open { task: Some(id) }));
        let row = div().flex().items_center().justify_end().gap(px(7.0));
        if req.question {
            return row
                .child(div().flex_1().text_size(px(tk::T11)).text_color(tk::FAINT).child("Pick an answer in Dove"))
                .child(primary((key, 1usize), "Answer in Dove").on_click(open))
                .into_any_element();
        }
        let rid = req.request_id.clone();
        let deny = cx.listener({
            let rid = rid.clone();
            move |this: &mut Notch, _: &ClickEvent, _, cx| this.deny(id, rid.clone(), cx)
        });
        let row = row
            .child(div().flex_1().text_size(px(tk::T11)).text_color(tk::GHOST).child(if req.safe { "A allow · D deny" } else { "" }))
            .child(pill((key, 2usize), "Deny", false).on_click(deny));
        if req.safe {
            let always = decide(id, PermissionDecision::AllowAlways, rid.clone(), cx);
            let once = decide(id, PermissionDecision::AllowOnce, rid, cx);
            row.child(pill((key, 3usize), "Always", false).on_click(always))
                .child(primary((key, 4usize), "Allow").on_click(once))
                .into_any_element()
        } else {
            row.child(primary((key, 5usize), "Review in Dove").on_click(open)).into_any_element()
        }
    }

    // ----- expanded ---------------------------------------------------------------

    pub(super) fn page_height(&self) -> f32 {
        const BOTTOM: f32 = 14.0;
        if !self.query.trim().is_empty() {
            let n = self.hits().len().max(1) as f32;
            return (tk::HEADER_H + 10.0 + n * 44.0 + BOTTOM).min(tk::EXPANDED_MAX_H);
        }
        let notice = if self.undo.is_some() || self.deny.is_some() { 40.0 } else { 0.0 };
        let body: f32 = match &self.page {
            Page::Tab(Tab::Home) => {
                let mut h = if self.connected() { 0.0 } else { 66.0 };
                if self.approval().is_some() {
                    h += 150.0;
                }
                let working = self.snap.agents.iter().filter(|a| a.active() && !a.needs_you()).take(3).count();
                h += if working > 0 { 44.0 + working as f32 * 46.0 } else { 0.0 };
                let mut row = 0.0f32;
                if self.media.is_some() {
                    row = row.max(98.0);
                }
                if self.timer.active() {
                    row = row.max(98.0);
                }
                h += if row > 0.0 { row + 10.0 } else { 0.0 };
                let mut row2 = 0.0f32;
                if !self.clips.is_empty() {
                    row2 = row2.max(120.0);
                }
                if self.notes.iter().any(|n| n.pinned) {
                    row2 = row2.max(120.0);
                }
                h += if row2 > 0.0 { row2 + 10.0 } else { 0.0 };
                if h < 1.0 { 150.0 } else { h }
            }
            Page::Tab(Tab::Agents) => {
                if self.snap.agents.is_empty() && self.snap.flock.is_empty() {
                    150.0
                } else {
                    let groups = [
                        self.snap.agents.iter().any(|a| a.needs_you()),
                        self.snap.agents.iter().any(|a| a.status == TaskStatus::Running && !a.needs_you()),
                        self.snap.agents.iter().any(|a| !a.active()),
                        !self.snap.flock.is_empty(),
                    ]
                    .iter()
                    .filter(|g| **g)
                    .count() as f32;
                    let rows = (self.snap.agents.len() + self.snap.flock.len()) as f32;
                    let limits = if self.snap.limits.is_empty() { 0.0 } else { 36.0 };
                    groups * 26.0 + rows * (tk::ROW_H + 2.0) + limits
                }
            }
            Page::Agent(_) => 330.0,
            Page::NewAgent => {
                let rows = |n: usize, per: usize| n.div_ceil(per).max(1) as f32;
                40.0 + 10.0 + 24.0 + rows(self.snap.installed.len(), 4) * 34.0 + 24.0
                    + rows(self.snap.projects.len().min(8), 4) * 34.0
                    + 16.0
                    + 30.0
            }
            Page::Tab(Tab::Music) => {
                if self.media.is_some() {
                    210.0
                } else {
                    130.0
                }
            }
            Page::Tab(Tab::Tray) => {
                let n = self.tray_items().len();
                34.0 + 10.0 + if n == 0 { 110.0 } else { (n.div_ceil(2) as f32) * 92.0 }
            }
            Page::Tab(Tab::Notes) => 250.0f32.max(36.0 + self.notes.len() as f32 * 48.0),
            Page::Tab(Tab::Timer) => 250.0,
            Page::Tab(Tab::Alerts) => {
                if self.notices.is_empty() { 130.0 } else { 34.0 + self.notices.len() as f32 * 46.0 }
            }
            Page::Tab(Tab::Dev) => {
                if self.ports.is_empty() { 140.0 } else { 34.0 + self.ports.len() as f32 * 50.0 }
            }
        };
        (tk::HEADER_H + body + notice + BOTTOM).clamp(170.0, tk::EXPANDED_MAX_H)
    }

    fn header(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        const SLOT: f32 = 30.0;
        const STEP: f32 = 32.0;
        let tabs = self.prefs.tabs();
        let current = tab_of(&self.page);
        let at = tabs.iter().position(|t| *t == current).unwrap_or(0) as f32;
        let from = self.tab_from.and_then(|f| tabs.iter().position(|t| *t == f)).map(|i| i as f32).unwrap_or(at);
        // The puck slides between tabs (critically damped feel, 220 ms).
        let p = (self.page_at.elapsed().as_secs_f32() / 0.22).min(1.0);
        let e = 1.0 - (1.0 - p).powi(3);
        if p < 1.0 && !crate::motion::reduced_now() {
            window.request_animation_frame();
        }
        let puck_x = if crate::motion::reduced_now() { at * STEP } else { (from + (at - from) * e) * STEP };
        let needs = self.snap.agents.iter().any(|a| a.needs_you());
        let typing = !self.query.is_empty();
        let mut strip = div().relative().flex().items_center().gap(px(STEP - SLOT)).child(
            div()
                .absolute()
                .left(px(puck_x))
                .top_0()
                .size(px(SLOT))
                .rounded_full()
                .bg(tk::WASH_ON)
                .border_1()
                .border_color(tk::HAIRLINE),
        );
        for (i, t) in tabs.iter().copied().enumerate() {
            let on = t == current;
            let hovered = self.hovered_tab == Some(t);
            let this = cx.entity().downgrade();
            strip = strip.child(
                div()
                    .id(("tab", i))
                    .relative()
                    .size(px(SLOT))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(tk::WASH))
                    .on_hover(move |h, _, cx| {
                        let _ = this.update(cx, |n, cx| {
                            n.hovered_tab = if *h { Some(t) } else if n.hovered_tab == Some(t) { None } else { n.hovered_tab };
                            cx.notify();
                        });
                    })
                    .child(icon(glyph(t)).size(px(15.0)).text_color(if on { tk::TEXT } else { tk::DIM }))
                    .when(t == Tab::Agents && needs, |d| d.child(self.dot(tk::AMBER, 6.0, true).absolute().top(px(4.0)).right(px(4.0))))
                    .when(t == Tab::Alerts && self.unread_notices > 0, |d| {
                        d.child(self.dot(tk::BLUE, 6.0, false).absolute().top(px(4.0)).right(px(4.0)))
                    })
                    .when(hovered, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(px(SLOT + 2.0))
                                .left(px(-20.0))
                                .w(px(70.0))
                                .flex()
                                .justify_center()
                                .child(
                                    div()
                                        .px(px(7.0))
                                        .py(px(2.0))
                                        .rounded_full()
                                        .bg(gpui::hsla(0.0, 0.0, 0.0, 0.7))
                                        .text_size(px(tk::T11))
                                        .text_color(tk::TEXT)
                                        .child(t.label()),
                                ),
                        )
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.open_page(Page::Tab(t), Some(window), cx))),
            );
        }
        let command = self.inputs.as_ref().map(|i| i.command.clone());
        div()
            .h(px(tk::HEADER_H))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .when(!typing, |d| d.child(strip))
            .child(
                div()
                    .id("command")
                    .flex_1()
                    .min_w(px(36.0))
                    .h(px(30.0))
                    .px(px(10.0))
                    .rounded_full()
                    .bg(if typing { tk::WASH_HOVER } else { tk::WASH })
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(tk::T12))
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, _, cx| this.take_focus(cx)))
                    .child(icon(icons::MAGNIFER).size(px(13.0)).text_color(tk::FAINT))
                    .children(command.map(|c| div().flex_1().min_w_0().child(Input::new(&c)))),
            )
            .when_some(self.toast.as_ref(), |d, (text, at)| {
                let t = (at.elapsed().as_secs_f32() / 0.18).min(1.0);
                d.child(
                    div()
                        .flex_none()
                        .px(px(10.0))
                        .h(px(24.0))
                        .rounded_full()
                        .bg(tk::GREEN.opacity(0.16))
                        .flex()
                        .items_center()
                        .text_size(px(tk::T11))
                        .text_color(tk::GREEN)
                        .opacity(t)
                        .child(text.clone()),
                )
            })
            .when(self.prefs.focus, |d| d.child(icon(icons::MOON).size(px(14.0)).text_color(tk::AMBER)))
            .child(
                ibtn("new-agent", icons::PLUS, 30.0, tk::TEXT)
                    .when(self.page == Page::NewAgent, |d| d.bg(tk::WASH_ON))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_page(Page::NewAgent, Some(window), cx))),
            )
            .child(
                ibtn("pin", icons::PIN, 30.0, if self.machine.pinned { tk::TEXT } else { tk::FAINT })
                    .when(self.machine.pinned, |d| d.bg(tk::WASH_ON))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        let pinned = !this.machine.pinned;
                        this.machine.set_pinned(pinned);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// The Undo bar for the last destructive action.
    fn undo_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let text = if self.deny.is_some() {
            "Denying…"
        } else {
            match self.undo.as_ref().map(|u| &u.0) {
                Some(super::data::Undo::Note(..)) => "Note deleted",
                Some(super::data::Undo::Clips(..)) => "Clipboard cleared",
                Some(super::data::Undo::Shelf(..)) => "Removed from the tray",
                None => return None,
            }
        };
        Some(
            div()
                .mt(px(8.0))
                .h(px(32.0))
                .px(px(12.0))
                .rounded_full()
                .bg(tk::WASH)
                .flex()
                .items_center()
                .text_size(px(tk::T12))
                .text_color(tk::DIM)
                .child(div().flex_1().child(text))
                .child(
                    div()
                        .id("undo")
                        .px(px(10.0))
                        .text_color(tk::TEXT)
                        .font_weight(FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .child("Undo")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.undo_last(cx))),
                )
                .into_any_element(),
        )
    }

    fn render_expanded(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let body = if !self.query.trim().is_empty() {
            self.page_hits(cx)
        } else {
            match self.page.clone() {
                Page::Tab(Tab::Home) => self.page_home(cx),
                Page::Tab(Tab::Agents) => self.page_agents(cx),
                Page::Agent(id) => self.page_agent(id, cx),
                Page::NewAgent => self.page_new(cx),
                Page::Tab(Tab::Music) => self.page_music(cx),
                Page::Tab(Tab::Tray) => self.page_tray(cx),
                Page::Tab(Tab::Notes) => self.page_notes(cx),
                Page::Tab(Tab::Timer) => self.page_timer(cx),
                Page::Tab(Tab::Alerts) => self.page_alerts(cx),
                Page::Tab(Tab::Dev) => self.page_dev(cx),
            }
        };
        let undo = self.undo_bar(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.header(window, cx))
            .child(div().flex_1().min_h_0().px(px(tk::PAD)).child(body))
            .children(undo.map(|u| div().px(px(tk::PAD)).child(u)))
            .child(div().h(px(14.0)))
            .into_any_element()
    }

    fn page_hits(&self, cx: &mut Context<Self>) -> AnyElement {
        let hits = self.hits();
        let mut col = div().id("hits").size_full().overflow_y_scroll().flex().flex_col().gap(px(2.0)).pt(px(4.0));
        for (i, (hit, label, kind)) in hits.into_iter().enumerate() {
            let on = i == self.hit;
            let glyph = match hit {
                Hit::Agent(_) => icons::BOT,
                Hit::Note(_) | Hit::NewNote(_) => icons::NOTE_TEXT,
                Hit::Clip(_) => icons::COPY,
                Hit::Tab(t) => glyph(t),
                Hit::NewAgent(_) => icons::SPARKLE_AGENT,
                Hit::Timer(_) => icons::TIMER,
                Hit::Capture => icons::GALLERY,
                Hit::Focus => icons::MOON,
            };
            col = col.child(
                div()
                    .id(("hit", i))
                    .h(px(42.0))
                    .px(px(10.0))
                    .rounded(px(tk::R_ROW))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .cursor_pointer()
                    .when(on, |d| d.bg(tk::WASH_HOVER))
                    .hover(|s| s.bg(tk::WASH))
                    .child(icon(glyph).size(px(15.0)).text_color(if on { tk::TEXT } else { tk::DIM }))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(tk::T13)).text_color(tk::TEXT).child(label))
                    .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(kind))
                    .when(on, |d| d.child(icon(icons::RETURN).size(px(12.0)).text_color(tk::FAINT)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.run_hit(Some(i), window, cx))),
            );
        }
        col.into_any_element()
    }

    fn agent_row(&self, a: &Agent, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let id = a.id;
        let (label, color) = status_of(a);
        let hovered = self.hovered_row == Some(id);
        let ticker = a.activity.clone().filter(|s| !s.is_empty()).or_else(|| (!a.steps.is_empty()).then(|| a.steps.clone()));
        let now = dove_core::store::now_ms();
        let time = a.elapsed_secs(now).filter(|s| *s > 0).map(elapsed);
        let this = cx.entity().downgrade();
        let running = a.status == TaskStatus::Running && a.live;
        div()
            .id(("agent", ix))
            .h(px(tk::ROW_H))
            .px(px(8.0))
            .rounded(px(tk::R_ROW))
            .flex()
            .items_center()
            .gap(px(10.0))
            .cursor_pointer()
            .hover(|s| s.bg(tk::WASH))
            .on_hover(move |h, _, cx| {
                let _ = this.update(cx, |n, cx| {
                    n.hovered_row = if *h { Some(id) } else if n.hovered_row == Some(id) { None } else { n.hovered_row };
                    cx.notify();
                });
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.open_page(Page::Agent(id), Some(window), cx)))
            .child(avatar(a.kind, 28.0, agent_progress(a)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(px(tk::T13)).font_weight(FontWeight::MEDIUM).text_color(tk::TEXT).truncate().child(a.title.clone()))
                    .child(match ticker {
                        Some(t) => div()
                            .font_family(tk::MONO)
                            .text_size(px(tk::T11))
                            .text_color(tk::FAINT)
                            .truncate()
                            .child(t)
                            .into_any_element(),
                        None => div().text_size(px(tk::T11)).text_color(tk::FAINT).truncate().child(a.project.clone()).into_any_element(),
                    }),
            )
            .child(if hovered {
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .child(ibtn(("row-msg", ix), icons::CHAT_ROUND_LINE, 26.0, tk::DIM).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.open_page(Page::Agent(id), Some(window), cx);
                    })))
                    .when(running, |d| {
                        d.child(ibtn(("row-stop", ix), icons::STOP, 26.0, tk::RED).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.command(ToEngine::Stop { task: id });
                            this.toast("Stopping");
                            cx.notify();
                        })))
                    })
                    .child(ibtn(("row-open", ix), icons::ARROW_UP_RIGHT, 26.0, tk::DIM).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.command(ToEngine::Open { task: Some(id) });
                    })))
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .flex_none()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .child(self.dot(color, 6.0, matches!(label, "Working" | "Needs you")))
                            .child(div().text_size(px(tk::T11)).text_color(color).child(label)),
                    )
                    .child(
                        div()
                            .text_size(px(tk::T11))
                            .text_color(tk::GHOST)
                            .child(match (time, a.cost) {
                                (Some(t), Some(c)) => format!("{t} · ${c:.2}"),
                                (Some(t), None) => t,
                                (None, Some(c)) => format!("${c:.2}"),
                                (None, None) => String::new(),
                            }),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }

    fn page_home(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let key = self.key();
        let mut cards: Vec<AnyElement> = Vec::new();
        if !self.connected() {
            cards.push(
                card()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(icon(icons::DOVE_LOGO).size(px(18.0)).text_color(tk::DIM))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(tk::T13)).text_color(tk::TEXT).child("Dove isn't running"))
                            .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child("Music, the tray, notes and the timer still work.")),
                    )
                    .child(primary("open-dove", "Open Dove").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.command(ToEngine::Open { task: None });
                        this.toast("Opening Dove…");
                        cx.notify();
                    })))
                    .into_any_element(),
            );
        }
        if let Some((agent, req)) = self.approval() {
            let (id, req, kind, title) = (agent.id, req.clone(), agent.kind, agent.title.clone());
            cards.push(
                card()
                    .border_color(tk::AMBER.opacity(0.35))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(mark(kind, 14.0))
                            .child(div().flex_1().min_w_0().truncate().text_size(px(tk::T13)).font_weight(FontWeight::SEMIBOLD).text_color(tk::TEXT).child(title))
                            .child(div().text_size(px(tk::T11)).text_color(tk::AMBER).child("Needs you")),
                    )
                    .child(div().text_size(px(tk::T12)).text_color(tk::DIM).line_clamp(2).child(format!("{} · {}", req.tool, req.summary)))
                    .when_some(req.preview.clone(), |d, p| d.child(preview(&p, 3)))
                    .child(self.decision_row(id, &req, "home-decide", cx))
                    .into_any_element(),
            );
        }
        let working: Vec<Agent> = self.snap.agents.iter().filter(|a| a.active() && !a.needs_you()).take(3).cloned().collect();
        if !working.is_empty() {
            let mut c = card().flex().flex_col().gap(px(2.0)).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .pb(px(4.0))
                    .child(icon(icons::BOT).size(px(13.0)).text_color(tk::DIM))
                    .child(section("Working"))
                    .child(div().flex_1())
                    .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(working.len().to_string())),
            );
            for (i, a) in working.iter().enumerate() {
                c = c.child(self.agent_row(a, 100 + i, cx));
            }
            cards.push(c.into_any_element());
        }
        // Now playing and the timer, side by side.
        let mut duo: Vec<AnyElement> = Vec::new();
        if let Some(m) = self.media.clone() {
            duo.push(
                card()
                    .id("home-music")
                    .flex_1()
                    .min_w_0()
                    .h(px(98.0))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_page(Page::Tab(Tab::Music), Some(window), cx)))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(self.art(34.0, 9.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().text_size(px(tk::T12)).text_color(tk::TEXT).truncate().child(m.title.clone()))
                                    .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).truncate().child(m.artist.clone())),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(self.eq(14.0))
                            .child(div().flex_1().child(bar(m.progress(), tk::TEXT, 3.0)))
                            .child(self.play_button("home-play", 26.0, cx)),
                    )
                    .into_any_element(),
            );
        }
        if self.timer.active() {
            duo.push(
                card()
                    .id("home-timer")
                    .flex_1()
                    .min_w_0()
                    .h(px(98.0))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_page(Page::Tab(Tab::Timer), Some(window), cx)))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(progress_ring(
                        self.timer.progress(),
                        48.0,
                        3.0,
                        tk::AMBER,
                        icon(icons::TIMER).size(px(16.0)).text_color(tk::AMBER).into_any_element(),
                    ))
                    .child(
                        div()
                            .child(div().text_size(px(tk::T20)).font_weight(FontWeight::SEMIBOLD).text_color(tk::TEXT).child(mins_secs(self.timer.remaining())))
                            .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(if self.timer.running() { "Running" } else { "Paused" })),
                    )
                    .into_any_element(),
            );
        }
        if !duo.is_empty() {
            cards.push(div().flex().gap(px(10.0)).children(duo).into_any_element());
        }
        // Recent clips and the pinned note.
        let mut duo: Vec<AnyElement> = Vec::new();
        if !self.clips.is_empty() {
            let mut c = card()
                .flex_1()
                .min_w_0()
                .h(px(120.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(div().flex().items_center().gap(px(6.0)).child(icon(icons::COPY).size(px(12.0)).text_color(tk::DIM)).child(section("Recent clips")));
            for (i, clip) in self.clips.iter().take(3).enumerate() {
                let text = clip.as_text().map(|t| t.trim().lines().next().unwrap_or("").to_string()).unwrap_or_else(|| "Image".into());
                let copy = clip.clone();
                c = c.child(
                    div()
                        .id(("home-clip", i))
                        .px(px(6.0))
                        .py(px(3.0))
                        .rounded(px(8.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(tk::WASH))
                        .text_size(px(tk::T12))
                        .text_color(tk::DIM)
                        .truncate()
                        .child(text)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.write_to_clipboard(match &copy.body {
                                ClipBody::Text(t) => ClipboardItem::new_string(t.clone()),
                                ClipBody::Image(im) => ClipboardItem::new_image(im),
                            });
                            this.toast("Copied");
                            cx.notify();
                        })),
                );
            }
            duo.push(c.into_any_element());
        }
        if let Some(n) = self.notes.iter().find(|n| n.pinned).cloned() {
            let id = n.id;
            duo.push(
                card()
                    .id("home-note")
                    .flex_1()
                    .min_w_0()
                    .h(px(120.0))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.selected_note = Some(id);
                        this.open_page(Page::Tab(Tab::Notes), Some(window), cx);
                    }))
                    .child(div().flex().items_center().gap(px(6.0)).pb(px(4.0)).child(icon(icons::PIN).size(px(12.0)).text_color(tk::AMBER)).child(section("Pinned note")))
                    .child(div().text_size(px(tk::T13)).font_weight(FontWeight::MEDIUM).text_color(tk::TEXT).truncate().child(n.title()))
                    .child(div().text_size(px(tk::T12)).text_color(tk::DIM).line_clamp(3).child(n.text.lines().skip(1).collect::<Vec<_>>().join(" ")))
                    .into_any_element(),
            );
        }
        if !duo.is_empty() {
            cards.push(div().flex().gap(px(10.0)).children(duo).into_any_element());
        }
        if cards.is_empty() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(12.0))
                .child(icon(icons::DOVE_LOGO).size(px(28.0)).text_color(tk::FAINT))
                .child(div().text_size(px(tk::T13)).text_color(tk::DIM).child("All quiet"))
                .child(
                    div()
                        .flex()
                        .gap(px(6.0))
                        .child(pill("empty-agent", "Start an agent", false).on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_page(Page::NewAgent, Some(window), cx))))
                        .child(pill("empty-music", "Play music", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.media_src.send(super::media::Control::PlayPause);
                            cx.notify();
                        })))
                        .child(pill("empty-note", "New note", false).on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.selected_note = None;
                            this.editing_note = true;
                            this.open_page(Page::Tab(Tab::Notes), Some(window), cx);
                        }))),
                )
                .into_any_element();
        }
        div()
            .id("home")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .children(cards.into_iter().enumerate().map(|(i, c)| rise(c, (key, i), i)))
            .into_any_element()
    }

    fn page_agents(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.snap.agents.is_empty() && self.snap.flock.is_empty() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.0))
                .child(div().text_size(px(tk::T13)).text_color(tk::DIM).child("No agents yet"))
                .child(primary("agents-new", "Start an agent").on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_page(Page::NewAgent, Some(window), cx))))
                .into_any_element();
        }
        let key = self.key();
        let mut col = div().id("agents").size_full().overflow_y_scroll().flex().flex_col().gap(px(2.0));
        let mut index = 0;
        let groups: [(&str, Vec<&Agent>); 3] = [
            ("Needs you", self.snap.agents.iter().filter(|a| a.needs_you()).collect()),
            ("Working", self.snap.agents.iter().filter(|a| a.status == TaskStatus::Running && !a.needs_you()).collect()),
            ("Recent", self.snap.agents.iter().filter(|a| !a.active()).collect()),
        ];
        for (label, agents) in groups {
            if agents.is_empty() {
                continue;
            }
            col = col.child(div().pt(px(6.0)).pb(px(2.0)).px(px(8.0)).child(section(format!("{label} · {}", agents.len()))));
            for a in agents {
                col = col.child(rise(self.agent_row(a, index, cx), (key, index), index));
                index += 1;
            }
        }
        if !self.snap.flock.is_empty() {
            col = col.child(div().pt(px(6.0)).pb(px(2.0)).px(px(8.0)).child(section(format!("Flock · {}", self.snap.flock.len()))));
            for (i, f) in self.snap.flock.iter().enumerate() {
                let (label, color) = if f.needs_input {
                    ("Needs you", tk::AMBER)
                } else if f.working {
                    ("Working", tk::BLUE)
                } else {
                    ("Idle", tk::FAINT)
                };
                col = col.child(
                    div()
                        .id(("flock", i))
                        .h(px(tk::ROW_H))
                        .px(px(8.0))
                        .rounded(px(tk::R_ROW))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(tk::WASH))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, _| this.command(ToEngine::OpenFlock)))
                        .child(avatar(f.kind, 28.0, (f.working || f.needs_input).then_some((-1.0, color))))
                        .child(div().flex_1().min_w_0().truncate().text_size(px(tk::T13)).text_color(tk::TEXT).child(f.place.clone()))
                        .child(self.dot(color, 6.0, f.working || f.needs_input))
                        .child(div().text_size(px(tk::T11)).text_color(color).child(label)),
                );
            }
        }
        if !self.snap.limits.is_empty() {
            let mut strip = div().mt(px(8.0)).px(px(8.0)).flex().gap(px(10.0));
            for m in self.snap.limits.iter().take(4) {
                let color = if m.used >= 80.0 { tk::AMBER } else { tk::DIM };
                strip = strip.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .text_size(px(tk::T11))
                                .child(mark(m.kind, 10.0))
                                .child(div().flex_1().min_w_0().truncate().text_color(tk::FAINT).child(m.label.clone()))
                                .child(div().text_color(color).child(format!("{:.0}%", m.used))),
                        )
                        .child(bar(m.used as f32 / 100.0, color, 3.0)),
                );
            }
            col = col.child(strip);
        }
        col.into_any_element()
    }

    fn page_agent(&self, id: TaskId, cx: &mut Context<Self>) -> AnyElement {
        let Some(a) = self.snap.agents.iter().find(|a| a.id == id).cloned() else { return div().into_any_element() };
        let (label, color) = status_of(&a);
        let running = a.status == TaskStatus::Running && a.live;
        let chip = |text: String, glyph: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(4.0))
                .px(px(7.0))
                .h(px(20.0))
                .rounded_full()
                .bg(tk::WASH)
                .text_size(px(tk::T11))
                .text_color(tk::DIM)
                .child(icon(glyph).size(px(11.0)).text_color(tk::FAINT))
                .child(div().max_w(px(120.0)).truncate().child(text))
        };
        let mut col = div().size_full().flex().flex_col().gap(px(8.0));
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(ibtn("agent-back", icons::ARROW_LEFT, 28.0, tk::DIM).on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_page(Page::Tab(Tab::Agents), Some(window), cx))))
                .child(avatar(a.kind, 30.0, agent_progress(&a)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .child(div().text_size(px(tk::T13)).font_weight(FontWeight::SEMIBOLD).text_color(tk::TEXT).truncate().child(a.title.clone()))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.0))
                                .child(div().flex().items_center().gap(px(4.0)).text_size(px(tk::T11)).child(self.dot(color, 6.0, running || label == "Needs you")).child(div().text_color(color).child(label)))
                                .when_some(a.branch.clone(), |d, b| d.child(chip(b, icons::GIT_BRANCH)))
                                .when(a.worktree, |d| d.child(chip("worktree".into(), icons::WORKTREE))),
                        ),
                )
                .when(running, |d| {
                    d.child(ibtn("agent-stop", icons::STOP, 30.0, tk::RED).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.command(ToEngine::Stop { task: id });
                        this.toast("Stopping");
                        cx.notify();
                    })))
                })
                .child(ibtn("agent-open", icons::ARROW_UP_RIGHT, 30.0, tk::DIM).on_click(cx.listener(move |this, _: &ClickEvent, _, _| {
                    this.command(ToEngine::Open { task: Some(id) });
                }))),
        );
        for (i, req) in a.pending.iter().enumerate() {
            col = col.child(
                card()
                    .border_color(tk::AMBER.opacity(0.35))
                    .flex()
                    .flex_col()
                    .gap(px(7.0))
                    .child(div().text_size(px(tk::T12)).text_color(tk::DIM).line_clamp(2).child(format!("{} · {}", req.tool, req.summary)))
                    .when_some(req.preview.clone(), |d, p| d.child(preview(&p, 6)))
                    .child(self.decision_row(id, req, if i == 0 { "agent-decide-0" } else { "agent-decide-n" }, cx)),
            );
        }
        let mut body = div()
            .id("agent-reply")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(11.0))
            .rounded(px(16.0))
            .bg(gpui::hsla(0.0, 0.0, 0.0, 0.18))
            .text_size(px(tk::T12))
            .flex()
            .flex_col()
            .gap(px(8.0));
        if !a.steps.is_empty() || a.todos.is_some() {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(px(tk::T11))
                    .text_color(tk::FAINT)
                    .child(icon(icons::CHECKLIST).size(px(12.0)).text_color(tk::FAINT))
                    .child(div().flex_1().min_w_0().truncate().child(a.steps.clone()))
                    .when_some(a.todos, |d, (done, total)| {
                        d.child(div().w(px(80.0)).child(bar(done as f32 / total.max(1) as f32, tk::BLUE, 3.0))).child(format!("{done}/{total}"))
                    }),
            );
        }
        let reply = if self.reply.trim().is_empty() {
            a.activity.clone().unwrap_or_else(|| "No reply yet.".into())
        } else {
            self.reply.clone()
        };
        body = body.child(rich_text(&reply));
        col = col.child(body);
        // Quick replies, @ attach, the composer.
        let quick = ["Continue", "Run the tests", "Explain what you did"];
        let mut chips = div().flex().gap(px(6.0)).flex_wrap();
        for (i, q) in quick.iter().enumerate() {
            let q = q.to_string();
            chips = chips.child(pill(("quick", i), q.clone(), false).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.command(ToEngine::Send { task: id, text: q.clone() });
                this.toast("Sent");
                cx.notify();
            })));
        }
        if let Some(p) = self.shelf.first().cloned() {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            chips = chips.child(
                pill("attach", format!("@ {name}"), false).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    if let Some(i) = this.inputs.clone() {
                        let current = i.message.read(cx).value().to_string();
                        let joined = if current.is_empty() { format!("{} ", p.display()) } else { format!("{current} {} ", p.display()) };
                        i.message.update(cx, |s, cx| s.set_value(joined, window, cx));
                    }
                    cx.notify();
                })),
            );
        }
        col = col.child(chips);
        let placeholder = if a.status == TaskStatus::Running {
            if a.steers { "Steer: it reads this now" } else { "Queue: it reads this when the turn ends" }
        } else {
            "Message this agent…"
        };
        let message = self.inputs.as_ref().map(|i| i.message.clone());
        col = col.child(
            div()
                .h(px(38.0))
                .pl(px(14.0))
                .pr(px(4.0))
                .rounded_full()
                .bg(tk::WASH)
                .border_1()
                .border_color(tk::HAIRLINE)
                .flex()
                .items_center()
                .gap(px(6.0))
                .text_size(px(tk::T12))
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, _, cx| this.take_focus(cx)))
                .children(message.map(|m| div().flex_1().min_w_0().child(Input::new(&m))))
                .child(div().text_size(px(tk::T11)).text_color(tk::GHOST).child(placeholder))
                .child(
                    div()
                        .id("agent-send")
                        .size(px(30.0))
                        .flex_none()
                        .rounded_full()
                        .bg(tk::WHITE)
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|s| s.bg(tk::WHITE_HOVER))
                        .child(icon(icons::SEND_UP).size(px(14.0)).text_color(tk::INK))
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.send_message(window, cx))),
                ),
        );
        col.into_any_element()
    }

    fn page_new(&self, cx: &mut Context<Self>) -> AnyElement {
        let form = &self.new_agent;
        let mut agents = div().flex().flex_wrap().gap(px(6.0));
        for (i, kind) in self.snap.installed.iter().enumerate() {
            let kind = *kind;
            agents = agents.child(
                pill(("new-kind", i), kind.label(), form.kind == Some(kind))
                    .child(mark(kind, 12.0))
                    .flex_row_reverse()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.new_agent.kind = Some(kind);
                        cx.notify();
                    })),
            );
        }
        let mut projects = div().flex().flex_wrap().gap(px(6.0));
        for (i, (pid, name)) in self.snap.projects.iter().take(8).enumerate() {
            let pid = *pid;
            projects = projects.child(pill(("new-project", i), name.clone(), form.project == Some(pid)).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.new_agent.project = Some(pid);
                cx.notify();
            })));
        }
        let prompt = self.inputs.as_ref().map(|i| i.prompt.clone());
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .h(px(40.0))
                    .px(px(14.0))
                    .rounded(px(16.0))
                    .bg(tk::WASH)
                    .border_1()
                    .border_color(tk::HAIRLINE)
                    .flex()
                    .items_center()
                    .text_size(px(tk::T13))
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, _, cx| this.take_focus(cx)))
                    .children(prompt.map(|p| div().flex_1().child(Input::new(&p)))),
            )
            .child(section("Agent"))
            .child(agents)
            .child(section("Project"))
            .child(if self.snap.projects.is_empty() {
                div().text_size(px(tk::T12)).text_color(tk::DIM).child(if self.connected() { "Add a project in Dove first." } else { "Dove starts when you press Start." }).into_any_element()
            } else {
                projects.into_any_element()
            })
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        pill("new-worktree", "Own worktree", form.worktree)
                            .child(icon(icons::WORKTREE).size(px(12.0)).text_color(if form.worktree { tk::TEXT } else { tk::DIM }))
                            .flex_row_reverse()
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.new_agent.worktree = !this.new_agent.worktree;
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1().min_w_0().truncate().text_size(px(tk::T11)).text_color(tk::RED).child(form.error.clone().unwrap_or_default()))
                    .child(primary("new-start", "Start").on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.start_agent(window, cx)))),
            )
            .into_any_element()
    }

    fn play_button(&self, id: &'static str, size: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let playing = self.media.as_ref().is_some_and(|m| m.playing);
        div()
            .id(id)
            .size(px(size))
            .flex_none()
            .rounded_full()
            .bg(tk::WHITE)
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(tk::WHITE_HOVER))
            .child(icon(if playing { icons::MEDIA_PAUSE } else { icons::MEDIA_PLAY }).size(px((size * 0.42).round())).text_color(tk::INK))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.toggle_play(cx);
            }))
    }

    pub(super) fn toggle_play(&mut self, cx: &mut Context<Self>) {
        self.media_src.send(super::media::Control::PlayPause);
        if let Some(m) = &mut self.media {
            m.position = m.now();
            m.at = Instant::now();
            m.playing = !m.playing;
        }
        cx.notify();
    }

    fn page_music(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(m) = self.media.clone() else {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .child(icon(icons::MUSIC_NOTE).size(px(26.0)).text_color(tk::FAINT))
                .child(div().text_size(px(tk::T13)).text_color(tk::DIM).child("Nothing playing"))
                .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child("Play something in Spotify, a browser or any player."))
                .child(
                    div()
                        .flex()
                        .gap(px(6.0))
                        .child(pill("open-spotify", "Spotify", false).on_click(cx.listener(|_, _: &ClickEvent, _, _| super::media::open_player("Spotify.exe")))),
                )
                .into_any_element();
        };
        let now = m.now();
        let progress = self.scrub.unwrap_or(m.progress());
        let art = if m.playing { 88.0 } else { 84.0 };
        let shown_volume = self.volume_shown_until.is_some();
        let this = cx.entity().downgrade();
        div()
            .relative()
            .size_full()
            .rounded(px(tk::R_CARD))
            .overflow_hidden()
            .when_some(m.backdrop.clone(), |d, b| {
                d.child(img(b).absolute().top_0().left_0().size_full().object_fit(ObjectFit::Cover).opacity(0.9))
            })
            .child(
                div()
                    .relative()
                    .size_full()
                    .p(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(14.0))
                            .child(div().size(px(88.0)).flex().items_center().justify_center().child(self.art(art, 18.0)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.0))
                                    .child(div().text_size(px(tk::T15)).font_weight(FontWeight::SEMIBOLD).text_color(tk::TEXT).truncate().child(m.title.clone()))
                                    .child(div().text_size(px(tk::T12)).text_color(tk::DIM).truncate().child(m.artist.clone()))
                                    .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).truncate().child(m.app.clone())),
                            )
                            .child(self.eq(18.0))
                            .when(shown_volume, |d| {
                                d.child(
                                    div()
                                        .w(px(6.0))
                                        .h(px(64.0))
                                        .rounded_full()
                                        .bg(tk::WASH_HOVER)
                                        .flex()
                                        .flex_col()
                                        .justify_end()
                                        .child(div().w_full().rounded_full().bg(tk::TEXT).h(gpui::relative(self.audio_state.volume.clamp(0.0, 1.0)))),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(5.0))
                            .child(
                                div()
                                    .id("scrubber")
                                    .group("scrub")
                                    .h(px(14.0))
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .on_mouse_down(MouseButton::Left, {
                                        let this = this.clone();
                                        move |ev: &MouseDownEvent, _, cx| {
                                            let x = ev.position.x;
                                            let _ = this.update(cx, |n, cx| {
                                                n.scrub = Some(-1.0 - f32::from(x));
                                                cx.notify();
                                            });
                                        }
                                    })
                                    .child(
                                        canvas(
                                            |bounds, _, _| bounds,
                                            {
                                                let this = this.clone();
                                                move |bounds, _, _, cx| {
                                                    // Resolve a pending press into a fraction of the bar's
                                                    // width now that its bounds are known.
                                                    let _ = this.update(cx, |n, _| {
                                                        if let Some(s) = n.scrub
                                                            && s <= -1.0
                                                        {
                                                            let x = -1.0 - s;
                                                            let left = f32::from(bounds.origin.x);
                                                            let w = f32::from(bounds.size.width).max(1.0);
                                                            n.scrub = Some(((x - left) / w).clamp(0.0, 1.0));
                                                            n.scrub_bounds = Some((left, w));
                                                        } else {
                                                            n.scrub_bounds = Some((f32::from(bounds.origin.x), f32::from(bounds.size.width).max(1.0)));
                                                        }
                                                    });
                                                }
                                            },
                                        )
                                        .absolute()
                                        .size_full(),
                                    )
                                    .child(div().w_full().h(px(4.0)).group_hover("scrub", |s| s.h(px(8.0))).rounded_full().bg(tk::WASH_HOVER).child(div().h_full().rounded_full().bg(tk::TEXT).w(gpui::relative(progress.clamp(0.0, 1.0)))))
                                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                                        if this.scrub.is_some()
                                            && ev.pressed_button == Some(MouseButton::Left)
                                            && let Some((left, w)) = this.scrub_bounds
                                        {
                                            this.scrub = Some(((f32::from(ev.position.x) - left) / w).clamp(0.0, 1.0));
                                            cx.notify();
                                        }
                                    }))
                                    .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                        if let Some(f) = this.scrub.take().filter(|f| *f >= 0.0)
                                            && this.media.as_ref().is_some_and(|m| m.can_seek)
                                        {
                                            this.media_src.send(super::media::Control::Seek(f));
                                            if let Some(m) = &mut this.media {
                                                m.position = m.duration.mul_f32(f);
                                                m.at = Instant::now();
                                            }
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .text_size(px(tk::T11))
                                    .text_color(tk::FAINT)
                                    .child(mins_secs(if self.scrub.is_some() { m.duration.mul_f32(progress) } else { now }))
                                    .child(if m.duration.is_zero() { String::new() } else { format!("-{}", mins_secs(m.duration.saturating_sub(now))) }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(20.0))
                            .when_some(m.shuffle, |d, on| {
                                d.child(ibtn("music-shuffle", icons::MEDIA_SHUFFLE, 32.0, if on { tk::GREEN } else { tk::DIM }).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.media_src.send(super::media::Control::Shuffle(!on));
                                    cx.notify();
                                })))
                            })
                            .child(ibtn("music-prev", icons::MEDIA_PREVIOUS, 38.0, tk::TEXT).on_click(cx.listener(|this, _: &ClickEvent, _, _| this.media_src.send(super::media::Control::Previous))))
                            .child(self.play_button("music-play", 48.0, cx))
                            .child(ibtn("music-next", icons::MEDIA_NEXT, 38.0, tk::TEXT).on_click(cx.listener(|this, _: &ClickEvent, _, _| this.media_src.send(super::media::Control::Next))))
                            .when(!self.audio_state.device.is_empty(), |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(4.0))
                                        .px(px(8.0))
                                        .h(px(24.0))
                                        .rounded_full()
                                        .bg(tk::WASH)
                                        .text_size(px(tk::T11))
                                        .text_color(tk::DIM)
                                        .child(icon(icons::VOLUME_LOUD).size(px(12.0)).text_color(tk::FAINT))
                                        .child(div().max_w(px(90.0)).truncate().child(self.audio_state.device.clone())),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    /// The tray's items under the current filter: files first (newest
    /// first), then clipboard entries.
    pub(super) fn tray_items(&self) -> Vec<TrayItem> {
        let mut out = Vec::new();
        let f = self.tray_filter;
        if matches!(f, TrayFilter::All | TrayFilter::Files) {
            out.extend((0..self.shelf.len()).map(TrayItem::File));
        }
        for (i, c) in self.clips.iter().enumerate() {
            let keep = match f {
                TrayFilter::All => true,
                TrayFilter::Files => false,
                TrayFilter::Text => matches!(c.kind(), ClipKind::Text | ClipKind::Code(_) | ClipKind::Color(..)),
                TrayFilter::Images => c.kind() == ClipKind::Image,
                TrayFilter::Links => matches!(c.kind(), ClipKind::Link(_)),
                TrayFilter::Pinned => c.pinned,
            };
            if keep {
                out.push(TrayItem::Clip(i));
            }
        }
        out
    }

    fn tray_tile(&mut self, item: TrayItem, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.tray_selected == Some(ix);
        let base = div()
            .id(("tile", ix))
            .group("tile")
            .relative()
            .h(px(84.0))
            .p(px(10.0))
            .rounded(px(16.0))
            .bg(tk::CARD)
            .border_1()
            .border_color(if selected { tk::TEXT.opacity(0.5) } else { tk::HAIRLINE })
            .overflow_hidden()
            .hover(|s| s.bg(tk::WASH_HOVER).border_color(tk::TEXT.opacity(0.25)))
            .cursor_pointer();
        let actions = |row: Div| div().absolute().top(px(6.0)).right(px(6.0)).flex().gap(px(2.0)).invisible().group_hover("tile", |s| s.visible()).child(row);
        match item {
            TrayItem::File(i) => {
                let Some(path) = self.shelf.get(i).cloned() else { return div().into_any_element() };
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string());
                let size = std::fs::metadata(&path).ok().filter(|m| m.is_file()).map(|m| human_size(m.len()));
                let icon_img = self.icon_for(&path);
                let (p_open, p_send, p_copy) = (path.clone(), path.clone(), path.clone());
                base.cursor_grab()
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, _, _| this.drag_from = Some((i, ev.position))))
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, _| this.drag_from = None))
                    .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                        let Some((ix, from)) = this.drag_from else { return };
                        if ev.pressed_button != Some(MouseButton::Left) {
                            this.drag_from = None;
                            return;
                        }
                        let d = ev.position - from;
                        if f32::from(d.x).abs() + f32::from(d.y).abs() < 6.0 {
                            return;
                        }
                        this.drag_from = None;
                        if let (Some(handle), Some(path)) = (this.win.as_ref().and_then(|w| w.native), this.shelf.get(ix).cloned()) {
                            cx.spawn(async move |_, _| {
                                handle.drag_file(&path);
                            })
                            .detach();
                        }
                    }))
                    .on_click(cx.listener(move |_, ev: &ClickEvent, _, cx| {
                        if ev.click_count() >= 2 {
                            cx.open_with_system(&p_open);
                        }
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(match icon_img {
                                Some(im) => img(im).size(px(28.0)).into_any_element(),
                                None => icon(if path.is_dir() { icons::FOLDER } else { icons::DOCUMENT }).size(px(24.0)).text_color(tk::DIM).into_any_element(),
                            })
                            .child(div().text_size(px(tk::T12)).text_color(tk::TEXT).truncate().child(name))
                            .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(size.unwrap_or_else(|| "Folder".into()))),
                    )
                    .child(actions(
                        div()
                            .flex()
                            .gap(px(2.0))
                            .child(ibtn(("t-send", ix), icons::SPARKLE_AGENT, 24.0, tk::DIM).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.send_to_agent(format!("Here's a file for you: {}", p_send.display()), cx);
                            })))
                            .child(ibtn(("t-copy", ix), icons::COPY, 24.0, tk::DIM).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                cx.write_to_clipboard(ClipboardItem::new_string(p_copy.display().to_string()));
                                this.toast("Path copied");
                                cx.notify();
                            })))
                            .child(ibtn(("t-remove", ix), icons::CLOSE, 24.0, tk::FAINT).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.remove_shelf(i, cx);
                            }))),
                    ))
                    .into_any_element()
            }
            TrayItem::Clip(i) => {
                let Some(clip) = self.clips.get(i).cloned() else { return div().into_any_element() };
                let body: AnyElement = match (&clip.body, clip.kind()) {
                    (ClipBody::Image(im), _) => img(im.clone()).absolute().top_0().left_0().size_full().object_fit(ObjectFit::Cover).into_any_element(),
                    (ClipBody::Text(t), ClipKind::Link(host)) => div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(div().flex().items_center().gap(px(5.0)).child(icon(icons::GLOBAL).size(px(13.0)).text_color(tk::BLUE)).child(div().text_size(px(tk::T12)).font_weight(FontWeight::MEDIUM).text_color(tk::TEXT).truncate().child(host)))
                        .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).line_clamp(2).child(t.trim().to_string()))
                        .into_any_element(),
                    (ClipBody::Text(t), ClipKind::Color(r, g, b)) => div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(div().size(px(34.0)).rounded(px(10.0)).border_1().border_color(tk::HAIRLINE).bg(gpui::Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }))
                        .child(div().font_family(tk::MONO).text_size(px(tk::T12)).text_color(tk::TEXT).child(t.trim().to_string()))
                        .into_any_element(),
                    (ClipBody::Text(t), ClipKind::Code(lang)) => div()
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .child(div().text_size(px(tk::T11)).text_color(tk::BLUE).child(lang))
                        .child(div().font_family(tk::MONO).text_size(px(tk::T11)).text_color(tk::TEXT).line_clamp(3).child(t.trim().to_string()))
                        .into_any_element(),
                    (ClipBody::Text(t), _) => div().text_size(px(tk::T12)).text_color(tk::TEXT).line_clamp(4).child(t.trim().to_string()).into_any_element(),
                };
                let copy = clip.clone();
                let text = clip.as_text().map(str::to_string);
                base.on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                    if ev.click_count() >= 2 {
                        let (t, im) = match &copy.body {
                            ClipBody::Text(t) => (Some(t.clone()), None),
                            ClipBody::Image(im) => (None, Some(im.clone())),
                        };
                        this.paste_back(t, im, cx);
                        return;
                    }
                    cx.write_to_clipboard(match &copy.body {
                        ClipBody::Text(t) => ClipboardItem::new_string(t.clone()),
                        ClipBody::Image(im) => ClipboardItem::new_image(im),
                    });
                    this.tray_selected = Some(ix);
                    this.toast("Copied · Enter pastes");
                    cx.notify();
                }))
                .child(body)
                .when(clip.pinned, |d| d.child(div().absolute().bottom(px(6.0)).right(px(8.0)).child(icon(icons::PIN).size(px(11.0)).text_color(tk::AMBER))))
                .child(actions(
                    div()
                        .flex()
                        .gap(px(2.0))
                        .when_some(text, |d, text| {
                            d.child(ibtn(("c-send", ix), icons::SPARKLE_AGENT, 24.0, tk::DIM).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.send_to_agent(text.clone(), cx);
                            })))
                        })
                        .child(ibtn(("c-pin", ix), icons::PIN, 24.0, if clip.pinned { tk::AMBER } else { tk::DIM }).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            if let Some(c) = this.clips.get_mut(i) {
                                c.pinned = !c.pinned;
                            }
                            this.save();
                            cx.notify();
                        }))),
                ))
                .into_any_element()
            }
        }
    }

    fn page_tray(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let filters = [
            (TrayFilter::All, "All"),
            (TrayFilter::Files, "Files"),
            (TrayFilter::Text, "Text"),
            (TrayFilter::Images, "Images"),
            (TrayFilter::Links, "Links"),
            (TrayFilter::Pinned, "Pinned"),
        ];
        let mut chips = div().flex().items_center().gap(px(5.0));
        for (i, (f, label)) in filters.into_iter().enumerate() {
            chips = chips.child(pill(("filter", i), label, self.tray_filter == f).h(px(24.0)).px(px(9.0)).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.tray_filter = f;
                this.tray_selected = None;
                this.retarget(cx);
                cx.notify();
            })));
        }
        let header = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(chips)
            .child(div().flex_1())
            .child(ibtn("tray-capture", icons::GALLERY, 26.0, tk::DIM).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.capture(false, cx))))
            .when(self.clips.iter().any(|c| !c.pinned), |d| {
                d.child(ibtn("tray-clear", icons::TRASH_BIN_MINIMALISTIC, 26.0, tk::FAINT).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.clear_clips(cx))))
            });
        let items = self.tray_items();
        if items.is_empty() {
            let hint = if self.prefs.clipboard { "Copy something, or drop files on the island" } else { "Drop files on the island · clipboard history is off in Settings" };
            return div()
                .size_full()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(header)
                .child(
                    div()
                        .flex_1()
                        .rounded(px(tk::R_CARD))
                        .border_1()
                        .border_dashed()
                        .border_color(tk::HAIRLINE)
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(8.0))
                        .child(icon(icons::DOCUMENT_ADD).size(px(20.0)).text_color(tk::FAINT))
                        .child(div().text_size(px(tk::T12)).text_color(tk::DIM).child(hint)),
                )
                .into_any_element();
        }
        // A two-column masonry: tiles alternate between columns.
        let (mut left, mut right) = (div().flex_1().min_w_0().flex().flex_col().gap(px(8.0)), div().flex_1().min_w_0().flex().flex_col().gap(px(8.0)));
        let key = self.key();
        for (ix, item) in items.into_iter().enumerate() {
            let tile = rise(self.tray_tile(item, ix, cx), (key, ix), ix);
            if ix % 2 == 0 {
                left = left.child(tile);
            } else {
                right = right.child(tile);
            }
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(header)
            .child(div().id("tray").flex_1().min_h_0().overflow_y_scroll().child(div().flex().gap(px(8.0)).child(left).child(right)))
            .into_any_element()
    }

    fn page_notes(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.selected_note.and_then(|id| self.notes.iter().find(|n| n.id == id).cloned());
        // The list.
        let mut list = div()
            .id("notes")
            .w(px(150.0))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(pill("note-new", "New note", self.editing_note && selected.is_none()).w_full().justify_center().on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.selected_note = None;
                this.editing_note = true;
                if let Some(i) = this.inputs.clone() {
                    i.note.update(cx, |s, cx| {
                        s.set_value("", window, cx);
                        s.focus(window, cx);
                    });
                }
                this.take_focus(cx);
                cx.notify();
            })));
        for (i, n) in self.notes.iter().enumerate() {
            let id = n.id;
            let on = self.selected_note == Some(id);
            list = list.child(
                div()
                    .id(("note", i))
                    .px(px(10.0))
                    .py(px(7.0))
                    .rounded(px(12.0))
                    .cursor_pointer()
                    .when(on, |d| d.bg(tk::WASH_HOVER))
                    .hover(|s| s.bg(tk::WASH))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.selected_note = Some(id);
                        this.editing_note = false;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .when(n.pinned, |d| d.child(icon(icons::PIN).size(px(10.0)).text_color(tk::AMBER)))
                            .child(div().flex_1().min_w_0().truncate().text_size(px(tk::T12)).font_weight(FontWeight::MEDIUM).text_color(tk::TEXT).child(n.title())),
                    )
                    .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(match n.checklist() {
                        Some((d, t)) => format!("{d}/{t} done"),
                        None => ago(n.id),
                    })),
            );
        }
        // The editor or the rendered note.
        let note_input = self.inputs.as_ref().map(|i| i.note.clone());
        let pane: AnyElement = if self.editing_note || selected.is_none() {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .px(px(12.0))
                        .py(px(10.0))
                        .rounded(px(16.0))
                        .bg(tk::WASH)
                        .border_1()
                        .border_color(tk::HAIRLINE)
                        .overflow_hidden()
                        .text_size(px(tk::T12))
                        .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, _, cx| this.take_focus(cx)))
                        .children(note_input.map(|n| Textarea::new(&n))),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(div().flex_1().text_size(px(tk::T11)).text_color(tk::FAINT).child("Ctrl+Enter saves"))
                        .when(self.editing_note && selected.is_some(), |d| {
                            d.child(pill("note-cancel", "Cancel", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.editing_note = false;
                                cx.notify();
                            })))
                        })
                        .child(primary("note-save", "Save").on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.save_note(window, cx)))),
                )
                .into_any_element()
        } else {
            let n = selected.clone().unwrap_or_else(|| super::data::Note::new(String::new()));
            let id = n.id;
            let mut lines = div().flex().flex_col().gap(px(4.0));
            for (li, line) in n.text.lines().enumerate() {
                let t = line.trim_start();
                let check = if t.starts_with("- [ ]") {
                    Some(false)
                } else if t.starts_with("- [x]") {
                    Some(true)
                } else {
                    None
                };
                lines = lines.child(match check {
                    Some(done) => div()
                        .id(("check", li))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(n) = this.notes.iter_mut().find(|n| n.id == id) {
                                n.toggle(li);
                            }
                            this.save();
                            cx.notify();
                        }))
                        .child(
                            div()
                                .size(px(15.0))
                                .rounded(px(5.0))
                                .border_1()
                                .border_color(if done { tk::GREEN } else { tk::DIM })
                                .when(done, |d| d.bg(tk::GREEN))
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(done, |d| d.child(icon(icons::CHECK).size(px(10.0)).text_color(tk::INK))),
                        )
                        .child(div().text_size(px(tk::T12)).text_color(if done { tk::FAINT } else { tk::TEXT }).child(t[5..].trim().to_string()))
                        .into_any_element(),
                    None => rich_text(line),
                });
            }
            let text = n.text.clone();
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(div().id("note-view").flex_1().min_h_0().overflow_y_scroll().p(px(12.0)).rounded(px(16.0)).bg(tk::CARD).child(lines))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .child(ibtn("note-pin", icons::PIN, 28.0, if n.pinned { tk::AMBER } else { tk::DIM }).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            for note in &mut this.notes {
                                note.pinned = note.id == id && !note.pinned;
                            }
                            this.save();
                            this.retarget(cx);
                            cx.notify();
                        })))
                        .child(ibtn("note-delete", icons::TRASH_BIN_MINIMALISTIC, 28.0, tk::FAINT).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.delete_note(id, cx))))
                        .child(div().flex_1())
                        .child(pill("note-send", "Send as prompt", false).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.send_to_agent(text.clone(), cx))))
                        .child(primary("note-edit", "Edit").on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.editing_note = true;
                            if let (Some(i), Some(n)) = (this.inputs.clone(), this.notes.iter().find(|n| n.id == id)) {
                                let t = n.text.clone();
                                i.note.update(cx, |s, cx| {
                                    s.set_value(t, window, cx);
                                    s.focus(window, cx);
                                });
                            }
                            this.take_focus(cx);
                            cx.notify();
                        }))),
                )
                .into_any_element()
        };
        div().size_full().flex().gap(px(10.0)).child(list).child(pane).into_any_element()
    }

    fn page_timer(&self, cx: &mut Context<Self>) -> AnyElement {
        let running = self.timer.running();
        let remaining = self.timer.remaining();
        let last = running && remaining.as_secs() < 10;
        let pulse = if last { 0.6 + 0.4 * breathe(self.t() * 2.0) } else { 1.0 };
        let presets = [1u64, 5, 10, 15, 25, 45, 60];
        let mut chips = div().flex().justify_center().gap(px(6.0));
        for (i, m) in presets.iter().enumerate() {
            let m = *m;
            let on = self.timer.total == Duration::from_secs(m * 60);
            chips = chips.child(pill(("preset", i), if m == 60 { "1h".into() } else { format!("{m}m") }, on).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.timer.set(Duration::from_secs(m * 60));
                this.retarget(cx);
                cx.notify();
            })));
        }
        let linked = self.timer_link.and_then(|id| self.snap.agents.iter().find(|a| a.id == id)).map(|a| a.title.clone());
        let candidate = self.snap.agents.iter().find(|a| a.status == TaskStatus::Running).map(|a| (a.id, a.kind.label()));
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.0))
            .child(
                div()
                    .id("timer-face")
                    .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                        if this.timer.running() {
                            return;
                        }
                        let dy = match ev.delta {
                            ScrollDelta::Pixels(p) => f32::from(p.y),
                            ScrollDelta::Lines(l) => l.y * 20.0,
                        };
                        if dy.abs() > 1.0 {
                            this.timer.nudge(if dy > 0.0 { 1 } else { -1 });
                            cx.notify();
                        }
                    }))
                    .child(progress_ring(
                        if self.timer.active() { 1.0 - self.timer.progress() } else { 1.0 },
                        120.0,
                        4.0,
                        tk::AMBER.opacity(pulse),
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(div().text_size(px(30.0)).font_weight(FontWeight::SEMIBOLD).text_color(if running { tk::TEXT } else { tk::DIM }).opacity(pulse).child(mins_secs(remaining)))
                            .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(if running { "running" } else { "scroll to set" }))
                            .into_any_element(),
                    )),
            )
            .child(chips)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(primary("timer-go", if running { "Pause" } else if self.timer.active() { "Resume" } else { "Start" }).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.timer.toggle();
                        this.retarget(cx);
                        cx.notify();
                    })))
                    .child(pill("timer-reset", "Reset", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.timer.reset();
                        this.timer_link = None;
                        this.retarget(cx);
                        cx.notify();
                    })))
                    .map(|d| match (&linked, candidate) {
                        (Some(title), _) => d.child(pill("timer-link", format!("Until “{title}” finishes"), true).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.timer_link = None;
                            cx.notify();
                        }))),
                        (None, Some((id, label))) => d.child(pill("timer-link", format!("Link to {label}"), false).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.timer_link = Some(id);
                            this.toast("Ends when the agent finishes");
                            cx.notify();
                        }))),
                        _ => d,
                    }),
            )
            .into_any_element()
    }

    fn page_alerts(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.notices.is_empty() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .child(icon(icons::BELL).size(px(22.0)).text_color(tk::FAINT))
                .child(div().text_size(px(tk::T13)).text_color(tk::DIM).child("Nothing new"))
                .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child("Finishes, failures and the timer show up here."))
                .into_any_element();
        }
        let key = self.key();
        let mut col = div()
            .id("alerts")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(px(8.0))
                    .pb(px(4.0))
                    .child(section(format!("{} notifications", self.notices.len())))
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("alerts-clear")
                            .text_size(px(tk::T11))
                            .text_color(tk::FAINT)
                            .cursor_pointer()
                            .hover(|s| s.text_color(tk::TEXT))
                            .child("Clear all")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.notices.clear();
                                this.retarget(cx);
                                cx.notify();
                            })),
                    ),
            );
        for (i, n) in self.notices.iter().enumerate() {
            let (glyph, color): (AnyElement, Hsla) = match (n.kind, n.agent) {
                (k, Some(kind)) => (avatar(kind, 26.0, None), if k == FlashKind::Failed { tk::RED } else { tk::GREEN }),
                (FlashKind::Timer, None) => (icon(icons::TIMER).size(px(15.0)).text_color(tk::AMBER).into_any_element(), tk::AMBER),
                (FlashKind::Failed, None) => (icon(icons::DANGER_TRIANGLE).size(px(15.0)).text_color(tk::RED).into_any_element(), tk::RED),
                _ => (icon(icons::CHECK).size(px(15.0)).text_color(tk::GREEN).into_any_element(), tk::GREEN),
            };
            let task = n.task;
            col = col.child(rise(
                div()
                    .id(("notice", i))
                    .h(px(44.0))
                    .px(px(8.0))
                    .rounded(px(tk::R_ROW))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .when(task.is_some(), |d| d.cursor_pointer().hover(|s| s.bg(tk::WASH)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        if let Some(id) = task {
                            this.open_page(Page::Agent(id), Some(window), cx);
                        }
                    }))
                    .child(glyph)
                    .child(div().flex_1().min_w_0().truncate().text_size(px(tk::T12)).text_color(tk::TEXT).child(n.text.clone()))
                    .child(self.dot(color, 6.0, false))
                    .child(div().text_size(px(tk::T11)).text_color(tk::GHOST).child(ago(n.at_ms))),
                (key, i),
                i,
            ));
        }
        col.into_any_element()
    }

    fn page_dev(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.ports.is_empty() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .child(icon(icons::TERMINAL).size(px(22.0)).text_color(tk::FAINT))
                .child(div().text_size(px(tk::T13)).text_color(tk::DIM).child("No dev servers running"))
                .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child("Servers your agents start (Vite, Next, Django…) show up here."))
                .into_any_element();
        }
        let mut col = div().id("dev").size_full().overflow_y_scroll().flex().flex_col().gap(px(2.0));
        for (i, p) in self.ports.iter().enumerate() {
            let url = format!("http://localhost:{}", p.port);
            let (u_open, u_copy, pid) = (url.clone(), url.clone(), p.pid);
            col = col.child(
                div()
                    .id(("port", i))
                    .h(px(48.0))
                    .px(px(10.0))
                    .rounded(px(tk::R_ROW))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .hover(|s| s.bg(tk::WASH))
                    .child(self.dot(tk::GREEN, 7.0, false))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().font_family(tk::MONO).text_size(px(tk::T13)).text_color(tk::TEXT).child(format!("localhost:{}", p.port)))
                            .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(p.process.clone())),
                    )
                    .child(ibtn(("p-open", i), icons::ARROW_UP_RIGHT, 28.0, tk::DIM).on_click(cx.listener(move |_, _: &ClickEvent, _, cx| cx.open_url(&u_open))))
                    .child(ibtn(("p-copy", i), icons::COPY, 28.0, tk::DIM).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(u_copy.clone()));
                        this.toast("URL copied");
                        cx.notify();
                    })))
                    .child(ibtn(("p-stop", i), icons::STOP, 28.0, tk::RED).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if super::win::stop_process(pid) {
                            this.ports.retain(|p| p.pid != pid);
                            this.toast("Stopped");
                        } else {
                            this.toast("Couldn't stop it");
                        }
                        this.retarget(cx);
                        cx.notify();
                    }))),
            );
        }
        col.into_any_element()
    }

    // ----- the canvas -------------------------------------------------------------

    /// The glow over the glass: amber while something needs you, green or red
    /// for a moment after a finish, else the album art's colour while music
    /// plays.
    fn glow(&self) -> Option<Hsla> {
        if self.snap.agents.iter().any(|a| a.needs_you()) {
            return Some(tk::AMBER.opacity(tk::GLOW * (0.6 + 0.4 * breathe(self.t()))));
        }
        if let Some(f) = &self.flash {
            return Some(match f.kind {
                FlashKind::Failed => tk::RED,
                FlashKind::Timer => tk::AMBER,
                FlashKind::Done => tk::GREEN,
            }
            .opacity(tk::GLOW));
        }
        let m = self.media.as_ref().filter(|m| m.playing)?;
        let (r, g, b) = m.tint?;
        let c: Hsla = gpui::Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }.into();
        Some(c.opacity(tk::GLOW * 1.4))
    }

    pub(super) fn render_canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        for image in self.retired.drain(..) {
            let _ = window.drop_image(image);
        }
        let now = Instant::now();
        let moving = self.springs.tick(now);
        if moving {
            window.request_animation_frame();
        }
        let g = Geo::from_array(self.springs.values());
        self.apply_shapes(g.shapes(), cx);
        if !moving && self.machine.state == State::Hidden && self.lift_at.is_none() && self.shown {
            self.settled_hidden(cx);
        }
        if self.focus_command
            && let Some(i) = self.inputs.clone()
        {
            self.focus_command = false;
            i.command.update(cx, |s, cx| s.focus(window, cx));
        }
        let state = self.machine.state;
        // Content is laid out at its final size, centered in the island and
        // clipped by it, so text never reflows while the shape springs.
        let target = self.pending_target.unwrap_or_else(|| self.target());
        let (tw, th) = (target.w, target.h);
        let reveal = if state == State::Hidden || self.dot_until.is_some() {
            0.0
        } else {
            (((g.w / tw).min(g.h / th) - 0.55) / 0.45).clamp(0.0, 1.0)
        };
        let content = match state {
            State::Hidden => div().into_any_element(),
            State::Slim => self.render_slim(cx),
            State::Alert => self.render_alert(cx),
            State::Expanded => self.render_expanded(window, cx),
        };
        let composition = self.win.as_ref().is_some_and(|w| w.glass.is_some());
        let frosted = self.prefs.glass && self.transparency && (composition || self.backdrop.is_some());
        let (top, bottom) = if frosted { (tk::TINT_TOP, tk::TINT_BOTTOM) } else { (tk::SOLID_TOP, tk::SOLID_BOTTOM) };
        let backdrop = self.backdrop.clone().filter(|_| frosted && !composition);
        let glow = self.glow();
        let needs = self.snap.agents.iter().any(|a| a.needs_you()) && state != State::Expanded;
        let border = if needs { tk::AMBER.opacity(0.35 + 0.35 * breathe(self.t())) } else { tk::HAIRLINE };
        let glass = move |d: Div, left: f32, y: f32, r: f32| {
            d.border_1()
                .border_color(border)
                .when_some(backdrop.clone(), |d, image| {
                    d.child(
                        img(image)
                            .absolute()
                            .left(px(-left - 1.0))
                            .top(px(-y - 1.0))
                            .w(px(tk::CANVAS_W))
                            .h(px(tk::CANVAS_H))
                            .object_fit(ObjectFit::Fill),
                    )
                })
                .child(div().absolute().top_0().left_0().size_full().bg(linear_gradient(180.0, linear_color_stop(top, 0.0), linear_color_stop(bottom, 1.0))))
                .when_some(glow, |d, c| {
                    d.child(div().absolute().top_0().left_0().size_full().bg(linear_gradient(180.0, linear_color_stop(c, 0.0), linear_color_stop(c.opacity(0.0), 0.7))))
                })
                // The specular line along the top edge.
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(px(r * 0.7))
                        .right(px(r * 0.7))
                        .h(px(1.0))
                        .bg(linear_gradient(90.0, linear_color_stop(tk::SPECULAR.opacity(0.0), 0.0), linear_color_stop(tk::SPECULAR, 0.5))),
                )
        };
        let r = g.r.min(g.h / 2.0);
        let this = cx.entity().downgrade();
        let island = glass(div(), g.x(), g.y, r)
            .id("island")
            .absolute()
            .left(px(g.x()))
            .top(px(g.y))
            .w(px(g.w))
            .h(px(g.h))
            .rounded(px(r))
            .overflow_hidden()
            .text_color(tk::TEXT)
            .on_hover(move |hovered, _, cx| {
                let _ = this.update(cx, |n, cx| {
                    n.hovered = *hovered;
                    if !hovered {
                        n.pressed = false;
                    }
                    n.retarget(cx);
                });
            })
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.pressed = true;
                this.retarget(cx);
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, cx| {
                this.pressed = false;
                this.retarget(cx);
            }))
            .on_mouse_down(MouseButton::Middle, cx.listener(|this, _: &MouseDownEvent, _, cx| {
                if this.media.is_some() {
                    this.toggle_play(cx);
                }
            }))
            .when(state == State::Slim, |d| {
                d.cursor_pointer()
                    .on_mouse_down(MouseButton::Right, cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        this.context_menu = !this.context_menu;
                        this.retarget(cx);
                        cx.notify();
                    }))
                    .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                        if this.context_menu || matches!(ev, ClickEvent::Mouse(m) if m.up.button != MouseButton::Left) {
                            return;
                        }
                        if this.machine.open().is_some() {
                            this.transition(cx);
                        }
                    }))
            })
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                let dy = match ev.delta {
                    ScrollDelta::Pixels(p) => f32::from(p.y),
                    ScrollDelta::Lines(l) => l.y * 20.0,
                };
                if dy.abs() < 1.0 {
                    return;
                }
                match this.machine.state {
                    State::Slim => {
                        this.primary = this.primary.wrapping_add(if dy < 0.0 { 1 } else { usize::MAX });
                        this.retarget(cx);
                        cx.notify();
                    }
                    State::Expanded if matches!(this.page, Page::Tab(Tab::Music)) => {
                        this.audio.nudge(if dy > 0.0 { 0.04 } else { -0.04 });
                        this.volume_shown_until = Some(Instant::now() + Duration::from_millis(1200));
                        cx.notify();
                    }
                    _ => {}
                }
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| this.add_paths(paths, cx)))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.border_color(tk::TEXT.opacity(0.6)).bg(tk::WASH))
            .child(div().absolute().top_0().left(px((g.w - tw) / 2.0)).w(px(tw)).h(px(th)).opacity(reveal).child(content));
        let shapes = g.shapes();
        let mut bubbles: Vec<AnyElement> = Vec::new();
        for (i, s) in shapes.iter().skip(1).enumerate() {
            let b = glass(div(), s.x, s.y, s.r)
                .id(("bubble", i))
                .absolute()
                .left(px(s.x))
                .top(px(s.y))
                .size(px(s.w))
                .rounded_full()
                .overflow_hidden()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    let tab = match this.slim_items().get(i + 1) {
                        Some(SlimItem::Music) => Tab::Music,
                        Some(SlimItem::Timer) => Tab::Timer,
                        Some(SlimItem::Note(_)) => Tab::Notes,
                        _ => Tab::Agents,
                    };
                    this.open_page(Page::Tab(tab), Some(window), cx);
                }))
                .child(div().absolute().top_0().left_0().size_full().opacity(reveal).child(self.render_bubble(i)));
            bubbles.push(b.into_any_element());
        }
        div()
            .id("notch-canvas")
            .w(px(tk::CANVAS_W))
            .h(px(tk::CANVAS_H))
            .relative()
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| this.on_key(ev, window, cx)))
            .child(island)
            .children(bubbles)
            .into_any_element()
    }

    /// Keys while the island has the keyboard.
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        let typing = self.inputs.as_ref().is_some_and(|i| {
            i.message.read(cx).focus_handle(cx).is_focused(window)
                || i.prompt.read(cx).focus_handle(cx).is_focused(window)
                || i.note.read(cx).focus_handle(cx).is_focused(window)
        });
        let in_command = self.inputs.as_ref().is_some_and(|i| i.command.read(cx).focus_handle(cx).is_focused(window));
        match key {
            "escape" => {
                if !self.query.is_empty() {
                    self.query.clear();
                    if let Some(i) = self.inputs.clone() {
                        i.command.update(cx, |s, cx| s.set_value("", window, cx));
                    }
                    self.retarget(cx);
                } else if self.machine.close(self.rest(), self.alerting()).is_some() {
                    self.transition(cx);
                }
                cx.notify();
            }
            "down" | "up" if !self.query.is_empty() => {
                let n = self.hits().len().max(1);
                self.hit = if key == "down" { (self.hit + 1) % n } else { (self.hit + n - 1) % n };
                cx.notify();
            }
            "down" | "up" | "left" | "right" if self.page == Page::Tab(Tab::Tray) && !typing && !in_command => {
                let n = self.tray_items().len();
                if n > 0 {
                    let cur = self.tray_selected.unwrap_or(0);
                    let step: isize = match key {
                        "down" => 2,
                        "up" => -2,
                        "right" => 1,
                        _ => -1,
                    };
                    self.tray_selected = Some(((cur as isize + step).rem_euclid(n as isize)) as usize);
                    cx.notify();
                }
            }
            "enter" if self.page == Page::Tab(Tab::Tray) && !typing && !in_command => {
                if let Some(ix) = self.tray_selected
                    && let Some(TrayItem::Clip(i)) = self.tray_items().get(ix).cloned()
                    && let Some(c) = self.clips.get(i).cloned()
                {
                    let (t, im) = match c.body {
                        ClipBody::Text(t) => (Some(t), None),
                        ClipBody::Image(im) => (None, Some(im)),
                    };
                    self.paste_back(t, im, cx);
                }
            }
            "a" | "d" if !typing && !in_command => {
                if let Some((agent, req)) = self.approval() {
                    let (id, rid, safe) = (agent.id, req.request_id.clone(), req.safe && !req.question);
                    if key == "d" {
                        self.deny(id, rid, cx);
                    } else if safe {
                        self.command(ToEngine::Decide { task: id, request_id: rid, decision: PermissionDecision::AllowOnce });
                        self.toast("Allowed");
                    }
                    cx.notify();
                }
            }
            k if !typing && !in_command && k.len() == 1 && k.as_bytes()[0].is_ascii_digit() => {
                let n = (k.as_bytes()[0] - b'0') as usize;
                if let Some(t) = n.checked_sub(1).and_then(|i| self.prefs.tabs().get(i).copied()) {
                    self.open_page(Page::Tab(t), Some(window), cx);
                }
            }
            _ => {}
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

/// A click that answers an approval the way the transcript's card does.
fn decide(
    id: TaskId,
    decision: PermissionDecision,
    rid: String,
    cx: &mut Context<Notch>,
) -> impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static {
    cx.listener(move |this: &mut Notch, _: &ClickEvent, _, cx| {
        this.command(ToEngine::Decide { task: id, request_id: rid.clone(), decision });
        this.toast("Allowed");
        cx.notify();
    })
}

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
    }
}
