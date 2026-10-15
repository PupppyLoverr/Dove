//! Small shared components on top of `ui.rs`: keyboard hints, ring meters,
//! switches, checkboxes, segmented controls, status glyphs, skeletons,
//! empty states, toasts and resize handles.
//!
//! Anything animated requests frames only while it is on screen and moving,
//! so an idle window still renders nothing.

use std::time::Duration;

use dove_core::proto::TaskStatus;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, Bounds, Context, Div, ElementId, Entity, Global,
    Hsla, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, PathBuilder, Pixels, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, canvas, div, point, px,
};

use crate::assets::{icon, icons};
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
        .text_size(px(10.5))
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
        .text_size(px(13.0))
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

/// A row of mutually exclusive options. `on_pick` gets the index.
pub fn segmented(
    id: &str,
    options: &[&str],
    selected: usize,
    t: &Theme,
    on_pick: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
) -> Div {
    div()
        .flex()
        .gap(px(2.0))
        .p(px(2.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(t.wash(0.08))
        .bg(t.wash(0.04))
        .children(options.iter().enumerate().map(|(i, label)| {
            let on = i == selected;
            let pick = on_pick.clone();
            div()
                .id(SharedString::from(format!("{id}-{i}")))
                .role(gpui::Role::RadioButton)
                .aria_selected(on)
                .aria_label(label.to_string())
                .h(px(24.0))
                .px(px(11.0))
                .flex()
                .items_center()
                .rounded(px(8.0))
                .text_size(px(12.5))
                .font_weight(gpui::FontWeight::MEDIUM)
                .cursor_pointer()
                .when(on, |d| d.bg(t.surface_raised).text_color(t.text).shadow_sm())
                .when(!on, |d| d.text_color(t.text_muted).hover(|s| s.text_color(t.text)))
                .child(label.to_string())
                .on_click(move |_, window, cx| pick(i, window, cx))
        }))
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

/// A rotating arc.
/// Zeron's activity glyph: a 2×3 grid of round cells whose brightness
/// chases around the ring, sized to fit a `size` square.
pub fn spinner(id: impl Into<ElementId>, size: f32, color: Hsla) -> impl IntoElement {
    // Clockwise ring position of each (row, col), top-left first.
    const RING: [[usize; 2]; 3] = [[0, 1], [5, 2], [4, 3]];
    let id: ElementId = id.into();
    let key = format!("{id:?}");
    let cell = (size / 4.0).max(2.0);
    let grid = div().flex().flex_col().gap(px(cell / 2.0)).children((0..3).map(|row| {
        let key = key.clone();
        div().flex().gap(px(cell / 2.0)).children((0..2).map(move |col| {
            let phase = RING[row][col] as f32 / 6.0;
            div().size(px(cell)).rounded_full().bg(color).with_animation(
                SharedString::from(format!("{key}-{row}{col}")),
                Animation::new(Duration::from_millis(750)).repeat(),
                // Zeron's mono spinner: the same gspin pulse and ring.
                move |el, delta| el.opacity(crate::loaders::gspin_opacity(delta + phase, 0.1)),
            )
        }))
    }));
    div().size(px(size)).flex().flex_none().items_center().justify_center().child(grid)
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
                .rounded(px(12.0))
                .bg(t.wash(0.05))
                .child(icon(glyph).size(px(20.0)).text_color(t.text_faint)),
        )
        .child(
            div().text_size(px(14.0)).font_weight(gpui::FontWeight::MEDIUM).text_color(t.text).child(title.to_string()),
        )
        .child(
            div().max_w(px(360.0)).text_center().text_size(px(13.0)).text_color(t.text_faint).child(body.to_string()),
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
        .rounded(px(5.0))
        .bg(color.opacity(0.12))
        .text_color(color)
        .text_size(px(11.0))
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
        .rounded(px(12.0))
        .border_1()
        .border_color(t.border)
        .bg(t.wash(0.022))
        .overflow_hidden()
}

/// A row's label with an optional one-line hint under it.
pub fn label_block(label: impl Into<SharedString>, hint: impl Into<SharedString>, t: &Theme) -> Div {
    let hint: SharedString = hint.into();
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(div().text_size(px(13.5)).text_color(t.text).child(label.into()))
        .when(!hint.is_empty(), |d| d.child(div().text_size(px(12.0)).text_color(t.text_faint).child(hint)))
}

/// A section heading above a group: small, quiet, never shouting.
pub fn section_label(text: impl Into<SharedString>, t: &Theme) -> Div {
    div()
        .px(px(2.0))
        .pb(px(8.0))
        .text_size(px(12.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(t.text_faint)
        .child(text.into())
}

/// The box around a single-line text input.
pub fn field_box(t: &Theme) -> Div {
    div()
        .h(px(34.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(t.border_strong)
        .bg(t.wash(0.03))
        .text_size(px(13.0))
}

/// The box around a multi-line text area.
pub fn area_box(t: &Theme) -> Div {
    div()
        .px(px(12.0))
        .py(px(10.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(t.border_strong)
        .bg(t.wash(0.03))
        .text_size(px(13.0))
}

/// A labelled form field: the label above, the control under it.
pub fn field(label: impl Into<SharedString>, control: impl IntoElement, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(div().text_size(px(12.5)).text_color(t.text_muted).child(label.into()))
        .child(control)
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
        .rounded(px((size * 0.28).max(4.0)))
        .bg(bg)
        .text_color(fg)
        .text_size(px(size * 0.52))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(label.into())
}

/// A bordered secondary button.
pub fn button_outline(id: impl Into<SharedString>, label: impl Into<SharedString>, t: &Theme) -> gpui::Stateful<Div> {
    let label: SharedString = label.into();
    let hover = t.wash(0.07);
    div()
        .id(gpui::ElementId::Name(id.into()))
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .h(px(30.0))
        .px(px(12.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(t.border_strong)
        .text_size(px(13.0))
        .text_color(t.text)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(label)
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

#[derive(Clone)]
pub struct Toast {
    pub id: u64,
    pub title: SharedString,
    pub body: Option<SharedString>,
    pub tone: Tone,
    /// Clicking the toast opens this thread, when set.
    pub thread: Option<dove_core::proto::TaskId>,
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

/// The app's toast stack, a global entity the shell renders.
pub struct Toasts {
    items: Vec<Toast>,
    next: u64,
    pub on_open: Option<OpenTask>,
}

struct ToastsGlobal(Entity<Toasts>);
impl Global for ToastsGlobal {}

impl Toasts {
    pub fn init(cx: &mut App) -> Entity<Toasts> {
        let e = cx.new(|_| Toasts { items: Vec::new(), next: 1, on_open: None });
        cx.set_global(ToastsGlobal(e.clone()));
        e
    }

    /// Show a toast for five seconds.
    pub fn push(
        cx: &mut App,
        title: impl Into<SharedString>,
        body: Option<String>,
        tone: Tone,
        thread: Option<dove_core::proto::TaskId>,
    ) {
        let Some(e) = cx.try_global::<ToastsGlobal>().map(|g| g.0.clone()) else { return };
        e.update(cx, |t, cx| {
            let id = t.next;
            t.next += 1;
            t.items.push(Toast { id, title: title.into(), body: body.map(Into::into), tone, thread });
            if t.items.len() > 4 {
                t.items.remove(0);
            }
            cx.notify();
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                let _ = this.update(cx, |t, cx| {
                    t.items.retain(|x| x.id != id);
                    cx.notify();
                });
            })
            .detach();
        });
    }

    fn dismiss(&mut self, id: u64, cx: &mut Context<Self>) {
        self.items.retain(|x| x.id != id);
        cx.notify();
    }
}

impl Render for Toasts {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(cx).clone();
        div().absolute().bottom(px(16.0)).right(px(16.0)).flex().flex_col().gap(px(8.0)).children(
            self.items.iter().map(|toast| {
                let id = toast.id;
                let color = match toast.tone {
                    Tone::Info => t.text_muted,
                    Tone::Success => t.success,
                    Tone::Warning => t.warning,
                    Tone::Danger => t.danger,
                };
                let thread = toast.thread;
                let open = self.on_open.clone();
                ui::frosted(
                    12.0,
                    18.0,
                    div()
                        .id(SharedString::from(format!("toast-{id}")))
                        .w(px(320.0))
                        .p(px(12.0))
                        .flex()
                        .items_start()
                        .gap(px(10.0))
                        .rounded(px(12.0))
                        .bg(t.glass_overlay())
                        .border_1()
                        .border_color(t.border_strong)
                        .shadow_lg()
                        .when(thread.is_some(), |d| d.cursor_pointer())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let (Some(task), Some(open)) = (thread, open.clone()) {
                                open(task, window, cx);
                            }
                            this.dismiss(id, cx);
                        }))
                        .child(ui::dot(color, 8.0).mt(px(5.0)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(t.text)
                                        .child(toast.title.clone()),
                                )
                                .when_some(toast.body.clone(), |d, b| {
                                    d.child(div().text_size(px(12.5)).text_color(t.text_muted).line_clamp(2).child(b))
                                }),
                        ),
                )
            }),
        )
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
}
