//! First-launch intro: a black screen, a silver shimmer running around the
//! window, then "Dove" handwritten in the center before the app fades in.
//!
//! Drawn natively, frame by frame, from geometry baked by
//! media/intro/src/build-assets.mjs (see `data`). The drawing mirrors
//! media/intro/src/scene.html, which renders the marketing video; keep the two
//! in step. Frames are only requested while the intro runs, and the overlay
//! removes itself when done.

#[rustfmt::skip]
mod data;

use std::f32::consts::PI;
use std::time::Instant;

use gpui::{
    Bounds, Context, EventEmitter, FocusHandle, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ParentElement as _, PathBuilder, Pixels, Point, Render, Rgba, Styled as _, Window, canvas, div, hsla, point, px,
};

use data::*;

/// The overlay finished or was skipped; the owner should drop it.
pub struct IntroFinished;

/// How long a skip takes to fade out.
const SKIP_FADE: f32 = 0.2;

pub struct Intro {
    /// Set on the first rendered frame, so window creation does not eat into
    /// the timeline.
    clock: Option<Instant>,
    skipped_at: Option<f32>,
    focus: FocusHandle,
    finished: bool,
}

impl EventEmitter<IntroFinished> for Intro {}

impl Intro {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self { clock: None, skipped_at: None, focus, finished: false }
    }

    /// Seconds into the timeline.
    fn tick(&mut self) -> f32 {
        self.clock.get_or_insert_with(Instant::now).elapsed().as_secs_f32()
    }

    fn skip(&mut self, cx: &mut Context<Self>) {
        if self.skipped_at.is_none() {
            self.skipped_at = Some(self.clock.map_or(0.0, |c| c.elapsed().as_secs_f32()));
            cx.notify();
        }
    }

    fn opacity(&self, t: f32) -> f32 {
        let natural = 1.0 - smooth(span(t, FADE_OUT));
        let skipped = self.skipped_at.map_or(1.0, |s| 1.0 - smooth((t - s) / SKIP_FADE));
        natural.min(skipped)
    }
}

impl Render for Intro {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("Intro");
        let t = self.tick();
        let opacity = self.opacity(t);
        if opacity <= 0.0 {
            if !self.finished {
                self.finished = true;
                cx.emit(IntroFinished);
            }
        } else {
            window.request_animation_frame();
        }

        div()
            .id("intro")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .track_focus(&self.focus)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.skip(cx)))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if matches!(e.keystroke.key.as_str(), "escape" | "enter" | "space") {
                    this.skip(cx);
                }
            }))
            .opacity(opacity)
            .bg(hsla(0.0, 0.0, 0.0, 1.0))
            .child(canvas(|_, _, _| (), move |bounds, _, window, _| paint_scene(t, bounds, window)).size_full())
    }
}

// ---------- math ----------

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

fn lerp(a: f32, b: f32, k: f32) -> f32 {
    a + (b - a) * k
}

fn smooth(x: f32) -> f32 {
    let x = clamp01(x);
    x * x * (3.0 - 2.0 * x)
}

fn ease_in_out_sine(x: f32) -> f32 {
    -((PI * clamp01(x)).cos() - 1.0) / 2.0
}

fn span(t: f32, (t0, t1): (f32, f32)) -> f32 {
    (t - t0) / (t1 - t0)
}

/// An offset from scene coordinates to window pixels.
#[derive(Clone, Copy)]
struct Affine([f32; 2]);

impl Affine {
    fn translate(x: f32, y: f32) -> Self {
        Self([x, y])
    }

    fn apply(&self, x: f32, y: f32) -> Point<Pixels> {
        point(px(x + self.0[0]), px(y + self.0[1]))
    }
}

// ---------- painting helpers ----------

fn white(a: f32) -> Hsla {
    hsla(0.0, 0.0, 1.0, a.clamp(0.0, 1.0))
}

fn paint(window: &mut Window, builder: PathBuilder, color: Hsla) {
    if color.a <= 0.001 {
        return;
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn trace(b: &mut PathBuilder, start: [f32; 2], curves: &[[f32; 6]], m: &Affine) {
    b.move_to(m.apply(start[0], start[1]));
    for c in curves {
        b.cubic_bezier_to(m.apply(c[4], c[5]), m.apply(c[0], c[1]), m.apply(c[2], c[3]));
    }
    b.close();
}

fn circle(b: &mut PathBuilder, x: f32, y: f32, r: f32) {
    let k = 0.552_284_8 * r;
    let m = Affine::translate(x, y);
    trace(
        b,
        [r, 0.0],
        &[[r, k, k, r, 0.0, r], [-k, r, -r, k, -r, 0.0], [-r, -k, -k, -r, 0.0, -r], [k, -r, r, -k, r, 0.0]],
        &m,
    );
}

/// Concentric discs; the same soft glow scene.html draws.
fn glow(window: &mut Window, x: f32, y: f32, radius: f32, alpha: f32) {
    const RINGS: usize = 6;
    if alpha <= 0.001 {
        return;
    }
    for k in 1..=RINGS {
        let mut b = PathBuilder::fill();
        circle(&mut b, x, y, radius * k as f32 / RINGS as f32);
        paint(window, b, white(alpha / RINGS as f32));
    }
}

// ---------- scene ----------

struct Frame {
    w: f32,
    h: f32,
    origin: Affine,
}

fn paint_scene(t: f32, bounds: Bounds<Pixels>, window: &mut Window) {
    let f = Frame {
        w: bounds.size.width.into(),
        h: bounds.size.height.into(),
        origin: Affine::translate(bounds.origin.x.into(), bounds.origin.y.into()),
    };

    border(&f, t, window);
    for s in SHIMMERS {
        shimmer(&f, t, s, window);
    }
    wordmark(&f, t, window);
    light(&f, t, window);
}

fn screen(f: &Frame, x: f32, y: f32) -> (f32, f32) {
    let p = f.origin.apply(x, y);
    (p.x.into(), p.y.into())
}

// ---------- border + shimmer ----------

const RADIUS: f32 = 10.0;
const INSET: f32 = 0.75;
const SHIMMER_SEGMENTS: usize = 40;

fn perimeter(f: &Frame) -> f32 {
    let (w, h) = (f.w - 2.0 * INSET, f.h - 2.0 * INSET);
    2.0 * (w - 2.0 * RADIUS) + 2.0 * (h - 2.0 * RADIUS) + 2.0 * PI * RADIUS
}

/// Point at arc length `s`, clockwise from the top edge just right of the
/// top-left corner.
fn perimeter_point(f: &Frame, s: f32) -> (f32, f32) {
    let (x0, y0, w, h, r) = (INSET, INSET, f.w - 2.0 * INSET, f.h - 2.0 * INSET, RADIUS);
    let l = perimeter(f);
    let mut s = s.rem_euclid(l);
    let (top, side, arc) = (w - 2.0 * r, h - 2.0 * r, PI * r / 2.0);
    let corner = |cx: f32, cy: f32, a: f32| (cx + r * a.cos(), cy + r * a.sin());
    if s <= top {
        return (x0 + r + s, y0);
    }
    s -= top;
    if s <= arc {
        return corner(x0 + w - r, y0 + r, -PI / 2.0 + s / r);
    }
    s -= arc;
    if s <= side {
        return (x0 + w, y0 + r + s);
    }
    s -= side;
    if s <= arc {
        return corner(x0 + w - r, y0 + h - r, s / r);
    }
    s -= arc;
    if s <= top {
        return (x0 + w - r - s, y0 + h);
    }
    s -= top;
    if s <= arc {
        return corner(x0 + r, y0 + h - r, PI / 2.0 + s / r);
    }
    s -= arc;
    if s <= side {
        return (x0, y0 + h - r - s);
    }
    s -= side;
    corner(x0 + r, y0 + r, PI + s / r)
}

fn border(f: &Frame, t: f32, window: &mut Window) {
    let alpha = BORDER_ALPHA * smooth(span(t, BORDER_IN));
    if alpha <= 0.0 {
        return;
    }
    let l = perimeter(f);
    let mut b = PathBuilder::stroke(px(1.0));
    let steps = 240;
    for i in 0..=steps {
        let (x, y) = perimeter_point(f, l * i as f32 / steps as f32);
        let p = f.origin.apply(x, y);
        if i == 0 { b.move_to(p) } else { b.line_to(p) }
    }
    paint(window, b, white(alpha));
}

fn shimmer(f: &Frame, t: f32, s: &Shimmer, window: &mut Window) {
    let p = span(t, (s.t0, s.t1));
    if p <= 0.0 || p >= 1.0 {
        return;
    }
    let l = perimeter(f);
    let head = ease_in_out_sine(p) * l;
    let env = (PI * p).sin().powf(0.5) * s.alpha;
    let trail = l * 0.26;
    for (width, pass_alpha) in [(9.0, 0.22), (2.0, 1.0)] {
        for k in 0..SHIMMER_SEGMENTS {
            let a = head - trail * (1.0 - k as f32 / SHIMMER_SEGMENTS as f32);
            let bb = head - trail * (1.0 - (k + 1) as f32 / SHIMMER_SEGMENTS as f32);
            let fk = (k + 1) as f32 / SHIMMER_SEGMENTS as f32;
            let ff = fk * fk;
            let color: Hsla = Rgba {
                r: lerp(150.0, 255.0, ff).round() / 255.0,
                g: lerp(156.0, 255.0, ff).round() / 255.0,
                b: lerp(168.0, 255.0, ff).round() / 255.0,
                a: (env * pass_alpha * fk.powf(1.8)).clamp(0.0, 1.0),
            }
            .into();
            let mut path = PathBuilder::stroke(px(width));
            for j in 0..=3 {
                let (x, y) = perimeter_point(f, lerp(a, bb, j as f32 / 3.0));
                let q = f.origin.apply(x, y);
                if j == 0 { path.move_to(q) } else { path.line_to(q) }
            }
            paint(window, path, color);
        }
    }
    let (hx, hy) = perimeter_point(f, head);
    let (hx, hy) = screen(f, hx, hy);
    glow(window, hx, hy, 14.0, 0.35 * env);
    let mut b = PathBuilder::fill();
    circle(&mut b, hx, hy, 1.8);
    paint(window, b, white(env));
}

// ---------- wordmark ----------

struct WordBox {
    w: f32,
    cx: f32,
    cy: f32,
}

fn word_box(f: &Frame) -> WordBox {
    let w = (f.w * WRITE_WIDTH_FRAC).min(f.h * WRITE_MAX_HEIGHT_FRAC / WORD_ASPECT);
    WordBox { w, cx: f.w / 2.0, cy: f.h / 2.0 }
}

fn add_rings(b: &mut PathBuilder, f: &Frame, wb: &WordBox, (start, len): (u32, u32)) {
    for &(p0, n) in &RINGS[start as usize..(start + len) as usize] {
        let pts = &POINTS[p0 as usize..(p0 + n) as usize];
        for (i, [x, y]) in pts.iter().enumerate() {
            let q = f.origin.apply(wb.cx + x * wb.w, wb.cy + y * wb.w);
            if i == 0 { b.move_to(q) } else { b.line_to(q) }
        }
        b.close();
    }
}

fn wordmark(f: &Frame, t: f32, window: &mut Window) {
    let wb = word_box(f);
    for l in LETTERS {
        if t < l.t0 {
            continue;
        }
        if t >= l.t1 {
            let mut b = PathBuilder::fill();
            add_rings(&mut b, f, &wb, l.glyph);
            paint(window, b, white(1.0));
            continue;
        }
        // Finished slices share one path so their seams do not show.
        let mut done = PathBuilder::fill();
        for s in &SLICES[l.slices.0 as usize..(l.slices.0 + l.slices.1) as usize] {
            if t >= s.t1 {
                add_rings(&mut done, f, &wb, s.rings);
            } else if t >= s.t0 {
                let mut b = PathBuilder::fill();
                add_rings(&mut b, f, &wb, s.rings);
                paint(window, b, white((t - s.t0) / (s.t1 - s.t0)));
            }
        }
        paint(window, done, white(1.0));
    }
}

fn pen_at(t: f32) -> [f32; 2] {
    let pen = |l: &Letter| &PEN[l.pen.0 as usize..(l.pen.0 + l.pen.1) as usize];
    let at = |p: &[f32; 3]| [p[1], p[2]];
    for (i, l) in LETTERS.iter().enumerate() {
        let p = pen(l);
        if t < l.t0 {
            if i == 0 {
                return at(&p[0]);
            }
            let prev = &LETTERS[i - 1];
            let (a, b) = (at(pen(prev).last().unwrap()), at(&p[0]));
            let k = ease_in_out_sine((t - prev.t1) / (l.t0 - prev.t1));
            return [lerp(a[0], b[0], k), lerp(a[1], b[1], k) - 0.02 * (PI * k).sin()];
        }
        if t <= l.t1 {
            let mut j = 1;
            while j < p.len() - 1 && p[j][0] < t {
                j += 1;
            }
            let dt = p[j][0] - p[j - 1][0];
            let k = clamp01((t - p[j - 1][0]) / if dt == 0.0 { 1.0 } else { dt });
            return [lerp(p[j - 1][1], p[j][1], k), lerp(p[j - 1][2], p[j][2], k)];
        }
    }
    at(pen(&LETTERS[LETTERS.len() - 1]).last().unwrap())
}

/// The point of light that writes the word: it blooms at the pen's start,
/// follows the pen, and fades once the word is done.
fn light(f: &Frame, t: f32, window: &mut Window) {
    let wb = word_box(f);
    let (a0, w1) = (PEN_APPEAR.0, WRITE_SPAN.1);
    if t < a0 || t > w1 + 0.5 {
        return;
    }
    let [px0, py0] = pen_at(t.max(WRITE_SPAN.0));
    let (x, y) = screen(f, wb.cx + px0 * wb.w, wb.cy + py0 * wb.w);
    let a = if t < w1 { smooth(span(t, PEN_APPEAR)) } else { 1.0 - smooth((t - w1) / 0.5) };
    glow(window, x, y, 22.0, 0.32 * a);
    let mut b = PathBuilder::fill();
    circle(&mut b, x, y, 2.4);
    paint(window, b, white(a));
}

// ---------- first-launch gate ----------

/// Store key recording that this device has seen the intro.
pub const SEEN_KEY: &str = "intro.seen";

/// Whether to play the intro. `DOVE_SHOW_INTRO` forces it (development);
/// `DOVE_SKIP_INTRO` turns it off (CI, power users). Otherwise it plays once
/// per data directory, i.e. once per device.
pub fn should_show(seen: Option<&str>, force: bool, skip: bool) -> bool {
    force || (!skip && seen.is_none())
}

pub fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate() {
        assert!(should_show(None, false, false));
        assert!(!should_show(Some("1"), false, false));
        assert!(!should_show(None, false, true));
        assert!(should_show(Some("1"), true, false));
        assert!(should_show(Some("1"), true, true));
    }

    #[test]
    fn timeline_is_ordered() {
        assert!(LETTERS.windows(2).all(|w| w[0].t1 <= w[1].t0));
        for l in LETTERS {
            let slices = &SLICES[l.slices.0 as usize..(l.slices.0 + l.slices.1) as usize];
            assert!(slices.windows(2).all(|w| w[0].t0 <= w[1].t0));
            assert!(slices.iter().all(|s| s.t0 >= l.t0 && s.t1 <= l.t1 + 1e-3));
        }
        assert!(SHIMMERS.iter().all(|s| s.t0 < s.t1 && s.t1 <= WRITE_SPAN.1));
        assert!(PEN_APPEAR.1 <= WRITE_SPAN.0);
        assert!(WRITE_SPAN.1 < FADE_OUT.0);
    }

    #[test]
    fn wordmark_data_is_in_bounds() {
        for l in LETTERS {
            assert!((l.slices.0 + l.slices.1) as usize <= SLICES.len());
            assert!((l.glyph.0 + l.glyph.1) as usize <= RINGS.len());
            assert!((l.pen.0 + l.pen.1) as usize <= PEN.len());
        }
        for &(p0, n) in RINGS {
            assert!(n >= 3 && (p0 + n) as usize <= POINTS.len());
        }
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for [x, _] in POINTS {
            lo = lo.min(*x);
            hi = hi.max(*x);
        }
        assert!((lo + 0.5).abs() < 1e-3 && (hi - 0.5).abs() < 1e-3, "word is centered and 1.0 wide");
    }

    #[test]
    fn perimeter_walk_closes() {
        let f = Frame { w: 1280.0, h: 800.0, origin: Affine::translate(0.0, 0.0) };
        let l = perimeter(&f);
        let (a, b) = (perimeter_point(&f, 0.0), perimeter_point(&f, l - 0.01));
        assert!((a.0 - b.0).abs() < 0.5 && (a.1 - b.1).abs() < 0.5);
    }
}
