//! Small shared components on top of `ui.rs`: keyboard hints, ring meters,
//! switches, checkboxes, status glyphs, empty states, the toaster (Zeron's)
//! and resize handles.
//!
//! Anything animated requests frames only while it is on screen and moving,
//! so an idle window still renders nothing.

pub mod controls;
pub mod page;
pub mod popover;

use std::time::{Duration, Instant};

use dove_core::proto::TaskStatus;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, Div, ElementId, Entity, Global, Hsla, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, PathBuilder, Pixels, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, point, px,
};

use crate::assets::{icon, icons};
use crate::design::{radius, ty};
use crate::theme::Theme;
use crate::ui;

/// A keyboard hint: `⏎`, `A`, `Ctrl K`.
pub fn kbd(key: &str, t: &Theme) -> Div {
    div()
        .h(px(17.0))
        .min_w(px(17.0))
        .px(px(4.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .border_1()
        .border_color(t.border)
        .bg(t.wash(0.05))
        .text_size(ty::BADGE)
        .font_family(t.font_mono.clone())
        .text_color(t.text_faint)
        .child(key.to_string())
}

/// Ring color by fill: calm until 75%, warning to 90%, then danger.
pub fn ring_color(fraction: f32, t: &Theme) -> Hsla {
    if fraction >= 0.9 {
        t.danger
    } else if fraction >= 0.75 {
        t.warning
    } else {
        t.text_muted
    }
}

/// A circular meter, `size` px, filled clockwise from 12 o'clock.
pub fn ring(fraction: f32, size: f32, color: Hsla, track: Hsla) -> impl IntoElement {
    let fraction = fraction.clamp(0.0, 1.0);
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window: &mut Window, _| {
            let stroke = px(2.4);
            let c = bounds.center();
            let r = (bounds.size.width.min(bounds.size.height) - stroke) / 2.0;
            let arc = |from: f32, to: f32| -> Option<gpui::Path<Pixels>> {
                if to - from <= 0.0001 {
                    return None;
                }
                let mut b = PathBuilder::stroke(stroke);
                let steps = ((to - from) * 64.0).ceil().max(2.0) as usize;
                for i in 0..=steps {
                    let a = (from + (to - from) * i as f32 / steps as f32) * std::f32::consts::TAU
                        - std::f32::consts::FRAC_PI_2;
                    let p = point(c.x + r * a.cos(), c.y + r * a.sin());
                    if i == 0 {
                        b.move_to(p);
                    } else {
                        b.line_to(p);
                    }
                }
                b.build().ok()
            };
            if let Some(p) = arc(0.0, 1.0) {
                window.paint_path(p, track);
            }
            if let Some(p) = arc(0.0, fraction) {
                window.paint_path(p, color);
            }
        },
    )
    .size(px(size))
    .flex_none()
}

/// An on/off switch.
pub fn switch(id: impl Into<ElementId>, on: bool, t: &Theme) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .role(gpui::Role::Switch)
        .aria_toggled(if on { gpui::Toggled::True } else { gpui::Toggled::False })
        .w(px(30.0))
        .h(px(18.0))
        .p(px(2.0))
        .flex()
        .flex_none()
        .items_center()
        .when(on, |d| d.justify_end())
        .rounded_full()
        .cursor_pointer()
        .bg(if on { t.accent } else { t.wash(0.16) })
        .child(div().size(px(14.0)).rounded_full().bg(if on { t.on_solid } else { t.text_muted }))
}

/// A square checkbox with an optional label.
pub fn checkbox(id: impl Into<ElementId>, on: bool, label: impl Into<SharedString>, t: &Theme) -> gpui::Stateful<Div> {
    let label: SharedString = label.into();
    div()
        .id(id)
        .role(gpui::Role::CheckBox)
        .aria_toggled(if on { gpui::Toggled::True } else { gpui::Toggled::False })
        .aria_label(label.clone())
        .flex()
        .items_center()
        .gap(px(6.0))
        .cursor_pointer()
        .text_size(ty::BODY)
        .text_color(t.text_muted)
        .hover(|s| s.text_color(t.text))
        .child(
            div()
                .size(px(14.0))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .border_1()
                .border_color(if on { t.accent } else { t.border_strong })
                .when(on, |d| d.bg(t.accent))
                .when(on, |d| d.child(icon(icons::CHECK).size(px(10.0)).text_color(t.on_solid))),
        )
        .child(label)
}

/// A thread's status: spinner (working), amber dot (needs you), check,
/// red cross, or a hollow dot.
pub fn status_glyph(id: impl Into<ElementId>, status: TaskStatus, unread: bool, t: &Theme) -> AnyElement {
    match status {
        TaskStatus::Running => spinner(id, 12.0, t.busy).into_any_element(),
        TaskStatus::NeedsInput => ui::dot(t.warning, 8.0).into_any_element(),
        TaskStatus::Done if unread => ui::dot(t.accent, 8.0).into_any_element(),
        TaskStatus::Done => icon(icons::CHECK).size(px(12.0)).text_color(t.success).into_any_element(),
        TaskStatus::Failed => icon(icons::CLOSE).size(px(12.0)).text_color(t.danger).into_any_element(),
        TaskStatus::Draft => icon(icons::PEN).size(px(11.0)).text_color(t.text_faint).into_any_element(),
        TaskStatus::Idle | TaskStatus::Archived => {
            div().size(px(8.0)).rounded_full().border_1().border_color(t.text_faint).into_any_element()
        }
    }
}

/// The sidebar's status mark: only what asks for attention (working,
/// waiting on you, an unread result, a failure, a draft). A thread that is
/// done and read, or idle, shows nothing, so a calm list stays calm.
pub fn status_mark(id: impl Into<ElementId>, status: TaskStatus, unread: bool, t: &Theme) -> Option<AnyElement> {
    match status {
        TaskStatus::Running => Some(spinner(id, 12.0, t.busy).into_any_element()),
        TaskStatus::NeedsInput => Some(ui::dot(t.warning, 7.0).into_any_element()),
        TaskStatus::Done if unread => Some(ui::dot(t.accent, 7.0).into_any_element()),
        TaskStatus::Failed => Some(icon(icons::CLOSE).size(px(11.0)).text_color(t.danger).into_any_element()),
        TaskStatus::Draft => Some(icon(icons::PEN).size(px(11.0)).text_color(t.text_faint).into_any_element()),
        TaskStatus::Done | TaskStatus::Idle | TaskStatus::Archived => None,
    }
}

/// The working indicator: a faint ring with a bright arc turning once per
/// [`crate::motion::SPIN_PERIOD`], drawn as two strokes. It runs on the
/// shared 30 Hz pulse clock (see `motion::pulse`), so a spinner re-renders
/// only its own view's subtree and stops scheduling frames the moment it
/// unmounts. Under reduced motion the arc holds still and breathes instead.
pub fn spinner(_id: impl Into<ElementId>, size: f32, color: Hsla) -> impl IntoElement {
    let ring = crate::motion::pulse(crate::motion::SPIN_PERIOD, 1, move |phase| {
        gpui::canvas(|_, _, _| {}, move |bounds, _, window, _| paint_spinner(bounds, color, phase, window))
            .size(px(size))
            .into_any_element()
    });
    div().size(px(size)).flex().flex_none().items_center().justify_center().child(ring)
}

/// One frame of [`spinner`]: the track, then the arc (a quarter turn with a
/// round head), as polylines fine enough to read as circles at any size.
fn paint_spinner(
    bounds: gpui::Bounds<gpui::Pixels>,
    color: Hsla,
    phase: crate::motion::ActivityPhase,
    window: &mut gpui::Window,
) {
    use std::f32::consts::TAU;
    let side = f32::from(bounds.size.width.min(bounds.size.height));
    let stroke = (side / 8.0).clamp(1.25, 2.0);
    let r = side / 2.0 - stroke / 2.0 - 0.25;
    let (cx, cy) = (
        f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.0,
        f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0,
    );
    let at = |a: f32| gpui::point(px(cx + r * a.cos()), px(cy + r * a.sin()));
    let arc = |from: f32, sweep: f32, fill: Hsla, window: &mut gpui::Window| {
        let steps = 40;
        let mut b = gpui::PathBuilder::stroke(px(stroke));
        b.move_to(at(from));
        for i in 1..=steps {
            b.line_to(at(from + sweep * i as f32 / steps as f32));
        }
        if let Ok(path) = b.build() {
            window.paint_path(path, fill);
        }
    };
    arc(0.0, TAU, color.opacity(color.a * 0.2), window);
    // Moving: the arc turns. Reduced motion: it stays at the top and
    // breathes, so work still reads as ongoing without anything moving.
    let (start, alpha) = if phase.still || phase.subtle {
        (-TAU / 4.0, if phase.still { 1.0 } else { 0.45 + 0.55 * crate::motion::pulse_wave(phase.phase) })
    } else {
        (phase.phase * TAU - TAU / 4.0, 1.0)
    };
    arc(start, TAU * 0.28, color.opacity(color.a * alpha), window);
}

/// A centered message for an empty page or list.
pub fn empty_state(glyph: &'static str, title: &str, body: &str, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .py(px(40.0))
        .child(
            div()
                .size(px(44.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(radius::RADIUS))
                .bg(t.wash(0.05))
                .child(icon(glyph).size(px(20.0)).text_color(t.text_faint)),
        )
        .child(
            div().text_size(ty::MD).font_weight(gpui::FontWeight::MEDIUM).text_color(t.text).child(title.to_string()),
        )
        .child(
            div().max_w(px(360.0)).text_center().text_size(ty::BODY).text_color(t.text_faint).child(body.to_string()),
        )
}

/// A small pill label.
pub fn badge(label: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .h(px(18.0))
        .px(px(6.0))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(radius::fit(18.0)))
        .bg(color.opacity(0.12))
        .text_color(color)
        .text_size(ty::MICRO)
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(label.into())
}

// ------------------------------------------------------------- page parts

/// A quiet plate that groups rows (settings, detail panes, lists).
pub fn group(t: &Theme) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .rounded(px(radius::RADIUS))
        .border_1()
        .border_color(t.border)
        .bg(t.wash(0.022))
        .overflow_hidden()
}

/// A rounded tile with a letter or emoji on a tinted ground: teammates,
/// projects, people.
pub fn avatar_tile(label: impl Into<SharedString>, hue: f32, size: f32, t: &Theme) -> Div {
    let (bg, fg) = if t.appearance.is_dark() {
        (gpui::hsla(hue, 0.42, 0.42, 0.30), gpui::hsla(hue, 0.70, 0.78, 1.0))
    } else {
        (gpui::hsla(hue, 0.60, 0.55, 0.18), gpui::hsla(hue, 0.60, 0.34, 1.0))
    };
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(if size < 16.0 { (size * 0.28).max(4.0) } else { radius::RADIUS }))
        .bg(bg)
        .text_color(fg)
        .text_size(px(size * 0.52))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(label.into())
}

/// A vertical drag strip on a panel edge. `on_drag` gets the pointer x.
pub fn resize_handle(
    id: impl Into<ElementId>,
    t: &Theme,
    on_start: impl Fn(Pixels, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<Div> {
    let hover = t.accent.opacity(0.4);
    div()
        .id(id)
        .w(px(6.0))
        .h_full()
        .flex_none()
        .cursor_col_resize()
        .flex()
        .justify_center()
        .group("resize")
        .child(div().w(px(1.0)).h_full().group_hover("resize", move |s| s.bg(hover)))
        .on_mouse_down(MouseButton::Left, move |e, window, cx| {
            cx.stop_propagation();
            on_start(e.position.x, window, cx)
        })
}

// ------------------------------------------------------------------ toasts
//
// Zeron's toaster (sonner's behaviour, MIT): toasts live four seconds, the
// countdown pausing while the pointer is over the stack or the window is
// inactive. Collapsed, the newest sits in front and up to two older ones
// peek above it; hovering fans them out into a list. New toasts rise in,
// dismissed ones fade, and a toast swiped right past a threshold leaves.
// Only opacity and offsets move: every toast keeps its natural size.

/// How long a toast stays up (paused while hovered or the window is idle).
pub const TOAST_LIFETIME: Duration = Duration::from_millis(4000);
/// Toasts drawn in the collapsed stack; older ones wait unseen behind.
const TOAST_VISIBLE: usize = 3;
/// Peek step when collapsed and the gap when fanned out.
const TOAST_GAP: f32 = 14.0;
const TOAST_WIDTH: f32 = 356.0;
/// Inset from the window's bottom-right corner.
const TOAST_OFFSET: f32 = 20.0;
/// Stack moves and the entrance rise.
const TOAST_MOVE: Duration = Duration::from_millis(400);
/// The exit fade.
const TOAST_EXIT: Duration = Duration::from_millis(200);
/// A swipe this far right dismisses; shorter ones spring back.
const SWIPE_THRESHOLD: f32 = 45.0;
/// Pointer travel under which a press counts as a click.
const CLICK_SLOP: f32 = 4.0;
/// Height assumed until a toast has been measured.
const TOAST_DEFAULT_H: f32 = 56.0;
/// Sonner's `cubic-bezier(.215, .61, .355, 1)`.
const TOAST_EASE: crate::motion::CubicBezier = crate::motion::CubicBezier::new(0.215, 0.61, 0.355, 1.0);

/// A value easing toward its target, restarting from wherever it is drawn
/// whenever the target moves.
#[derive(Clone, Copy)]
struct Glide {
    from: f32,
    to: f32,
    started: Instant,
}

impl Glide {
    fn new(value: f32, now: Instant) -> Self {
        Self { from: value, to: value, started: now }
    }

    fn progress(&self, now: Instant) -> f32 {
        let total = TOAST_MOVE.mul_f32(crate::motion::speed_scale());
        TOAST_EASE.eval(now.saturating_duration_since(self.started).as_secs_f32() / total.as_secs_f32())
    }

    fn value(&self, now: Instant) -> f32 {
        crate::motion::lerp(self.from, self.to, self.progress(now))
    }

    fn retarget(&mut self, target: f32, now: Instant, snap: bool) {
        if snap {
            *self = Self::new(target, now);
        } else if (self.to - target).abs() > 0.01 {
            *self = Self { from: self.value(now), to: target, started: now };
        }
    }

    fn moving(&self, now: Instant) -> bool {
        self.progress(now) < 1.0
    }
}

/// One toast and its place in the stack.
#[derive(Clone)]
pub struct Toast {
    pub id: u64,
    pub title: SharedString,
    pub body: Option<SharedString>,
    pub tone: Tone,
    /// Clicking the toast opens this thread, when set.
    pub thread: Option<dove_core::proto::TaskId>,
    created: Instant,
    /// Countdown left, and when it last resumed (`None` while paused).
    remaining: Duration,
    resumed: Option<Instant>,
    closing: Option<Instant>,
    bottom: Option<Glide>,
    swipe: f32,
    /// Where a press started, while the pointer is down on this toast.
    press_x: Option<f32>,
}

impl Toast {
    fn expired(&self, now: Instant) -> bool {
        self.resumed.is_some_and(|since| now.saturating_duration_since(since) >= self.remaining)
    }

    fn pause(&mut self, now: Instant) {
        if let Some(since) = self.resumed.take() {
            self.remaining = self.remaining.saturating_sub(now.saturating_duration_since(since));
        }
    }

    fn resume(&mut self, now: Instant) {
        if self.resumed.is_none() {
            self.resumed = Some(now);
        }
    }

    fn deadline(&self, exit: Duration) -> Option<Instant> {
        match (self.closing, self.resumed) {
            (Some(at), _) => Some(at + exit),
            (None, Some(since)) => Some(since + self.remaining),
            (None, None) => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Success,
    Warning,
    Danger,
}

/// Opens the thread a toast is about.
pub type OpenTask = std::rc::Rc<dyn Fn(dove_core::proto::TaskId, &mut Window, &mut App)>;

type Heights = std::rc::Rc<std::cell::RefCell<std::collections::HashMap<u64, f32>>>;

/// The app's toast stack, a global entity the window root renders.
pub struct Toasts {
    items: Vec<Toast>,
    next: u64,
    hovered: bool,
    expand: Option<Glide>,
    /// Natural heights, measured as each toast paints.
    heights: Heights,
    /// The single wake-up for the next expiry, replaced as deadlines move.
    wake: Option<(Instant, gpui::Task<()>)>,
    pub on_open: Option<OpenTask>,
}

struct ToastsGlobal(Entity<Toasts>);
impl Global for ToastsGlobal {}

impl Toasts {
    fn empty() -> Self {
        Toasts {
            items: Vec::new(),
            next: 1,
            hovered: false,
            expand: None,
            heights: Heights::default(),
            wake: None,
            on_open: None,
        }
    }

    pub fn init(cx: &mut App) -> Entity<Toasts> {
        let e = cx.new(|_| Toasts::empty());
        cx.set_global(ToastsGlobal(e.clone()));
        e
    }

    /// Show a toast for [`TOAST_LIFETIME`]. A repeat of the live front toast
    /// restarts its countdown instead of stacking a copy.
    pub fn push(
        cx: &mut App,
        title: impl Into<SharedString>,
        body: Option<String>,
        tone: Tone,
        thread: Option<dove_core::proto::TaskId>,
    ) {
        let Some(e) = cx.try_global::<ToastsGlobal>().map(|g| g.0.clone()) else { return };
        let (title, body) = (title.into(), body.map(SharedString::from));
        e.update(cx, |t, cx| {
            t.add(title, body, tone, thread, Instant::now());
            cx.notify();
        });
    }

    fn add(
        &mut self,
        title: SharedString,
        body: Option<SharedString>,
        tone: Tone,
        thread: Option<dove_core::proto::TaskId>,
        now: Instant,
    ) {
        let hovered = self.hovered;
        if let Some(front) = self.items.iter_mut().rev().find(|x| x.closing.is_none())
            && front.tone == tone
            && front.title == title
            && front.body == body
            && front.thread == thread
        {
            front.remaining = TOAST_LIFETIME;
            front.resumed = (!hovered).then_some(now);
            return;
        }
        let id = self.next;
        self.next += 1;
        self.items.push(Toast {
            id,
            title,
            body,
            tone,
            thread,
            created: now,
            remaining: TOAST_LIFETIME,
            resumed: (!hovered).then_some(now),
            closing: None,
            bottom: None,
            swipe: 0.0,
            press_x: None,
        });
    }

    fn dismiss(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(toast) = self.items.iter_mut().find(|x| x.id == id && x.closing.is_none()) {
            toast.closing = Some(Instant::now());
        }
        cx.notify();
    }

    fn set_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if self.hovered == hovered {
            return;
        }
        self.hovered = hovered;
        let now = Instant::now();
        for toast in &mut self.items {
            if hovered { toast.pause(now) } else { toast.resume(now) }
        }
        cx.notify();
    }

    /// The pointer came up on (`inside`) or off a toast it pressed: a short
    /// press is a click (open its thread, dismiss), a long swipe dismisses,
    /// anything between springs back.
    fn release(&mut self, id: u64, inside: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(toast) = self.items.iter_mut().find(|x| x.id == id) else { return };
        if toast.press_x.take().is_none() {
            return;
        }
        let swiped = std::mem::take(&mut toast.swipe);
        let thread = toast.thread;
        if swiped >= SWIPE_THRESHOLD {
            toast.swipe = swiped;
            self.dismiss(id, cx);
        } else if inside && swiped < CLICK_SLOP {
            if let (Some(task), Some(open)) = (thread, self.on_open.clone()) {
                open(task, window, cx);
            }
            self.dismiss(id, cx);
        } else {
            cx.notify();
        }
    }

    fn exit_duration(reduced: bool) -> Duration {
        if reduced { Duration::ZERO } else { TOAST_EXIT.mul_f32(crate::motion::speed_scale()) }
    }

    /// Advance the stack: pause or resume the countdowns, expire, drop
    /// finished exits. True while anything is still moving.
    fn tick(&mut self, active: bool, reduced: bool, now: Instant) -> bool {
        let paused = self.hovered || !active;
        for toast in &mut self.items {
            if paused {
                toast.pause(now)
            } else {
                toast.resume(now)
            }
            if toast.closing.is_none() && toast.expired(now) {
                toast.closing = Some(now);
            }
        }
        let exit = Self::exit_duration(reduced);
        self.items.retain(|x| x.closing.is_none_or(|at| now.saturating_duration_since(at) < exit));
        if self.items.is_empty() {
            self.hovered = false;
            self.expand = None;
            self.heights.borrow_mut().clear();
            return false;
        }
        let live: std::collections::HashSet<u64> = self.items.iter().map(|x| x.id).collect();
        self.heights.borrow_mut().retain(|id, _| live.contains(id));
        let enter = TOAST_MOVE.mul_f32(crate::motion::speed_scale());
        self.items.iter().any(|x| {
            x.closing.is_some()
                || (!reduced && now.saturating_duration_since(x.created) < enter)
                || x.bottom.is_some_and(|g| g.moving(now))
        }) || self.expand.is_some_and(|g| g.moving(now))
    }

    /// Wake the view at the next expiry; animation frames cover the rest.
    fn schedule_wake(&mut self, reduced: bool, cx: &mut Context<Self>) {
        let exit = Self::exit_duration(reduced);
        let next = self.items.iter().filter_map(|x| x.deadline(exit)).min();
        if next.is_some() && self.wake.as_ref().map(|(at, _)| *at) == next {
            return;
        }
        self.wake = next.map(|at| {
            let delay = at.saturating_duration_since(Instant::now());
            let task = cx.spawn(async move |this, cx| {
                cx.background_executor().timer(delay).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            });
            (at, task)
        });
    }
}

fn toast_glyph(tone: Tone, t: &Theme) -> AnyElement {
    let (glyph, color) = match tone {
        Tone::Info => (icons::INFO_CIRCLE, t.text_muted),
        Tone::Success => (icons::CHECK, t.success),
        Tone::Warning => (icons::DANGER_TRIANGLE, t.warning),
        Tone::Danger => (icons::CLOSE_CIRCLE, t.danger),
    };
    icon(glyph).size(px(16.0)).flex_none().mt(px(1.0)).text_color(color).into_any_element()
}

impl Render for Toasts {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("Toasts");
        let now = Instant::now();
        let reduced = crate::motion::reduced_now();
        let mut moving = self.tick(window.is_window_active(), reduced, now);
        if self.items.is_empty() {
            self.wake = None;
            return div().absolute().into_any_element();
        }
        let t = Theme::of(cx).clone();
        let measured = self.heights.borrow().clone();
        let height_of = |id: u64| measured.get(&id).copied().filter(|h| *h > 0.0).unwrap_or(TOAST_DEFAULT_H);

        let expanded = self.hovered;
        let expand = {
            let glide = self.expand.get_or_insert_with(|| Glide::new(if expanded { 1.0 } else { 0.0 }, now));
            glide.retarget(if expanded { 1.0 } else { 0.0 }, now, reduced);
            moving |= glide.moving(now);
            glide.value(now)
        };

        // Newest first: depth 0 is the front toast.
        let live: Vec<u64> = self.items.iter().rev().filter(|x| x.closing.is_none()).map(|x| x.id).collect();
        let mut targets: std::collections::HashMap<u64, (f32, usize)> = std::collections::HashMap::new();
        let (mut fanned, mut collapsed_extent) = (0.0_f32, 0.0_f32);
        for (depth, id) in live.iter().enumerate() {
            let h = height_of(*id);
            let collapsed = depth as f32 * TOAST_GAP;
            if depth < TOAST_VISIBLE {
                collapsed_extent = collapsed_extent.max(collapsed + h);
            }
            targets.insert(*id, (crate::motion::lerp(collapsed, fanned, expand), depth));
            fanned += h + TOAST_GAP;
        }
        let fanned_extent = (fanned - TOAST_GAP).max(0.0);
        // The hit area: the collapsed stack, or the whole fan while the
        // pointer holds it open (it paints nothing, so it simply snaps).
        let stack_h = if expanded { fanned_extent.max(collapsed_extent) } else { collapsed_extent };
        let width = TOAST_WIDTH.min(f32::from(window.viewport_size().width) - 2.0 * TOAST_OFFSET).max(160.0);
        let enter_total = TOAST_MOVE.mul_f32(crate::motion::speed_scale()).as_secs_f32();
        let exit_total = Self::exit_duration(reduced).as_secs_f32().max(0.001);
        let entity = cx.entity_id();

        let mut children: Vec<AnyElement> = Vec::new();
        // Paint oldest first so the front toast lands on top.
        for toast in self.items.iter_mut() {
            let id = toast.id;
            let h = height_of(id);
            let (target, depth) = targets.get(&id).copied().unwrap_or((toast.bottom.map_or(0.0, |g| g.to), usize::MAX));
            let first = toast.bottom.is_none();
            let glide = toast.bottom.get_or_insert_with(|| Glide::new(target, now));
            glide.retarget(target, now, reduced || first);
            moving |= glide.moving(now);
            let mut y = glide.value(now);

            // Enter: rise from below the stack and fade in.
            let enter = if reduced {
                1.0
            } else {
                TOAST_EASE.eval(now.saturating_duration_since(toast.created).as_secs_f32() / enter_total)
            };
            y -= (1.0 - enter) * (h + TOAST_OFFSET);
            let mut opacity = enter;
            // Exit: fade out; a toast leaving on its own sinks a little.
            if let Some(at) = toast.closing {
                let k = (now.saturating_duration_since(at).as_secs_f32() / exit_total).clamp(0.0, 1.0);
                opacity *= 1.0 - k;
                if toast.swipe < 0.5 {
                    y -= k * TOAST_GAP;
                }
            }
            let depth = if depth == usize::MAX { 0 } else { depth };
            if depth >= TOAST_VISIBLE {
                opacity *= expand;
            }
            if toast.swipe > 0.0 {
                opacity *= (1.0 - toast.swipe / (width * 0.6)).clamp(0.0, 1.0);
            }
            if opacity <= 0.001 && toast.closing.is_some() {
                continue;
            }
            // Behind the front, a collapsed toast shows only its edge.
            let content_opacity = if depth == 0 { 1.0 } else { expand };
            let heights = self.heights.clone();
            let content = div()
                .w_full()
                .px(px(14.0))
                .py(px(12.0))
                .flex()
                .items_start()
                .gap(px(10.0))
                .opacity(content_opacity)
                .child(toast_glyph(toast.tone, &t))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(ty::BODY)
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(t.text)
                                .child(toast.title.clone()),
                        )
                        .when_some(toast.body.clone(), |d, b| {
                            d.child(div().text_size(ty::SMALL).text_color(t.text_muted).line_clamp(2).child(b))
                        }),
                );
            let card = div()
                .relative()
                .w_full()
                .rounded(px(popover::CARD_RADIUS))
                .bg(popover::surface_bg(&t))
                .border_1()
                .border_color(t.border_strong)
                .shadow_lg()
                .overflow_hidden()
                .when(toast.thread.is_some() && depth == 0, |d| d.cursor_pointer())
                .child(content)
                // Measure the natural height for the stack's layout.
                .child(
                    canvas(
                        move |bounds, _, cx| {
                            let h = f32::from(bounds.size.height);
                            let changed = heights.borrow().get(&id).is_none_or(|old| (old - h).abs() > 0.5);
                            if changed {
                                heights.borrow_mut().insert(id, h);
                                cx.notify(entity);
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                );
            children.push(
                div()
                    .id(SharedString::from(format!("toast-{id}")))
                    .role(if toast.tone == Tone::Danger { gpui::Role::Alert } else { gpui::Role::Status })
                    .aria_label(toast.title.clone())
                    .absolute()
                    .left(px(toast.swipe.max(0.0)))
                    .bottom(px(y))
                    .w(px(width))
                    .opacity(opacity)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                            if let Some(x) = this.items.iter_mut().find(|x| x.id == id && x.closing.is_none()) {
                                x.press_x = Some(f32::from(e.position.x));
                                cx.stop_propagation();
                            }
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, e: &gpui::MouseMoveEvent, _, cx| {
                        let Some(x) = this.items.iter_mut().find(|x| x.id == id) else { return };
                        let Some(from) = x.press_x else { return };
                        if e.dragging() {
                            x.swipe = (f32::from(e.position.x) - from).max(0.0);
                        } else {
                            x.press_x = None;
                            x.swipe = 0.0;
                        }
                        cx.notify();
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| this.release(id, true, window, cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| this.release(id, false, window, cx)),
                    )
                    .child(ui::frosted(popover::CARD_RADIUS, 18.0, card))
                    .into_any_element(),
            );
        }

        if moving {
            window.request_animation_frame();
        }
        self.schedule_wake(reduced, cx);

        div()
            .id("toaster")
            .absolute()
            .bottom(px(TOAST_OFFSET))
            .right(px(TOAST_OFFSET))
            .w(px(width))
            .h(px(stack_h))
            // The stack, not each toast, blocks what is under it, so the
            // hover that holds every countdown sees the whole fan.
            .occlude()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| this.set_hovered(*hovered, cx)))
            .children(children)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_thresholds() {
        let t =
            Theme::new(crate::theme::Appearance::Dark, crate::theme::Accent::default(), crate::theme::Surface::Normal);
        assert_eq!(ring_color(0.5, &t), t.text_muted);
        assert_eq!(ring_color(0.75, &t), t.warning);
        assert_eq!(ring_color(0.95, &t), t.danger);
    }

    #[test]
    fn toasts_expire_pause_and_leave_after_exit() {
        let start = Instant::now();
        let mut s = Toasts::empty();
        s.add("one".into(), None, Tone::Info, None, start);
        s.tick(true, false, start + Duration::from_secs(1));
        // Hovering banks the elapsed second and holds the clock.
        s.hovered = true;
        s.tick(true, false, start + Duration::from_secs(1));
        s.tick(true, false, start + Duration::from_secs(30));
        assert!(s.items[0].closing.is_none(), "paused while hovered");
        s.hovered = false;
        // An inactive window holds it too.
        s.tick(false, false, start + Duration::from_secs(31));
        s.tick(false, false, start + Duration::from_secs(60));
        assert!(s.items[0].closing.is_none(), "paused while inactive");
        // Three seconds remain once resumed.
        s.tick(true, false, start + Duration::from_secs(61));
        s.tick(true, false, start + Duration::from_millis(63_900));
        assert!(s.items[0].closing.is_none());
        s.tick(true, false, start + Duration::from_millis(64_100));
        assert!(s.items[0].closing.is_some(), "expires on time");
        s.tick(true, false, start + Duration::from_millis(64_400));
        assert!(s.items.is_empty(), "removed after its exit");
    }

    #[test]
    fn repeat_of_front_toast_restarts_it() {
        let now = Instant::now();
        let mut s = Toasts::empty();
        s.add("Code copied".into(), None, Tone::Success, None, now);
        s.add("Code copied".into(), None, Tone::Success, None, now);
        assert_eq!(s.items.len(), 1);
        s.add("Could not start the draft".into(), Some("boom".into()), Tone::Danger, None, now);
        assert_eq!(s.items.len(), 2);
    }

    #[test]
    fn reduced_motion_removes_closed_toasts_at_once() {
        let now = Instant::now();
        let mut s = Toasts::empty();
        s.add("one".into(), None, Tone::Info, None, now);
        s.items[0].closing = Some(now);
        s.tick(true, true, now);
        assert!(s.items.is_empty());
    }

    #[test]
    fn glide_restarts_from_its_drawn_value() {
        let now = Instant::now();
        let mut g = Glide::new(0.0, now);
        g.retarget(100.0, now, false);
        let mid = now + TOAST_MOVE / 2;
        let halfway = g.value(mid);
        assert!(halfway > 0.0 && halfway < 100.0);
        g.retarget(0.0, mid, false);
        assert!((g.value(mid) - halfway).abs() < 0.01, "no jump on reversal");
    }
}
