//! The new-session hero: a user-chosen wallpaper rendered through a retro
//! effect and faded into the canvas, with the composer cut out of it.
//! Effects and mask geometry adapted from Zeron (MIT). Decoding and effect
//! rasterization run once per image on the background executor.
//!
//! "Animate backdrop" never rasterizes per frame. Each effect is decomposed
//! into a few textures built once with the still (the dithered aircraft at
//! every heading, contrail strips, wave masks, a halftone ring, a scanline
//! texture), and a frame only places them: paint transforms (offset and
//! scale), clip rects and opacity. Frames are paced by the display
//! (`request_animation_frame`) and requested only by the [`Hero`] view, only
//! while it is drawn and decorative motion is allowed.

use std::cell::Cell;
use std::f32::consts::TAU;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use gpui::{
    App, Bounds, Context, Corners, ImageAlphaMask, IntoElement, ParentElement as _, Pixels, Render, RenderImage,
    Styled as _, Window, canvas, div, point, px, size,
};

use crate::motion;

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

    /// What "Animate backdrop" does with this effect.
    pub fn motion_hint(self) -> &'static str {
        match self {
            Self::None => "Slow push toward the brightest part of the image",
            Self::Dither => "An aircraft crosses toward the brightest point, trailing light in the image's colors",
            Self::Halftone => "Rings of dots ripple outward from the brightest point",
            Self::Scanlines => "CRT roll: the lines scroll and a soft band sweeps down",
        }
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

/// Only Halftone and Scanlines have a light-paper variant.
fn paper_light(effect: Effect, light: bool) -> bool {
    light && matches!(effect, Effect::Halftone | Effect::Scanlines)
}

/// The device-pixel size an effect is rasterized at, in 64 px steps.
fn step_target(target: (f32, f32)) -> (u32, u32) {
    let step = |v: f32| ((v.max(64.0) / 64.0).ceil() as u32 * 64).min(8192);
    (step(target.0), step(target.1))
}

// ------------------------------------------------------------ atlas hygiene

/// Rasters no longer shown, freed from the sprite atlas at the next paint
/// (dropping needs the window, and the background jobs that replace them
/// have none).
static RETIRED: Mutex<Vec<Arc<RenderImage>>> = Mutex::new(Vec::new());

fn retire(images: impl IntoIterator<Item = Arc<RenderImage>>) {
    if let Ok(mut retired) = RETIRED.lock() {
        retired.extend(images);
    }
}

fn drop_retired(window: &mut Window) {
    let images = RETIRED.lock().map(|mut r| std::mem::take(&mut *r)).unwrap_or_default();
    for image in images {
        let _ = window.drop_image(image);
    }
}

// ------------------------------------------------------------ static still

type Slot = Arc<Mutex<Option<Arc<RenderImage>>>>;
struct Entry {
    path: PathBuf,
    effect: Effect,
    light: bool,
    /// The device-pixel size it was rasterized for, in 64 px steps.
    target: (u32, u32),
    slot: Slot,
    /// The previous size's raster, shown until this one is ready.
    fallback: Option<Arc<RenderImage>>,
}

impl Entry {
    fn images(self) -> impl Iterator<Item = Arc<RenderImage>> {
        let ready = self.slot.lock().ok().and_then(|s| s.clone());
        ready.into_iter().chain(self.fallback)
    }
}

/// Still rasters, most recently used last.
static CACHE: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();

/// The rendered artwork for `path`, or `None` while it is still decoding
/// (the window refreshes when it is ready). `target` is the hero's size in
/// device pixels: the effect is rasterized at that size, as Zeron does, so a
/// dither or halftone cell is the same few screen pixels on every display
/// instead of growing with the source image's upscale.
fn prepare(path: &Path, effect: Effect, light: bool, target: (f32, f32), cx: &mut App) -> Option<Arc<RenderImage>> {
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    let light = paper_light(effect, light);
    let target = step_target(target);
    let mut stale = None;
    {
        let mut entries = cache.lock().ok()?;
        if let Some(i) = entries.iter().position(|e| e.path == path && e.effect == effect && e.light == light) {
            let e = entries.remove(i);
            let ready = e.slot.lock().ok()?.clone();
            let (hit, fallback) = (e.target == target, e.fallback.clone());
            entries.push(e);
            if hit {
                return ready.or(fallback);
            }
            // Resized: keep showing the old raster until the new one lands.
            stale = ready;
        }
    }
    let slot: Slot = Arc::new(Mutex::new(None));
    {
        let mut entries = cache.lock().ok()?;
        let mut gone = Vec::new();
        while let Some(i) = entries.iter().position(|e| e.path == path && e.effect == effect && e.light == light) {
            gone.extend(entries.remove(i).fallback);
        }
        entries.push(Entry {
            path: path.to_path_buf(),
            effect,
            light,
            target,
            slot: slot.clone(),
            fallback: stale.clone(),
        });
        // A handful of variants is plenty; drop the least recently used.
        while entries.len() > 4 {
            gone.extend(entries.remove(0).images());
        }
        retire(gone);
    }
    let path = path.to_path_buf();
    let work = cx.background_executor().spawn(async move {
        if effect == Effect::Dither {
            return zeron_dither(&path, target).map(raster);
        }
        let src = load_source(&path, target)?;
        Some(raster(render(&src, effect, light)))
    });
    cx.spawn(async move |cx| {
        let image = work.await;
        *slot.lock().unwrap() = image;
        // The resized raster replaces the old one, which can go.
        if let Some(cache) = CACHE.get()
            && let Ok(mut entries) = cache.lock()
            && let Some(e) = entries.iter_mut().find(|e| Arc::ptr_eq(&e.slot, &slot))
        {
            retire(e.fallback.take());
        }
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
    stale
}

// ------------------------------------------------------------ settings preview

/// The thumbnail's size in Settings, in logical pixels (16:10, like most
/// windows the art lands in).
pub const PREVIEW_SIZE: (f32, f32) = (128.0, 80.0);

/// The one thumbnail Settings shows. Kept apart from the hero's cache: the
/// cache keys on image and effect, so a small raster there would push out
/// the full-size one and both would rebuild on every visit.
static PREVIEW: Mutex<Option<(PathBuf, Effect, bool, Slot)>> = Mutex::new(None);

/// The chosen image as Settings previews it: the effect applied, rasterized
/// small. `None` while it decodes (the window refreshes when it is ready).
pub fn preview(path: &Path, effect: Effect, light: bool, scale: f32, cx: &mut App) -> Option<Arc<RenderImage>> {
    let light = paper_light(effect, light);
    let mut guard = PREVIEW.lock().ok()?;
    if let Some((p, e, l, slot)) = guard.as_ref()
        && p == path
        && *e == effect
        && *l == light
    {
        return slot.lock().ok()?.clone();
    }
    let slot: Slot = Arc::new(Mutex::new(None));
    if let Some((_, _, _, old)) = guard.replace((path.to_path_buf(), effect, light, slot.clone())) {
        retire(old.lock().ok().and_then(|s| s.clone()));
    }
    drop(guard);
    let target = step_target((PREVIEW_SIZE.0 * scale, PREVIEW_SIZE.1 * scale));
    let path = path.to_path_buf();
    let work = cx.background_executor().spawn(async move {
        if effect == Effect::Dither {
            return zeron_dither(&path, target).map(raster);
        }
        let src = load_source(&path, target)?;
        Some(raster(render(&src, effect, light)))
    });
    cx.spawn(async move |cx| {
        let image = work.await;
        if let Ok(mut s) = slot.lock() {
            *s = image;
        }
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
    None
}

fn raster(img: image::RgbaImage) -> Arc<RenderImage> {
    Arc::new(RenderImage::new([image::Frame::new(img)]))
}

fn raster_frames(frames: Vec<image::RgbaImage>) -> Arc<RenderImage> {
    Arc::new(RenderImage::new(frames.into_iter().map(image::Frame::new).collect::<Vec<_>>()))
}

/// GPUI's RenderImage consumes BGRA.
fn render(src: &Source, effect: Effect, light: bool) -> image::RgbaImage {
    let (w, h) = (src.width, src.height);
    let px_at = |x: u32, y: u32| src.rgba[(y * w + x) as usize];
    match effect {
        Effect::None => plain(src),
        Effect::Scanlines => scanlines(src, light),
        Effect::Dither => {
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
        Effect::Halftone => halftone(src, light),
    }
}

/// The image as is, BGRA.
fn plain(src: &Source) -> image::RgbaImage {
    image::RgbaImage::from_fn(src.width, src.height, |x, y| {
        let [r, g, b, a] = src.rgba[(y * src.width + x) as usize];
        image::Rgba([b, g, r, a])
    })
}

/// How much a scanline darkens (or, on light paper, lifts) its row.
const SCANLINE_ALPHA: f32 = 0.48;

/// Every third row darkened (or lifted, on light paper).
fn scanlines(src: &Source, light: bool) -> image::RgbaImage {
    let (w, h) = (src.width, src.height);
    let mut out = image::RgbaImage::new(w, h);
    let gain = 1.0 - SCANLINE_ALPHA;
    let mut lut = [0u8; 256];
    for (v, slot) in lut.iter_mut().enumerate() {
        let v = v as f32;
        *slot = if light { (v + (255.0 - v) * (1.0 - gain)) as u8 } else { (v * gain) as u8 };
    }
    for y in 0..h {
        let lined = y.is_multiple_of(3);
        for x in 0..w {
            let [r, g, b, a] = src.rgba[(y * w + x) as usize];
            let px = if lined { [lut[b as usize], lut[g as usize], lut[r as usize], a] } else { [b, g, r, a] };
            out.put_pixel(x, y, image::Rgba(px));
        }
    }
    out
}

/// 4 px dots sized by luma.
fn halftone(src: &Source, light: bool) -> image::RgbaImage {
    let (w, h) = (src.width, src.height);
    let px_at = |x: u32, y: u32| src.rgba[(y * w + x) as usize];
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
                    let blend =
                        |s: u8, dot: u8| (s as f32 * 0.6 + (dot as f32 * cov + paper as f32 * (1.0 - cov)) * 0.4) as u8;
                    out.put_pixel(x + dx, y + dy, image::Rgba([blend(sb, b), blend(sg, g), blend(sr, r), sa]));
                }
            }
        }
    }
    out
}

/// Zeron's dither, exactly: the photo shrunk to at most 2048 px, dithered at
/// that size in 2x2 dots on the 4x4 Bayer rule, then cropped (not rescaled)
/// to the hero's shape. The hero stretches it to fit, so the dots scale with
/// the picture, as in Zeron, rather than sitting on a fixed screen-pixel grid.
fn zeron_dither(path: &Path, target: (u32, u32)) -> Option<image::RgbaImage> {
    let bytes = std::fs::read(path).ok()?;
    let img = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.decode().ok()?;
    let proxy = img.thumbnail(2048, 2048);
    let src = Source {
        width: proxy.width(),
        height: proxy.height(),
        luma: proxy.to_luma8().into_raw().into_boxed_slice(),
        rgba: proxy.to_rgba8().pixels().map(|p| p.0).collect(),
    };
    let full = render(&src, Effect::Dither, false);
    let (w, h) = (src.width.max(1), src.height.max(1));
    let (tw, th) = (target.0.max(1) as f32, target.1.max(1) as f32);
    let scale = (tw / w as f32).max(th / h as f32);
    let cw = ((tw / scale).round() as u32).clamp(1, w);
    let ch = ((th / scale).round() as u32).clamp(1, h);
    Some(image::imageops::crop_imm(&full, (w - cw) / 2, (h - ch) / 2, cw, ch).to_image())
}

/// Decode `path` and cover-crop it to the hero's own pixel size.
fn load_source(path: &Path, target: (u32, u32)) -> Option<Source> {
    let bytes = std::fs::read(path).ok()?;
    let img = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.decode().ok()?;
    let proxy = img.resize_to_fill(target.0, target.1, image::imageops::FilterType::Triangle);
    Some(Source {
        width: proxy.width(),
        height: proxy.height(),
        luma: proxy.to_luma8().into_raw().into_boxed_slice(),
        rgba: proxy.to_rgba8().pixels().map(|p| p.0).collect(),
    })
}

fn dither([r, g, b, a]: [u8; 4], threshold: u8) -> [u8; 4] {
    let peak = r.max(g).max(b) as f32;
    let bright = peak / 255.0 > (threshold as f32 + 0.5) / 16.0;
    let gain = if bright { 255.0 / peak.max(1.0) } else { 0.08 };
    [(r as f32 * gain).round() as u8, (g as f32 * gain).round() as u8, (b as f32 * gain).round() as u8, a]
}

const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// The Bayer threshold of 2x2 cell `(cx, cy)`, 0..1.
fn bayer(cx: u32, cy: u32) -> f32 {
    (BAYER[cy as usize % 4][cx as usize % 4] as f32 + 0.5) / 16.0
}

// ------------------------------------------------------------ analysis
//
// Motion is derived from the image (its brightest region and two dominant
// hues), so it works on any wallpaper.

#[derive(Debug, Clone, Copy, PartialEq)]
struct Analysis {
    /// Brightest blurred region, normalized to the hero (y down).
    focal: (f32, f32),
    /// Two dominant saturated hues, RGB 0..1.
    hues: [[f32; 3]; 2],
}

/// Grey art falls back to the blue and orange of the original dither demo.
const FALLBACK_HUES: [[f32; 3]; 2] = [[0.28, 0.48, 1.0], [1.0, 0.51, 0.18]];

/// Find the focal point and accent hues on a 48-wide grid of block averages.
fn analyze(src: &Source) -> Analysis {
    const GW: u32 = 48;
    let gh = ((GW as f32 * src.height as f32 / src.width.max(1) as f32).round() as u32).clamp(8, 96);
    let n = (GW * gh) as usize;
    let mut luma = vec![0f32; n];
    let mut bins = [0f32; 12];
    let mut total = 0.0;
    for gy in 0..gh {
        let (y0, y1) =
            (gy * src.height / gh, ((gy + 1) * src.height / gh).max(gy * src.height / gh + 1).min(src.height));
        for gx in 0..GW {
            let (x0, x1) =
                (gx * src.width / GW, ((gx + 1) * src.width / GW).max(gx * src.width / GW + 1).min(src.width));
            let mut sum = [0f32; 4];
            let mut count = 0.0;
            for y in y0..y1 {
                for x in x0..x1 {
                    for (s, v) in sum.iter_mut().zip(src.rgba[(y * src.width + x) as usize]) {
                        *s += v as f32;
                    }
                    count += 1.0;
                }
            }
            if count == 0.0 {
                continue;
            }
            let [r, g, b, a] = sum.map(|s| s / count / 255.0);
            luma[(gy * GW + gx) as usize] = (0.299 * r + 0.587 * g + 0.114 * b) * a;
            let (max, min) = (r.max(g).max(b), r.min(g).min(b));
            let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
            if sat > 0.2 && max > 0.15 && a > 0.5 {
                let weight = sat * max;
                bins[(hue(r, g, b) * 12.0) as usize % 12] += weight;
                total += weight;
            }
        }
    }

    // Two 3x3 box blurs, so the focus is a region rather than a speck.
    for _ in 0..2 {
        let mut out = vec![0f32; n];
        for y in 0..gh as i32 {
            for x in 0..GW as i32 {
                let (mut sum, mut count) = (0.0, 0.0);
                for (dx, dy) in (-1..=1).flat_map(|dx| (-1..=1).map(move |dy| (dx, dy))) {
                    let (xx, yy) = (x + dx, y + dy);
                    if xx >= 0 && yy >= 0 && xx < GW as i32 && yy < gh as i32 {
                        sum += luma[(yy * GW as i32 + xx) as usize];
                        count += 1.0;
                    }
                }
                out[(y * GW as i32 + x) as usize] = sum / count;
            }
        }
        luma = out;
    }
    // Brightest wins, nudged toward the centre so a lit edge does not.
    let mut best = (f32::MIN, (0.5, 0.5));
    for y in 0..gh {
        for x in 0..GW {
            let (u, v) = ((x as f32 + 0.5) / GW as f32, (y as f32 + 0.5) / gh as f32);
            let r2 = ((u - 0.5) * 2.0).powi(2) + ((v - 0.5) * 2.0).powi(2);
            let score = luma[(y * GW + x) as usize] * (1.0 - 0.35 * r2.min(2.0) / 2.0);
            if score > best.0 {
                best = (score, (u, v));
            }
        }
    }
    let focal = (best.1.0.clamp(0.08, 0.92), best.1.1.clamp(0.08, 0.92));

    // Strongest hue bin, then the strongest at least 90 degrees away.
    if total <= n as f32 * 0.02 {
        return Analysis { focal, hues: FALLBACK_HUES };
    }
    let a = (0..12).max_by(|&i, &j| bins[i].total_cmp(&bins[j])).unwrap_or(0);
    let b = (0..12)
        .filter(|&i| {
            let d = (i as i32 - a as i32).unsigned_abs().min(12 - (i as i32 - a as i32).unsigned_abs());
            d >= 3 && bins[i] > total * 0.04
        })
        .max_by(|&i, &j| bins[i].total_cmp(&bins[j]));
    let ha = (a as f32 + 0.5) / 12.0;
    let hb = b.map(|b| (b as f32 + 0.5) / 12.0).unwrap_or((ha + 0.5) % 1.0);
    Analysis { focal, hues: [hsv(ha, 0.75, 1.0), hsv(hb, 0.75, 1.0)] }
}

fn hue(r: f32, g: f32, b: f32) -> f32 {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    if d == 0.0 {
        return 0.0;
    }
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0).rem_euclid(1.0)
}

fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let f = |n: f32| {
        let k = (n + h * 6.0) % 6.0;
        v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    [f(5.0), f(3.0), f(1.0)]
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (ex, ey) = (b.0 - a.0, b.1 - a.1);
    let k = (((p.0 - a.0) * ex + (p.1 - a.1) * ey) / (ex * ex + ey * ey).max(1e-6)).clamp(0.0, 1.0);
    (p.0 - a.0 - ex * k).hypot(p.1 - a.1 - ey * k)
}

/// Round to the 2 px dither cell, so moving dithered sprites stay on it.
fn snap2(v: f32) -> f32 {
    (v / 2.0).round() * 2.0
}

/// The smallest even size of at least `v` (and 2).
fn even(v: f32) -> u32 {
    ((v.max(2.0) / 2.0).ceil() as u32) * 2
}

/// BGRA of `light` with its brightest channel at full: a dithered "on" cell.
fn on_color(light: [f32; 3]) -> [u8; 4] {
    let peak = light[0].max(light[1]).max(light[2]).max(1e-6);
    let [r, g, b] = light.map(|c| (c / peak * 255.0).round().clamp(0.0, 255.0) as u8);
    [b, g, r, 255]
}

// ------------------------------------------------------------ the flight
//
// Dither: an aircraft crosses the image toward its brightest point,
// trailing two luminous contrails in the image's own hues, while slow
// brightness waves move through the signal.

/// One loop of the Dither flight, in seconds. Every Dither time term is
/// periodic in it, so the animation repeats without a seam.
const FLIGHT_LOOP: f32 = 24.0;
/// The flight's x at path phase 0 and its travel over one loop, in heroes.
const PATH_X0: f32 = -0.18;
const PATH_DX: f32 = 1.36;
/// Speed swing of the eased flight: fastest off screen, slowest over the
/// middle of the image.
const FLIGHT_EASE: f32 = 0.2;
/// How far back along the path (in loop phase) the contrails reach.
const TRAIL_SPAN: f32 = 0.42;
/// Age slices of the contrail reveal.
const TRAIL_SEGMENTS: usize = 12;
/// Headings the aircraft sprite is pre-rendered at.
const PLANE_HEADINGS: usize = 48;

/// Path phase at loop phase `ph`: an eased glide, `s(ph + 1) = s(ph) + 1`
/// and strictly increasing, so the loop has no seam and never reverses.
fn flight_s(ph: f32) -> f32 {
    ph + FLIGHT_EASE / TAU * (TAU * ph).sin()
}

/// The aircraft's position in hero pixels at path phase `s`: lower left to
/// upper right, off screen at both ends so the wrap is never seen, banking
/// through the image's focal point so it crosses the subject.
fn flight_path(a: &Analysis, w: f32, h: f32, s: f32) -> (f32, f32) {
    let s = s.rem_euclid(1.0);
    let x = PATH_X0 + PATH_DX * s;
    let base = |x: f32| 0.82 - 0.6 * x;
    let over_focal = (a.focal.0 - PATH_X0) / PATH_DX;
    let bend = (a.focal.1 - base(a.focal.0)) * (-((s - over_focal) / 0.22).powi(2)).exp();
    (x * w, (base(x) + bend + 0.03 * (s * TAU).sin()) * h)
}

/// Heading (radians, y down) at path phase `s`.
fn flight_heading(a: &Analysis, w: f32, h: f32, s: f32) -> f32 {
    let p = flight_path(a, w, h, s);
    let ahead = flight_path(a, w, h, s + 0.004);
    (ahead.1 - p.1).atan2(ahead.0 - p.0)
}

/// The two pre-rendered headings around `angle` and the weight of the
/// second, for a crossfade that turns smoothly between them.
fn heading_frames(angle: f32, n: usize) -> (usize, usize, f32) {
    let f = angle.rem_euclid(TAU) / TAU * n as f32;
    let i0 = (f.floor() as usize) % n;
    (i0, (i0 + 1) % n, f - f.floor())
}

/// The hero-x spans (in heroes) of contrail slice `seg` at loop phase `ph`
/// and that slice's age, 0 at the aircraft. A slice straddling the loop
/// wrap is split in two. x is linear in path phase, so a clip in x reveals
/// exactly that stretch of the strip.
fn trail_clips(ph: f32, seg: usize, segments: usize) -> (f32, [(f32, f32); 2]) {
    let (a0, a1) = (seg as f32 / segments as f32, (seg + 1) as f32 / segments as f32);
    let s_hi = flight_s(ph - a0 * TRAIL_SPAN);
    let s_lo = flight_s(ph - a1 * TRAIL_SPAN);
    let x = |s: f32| PATH_X0 + PATH_DX * s;
    let (lo, hi) = (s_lo.rem_euclid(1.0), s_hi.rem_euclid(1.0));
    let age = (a0 + a1) * 0.5;
    if s_lo.floor() == s_hi.floor() {
        (age, [(x(lo), x(hi)), (0.0, 0.0)])
    } else {
        (age, [(x(lo), x(1.0)), (x(0.0), x(hi))])
    }
}

/// Wingtip lights: on for a moment once a second, eased in and out.
fn blink(t: f32) -> f32 {
    let f = t.rem_euclid(1.0);
    smoothstep(0.78, 0.84, f) * (1.0 - smoothstep(0.95, 1.0, f))
}

/// A brightness wave: a soft band of a pre-dithered gain layer, swept
/// across the hero.
struct Wave {
    /// The brighter (true) or darker gain layer.
    lift: bool,
    /// Sweeps along x (true) or y.
    along_x: bool,
    /// Seconds per sweep; divides [`FLIGHT_LOOP`].
    period: f32,
    /// Band width, in heroes.
    width: f32,
    peak: f32,
    reverse: bool,
}

const WAVES: [Wave; 3] = [
    Wave { lift: true, along_x: true, period: 12.0, width: 0.45, peak: 0.9, reverse: false },
    Wave { lift: true, along_x: false, period: 8.0, width: 0.5, peak: 0.7, reverse: true },
    Wave { lift: false, along_x: true, period: 24.0, width: 0.6, peak: 0.8, reverse: true },
];
const WAVE_STRIPS: usize = 6;
/// Gains of the wave layers against the base's 1.0.
const WAVE_LIFT: f32 = 1.16;
const WAVE_DIM: f32 = 0.84;

/// The band centre at `t`, in heroes: it enters fully off one edge and
/// leaves fully off the other, so the wrap is invisible.
fn wave_centre(wave: &Wave, t: f32) -> f32 {
    let c = -wave.width * 0.5 + (1.0 + wave.width) * (t / wave.period).rem_euclid(1.0);
    if wave.reverse { 1.0 - c } else { c }
}

/// Split a band of `width` centred at `centre` into `n` strips, each with
/// the weight of a sine profile: `(start, end, weight)`, in heroes.
fn band_strips(centre: f32, width: f32, n: usize) -> impl Iterator<Item = (f32, f32, f32)> {
    let step = width / n as f32;
    let start = centre - width * 0.5;
    (0..n).map(move |i| {
        let lo = start + step * i as f32;
        (lo, lo + step, (std::f32::consts::PI * (i as f32 + 0.5) / n as f32).sin())
    })
}

/// Light per 2x2 dither cell, RGB.
struct LightCells {
    gw: u32,
    gh: u32,
    rgb: Vec<[f32; 3]>,
}

impl LightCells {
    /// Visit the cells whose centre lies within `pad` of the box `lo..hi`
    /// (hero pixels) with that centre.
    fn each(&mut self, lo: (f32, f32), hi: (f32, f32), pad: f32, mut f: impl FnMut((f32, f32), &mut [f32; 3])) {
        let cell = |v: f32, n: u32| ((v / 2.0).max(0.0) as u32).min(n);
        let (x0, x1) = (cell(lo.0 - pad, self.gw), cell(hi.0 + pad + 2.0, self.gw));
        let (y0, y1) = (cell(lo.1 - pad, self.gh), cell(hi.1 + pad + 2.0, self.gh));
        for cy in y0..y1 {
            for cx in x0..x1 {
                let centre = (cx as f32 * 2.0 + 1.0, cy as f32 * 2.0 + 1.0);
                f(centre, &mut self.rgb[(cy * self.gw + cx) as usize]);
            }
        }
    }

    /// A soft line from `a` to `b`: core of width `w` plus a wide haze.
    fn trail(&mut self, a: (f32, f32), b: (f32, f32), w: f32, amp: f32, hue: [f32; 3]) {
        let lo = (a.0.min(b.0), a.1.min(b.1));
        let hi = (a.0.max(b.0), a.1.max(b.1));
        // Haze of width 2w; the core is its 4th power, so one exp per cell.
        // Beyond 3w the light is under 3% and is skipped.
        let reach = w * 3.0;
        self.each(lo, hi, reach, |p, light| {
            let d = seg_dist(p, a, b);
            if d > reach {
                return;
            }
            let haze = (-(d * d) / (4.0 * w * w)).exp();
            let g = amp * (haze.powi(4) + 0.3 * haze);
            for (l, c) in light.iter_mut().zip(hue) {
                *l = l.max(g * c);
            }
        });
    }
}

/// One 2x2 dither cell's source colour.
struct DitherCell {
    /// RGB, 0..1.
    rgb: [f32; 3],
    /// Brightest channel, 0..1.
    peak: f32,
    /// BGRA with the brightest channel normalised to 255: the "on" colour.
    /// A uniform gain cancels out of it.
    on: [u8; 4],
    alpha: u8,
}

fn dither_cells(src: &Source) -> Vec<DitherCell> {
    let (w, h) = (src.width, src.height);
    let mut out = Vec::with_capacity((w.div_ceil(2) * h.div_ceil(2)) as usize);
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            let [r, g, b, alpha] = src.rgba[(((y + 1).min(h - 1)) * w + (x + 1).min(w - 1)) as usize];
            let peak = r.max(g).max(b);
            let norm = |c: u8| if peak == 0 { 0 } else { (c as f32 * 255.0 / peak as f32).round() as u8 };
            out.push(DitherCell {
                rgb: [r, g, b].map(|c| c as f32 / 255.0),
                peak: peak as f32 / 255.0,
                on: [norm(b), norm(g), norm(r), alpha],
                alpha,
            });
        }
    }
    out
}

/// Paint one colour per 2x2 cell; `None` leaves the cell transparent.
fn cell_image(w: u32, h: u32, mut cell: impl FnMut(u32, u32) -> Option<[u8; 4]>) -> image::RgbaImage {
    let mut out = image::RgbaImage::new(w, h);
    for cy in 0..h.div_ceil(2) {
        for cx in 0..w.div_ceil(2) {
            let Some(px) = cell(cx, cy) else { continue };
            for y in cy * 2..(cy * 2 + 2).min(h) {
                for x in cx * 2..(cx * 2 + 2).min(w) {
                    out.put_pixel(x, y, image::Rgba(px));
                }
            }
        }
    }
    out
}

/// The dithered image at `gain`, keeping only the cells that differ from
/// the base (gain 1): a wave mask that, laid over the base, re-dithers it
/// brighter or darker.
fn dither_shift(src: &Source, cells: &[DitherCell], gain: f32) -> image::RgbaImage {
    let gw = src.width.div_ceil(2);
    cell_image(src.width, src.height, |cx, cy| {
        let cell = &cells[(cy * gw + cx) as usize];
        let threshold = bayer(cx, cy);
        let (was, now) = (cell.peak > threshold, cell.peak * gain > threshold);
        if was == now {
            return None;
        }
        Some(if now {
            cell.on
        } else {
            let [r, g, b] = cell.rgb.map(|v| (v * gain * 0.08 * 255.0).round() as u8);
            [b, g, r, cell.alpha]
        })
    })
}

/// Dither a light field (no base under it): on cells at full brightness in
/// their own colour, off cells transparent.
fn dither_light(w: u32, h: u32, light: impl Fn(u32, u32) -> [f32; 3]) -> image::RgbaImage {
    cell_image(w, h, |cx, cy| {
        let l = light(cx, cy);
        let peak = l[0].max(l[1]).max(l[2]).min(1.0);
        (peak > bayer(cx, cy)).then(|| on_color(l))
    })
}

/// The aircraft at every heading: body along the heading, wings and
/// tailplane across, and a soft glow, dithered. Square, `side` px.
fn plane_sheet(h: u32) -> (Vec<image::RgbaImage>, u32) {
    let hf = h as f32;
    let side = even(0.17 * hf).max(16);
    let c = side as f32 * 0.5;
    let frames = (0..PLANE_HEADINGS)
        .map(|i| {
            let angle = i as f32 / PLANE_HEADINGS as f32 * TAU;
            let (ty, tx) = angle.sin_cos();
            let (nx, ny) = (-ty, tx);
            let at = |along: f32, across: f32| ((tx * along + nx * across) * hf, (ty * along + ny * across) * hf);
            let parts = [
                (at(-0.026, 0.0), at(0.02, 0.0)),
                (at(-0.003, -0.032), at(-0.003, 0.032)),
                (at(-0.026, -0.011), at(-0.026, 0.011)),
            ];
            dither_light(side, side, |cx, cy| {
                let q = (cx as f32 * 2.0 + 1.0 - c, cy as f32 * 2.0 + 1.0 - c);
                let d = parts.iter().map(|(a, b)| seg_dist(q, *a, *b)).fold(f32::MAX, f32::min);
                let r2 = (q.0 * q.0 + q.1 * q.1) / (hf * hf);
                let body = smoothstep(0.005 * hf, 0.002 * hf, d) + 0.4 * (-r2 / 0.002).exp();
                [0.9 * body, 0.92 * body, body]
            })
        })
        .collect();
    (frames, side)
}

/// Port (red) and starboard (green) wingtip lights, dithered. Square, `side` px.
fn light_sheet(h: u32) -> (Vec<image::RgbaImage>, u32) {
    let hf = h as f32;
    let side = even(0.04 * hf).max(8);
    let c = side as f32 * 0.5;
    let frames = [[1.0, 0.2, 0.15], [0.2, 1.0, 0.4]]
        .map(|tint: [f32; 3]| {
            dither_light(side, side, |cx, cy| {
                let q = (cx as f32 * 2.0 + 1.0 - c, cy as f32 * 2.0 + 1.0 - c);
                let tip = (-(q.0 * q.0 + q.1 * q.1) / (hf * hf) / 0.00004).exp();
                tint.map(|v| v * tip)
            })
        })
        .into();
    (frames, side)
}

/// The twin contrails along the whole flight path at one `age` (tight and
/// bright when fresh, spread and soft when old), dithered on the base's
/// Bayer grid. Cropped to the rows they touch: `(image, top row)`.
fn trail_strip(a: &Analysis, w: u32, h: u32, age: f32, amp: f32) -> (image::RgbaImage, u32) {
    let (wf, hf) = (w as f32, h as f32);
    let (gw, gh) = (w.div_ceil(2), h.div_ceil(2));
    let mut cells = LightCells { gw, gh, rgb: vec![[0.0; 3]; (gw * gh) as usize] };
    // Path phases that cross the hero (x in 0..w), with a margin.
    let (s0, s1) = ((-PATH_X0 - 0.03) / PATH_DX, (1.0 - PATH_X0 + 0.03) / PATH_DX);
    let samples = 240;
    let spread = (0.009 + 0.04 * age) * hf;
    let width = (0.006 + 0.02 * age) * hf;
    let mut prev: Option<((f32, f32), (f32, f32))> = None;
    for i in 0..=samples {
        let s = s0 + (s1 - s0) * i as f32 / samples as f32;
        let c0 = flight_path(a, wf, hf, s);
        let c1 = flight_path(a, wf, hf, s + 0.004);
        let len = (c1.0 - c0.0).hypot(c1.1 - c0.1).max(1e-3);
        let (nx, ny) = (-(c1.1 - c0.1) / len, (c1.0 - c0.0) / len);
        let (pa, pb) = ((c0.0 + nx * spread, c0.1 + ny * spread), (c0.0 - nx * spread, c0.1 - ny * spread));
        if let Some((qa, qb)) = prev {
            cells.trail(qa, pa, width, amp, a.hues[0]);
            cells.trail(qb, pb, width, amp, a.hues[1]);
        }
        prev = Some((pa, pb));
    }
    let lit = |cy: u32| (0..gw).any(|cx| cells.rgb[(cy * gw + cx) as usize].iter().any(|&v| v > 1.0 / 32.0));
    let first = (0..gh).find(|&cy| lit(cy)).unwrap_or(0);
    let last = (0..gh).rev().find(|&cy| lit(cy)).unwrap_or(0);
    // A multiple of 8 rows keeps the strip on the base's Bayer phase.
    let top = (first * 2) / 8 * 8;
    let bottom = ((last + 1) * 2).clamp(top + 2, h.max(top + 2));
    let strip = dither_light(w, bottom - top, |cx, cy| {
        let cy = cy + top / 2;
        if cy >= gh { [0.0; 3] } else { cells.rgb[(cy * gw + cx) as usize] }
    });
    (strip, top)
}

// ------------------------------------------------------------ the ripple

/// Rings in flight at once, and the seconds each lives.
const RINGS: usize = 3;
const RING_PERIOD: f32 = 6.0;
const RING_PEAK: f32 = 0.55;

/// A ring of halftone dots in the image's lead hue (ink on light paper),
/// dot size following a soft band profile. Square, `side` px; the ring sits
/// at `0.36 * side`.
fn ring_sprite(reach: f32, hue: [f32; 3], light: bool) -> (image::RgbaImage, u32) {
    let side = even(reach * 0.7).clamp(32, 1024);
    let s = side as f32;
    let (radius, sigma) = (0.36 * s, 0.045 * s);
    let ink = if light { hue.map(|c| c * 0.42) } else { hue };
    let [r, g, b] = ink.map(|c| (c * 255.0).round().clamp(0.0, 255.0) as u8);
    let mut out = image::RgbaImage::new(side, side);
    for y in (0..side).step_by(4) {
        for x in (0..side).step_by(4) {
            let d0 = (x as f32 + 1.5 - s * 0.5).hypot(y as f32 + 1.5 - s * 0.5);
            let band = (-((d0 - radius) / sigma).powi(2)).exp();
            if band < 0.03 {
                continue;
            }
            let dot = 1.9 * band;
            for dy in 0..4.min(side - y) {
                for dx in 0..4.min(side - x) {
                    let d = (dx as f32 - 1.5).hypot(dy as f32 - 1.5);
                    let cov = (dot + 0.5 - d).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        out.put_pixel(x + dx, y + dy, image::Rgba([b, g, r, (cov * 255.0).round() as u8]));
                    }
                }
            }
        }
    }
    (out, side)
}

/// Ring radius (in units of `reach`, the focal point's farthest corner) and
/// opacity at life fraction `u`: it decelerates outward, fading in fast and
/// out slowly, invisible at both ends of its life.
fn ring_state(u: f32) -> (f32, f32) {
    let u = u.clamp(0.0, 1.0);
    let radius = 0.03 + 1.05 * motion::EASE_OUT.eval(u);
    (radius, RING_PEAK * smoothstep(0.0, 0.12, u) * (1.0 - u).powi(2))
}

// ------------------------------------------------------------ the CRT

const BAND_PEAK: f32 = 0.14;
const VIGNETTE_DARK: f32 = 0.42;
const VIGNETTE_LIGHT: f32 = 0.2;

/// Which third row is lined at `t`: the roll steps 8 rows a second, as the
/// rasterized roll did.
fn scan_roll(t: f32) -> u32 {
    ((t.max(0.0) * 8.0) as u32) % 3
}

/// The sweep band's centre at `t`, in heroes: once down every 6 s.
fn band_position(t: f32) -> f32 {
    (t / 6.0).rem_euclid(1.0) * 1.3 - 0.15
}

/// Every third row, as a 4 px wide strip stretched across the hero: black
/// (or white on light paper) at the scanline strength.
fn lines_strip(h: u32, light: bool) -> image::RgbaImage {
    let v = if light { 255 } else { 0 };
    let alpha = (SCANLINE_ALPHA * 255.0).round() as u8;
    image::RgbaImage::from_fn(4, h + 3, |_, y| image::Rgba([v, v, v, if y % 3 == 0 { alpha } else { 0 }]))
}

/// The soft sweep band, a vertical gaussian, stretched across the hero.
fn band_sprite() -> image::RgbaImage {
    image::RgbaImage::from_fn(4, 64, |_, y| {
        let v = (y as f32 + 0.5) / 64.0 - 0.5;
        image::Rgba([255, 255, 255, ((-(v / 0.16).powi(2)).exp() * 255.0).round() as u8])
    })
}

/// Darkened corners, stretched over the hero.
fn vignette(light: bool) -> image::RgbaImage {
    let strength = if light { VIGNETTE_LIGHT } else { VIGNETTE_DARK };
    image::RgbaImage::from_fn(64, 64, |x, y| {
        let (u, v) = ((x as f32 + 0.5) / 32.0 - 1.0, (y as f32 + 0.5) / 32.0 - 1.0);
        let r = u.hypot(v) / std::f32::consts::SQRT_2;
        image::Rgba([0, 0, 0, (strength * smoothstep(0.35, 1.0, r).powi(2) * 255.0).round() as u8])
    })
}

// ------------------------------------------------------------ layers

/// Everything an animated hero is drawn from, built once per image, effect
/// and size on the background executor. Frames only place these.
struct Layers {
    effect: Effect,
    width: u32,
    height: u32,
    analysis: Analysis,
    /// The still (Scanlines: the image without lines, which roll on top).
    base: Arc<RenderImage>,
    kind: Kind,
}

enum Kind {
    Original,
    Dither {
        lift: Arc<RenderImage>,
        dim: Arc<RenderImage>,
        /// Fresh and dissipated contrails, and the row the strips start at.
        trails: [Arc<RenderImage>; 2],
        trail_top: [u32; 2],
        /// One frame per heading.
        plane: Arc<RenderImage>,
        plane_side: u32,
        /// Frame 0 port, frame 1 starboard.
        lights: Arc<RenderImage>,
        light_side: u32,
    },
    Halftone {
        ring: Arc<RenderImage>,
        ring_side: u32,
        /// Distance from the focal point to the farthest corner, in px.
        reach: f32,
    },
    Scanlines {
        lines: Arc<RenderImage>,
        band: Arc<RenderImage>,
        vignette: Arc<RenderImage>,
    },
}

impl Layers {
    fn build(src: &Source, effect: Effect, light: bool) -> Self {
        let analysis = analyze(src);
        let (w, h) = (src.width, src.height);
        let base = raster(if effect == Effect::Scanlines { plain(src) } else { render(src, effect, light) });
        let kind = match effect {
            Effect::None => Kind::Original,
            Effect::Dither => {
                let cells = dither_cells(src);
                let (fresh, fresh_top) = trail_strip(&analysis, w, h, 0.12, 1.0);
                let (old, old_top) = trail_strip(&analysis, w, h, 0.7, 0.8);
                let (plane, plane_side) = plane_sheet(h);
                let (lights, light_side) = light_sheet(h);
                Kind::Dither {
                    lift: raster(dither_shift(src, &cells, WAVE_LIFT)),
                    dim: raster(dither_shift(src, &cells, WAVE_DIM)),
                    trails: [raster(fresh), raster(old)],
                    trail_top: [fresh_top, old_top],
                    plane: raster_frames(plane),
                    plane_side,
                    lights: raster_frames(lights),
                    light_side,
                }
            }
            Effect::Halftone => {
                let (fx, fy) = (analysis.focal.0 * w as f32, analysis.focal.1 * h as f32);
                let reach = [(0.0, 0.0), (w as f32, 0.0), (0.0, h as f32), (w as f32, h as f32)]
                    .into_iter()
                    .map(|(x, y)| (x - fx).hypot(y - fy))
                    .fold(1.0, f32::max);
                let (ring, ring_side) = ring_sprite(reach, analysis.hues[0], light);
                Kind::Halftone { ring: raster(ring), ring_side, reach }
            }
            Effect::Scanlines => Kind::Scanlines {
                lines: raster(lines_strip(h, light)),
                band: raster(band_sprite()),
                vignette: raster(vignette(light)),
            },
        };
        Layers { effect, width: w, height: h, analysis, base, kind }
    }

    /// Every texture, for freeing them from the atlas.
    fn images(&self) -> Vec<Arc<RenderImage>> {
        let mut out = vec![self.base.clone()];
        match &self.kind {
            Kind::Original => {}
            Kind::Dither { lift, dim, trails, plane, lights, .. } => {
                out.extend([lift, dim, &trails[0], &trails[1], plane, lights].map(Arc::clone))
            }
            Kind::Halftone { ring, .. } => out.push(ring.clone()),
            Kind::Scanlines { lines, band, vignette } => out.extend([lines, band, vignette].map(Arc::clone)),
        }
        out
    }
}

// ------------------------------------------------------------ composition

/// One texture placed in hero pixels (the base raster's space).
#[derive(Clone)]
struct Sprite {
    image: Arc<RenderImage>,
    frame: usize,
    /// x, y, width, height.
    rect: [f32; 4],
    /// Optional clip, x0, y0, x1, y1.
    clip: Option<[f32; 4]>,
}

impl Sprite {
    fn new(image: &Arc<RenderImage>, frame: usize, rect: [f32; 4]) -> Self {
        Sprite { image: image.clone(), frame, rect, clip: None }
    }

    fn clipped(mut self, clip: [f32; 4]) -> Self {
        self.clip = Some(clip);
        self
    }
}

/// Sprites that share one opacity.
struct Group {
    opacity: f32,
    sprites: Vec<Sprite>,
}

/// What the hero draws this frame.
struct Composition {
    motion: Motion,
    /// The base raster's size, which the sprites are placed in.
    size: (f32, f32),
    groups: Vec<Group>,
}

impl Composition {
    fn still(image: Arc<RenderImage>) -> Self {
        let s = image.size(0);
        let (w, h) = ((s.width.0 as f32).max(1.0), (s.height.0 as f32).max(1.0));
        Composition {
            motion: Motion::STILL,
            size: (w, h),
            groups: vec![Group { opacity: 1.0, sprites: vec![Sprite::new(&image, 0, [0.0, 0.0, w, h])] }],
        }
    }

    fn push(&mut self, opacity: f32, sprites: impl IntoIterator<Item = Sprite>) {
        let opacity = opacity.clamp(0.0, 1.0);
        if opacity < 0.004 {
            return;
        }
        let sprites: Vec<Sprite> = sprites.into_iter().collect();
        if !sprites.is_empty() {
            self.groups.push(Group { opacity, sprites });
        }
    }
}

/// Place the layers for clock time `t`. `intro` (0..1) fades the moving
/// overlays in when the animation starts. Pure: no rasterizing, only
/// rects, clips and opacities over textures that already exist.
fn compose(layers: &Layers, t: f32, intro: f32) -> Composition {
    let (w, h) = (layers.width as f32, layers.height as f32);
    let a = &layers.analysis;
    let mut c = Composition { motion: motion(layers.effect, a, t), size: (w, h), groups: Vec::new() };
    c.push(1.0, [Sprite::new(&layers.base, 0, [0.0, 0.0, w, h])]);
    match &layers.kind {
        Kind::Original => {}
        Kind::Dither { lift, dim, trails, trail_top, plane, plane_side, lights, light_side } => {
            let ph = (t / FLIGHT_LOOP).rem_euclid(1.0);
            // Waves: strips of a re-dithered gain layer, swept by offset.
            for wave in &WAVES {
                let image = if wave.lift { lift } else { dim };
                for (lo, hi, weight) in band_strips(wave_centre(wave, t), wave.width, WAVE_STRIPS) {
                    let clip = if wave.along_x {
                        [snap2(lo * w), 0.0, snap2(hi * w), h]
                    } else {
                        [0.0, snap2(lo * h), w, snap2(hi * h)]
                    };
                    c.push(wave.peak * weight * intro, [Sprite::new(image, 0, [0.0, 0.0, w, h]).clipped(clip)]);
                }
            }
            // Contrails: the strips revealed slice by slice behind the
            // aircraft, fading with age as the fresh trail dissipates.
            for seg in 0..TRAIL_SEGMENTS {
                let (age, spans) = trail_clips(ph, seg, TRAIL_SEGMENTS);
                let fade = (1.0 - age).powi(2) * (0.8 + 0.2 * (age * 48.0 - ph * TAU * 12.0).sin());
                let fresh = 1.0 - smoothstep(0.15, 0.75, age);
                for (v, weight) in [(0, fresh), (1, 1.0 - fresh)] {
                    let (image, top) = (&trails[v], trail_top[v] as f32);
                    let rows = trails[v].size(0).height.0 as f32;
                    let sprites = spans
                        .iter()
                        .filter(|(x0, x1)| x1 - x0 > 1e-4)
                        .map(|&(x0, x1)| Sprite::new(image, 0, [0.0, top, w, rows]).clipped([x0 * w, 0.0, x1 * w, h]));
                    c.push(fade * weight * intro, sprites);
                }
            }
            // The aircraft: translate along the eased path, turn by
            // crossfading the two nearest pre-rendered headings.
            let s = flight_s(ph);
            let p = flight_path(a, w, h, s);
            let heading = flight_heading(a, w, h, s);
            let (i0, i1, mix) = heading_frames(heading, PLANE_HEADINGS);
            let side = *plane_side as f32;
            let rect = [snap2(p.0 - side * 0.5), snap2(p.1 - side * 0.5), side, side];
            c.push((1.0 - mix) * intro, [Sprite::new(plane, i0, rect)]);
            c.push(mix * intro, [Sprite::new(plane, i1, rect)]);
            // Wingtip lights, blinking once a second.
            let (ty, tx) = heading.sin_cos();
            let at =
                |along: f32, across: f32| (p.0 + (tx * along - ty * across) * h, p.1 + (ty * along + tx * across) * h);
            let side = *light_side as f32;
            let light = |frame: usize, q: (f32, f32)| {
                Sprite::new(lights, frame, [snap2(q.0 - side * 0.5), snap2(q.1 - side * 0.5), side, side])
            };
            c.push(blink(t) * intro, [light(0, at(-0.003, 0.032)), light(1, at(-0.003, -0.032))]);
        }
        Kind::Halftone { ring, ring_side, reach } => {
            let (fx, fy) = (a.focal.0 * w, a.focal.1 * h);
            let ring_radius = 0.36 * *ring_side as f32;
            for i in 0..RINGS {
                let u = (t / RING_PERIOD + i as f32 / RINGS as f32).rem_euclid(1.0);
                let (radius, opacity) = ring_state(u);
                let side = *ring_side as f32 * radius * reach / ring_radius;
                c.push(opacity * intro, [Sprite::new(ring, 0, [fx - side * 0.5, fy - side * 0.5, side, side])]);
            }
        }
        Kind::Scanlines { lines, band, vignette } => {
            let full = [0.0, 0.0, w, h];
            // Rows y with (y + roll) % 3 == 0 are lined.
            let offset = ((3 - scan_roll(t)) % 3) as f32;
            c.push(1.0, [Sprite::new(lines, 0, [0.0, offset - 3.0, w, h + 3.0]).clipped(full)]);
            let y = band_position(t) * h;
            c.push(BAND_PEAK * intro, [Sprite::new(band, 0, [0.0, y - 0.15 * h, w, 0.3 * h]).clipped(full)]);
            c.push(intro, [Sprite::new(vignette, 0, full)]);
        }
    }
    c
}

/// How the whole painted hero moves this frame: zoom and opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    pub zoom: f32,
    /// Zoom origin, normalized to the hero.
    pub center: (f32, f32),
    pub opacity: f32,
}

impl Motion {
    pub const STILL: Motion = Motion { zoom: 1.0, center: (0.5, 0.5), opacity: 1.0 };
}

/// Ken Burns period for Original, in seconds.
const PUSH_PERIOD: f32 = 20.0;

/// Paint-transform motion at `t`: Ken Burns for Original, flicker for
/// Scanlines.
fn motion(effect: Effect, a: &Analysis, t: f32) -> Motion {
    match effect {
        Effect::None => {
            // Out and back along an eased drift toward the focal point.
            let tri = 1.0 - (2.0 * (t / PUSH_PERIOD).rem_euclid(1.0) - 1.0).abs();
            let k = motion::EASE_IN_OUT.eval(tri);
            Motion {
                zoom: 1.0 + 0.06 * k,
                center: (0.5 + (a.focal.0 - 0.5) * k, 0.5 + (a.focal.1 - 0.5) * k),
                opacity: 1.0,
            }
        }
        Effect::Scanlines => {
            Motion { opacity: 1.0 - 0.015 * 0.5 * (1.0 + (t * 31.0).sin() * (t * 7.3).sin()), ..Motion::STILL }
        }
        Effect::Dither | Effect::Halftone => Motion::STILL,
    }
}

// ------------------------------------------------------------ the loop

/// Longest step one frame may advance the clock, so a stall or a resume
/// never jumps the scene.
const MAX_STEP: f32 = 0.1;
/// The clock wraps at a common multiple of every loop period (24 s flight,
/// 20 s push, 6 s ripple and band, 3/8 s roll), so long sessions keep
/// full float precision without a seam.
const CLOCK_WRAP: f32 = 120.0;

/// Animation time that only advances while the loop runs, so pausing
/// (background, reduced motion) and resuming continue where it left off.
#[derive(Debug, Default, Clone, Copy)]
struct Clock {
    /// Scene time, wrapped.
    t: f32,
    /// Unwrapped running time, for the intro.
    elapsed: f32,
    last: Option<Instant>,
}

impl Clock {
    /// Advance to `now` if running (by real time over `scale`), else park.
    fn advance(&mut self, now: Instant, running: bool, scale: f32) -> f32 {
        if running {
            if let Some(last) = self.last {
                let dt = now.saturating_duration_since(last).as_secs_f32().min(MAX_STEP) / scale.max(0.01);
                self.t = (self.t + dt).rem_euclid(CLOCK_WRAP);
                self.elapsed += dt;
            }
            self.last = Some(now);
        } else {
            self.last = None;
        }
        self.t
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Key {
    path: PathBuf,
    effect: Effect,
    light: bool,
    target: (u32, u32),
}

struct Anim {
    /// `None` once home was left: only `still` is kept.
    key: Option<Key>,
    generation: u64,
    layers: Option<Arc<Layers>>,
    /// Shown until `layers` land: the previous image, or the one still
    /// kept after leaving home.
    still: Option<Arc<RenderImage>>,
    clock: Clock,
    /// Clock `elapsed` when the layers were first drawn.
    shown_at: Option<f32>,
}

static ANIM: OnceLock<Mutex<Option<Anim>>> = OnceLock::new();
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Make the animation state match `key`, building its layers on the
/// background executor. True when there is something to show.
fn sync_animation(key: Key, cx: &mut App) -> bool {
    let slot = ANIM.get_or_init(|| Mutex::new(None));
    let Ok(mut guard) = slot.lock() else { return false };
    if guard.as_ref().is_none_or(|a| a.key.as_ref() != Some(&key)) {
        // Keep the old picture on screen until the new layers are ready.
        let mut gone = Vec::new();
        let still = guard.take().and_then(|old| {
            let still = old.layers.as_ref().map(|l| l.base.clone()).or_else(|| old.still.clone());
            gone.extend(old.layers.iter().flat_map(|l| l.images()));
            gone.extend(old.still);
            gone.retain(|i| still.as_ref().is_none_or(|s| !Arc::ptr_eq(i, s)));
            still
        });
        retire(gone);
        let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let job = key.clone();
        let work = cx.background_executor().spawn(async move {
            let src = load_source(&job.path, job.target)?;
            Some(Arc::new(Layers::build(&src, job.effect, job.light)))
        });
        cx.spawn(async move |cx| {
            let layers = work.await;
            if let Some(mut guard) = ANIM.get().and_then(|s| s.lock().ok())
                && let Some(a) = guard.as_mut()
                && a.generation == generation
                && layers.is_some()
            {
                retire(a.still.take());
                a.layers = layers;
            }
            cx.update(|cx| cx.refresh_windows());
        })
        .detach();
        *guard =
            Some(Anim { key: Some(key), generation, layers: None, still, clock: Clock::default(), shown_at: None });
    }
    guard.as_ref().is_some_and(|a| a.layers.is_some() || a.still.is_some())
}

/// This frame of the animation, and whether the loop should keep going.
fn animation_frame(cx: &App) -> (Option<Composition>, bool) {
    let Some(mut guard) = ANIM.get().and_then(|s| s.lock().ok()) else { return (None, false) };
    let Some(a) = guard.as_mut() else { return (None, false) };
    let Some(layers) = a.layers.clone() else { return (a.still.clone().map(Composition::still), false) };
    // Motion only while decorative motion is allowed: not reduced, and the
    // window active (so not minimized or in the background).
    let running = motion::decorative_motion(cx);
    let t = a.clock.advance(Instant::now(), running, motion::speed_scale());
    let elapsed = a.clock.elapsed;
    // First drawn while paused: no intro, it is simply there.
    let shown = *a.shown_at.get_or_insert(if running { elapsed } else { f32::NEG_INFINITY });
    let intro_len = motion::FADE_IN.total().as_secs_f32() / motion::speed_scale();
    let intro = motion::FADE_IN.progress((elapsed - shown) / intro_len.max(1e-3));
    (Some(compose(&layers, t, intro)), running)
}

/// Stop animating and free its rasters, e.g. when the switch is turned off.
pub fn release_animation(window: &mut Window) {
    drop_retired(window);
    let Some(slot) = ANIM.get() else { return };
    let Some(old) = slot.lock().ok().and_then(|mut g| g.take()) else { return };
    let images = old.layers.iter().flat_map(|l| l.images()).chain(old.still).collect::<Vec<_>>();
    for image in images {
        let _ = window.drop_image(image);
    }
}

/// Home is no longer drawn: free the animation's layers and the cached
/// still rasters, keeping one still so coming back shows the art at once.
fn leave_home(window: &mut Window) {
    let mut gone = RETIRED.lock().map(|mut r| std::mem::take(&mut *r)).unwrap_or_default();
    let mut kept = None;
    if let Some(slot) = ANIM.get()
        && let Ok(mut guard) = slot.lock()
        && let Some(old) = guard.take()
    {
        kept = old.layers.as_ref().map(|l| l.base.clone()).or_else(|| old.still.clone());
        gone.extend(old.layers.iter().flat_map(|l| l.images()));
        gone.extend(old.still);
        if let Some(still) = &kept {
            gone.retain(|i| !Arc::ptr_eq(i, still));
            *guard = Some(Anim {
                key: None,
                generation: 0,
                layers: None,
                still: Some(still.clone()),
                clock: Clock::default(),
                shown_at: None,
            });
        }
    }
    if let Some(cache) = CACHE.get()
        && let Ok(mut entries) = cache.lock()
    {
        // The animation's still is the one kept; otherwise the most recent.
        let keep = if kept.is_some() { None } else { entries.pop() };
        gone.extend(entries.drain(..).flat_map(Entry::images));
        if let Some(mut entry) = keep {
            gone.extend(entry.fallback.take());
            entries.push(entry);
        }
    }
    for image in gone {
        let _ = window.drop_image(image);
    }
}

// ------------------------------------------------------------ the view

/// What home wants the hero to show.
pub struct HeroProps {
    pub path: PathBuf,
    pub effect: Effect,
    /// Light appearance (Halftone and Scanlines print on light paper).
    pub light: bool,
    /// The hero's size in device pixels.
    pub target: (f32, f32),
    pub animate: bool,
    pub frosted: bool,
    /// Home fades the art into the canvas. A thread keeps it full-bleed
    /// under the glass, so the color does not disappear down the page.
    pub fade: bool,
    /// The composer's bounds, cut out of the art.
    pub composer: Rc<Cell<Option<Bounds<Pixels>>>>,
}

/// The hero as its own small view: an animation frame notifies only this
/// view, never refreshes every window. Home holds it in element state, so
/// it is released when home stops drawing it, which frees the layers.
pub struct Hero {
    props: Option<HeroProps>,
}

impl Hero {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.on_release_in(window, |_, window, _| leave_home(window)).detach();
        Hero { props: None }
    }

    /// Take this frame's props; true when there is art to show (otherwise
    /// it is still decoding, and the window refreshes when it lands).
    pub fn show(&mut self, props: HeroProps, cx: &mut App) -> bool {
        let ready = if props.animate {
            let key = Key {
                path: props.path.clone(),
                effect: props.effect,
                light: paper_light(props.effect, props.light),
                target: step_target(props.target),
            };
            sync_animation(key, cx)
        } else {
            prepare(&props.path, props.effect, props.light, props.target, cx).is_some()
        };
        self.props = Some(props);
        ready
    }
}

impl Render for Hero {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        drop_retired(window);
        let Some(p) = self.props.as_ref() else { return div() };
        let (composition, running) = if p.animate {
            animation_frame(cx)
        } else {
            (prepare(&p.path, p.effect, p.light, p.target, cx).map(Composition::still), false)
        };
        let Some(composition) = composition else { return div() };
        // Paced by the display; notifies this view only.
        if running {
            window.request_animation_frame();
        }
        let opacity = composition.motion.opacity * if p.frosted { FROSTED_OPACITY } else { 1.0 };
        let composition = Rc::new(composition);
        let fade = p.fade;
        let layers = [false, true].map(|cutout| {
            let groups = (0..composition.groups.len()).map(|i| {
                let (composition, composer) = (composition.clone(), p.composer.clone());
                div().absolute().inset_0().opacity(composition.groups[i].opacity).child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            paint_group(&composition, i, bounds, composer.get(), cutout, fade, window)
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
            });
            div()
                .absolute()
                .inset_0()
                .opacity(if cutout { 1.0 } else { CUTOUT_REVEAL_OPACITY })
                .children(groups.collect::<Vec<_>>())
        });
        div().size_full().opacity(opacity).children(layers)
    }
}

// ------------------------------------------------------------ paint

/// Where hero pixel (0, 0) lands in the window, and window px per hero px:
/// the base cover-fitted into the bounds, zoomed by `motion` about its
/// centre. Zooming about a point inside the hero keeps it covered.
fn placement(bounds: Bounds<Pixels>, src: (f32, f32), motion: Motion) -> Option<(f32, f32, f32)> {
    let (sw, sh) = (src.0.max(1.0), src.1.max(1.0));
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let scale = (w / sw).max(h / sh);
    let (fw, fh) = (sw * scale, sh * scale);
    let (zx, zy) = (w * motion.center.0, h * motion.center.1);
    let zoom = motion.zoom.max(1.0);
    let (left, top) = (zx + ((w - fw) * 0.5 - zx) * zoom, zy + ((h - fh) * 0.5 - zy) * zoom);
    Some((f32::from(bounds.left()) + left, f32::from(bounds.top()) + top, scale * zoom))
}

/// The bottom fade, and with `cutout` the composer's feathered rounded rect.
fn alpha_mask(bounds: Bounds<Pixels>, composer: Option<Bounds<Pixels>>, cutout: bool, fade: bool) -> ImageAlphaMask {
    let h = f32::from(bounds.size.height);
    let bottom_fade = fade.then(|| (bounds.bottom(), px(h.max(1.0))));
    match (cutout, composer) {
        (true, Some(c)) => ImageAlphaMask {
            bounds: Bounds::new(c.origin, size(c.size.width, c.bottom().max(bounds.bottom()) - c.top())),
            radius: px(crate::design::radius::COMPOSER),
            feather: px((h * 0.52).clamp(120.0, 280.0)),
            clearance: px(8.0),
            bottom_fade,
        },
        // No exclusion: park the rect below the image so only the fade applies.
        _ => ImageAlphaMask {
            bounds: Bounds::new(point(bounds.left(), bounds.bottom() + px(1.0)), bounds.size),
            radius: px(0.0),
            feather: px(1.0),
            clearance: px(0.0),
            bottom_fade,
        },
    }
}

/// Paint group `index` into the hero `bounds`. Each sprite reuses its atlas
/// tile; only its rect, crop and opacity change between frames.
fn paint_group(
    composition: &Composition,
    index: usize,
    bounds: Bounds<Pixels>,
    composer: Option<Bounds<Pixels>>,
    cutout: bool,
    fade: bool,
    window: &mut Window,
) {
    let Some(group) = composition.groups.get(index) else { return };
    let Some((ox, oy, k)) = placement(bounds, composition.size, composition.motion) else { return };
    let mask = alpha_mask(bounds, composer, cutout, fade);
    let map = |x: f32, y: f32| (ox + x * k, oy + y * k);
    let (bx0, by0) = (f32::from(bounds.left()), f32::from(bounds.top()));
    let (bx1, by1) = (f32::from(bounds.right()), f32::from(bounds.bottom()));
    for sprite in &group.sprites {
        let [x, y, w, h] = sprite.rect;
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        let (fx0, fy0) = map(x, y);
        let (fx1, fy1) = map(x + w, y + h);
        let (mut vx0, mut vy0, mut vx1, mut vy1) = (fx0.max(bx0), fy0.max(by0), fx1.min(bx1), fy1.min(by1));
        if let Some([cx0, cy0, cx1, cy1]) = sprite.clip {
            let ((cx0, cy0), (cx1, cy1)) = (map(cx0, cy0), map(cx1, cy1));
            (vx0, vy0, vx1, vy1) = (vx0.max(cx0), vy0.max(cy0), vx1.min(cx1), vy1.min(cy1));
        }
        if vx1 - vx0 < 0.5 || vy1 - vy0 < 0.5 {
            continue;
        }
        let rect =
            |x0: f32, y0: f32, x1: f32, y1: f32| Bounds::new(point(px(x0), px(y0)), size(px(x1 - x0), px(y1 - y0)));
        let _ = window.paint_image_fitted_masked(
            rect(vx0, vy0, vx1, vy1),
            rect(fx0, fy0, fx1, fy1),
            Corners::default(),
            sprite.image.clone(),
            sprite.frame,
            false,
            Some(mask),
        );
    }
}

pub fn hero_height(viewport_height: f32) -> f32 {
    (viewport_height.max(0.0) * VIEWPORT_RATIO).min(MAX_HEIGHT)
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

    /// Dark image with a bright warm patch in the top-right quadrant.
    fn lit_corner(w: u32, h: u32) -> Source {
        let rgba: Box<[[u8; 4]]> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                if x > w * 3 / 4 && y < h / 4 { [255, 200, 40, 255] } else { [10, 20, 60, 255] }
            })
            .collect();
        let luma =
            rgba.iter().map(|p| ((p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000) as u8).collect();
        Source { width: w, height: h, rgba, luma }
    }

    #[test]
    fn focal_point_finds_the_bright_region() {
        let a = analyze(&lit_corner(320, 180));
        assert!(a.focal.0 > 0.6 && a.focal.1 < 0.4, "{a:?}");
    }

    #[test]
    fn grey_art_falls_back_to_default_hues() {
        let grey = Source {
            width: 64,
            height: 36,
            rgba: vec![[128, 128, 128, 255]; 64 * 36].into_boxed_slice(),
            luma: vec![128; 64 * 36].into_boxed_slice(),
        };
        assert_eq!(analyze(&grey).hues, FALLBACK_HUES);
    }

    #[test]
    fn colorful_art_picks_its_own_hues() {
        let a = analyze(&lit_corner(320, 180));
        assert_ne!(a.hues, FALLBACK_HUES);
    }

    #[test]
    fn flight_path_crosses_the_focal_point_and_starts_off_screen() {
        let a = Analysis { focal: (0.6, 0.35), hues: FALLBACK_HUES };
        let (x0, _) = flight_path(&a, 1000.0, 500.0, 0.0);
        let (x1, _) = flight_path(&a, 1000.0, 500.0, 0.999);
        assert!(x0 < 0.0 && x1 > 1000.0);
        let over = (0.6 - PATH_X0) / PATH_DX;
        let (x, y) = flight_path(&a, 1000.0, 500.0, over);
        assert!((x - 600.0).abs() < 1.0 && (y / 500.0 - 0.35).abs() < 0.04, "{x}, {y}");
    }

    #[test]
    fn eased_flight_never_reverses_and_loops() {
        let mut prev = flight_s(0.0);
        for i in 1..=1000 {
            let s = flight_s(i as f32 / 1000.0);
            assert!(s > prev, "reversed at {i}");
            prev = s;
        }
        assert!((flight_s(1.0) - 1.0).abs() < 1e-5);
        assert!((flight_s(0.3) + 1.0 - flight_s(1.3)).abs() < 1e-4);
        // Slower over the middle of the image than off screen.
        let speed = |ph: f32| flight_s(ph + 0.001) - flight_s(ph);
        assert!(speed(0.5) < speed(0.0));
    }

    #[test]
    fn heading_frames_crossfade_between_neighbours() {
        assert_eq!(heading_frames(0.0, 48), (0, 1, 0.0));
        let (i0, i1, mix) = heading_frames(TAU / 48.0 * 2.5, 48);
        assert_eq!((i0, i1), (2, 3));
        assert!((mix - 0.5).abs() < 1e-3);
        // Negative angles wrap; the last heading blends into the first.
        let (i0, i1, _) = heading_frames(-0.01, 48);
        assert_eq!((i0, i1), (47, 0));
    }

    #[test]
    fn band_strips_are_contiguous_with_a_soft_profile() {
        let strips: Vec<_> = band_strips(0.5, 0.4, 6).collect();
        assert_eq!(strips.len(), 6);
        assert!((strips[0].0 - 0.3).abs() < 1e-5 && (strips[5].1 - 0.7).abs() < 1e-5);
        for pair in strips.windows(2) {
            assert!((pair[0].1 - pair[1].0).abs() < 1e-5);
        }
        assert!(strips[0].2 < strips[2].2 && (strips[2].2 - strips[3].2).abs() < 1e-5);
        assert!(strips.iter().all(|s| s.2 > 0.0 && s.2 <= 1.0));
    }

    #[test]
    fn waves_enter_and_leave_off_screen() {
        for wave in &WAVES {
            assert!(FLIGHT_LOOP % wave.period == 0.0, "wave period must divide the loop");
            let start = wave_centre(wave, 0.0);
            let edge = if wave.reverse { 1.0 + wave.width * 0.5 } else { -wave.width * 0.5 };
            assert!((start - edge).abs() < 1e-5);
        }
    }

    #[test]
    fn trail_slices_tile_the_trail_and_split_at_the_wrap() {
        let ph = 0.6;
        let (age0, spans0) = trail_clips(ph, 0, 12);
        assert!(age0 < 0.1);
        // The freshest slice ends at the aircraft.
        let head = PATH_X0 + PATH_DX * flight_s(ph);
        assert!((spans0[0].1 - head).abs() < 1e-4);
        // Consecutive slices meet.
        for seg in 0..11 {
            let (_, a) = trail_clips(ph, seg, 12);
            let (_, b) = trail_clips(ph, seg + 1, 12);
            assert!((a[0].0 - b[0].1).abs() < 1e-4, "{seg}");
        }
        // Early in the loop the tail is the previous pass, split at the wrap.
        let wrapped = (0..12).map(|seg| trail_clips(0.02, seg, 12).1).find(|s| s[1].1 > s[1].0);
        let [right, left] = wrapped.expect("a slice straddles the wrap");
        assert!((right.1 - (PATH_X0 + PATH_DX)).abs() < 1e-5 && (left.0 - PATH_X0).abs() < 1e-5);
    }

    #[test]
    fn blink_and_rings_are_seamless() {
        assert_eq!(blink(0.0), 0.0);
        assert!(blink(0.9) > 0.99);
        assert!(blink(0.5) == 0.0 && blink(0.9999) < 0.01);
        let (r0, o0) = ring_state(0.0);
        let (_, o1) = ring_state(1.0);
        assert!(r0 < 0.05 && o0 == 0.0 && o1 == 0.0);
        let (mid, peak) = ring_state(0.2);
        assert!(mid > r0 && peak > 0.2);
        assert!(ring_state(0.9).0 > 1.0);
    }

    #[test]
    fn scan_roll_and_band_match_the_raster_cadence() {
        assert_eq!([0.0, 0.124, 0.125, 0.25, 0.375].map(scan_roll), [0, 0, 1, 2, 0]);
        assert!((band_position(0.0) + 0.15).abs() < 1e-6);
        assert!((band_position(5.999) - 1.15).abs() < 1e-3);
    }

    #[test]
    fn clock_pauses_and_resumes_without_a_jump() {
        let start = Instant::now();
        let at = |ms: u64| start + std::time::Duration::from_millis(ms);
        let mut clock = Clock::default();
        assert_eq!(clock.advance(at(0), true, 1.0), 0.0);
        assert!((clock.advance(at(50), true, 1.0) - 0.05).abs() < 1e-4);
        // Paused for ten seconds: no time passes.
        clock.advance(at(60), false, 1.0);
        assert!((clock.advance(at(10_060), true, 1.0) - 0.05).abs() < 1e-4);
        // A long stall advances at most one step; the debug scale slows it.
        assert!((clock.advance(at(12_000), true, 1.0) - 0.15).abs() < 1e-4);
        assert!((clock.advance(at(12_040), true, 2.0) - 0.17).abs() < 1e-4);
        assert!((clock.elapsed - 0.17).abs() < 1e-4);
    }

    #[test]
    fn ken_burns_drifts_out_and_back() {
        let a = Analysis { focal: (0.7, 0.3), hues: FALLBACK_HUES };
        assert_eq!(motion(Effect::Dither, &a, 5.0), Motion::STILL);
        assert_eq!(motion(Effect::None, &a, 0.0), Motion::STILL);
        let mid = motion(Effect::None, &a, 10.0);
        assert!((mid.zoom - 1.06).abs() < 1e-4 && (mid.center.0 - 0.7).abs() < 1e-4);
        let back = motion(Effect::None, &a, 20.0);
        assert!((back.zoom - 1.0).abs() < 1e-4);
        assert!(motion(Effect::Scanlines, &a, 1.0).opacity <= 1.0);
    }

    #[test]
    fn placement_covers_the_hero() {
        let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(300.0), px(100.0)));
        let (ox, oy, k) = placement(bounds, (640.0, 256.0), Motion::STILL).unwrap();
        assert!(ox <= 10.0 && oy <= 20.0);
        assert!(ox + 640.0 * k >= 310.0 - 1e-3 && oy + 256.0 * k >= 120.0 - 1e-3);
        let zoomed = Motion { zoom: 1.06, center: (0.8, 0.2), opacity: 1.0 };
        let (zx, zy, zk) = placement(bounds, (640.0, 256.0), zoomed).unwrap();
        assert!(zx <= 10.0 && zy <= 20.0 && zx + 640.0 * zk >= 310.0 && zy + 256.0 * zk >= 120.0);
    }

    #[test]
    fn frames_only_place_precomputed_layers() {
        for e in Effect::ALL {
            let layers = Layers::build(&lit_corner(97, 61), e, false);
            let owned = layers.images();
            for t in [0.0, 0.7, 3.3, 11.9, 23.5] {
                let c = compose(&layers, t, 1.0);
                assert_eq!(c.size, (97.0, 61.0));
                assert!(!c.groups.is_empty(), "{e:?}");
                for g in &c.groups {
                    assert!(g.opacity > 0.0 && g.opacity <= 1.0, "{e:?} {}", g.opacity);
                    for s in &g.sprites {
                        assert!(owned.iter().any(|i| Arc::ptr_eq(i, &s.image)), "{e:?}: a per-frame raster");
                        assert!(s.frame < s.image.frame_count());
                        assert!(s.rect.iter().all(|v| v.is_finite()));
                    }
                }
            }
        }
    }

    fn placed(c: &Composition) -> Vec<(u32, usize, [i32; 4])> {
        c.groups
            .iter()
            .flat_map(|g| {
                let o = (g.opacity * 1000.0).round() as u32;
                g.sprites.iter().map(move |s| (o, s.frame, s.rect.map(|v| v.round() as i32)))
            })
            .collect()
    }

    #[test]
    fn dither_flight_moves_and_loops_without_a_seam() {
        let layers = Layers::build(&lit_corner(256, 144), Effect::Dither, false);
        let a = compose(&layers, 6.0, 1.0);
        assert_ne!(placed(&a), placed(&compose(&layers, 7.0, 1.0)));
        assert_eq!(placed(&a), placed(&compose(&layers, 6.0 + FLIGHT_LOOP, 1.0)));
        // The intro holds the overlays back; the base is always there.
        let start = compose(&layers, 6.0, 0.0);
        assert_eq!(start.groups.len(), 1);
    }

    #[test]
    fn dither_layers_stay_on_the_bayer_rule() {
        let s = lit_corner(256, 144);
        let layers = Layers::build(&s, Effect::Dither, false);
        let Kind::Dither { lift, trails, plane, trail_top, .. } = &layers.kind else { panic!() };
        // Every opaque cell is normalised to full brightness or kept at 8%.
        for image in [lift, &trails[0], plane] {
            let bytes = image.as_bytes(0).unwrap();
            for px in bytes.chunks(4).filter(|p| p[3] == 255) {
                let peak = px[0].max(px[1]).max(px[2]);
                assert!(peak >= 254 || peak <= 21, "{px:?}");
            }
        }
        assert!(trail_top.iter().all(|t| t % 8 == 0));
        assert_eq!(plane.frame_count(), PLANE_HEADINGS);
        // The lift mask only holds cells that turn on at the higher gain.
        let base = render(&s, Effect::Dither, false);
        let lift = lift.as_bytes(0).unwrap();
        for (i, px) in lift.chunks(4).enumerate().filter(|(_, p)| p[3] > 0) {
            let b = base.as_raw()[i * 4..i * 4 + 4].to_vec();
            assert_ne!(px, &b[..], "pixel {i} is already in the base");
        }
    }

    #[test]
    fn scanline_layers_reproduce_the_still() {
        let s = src(9, 7);
        let still = render(&s, Effect::Scanlines, false);
        assert_eq!(still.get_pixel(0, 0).0, [26, 52, 104, 255]);
        assert_eq!(still.get_pixel(0, 1).0, [50, 100, 200, 255]);
        // Base plus the line strip at roll 0, blended as the GPU does.
        let lines = lines_strip(7, false);
        let base = plain(&s);
        for y in 0..7u32 {
            let line = lines.get_pixel(0, y + 3).0;
            let a = line[3] as f32 / 255.0;
            let b = base.get_pixel(0, y).0;
            let over = [0, 1, 2].map(|i| b[i] as f32 * (1.0 - a) + line[i] as f32 * a);
            let want = still.get_pixel(0, y).0;
            for i in 0..3 {
                assert!((over[i] - want[i] as f32).abs() <= 1.0, "row {y}: {over:?} vs {want:?}");
            }
        }
        let layers = Layers::build(&s, Effect::Scanlines, false);
        let c = compose(&layers, 0.0, 1.0);
        let line = c.groups.iter().flat_map(|g| &g.sprites).find(|s| s.image.size(0).height.0 == 10).unwrap();
        assert_eq!(line.rect[1], -3.0);
    }

    #[test]
    fn hero_height_caps() {
        assert_eq!(hero_height(2000.0), MAX_HEIGHT);
        assert!((hero_height(500.0) - 360.0).abs() < 1e-3);
    }
}
