//! Dove's mark as a living thing: a small pixel dove that perches on the
//! new-thread card. It idles between a few poses (a blink, a peck, a wing
//! ruffle, a head turn) on a slow, uneven clock, and holds still when
//! motion is reduced or the window is in the background. Drawn as quads,
//! so it is crisp at any scale and costs nothing between poses.

use std::time::Duration;

use gpui::{
    Bounds, Context, Hsla, IntoElement, ParentElement as _, Render, Styled as _, Window, canvas, div, fill, point, px,
    size,
};

use crate::theme::{Theme, mix};

/// One pose, top row first. `W` body, `S` wing shade, `E` eye, `B` beak,
/// `F` feet, `.` empty.
type Pose = [&'static str; 11];

const IDLE: Pose = [
    ".....WWW........",
    "....WWWWW.......",
    "...BWEWWW.......",
    "....WWWWW.......",
    ".....WWWWW......",
    "....WWWWWWWSS...",
    "....WWWWWWSSSSS.",
    ".....WWWWWSSS...",
    "......WWWWW.....",
    ".......F..F.....",
    "................",
];

const BLINK: Pose = [
    ".....WWW........",
    "....WWWWW.......",
    "...BWSWWW.......",
    "....WWWWW.......",
    ".....WWWWW......",
    "....WWWWWWWSS...",
    "....WWWWWWSSSSS.",
    ".....WWWWWSSS...",
    "......WWWWW.....",
    ".......F..F.....",
    "................",
];

const PECK: Pose = [
    "................",
    "................",
    ".....WWW........",
    "....WWWWW.......",
    "...BWEWWWW......",
    "....WWWWWWWSS...",
    "....WWWWWWSSSSS.",
    ".....WWWWWSSS...",
    "......WWWWW.....",
    ".......F..F.....",
    "................",
];

const RUFFLE: Pose = [
    ".....WWW........",
    "....WWWWW..SS...",
    "...BWEWWW.SSS...",
    "....WWWWWSSS....",
    ".....WWWWSS.....",
    "....WWWWWWWSS...",
    "....WWWWWWSSSSS.",
    ".....WWWWWSSS...",
    "......WWWWW.....",
    ".......F..F.....",
    "................",
];

const LOOK: Pose = [
    "........WWW.....",
    ".......WWWWW....",
    ".......WWWEWB...",
    ".......WWWWW....",
    "......WWWWW.....",
    "....WWWWWWWSS...",
    "....WWWWWWSSSSS.",
    ".....WWWWWSSS...",
    "......WWWWW.....",
    ".......F..F.....",
    "................",
];

/// The idle loop: which pose, for how long (ms). Uneven on purpose, so it
/// reads as a bird rather than a metronome.
const SCRIPT: [(Pose, u64); 12] = [
    (IDLE, 3200),
    (BLINK, 140),
    (IDLE, 2600),
    (PECK, 260),
    (IDLE, 180),
    (PECK, 240),
    (IDLE, 3400),
    (LOOK, 1800),
    (IDLE, 2200),
    (BLINK, 140),
    (IDLE, 900),
    (RUFFLE, 420),
];

pub const COLS: usize = 16;
pub const ROWS: usize = 11;

pub struct Dove {
    step: usize,
    /// Pixel size in logical px.
    cell: f32,
    timer: Option<gpui::Task<()>>,
}

impl Dove {
    pub fn new(cell: f32, _: &mut Window, _: &mut Context<Self>) -> Self {
        Dove { step: 0, cell, timer: None }
    }

    pub fn size(&self) -> (f32, f32) {
        (COLS as f32 * self.cell, ROWS as f32 * self.cell)
    }
}

/// The dove's colours on this theme: body, wing, eye, beak and feet.
fn palette(t: &Theme) -> [Hsla; 4] {
    let body = if t.appearance.is_dark() { t.text.opacity(0.94) } else { mix(t.text, t.bg, 0.86) };
    let shade = if t.appearance.is_dark() { mix(t.text, t.bg, 0.42) } else { mix(t.text, t.bg, 0.55) };
    let eye = if t.appearance.is_dark() { t.bg } else { t.text };
    [body, shade, eye, t.accent]
}

pub fn paint(pose: &Pose, origin: gpui::Point<gpui::Pixels>, cell: f32, t: &Theme, window: &mut Window) {
    let [body, shade, eye, accent] = palette(t);
    for (y, row) in pose.iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            let color = match c {
                b'W' => body,
                b'S' => shade,
                b'E' => eye,
                b'B' | b'F' => accent,
                _ => continue,
            };
            let at = point(origin.x + px(x as f32 * cell), origin.y + px(y as f32 * cell));
            window.paint_quad(fill(Bounds::new(at, size(px(cell), px(cell))), color));
        }
    }
}

impl Render for Dove {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (pose, hold) = SCRIPT[self.step % SCRIPT.len()];
        let alive = crate::motion::decorative_motion(cx);
        if alive {
            if self.timer.is_none() {
                self.timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(hold)).await;
                    let _ = this.update(cx, |dove, cx| {
                        dove.step = (dove.step + 1) % SCRIPT.len();
                        dove.timer = None;
                        cx.notify();
                    });
                }));
            }
        } else {
            // Still: the resting pose, no wakeups.
            self.timer = None;
            self.step = 0;
        }
        let t = Theme::of(cx).clone();
        let cell = self.cell;
        let (w, h) = self.size();
        div().w(px(w)).h(px(h)).child(
            canvas(|_, _, _| {}, move |bounds, _, window, _| paint(&pose, bounds.origin, cell, &t, window)).size_full(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pose_is_a_full_grid_with_a_beak_and_feet() {
        for (pose, ms) in SCRIPT {
            assert!(ms >= 100);
            assert_eq!(pose.len(), ROWS);
            assert!(pose.iter().all(|r| r.len() == COLS), "{pose:?}");
            assert!(pose.iter().any(|r| r.contains('B')));
            assert_eq!(pose.iter().map(|r| r.matches('F').count()).sum::<usize>(), 2);
        }
    }
}
