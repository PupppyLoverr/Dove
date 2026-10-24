//! The real-time call: microphone → echo cancellation → voice activity →
//! turn end → speech-to-text, and text → speech → cancellable playback.
//!
//! Nothing here runs on the UI thread. A call owns three threads:
//!
//! - `voice-call` owns the microphone and speaker streams and the
//!   [`Listener`], and handles controls (speak, silence, mute, push-to-talk);
//! - `voice-stt` owns the speech-to-text engine (partials are dropped when
//!   it is busy, finals never are);
//! - `voice-tts` owns the speech engine and renders one clause at a time.
//!
//! Barge-in: when the user starts talking over Dove, the speaker queue is
//! cleared at once, the clause being rendered is cancelled, and every clause
//! queued under the old generation is discarded instead of played.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded, unbounded};
use parking_lot::Mutex;

use crate::engines::{EchoCanceller, SpeechOptions, SpeechToText, TextToSpeech, TurnDecision, TurnDetector, Vad};
use crate::metrics::TurnTimings;

pub(crate) const FRAME_MS: u32 = (crate::FRAME as u32 * 1000) / crate::RATE;

#[derive(Debug, Clone)]
pub struct ListenerConfig {
    pub start_prob: f32,
    pub stop_prob: f32,
    /// Consecutive voiced frames that start a turn.
    pub start_frames: u32,
    /// Stricter thresholds while Dove is talking, so echo residue does not
    /// count as the user.
    pub barge_prob: f32,
    pub barge_frames: u32,
    /// Audio kept from before speech was detected.
    pub preroll_frames: usize,
    /// Turns shorter than this are a cough or a click.
    pub min_voiced_frames: u32,
    pub max_turn_ms: u32,
    pub partial_every_ms: u32,
    /// Silent frames after which the turn so far is transcribed early, so
    /// the text is ready (or nearly) when the turn detector decides.
    pub speculate_frames: u32,
    /// Trailing silent frames kept on the audio handed to speech
    /// recognition (more invites recognizers to invent words).
    pub keep_tail_frames: u32,
}

impl Default for ListenerConfig {
    fn default() -> Self {
        Self {
            start_prob: 0.5,
            stop_prob: 0.35,
            start_frames: 2,
            barge_prob: 0.75,
            barge_frames: 4,
            preroll_frames: 10,
            min_voiced_frames: 5,
            max_turn_ms: 30_000,
            partial_every_ms: 900,
            speculate_frames: 4,
            keep_tail_frames: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Heard {
    Started {
        barge_in: bool,
    },
    WantPartial(Vec<f32>),
    /// The user paused: transcribe what was said so far, in case the pause
    /// is the end of the turn.
    Speculate(Vec<f32>),
    /// The turn ended. `speculated`: nothing was said after the last
    /// speculation, so its transcript is the turn's transcript.
    Ended {
        audio: Vec<f32>,
        timings: TurnTimings,
        speculated: bool,
    },
    Discarded,
}

/// The device-free listening state machine, fed 16 kHz frames.
pub struct Listener {
    vad: Box<dyn Vad>,
    aec: Option<Box<dyn EchoCanceller>>,
    turn: Box<dyn TurnDetector>,
    pub cfg: ListenerConfig,
    preroll: VecDeque<Vec<f32>>,
    speech: Vec<f32>,
    in_turn: bool,
    voiced_run: u32,
    voiced_frames: u32,
    silence_frames: u32,
    since_partial_ms: u32,
    partial: Option<String>,
    turn_decide_ms: f32,
    /// `voiced_frames` when the current pause was speculatively transcribed.
    spec_voiced: Option<u32>,
    pub last_voiced_at: Option<Instant>,
}

impl Listener {
    pub fn new(vad: Box<dyn Vad>, aec: Option<Box<dyn EchoCanceller>>, turn: Box<dyn TurnDetector>) -> Self {
        Self {
            vad,
            aec,
            turn,
            cfg: ListenerConfig::default(),
            preroll: VecDeque::new(),
            speech: Vec::new(),
            in_turn: false,
            voiced_run: 0,
            voiced_frames: 0,
            silence_frames: 0,
            since_partial_ms: 0,
            partial: None,
            turn_decide_ms: 0.0,
            spec_voiced: None,
            last_voiced_at: None,
        }
    }

    pub fn names(&self) -> (&'static str, Option<&'static str>, &'static str) {
        (self.vad.name(), self.aec.as_ref().map(|a| a.name()), self.turn.name())
    }

    pub fn in_turn(&self) -> bool {
        self.in_turn
    }

    /// The latest partial transcript, for turn decisions.
    pub fn set_partial(&mut self, text: String) {
        if self.in_turn {
            self.partial = Some(text);
        }
    }

    /// Drop the current turn (mute, push-to-talk release with nothing said).
    pub fn reset(&mut self) {
        self.in_turn = false;
        self.speech.clear();
        self.preroll.clear();
        self.voiced_run = 0;
        self.voiced_frames = 0;
        self.silence_frames = 0;
        self.partial = None;
        self.turn_decide_ms = 0.0;
        self.spec_voiced = None;
        self.vad.reset();
    }

    /// End the current turn now (push-to-talk released).
    pub fn force_end(&mut self) -> Option<Heard> {
        if !self.in_turn {
            self.reset();
            return None;
        }
        Some(self.finish())
    }

    /// Push-to-talk: treat what follows as speech without waiting for VAD.
    pub fn begin(&mut self) {
        if !self.in_turn {
            self.in_turn = true;
            self.speech = self.preroll.drain(..).flatten().collect();
            self.voiced_frames = 0;
            self.silence_frames = 0;
            self.since_partial_ms = 0;
        }
    }

    fn finish(&mut self) -> Heard {
        let voiced = self.voiced_frames;
        let timings = TurnTimings {
            speech_ms: voiced * FRAME_MS,
            endpoint_silence_ms: self.silence_frames * FRAME_MS,
            turn_decide_ms: self.turn_decide_ms,
            ..Default::default()
        };
        let mut audio = std::mem::take(&mut self.speech);
        let trailing = self.silence_frames.saturating_sub(self.cfg.keep_tail_frames) as usize * crate::FRAME;
        audio.truncate(audio.len().saturating_sub(trailing));
        let speculated = self.spec_voiced == Some(voiced);
        let last = self.last_voiced_at;
        self.reset();
        self.last_voiced_at = last;
        if voiced < self.cfg.min_voiced_frames { Heard::Discarded } else { Heard::Ended { audio, timings, speculated } }
    }

    /// One [`crate::FRAME`] of microphone audio, the matching speaker
    /// reference, and whether Dove is talking right now.
    pub fn feed(&mut self, mic: &mut [f32], reference: &[f32], dove_speaking: bool) -> Option<Heard> {
        if let Some(aec) = self.aec.as_mut() {
            aec.process(mic, reference);
        }
        let p = self.vad.speech(mic);
        if !self.in_turn {
            self.preroll.push_back(mic.to_vec());
            while self.preroll.len() > self.cfg.preroll_frames {
                self.preroll.pop_front();
            }
            let (need_p, need_n) = if dove_speaking {
                (self.cfg.barge_prob, self.cfg.barge_frames)
            } else {
                (self.cfg.start_prob, self.cfg.start_frames)
            };
            self.voiced_run = if p >= need_p { self.voiced_run + 1 } else { 0 };
            if self.voiced_run >= need_n {
                self.in_turn = true;
                self.speech = self.preroll.drain(..).flatten().collect();
                self.voiced_frames = self.voiced_run;
                self.silence_frames = 0;
                self.since_partial_ms = 0;
                self.turn_decide_ms = 0.0;
                self.partial = None;
                self.last_voiced_at = Some(Instant::now());
                return Some(Heard::Started { barge_in: dove_speaking });
            }
            return None;
        }
        self.speech.extend_from_slice(mic);
        self.since_partial_ms += FRAME_MS;
        if p >= self.cfg.stop_prob {
            self.voiced_frames += 1;
            self.silence_frames = 0;
            self.last_voiced_at = Some(Instant::now());
        } else {
            self.silence_frames += 1;
        }
        if self.speech.len() as u64 * 1000 / u64::from(crate::RATE) >= u64::from(self.cfg.max_turn_ms) {
            return Some(self.finish());
        }
        if self.silence_frames > 0 {
            let t = Instant::now();
            let d = self.turn.decide(&self.speech, self.silence_frames * FRAME_MS, self.partial.as_deref());
            self.turn_decide_ms += t.elapsed().as_secs_f32() * 1000.0;
            if d == TurnDecision::Complete {
                return Some(self.finish());
            }
            if self.silence_frames >= self.cfg.speculate_frames && self.spec_voiced != Some(self.voiced_frames) {
                self.spec_voiced = Some(self.voiced_frames);
                // The same trim `finish` applies, so a speculation that holds
                // transcribes exactly the audio the final would.
                let trailing = self.silence_frames.saturating_sub(self.cfg.keep_tail_frames) as usize * crate::FRAME;
                let keep = self.speech.len().saturating_sub(trailing);
                return Some(Heard::Speculate(self.speech[..keep].to_vec()));
            }
            return None;
        }
        if self.since_partial_ms >= self.cfg.partial_every_ms {
            self.since_partial_ms = 0;
            return Some(Heard::WantPartial(self.speech.clone()));
        }
        None
    }
}

/// Which engines a call uses.
pub enum SttChoice {
    /// Moonshine; the path is Dove's data folder.
    #[cfg(feature = "local")]
    Moonshine(PathBuf, crate::models::MoonshineSize),
    #[cfg(feature = "local")]
    Parakeet(PathBuf),
    Cloud {
        base_url: String,
        api_key: String,
        model: String,
    },
    Deepgram {
        api_key: String,
        model: String,
    },
}

pub enum TtsChoice {
    System,
    Cloud {
        base_url: String,
        api_key: String,
        model: String,
        voice: String,
    },
    ElevenLabs {
        api_key: String,
        voice: String,
        model: String,
    },
    /// A streaming cloud voice (Fish Audio, Deepgram, Cartesia, Inworld).
    Stream(crate::tts_stream::Provider),
}

/// Where a call's audio comes from.
pub enum Input {
    /// A microphone (`None`: the system default).
    Device(Option<String>),
    /// Mono samples at `rate` pushed by someone else: a phone remote
    /// streaming its microphone to this desktop, or a test.
    Stream { rate: u32, samples: Receiver<Vec<f32>> },
}

impl Input {
    /// A WAV file played in real time after `lead` of silence, then silence
    /// for the rest of the call (or silence only, with no file). For QA of
    /// the app without opening a microphone; the feeder stops with the call.
    pub fn from_file(path: Option<&std::path::Path>, lead: Duration) -> Result<Input> {
        let (samples, rate) = match path {
            Some(p) => {
                let mut r = hound::WavReader::open(p).with_context(|| format!("open {}", p.display()))?;
                let spec = r.spec();
                let data: Vec<f32> = match spec.sample_format {
                    hound::SampleFormat::Int => {
                        let scale = (1u64 << (spec.bits_per_sample - 1)) as f32;
                        r.samples::<i32>().map(|s| s.map(|v| v as f32 / scale)).collect::<Result<_, _>>()?
                    }
                    hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>()?,
                };
                let ch = usize::from(spec.channels.max(1));
                (data.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect::<Vec<f32>>(), spec.sample_rate)
            }
            None => (Vec::new(), crate::RATE),
        };
        let (tx, rx) = unbounded();
        let chunk = (rate as usize * 32 / 1000).max(1);
        let lead_chunks = (lead.as_millis() / 32) as usize;
        std::thread::Builder::new().name("voice-file-input".into()).spawn(move || {
            let silent = vec![0.0f32; chunk];
            let audio = std::iter::repeat_n(silent.clone(), lead_chunks)
                .chain(samples.chunks(chunk).map(<[f32]>::to_vec))
                .chain(std::iter::repeat(silent));
            for piece in audio {
                if tx.send(piece).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(32));
            }
        })?;
        Ok(Input::Stream { rate, samples: rx })
    }
}

/// Where a call's speech goes.
pub enum Output {
    /// A speaker (`None`: the system default).
    Device(Option<String>),
    /// Nowhere: replies are shown, not spoken (and a remote plays its own).
    Silent,
}

pub struct CallConfig {
    /// Dove's data folder (models live under `voice/`).
    pub root: PathBuf,
    pub input: Input,
    pub output: Output,
    pub push_to_talk: bool,
    pub echo_cancel: bool,
    pub rate: f32,
    pub voice: Option<String>,
    pub stt: SttChoice,
    pub tts: TtsChoice,
    pub earcons: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Loading,
    Listening,
    /// The user is talking.
    Hearing,
    /// A turn was sent; waiting on the agent.
    Thinking,
    Speaking,
    Muted,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    State(State),
    Ready {
        startup_ms: u32,
        warm: bool,
        stt: String,
        tts: String,
        vad: String,
        turn: String,
        echo_cancel: bool,
    },
    Partial(String),
    Turn {
        text: String,
        timings: TurnTimings,
    },
    BargeIn {
        dropped_ms: u64,
    },
    /// Time from the reply's first text to its first audio.
    ReplyAudio {
        text_to_audio_ms: f32,
    },
    Level {
        mic: f32,
        speaker: f32,
    },
    Error(String),
    Ended,
}

enum Ctl {
    Say(String),
    /// The agent's reply started streaming text now.
    ReplyText,
    Silence,
    Mute(bool),
    Ptt(bool),
    Rate(f32),
    Voice(Option<String>),
    End,
}

/// Engines kept loaded between calls, dropped after a minute, so calling
/// straight back starts warm and an idle Dove holds no voice models (they
/// take several hundred megabytes).
struct Warm {
    stt: Option<(String, Box<dyn SpeechToText>)>,
    vad: Option<Box<dyn Vad>>,
    turn: Option<Box<dyn TurnDetector>>,
    at: Instant,
}

static WARM: Mutex<Option<Warm>> = Mutex::new(None);
const WARM_TTL: Duration = Duration::from_secs(60);

/// Drop cached engines now (Settings › Voice › remove, or memory pressure).
pub fn unload() {
    WARM.lock().take();
}

/// The cached engine for `key` and the cached VAD, leaving the rest (the
/// turn detector) for the next call. Dictation and calls share one cache.
/// A cached recognizer and VAD, either of which may be missing.
pub(crate) type WarmPair = (Option<Box<dyn SpeechToText>>, Option<Box<dyn Vad>>);

pub(crate) fn take_warm(key: &str) -> WarmPair {
    let mut w = WARM.lock();
    let Some(warm) = w.as_mut() else { return (None, None) };
    let stt = match warm.stt.take() {
        Some((k, e)) if k == key => Some(e),
        other => {
            warm.stt = other;
            None
        }
    };
    (stt, warm.vad.take())
}

/// Return engines to the cache for the next call or dictation.
pub(crate) fn park_warm(key: String, stt: Option<Box<dyn SpeechToText>>, vad: Option<Box<dyn Vad>>) {
    let mut w = WARM.lock();
    let warm = w.get_or_insert_with(|| Warm { stt: None, vad: None, turn: None, at: Instant::now() });
    if let Some(e) = stt {
        warm.stt = Some((key, e));
    }
    if vad.is_some() {
        warm.vad = vad;
    }
    warm.at = Instant::now();
    drop(w);
    reap_later();
}

fn reap_later() {
    std::thread::Builder::new()
        .name("voice-reaper".into())
        .spawn(|| {
            std::thread::sleep(WARM_TTL + Duration::from_secs(1));
            let mut w = WARM.lock();
            if w.as_ref().is_some_and(|w| w.at.elapsed() >= WARM_TTL) {
                *w = None;
            }
        })
        .ok();
}

pub(crate) fn stt_key(c: &SttChoice) -> String {
    match c {
        #[cfg(feature = "local")]
        SttChoice::Moonshine(root, size) => format!("{}:{}", size.id(), root.display()),
        #[cfg(feature = "local")]
        SttChoice::Parakeet(dir) => format!("parakeet:{}", dir.display()),
        SttChoice::Cloud { base_url, model, .. } => format!("cloud:{base_url}:{model}"),
        SttChoice::Deepgram { model, .. } => format!("deepgram:{model}"),
    }
}

pub(crate) fn load_stt(c: &SttChoice) -> Result<Box<dyn SpeechToText>> {
    Ok(match c {
        #[cfg(feature = "local")]
        SttChoice::Moonshine(root, size) => Box::new(crate::stt::Moonshine::load(root, *size)?),
        #[cfg(feature = "local")]
        SttChoice::Parakeet(dir) => Box::new(crate::stt::Parakeet::load(dir)?),
        SttChoice::Cloud { base_url, api_key, model } => {
            Box::new(crate::stt::OpenAiCompatible::new(base_url, api_key, model)?)
        }
        SttChoice::Deepgram { api_key, model } => Box::new(crate::stt::Deepgram::new(api_key, model)?),
    })
}

pub(crate) fn load_vad(root: &std::path::Path) -> Box<dyn Vad> {
    #[cfg(feature = "local")]
    if crate::models::installed(root, &crate::models::SILERO) {
        match crate::vad::SileroVad::load(&crate::models::path(root, &crate::models::SILERO)) {
            Ok(v) => return Box::new(v),
            Err(e) => tracing::warn!("silero unavailable, using energy VAD: {e:#}"),
        }
    }
    let _ = root;
    Box::new(crate::vad::EnergyVad::default())
}

fn load_turn(root: &std::path::Path) -> Box<dyn TurnDetector> {
    #[cfg(feature = "local")]
    if crate::models::installed(root, &crate::models::SMART_TURN) {
        match crate::turn::SmartTurn::load(&crate::models::path(root, &crate::models::SMART_TURN)) {
            Ok(t) => return Box::new(t),
            Err(e) => tracing::warn!("smart turn unavailable, using pauses: {e:#}"),
        }
    }
    let _ = root;
    Box::new(crate::turn::PauseTurn::default())
}

/// A running call. Dropping it ends the call.
pub struct Call {
    ctl: Sender<Ctl>,
    pub events: Receiver<Event>,
    ended: Arc<AtomicBool>,
}

impl Call {
    /// Start a call. Returns at once; engines load on the call's threads and
    /// [`Event::Ready`] arrives when the microphone is live.
    pub fn start(config: CallConfig) -> Self {
        let (ctl, ctl_rx) = unbounded();
        let (ev, events) = unbounded();
        let ended = Arc::new(AtomicBool::new(false));
        let e2 = ended.clone();
        std::thread::Builder::new()
            .name("voice-call".into())
            .spawn(move || {
                if let Err(err) = run(config, ctl_rx, ev.clone(), e2.clone()) {
                    let _ = ev.send(Event::Error(format!("{err:#}")));
                }
                e2.store(true, Ordering::Release);
                let _ = ev.send(Event::Ended);
            })
            .expect("spawning the voice call thread");
        Self { ctl, events, ended }
    }

    /// Speak one clause (queued behind what is playing).
    pub fn say(&self, text: impl Into<String>) {
        let _ = self.ctl.send(Ctl::Say(text.into()));
    }
    pub fn reply_text_started(&self) {
        let _ = self.ctl.send(Ctl::ReplyText);
    }
    pub fn silence(&self) {
        let _ = self.ctl.send(Ctl::Silence);
    }
    pub fn mute(&self, on: bool) {
        let _ = self.ctl.send(Ctl::Mute(on));
    }
    pub fn push_to_talk(&self, held: bool) {
        let _ = self.ctl.send(Ctl::Ptt(held));
    }
    pub fn set_rate(&self, rate: f32) {
        let _ = self.ctl.send(Ctl::Rate(rate));
    }
    pub fn set_voice(&self, voice: Option<String>) {
        let _ = self.ctl.send(Ctl::Voice(voice));
    }
    pub fn end(&self) {
        let _ = self.ctl.send(Ctl::End);
    }
    pub fn is_ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        let _ = self.ctl.send(Ctl::End);
    }
}

/// Microphone samples, written by the device callback, read by `voice-call`.
#[derive(Default)]
pub(crate) struct Mic {
    pub(crate) buf: Mutex<Vec<f32>>,
    pub(crate) dropped: AtomicU64,
    pub(crate) failed: AtomicBool,
}

pub(crate) fn open_mic(device: Option<&str>, mic: Arc<Mic>) -> Result<(cpal::Stream, u32)> {
    let host = cpal::default_host();
    let device = device
        .and_then(|id| id.parse::<cpal::DeviceId>().ok())
        .and_then(|id| host.device_by_id(&id))
        .or_else(|| host.default_input_device())
        .context("No microphone available")?;
    let config = device.default_input_config()?;
    let rate = config.sample_rate();
    let channels = config.channels() as usize;
    let m2 = mic.clone();
    let err = move |_| m2.failed.store(true, Ordering::Release);
    macro_rules! stream {
        ($t:ty) => {{
            let m = mic.clone();
            device.build_input_stream(
                &config.clone().into(),
                move |d: &[$t], _| {
                    use cpal::Sample;
                    match m.buf.try_lock() {
                        Some(mut b) if b.len() < rate as usize * 2 => {
                            for f in d.chunks_exact(channels.max(1)) {
                                b.push(f.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32);
                            }
                        }
                        _ => {
                            m.dropped.fetch_add(d.len() as u64, Ordering::Relaxed);
                        }
                    }
                },
                err,
                None,
            )?
        }};
    }
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => stream!(f32),
        cpal::SampleFormat::I16 => stream!(i16),
        cpal::SampleFormat::U16 => stream!(u16),
        cpal::SampleFormat::I32 => stream!(i32),
        f => anyhow::bail!("Unsupported microphone format {f:?}"),
    };
    stream.play()?;
    Ok((stream, rate))
}

enum SttJob {
    Partial(Vec<f32>),
    Speculate(Vec<f32>),
    Final { audio: Vec<f32>, timings: TurnTimings, last_voiced: Option<Instant>, speculated: bool },
}

enum SttOut {
    Ready(String),
    Failed(String),
    Partial(String),
    Final { text: String, timings: TurnTimings },
}

fn run(config: CallConfig, ctl: Receiver<Ctl>, ev: Sender<Event>, ended: Arc<AtomicBool>) -> Result<()> {
    let started = Instant::now();
    let _ = ev.send(Event::State(State::Loading));
    // Dictation's cached Parakeet would double the model's memory.
    #[cfg(feature = "local")]
    zeron_voice::unload();

    let warm = WARM.lock().take();
    let key = stt_key(&config.stt);
    let (mut warm_stt, warm_vad, warm_turn) = match warm {
        Some(w) => (w.stt.filter(|(k, _)| *k == key).map(|(_, s)| s), w.vad, w.turn),
        None => (None, None, None),
    };
    let was_warm = warm_stt.is_some();

    // Speech-to-text on its own thread; a cold load happens there too.
    let (stt_tx, stt_rx) = bounded::<SttJob>(2);
    let (stt_out_tx, stt_out) = unbounded::<SttOut>();
    let stt_choice = config.stt;
    let stt_ended = ended.clone();
    let stt_thread = std::thread::Builder::new().name("voice-stt".into()).spawn(move || {
        let mut engine = match warm_stt.take() {
            Some(e) => e,
            None => match load_stt(&stt_choice) {
                Ok(e) => e,
                Err(e) => {
                    let _ = stt_out_tx.send(SttOut::Failed(format!("{e:#}")));
                    return None;
                }
            },
        };
        let _ = stt_out_tx.send(SttOut::Ready(engine.name()));
        // The transcript of the latest pause, and how long it took.
        let mut speculative: Option<(String, f32)> = None;
        while let Ok(job) = stt_rx.recv() {
            if stt_ended.load(Ordering::Acquire) {
                break;
            }
            match job {
                SttJob::Partial(audio) => {
                    // A final may be queued behind this partial; skip it.
                    if !stt_rx.is_empty() {
                        continue;
                    }
                    if let Ok(t) = engine.partial(&audio)
                        && !t.is_empty()
                    {
                        let _ = stt_out_tx.send(SttOut::Partial(t));
                    }
                }
                SttJob::Speculate(audio) => {
                    let t = Instant::now();
                    speculative = engine.finish(&audio).ok().map(|text| (text, t.elapsed().as_secs_f32() * 1000.0));
                }
                SttJob::Final { audio, mut timings, last_voiced, speculated } => {
                    if speculated && let Some((text, ms)) = speculative.take() {
                        timings.stt_final_ms = ms;
                        timings.end_of_speech_to_text_ms =
                            last_voiced.map_or(ms, |l| l.elapsed().as_secs_f32() * 1000.0);
                        let _ = stt_out_tx.send(SttOut::Final { text, timings });
                        continue;
                    }
                    speculative = None;
                    let t = Instant::now();
                    match engine.finish(&audio) {
                        Ok(text) => {
                            timings.stt_final_ms = t.elapsed().as_secs_f32() * 1000.0;
                            timings.end_of_speech_to_text_ms =
                                last_voiced.map_or(timings.stt_final_ms, |l| l.elapsed().as_secs_f32() * 1000.0);
                            let _ = stt_out_tx.send(SttOut::Final { text, timings });
                        }
                        Err(e) => {
                            let _ = stt_out_tx.send(SttOut::Failed(format!("{e:#}")));
                        }
                    }
                }
            }
        }
        Some(engine)
    })?;

    // Text-to-speech on its own thread.
    let generation = Arc::new(AtomicU64::new(0));
    let cancel = Arc::new(Mutex::new(Arc::new(AtomicBool::new(false))));
    let (tts_tx, tts_rx) = unbounded::<(u64, String, SpeechOptions)>();
    let (pcm_tx, pcm_rx) = unbounded::<(u64, crate::engines::Pcm, Instant)>();
    let cloud: Option<Box<dyn TextToSpeech>> = match &config.tts {
        TtsChoice::System => None,
        TtsChoice::Cloud { base_url, api_key, model, voice } => {
            Some(Box::new(crate::tts::OpenAiCompatible::new(base_url, api_key, model, voice)?))
        }
        TtsChoice::ElevenLabs { api_key, voice, model } => {
            Some(Box::new(crate::tts::ElevenLabs::new(api_key, voice, model)?))
        }
        TtsChoice::Stream(provider) => Some(provider.build()?),
    };
    // A cloud voice that fails mid-call (offline, out of credits) hands the
    // clause to the system voice and says so once, so the call never goes mute.
    let mut tts: Box<dyn TextToSpeech> = match cloud {
        None => Box::new(crate::tts::System::new()),
        Some(primary) => {
            let ev_tts = ev.clone();
            Box::new(crate::tts_stream::WithFallback::new(
                primary,
                Box::new(move |msg| {
                    let _ = ev_tts.send(Event::Error(msg));
                }),
            ))
        }
    };
    let tts_name = tts.name();
    let (g2, c2) = (generation.clone(), cancel.clone());
    let tts_thread = std::thread::Builder::new().name("voice-tts".into()).spawn(move || {
        while let Ok((gen_, text, opts)) = tts_rx.recv() {
            if gen_ != g2.load(Ordering::Acquire) {
                continue;
            }
            let flag = c2.lock().clone();
            let t = Instant::now();
            // Each chunk goes to the speaker as it arrives; a barge-in
            // (new generation) stops reading the rest of the clause.
            let result = tts.synthesize_stream(&text, &opts, &flag, &mut |pcm| {
                gen_ == g2.load(Ordering::Acquire) && pcm_tx.send((gen_, pcm, t)).is_ok()
            });
            if let Err(e) = result {
                tracing::warn!("speech failed: {e:#}");
            }
        }
    })?;

    // The speaker opens on its own thread (device start-up can take a few
    // hundred milliseconds); listening does not wait for it.
    let (spk_tx, spk_rx) = bounded::<Result<Arc<crate::player::Shared>, String>>(1);
    let spk_done = Arc::new(AtomicBool::new(false));
    let output = match &config.output {
        Output::Device(d) => Some(d.clone()),
        Output::Silent => None,
    };
    let done = spk_done.clone();
    let speaker_thread = std::thread::Builder::new().name("voice-speaker".into()).spawn(move || {
        let Some(output) = output else { return };
        match crate::player::Player::start(output.as_deref()) {
            Ok(p) => {
                let _ = spk_tx.send(Ok(p.shared()));
                while !done.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(50));
                }
                p.stop();
            }
            Err(e) => {
                let _ = spk_tx.send(Err(format!("{e:#}")));
            }
        }
    })?;
    let mut speaker: Option<Arc<crate::player::Shared>> = None;
    let silent = vec![0.0f32; crate::FRAME];

    // Microphone, VAD, echo canceller, turn detector.
    let mic = Arc::new(Mic {
        buf: Mutex::new(Vec::with_capacity(48_000)),
        dropped: AtomicU64::new(0),
        failed: AtomicBool::new(false),
    });
    let (stream, mic_rate, pushed) = match config.input {
        Input::Device(ref d) => {
            let (s, r) = open_mic(d.as_deref(), mic.clone())?;
            (Some(s), r, None)
        }
        Input::Stream { rate, ref samples } => (None, rate, Some(samples.clone())),
    };
    let mut to16k = crate::resample::Stream::new(mic_rate, crate::RATE)?;
    let vad = warm_vad.unwrap_or_else(|| load_vad(&config.root));
    let turn = warm_turn.unwrap_or_else(|| load_turn(&config.root));
    let aec: Option<Box<dyn EchoCanceller>> =
        if config.echo_cancel { Some(Box::new(crate::aec::Nlms::default())) } else { None };
    let mut listener = Listener::new(vad, aec, turn);
    let (vad_name, aec_name, turn_name) = listener.names();
    let (vad_name, turn_name) = (vad_name.to_string(), turn_name.to_string());

    let mut opts = SpeechOptions { rate: config.rate, voice: config.voice.clone() };
    let mut muted = false;
    let mut ptt_held = false;
    let mut stt_ready = false;
    let mut pending16: Vec<f32> = Vec::with_capacity(crate::FRAME * 4);
    let mut reply_text_at: Option<Instant> = None;
    let mut state = State::Loading;
    let mut last_level = Instant::now();
    let mut mic_level = 0f32;
    let set_state = |s: State, state: &mut State| {
        if *state != s {
            *state = s;
            let _ = ev.send(Event::State(s));
        }
    };

    'call: loop {
        // Controls.
        loop {
            match ctl.try_recv() {
                Ok(Ctl::Say(text)) => {
                    let _ = tts_tx.send((generation.load(Ordering::Acquire), text, opts.clone()));
                }
                Ok(Ctl::ReplyText) => reply_text_at = Some(Instant::now()),
                Ok(Ctl::Silence) => {
                    generation.fetch_add(1, Ordering::AcqRel);
                    cancel.lock().store(true, Ordering::Release);
                    *cancel.lock() = Arc::new(AtomicBool::new(false));
                    if let Some(s) = &speaker {
                        s.stop();
                    }
                }
                Ok(Ctl::Mute(on)) => {
                    muted = on;
                    if on {
                        listener.reset();
                    }
                }
                Ok(Ctl::Ptt(held)) => {
                    ptt_held = held;
                    if held {
                        generation.fetch_add(1, Ordering::AcqRel);
                        cancel.lock().store(true, Ordering::Release);
                        *cancel.lock() = Arc::new(AtomicBool::new(false));
                        let dropped = speaker.as_ref().map_or(0, |s| s.stop());
                        listener.begin();
                        if dropped > 0 {
                            let _ = ev.send(Event::BargeIn { dropped_ms: dropped });
                        }
                    } else if (config.push_to_talk || muted)
                        && let Some(h) = listener.force_end()
                    {
                        handle(h, &ev, &stt_tx, &listener);
                    }
                }
                Ok(Ctl::Rate(r)) => opts.rate = r,
                Ok(Ctl::Voice(v)) => opts.voice = v,
                Ok(Ctl::End) | Err(TryRecvError::Disconnected) => break 'call,
                Err(TryRecvError::Empty) => break,
            }
        }
        if speaker.is_none()
            && let Ok(s) = spk_rx.try_recv()
        {
            match s {
                Ok(s) => speaker = Some(s),
                Err(e) => {
                    let _ = ev.send(Event::Error(format!(
                        "Speaker unavailable ({e}); you'll see replies but not hear them."
                    )));
                }
            }
        }
        let playing = speaker.as_ref().is_some_and(|s| s.playing());
        // Speech results.
        while let Ok(out) = stt_out.try_recv() {
            match out {
                SttOut::Ready(name) => {
                    stt_ready = true;
                    let _ = ev.send(Event::Ready {
                        startup_ms: started.elapsed().as_millis() as u32,
                        warm: was_warm,
                        stt: name,
                        tts: tts_name.clone(),
                        vad: vad_name.clone(),
                        turn: turn_name.clone(),
                        echo_cancel: aec_name.is_some(),
                    });
                    set_state(if muted { State::Muted } else { State::Listening }, &mut state);
                    // A soft cue: the call is listening (hands-free, eyes elsewhere).
                    if config.earcons
                        && let Some(s) = &speaker
                    {
                        s.earcon();
                    }
                }
                SttOut::Failed(e) => {
                    let _ = ev.send(Event::Error(e));
                    if !stt_ready {
                        break 'call;
                    }
                }
                SttOut::Partial(t) => {
                    listener.set_partial(t.clone());
                    let _ = ev.send(Event::Partial(t));
                }
                SttOut::Final { text, timings } => {
                    if text.trim().is_empty() {
                        set_state(State::Listening, &mut state);
                    } else {
                        set_state(State::Thinking, &mut state);
                        let _ = ev.send(Event::Turn { text, timings });
                        // "I heard you": the turn is in, before any reply.
                        if config.earcons
                            && let Some(s) = &speaker
                        {
                            s.earcon();
                        }
                    }
                }
            }
        }
        // Rendered speech.
        while let Ok((gen_, pcm, _)) = pcm_rx.try_recv() {
            // No speaker (replies shown, not spoken): a reply is done with
            // as soon as it is rendered, so the call goes back to listening.
            if gen_ == generation.load(Ordering::Acquire) && speaker.is_none() && state == State::Thinking {
                set_state(if muted { State::Muted } else { State::Listening }, &mut state);
            }
            if gen_ == generation.load(Ordering::Acquire)
                && let Some(s) = &speaker
            {
                s.play(&pcm);
                if let Some(at) = reply_text_at.take() {
                    let _ = ev.send(Event::ReplyAudio { text_to_audio_ms: at.elapsed().as_secs_f32() * 1000.0 });
                }
                set_state(State::Speaking, &mut state);
            }
        }
        if state == State::Speaking && !speaker.as_ref().is_some_and(|s| s.playing()) {
            set_state(if muted { State::Muted } else { State::Listening }, &mut state);
        }
        if mic.failed.load(Ordering::Acquire) {
            let _ = ev
                .send(Event::Error("The microphone disconnected. The call ended; your threads are unaffected.".into()));
            break 'call;
        }
        // Microphone (or pushed audio).
        if let Some(rx) = &pushed {
            loop {
                match rx.try_recv() {
                    Ok(chunk) => mic.buf.lock().extend_from_slice(&chunk),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        let _ = ev.send(Event::Error("The audio stream ended.".into()));
                        break 'call;
                    }
                }
            }
        }
        let raw = std::mem::take(&mut *mic.buf.lock());
        if !raw.is_empty() {
            pending16.extend(to16k.push(&raw));
        }
        while pending16.len() >= crate::FRAME {
            let mut frame: Vec<f32> = pending16.drain(..crate::FRAME).collect();
            let reference = speaker.as_ref().map_or_else(|| silent.clone(), |s| s.take_reference(crate::FRAME));
            mic_level = mic_level.max(crate::vad::rms(&frame));
            if !stt_ready || (muted && !ptt_held) || (config.push_to_talk && !ptt_held) {
                continue;
            }
            if let Some(h) = listener.feed(&mut frame, &reference, playing) {
                if let Heard::Started { barge_in } = h {
                    if barge_in || playing {
                        generation.fetch_add(1, Ordering::AcqRel);
                        cancel.lock().store(true, Ordering::Release);
                        *cancel.lock() = Arc::new(AtomicBool::new(false));
                        let dropped = speaker.as_ref().map_or(0, |s| s.stop());
                        let _ = ev.send(Event::BargeIn { dropped_ms: dropped });
                    }
                    reply_text_at = None;
                    set_state(State::Hearing, &mut state);
                } else if matches!(h, Heard::Discarded) {
                    set_state(State::Listening, &mut state);
                }
                handle(h, &ev, &stt_tx, &listener);
            }
        }
        if last_level.elapsed() >= Duration::from_millis(50) {
            last_level = Instant::now();
            let out = speaker.as_ref().map_or(0.0, |s| s.take_level());
            let _ = ev.send(Event::Level { mic: std::mem::take(&mut mic_level), speaker: out });
        }
        std::thread::sleep(Duration::from_millis(8));
    }

    // Tear down: devices first, then park the engines for a warm restart.
    ended.store(true, Ordering::Release);
    drop(stream);
    if let Some(s) = &speaker {
        s.stop();
    }
    spk_done.store(true, Ordering::Release);
    let _ = speaker_thread.join();
    drop(tts_tx);
    drop(stt_tx);
    let stt_engine = stt_thread.join().ok().flatten();
    let _ = tts_thread.join();
    let Listener { vad, turn, .. } = listener;
    *WARM.lock() =
        Some(Warm { stt: stt_engine.map(|e| (key, e)), vad: Some(vad), turn: Some(turn), at: Instant::now() });
    reap_later();
    Ok(())
}

fn handle(h: Heard, ev: &Sender<Event>, stt: &Sender<SttJob>, listener: &Listener) {
    match h {
        Heard::WantPartial(audio) => {
            let _ = stt.try_send(SttJob::Partial(audio));
        }
        Heard::Speculate(audio) => {
            let _ = stt.try_send(SttJob::Speculate(audio));
        }
        Heard::Ended { audio, timings, speculated } => {
            let _ = stt.send(SttJob::Final { audio, timings, last_voiced: listener.last_voiced_at, speculated });
        }
        Heard::Started { .. } | Heard::Discarded => {}
    }
    let _ = ev;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turn::PauseTurn;
    use crate::vad::EnergyVad;

    /// A voiced-sounding signal: a few harmonics with a slow envelope.
    fn voice(ms: u32) -> Vec<f32> {
        let n = (crate::RATE * ms / 1000) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / crate::RATE as f32;
                let env = 0.6 + 0.4 * (t * 3.0 * std::f32::consts::TAU).sin().abs();
                env * 0.15
                    * ((t * 180.0 * std::f32::consts::TAU).sin() + 0.5 * (t * 360.0 * std::f32::consts::TAU).sin())
            })
            .collect()
    }

    fn quiet(ms: u32) -> Vec<f32> {
        let n = (crate::RATE * ms / 1000) as usize;
        (0..n).map(|i| 0.001 * (((i * 7919) % 101) as f32 / 101.0 - 0.5)).collect()
    }

    fn run(listener: &mut Listener, audio: &[f32], speaking: bool) -> Vec<Heard> {
        let mut out = Vec::new();
        for f in audio.chunks_exact(crate::FRAME) {
            let mut f = f.to_vec();
            if let Some(h) = listener.feed(&mut f, &vec![0.0; crate::FRAME], speaking) {
                out.push(h);
            }
        }
        out
    }

    fn listener() -> Listener {
        Listener::new(Box::new(EnergyVad::default()), None, Box::new(PauseTurn::default()))
    }

    #[test]
    fn a_spoken_turn_starts_and_ends_after_the_pause() {
        let mut l = listener();
        let mut audio = quiet(1000);
        audio.extend(voice(1500));
        audio.extend(quiet(1200));
        let heard = run(&mut l, &audio, false);
        assert_eq!(heard.first(), Some(&Heard::Started { barge_in: false }));
        let Some(Heard::Ended { audio: turn, timings, speculated }) =
            heard.iter().find(|h| matches!(h, Heard::Ended { .. }))
        else {
            panic!("no turn end in {heard:?}");
        };
        // The turn holds the speech (plus pre-roll), not the whole recording.
        let ms = turn.len() as u32 * 1000 / crate::RATE;
        assert!((1500..2600).contains(&ms), "{ms} ms");
        assert!(timings.speech_ms >= 1300, "{timings:?}");
        assert!((550..700).contains(&timings.endpoint_silence_ms), "{timings:?}");
        // The pause was transcribed early and nothing was said after it.
        assert!(*speculated);
        assert!(heard.iter().any(|h| matches!(h, Heard::Speculate(_))));
    }

    #[test]
    fn speech_after_a_speculated_pause_invalidates_it() {
        let mut l = listener();
        let mut audio = quiet(500);
        audio.extend(voice(800));
        audio.extend(quiet(300)); // a pause long enough to speculate, short of the turn end
        audio.extend(voice(800));
        audio.extend(quiet(1200));
        let heard = run(&mut l, &audio, false);
        assert_eq!(heard.iter().filter(|h| matches!(h, Heard::Speculate(_))).count(), 2, "{heard:?}");
        assert!(matches!(heard.last(), Some(Heard::Ended { speculated: true, .. })), "the second pause was the end");
    }

    #[test]
    fn a_click_is_not_a_turn() {
        let mut l = listener();
        let mut audio = quiet(800);
        audio.extend(voice(96));
        audio.extend(quiet(1500));
        let heard = run(&mut l, &audio, false);
        assert!(!heard.iter().any(|h| matches!(h, Heard::Ended { .. })), "{heard:?}");
    }

    #[test]
    fn barging_in_takes_longer_than_starting_a_turn() {
        // A 96 ms burst: enough to start a turn when Dove is quiet...
        let mut blip = quiet(1000);
        blip.extend(voice(96));
        let mut l = listener();
        assert_eq!(run(&mut l, &blip, false).first(), Some(&Heard::Started { barge_in: false }));
        // ...but not to cut Dove off (echo residue and clicks come in bursts).
        let mut l = listener();
        assert!(run(&mut l, &blip, true).is_empty());
        // Sustained speech over Dove does barge in.
        let heard = run(&mut l, &voice(400), true);
        assert_eq!(heard.first(), Some(&Heard::Started { barge_in: true }));
    }

    #[test]
    fn long_speech_asks_for_partials_and_is_capped() {
        let mut l = listener();
        l.cfg.max_turn_ms = 4000;
        let mut audio = quiet(500);
        audio.extend(voice(6000));
        let heard = run(&mut l, &audio, false);
        assert!(heard.iter().filter(|h| matches!(h, Heard::WantPartial(_))).count() >= 2);
        assert!(heard.iter().any(|h| matches!(h, Heard::Ended { .. })), "a 4 s cap ends a 6 s monologue");
    }

    #[test]
    fn push_to_talk_ends_on_release() {
        let mut l = listener();
        l.begin();
        run(&mut l, &voice(800), false);
        assert!(matches!(l.force_end(), Some(Heard::Ended { .. })));
        assert!(!l.in_turn());
        // Released with nothing said.
        let mut l = listener();
        assert_eq!(l.force_end(), None);
    }
}
