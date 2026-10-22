//! Dove Voice Mode: a hands-free, local-first call with every agent Dove runs.
//!
//! The crate sits above the harness boundary and knows nothing about GPUI or
//! any one agent:
//!
//! - [`conductor`] turns one final transcript into [`conductor::Effect`]s,
//!   using the deterministic [`fastpath`] grammar, the [`rules`] router and the
//!   [`intent`] schema, with [`safety`] deciding which approvals voice may give.
//! - [`pipeline`] is the real-time audio loop: microphone capture, echo
//!   cancellation, voice activity, turn end, speech-to-text and cancellable
//!   speech playback, all off the UI thread.
//! - [`engines`] holds the swappable traits ([`engines::SpeechToText`],
//!   [`engines::TextToSpeech`], [`engines::TurnDetector`],
//!   [`engines::EchoCanceller`], [`engines::Vad`]) and their implementations.
//!
//! The app builds a [`snapshot::Snapshot`] from state it already holds and
//! performs effects through the same paths a click uses, so a voice turn is
//! recorded, permissioned and cancelled exactly like a typed one.

pub mod aec;
pub mod calls;
pub mod conductor;
pub mod dictation;
pub mod engines;
pub mod fastpath;
pub mod intent;
pub mod metrics;
pub mod models;
pub mod pipeline;
pub mod player;
pub mod resample;
pub mod rules;
pub mod safety;
pub mod snapshot;
pub mod speakable;
pub mod stt;
pub mod tts;
pub mod tts_stream;
pub mod turn;
pub mod vad;

/// Every engine works at this rate internally.
pub const RATE: u32 = 16_000;
/// One analysis frame: 512 samples (32 ms) at [`RATE`], Silero VAD's window.
pub const FRAME: usize = 512;
