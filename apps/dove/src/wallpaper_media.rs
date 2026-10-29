//! Backgrounds that move, and the chat backdrop's baked frost.
//!
//! A background can be a still, an animated image (GIF, APNG, animated
//! WebP) or a video (MP4, WebM, MOV, MKV, decoded through ffmpeg when it is
//! installed). Stills and animated images are decoded once, off the UI
//! thread, into one multi-frame [`RenderImage`] at no more than their own
//! resolution; playback only picks the frame to paint. A video plays as it
//! decodes: ffmpeg paces and loops it and only the newest frame is kept, so
//! a long 4K clip costs a frame or two of memory, not the whole clip.
//!
//! The chat backdrop never runs a live blur. Its frames are baked: shrunk
//! to an eighth, blurred until only colour fields remain, lifted back to a
//! quarter with a little grain so dark gradients do not band, and painted
//! under a scrim whose strength is solved from the art's brightest region
//! so text always reads (Zeron's frosted wallpaper look). The frost is a
//! fraction of the screen's pixels, so an animated chat background costs
//! a few megabytes and no per-frame GPU blur.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, Context, Corners, IntoElement, ParentElement as _, Pixels, Render, RenderImage, Styled as _, Window,
    canvas, div, point, px, size,
};
use image::{AnimationDecoder, RgbaImage, imageops};

use crate::wallpaper::{Motion, alpha_mask, drop_retired, placement, retire};

/// Video files, decoded through ffmpeg.
pub const VIDEO_EXTS: [&str; 5] = ["mp4", "webm", "mov", "mkv", "m4v"];

/// Frames a full-size (home) clip may hold, in bytes.
const HERO_BUDGET: usize = 192 * 1024 * 1024;
/// Frames a chat frost may hold, in bytes.
const FROST_BUDGET: usize = 64 * 1024 * 1024;
/// Longest stretch of a video that is decoded, in seconds.
const VIDEO_SECONDS: u32 = 30;
/// A frame shorter than this is shown this long (browsers do the same, so
/// a GIF with a zero delay does not spin the CPU).
const MIN_DELAY: f32 = 0.02;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Still,
    Animated,
    Video,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Still => "Image",
            Self::Animated => "Animated image",
            Self::Video => "Video",
        }
    }
}

fn ext(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase()
}

/// What a background file is. Animated PNG and WebP are told from stills
/// by their headers, so a plain `.png` stays on the still pipeline.
pub fn kind_of(path: &Path) -> Kind {
    let e = ext(path);
    if VIDEO_EXTS.contains(&e.as_str()) {
        return Kind::Video;
    }
    let open = || std::fs::File::open(path).ok().map(std::io::BufReader::new);
    let animated = match e.as_str() {
        "gif" => open()
            .and_then(|r| image::codecs::gif::GifDecoder::new(r).ok())
            .is_some_and(|d| d.into_frames().take(2).count() > 1),
        "png" | "apng" => {
            open().and_then(|r| image::codecs::png::PngDecoder::new(r).ok()).and_then(|d| d.is_apng().ok())
                == Some(true)
        }
        "webp" => open().and_then(|r| image::codecs::webp::WebPDecoder::new(r).ok()).is_some_and(|d| d.has_animation()),
        _ => false,
    };
    if animated { Kind::Animated } else { Kind::Still }
}

/// [`kind_of`], remembered per path for a few seconds: a render asks often
/// and the answer reads the file's header.
pub fn kind_cached(path: &Path) -> Kind {
    static SEEN: Mutex<Option<(PathBuf, Kind, Instant)>> = Mutex::new(None);
    let Ok(mut seen) = SEEN.lock() else { return kind_of(path) };
    if let Some((p, k, at)) = seen.as_ref()
        && p == path
        && at.elapsed() < Duration::from_secs(5)
    {
        return *k;
    }
    let k = kind_of(path);
    *seen = Some((path.to_path_buf(), k, Instant::now()));
    k
}

/// The ffmpeg binary: `DOVE_FFMPEG`, then PATH, then the usual install
/// folders. `None` when there is none, and video backgrounds are offered
/// as an install step instead.
pub fn ffmpeg() -> Option<PathBuf> {
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
            let mut candidates: Vec<PathBuf> = std::env::var_os("DOVE_FFMPEG").map(PathBuf::from).into_iter().collect();
            if let Some(path) = std::env::var_os("PATH") {
                candidates.extend(std::env::split_paths(&path).map(|d| d.join(exe)));
            }
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                let local = PathBuf::from(local);
                candidates.push(local.join("Microsoft").join("WinGet").join("Links").join(exe));
                candidates.push(local.join("Dove").join("tools").join(exe));
            }
            if let Some(home) = dirs_home() {
                candidates.push(home.join(".dove").join("tools").join(exe));
                candidates.push(home.join("scoop").join("shims").join(exe));
            }
            candidates
                .extend(["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg", "/usr/bin/ffmpeg"].map(PathBuf::from));
            candidates.into_iter().find(|p| p.is_file())
        })
        .clone()
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

// ------------------------------------------------------------ decoding

/// Decoded frames in GPUI's BGRA order, each shown for its delay.
struct Frames {
    images: Vec<RgbaImage>,
    delays: Vec<f32>,
}

impl Frames {
    fn bytes(&self) -> usize {
        self.images.iter().map(|i| i.as_raw().len()).sum()
    }

    /// Keep every k-th frame (folding the dropped frames' time into the
    /// kept ones) until the clip fits `budget`.
    fn fit(mut self, budget: usize) -> Self {
        let bytes = self.bytes();
        if bytes <= budget || self.images.len() < 2 {
            return self;
        }
        let k = bytes.div_ceil(budget).max(2);
        let mut images = Vec::new();
        let mut delays = Vec::new();
        for (i, (img, d)) in self.images.drain(..).zip(self.delays.drain(..)).enumerate() {
            if i % k == 0 {
                images.push(img);
                delays.push(d);
            } else if let Some(last) = delays.last_mut() {
                *last += d;
            }
        }
        Frames { images, delays }
    }
}

/// Cover-fit `img` into `target`, as BGRA.
fn cover_bgra(img: &RgbaImage, target: (u32, u32)) -> RgbaImage {
    let fitted = image::DynamicImage::ImageRgba8(img.clone())
        .resize_to_fill(target.0.max(1), target.1.max(1), imageops::FilterType::Triangle)
        .to_rgba8();
    swap_rb(fitted)
}

/// Like [`cover_bgra`], but never past the source's own resolution: a
/// small GIF keeps its pixels and the GPU scales it up when painting, so
/// every frame fits the budget instead of every nineteenth.
fn cover_within(img: &RgbaImage, target: (u32, u32)) -> RgbaImage {
    let (w, h) = img.dimensions();
    let (tw, th) = (target.0.max(1), target.1.max(1));
    let k = (w as f32 / tw as f32).min(h as f32 / th as f32);
    if k >= 1.0 {
        return cover_bgra(img, target);
    }
    // The largest centred crop with the target's shape.
    let (cw, ch) = (((tw as f32 * k).round() as u32).clamp(1, w), ((th as f32 * k).round() as u32).clamp(1, h));
    swap_rb(imageops::crop_imm(img, (w - cw) / 2, (h - ch) / 2, cw, ch).to_image())
}

fn swap_rb(mut img: RgbaImage) -> RgbaImage {
    for p in img.pixels_mut() {
        p.0.swap(0, 2);
    }
    img
}

fn delay_secs(frame: &image::Frame) -> f32 {
    let (n, d) = frame.delay().numer_denom_ms();
    (n as f32 / d.max(1) as f32 / 1000.0).max(MIN_DELAY)
}

/// Decode an animated image's frames, each fitted to `target` and passed
/// through `look`, stopping once `budget` is reached.
fn decode_animated(
    path: &Path,
    target: (u32, u32),
    budget: usize,
    look: &dyn Fn(RgbaImage) -> RgbaImage,
) -> Option<Frames> {
    let reader = || std::fs::File::open(path).ok().map(std::io::BufReader::new);
    let frames: Box<dyn Iterator<Item = image::ImageResult<image::Frame>>> = match ext(path).as_str() {
        "gif" => Box::new(image::codecs::gif::GifDecoder::new(reader()?).ok()?.into_frames()),
        "png" | "apng" => Box::new(image::codecs::png::PngDecoder::new(reader()?).ok()?.apng().ok()?.into_frames()),
        "webp" => Box::new(image::codecs::webp::WebPDecoder::new(reader()?).ok()?.into_frames()),
        _ => return None,
    };
    let mut out = Frames { images: Vec::new(), delays: Vec::new() };
    let mut bytes = 0usize;
    // Decode past the budget only as far as twice it, then thin out: a long
    // GIF keeps its whole loop at a lower frame rate rather than stopping.
    for frame in frames {
        let Ok(frame) = frame else { break };
        let delay = delay_secs(&frame);
        let img = look(cover_within(frame.buffer(), target));
        bytes += img.as_raw().len();
        out.images.push(img);
        out.delays.push(delay);
        if bytes > budget.saturating_mul(2) {
            out = out.fit(budget);
            bytes = out.bytes();
        }
    }
    (!out.images.is_empty()).then(|| out.fit(budget))
}

/// Decode up to [`VIDEO_SECONDS`] (and at most `max_frames`) of a video
/// through ffmpeg as raw BGRA at exactly `target` (cover-cropped), at `fps`.
fn decode_video(
    path: &Path,
    target: (u32, u32),
    fps: u32,
    budget: usize,
    max_frames: usize,
    look: &dyn Fn(RgbaImage) -> RgbaImage,
) -> Option<Frames> {
    use std::io::Read as _;
    let ffmpeg = ffmpeg()?;
    let (w, h) = (target.0.max(2) & !1, target.1.max(2) & !1);
    let filter = format!("fps={fps},scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}");
    let mut cmd = std::process::Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-t"])
        .arg(VIDEO_SECONDS.to_string())
        .arg("-i")
        .arg(path)
        .args(["-an", "-vf", &filter, "-frames:v"])
        .arg(max_frames.min(u32::MAX as usize).to_string())
        .args(["-f", "rawvideo", "-pix_fmt", "bgra", "-"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let frame_len = w as usize * h as usize * 4;
    let mut out = Frames { images: Vec::new(), delays: Vec::new() };
    let mut bytes = 0usize;
    while out.images.len() < max_frames {
        let mut buf = vec![0u8; frame_len];
        if stdout.read_exact(&mut buf).is_err() {
            break;
        }
        let Some(img) = RgbaImage::from_raw(w, h, buf) else { break };
        let img = look(img);
        bytes += img.as_raw().len();
        out.images.push(img);
        out.delays.push(1.0 / fps as f32);
        if bytes > budget.saturating_mul(2) {
            out = out.fit(budget);
            bytes = out.bytes();
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    (!out.images.is_empty()).then(|| out.fit(budget))
}

/// The first frame of any background, for stills and thumbnails.
fn decode_still(path: &Path, target: (u32, u32), look: &dyn Fn(RgbaImage) -> RgbaImage) -> Option<Frames> {
    let img = match kind_of(path) {
        Kind::Video => {
            return decode_video(path, target, 1, usize::MAX, 1, look).map(|mut f| {
                f.images.truncate(1);
                f.delays.truncate(1);
                f
            });
        }
        _ => {
            let bytes = std::fs::read(path).ok()?;
            image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.decode().ok()?.to_rgba8()
        }
    };
    Some(Frames { images: vec![look(cover_bgra(&img, target))], delays: vec![1.0] })
}

/// The wallpaper's dominant colour, for tinting the sidebar and dock: a
/// quantized histogram over a small first frame (Zeron's, coarser) that
/// skips near-black pixels and favours bright, saturated ones, so a dark
/// photo gives its sky or its lights rather than its shadows. Blocking (it decodes, a video through ffmpeg), so call
/// it off the UI thread.
pub fn dominant_color(path: &Path) -> Option<gpui::Hsla> {
    let frames = decode_still(path, (160, 90), &|img| img)?;
    let img = frames.images.first()?;
    let mut bins = vec![(0.0_f64, [0.0_f64; 3]); 512];
    // `cover_bgra` hands back BGRA.
    for p in img.pixels() {
        let [b, g, r, a] = p.0;
        let high = r.max(g).max(b) as f64;
        if a < 128 || high < 48.0 {
            continue;
        }
        let low = r.min(g).min(b) as f64;
        let saturation = (high - low) / high;
        let weight = 0.05 + saturation * saturation * high / 255.0;
        let index = ((r as usize >> 5) << 6) | ((g as usize >> 5) << 3) | (b as usize >> 5);
        let (count, sums) = &mut bins[index];
        *count += weight;
        for (sum, channel) in sums.iter_mut().zip([r, g, b]) {
            *sum += channel as f64 * weight;
        }
    }
    let (count, sums) = bins.into_iter().max_by(|a, b| a.0.total_cmp(&b.0))?;
    (count > 0.0).then(|| {
        let c = |i: usize| (sums[i] / count / 255.0) as f32;
        gpui::Rgba { r: c(0), g: c(1), b: c(2), a: 1.0 }.into()
    })
}

// ------------------------------------------------------------ the frost

/// How strongly the chat backdrop is frosted, from the "Chat blur" slider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrostLook {
    /// Blur in screen pixels (Soft 40, Frost 80, Deep 120).
    pub blur: f32,
}

impl FrostLook {
    pub const DEFAULT_BLUR: f32 = 80.0;
    pub const MIN_BLUR: f32 = 24.0;
    pub const MAX_BLUR: f32 = 140.0;
}

/// The frost of one BGRA frame at `target` (device px): blurred at an
/// eighth, lifted to a quarter with blue-ish grain. Returns the frame at
/// the quarter size; the GPU's bilinear upscale is invisible at this blur.
fn frost_frame(img: RgbaImage, target: (u32, u32), look: FrostLook) -> RgbaImage {
    let (qw, qh) = ((target.0 / 4).max(8), (target.1 / 4).max(8));
    let (ew, eh) = ((target.0 / 8).max(4), (target.1 / 8).max(4));
    let small = imageops::resize(&img, ew, eh, imageops::FilterType::Triangle);
    let small = imageops::blur(&small, (look.blur / 8.0).max(1.0));
    let mut out = imageops::resize(&small, qw, qh, imageops::FilterType::Triangle);
    grain(&mut out);
    out
}

/// ±1.5% luminance noise from a fixed hash, so every frame of a clip has
/// the same grain and the frost does not shimmer.
fn grain(img: &mut RgbaImage) {
    for (x, y, p) in img.enumerate_pixels_mut() {
        let mut h = x.wrapping_mul(0x9E37_79B9) ^ y.wrapping_mul(0x85EB_CA6B);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2C1B_3C6D);
        h ^= h >> 12;
        let n = ((h & 0xff) as f32 / 255.0 - 0.5) * 2.0 * 3.8;
        for c in &mut p.0[..3] {
            *c = (*c as f32 + n).clamp(0.0, 255.0) as u8;
        }
    }
}

/// The relative luminance (0..1) below which 95% of a BGRA frame lies:
/// the "bright region" the scrim is solved against.
fn bright_luma(img: &RgbaImage) -> f32 {
    let mut hist = [0u32; 256];
    for p in img.pixels() {
        let [b, g, r, _] = p.0;
        let l = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) as usize;
        hist[l.min(255)] += 1;
    }
    let total: u32 = hist.iter().sum();
    let mut seen = 0;
    for (i, n) in hist.iter().enumerate() {
        seen += n;
        if seen as f32 >= total as f32 * 0.95 {
            return linear(i as f32 / 255.0);
        }
    }
    1.0
}

fn linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// The least scrim coverage, from `floor` up, at which `text` reads at 7:1
/// over a backdrop whose bright region is `bright` (linear luminance),
/// painted with `scrim` (both linear luminances of opaque colours).
pub fn scrim_alpha(bright: f32, scrim: f32, text: f32, floor: f32) -> f32 {
    let contrast = |a: f32, b: f32| (a.max(b) + 0.05) / (a.min(b) + 0.05);
    let mut alpha = floor.clamp(0.0, 1.0);
    while alpha < 0.97 {
        let under = scrim * alpha + bright * (1.0 - alpha);
        if contrast(text, under) >= 7.0 {
            break;
        }
        alpha += 0.01;
    }
    alpha
}

// ------------------------------------------------------------ clips and the cache

/// One background, decoded: every frame in one image, with timings.
pub struct Clip {
    pub image: Arc<RenderImage>,
    delays: Vec<f32>,
    total: f32,
    /// Size of a frame, in its own pixels.
    pub size: (f32, f32),
    /// Linear luminance of the brightest 5% (see [`bright_luma`]).
    pub bright: f32,
}

impl Clip {
    fn new(frames: Frames) -> Self {
        let bright =
            frames.images.iter().step_by((frames.images.len() / 8).max(1)).map(bright_luma).fold(0.0, f32::max);
        let size = frames.images.first().map_or((1.0, 1.0), |i| (i.width() as f32, i.height() as f32));
        let total = frames.delays.iter().sum::<f32>().max(MIN_DELAY);
        let image = Arc::new(RenderImage::new(frames.images.into_iter().map(image::Frame::new).collect::<Vec<_>>()));
        Clip { image, delays: frames.delays, total, size, bright }
    }

    pub fn animated(&self) -> bool {
        self.delays.len() > 1
    }

    /// The frame on screen `t` seconds into the loop, and how long until the next.
    fn frame_at(&self, t: f32) -> (usize, f32) {
        let mut t = t.rem_euclid(self.total);
        for (i, d) in self.delays.iter().enumerate() {
            if t < *d {
                return (i, d - t);
            }
            t -= d;
        }
        (0, self.delays.first().copied().unwrap_or(1.0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// Full size, for home.
    Hero,
    /// The chat frost, at this blur.
    Frost(u32),
}

#[derive(Clone, PartialEq, Eq)]
struct Key {
    path: PathBuf,
    variant: Variant,
    target: (u32, u32),
    /// Decode every frame, or only the first.
    animate: bool,
}

type Slot = Arc<Mutex<Option<Arc<Clip>>>>;
struct Entry {
    key: Key,
    slot: Slot,
    /// The previous size's clip, shown until this one lands.
    fallback: Option<Arc<Clip>>,
}

static CACHE: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

/// The device-pixel size a variant is decoded at, in 64 px steps (the frost
/// steps coarser: it is a quarter size and blurred).
fn step(target: (f32, f32), variant: Variant) -> (u32, u32) {
    let unit = if matches!(variant, Variant::Frost(_)) { 256.0 } else { 64.0 };
    let s = |v: f32| ((v.max(unit) / unit).ceil() * unit).min(7680.0) as u32;
    (s(target.0), s(target.1))
}

/// The clip for `path` in `variant` at `target` device pixels, or the
/// previous size's while it decodes (`None` before anything is ready; the
/// window refreshes when it lands).
pub fn clip(path: &Path, variant: Variant, target: (f32, f32), animate: bool, cx: &mut App) -> Option<Arc<Clip>> {
    let key = Key { path: path.to_path_buf(), variant, target: step(target, variant), animate };
    let mut stale = None;
    {
        let mut entries = CACHE.lock().ok()?;
        if let Some(i) = entries.iter().position(|e| e.key == key) {
            let e = entries.remove(i);
            let ready = e.slot.lock().ok()?.clone();
            let fallback = e.fallback.clone();
            entries.push(e);
            return ready.or(fallback);
        }
        // Same art at another size: keep showing it until the new one lands.
        if let Some(e) = entries.iter().rev().find(|e| e.key.path == key.path && e.key.variant == key.variant) {
            stale = e.slot.lock().ok()?.clone().or_else(|| e.fallback.clone());
        }
    }
    let slot: Slot = Arc::new(Mutex::new(None));
    {
        let mut entries = CACHE.lock().ok()?;
        entries.push(Entry { key: key.clone(), slot: slot.clone(), fallback: stale.clone() });
        let mut gone = Vec::new();
        while entries.len() > 4 {
            let e = entries.remove(0);
            gone.extend(e.slot.lock().ok().and_then(|s| s.clone()).map(|c| c.image.clone()));
            gone.extend(e.fallback.map(|c| c.image.clone()));
        }
        retire(gone);
    }
    // A moving clip takes seconds to decode (a video, up to 30 s of
    // frames). With nothing to show meanwhile, its first frame stands in.
    if key.animate && stale.is_none() && kind_cached(&key.path) == Kind::Animated {
        let first = Key { animate: false, ..key.clone() };
        let work = cx.background_executor().spawn(async move { decode(&first).map(|f| Arc::new(Clip::new(f))) });
        let slot = slot.clone();
        cx.spawn(async move |cx| {
            let Some(still) = work.await else { return };
            let mut placed = false;
            if let Ok(mut entries) = CACHE.lock()
                && let Some(e) = entries.iter_mut().find(|e| Arc::ptr_eq(&e.slot, &slot))
                && e.fallback.is_none()
                && slot.lock().is_ok_and(|s| s.is_none())
            {
                e.fallback = Some(still.clone());
                placed = true;
            }
            if placed {
                cx.update(|cx| cx.refresh_windows());
            } else {
                retire(Some(still.image.clone()));
            }
        })
        .detach();
    }
    let work = cx.background_executor().spawn(async move { decode(&key).map(|f| Arc::new(Clip::new(f))) });
    cx.spawn(async move |cx| {
        let clip = work.await;
        if let Ok(mut s) = slot.lock() {
            *s = clip;
        }
        if let Ok(mut entries) = CACHE.lock()
            && let Some(e) = entries.iter_mut().find(|e| Arc::ptr_eq(&e.slot, &slot))
        {
            retire(e.fallback.take().map(|c| c.image.clone()));
        }
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
    stale
}

fn decode(key: &Key) -> Option<Frames> {
    let target = key.target;
    let look: Box<dyn Fn(RgbaImage) -> RgbaImage> = match key.variant {
        Variant::Hero => Box::new(|img| img),
        Variant::Frost(blur) => Box::new(move |img| frost_frame(img, target, FrostLook { blur: blur as f32 })),
    };
    let (budget, fps) = match key.variant {
        Variant::Hero => (HERO_BUDGET, 24),
        Variant::Frost(_) => (FROST_BUDGET, 15),
    };
    if !key.animate {
        return decode_still(&key.path, target, &*look);
    }
    let _ = fps;
    match kind_of(&key.path) {
        Kind::Animated => decode_animated(&key.path, target, budget, &*look),
        // A video's clip is its first frame (the scrim's brightness, and
        // what shows while paused); [`Backdrop`] streams the moving frames.
        Kind::Still | Kind::Video => decode_still(&key.path, target, &*look),
    }
}

/// A small first-frame thumbnail for Settings (no effect applied).
pub fn thumbnail(path: &Path, target: (f32, f32), cx: &mut App) -> Option<Arc<RenderImage>> {
    clip(path, Variant::Hero, target, false, cx).map(|c| c.image.clone())
}

/// Free every decoded clip (the background changed or was removed).
pub fn release(window: &mut Window) {
    let entries = CACHE.lock().map(|mut e| std::mem::take(&mut *e)).unwrap_or_default();
    for e in entries {
        if let Some(c) = e.slot.lock().ok().and_then(|s| s.clone()) {
            let _ = window.drop_image(c.image.clone());
        }
        if let Some(c) = e.fallback {
            let _ = window.drop_image(c.image.clone());
        }
    }
}

// ------------------------------------------------------------ the player

/// What the shell wants painted this frame.
pub struct BackdropProps {
    pub path: PathBuf,
    pub variant: Variant,
    /// The area's size in device pixels.
    pub target: (f32, f32),
    /// Advance through the frames (otherwise the first frame holds).
    pub play: bool,
    /// Fade into the canvas at the bottom (home).
    pub fade: bool,
    /// The composer, cut out of the art (home), read at paint time.
    pub composer: Option<std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>>,
    pub opacity: f32,
}

/// A video playing as it decodes. ffmpeg reads the file at its own pace
/// (`-re`), loops it (`-stream_loop -1`) and scales each frame to the size
/// asked for; the newest frame waits here for the next paint. The decoder
/// thread holds only a weak reference and stops (killing ffmpeg) within a
/// frame once the stream is dropped.
struct Stream {
    latest: Mutex<Option<RgbaImage>>,
}

/// Longest edge a streamed full-size frame is decoded at, in device pixels:
/// the window's own resolution on any common screen, so the clip shows as
/// attached, while a 5K window does not copy 60 MB a frame.
const STREAM_EDGE: f32 = 2560.0;

/// The size a variant's video is streamed at: the full-size area capped at
/// [`STREAM_EDGE`], or the frost's quarter size (it is blurred anyway).
fn stream_size(target: (u32, u32), variant: Variant) -> (u32, u32) {
    let (w, h) = match variant {
        Variant::Hero => {
            let k = (STREAM_EDGE / target.0.max(target.1).max(1) as f32).min(1.0);
            ((target.0 as f32 * k) as u32, (target.1 as f32 * k) as u32)
        }
        Variant::Frost(_) => (target.0 / 4, target.1 / 4),
    };
    (w.max(64) & !1, h.max(64) & !1)
}

fn stream_video(
    path: &Path,
    size: (u32, u32),
    fps: u32,
    look: Box<dyn Fn(RgbaImage) -> RgbaImage + Send>,
) -> Option<Arc<Stream>> {
    use std::io::Read as _;
    let ffmpeg = ffmpeg()?;
    let (w, h) = size;
    let filter = format!("fps={fps},scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}");
    let mut cmd = std::process::Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-re", "-stream_loop", "-1", "-i"])
        .arg(path)
        .args(["-an", "-vf", &filter, "-f", "rawvideo", "-pix_fmt", "bgra", "-"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let stream = Arc::new(Stream { latest: Mutex::new(None) });
    let weak: Weak<Stream> = Arc::downgrade(&stream);
    let frame_len = w as usize * h as usize * 4;
    let spawned = std::thread::Builder::new().name("dove-wallpaper-video".into()).spawn(move || {
        loop {
            let mut buf = vec![0u8; frame_len];
            if stdout.read_exact(&mut buf).is_err() {
                break;
            }
            let Some(img) = RgbaImage::from_raw(w, h, buf) else { break };
            let img = look(img);
            let Some(stream) = weak.upgrade() else { break };
            if let Ok(mut latest) = stream.latest.lock() {
                *latest = Some(img);
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    });
    spawned.ok().map(|_| stream)
}

/// Paints a [`Clip`], advancing on each frame's own delay, or a video's
/// [`Stream`]. The next frame is scheduled by a timer, never by a
/// display-rate loop, so a 10 fps GIF wakes the app 10 times a second.
pub struct Backdrop {
    props: Option<BackdropProps>,
    clock: f32,
    last: Option<Instant>,
    timer: Option<gpui::Task<()>>,
    /// The playing video, and what it was started for.
    stream: Option<(StreamKey, Arc<Stream>)>,
    /// The video frame on screen, uploaded once.
    live: Option<Arc<RenderImage>>,
}

type StreamKey = (PathBuf, Variant, (u32, u32));

impl Backdrop {
    pub fn new(_: &mut Window, _: &mut Context<Self>) -> Self {
        Backdrop { props: None, clock: 0.0, last: None, timer: None, stream: None, live: None }
    }

    /// Take this frame's props; true when there is art to paint.
    pub fn show(&mut self, props: BackdropProps, cx: &mut App) -> bool {
        let animate = props.play || matches!(props.variant, Variant::Frost(_));
        let ready = clip(&props.path, props.variant, props.target, animate, cx).is_some();
        self.props = Some(props);
        ready
    }

    fn tick(&mut self, wait: f32, cx: &mut Context<Self>) {
        let wait = Duration::from_secs_f32(wait.max(MIN_DELAY));
        self.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        }));
    }
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        retire(self.live.take());
    }
}

impl Render for Backdrop {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        drop_retired(window);
        let Some(p) = self.props.as_ref() else { return div() };
        let animate = p.play || matches!(p.variant, Variant::Frost(_));
        let Some(clip) = clip(&p.path, p.variant, p.target, animate, cx) else { return div() };
        let (fade, composer, opacity) = (p.fade, p.composer.clone(), p.opacity);
        let playing = p.play && crate::motion::media_motion(cx);

        if kind_cached(&p.path) == Kind::Video {
            let target = step(p.target, p.variant);
            let key: StreamKey = (p.path.clone(), p.variant, stream_size(target, p.variant));
            let fps = if matches!(p.variant, Variant::Frost(_)) { 15 } else { 30 };
            if !playing {
                // Paused: ffmpeg stops; the last frame (or the first) holds.
                self.stream = None;
            } else if self.stream.as_ref().is_none_or(|(k, _)| *k != key) {
                let look: Box<dyn Fn(RgbaImage) -> RgbaImage + Send> = match p.variant {
                    Variant::Hero => Box::new(|img| img),
                    Variant::Frost(blur) => {
                        Box::new(move |img| frost_frame(img, target, FrostLook { blur: blur as f32 }))
                    }
                };
                let size = key.2;
                self.stream = stream_video(&p.path, size, fps, look).map(|s| (key, s));
            }
            let fresh = self.stream.as_ref().and_then(|(_, s)| s.latest.lock().ok().and_then(|mut l| l.take()));
            if let Some(frame) = fresh {
                let image = Arc::new(RenderImage::new(vec![image::Frame::new(frame)]));
                if let Some(old) = self.live.replace(image) {
                    let _ = window.drop_image(old);
                }
            }
            if self.stream.is_some() {
                self.tick(1.0 / fps as f32, cx);
            } else {
                self.timer = None;
            }
            let (image, src) = match &self.live {
                Some(live) => {
                    let s = live.size(0);
                    (live.clone(), (s.width.0 as f32, s.height.0 as f32))
                }
                None => (clip.image.clone(), clip.size),
            };
            return paint_art(image, 0, src, fade, composer, opacity);
        }

        let now = Instant::now();
        let running = playing && clip.animated();
        if running {
            if let Some(last) = self.last {
                self.clock += now.duration_since(last).as_secs_f32().min(0.5);
            }
            self.last = Some(now);
        } else {
            self.last = None;
        }
        let (frame, wait) = clip.frame_at(self.clock);
        if running {
            self.tick(wait, cx);
        } else {
            self.timer = None;
        }
        paint_art(clip.image.clone(), frame, clip.size, fade, composer, opacity)
    }
}

/// One frame of `image` covering the area, faded and cut around the
/// composer as asked.
fn paint_art(
    image: Arc<RenderImage>,
    frame: usize,
    src: (f32, f32),
    fade: bool,
    composer: Option<std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>>,
    opacity: f32,
) -> gpui::Div {
    div().size_full().opacity(opacity).child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let composer = composer.as_ref().and_then(|c| c.get());
                let Some((ox, oy, k)) = placement(bounds, src, Motion::STILL) else { return };
                let full = Bounds::new(point(px(ox), px(oy)), size(px(src.0 * k), px(src.1 * k)));
                let mask = (fade || composer.is_some()).then(|| alpha_mask(bounds, composer, composer.is_some(), fade));
                let _ = window.paint_image_fitted_masked(bounds, full, Corners::default(), image, frame, false, mask);
            },
        )
        .size_full(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small GIF keeps its own pixels (cropped to the area's shape)
    /// instead of being blown up to the window, so all its frames fit.
    #[test]
    fn small_frames_keep_their_resolution() {
        let gif = RgbaImage::new(400, 225);
        let out = cover_within(&gif, (2400, 1300));
        assert!(out.width() <= 400 && out.height() <= 225, "{:?}", out.dimensions());
        let shape = out.width() as f32 / out.height() as f32;
        assert!((shape - 2400.0 / 1300.0).abs() < 0.02, "{shape}");
        // A big source still comes down to the area.
        assert_eq!(cover_within(&RgbaImage::new(3840, 2160), (1920, 1080)).dimensions(), (1920, 1080));
        // 301 frames of it fit the full-size budget whole.
        assert!(301 * out.as_raw().len() < HERO_BUDGET);
    }

    #[test]
    fn streamed_video_is_capped_and_even() {
        let (w, h) = stream_size((3840, 2112), Variant::Hero);
        assert!(w <= 2560 && h <= 2560 && w % 2 == 0 && h % 2 == 0, "{w}x{h}");
        assert_eq!(stream_size((1280, 704), Variant::Hero), (1280, 704));
        assert_eq!(stream_size((2560, 1536), Variant::Frost(80)), (640, 384));
    }

    /// A video's still (thumbnails, and the frame shown when playback is
    /// off)
    /// has no byte budget: it used to overflow `budget * 2` and abort the
    /// app. It also reads one frame, not the whole clip.
    #[test]
    fn video_still_decodes_one_frame_without_overflow() {
        let Some(ffmpeg) = ffmpeg() else { return };
        let dir = std::env::temp_dir().join(format!("dove-wm-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let clip = dir.join("clip.mp4");
        let made = std::process::Command::new(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x180:rate=24:duration=2",
            ])
            .args(["-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            return;
        }
        let still = decode_still(&clip, (640, 360), &|i| i).expect("a first frame");
        assert_eq!(still.images.len(), 1);
        assert_eq!(still.images[0].dimensions(), (640, 360));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scrim_rises_until_text_reads() {
        // White art under a near-black scrim with near-white text.
        let alpha = scrim_alpha(1.0, linear(0.08), linear(0.92), 0.5);
        assert!(alpha > 0.5 && alpha < 0.97, "{alpha}");
        // Dark art needs only the floor.
        assert_eq!(scrim_alpha(0.01, linear(0.08), linear(0.92), 0.78), 0.78);
    }

    #[test]
    fn frames_thin_out_to_the_budget_and_keep_their_time() {
        let img = RgbaImage::new(10, 10);
        let f = Frames { images: vec![img; 10], delays: vec![0.1; 10] };
        let fitted = f.fit(400 * 3);
        assert!(fitted.bytes() <= 400 * 4);
        assert!((fitted.delays.iter().sum::<f32>() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn clips_loop_on_their_delays() {
        let f = Frames { images: vec![RgbaImage::new(2, 2); 3], delays: vec![0.1, 0.2, 0.3] };
        let c = Clip::new(f);
        assert_eq!(c.frame_at(0.05).0, 0);
        assert_eq!(c.frame_at(0.15).0, 1);
        assert_eq!(c.frame_at(0.45).0, 2);
        assert_eq!(c.frame_at(0.65).0, 0);
    }

    #[test]
    fn frost_shrinks_to_a_quarter_and_removes_detail() {
        let mut img = RgbaImage::new(256, 128);
        for (x, _, p) in img.enumerate_pixels_mut() {
            p.0 = if x % 2 == 0 { [255, 255, 255, 255] } else { [0, 0, 0, 255] };
        }
        let out = frost_frame(img, (256, 128), FrostLook { blur: FrostLook::DEFAULT_BLUR });
        assert_eq!(out.dimensions(), (64, 32));
        let (min, max) = out.pixels().fold((255u8, 0u8), |(lo, hi), p| (lo.min(p.0[0]), hi.max(p.0[0])));
        assert!(max - min < 24, "stripes survived: {min}..{max}");
    }

    #[test]
    fn plain_png_is_a_still() {
        assert_eq!(kind_of(Path::new("no-such-file.png")), Kind::Still);
        assert_eq!(kind_of(Path::new("clip.MP4")), Kind::Video);
    }
}
