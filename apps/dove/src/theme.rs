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
    Amber,
    Cyan,
    Blue,
}

impl Accent {
    pub const ALL: [Accent; 8] =
        [Self::Sky, Self::Indigo, Self::Orange, Self::Green, Self::Pink, Self::Amber, Self::Cyan, Self::Blue];

    pub fn id(self) -> &'static str {
        match self {
            Self::Sky => "sky",
            Self::Indigo => "indigo",
            Self::Orange => "orange",
            Self::Green => "green",
            Self::Pink => "pink",
            Self::Amber => "amber",
            Self::Cyan => "cyan",
            Self::Blue => "blue",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Sky => "Sky",
            Self::Indigo => "Indigo",
            Self::Orange => "Orange",
            Self::Green => "Green",
            Self::Pink => "Pink",
            Self::Amber => "Amber",
            Self::Cyan => "Cyan",
            Self::Blue => "Blue",
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
            // Zeron's extra presets (#fbbf24 / #22d3ee / #60a5fa dark).
            Self::Amber if dark => (oklch(0.837, 0.164, 84.4), oklch(0.666, 0.157, 58.3)),
            Self::Amber => (oklch(0.555, 0.146, 49.0), oklch(0.555, 0.146, 49.0)),
            Self::Cyan if dark => (oklch(0.797, 0.134, 211.5), oklch(0.609, 0.111, 221.7)),
            Self::Cyan => (oklch(0.52, 0.094, 223.1), oklch(0.52, 0.094, 223.1)),
            Self::Blue if dark => (oklch(0.707, 0.165, 254.6), oklch(0.546, 0.215, 262.9)),
            Self::Blue => (oklch(0.546, 0.215, 262.9), oklch(0.546, 0.215, 262.9)),
        }
    }
}

/// The material the chrome and floating surfaces (the composer above all)
/// are made of. Glass is Zeron's frosted recipe: a faint tint over a scene
/// blur, so what is behind stays legible as shapes. Acrylic is the denser
/// material: more tint, a wider blur, a calmer read. Normal is opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Surface {
    #[default]
    Glass,
    Acrylic,
    Normal,
}

impl Surface {
    pub const ALL: [Surface; 3] = [Self::Glass, Self::Acrylic, Self::Normal];

    pub fn id(self) -> &'static str {
        match self {
            Self::Glass => "glass",
            Self::Acrylic => "acrylic",
            Self::Normal => "normal",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Glass => "Glass",
            Self::Acrylic => "Acrylic",
            Self::Normal => "Normal",
        }
    }

    /// `surface` wins; before it existed the only choice was the `frosted`
    /// switch, whose "off" was the opaque look.
    pub fn from_settings(surface: Option<&str>, legacy_frosted: Option<&str>) -> Self {
        match surface {
            Some(id) => Self::ALL.into_iter().find(|s| s.id() == id).unwrap_or_default(),
            None if legacy_frosted == Some("false") => Self::Normal,
            None => Self::default(),
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // a token set: not every token is used yet
pub struct Theme {
    pub appearance: Appearance,
    pub accent_choice: Accent,
    /// What the chrome is made of: glass, acrylic or opaque.
    pub surface_style: Surface,

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

    /// Dialogs: one step above cards (Zeron's card ⊕ raised 18%).
    pub surface_dialog: Hsla,
    /// Pressed and selected rows: an accent tint.
    pub element_active: Hsla,
    /// Accent wash behind accent text (badges, selected chips).
    pub accent_wash: Hsla,
    /// Softer status colours for text on washes.
    pub danger_muted: Hsla,
    pub warning_muted: Hsla,
    pub success_muted: Hsla,
    /// Behind modals.
    pub scrim: Hsla,

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

    /// The library theme painted over Dove's look (`theme_light` /
    /// `theme_dark`); `None` is Dove's own palette.
    pub themed: Option<crate::themes::Applied>,
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
    pub const TITLEBAR_HEIGHT: f32 = crate::design::size::TITLEBAR;
    pub const TITLEBAR_TOP_PAD: f32 = 4.0;
    pub const BUBBLE_RADIUS: f32 = crate::design::radius::BUBBLE;
    /// Frost tint coverage over the blurred window: Zeron's 0.80, matched by
    /// eye to a vibrancy material. Linux compositors do not all blur, so it
    /// stays opaque there.
    pub const GLASS_ALPHA: f32 = if cfg!(any(target_os = "macos", target_os = "windows")) { 0.80 } else { 1.0 };
    /// Acrylic is the denser material: the desktop reads as colour, not shapes.
    pub const ACRYLIC_ALPHA: f32 = if cfg!(any(target_os = "macos", target_os = "windows")) { 0.90 } else { 1.0 };

    pub fn of(cx: &App) -> &Theme {
        cx.global::<Theme>()
    }

    pub fn new(appearance: Appearance, accent_choice: Accent, surface_style: Surface) -> Self {
        let (primary, strong) = accent_choice.pair(appearance);
        let dark = appearance.is_dark();
        let theme = if dark {
            Self {
                appearance,
                accent_choice,
                surface_style,
                // Charcoal planes, one step apart: canvas, sidebar, card,
                // menu, raised. Colour is kept for meaning.
                bg: grey(0x16),
                surface: grey(0x1b),
                surface_raised: grey(0x2b),
                surface_card: grey(0x1e),
                surface_overlay: grey(0x25),
                element_hover: hsla(0.0, 0.0, 0.92, 0.11),
                border: hsla(0.0, 0.0, 1.0, 0.08),
                border_strong: hsla(0.0, 0.0, 1.0, 0.14),
                text: neutral(0.922),
                text_muted: neutral(0.708),
                text_faint: neutral(0.556),
                solid: neutral(0.922),
                on_solid: grey(0x16),
                accent: primary,
                accent_strong: strong,
                danger: oklch(0.704, 0.191, 22.216),
                warning: oklch(0.828, 0.189, 84.429),
                success: oklch(0.765, 0.177, 163.223),
                busy: primary,
                surface_dialog: grey(0x23),
                element_active: primary.opacity(0.18),
                accent_wash: primary.opacity(0.22),
                danger_muted: mix(oklch(0.704, 0.191, 22.216), neutral(0.922), 0.28),
                warning_muted: mix(oklch(0.828, 0.189, 84.429), neutral(0.922), 0.25),
                success_muted: mix(oklch(0.765, 0.177, 163.223), neutral(0.922), 0.25),
                scrim: hsla(0.0, 0.0, 0.0, 0.60),
                selection: primary.opacity(0.35),
                code_text: primary,
                code_wash: primary.opacity(0.12),
                diff_add: oklch(0.765, 0.177, 163.223),
                diff_del: oklch(0.704, 0.191, 22.216),
                diff_hunk_bg: hsla(0.6, 0.35, 0.6, 0.05),
                claude: oklch(0.70, 0.15, 45.0),
                font_sans: "Inter".into(),
                font_mono: "JetBrains Mono".into(),
                terminal_bg: grey(0x13),
                terminal_fg: hsla(0.0, 0.0, 0.91, 1.0),
                terminal_selection: hsla(0.0, 0.0, 1.0, 0.22),
                ansi: palette(ANSI_DARK),
                cursor: hsla(0.0, 0.0, 1.0, 0.35),
                themed: None,
            }
        } else {
            Self {
                appearance,
                accent_choice,
                surface_style,
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
                surface_dialog: mix(grey(0xff), neutral(0.940), 0.04),
                element_active: primary.opacity(0.10),
                accent_wash: primary.opacity(0.12),
                danger_muted: mix(oklch(0.577, 0.245, 27.325), neutral(0.25), 0.28),
                warning_muted: mix(oklch(0.555, 0.163, 48.998), neutral(0.25), 0.25),
                success_muted: mix(oklch(0.596, 0.145, 163.225), neutral(0.25), 0.25),
                scrim: hsla(0.0, 0.0, 0.0, 0.32),
                selection: primary.opacity(0.24),
                code_text: primary,
                code_wash: primary.opacity(0.10),
                diff_add: oklch(0.596, 0.145, 163.225),
                diff_del: oklch(0.577, 0.245, 27.325),
                diff_hunk_bg: hsla(0.6, 0.35, 0.4, 0.06),
                claude: oklch(0.60, 0.16, 45.0),
                font_sans: "Inter".into(),
                font_mono: "JetBrains Mono".into(),
                terminal_bg: grey(0xfa),
                terminal_fg: neutral(0.25),
                terminal_selection: hsla(0.0, 0.0, 0.0, 0.16),
                ansi: palette(ANSI_LIGHT),
                cursor: hsla(0.0, 0.0, 0.0, 0.55),
                themed: None,
            }
        };
        theme.enforce_contrast()
    }

    /// [`Theme::new`] with a library theme's palette laid over it (see
    /// [`crate::themes`]); `None` is exactly [`Theme::new`].
    pub fn with_library(
        appearance: Appearance,
        accent_choice: Accent,
        surface_style: Surface,
        library: Option<crate::themes::Applied>,
    ) -> Self {
        let base = Self::new(appearance, accent_choice, surface_style);
        match library {
            Some(applied) => applied.apply(base).enforce_contrast(),
            None => base,
        }
    }

    /// Raise `text_muted` and `text_faint` until they read on the content
    /// plane (WCAG 4.5:1 and 3:1), as Zeron's theme does for every palette.
    fn enforce_contrast(mut self) -> Self {
        let plane = flatten(self.panel_bg(), flatten(self.glass(), self.bg));
        self.text_muted = ensure_contrast(self.text_muted, plane, 4.5);
        self.text_faint = ensure_contrast(self.text_faint, plane, 3.0);
        self
    }

    /// The shell tint painted over the blurred window background.
    pub fn glass(&self) -> Hsla {
        match self.surface_style {
            Surface::Glass => self.surface.opacity(Self::GLASS_ALPHA),
            Surface::Acrylic => self.surface.opacity(Self::ACRYLIC_ALPHA),
            Surface::Normal => self.surface,
        }
    }

    pub fn is_frost(&self) -> bool {
        self.glass().a < 1.0
    }

    /// The composer's fill. Glass is Zeron's dark frosted tint: a 15% plate
    /// solved so the bar lands on the content plane's tone, leaving the blur
    /// to do the work. Acrylic is a matte tint that hides shapes behind it.
    /// Normal is an opaque plate one step above the page.
    pub fn composer_bg(&self) -> Hsla {
        let dark = self.appearance.is_dark();
        match (self.surface_style, dark) {
            _ if !self.is_frost() => {
                if dark {
                    grey(0x21)
                } else {
                    self.bg
                }
            }
            (Surface::Acrylic, true) => grey(0x20).opacity(0.66),
            (Surface::Acrylic, false) => self.bg.opacity(0.70),
            (_, true) => self.composer_glass_tint(),
            (_, false) => self.bg.opacity(0.35),
        }
    }

    /// The new-thread bar sits on artwork rather than the page, so it needs
    /// more body than the thread pill while keeping the material's character.
    pub fn composer_hero_bg(&self) -> Hsla {
        let dark = self.appearance.is_dark();
        match (self.surface_style, dark) {
            _ if !self.is_frost() => {
                if dark {
                    grey(0x21)
                } else {
                    self.bg
                }
            }
            (Surface::Acrylic, true) => self.surface_overlay.opacity(0.74),
            (Surface::Acrylic, false) => self.bg.opacity(0.84),
            (_, true) => self.surface_overlay.opacity(0.52),
            (_, false) => self.bg.opacity(0.66),
        }
    }

    /// Zeron's `composer_sidebar_tint`: the colour that, laid at 15% over the
    /// window glass, reproduces the content plane, so a frosted bar reads as
    /// the same material as the page around it.
    fn composer_glass_tint(&self) -> Hsla {
        let canvas = flatten(self.glass(), self.bg);
        let target = flatten(self.panel_bg(), canvas);
        let (canvas, target) = (hsl_to_rgb(canvas.h, canvas.s, canvas.l), hsl_to_rgb(target.h, target.s, target.l));
        let mut alpha: f32 = 0.60;
        for (base, desired) in canvas.into_iter().zip(target) {
            let needed = if desired > base {
                (desired - base) / (1.0 - base).max(f32::EPSILON)
            } else {
                (base - desired) / base.max(f32::EPSILON)
            };
            alpha = alpha.max(needed);
        }
        let rgb: [f32; 3] = std::array::from_fn(|i| ((target[i] - canvas[i] * (1.0 - alpha)) / alpha).clamp(0.0, 1.0));
        let (h, s, l) = rgb_to_hsl(rgb[0], rgb[1], rgb[2]);
        hsla(h, s, l, 0.15)
    }

    /// Backdrop blur behind the composer; zero when it is opaque. Acrylic
    /// blurs wider, so what is behind reads as colour rather than shapes.
    pub fn composer_blur(&self) -> f32 {
        match self.surface_style {
            _ if !self.is_frost() => 0.0,
            Surface::Acrylic => 30.0,
            _ => 16.0,
        }
    }

    /// The composer's edge: Zeron's translucent cool silver on glass, a
    /// softer neutral rim on acrylic, the theme's strong border when opaque.
    pub fn composer_border(&self) -> Hsla {
        if !self.is_frost() {
            return self.border_strong;
        }
        match (self.surface_style, self.appearance) {
            (Surface::Glass, Appearance::Dark) => hsla(210.0 / 360.0, 0.18, 0.78, 0.11),
            (Surface::Glass, Appearance::Light) => hsla(210.0 / 360.0, 0.18, 0.32, 0.12),
            (_, Appearance::Dark) => hsla(0.0, 0.0, 1.0, 0.08),
            (_, Appearance::Light) => hsla(0.0, 0.0, 0.0, 0.09),
        }
    }

    /// Light and depth for the composer. Glass is lit from above (a specular
    /// top edge and a faint bottom glint, like Zeron's glass plates); acrylic
    /// is matte with a soft inner glow; the opaque plate floats on a shadow.
    pub fn composer_shadows(&self, focused: bool) -> Vec<gpui::BoxShadow> {
        let dark = self.appearance.is_dark();
        let white = |a: f32| hsla(0.0, 0.0, 1.0, a);
        let black = |a: f32| hsla(0.0, 0.0, 0.0, a);
        let lift = if focused { 1.0 } else { 0.0 };
        match self.surface_style {
            _ if !self.is_frost() => vec![
                shadow(white(if dark { 0.05 } else { 0.9 }), 1.0, 0.0, 0.0, true),
                shadow(black(if dark { 0.40 } else { 0.08 } + 0.06 * lift), 10.0, 28.0, -4.0, false),
                shadow(black(if dark { 0.25 } else { 0.05 }), 1.0, 3.0, 0.0, false),
            ],
            Surface::Glass => vec![
                shadow(white(if dark { 0.10 } else { 0.70 }), 1.0, 0.0, 0.0, true),
                shadow(white(if dark { 0.03 } else { 0.25 }), -1.0, 0.0, 0.0, true),
                shadow(white(if dark { 0.02 + 0.02 * lift } else { 0.10 }), 0.0, 18.0, 0.0, true),
            ],
            _ => vec![
                shadow(white(if dark { 0.05 } else { 0.45 }), 1.0, 0.0, 0.0, true),
                shadow(white(if dark { 0.015 + 0.015 * lift } else { 0.06 }), 0.0, 24.0, 0.0, true),
            ],
        }
    }

    pub fn glass_hover(&self) -> Hsla {
        self.element_hover
    }

    /// Popovers and menus over glass: Zeron's tint, raised only as far as
    /// menu text needs to stay legible over a bright backdrop. Acrylic menus
    /// are near-solid.
    pub fn glass_overlay(&self) -> Hsla {
        if !self.is_frost() {
            return self.surface_overlay;
        }
        let dark = self.appearance.is_dark();
        match self.surface_style {
            Surface::Acrylic => self.surface_overlay.opacity(if dark { 0.88 } else { 0.84 }),
            _ if !dark => self.surface_overlay.opacity(0.62),
            _ => self.surface_overlay.opacity(self.legible_alpha(self.surface_overlay, 0.66)),
        }
    }

    /// The content plane; translucent on glass so the frost reads through
    /// (Zeron's 40%), denser on acrylic, solid when opaque.
    pub fn panel_bg(&self) -> Hsla {
        match self.surface_style {
            _ if !self.is_frost() => self.bg,
            Surface::Acrylic => self.bg.opacity(0.62),
            _ => self.bg.opacity(0.40),
        }
    }

    /// The least coverage of `tint`, from `base` up, at which primary and
    /// muted text stay legible over the window glass on the worst-case
    /// desktop (white behind a dark theme, black behind a light one).
    fn legible_alpha(&self, tint: Hsla, base: f32) -> f32 {
        let worst = if self.appearance.is_dark() { grey(0xff) } else { grey(0) };
        let backdrop = flatten(self.glass(), worst);
        for step in 0..=20 {
            let alpha = base + (1.0 - base) * step as f32 / 20.0;
            let plate = flatten(tint.opacity(alpha), backdrop);
            if contrast(flatten(self.text, plate), plate) >= 4.5
                && contrast(flatten(self.text_muted, plate), plate) >= 3.0
            {
                return alpha;
            }
        }
        1.0
    }

    /// A translucent wash for hover/selection that keeps the glass visible.
    pub fn wash(&self, alpha: f32) -> Hsla {
        if self.appearance.is_dark() { hsla(0.0, 0.0, 0.92, alpha) } else { hsla(0.0, 0.0, 0.10, alpha * 0.55) }
    }

    /// Syntax colors, Zeron's palette: indigo / pink / emerald / amber hue
    /// families at 72% saturation, so code reads colorful but calm.
    pub fn syntax(&self, kind: zeron_syntax::HighlightKind) -> Hsla {
        use zeron_syntax::HighlightKind as K;
        if let Some(color) = self.themed.as_ref().and_then(|applied| applied.syntax(kind)) {
            return color;
        }
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

fn shadow(color: Hsla, y: f32, blur: f32, spread: f32, inset: bool) -> gpui::BoxShadow {
    gpui::BoxShadow {
        color,
        offset: gpui::point(gpui::px(0.0), gpui::px(y)),
        blur_radius: gpui::px(blur),
        spread_radius: gpui::px(spread),
        inset,
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
    #[allow(dead_code)] // Zeron parity
    pub fn for_popup(&self) -> Theme {
        self.clone()
    }

    #[allow(dead_code)] // Zeron parity
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

/// Blend `a` toward `b` by `t` in sRGB (opaque results).
pub fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let t = t.clamp(0.0, 1.0);
    let [ar, ag, ab] = hsl_to_rgb(a.h, a.s, a.l);
    let [br, bg, bb] = hsl_to_rgb(b.h, b.s, b.l);
    let (h, s, l) = rgb_to_hsl(ar + (br - ar) * t, ag + (bg - ag) * t, ab + (bb - ab) * t);
    hsla(h, s, l, 1.0)
}

/// Move `fg`'s lightness away from `bg` until their contrast reaches
/// `ratio` (or it can go no further).
pub fn ensure_contrast(fg: Hsla, bg: Hsla, ratio: f32) -> Hsla {
    if contrast(flatten(fg, bg), bg) >= ratio {
        return fg;
    }
    let lighten = bg.l < 0.5;
    let mut c = fg;
    for _ in 0..40 {
        c.l = (c.l + if lighten { 0.02 } else { -0.02 }).clamp(0.0, 1.0);
        if contrast(flatten(c, bg), bg) >= ratio || c.l <= 0.0 || c.l >= 1.0 {
            break;
        }
    }
    c
}

/// Composite a translucent `fg` over an opaque `bg`: the colour the eye gets.
pub fn flatten(fg: Hsla, bg: Hsla) -> Hsla {
    let a = fg.a.clamp(0.0, 1.0);
    let [fr, fg_, fb] = hsl_to_rgb(fg.h, fg.s, fg.l);
    let [br, bg_, bb] = hsl_to_rgb(bg.h, bg.s, bg.l);
    let (h, s, l) = rgb_to_hsl(fr * a + br * (1.0 - a), fg_ * a + bg_ * (1.0 - a), fb * a + bb * (1.0 - a));
    hsla(h, s, l, 1.0)
}

/// WCAG relative luminance of an opaque colour (0 to 1).
pub fn luminance(c: Hsla) -> f32 {
    let lin = |v: f32| if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    let [r, g, b] = hsl_to_rgb(c.h, c.s, c.l);
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// WCAG 2.1 contrast ratio between two opaque colours (1 to 21).
fn contrast(a: Hsla, b: Hsla) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    if s <= f32::EPSILON {
        return [l, l, l];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0)]
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
    fn muted_text_stays_legible_on_every_material() {
        for appearance in [Appearance::Dark, Appearance::Light] {
            for surface in Surface::ALL {
                for accent in Accent::ALL {
                    let t = Theme::new(appearance, accent, surface);
                    let plane = flatten(t.panel_bg(), flatten(t.glass(), t.bg));
                    assert!(contrast(flatten(t.text_muted, plane), plane) >= 4.4, "{appearance:?} {surface:?}");
                }
            }
        }
    }

    #[test]
    fn accent_ids_round_trip() {
        for a in Accent::ALL {
            assert_eq!(Accent::from_id(a.id()), a);
        }
    }

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
    fn frost_follows_surface() {
        let t = Theme::new(Appearance::Dark, Accent::Sky, Surface::Normal);
        assert!(!t.is_frost());
        assert_eq!(t.composer_blur(), 0.0);
        let t = Theme::new(Appearance::Dark, Accent::Sky, Surface::Glass);
        assert_eq!(t.is_frost(), Theme::GLASS_ALPHA < 1.0);
        let t = Theme::new(Appearance::Dark, Accent::Sky, Surface::Acrylic);
        assert_eq!(t.is_frost(), Theme::ACRYLIC_ALPHA < 1.0);
    }

    #[test]
    fn materials_get_denser_from_glass_to_normal() {
        for appearance in [Appearance::Dark, Appearance::Light] {
            let [glass, acrylic, normal] = Surface::ALL.map(|s| Theme::new(appearance, Accent::Sky, s));
            assert!(glass.panel_bg().a <= acrylic.panel_bg().a && acrylic.panel_bg().a <= normal.panel_bg().a);
            assert!(glass.composer_hero_bg().a <= acrylic.composer_hero_bg().a);
            assert_eq!(normal.composer_bg().a, 1.0);
            assert!(glass.glass_overlay().a <= 1.0 && acrylic.glass_overlay().a <= 1.0);
        }
    }

    #[test]
    fn flatten_composites_over_the_backdrop() {
        let half_white = hsla(0.0, 0.0, 1.0, 0.5);
        assert!((flatten(half_white, grey(0)).l - 0.5).abs() < 1e-3);
        assert!((contrast(grey(0xff), grey(0)) - 21.0).abs() < 0.01);
    }

    #[test]
    fn surface_reads_legacy_switch() {
        assert_eq!(Surface::from_settings(None, Some("false")), Surface::Normal);
        assert_eq!(Surface::from_settings(None, Some("true")), Surface::Glass);
        assert_eq!(Surface::from_settings(None, None), Surface::Glass);
        assert_eq!(Surface::from_settings(Some("acrylic"), Some("false")), Surface::Acrylic);
        for s in Surface::ALL {
            assert_eq!(Surface::from_settings(Some(s.id()), None), s);
        }
    }
}
