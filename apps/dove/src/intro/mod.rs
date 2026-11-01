//! First-launch intro: a flight through space. Stars streak past as the
//! window accelerates, slow again, and "Keplorayx" arrives out of the middle
//! before the app fades in. The iPhone app's onboarding flies the same way.
//!
//! Drawn natively, frame by frame. Frames are only requested while the intro
//! runs, and the overlay removes itself when done.

use std::time::Instant;

use gpui::{
    Bounds, Context, EventEmitter, FocusHandle, FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent,
    MouseButton, ParentElement as _, PathBuilder, Pixels, Render, Styled as _, Window, canvas, div, hsla, point, px,
};

/// The overlay finished or was skipped; the owner should drop it.
pub struct IntroFinished;

/// How long a skip takes to fade out.
const SKIP_FADE: f32 = 0.25;
/// The timeline, in seconds.
const STARS_IN: (f32, f32) = (0.0, 0.5);
const WARP: (f32, f32) = (0.3, 3.0);
const WORD_IN: (f32, f32) = (2.3, 3.1);
const FADE_OUT: (f32, f32) = (4.3, 5.0);

const STARS: u32 = 420;
/// Brightness steps. Stars of a step share one path, so a frame costs a few
/// draws, not hundreds.
const BANDS: usize = 4;

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
        // It comes up from the distance and keeps drifting toward the viewer.
        let arrive = smooth(span(t, WORD_IN));
        let size = 26.0 + 22.0 * arrive + (t - WORD_IN.1).max(0.0) * 1.2;

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
            .child(canvas(|_, _, _| (), move |bounds, _, window, _| paint_stars(t, bounds, window)).size_full())
            .child(
                div().absolute().inset_0().flex().items_center().justify_center().child(
                    div()
                        .opacity(arrive)
                        .text_size(px(size))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hsla(0.0, 0.0, 1.0, 1.0))
                        .child("Keplorayx"),
                ),
            )
    }
}

// ---------- math ----------

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

fn smooth(x: f32) -> f32 {
    let x = clamp01(x);
    x * x * (3.0 - 2.0 * x)
}

fn span(t: f32, (t0, t1): (f32, f32)) -> f32 {
    (t - t0) / (t1 - t0)
}

/// How far the window has flown by `t`, in depth units: a slow drift, plus the
/// long push of the warp, which rises and settles on its own.
fn travel(t: f32) -> f32 {
    0.06 * t + 3.2 * smooth(span(t, WARP))
}

/// A stable pseudo-random fraction for star `i`, channel `k`.
fn hash(i: u32, k: u32) -> f32 {
    let mut h = i.wrapping_mul(0x9E37_79B9) ^ k.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 65535.0
}

/// A star's position across the screen's plane, off the axis so nothing sits
/// still in the middle.
fn star_xy(i: u32) -> (f32, f32) {
    let angle = hash(i, 1) * std::f32::consts::TAU;
    let radius = 0.08 + hash(i, 2) * 1.3;
    (angle.cos() * radius, angle.sin() * radius)
}

/// Depth in (0, 1]; it falls with travel and wraps back to the far plane.
fn depth(i: u32, flown: f32) -> f32 {
    let z = (hash(i, 3) - flown).rem_euclid(1.0);
    z.max(0.015)
}

// ---------- painting ----------

fn paint_stars(t: f32, bounds: Bounds<Pixels>, window: &mut Window) {
    let (w, h): (f32, f32) = (bounds.size.width.into(), bounds.size.height.into());
    let (ox, oy): (f32, f32) = (bounds.origin.x.into(), bounds.origin.y.into());
    let (cx, cy) = (ox + w / 2.0, oy + h / 2.0);
    let scale = w.max(h) * 0.55;
    let fade = smooth(span(t, STARS_IN));
    let flown = travel(t);
    // A streak covers the depth crossed in the last moment, and a still star
    // is a dot, not nothing.
    let trail = (travel(t) - travel(t - 0.09)).clamp(0.002, 0.5);

    let mut bands: Vec<PathBuilder> = (0..BANDS).map(|_| PathBuilder::stroke(px(1.3))).collect();
    let mut used = [false; BANDS];
    for i in 0..STARS {
        let (x, y) = star_xy(i);
        let z = depth(i, flown);
        let head = (cx + x / z * scale, cy + y / z * scale);
        if head.0 < ox - 60.0 || head.0 > ox + w + 60.0 || head.1 < oy - 60.0 || head.1 > oy + h + 60.0 {
            continue;
        }
        let far = (z + trail).min(1.0);
        let tail = (cx + x / far * scale, cy + y / far * scale);
        let band = (((1.0 - z) * BANDS as f32) as usize).min(BANDS - 1);
        bands[band].move_to(point(px(tail.0), px(tail.1)));
        bands[band].line_to(point(px(head.0), px(head.1)));
        used[band] = true;
    }
    for (k, builder) in bands.into_iter().enumerate() {
        if !used[k] {
            continue;
        }
        let near = (k as f32 + 1.0) / BANDS as f32;
        let color: Hsla = hsla(0.59, 0.5, 0.92, fade * (0.25 + 0.75 * near));
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
    }
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
        assert!(STARS_IN.1 <= WARP.1 && WARP.0 < WORD_IN.1 && WORD_IN.1 < FADE_OUT.0);
    }

    #[test]
    fn the_flight_only_moves_forward() {
        let mut last = travel(0.0);
        for k in 1..100 {
            let now = travel(k as f32 * 0.05);
            assert!(now > last);
            last = now;
        }
    }

    #[test]
    fn stars_stay_in_front_of_the_viewer_and_off_the_axis() {
        for i in 0..STARS {
            let (x, y) = star_xy(i);
            assert!(x * x + y * y >= 0.07 * 0.07, "star {i}");
            for k in 0..50 {
                let z = depth(i, k as f32 * 0.137);
                assert!(z > 0.0 && z <= 1.0, "star {i}: {z}");
            }
        }
    }
}
