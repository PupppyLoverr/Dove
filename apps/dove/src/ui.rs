//! Shared presentation primitives. The frost, layer, rounded-clip elements
//! and the control styles are adapted from Zeron (MIT).

use dove_core::proto::{HarnessKind, TaskStatus, Usage};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AnyView, App, AppContext as _, Bounds, ClickEvent, Context, Corners, Div, Element, GlobalElementId,
    Hsla, InspectorElementId, InteractiveElement, IntoElement, LayoutId, MouseButton, ParentElement, Pixels, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, WindowControlArea, div, hsla, px,
};

use crate::assets::{icon, icons};
use crate::design::{radius, ty};
use crate::theme::Theme;

// ---------------------------------------------------------------- elements

/// Paints `child` in its own scene layer with a backdrop blur first, so the
/// card reads as frosted glass. Pass-through when the theme is opaque.
pub fn frosted(corner_radius: f32, blur_radius: f32, child: impl IntoElement) -> Frosted {
    Frosted { corner_radius, blur_radius, child: child.into_any_element() }
}

pub struct Frosted {
    corner_radius: f32,
    blur_radius: f32,
    child: AnyElement,
}

impl Element for Frosted {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
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
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if Theme::of(cx).is_frost() {
            window.paint_layer(bounds, |window| {
                window.paint_backdrop_blur(bounds, Corners::all(px(self.corner_radius)), px(self.blur_radius));
                self.child.paint(window, cx);
            });
        } else {
            self.child.paint(window, cx);
        }
    }
}

impl IntoElement for Frosted {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// Fades `child`'s content out toward its bottom edge, per pixel: fully gone
/// in the last `clear` px, ramping back to full over the `band` above that.
/// For content sliding under floating glass, where no painted overlay could
/// hide it (what is behind the window is not a paintable color). Adapted
/// from Zeron's `edge_faded`.
pub fn bottom_faded(clear: f32, band: f32, child: impl IntoElement) -> BottomFaded {
    BottomFaded { clear, band, child: child.into_any_element() }
}

pub struct BottomFaded {
    clear: f32,
    band: f32,
    child: AnyElement,
}

impl Element for BottomFaded {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
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
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut region = bounds;
        region.size.height = (region.size.height - px(self.clear)).max(px(0.0));
        let fade = gpui::EdgeFade {
            bounds: region,
            band: px(self.band),
            band_top: None,
            band_bottom: None,
            band_left: None,
            band_right: None,
            top: false,
            bottom: true,
            left: false,
            right: false,
        };
        window.with_edge_fade(Some(fade), |window| self.child.paint(window, cx));
    }
}

impl IntoElement for BottomFaded {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// Clips a subtree to its bounds with rounded corners, per pixel, so content
/// scrolling inside a rounded card on glass never pokes past the arcs.
pub fn rounded_clip(radius: f32, child: impl IntoElement) -> RoundedClip {
    RoundedClip { radius, child: child.into_any_element() }
}

pub struct RoundedClip {
    radius: f32,
    child: AnyElement,
}

impl Element for RoundedClip {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
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
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| self.child.prepaint(window, cx));
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_rounded_clip(bounds, px(self.radius), |window| self.child.paint(window, cx));
    }
}

impl IntoElement for RoundedClip {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

// ----------------------------------------------------------------- tooltip

pub struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("Tip");
        let t = Theme::of(cx);
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(radius::RADIUS))
            .bg(t.surface_overlay)
            .border_1()
            .border_color(t.border)
            .text_color(t.text)
            .text_size(ty::META)
            .child(self.0.clone())
    }
}

pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
}

// ---------------------------------------------------------------- controls

/// Icon buttons follow Zeron's two shapes:
/// - square: [`BUTTON`] px, [`BUTTON_RADIUS`] corners. Title bar glyphs are
///   [`ICON_LG`], glyphs in rows and toolbars [`ICON_SM`] (see [`row_button`]).
/// - round: [`ROUND_BUTTON`] px, for the composer (see [`round_button`]).
///
/// All share the same hover wash and a slightly deeper pressed one.
pub const BUTTON: f32 = 24.0;
pub const BUTTON_RADIUS: f32 = radius::CONTROL;
pub const ROUND_BUTTON: f32 = 28.0;
pub const ICON_LG: f32 = 16.0;
pub const ICON_SM: f32 = 14.0;

/// A 24px window-chrome button (sidebar toggle, history, new). Occludes the
/// titlebar drag strip so clicks never start a window move.
pub fn window_button(
    id: impl Into<SharedString>,
    glyph: impl IntoElement,
    label: &'static str,
    theme: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<Div> {
    square_button(id, label, theme)
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(move |e, window, cx| {
            cx.stop_propagation();
            on_click(e, window, cx)
        })
        .child(glyph)
}

pub fn icon_button(
    id: impl Into<SharedString>,
    path: &'static str,
    label: &'static str,
    theme: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<Div> {
    window_button(id, icon(path).size(px(ICON_LG)).text_color(theme.text_muted), label, theme, on_click)
}

/// The bare square button: size, corners, hover and pressed washes, a11y
/// label and tooltip. Callers add the glyph and the click.
pub fn square_button(id: impl Into<SharedString>, label: &'static str, theme: &Theme) -> gpui::Stateful<Div> {
    let (hover, pressed) = (theme.glass_hover(), theme.wash(0.14));
    div()
        .id(gpui::ElementId::Name(id.into()))
        .size(px(BUTTON))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(BUTTON_RADIUS))
        .cursor_pointer()
        .role(gpui::Role::Button)
        .aria_label(label)
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(pressed))
        .tooltip(tip(label))
}

/// A small icon button for rows and toolbars (copy, edit, retry…): the
/// square shape with a [`ICON_SM`] glyph. Keeps focus where it was.
pub fn row_button(
    id: impl Into<SharedString>,
    path: &'static str,
    label: &'static str,
    theme: &Theme,
) -> gpui::Stateful<Div> {
    square_button(id, label, theme)
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .child(icon(path).size(px(ICON_SM)).text_color(theme.text_muted))
}

/// [`round_button`] at a chosen diameter. The bottom-center composer uses a
/// smaller hit target so the field stays a single slim row.
pub fn round_button_sized(
    id: impl Into<SharedString>,
    label: &'static str,
    size: f32,
    theme: &Theme,
) -> gpui::Stateful<Div> {
    let (hover, pressed) = (theme.glass_hover(), theme.wash(0.14));
    div()
        .id(gpui::ElementId::Name(id.into()))
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .role(gpui::Role::Button)
        .aria_label(label)
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(pressed))
        .tooltip(tip(label))
}

const CAPTION_W: f32 = 36.0;

/// Native Windows caption buttons (min/max/close) drawn with the system
/// glyph font; `WindowControlArea` hands their behavior to Windows,
/// including Snap Layouts.
pub fn windows_captions(window: &Window, theme: &Theme) -> Option<AnyElement> {
    if !cfg!(target_os = "windows") {
        return None;
    }
    let (max_id, max_glyph) = if window.is_maximized() { ("win-restore", "\u{e923}") } else { ("win-max", "\u{e922}") };
    let button = |id: &'static str, glyph: &'static str, area: WindowControlArea, close: bool| {
        let red: Hsla = gpui::rgb(0xe81123).into();
        let (hover_bg, hover_fg, active_bg, active_fg) = if close {
            (red, gpui::white(), red.opacity(0.8), gpui::white().opacity(0.8))
        } else {
            (theme.glass_hover(), theme.text, theme.glass_hover().opacity(0.7), theme.text)
        };
        div()
            .id(id)
            .w(px(CAPTION_W))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .font_family("Segoe Fluent Icons")
            .font_weight(gpui::FontWeight::NORMAL)
            .text_size(px(10.0))
            .line_height(px(10.0))
            .text_color(theme.text)
            .hover(move |s| s.bg(hover_bg).text_color(hover_fg))
            .active(move |s| s.bg(active_bg).text_color(active_fg))
            .occlude()
            .window_control_area(area)
            .child(glyph)
    };
    Some(
        div()
            .id("win-captions")
            .absolute()
            .top_0()
            .right_0()
            .h(px(Theme::TITLEBAR_HEIGHT))
            .flex()
            .font_family("Segoe Fluent Icons")
            .child(button("win-min", "\u{e921}", WindowControlArea::Min, false))
            .child(button(max_id, max_glyph, WindowControlArea::Max, false))
            .child(button("win-close", "\u{e8bb}", WindowControlArea::Close, true))
            .into_any_element(),
    )
}

/// Width the titlebar's right-side content must leave for native captions.
pub fn caption_reserve() -> f32 {
    if cfg!(target_os = "windows") { CAPTION_W * 3.0 } else { 0.0 }
}

/// Linux client-side decorations: Dove draws its own caption buttons.
pub fn linux_captions(theme: &Theme) -> Option<AnyElement> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let b = |id: &'static str, path: &'static str, label: &'static str, f: fn(&mut Window)| {
        icon_button(id, path, label, theme, move |_, window, _| f(window))
    };
    Some(
        div()
            .absolute()
            .top_0()
            .right(px(10.0))
            .h(px(Theme::TITLEBAR_HEIGHT))
            .flex()
            .items_center()
            .gap(px(4.0))
            .child(b("lx-min", icons::WINDOW_MINIMIZE, "Minimize", |w| w.minimize_window()))
            .child(b("lx-max", icons::WINDOW_MAXIMIZE, "Maximize", |w| w.zoom_window()))
            .child(b("lx-close", icons::CLOSE, "Close", |w| w.remove_window()))
            .into_any_element(),
    )
}

/// A primary (solid) pill button.
pub fn button_primary(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    theme: &Theme,
) -> gpui::Stateful<Div> {
    let solid = theme.solid;
    let label: SharedString = label.into();
    div()
        .id(gpui::ElementId::Name(id.into()))
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .h(px(30.0))
        .px(px(12.0))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .rounded(px(radius::RADIUS))
        .bg(solid)
        .text_color(theme.on_solid)
        .text_size(ty::BODY)
        .font_weight(gpui::FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(move |s| s.bg(solid.opacity(0.88)))
        .child(label)
}

/// A quiet (ghost) pill button.
pub fn button_ghost(id: impl Into<SharedString>, label: impl Into<SharedString>, theme: &Theme) -> gpui::Stateful<Div> {
    let hover = theme.glass_hover();
    let label: SharedString = label.into();
    div()
        .id(gpui::ElementId::Name(id.into()))
        .role(gpui::Role::Button)
        .aria_label(label.clone())
        .h(px(28.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(6.0))
        .rounded(px(radius::RADIUS))
        .text_color(theme.text_muted)
        .text_size(ty::BODY)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(label)
}

/// A borderless selector for the new-thread row: icon, name, chevron.
pub fn ghost_chip(id: impl Into<SharedString>, theme: &Theme) -> gpui::Stateful<Div> {
    let hover = theme.wash(0.10);
    div()
        .id(gpui::ElementId::Name(id.into()))
        .role(gpui::Role::Button)
        .h(px(26.0))
        .px(px(7.0))
        .flex()
        .items_center()
        .gap(px(5.0))
        .rounded(px(radius::RADIUS))
        .text_size(ty::SMALL)
        .text_color(theme.text_muted)
        .cursor_pointer()
        .hover(move |s| s.bg(hover).text_color(theme.text))
}

/// Zeron's sidebar glyph: a rounded frame holding a panel whose width says
/// whether the sidebar is open (`open` 1) or closed (0). Drawn from quads in
/// the source's 24-unit space so the strokes stay crisp; `right` mirrors it
/// for the side panel. Ported from Zeron's `icons::sidebar_glyph` (MIT).
pub fn sidebar_glyph(open: f32, right: bool, size: f32, color: Hsla) -> Div {
    let s = size / 24.0;
    let stroke = 1.75;
    let frame = div()
        .absolute()
        .left(px((3.0 - stroke / 2.0) * s))
        .top(px((4.0 - stroke / 2.0) * s))
        .w(px((18.0 + stroke) * s))
        .h(px((16.0 + stroke) * s))
        .rounded(px((4.0 + stroke / 2.0) * s))
        .border(px(stroke * s))
        .border_color(color);
    let width = 1.75 + (5.5 - 1.75) * open.clamp(0.0, 1.0);
    let inset = px(6.5 * s);
    let panel = div().absolute().top(px(7.5 * s)).w(px(width * s)).h(px(9.0 * s)).rounded(px(0.875 * s)).bg(color);
    let panel = if right { panel.right(inset) } else { panel.left(inset) };
    div().relative().flex_none().size(px(size)).child(frame).child(panel)
}

/// A borderless selector that floats beside the new-thread composer
/// (project, device, branch): a muted label that only shows a plate on
/// hover or while its menu is open.
pub fn footer_chip(
    id: impl Into<SharedString>,
    glyph: &'static str,
    label: impl Into<SharedString>,
    open: bool,
    theme: &Theme,
) -> gpui::Stateful<Div> {
    let hover = theme.element_hover;
    let rest = theme.text.opacity(0.74);
    let lit = theme.text.opacity(0.95);
    div()
        .id(gpui::ElementId::Name(id.into()))
        .role(gpui::Role::Button)
        .h(px(22.0))
        .max_w(px(280.0))
        .px(px(8.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .rounded(px(radius::fit(22.0)))
        .text_size(ty::META)
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(if open { lit } else { rest })
        .when(open, |d| d.bg(hover))
        .cursor_pointer()
        .hover(move |s| s.bg(hover).text_color(lit))
        .child(icon(glyph).size(px(12.0)).text_color(theme.text.opacity(0.62)))
        .child(div().min_w_0().truncate().child(label.into()))
        .child(icon(icons::ALT_ARROW_DOWN).size(px(10.0)).text_color(theme.text_faint))
}

/// A small selectable chip (picker triggers above the composer).
/// A selector pill: hairline border, soft fill (project, branch, device…).
/// Zeron's selected fill for rows and chips on glass: a light wash. The
/// ring from [`selected_ring`] is what tells selection from hover.
pub fn selected_bg(theme: &Theme) -> gpui::Hsla {
    theme.wash(if theme.appearance.is_dark() { 0.10 } else { 0.11 })
}

/// The selected outline as an inset hairline: drawn over the fill at the
/// edge only, so nothing paints behind a translucent chip.
pub fn selected_ring(theme: &Theme) -> Vec<gpui::BoxShadow> {
    let color =
        if theme.appearance.is_dark() { gpui::hsla(0.0, 0.0, 1.0, 0.09) } else { gpui::hsla(0.0, 0.0, 0.0, 0.07) };
    vec![gpui::BoxShadow {
        color,
        offset: gpui::point(px(0.0), px(0.0)),
        blur_radius: px(0.0),
        spread_radius: px(1.0),
        inset: true,
    }]
}

pub fn chip(id: impl Into<SharedString>, theme: &Theme) -> gpui::Stateful<Div> {
    let hover = theme.wash(0.08);
    let border = theme.wash(0.14);
    div()
        .id(gpui::ElementId::Name(id.into()))
        .role(gpui::Role::Button)
        .h(px(28.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(6.0))
        .rounded(px(radius::RADIUS))
        .border_1()
        .border_color(theme.wash(0.08))
        .bg(theme.wash(0.03))
        .text_size(ty::SMALL)
        .text_color(theme.text_muted)
        .cursor_pointer()
        .hover(move |s| s.bg(hover).border_color(border))
}

/// One row of a popover menu: Zeron's menu grammar (13 px, concentric
/// corners), with an immediate hover so it works inside cached views.
pub fn menu_row(
    id: impl Into<SharedString>,
    path: &'static str,
    label: impl Into<SharedString>,
    danger: bool,
    theme: &Theme,
) -> gpui::Stateful<Div> {
    crate::kit::popover::menu_item(theme, id, Some(path), label, danger, false, false)
}

/// The floating card behind menus and popovers: border, 10 px corners, a
/// 4 px inset, frost on glass, and the menu entrance (140 ms fade and rise).
pub fn menu_card(theme: &Theme, child: impl IntoElement) -> impl IntoElement {
    crate::motion::menu_in(
        "menu-card",
        div().child(frosted(crate::kit::popover::CARD_RADIUS, 16.0, crate::kit::popover::card(theme).child(child))),
    )
}

pub fn separator(theme: &Theme) -> Div {
    crate::kit::popover::menu_separator(theme)
}

/// Colors for gpui-base text inputs (their defaults are transparent).
pub fn input_style(theme: &Theme) -> gpui_base::input::InputEditorStyle {
    gpui_base::input::InputEditorStyle {
        foreground: theme.text,
        muted_foreground: theme.text_faint,
        background: hsla(0.0, 0.0, 0.0, 0.0),
        border: theme.border,
        selection: theme.selection,
        caret: theme.accent,
        ..Default::default()
    }
}

// -------------------------------------------------------------------- data

pub fn status_color(status: TaskStatus, theme: &Theme) -> Hsla {
    match status {
        TaskStatus::Running => theme.busy,
        TaskStatus::NeedsInput => theme.warning,
        TaskStatus::Done => theme.success,
        TaskStatus::Failed => theme.danger,
        TaskStatus::Idle | TaskStatus::Archived | TaskStatus::Draft => theme.text_faint,
    }
}

pub fn dot(color: Hsla, size: f32) -> Div {
    div().size(px(size)).rounded_full().bg(color).flex_none()
}

/// The brand mark for an agent, with a generic bot glyph where none exists.
pub fn harness_icon(kind: HarnessKind) -> &'static str {
    match kind {
        HarnessKind::ClaudeCode => icons::CLAUDE_MARK,
        HarnessKind::Codex => icons::OPENAI_MARK,
        HarnessKind::Cursor => icons::CURSOR_MARK,
        HarnessKind::Devin => icons::DEVIN_MARK,
        HarnessKind::Grok => icons::GROK_MARK,
        HarnessKind::OpenCode => icons::OPENCODE_MARK,
        HarnessKind::Pi => icons::PI_MARK,
        HarnessKind::Hermes => icons::HERMES_MARK,
        HarnessKind::Antigravity => icons::ANTIGRAVITY_MARK,
        HarnessKind::Shell => icons::TERMINAL,
        _ => icons::BOT,
    }
}

/// Brand tint: Claude's mark is orange; every other mark follows the text.
pub fn harness_tint(kind: HarnessKind, theme: &Theme, fallback: Hsla) -> Hsla {
    if kind == HarnessKind::ClaudeCode { theme.claude } else { fallback }
}

/// "12.4k tok · $0.031"
pub fn usage_line(u: &Usage) -> String {
    let tokens = u.input_tokens + u.output_tokens + u.cache_write_tokens;
    let mut s = format!("{} tok", compact(tokens));
    if let Some(c) = u.cost_usd {
        s.push_str(&format!(" · ${c:.2}"));
    }
    s
}

pub fn compact(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// A 3–6 word thread title from the first prompt: filler like "can you
/// please" dropped, the rest capitalized.
pub fn auto_title(prompt: &str) -> String {
    const FILLER: &[&str] = &[
        "please", "can", "could", "would", "you", "i", "want", "need", "to", "hey", "hi", "so", "just", "help", "me",
        "let's", "lets", "we", "the", "a", "an",
    ];
    let line = prompt.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let words: Vec<&str> = line
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.' && c != '/'))
        .filter(|w| !w.is_empty())
        .collect();
    let start = words.iter().position(|w| !FILLER.contains(&w.to_lowercase().as_str())).unwrap_or(0);
    let picked: Vec<&str> = words[start..].iter().take(6).copied().collect();
    if picked.len() < 2 {
        return title_from_prompt(prompt);
    }
    let mut t = picked.join(" ");
    if let Some(first) = t.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    t.trim_end_matches(['.', ',', ':', ';']).to_string()
}

pub fn title_from_prompt(prompt: &str) -> String {
    let line = prompt.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("New session");
    let mut t: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        t.push('…');
    }
    t
}

/// "now", "5m", "3h", "2d", "1w"
pub fn relative_time(ms: i64) -> String {
    let secs = (dove_core::store::now_ms() - ms).max(0) / 1000;
    match secs {
        0..60 => "now".into(),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h", secs / 3600),
        86400..604800 => format!("{}d", secs / 86400),
        _ => format!("{}w", secs / 604800),
    }
}

pub fn host_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "This device".into())
        .to_uppercase()
}
