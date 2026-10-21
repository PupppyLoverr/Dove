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
//! keeps it going. Ending the call (or five idle minutes after it) unloads
//! the voice models.

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
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
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
}

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
    router: Option<Router>,
    _pump: gpui::Task<()>,
}

impl EventEmitter<VoiceCallEvent> for VoiceCall {}

/// A model router over an OpenAI-compatible chat endpoint (a local
/// llama.cpp server or a cloud model), when the user configured one.
#[derive(Clone)]
struct Router {
    url: String,
    model: String,
    key: Option<String>,
}

impl Router {
    fn from_settings(engine: &Engine) -> Option<Self> {
        let url = engine.store.setting("voice_router_url").filter(|u| !u.trim().is_empty())?;
        let model = engine.store.setting("voice_router_model").unwrap_or_else(|| "default".into());
        Some(Self { url: url.trim_end_matches('/').to_string(), model, key: std::env::var("DOVE_VOICE_API_KEY").ok() })
    }

    fn ask(&self, prompt: String) -> anyhow::Result<String> {
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(8)).build()?;
        let mut req = client.post(format!("{}/chat/completions", self.url)).json(&serde_json::json!({
            "model": self.model,
            "temperature": 0,
            "max_tokens": 200,
            "messages": [{"role": "user", "content": prompt}],
        }));
        if let Some(k) = &self.key {
            req = req.bearer_auth(k);
        }
        let v: serde_json::Value = req.send()?.error_for_status()?.json()?;
        Ok(v.pointer("/choices/0/message/content").and_then(|c| c.as_str()).unwrap_or_default().to_string())
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
    let cloud_url = s.setting("voice_cloud_url").filter(|u| !u.trim().is_empty());
    let key = std::env::var("DOVE_VOICE_API_KEY").unwrap_or_default();
    let cloud = s.flag("voice_cloud", false) && cloud_url.is_some();
    let stt = match (&cloud_url, cloud) {
        (Some(url), true) => SttChoice::Cloud {
            base_url: url.clone(),
            api_key: key.clone(),
            model: s.setting("voice_cloud_stt_model").unwrap_or_else(|| "whisper-1".into()),
        },
        _ => local_stt(engine),
    };
    let tts = match (&cloud_url, cloud && s.flag("voice_cloud_tts", false)) {
        (Some(url), true) => TtsChoice::Cloud {
            base_url: url.clone(),
            api_key: key,
            model: s.setting("voice_cloud_tts_model").unwrap_or_else(|| "tts-1".into()),
            voice: s.setting("voice_cloud_voice").unwrap_or_else(|| "alloy".into()),
        },
        _ => TtsChoice::System,
    };
    CallConfig {
        root: engine.store.root().to_path_buf(),
        input: pipeline::Input::Device(s.setting("voice_device").filter(|d| !d.is_empty())),
        output: pipeline::Output::Device(None),
        push_to_talk: s.setting("voice_mode").as_deref() == Some("push"),
        echo_cancel: s.flag("voice_echo_cancel", true),
        rate: s.setting("voice_rate").and_then(|r| r.parse().ok()).unwrap_or(1.0),
        voice: s.setting("voice_name").filter(|v| !v.is_empty()),
        stt,
        tts,
        earcons: false,
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
        let router = Router::from_settings(&engine);
        Self {
            engine,
            call,
            conductor,
            state: State::Loading,
            partial: None,
            caption: Some("Starting…".into()),
            error: None,
            muted: false,
            level: 0.0,
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
            router,
            _pump: pump,
        }
    }

    pub fn has_router(&self) -> bool {
        self.router.is_some()
    }

    /// Drain pipeline events; true once the call has ended.
    fn drain(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        let mut ended = false;
        while let Ok(ev) = self.call.events.try_recv() {
            changed = true;
            match ev {
                Event::State(s) => self.state = s,
                Event::Ready { startup_ms, warm, stt, tts, vad, turn, echo_cancel } => {
                    self.startup_ms = Some(startup_ms);
                    self.caption = Some("Listening".into());
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
                    cx.emit(VoiceCallEvent::Heard(text));
                }
                Event::BargeIn { .. } => {
                    // The user talked over Dove: stop reading the reply.
                    self.stop_reading(cx);
                }
                Event::ReplyAudio { text_to_audio_ms } => {
                    tracing::info!("voice reply: first audio {text_to_audio_ms:.0} ms after first text");
                }
                Event::Level { mic, speaker } => self.level = mic.max(speaker),
                Event::Error(e) => {
                    self.error = Some(e.into());
                }
                Event::Ended => ended = true,
            }
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
                    this.call.say(clause);
                }
            }
            TaskViewEvent::VoiceTurnEnded { failed } => {
                for clause in this.chunker.finish() {
                    this.call.say(clause);
                }
                if *failed {
                    this.call.say("That turn ended with an error.");
                }
                this.stop_reading(cx);
            }
            _ => {}
        });
        self.reading = Some((id, sub));
        self.reply_started = false;
        self.chunker = Chunker::new();
        if !self.touched.contains(&id) {
            self.touched.push(id);
        }
    }

    fn stop_reading(&mut self, _cx: &mut Context<Self>) {
        self.reading = None;
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

    /// Ask the model router, off the UI thread.
    pub fn route(&mut self, utterance: String, snap: &Snapshot, cx: &mut Context<Self>) {
        let Some(router) = self.router.clone() else { return };
        let prompt = dove_voice::intent::router_prompt(&utterance, snap);
        let work = cx.background_spawn(async move { router.ask(prompt) });
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
    /// Start a call, or end the one running.
    pub fn toggle_voice_call(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(v) = self.voice_call.clone() {
            v.update(cx, |v, cx| v.end(cx));
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
        // Text already in the composer goes out first, as the user wrote it.
        let draft = match self.active_thread() {
            Some(v) => {
                let composer = v.read(cx).chat.read(cx).composer.clone();
                crate::composer::take_text(&composer, window, cx)
            }
            None => None,
        };
        if let (Some(text), Some(v)) = (draft.filter(|t| !t.trim().is_empty()), self.active_thread().cloned()) {
            let chat = v.read(cx).chat.clone();
            chat.update(cx, |c, cx| c.submit(text, window, cx));
        }
        let current = match self.route {
            Route::Thread(id) => Some(id),
            _ => None,
        };
        let engine = self.engine.clone();
        let call = cx.new(|cx| VoiceCall::start(engine, current, cx));
        let sub = cx.subscribe_in(&call, window, |this, call, ev: &VoiceCallEvent, window, cx| match ev {
            VoiceCallEvent::Heard(text) => this.voice_heard(call.clone(), text.clone(), window, cx),
            VoiceCallEvent::Observe => this.voice_observe(call.clone(), cx),
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
                cx.notify();
            }
        });
        self.voice_call = Some(call);
        self.voice_sub = Some(sub);
        cx.notify();
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
            });
        }
        let current = match self.route {
            Route::Thread(id) => Some(id),
            _ => call.current,
        };
        let default_project =
            current.and_then(|id| self.task(id)).map(|t| t.project_id).or(self.home_project).map(|p| p.to_string());
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
        }
    }

    fn voice_heard(&mut self, call: Entity<VoiceCall>, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let snap = self.voice_snapshot(call.read(cx), cx);
        let router = call.read(cx).has_router();
        let effects = call.update(cx, |v, _| v.conductor.utterance(&text, &snap, router));
        self.voice_apply(call, effects, Some(text), window, cx);
    }

    fn voice_observe(&mut self, call: Entity<VoiceCall>, cx: &mut Context<Self>) {
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
        let snap = self.voice_snapshot(call.read(cx), cx);
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
                let new = dove_core::NewTask {
                    title: crate::ui::title_from_prompt(&prompt),
                    prompt,
                    harness: kind,
                    run: Default::default(),
                    join,
                    persona: None,
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
                    dove_voice::intent::LookTarget::Window => None,
                };
                call.update(cx, |v, cx| match note {
                    Some(n) => v.attach = Some(n),
                    None => v.say("I can't see that from here. Try the diff or the browser panel.", cx),
                });
                None
            }
            Action::Status { .. } | Action::SetVoiceStyle { .. } | Action::SwitchBack | Action::EndCall => None,
        }
    }

    /// The floating call bar, shown on every page while a call runs.
    pub fn render_voice_bar(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let call = self.voice_call.clone()?;
        let t = Theme::of(cx).clone();
        let v = call.read(cx);
        let (label, color) = match (v.state, v.muted) {
            (_, true) => ("Muted · hold the orb to talk", t.text_muted),
            (State::Loading, _) => ("Loading voice models…", t.text_muted),
            (State::Listening, _) => ("Listening", t.accent),
            (State::Hearing, _) => ("Hearing you", t.accent),
            (State::Thinking, _) => ("Thinking", t.text_muted),
            (State::Speaking, _) => ("Speaking · talk to interrupt", t.text),
            (State::Muted, _) => ("Muted", t.text_muted),
        };
        let line: SharedString =
            v.error.clone().or_else(|| v.partial.clone()).or_else(|| v.caption.clone()).unwrap_or_default();
        let awaiting = v.awaiting_click.is_some();
        let level = v.level.clamp(0.0, 0.3) / 0.3;
        let orb = 26.0 + 10.0 * level;
        let muted = v.muted;
        let (c1, c2, c3, c4) = (call.clone(), call.clone(), call.clone(), call.clone());
        Some(
            div()
                .absolute()
                .bottom(px(20.0))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("voice-bar")
                        .occlude()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .px(px(10.0))
                        .py(px(8.0))
                        .max_w(px(560.0))
                        .rounded_full()
                        .bg(t.glass())
                        .border_1()
                        .border_color(t.wash(0.10))
                        .shadow_lg()
                        .child(
                            // The orb: its size follows the voice level. Holding it
                            // talks while muted (push-to-talk).
                            div()
                                .id("voice-orb")
                                .size(px(36.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .child(div().size(px(orb)).rounded_full().bg(color.opacity(if muted {
                                    0.25
                                } else {
                                    0.85
                                })))
                                .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                    c1.update(cx, |v, _| v.push_to_talk(true));
                                })
                                .on_mouse_up(gpui::MouseButton::Left, move |_, _, cx| {
                                    c2.update(cx, |v, _| v.push_to_talk(false));
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w(px(160.0))
                                .max_w(px(360.0))
                                .child(div().text_size(ty::META).text_color(t.text_muted).child(if awaiting {
                                    "Waiting for your click on Approve in the chat"
                                } else {
                                    label
                                }))
                                .child(div().text_size(ty::BODY).text_color(t.text).truncate().child(line)),
                        )
                        .child(
                            crate::ui::round_button_sized(
                                "voice-mute",
                                if muted { "Unmute microphone" } else { "Mute microphone" },
                                28.0,
                                &t,
                            )
                            .child(
                                icon(if muted { icons::MICROPHONE_OFF } else { icons::MICROPHONE })
                                    .size(px(14.0))
                                    .text_color(if muted { t.danger } else { t.text_muted }),
                            )
                            .on_click(move |_, _, cx| c3.update(cx, |v, cx| v.set_muted(!muted, cx))),
                        )
                        .child(
                            crate::ui::round_button_sized(
                                "voice-silence",
                                "Stop speaking (agent keeps working)",
                                28.0,
                                &t,
                            )
                            .child(icon(icons::STOP).size(px(12.0)).text_color(t.text_muted))
                            .on_click(move |_, _, cx| c4.update(cx, |v, cx| v.silence(cx))),
                        )
                        .child(
                            crate::ui::round_button_sized("voice-end", "End call", 28.0, &t)
                                .bg(t.danger.opacity(0.14))
                                .child(icon(icons::PHONE_HANG_UP).size(px(14.0)).text_color(t.danger))
                                .on_click(move |_, _, cx| call.update(cx, |v, cx| v.end(cx))),
                        ),
                )
                .into_any_element(),
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
        drop(engine);
        let _ = std::fs::remove_dir_all(dir);
    }
}
