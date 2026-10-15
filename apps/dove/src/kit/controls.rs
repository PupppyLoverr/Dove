//! Controls: buttons in five tones, the toggle switch, a segmented control
//! with a sliding indicator, colour swatches, the select dropdown, text
//! fields and a slider. Metrics and motion follow Zeron's settings widgets
//! (MIT); every control is keyboard operable and shows a focus ring only
//! when reached from the keyboard.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, App, Bounds, Context, Div, ElementId, Entity, Hsla, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement as _, Pixels, RenderOnce, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, anchored, canvas, deferred, div, px, relative,
};
use gpui_base::input::{Input, InputState};

use crate::assets::{icon, icons};
use crate::design::{radius, size, space, ty};
use crate::kit::popover::{self, Popup};
use crate::motion;
use crate::theme::{self, Theme};

// ---------------------------------------------------------------------------
// Buttons
// ---------------------------------------------------------------------------

/// Button tones, strongest first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// The page's one primary action: a solid plate.
    Solid,
    /// A bordered secondary action.
    Outlined,
    /// A filled wash, for actions inside cards.
    Filled,
    /// Text only, for low-emphasis actions.
    Quiet,
    /// Destructive.
    Danger,
}

/// A text button. 30 px tall (26 px when `small`), 8 px corners.
pub fn button(
    t: &Theme,
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    tone: Tone,
) -> gpui::Stateful<Div> {
    let label: SharedString = label.into();
    let (bg, fg, hover, border) = match tone {
        Tone::Solid => (t.solid, t.on_solid, t.solid.opacity(0.88), None),
        Tone::Outlined => (theme::wash(0.0), t.text, t.wash(0.07), Some(t.border_strong)),
        Tone::Filled => (t.wash(0.07), t.text, t.wash(0.11), None),
        Tone::Quiet => (theme::wash(0.0), t.text_muted, t.wash(0.08), None),
        Tone::Danger => (t.danger.opacity(0.12), t.danger, t.danger.opacity(0.18), None),
    };
    let pressed = match tone {
        Tone::Solid => t.solid.opacity(0.78),
        Tone::Danger => t.danger.opacity(0.24),
        _ => t.wash(0.14),
    };
    let text_hover = if tone == Tone::Quiet { t.text } else { fg };
    div()
        .id(ElementId::Name(id.into()))
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .h(px(30.0))
        .px(px(space::MD))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .rounded(px(radius::SELECT))
        .bg(bg)
        .when_some(border, |d, b| d.border_1().border_color(b))
        .text_size(ty::SMALL)
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(fg)
        .cursor_pointer()
        .hover(move |s| s.bg(hover).text_color(text_hover))
        .active(move |s| s.bg(pressed))
        .child(label)
}

/// A smaller [`button`] for dense rows.
pub fn button_sm(
    t: &Theme,
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    tone: Tone,
) -> gpui::Stateful<Div> {
    button(t, id, label, tone).h(px(26.0)).px(px(10.0))
}

/// Make a control reachable with Tab and ring it when focused from the keyboard.
pub fn focusable(el: gpui::Stateful<Div>, t: &Theme) -> gpui::Stateful<Div> {
    let ring = t.accent;
    el.tab_index(0).focus_visible(move |s| s.border_1().border_color(ring))
}

/// A disabled look: dimmed, no pointer, with the reason as a tooltip.
pub fn disabled(el: gpui::Stateful<Div>, reason: impl Into<SharedString>) -> gpui::Stateful<Div> {
    el.opacity(0.45).cursor_default().tooltip(crate::ui::tip(reason))
}

// ---------------------------------------------------------------------------
// Toggle
// ---------------------------------------------------------------------------

pub const TOGGLE_W: f32 = 36.0;
pub const TOGGLE_H: f32 = 22.0;
const TOGGLE_INSET: f32 = 2.0;
const THUMB: f32 = TOGGLE_H - 2.0 * TOGGLE_INSET;

/// A pill switch whose thumb travels over 180 ms (snapping under reduced
/// motion). The caller adds the click; Enter and Space also toggle when it
/// has keyboard focus, through the same handler.
pub fn toggle(t: &Theme, id: impl Into<SharedString>, on: bool) -> gpui::Stateful<Div> {
    let id: SharedString = id.into();
    let ring = t.accent;
    div()
        .id(ElementId::Name(id.clone()))
        .role(gpui::Role::Switch)
        .aria_toggled(if on { gpui::Toggled::True } else { gpui::Toggled::False })
        .flex_none()
        .w(px(TOGGLE_W + 4.0))
        .h(px(TOGGLE_H + 4.0))
        .p(px(2.0))
        .rounded_full()
        .border_1()
        .border_color(gpui::transparent_black())
        .cursor_pointer()
        .tab_index(0)
        .focus_visible(move |s| s.border_color(ring))
        .child(ToggleVisual { theme: t.clone(), on, key: id })
}

#[derive(IntoElement)]
struct ToggleVisual {
    theme: Theme,
    on: bool,
    key: SharedString,
}

struct Travel {
    from: f32,
    target: f32,
    started: Instant,
}

impl Travel {
    fn value(&self, now: Instant) -> f32 {
        motion::lerp(self.from, self.target, motion::TOGGLE.since(self.started, now))
    }
}

impl RenderOnce for ToggleVisual {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = Instant::now();
        let target = if self.on { 1.0 } else { 0.0 };
        let reduced = cx.reduce_motion();
        let key: SharedString = format!("toggle-{}", self.key).into();
        let position = window.with_global_id(ElementId::Name(key), |id, window| {
            window.with_element_state(id, |previous: Option<Travel>, _| {
                let mut travel = previous.unwrap_or(Travel { from: target, target, started: now });
                if travel.target != target {
                    travel = Travel { from: travel.value(now), target, started: now };
                }
                if reduced {
                    travel.from = target;
                    travel.target = target;
                }
                (travel.value(now), travel)
            })
        });
        if (position - target).abs() > 0.001 {
            window.request_animation_frame();
        }
        let t = &self.theme;
        let off = theme::flatten(t.wash(if t.appearance.is_dark() { 0.22 } else { 0.14 }), t.surface);
        let on = t.accent;
        let track = motion::mix(off, on, position);
        let thumb = theme::flatten(gpui::white().opacity(if t.appearance.is_dark() { 0.96 } else { 1.0 }), t.surface);
        div()
            .relative()
            .w(px(TOGGLE_W))
            .h(px(TOGGLE_H))
            .rounded_full()
            .bg(track)
            .border_1()
            .border_color(motion::mix(t.border, on.opacity(0.0), position))
            .child(
                div()
                    .absolute()
                    .top(px(TOGGLE_INSET - 1.0))
                    .left(px(TOGGLE_INSET - 1.0 + (TOGGLE_W - THUMB - 2.0 * TOGGLE_INSET) * position))
                    .size(px(THUMB))
                    .rounded_full()
                    .bg(thumb)
                    .shadow(vec![gpui::BoxShadow {
                        color: gpui::hsla(0.0, 0.0, 0.0, 0.22),
                        offset: gpui::point(px(0.0), px(1.0)),
                        blur_radius: px(2.0),
                        spread_radius: px(0.0),
                        inset: false,
                    }]),
            )
    }
}

/// Wire a toggle's click and keyboard activation to one handler.
pub fn on_toggle<V: 'static>(
    el: gpui::Stateful<Div>,
    cx: &mut Context<V>,
    f: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
) -> gpui::Stateful<Div> {
    let f = Rc::new(f);
    let g = f.clone();
    el.on_click(cx.listener(move |this, _, window, cx| f(this, window, cx))).on_key_down(cx.listener(
        move |this, e: &KeyDownEvent, window, cx| {
            if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                g(this, window, cx);
                cx.stop_propagation();
            }
        },
    ))
}

// ---------------------------------------------------------------------------
// Segmented control
// ---------------------------------------------------------------------------

/// One choice of a segmented control: a label, optionally with a glyph.
pub struct Segment {
    pub label: SharedString,
    pub glyph: Option<&'static str>,
}

impl Segment {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self { label: label.into(), glyph: None }
    }

    pub fn glyph(mut self, glyph: &'static str) -> Self {
        self.glyph = Some(glyph);
        self
    }
}

/// Mutually exclusive choices in equal-width slots, with a selection plate
/// that slides between them (260 ms, snapping under reduced motion). Arrow
/// keys move the choice when it has focus.
pub fn segmented(
    t: &Theme,
    id: impl Into<SharedString>,
    segments: Vec<Segment>,
    selected: usize,
    on_pick: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let id: SharedString = id.into();
    let longest =
        segments.iter().map(|s| s.label.chars().count() + if s.glyph.is_some() { 3 } else { 0 }).max().unwrap_or(4);
    let slot = (longest as f32 * 6.9 + 24.0).max(56.0);
    let n = segments.len().max(1);
    SegmentedControl { theme: t.clone(), id, segments, selected: selected.min(n - 1), slot, on_pick: Rc::new(on_pick) }
        .into_any_element()
}

#[derive(IntoElement)]
struct SegmentedControl {
    theme: Theme,
    id: SharedString,
    segments: Vec<Segment>,
    selected: usize,
    slot: f32,
    on_pick: Rc<dyn Fn(usize, &mut Window, &mut App)>,
}

impl RenderOnce for SegmentedControl {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = &self.theme;
        let n = self.segments.len().max(1);
        let reduced = cx.reduce_motion();
        let target = self.selected as f32;
        let position = window.with_global_id(ElementId::Name(format!("seg-{}", self.id).into()), |id, window| {
            window.with_element_state(id, |previous: Option<motion::IndicatorSlide>, _| {
                let mut slide = previous.unwrap_or_else(|| motion::IndicatorSlide::at(target));
                slide.retarget(target, reduced);
                (slide.value(), slide)
            })
        });
        if (position - target).abs() > 0.001 {
            window.request_animation_frame();
        }
        let width = self.slot * n as f32;
        let pick = self.on_pick.clone();
        let count = self.segments.len();
        let selected = self.selected;
        let ring = t.accent;
        div()
            .id(ElementId::Name(self.id.clone()))
            .role(gpui::Role::RadioGroup)
            .relative()
            .w(px(width + 6.0))
            .h(px(32.0))
            .p(px(2.0))
            .flex()
            .rounded(px(radius::SELECT + 2.0))
            .bg(t.wash(0.05))
            .border_1()
            .border_color(t.wash(0.06))
            .tab_index(0)
            .focus_visible(move |s| s.border_color(ring))
            .on_key_down(move |e: &KeyDownEvent, window, cx| {
                let next = match e.keystroke.key.as_str() {
                    "left" | "up" => Some(selected.saturating_sub(1)),
                    "right" | "down" => Some((selected + 1).min(count.saturating_sub(1))),
                    "home" => Some(0),
                    "end" => Some(count.saturating_sub(1)),
                    _ => None,
                };
                if let Some(i) = next.filter(|i| *i != selected) {
                    pick(i, window, cx);
                    cx.stop_propagation();
                }
            })
            // The plate under the labels, slid by a relative offset.
            .child(
                div()
                    .absolute()
                    .top(px(2.0))
                    .bottom(px(2.0))
                    .left(px(2.0 + position * self.slot))
                    .w(px(self.slot))
                    .rounded(px(radius::SELECT))
                    .bg(t.surface_raised)
                    .shadow(crate::ui::selected_ring(t)),
            )
            .children(self.segments.into_iter().enumerate().map(|(i, seg)| {
                let on = i == self.selected;
                let pick = self.on_pick.clone();
                let label = seg.label.clone();
                div()
                    .id(ElementId::Name(format!("{}-{i}", self.id).into()))
                    .role(gpui::Role::RadioButton)
                    .aria_selected(on)
                    .aria_label(label.clone())
                    .relative()
                    .w(px(self.slot))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(5.0))
                    .rounded(px(radius::SELECT))
                    .text_size(ty::SMALL)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(if on { t.text } else { t.text_muted })
                    .cursor_pointer()
                    .when(!on, |d| d.hover(|s| s.text_color(t.text)))
                    .when_some(seg.glyph, |d, g| {
                        d.child(icon(g).size(px(13.0)).text_color(if on { t.text } else { t.text_muted }))
                    })
                    .child(label)
                    .on_click(move |_, window, cx| pick(i, window, cx))
            }))
    }
}

// ---------------------------------------------------------------------------
// Swatch
// ---------------------------------------------------------------------------

/// A round colour swatch; the selected one wears a ring and a check.
pub fn swatch(
    t: &Theme,
    id: impl Into<SharedString>,
    color: Hsla,
    selected: bool,
    label: impl Into<SharedString>,
) -> gpui::Stateful<Div> {
    let ring = t.accent;
    div()
        .id(ElementId::Name(id.into()))
        .role(gpui::Role::RadioButton)
        .aria_selected(selected)
        .aria_label(label.into())
        .size(px(28.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_2()
        .border_color(if selected { t.text.opacity(0.85) } else { gpui::transparent_black() })
        .cursor_pointer()
        .tab_index(0)
        .focus_visible(move |s| s.border_color(ring))
        .child(
            div()
                .size(px(20.0))
                .rounded_full()
                .bg(color)
                .flex()
                .items_center()
                .justify_center()
                .when(selected, |d| d.child(icon(icons::CHECK).size(px(11.0)).text_color(gpui::white()))),
        )
}

// ---------------------------------------------------------------------------
// Text field
// ---------------------------------------------------------------------------

/// A single-line field: 32 px, 8 px corners, a wash fill and an accent edge
/// while focused.
pub fn text_field(input: &Entity<InputState>, width: Option<f32>, focused: bool, t: &Theme) -> Div {
    div()
        .when_some(width, |d, w| d.w(px(w)))
        .when(width.is_none(), |d| d.w_full())
        .h(px(size::FIELD))
        .px(px(10.0))
        .flex()
        .items_center()
        .rounded(px(radius::SELECT))
        .bg(t.wash(0.06))
        .border_1()
        .border_color(if focused { t.accent.opacity(0.7) } else { t.border })
        .text_size(ty::BODY)
        .child(div().flex_1().min_w_0().child(Input::new(input)))
}

// ---------------------------------------------------------------------------
// Select
// ---------------------------------------------------------------------------

/// One option of a [`Select`].
#[derive(Clone)]
pub struct SelectOption {
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub glyph: Option<&'static str>,
    pub swatch: Option<Hsla>,
}

impl SelectOption {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self { label: label.into(), detail: None, glyph: None, swatch: None }
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn glyph(mut self, glyph: &'static str) -> Self {
        self.glyph = Some(glyph);
        self
    }

    pub fn swatch(mut self, color: Hsla) -> Self {
        self.swatch = Some(color);
        self
    }
}

/// Open state of one [`Select`], owned by the view that renders it.
#[derive(Default)]
pub struct SelectState {
    popup: Popup<()>,
    highlighted: Option<usize>,
}

impl SelectState {
    pub fn is_open(&self) -> bool {
        self.popup.is_open()
    }
}

type Reach<V> = Rc<dyn Fn(&mut V) -> &mut SelectState>;
type OnSelect<V> = Rc<dyn Fn(&mut V, usize, &mut Window, &mut Context<V>)>;

/// A dropdown: a filled trigger showing the current value and a chevron,
/// opening a menu with a check on the current option. Pointer: the trigger
/// toggles and an outside press closes. Keyboard (trigger focused): ↑/↓
/// open and move, Home/End jump, Enter/Space open or commit, Escape closes.
pub struct Select<V: 'static> {
    id: SharedString,
    label: SharedString,
    options: Vec<SelectOption>,
    selected: usize,
    width: Option<f32>,
    menu_width: Option<f32>,
    reach: Reach<V>,
    on_select: Option<OnSelect<V>>,
}

pub fn select<V: 'static>(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    reach: impl Fn(&mut V) -> &mut SelectState + 'static,
) -> Select<V> {
    Select {
        id: id.into(),
        label: label.into(),
        options: Vec::new(),
        selected: 0,
        width: None,
        menu_width: None,
        reach: Rc::new(reach),
        on_select: None,
    }
}

fn close<V: 'static>(view: &mut V, reach: &Reach<V>, cx: &mut Context<V>) {
    if reach(view).popup.begin_close() {
        let reach = reach.clone();
        popover::reap_popup(cx, move |v: &mut V| &mut reach(v).popup);
    }
    cx.notify();
}

impl<V: 'static> Select<V> {
    pub fn options(mut self, options: impl IntoIterator<Item = SelectOption>, selected: usize) -> Self {
        self.options = options.into_iter().collect();
        self.selected = selected;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn menu_width(mut self, width: f32) -> Self {
        self.menu_width = Some(width);
        self
    }

    pub fn on_select(mut self, f: impl Fn(&mut V, usize, &mut Window, &mut Context<V>) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }

    pub fn render(self, t: &Theme, state: &SelectState, cx: &mut Context<V>) -> AnyElement {
        let Self { id, label, options, selected, width, menu_width, reach, on_select } = self;
        let open = state.popup.is_open();
        let mounted = state.popup.is_mounted();
        let closing = state.popup.closing_since();
        let highlighted = state.highlighted;
        let count = options.len();
        let current = options.get(selected).cloned();
        let ring = t.accent;
        let hover_bg = t.wash(0.10);

        let trigger = {
            let (reach_down, reach_click, reach_key) = (reach.clone(), reach.clone(), reach.clone());
            let on_key = on_select.clone();
            div()
                .id(ElementId::Name(id.clone()))
                .role(gpui::Role::ComboBox)
                .aria_label(SharedString::from(format!(
                    "{label}: {}",
                    current.as_ref().map(|o| o.label.clone()).unwrap_or_default()
                )))
                .aria_expanded(open)
                .when_some(width, |d, w| d.w(px(w)))
                .max_w_full()
                .h(px(size::FIELD))
                .pl(px(10.0))
                .pr(px(8.0))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.0))
                .rounded(px(radius::SELECT))
                .bg(if open { hover_bg } else { t.wash(0.06) })
                .border_1()
                .border_color(gpui::transparent_black())
                .text_size(ty::SMALL)
                .text_color(t.text)
                .cursor_pointer()
                .hover(move |s| s.bg(hover_bg))
                .tab_index(0)
                .focus_visible(move |s| s.border_color(ring))
                .when_some(current.as_ref().and_then(|o| o.swatch), |d, c| {
                    d.child(div().size(px(12.0)).flex_none().rounded_full().bg(c))
                })
                .when_some(current.as_ref().and_then(|o| o.glyph), |d, g| {
                    d.child(icon(g).size(px(14.0)).flex_none().text_color(t.text_muted))
                })
                .child(div().flex_1().min_w_0().truncate().child(current.map(|o| o.label).unwrap_or_default()))
                .child(icon(icons::ALT_ARROW_DOWN).size(px(13.0)).flex_none().text_color(if open {
                    t.text
                } else {
                    t.text_muted
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _, _| reach_down(this).popup.note_trigger_press()),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let state = reach_click(this);
                    if state.popup.take_press_was_open() {
                        return;
                    }
                    if state.popup.is_open() {
                        close(this, &reach_click, cx);
                    } else {
                        state.highlighted = Some(selected);
                        state.popup.open(());
                        cx.notify();
                    }
                }))
                .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                    let key = popover::classify(&e.keystroke.key, e.keystroke.modifiers.control);
                    let state = reach_key(this);
                    if !state.popup.is_open() {
                        if matches!(key, popover::MenuKey::Up | popover::MenuKey::Down | popover::MenuKey::Activate) {
                            state.highlighted = Some(selected);
                            state.popup.open(());
                            cx.stop_propagation();
                            cx.notify();
                        }
                        return;
                    }
                    if key == popover::MenuKey::Close {
                        close(this, &reach_key, cx);
                        cx.stop_propagation();
                        return;
                    }
                    let mut active = state.highlighted;
                    let commit = popover::apply_key(key, &mut active, count);
                    reach_key(this).highlighted = active;
                    if let Some(ix) = commit {
                        close(this, &reach_key, cx);
                        if let Some(f) = &on_key {
                            f(this, ix, window, cx);
                        }
                    }
                    if key != popover::MenuKey::Other {
                        cx.stop_propagation();
                    }
                    cx.notify();
                }))
        };

        let menu = mounted.then(|| {
            let rows = options.into_iter().enumerate().map(|(ix, option)| {
                let reach = reach.clone();
                let on_select = on_select.clone();
                let is_selected = ix == selected;
                popover::menu_row(t, format!("{id}-opt-{ix}"), highlighted == Some(ix))
                    .role(gpui::Role::ListBoxOption)
                    .aria_selected(is_selected)
                    .when_some(option.swatch, |d, c| d.child(div().size(px(12.0)).flex_none().rounded_full().bg(c)))
                    .when_some(option.glyph, |d, g| {
                        d.child(icon(g).size(px(14.0)).flex_none().text_color(t.text_muted))
                    })
                    .child(div().flex_1().min_w_0().truncate().child(option.label))
                    .when_some(option.detail, |d, detail| {
                        d.child(div().flex_none().text_size(ty::META).text_color(t.text_faint).child(detail))
                    })
                    .child(
                        div()
                            .w(px(16.0))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .when(is_selected, |d| d.child(icon(icons::CHECK).size(px(13.0)).text_color(t.accent))),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        close(this, &reach, cx);
                        if let Some(f) = &on_select {
                            f(this, ix, window, cx);
                        }
                    }))
            });
            let reach_out = reach.clone();
            let body = popover::card(t)
                .min_w(px(menu_width.or(width).unwrap_or(160.0).max(160.0)))
                .when_some(menu_width, |d, w| d.w(px(w)))
                .max_h(px(360.0))
                .child(
                    div()
                        .id(ElementId::Name(format!("{id}-menu-list").into()))
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .overflow_y_scroll()
                        .children(rows),
                );
            deferred(
                anchored().anchor(Anchor::TopLeft).snap_to_window_with_margin(px(8.0)).child(
                    div()
                        .mt(px(4.0))
                        .when(closing.is_none(), |d| {
                            d.on_mouse_down_out(cx.listener(move |this, _, _, cx| close(this, &reach_out, cx)))
                        })
                        .child(popover::animate(
                            ElementId::Name(format!("{id}-menu").into()),
                            closing,
                            div().child(crate::ui::frosted(popover::CARD_RADIUS, 16.0, body)),
                        )),
                ),
            )
            .with_priority(3)
        });

        div().relative().flex_none().child(trigger).children(menu).into_any_element()
    }
}

// ---------------------------------------------------------------------------
// Slider
// ---------------------------------------------------------------------------

/// Drag state of one [`slider`], owned by the view.
#[derive(Default)]
pub struct SliderState {
    dragging: bool,
    /// The latest value of the drag in progress, committed on release.
    last: Option<f32>,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

/// A horizontal slider over `[min, max]` in `step`s. Drag or click the
/// track; ←/→ step and Home/End jump when focused. `on_change` gets each new
/// value; `on_commit` runs when a drag ends (persist there).
#[allow(clippy::too_many_arguments)]
pub fn slider<V: 'static>(
    t: &Theme,
    id: impl Into<SharedString>,
    value: f32,
    (min, max, step): (f32, f32, f32),
    width: f32,
    state: &SliderState,
    reach: impl Fn(&mut V) -> &mut SliderState + 'static,
    on_change: impl Fn(&mut V, f32, bool, &mut Window, &mut Context<V>) + 'static,
    cx: &mut Context<V>,
) -> AnyElement {
    let id: SharedString = id.into();
    let span = (max - min).max(f32::EPSILON);
    let frac = ((value - min) / span).clamp(0.0, 1.0);
    let reach = Rc::new(reach);
    let on_change = Rc::new(on_change);
    let snap = move |v: f32| ((v - min) / step).round().mul_add(step, min).clamp(min, max);
    let from_x = {
        let bounds = state.bounds.clone();
        move |x: Pixels| -> Option<f32> {
            let b = bounds.get()?;
            let w = f32::from(b.size.width).max(1.0);
            Some(snap(min + span * (f32::from(x - b.origin.x) / w).clamp(0.0, 1.0)))
        }
    };
    let bounds = state.bounds.clone();
    let ring = t.accent;
    let (r1, r2, r3, r4) = (reach.clone(), reach.clone(), reach.clone(), reach.clone());
    let (c1, c2, c3, c4) = (on_change.clone(), on_change.clone(), on_change.clone(), on_change.clone());
    let (f1, f2) = (from_x.clone(), from_x);
    div()
        .id(ElementId::Name(id))
        .role(gpui::Role::Slider)
        .w(px(width))
        .h(px(28.0))
        .flex()
        .items_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(gpui::transparent_black())
        .cursor_pointer()
        .tab_index(0)
        .focus_visible(move |s| s.border_color(ring))
        .child(
            div()
                .relative()
                .w_full()
                .h(px(4.0))
                .rounded_full()
                .bg(t.wash(0.12))
                .child(div().absolute().left_0().top_0().bottom_0().w(relative(frac)).rounded_full().bg(t.accent))
                .child(
                    div()
                        .absolute()
                        .top(px(-6.0))
                        .left(relative(frac))
                        .ml(px(-8.0))
                        .size(px(16.0))
                        .rounded_full()
                        .bg(gpui::white())
                        .border_1()
                        .border_color(t.border_strong)
                        .shadow_sm(),
                )
                .child(canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {}).absolute().inset_0()),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                let state = r1(this);
                state.dragging = true;
                if let Some(v) = f1(e.position.x) {
                    state.last = Some(v);
                    c1(this, v, false, window, cx);
                }
                cx.notify();
            }),
        )
        .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, window, cx| {
            let state = r2(this);
            if state.dragging && e.pressed_button == Some(MouseButton::Left) {
                if let Some(v) = f2(e.position.x) {
                    state.last = Some(v);
                    c2(this, v, false, window, cx);
                }
            }
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                let state = r3(this);
                if std::mem::take(&mut state.dragging) {
                    let v = state.last.take().unwrap_or(value);
                    c3(this, v, true, window, cx);
                }
            }),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                let state = r4(this);
                if std::mem::take(&mut state.dragging) {
                    let v = state.last.take().unwrap_or(value);
                    c4(this, v, true, window, cx);
                }
            }),
        )
        .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
            let next = match e.keystroke.key.as_str() {
                "left" | "down" => Some(snap(value - step)),
                "right" | "up" => Some(snap(value + step)),
                "home" => Some(min),
                "end" => Some(max),
                _ => None,
            };
            if let Some(v) = next {
                on_change(this, v, true, window, cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
}
