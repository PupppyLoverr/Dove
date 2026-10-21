//! End-of-turn decisions.
//!
//! [`PauseTurn`] ends a turn after a pause, longer when the words so far
//! sound unfinished ("and", "so", "the"). [`SmartTurn`] asks Smart Turn v3
//! (an 8 MB audio model) whether the speech itself sounds finished, which
//! lets the pause be much shorter without cutting people off.

use crate::engines::{TurnDecision, TurnDetector};

/// Words that usually mean more is coming.
const DANGLING: &[&str] = &[
    "and", "but", "or", "so", "because", "the", "a", "an", "to", "of", "with", "for", "in", "on", "that", "which",
    "if", "then", "um", "uh", "like", "also", "my", "your", "is", "are", "was", "should", "could", "would", "can",
];

#[derive(Debug, Clone)]
pub struct PauseTurn {
    /// Pause that ends a finished-sounding turn.
    pub short_ms: u32,
    /// Pause that ends a turn that sounds cut off.
    pub long_ms: u32,
}

impl Default for PauseTurn {
    fn default() -> Self {
        Self { short_ms: 550, long_ms: 1000 }
    }
}

pub fn sounds_unfinished(partial: &str) -> bool {
    let t = partial.trim();
    if t.ends_with(',') || t.ends_with('-') || t.ends_with("...") {
        return true;
    }
    let last =
        t.rsplit(char::is_whitespace).next().unwrap_or("").trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    DANGLING.contains(&last.as_str())
}

impl TurnDetector for PauseTurn {
    fn name(&self) -> &'static str {
        "pause"
    }

    fn decide(&mut self, _audio: &[f32], silence_ms: u32, partial: Option<&str>) -> TurnDecision {
        let need = if partial.is_some_and(sounds_unfinished) { self.long_ms } else { self.short_ms };
        if silence_ms >= need { TurnDecision::Complete } else { TurnDecision::Wait }
    }
}

/// Whisper's log-mel front end (80 bins, 25 ms window, 10 ms hop), which
/// Smart Turn was trained on. Matches `WhisperFeatureExtractor` with
/// `do_normalize=True` over an 8 s window padded at the start.
pub mod mel {
    use std::sync::OnceLock;

    pub const N_FFT: usize = 400;
    pub const HOP: usize = 160;
    pub const N_MELS: usize = 80;
    pub const SECONDS: usize = 8;
    pub const SAMPLES: usize = SECONDS * crate::RATE as usize;
    pub const FRAMES: usize = SAMPLES / HOP;

    fn hz_to_mel(f: f64) -> f64 {
        // Slaney scale, as librosa (htk=False).
        let f_sp = 200.0 / 3.0;
        let min_log_hz = 1000.0;
        let min_log_mel = min_log_hz / f_sp;
        let logstep = (6.4f64).ln() / 27.0;
        if f >= min_log_hz { min_log_mel + (f / min_log_hz).ln() / logstep } else { f / f_sp }
    }

    fn mel_to_hz(m: f64) -> f64 {
        let f_sp = 200.0 / 3.0;
        let min_log_hz = 1000.0;
        let min_log_mel = min_log_hz / f_sp;
        let logstep = (6.4f64).ln() / 27.0;
        if m >= min_log_mel { min_log_hz * (logstep * (m - min_log_mel)).exp() } else { f_sp * m }
    }

    /// Slaney-normalised triangular filters, `N_MELS x (N_FFT/2+1)`.
    pub fn filters() -> &'static Vec<f32> {
        static F: OnceLock<Vec<f32>> = OnceLock::new();
        F.get_or_init(|| {
            let bins = N_FFT / 2 + 1;
            let sr = crate::RATE as f64;
            let fft_freqs: Vec<f64> = (0..bins).map(|i| i as f64 * sr / N_FFT as f64).collect();
            let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(sr / 2.0));
            let mel_pts: Vec<f64> =
                (0..N_MELS + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (N_MELS + 1) as f64)).collect();
            let mut out = vec![0f32; N_MELS * bins];
            for m in 0..N_MELS {
                let (l, c, r) = (mel_pts[m], mel_pts[m + 1], mel_pts[m + 2]);
                let enorm = 2.0 / (r - l);
                for (k, &f) in fft_freqs.iter().enumerate() {
                    let lower = (f - l) / (c - l);
                    let upper = (r - f) / (r - c);
                    let w = lower.min(upper).max(0.0);
                    out[m * bins + k] = (w * enorm) as f32;
                }
            }
            out
        })
    }

    /// `audio` (any length, 16 kHz) to `N_MELS x FRAMES` features, row-major.
    pub fn features(audio: &[f32]) -> Vec<f32> {
        // Last 8 s, zero-padded at the start.
        let mut x = vec![0f32; SAMPLES];
        let take = audio.len().min(SAMPLES);
        x[SAMPLES - take..].copy_from_slice(&audio[audio.len() - take..]);
        // Zero mean, unit variance.
        let mean = x.iter().map(|v| *v as f64).sum::<f64>() / SAMPLES as f64;
        let var = x.iter().map(|v| (*v as f64 - mean).powi(2)).sum::<f64>() / SAMPLES as f64;
        let std = (var + 1e-7).sqrt();
        x.iter_mut().for_each(|v| *v = ((*v as f64 - mean) / std) as f32);

        // Centered STFT with reflect padding, periodic Hann window.
        let pad = N_FFT / 2;
        let mut padded = Vec::with_capacity(SAMPLES + 2 * pad);
        padded.extend((1..=pad).rev().map(|i| x[i]));
        padded.extend_from_slice(&x);
        padded.extend((SAMPLES - pad - 1..SAMPLES - 1).rev().map(|i| x[i]));
        let window: Vec<f32> =
            (0..N_FFT).map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / N_FFT as f64).cos()) as f32).collect();

        let mut planner = realfft::RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(N_FFT);
        let bins = N_FFT / 2 + 1;
        let mut frame = fft.make_input_vec();
        let mut spec = fft.make_output_vec();
        let filters = filters();
        let mut out = vec![0f32; N_MELS * FRAMES];
        // Whisper drops the last STFT frame.
        for t in 0..FRAMES {
            let start = t * HOP;
            for i in 0..N_FFT {
                frame[i] = padded[start + i] * window[i];
            }
            fft.process(&mut frame, &mut spec).expect("fft sizes match");
            let power: Vec<f32> = spec.iter().map(|c| c.norm_sqr()).collect();
            for m in 0..N_MELS {
                let row = &filters[m * bins..(m + 1) * bins];
                let e: f32 = row.iter().zip(&power).map(|(w, p)| w * p).sum();
                out[m * FRAMES + t] = e.max(1e-10).log10();
            }
        }
        let max = out.iter().cloned().fold(f32::MIN, f32::max);
        out.iter_mut().for_each(|v| *v = ((*v).max(max - 8.0) + 4.0) / 4.0);
        out
    }
}

#[cfg(feature = "local")]
pub use smart::SmartTurn;

#[cfg(feature = "local")]
mod smart {
    use std::path::Path;

    use anyhow::{Context, Result};
    use ort::session::Session;
    use ort::value::Tensor;

    use super::{PauseTurn, mel};
    use crate::engines::{TurnDecision, TurnDetector};

    /// Smart Turn v3 with a pause floor: the model is asked once the user has
    /// paused for `ask_ms`; a confident "finished" ends the turn then, and a
    /// long pause ends it regardless.
    pub struct SmartTurn {
        session: Session,
        pub ask_ms: u32,
        pub threshold: f32,
        fallback: PauseTurn,
        /// The silence length the model was last asked at, so one pause is
        /// scored once.
        asked_at: Option<u32>,
        pub last_ms: f32,
    }

    impl SmartTurn {
        /// Tuning measured by the stt_tune example (see docs/VOICE.md).
        pub const TUNING: crate::vad::OnnxTuning = crate::vad::OnnxTuning { threads: 4, spin: true, optimize: 3 };

        pub fn load(path: &Path) -> Result<Self> {
            Self::load_tuned(path, Self::TUNING)
        }

        pub fn load_tuned(path: &Path, tuning: crate::vad::OnnxTuning) -> Result<Self> {
            crate::models::verify(path, &crate::models::SMART_TURN)?;
            let session = crate::vad::onnx_session_tuned(path, tuning).context("loading Smart Turn")?;
            Ok(Self {
                session,
                ask_ms: 200,
                threshold: 0.5,
                fallback: PauseTurn::default(),
                asked_at: None,
                last_ms: 0.0,
            })
        }

        /// Probability that the turn in `audio` is complete.
        pub fn probability(&mut self, audio: &[f32]) -> Result<f32> {
            let t = std::time::Instant::now();
            let feats = mel::features(audio);
            let outputs = self.session.run(
                ort::inputs!["input_features" => Tensor::from_array(([1usize, mel::N_MELS, mel::FRAMES], feats))?],
            )?;
            let (_, p) = outputs["logits"].try_extract_tensor::<f32>()?;
            self.last_ms = t.elapsed().as_secs_f32() * 1000.0;
            Ok(p.first().copied().unwrap_or(0.0))
        }
    }

    impl TurnDetector for SmartTurn {
        fn name(&self) -> &'static str {
            "smart-turn-v3.2"
        }

        fn decide(&mut self, audio: &[f32], silence_ms: u32, partial: Option<&str>) -> TurnDecision {
            if silence_ms < self.asked_at.unwrap_or(0) {
                // A new pause: ask again.
                self.asked_at = None;
            }
            if self.fallback.long_ms <= silence_ms {
                return TurnDecision::Complete;
            }
            if silence_ms >= self.ask_ms && self.asked_at.is_none() {
                self.asked_at = Some(silence_ms);
                match self.probability(audio) {
                    Ok(p) if p >= self.threshold => return TurnDecision::Complete,
                    Ok(_) => return TurnDecision::Wait,
                    Err(e) => tracing::warn!("smart turn: {e:#}"),
                }
            }
            if self.asked_at.is_some() {
                return TurnDecision::Wait;
            }
            self.fallback.decide(audio, silence_ms, partial)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_turn_waits_longer_after_a_dangling_word() {
        let mut t = PauseTurn::default();
        assert_eq!(t.decide(&[], 600, Some("Run the tests.")), TurnDecision::Complete);
        assert_eq!(t.decide(&[], 600, Some("Run the tests and")), TurnDecision::Wait);
        assert_eq!(t.decide(&[], 1400, Some("Run the tests and")), TurnDecision::Complete);
        assert_eq!(t.decide(&[], 300, None), TurnDecision::Wait);
        assert!(sounds_unfinished("I want to,"));
        assert!(!sounds_unfinished("Stop."));
    }

    #[test]
    fn mel_features_have_whisper_shape_and_range() {
        let f = mel::features(&[]);
        assert_eq!(f.len(), mel::N_MELS * mel::FRAMES);
        let tone: Vec<f32> =
            (0..16_000).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 16_000.0).sin() * 0.3).collect();
        let f = mel::features(&tone);
        let max = f.iter().cloned().fold(f32::MIN, f32::max);
        let min = f.iter().cloned().fold(f32::MAX, f32::min);
        // Whisper's clamp: at most 8 log10 units (2.0 after scaling) below the peak.
        assert!(max - min <= 2.0 + 1e-4, "{min}..{max}");
        // The 440 Hz bin is loudest in the last second, not in the padding.
        let bin_440 = (0..mel::N_MELS)
            .max_by(|a, b| f[a * mel::FRAMES + mel::FRAMES - 10].total_cmp(&f[b * mel::FRAMES + mel::FRAMES - 10]))
            .unwrap();
        assert!((5..20).contains(&bin_440), "440 Hz landed in mel bin {bin_440}");
    }

    #[test]
    fn mel_filters_are_slaney_normalised() {
        let f = mel::filters();
        let bins = mel::N_FFT / 2 + 1;
        // Every filter has weight; area-normalised filters shrink with width.
        let peak = |m: usize| f[m * bins..(m + 1) * bins].iter().cloned().fold(0f32, f32::max);
        assert!((0..mel::N_MELS).all(|m| peak(m) > 0.0));
        assert!(peak(0) > peak(mel::N_MELS - 1));
    }
}
