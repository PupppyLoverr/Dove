//! Interruptible springs. Each value keeps its velocity, so a new target
//! mid-flight (the pointer leaves halfway through opening) bends the motion
//! instead of restarting it. Integrated with semi-implicit Euler in small
//! fixed steps, so the result doesn't depend on the frame rate.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
    pub stiffness: f32,
    /// Damping ratio: 1 is critical (no overshoot), below it bounces.
    pub damping: f32,
}

/// Integration step.
const STEP: f32 = 1.0 / 240.0;

impl Spring {
    pub fn new(value: f32, stiffness: f32, damping: f32) -> Self {
        Spring { value, velocity: 0.0, target: value, stiffness, damping }
    }

    pub fn advance(&mut self, dt: f32) {
        let mut left = dt.min(0.1);
        let c = 2.0 * self.damping * self.stiffness.sqrt();
        while left > 0.0 {
            let h = left.min(STEP);
            let force = -self.stiffness * (self.value - self.target) - c * self.velocity;
            self.velocity += force * h;
            self.value += self.velocity * h;
            left -= h;
        }
    }

    pub fn settled(&self) -> bool {
        (self.value - self.target).abs() < 0.05 && self.velocity.abs() < 0.5
    }

    /// Jumps to the target (reduced motion, or a first placement).
    pub fn snap(&mut self) {
        self.value = self.target;
        self.velocity = 0.0;
    }
}

/// A group of springs advanced together from the last frame's time.
#[derive(Clone, Debug)]
pub struct Springs<const N: usize> {
    pub s: [Spring; N],
    last: Option<Instant>,
}

impl<const N: usize> Springs<N> {
    pub fn new(values: [f32; N], stiffness: f32, damping: f32) -> Self {
        Springs { s: values.map(|v| Spring::new(v, stiffness, damping)), last: None }
    }

    pub fn set_targets(&mut self, targets: [f32; N]) {
        for (s, t) in self.s.iter_mut().zip(targets) {
            s.target = t;
        }
    }

    pub fn targets(&self) -> [f32; N] {
        self.s.map(|s| s.target)
    }

    pub fn values(&self) -> [f32; N] {
        self.s.map(|s| s.value)
    }

    /// Advances to `now`; true while anything still moves.
    pub fn tick(&mut self, now: Instant) -> bool {
        let dt = self.last.map(|t| now.saturating_duration_since(t)).unwrap_or(Duration::from_millis(16));
        self.last = Some(now);
        for s in &mut self.s {
            s.advance(dt.as_secs_f32());
        }
        !self.s.iter().all(Spring::settled)
    }

    /// Movement paused (nothing drawn): the next tick starts fresh.
    pub fn rest(&mut self) {
        self.last = None;
    }

    pub fn snap(&mut self) {
        for s in &mut self.s {
            s.snap();
        }
        self.last = None;
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_damped_overshoots_a_little_and_settles() {
        let mut s = Spring::new(0.0, 420.0, 0.68);
        s.target = 100.0;
        let mut peak: f32 = 0.0;
        for _ in 0..240 {
            s.advance(1.0 / 120.0);
            peak = peak.max(s.value);
        }
        assert!(peak > 100.5 && peak < 112.0, "peak {peak}");
        assert!(s.settled(), "value {} velocity {}", s.value, s.velocity);
    }

    #[test]
    fn critical_damping_never_overshoots() {
        let mut s = Spring::new(0.0, 420.0, 1.0);
        s.target = 100.0;
        for _ in 0..240 {
            s.advance(1.0 / 60.0);
            assert!(s.value <= 100.01);
        }
    }

    #[test]
    fn a_new_target_keeps_the_velocity() {
        let mut s = Spring::new(0.0, 420.0, 0.68);
        s.target = 100.0;
        s.advance(0.05);
        let v = s.velocity;
        assert!(v > 0.0);
        s.target = 0.0;
        s.advance(1.0 / 240.0);
        // Still moving up for a moment: no restart from rest.
        assert!(s.velocity > 0.0 && s.velocity < v * 1.1);
    }

    #[test]
    fn frame_rate_doesnt_change_the_path() {
        let mut a = Spring::new(0.0, 420.0, 0.68);
        let mut b = a;
        a.target = 50.0;
        b.target = 50.0;
        for _ in 0..30 {
            a.advance(1.0 / 30.0);
        }
        for _ in 0..120 {
            b.advance(1.0 / 120.0);
        }
        assert!((a.value - b.value).abs() < 0.5);
    }
}
