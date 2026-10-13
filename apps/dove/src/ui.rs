//! Shared presentation primitives. The frost, layer, rounded-clip elements
//! and the control styles are adapted from Zeron (MIT).

use dove_core::proto::{HarnessKind, TaskStatus, Usage};
use gpui::{
    AnyElement, AnyView, App, AppContext as _, Bounds, BoxShadow, ClickEvent, Context, Corners, Div, Element,
    GlobalElementId, Hsla, InspectorElementId, InteractiveElement, IntoElement, LayoutId, MouseButton, ParentElement,
    Pixels, Render, SharedString, StatefulInteractiveElement, Styled, Window, WindowControlArea, div, hsla, point, px,
};

use crate::assets::{icon, icons};
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

// ------------------------------------------------------------------- glass

fn shadow(color: Hsla, y: f32, blur: f32, spread: f32, inset: bool) -> BoxShadow {
    BoxShadow { color, offset: point(px(0.0), px(y)), blur_radius: px(blur), spread_radius: px(spread), inset }
}

/// The composer / card plate: a translucent lifted panel over glass.
pub fn card<E: Styled>(el: E, theme: &Theme) -> E {
    let base = if theme.appearance.is_dark() { theme.surface_card } else { theme.bg };
    let fill = if theme.is_frost() { base.opacity(0.72) } else { base };
    el.bg(fill).border_1().border_color(theme.border).shadow(vec![
        shadow(hsla(0.0, 0.0, 1.0, if theme.appearance.is_dark() { 0.04 } else { 0.6 }), 1.0, 0.0, 0.0, true),
        shadow(hsla(0.0, 0.0, 0.0, if theme.appearance.is_dark() { 0.35 } else { 0.08 }), 8.0, 24.0, 0.0, false),
    ])
}

// ----------------------------------------------------------------- tooltip

pub struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(cx);
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(6.0))
            .bg(t.surface_overlay)
            .border_1()
            .border_color(t.border)
            .text_color(t.text)
            .text_size(px(12.0))
            .child(self.0.clone())
    }
}

pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
}

// ---------------------------------------------------------------- controls

/// A 24px window-chrome button (sidebar toggle, history, new). Occludes the
/// titlebar drag strip so clicks never start a window move.
pub fn window_button(
    id: impl Into<SharedString>,
    glyph: impl IntoElement,
    label: &'static str,
    theme: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<Div> {
    let hover = theme.glass_hover();
    div()
        .id(gpui::ElementId::Name(id.into()))
        .size(px(24.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .role(gpui::Role::Button)
        .aria_label(label)
        .hover(move |s| s.bg(hover))
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(move |e, window, cx| {
            cx.stop_propagation();
            on_click(e, window, cx)
        })
        .tooltip(tip(label))
        .child(glyph)
}

pub fn icon_button(
    id: impl Into<SharedString>,
    path: &'static str,
    label: &'static str,
    theme: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<Div> {
    window_button(id, icon(path).text_color(theme.text_muted), label, theme, on_click)
}

/// A disabled-looking icon slot that still blocks the drag strip.
pub fn icon_disabled(path: &'static str, theme: &Theme) -> Div {
    div()
        .size(px(24.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .child(icon(path).text_color(theme.text_muted.opacity(0.35)))
}

const CAPTION_W: f32 = 46.0;

/// Native Windows caption buttons (min/max/close) drawn with the system
/// glyph font; `WindowControlArea` hands their behavior to Windows,
/// including Snap Layouts.
pub fn windows_captions(window: &Window, theme: &Theme) -> Option<AnyElement> {
    if !cfg!(target_os = "windows") {
        return None;
    }
    let (max_id, max_glyph) = if window.is_maximized() { ("win-restore", "\u{e923}") } else { ("win-max", "\u{e922}") };
    let button = |id: &'static str, glyph: &'static str, area: WindowControlArea, close: bool| {
        let (hover_bg, hover_fg) =
            if close { (gpui::rgb(0xe81123).into(), gpui::white()) } else { (theme.glass_hover(), theme.text) };
        div()
            .id(id)
            .w(px(CAPTION_W))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(10.0))
            .text_color(theme.text)
            .hover(move |s| s.bg(hover_bg).text_color(hover_fg))
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
    let hover = theme.glass_hover();
    let b = |id: &'static str, path: &'static str, f: fn(&mut Window)| {
        div()
            .id(id)
            .size(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .hover(move |s| s.bg(hover))
            .occlude()
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                f(window)
            })
            .child(icon(path).text_color(theme.text_muted))
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
            .child(b("lx-min", icons::WINDOW_MINIMIZE, |w| w.minimize_window()))
            .child(b("lx-max", icons::WINDOW_MAXIMIZE, |w| w.zoom_window()))
            .child(b("lx-close", icons::CLOSE, |w| w.remove_window()))
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
        .rounded(px(8.0))
        .bg(solid)
        .text_color(theme.on_solid)
        .text_size(px(13.0))
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
        .rounded(px(7.0))
        .text_color(theme.text_muted)
        .text_size(px(13.0))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(label)
}

/// A small selectable chip (picker triggers above the composer).
/// A selector pill: hairline border, soft fill (project, branch, device…).
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
        .rounded(px(9.0))
        .border_1()
        .border_color(theme.wash(0.08))
        .bg(theme.wash(0.03))
        .text_size(px(12.5))
        .text_color(theme.text_muted)
        .cursor_pointer()
        .hover(move |s| s.bg(hover).border_color(border))
}

/// One row of a popover menu.
pub fn menu_row(
    id: impl Into<SharedString>,
    path: &'static str,
    label: impl Into<SharedString>,
    danger: bool,
    theme: &Theme,
) -> gpui::Stateful<Div> {
    let hover = theme.wash(0.10);
    let color = if danger { theme.danger } else { theme.text };
    let label: SharedString = label.into();
    div()
        .id(gpui::ElementId::Name(id.into()))
        .role(gpui::Role::MenuItem)
        .aria_label(label.clone())
        .h(px(34.0))
        .px(px(10.0))
        .mx(px(4.0))
        .flex()
        .items_center()
        .gap(px(10.0))
        .rounded(px(7.0))
        .text_size(px(14.0))
        .text_color(color)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(icon(path).size(px(16.0)).text_color(if danger { theme.danger } else { theme.text_muted }))
        .child(div().flex_1().child(label))
}

/// The floating card behind menus and popovers.
pub fn menu_card(theme: &Theme, child: impl IntoElement) -> impl IntoElement {
    frosted(
        12.0,
        16.0,
        div()
            .py(px(4.0))
            .rounded(px(12.0))
            .bg(theme.glass_overlay())
            .border_1()
            .border_color(theme.border_strong)
            .shadow(vec![shadow(hsla(0.0, 0.0, 0.0, 0.35), 10.0, 30.0, 0.0, false)])
            .child(child),
    )
}

pub fn separator(theme: &Theme) -> Div {
    div().mx(px(10.0)).my(px(4.0)).h(px(1.0)).bg(theme.border)
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
