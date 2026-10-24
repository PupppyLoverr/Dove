//! Live dictation: words appear while you talk, not after you stop.
//!
//! The microphone is cut into phrases by voice activity. While a phrase is
//! being spoken, its transcript is refreshed every [`DictationConfig::partial_every_ms`]
//! (the *live* text, which may still change); a pause commits it and starts
//! the next phrase. Stopping commits whatever was being said. Everything runs
//! on this computer with the same models as Voice Mode (Moonshine or
//! Parakeet), shared through the same warm cache, and no audio is stored.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded, unbounded};

use crate::engines::{SpeechToText, Vad};
use crate::pipeline::{Input, SttChoice};

pub struct DictationConfig {
    /// Dove's data folder (models live under `voice/`).
    pub root: std::path::PathBuf,
    pub stt: SttChoice,
    pub input: Input,
    /// How often the phrase being spoken is re-transcribed.
    pub partial_every_ms: u32,
    /// The pause that ends a phrase.
    pub pause_ms: u32,
    /// A phrase is committed after this long even without a pause.
    pub max_phrase_ms: u32,
}

impl DictationConfig {
    pub fn new(root: std::path::PathBuf, stt: SttChoice, input: Input) -> Self {
        Self { root, stt, input, partial_every_ms: 450, pause_ms: 650, max_phrase_ms: 20_000 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The microphone is live.
    Ready {
        stt: String,
        startup_ms: u32,
    },
    /// `committed` won't change; `live` is the phrase being spoken.
    Text {
        committed: String,
        live: String,
    },
    /// Peak microphone level (0–1) since the last one.
    Level(f32),
    Error(String),
    /// Dictation is over; `text` is everything committed.
    Ended {
        text: String,
    },
}

impl Event {
    /// The whole dictated text a [`Event::Text`] stands for.
    pub fn joined(committed: &str, live: &str) -> String {
        join(committed, live)
    }
}

fn join(a: &str, b: &str) -> String {
    let (a, b) = (a.trim(), b.trim());
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        _ => format!("{a} {b}"),
    }
}

enum Ctl {
    Stop,
    Cancel,
}

/// A running dictation. Dropping it cancels (nothing more is committed).
pub struct LiveDictation {
    ctl: Sender<Ctl>,
    pub events: Receiver<Event>,
    ended: Arc<AtomicBool>,
}

impl LiveDictation {
    /// Start listening. Returns at once; [`Event::Ready`] arrives when the
    /// model is loaded and the microphone is open.
    pub fn start(config: DictationConfig) -> Self {
        Self::spawn(config, None)
    }

    /// For tests: a ready-made engine instead of loading `config.stt`.
    pub fn with_engine(config: DictationConfig, engine: Box<dyn SpeechToText>) -> Self {
        Self::spawn(config, Some(engine))
    }

    fn spawn(config: DictationConfig, engine: Option<Box<dyn SpeechToText>>) -> Self {
        let (ctl, ctl_rx) = unbounded();
        let (ev, events) = unbounded();
        let ended = Arc::new(AtomicBool::new(false));
        let e2 = ended.clone();
        std::thread::Builder::new()
            .name("voice-dictation".into())
            .spawn(move || {
                let text = match run(config, engine, ctl_rx, ev.clone()) {
                    Ok(text) => text,
                    Err(err) => {
                        let _ = ev.send(Event::Error(format!("{err:#}")));
                        String::new()
                    }
                };
                e2.store(true, Ordering::Release);
                let _ = ev.send(Event::Ended { text });
            })
            .expect("spawning the dictation thread");
        Self { ctl, events, ended }
    }

    /// Commit what is being said and end.
    pub fn stop(&self) {
        let _ = self.ctl.send(Ctl::Stop);
    }

    /// End without transcribing the phrase in progress.
    pub fn cancel(&self) {
        let _ = self.ctl.send(Ctl::Cancel);
    }

    pub fn is_ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }
}

impl Drop for LiveDictation {
    fn drop(&mut self) {
        let _ = self.ctl.send(Ctl::Cancel);
    }
}

enum Job {
    Partial { phrase: u64, audio: Vec<f32> },
    Final { phrase: u64, audio: Vec<f32> },
}

enum Done {
    Ready(String),
    Failed(String),
    Partial { phrase: u64, text: String },
    Final { phrase: u64, text: String },
}

/// Phrase segmentation, separate from devices and models so it can be
/// tested on synthetic audio.
pub(crate) struct Segmenter {
    vad: Box<dyn Vad>,
    pause_frames: u32,
    partial_frames: u32,
    max_frames: u32,
    /// Audio just before speech starts, so the first syllable isn't clipped.
    preroll: std::collections::VecDeque<Vec<f32>>,
    phrase: Vec<f32>,
    in_phrase: bool,
    voiced_frames: u32,
    quiet_frames: u32,
    since_partial: u32,
    onset: u32,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Cut {
    Partial(Vec<f32>),
    Phrase(Vec<f32>),
}

const PREROLL_FRAMES: usize = 8; // 256 ms
const ONSET_FRAMES: u32 = 2; // 64 ms of speech starts a phrase

impl Segmenter {
    pub(crate) fn new(vad: Box<dyn Vad>, partial_ms: u32, pause_ms: u32, max_ms: u32) -> Self {
        let frames = |ms: u32| (ms / crate::pipeline::FRAME_MS).max(1);
        Self {
            vad,
            pause_frames: frames(pause_ms),
            partial_frames: frames(partial_ms),
            max_frames: frames(max_ms),
            preroll: Default::default(),
            phrase: Vec::new(),
            in_phrase: false,
            voiced_frames: 0,
            quiet_frames: 0,
            since_partial: 0,
            onset: 0,
        }
    }

    pub(crate) fn feed(&mut self, frame: &[f32]) -> Option<Cut> {
        let voiced = self.vad.speech(frame) >= 0.5;
        if !self.in_phrase {
            self.preroll.push_back(frame.to_vec());
            if self.preroll.len() > PREROLL_FRAMES {
                self.preroll.pop_front();
            }
            self.onset = if voiced { self.onset + 1 } else { 0 };
            if self.onset >= ONSET_FRAMES {
                self.in_phrase = true;
                self.phrase = self.preroll.drain(..).flatten().collect();
                self.voiced_frames = self.onset;
                self.quiet_frames = 0;
                self.since_partial = 0;
            }
            return None;
        }
        self.phrase.extend_from_slice(frame);
        self.since_partial += 1;
        if voiced {
            self.voiced_frames += 1;
            self.quiet_frames = 0;
        } else {
            self.quiet_frames += 1;
        }
        let frames = (self.phrase.len() / crate::FRAME) as u32;
        if self.quiet_frames >= self.pause_frames || frames >= self.max_frames {
            return self.take();
        }
        if self.since_partial >= self.partial_frames && self.voiced_frames >= 6 {
            self.since_partial = 0;
            return Some(Cut::Partial(self.phrase.clone()));
        }
        None
    }

    /// End the phrase in progress (on a pause, or when dictation stops).
    pub(crate) fn take(&mut self) -> Option<Cut> {
        if !self.in_phrase {
            return None;
        }
        self.in_phrase = false;
        self.onset = 0;
        let mut audio = std::mem::take(&mut self.phrase);
        // Trailing silence only costs recognition time.
        let keep_quiet = (self.quiet_frames.min(6) as usize) * crate::FRAME;
        let cut = (self.quiet_frames as usize * crate::FRAME).saturating_sub(keep_quiet);
        audio.truncate(audio.len().saturating_sub(cut));
        let voiced = self.voiced_frames;
        self.voiced_frames = 0;
        self.quiet_frames = 0;
        (voiced >= 4).then_some(Cut::Phrase(audio))
    }
}

fn run(
    config: DictationConfig,
    engine: Option<Box<dyn SpeechToText>>,
    ctl: Receiver<Ctl>,
    ev: Sender<Event>,
) -> Result<String> {
    let started = Instant::now();
    let key = crate::pipeline::stt_key(&config.stt);
    let (warm_stt, warm_vad) = crate::pipeline::take_warm(&key);

    // Recognition on its own thread: the microphone loop never waits on it.
    let (job_tx, job_rx) = bounded::<Job>(8);
    let (done_tx, done_rx) = unbounded::<Done>();
    let stt_choice = config.stt;
    let stt_thread = std::thread::Builder::new().name("voice-dictation-stt".into()).spawn(move || {
        let mut engine = match engine.or(warm_stt) {
            Some(e) => e,
            None => match crate::pipeline::load_stt(&stt_choice) {
                Ok(e) => e,
                Err(e) => {
                    let _ = done_tx.send(Done::Failed(format!("{e:#}")));
                    return None;
                }
            },
        };
        let _ = done_tx.send(Done::Ready(engine.name()));
        while let Ok(job) = job_rx.recv() {
            match job {
                Job::Partial { phrase, audio } => {
                    // A newer partial or a final is waiting: this one is stale.
                    if !job_rx.is_empty() {
                        continue;
                    }
                    if let Ok(text) = engine.partial(&audio) {
                        let _ = done_tx.send(Done::Partial { phrase, text: crate::stt::collapse_repeats(&text) });
                    }
                }
                Job::Final { phrase, audio } => match engine.finish(&audio) {
                    Ok(text) => {
                        let _ = done_tx.send(Done::Final { phrase, text: crate::stt::collapse_repeats(&text) });
                    }
                    Err(e) => {
                        let _ = done_tx.send(Done::Failed(format!("{e:#}")));
                    }
                },
            }
        }
        Some(engine)
    })?;

    let mic = Arc::new(crate::pipeline::Mic::default());
    let (stream, mic_rate, pushed) = match config.input {
        Input::Device(ref d) => {
            let (s, r) = crate::pipeline::open_mic(d.as_deref(), mic.clone())?;
            (Some(s), r, None)
        }
        Input::Stream { rate, ref samples } => (None, rate, Some(samples.clone())),
    };
    let mut to16k = crate::resample::Stream::new(mic_rate, crate::RATE)?;
    let vad = warm_vad.unwrap_or_else(|| crate::pipeline::load_vad(&config.root));
    let mut seg = Segmenter::new(vad, config.partial_every_ms, config.pause_ms, config.max_phrase_ms);

    let mut ready = false;
    let mut phrase: u64 = 0;
    // Committed phrases in order; a later final can't overtake an earlier one.
    let mut finals: std::collections::BTreeMap<u64, String> = Default::default();
    let mut awaiting: std::collections::BTreeSet<u64> = Default::default();
    let mut live = String::new();
    let mut pending: Vec<f32> = Vec::new();
    let mut level = 0f32;
    let mut last_level = Instant::now();
    let mut stopping = false;
    let mut stop_deadline: Option<Instant> = None;
    let mut cancelled = false;
    let committed =
        |finals: &std::collections::BTreeMap<u64, String>| finals.values().fold(String::new(), |acc, t| join(&acc, t));

    loop {
        match ctl.try_recv() {
            Ok(Ctl::Stop) => stopping = true,
            Ok(Ctl::Cancel) | Err(TryRecvError::Disconnected) => {
                cancelled = true;
                break;
            }
            Err(TryRecvError::Empty) => {}
        }
        while let Ok(done) = done_rx.try_recv() {
            match done {
                Done::Ready(name) => {
                    ready = true;
                    let _ = ev.send(Event::Ready { stt: name, startup_ms: started.elapsed().as_millis() as u32 });
                }
                Done::Failed(e) => {
                    let _ = ev.send(Event::Error(e));
                    if !ready {
                        cancelled = true;
                    }
                    // A failed final won't arrive; don't wait for it.
                    awaiting.clear();
                }
                Done::Partial { phrase: p, text } => {
                    // Only the phrase still being spoken shows a live text.
                    if p == phrase && seg.in_phrase {
                        live = text;
                        let _ = ev.send(Event::Text { committed: committed(&finals), live: live.clone() });
                    }
                }
                Done::Final { phrase: p, text } => {
                    awaiting.remove(&p);
                    if !text.trim().is_empty() {
                        finals.insert(p, text.trim().to_string());
                    }
                    let _ = ev.send(Event::Text {
                        committed: committed(&finals),
                        live: if seg.in_phrase { live.clone() } else { String::new() },
                    });
                }
            }
        }
        if cancelled {
            break;
        }
        if stopping {
            // The phrase being spoken keeps its number: its partials used it.
            if let Some(Cut::Phrase(audio)) = seg.take() {
                awaiting.insert(phrase);
                let _ = job_tx.send(Job::Final { phrase, audio });
            }
            live.clear();
            let stop_at = *stop_deadline.get_or_insert_with(|| Instant::now() + Duration::from_secs(15));
            if awaiting.is_empty() || !ready || Instant::now() >= stop_at {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        if mic_failed(&mic) {
            let _ = ev.send(Event::Error("The microphone disconnected. What you said so far is kept.".into()));
            stopping = true;
            continue;
        }
        if let Some(rx) = &pushed {
            loop {
                match rx.try_recv() {
                    Ok(chunk) => mic.buf.lock().extend_from_slice(&chunk),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        stopping = true;
                        break;
                    }
                }
            }
        }
        let raw = std::mem::take(&mut *mic.buf.lock());
        if !raw.is_empty() {
            pending.extend(to16k.push(&raw));
        }
        while pending.len() >= crate::FRAME {
            let frame: Vec<f32> = pending.drain(..crate::FRAME).collect();
            level = level.max(crate::vad::rms(&frame));
            // Audio before the model is ready is not dictation yet.
            if !ready {
                continue;
            }
            let was_in = seg.in_phrase;
            match seg.feed(&frame) {
                Some(Cut::Partial(audio)) => {
                    let _ = job_tx.try_send(Job::Partial { phrase, audio });
                }
                Some(Cut::Phrase(audio)) => {
                    awaiting.insert(phrase);
                    let _ = job_tx.send(Job::Final { phrase, audio });
                    live.clear();
                }
                None => {}
            }
            if !was_in && seg.in_phrase {
                phrase += 1;
            }
        }
        if last_level.elapsed() >= Duration::from_millis(50) {
            last_level = Instant::now();
            let _ = ev.send(Event::Level(std::mem::take(&mut level)));
        }
        std::thread::sleep(Duration::from_millis(8));
    }

    drop(stream);
    drop(job_tx);
    let engine = stt_thread.join().ok().flatten();
    // Late finals that arrived while the worker drained.
    while let Ok(done) = done_rx.try_recv() {
        if let Done::Final { phrase: p, text } = done
            && !cancelled
            && !text.trim().is_empty()
        {
            finals.insert(p, text.trim().to_string());
        }
    }
    crate::pipeline::park_warm(key, engine, Some(seg.vad));
    Ok(committed(&finals))
}

fn mic_failed(mic: &crate::pipeline::Mic) -> bool {
    mic.failed.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tone burst of `ms` at 16 kHz, loud enough for the energy VAD.
    fn burst(ms: usize) -> Vec<f32> {
        (0..ms * 16).map(|i| (i as f32 * 0.07).sin() * 0.3).collect()
    }
    fn quiet(ms: usize) -> Vec<f32> {
        vec![0.0005; ms * 16]
    }

    fn frames(audio: &[f32]) -> impl Iterator<Item = &[f32]> {
        audio.chunks_exact(crate::FRAME)
    }

    /// Voiced above a fixed level. The energy VAD adapts to a steady tone
    /// (after ~1.8 s it calls it background), which real speech never is.
    struct Threshold;
    impl Vad for Threshold {
        fn name(&self) -> &'static str {
            "threshold"
        }
        fn speech(&mut self, frame: &[f32]) -> f32 {
            if crate::vad::rms(frame) > 0.01 { 1.0 } else { 0.0 }
        }
        fn reset(&mut self) {}
    }

    #[test]
    fn segmenter_cuts_phrases_on_pauses_and_asks_for_partials() {
        let mut seg = Segmenter::new(Box::new(Threshold), 450, 650, 20_000);
        let mut audio = quiet(500);
        audio.extend(burst(1500));
        audio.extend(quiet(900));
        audio.extend(burst(700));
        audio.extend(quiet(900));
        let cuts: Vec<Cut> = frames(&audio).filter_map(|f| seg.feed(f)).collect();
        let phrases: Vec<usize> =
            cuts.iter().filter_map(|c| if let Cut::Phrase(a) = c { Some(a.len()) } else { None }).collect();
        let partials = cuts.iter().filter(|c| matches!(c, Cut::Partial(_))).count();
        assert_eq!(phrases.len(), 2, "two phrases, split at the pause");
        assert!(partials >= 2, "the 1.5 s phrase was transcribed while spoken ({partials} partials)");
        // Pre-roll keeps the onset; trailing silence is trimmed to ~192 ms.
        assert!(phrases[0] >= 1500 * 16 && phrases[0] < (1500 + 256 + 250) * 16, "{}", phrases[0]);
        assert!(seg.take().is_none(), "nothing left after the pause");
    }

    #[test]
    fn a_click_is_not_a_phrase_and_long_speech_is_split() {
        let mut seg = Segmenter::new(Box::new(Threshold), 450, 650, 2_000);
        let mut audio = quiet(500);
        audio.extend(burst(70));
        audio.extend(quiet(900));
        assert!(frames(&audio).filter_map(|f| seg.feed(f)).all(|c| !matches!(c, Cut::Phrase(_))));
        let mut long = quiet(300);
        long.extend(burst(5000));
        let n = frames(&long).filter_map(|f| seg.feed(f)).filter(|c| matches!(c, Cut::Phrase(_))).count();
        assert!(n >= 2, "speech without a pause is still committed every 2 s ({n})");
    }

    /// Counts calls so the test can tell partials from finals.
    struct Fake {
        finals: usize,
    }
    impl SpeechToText for Fake {
        fn name(&self) -> String {
            "fake".into()
        }
        fn partial(&mut self, audio: &[f32]) -> Result<String> {
            Ok(format!("partial{}", audio.len() / 16_000))
        }
        fn finish(&mut self, _audio: &[f32]) -> Result<String> {
            self.finals += 1;
            Ok(["Hello there.", "Second phrase."][(self.finals - 1).min(1)].to_string())
        }
    }

    #[test]
    fn live_dictation_streams_text_then_commits_on_pause_and_stop() {
        let (tx, rx) = unbounded();
        let cfg = DictationConfig::new(
            std::env::temp_dir(),
            SttChoice::Deepgram { api_key: String::new(), model: String::new() },
            Input::Stream { rate: 16_000, samples: rx },
        );
        let d = LiveDictation::with_engine(cfg, Box::new(Fake { finals: 0 }));
        // Wait for ready, then talk: phrase, pause, phrase still going at stop.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !matches!(d.events.recv_timeout(Duration::from_millis(100)), Ok(Event::Ready { .. })) {
            assert!(Instant::now() < deadline, "never ready");
        }
        let mut audio = quiet(300);
        audio.extend(burst(1400));
        audio.extend(quiet(1000));
        audio.extend(burst(1200));
        // 32 ms of audio every 20 ms: faster than real time, slow enough
        // for a partial to come back while the phrase is still going.
        for chunk in audio.chunks(512) {
            tx.send(chunk.to_vec()).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(300));
        d.stop();
        let mut texts = Vec::new();
        let ended = loop {
            match d.events.recv_timeout(Duration::from_secs(5)).expect("ended in time") {
                Event::Text { committed, live } => texts.push((committed, live)),
                Event::Ended { text } => break text,
                _ => {}
            }
        };
        assert_eq!(ended, "Hello there. Second phrase.");
        assert!(
            texts.iter().any(|(c, l)| c.is_empty() && l.starts_with("partial")),
            "live text before the first pause: {texts:?}"
        );
        assert!(texts.iter().any(|(c, _)| c == "Hello there."), "first phrase committed at the pause: {texts:?}");
    }

    #[test]
    fn cancel_keeps_only_what_was_committed() {
        let (tx, rx) = unbounded();
        let cfg = DictationConfig::new(
            std::env::temp_dir(),
            SttChoice::Deepgram { api_key: String::new(), model: String::new() },
            Input::Stream { rate: 16_000, samples: rx },
        );
        let d = LiveDictation::with_engine(cfg, Box::new(Fake { finals: 0 }));
        while !matches!(d.events.recv_timeout(Duration::from_secs(5)), Ok(Event::Ready { .. })) {}
        let mut audio = quiet(300);
        audio.extend(burst(1000));
        for chunk in audio.chunks(512) {
            tx.send(chunk.to_vec()).unwrap();
        }
        std::thread::sleep(Duration::from_millis(400));
        d.cancel();
        let ended = loop {
            if let Event::Ended { text } = d.events.recv_timeout(Duration::from_secs(5)).unwrap() {
                break text;
            }
        };
        assert_eq!(ended, "", "the phrase in progress is dropped, not transcribed");
    }
}
