//! The new-session hero: a user-chosen wallpaper rendered through a retro
//! effect and faded into the canvas, with the composer cut out of it.
//! Effects and mask geometry adapted from Zeron (MIT). Decoding and effect
//! rasterization run once per image on the background executor.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

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

    /// What "Animate backdrop" does with this effect.
    pub fn motion_hint(self) -> &'static str {
        match self {
            Self::None => "Slow push toward the brightest part of the image",
            Self::Dither => "An aircraft crosses toward the brightest point, trailing light in the image's colors",
            Self::Halftone => "Dots swell and shrink in a ripple from the brightest point",
            Self::Scanlines => "CRT roll: the lines scroll and a soft band sweeps down",
        }
    }

    /// Frame cadence. Original moves on the GPU only, so it just repaints.
    fn frame_interval(self) -> Duration {
        Duration::from_secs_f32(match self {
            Self::None => 1.0 / 30.0,
            Self::Halftone => 1.0 / 15.0,
            Self::Dither | Self::Scanlines => 1.0 / 24.0,
        })
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
    /// The device-pixel size it was rasterized for, in 64 px steps.
    target: (u32, u32),
    slot: Slot,
    /// The previous size's raster, shown until this one is ready.
    fallback: Option<Arc<RenderImage>>,
}

static CACHE: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();

/// The rendered artwork for `path`, or `None` while it is still decoding
/// (the window refreshes when it is ready). `target` is the hero's size in
/// device pixels: the effect is rasterized at that size, as Zeron does, so a
/// dither or halftone cell is the same few screen pixels on every display
/// instead of growing with the source image's upscale.
pub fn prepare(path: &Path, effect: Effect, light: bool, target: (f32, f32), cx: &mut App) -> Option<Arc<RenderImage>> {
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    let light = light && matches!(effect, Effect::Halftone | Effect::Scanlines);
    let step = |v: f32| ((v.max(64.0) / 64.0).ceil() as u32 * 64).min(8192);
    let target = (step(target.0), step(target.1));
    let mut stale = None;
    {
        let entries = cache.lock().ok()?;
        if let Some(e) = entries.iter().find(|e| e.path == path && e.effect == effect && e.light == light) {
            let ready = e.slot.lock().ok()?.clone();
            if e.target == target {
                return ready.or_else(|| e.fallback.clone());
            }
            // Resized: keep showing the old raster until the new one lands.
            stale = ready;
        }
    }
    let slot: Slot = Arc::new(Mutex::new(None));
    {
        let mut entries = cache.lock().ok()?;
        entries.retain(|e| !(e.path == path && e.effect == effect && e.light == light));
        entries.push(Entry {
            path: path.to_path_buf(),
            effect,
            light,
            target,
            slot: slot.clone(),
            fallback: stale.clone(),
        });
        // A handful of variants is plenty; drop the oldest.
        if entries.len() > 4 {
            entries.remove(0);
        }
    }
    let path = path.to_path_buf();
    let work = cx.background_executor().spawn(async move {
        let src = load_source(&path, target)?;
        Some(Arc::new(RenderImage::new([image::Frame::new(render(&src, effect, light))])))
    });
    cx.spawn(async move |cx| {
        let image = work.await;
        *slot.lock().unwrap() = image;
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
    stale
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
        Effect::Scanlines => scanlines(src, light, 0, |_| 1.0),
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
        Effect::Halftone => halftone(src, light, |_, _| 1.0),
    }
}

/// Every third row darkened (or lifted, on light paper). `roll` shifts the
/// pattern down, `row_gain` brightens individual rows for the sweep band.
fn scanlines(src: &Source, light: bool, roll: u32, row_gain: impl Fn(u32) -> f32) -> image::RgbaImage {
    let (w, h) = (src.width, src.height);
    let mut out = image::RgbaImage::new(w, h);
    let mut lut = [0u8; 256];
    for y in 0..h {
        let gain = if (y + roll).is_multiple_of(3) { 0.52 } else { 1.0 };
        let extra = row_gain(y);
        for (v, slot) in lut.iter_mut().enumerate() {
            let v = v as f32;
            let c = if light { (v + (255.0 - v) * (1.0 - gain)) as u8 } else { (v * gain) as u8 };
            *slot = if extra == 1.0 { c } else { (c as f32 * extra).min(255.0) as u8 };
        }
        for x in 0..w {
            let [r, g, b, a] = src.rgba[(y * w + x) as usize];
            out.put_pixel(x, y, image::Rgba([lut[b as usize], lut[g as usize], lut[r as usize], a]));
        }
    }
    out
}

/// 4 px dots sized by luma. `ripple(cx, cy)` scales each dot's radius.
fn halftone(src: &Source, light: bool, ripple: impl Fn(u32, u32) -> f32) -> image::RgbaImage {
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
            let radius = 2.0 * (0.3 + 0.7 * (luma as f32 / 255.0).sqrt()) * ripple(cx_, cy_);
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

// ------------------------------------------------------------ animation
//
// "Animate backdrop" gives each effect its own motion. Everything is derived
// from the image (its brightest region and two dominant hues), so it works on
// any wallpaper. Raster motion runs on the background executor at a capped
// rate; motion the GPU can do alone (zoom, flicker) is a paint parameter.

const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// How the painted hero moves this frame, for motion that needs no raster.
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

/// One loop of the Dither flight, in seconds. Every time term in the frame is
/// periodic in it, so the animation repeats without a seam.
const FLIGHT_LOOP: f32 = 24.0;

/// The aircraft's position in hero pixels at loop phase `s`: lower left to
/// upper right, off screen at both ends so the wrap is never seen, bending
/// through the image's focal point so it crosses the subject.
fn flight_path(a: &Analysis, w: f32, h: f32, s: f32) -> (f32, f32) {
    let s = s.rem_euclid(1.0);
    let x = -0.18 + 1.36 * s;
    let base = |x: f32| 0.82 - 0.6 * x;
    let over_focal = (a.focal.0 + 0.18) / 1.36;
    let bend = (a.focal.1 - base(a.focal.0)) * (-((s - over_focal) / 0.22).powi(2)).exp();
    (x * w, (base(x) + bend + 0.03 * (s * std::f32::consts::TAU).sin()) * h)
}

fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (ex, ey) = (b.0 - a.0, b.1 - a.1);
    let k = (((p.0 - a.0) * ex + (p.1 - a.1) * ey) / (ex * ex + ey * ey).max(1e-6)).clamp(0.0, 1.0);
    (p.0 - a.0 - ex * k).hypot(p.1 - a.1 - ey * k)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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

/// One 2x2 dither cell's source colour, prepared once per image so a frame
/// only scales it by the wave gain and compares it with the threshold.
struct DitherCell {
    /// RGB, 0..1.
    rgb: [f32; 3],
    /// Brightest channel, 0..1.
    peak: f32,
    /// BGRA with the brightest channel normalised to 255: the "on" colour.
    /// A uniform gain cancels out of it, so it never changes per frame.
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

/// The Dither flight: an aircraft crosses the image toward its brightest
/// point, trailing two luminous contrails in the image's own hues, while
/// slow brightness waves move through the signal. The whole frame is
/// re-dithered each time, but the Bayer grid never moves.
fn dither_frame(src: &Source, table: &[DitherCell], a: &Analysis, t: f32) -> image::RgbaImage {
    use std::f32::consts::TAU;
    let (w, h) = (src.width, src.height);
    let (wf, hf) = (w as f32, h as f32);
    let ph = (t / FLIGHT_LOOP).rem_euclid(1.0);
    let (gw, gh) = (w.div_ceil(2), h.div_ceil(2));
    let mut cells = LightCells { gw, gh, rgb: vec![[0.0; 3]; (gw * gh) as usize] };

    // Twin trails: 44 samples back along the path, drifting apart and
    // widening with age, shimmering as they fade.
    // Previous sample: trail A point, trail B point, wrapped path phase.
    type Sample = ((f32, f32), (f32, f32), f32);
    let mut prev: Option<Sample> = None;
    for i in 0..44 {
        let age = i as f32 / 43.0;
        let s = ph - age * 0.42;
        let c0 = flight_path(a, wf, hf, s);
        let c1 = flight_path(a, wf, hf, s + 0.004);
        let len = (c1.0 - c0.0).hypot(c1.1 - c0.1).max(1e-3);
        let (nx, ny) = (-(c1.1 - c0.1) / len, (c1.0 - c0.0) / len);
        let spread = (0.009 + 0.04 * age) * hf;
        let (pa, pb) = ((c0.0 + nx * spread, c0.1 + ny * spread), (c0.0 - nx * spread, c0.1 - ny * spread));
        let s_wrapped = s.rem_euclid(1.0);
        // Skip the segment that would join both ends of the wrap.
        if let Some((qa, qb, qs)) = prev
            && (s_wrapped - qs).abs() < 0.5
        {
            let width = (0.006 + 0.02 * age) * hf;
            let amp = (1.0 - age).powi(2) * (0.8 + 0.2 * (age * 48.0 - ph * TAU * 12.0).sin());
            cells.trail(qa, pa, width, amp, a.hues[0]);
            cells.trail(qb, pb, width, amp, a.hues[1]);
        }
        prev = Some((pa, pb, s_wrapped));
    }

    // The aircraft: body along its heading, wings and tailplane across,
    // a soft glow, and wingtip lights blinking once a second.
    let p = flight_path(a, wf, hf, ph);
    let ahead = flight_path(a, wf, hf, ph + 0.004);
    let len = (ahead.0 - p.0).hypot(ahead.1 - p.1).max(1e-3);
    let (tx, ty) = ((ahead.0 - p.0) / len, (ahead.1 - p.1) / len);
    let (nx, ny) = (-ty, tx);
    let at = |along: f32, across: f32| (p.0 + (tx * along + nx * across) * hf, p.1 + (ty * along + ny * across) * hf);
    let parts = [
        (at(-0.026, 0.0), at(0.02, 0.0)),
        (at(-0.003, -0.032), at(-0.003, 0.032)),
        (at(-0.026, -0.011), at(-0.026, 0.011)),
    ];
    let blink = if (ph * FLIGHT_LOOP).fract() > 0.82 { 1.0 } else { 0.0 };
    let (port, starboard) = (at(-0.003, 0.032), at(-0.003, -0.032));
    let reach = 0.08 * hf;
    cells.each(p, p, reach, |q, light| {
        let d = parts.iter().map(|(a, b)| seg_dist(q, *a, *b)).fold(f32::MAX, f32::min);
        let r2 = ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)) / (hf * hf);
        let body = smoothstep(0.005 * hf, 0.002 * hf, d) + 0.4 * (-r2 / 0.002).exp();
        let tip = |c: (f32, f32)| blink * (-((q.0 - c.0).powi(2) + (q.1 - c.1).powi(2)) / (hf * hf) / 0.00004).exp();
        let (red, green) = (tip(port), tip(starboard));
        let add = [0.9 * body + red + 0.2 * green, 0.92 * body + 0.2 * red + green, body + 0.15 * red + 0.4 * green];
        for (l, v) in light.iter_mut().zip(add) {
            *l += v;
        }
    });

    // Brightness waves: sin(kx·x + ky·y − φ) split into per-column and
    // per-row terms, so each cell costs a few multiplies.
    let wave = |kx: f32, ky: f32, phase: f32| {
        let cols: Vec<(f32, f32)> = (0..gw).map(|cx| ((cx * 2 + 1) as f32 / hf * kx).sin_cos()).collect();
        let rows: Vec<(f32, f32)> =
            (0..gh).map(|cy| ((1.0 - (cy * 2 + 1) as f32 / hf) * ky + phase).sin_cos()).collect();
        (cols, rows)
    };
    let (cols_a, rows_a) = wave(5.0, 2.75, -ph * TAU * 2.0);
    let (cols_b, rows_b) = wave(-1.8, 3.0, ph * TAU);

    let mut out = vec![0u8; (w * h * 4) as usize];
    for cy in 0..gh {
        let (y, (ras, rac), (rbs, rbc)) = (cy * 2, rows_a[cy as usize], rows_b[cy as usize]);
        let bayer = &BAYER[cy as usize % 4];
        for cx in 0..gw {
            let x = cx * 2;
            let (cas, cac) = cols_a[cx as usize];
            let (cbs, cbc) = cols_b[cx as usize];
            let wa = 0.5 + 0.5 * (cas * rac + cac * ras);
            let wb = 0.5 + 0.5 * (cbs * rbc + cbc * rbs);
            let gain = 0.86 + 0.22 * wa * wb;
            let i = (cy * gw + cx) as usize;
            let cell = &table[i];
            let light = cells.rgb[i];
            let threshold = (bayer[cx as usize % 4] as f32 + 0.5) / 16.0;
            // The `dither` rule: brightest channel against the Bayer
            // threshold; on cells reach full brightness, off cells keep 8%.
            let peak_light = light[0].max(light[1]).max(light[2]).min(1.0);
            let px = if peak_light <= 0.0 {
                if cell.peak * gain > threshold {
                    cell.on
                } else {
                    let [r, g, b] = cell.rgb.map(|v| (v * gain * 0.08 * 255.0).round() as u8);
                    [b, g, r, cell.alpha]
                }
            } else {
                // Wave gain, then the light screened on top, all in 0..1.
                let c = [0, 1, 2].map(|k| 1.0 - (1.0 - (cell.rgb[k] * gain).min(1.0)) * (1.0 - light[k].min(1.0)));
                let peak = c[0].max(c[1]).max(c[2]);
                let scale = if peak > threshold { 255.0 / peak.max(1.0 / 255.0) } else { 0.08 * 255.0 };
                let [r, g, b] = c.map(|v| (v * scale).round().min(255.0) as u8);
                // Light shows even over transparent parts of the art.
                [b, g, r, cell.alpha.max((peak_light * 255.0) as u8)]
            };
            let pair = [px[0], px[1], px[2], px[3], px[0], px[1], px[2], px[3]];
            let span = (2.min(w - x) * 4) as usize;
            for dy in 0..2.min(h - y) {
                let at = (((y + dy) * w + x) * 4) as usize;
                out[at..at + span].copy_from_slice(&pair[..span]);
            }
        }
    }
    image::RgbaImage::from_raw(w, h, out).unwrap_or_else(|| image::RgbaImage::new(w, h))
}

/// One animated frame at `t` seconds, or `None` when the effect moves on
/// the GPU only (Original).
fn render_frame(p: &Prepared, t: f32) -> Option<image::RgbaImage> {
    let (w, h) = (p.src.width as f32, p.src.height as f32);
    match p.effect {
        Effect::None => None,
        Effect::Dither => Some(dither_frame(&p.src, &p.cells, &p.analysis, t)),
        Effect::Halftone => Some(halftone(&p.src, p.light, |cx_, cy_| {
            let rel = ((cx_ as f32 / w - p.analysis.focal.0) * w / h, cy_ as f32 / h - p.analysis.focal.1);
            1.0 + 0.15 * (rel.0.hypot(rel.1) * 16.0 - t * 2.2).sin()
        })),
        Effect::Scanlines => {
            let band_pos = (t / 6.0).fract() * 1.3 - 0.15;
            Some(scanlines(&p.src, p.light, (t * 8.0) as u32 % 3, |y| {
                1.0 + 0.18 * (-((y as f32 / h - band_pos) / 0.05).powi(2)).exp()
            }))
        }
    }
}

/// GPU-side motion at `t`: Ken Burns for Original, flicker for Scanlines.
fn motion(effect: Effect, a: &Analysis, t: f32) -> Motion {
    match effect {
        Effect::None => {
            let k = 0.5 - 0.5 * (std::f32::consts::TAU * t / 20.0).cos();
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

/// Everything decoded once per image, effect and size.
struct Prepared {
    effect: Effect,
    light: bool,
    src: Source,
    analysis: Analysis,
    /// Dither only: per-cell source colours.
    cells: Vec<DitherCell>,
    still: Arc<RenderImage>,
}

struct Anim {
    path: PathBuf,
    effect: Effect,
    light: bool,
    target: (u32, u32),
    generation: u64,
    prepared: Option<Arc<Prepared>>,
    frame: Option<Arc<RenderImage>>,
    /// Frames replaced since the last paint, to free from the sprite atlas.
    retired: Vec<Arc<RenderImage>>,
    /// A decode, timer or frame job is in flight.
    busy: bool,
    start: Instant,
}

static ANIM: OnceLock<Mutex<Option<Anim>>> = OnceLock::new();
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The animated hero for this frame and its paint motion, or `None` while
/// the image decodes. It keeps the next frame coming: each finished frame
/// refreshes the window, which calls this again.
pub fn animate(
    path: &Path,
    effect: Effect,
    light: bool,
    target: (f32, f32),
    window: &mut Window,
    cx: &mut App,
) -> Option<(Arc<RenderImage>, Motion)> {
    let light = light && matches!(effect, Effect::Halftone | Effect::Scanlines);
    let step = |v: f32| ((v.max(64.0) / 64.0).ceil() as u32 * 64).min(8192);
    let target = (step(target.0), step(target.1));
    let slot = ANIM.get_or_init(|| Mutex::new(None));
    let mut guard = slot.lock().ok()?;

    let same =
        guard.as_ref().is_some_and(|a| a.path == path && a.effect == effect && a.light == light && a.target == target);
    if !same {
        // Keep the old picture on screen until the new image is ready.
        let (retired, carried) = match guard.take() {
            Some(old) => {
                let still = old.prepared.map(|p| p.still.clone());
                match old.frame {
                    Some(frame) => (old.retired.into_iter().chain(still).collect(), Some(frame)),
                    None => (old.retired, still),
                }
            }
            None => (Vec::new(), None),
        };
        let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let job_path = path.to_path_buf();
        let work = cx.background_executor().spawn(async move {
            let src = load_source(&job_path, target)?;
            let analysis = analyze(&src);
            let cells = if effect == Effect::Dither { dither_cells(&src) } else { Vec::new() };
            let still = Arc::new(RenderImage::new([image::Frame::new(render(&src, effect, light))]));
            Some(Arc::new(Prepared { effect, light, src, analysis, cells, still }))
        });
        cx.spawn(async move |cx| {
            let prepared = work.await;
            if let Some(mut guard) = ANIM.get().and_then(|s| s.lock().ok())
                && let Some(a) = guard.as_mut()
                && a.generation == generation
            {
                a.prepared = prepared;
                a.retired.extend(a.frame.take());
                a.busy = false;
                a.start = Instant::now();
            }
            cx.update(|cx| cx.refresh_windows());
        })
        .detach();
        *guard = Some(Anim {
            path: path.to_path_buf(),
            effect,
            light,
            target,
            generation,
            prepared: None,
            frame: carried,
            retired,
            busy: true,
            start: Instant::now(),
        });
    }

    let a = guard.as_mut()?;
    for image in a.retired.drain(..) {
        let _ = window.drop_image(image);
    }
    let t = a.start.elapsed().as_secs_f32();
    let motion = a.prepared.as_ref().map_or(Motion::STILL, |p| motion(effect, &p.analysis, t));
    // Frames are only requested from here, so the loop stops on its own when
    // the home page is not being drawn (another page, or a hidden window).
    if !a.busy
        && let Some(prepared) = a.prepared.clone()
    {
        a.busy = true;
        let generation = a.generation;
        let interval = effect.frame_interval();
        let next_t = t + interval.as_secs_f32();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |cx| {
            executor.timer(interval).await;
            let frame = executor
                .spawn(async move {
                    render_frame(&prepared, next_t).map(|img| Arc::new(RenderImage::new([image::Frame::new(img)])))
                })
                .await;
            if let Some(mut guard) = ANIM.get().and_then(|s| s.lock().ok())
                && let Some(a) = guard.as_mut()
                && a.generation == generation
            {
                if let Some(frame) = frame {
                    a.retired.extend(a.frame.replace(frame));
                }
                a.busy = false;
            }
            cx.update(|cx| cx.refresh_windows());
        })
        .detach();
    }
    let image = a.frame.clone().or_else(|| a.prepared.as_ref().map(|p| p.still.clone()))?;
    Some((image, motion))
}

/// Stop animating and free its rasters, e.g. when the switch is turned off.
pub fn release_animation(window: &mut Window) {
    let Some(slot) = ANIM.get() else { return };
    let Some(old) = slot.lock().ok().and_then(|mut g| g.take()) else { return };
    let images = old.retired.into_iter().chain(old.frame).chain(old.prepared.map(|p| p.still.clone()));
    for image in images {
        let _ = window.drop_image(image);
    }
}

pub fn hero_height(viewport_height: f32) -> f32 {
    (viewport_height.max(0.0) * VIEWPORT_RATIO).min(MAX_HEIGHT)
}

/// Paint `image` cover-fitted into `bounds`, fading toward the bottom.
/// With `cutout`, the composer's rounded rect is feathered out of it.
/// `motion` zooms the picture inside `bounds`; the mask stays put.
pub fn paint(
    image: Arc<RenderImage>,
    bounds: Bounds<Pixels>,
    composer: Option<Bounds<Pixels>>,
    cutout: bool,
    motion: Motion,
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
    // Scaling about a point inside the hero keeps it covered for zoom >= 1.
    let (zx, zy) = (w * motion.center.0, h * motion.center.1);
    let zoom = motion.zoom.max(1.0);
    let (left, top) = (zx + ((w - fw) * 0.5 - zx) * zoom, zy + ((h - fh) * 0.5 - zy) * zoom);
    let fitted =
        Bounds::new(point(bounds.left() + px(left), bounds.top() + px(top)), size(px(fw * zoom), px(fh * zoom)));
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

    fn prepared(src: Source, effect: Effect) -> Prepared {
        let analysis = analyze(&src);
        let still = Arc::new(RenderImage::new([image::Frame::new(render(&src, effect, false))]));
        let cells = dither_cells(&src);
        Prepared { effect, light: false, src, analysis, cells, still }
    }

    #[test]
    fn frames_keep_dimensions() {
        for e in [Effect::Dither, Effect::Halftone, Effect::Scanlines] {
            let p = prepared(lit_corner(97, 61), e);
            for t in [0.0, 0.7, 3.3] {
                let img = render_frame(&p, t).expect("raster effect");
                assert_eq!((img.width(), img.height()), (97, 61), "{e:?} at {t}");
            }
        }
        assert!(render_frame(&prepared(lit_corner(8, 8), Effect::None), 1.0).is_none());
    }

    #[test]
    fn dither_flight_follows_the_bayer_rule() {
        // Every cell is either normalised to full brightness or kept at 8%.
        let p = prepared(lit_corner(256, 144), Effect::Dither);
        let frame = render_frame(&p, 9.0).unwrap();
        for px in frame.pixels() {
            let peak = px.0[0].max(px.0[1]).max(px.0[2]);
            assert!(peak >= 254 || peak <= 21, "{:?}", px.0);
        }
        // Each 2x2 cell is one colour, so the grid stays put.
        for y in (0..144).step_by(2) {
            for x in (0..256).step_by(2) {
                let c = frame.get_pixel(x, y);
                assert_eq!(c, frame.get_pixel(x + 1, y + 1));
            }
        }
    }

    #[test]
    fn dither_flight_moves_and_loops_without_a_seam() {
        let p = prepared(lit_corner(256, 144), Effect::Dither);
        let a = render_frame(&p, 6.0).unwrap();
        assert_ne!(a, render_frame(&p, 7.0).unwrap());
        assert_eq!(a, render_frame(&p, 6.0 + FLIGHT_LOOP).unwrap());
    }

    #[test]
    fn flight_path_crosses_the_focal_point_and_starts_off_screen() {
        let a = Analysis { focal: (0.6, 0.35), hues: FALLBACK_HUES };
        let (x0, _) = flight_path(&a, 1000.0, 500.0, 0.0);
        let (x1, _) = flight_path(&a, 1000.0, 500.0, 0.999);
        assert!(x0 < 0.0 && x1 > 1000.0);
        let over = (0.6 + 0.18) / 1.36;
        let (x, y) = flight_path(&a, 1000.0, 500.0, over);
        assert!((x - 600.0).abs() < 1.0 && (y / 500.0 - 0.35).abs() < 0.04, "{x}, {y}");
    }

    #[test]
    fn static_effects_match_unanimated_helpers() {
        let s = src(9, 7);
        assert_eq!(render(&s, Effect::Scanlines, false).get_pixel(0, 0).0, [26, 52, 104, 255]);
        assert_eq!(render(&s, Effect::Scanlines, false).get_pixel(0, 1).0, [50, 100, 200, 255]);
    }

    #[test]
    fn motion_is_still_except_original_and_scanlines() {
        let a = Analysis { focal: (0.7, 0.3), hues: FALLBACK_HUES };
        assert_eq!(motion(Effect::Dither, &a, 5.0), Motion::STILL);
        assert_eq!(motion(Effect::None, &a, 0.0), Motion::STILL);
        let mid = motion(Effect::None, &a, 10.0);
        assert!((mid.zoom - 1.06).abs() < 1e-4 && (mid.center.0 - 0.7).abs() < 1e-4);
        assert!(motion(Effect::Scanlines, &a, 1.0).opacity <= 1.0);
    }

    #[test]
    fn hero_height_caps() {
        assert_eq!(hero_height(2000.0), MAX_HEIGHT);
        assert!((hero_height(500.0) - 360.0).abs() < 1e-3);
    }
}
