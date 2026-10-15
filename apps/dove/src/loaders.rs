//! Zeron's working loaders (MIT): the 3×3 gradient matrix spinner that sits
//! beside the in-flow "working" word, and the 2×3 glyph spinner in an
//! accent palette for compact status slots (a running subagent chip).
//!
//! Each cell pulses once per period on Zeron's `gspin` curve: full, fading
//! to a dim rest by 45%, holding, then flaring back. The per-cell phase
//! decides which way the wave travels.

use std::time::Duration;

use gpui::{Animation, AnimationExt as _, ElementId, Hsla, IntoElement, ParentElement as _, SharedString, Styled as _, div, px};

use crate::theme::Theme;

/// One pulse per cell (Zeron's `GRADIENT_SPIN`).
const PERIOD: Duration = Duration::from_millis(750);
/// Where a cell rests between pulses.
const DIM: f32 = 0.1;
/// The sunrise rows of the gradient spinner, top to bottom.
const ROW_TINTS: [u32; 3] = [0xB6D3EF, 0xEDB185, 0xF888A0];

/// Zeron's `gspin_opacity`: full at 0, dim by 0.45, back to full at 1.
pub fn gspin_opacity(t: f32, dim: f32) -> f32 {
    let t = t.rem_euclid(1.0);
    if t < 0.45 {
        1.0 + (dim - 1.0) * (t / 0.45)
    } else if t < 0.92 {
        dim
    } else {
        dim + (1.0 - dim) * ((t - 0.92) / 0.08)
    }
}

fn cell(key: SharedString, size: f32, tint: Hsla, phase: f32) -> impl IntoElement {
    div().size(px(size)).flex_none().rounded(px(size / 2.0)).bg(tint).with_animation(
        key,
        Animation::new(PERIOD).repeat(),
        move |el, delta| el.opacity(gspin_opacity(delta + phase, DIM)),
    )
}

/// The working indicator: a 3×3 grid tinted per row from the sunrise
/// gradient. The pulse enters at the bottom edge and converges on the
/// top-centre cell, so the wave reads as travelling upward.
pub fn gradient_spinner(id: impl Into<ElementId>, cell_px: f32) -> impl IntoElement {
    let key = format!("{:?}", id.into());
    let center = 1.0f32;
    let max = 2.0 + center;
    div().flex().flex_none().flex_col().gap(px(cell_px / 2.0)).children((0..3).map(|row| {
        let tint: Hsla = gpui::rgb(ROW_TINTS[row]).into();
        let key = key.clone();
        div().flex().gap(px(cell_px / 2.0)).children((0..3).map(move |col| {
            let d = 2.0 - row as f32 + (col as f32 - center).abs();
            cell(SharedString::from(format!("{key}-g{row}{col}")), cell_px, tint, d / (max + 1.0))
        }))
    }))
}

/// Zeron's glyph palette: the accent lifted, the accent, the accent deepened.
pub fn glyph_rows(t: &Theme) -> [Hsla; 3] {
    let (mut light, mid, mut deep) = (t.accent, t.accent, t.accent_strong);
    if t.appearance.is_dark() {
        light.l = (light.l + 0.14).min(0.90);
        light.s *= 0.72;
    } else {
        light.l = (light.l + 0.11).min(0.76);
        light.s *= 0.78;
        deep.l = (deep.l - 0.09).max(0.22);
    }
    [light, mid, deep]
}

/// The 2×3 glyph spinner: brightness snakes clockwise around the ring.
pub fn glyph_spinner(id: impl Into<ElementId>, cell_px: f32, rows: [Hsla; 3]) -> impl IntoElement {
    // Clockwise ring position of each (row, col), top-left first.
    const RING: [[usize; 2]; 3] = [[0, 1], [5, 2], [4, 3]];
    let key = format!("{:?}", id.into());
    div()
        .w(px(cell_px * 2.5))
        .h(px(cell_px * 4.0))
        .flex()
        .flex_none()
        .flex_col()
        .gap(px(cell_px / 2.0))
        .children((0..3).map(|row| {
            let key = key.clone();
            div().flex().gap(px(cell_px / 2.0)).children((0..2).map(move |col| {
                cell(
                    SharedString::from(format!("{key}-m{row}{col}")),
                    cell_px,
                    rows[row],
                    RING[row][col] as f32 / 6.0,
                )
            }))
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gspin_pulses_between_dim_and_full() {
        assert!((gspin_opacity(0.0, DIM) - 1.0).abs() < 1e-6);
        assert!((gspin_opacity(0.45, DIM) - DIM).abs() < 1e-6);
        assert!((gspin_opacity(0.7, DIM) - DIM).abs() < 1e-6);
        assert!((gspin_opacity(1.0, DIM) - 1.0).abs() < 1e-6);
        for step in 0..200 {
            let v = gspin_opacity(step as f32 / 100.0, DIM);
            assert!((DIM - 1e-6..=1.0 + 1e-6).contains(&v), "{step} -> {v}");
        }
    }
}
