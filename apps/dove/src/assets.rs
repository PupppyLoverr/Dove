//! Embedded icons and fonts, served to GPUI.
//!
//! The control set is Zeron's: Solar Icons (Linear, CC BY 4.0, 480 Design)
//! plus the hand-drawn glyphs Zeron uses for the terminal, git family, and
//! window chrome. They are served as drawn — stroke, fill, and viewBox stay
//! in the file. Agent brand marks keep their own art. GPUI tints `currentColor`
//! with the element's text color.

use std::borrow::Cow;

use gpui::{
    App, AssetSource, InteractiveElement, Interactivity, IntoElement, Result, SharedString, StyleRefinement, Styled,
    Svg, Transformation, px, svg,
};

/// Every embedded icon has a constant, used or not.
#[allow(dead_code)]
pub mod icons {
    include!(concat!(env!("OUT_DIR"), "/icons_gen.rs"));
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match icons::icon_bytes(path) {
            Some(bytes) => Ok(Some(Cow::Borrowed(bytes))),
            // Zeron's file-type icons (`file-icons/...`).
            None => crate::file_icons::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all: Vec<SharedString> =
            icons::ALL_ICONS.iter().filter(|p| p.starts_with(path)).map(|p| SharedString::from(*p)).collect();
        all.extend(crate::file_icons::Assets.list(path)?);
        Ok(all)
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
pub fn icon(path: &'static str) -> Icon {
    let color = ICON_COLOR.read().map(|c| *c).unwrap_or(gpui::Hsla { h: 0.0, s: 0.0, l: 0.6, a: 1.0 });
    Icon { path, svg: svg().size(px(16.0)).flex_none().text_color(color) }
}

/// An embedded icon. Size and color are on the element; the SVG is served as drawn.
pub struct Icon {
    path: &'static str,
    svg: Svg,
}

impl Icon {
    pub fn with_transformation(mut self, transformation: Transformation) -> Self {
        self.svg = self.svg.with_transformation(transformation);
        self
    }
}

impl Styled for Icon {
    fn style(&mut self) -> &mut StyleRefinement {
        self.svg.style()
    }
}

impl InteractiveElement for Icon {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.svg.interactivity()
    }
}

impl IntoElement for Icon {
    type Element = Svg;

    fn into_element(self) -> Svg {
        self.svg.path(self.path)
    }
}

const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Geist.ttf"),
    include_bytes!("../assets/fonts/Geist-Medium.ttf"),
    include_bytes!("../assets/fonts/Geist-SemiBold.ttf"),
    include_bytes!("../assets/fonts/GeistMono.ttf"),
    include_bytes!("../assets/fonts/GeistMono-Medium.ttf"),
];

/// The natural dove mark, decoded from the embedded photo.
pub fn brand_icon() -> Option<image::RgbaImage> {
    image::load_from_memory(include_bytes!("../assets/dove-icon.jpg")).ok().map(|img| img.to_rgba8())
}

/// `brand_icon` scaled to a square, for the tray and other small slots.
pub fn brand_icon_square(size: u32) -> Option<image::RgbaImage> {
    let img = brand_icon()?;
    Some(image::imageops::resize(&img, size, size, image::imageops::FilterType::Lanczos3))
}

pub fn load_fonts(cx: &mut App) {
    let fonts = FONTS.iter().map(|f| Cow::Borrowed(*f)).collect();
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        tracing::warn!("bundled fonts not loaded: {e:#}");
    }
}
