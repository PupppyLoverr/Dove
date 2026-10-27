//! The island's design tokens (docs/NOTCH-V2.md §4–§5.4): colour, type,
//! radius, spacing, sizes and motion. Views use these and nothing else, so
//! the look changes in one place.

use gpui::Hsla;

const fn hsla(h: f32, s: f32, l: f32, a: f32) -> Hsla {
    Hsla { h, s, l, a }
}

// ----- colour ---------------------------------------------------------------

pub const TEXT: Hsla = hsla(0.0, 0.0, 1.0, 0.95);
pub const DIM: Hsla = hsla(0.0, 0.0, 1.0, 0.64);
pub const FAINT: Hsla = hsla(0.0, 0.0, 1.0, 0.42);
pub const GHOST: Hsla = hsla(0.0, 0.0, 1.0, 0.26);
/// Inner glass for cards and rows.
pub const CARD: Hsla = hsla(0.0, 0.0, 1.0, 0.06);
pub const WASH: Hsla = hsla(0.0, 0.0, 1.0, 0.08);
pub const WASH_HOVER: Hsla = hsla(0.0, 0.0, 1.0, 0.13);
pub const WASH_ON: Hsla = hsla(0.0, 0.0, 1.0, 0.18);
pub const HAIRLINE: Hsla = hsla(0.0, 0.0, 1.0, 0.10);
/// The specular line along the island's top edge.
pub const SPECULAR: Hsla = hsla(0.0, 0.0, 1.0, 0.22);
pub const INK: Hsla = hsla(0.0, 0.0, 0.07, 1.0);
pub const WHITE: Hsla = hsla(0.0, 0.0, 1.0, 0.96);
pub const WHITE_HOVER: Hsla = hsla(0.0, 0.0, 1.0, 0.82);

/// One accent per meaning.
pub const AMBER: Hsla = hsla(37.0 / 360.0, 1.0, 0.56, 1.0);
pub const BLUE: Hsla = hsla(211.0 / 360.0, 1.0, 0.62, 1.0);
pub const GREEN: Hsla = hsla(138.0 / 360.0, 0.62, 0.52, 1.0);
pub const RED: Hsla = hsla(3.0 / 360.0, 1.0, 0.63, 1.0);
pub const CLAUDE: Hsla = hsla(15.0 / 360.0, 0.63, 0.60, 1.0);

/// The glass tint over the blurred backdrop (top, bottom), dark default.
pub const TINT_TOP: Hsla = hsla(240.0 / 360.0, 0.06, 0.13, 0.66);
pub const TINT_BOTTOM: Hsla = hsla(240.0 / 360.0, 0.08, 0.05, 0.76);
/// The solid material (transparency effects off, or no glass at all).
pub const SOLID_TOP: Hsla = hsla(240.0 / 360.0, 0.06, 0.13, 0.97);
pub const SOLID_BOTTOM: Hsla = hsla(240.0 / 360.0, 0.08, 0.07, 0.98);
/// How strongly a glow (art colour, alert) tints the glass.
pub const GLOW: f32 = 0.09;

// ----- type -------------------------------------------------------------------

/// The six sizes.
pub const T11: f32 = 11.0;
pub const T12: f32 = 12.0;
pub const T13: f32 = 13.0;
pub const T15: f32 = 15.0;
pub const T20: f32 = 20.0;
pub const MONO: &str = "Geist Mono";

// ----- radius, spacing --------------------------------------------------------

pub const R_ISLAND: f32 = 30.0;
pub const R_CARD: f32 = 22.0;
pub const R_ROW: f32 = 14.0;
pub const PAD: f32 = 14.0;

// ----- sizes ------------------------------------------------------------------

/// The window: a fixed canvas the island moves and morphs in.
pub const CANVAS_W: f32 = 520.0;
pub const CANVAS_H: f32 = 440.0;
/// The island floats this far below the screen's top edge.
pub const TOP_GAP: f32 = 8.0;
pub const SLIM_H: f32 = 36.0;
/// The slim island's three widths: one activity, two, or a wide one.
pub const SLIM_SINGLE: f32 = 196.0;
pub const SLIM_DUAL: f32 = 264.0;
pub const SLIM_WIDE: f32 = 320.0;
pub const BUBBLE: f32 = 36.0;
pub const BUBBLE_GAP: f32 = 6.0;
pub const BANNER_W: f32 = 384.0;
pub const BANNER_H: f32 = 96.0;
pub const FLASH_H: f32 = 52.0;
pub const EXPANDED_W: f32 = 468.0;
pub const EXPANDED_MAX_H: f32 = 404.0;
pub const HEADER_H: f32 = 48.0;
pub const ROW_H: f32 = 52.0;

// ----- motion -----------------------------------------------------------------

/// Spring stiffness and damping ratio for the island's shape: a quick settle
/// with about 6 % overshoot.
pub const SPRING_STIFFNESS: f32 = 420.0;
pub const SPRING_DAMPING: f32 = 0.68;
/// How long the dot takes to drop out of the edge.
pub const DOT_MS: u64 = 160;
/// Breathing (needs you, working).
pub const BREATHE_S: f32 = 1.6;
/// Rows rise and fade in, one after another.
pub const STAGGER_MS: u64 = 18;
pub const RISE_PX: f32 = 6.0;
/// The pointer pulls the slim island toward it within this distance, by at
/// most this much.
pub const MAGNET_RANGE: f32 = 40.0;
pub const MAGNET_PULL: f32 = 3.0;
/// A press squishes the island this much.
pub const SQUISH: f32 = 0.02;
