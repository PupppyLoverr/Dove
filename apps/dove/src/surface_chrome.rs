//! Shared metrics and pieces for side-panel surfaces (browser, terminal,
//! diffs, files): toolbars, toolbar buttons, the tab chips over the panel,
//! and small popover helpers. Ported from Zeron's `surface_chrome` and
//! `popover` (MIT).

use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Div, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Point, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::theme::{Theme, wash};

pub const HEADER_HEIGHT: f32 = 38.0;
pub const CONTROL_SIZE: f32 = 24.0;
pub const CONTROL_RADIUS: f32 = 6.0;
pub const ICON_SIZE: f32 = 14.0;
pub const CONTROL_GAP: f32 = 4.0;
pub const EDGE_INSET: f32 = 8.0;

/// The field treatment shared by file search and the browser address bar.
pub fn input() -> Div {
    div()
        .h(px(CONTROL_SIZE))
        .min_w_0()
        .flex_1()
        .px(px(8.0))
        .rounded(px(CONTROL_RADIUS))
        .bg(crate::theme::ink(0.035))
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_size(px(11.5))
}

pub fn toolbar(theme: &Theme) -> Div {
    div()
        .h(px(HEADER_HEIGHT))
        .w_full()
        .flex_none()
        .px(px(EDGE_INSET))
        .flex()
        .items_center()
        .gap(px(CONTROL_GAP))
        .border_t_1()
        .border_b_1()
        .border_color(theme.border)
        .bg(if theme.is_frost() { theme.surface.opacity(0.26) } else { theme.surface })
}

/// A square icon button in a surface toolbar: tooltip, a11y label, and a
/// focus-preserving mouse-down.
pub fn toolbar_button(id: impl Into<gpui::ElementId>, label: &'static str) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .size(px(CONTROL_SIZE))
        .flex_none()
        .rounded(px(CONTROL_RADIUS))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .role(gpui::Role::Button)
        .aria_label(label)
        .occlude()
        .on_mouse_down(gpui::MouseButton::Left, |_, window, _| window.prevent_default())
        .hover(|style| style.bg(wash(0.14)))
        .tooltip(crate::ui::tip(label))
        .tooltip_show_delay(Duration::from_millis(350))
}

/// A neutral chip: the right-panel tab strip and page navigation.
/// Matches Zeron's `surface_chrome::tab_frame` exactly.
pub fn tab_frame(id: impl Into<gpui::ElementId>, selected: bool, theme: &Theme) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .h(px(CONTROL_SIZE))
        .flex_none()
        .rounded(px(CONTROL_RADIUS))
        .flex()
        .items_center()
        .gap(px(CONTROL_GAP))
        .cursor_pointer()
        .role(gpui::Role::Button)
        .aria_selected(selected)
        .text_size(px(12.0))
        .text_color(if selected { theme.text } else { theme.text_muted })
        .when(selected, |el| el.bg(wash(0.10)))
}

pub fn tab(id: impl Into<gpui::ElementId>, selected: bool, theme: &Theme) -> gpui::Stateful<Div> {
    tab_frame(id, selected, theme).when(!selected, |el| el.hover(|style| style.bg(wash(0.06))))
}

// ------------------------------------------------------------------ popovers

#[allow(dead_code)] // Zeron parity
pub fn popover_card(theme: &Theme) -> Div {
    div()
        .border_1()
        .border_color(theme.border)
        .rounded(px(12.0))
        .when(!theme.is_frost(), |el| el.shadow_lg())
        .bg(theme.surface_overlay)
        .p(px(4.0))
        .gap(px(2.0))
        .overflow_hidden()
        .text_size(px(13.0))
        .text_color(theme.text)
}

#[allow(dead_code)] // Zeron parity
pub fn menu_row(theme: &Theme, active: bool, _key: impl Into<SharedString>) -> Div {
    let row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(7.0))
        .text_size(px(13.0))
        .cursor_pointer();
    if active {
        row.bg(wash(0.12)).text_color(theme.text)
    } else {
        row.text_color(theme.text_muted).hover(|s| s.bg(wash(0.08)))
    }
}

#[allow(dead_code)] // Zeron parity
pub fn menu_at(
    id: impl Into<SharedString>,
    position: Point<Pixels>,
    content: AnyElement,
    _closing: Option<std::time::Instant>,
) -> AnyElement {
    let id: SharedString = id.into();
    gpui::deferred(
        gpui::anchored()
            .position(position)
            .anchor(gpui::Anchor::TopLeft)
            .snap_to_window_with_margin(px(8.0))
            .child(div().id(id).occlude().child(content)),
    )
    .priority(1)
    .into_any_element()
}
