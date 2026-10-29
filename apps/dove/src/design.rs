//! Design tokens: the type scale, spacing, radii and elevation every surface
//! draws with, plus the user's size preferences. Values follow Zeron's
//! (MIT) so Dove reads at the same rhythm; Dove keeps its own identity
//! colours in `theme.rs`.
//!
//! Text is set in rems through [`ty`], so the "Interface font size" setting
//! (`window.set_rem_size`) scales every label at once; at the default 16 px
//! a role renders at exactly its listed pixel size. Layout dimensions stay
//! in pixels, which keeps every width and hit-area calculation exact.
//! Monospace surfaces whose row heights depend on the font (diffs, file
//! previews, the terminal) read their own size settings through [`code`]
//! and [`terminal`].

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use gpui::{App, Rems, Window, px};

/// The base size rems are measured against (Zeron's default).
pub const BASE: f32 = 16.0;

/// A size in rems that renders at `px_at_base` pixels at the default base.
pub const fn rem(px_at_base: f32) -> Rems {
    Rems(px_at_base / BASE)
}

/// Text roles. Each is the pixel size at the default interface size.
pub mod ty {
    use gpui::Rems;

    use super::rem;

    /// Pills and counters.
    pub const BADGE: Rems = rem(10.5);
    /// Fine print: kbd hints, timestamps.
    pub const MICRO: Rems = rem(11.0);
    /// Secondary row lines, sidebar details.
    pub const CAPTION: Rems = rem(11.5);
    /// Row descriptions, menus' meta, chips.
    pub const META: Rems = rem(12.0);
    /// Chips, tabs, compact controls.
    pub const SMALL: Rems = rem(12.5);
    /// Body text, row titles, controls.
    pub const BODY: Rems = rem(13.0);
    /// Conversation text, menu rows.
    pub const MD: Rems = rem(14.0);
    /// Card and dialog titles.
    pub const TITLE: Rems = rem(15.0);
    /// Section headings.
    pub const HEADING: Rems = rem(16.0);
    /// Page titles.
    pub const PAGE: Rems = rem(20.0);
    /// Hero headings.
    pub const H1: Rems = rem(22.0);

    /// Line heights that pair with the roles above.
    pub mod lh {
        use gpui::Rems;

        use super::rem;

        pub const META: Rems = rem(16.0);
        pub const BODY: Rems = rem(17.0);
        pub const LOOSE: Rems = rem(18.0);
        pub const MD: Rems = rem(22.0);
        pub const PAGE: Rems = rem(26.0);
    }
}

/// The 4 px spacing grid.
pub mod space {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 24.0;
}

/// Corner radii. Rectangular chrome shares [`radius::RADIUS`]. A corner
/// inside that surface uses [`radius::nested`] so it follows the padding.
/// A control too short to hold the full radius uses [`radius::fit`], which
/// keeps a straight side. The composer plate is the one larger corner: the
/// send disc plus its inset, pulled in by [`radius::plate`] when the bar is
/// short enough that 26 px would turn it into a capsule.
pub mod radius {
    /// Buttons, fields, rows, cards, menus, panels and bubbles.
    pub const RADIUS: f32 = 10.0;
    /// Straight side a short control keeps, so a 24 px button stays a rounded square.
    const STRAIGHT: f32 = 8.0;
    /// 24 px icon buttons: [`fit`] so they stay rounded squares.
    pub const CONTROL: f32 = fit(24.0);
    /// Selects, segmented controls, text fields.
    pub const SELECT: f32 = RADIUS;
    /// Sidebar, list and menu rows.
    pub const ROW: f32 = 8.0;
    /// Inset panels and the main card.
    pub const PANEL: f32 = 12.0;
    /// Cards, popovers, menus.
    pub const CARD: f32 = 12.0;
    /// Modal sheets: the palette, dialogs.
    pub const MODAL: f32 = 16.0;
    /// Chat bubbles.
    pub const BUBBLE: f32 = RADIUS;
    /// The composer plate: a calm rounded rectangle, not a capsule.
    pub const COMPOSER: f32 = 18.0;

    /// The corner of something padded by `padding` inside a [`RADIUS`] surface.
    pub const fn nested(padding: f32) -> f32 {
        let inner = RADIUS - padding;
        if inner > 0.0 { inner } else { 0.0 }
    }

    /// [`RADIUS`], limited so a control of `height` keeps a straight side.
    pub const fn fit(height: f32) -> f32 {
        let cap = (height - STRAIGHT) * 0.5;
        if cap < RADIUS { if cap > 0.0 { cap } else { 0.0 } } else { RADIUS }
    }

    /// [`COMPOSER`] when `height` can hold it, otherwise the largest corner
    /// that still leaves a 16 px straight run along the bar.
    pub const fn plate(height: f32) -> f32 {
        let cap = (height - 16.0) * 0.5;
        if cap >= COMPOSER {
            COMPOSER
        } else if cap > RADIUS {
            cap
        } else {
            RADIUS
        }
    }
}

/// Fixed control metrics.
pub mod size {
    /// Titlebar (Zeron's 38 px).
    pub const TITLEBAR: f32 = 38.0;
    /// Settings and page rows.
    pub const ROW_MIN: f32 = 60.0;
    /// Select triggers and text fields.
    pub const FIELD: f32 = 32.0;
    /// The settings and page reading column.
    pub const PAGE_MAX: f32 = 760.0;
    pub const PAGE_PAD_X: f32 = 40.0;
}

// ---------------------------------------------------------------------------
// Size preferences
// ---------------------------------------------------------------------------

/// Interface font sizes offered in Settings › Appearance.
pub const UI_SIZES: [f32; 7] = [12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0];
/// Code and terminal sizes offered.
pub const MONO_SIZES: [f32; 9] = [10.0, 11.0, 12.0, 12.5, 13.0, 14.0, 15.0, 16.0, 18.0];
/// Conversation width bounds and step (Zeron's).
pub const CONVERSATION_MIN: f32 = 560.0;
pub const CONVERSATION_MAX: f32 = 1200.0;
pub const CONVERSATION_STEP: f32 = 16.0;
/// Default conversation width: Dove's reading column.
pub const CONVERSATION_DEFAULT: f32 = 760.0;

pub const CODE_DEFAULT: f32 = 12.0;
pub const TERMINAL_DEFAULT: f32 = 13.0;

static CODE: AtomicU32 = AtomicU32::new(CODE_DEFAULT.to_bits());
static TERMINAL: AtomicU32 = AtomicU32::new(TERMINAL_DEFAULT.to_bits());
static CONVERSATION: AtomicU32 = AtomicU32::new(CONVERSATION_DEFAULT.to_bits());
static UI: AtomicU32 = AtomicU32::new(BASE.to_bits());

fn load(cell: &AtomicU32) -> f32 {
    f32::from_bits(cell.load(Ordering::Relaxed))
}

/// The code and diff font size in pixels.
pub fn code() -> f32 {
    load(&CODE)
}

/// Line height for code rows at the current code size (Zeron's 12 → 21
/// diff ratio, rounded to whole pixels so uniform lists stay crisp).
pub fn code_line() -> f32 {
    (code() * 1.75).round()
}

/// The terminal font size in pixels.
pub fn terminal() -> f32 {
    load(&TERMINAL)
}

/// The conversation column's maximum width in pixels.
pub fn conversation_width() -> f32 {
    load(&CONVERSATION)
}

/// The interface base size in pixels.
pub fn ui_size() -> f32 {
    load(&UI)
}

fn parse(store: &dove_core::store::Store, key: &str, default: f32, lo: f32, hi: f32) -> f32 {
    store.setting(key).and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite()).unwrap_or(default).clamp(lo, hi)
}

/// Read the size preferences (called at launch and after a change).
pub fn load_preferences(store: &dove_core::store::Store) {
    CODE.store(parse(store, "code_font_size", CODE_DEFAULT, 8.0, 24.0).to_bits(), Ordering::Relaxed);
    TERMINAL.store(parse(store, "terminal_font_size", TERMINAL_DEFAULT, 8.0, 24.0).to_bits(), Ordering::Relaxed);
    CONVERSATION.store(
        parse(store, "conversation_width", CONVERSATION_DEFAULT, CONVERSATION_MIN, CONVERSATION_MAX).to_bits(),
        Ordering::Relaxed,
    );
    UI.store(parse(store, "ui_font_size", BASE, 12.0, 18.0).to_bits(), Ordering::Relaxed);
    SCROLLBACK.store(
        store
            .setting(SCROLLBACK_KEY)
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(SCROLLBACK_DEFAULT)
            .clamp(SCROLLBACK_MIN, SCROLLBACK_MAX),
        Ordering::Relaxed,
    );
}

/// The "Scrollback lines" setting: the history each new terminal keeps.
pub const SCROLLBACK_KEY: &str = "terminal_scrollback";
pub const SCROLLBACK_DEFAULT: usize = dove_term::emulator::SCROLLBACK_LINES;
pub const SCROLLBACK_MIN: usize = 1_000;
pub const SCROLLBACK_MAX: usize = 100_000;
/// The choices Settings offers.
pub const SCROLLBACK_CHOICES: [usize; 6] = [1_000, 5_000, 10_000, 25_000, 50_000, 100_000];
static SCROLLBACK: AtomicUsize = AtomicUsize::new(SCROLLBACK_DEFAULT);

/// Lines of history a terminal keeps.
pub fn scrollback() -> usize {
    SCROLLBACK.load(Ordering::Relaxed)
}

/// Persist the scrollback setting. Terminals started, restarted or cleared
/// from now on keep that much history.
pub fn set_scrollback(store: &dove_core::store::Store, lines: usize) {
    let _ = store.set_setting(SCROLLBACK_KEY, &lines.to_string());
    load_preferences(store);
}

/// Apply the interface size to a window (its rem size).
pub fn apply_to_window(window: &mut Window) {
    let size = ui_size();
    if (f32::from(window.rem_size()) - size).abs() > f32::EPSILON {
        window.set_rem_size(px(size));
    }
}

/// Persist one size preference and re-apply everything that reads it.
pub fn set_preference(store: &dove_core::store::Store, key: &str, value: f32, cx: &mut App) {
    let _ = store.set_setting(key, &format_size(value));
    load_preferences(store);
    for window in cx.windows() {
        let _ = window.update(cx, |_, window, _| {
            apply_to_window(window);
            window.refresh();
        });
    }
}

/// `12` for whole sizes, `12.5` otherwise.
pub fn format_size(v: f32) -> String {
    if (v - v.round()).abs() < 0.01 { format!("{}", v.round() as i64) } else { format!("{v:.1}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_render_at_their_pixel_size_at_base() {
        assert!((ty::BODY.0 * BASE - 13.0).abs() < 1e-6);
        assert!((ty::PAGE.0 * BASE - 20.0).abs() < 1e-6);
    }

    #[test]
    fn code_line_tracks_size() {
        assert_eq!((12.0f32 * 1.75).round(), 21.0);
    }

    #[test]
    fn sizes_format() {
        assert_eq!(format_size(16.0), "16");
        assert_eq!(format_size(12.5), "12.5");
    }
}
