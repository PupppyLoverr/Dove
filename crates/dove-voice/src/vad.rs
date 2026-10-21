//! Voice activity detection: Silero VAD on ONNX Runtime, with an adaptive
//! energy detector for machines where the model is not installed.

use crate::engines::Vad;

/// RMS against a tracked noise floor. Good in a quiet room, fooled by loud
/// steady noise; Silero is the default when its model is present.
#[derive(Debug, Clone)]
pub struct EnergyVad {
    floor: f32,
}

impl Default for EnergyVad {
    fn default() -> Self {
        Self { floor: 0.003 }
    }
}

pub fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
}

impl Vad for EnergyVad {
    fn name(&self) -> &'static str {
        "energy"
    }

    fn speech(&mut self, frame: &[f32]) -> f32 {
        let e = rms(frame);
        // The floor falls quickly and rises slowly, so speech does not
        // become the new floor.
        self.floor = if e < self.floor { self.floor * 0.9 + e * 0.1 } else { self.floor * 0.995 + e * 0.005 };
        self.floor = self.floor.max(0.0005);
        let snr = e / self.floor;
        ((snr - 2.0) / 4.0).clamp(0.0, 1.0)
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(feature = "local")]
pub use silero::SileroVad;

/// An ONNX Runtime session for a small model: few threads, so a call never
/// competes with the agents it is driving. `ort` builder errors carry the
/// builder (not `Send`), so they are flattened to text here.
#[cfg(feature = "local")]
pub(crate) fn onnx_session(path: &std::path::Path, threads: usize) -> anyhow::Result<ort::session::Session> {
    // No spinning: the VAD runs every 32 ms all call long, and spinning
    // worker threads would burn CPU between frames.
    onnx_session_tuned(path, OnnxTuning { threads, spin: false, optimize: 3 })
}

/// How an ONNX Runtime session is built. Models that run in short bursts
/// (speech recognition, turn detection) are faster with spinning workers;
/// the always-on VAD is not.
#[cfg(feature = "local")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnnxTuning {
    pub threads: usize,
    pub spin: bool,
    /// Graph optimization: 0 off, 1 basic, 2 extended, 3 all.
    pub optimize: u8,
}

#[cfg(feature = "local")]
pub fn onnx_session_tuned(path: &std::path::Path, t: OnnxTuning) -> anyhow::Result<ort::session::Session> {
    use ort::session::builder::GraphOptimizationLevel as L;
    let e = |e: ort::Error<ort::session::builder::SessionBuilder>| anyhow::anyhow!("{e}");
    let level = match t.optimize {
        0 => L::Disable,
        1 => L::Level1,
        2 => L::Level2,
        _ => L::Level3,
    };
    ort::session::Session::builder()?
        .with_optimization_level(level)
        .map_err(e)?
        .with_intra_threads(t.threads)
        .map_err(e)?
        .with_inter_threads(1)
        .map_err(e)?
        .with_intra_op_spinning(t.spin)
        .map_err(e)?
        .with_inter_op_spinning(false)
        .map_err(e)?
        .commit_from_file(path)
        .map_err(|err| anyhow::anyhow!("loading {}: {err}", path.display()))
}

#[cfg(feature = "local")]
mod silero {
    use std::path::Path;

    use anyhow::{Context, Result};
    use ort::session::Session;
    use ort::value::Tensor;

    use crate::engines::Vad;

    /// Samples of the previous window Silero v5 expects in front of each
    /// 512-sample window at 16 kHz.
    const CONTEXT: usize = 64;

    pub struct SileroVad {
        session: Session,
        state: Vec<f32>,
        context: Vec<f32>,
        /// The room's noise floor; frames well under it skip the model.
        floor: f32,
        /// The model's recurrent state is stale after skipped frames.
        skipped: bool,
        pub gate: bool,
        pub runs: u64,
        pub frames: u64,
    }

    impl SileroVad {
        pub fn load(path: &Path) -> Result<Self> {
            crate::models::verify(path, &crate::models::SILERO)?;
            let session = super::onnx_session(path, 1).context("loading Silero VAD")?;
            Ok(Self {
                session,
                state: vec![0.0; 2 * 128],
                context: vec![0.0; CONTEXT],
                floor: 0.003,
                skipped: false,
                gate: true,
                runs: 0,
                frames: 0,
            })
        }

        fn run(&mut self, frame: &[f32]) -> Result<f32> {
            let mut input = Vec::with_capacity(CONTEXT + frame.len());
            input.extend_from_slice(&self.context);
            input.extend_from_slice(frame);
            let n = input.len();
            let outputs = self.session.run(ort::inputs![
                "input" => Tensor::from_array(([1usize, n], input))?,
                "state" => Tensor::from_array(([2usize, 1, 128], self.state.clone()))?,
                "sr" => Tensor::from_array(([0usize; 0], vec![crate::RATE as i64]))?,
            ])?;
            let (_, p) = outputs["output"].try_extract_tensor::<f32>()?;
            let p = p.first().copied().unwrap_or(0.0);
            let (_, s) = outputs["stateN"].try_extract_tensor::<f32>()?;
            if s.len() == self.state.len() {
                self.state.copy_from_slice(s);
            }
            if frame.len() >= CONTEXT {
                self.context.copy_from_slice(&frame[frame.len() - CONTEXT..]);
            }
            Ok(p)
        }
    }

    impl Vad for SileroVad {
        fn name(&self) -> &'static str {
            "silero"
        }

        fn speech(&mut self, frame: &[f32]) -> f32 {
            self.frames += 1;
            // An energy gate: a frame within 1.5x of the room's noise floor
            // (and below any plausible speech level) is silence without
            // asking the model. Silent stretches then cost almost nothing.
            let e = super::rms(frame);
            self.floor = if e < self.floor { self.floor * 0.9 + e * 0.1 } else { self.floor * 0.995 + e * 0.005 };
            self.floor = self.floor.max(0.0003);
            if self.gate && e < (self.floor * 1.5).min(0.01) {
                if !self.skipped {
                    self.skipped = true;
                }
                return 0.0;
            }
            if self.skipped {
                // Start the recurrent state fresh after a skipped stretch.
                self.state.iter_mut().for_each(|s| *s = 0.0);
                self.context.iter_mut().for_each(|s| *s = 0.0);
                self.skipped = false;
            }
            self.runs += 1;
            match self.run(frame) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("silero vad: {e:#}");
                    0.0
                }
            }
        }

        fn reset(&mut self) {
            self.state.iter_mut().for_each(|s| *s = 0.0);
            self.context.iter_mut().for_each(|s| *s = 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amp: f32, n: usize, freq: f32) -> Vec<f32> {
        (0..n).map(|i| amp * (i as f32 * freq * std::f32::consts::TAU / crate::RATE as f32).sin()).collect()
    }

    #[test]
    fn energy_vad_tracks_the_room_and_hears_speech() {
        let mut v = EnergyVad::default();
        // A noisy room settles below the threshold.
        let noise: Vec<f32> = (0..crate::FRAME).map(|i| 0.004 * ((i * 7919 % 97) as f32 / 97.0 - 0.5)).collect();
        let mut p = 1.0;
        for _ in 0..50 {
            p = v.speech(&noise);
        }
        assert!(p < 0.1, "noise scored {p}");
        // Speech-level audio scores high.
        let voice = tone(0.2, crate::FRAME, 220.0);
        assert!(v.speech(&voice) > 0.9);
    }
}
