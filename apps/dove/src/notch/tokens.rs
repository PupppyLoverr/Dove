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
/// Cards: a flat raised surface on the black, no edge.
pub const CARD: Hsla = hsla(0.0, 0.0, 1.0, 0.06);
pub const NONE: Hsla = hsla(0.0, 0.0, 0.0, 0.0);
pub const WASH: Hsla = hsla(0.0, 0.0, 1.0, 0.08);
pub const WASH_HOVER: Hsla = hsla(0.0, 0.0, 1.0, 0.13);
pub const WASH_ON: Hsla = hsla(0.0, 0.0, 1.0, 0.18);
pub const HAIRLINE: Hsla = hsla(0.0, 0.0, 1.0, 0.10);
/// The specular line along the island's top edge.
pub const SPECULAR: Hsla = hsla(0.0, 0.0, 1.0, 0.26);
/// The island's rim: a bright outer edge and a faint inner one, so the
/// glass reads as a thick pane rather than a cut-out.
pub const RIM: Hsla = hsla(0.0, 0.0, 1.0, 0.11);
pub const RIM_INNER: Hsla = hsla(0.0, 0.0, 1.0, 0.035);
/// Light falling on the glass from above, fading out by this far down.
pub const SHEEN: Hsla = hsla(0.0, 0.0, 1.0, 0.05);
pub const SHEEN_TO: f32 = 0.45;
/// The milky frost over the blur, and the grain that makes it read as
/// frosted rather than smoked.
pub const FROST: Hsla = hsla(0.0, 0.0, 0.9, 0.03);
pub const GRAIN: u8 = 9;
/// The plate behind an empty state's glyph (top, bottom).
pub const PLATE_TOP: Hsla = hsla(0.0, 0.0, 1.0, 0.11);
pub const PLATE_BOTTOM: Hsla = hsla(0.0, 0.0, 1.0, 0.03);
/// Floating surfaces over the island (menus, tooltips).
pub const FLOAT: Hsla = hsla(0.0, 0.0, 0.09, 0.98);
pub const INK: Hsla = hsla(0.0, 0.0, 0.07, 1.0);
pub const WHITE: Hsla = hsla(0.0, 0.0, 1.0, 0.96);
pub const WHITE_HOVER: Hsla = hsla(0.0, 0.0, 1.0, 0.82);

/// Black and white: meaning is carried by brightness, words and motion,
/// never hue. Needs-you and failures are full white (and breathe or say
/// so); working is a softer white; done is quiet.
pub const AMBER: Hsla = hsla(0.0, 0.0, 1.0, 0.96);
pub const BLUE: Hsla = hsla(0.0, 0.0, 1.0, 0.70);
pub const GREEN: Hsla = hsla(0.0, 0.0, 1.0, 0.86);
pub const RED: Hsla = hsla(0.0, 0.0, 1.0, 1.0);
/// K22 (live voice).
pub const K22: Hsla = hsla(0.0, 0.0, 1.0, 0.96);

/// The opt-in glass's tint over the blurred backdrop (top, bottom): neutral
/// charcoal, clear over a dark backdrop and denser over a bright one so white
/// type stays readable.
pub const TINT_TOP: Hsla = hsla(0.0, 0.0, 0.14, 0.42);
pub const TINT_BOTTOM: Hsla = hsla(0.0, 0.0, 0.085, 0.54);
/// How much alpha the tint gains over a fully bright backdrop.
pub const TINT_DENSE: f32 = 0.30;
/// The backdrop brightness (0–1) where the tint starts and finishes
/// thickening.
pub const LUMA_FROM: f32 = 0.30;
pub const LUMA_TO: f32 = 0.80;
/// The default material: true black, so the island reads as part of the
/// screen's edge (glass is opt-in, Settings → Frosted glass).
pub const SOLID_TOP: Hsla = hsla(0.0, 0.0, 0.0, 1.0);
pub const SOLID_BOTTOM: Hsla = hsla(0.0, 0.0, 0.0, 1.0);
/// How strongly a glow (art colour, alert) tints the glass.
pub const GLOW: f32 = 0.09;

// ----- type -------------------------------------------------------------------

/// The six sizes.
pub const T11: f32 = 11.0;
pub const T12: f32 = 12.0;
pub const T13: f32 = 13.0;
pub const T15: f32 = 15.0;
pub const T20: f32 = 20.0;
pub const MONO: &str = "JetBrains Mono";

// ----- radius, spacing --------------------------------------------------------

pub const R_ISLAND: f32 = 30.0;
/// Concentric with the island: its radius less the padding.
pub const R_CARD: f32 = R_ISLAND - PAD;
pub const R_ROW: f32 = 12.0;
pub const PAD: f32 = 14.0;
/// Gaps between cards and groups (spacing instead of dividers).
pub const GAP: f32 = 12.0;
/// One height for pills and icon buttons.
pub const CONTROL_H: f32 = 28.0;
/// Icon sizes: in rows and controls, and in the header.
pub const ICON_ROW: f32 = 16.0;
pub const ICON_HEADER: f32 = 18.0;

// ----- sizes ------------------------------------------------------------------

/// The window: a fixed canvas the island moves and morphs in.
pub const CANVAS_W: f32 = 720.0;
pub const CANVAS_H: f32 = 660.0;
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
/// The wide island: the call, a plan, a view (V3), up to this tall, and
/// never past this share of the display's height.
pub const WIDE_W: f32 = 640.0;
pub const WIDE_MAX_H: f32 = 600.0;
pub const WIDE_SHARE: f32 = 0.7;
pub const HEADER_H: f32 = 48.0;
pub const ROW_H: f32 = 52.0;

// ----- motion -----------------------------------------------------------------

/// Springs for the island's shape (stiffness, damping ratio), by direction
/// (docs/NOTCH-V2.md §11.7): opening has one small overshoot (~220 ms);
/// closing and resizing are critically damped (~150 ms, no bounce).
pub const SPRING_OPEN: (f32, f32) = (700.0, 0.78);
pub const SPRING_CLOSE: (f32, f32) = (900.0, 1.0);
pub const SPRING_RESIZE: (f32, f32) = (600.0, 1.0);
/// Breathing (needs you, working).
pub const BREATHE_S: f32 = 1.6;
/// Content fades in over the last part of the opening, from this far open
/// (about the last 90 ms of it).
pub const REVEAL_AT: f32 = 0.6;
/// Rows rise and fade in, one after another (the first four only).
pub const RISE_MS: u64 = 140;
pub const STAGGER_MS: u64 = 12;
pub const RISE_PX: f32 = 4.0;
pub const RISE_ROWS: usize = 4;
/// A page change cross-fades over this long; the tab puck slides in it.
pub const PAGE_MS: f32 = 140.0;
pub const PUCK_MS: f32 = 160.0;
/// A shared element (art, a timer ring) flies between views this long.
pub const HERO_MS: f32 = 260.0;
/// The pointer pulls the slim island toward it within this distance, by at
/// most this much.
pub const MAGNET_RANGE: f32 = 40.0;
pub const MAGNET_PULL: f32 = 3.0;
/// A press squishes the island this much.
pub const SQUISH: f32 = 0.02;
