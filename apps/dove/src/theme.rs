//! Design tokens. Values and color math adapted from Zeron's theme (MIT):
//! a frosted shell over the blurred desktop, a near-black content plane, and
//! accents authored in OKLCH so light and dark keep the same hue.

use gpui::{App, Global, Hsla, SharedString, WindowAppearance, hsla};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

impl Appearance {
    pub fn is_dark(self) -> bool {
        self == Self::Dark
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Accent {
    #[default]
    Sky,
    Indigo,
    Orange,
    Green,
    Pink,
}

impl Accent {
    pub const ALL: [Accent; 5] = [Self::Sky, Self::Indigo, Self::Orange, Self::Green, Self::Pink];

    pub fn id(self) -> &'static str {
        match self {
            Self::Sky => "sky",
            Self::Indigo => "indigo",
            Self::Orange => "orange",
            Self::Green => "green",
            Self::Pink => "pink",
        }
    }

    pub fn from_id(id: &str) -> Self {
        Self::ALL.into_iter().find(|a| a.id() == id).unwrap_or_default()
    }

    fn pair(self, appearance: Appearance) -> (Hsla, Hsla) {
        let dark = appearance.is_dark();
        match self {
            Self::Sky if dark => (oklch(0.74, 0.14, 235.0), oklch(0.56, 0.17, 245.0)),
            Self::Sky => (oklch(0.50, 0.17, 245.0), oklch(0.50, 0.17, 245.0)),
            Self::Indigo if dark => (oklch(0.673, 0.182, 276.935), oklch(0.585, 0.233, 277.117)),
            Self::Indigo => (oklch(0.511, 0.262, 276.966), oklch(0.511, 0.262, 276.966)),
            Self::Orange if dark => (oklch(0.75, 0.18, 55.0), oklch(0.54, 0.19, 55.0)),
            Self::Orange => (oklch(0.50, 0.19, 55.0), oklch(0.50, 0.19, 55.0)),
            Self::Green if dark => (oklch(0.75, 0.17, 150.0), oklch(0.50, 0.15, 150.0)),
            Self::Green => (oklch(0.46, 0.14, 150.0), oklch(0.46, 0.14, 150.0)),
            Self::Pink if dark => (oklch(0.72, 0.18, 350.0), oklch(0.51, 0.20, 350.0)),
            Self::Pink => (oklch(0.48, 0.20, 350.0), oklch(0.48, 0.20, 350.0)),
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // a token set: not every token is used yet
pub struct Theme {
    pub appearance: Appearance,
    pub accent_choice: Accent,
    /// Paint translucent chrome over the blurred desktop.
    pub frosted: bool,

    pub bg: Hsla,
    pub surface: Hsla,
    pub surface_raised: Hsla,
    pub surface_card: Hsla,
    pub surface_overlay: Hsla,
    pub element_hover: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,

    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,

    pub solid: Hsla,
    pub on_solid: Hsla,

    pub accent: Hsla,
    pub accent_strong: Hsla,
    pub danger: Hsla,
    pub warning: Hsla,
    pub success: Hsla,
    pub busy: Hsla,

    pub selection: Hsla,
    pub code_text: Hsla,
    pub code_wash: Hsla,
    pub diff_add: Hsla,
    pub diff_del: Hsla,
    pub diff_hunk_bg: Hsla,
    pub claude: Hsla,

    pub font_sans: SharedString,
    pub font_mono: SharedString,

    pub terminal_bg: Hsla,
    pub terminal_fg: Hsla,
    pub terminal_selection: Hsla,
    pub ansi: [Hsla; 16],
    pub cursor: Hsla,
}

impl Global for Theme {}

/// Terminal ANSI palettes (Zeron's, MIT).
const ANSI_DARK: [u32; 16] = [
    0x242424, 0xf87171, 0x4ade80, 0xfacc15, 0x60a5fa, 0xc084fc, 0x22d3ee, 0xd4d4d8, 0x52525b, 0xfca5a5, 0x86efac,
    0xfde047, 0x93c5fd, 0xd8b4fe, 0x67e8f9, 0xfafafa,
];
const ANSI_LIGHT: [u32; 16] = [
    0x1f1f1f, 0xdc2626, 0x16a34a, 0xb45309, 0x2563eb, 0x9333ea, 0x0e7490, 0x3f3f46, 0x71717a, 0xb91c1c, 0x15803d,
    0x92400e, 0x1d4ed8, 0x7e22ce, 0x155e75, 0x18181b,
];

fn palette(hex: [u32; 16]) -> [Hsla; 16] {
    hex.map(|h| gpui::rgb(h).into())
}

impl Theme {
    pub const TITLEBAR_HEIGHT: f32 = 36.0;
    pub const TITLEBAR_TOP_PAD: f32 = 4.0;
    pub const BUBBLE_RADIUS: f32 = 18.0;
    pub const COMPOSER_RADIUS: f32 = 20.0;
    /// Frost tint coverage over the blurred window. Linux compositors do not
    /// all blur, so it stays opaque there.
    pub const GLASS_ALPHA: f32 = if cfg!(any(target_os = "macos", target_os = "windows")) { 0.80 } else { 1.0 };

    pub fn of(cx: &App) -> &Theme {
        cx.global::<Theme>()
    }

    pub fn new(appearance: Appearance, accent_choice: Accent, frosted: bool) -> Self {
        let (primary, strong) = accent_choice.pair(appearance);
        let dark = appearance.is_dark();
        if dark {
            Self {
                appearance,
                accent_choice,
                frosted,
                bg: grey(6),
                surface: grey(13),
                surface_raised: neutral(0.235),
                surface_card: grey(0x0e),
                surface_overlay: grey(0x16),
                element_hover: hsla(0.0, 0.0, 0.92, 0.11),
                border: hsla(0.0, 0.0, 1.0, 0.08),
                border_strong: hsla(0.0, 0.0, 1.0, 0.14),
                text: neutral(0.922),
                text_muted: neutral(0.708),
                text_faint: neutral(0.556),
                solid: neutral(0.922),
                on_solid: grey(0x0e),
                accent: primary,
                accent_strong: strong,
                danger: oklch(0.704, 0.191, 22.216),
                warning: oklch(0.828, 0.189, 84.429),
                success: oklch(0.765, 0.177, 163.223),
                busy: primary,
                selection: primary.opacity(0.35),
                code_text: primary,
                code_wash: primary.opacity(0.12),
                diff_add: oklch(0.765, 0.177, 163.223),
                diff_del: oklch(0.704, 0.191, 22.216),
                diff_hunk_bg: hsla(0.6, 0.35, 0.6, 0.05),
                claude: oklch(0.70, 0.15, 45.0),
                font_sans: "Geist".into(),
                font_mono: "Geist Mono".into(),
                terminal_bg: grey(0x09),
                terminal_fg: hsla(0.0, 0.0, 0.91, 1.0),
                terminal_selection: hsla(0.0, 0.0, 1.0, 0.22),
                ansi: palette(ANSI_DARK),
                cursor: hsla(0.0, 0.0, 1.0, 0.35),
            }
        } else {
            Self {
                appearance,
                accent_choice,
                frosted,
                bg: grey(0xff),
                surface: neutral(0.968),
                surface_raised: neutral(0.940),
                surface_card: grey(0xff),
                surface_overlay: grey(0xff),
                element_hover: hsla(0.0, 0.0, 0.10, 0.06),
                border: hsla(0.0, 0.0, 0.0, 0.10),
                border_strong: hsla(0.0, 0.0, 0.0, 0.17),
                text: neutral(0.25),
                text_muted: neutral(0.439),
                text_faint: neutral(0.535),
                solid: neutral(0.205),
                on_solid: neutral(0.985),
                accent: primary,
                accent_strong: strong,
                danger: oklch(0.577, 0.245, 27.325),
                warning: oklch(0.555, 0.163, 48.998),
                success: oklch(0.596, 0.145, 163.225),
                busy: primary,
                selection: primary.opacity(0.24),
                code_text: primary,
                code_wash: primary.opacity(0.10),
                diff_add: oklch(0.596, 0.145, 163.225),
                diff_del: oklch(0.577, 0.245, 27.325),
                diff_hunk_bg: hsla(0.6, 0.35, 0.4, 0.06),
                claude: oklch(0.60, 0.16, 45.0),
                font_sans: "Geist".into(),
                font_mono: "Geist Mono".into(),
                terminal_bg: grey(0xfa),
                terminal_fg: neutral(0.25),
                terminal_selection: hsla(0.0, 0.0, 0.0, 0.16),
                ansi: palette(ANSI_LIGHT),
                cursor: hsla(0.0, 0.0, 0.0, 0.55),
            }
        }
    }

    /// The shell tint painted over the blurred window background.
    pub fn glass(&self) -> Hsla {
        if self.frosted { self.surface.opacity(Self::GLASS_ALPHA) } else { self.surface }
    }

    pub fn is_frost(&self) -> bool {
        self.glass().a < 1.0
    }

    pub fn glass_hover(&self) -> Hsla {
        self.element_hover
    }

    /// Popovers and menus over glass.
    pub fn glass_overlay(&self) -> Hsla {
        if !self.is_frost() {
            return self.surface_overlay;
        }
        if self.appearance.is_dark() { self.surface_overlay.opacity(0.78) } else { self.surface_overlay.opacity(0.72) }
    }

    /// The content plane; translucent on glass so the frost reads through.
    pub fn panel_bg(&self) -> Hsla {
        if self.is_frost() { self.bg.opacity(0.55) } else { self.bg }
    }

    /// A translucent wash for hover/selection that keeps the glass visible.
    pub fn wash(&self, alpha: f32) -> Hsla {
        if self.appearance.is_dark() { hsla(0.0, 0.0, 0.92, alpha) } else { hsla(0.0, 0.0, 0.10, alpha * 0.55) }
    }

    /// Syntax colors, Zeron's palette: indigo / pink / emerald / amber hue
    /// families at 72% saturation, so code reads colorful but calm.
    pub fn syntax(&self, kind: zeron_syntax::HighlightKind) -> Hsla {
        use zeron_syntax::HighlightKind as K;
        let tone = |mut c: Hsla| {
            c.s *= 0.72;
            c
        };
        let dark = self.appearance.is_dark();
        let indigo = tone(if dark { oklch(0.673, 0.182, 276.935) } else { oklch(0.47, 0.20, 276.966) });
        let pink = tone(if dark { oklch(0.718, 0.202, 349.761) } else { oklch(0.47, 0.17, 0.584) });
        let emerald = tone(if dark { oklch(0.765, 0.177, 163.223) } else { oklch(0.46, 0.11, 163.225) });
        let amber = tone(if dark { oklch(0.828, 0.189, 84.429) } else { oklch(0.47, 0.12, 48.998) });
        match kind {
            K::Comment => self.text_faint,
            K::Keyword | K::Function | K::MarkupHeading | K::MarkupStrong => indigo,
            K::String | K::TypeBuiltin | K::Constant | K::MarkupRaw => emerald,
            K::StringSpecial
            | K::Escape
            | K::FunctionBuiltin
            | K::Macro
            | K::VariableSpecial
            | K::Tag
            | K::MarkupLink
            | K::MarkupEmphasis => pink,
            K::Number
            | K::Boolean
            | K::Type
            | K::Constructor
            | K::Property
            | K::Attribute
            | K::Label
            | K::MarkupReference => amber,
            K::Invalid => tone(self.danger),
            K::Variable | K::Parameter | K::Operator | K::Punctuation | K::Embedded => self.text,
        }
    }

    pub fn window_background(&self) -> gpui::WindowBackgroundAppearance {
        if self.is_frost() {
            gpui::WindowBackgroundAppearance::Blurred
        } else {
            gpui::WindowBackgroundAppearance::Opaque
        }
    }
}

/// The current appearance for free-standing helpers that have no `cx`
/// (ported Zeron surfaces). `apply_theme` keeps it in step.
static DARK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_current_appearance(a: Appearance) {
    DARK.store(a.is_dark(), std::sync::atomic::Ordering::Relaxed);
}

fn current_dark() -> bool {
    DARK.load(std::sync::atomic::Ordering::Relaxed)
}

/// A neutral overlay at `alpha`: white over dark themes, black over light.
pub fn ink(alpha: f32) -> Hsla {
    if current_dark() { hsla(0.0, 0.0, 1.0, alpha) } else { hsla(0.0, 0.0, 0.0, alpha) }
}

/// The hover / selection wash, as [`Theme::wash`].
pub fn wash(alpha: f32) -> Hsla {
    if current_dark() { hsla(0.0, 0.0, 0.92, alpha) } else { hsla(0.0, 0.0, 0.10, alpha * 0.55) }
}

impl Theme {
    /// Popups share the window's theme in Dove.
    pub fn for_popup(&self) -> Theme {
        self.clone()
    }

    pub fn is_glass(&self) -> bool {
        self.is_frost()
    }
}

pub fn appearance_from_window(a: WindowAppearance) -> Appearance {
    match a {
        WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
        _ => Appearance::Dark,
    }
}

/// An exact achromatic tone from an 8-bit value (`grey(13)` is `#0d0d0d`).
pub fn grey(value: u8) -> Hsla {
    hsla(0.0, 0.0, value as f32 / 255.0, 1.0)
}

/// A neutral OKLCH tone.
pub fn neutral(lightness: f32) -> Hsla {
    let [v, _, _] = oklch_to_srgb(lightness, 0.0, 0.0);
    hsla(0.0, 0.0, v, 1.0)
}

/// OKLCH (CSS notation: L 0..1, C, H degrees) to GPUI's HSLA.
pub fn oklch(l: f32, c: f32, h_deg: f32) -> Hsla {
    let [r, g, b] = oklch_to_srgb(l, c, h_deg);
    let (h, s, l) = rgb_to_hsl(r, g, b);
    hsla(h, s, l, 1.0)
}

fn oklch_to_srgb(l: f32, c: f32, h_deg: f32) -> [f32; 3] {
    let h = h_deg.to_radians();
    let (a, b) = (c * h.cos(), c * h.sin());
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let r = 4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_93 * s3;
    let g = -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_4 * s3;
    let b = -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3;
    [gamma_encode(r), gamma_encode(g), gamma_encode(b)]
}

fn gamma_encode(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if x <= 0.003_130_8 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 }
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let delta = max - min;
    if delta < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 { delta / (2.0 - max - min) } else { delta / (max + min) };
    let h = if (max - r).abs() < f32::EPSILON {
        ((g - b) / delta).rem_euclid(6.0)
    } else if (max - g).abs() < f32::EPSILON {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    } / 6.0;
    (h, s, l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutrals_are_achromatic_and_ordered() {
        let (a, b) = (neutral(0.3), neutral(0.8));
        assert_eq!(a.s, 0.0);
        assert!(a.l < b.l);
    }

    #[test]
    fn grey_matches_byte_value() {
        assert!((grey(255).l - 1.0).abs() < 1e-6);
        assert!((grey(0).l).abs() < 1e-6);
    }

    #[test]
    fn frost_follows_flag() {
        let t = Theme::new(Appearance::Dark, Accent::Sky, false);
        assert!(!t.is_frost());
        let t = Theme::new(Appearance::Dark, Accent::Sky, true);
        assert_eq!(t.is_frost(), Theme::GLASS_ALPHA < 1.0);
    }
}
