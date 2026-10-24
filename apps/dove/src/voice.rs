//! Local dictation on this device. Nothing leaves the machine and no audio
//! is stored.
//!
//! Live dictation (the default when a local recognizer is installed) writes
//! words into the composer while you talk: the phrase being spoken updates in
//! place and each pause commits it ([`dove_voice::dictation`]). Without it,
//! Parakeet TDT 0.6B v3 (Zeron's voice crate) transcribes after you stop.
//!
//! The model (≈670 MB) is installed from Settings › Voice, verified file by
//! file against pinned SHA-256 sums, into `~/.dove/voice/parakeet-v3`.
//! [`VoiceModel`] is the app-wide install state; each composer owns a
//! [`Dictation`] that records and transcribes.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dove_core::Engine;
use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global};

/// Presses shorter than this toggle recording; longer ones are hold-to-talk.
const TAP: Duration = Duration::from_millis(300);
const POLL: Duration = Duration::from_millis(40);

pub fn model_dir(engine: &Engine) -> PathBuf {
    engine.store.root().join("voice").join("parakeet-v3")
}

#[derive(Clone, Debug, PartialEq)]
pub enum InstallState {
    Missing,
    Downloading { done: u64, total: u64 },
    Installed,
    Failed(String),
}

/// Install state of the local model, shared by Settings and every mic.
pub struct VoiceModel {
    engine: Arc<Engine>,
    pub state: InstallState,
    cancel: Option<Arc<AtomicBool>>,
}

/// Where [VoiceModel::init] found the model, for [VoiceModel::download_size].
static MODEL_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

struct VoiceModelGlobal(Entity<VoiceModel>);
impl Global for VoiceModelGlobal {}

impl VoiceModel {
    pub fn init(engine: Arc<Engine>, cx: &mut App) -> Entity<VoiceModel> {
        let dir = model_dir(&engine);
        // No warm-up here: Parakeet holds about 670 MB once loaded, and a
        // dictation loads it while the microphone records anyway.
        let state = if zeron_voice::installed(&dir) { InstallState::Installed } else { InstallState::Missing };
        let _ = MODEL_DIR.set(dir);
        let e = cx.new(|_| VoiceModel { engine, state, cancel: None });
        cx.set_global(VoiceModelGlobal(e.clone()));
        e
    }

    pub fn global(cx: &App) -> Option<Entity<VoiceModel>> {
        cx.try_global::<VoiceModelGlobal>().map(|g| g.0.clone())
    }

    /// The model plus ONNX Runtime while that is missing.
    pub fn download_size() -> u64 {
        MODEL_DIR.get().map_or_else(zeron_voice::download_size, |d| zeron_voice::download_size_with_runtime(d))
    }

    /// Download and verify the model on a worker thread.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        if matches!(self.state, InstallState::Downloading { .. } | InstallState::Installed) {
            return;
        }
        let total = Self::download_size();
        self.state = InstallState::Downloading { done: 0, total };
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        let progress = Arc::new(AtomicU64::new(0));
        let dir = model_dir(&self.engine);
        let p2 = progress.clone();
        let work = cx.background_spawn(async move {
            zeron_voice::download(&dir, &cancel, |n| p2.store(n, Ordering::Relaxed)).map_err(|e| format!("{e:#}"))
        });
        // Progress ticks while the download runs.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let done = progress.load(Ordering::Relaxed);
                let still = this
                    .update(cx, |m, cx| {
                        if let InstallState::Downloading { total, .. } = m.state {
                            m.state = InstallState::Downloading { done, total };
                            cx.notify();
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if !still {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |m, cx| {
                m.cancel = None;
                m.state = match result {
                    Ok(()) => {
                        if batch_dictation(&m.engine) {
                            zeron_voice::warm(model_dir(&m.engine));
                        }
                        InstallState::Installed
                    }
                    Err(e) if e.contains("cancelled") => InstallState::Missing,
                    Err(e) => InstallState::Failed(e),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::Release);
        }
        cx.notify();
    }

    /// Delete the model files.
    pub fn remove(&mut self, cx: &mut Context<Self>) {
        zeron_voice::unload();
        let dir = model_dir(&self.engine);
        let _ = std::fs::remove_dir_all(&dir);
        if let Some(voice_root) = dir.parent() {
            zeron_voice::runtime::remove_if_unused(voice_root);
        }
        self.state = InstallState::Missing;
        cx.notify();
    }
}

pub enum DictationEvent {
    /// Final text to insert.
    Text(String),
    /// Live dictation: everything dictated so far this time (it replaces the
    /// previous `Live` text in place).
    Live(String),
    /// Live dictation ended with this text (empty: nothing was said).
    LiveDone(String),
    /// Recording state or level changed.
    Changed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Idle,
    Listening,
    Finalizing,
}

/// One composer's microphone.
pub struct Dictation {
    engine: Arc<Engine>,
    session: Option<zeron_voice::Session>,
    state: State,
    pressed_at: Option<Instant>,
    /// Started by a tap: stays on until the next press.
    toggled: bool,
    pub level: f32,
    pub error: Option<String>,
    /// The running live dictation, when live mode is on.
    live: Option<dove_voice::dictation::LiveDictation>,
    /// The recognizer live dictation reported, for the status line.
    live_model: Option<String>,
    /// Phrases live dictation has committed so far (kept if it's cancelled).
    live_committed: String,
    _poll: Option<gpui::Task<()>>,
}

impl EventEmitter<DictationEvent> for Dictation {}

impl Dictation {
    pub fn new(engine: Arc<Engine>, _cx: &mut Context<Self>) -> Self {
        Self {
            engine,
            session: None,
            state: State::Idle,
            pressed_at: None,
            toggled: false,
            level: 0.0,
            error: None,
            live: None,
            live_model: None,
            live_committed: String::new(),
            _poll: None,
        }
    }

    pub fn is_ready(&self) -> bool {
        zeron_voice::installed(&model_dir(&self.engine)) || self.live_available()
    }

    /// Live dictation is on (Settings › Voice) and a local recognizer is installed.
    pub fn live_available(&self) -> bool {
        self.engine.store.flag("dictation_live", true) && crate::voice_call::local_stt_ready(&self.engine)
    }

    pub fn is_recording(&self) -> bool {
        self.state == State::Listening
    }

    pub fn is_transcribing(&self) -> bool {
        self.state == State::Finalizing
    }

    pub fn status_line(&self) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        if self.live_available() {
            match &self.live_model {
                Some(m) => format!("Live dictation · {m} · runs on this device"),
                None => "Live dictation · words appear as you talk · runs on this device".into(),
            }
        } else if self.is_ready() {
            "Parakeet v3 · runs on this device · ready".into()
        } else {
            format!("Local voice is not installed ({} MB download)", VoiceModel::download_size() / 1_000_000)
        }
    }

    /// Mouse down on the mic.
    pub fn press(&mut self, cx: &mut Context<Self>) {
        if self.state == State::Listening {
            // A second press ends a tap-started recording.
            self.stop(cx);
            return;
        }
        if self.state == State::Finalizing {
            return;
        }
        if !self.is_ready() {
            self.error = Some("Install local voice in Settings › Voice to dictate.".into());
            cx.emit(DictationEvent::Changed);
            cx.notify();
            return;
        }
        let device = self.engine.store.setting("voice_device").filter(|d| !d.is_empty());
        if self.live_available() {
            self.start_live(device, cx);
            return;
        }
        match zeron_voice::Session::start(model_dir(&self.engine), device) {
            Ok(session) => {
                self.session = Some(session);
                self.state = State::Listening;
                self.error = None;
                self.pressed_at = Some(Instant::now());
                self.toggled = false;
                self.poll(cx);
            }
            Err(e) => self.error = Some(format!("{e:#}")),
        }
        cx.emit(DictationEvent::Changed);
        cx.notify();
    }

    /// Mouse up: a long press ends here; a tap keeps recording.
    pub fn release(&mut self, cx: &mut Context<Self>) {
        let Some(at) = self.pressed_at.take() else { return };
        if self.state != State::Listening {
            return;
        }
        if at.elapsed() < TAP {
            self.toggled = true;
        } else {
            self.stop(cx);
        }
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(l) = &self.live {
            l.stop();
        }
        if let Some(s) = self.session.as_mut() {
            s.finish();
        }
        self.state = State::Finalizing;
        cx.emit(DictationEvent::Changed);
        cx.notify();
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(l) = self.live.take() {
            l.cancel();
            // Only the phrase in progress is dropped; committed ones stay.
            cx.emit(DictationEvent::LiveDone(std::mem::take(&mut self.live_committed)));
        }
        self.session = None;
        self.state = State::Idle;
        self._poll = None;
        cx.emit(DictationEvent::Changed);
        cx.notify();
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        self._poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let done = this
                    .update(cx, |d, cx| {
                        let Some(s) = d.session.as_mut() else { return true };
                        d.level = s.take_level();
                        let mut done = false;
                        while let Some(ev) = s.poll() {
                            match ev {
                                zeron_voice::Event::Listening => {}
                                zeron_voice::Event::Finalizing => d.state = State::Finalizing,
                                zeron_voice::Event::Final(text) => {
                                    let text = text.trim().to_string();
                                    if !text.is_empty() {
                                        cx.emit(DictationEvent::Text(text));
                                    }
                                    done = true;
                                }
                                zeron_voice::Event::Failed(e) => {
                                    d.error = Some(e);
                                    done = true;
                                }
                            }
                        }
                        if done {
                            d.session = None;
                            d.state = State::Idle;
                        }
                        cx.emit(DictationEvent::Changed);
                        cx.notify();
                        done
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        }));
    }
}

/// Dictation records, then transcribes with Parakeet (live dictation is
/// off, or no streaming recognizer is installed).
fn batch_dictation(engine: &Engine) -> bool {
    !(engine.store.flag("dictation_live", true) && crate::voice_call::local_stt_ready(engine))
}

impl Dictation {
    fn start_live(&mut self, device: Option<String>, cx: &mut Context<Self>) {
        // Batch dictation's cached Parakeet would only sit in memory.
        zeron_voice::unload();
        let stt = crate::voice_call::local_stt(&self.engine);
        let cfg = dove_voice::dictation::DictationConfig::new(
            self.engine.store.root().to_path_buf(),
            stt,
            // `DOVE_VOICE_INPUT` (a WAV file) stands in for the microphone in QA.
            crate::voice_call::qa_input().unwrap_or(dove_voice::pipeline::Input::Device(device)),
        );
        self.live = Some(dove_voice::dictation::LiveDictation::start(cfg));
        self.live_committed.clear();
        self.state = State::Listening;
        self.error = None;
        self.pressed_at = Some(Instant::now());
        self.toggled = false;
        self.poll_live(cx);
        cx.emit(DictationEvent::Changed);
        cx.notify();
    }

    fn poll_live(&mut self, cx: &mut Context<Self>) {
        use dove_voice::dictation::Event;
        self._poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let done = this
                    .update(cx, |d, cx| {
                        let Some(live) = d.live.as_ref() else { return true };
                        let mut done = false;
                        let mut text = None;
                        while let Ok(ev) = live.events.try_recv() {
                            match ev {
                                Event::Ready { stt, .. } => d.live_model = Some(stt),
                                Event::Text { committed, live } => {
                                    text = Some(Event::joined(&committed, &live));
                                    d.live_committed = committed;
                                }
                                Event::Level(l) => d.level = l,
                                Event::Error(e) => d.error = Some(e),
                                Event::Ended { text: t } => {
                                    cx.emit(DictationEvent::LiveDone(t));
                                    done = true;
                                    text = None;
                                    break;
                                }
                            }
                        }
                        // Only the newest text matters; earlier ones were replaced.
                        if let Some(t) = text {
                            cx.emit(DictationEvent::Live(t));
                        }
                        if done {
                            d.live = None;
                            d.state = State::Idle;
                        }
                        cx.emit(DictationEvent::Changed);
                        cx.notify();
                        done
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        }));
    }
}

/// Input devices for Settings (call off the UI thread).
pub fn input_devices() -> Vec<(String, String)> {
    zeron_voice::input_devices().into_iter().map(|d| (d.id, d.name)).collect()
}
