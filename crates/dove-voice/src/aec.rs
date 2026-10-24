//! Echo cancellation so a call works on speakers, not only headphones.
//!
//! [`Nlms`] is a normalised least-mean-squares adaptive filter over the
//! signal Dove itself plays (the player hands every rendered sample to the
//! pipeline as the reference). It learns the room's echo path, a few hundred
//! milliseconds long, and subtracts the predicted echo from the microphone.
//! A double-talk guard freezes adaptation while the user talks over Dove, so
//! their voice does not train the filter away.
//!
//! It is a plain time-domain canceller: weaker than WebRTC's AEC3 on long,
//! non-linear echo paths (cheap laptop speakers at high volume), which is why
//! barge-in also requires speech well above the cancelled echo level.

use crate::engines::EchoCanceller;

pub struct Nlms {
    /// Weights aligned with [`Self::window`]: oldest reference sample first.
    taps: Vec<f32>,
    /// Reference history, mirrored twice so the last `n` samples are always
    /// one contiguous slice.
    history: Vec<f32>,
    pos: usize,
    mu: f32,
    /// Smoothed energies for the double-talk guard.
    mic_energy: f32,
    err_energy: f32,
}

impl Nlms {
    /// `tail_ms` of echo path at 16 kHz.
    pub fn new(tail_ms: u32) -> Self {
        let n = (crate::RATE * tail_ms / 1000) as usize;
        Self { taps: vec![0.0; n], history: vec![0.0; 2 * n], pos: 0, mu: 0.3, mic_energy: 0.0, err_energy: 0.0 }
    }

    fn push(&mut self, r: f32) -> f32 {
        let n = self.taps.len();
        let old = self.history[self.pos];
        self.history[self.pos] = r;
        self.history[self.pos + n] = r;
        self.pos = (self.pos + 1) % n;
        old
    }

    /// The last `n` reference samples, oldest first.
    fn window(&self) -> &[f32] {
        &self.history[self.pos..self.pos + self.taps.len()]
    }

    /// Echo-return-loss enhancement over the last frame, in dB (diagnostics).
    pub fn erle_db(&self) -> f32 {
        if self.err_energy <= 0.0 || self.mic_energy <= 0.0 {
            return 0.0;
        }
        10.0 * (self.mic_energy / self.err_energy).log10()
    }
}

impl Default for Nlms {
    fn default() -> Self {
        Self::new(128)
    }
}

impl EchoCanceller for Nlms {
    fn name(&self) -> &'static str {
        "nlms"
    }

    fn process(&mut self, mic: &mut [f32], reference: &[f32]) {
        debug_assert_eq!(mic.len(), reference.len());
        let n = self.taps.len();
        if n == 0 {
            return;
        }
        let ref_energy: f32 = reference.iter().map(|r| r * r).sum();
        if ref_energy < 1e-9 && self.window().iter().all(|h| *h == 0.0) {
            // Dove is silent: nothing to cancel.
            return;
        }
        // Rolling power of the reference window, kept by add/remove.
        let mut power: f32 = self.window().iter().map(|h| h * h).sum();
        let mut frame_mic = 0.0;
        let mut frame_err = 0.0;
        let per_sample = (self.err_energy / mic.len() as f32).max(1e-8);
        let converged = self.erle_db() > 6.0;
        for (m, &r) in mic.iter_mut().zip(reference) {
            let old = self.push(r);
            power = (power - old * old + r * r).max(0.0);
            let window = &self.history[self.pos..self.pos + n];
            let y: f32 = self.taps.iter().zip(window).map(|(w, x)| w * x).sum();
            let e = *m - y;
            frame_mic += *m * *m;
            frame_err += e * e;
            // Adapt only while the residual looks like echo, not like the user.
            let double_talk = converged && e * e > 4.0 * per_sample;
            if !double_talk && power > 1e-6 {
                let g = self.mu * e / (power + 1e-3);
                for (w, x) in self.taps.iter_mut().zip(window) {
                    *w += g * x;
                }
            }
            *m = e;
        }
        self.mic_energy = 0.8 * self.mic_energy + 0.2 * frame_mic;
        self.err_energy = 0.8 * self.err_energy + 0.2 * frame_err;
    }

    fn reset(&mut self) {
        self.taps.iter_mut().for_each(|t| *t = 0.0);
        self.history.iter_mut().for_each(|h| *h = 0.0);
        self.pos = 0;
        self.mic_energy = 0.0;
        self.err_energy = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic white-ish noise.
    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect()
    }

    #[test]
    fn learns_a_delayed_attenuated_echo() {
        let far = noise(16_000 * 3, 7);
        // Room: 12 ms delay, 0.5 gain, plus a weaker reflection at 30 ms.
        let echo: Vec<f32> = (0..far.len())
            .map(|i| {
                0.5 * i.checked_sub(192).map_or(0.0, |j| far[j]) + 0.2 * i.checked_sub(480).map_or(0.0, |j| far[j])
            })
            .collect();
        let mut aec = Nlms::new(64);
        let mut residual_last = 0.0;
        let mut echo_last = 0.0;
        for (k, (f, e)) in far.chunks(crate::FRAME).zip(echo.chunks(crate::FRAME)).enumerate() {
            let mut mic = e.to_vec();
            aec.process(&mut mic, f);
            if k >= 80 {
                residual_last += mic.iter().map(|x| x * x).sum::<f32>();
                echo_last += e.iter().map(|x| x * x).sum::<f32>();
            }
        }
        let erle = 10.0 * (echo_last / residual_last).log10();
        assert!(erle > 20.0, "only {erle:.1} dB of echo removed");
    }

    #[test]
    fn near_end_speech_survives() {
        let far = noise(16_000 * 2, 3);
        let near = noise(16_000 * 2, 99);
        let mut aec = Nlms::new(32);
        // Train on echo alone, then the user talks over it.
        for f in far.chunks(crate::FRAME).take(40) {
            let mut mic: Vec<f32> = f.iter().map(|x| x * 0.5).collect();
            aec.process(&mut mic, f);
        }
        let mut kept = 0.0;
        let mut spoken = 0.0;
        for (f, n) in far.chunks(crate::FRAME).zip(near.chunks(crate::FRAME)).skip(40) {
            let mut mic: Vec<f32> = f.iter().zip(n).map(|(e, v)| e * 0.5 + v).collect();
            aec.process(&mut mic, f);
            kept += mic.iter().zip(n).map(|(m, v)| m * v).sum::<f32>();
            spoken += n.iter().map(|v| v * v).sum::<f32>();
        }
        assert!(kept / spoken > 0.8, "the user's voice was cancelled too ({:.2})", kept / spoken);
    }

    #[test]
    fn silence_from_dove_leaves_the_mic_untouched() {
        let mut aec = Nlms::default();
        let mut mic = noise(crate::FRAME, 5);
        let before = mic.clone();
        aec.process(&mut mic, &vec![0.0; crate::FRAME]);
        assert_eq!(mic, before);
    }
}
