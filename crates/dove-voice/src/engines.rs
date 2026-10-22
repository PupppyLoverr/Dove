//! The swappable parts of a call. Each engine is a trait so local, system
//! and cloud implementations plug in without the pipeline knowing which.

use std::sync::atomic::AtomicBool;

/// Mono PCM at `rate`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pcm {
    pub samples: Vec<f32>,
    pub rate: u32,
}

impl Pcm {
    pub fn duration_ms(&self) -> u64 {
        if self.rate == 0 { 0 } else { self.samples.len() as u64 * 1000 / u64::from(self.rate) }
    }
}

/// Voice activity: speech probability for one [`crate::FRAME`] at [`crate::RATE`].
pub trait Vad: Send {
    fn name(&self) -> &'static str;
    fn speech(&mut self, frame: &[f32]) -> f32;
    fn reset(&mut self);
}

/// Removes the speaker's echo from the microphone signal.
pub trait EchoCanceller: Send {
    fn name(&self) -> &'static str;
    /// `mic` and `reference` are the same length and time-aligned frames at
    /// [`crate::RATE`]: what the microphone heard, and what Dove played.
    fn process(&mut self, mic: &mut [f32], reference: &[f32]);
    fn reset(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnDecision {
    /// The user has finished; transcribe and answer.
    Complete,
    /// A pause mid-thought; keep listening.
    Wait,
}

/// Decides when a spoken turn has ended.
pub trait TurnDetector: Send {
    fn name(&self) -> &'static str;
    /// `audio` is the turn so far, `silence_ms` the trailing pause, `partial`
    /// the latest partial transcript when there is one.
    fn decide(&mut self, audio: &[f32], silence_ms: u32, partial: Option<&str>) -> TurnDecision;
}

pub trait SpeechToText: Send {
    fn name(&self) -> String;
    /// A transcript of the turn so far (for the live caption). May be
    /// slower than real time on weak machines; the pipeline skips partials
    /// it cannot afford.
    fn partial(&mut self, audio: &[f32]) -> anyhow::Result<String>;
    /// The final transcript of a finished turn.
    fn finish(&mut self, audio: &[f32]) -> anyhow::Result<String>;
    /// Whether audio leaves this machine.
    fn remote(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpeechOptions {
    /// 1.0 is the voice's normal pace.
    pub rate: f32,
    pub voice: Option<String>,
}

impl Default for SpeechOptions {
    fn default() -> Self {
        Self { rate: 1.0, voice: None }
    }
}

pub trait TextToSpeech: Send {
    fn name(&self) -> String;
    /// Speak one clause. Returns early (with what it has) when `cancel` is set.
    fn synthesize(&mut self, text: &str, opts: &SpeechOptions, cancel: &AtomicBool) -> anyhow::Result<Pcm>;
    /// Speak one clause, handing audio to `out` as it arrives so playback
    /// starts on the first chunk. `out` returns false to stop early (the
    /// reply was interrupted). Engines that render a clause in one piece
    /// keep this default.
    fn synthesize_stream(
        &mut self,
        text: &str,
        opts: &SpeechOptions,
        cancel: &AtomicBool,
        out: &mut dyn FnMut(Pcm) -> bool,
    ) -> anyhow::Result<()> {
        let pcm = self.synthesize(text, opts, cancel)?;
        if !pcm.samples.is_empty() {
            out(pcm);
        }
        Ok(())
    }
    fn voices(&self) -> Vec<String> {
        Vec::new()
    }
    /// Whether text leaves this machine.
    fn remote(&self) -> bool {
        false
    }
}
