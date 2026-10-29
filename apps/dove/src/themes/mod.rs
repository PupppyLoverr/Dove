//! The theme library: Zeron's 30 built-in themes and VS Code theme import,
//! laid over Dove's own look.
//!
//! The palettes, their derivation and the VS Code mapping are ported from
//! Zeron's `crates/theme` (zeronsh/zeron, MIT, Copyright (c) 2026 Wing).
//! Dove keeps its own [`Theme`] as the token set every surface reads: a
//! library theme is a [`Palette`] that [`Applied::apply`] paints over
//! [`Theme::new`], after which Dove's contrast enforcement runs as usual.
//!
//! The light and dark choices live under `theme_light` and `theme_dark`;
//! unset (or `dove`) is Dove's own look. The `accent` key is unchanged: any of
//! Dove's accents can sit on any theme, and `theme` (the default while a
//! library theme is active and no accent was chosen) uses the theme's own.
//! Imported themes are saved as resolved palettes in `<DOVE_HOME>/themes`.

mod builtins;
pub mod vscode;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, OnceLock, RwLock};

use gpui::Hsla;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::theme::{Appearance, Theme};

/// Settings key for the theme used in light mode.
pub const LIGHT_KEY: &str = "theme_light";
/// Settings key for the theme used in dark mode.
pub const DARK_KEY: &str = "theme_dark";
/// The id of Dove's own look (also what an unset key means).
pub const DOVE_ID: &str = "dove";
/// The `accent` value that follows the active theme's own accent.
pub const THEME_ACCENT: &str = "theme";
/// Folder (under DOVE_HOME) holding imported themes.
const DIR: &str = "themes";

/// The settings key holding the theme for `appearance`.
pub fn key_for(appearance: Appearance) -> &'static str {
    if appearance.is_dark() { DARK_KEY } else { LIGHT_KEY }
}

// ---------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------

/// An sRGB colour with alpha, serialized as `#rrggbb` or `#rrggbbaa`
/// (Zeron's `Color`, with the same blending and contrast maths).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn opaque(self) -> Self {
        Self { a: 255, ..self }
    }

    pub fn with_alpha(self, alpha: f32) -> Self {
        Self { a: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8, ..self }
    }

    /// Composite over an opaque `background`.
    pub fn blend_over(self, background: Self) -> Self {
        let alpha = self.a as f32 / 255.0;
        let blend = |front: u8, back: u8| (front as f32 * alpha + back as f32 * (1.0 - alpha)).round() as u8;
        Self::rgb(blend(self.r, background.r), blend(self.g, background.g), blend(self.b, background.b))
    }

    pub fn mix(self, other: Self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
        Self::rgba(mix(self.r, other.r), mix(self.g, other.g), mix(self.b, other.b), mix(self.a, other.a))
    }

    /// WCAG contrast against an opaque `background` (translucent colours are
    /// composited first).
    pub fn contrast(self, background: Self) -> f32 {
        let foreground = if self.a == 255 { self } else { self.blend_over(background) };
        let (a, b) = (foreground.luminance(), background.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    pub fn best_on_color(self) -> Self {
        if Self::WHITE.contrast(self) >= Self::BLACK.contrast(self) { Self::WHITE } else { Self::BLACK }
    }

    /// Move toward black or white until the requested contrast is met.
    pub fn ensure_contrast(self, background: Self, minimum: f32) -> Self {
        if self.contrast(background) >= minimum {
            return self;
        }
        let target = if Self::BLACK.contrast(background) >= Self::WHITE.contrast(background) {
            Self::BLACK
        } else {
            Self::WHITE
        };
        for step in 1..=20 {
            let candidate = self.mix(target, step as f32 / 20.0);
            if candidate.contrast(background) >= minimum {
                return candidate;
            }
        }
        target
    }

    /// Whether this is a light colour (black text reads better on it).
    pub fn is_light(self) -> bool {
        Self::BLACK.contrast(self) > Self::WHITE.contrast(self)
    }

    fn luminance(self) -> f32 {
        let linear = |channel: u8| {
            let value = channel as f32 / 255.0;
            if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * linear(self.r) + 0.7152 * linear(self.g) + 0.0722 * linear(self.b)
    }

    pub fn hsla(self) -> Hsla {
        gpui::Rgba {
            r: self.r as f32 / 255.0,
            g: self.g as f32 / 255.0,
            b: self.b as f32 / 255.0,
            a: self.a as f32 / 255.0,
        }
        .into()
    }
}

impl fmt::Debug for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.a == 255 {
            write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            write!(f, "#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorParseError;

impl fmt::Display for ColorParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected a CSS hex color (#rgb, #rgba, #rrggbb or #rrggbbaa)")
    }
}

impl std::error::Error for ColorParseError {}

impl FromStr for Color {
    type Err = ColorParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim().strip_prefix('#').ok_or(ColorParseError)?;
        let nibble = |c: u8| match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err(ColorParseError),
        };
        let short = |c: u8| nibble(c).map(|n| (n << 4) | n);
        let pair = |b: &[u8]| -> Result<u8, ColorParseError> { Ok((nibble(b[0])? << 4) | nibble(b[1])?) };
        let b = value.as_bytes();
        match b.len() {
            3 => Ok(Self::rgb(short(b[0])?, short(b[1])?, short(b[2])?)),
            4 => Ok(Self::rgba(short(b[0])?, short(b[1])?, short(b[2])?, short(b[3])?)),
            6 => Ok(Self::rgb(pair(&b[0..2])?, pair(&b[2..4])?, pair(&b[4..6])?)),
            8 => Ok(Self::rgba(pair(&b[0..2])?, pair(&b[2..4])?, pair(&b[4..6])?, pair(&b[6..8])?)),
            _ => Err(ColorParseError),
        }
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------
// Palette
// ---------------------------------------------------------------------------

/// A complete, resolved theme variant: what a built-in derives from its seeds
/// and what an import writes to disk. Field names follow Zeron's
/// `ThemeColors`; [`Applied::apply`] maps them onto Dove's tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Palette {
    pub id: String,
    pub name: String,
    pub dark: bool,
    pub background: Color,
    pub shell: Color,
    pub raised: Color,
    pub card: Color,
    pub dialog: Color,
    pub overlay: Color,
    pub hover: Color,
    pub active: Color,
    pub border: Color,
    pub border_strong: Color,
    pub text: Color,
    pub text_muted: Color,
    pub text_faint: Color,
    pub solid: Color,
    pub on_solid: Color,
    pub accent: Color,
    pub danger: Color,
    pub warning: Color,
    pub success: Color,
    pub cursor: Color,
    pub diff_add: Color,
    pub diff_delete: Color,
    pub diff_hunk: Color,
    pub terminal_background: Color,
    pub terminal_foreground: Color,
    pub terminal_selection: Color,
    pub ansi: [Color; 16],
    /// Zeron's syntax role names (`keyword`, `stringSpecial`, …).
    #[serde(default)]
    pub syntax: BTreeMap<String, Color>,
}

impl Palette {
    pub fn appearance(&self) -> Appearance {
        if self.dark { Appearance::Dark } else { Appearance::Light }
    }
}

/// The Zeron syntax role a highlight kind is drawn with. Roles a palette
/// leaves out fall back to Dove's own syntax colours.
pub fn syntax_role(kind: zeron_syntax::HighlightKind) -> &'static str {
    use zeron_syntax::HighlightKind as K;
    match kind {
        K::Comment => "comment",
        K::Keyword => "keyword",
        K::String => "string",
        K::StringSpecial => "stringSpecial",
        K::Escape => "escape",
        K::Number => "number",
        K::Boolean => "boolean",
        K::Type => "type",
        K::TypeBuiltin => "typeBuiltin",
        K::Constructor => "constructor",
        K::Function => "function",
        K::FunctionBuiltin => "functionBuiltin",
        K::Macro => "macro",
        K::Property => "property",
        K::Constant => "constant",
        K::Variable => "variable",
        K::VariableSpecial => "variableSpecial",
        K::Parameter => "parameter",
        K::Operator => "operator",
        K::Punctuation | K::Embedded => "punctuation",
        K::Tag => "tag",
        K::Attribute => "attribute",
        K::Label => "label",
        K::MarkupHeading => "markupHeading",
        K::MarkupRaw => "markupRaw",
        K::MarkupLink => "markupLink",
        K::MarkupReference => "markupReference",
        K::MarkupEmphasis => "markupEmphasis",
        K::MarkupStrong => "markupStrong",
        K::Invalid => "invalid",
    }
}

/// A library theme as applied: the palette and whether the accent follows
/// the theme (rather than one of Dove's accents).
#[derive(Debug, Clone)]
pub struct Applied {
    pub palette: Arc<Palette>,
    pub theme_accent: bool,
}

impl Applied {
    /// Whether `self` and `other` paint the same thing (an import replaced
    /// in place is a new palette, so it re-themes).
    pub fn same(&self, other: &Applied) -> bool {
        Arc::ptr_eq(&self.palette, &other.palette) && self.theme_accent == other.theme_accent
    }

    /// Paint the palette over Dove's base theme. The caller runs Dove's
    /// contrast enforcement afterwards ([`Theme::with_library`]).
    pub fn apply(&self, mut t: Theme) -> Theme {
        let p = &*self.palette;
        let dark = p.dark;
        t.bg = p.background.hsla();
        t.surface = p.shell.hsla();
        t.surface_raised = p.raised.hsla();
        t.surface_card = p.card.hsla();
        t.surface_overlay = p.overlay.hsla();
        t.surface_dialog = p.dialog.hsla();
        t.element_hover = p.hover.hsla();
        t.border = p.border.hsla();
        t.border_strong = p.border_strong.hsla();
        t.text = p.text.hsla();
        t.text_muted = p.text_muted.hsla();
        t.text_faint = p.text_faint.hsla();
        t.solid = p.solid.hsla();
        t.on_solid = p.on_solid.hsla();
        t.danger = p.danger.hsla();
        t.warning = p.warning.hsla();
        t.success = p.success.hsla();
        t.danger_muted = p.danger.mix(p.text, 0.28).hsla();
        t.warning_muted = p.warning.mix(p.text, 0.25).hsla();
        t.success_muted = p.success.mix(p.text, 0.25).hsla();
        t.diff_add = p.diff_add.hsla();
        t.diff_del = p.diff_delete.hsla();
        t.diff_hunk_bg = p.diff_hunk.hsla();
        t.terminal_bg = p.terminal_background.hsla();
        t.terminal_fg = p.terminal_foreground.hsla();
        t.terminal_selection = p.terminal_selection.hsla();
        t.ansi = p.ansi.map(Color::hsla);
        t.cursor = p.cursor.hsla();
        if self.theme_accent {
            let primary = p.accent;
            t.accent = primary.hsla();
            t.accent_strong = primary.mix(Color::BLACK, if dark { 0.18 } else { 0.26 }).hsla();
            t.busy = t.accent;
            t.element_active = p.active.hsla();
            t.accent_wash = primary.with_alpha(if dark { 0.22 } else { 0.12 }).hsla();
            t.selection = primary.with_alpha(if dark { 0.35 } else { 0.24 }).hsla();
            t.code_text = t.accent;
            t.code_wash = primary.with_alpha(if dark { 0.12 } else { 0.10 }).hsla();
        } else {
            // One of Dove's accents on the theme's plane: keep it legible.
            let accent = crate::theme::ensure_contrast(t.accent, t.bg, 3.0);
            t.accent = accent;
            t.busy = accent;
            t.code_text = accent;
        }
        t.themed = Some(self.clone());
        t
    }

    /// The palette's colour for a highlight kind, if it defines one.
    pub fn syntax(&self, kind: zeron_syntax::HighlightKind) -> Option<Hsla> {
        self.palette.syntax.get(syntax_role(kind)).map(|c| c.hsla())
    }
}

// ---------------------------------------------------------------------------
// The registry
// ---------------------------------------------------------------------------

fn builtin() -> &'static [Arc<Palette>] {
    static BUILTIN: OnceLock<Vec<Arc<Palette>>> = OnceLock::new();
    BUILTIN.get_or_init(|| builtins::all().into_iter().map(Arc::new).collect())
}

static CUSTOM: RwLock<Vec<Arc<Palette>>> = RwLock::new(Vec::new());

fn custom() -> Vec<Arc<Palette>> {
    CUSTOM.read().map(|c| c.clone()).unwrap_or_default()
}

/// A built-in or imported theme by id.
pub fn find(id: &str) -> Option<Arc<Palette>> {
    builtin().iter().find(|p| p.id == id).cloned().or_else(|| custom().into_iter().find(|p| p.id == id))
}

/// The palette to paint for `appearance`, given the stored choice. `None`
/// means Dove's own look: unset, `dove`, unknown (an import since deleted),
/// or a theme of the other appearance.
pub fn resolve(id: Option<&str>, appearance: Appearance) -> Option<Arc<Palette>> {
    let id = id.filter(|id| !id.is_empty() && *id != DOVE_ID)?;
    find(id).filter(|p| p.dark == appearance.is_dark())
}

/// Whether the accent follows the theme: chosen as `theme`, or no accent
/// chosen while a library theme is active.
pub fn follows_theme_accent(accent: Option<&str>) -> bool {
    matches!(accent, None | Some("") | Some(THEME_ACCENT))
}

/// One entry of a theme picker.
#[derive(Debug, Clone)]
pub struct Choice {
    pub id: String,
    pub name: String,
    pub imported: bool,
    /// The theme's accent, for the picker's swatch (`None` for Dove's look).
    pub accent: Option<Color>,
}

/// Every theme for `appearance`: Dove's own look first, then the built-ins in
/// Zeron's order, then imports by name.
pub fn choices(appearance: Appearance) -> Vec<Choice> {
    let dark = appearance.is_dark();
    let mut out = vec![Choice { id: DOVE_ID.into(), name: "Keplorayx".into(), imported: false, accent: None }];
    let entry = |p: &Arc<Palette>, imported: bool| Choice {
        id: p.id.clone(),
        name: p.name.clone(),
        imported,
        accent: Some(p.accent),
    };
    out.extend(builtin().iter().filter(|p| p.dark == dark).map(|p| entry(p, false)));
    let mut imported: Vec<_> = custom().into_iter().filter(|p| p.dark == dark).collect();
    imported.sort_by_key(|p| p.name.to_lowercase());
    out.extend(imported.iter().map(|p| entry(p, true)));
    out
}

fn dir(root: &Path) -> PathBuf {
    root.join(DIR)
}

/// Load the imported themes saved under `root` (DOVE_HOME). Unreadable or
/// invalid files are skipped; the rest replace the in-memory list.
pub fn load_custom(root: &Path) {
    let mut loaded: Vec<Arc<Palette>> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir(root)) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(palette) =
                std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<Palette>(&s).ok())
            else {
                tracing::warn!("skipping unreadable theme {}", path.display());
                continue;
            };
            if find_builtin(&palette.id) || loaded.iter().any(|p| p.id == palette.id) {
                continue;
            }
            loaded.push(Arc::new(palette));
        }
    }
    if let Ok(mut custom) = CUSTOM.write() {
        *custom = loaded;
    }
}

fn find_builtin(id: &str) -> bool {
    builtin().iter().any(|p| p.id == id)
}

/// Save an imported palette under `root` and add (or replace) it in the
/// library.
pub fn install(palette: Palette, root: &Path) -> anyhow::Result<Arc<Palette>> {
    anyhow::ensure!(!find_builtin(&palette.id), "a built-in theme already uses the id {}", palette.id);
    let folder = dir(root);
    std::fs::create_dir_all(&folder)?;
    let path = folder.join(format!("{}.json", palette.id));
    let tmp = folder.join(format!("{}.json.tmp", palette.id));
    std::fs::write(&tmp, serde_json::to_vec_pretty(&palette)?)?;
    std::fs::rename(&tmp, &path)?;
    let palette = Arc::new(palette);
    if let Ok(mut custom) = CUSTOM.write() {
        custom.retain(|p| p.id != palette.id);
        custom.push(palette.clone());
    }
    Ok(palette)
}

/// Import a VS Code theme file: parse, map, save under `root`, register.
pub fn import_vscode(path: &Path, root: &Path) -> anyhow::Result<Arc<Palette>> {
    install(vscode::import_file(path)?, root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Accent, Surface};

    #[test]
    fn thirty_builtins_with_unique_ids() {
        let all = builtin();
        assert_eq!(all.len(), 30);
        let mut ids: Vec<_> = all.iter().map(|p| p.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 30);
        assert!(!ids.contains(&DOVE_ID));
        assert_eq!(all.iter().filter(|p| p.dark).count(), 20);
        assert_eq!(all.iter().filter(|p| !p.dark).count(), 10);
    }

    #[test]
    fn builtins_are_legible() {
        for p in builtin() {
            assert!(p.text.contrast(p.background) >= 4.5, "{} text", p.id);
            assert!(p.text_muted.contrast(p.background) >= 4.5, "{} muted", p.id);
            assert!(p.accent.contrast(p.background) >= 3.0, "{} accent", p.id);
            assert!(p.terminal_foreground.contrast(p.terminal_background) >= 4.5, "{} terminal", p.id);
            assert_eq!(p.dark, !p.background.is_light(), "{} appearance", p.id);
        }
    }

    #[test]
    fn lookup_resolves_by_id_and_appearance() {
        assert!(find("dracula").is_some());
        assert!(find("nope").is_none());
        assert_eq!(resolve(Some("dracula"), Appearance::Dark).unwrap().name, "Dracula");
        assert_eq!(resolve(Some("catppuccin-latte"), Appearance::Light).unwrap().id, "catppuccin-latte");
        // The other appearance's theme, Dove's own look and unknown ids all
        // fall back to Dove.
        assert!(resolve(Some("dracula"), Appearance::Light).is_none());
        assert!(resolve(Some(DOVE_ID), Appearance::Dark).is_none());
        assert!(resolve(None, Appearance::Dark).is_none());
        assert!(resolve(Some(""), Appearance::Light).is_none());
        assert!(resolve(Some("imported-gone"), Appearance::Dark).is_none());
    }

    #[test]
    fn choices_start_with_dove_and_split_by_appearance() {
        for appearance in [Appearance::Dark, Appearance::Light] {
            let list = choices(appearance);
            assert_eq!(list[0].id, DOVE_ID);
            assert!(list[0].accent.is_none());
            for c in list.iter().skip(1).filter(|c| !c.imported) {
                assert_eq!(find(&c.id).unwrap().dark, appearance.is_dark(), "{}", c.id);
            }
        }
        assert!(choices(Appearance::Dark).iter().any(|c| c.id == "zeron-dark"));
        assert!(choices(Appearance::Light).iter().any(|c| c.id == "zeron-light"));
    }

    #[test]
    fn colors_parse_and_round_trip() {
        for source in ["#abc", "#abcd", "#102030", "#10203040"] {
            let color: Color = source.parse().unwrap();
            let json = serde_json::to_string(&color).unwrap();
            assert_eq!(serde_json::from_str::<Color>(&json).unwrap(), color);
        }
        assert_eq!("#fff".parse::<Color>().unwrap(), Color::WHITE);
        assert!("fff".parse::<Color>().is_err());
        assert!("#ggg".parse::<Color>().is_err());
    }

    #[test]
    fn no_library_theme_is_exactly_dove() {
        for appearance in [Appearance::Dark, Appearance::Light] {
            let a = Theme::new(appearance, Accent::Sky, Surface::Glass);
            let b = Theme::with_library(appearance, Accent::Sky, Surface::Glass, None);
            assert_eq!(a.bg, b.bg);
            assert_eq!(a.text_muted, b.text_muted);
            assert_eq!(a.accent, b.accent);
            assert!(b.themed.is_none());
        }
    }

    #[test]
    fn applying_a_palette_repaints_the_tokens() {
        let palette = find("dracula").unwrap();
        let applied = Applied { palette: palette.clone(), theme_accent: true };
        let t = Theme::with_library(Appearance::Dark, Accent::Sky, Surface::Normal, Some(applied));
        assert_eq!(t.bg, palette.background.hsla());
        assert_eq!(t.text, palette.text.hsla());
        assert_eq!(t.accent, palette.accent.hsla());
        assert_eq!(t.terminal_bg, palette.terminal_background.hsla());
        assert_eq!(t.syntax(zeron_syntax::HighlightKind::Keyword), palette.syntax["keyword"].hsla());
        assert!(t.themed.is_some());

        // A Dove accent stays chosen on a library theme.
        let applied = Applied { palette: palette.clone(), theme_accent: false };
        let dove = Theme::new(Appearance::Dark, Accent::Orange, Surface::Normal);
        let t = Theme::with_library(Appearance::Dark, Accent::Orange, Surface::Normal, Some(applied));
        assert_eq!(t.bg, palette.background.hsla());
        assert_eq!(t.accent_choice, Accent::Orange);
        assert!((t.accent.h - dove.accent.h).abs() < 0.02);
    }

    #[test]
    fn same_tracks_palette_identity() {
        let p = find("nord").unwrap();
        let a = Applied { palette: p.clone(), theme_accent: true };
        assert!(a.same(&Applied { palette: p.clone(), theme_accent: true }));
        assert!(!a.same(&Applied { palette: p.clone(), theme_accent: false }));
        let copy = Arc::new((*p).clone());
        assert!(!a.same(&Applied { palette: copy, theme_accent: true }));
    }

    #[test]
    fn accent_follows_theme_only_when_unset_or_chosen() {
        assert!(follows_theme_accent(None));
        assert!(follows_theme_accent(Some("")));
        assert!(follows_theme_accent(Some(THEME_ACCENT)));
        assert!(!follows_theme_accent(Some("sky")));
        assert!(!follows_theme_accent(Some("orange")));
    }

    #[test]
    fn imports_persist_under_dove_home() {
        let root = std::env::temp_dir().join(format!("dove-themes-{}", uuid::Uuid::new_v4()));
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        let file = source.join("my-theme.json");
        std::fs::write(
            &file,
            r##"{
                // JSONC, as VS Code writes it
                "name": "Persisted Test Theme",
                "type": "dark",
                "colors": { "editor.background": "#101418", "foreground": "#e0e0e0", },
            }"##,
        )
        .unwrap();
        let palette = import_vscode(&file, &root).unwrap();
        assert_eq!(palette.id, "imported-persisted-test-theme");
        assert!(root.join(DIR).join("imported-persisted-test-theme.json").is_file());
        assert!(choices(Appearance::Dark).iter().any(|c| c.id == palette.id && c.imported));

        // A fresh load (next launch) finds it again.
        load_custom(&root);
        let again = resolve(Some(&palette.id), Appearance::Dark).unwrap();
        assert_eq!(*again, *palette);
        let _ = std::fs::remove_dir_all(&root);
    }
}
