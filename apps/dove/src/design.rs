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

use gpui::{App, Pixels, Rems, Window, px};

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
    /// Large headings.
    pub const H2: Rems = rem(18.0);
    /// Page titles.
    pub const PAGE: Rems = rem(20.0);
    /// Hero headings.
    pub const H1: Rems = rem(22.0);
    /// Display text.
    pub const DISPLAY: Rems = rem(26.0);

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

    /// The role closest to a legacy pixel size (used by the size codemod
    /// and by builders that take a numeric size).
    pub const fn nearest(px: f32) -> Rems {
        rem(snap(px))
    }

    /// The scale step closest to `px`.
    pub const fn snap(px: f32) -> f32 {
        if px <= 10.75 {
            10.5
        } else if px <= 11.25 {
            11.0
        } else if px <= 11.75 {
            11.5
        } else if px <= 12.25 {
            12.0
        } else if px <= 12.75 {
            12.5
        } else if px <= 13.5 {
            13.0
        } else if px <= 14.5 {
            14.0
        } else if px <= 15.5 {
            15.0
        } else if px <= 17.0 {
            16.0
        } else if px <= 19.0 {
            18.0
        } else if px <= 21.0 {
            20.0
        } else if px <= 24.0 {
            22.0
        } else {
            26.0
        }
    }
}

/// The 4 px spacing grid.
pub mod space {
    pub const XXS: f32 = 2.0;
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 24.0;
    pub const XXL: f32 = 32.0;
}

/// Corner radii.
pub mod radius {
    /// Icon buttons, small controls.
    pub const CONTROL: f32 = 6.0;
    /// Menu and picker rows.
    pub const MENU_ITEM: f32 = 7.0;
    /// Selects, segmented controls, text fields.
    pub const SELECT: f32 = 8.0;
    /// Inset panels and the main card.
    pub const PANEL: f32 = 10.0;
    /// Cards, popovers, menus.
    pub const CARD: f32 = 12.0;
    /// Chat bubbles.
    pub const BUBBLE: f32 = 16.0;
    /// The composer plate.
    pub const COMPOSER: f32 = 26.0;
    /// Pills.
    pub const PILL: f32 = 999.0;
}

/// Fixed control metrics.
pub mod size {
    /// Titlebar (Zeron's 38 px).
    pub const TITLEBAR: f32 = 38.0;
    /// Settings and page rows.
    pub const ROW_MIN: f32 = 60.0;
    /// Select triggers and text fields.
    pub const FIELD: f32 = 32.0;
    /// Icon buttons.
    pub const BUTTON: f32 = 24.0;
    /// Toggle switch.
    pub const TOGGLE_W: f32 = 36.0;
    pub const TOGGLE_H: f32 = 20.0;
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

/// How much the interface text is scaled from the default.
pub fn ui_scale() -> f32 {
    ui_size() / BASE
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

/// A pixel length scaled with the interface size, for the few layout
/// numbers that must track text (row heights holding a single label).
pub fn scaled(px_at_base: f32) -> Pixels {
    px((px_at_base * ui_scale()).round())
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
    fn snap_lands_on_the_scale() {
        assert_eq!(ty::snap(10.0), 10.5);
        assert_eq!(ty::snap(11.5), 11.5);
        assert_eq!(ty::snap(13.5), 13.0);
        assert_eq!(ty::snap(14.5), 14.0);
        assert_eq!(ty::snap(21.0), 20.0);
        assert_eq!(ty::snap(26.0), 26.0);
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
