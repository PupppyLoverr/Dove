//! Motion: the timing catalog, the leased pulse clock that drives every
//! looping indicator, hover and state fades, and reduced motion. Ported from
//! Zeron's `motion.rs` (MIT) and adapted to Dove's settings store.
//!
//! Rules every animation in Dove follows:
//! - Only opacity and position offsets move (a relative `top`/`left` inset,
//!   which taffy applies after layout, so siblings never shift), plus paint
//!   transforms on images and SVGs. Sizes change once, then content reveals.
//! - Nothing schedules frames unless something on screen is moving. Looping
//!   indicators share one 30 Hz clock that parks when no view leases it.
//! - Reduced motion (the OS setting, or the user's override) snaps every
//!   transition to its end and stills every loop. "Pause animations in
//!   background" does the same while the window is inactive.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationElement, AnimationExt as _, AnyElement, App, Bounds, ElementId, EntityId, Global,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, Rgba, SharedString, Styled, Window,
    canvas, px,
};

use dove_core::store::Store;

// ---------------------------------------------------------------------------
// Pulse clock: the shared, throttled drive for looping indicators
// ---------------------------------------------------------------------------

/// Tick of the shared loop clock (~30 fps). A `with_animation(...repeat())`
/// element requests a frame every display refresh for as long as it is
/// mounted, and in GPUI that re-renders every view above it; one spinner in
/// the sidebar used to rebuild the whole shell at 60–120 Hz.
const PULSE_TICK: Duration = Duration::from_millis(33);

/// How long a view stays on the tick list after its last paint of a looping
/// indicator. Unmounted indicators stop renewing and the clock parks.
const PULSE_LEASE: Duration = Duration::from_millis(300);

struct PulseClock {
    epoch: Instant,
    leases: HashMap<EntityId, PulseLease>,
    tick: u64,
    running: bool,
}

struct PulseLease {
    until: Instant,
    stride: u64,
}

impl PulseLease {
    fn renew(&mut self, now: Instant, stride: u64) {
        self.until = now + PULSE_LEASE;
        self.stride = self.stride.min(stride);
    }

    fn take_tick(&mut self, tick: u64) -> bool {
        if !tick.is_multiple_of(self.stride) {
            return false;
        }
        // Each paint re-establishes the fastest mounted loop, so a retired
        // 30 Hz loop does not hold a 15 Hz one at 30 Hz forever.
        self.stride = u64::MAX;
        true
    }
}

impl Global for PulseClock {}

impl Default for PulseClock {
    fn default() -> Self {
        Self { epoch: Instant::now(), leases: HashMap::new(), tick: 0, running: false }
    }
}

/// Keep `view` re-rendering on the pulse clock (every `stride` ticks) while
/// it keeps painting a looping indicator. No-op under reduced motion.
pub fn pulse_lease(view: EntityId, stride: u64, cx: &mut App) {
    if cx.reduce_motion() {
        return;
    }
    schedule_pulse(view, stride.max(1), cx);
}

/// Phase `[0, 1)` of a loop of `period`, on the shared epoch, so every
/// instance of an indicator stays in step.
pub fn pulse_phase(period: Duration, cx: &mut App) -> f32 {
    let clock = cx.default_global::<PulseClock>();
    (clock.epoch.elapsed().as_secs_f32() / period.as_secs_f32().max(0.001)).fract()
}

fn schedule_pulse(view: EntityId, stride: u64, cx: &mut App) {
    let clock = cx.default_global::<PulseClock>();
    let now = Instant::now();
    clock.leases.entry(view).or_insert(PulseLease { until: now + PULSE_LEASE, stride }).renew(now, stride);
    if clock.running {
        return;
    }
    clock.running = true;
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(PULSE_TICK).await;
            let parked = cx.update(|cx| {
                let clock = cx.default_global::<PulseClock>();
                let now = Instant::now();
                clock.leases.retain(|_, lease| lease.until > now);
                if clock.leases.is_empty() {
                    clock.running = false;
                    return true;
                }
                clock.tick = clock.tick.wrapping_add(1);
                let tick = clock.tick;
                let due: Vec<EntityId> = clock
                    .leases
                    .iter_mut()
                    .filter_map(|(view, lease)| lease.take_tick(tick).then_some(*view))
                    .collect();
                for view in due {
                    cx.notify(view);
                }
                false
            });
            if parked {
                break;
            }
        }
    })
    .detach();
}

/// What an activity indicator shows this frame.
#[derive(Clone, Copy, Debug)]
pub struct ActivityPhase {
    pub phase: f32,
    /// System reduced motion: keep a gentle uniform brightness cue instead
    /// of a travelling wave. Explicit On and background pause keep it still.
    pub subtle: bool,
    pub still: bool,
}

impl ActivityPhase {
    /// Opacity of one cell whose wave offset is `cell_phase`.
    pub fn opacity(&self, cell_phase: f32, dim: f32) -> f32 {
        if self.still {
            return if cell_phase.abs() < f32::EPSILON { 1.0 } else { (dim + 1.0) * 0.5 };
        }
        if self.subtle {
            0.6 + 0.2 * pulse_wave(self.phase)
        } else {
            crate::loaders::gspin_opacity(self.phase + cell_phase, dim)
        }
    }
}

/// A smooth 0→1→0 wave over one period.
pub fn pulse_wave(phase: f32) -> f32 {
    0.5 - 0.5 * (phase.rem_euclid(1.0) * std::f32::consts::TAU).cos()
}

fn activity_phase(view: EntityId, period: Duration, stride: u64, cx: &mut App) -> ActivityPhase {
    let reduced = cx.reduce_motion();
    let system_only = cx
        .try_global::<MotionState>()
        .is_some_and(|s| s.preference == ReduceMotion::System && s.system && !(s.pause_in_background && !s.active));
    if reduced && !system_only {
        return ActivityPhase { phase: 0.0, subtle: false, still: true };
    }
    let subtle = reduced;
    schedule_pulse(view, if subtle { 2 } else { stride.max(1) }, cx);
    let phase = pulse_phase(if subtle { Duration::from_millis(2400) } else { period }, cx);
    ActivityPhase { phase, subtle, still: false }
}

/// An element that builds its content from the pulse clock's phase at
/// layout time and leases the clock for whichever view paints it. Lets
/// free-function builders (status glyphs, loaders) animate without a `cx`
/// and without re-rendering anything but their own view's subtree.
pub struct Pulse {
    period: Duration,
    stride: u64,
    build: Option<Box<dyn FnOnce(ActivityPhase) -> AnyElement>>,
}

/// A looping indicator of `period`, redrawn at 30 Hz / `stride`.
pub fn pulse(period: Duration, stride: u64, build: impl FnOnce(ActivityPhase) -> AnyElement + 'static) -> Pulse {
    Pulse { period, stride, build: Some(Box::new(build)) }
}

impl gpui::Element for Pulse {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnyElement) {
        let view = window.current_view();
        let phase = activity_phase(view, self.period, self.stride, cx);
        let mut child = match self.build.take() {
            Some(build) => build(phase),
            None => gpui::Empty.into_any_element(),
        };
        (child.request_layout(window, cx), child)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
    }
}

impl IntoElement for Pulse {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// A zero-size element that keeps the painting view on the pulse clock: for
/// views that compute a looping effect themselves (the activity shimmer).
pub fn pulse_driver(stride: u64) -> AnyElement {
    canvas(move |_, window, cx| pulse_lease(window.current_view(), stride, cx), |_, _, _, _| {})
        .absolute()
        .size_0()
        .into_any_element()
}

// ---------------------------------------------------------------------------
// Cubic bezier
// ---------------------------------------------------------------------------

/// A CSS `cubic-bezier(x1, y1, x2, y2)` timing function, evaluated exactly
/// (Newton iteration with a bisection fallback).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CubicBezier {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl CubicBezier {
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    fn coefficients(a: f32, b: f32) -> (f32, f32, f32) {
        let c = 3.0 * a;
        let bb = 3.0 * (b - a) - c;
        let aa = 1.0 - c - bb;
        (aa, bb, c)
    }

    fn sample_x(&self, t: f32) -> f32 {
        let (a, b, c) = Self::coefficients(self.x1, self.x2);
        ((a * t + b) * t + c) * t
    }

    fn sample_y(&self, t: f32) -> f32 {
        let (a, b, c) = Self::coefficients(self.y1, self.y2);
        ((a * t + b) * t + c) * t
    }

    fn sample_x_derivative(&self, t: f32) -> f32 {
        let (a, b, c) = Self::coefficients(self.x1, self.x2);
        (3.0 * a * t + 2.0 * b) * t + c
    }

    fn solve_t_for_x(&self, x: f32) -> f32 {
        let mut t = x;
        for _ in 0..8 {
            let err = self.sample_x(t) - x;
            if err.abs() < 1e-6 {
                return t;
            }
            let d = self.sample_x_derivative(t);
            if d.abs() < 1e-6 {
                break;
            }
            t -= err / d;
        }
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        for _ in 0..32 {
            let mid = (lo + hi) / 2.0;
            if self.sample_x(mid) < x { lo = mid } else { hi = mid }
        }
        (lo + hi) / 2.0
    }

    /// Eased output for progress `x` (clamped to `[0, 1]`). GPUI asserts an
    /// animation delta stays inside `[0, 1]`, so the output is clamped too.
    pub fn eval(&self, x: f32) -> f32 {
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        self.sample_y(self.solve_t_for_x(x)).clamp(0.0, 1.0)
    }
}

/// Entrances — CSS `cubic-bezier(0.16, 1, 0.3, 1)`.
pub const EASE_OUT_EXPO: CubicBezier = CubicBezier::new(0.16, 1.0, 0.3, 1.0);
/// CSS `ease-out`.
pub const EASE_OUT: CubicBezier = CubicBezier::new(0.0, 0.0, 0.58, 1.0);
/// CSS `ease`.
pub const EASE: CubicBezier = CubicBezier::new(0.25, 0.1, 0.25, 1.0);
/// `easeOutQuint`.
pub const EASE_OUT_QUINT: CubicBezier = CubicBezier::new(0.22, 1.0, 0.36, 1.0);
/// CSS `ease-in-out`.
pub const EASE_IN_OUT: CubicBezier = CubicBezier::new(0.42, 0.0, 0.58, 1.0);
/// Tailwind's default transition curve.
pub const EASE_TAILWIND: CubicBezier = CubicBezier::new(0.4, 0.0, 0.2, 1.0);
/// Fast start, long soft landing.
pub const EASE_OUT_SOFT: CubicBezier = CubicBezier::new(0.2, 0.0, 0.0, 1.0);

// ---------------------------------------------------------------------------
// The catalog
// ---------------------------------------------------------------------------

/// One catalog entry: duration, optional delay, curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSpec {
    pub duration_ms: u64,
    pub delay_ms: u64,
    pub curve: CubicBezier,
}

impl MotionSpec {
    pub const fn new(duration_ms: u64, curve: CubicBezier) -> Self {
        Self { duration_ms, delay_ms: 0, curve }
    }

    #[cfg(test)]
    pub const fn with_delay(mut self, delay_ms: u64) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    /// The whole timeline (delay + duration), stretched by [`speed_scale`].
    pub fn total(&self) -> Duration {
        Duration::from_millis(self.delay_ms + self.duration_ms).mul_f32(speed_scale())
    }

    /// Eased progress for a raw timeline fraction (`[0, 1]` over [`total`]).
    pub fn progress(&self, raw: f32) -> f32 {
        let total = (self.delay_ms + self.duration_ms) as f32;
        if total <= 0.0 || self.duration_ms == 0 {
            return 1.0;
        }
        let t = (raw.clamp(0.0, 1.0) * total - self.delay_ms as f32) / self.duration_ms as f32;
        self.curve.eval(t.clamp(0.0, 1.0))
    }

    /// Eased progress of a transition that started at `start`.
    pub fn since(&self, start: Instant, now: Instant) -> f32 {
        let total = self.total();
        if total.is_zero() {
            return 1.0;
        }
        self.progress(now.saturating_duration_since(start).as_secs_f32() / total.as_secs_f32())
    }

    /// A oneshot GPUI animation for this spec.
    pub fn animation(&self) -> Animation {
        let spec = *self;
        Animation::new(spec.total()).with_easing(move |d| spec.progress(d))
    }
}

/// Every hover, press and selection wash.
pub const HOVER_FADE: MotionSpec = MotionSpec::new(150, EASE_TAILWIND);
/// Popovers and menus in.
pub const MENU_IN: MotionSpec = MotionSpec::new(140, EASE);
/// Popovers and menus out: quicker than the entrance.
pub const MENU_OUT: MotionSpec = MotionSpec::new(100, EASE);
/// Modals and sheets in.
pub const DIALOG_IN: MotionSpec = MotionSpec::new(180, EASE);
/// Sidebar and dock slides (stretched toward 240 ms for wide moves).
pub const PANEL: MotionSpec = DIALOG_IN;
/// Segment and nav selection indicators.
pub const INDICATOR_SLIDE: MotionSpec = MotionSpec::new(260, EASE_OUT_SOFT);
/// Two-state glyph morphs (sidebar and panel toggles).
pub const GLYPH_STATE: MotionSpec = MotionSpec::new(280, EASE_OUT_QUINT);
/// List resort glides (sidebar attention buckets, Kanban drops).
pub const LIST_REORDER: MotionSpec = MotionSpec::new(260, EASE_OUT_QUINT);
/// First appearance of a page or row: opacity + 4 px rise.
pub const FADE_IN: MotionSpec = MotionSpec::new(500, EASE_OUT_EXPO);
/// Crossfades.
pub const FADE_QUICK: MotionSpec = MotionSpec::new(150, EASE);
/// Chevron disclosure crossfade.
pub const CHEVRON: MotionSpec = MotionSpec::new(200, EASE);
/// Collapsible content reveal (opacity + rise; the height snaps).
pub const REVEAL: MotionSpec = MotionSpec::new(180, EASE_OUT);
/// Rail jumps and scroll-to-row.
pub const SCROLL_GLIDE: MotionSpec = MotionSpec::new(500, EASE_IN_OUT);
/// Toggle thumb travel.
pub const TOGGLE: MotionSpec = MotionSpec::new(180, EASE_OUT);
/// Activity spinner period.
pub const SPIN_PERIOD: Duration = Duration::from_millis(750);

// ---------------------------------------------------------------------------
// Element helpers
// ---------------------------------------------------------------------------

/// Standard entrance: opacity 0→1 and a 4 px rise over [`FADE_IN`].
pub fn fade_in<E>(id: impl Into<ElementId>, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    element.with_animation(id, FADE_IN.animation(), |el, t| el.relative().opacity(t).top(px(4.0 * (1.0 - t))))
}

/// Opacity-only crossfade over [`FADE_QUICK`].
pub fn fade_quick<E>(id: impl Into<ElementId>, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    element.with_animation(id, FADE_QUICK.animation(), |el, t| el.opacity(t))
}

/// Popover entrance: fade from 0.3 plus a 2 px drop into place.
pub fn menu_in<E>(id: impl Into<ElementId>, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    menu_in_from(id, -2.0, element)
}

/// [`menu_in`] travelling from `from` px (negative: from above).
pub fn menu_in_from<E>(id: impl Into<ElementId>, from: f32, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    element.with_animation(id, MENU_IN.animation(), move |el, t| {
        el.relative().opacity(0.3 + 0.7 * t).top(px(from * (1.0 - t)))
    })
}

/// Popover exit at progress `t` (computed by the caller from the closing
/// instant, so a remount mid-exit never replays a full-opacity frame).
pub fn menu_out<E>(id: impl Into<ElementId>, t: f32, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    element.with_animation(id, MENU_OUT.animation(), move |el, _| el.relative().opacity(1.0 - t).top(px(-2.0 * t)))
}

/// Dialog entrance: fade plus a 2 px settle.
pub fn dialog_in<E>(id: impl Into<ElementId>, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    element.with_animation(id, DIALOG_IN.animation(), |el, t| el.relative().opacity(t).top(px(2.0 * (1.0 - t))))
}

/// Content reveal for collapsibles: the height is final at once, the
/// content fades and rises 3 px into it.
pub fn reveal<E>(id: impl Into<ElementId>, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    element.with_animation(id, REVEAL.animation(), |el, t| el.relative().opacity(t).top(px(-3.0 * (1.0 - t))))
}

/// A panel sliding in from the right (`from_right`) or left by `distance`.
pub fn slide_in<E>(id: impl Into<ElementId>, distance: f32, from_right: bool, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    let spec = panel_spec(distance);
    let sign = if from_right { 1.0 } else { -1.0 };
    element.with_animation(id, spec.animation(), move |el, t| {
        el.relative().opacity(0.4 + 0.6 * t).left(px(sign * 24.0 * (1.0 - t)))
    })
}

/// Panel motion stretches gently with distance: 180 ms for short moves, up
/// to 240 ms for wide ones.
pub fn panel_spec(distance: f32) -> MotionSpec {
    let extra = ((distance.abs() / 256.0).sqrt() * 30.0).min(60.0) as u64;
    MotionSpec::new(PANEL.duration_ms + extra, PANEL.curve)
}

/// Linear interpolation.
pub fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

// ---------------------------------------------------------------------------
// Interruptible value tweens
// ---------------------------------------------------------------------------

/// A selection indicator gliding between slots. `position` is in slots, so
/// a retarget mid-flight continues from what is drawn.
#[derive(Debug, Clone, Copy)]
pub struct IndicatorSlide {
    from: f32,
    to: f32,
    started: Instant,
}

impl IndicatorSlide {
    pub fn at(position: f32) -> Self {
        Self { from: position, to: position, started: Instant::now() }
    }

    pub fn value(&self) -> f32 {
        lerp(self.from, self.to, INDICATOR_SLIDE.since(self.started, Instant::now()))
    }

    /// Aim at a new slot; reduced motion snaps.
    pub fn retarget(&mut self, to: f32, reduced: bool) {
        if (to - self.to).abs() < f32::EPSILON {
            return;
        }
        let from = if reduced { to } else { self.value() };
        *self = Self { from, to, started: Instant::now() };
    }
}

// ---------------------------------------------------------------------------
// Hover and state fades
// ---------------------------------------------------------------------------
//
// GPUI's `.hover()` style snaps the frame the pointer enters. These stores
// give washes a 150 ms fade instead: an `.on_hover` listener flips a key,
// builders read the key's progress, and the shell's render tail requests
// frames while any fade is mid-flight. Entries unread for a whole frame are
// pruned, so a reopened menu never inherits a dead entry.

#[derive(Debug, Clone, Copy)]
struct FadeEntry {
    origin: f32,
    target: f32,
    started: Instant,
    spec: MotionSpec,
    seen: u64,
}

impl FadeEntry {
    fn value(&self, now: Instant) -> f32 {
        lerp(self.origin, self.target, self.spec.since(self.started, now))
    }

    fn settled(&self, now: Instant) -> bool {
        self.origin == self.target || now.saturating_duration_since(self.started) >= self.spec.total()
    }
}

#[derive(Default)]
struct Fades {
    entries: HashMap<String, FadeEntry>,
    frame: u64,
}

impl Fades {
    fn set(&mut self, key: &str, on: bool, spec: MotionSpec, reduced: bool, now: Instant) {
        let target = if on { 1.0 } else { 0.0 };
        if target == 0.0 && !self.entries.contains_key(key) {
            return;
        }
        let current = self.entries.get(key).map(|e| e.value(now)).unwrap_or(0.0);
        let origin = if reduced { target } else { current };
        let seen = self.frame;
        self.entries.insert(key.to_string(), FadeEntry { origin, target, started: now, spec, seen });
    }

    /// A state morph: the first read snaps to `on`, later flips animate.
    fn state(&mut self, key: &str, on: bool, spec: MotionSpec, reduced: bool, now: Instant) -> f32 {
        let target = if on { 1.0 } else { 0.0 };
        let frame = self.frame;
        let Some(entry) = self.entries.get_mut(key) else {
            self.entries.insert(key.to_string(), FadeEntry { origin: target, target, started: now, spec, seen: frame });
            return target;
        };
        if entry.target != target {
            let current = entry.value(now);
            *entry =
                FadeEntry { origin: if reduced { target } else { current }, target, started: now, spec, seen: frame };
        }
        entry.seen = frame;
        entry.value(now)
    }

    fn value(&mut self, key: &str, now: Instant) -> f32 {
        let frame = self.frame;
        match self.entries.get_mut(key) {
            Some(entry) => {
                entry.seen = frame;
                entry.value(now)
            }
            None => 0.0,
        }
    }

    /// Once per frame: prune dead entries, report whether any is moving.
    fn tick(&mut self, now: Instant, keep_rest: bool) -> bool {
        self.frame += 1;
        let frame = self.frame;
        let mut active = false;
        self.entries.retain(|_, entry| {
            if entry.seen + 1 < frame {
                return false;
            }
            let settled = entry.settled(now);
            active |= !settled;
            keep_rest || !(settled && entry.target == 0.0)
        });
        active
    }
}

thread_local! {
    static HOVERS: RefCell<Fades> = RefCell::new(Fades::default());
    static STATES: RefCell<Fades> = RefCell::new(Fades::default());
    /// The view whose frames advance the fades (the shell).
    static CHROME_VIEW: std::cell::Cell<Option<gpui::EntityId>> = const { std::cell::Cell::new(None) };
}

/// Name the view that draws faded chrome (the shell calls this as it
/// renders), so a hover flip repaints just that view instead of the window.
pub fn set_chrome_view(id: gpui::EntityId) {
    CHROME_VIEW.with(|v| v.set(Some(id)));
}

/// Hover progress `[0, 1]` of `key` this frame.
pub fn hover_t(key: &str) -> f32 {
    HOVERS.with(|f| f.borrow_mut().value(key, Instant::now()))
}

/// Record a hover flip for `key`.
pub fn set_hover(key: &str, hovered: bool, reduced: bool) {
    HOVERS.with(|f| f.borrow_mut().set(key, hovered, HOVER_FADE, reduced, Instant::now()));
}

/// An `.on_hover` listener that drives the fade for `key`.
pub fn hover_listener(key: impl Into<SharedString>) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
    let key = key.into();
    move |hovered, window, cx| {
        set_hover(&key, *hovered, cx.reduce_motion());
        // Repaint the shell (cached children are reused); its render tail
        // keeps frames coming while the fade is mid-flight.
        match CHROME_VIEW.with(|v| v.get()) {
            Some(id) => cx.notify(id),
            None => window.refresh(),
        }
    }
}

/// A shell-chrome row's hover: its wash (and optionally its text) fades
/// between rest and hover over [`HOVER_FADE`]. Only for elements the shell
/// draws itself; cached views keep instant `.hover()` styles.
pub fn hover_fade<E>(el: E, key: impl Into<SharedString>, bg: (Hsla, Hsla), fg: Option<(Hsla, Hsla)>) -> E
where
    E: gpui::InteractiveElement + Styled + gpui::StatefulInteractiveElement,
{
    let key: SharedString = key.into();
    let el = el.bg(hover_blend(&key, bg.0, bg.1));
    let el = match fg {
        Some((rest, hover)) => el.text_color(hover_blend(&key, rest, hover)),
        None => el,
    };
    el.on_hover(hover_listener(key))
}

/// The rest → hover blend of `key`.
pub fn hover_blend(key: &str, rest: Hsla, hover: Hsla) -> Hsla {
    mix(rest, hover, hover_t(key))
}

/// Progress `[0, 1]` of a two-state morph at `key` toward `on`.
pub fn state_t(key: &str, on: bool, spec: MotionSpec, reduced: bool) -> f32 {
    STATES.with(|f| f.borrow_mut().state(key, on, spec, reduced, Instant::now()))
}

/// Call once per window frame (the shell's render tail): true while any
/// hover or state fade is still moving and frames must keep coming.
pub fn fades_active() -> bool {
    let now = Instant::now();
    let hovers = HOVERS.with(|f| f.borrow_mut().tick(now, false));
    let states = STATES.with(|f| f.borrow_mut().tick(now, true));
    hovers | states
}

/// Blend two colours the way a browser transitions them: sRGB with
/// premultiplied alpha, so a wash fading in from transparent never greys.
pub fn mix(from: Hsla, to: Hsla, t: f32) -> Hsla {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return from;
    }
    if t >= 1.0 {
        return to;
    }
    let (f, g) = (Rgba::from(from), Rgba::from(to));
    let a = lerp(f.a, g.a, t);
    if a <= f32::EPSILON {
        return Hsla::from(Rgba { a: 0.0, ..g });
    }
    Hsla::from(Rgba {
        r: lerp(f.r * f.a, g.r * g.a, t) / a,
        g: lerp(f.g * f.a, g.g * g.a, t) / a,
        b: lerp(f.b * f.a, g.b * g.a, t) / a,
        a,
    })
}

// ---------------------------------------------------------------------------
// Resize-edge feedback
// ---------------------------------------------------------------------------

/// How far a seam nudges past a limit while a drag holds it there.
pub const RESIZE_EDGE_NUDGE: f32 = 5.0;
const RESIZE_EDGE_BOUNCE: Duration = Duration::from_millis(220);
const RESIZE_EDGE_OUT_FRACTION: f32 = 0.32;

/// Which limit a drag is pressing against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    Min,
    Max,
}

/// A clamped resize sample and the limit it presses, if any.
pub fn clamp_resize(requested: f32, min: f32, max: f32) -> (f32, Option<ResizeEdge>) {
    let edge = if requested <= min {
        Some(ResizeEdge::Min)
    } else if requested >= max {
        Some(ResizeEdge::Max)
    } else {
        None
    };
    (requested.clamp(min, max), edge)
}

/// The seam's offset `elapsed` into a bounce against `edge`: out, then a
/// slower settle home, both ends with zero velocity.
pub fn resize_bounce(edge: ResizeEdge, started: Instant) -> Option<f32> {
    let total = RESIZE_EDGE_BOUNCE.mul_f32(speed_scale());
    let elapsed = started.elapsed();
    if elapsed >= total {
        return None;
    }
    let raw = elapsed.as_secs_f32() / total.as_secs_f32();
    let smooth = |t: f32| {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let magnitude = if raw < RESIZE_EDGE_OUT_FRACTION {
        smooth(raw / RESIZE_EDGE_OUT_FRACTION)
    } else {
        1.0 - smooth((raw - RESIZE_EDGE_OUT_FRACTION) / (1.0 - RESIZE_EDGE_OUT_FRACTION))
    } * RESIZE_EDGE_NUDGE;
    Some(match edge {
        ResizeEdge::Min => -magnitude,
        ResizeEdge::Max => magnitude,
    })
}

// ---------------------------------------------------------------------------
// Reduced motion
// ---------------------------------------------------------------------------

/// `DOVE_MOTION_SCALE` (default 1, clamped to 0.01–100): stretches every
/// timeline, for reviewing motion frame by frame. Never set in production.
pub fn speed_scale() -> f32 {
    static SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SCALE.get_or_init(|| {
        std::env::var("DOVE_MOTION_SCALE")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|s| s.is_finite())
            .map(|s| s.clamp(0.01, 100.0))
            .unwrap_or(1.0)
    })
}

/// The user's reduced-motion preference (setting `reduce_motion`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReduceMotion {
    /// Follow the OS accessibility setting.
    #[default]
    System,
    On,
    Off,
}

impl ReduceMotion {
    pub const ALL: [Self; 3] = [Self::System, Self::On, Self::Off];

    pub const fn id(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::On => "on",
            Self::Off => "off",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::On => "On",
            Self::Off => "Off",
        }
    }

    pub fn from_id(id: Option<&str>) -> Self {
        match id {
            Some("on") => Self::On,
            Some("off") => Self::Off,
            _ => Self::System,
        }
    }
}

struct MotionState {
    preference: ReduceMotion,
    pause_in_background: bool,
    system: bool,
    active: bool,
}

impl Global for MotionState {}

/// Combine the preference with the OS setting and window focus.
pub fn resolve(preference: ReduceMotion, system: bool, pause_in_background: bool, active: bool) -> bool {
    let reduced = match preference {
        ReduceMotion::System => system,
        ReduceMotion::On => true,
        ReduceMotion::Off => false,
    };
    reduced || (pause_in_background && !active)
}

/// Read the saved preferences and set the flag before the first window
/// opens, so even boot animations honour it.
pub fn init(store: &Store, cx: &mut App) {
    let preference = ReduceMotion::from_id(store.setting("reduce_motion").as_deref());
    let pause_in_background = store.setting("pause_animations_in_background").as_deref() == Some("true");
    cx.set_global(MotionState { preference, pause_in_background, system: false, active: true });
    refresh_system(cx);
    apply(cx);
}

pub fn preference(cx: &App) -> ReduceMotion {
    cx.try_global::<MotionState>().map(|s| s.preference).unwrap_or_default()
}

pub fn pause_in_background(cx: &App) -> bool {
    cx.try_global::<MotionState>().is_some_and(|s| s.pause_in_background)
}

/// The OS setting as last read (Settings shows it beside "System").
pub fn system_reduces_motion(cx: &App) -> bool {
    cx.try_global::<MotionState>().is_some_and(|s| s.system)
}

/// Whether decorative loops should run now: motion is not reduced and the
/// window is active. Status indicators use [`pulse`] instead, which keeps a
/// subtle cue under system reduced motion.
pub fn decorative_motion(cx: &App) -> bool {
    !cx.reduce_motion() && cx.try_global::<MotionState>().is_none_or(|s| s.active)
}

pub fn set_preference(preference: ReduceMotion, store: &Store, cx: &mut App) {
    let _ = store.set_setting("reduce_motion", preference.id());
    if let Some(state) = cx.has_global::<MotionState>().then(|| cx.global_mut::<MotionState>()) {
        state.preference = preference;
    }
    apply(cx);
}

pub fn set_pause_in_background(enabled: bool, store: &Store, cx: &mut App) {
    let _ = store.set_setting("pause_animations_in_background", if enabled { "true" } else { "false" });
    if let Some(state) = cx.has_global::<MotionState>().then(|| cx.global_mut::<MotionState>()) {
        state.pause_in_background = enabled;
    }
    apply(cx);
}

/// The main window gained or lost focus. Regaining focus re-reads the OS
/// setting: changing it means visiting system settings, so coming back is
/// when it can have moved.
pub fn window_activation_changed(active: bool, cx: &mut App) {
    if !cx.has_global::<MotionState>() {
        return;
    }
    if active {
        refresh_system(cx);
    }
    cx.global_mut::<MotionState>().active = active;
    apply(cx);
}

fn set_system(system: bool, cx: &mut App) {
    if let Some(state) = cx.has_global::<MotionState>().then(|| cx.global_mut::<MotionState>()) {
        state.system = system;
    }
    apply(cx);
}

/// The resolved flag, mirrored for free-function builders that have no `cx`.
static REDUCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether motion is reduced right now (see [`resolve`]).
pub fn reduced_now() -> bool {
    REDUCED.load(std::sync::atomic::Ordering::Relaxed)
}

fn apply(cx: &mut App) {
    let Some(state) = cx.try_global::<MotionState>() else { return };
    let reduced = resolve(state.preference, state.system, state.pause_in_background, state.active);
    REDUCED.store(reduced, std::sync::atomic::Ordering::Relaxed);
    if cx.reduce_motion() != reduced {
        cx.set_reduce_motion(reduced);
    }
}

#[cfg(target_os = "macos")]
fn refresh_system(cx: &mut App) {
    use objc::runtime::{NO, Object};
    use objc::{class, msg_send, sel, sel_impl};
    // SAFETY: a read-only query of the shared workspace on the main thread.
    let reduce = unsafe {
        let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        let reduce: objc::runtime::BOOL = msg_send![workspace, accessibilityDisplayShouldReduceMotion];
        reduce != NO
    };
    set_system(reduce, cx);
}

/// "Animation effects" (Settings › Accessibility › Visual effects).
#[cfg(windows)]
fn refresh_system(cx: &mut App) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW};
    let mut enabled: windows_sys::core::BOOL = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to the pointer.
    let read = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut enabled).cast(), 0) };
    set_system(read != 0 && enabled == 0, cx);
}

/// GNOME's `enable-animations`, read off the UI thread with `gsettings`;
/// desktops without it read as no preference.
#[cfg(target_os = "linux")]
fn refresh_system(cx: &mut App) {
    use gpui::AppContext as _;
    let read = cx.background_spawn(async {
        let out = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "enable-animations"])
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&out.stdout).trim() == "false")
    });
    cx.spawn(async move |cx| {
        let reduce = read.await.unwrap_or(false);
        cx.update(|cx| set_system(reduce, cx));
    })
    .detach();
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn refresh_system(_cx: &mut App) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezier_endpoints_and_monotonic() {
        for curve in [EASE, EASE_OUT, EASE_OUT_EXPO, EASE_OUT_QUINT, EASE_IN_OUT, EASE_TAILWIND, EASE_OUT_SOFT] {
            assert_eq!(curve.eval(0.0), 0.0);
            assert_eq!(curve.eval(1.0), 1.0);
            let mut last = 0.0;
            for i in 1..100 {
                let v = curve.eval(i as f32 / 100.0);
                assert!(v >= last - 1e-5, "non-monotonic");
                assert!((0.0..=1.0).contains(&v));
                last = v;
            }
        }
    }

    #[test]
    fn spec_progress_honours_delay() {
        let spec = MotionSpec::new(100, EASE).with_delay(100);
        assert_eq!(spec.progress(0.25), 0.0);
        assert_eq!(spec.progress(1.0), 1.0);
        assert!(spec.progress(0.75) > 0.0);
    }

    #[test]
    fn resolve_combines_inputs() {
        assert!(resolve(ReduceMotion::System, true, false, true));
        assert!(!resolve(ReduceMotion::System, false, false, true));
        assert!(resolve(ReduceMotion::On, false, false, true));
        assert!(!resolve(ReduceMotion::Off, true, false, true));
        assert!(resolve(ReduceMotion::Off, false, true, false));
    }

    #[test]
    fn reduce_motion_ids_round_trip() {
        for m in ReduceMotion::ALL {
            assert_eq!(ReduceMotion::from_id(Some(m.id())), m);
        }
        assert_eq!(ReduceMotion::from_id(None), ReduceMotion::System);
    }

    #[test]
    fn fades_snap_when_reduced_and_prune_unread() {
        let mut f = Fades::default();
        let now = Instant::now();
        f.set("a", true, HOVER_FADE, true, now);
        assert_eq!(f.value("a", now), 1.0);
        f.tick(now, false);
        f.tick(now, false);
        f.tick(now, false);
        assert!(f.entries.is_empty());
    }

    #[test]
    fn state_first_read_snaps() {
        let mut f = Fades::default();
        let now = Instant::now();
        assert_eq!(f.state("g", true, GLYPH_STATE, false, now), 1.0);
        assert_eq!(f.state("g", false, GLYPH_STATE, false, now), 1.0);
    }

    #[test]
    fn resize_clamps_and_reports_edge() {
        assert_eq!(clamp_resize(10.0, 20.0, 40.0), (20.0, Some(ResizeEdge::Min)));
        assert_eq!(clamp_resize(30.0, 20.0, 40.0), (30.0, None));
        assert_eq!(clamp_resize(50.0, 20.0, 40.0), (40.0, Some(ResizeEdge::Max)));
    }

    #[test]
    fn mix_endpoints() {
        let a = gpui::hsla(0.0, 0.0, 0.0, 0.0);
        let b = gpui::hsla(0.0, 0.0, 1.0, 1.0);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
    }
}
