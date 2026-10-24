//! Voice Mode in the app: one hands-free call at a time, over every thread.
//!
//! [`dove_voice`] does the listening, speaking and deciding; this module
//! connects it to Dove. Each final transcript goes to the call's
//! [`Conductor`], and each effect it returns is carried out through the same
//! paths a click takes: [`TaskView::submit`], [`TaskView::interrupt`],
//! [`TaskView::decide_from_outside`], [`Shell::open_task`]. So a spoken turn
//! lands in the thread's normal transcript in order, permission modes are
//! the thread's own, and a destructive approval still needs the user's
//! click in the chat.
//!
//! The call lives on the shell, not on a view: navigating anywhere in Dove
//! keeps it going. A minute after the call ends, its voice models unload.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dove_core::Engine;
use dove_core::proto::{HarnessKind, PermissionDecision, TaskId, TaskStatus};
use dove_voice::conductor::{Conductor, Effect};
use dove_voice::intent::{Action, Panel, Scope};
use dove_voice::pipeline::{self, Call, CallConfig, Event, State, SttChoice, TtsChoice};
use dove_voice::snapshot::{self, DiffStat, Snapshot, ThreadStatus};
use dove_voice::speakable::Chunker;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    px,
};

use crate::assets::{icon, icons};
use crate::design::ty;
use crate::shell::{Route, Shell};
use crate::theme::Theme;
use crate::transcript::{TaskView, TaskViewEvent};

/// How often the call looks at Dove for changes worth announcing.
const OBSERVE: Duration = Duration::from_millis(1000);
const PUMP: Duration = Duration::from_millis(30);
const DIFF_EVERY: Duration = Duration::from_secs(10);

pub enum VoiceCallEvent {
    /// The user finished a turn (final transcript).
    Heard(String),
    /// Time to look for thread changes worth announcing.
    Observe,
    /// The model router answered for an utterance.
    Routed {
        utterance: String,
        json: String,
    },
    Ended,
    /// The strip asked for the full call view.
    ShowView,
    /// A button answered the call as if spoken ("yes", "no").
    Answer(&'static str),
}

/// One line of the call's conversation, for the call view.
#[derive(Clone, Debug, PartialEq)]
pub struct CallLine {
    /// Said by the user (else by Dove or the agent's reply).
    pub you: bool,
    pub text: String,
}

/// Lines the call keeps for its view.
const LINES_KEPT: usize = 8;

pub struct VoiceCall {
    engine: Arc<Engine>,
    call: Call,
    pub conductor: Conductor,
    pub state: State,
    pub partial: Option<SharedString>,
    /// The last thing heard or said, for the call bar.
    pub caption: Option<SharedString>,
    pub error: Option<SharedString>,
    pub muted: bool,
    pub level: f32,
    /// The elapsed second the strip last drew.
    shown_second: u64,
    pub engines: Option<SharedString>,
    pub startup_ms: Option<u32>,
    /// The thread the call talks to.
    pub current: Option<TaskId>,
    /// The thread whose reply is being read aloud, and its subscription.
    reading: Option<(TaskId, Subscription)>,
    chunker: Chunker,
    reply_started: bool,
    call_id: String,
    started: Instant,
    started_ms: i64,
    touched: Vec<TaskId>,
    /// `git diff --shortstat` per thread folder, refreshed off-thread.
    pub diffs: HashMap<TaskId, DiffStat>,
    diff_at: Option<Instant>,
    /// Context the user asked to attach ("take a look at this"), sent with
    /// their next message.
    pub attach: Option<String>,
    /// A destructive request confirmed by voice, awaiting the user's click.
    pub awaiting_click: Option<(TaskId, String)>,
    /// Who the call talks to, for the strip: a thread's title, or the agent
    /// a new thread would start on.
    pub target: Option<SharedString>,
    /// No thread yet: the first request starts one with the picker's agent.
    pub new_thread: bool,
    /// The conversation so far (the last few lines).
    pub lines: Vec<CallLine>,
    /// The reply being read joins the last line instead of adding one.
    reply_line: bool,
    router: Option<Router>,
    _pump: gpui::Task<()>,
}

impl EventEmitter<VoiceCallEvent> for VoiceCall {}

/// The call while one runs. The views that own an input (a thread's dock,
/// home) draw its strip right above it.
#[derive(Default)]
pub struct ActiveCall(pub Option<Entity<VoiceCall>>);

impl gpui::Global for ActiveCall {}

/// The running call's strip, for a view to place above its input.
pub fn strip(cx: &App) -> Option<Entity<VoiceCall>> {
    cx.try_global::<ActiveCall>().and_then(|a| a.0.clone())
}

/// `DOVE_VOICE_INPUT`: `silence`, or a WAV file played in place of the
/// microphone (QA and demos; the mic is never opened). The file starts
/// `DOVE_VOICE_INPUT_DELAY_MS` (default 8 s) after the call does: audio from
/// before the models are ready is dropped, as a real microphone's is.
pub(crate) fn qa_input() -> Option<pipeline::Input> {
    let spec = std::env::var("DOVE_VOICE_INPUT").ok().filter(|s| !s.is_empty())?;
    let path = (spec != "silence").then(|| std::path::PathBuf::from(&spec));
    let delay = std::env::var("DOVE_VOICE_INPUT_DELAY_MS").ok().and_then(|d| d.parse().ok()).unwrap_or(8000);
    pipeline::Input::from_file(path.as_deref(), Duration::from_millis(delay))
        .map_err(|e| tracing::warn!("DOVE_VOICE_INPUT {spec}: {e:#}"))
        .ok()
}

/// Routes requests the rules do not recognise (Settings › Voice ›
/// Command router). Off by default: such requests go to the current thread.
#[derive(Clone)]
enum Router {
    /// An OpenAI-compatible chat endpoint (a local llama.cpp server, or a
    /// cloud model).
    Endpoint { url: String, model: String, key: Option<String> },
    /// A restricted, tool-less run on an installed agent (its cheapest
    /// model): no extra service, but a cold start of a few seconds.
    Agent { engine: Arc<Engine>, kind: Option<HarnessKind> },
}

impl Router {
    fn from_settings(engine: &Arc<Engine>, current: Option<HarnessKind>) -> Option<Self> {
        let s = &engine.store;
        let url = s.setting("voice_router_url").filter(|u| !u.trim().is_empty());
        let mode =
            s.setting("voice_router").unwrap_or_else(|| if url.is_some() { "endpoint".into() } else { "off".into() });
        match mode.as_str() {
            "endpoint" => Some(Self::Endpoint {
                url: url?.trim_end_matches('/').to_string(),
                model: s.setting("voice_router_model").unwrap_or_else(|| "default".into()),
                key: std::env::var("DOVE_VOICE_API_KEY").ok(),
            }),
            "agent" => Some(Self::Agent { engine: engine.clone(), kind: current }),
            _ => None,
        }
    }

    async fn ask(self, prompt: String) -> anyhow::Result<String> {
        match self {
            Self::Endpoint { url, model, key } => {
                let client = reqwest::Client::builder().timeout(Duration::from_secs(8)).build()?;
                let mut req = client.post(format!("{url}/chat/completions")).json(&serde_json::json!({
                    "model": model,
                    "temperature": 0,
                    "max_tokens": 200,
                    "messages": [{"role": "user", "content": prompt}],
                }));
                if let Some(k) = &key {
                    req = req.bearer_auth(k);
                }
                let v: serde_json::Value = req.send().await?.error_for_status()?.json().await?;
                Ok(v.pointer("/choices/0/message/content").and_then(|c| c.as_str()).unwrap_or_default().to_string())
            }
            Self::Agent { engine, kind } => engine
                .complete_restricted(kind, prompt)
                .await
                .ok()
                .flatten()
                .ok_or_else(|| anyhow::anyhow!("no agent could route the request")),
        }
    }
}

/// The local recognizer: the one chosen in Settings › Voice when it is
/// installed, else Moonshine base when installed, else Parakeet.
pub fn local_stt(engine: &Engine) -> SttChoice {
    use dove_voice::models::MoonshineSize;
    let root = engine.store.root().to_path_buf();
    let choice = engine.store.setting("voice_stt").unwrap_or_default();
    let parakeet = crate::voice::model_dir(engine);
    let parakeet_ok = zeron_voice::installed(&parakeet);
    if choice == "parakeet" && parakeet_ok {
        return SttChoice::Parakeet(parakeet);
    }
    if let Some(size) = MoonshineSize::from_id(&choice).filter(|s| dove_voice::stt::Moonshine::installed(&root, *s)) {
        return SttChoice::Moonshine(root, size);
    }
    if dove_voice::stt::Moonshine::installed(&root, MoonshineSize::Base) {
        return SttChoice::Moonshine(root, MoonshineSize::Base);
    }
    SttChoice::Parakeet(parakeet)
}

/// Whether any local recognizer is installed (a call can start offline).
pub fn local_stt_ready(engine: &Engine) -> bool {
    dove_voice::stt::Moonshine::installed(engine.store.root(), dove_voice::models::MoonshineSize::Base)
        || dove_voice::stt::Moonshine::installed(engine.store.root(), dove_voice::models::MoonshineSize::Tiny)
        || zeron_voice::installed(&crate::voice::model_dir(engine))
}

/// The call's configuration, from Settings › Voice.
pub fn config(engine: &Engine) -> CallConfig {
    let s = &engine.store;
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let cloud_url = s.setting("voice_cloud_url").filter(|u| !u.trim().is_empty());
    // Each cloud service is its own opt-in, and keys come from the
    // environment, never from Dove's database.
    let stt = if s.flag("voice_cloud", false) {
        match (s.setting("voice_cloud_stt_provider").as_deref(), &cloud_url) {
            (Some("deepgram"), _) => SttChoice::Deepgram {
                api_key: env("DEEPGRAM_API_KEY"),
                model: s.setting("voice_deepgram_model").unwrap_or_default(),
            },
            (_, Some(url)) => SttChoice::Cloud {
                base_url: url.clone(),
                api_key: env("DOVE_VOICE_API_KEY"),
                model: s.setting("voice_cloud_stt_model").unwrap_or_else(|| "whisper-1".into()),
            },
            (_, None) => local_stt(engine),
        }
    } else {
        local_stt(engine)
    };
    let tts = if s.flag("voice_cloud_tts", false) {
        let provider = s.setting("voice_cloud_tts_provider").unwrap_or_default();
        // Fish Audio, Deepgram, Cartesia and Inworld stream; each keeps its
        // own model and voice (`voice_tts_model.<id>`, `voice_tts_voice.<id>`).
        let streaming = dove_voice::tts_stream::Provider::from_env(
            &provider,
            &s.setting(&format!("voice_tts_model.{provider}")).unwrap_or_default(),
            &s.setting(&format!("voice_tts_voice.{provider}")).unwrap_or_default(),
        );
        match (provider.as_str(), &cloud_url, streaming) {
            (_, _, Some(p)) => TtsChoice::Stream(p),
            ("elevenlabs", _, None) => TtsChoice::ElevenLabs {
                api_key: env("ELEVENLABS_API_KEY"),
                voice: s.setting("voice_elevenlabs_voice").unwrap_or_default(),
                model: s.setting("voice_elevenlabs_model").unwrap_or_default(),
            },
            (_, Some(url), None) => TtsChoice::Cloud {
                base_url: url.clone(),
                api_key: env("DOVE_VOICE_API_KEY"),
                model: s.setting("voice_cloud_tts_model").unwrap_or_else(|| "tts-1".into()),
                voice: s.setting("voice_cloud_voice").unwrap_or_else(|| "alloy".into()),
            },
            (_, None, None) => TtsChoice::System,
        }
    } else {
        TtsChoice::System
    };
    CallConfig {
        root: engine.store.root().to_path_buf(),
        input: qa_input()
            .unwrap_or_else(|| pipeline::Input::Device(s.setting("voice_device").filter(|d| !d.is_empty()))),
        // QA with `DOVE_VOICE_INPUT`: `DOVE_VOICE_OUTPUT=silent` keeps replies off the speaker.
        output: if std::env::var("DOVE_VOICE_OUTPUT").as_deref() == Ok("silent") {
            pipeline::Output::Silent
        } else {
            pipeline::Output::Device(None)
        },
        push_to_talk: s.setting("voice_mode").as_deref() == Some("push"),
        echo_cancel: s.flag("voice_echo_cancel", true),
        rate: s.setting("voice_rate").and_then(|r| r.parse().ok()).unwrap_or(1.0),
        voice: s.setting("voice_name").filter(|v| !v.is_empty()),
        stt,
        tts,
        earcons: s.flag("voice_earcons", true),
    }
}

impl VoiceCall {
    pub fn start(engine: Arc<Engine>, current: Option<TaskId>, cx: &mut Context<Self>) -> Self {
        let call = Call::start(config(&engine));
        let mut conductor = Conductor::new();
        conductor.announce = engine.store.flag("voice_announce", true);
        conductor.rate = engine.store.setting("voice_rate").and_then(|r| r.parse().ok()).unwrap_or(1.0);
        let call_id = uuid::Uuid::new_v4().simple().to_string();
        let started_ms = dove_voice::calls::now_ms();
        let pump = cx.spawn(async move |this, cx| {
            let mut last_observe = Instant::now();
            loop {
                cx.background_executor().timer(PUMP).await;
                let alive = this
                    .update(cx, |v, cx| {
                        let ended = v.drain(cx);
                        if last_observe.elapsed() >= OBSERVE {
                            last_observe = Instant::now();
                            cx.emit(VoiceCallEvent::Observe);
                        }
                        !ended
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        });
        // The agent router picks the current thread's agent per request.
        let router = Router::from_settings(&engine, None);
        Self {
            engine,
            call,
            conductor,
            state: State::Loading,
            partial: None,
            caption: None,
            error: None,
            muted: false,
            level: 0.0,
            shown_second: 0,
            engines: None,
            startup_ms: None,
            current,
            reading: None,
            chunker: Chunker::new(),
            reply_started: false,
            call_id,
            started: Instant::now(),
            started_ms,
            touched: Vec::new(),
            diffs: HashMap::new(),
            diff_at: None,
            attach: None,
            awaiting_click: None,
            target: None,
            new_thread: false,
            lines: Vec::new(),
            reply_line: false,
            router,
            _pump: pump,
        }
    }

    pub fn has_router(&self) -> bool {
        self.router.is_some()
    }

    /// When the call started.
    pub fn started(&self) -> Instant {
        self.started
    }

    /// Add to the conversation: a new line, or more of the reply being read.
    fn push_line(&mut self, you: bool, text: &str, joins_reply: bool) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        match self.lines.last_mut() {
            Some(last) if joins_reply && self.reply_line && !last.you => {
                last.text.push(' ');
                last.text.push_str(text);
            }
            _ => {
                self.lines.push(CallLine { you, text: text.to_string() });
                if self.lines.len() > LINES_KEPT {
                    self.lines.remove(0);
                }
            }
        }
        self.reply_line = joins_reply;
    }

    /// Drain pipeline events; true once the call has ended.
    fn drain(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        let mut ended = false;
        while let Ok(ev) = self.call.events.try_recv() {
            // Levels arrive every 50 ms: redraw only when the waveform's
            // visible height changes (the strip's views are ancestors' too).
            if let Event::Level { mic, speaker } = ev {
                let level = mic.max(speaker);
                if level_step(level) != level_step(self.level) {
                    changed = true;
                }
                self.level = level;
                continue;
            }
            changed = true;
            match ev {
                Event::State(s) => self.state = s,
                Event::Ready { startup_ms, warm, stt, tts, vad, turn, echo_cancel } => {
                    self.startup_ms = Some(startup_ms);
                    let engines = format!(
                        "{stt} · {tts} · {vad} VAD · {turn} turns{}{}",
                        if echo_cancel { " · echo cancel" } else { "" },
                        if warm { " · warm" } else { "" }
                    );
                    tracing::info!("voice call ready in {startup_ms} ms ({engines})");
                    dove_voice::calls::append(
                        self.engine.store.root(),
                        &dove_voice::calls::Entry::Started {
                            call: self.call_id.clone(),
                            at_ms: self.started_ms,
                            engines: engines.clone(),
                        },
                    );
                    self.engines = Some(engines.into());
                }
                Event::Partial(t) => self.partial = Some(t.into()),
                Event::Turn { text, timings } => {
                    tracing::info!(
                        "voice turn: speech {} ms, end of speech to text {:.0} ms (stt {:.0} ms, turn {:.1} ms)",
                        timings.speech_ms,
                        timings.end_of_speech_to_text_ms,
                        timings.stt_final_ms,
                        timings.turn_decide_ms
                    );
                    self.partial = None;
                    self.caption = Some(text.clone().into());
                    self.push_line(true, &text, false);
                    cx.emit(VoiceCallEvent::Heard(text));
                }
                Event::BargeIn { .. } => {
                    // The user talked over Dove: stop reading the reply.
                    self.stop_reading(cx);
                }
                Event::ReplyAudio { text_to_audio_ms } => {
                    tracing::info!("voice reply: first audio {text_to_audio_ms:.0} ms after first text");
                }
                Event::Level { .. } => {}
                Event::Error(e) => {
                    self.error = Some(e.into());
                }
                Event::Ended => ended = true,
            }
        }
        // The strip's timer ticks once a second.
        let second = self.started.elapsed().as_secs();
        if second != self.shown_second {
            self.shown_second = second;
            changed = true;
        }
        if ended {
            self.finish_log();
            cx.emit(VoiceCallEvent::Ended);
        }
        if changed {
            cx.notify();
        }
        ended
    }

    fn finish_log(&mut self) {
        dove_voice::calls::append(
            self.engine.store.root(),
            &dove_voice::calls::Entry::Ended {
                call: self.call_id.clone(),
                at_ms: dove_voice::calls::now_ms(),
                duration_ms: self.started.elapsed().as_millis() as i64,
                threads: self.touched.iter().map(|t| t.to_string()).collect(),
            },
        );
    }

    fn log_user(&self, text: &str, thread: Option<TaskId>) {
        dove_voice::calls::append(
            self.engine.store.root(),
            &dove_voice::calls::Entry::User {
                call: self.call_id.clone(),
                at_ms: dove_voice::calls::now_ms(),
                text: text.to_string(),
                thread: thread.map(|t| t.to_string()),
            },
        );
    }

    /// Say something (Dove's own words: statuses, acknowledgements).
    pub fn say(&mut self, text: &str, cx: &mut Context<Self>) {
        dove_voice::calls::append(
            self.engine.store.root(),
            &dove_voice::calls::Entry::Dove {
                call: self.call_id.clone(),
                at_ms: dove_voice::calls::now_ms(),
                text: text.to_string(),
                thread: self.current.map(|t| t.to_string()),
            },
        );
        self.caption = Some(text.to_string().into());
        self.push_line(false, text, false);
        let mut c = Chunker::new();
        let mut clauses = c.push(text);
        clauses.extend(c.finish());
        for clause in clauses {
            self.call.say(clause);
        }
        cx.notify();
    }

    pub fn silence(&mut self, cx: &mut Context<Self>) {
        self.call.silence();
        self.stop_reading(cx);
    }

    pub fn set_muted(&mut self, on: bool, cx: &mut Context<Self>) {
        self.muted = on;
        self.call.mute(on);
        cx.notify();
    }

    pub fn push_to_talk(&mut self, held: bool) {
        self.call.push_to_talk(held);
    }

    pub fn set_rate(&mut self, rate: f32) {
        self.call.set_rate(rate);
        let _ = self.engine.store.set_setting("voice_rate", &format!("{rate:.2}"));
    }

    pub fn end(&mut self, cx: &mut Context<Self>) {
        self.stop_reading(cx);
        self.call.end();
    }

    /// Read `view`'s next reply aloud.
    pub fn read_replies(&mut self, id: TaskId, view: &Entity<TaskView>, cx: &mut Context<Self>) {
        self.stop_reading(cx);
        view.update(cx, |v, _| v.set_voice_tap(true));
        let sub = cx.subscribe(view, move |this, _, ev: &TaskViewEvent, cx| match ev {
            TaskViewEvent::VoiceText(t) => {
                if !this.reply_started {
                    this.reply_started = true;
                    this.call.reply_text_started();
                }
                for clause in this.chunker.push(t) {
                    // The strip shows the reply as it is read.
                    this.caption = Some(clause.clone().into());
                    this.push_line(false, &clause, true);
                    this.call.say(clause);
                    cx.notify();
                }
            }
            TaskViewEvent::VoiceTurnEnded { failed } => {
                for clause in this.chunker.finish() {
                    this.caption = Some(clause.clone().into());
                    this.push_line(false, &clause, true);
                    this.call.say(clause);
                    cx.notify();
                }
                if *failed {
                    this.call.say("That turn ended with an error.");
                }
                this.stop_reading(cx);
            }
            _ => {}
        });
        self.reading = Some((id, sub));
        self.conductor.reading_reply(&id.to_string());
        self.reply_started = false;
        self.chunker = Chunker::new();
        if !self.touched.contains(&id) {
            self.touched.push(id);
        }
    }

    fn stop_reading(&mut self, _cx: &mut Context<Self>) {
        self.reading = None;
        self.reply_line = false;
        self.chunker = Chunker::new();
        self.reply_started = false;
    }

    /// The thread being read aloud, so the shell can drop its tap.
    pub fn reading(&self) -> Option<TaskId> {
        self.reading.as_ref().map(|(id, _)| *id)
    }

    /// Refresh diff stats off the UI thread every few seconds.
    pub fn refresh_diffs(&mut self, folders: Vec<(TaskId, std::path::PathBuf)>, cx: &mut Context<Self>) {
        if self.diff_at.is_some_and(|t| t.elapsed() < DIFF_EVERY) {
            return;
        }
        self.diff_at = Some(Instant::now());
        let work = cx.background_spawn(async move {
            folders.into_iter().filter_map(|(id, dir)| shortstat(&dir).map(|d| (id, d))).collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let found = work.await;
            let _ = this.update(cx, |v, _| {
                for (id, d) in found {
                    v.diffs.insert(id, d);
                }
            });
        })
        .detach();
    }

    /// Ask the router, off the UI thread. The agent router prefers the
    /// current thread's agent.
    pub fn route(&mut self, utterance: String, snap: &Snapshot, cx: &mut Context<Self>) {
        let Some(mut router) = self.router.clone() else { return };
        if let Router::Agent { kind, .. } = &mut router {
            *kind = snap.current_thread().and_then(|t| HarnessKind::from_id(&t.agent_id));
        }
        let prompt = dove_voice::intent::router_prompt(&utterance, snap);
        // The endpoint client and the agent run are tokio futures.
        let rt = self.engine.runtime();
        let work = cx.background_spawn(async move { rt.spawn(router.ask(prompt)).await.map_err(anyhow::Error::from)? });
        cx.spawn(async move |this, cx| {
            let json = work.await.unwrap_or_default();
            let _ = this.update(cx, |_, cx| cx.emit(VoiceCallEvent::Routed { utterance, json }));
        })
        .detach();
    }

    pub fn note_user(&mut self, text: &str, thread: Option<TaskId>) {
        self.log_user(text, thread);
        if let Some(t) = thread
            && !self.touched.contains(&t)
        {
            self.touched.push(t);
        }
    }
}

/// `git diff --shortstat` of a folder's working tree against HEAD.
fn shortstat(dir: &std::path::Path) -> Option<DiffStat> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(dir).args(["diff", "HEAD", "--shortstat"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(parse_shortstat(&String::from_utf8_lossy(&out.stdout)))
}

/// " 3 files changed, 120 insertions(+), 4 deletions(-)"
pub fn parse_shortstat(s: &str) -> DiffStat {
    let mut d = DiffStat::default();
    for part in s.split(',') {
        let mut words = part.split_whitespace();
        let (Some(n), Some(what)) = (words.next().and_then(|n| n.parse::<u32>().ok()), words.next()) else { continue };
        if what.starts_with("file") {
            d.files = n;
        } else if what.starts_with("insertion") {
            d.added = n;
        } else if what.starts_with("deletion") {
            d.removed = n;
        }
    }
    d
}

fn status_of(s: TaskStatus) -> ThreadStatus {
    match s {
        TaskStatus::Running => ThreadStatus::Running,
        TaskStatus::NeedsInput => ThreadStatus::NeedsInput,
        TaskStatus::Done => ThreadStatus::Done,
        TaskStatus::Failed => ThreadStatus::Failed,
        _ => ThreadStatus::Idle,
    }
}

impl Shell {
    /// Who the call talks to: the thread it was pointed at (by voice, the
    /// shortcut or opening a thread), else the open one.
    pub(crate) fn call_thread(&self, call: &VoiceCall) -> Option<TaskId> {
        call.current.filter(|id| self.task(*id).is_some()).or(match self.route {
            Route::Thread(id) => Some(id),
            _ => None,
        })
    }

    /// The thread the keyboard is in: a pane's thread beside the open one,
    /// or the open thread.
    pub fn focused_thread(&self, window: &Window, cx: &App) -> Option<TaskId> {
        self.threads
            .iter()
            .find(|(_, v)| v.read(cx).focus_handle(cx).contains_focused(window, cx))
            .map(|(id, _)| *id)
            .or(match self.route {
                Route::Thread(id) => Some(id),
                _ => None,
            })
    }

    /// Start a call on the thread the keyboard is in, or end the one
    /// running. While a call runs, the shortcut in another thread (a pane
    /// beside, a terminal) moves the call there instead of ending it.
    pub fn toggle_voice_call(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.focused_thread(window, cx);
        if let Some(v) = self.voice_call.clone() {
            match here.filter(|id| v.read(cx).current != Some(*id)) {
                Some(id) => {
                    let name =
                        self.task(id).map(|t| format!("{} on {}", t.harness.label(), t.title)).unwrap_or_default();
                    let chat = self.threads.get(&id).map(|t| t.read(cx).chat.clone());
                    v.update(cx, |v, cx| {
                        v.current = Some(id);
                        if let Some(chat) = &chat {
                            v.read_replies(id, chat, cx);
                        }
                        v.say(&format!("Talking to {name}."), cx);
                    });
                    self.voice_target(&v, cx);
                }
                None => v.update(cx, |v, cx| v.end(cx)),
            }
            return;
        }
        if !local_stt_ready(&self.engine) && !self.engine.store.flag("voice_cloud", false) {
            crate::kit::Toasts::push(
                cx,
                "Voice Mode needs local speech recognition",
                Some("Install it in Settings › Voice (it runs on this device).".to_string()),
                crate::kit::Tone::Info,
                None,
            );
            self.open_settings_section("voice", window, cx);
            return;
        }
        // Text already in the composer goes out first, as the user wrote it:
        // to the open thread, or (from home) as a new thread with the
        // picker's agent, model and mode, which the call then talks to.
        match self.active_thread().cloned() {
            Some(v) => {
                let chat = v.read(cx).chat.clone();
                let composer = chat.read(cx).composer.clone();
                if let Some(text) = crate::composer::take_text(&composer, window, cx) {
                    chat.update(cx, |c, cx| c.submit(text, window, cx));
                }
            }
            None if matches!(self.route, Route::Home) => {
                if let Some(text) = crate::composer::take_text(&self.composer, window, cx) {
                    self.start(text, window, cx);
                }
            }
            None => {}
        }
        let current = here;
        let engine = self.engine.clone();
        let call = cx.new(|cx| VoiceCall::start(engine, current, cx));
        let view = cx.new(|cx| crate::voice_view::CallView::new(call.clone(), cx));
        self.call_view_sub = Some(cx.subscribe_in(&view, window, |this, _, ev, window, cx| match ev {
            crate::voice_view::CallViewEvent::ShowChat => this.show_call_view(false, window, cx),
            crate::voice_view::CallViewEvent::OpenThread(id) => {
                this.open_task(*id, window, cx);
                this.show_call_view(false, window, cx);
            }
            crate::voice_view::CallViewEvent::TalkTo(id) => {
                if let Some(call) = this.voice_call.clone() {
                    let snap = this.voice_snapshot(call.read(cx), cx);
                    let effects = call.update(cx, |v, _| v.conductor.talk_to(&id.to_string(), &snap));
                    this.voice_apply(call, effects, None, window, cx);
                }
            }
            crate::voice_view::CallViewEvent::Say(word) => {
                if let Some(call) = this.voice_call.clone() {
                    this.voice_heard(call, word.to_string(), window, cx);
                }
            }
        }));
        self.call_view = Some(view);
        let sub = cx.subscribe_in(&call, window, |this, call, ev: &VoiceCallEvent, window, cx| match ev {
            VoiceCallEvent::Heard(text) => this.voice_heard(call.clone(), text.clone(), window, cx),
            VoiceCallEvent::Observe => this.voice_observe(call.clone(), cx),
            VoiceCallEvent::ShowView => this.show_call_view(true, window, cx),
            VoiceCallEvent::Answer(word) => this.voice_heard(call.clone(), word.to_string(), window, cx),
            VoiceCallEvent::Routed { utterance, json } => {
                let snap = this.voice_snapshot(call.read(cx), cx);
                let effects = call.update(cx, |v, _| v.conductor.routed(utterance, json, &snap));
                this.voice_apply(call.clone(), effects, None, window, cx);
            }
            VoiceCallEvent::Ended => {
                if let Some(id) = call.read(cx).reading()
                    && let Some(t) = this.threads.get(&id)
                {
                    let chat = t.read(cx).chat.clone();
                    chat.update(cx, |c, _| c.set_voice_tap(false));
                }
                this.voice_call = None;
                this.voice_sub = None;
                if let Some(v) = this.call_view.take() {
                    v.update(cx, |v, cx| v.set_visible(false, cx));
                }
                this.call_view_sub = None;
                this.call_view_shown = false;
                cx.set_global(ActiveCall(None));
                this.voice_redraw_docks(cx);
            }
        });
        cx.set_global(ActiveCall(Some(call.clone())));
        self.voice_call = Some(call.clone());
        self.voice_sub = Some(sub);
        self.voice_target(&call, cx);
        self.voice_redraw_docks(cx);
        self.show_call_view(true, window, cx);
    }

    /// Show the call view over the window, or flip back to the chat (the
    /// strip above the composer keeps the call in view there).
    pub fn show_call_view(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.call_view.clone() else { return };
        self.call_view_shown = on;
        view.update(cx, |v, cx| v.set_visible(on, cx));
        if on {
            let f = view.read(cx).focus_handle(cx);
            window.focus(&f, cx);
        } else {
            // Hand focus to the input the user flips back to, so keys and
            // the strip's buttons reach the window again.
            match self.active_thread() {
                Some(v) => {
                    let composer = v.read(cx).chat.read(cx).composer.clone();
                    crate::composer::focus(&composer, window, cx);
                }
                None => crate::composer::focus(&self.composer, window, cx),
            }
        }
        self.voice_redraw_docks(cx);
    }

    /// Thread views are cached: redraw them so their docks show (or drop)
    /// the call strip.
    fn voice_redraw_docks(&mut self, cx: &mut Context<Self>) {
        for v in self.threads.values() {
            let chat = v.read(cx).chat.clone();
            let composer = chat.read(cx).composer.clone();
            composer.update(cx, |_, cx| cx.notify());
            chat.update(cx, |_, cx| cx.notify());
            v.update(cx, |_, cx| cx.notify());
        }
        self.composer.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    /// Name who the call talks to on its strip: the thread, or the agent a
    /// new thread would start on.
    fn voice_target(&self, call: &Entity<VoiceCall>, cx: &mut Context<Self>) {
        let current = self.call_thread(call.read(cx));
        let task = current.and_then(|id| self.task(id));
        let working =
            |id: TaskId| self.threads.get(&id).is_some_and(|v| v.read(cx).chat.read(cx).status == TaskStatus::Running);
        let target: SharedString = match task {
            Some(task) if working(task.id) => format!("{} · {} is working", task.title, task.harness.label()).into(),
            Some(task) => format!("{} · {}", task.title, task.harness.label()).into(),
            None => format!("New thread · {}", self.composer.read(cx).harness(cx).label()).into(),
        };
        let new_thread = task.is_none();
        call.update(cx, |v, cx| {
            if v.target.as_ref() != Some(&target) || v.new_thread != new_thread {
                v.target = Some(target);
                v.new_thread = new_thread;
                cx.notify();
            }
        });
    }

    /// What the conductor sees: every open thread, from state Dove holds.
    pub fn voice_snapshot(&self, call: &VoiceCall, cx: &App) -> Snapshot {
        let mut threads = Vec::new();
        let recent = self.tasks.iter().filter(|t| t.status != TaskStatus::Draft).take(40);
        for task in recent {
            let project = self.project(task.project_id);
            let view = self.threads.get(&task.id).map(|v| v.read(cx).chat.clone());
            let state = view.as_ref().map(|v| v.read(cx).voice_state()).unwrap_or_default();
            let status = view.as_ref().map(|v| v.read(cx).status).unwrap_or(task.status);
            let (done, total) = (
                state.todos.iter().filter(|t| t.status == dove_core::proto::TodoStatus::Completed).count() as u32,
                state.todos.len() as u32,
            );
            let current_todo = state
                .todos
                .iter()
                .find(|t| t.status == dove_core::proto::TodoStatus::InProgress)
                .map(|t| t.text.clone());
            threads.push(snapshot::Thread {
                id: task.id.to_string(),
                title: task.title.clone(),
                agent: task.harness.label().to_string(),
                agent_id: task.harness.id().to_string(),
                project: project.map(|p| p.name.clone()).unwrap_or_default(),
                status: status_of(status),
                live: state.live,
                steers: dove_core::steers(task.harness),
                interrupts: state.live && state.capabilities.interrupt,
                session_approvals: state.live && state.capabilities.permissions,
                pending: state
                    .pending
                    .iter()
                    .map(|r| snapshot::Pending {
                        request_id: r.request_id.clone(),
                        tool: r.tool.clone(),
                        summary: r.summary.to_string(),
                        input: r.input.clone(),
                        question: r.is_question(),
                    })
                    .collect(),
                activity: state.status_text,
                todos: (total > 0).then_some((done, total, current_todo)),
                diff: call.diffs.get(&task.id).copied(),
                cwd: task.worktree.clone().or_else(|| project.map(|p| p.path.clone())).map(|p| p.display().to_string()),
                error: state.last_error,
                teammate: self.persona_for(task.id).map(|p| p.name.clone()),
                terminal: self
                    .threads
                    .get(&task.id)
                    .is_some_and(|v| v.read(cx).view == crate::thread::ViewMode::Terminal),
            });
        }
        let current = self.call_thread(call);
        // A new thread goes where the home composer would start it.
        let composer = self.composer.read(cx);
        let default_project = current
            .and_then(|id| self.task(id))
            .map(|t| t.project_id)
            .or(composer.project.as_ref().map(|p| p.id))
            .or(self.home_project)
            .map(|p| p.to_string());
        let default_agent = Some(composer.harness(cx).id().to_string());
        Snapshot {
            threads,
            current: current.map(|c| c.to_string()),
            agents: self
                .installed
                .iter()
                .map(|i| snapshot::Agent { id: i.kind.id().to_string(), label: i.kind.label().to_string() })
                .collect(),
            projects: self
                .projects
                .iter()
                .map(|p| snapshot::Project { id: p.id.to_string(), name: p.name.clone() })
                .collect(),
            default_project,
            default_agent,
            showing: if self.panel_open {
                match self.panel.read(cx).active() {
                    Some(crate::side_panel::Surface::Browser(_)) => Some(dove_voice::intent::LookTarget::Browser),
                    Some(crate::side_panel::Surface::Changes) => Some(dove_voice::intent::LookTarget::Diff),
                    _ if self.panel.read(cx).showing(crate::side_panel::Surface::Changes) => {
                        Some(dove_voice::intent::LookTarget::Diff)
                    }
                    _ => None,
                }
            } else {
                None
            },
            teammates: self
                .personas
                .iter()
                .map(|p| snapshot::Teammate { id: p.id.clone(), name: p.name.clone(), role: p.role.clone() })
                .collect(),
        }
    }

    fn voice_heard(&mut self, call: Entity<VoiceCall>, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let snap = self.voice_snapshot(call.read(cx), cx);
        let router = call.read(cx).has_router();
        let effects = call.update(cx, |v, _| v.conductor.utterance(&text, &snap, router));
        self.voice_apply(call, effects, Some(text), window, cx);
    }

    fn voice_observe(&mut self, call: Entity<VoiceCall>, cx: &mut Context<Self>) {
        crate::look::note_foreground();
        let folders: Vec<(TaskId, std::path::PathBuf)> = self
            .threads
            .keys()
            .filter_map(|id| {
                let t = self.task(*id)?;
                let dir = t.worktree.clone().or_else(|| self.project(t.project_id).map(|p| p.path.clone()))?;
                Some((*id, dir))
            })
            .collect();
        call.update(cx, |v, cx| v.refresh_diffs(folders, cx));
        self.voice_target(&call, cx);
        let snap = self.voice_snapshot(call.read(cx), cx);
        if let Some(view) = self.call_view.clone() {
            let rail = crate::voice_view::rail(&snap);
            view.update(cx, |v, cx| v.set_rail(rail, cx));
        }
        let effects = call.update(cx, |v, _| v.conductor.observe(&snap));
        call.update(cx, |v, cx| {
            for e in effects {
                if let Effect::Speak(t) = e {
                    v.say(&t, cx);
                }
            }
            // A confirmed destructive request that was answered or replaced.
            if let Some((id, req)) = v.awaiting_click.clone() {
                let still = snap.thread(&id.to_string()).is_some_and(|t| t.pending.iter().any(|p| p.request_id == req));
                if !still {
                    v.awaiting_click = None;
                    cx.notify();
                }
            }
        });
    }

    fn task_by_str(&self, id: &str) -> Option<TaskId> {
        self.tasks.iter().find(|t| t.id.to_string() == id).map(|t| t.id)
    }

    fn chat_of(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<TaskView>> {
        if !self.threads.contains_key(&id) {
            let task = self.task(id)?.clone();
            let project = self.project(task.project_id)?.clone();
            self.make_thread(task, project, None, window, cx);
        }
        self.threads.get(&id).map(|v| v.read(cx).chat.clone())
    }

    fn voice_apply(
        &mut self,
        call: Entity<VoiceCall>,
        effects: Vec<Effect>,
        utterance: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut acted_on = None;
        for effect in effects {
            match effect {
                Effect::Speak(t) => call.update(cx, |v, cx| v.say(&t, cx)),
                Effect::StopSpeaking => call.update(cx, |v, cx| v.silence(cx)),
                Effect::MuteMic(on) => call.update(cx, |v, cx| v.set_muted(on, cx)),
                Effect::SetRate(r) => call.update(cx, |v, _| v.set_rate(r)),
                Effect::SetStyle(_) => {}
                Effect::EndCall => call.update(cx, |v, cx| v.end(cx)),
                Effect::Route(text) => {
                    let snap = self.voice_snapshot(call.read(cx), cx);
                    call.update(cx, |v, cx| v.route(text, &snap, cx));
                }
                Effect::AwaitClick { thread, request_id } => {
                    if let Some(id) = self.task_by_str(&thread) {
                        self.open_task(id, window, cx);
                        call.update(cx, |v, cx| {
                            v.awaiting_click = Some((id, request_id));
                            cx.notify();
                        });
                    }
                }
                Effect::Act(action) => {
                    if let Some(id) = self.voice_act(&call, action, window, cx) {
                        acted_on = Some(id);
                    }
                }
            }
        }
        if let Some(text) = utterance {
            call.update(cx, |v, _| v.note_user(&text, acted_on));
        }
        self.voice_target(&call, cx);
        cx.notify();
    }

    /// Perform one checked action. Returns the thread it went to.
    fn voice_act(
        &mut self,
        call: &Entity<VoiceCall>,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TaskId> {
        match action {
            Action::Send { thread, text, .. } => {
                let id = self.task_by_str(&thread)?;
                let chat = self.chat_of(id, window, cx)?;
                let attach = call.update(cx, |v, _| v.attach.take());
                let text = match attach {
                    Some(a) => format!("{text}\n\n{a}"),
                    None => text,
                };
                call.update(cx, |v, cx| {
                    v.current = Some(id);
                    v.read_replies(id, &chat, cx);
                });
                // The same path as pressing Send: steering, queueing and
                // resuming all behave as they do for typed text.
                chat.update(cx, |c, cx| c.submit(text, window, cx));
                Some(id)
            }
            Action::Interrupt { thread } => {
                let id = self.task_by_str(&thread)?;
                let chat = self.chat_of(id, window, cx)?;
                chat.update(cx, |c, cx| c.interrupt(cx));
                Some(id)
            }
            Action::Approve { thread, request_id, scope } => {
                let id = self.task_by_str(&thread)?;
                let chat = self.chat_of(id, window, cx)?;
                // Defense in depth: the conductor never approves a destructive
                // request, and neither does this.
                let state = chat.read(cx).voice_state();
                let req = state.pending.iter().find(|r| r.request_id == request_id)?;
                let pending = snapshot::Pending {
                    request_id: req.request_id.clone(),
                    tool: req.tool.clone(),
                    summary: req.summary.to_string(),
                    input: req.input.clone(),
                    question: req.is_question(),
                };
                let cwd = self
                    .task(id)
                    .and_then(|t| t.worktree.clone().or_else(|| self.project(t.project_id).map(|p| p.path.clone())));
                let cwd = cwd.map(|c| c.display().to_string());
                if dove_voice::safety::classify(&pending, cwd.as_deref()) != dove_voice::safety::Risk::Normal {
                    call.update(cx, |v, cx| v.say("That one needs your click in the chat.", cx));
                    return Some(id);
                }
                let decision = match scope {
                    Scope::Once => PermissionDecision::AllowOnce,
                    Scope::Session => PermissionDecision::AllowAlways,
                };
                chat.update(cx, |c, cx| c.decide_from_outside(request_id, decision, cx));
                Some(id)
            }
            Action::Deny { thread, request_id } => {
                let id = self.task_by_str(&thread)?;
                let chat = self.chat_of(id, window, cx)?;
                chat.update(cx, |c, cx| c.decide_from_outside(request_id, PermissionDecision::Deny, cx));
                Some(id)
            }
            Action::TalkTo { thread } => {
                let id = self.task_by_str(&thread)?;
                call.update(cx, |v, _| v.current = Some(id));
                self.open_task(id, window, cx);
                Some(id)
            }
            Action::StartTask { agent, project, prompt, worktree } => {
                let kind = HarnessKind::from_id(&agent)?;
                let project = self.projects.iter().find(|p| p.id.to_string() == project)?.clone();
                let join = if worktree {
                    match dove_core::git::create_worktree(&project.path, &dove_core::git::slug(&prompt), "dove/") {
                        Ok(w) => Some(dove_core::Joined {
                            worktree: Some(w.path),
                            branch: Some(w.branch),
                            base: Some(w.base),
                        }),
                        Err(e) => {
                            call.update(cx, |v, cx| v.say(&format!("I couldn't make a worktree: {e}."), cx));
                            return None;
                        }
                    }
                } else {
                    None
                };
                // On the picked agent, the picker's model, effort, mode and
                // teammate apply, as they do for a typed new thread.
                let (run, persona) = {
                    let c = self.composer.read(cx);
                    if c.harness(cx) == kind { (c.run_settings(cx), c.persona.clone()) } else { Default::default() }
                };
                let new = dove_core::NewTask {
                    title: crate::ui::title_from_prompt(&prompt),
                    prompt,
                    harness: kind,
                    run,
                    join,
                    persona,
                };
                match self.engine.start_task(&project, new) {
                    Ok((task, live)) => {
                        let id = task.id;
                        self.tasks.insert(0, task.clone());
                        let view = self.make_thread(task, project, Some(live), window, cx);
                        let chat = view.read(cx).chat.clone();
                        call.update(cx, |v, cx| {
                            v.current = Some(id);
                            v.read_replies(id, &chat, cx);
                        });
                        self.navigate(Route::Thread(id), window, cx);
                        Some(id)
                    }
                    Err(e) => {
                        call.update(cx, |v, cx| v.say(&format!("It didn't start: {e}."), cx));
                        None
                    }
                }
            }
            Action::Show { panel } => {
                match panel {
                    Panel::Diff => self.show_panel(crate::side_panel::Tab::Changes, window, cx),
                    Panel::Files => self.show_panel(crate::side_panel::Tab::Files, window, cx),
                    Panel::Browser => self.show_panel(crate::side_panel::Tab::Browser, window, cx),
                    Panel::Terminal => self.toggle_view_mode(window, cx),
                    Panel::Plan | Panel::Chat => {
                        if let Some(id) = call.read(cx).current {
                            self.open_task(id, window, cx);
                        }
                    }
                }
                None
            }
            Action::LookAt { target } => {
                let note = match target {
                    dove_voice::intent::LookTarget::Diff => {
                        let current = call.read(cx).current;
                        current.and_then(|id| call.read(cx).diffs.get(&id).copied()).map(|d| {
                            format!(
                                "(From the voice call: please look at the current diff in this thread's folder: {} files changed, +{} −{}. Run `git diff` to see it.)",
                                d.files, d.added, d.removed
                            )
                        })
                    }
                    dove_voice::intent::LookTarget::Browser => {
                        self.panel.read(cx).browser_context(cx).map(|(title, url)| {
                            format!(
                                "(From the voice call: the user is looking at \"{title}\" — {url} in Dove's browser.)"
                            )
                        })
                    }
                    dove_voice::intent::LookTarget::Window => {
                        match crate::look::capture_last(&self.engine.store.root().join("voice").join("look")) {
                            Ok((path, title)) => Some(format!(
                                "(From the voice call: a screenshot of the window \"{title}\" the user is looking at is at {}. Open it to see what they mean.)",
                                path.display()
                            )),
                            Err(e) => {
                                call.update(cx, |v, cx| v.say(&e, cx));
                                return None;
                            }
                        }
                    }
                };
                call.update(cx, |v, cx| match note {
                    Some(n) => v.attach = Some(n),
                    None => v.say("I can't see that from here. Try the diff or the browser panel.", cx),
                });
                None
            }
            Action::Launch { project, items } => self.voice_launch(call, &project, items, window, cx),
            Action::AskTeammate { teammate, text } => {
                let id = self.ask_teammate(&teammate, &text, window, cx)?;
                if let Some(chat) = self.threads.get(&id).map(|v| v.read(cx).chat.clone()) {
                    call.update(cx, |v, cx| v.read_replies(id, &chat, cx));
                }
                Some(id)
            }
            Action::Draft { thread, text } => {
                let id = self.task_by_str(&thread)?;
                let view = self.thread_view_for(id, window, cx)?;
                // The draft shows where the user can see it.
                match self.route {
                    Route::Thread(main) if main == id => {}
                    Route::Thread(_) => self.split_with(id, window, cx),
                    _ => self.open_task(id, window, cx),
                }
                view.update(cx, |v, cx| v.draft(&text, window, cx));
                Some(id)
            }
            Action::SendDraft { thread } => {
                let id = self.task_by_str(&thread)?;
                let view = self.threads.get(&id)?.clone();
                let chat = view.read(cx).chat.clone();
                if view.update(cx, |v, cx| v.send_draft(window, cx)) {
                    call.update(cx, |v, cx| {
                        v.current = Some(id);
                        v.read_replies(id, &chat, cx);
                    });
                } else {
                    call.update(cx, |v, cx| v.say("The box was empty, so nothing went out.", cx));
                }
                Some(id)
            }
            Action::ClearDraft { thread } => {
                let id = self.task_by_str(&thread)?;
                let view = self.threads.get(&id)?.clone();
                view.update(cx, |v, cx| v.clear_draft(window, cx));
                Some(id)
            }
            Action::OpenBeside { thread } => {
                let id = self.task_by_str(&thread)?;
                self.split_with(id, window, cx);
                Some(id)
            }
            Action::Status { .. }
            | Action::TeamStatus { .. }
            | Action::SetVoiceStyle { .. }
            | Action::SwitchBack
            | Action::EndCall => None,
        }
    }

    /// Open every thread a spoken launch asked for, in chat or the agent's
    /// terminal UI. The first becomes the open thread and the rest sit in
    /// panes beside it, so the user sees them all start.
    fn voice_launch(
        &mut self,
        call: &Entity<VoiceCall>,
        project: &str,
        items: Vec<dove_voice::intent::LaunchItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TaskId> {
        let project = self.projects.iter().find(|p| p.id.to_string() == project)?.clone();
        let mut opened: Vec<TaskId> = Vec::new();
        let mut failed = Vec::new();
        for item in items {
            let Some(kind) = HarnessKind::from_id(&item.agent) else { continue };
            let persona = item.teammate.as_deref().and_then(|id| self.persona(id)).cloned();
            let prompt = item.prompt.clone().unwrap_or_default();
            let title = if prompt.is_empty() {
                format!("{} terminal", persona.as_ref().map_or(kind.label(), |p| p.name.as_str()))
            } else {
                crate::ui::title_from_prompt(&prompt)
            };
            let join = if item.worktree {
                match dove_core::git::create_worktree(&project.path, &dove_core::git::slug(&title), "dove/") {
                    Ok(w) => {
                        Some(dove_core::Joined { worktree: Some(w.path), branch: Some(w.branch), base: Some(w.base) })
                    }
                    Err(e) => {
                        failed.push(format!("a worktree couldn't be made: {e}"));
                        continue;
                    }
                }
            } else {
                None
            };
            let run = {
                let c = self.composer.read(cx);
                if c.harness(cx) == kind { c.run_settings(cx) } else { Default::default() }
            };
            let new = dove_core::NewTask { title, prompt, harness: kind, run, join, persona: persona.clone() };
            if item.terminal {
                match self.engine.create_thread(&project, new) {
                    Ok((task, first)) => {
                        let id = task.id;
                        if let Some(p) = &persona {
                            self.persona_of.insert(id, p.id.clone());
                        }
                        self.tasks.insert(0, task.clone());
                        let view = self.make_thread(task, project.clone(), None, window, cx);
                        view.update(cx, |v, cx| v.open_terminal(Some(first), window, cx));
                        opened.push(id);
                    }
                    Err(e) => failed.push(format!("{e:#}")),
                }
            } else {
                match self.engine.start_task(&project, new) {
                    Ok((task, live)) => {
                        let id = task.id;
                        if let Some(p) = &persona {
                            self.persona_of.insert(id, p.id.clone());
                        }
                        self.tasks.insert(0, task.clone());
                        self.make_thread(task, project.clone(), Some(live), window, cx);
                        opened.push(id);
                    }
                    Err(e) => failed.push(format!("{e:#}")),
                }
            }
        }
        let first = *opened.first()?;
        self.navigate(Route::Thread(first), window, cx);
        for id in opened.iter().skip(1) {
            self.split_with(*id, window, cx);
        }
        if let Some(chat) = self.threads.get(&first).map(|v| v.read(cx).chat.clone()) {
            call.update(cx, |v, cx| {
                v.current = Some(first);
                v.read_replies(first, &chat, cx);
            });
        }
        if !failed.is_empty() {
            let n = failed.len();
            call.update(cx, |v, cx| {
                v.say(&format!("{n} didn't start: {}.", failed.join("; ")), cx);
            });
        }
        cx.notify();
        Some(first)
    }

    /// Ask teammate `id` something as the user: their latest thread in the
    /// open project gets it (steering it if it's working), or a new thread
    /// starts as them. Returns the thread.
    pub(crate) fn ask_teammate(
        &mut self,
        id: &str,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TaskId> {
        let mate = self.persona(id).cloned()?;
        let project_id = match self.route {
            Route::Thread(t) => self.task(t).map(|t| t.project_id),
            _ => None,
        }
        .or(self.composer.read(cx).project.as_ref().map(|p| p.id))
        .or(self.home_project)?;
        let latest = self
            .tasks
            .iter()
            .filter(|t| t.project_id == project_id && t.status != TaskStatus::Draft)
            .filter(|t| self.persona_of.get(&t.id).is_some_and(|p| *p == mate.id))
            .max_by_key(|t| t.updated_at)
            .map(|t| t.id);
        if let Some(tid) = latest {
            let chat = self.chat_of(tid, window, cx)?;
            let text = text.to_string();
            chat.update(cx, |c, cx| c.submit(text, window, cx));
            return Some(tid);
        }
        let project = self.project(project_id)?.clone();
        let kind = mate.harness.unwrap_or_else(|| self.composer.read(cx).harness(cx));
        let run = dove_core::RunSettings {
            model: mate.model.clone().filter(|_| mate.harness.is_some()),
            ..Default::default()
        };
        let new = dove_core::NewTask {
            title: crate::ui::title_from_prompt(text),
            prompt: text.to_string(),
            harness: kind,
            run,
            join: None,
            persona: Some(mate.clone()),
        };
        match self.engine.start_task(&project, new) {
            Ok((task, live)) => {
                let tid = task.id;
                self.persona_of.insert(tid, mate.id.clone());
                self.tasks.insert(0, task.clone());
                self.make_thread(task, project, Some(live), window, cx);
                cx.notify();
                Some(tid)
            }
            Err(e) => {
                crate::kit::Toasts::push(
                    cx,
                    format!("{} couldn't start", mate.name),
                    Some(format!("{e:#}")),
                    crate::kit::Tone::Danger,
                    None,
                );
                None
            }
        }
    }

    /// The call strip on pages without an input of their own (Settings,
    /// Threads, Kanban…), floating at the foot of the content area. A thread
    /// and home draw it right above their composer instead.
    pub fn render_voice_bar(&self, _cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let call = self.voice_call.clone()?;
        if matches!(self.route, Route::Home | Route::Thread(_)) && !self.projects.is_empty() {
            return None;
        }
        let left = if self.sidebar_shown() { self.sidebar_w } else { 0.0 };
        Some(
            div()
                .absolute()
                .bottom(px(20.0))
                .left(px(left))
                .right_0()
                .px(px(24.0))
                .flex()
                .justify_center()
                .child(div().w_full().max_w(px(STRIP_MAX_W)).child(call))
                .into_any_element(),
        )
    }
}

/// The floating strip's width; above a composer it takes the composer's.
const STRIP_MAX_W: f32 = 560.0;
const STRIP_H: f32 = 48.0;
/// The level as the waveform draws it: eight steps.
fn level_step(level: f32) -> u8 {
    (level.clamp(0.0, 0.3) / 0.3 * 8.0).round() as u8
}

/// The waveform bars' share of the level, left to right.
const WAVE: [f32; 4] = [0.55, 1.0, 0.8, 0.45];
const ORB: f32 = 32.0;

impl Render for VoiceCall {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(cx).clone();
        let awaiting = self.awaiting_click.is_some();
        let loading = self.state == State::Loading;
        let (label, tint): (&str, _) = if awaiting {
            ("Click Approve in the chat to run it", t.warning)
        } else if self.muted {
            ("Muted · hold the orb or Space to talk", t.text_muted)
        } else {
            match self.state {
                State::Loading => ("Starting the call…", t.text_muted),
                State::Listening => ("Listening", t.accent),
                State::Hearing => ("Hearing you", t.accent),
                State::Thinking => ("Working on it", t.text_muted),
                State::Speaking => ("Speaking · talk to interrupt", t.text),
                State::Muted => ("Muted", t.text_muted),
            }
        };
        // What was just heard or said; a live partial while the user talks.
        let (line, line_color): (SharedString, _) = match (&self.error, &self.partial, &self.caption) {
            (Some(e), _, _) => (e.clone(), t.danger),
            (None, Some(p), _) => (format!("{p}…").into(), t.text_muted),
            (None, None, Some(c)) => (c.clone(), t.text),
            // Nothing said yet: what talking does from here.
            (None, None, None) if loading => (SharedString::default(), t.text),
            (None, None, None) if self.new_thread => {
                ("Say what you'd like done. A new thread starts on your picked model.".into(), t.text_faint)
            }
            (None, None, None) => ("Talk to this thread. Ask what's happening, or say stop.".into(), t.text_faint),
        };
        let level = if self.muted { 0.0 } else { self.level.clamp(0.0, 0.3) / 0.3 };
        let speaking = self.state == State::Speaking && !self.muted;
        let muted = self.muted;
        // A question the call is waiting on: several threads to open, or a
        // draft to send. It replaces the caption, with its two answers.
        let pending: Option<(SharedString, &'static str, &'static str)> = match self.conductor.pending_plan() {
            Some(Action::Launch { items, .. }) => {
                let kind = if items.first().is_some_and(|i| i.terminal) { "terminals" } else { "chats" };
                Some((format!("Open {} {kind}?", items.len()).into(), "Open", "Not now"))
            }
            _ => self.conductor.pending_draft().map(|(_, text)| (format!("Draft: {text}").into(), "Send", "Clear")),
        };
        let (line, line_color) = match &pending {
            Some((q, _, _)) => (q.clone(), t.text),
            None => (line, line_color),
        };

        // The orb follows the voice level; holding it talks while muted.
        let orb =
            div()
                .id("voice-orb")
                .size(px(ORB))
                .flex_none()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(tint.opacity(0.14))
                .cursor_pointer()
                .tooltip(crate::ui::tip("Hold to talk"))
                .child(if muted {
                    icon(icons::MICROPHONE_OFF).size(px(14.0)).text_color(t.text_muted).into_any_element()
                } else {
                    // A small live waveform: four bars that follow the level.
                    let alpha = if loading { 0.35 } else { 0.9 };
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.0))
                        .children(WAVE.iter().map(|k| {
                            div().w(px(3.0)).h(px(4.0 + 12.0 * level * k)).rounded_full().bg(tint.opacity(alpha))
                        }))
                        .into_any_element()
                })
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(|v, _, _, _| v.push_to_talk(true)))
                .on_mouse_up(gpui::MouseButton::Left, cx.listener(|v, _, _, _| v.push_to_talk(false)));

        let text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .min_w_0()
                    .text_size(ty::META)
                    .child(div().flex_none().text_color(tint).font_weight(gpui::FontWeight::MEDIUM).child(label))
                    .children(self.target.clone().map(|target| {
                        div().min_w_0().truncate().text_color(t.text_faint).child(format!("· {target}"))
                    })),
            )
            .when(!line.is_empty(), |d| {
                d.child(div().min_w_0().truncate().text_size(ty::BODY).text_color(line_color).child(line))
            });

        div()
            .id("voice-strip")
            .occlude()
            .w_full()
            .h(px(STRIP_H))
            .px(px((STRIP_H - ORB) / 2.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .rounded(px(STRIP_H / 2.0))
            // The composer's own surface: it reads as part of the input, and
            // stays legible where it floats over a page.
            .bg(t.composer_bg())
            .border_1()
            .border_color(if awaiting { t.warning.opacity(0.5) } else { t.composer_border() })
            .shadow(t.composer_shadows(false))
            .child(orb)
            .child(text)
            .when_some(pending, |d, (_, yes, no)| {
                let answer = |id: &'static str, label: &'static str, word: &'static str, primary: bool| {
                    div()
                        .id(id)
                        .h(px(28.0))
                        .px(px(12.0))
                        .flex()
                        .flex_none()
                        .items_center()
                        .rounded_full()
                        .cursor_pointer()
                        .text_size(ty::META)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .when(primary, |d| d.bg(t.accent).text_color(gpui::white()))
                        .when(!primary, |d| d.bg(t.wash(0.07)).text_color(t.text).hover(|s| s.bg(t.wash(0.11))))
                        .child(label)
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(VoiceCallEvent::Answer(word))))
                };
                d.child(answer("voice-yes", yes, "yes", true)).child(answer("voice-no", no, "no", false))
            })
            .when(!loading, |d| {
                let secs = self.started.elapsed().as_secs();
                d.child(div().flex_none().text_size(ty::META).text_color(t.text_faint).child(format!(
                    "{}:{:02}",
                    secs / 60,
                    secs % 60
                )))
            })
            .when(speaking, |d| {
                d.child(
                    crate::ui::round_button_sized("voice-silence", "Stop speaking (the agent keeps working)", 30.0, &t)
                        .child(icon(icons::STOP).size(px(12.0)).text_color(t.text_muted))
                        .on_click(cx.listener(|v, _, _, cx| v.silence(cx))),
                )
            })
            .child(
                crate::ui::round_button_sized(
                    "voice-mute",
                    if muted { "Unmute microphone" } else { "Mute microphone" },
                    30.0,
                    &t,
                )
                .child(
                    icon(if muted { icons::MICROPHONE_OFF } else { icons::MICROPHONE })
                        .size(px(14.0))
                        .text_color(if muted { t.danger } else { t.text_muted }),
                )
                .on_click(cx.listener(move |v, _, _, cx| v.set_muted(!muted, cx))),
            )
            .child(
                crate::ui::round_button_sized("voice-open-view", "Open the call view", 30.0, &t)
                    .child(icon(icons::EXPAND_ARROWS).size(px(13.0)).text_color(t.text_muted))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(VoiceCallEvent::ShowView))),
            )
            .child(
                crate::ui::round_button_sized("voice-end", "End call", 30.0, &t)
                    .bg(t.danger.opacity(0.16))
                    .child(icon(icons::PHONE_HANG_UP).size(px(14.0)).text_color(t.danger))
                    .on_click(cx.listener(|v, _, _, cx| v.end(cx))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortstat_parses_git_output() {
        assert_eq!(
            parse_shortstat(" 3 files changed, 120 insertions(+), 4 deletions(-)\n"),
            DiffStat { files: 3, added: 120, removed: 4 }
        );
        assert_eq!(parse_shortstat(" 1 file changed, 1 deletion(-)"), DiffStat { files: 1, added: 0, removed: 1 });
        assert_eq!(parse_shortstat(""), DiffStat::default());
    }

    #[test]
    fn task_statuses_map_to_spoken_ones() {
        assert_eq!(status_of(TaskStatus::Running), ThreadStatus::Running);
        assert_eq!(status_of(TaskStatus::Draft), ThreadStatus::Idle);
        assert_eq!(status_of(TaskStatus::NeedsInput), ThreadStatus::NeedsInput);
    }

    #[test]
    fn engines_default_to_local_and_never_use_the_cloud_unasked() {
        let dir = std::env::temp_dir().join(format!("dove-voice-config-{}", std::process::id()));
        let engine = Engine::new(dir.clone()).unwrap();
        let c = config(&engine);
        // Nothing installed: the call falls back to Parakeet's (missing) path
        // and says so when it starts; it never reaches for the cloud.
        assert!(matches!(c.stt, SttChoice::Parakeet(_)));
        assert!(matches!(c.tts, TtsChoice::System));
        assert!(c.echo_cancel && !c.push_to_talk);
        // A URL alone does not switch anything to the cloud.
        engine.store.set_setting("voice_cloud_url", "https://api.example.com/v1").unwrap();
        assert!(matches!(config(&engine).stt, SttChoice::Parakeet(_)));
        engine.store.set_setting("voice_cloud", "true").unwrap();
        assert!(matches!(config(&engine).stt, SttChoice::Cloud { .. }));
        assert!(matches!(config(&engine).tts, TtsChoice::System), "cloud speech output is a second, separate opt-in");
        engine.store.set_setting("voice_cloud_stt_provider", "deepgram").unwrap();
        assert!(matches!(config(&engine).stt, SttChoice::Deepgram { .. }));
        engine.store.set_setting("voice_cloud_tts", "true").unwrap();
        assert!(matches!(config(&engine).tts, TtsChoice::Cloud { .. }));
        engine.store.set_setting("voice_cloud_tts_provider", "elevenlabs").unwrap();
        assert!(matches!(config(&engine).tts, TtsChoice::ElevenLabs { .. }));
        engine.store.set_setting("voice_cloud_tts_provider", "fish").unwrap();
        engine.store.set_setting("voice_tts_voice.fish", "abc123").unwrap();
        match config(&engine).tts {
            TtsChoice::Stream(dove_voice::tts_stream::Provider::FishAudio { voice, model, .. }) => {
                assert_eq!(voice, "abc123");
                assert_eq!(model, "", "empty model: the free S2.1 Pro");
            }
            _ => panic!("Fish Audio should stream"),
        }
        engine.store.set_setting("voice_cloud_tts_provider", "deepgram").unwrap();
        assert!(matches!(config(&engine).tts, TtsChoice::Stream(dove_voice::tts_stream::Provider::Deepgram { .. })));
        engine.store.set_setting("voice_cloud", "false").unwrap();
        assert!(
            !matches!(config(&engine).stt, SttChoice::Deepgram { .. } | SttChoice::Cloud { .. }),
            "off means local"
        );
        drop(engine);
        let _ = std::fs::remove_dir_all(dir);
    }
}
