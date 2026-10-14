//! Speaking replies and hands-free voice mode ("Jarvis").
//!
//! Voice mode is one loop: listen until you stop talking, transcribe, send
//! the words to the agent, and when its turn ends, read the answer aloud,
//! then listen again. Speech-to-text uses the built-in Parakeet model or
//! NEXI-STT; text-to-speech uses NEXI-TTS (local), Fish Audio (your API
//! key) or the operating system's own voice (no install).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use dove_core::Engine;
use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global};

use crate::nexi::{self, Component};

// ------------------------------------------------------------- settings

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SttEngine {
    Parakeet,
    Nexi,
}

impl SttEngine {
    pub fn of(engine: &Engine) -> Self {
        match engine.store.setting("stt_engine").as_deref() {
            Some("nexi") => Self::Nexi,
            _ => Self::Parakeet,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TtsEngine {
    System,
    Nexi,
    Fish,
}

impl TtsEngine {
    pub fn of(engine: &Engine) -> Self {
        match engine.store.setting("tts_engine").as_deref() {
            Some("nexi") => Self::Nexi,
            Some("fish") => Self::Fish,
            _ => Self::System,
        }
    }
}

// ------------------------------------------------------------ text prep

/// What is worth saying out loud from a markdown reply: prose without code,
/// tables, links or markup, capped so a long answer does not drone on.
pub fn speakable(md: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in md.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            in_fence = !in_fence;
            if in_fence {
                out.push_str(" (code omitted) ");
            }
            continue;
        }
        if in_fence || t.starts_with('|') || t.starts_with("$$") {
            continue;
        }
        let t = t.trim_start_matches(['#', '>', '-', '*', '+']).trim();
        // "1. item" / "2) item": drop the list marker, keep "3 files changed".
        let digits = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let t = if digits > 0 && t[digits..].starts_with(['.', ')']) { t[digits + 1..].trim() } else { t };
        if t.is_empty() {
            continue;
        }
        let mut s = String::with_capacity(t.len());
        let mut chars = t.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '*' | '_' | '`' | '~' => {}
                '[' => {
                    // [label](url) -> label
                    let label: String = chars.by_ref().take_while(|&c| c != ']').collect();
                    s.push_str(&label);
                    if chars.peek() == Some(&'(') {
                        for c in chars.by_ref() {
                            if c == ')' {
                                break;
                            }
                        }
                    }
                }
                _ => s.push(c),
            }
        }
        let s: String = s.split_whitespace().filter(|w| !w.starts_with("http")).collect::<Vec<_>>().join(" ");
        if !s.is_empty() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&s);
        }
        if out.chars().count() >= max_chars {
            break;
        }
    }
    if out.chars().count() > max_chars {
        // End on a sentence when we can.
        let cut: String = out.chars().take(max_chars).collect();
        out = match cut.rfind(['.', '!', '?']) {
            Some(i) if i >= max_chars / 3 => cut[..=i].to_string(),
            _ => format!("{cut}…"),
        };
    }
    out.trim().to_string()
}

/// Split text into sentence-sized pieces for streaming synthesis.
fn chunks(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for sentence in text.split_inclusive(['.', '!', '?', ';']) {
        if cur.len() + sentence.len() > max && !cur.is_empty() {
            out.push(std::mem::take(&mut cur).trim().to_string());
        }
        cur.push_str(sentence);
        while cur.len() > max {
            let cut = cur[..max].rfind(' ').unwrap_or(max);
            out.push(cur[..cut].trim().to_string());
            cur = cur[cut..].to_string();
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

// --------------------------------------------------------------- output

/// Play mono samples on the default output device; blocks until done or
/// `stop` is set.
pub fn play(samples: &[f32], rate: u32, stop: &AtomicBool) -> Result<()> {
    use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
    let device = cpal::default_host().default_output_device().context("No speaker or headphones found")?;
    let config = device.default_output_config()?;
    let out_rate = config.sample_rate();
    let channels = config.channels() as usize;
    let data = Arc::new(nexi::resample(samples, rate, out_rate));
    let pos = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));
    macro_rules! stream {
        ($t:ty) => {{
            let (data, pos, done) = (data.clone(), pos.clone(), done.clone());
            device.build_output_stream(
                &config.clone().into(),
                move |out: &mut [$t], _| {
                    for frame in out.chunks_mut(channels) {
                        let i = pos.fetch_add(1, Ordering::Relaxed);
                        let v = match data.get(i) {
                            Some(v) => *v,
                            None => {
                                done.store(true, Ordering::Release);
                                0.0
                            }
                        };
                        for s in frame.iter_mut() {
                            *s = <$t as cpal::FromSample<f32>>::from_sample_(v);
                        }
                    }
                },
                |_| {},
                None,
            )?
        }};
    }
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => stream!(f32),
        cpal::SampleFormat::I16 => stream!(i16),
        cpal::SampleFormat::U16 => stream!(u16),
        cpal::SampleFormat::I32 => stream!(i32),
        other => bail!("Unsupported speaker format {other:?}"),
    };
    stream.play()?;
    let deadline = Instant::now() + Duration::from_secs_f64(data.len() as f64 / out_rate as f64 + 2.0);
    while !done.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(30));
    }
    // Let the device drain its last buffer.
    if !stop.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(120));
    }
    Ok(())
}

/// Speak `text` with the chosen engine; blocks until finished or stopped.
pub fn speak(engine: &Engine, text: &str, stop: &AtomicBool) -> Result<()> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    match TtsEngine::of(engine) {
        TtsEngine::System => system_say(text, stop),
        TtsEngine::Nexi => {
            let root = nexi::root(engine.store.root());
            nexi::ensure_running(&root, Component::Tts, Duration::from_secs(40))?;
            let voice = engine.store.setting("tts_voice").unwrap_or_else(|| "Jasper".into());
            let speed: f32 = engine.store.setting("tts_speed").and_then(|s| s.parse().ok()).unwrap_or(1.0);
            // Synthesize the next sentence while this one plays.
            let pieces = chunks(text, 220);
            let (tx, rx) = std::sync::mpsc::sync_channel::<Result<(Vec<f32>, u32)>>(1);
            let voice2 = voice.clone();
            std::thread::spawn(move || {
                for p in pieces {
                    if tx.send(nexi::synthesize(&p, &voice2, speed)).is_err() {
                        break;
                    }
                }
            });
            for piece in rx {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let (s, r) = piece?;
                play(&s, r, stop)?;
            }
            Ok(())
        }
        TtsEngine::Fish => {
            let key = engine
                .store
                .setting("fish_api_key")
                .filter(|k| !k.trim().is_empty())
                .ok_or_else(|| anyhow!("Add your Fish Audio API key in Settings › Voice"))?;
            let reference = engine.store.setting("fish_voice").filter(|v| !v.trim().is_empty());
            let rt = engine.runtime();
            let text = text.to_string();
            let wav = rt.block_on(async move {
                let mut body =
                    serde_json::json!({"text": text, "format": "wav", "normalize": true, "latency": "balanced"});
                if let Some(r) = reference {
                    body["reference_id"] = serde_json::Value::String(r);
                }
                let res = reqwest::Client::new()
                    .post("https://api.fish.audio/v1/tts")
                    .bearer_auth(key.trim())
                    .header("model", "s1")
                    .json(&body)
                    .send()
                    .await?;
                let status = res.status();
                let bytes = res.bytes().await?;
                if !status.is_success() {
                    bail!(
                        "Fish Audio {status}: {}",
                        String::from_utf8_lossy(&bytes).chars().take(200).collect::<String>()
                    );
                }
                Ok::<_, anyhow::Error>(bytes.to_vec())
            })?;
            let (s, r) = nexi::parse_wav(&wav)?;
            play(&s, r, stop)
        }
    }
}

/// The operating system's own text-to-speech: always available.
fn system_say(text: &str, stop: &AtomicBool) -> Result<()> {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("powershell");
        c.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Add-Type -AssemblyName System.Speech; $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; $s.Speak([Console]::In.ReadToEnd())",
        ]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            c.creation_flags(0x0800_0000);
        }
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("say");
        c.args(["-f", "-"]);
        c
    } else {
        let mut c = Command::new("spd-say");
        c.args(["-w", "-e"]);
        c
    };
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write as _;
        stdin.write_all(text.as_bytes())?;
    }
    loop {
        if stop.load(Ordering::Acquire) {
            let _ = child.kill();
            return Ok(());
        }
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}

// ---------------------------------------------------------------- input

static PARAKEET: Mutex<Option<zeron_voice::Recognizer>> = Mutex::new(None);

/// Transcribe with the selected engine.
pub fn transcribe(engine: &Engine, samples: Vec<f32>, rate: u32) -> Result<String> {
    match SttEngine::of(engine) {
        SttEngine::Nexi => {
            let root = nexi::root(engine.store.root());
            nexi::ensure_running(&root, Component::Stt, Duration::from_secs(40))?;
            nexi::transcribe(&samples, rate)
        }
        SttEngine::Parakeet => {
            let dir = crate::voice::model_dir(engine);
            if !zeron_voice::installed(&dir) {
                bail!("Install local voice in Settings › Voice (or pick NEXI-STT)");
            }
            let mut slot = PARAKEET.lock().unwrap_or_else(|e| e.into_inner());
            if slot.is_none() {
                *slot = Some(zeron_voice::Recognizer::load(&dir)?);
            }
            slot.as_mut().expect("loaded above").transcribe(samples, rate)
        }
    }
}

/// Record one utterance: wait for speech, stop after `silence` of quiet.
/// `level` mirrors the microphone for the UI.
pub fn record_utterance(
    device: Option<String>,
    stop: &AtomicBool,
    level_out: &AtomicU32,
    silence: Duration,
) -> Result<Option<(Vec<f32>, u32)>> {
    let level = Arc::new(AtomicU32::new(0));
    let capture = zeron_voice::Capture::start(device.as_deref(), level.clone())?;
    let started = Instant::now();
    let mut floor = 0.01f32;
    let mut speaking_since: Option<Instant> = None;
    let mut last_voice = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(30));
        let l = f32::from_bits(level.swap(0, Ordering::Relaxed));
        level_out.store(l.to_bits(), Ordering::Relaxed);
        if stop.load(Ordering::Acquire) {
            return Ok(None);
        }
        // Adaptive threshold: a few times the room's noise floor.
        if speaking_since.is_none() {
            floor = floor * 0.95 + l.min(0.08) * 0.05;
        }
        let threshold = (floor * 3.0).max(0.02);
        if l > threshold {
            last_voice = Instant::now();
            speaking_since.get_or_insert_with(Instant::now);
        }
        let ended = capture.ended();
        match speaking_since {
            Some(since)
                if ended || (last_voice.elapsed() >= silence && since.elapsed() > Duration::from_millis(300)) =>
            {
                let (samples, rate) = capture.finish()?;
                return Ok(Some((samples, rate)));
            }
            None if ended || started.elapsed() > Duration::from_secs(45) => {
                // Nothing said; let the caller start a fresh window.
                drop(capture);
                return Ok(Some((Vec::new(), 16_000)));
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------ voice mode

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Off,
    Listening,
    Transcribing,
    Thinking,
    Speaking,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Off => "Voice mode",
            Phase::Listening => "Listening…",
            Phase::Transcribing => "Transcribing…",
            Phase::Thinking => "Thinking…",
            Phase::Speaking => "Speaking…",
        }
    }
}

pub enum VoiceModeEvent {
    /// The user said this; send it to the agent.
    Heard(String),
    Changed,
}

/// App-wide hands-free conversation state.
pub struct VoiceMode {
    engine: Arc<Engine>,
    pub phase: Phase,
    pub error: Option<String>,
    level: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    _task: Option<gpui::Task<()>>,
    _meter: Option<gpui::Task<()>>,
    pub shown_level: f32,
}

impl EventEmitter<VoiceModeEvent> for VoiceMode {}

struct VoiceModeGlobal(Entity<VoiceMode>);
impl Global for VoiceModeGlobal {}

impl VoiceMode {
    pub fn init(engine: Arc<Engine>, cx: &mut App) -> Entity<VoiceMode> {
        let e = cx.new(|_| VoiceMode {
            engine,
            phase: Phase::Off,
            error: None,
            level: Arc::new(AtomicU32::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            _task: None,
            _meter: None,
            shown_level: 0.0,
        });
        cx.set_global(VoiceModeGlobal(e.clone()));
        e
    }

    pub fn global(cx: &App) -> Option<Entity<VoiceMode>> {
        cx.try_global::<VoiceModeGlobal>().map(|g| g.0.clone())
    }

    pub fn active(&self) -> bool {
        self.phase != Phase::Off
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.active() {
            self.off(cx);
        } else {
            self.error = None;
            self.listen(cx);
        }
    }

    pub fn off(&mut self, cx: &mut Context<Self>) {
        self.stop.store(true, Ordering::Release);
        self.phase = Phase::Off;
        self._task = None;
        self._meter = None;
        self.shown_level = 0.0;
        cx.emit(VoiceModeEvent::Changed);
        cx.notify();
    }

    fn fresh_stop(&mut self) -> Arc<AtomicBool> {
        self.stop.store(true, Ordering::Release);
        self.stop = Arc::new(AtomicBool::new(false));
        self.stop.clone()
    }

    /// Start (or restart) listening for the next utterance.
    pub fn listen(&mut self, cx: &mut Context<Self>) {
        let stop = self.fresh_stop();
        self.phase = Phase::Listening;
        let engine = self.engine.clone();
        let level = self.level.clone();
        let device = engine.store.setting("voice_device").filter(|d| !d.is_empty());
        let silence = Duration::from_millis(
            engine.store.setting("voice_mode_silence_ms").and_then(|s| s.parse().ok()).unwrap_or(900),
        );
        self.start_meter(cx);
        self._task = Some(cx.spawn(async move |this, cx| {
            loop {
                let (stop2, level2, device2) = (stop.clone(), level.clone(), device.clone());
                let rec = cx.background_spawn(async move { record_utterance(device2, &stop2, &level2, silence) }).await;
                if stop.load(Ordering::Acquire) {
                    return;
                }
                let (samples, rate) = match rec {
                    Ok(Some(r)) => r,
                    Ok(None) => return,
                    Err(e) => {
                        let _ = this.update(cx, |v, cx| v.fail(format!("{e:#}"), cx));
                        return;
                    }
                };
                if samples.is_empty() {
                    continue; // Silence: keep listening.
                }
                let _ = this.update(cx, |v, cx| {
                    v.phase = Phase::Transcribing;
                    cx.notify();
                });
                let eng = engine.clone();
                let text = cx.background_spawn(async move { transcribe(&eng, samples, rate) }).await;
                if stop.load(Ordering::Acquire) {
                    return;
                }
                match text {
                    Ok(t) if !t.trim().is_empty() => {
                        let _ = this.update(cx, |v, cx| {
                            v.phase = Phase::Thinking;
                            v._meter = None;
                            cx.emit(VoiceModeEvent::Heard(t.trim().to_string()));
                            cx.notify();
                        });
                        return;
                    }
                    Ok(_) => {
                        let _ = this.update(cx, |v, cx| {
                            v.phase = Phase::Listening;
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        let _ = this.update(cx, |v, cx| v.fail(format!("{e:#}"), cx));
                        return;
                    }
                }
            }
        }));
        cx.emit(VoiceModeEvent::Changed);
        cx.notify();
    }

    /// Read the agent's answer aloud, then listen again.
    pub fn speak_reply(&mut self, markdown: &str, cx: &mut Context<Self>) {
        if !self.active() {
            return;
        }
        let max = self.engine.store.setting("voice_mode_max_chars").and_then(|s| s.parse().ok()).unwrap_or(700);
        let text = speakable(markdown, max);
        let stop = self.fresh_stop();
        self.phase = Phase::Speaking;
        let engine = self.engine.clone();
        self._task = Some(cx.spawn(async move |this, cx| {
            let s2 = stop.clone();
            let res = cx.background_spawn(async move { speak(&engine, &text, &s2) }).await;
            if stop.load(Ordering::Acquire) {
                return;
            }
            let _ = this.update(cx, |v, cx| match res {
                Ok(()) => v.listen(cx),
                Err(e) => {
                    // Keep the conversation going even if speech failed.
                    v.error = Some(format!("{e:#}"));
                    v.listen(cx);
                }
            });
        }));
        cx.emit(VoiceModeEvent::Changed);
        cx.notify();
    }

    /// Interrupt speech and listen right away (barge-in).
    pub fn interrupt(&mut self, cx: &mut Context<Self>) {
        if self.phase == Phase::Speaking {
            self.listen(cx);
        }
    }

    fn fail(&mut self, e: String, cx: &mut Context<Self>) {
        self.error = Some(e);
        self.phase = Phase::Off;
        self._meter = None;
        self.stop.store(true, Ordering::Release);
        cx.emit(VoiceModeEvent::Changed);
        cx.notify();
    }

    fn start_meter(&mut self, cx: &mut Context<Self>) {
        let level = self.level.clone();
        self._meter = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(60)).await;
                let l = f32::from_bits(level.load(Ordering::Relaxed));
                if this
                    .update(cx, |v, cx| {
                        let next = (l * 8.0).clamp(0.0, 1.0);
                        if (next - v.shown_level).abs() > 0.02 {
                            v.shown_level = next;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }
}

// ------------------------------------------------------- NEXI installs

#[derive(Clone, Debug, PartialEq)]
pub enum InstallStatus {
    Missing,
    Working(String),
    Installed,
    Failed(String),
}

/// App-wide install state of the NEXI components (Settings and
/// Integrations both read it).
pub struct NexiInstaller {
    engine: Arc<Engine>,
    status: std::collections::HashMap<Component, InstallStatus>,
    _tasks: Vec<gpui::Task<()>>,
}

struct NexiInstallerGlobal(Entity<NexiInstaller>);
impl Global for NexiInstallerGlobal {}

impl NexiInstaller {
    pub fn init(engine: Arc<Engine>, cx: &mut App) -> Entity<NexiInstaller> {
        let root = nexi::root(engine.store.root());
        let status = Component::ALL
            .into_iter()
            .map(|c| (c, if nexi::installed(&root, c) { InstallStatus::Installed } else { InstallStatus::Missing }))
            .collect();
        let e = cx.new(|_| NexiInstaller { engine, status, _tasks: Vec::new() });
        cx.set_global(NexiInstallerGlobal(e.clone()));
        e
    }

    pub fn global(cx: &App) -> Option<Entity<NexiInstaller>> {
        cx.try_global::<NexiInstallerGlobal>().map(|g| g.0.clone())
    }

    pub fn status(&self, c: Component) -> InstallStatus {
        self.status.get(&c).cloned().unwrap_or(InstallStatus::Missing)
    }

    pub fn install(&mut self, c: Component, cx: &mut Context<Self>) {
        if matches!(self.status(c), InstallStatus::Working(_)) {
            return;
        }
        self.status.insert(c, InstallStatus::Working("Starting…".into()));
        let root = nexi::root(self.engine.store.root());
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let job = cx.background_spawn(async move { nexi::install(&root, c, &|m: &str| drop(tx.send(m.to_string()))) });
        let rx = Arc::new(Mutex::new(rx));
        self._tasks.push(cx.spawn(async move |this, cx| {
            let poll = {
                let rx = rx.clone();
                let this = this.clone();
                let mut cx2 = cx.clone();
                async move {
                    loop {
                        cx2.background_executor().timer(Duration::from_millis(300)).await;
                        let msg = rx.lock().ok().and_then(|r| r.try_iter().last());
                        if let Some(m) = msg
                            && this
                                .update(&mut cx2, |i, cx| {
                                    if matches!(i.status(c), InstallStatus::Working(_)) {
                                        i.status.insert(c, InstallStatus::Working(m));
                                        cx.notify();
                                    }
                                })
                                .is_err()
                        {
                            break;
                        }
                    }
                }
            };
            let poller = cx.spawn(async move |_| poll.await);
            let result = job.await;
            drop(poller);
            let _ = this.update(cx, |i, cx| {
                i.status.insert(
                    c,
                    match result {
                        Ok(()) => InstallStatus::Installed,
                        Err(e) => InstallStatus::Failed(format!("{e:#}")),
                    },
                );
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub fn remove(&mut self, c: Component, cx: &mut Context<Self>) {
        let root = nexi::root(self.engine.store.root());
        let _ = nexi::uninstall(&root, c);
        self.status.insert(c, InstallStatus::Missing);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speakable_drops_code_and_markup() {
        let md = "## Done\n\nI fixed **two** bugs in [retry](http://x.y).\n\n```rust\nfn x() {}\n```\n- Tests pass\n| a | b |";
        let s = speakable(md, 500);
        assert!(s.starts_with("Done I fixed two bugs in retry."), "{s}");
        assert!(s.contains("(code omitted)"));
        assert!(s.contains("Tests pass"));
        assert_eq!(
            speakable(
                "1. Ran the tests
3 files changed",
                100
            ),
            "Ran the tests 3 files changed"
        );
        assert!(!s.contains("fn x"));
        assert!(!s.contains('|'));
    }

    #[test]
    fn speakable_caps_on_a_sentence() {
        let md = "First sentence here. Second one is longer and keeps going for a while.";
        let s = speakable(md, 40);
        assert_eq!(s, "First sentence here.");
    }

    #[test]
    fn chunks_respect_sentences() {
        let c = chunks("One. Two two. Three three three.", 14);
        assert_eq!(c, vec!["One. Two two.", "Three three", "three."]);
    }
}
