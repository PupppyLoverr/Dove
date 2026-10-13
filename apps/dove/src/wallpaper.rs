//! The new-session hero: a user-chosen wallpaper rendered through a retro
//! effect and faded into the canvas, with the composer cut out of it.
//! Effects and mask geometry adapted from Zeron (MIT). Decoding and effect
//! rasterization run once per image on the background executor.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use gpui::{App, Bounds, Corners, ImageAlphaMask, Pixels, RenderImage, Window, point, px, size};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Effect {
    None,
    #[default]
    Dither,
    Halftone,
    Scanlines,
}

impl Effect {
    pub const ALL: [Effect; 4] = [Self::None, Self::Dither, Self::Halftone, Self::Scanlines];

    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Dither => "dither",
            Self::Halftone => "halftone",
            Self::Scanlines => "scanlines",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "Original",
            Self::Dither => "Dither",
            Self::Halftone => "Halftone",
            Self::Scanlines => "Scanlines",
        }
    }

    pub fn from_id(id: &str) -> Self {
        Self::ALL.into_iter().find(|e| e.id() == id).unwrap_or_default()
    }
}

/// Fraction of the viewport the hero covers, and its cap.
pub const VIEWPORT_RATIO: f32 = 0.72;
pub const MAX_HEIGHT: f32 = 760.0;
/// Hero opacity over frosted glass.
pub const FROSTED_OPACITY: f32 = 0.84;
/// The cutout's soft underlay keeps the composer area faintly lit.
pub const CUTOUT_REVEAL_OPACITY: f32 = 0.5;

struct Source {
    width: u32,
    height: u32,
    rgba: Box<[[u8; 4]]>,
    luma: Box<[u8]>,
}

type Slot = Arc<Mutex<Option<Arc<RenderImage>>>>;
struct Entry {
    path: PathBuf,
    effect: Effect,
    light: bool,
    slot: Slot,
}

static CACHE: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();

/// The rendered artwork for `path`, or `None` while it is still decoding
/// (the window refreshes when it is ready).
pub fn prepare(path: &Path, effect: Effect, light: bool, cx: &mut App) -> Option<Arc<RenderImage>> {
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    let light = light && matches!(effect, Effect::Halftone | Effect::Scanlines);
    {
        let entries = cache.lock().ok()?;
        if let Some(e) = entries.iter().find(|e| e.path == path && e.effect == effect && e.light == light) {
            return e.slot.lock().ok()?.clone();
        }
    }
    let slot: Slot = Arc::new(Mutex::new(None));
    {
        let mut entries = cache.lock().ok()?;
        entries.push(Entry { path: path.to_path_buf(), effect, light, slot: slot.clone() });
        // A handful of variants is plenty; drop the oldest.
        if entries.len() > 4 {
            entries.remove(0);
        }
    }
    let path = path.to_path_buf();
    let work = cx.background_executor().spawn(async move {
        let bytes = std::fs::read(&path).ok()?;
        let img = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.decode().ok()?;
        let proxy = img.thumbnail(2048, 2048);
        let src = Source {
            width: proxy.width(),
            height: proxy.height(),
            luma: proxy.to_luma8().into_raw().into_boxed_slice(),
            rgba: proxy.to_rgba8().pixels().map(|p| p.0).collect(),
        };
        Some(Arc::new(RenderImage::new([image::Frame::new(render(&src, effect, light))])))
    });
    cx.spawn(async move |cx| {
        let image = work.await;
        *slot.lock().unwrap() = image;
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
    None
}

/// GPUI's RenderImage consumes BGRA.
fn render(src: &Source, effect: Effect, light: bool) -> image::RgbaImage {
    let (w, h) = (src.width, src.height);
    let px_at = |x: u32, y: u32| src.rgba[(y * w + x) as usize];
    match effect {
        Effect::None => image::RgbaImage::from_fn(w, h, |x, y| {
            let [r, g, b, a] = px_at(x, y);
            image::Rgba([b, g, r, a])
        }),
        Effect::Scanlines => image::RgbaImage::from_fn(w, h, |x, y| {
            let [r, g, b, a] = px_at(x, y);
            let gain = if y % 3 == 0 { 0.52 } else { 1.0 };
            let c = |v: u8| {
                if light { (v as f32 + (255.0 - v as f32) * (1.0 - gain)) as u8 } else { (v as f32 * gain) as u8 }
            };
            image::Rgba([c(b), c(g), c(r), a])
        }),
        Effect::Dither => {
            const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
            let mut out = image::RgbaImage::new(w, h);
            for y in (0..h).step_by(2) {
                for x in (0..w).step_by(2) {
                    let [r, g, b, a] = dither(
                        px_at((x + 1).min(w - 1), (y + 1).min(h - 1)),
                        BAYER[y as usize / 2 % 4][x as usize / 2 % 4],
                    );
                    for dy in 0..2.min(h - y) {
                        for dx in 0..2.min(w - x) {
                            out.put_pixel(x + dx, y + dy, image::Rgba([b, g, r, a]));
                        }
                    }
                }
            }
            out
        }
        Effect::Halftone => {
            let paper = if light { 255 } else { 0 };
            let mut out = image::RgbaImage::from_pixel(w, h, image::Rgba([paper, paper, paper, 255]));
            for y in (0..h).step_by(4) {
                for x in (0..w).step_by(4) {
                    let cx_ = (x + 2).min(w - 1);
                    let cy_ = (y + 2).min(h - 1);
                    let luma = src.luma[(cy_ * w + cx_) as usize];
                    let luma = if light { 255 - luma } else { luma };
                    let radius = 2.0 * (0.3 + 0.7 * (luma as f32 / 255.0).sqrt());
                    let [r, g, b, a] = px_at(cx_, cy_);
                    for dy in 0..4.min(h - y) {
                        for dx in 0..4.min(w - x) {
                            let d = ((dx as f32 - 1.5).powi(2) + (dy as f32 - 1.5).powi(2)).sqrt();
                            let cov = (radius + 0.5 - d).clamp(0.0, 1.0) * a as f32 / 255.0;
                            let [sr, sg, sb, sa] = px_at(x + dx, y + dy);
                            let blend = |s: u8, dot: u8| {
                                (s as f32 * 0.6 + (dot as f32 * cov + paper as f32 * (1.0 - cov)) * 0.4) as u8
                            };
                            out.put_pixel(x + dx, y + dy, image::Rgba([blend(sb, b), blend(sg, g), blend(sr, r), sa]));
                        }
                    }
                }
            }
            out
        }
    }
}

fn dither([r, g, b, a]: [u8; 4], threshold: u8) -> [u8; 4] {
    let peak = r.max(g).max(b) as f32;
    let bright = peak / 255.0 > (threshold as f32 + 0.5) / 16.0;
    let gain = if bright { 255.0 / peak.max(1.0) } else { 0.08 };
    [(r as f32 * gain).round() as u8, (g as f32 * gain).round() as u8, (b as f32 * gain).round() as u8, a]
}

pub fn hero_height(viewport_height: f32) -> f32 {
    (viewport_height.max(0.0) * VIEWPORT_RATIO).min(MAX_HEIGHT)
}

/// Paint `image` cover-fitted into `bounds`, fading toward the bottom.
/// With `cutout`, the composer's rounded rect is feathered out of it.
pub fn paint(
    image: Arc<RenderImage>,
    bounds: Bounds<Pixels>,
    composer: Option<Bounds<Pixels>>,
    cutout: bool,
    window: &mut Window,
) {
    let src = image.size(0);
    let (sw, sh) = ((src.width.0 as f32).max(1.0), (src.height.0 as f32).max(1.0));
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let scale = (w / sw).max(h / sh);
    let (fw, fh) = (sw * scale, sh * scale);
    let fitted =
        Bounds::new(point(bounds.left() - px((fw - w) * 0.5), bounds.top() - px((fh - h) * 0.5)), size(px(fw), px(fh)));
    let mask = match (cutout, composer) {
        (true, Some(c)) => ImageAlphaMask {
            bounds: Bounds::new(c.origin, size(c.size.width, c.bottom().max(bounds.bottom()) - c.top())),
            radius: px(crate::theme::Theme::COMPOSER_RADIUS),
            feather: px((h * 0.52).clamp(120.0, 280.0)),
            clearance: px(8.0),
            bottom_fade: Some((bounds.bottom(), px(h.max(1.0)))),
        },
        // No exclusion: park the rect below the image so only the fade applies.
        _ => ImageAlphaMask {
            bounds: Bounds::new(point(bounds.left(), bounds.bottom() + px(1.0)), bounds.size),
            radius: px(0.0),
            feather: px(1.0),
            clearance: px(0.0),
            bottom_fade: Some((bounds.bottom(), px(h.max(1.0)))),
        },
    };
    let _ = window.paint_image_fitted_masked(bounds, fitted, Corners::default(), image, 0, false, Some(mask));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(w: u32, h: u32) -> Source {
        Source {
            width: w,
            height: h,
            rgba: vec![[200, 100, 50, 255]; (w * h) as usize].into_boxed_slice(),
            luma: vec![128; (w * h) as usize].into_boxed_slice(),
        }
    }

    #[test]
    fn effects_keep_dimensions_and_output_bgra() {
        for e in Effect::ALL {
            let img = render(&src(9, 7), e, false);
            assert_eq!((img.width(), img.height()), (9, 7), "{e:?}");
        }
        let none = render(&src(1, 1), Effect::None, false);
        assert_eq!(none.get_pixel(0, 0).0, [50, 100, 200, 255]);
    }

    #[test]
    fn dither_is_binary_per_channel_gain() {
        assert_eq!(dither([255, 255, 255, 255], 0), [255, 255, 255, 255]);
        let dark = dither([10, 10, 10, 255], 15);
        assert!(dark[0] < 10);
    }

    #[test]
    fn hero_height_caps() {
        assert_eq!(hero_height(2000.0), MAX_HEIGHT);
        assert!((hero_height(500.0) - 360.0).abs() < 1e-3);
    }
}
