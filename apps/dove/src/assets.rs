//! Embedded icons and fonts, served to GPUI.
//!
//! Icons are Solar Icons (Linear, CC BY 4.0, 480 Design) and agent brand
//! marks, adopted from Zeron's icon set. GPUI tints SVGs with the text color.

use std::borrow::Cow;

use gpui::{App, AssetSource, Result, SharedString, Styled as _, Svg, px, svg};

/// Every embedded icon has a constant, used or not.
#[allow(dead_code)]
pub mod icons {
    include!(concat!(env!("OUT_DIR"), "/icons_gen.rs"));
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(icons::icon_bytes(path).map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(icons::ALL_ICONS.iter().filter(|p| p.starts_with(path)).map(|p| SharedString::from(*p)).collect())
    }
}

/// The color icons paint in unless a caller sets one (SVGs do not inherit
/// the parent's text color). Kept in step with the theme by `apply_theme`.
static ICON_COLOR: std::sync::RwLock<gpui::Hsla> =
    std::sync::RwLock::new(gpui::Hsla { h: 0.0, s: 0.0, l: 0.6, a: 1.0 });

pub fn set_icon_color(color: gpui::Hsla) {
    if let Ok(mut c) = ICON_COLOR.write() {
        *c = color;
    }
}

/// An icon at the default 16px. Chain `.size()` / `.text_color()` to adjust.
pub fn icon(path: &'static str) -> Svg {
    let color = ICON_COLOR.read().map(|c| *c).unwrap_or(gpui::Hsla { h: 0.0, s: 0.0, l: 0.6, a: 1.0 });
    svg().path(path).size(px(16.0)).flex_none().text_color(color)
}

const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Geist.ttf"),
    include_bytes!("../assets/fonts/Geist-Medium.ttf"),
    include_bytes!("../assets/fonts/Geist-SemiBold.ttf"),
    include_bytes!("../assets/fonts/GeistMono.ttf"),
    include_bytes!("../assets/fonts/GeistMono-Medium.ttf"),
];

pub fn load_fonts(cx: &mut App) {
    let fonts = FONTS.iter().map(|f| Cow::Borrowed(*f)).collect();
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        tracing::warn!("bundled fonts not loaded: {e:#}");
    }
}
