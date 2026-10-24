//! The side pill every terminal wears instead of a header bar: a slim
//! capsule on the terminal's right edge (the agent's mark and a status dot)
//! that opens on hover into a small card naming the session, with its
//! actions as labelled rows. The terminal keeps every row of its height.
//!
//! The owner keeps a [`PillState`] and forwards hover changes to
//! [`hover`], which closes the card a moment after the pointer leaves so a
//! diagonal move between capsule and card never flickers.

use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Hsla, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::assets::icon;
use crate::design::{radius, ty};
use crate::theme::Theme;
use crate::ui;

/// How long the card stays after the pointer leaves it.
const CLOSE_DELAY: Duration = Duration::from_millis(220);

/// Open or closed, and a counter that lets a late close timer see that the
/// pointer came back.
#[derive(Default, Clone, Copy)]
pub struct PillState {
    pub open: bool,
    epoch: u64,
}

/// A hover change on the pill of `this`: open at once, close after
/// [`CLOSE_DELAY`] unless hovered again. `state` finds the pill's state.
pub fn hover<T: 'static>(
    this: &mut T,
    hovered: bool,
    state: impl Fn(&mut T) -> &mut PillState + Copy + 'static,
    cx: &mut Context<T>,
) {
    let s = state(this);
    s.epoch = s.epoch.wrapping_add(1);
    let epoch = s.epoch;
    if hovered {
        if !s.open {
            s.open = true;
            cx.notify();
        }
        return;
    }
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(CLOSE_DELAY).await;
        let _ = this.update(cx, |this, cx| {
            let s = state(this);
            if s.epoch == epoch && s.open {
                s.open = false;
                cx.notify();
            }
        });
    })
    .detach();
}

/// What the pill shows about its session.
pub struct Pill {
    pub id: SharedString,
    pub open: bool,
    /// Running, or exited.
    pub live: bool,
    pub glyph: &'static str,
    pub tint: Hsla,
    pub title: SharedString,
    pub detail: SharedString,
    /// Dictation is listening (a red dot on the capsule).
    pub recording: bool,
}

type Act = Rc<dyn Fn(&mut Window, &mut App)>;

/// One row of the open card.
pub struct Item {
    id: SharedString,
    glyph: &'static str,
    label: SharedString,
    hint: Option<&'static str>,
    tone: Option<Hsla>,
    run: Run,
}

enum Run {
    Click(Act),
    /// Held down while it works (dictation).
    Hold(Act, Act),
}

impl Item {
    pub fn new(
        id: impl Into<SharedString>,
        glyph: &'static str,
        label: impl Into<SharedString>,
        run: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), glyph, label: label.into(), hint: None, tone: None, run: Run::Click(Rc::new(run)) }
    }

    /// A row that acts while pressed (press on mouse down, release on up).
    pub fn hold(
        id: impl Into<SharedString>,
        glyph: &'static str,
        label: impl Into<SharedString>,
        press: impl Fn(&mut Window, &mut App) + 'static,
        release: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            glyph,
            label: label.into(),
            hint: None,
            tone: None,
            run: Run::Hold(Rc::new(press), Rc::new(release)),
        }
    }

    /// A shortcut shown at the row's end.
    pub fn hint(mut self, hint: &'static str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// Tint the row (an active state, a warning).
    pub fn tone(mut self, tone: Hsla) -> Self {
        self.tone = Some(tone);
        self
    }
}

/// The pill, closed or open. Place it with [`overlay`].
pub fn render(
    p: Pill,
    items: Vec<Item>,
    on_hover: impl Fn(&bool, &mut Window, &mut App) + 'static,
    t: &Theme,
) -> AnyElement {
    let status = if p.live { t.success } else { t.text_faint };
    if !p.open {
        return div()
            .id(p.id.clone())
            .w(px(24.0))
            .py(px(8.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(7.0))
            .rounded_full()
            .bg(t.surface_raised.opacity(0.94))
            .border_1()
            .border_color(t.border)
            .shadow_md()
            .cursor_pointer()
            .aria_label(format!("{}: hover for actions", p.title))
            .child(ui::dot(status, 6.0))
            .child(icon(p.glyph).size(px(13.0)).text_color(p.tint))
            .when(p.recording, |d| d.child(ui::dot(t.danger, 6.0)))
            .on_hover(on_hover)
            .into_any_element();
    }
    let rows = items.into_iter().map(|item| {
        let ink = item.tone.unwrap_or(t.text);
        let row = div()
            .id(item.id.clone())
            .h(px(30.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(9.0))
            .rounded(px(radius::RADIUS))
            .cursor_pointer()
            .hover(|s| s.bg(t.wash(0.07)))
            .child(icon(item.glyph).size(px(14.0)).flex_none().text_color(item.tone.unwrap_or(t.text_muted)))
            .child(div().flex_1().min_w_0().truncate().text_size(ty::SMALL).text_color(ink).child(item.label))
            .children(item.hint.map(|h| {
                div()
                    .flex_none()
                    .px(px(5.0))
                    .h(px(18.0))
                    .flex()
                    .items_center()
                    .rounded(px(4.0))
                    .bg(t.wash(0.06))
                    .font_family(t.font_mono.clone())
                    .text_size(ty::MICRO)
                    .text_color(t.text_faint)
                    .child(h)
            }));
        match item.run {
            Run::Click(f) => row
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(move |_, window, cx| f(window, cx))
                .into_any_element(),
            Run::Hold(press, release) => row
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    press(window, cx);
                })
                .on_mouse_up(MouseButton::Left, move |_, window, cx| release(window, cx))
                .into_any_element(),
        }
    });
    let card = div()
        .id(SharedString::from(format!("{}-card", p.id)))
        .w(px(268.0))
        .p(px(6.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .rounded(px(14.0))
        .bg(t.surface_raised)
        .border_1()
        .border_color(t.border)
        .shadow_lg()
        .on_hover(on_hover)
        .child(
            div()
                .px(px(6.0))
                .pt(px(4.0))
                .pb(px(8.0))
                .mb(px(2.0))
                .flex()
                .items_center()
                .gap(px(9.0))
                .border_b_1()
                .border_color(t.border)
                .child(
                    div()
                        .relative()
                        .size(px(28.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(8.0))
                        .bg(t.wash(0.06))
                        .child(icon(p.glyph).size(px(15.0)).text_color(p.tint))
                        .child(div().absolute().right(px(-2.0)).bottom(px(-2.0)).child(ui::dot(status, 7.0))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .text_size(ty::SMALL)
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(t.text)
                                .child(p.title),
                        )
                        .child(div().truncate().text_size(ty::CAPTION).text_color(t.text_faint).child(p.detail)),
                ),
        )
        .children(rows);
    crate::motion::menu_in_from(SharedString::from(format!("{}-in", p.id)), 0.0, card).into_any_element()
}

/// The pill laid over a terminal: vertically centred on its right edge,
/// only as wide as the pill so the terminal keeps its clicks.
pub fn overlay(pill: AnyElement) -> AnyElement {
    div().absolute().top_0().bottom_0().right(px(8.0)).flex().items_center().child(pill).into_any_element()
}
