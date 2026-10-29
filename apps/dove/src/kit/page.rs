//! Page grammar shared by Settings and the full-page routes: a centred
//! reading column, a headline, labelled sections of filled cards, and rows
//! with a title and description on the left and a control on the right.
//! Metrics follow Zeron's settings widgets (MIT).

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Div, ElementId, Entity, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_base::input::{Input, InputState};

use crate::assets::{icon, icons};
use crate::design::{radius, size, space, ty};
use crate::theme::Theme;

/// Section labels and titles sit a little inside the cards' edge.
pub const LABEL_INSET: f32 = 8.0;
/// Air between one section and the next.
pub const SECTION_GAP: f32 = 32.0;

/// The centred reading column (Zeron's 760 px measure).
pub fn page_column() -> Div {
    div()
        .w_full()
        .max_w(px(size::PAGE_MAX))
        .mx_auto()
        .px(px(size::PAGE_PAD_X))
        .pt(px(28.0))
        .pb(px(48.0))
        .flex()
        .flex_col()
}

/// The measure for pages whose tables need more room than the reading
/// column (Threads, Team, Usage).
pub const PAGE_WIDE: f32 = 980.0;
/// Row height of table-like cards (Threads, Archived, Inbox, PRs).
pub const TABLE_ROW: f32 = 40.0;
/// Height of a table card's column header strip.
pub const TABLE_HEAD: f32 = 32.0;

/// [`page_column`] at a custom measure.
pub fn page_column_max(max: f32) -> Div {
    page_column().max_w(px(max))
}

/// A scrolling page: the whole route scrolls under its column.
pub fn page_scroll(id: impl Into<ElementId>) -> Stateful<Div> {
    div().id(id).size_full().overflow_y_scroll()
}

/// A page whose column fills the route's height without scrolling, for
/// pages whose list scrolls on its own (a `uniform_list`): give the list's
/// card `flex_shrink` and `min_h_0` so it takes what is left.
pub fn page_fill(id: impl Into<ElementId>, max: f32) -> (Stateful<Div>, Div) {
    (div().id(id).size_full().flex().flex_col().overflow_hidden(), page_column_max(max).h_full().min_h_0())
}

/// A page's headline block: the title with an optional trailing control,
/// and a muted subtitle under it.
pub fn page_head(
    t: &Theme,
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    trailing: Option<AnyElement>,
) -> Div {
    let subtitle: SharedString = subtitle.into();
    div()
        .flex_none()
        .flex()
        .flex_col()
        .child(page_header(t, title, trailing))
        .when(!subtitle.is_empty(), |d| d.child(page_subtitle(t, subtitle)))
}

/// One row of a table card: 40 px, inset 16, a hairline above every row
/// but the first.
pub fn table_row(t: &Theme, first: bool) -> Div {
    div()
        .flex_none()
        .h(px(TABLE_ROW))
        .px(px(space::LG))
        .flex()
        .items_center()
        .gap(px(space::MD))
        .when(!first, |d| d.border_t_1().border_color(row_divider(t)))
}

/// The column header strip at the top of a table card.
pub fn table_head(t: &Theme) -> Div {
    div()
        .flex_none()
        .h(px(TABLE_HEAD))
        .px(px(space::LG))
        .flex()
        .items_center()
        .gap(px(space::MD))
        .text_size(ty::META)
        .line_height(ty::lh::META)
        .text_color(t.text_muted)
}

/// A page's search field: a magnifier and a borderless input in the field
/// tone.
pub fn search_field(t: &Theme, input: &Entity<InputState>) -> Div {
    div()
        .h(px(size::FIELD))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(space::SM))
        .text_size(ty::BODY)
        .child(icon(icons::MAGNIFER).size(px(14.0)).flex_none().text_color(t.text_faint))
        .child(div().flex_1().min_w_0().child(Input::new(input)))
}

/// A figure tile: a muted label over a large value, in the card fill.
pub fn stat(t: &Theme, label: impl Into<SharedString>, value: impl Into<SharedString>, tint: Option<Hsla>) -> Div {
    div()
        .flex_1()
        .min_w(px(120.0))
        .px(px(space::LG))
        .py(px(space::MD))
        .flex()
        .flex_col()
        .gap(px(space::XS))
        .child(div().text_size(ty::META).line_height(ty::lh::META).text_color(t.text_muted).child(label.into()))
        .child(
            div()
                .text_size(ty::H1)
                .line_height(ty::lh::PAGE)
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(tint.unwrap_or(t.text))
                .child(value.into()),
        )
}

/// An empty state for a failure: the same grammar as [`empty_state`], with
/// the glyph in the danger tone.
pub fn error_state(
    t: &Theme,
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    action: Option<AnyElement>,
) -> Div {
    let body: SharedString = body.into();
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(space::SM))
        .py(px(40.0))
        .px(px(space::LG))
        .child(icon(icons::DANGER_TRIANGLE).size(px(20.0)).text_color(t.danger))
        .child(div().text_size(ty::MD).font_weight(gpui::FontWeight::MEDIUM).text_color(t.text).child(title.into()))
        .when(!body.is_empty(), |d| {
            d.child(div().max_w(px(420.0)).text_center().text_size(ty::BODY).text_color(t.text_faint).child(body))
        })
        .when_some(action, |d, a| d.child(div().mt(px(space::XS)).child(a)))
}

/// A table row that stays flat. Selection is a faint wash; the pointer
/// does not paint a plate over the row.
pub fn row_hover(t: &Theme, el: Stateful<Div>, selected: bool) -> Stateful<Div> {
    el.cursor_pointer().when(selected, |d| d.bg(t.wash(0.045)))
}

/// The page headline, an optional count beside it, and an optional trailing
/// action on the same line.
pub fn page_header(t: &Theme, title: impl Into<SharedString>, trailing: Option<AnyElement>) -> Div {
    div()
        .px(px(LABEL_INSET))
        .flex()
        .items_center()
        .gap(px(10.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(ty::PAGE)
                .line_height(ty::lh::PAGE)
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(t.text)
                .child(title.into()),
        )
        .children(trailing)
}

/// A muted line under the headline.
pub fn page_subtitle(t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .mt(px(space::XS))
        .px(px(LABEL_INSET))
        .min_w_0()
        .text_size(ty::BODY)
        .line_height(ty::lh::BODY)
        .text_color(t.text_muted)
        .child(text.into())
}

/// The quiet label above a card: sentence case, muted.
pub fn section_label(t: &Theme, label: impl Into<SharedString>) -> Div {
    div().px(px(LABEL_INSET)).text_size(ty::BODY).line_height(ty::lh::BODY).text_color(t.text_muted).child(label.into())
}

/// A labelled section: the label, then its block close underneath. An empty
/// label gives just the spacing.
pub fn section(t: &Theme, label: impl Into<SharedString>, block: impl IntoElement) -> Div {
    let label: SharedString = label.into();
    div()
        .mt(px(SECTION_GAP))
        .flex()
        .flex_col()
        .gap(px(space::SM))
        .when(!label.is_empty(), |d| d.child(section_label(t, label)))
        .child(block)
}

/// The card fill: a faint wash of the theme's ink, no outline.
pub fn card_fill(t: &Theme) -> Hsla {
    t.wash(0.045)
}

/// The hairline between rows in a card.
pub fn row_divider(t: &Theme) -> Hsla {
    t.border.opacity(0.6)
}

/// A stack of rows on a card: a hair lighter than the page, a low-contrast
/// hairline, the card radius. Cards part by fill more than by line.
pub fn card(t: &Theme) -> Div {
    div()
        .w_full()
        .overflow_hidden()
        .flex()
        .flex_col()
        .rounded(px(crate::design::radius::CARD))
        .bg(t.surface_card)
        .border_1()
        .border_color(t.border.opacity(0.55))
}

/// One row in a [`card`]: inset 16 px, at least 60 px tall, a hairline above
/// every row but the first. The text side wraps above the control when the
/// card is narrow, instead of squeezing into one word per line.
pub fn row(t: &Theme, first: bool) -> Div {
    div()
        .mx(px(space::LG))
        .py(px(space::MD))
        .min_h(px(size::ROW_MIN))
        .when(!first, |d| d.border_t_1().border_color(row_divider(t)))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(space::LG))
}

/// A row's title and optional description.
pub fn row_text(t: &Theme, title: impl Into<SharedString>, description: impl Into<SharedString>) -> Div {
    let description: SharedString = description.into();
    div()
        .flex_1()
        .min_w(px(180.0))
        .flex()
        .flex_col()
        .gap(px(1.0))
        .child(
            div()
                .text_size(ty::BODY)
                .line_height(ty::lh::BODY)
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(t.text)
                .child(title.into()),
        )
        .when(!description.is_empty(), |d| {
            d.child(div().text_size(ty::META).line_height(ty::lh::META).text_color(t.text_muted).child(description))
        })
}

/// A complete setting row: text on the left, the control on the right. The
/// control never shrinks; on a narrow card it wraps under the text.
pub fn setting_row(
    t: &Theme,
    first: bool,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    control: impl IntoElement,
) -> Div {
    row(t, first)
        .child(row_text(t, title, description))
        .child(div().flex_none().max_w_full().flex().items_center().justify_end().child(control))
}

/// A leading glyph for rows whose icon carries identity (an agent, a device).
pub fn row_tile(t: &Theme, glyph: &'static str, tint: Option<Hsla>) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(icon(glyph).size(px(16.0)).text_color(tint.unwrap_or(t.text_muted)))
}

/// A quiet pill.
pub fn badge(t: &Theme, label: impl Into<SharedString>) -> Div {
    div().flex_none().text_size(ty::BADGE).text_color(t.text_muted).child(label.into())
}

/// A pill in a status colour.
pub fn badge_tone(label: impl Into<SharedString>, color: Hsla) -> Div {
    div().flex_none().text_size(ty::BADGE).font_weight(gpui::FontWeight::MEDIUM).text_color(color).child(label.into())
}

fn strip(glyph: &'static str, color: Hsla, t: &Theme, text: impl Into<SharedString>) -> Div {
    div()
        .w_full()
        .min_w_0()
        .px(px(space::MD))
        .py(px(space::SM))
        .flex()
        .items_start()
        .gap(px(space::SM))
        .text_size(ty::META)
        .line_height(ty::lh::META)
        .text_color(t.text)
        .child(icon(glyph).size(px(14.0)).flex_none().mt(px(1.0)).text_color(color))
        // Long messages (a tool's stderr) wrap and stay inside the strip.
        .child(div().flex_1().min_w_0().overflow_hidden().child(text.into()))
}

/// An inline error that wraps inside its own bounds.
pub fn error_strip(t: &Theme, text: impl Into<SharedString>) -> Div {
    strip(icons::CLOSE_CIRCLE, t.danger, t, text)
}

/// An inline note.
pub fn info_strip(t: &Theme, text: impl Into<SharedString>) -> Div {
    strip(icons::INFO_CIRCLE, t.text_muted, t, text)
}

/// A centred empty state: a quiet glyph tile, a title, a body line and an
/// optional action.
pub fn empty_state(
    t: &Theme,
    glyph: &'static str,
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    action: Option<AnyElement>,
) -> Div {
    let body: SharedString = body.into();
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(space::SM))
        .py(px(40.0))
        .px(px(space::LG))
        .child(icon(glyph).size(px(20.0)).text_color(t.text_faint))
        .child(div().text_size(ty::MD).font_weight(gpui::FontWeight::MEDIUM).text_color(t.text).child(title.into()))
        .when(!body.is_empty(), |d| {
            d.child(div().max_w(px(380.0)).text_center().text_size(ty::BODY).text_color(t.text_faint).child(body))
        })
        .when_some(action, |d, a| d.child(div().mt(px(space::XS)).child(a)))
}

/// A placeholder bar for content that is loading, gently pulsing on the
/// shared clock (still under reduced motion).
pub fn skeleton(t: &Theme, width: gpui::DefiniteLength, height: f32) -> AnyElement {
    let base = t.wash(0.06);
    let lift = t.wash(0.11);
    crate::motion::pulse(std::time::Duration::from_millis(1600), 2, move |phase| {
        let k = if phase.still { 0.0 } else { crate::motion::pulse_wave(phase.phase) };
        div()
            .w(width)
            .h(px(height))
            .rounded(px(radius::RADIUS))
            .bg(crate::motion::mix(base, lift, k))
            .into_any_element()
    })
    .into_any_element()
}

/// A few skeleton lines standing in for a block of text.
pub fn skeleton_lines(t: &Theme, lines: usize) -> Div {
    let widths = [0.92, 0.78, 0.86, 0.64, 0.72];
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .children((0..lines).map(|i| skeleton(t, gpui::relative(widths[i % widths.len()]), 12.0)))
}
