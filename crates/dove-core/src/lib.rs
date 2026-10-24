//! The engine behind every Dove front end.
//!
//! It owns the tokio runtime, the store and the installed-harness list, and it
//! records every live session: each event is appended to the transcript, task
//! status and usage are updated in SQLite, and the event is then forwarded to
//! whoever is watching. A UI only consumes [`LiveSession::events`] and sends
//! [`Command`]s; it never writes to the store for session activity.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use dove_harness::detect::Installed;
use dove_harness::{SessionHandle, SessionSpec};
use dove_proto::{
    AgentEvent, HarnessKind, PermissionMode, RateWindow, ReasoningLevel, SlashCommand, TaskId, TaskStatus,
};
use dove_store::{Automation, Project, Store, Task};
use parking_lot::RwLock;

pub mod automation;
pub mod digest;
pub mod memory;
pub mod mirror;
pub mod orchestra;

pub use dove_git as git;
pub use dove_harness::Command;
pub use dove_harness::detect as harness_detect;
pub use dove_harness::is_supported as harness_supported;
pub use dove_harness::models;
pub use dove_harness::office_skills;
pub use dove_harness::process as harness_process;
pub use dove_harness::skills;
pub use dove_harness::teach;
pub use dove_harness::tui;
pub use dove_harness::walkthrough;
pub use dove_harness::{
    ToolServer, allow_ego_browser, is_dove_tool, set_allow_ego_browser, set_tool_server, tool_server,
};
pub use dove_proto as proto;
pub use dove_store as store;

pub struct Engine {
    pub store: Arc<Store>,
    rt: tokio::runtime::Runtime,
    installed: RwLock<Vec<Installed>>,
    shared: Arc<RwLock<Shared>>,
    /// Which driver owns each task's live session right now.
    drivers: Arc<parking_lot::Mutex<HashMap<TaskId, Driver>>>,
    /// Stops each chat session, even one its view let go of (it keeps
    /// recording in the background until the agent exits).
    stops: Arc<parking_lot::Mutex<HashMap<TaskId, flume::Sender<()>>>>,
    /// Live model catalogs; see [`Engine::models`].
    live_models: Arc<LiveModels>,
    /// One pre-started session, adopted by the next matching start.
    warm: Arc<parking_lot::Mutex<Option<Warm>>>,
    /// Session key (`DOVE_SESSION` in a session's tool server) to the
    /// thread that session belongs to, so tool calls know their caller.
    sessions: Arc<parking_lot::Mutex<HashMap<String, TaskId>>>,
}

/// How long a pre-started session waits for its first message.
const WARM_TTL: std::time::Duration = std::time::Duration::from_secs(240);

/// What a pre-started session must match to be adopted.
#[derive(Debug, Clone, PartialEq)]
struct WarmKey {
    kind: HarnessKind,
    cwd: PathBuf,
    model: Option<String>,
    reasoning: Option<ReasoningLevel>,
    options: Vec<(String, String)>,
    mode: PermissionMode,
    resume: Option<String>,
}

impl WarmKey {
    fn of(spec: &SessionSpec) -> Self {
        Self {
            kind: spec.kind,
            cwd: spec.cwd.clone(),
            model: spec.model.clone(),
            reasoning: spec.reasoning,
            options: spec.model_options.clone(),
            mode: spec.permission_mode,
            resume: spec.resume.clone(),
        }
    }
}

struct Warm {
    key: WarmKey,
    handle: SessionHandle,
    /// Its session key: registered to the thread that adopts it.
    session: String,
    at: std::time::Instant,
}

/// Whether a harness boots before the first message (its driver can hold
/// the prompt back while the CLI starts).
pub fn prewarms(kind: HarnessKind) -> bool {
    dove_harness::defers_prompt(kind)
}

/// Who runs a session: Dove's structured (chat) driver or the agent's own
/// terminal UI. Never both: they would write the same session file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    Chat,
    Terminal,
}

/// What sessions learn about their harness that outlives them.
#[derive(Default)]
struct Shared {
    rate_limits: HashMap<HarnessKind, Vec<RateWindow>>,
    commands: HashMap<HarnessKind, Vec<SlashCommand>>,
}

/// How a turn runs: the composer's mode, model and effort.
#[derive(Debug, Clone, Default)]
pub struct RunSettings {
    pub mode: PermissionMode,
    pub model: Option<String>,
    pub reasoning: Option<ReasoningLevel>,
    pub model_options: Vec<(String, String)>,
    /// Extra folders the agent may use (`--add-dir`, where supported).
    pub add_dirs: Vec<PathBuf>,
}

impl RunSettings {
    /// Model options plus Dove's own pass-through keys.
    fn options(&self) -> Vec<(String, String)> {
        let mut out = self.model_options.clone();
        if !self.add_dirs.is_empty() {
            let dirs = self.add_dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join("\n");
            out.push(("dove.addDirs".into(), dirs));
        }
        out
    }
}

/// How to start a task.
#[derive(Debug, Clone)]
pub struct NewTask {
    pub title: String,
    pub prompt: String,
    pub harness: HarnessKind,
    pub run: RunSettings,
    /// Join an existing task's folder instead (a side chat).
    pub join: Option<Joined>,
    /// The teammate this thread runs as.
    pub persona: Option<dove_store::Persona>,
}

const PERSONA_OPEN: &str = "<dove-persona>";
const PERSONA_CLOSE: &str = "</dove-persona>";

/// The brief that leads a teammate's first turn: who it is, how it works,
/// who else is on the team and how to reach them, and the notes it keeps
/// between threads. Tagged so the chat can hide it.
pub fn persona_preamble(
    p: &dove_store::Persona,
    notes: Option<&std::path::Path>,
    team: &[dove_store::Persona],
) -> String {
    let role = if p.role.trim().is_empty() { String::new() } else { format!(", {}", p.role.trim()) };
    let name = p.name.trim();
    let mut out = format!(
        "{PERSONA_OPEN}\nYou are {name}{role}, a member of the user's team in Dove. Work as {name} for this whole thread.\n"
    );
    if !p.instructions.trim().is_empty() {
        out.push_str(&format!("\nHow you work:\n{}\n", p.instructions.trim()));
    }
    let others: Vec<String> = team
        .iter()
        .filter(|t| t.id != p.id)
        .map(|t| match t.role.trim() {
            "" => format!("- {}", t.name.trim()),
            r => format!("- {} ({r})", t.name.trim()),
        })
        .collect();
    if !others.is_empty() {
        out.push_str(&format!(
            "\nYour teammates:\n{}\nWork with them through Dove's tools (the `dove` MCP server): `team_message` asks a \
             teammate something and their reply comes back to you, `delegate_task` hands a teammate (or any agent \
             and model) a task, and `thread_read` reads what they did. Ask a teammate when their role fits the work \
             better than yours.\n",
            others.join("\n")
        ));
    }
    if let Some(notes) = notes {
        out.push_str(&format!(
            "\nYour notes file is {}. If it exists, read it before you start. Before you finish, update it with \
             what you would want to know at the start of your next thread: decisions, conventions, open questions. \
             Keep it short.\n",
            notes.display()
        ));
    }
    out.push_str(&format!("{PERSONA_CLOSE}\n\n"));
    out
}

/// A prompt without its teammate brief (or recalled memories), as the user
/// wrote it.
pub fn strip_persona(text: &str) -> &str {
    let text = memory::strip(text);
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed.strip_prefix(PERSONA_OPEN)
        && let Some(end) = rest.find(PERSONA_CLOSE)
    {
        return rest[end + PERSONA_CLOSE.len()..].trim_start();
    }
    text
}

/// Where a side chat runs: another task's worktree and branch.
#[derive(Debug, Clone)]
pub struct Joined {
    pub worktree: Option<PathBuf>,
    pub branch: Option<String>,
    pub base: Option<String>,
}

impl Joined {
    pub fn of(task: &Task) -> Self {
        Self { worktree: task.worktree.clone(), branch: task.branch.clone(), base: task.base.clone() }
    }
}

/// A running session as the UI sees it.
pub struct LiveSession {
    pub task: TaskId,
    /// Recorded events, in order. Drain all that are ready once per frame.
    pub events: flume::Receiver<AgentEvent>,
    commands: flume::Sender<Command>,
    pub capabilities: dove_proto::Capabilities,
}

impl LiveSession {
    pub fn send(&self, cmd: Command) -> bool {
        self.commands.send(cmd).is_ok()
    }
}

/// Whether a message sent mid-turn steers the running turn (Zeron's drivers)
/// rather than waiting for it to end.
pub fn steers(kind: HarnessKind) -> bool {
    matches!(
        kind,
        HarnessKind::ClaudeCode
            | HarnessKind::Codex
            | HarnessKind::Cursor
            | HarnessKind::OpenCode
            | HarnessKind::Pi
            | HarnessKind::Devin
            | HarnessKind::Grok
            | HarnessKind::Hermes
            | HarnessKind::Antigravity
    )
}

/// Whether extra folders (`--add-dir`) reach the agent.
pub fn supports_add_dirs(kind: HarnessKind) -> bool {
    kind == HarnessKind::ClaudeCode
}

/// How an event moves a task's status. Shared by the recorder and any UI that
/// wants to update optimistically from the same stream.
pub fn next_status(event: &AgentEvent) -> Option<TaskStatus> {
    match event {
        AgentEvent::UserMessage { .. } | AgentEvent::PermissionResolved { .. } => Some(TaskStatus::Running),
        AgentEvent::PermissionRequested(_) => Some(TaskStatus::NeedsInput),
        AgentEvent::TurnEnded { is_error: true, .. } => Some(TaskStatus::Failed),
        AgentEvent::TurnEnded { .. } => Some(TaskStatus::Done),
        // Exit status depends on what the task was doing; see `Recorder`.
        _ => None,
    }
}

impl Engine {
    pub fn new(root: PathBuf) -> Result<Arc<Self>> {
        let store = Arc::new(Store::open(&root)?);
        memory::apply_settings(&store);
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("dove-io")
            .enable_all()
            .build()
            .context("starting tokio runtime")?;
        Ok(Arc::new(Self {
            store,
            rt,
            installed: RwLock::new(Vec::new()),
            shared: Default::default(),
            drivers: Default::default(),
            stops: Default::default(),
            live_models: Default::default(),
            warm: Default::default(),
            sessions: Default::default(),
        }))
    }

    /// The latest plan usage windows a session of `kind` reported (persisted,
    /// so the meter shows before the first turn of a new session).
    pub fn rate_limits(&self, kind: HarnessKind) -> Vec<RateWindow> {
        if let Some(w) = self.shared.read().rate_limits.get(&kind) {
            return w.clone();
        }
        let saved: Vec<RateWindow> = self
            .store
            .setting(&format!("rate_limits.{}", kind.id()))
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        self.shared.write().rate_limits.insert(kind, saved.clone());
        saved
    }

    /// The slash commands `kind` last advertised.
    pub fn commands(&self, kind: HarnessKind) -> Vec<SlashCommand> {
        self.shared.read().commands.get(&kind).cloned().unwrap_or_default()
    }

    pub fn runtime(&self) -> tokio::runtime::Handle {
        self.rt.handle().clone()
    }

    /// Re-scan PATH for agent CLIs. Cheap (no processes), but call it off the
    /// UI thread anyway. Model lists are not fetched here, so launching starts
    /// no agent processes: saved lists serve at once, a picker refreshes the
    /// agent it shows, and a daily sweep runs a while after launch.
    pub fn rescan_harnesses(&self) -> Vec<Installed> {
        let found = dove_harness::detect::scan();
        *self.installed.write() = found.clone();
        self.schedule_model_sweep();
        found
    }

    pub fn installed(&self) -> Vec<Installed> {
        self.installed.read().clone()
    }

    fn program(&self, kind: HarnessKind) -> Result<PathBuf> {
        // Settings › Agents can point an agent at a specific binary.
        if let Some(p) =
            self.store.setting(&format!("agent_path.{}", kind.id())).map(PathBuf::from).filter(|p| p.is_file())
        {
            return Ok(p);
        }
        if let Some(i) = self.installed.read().iter().find(|i| i.kind == kind) {
            return Ok(i.program.clone());
        }
        dove_harness::detect::find(kind).with_context(|| format!("{} is not installed", kind.label()))
    }

    /// Create a task in the project folder and start its first turn.
    pub fn start_task(&self, project: &Project, new: NewTask) -> Result<(Task, LiveSession)> {
        self.start_task_with(project, new, None, None)
    }

    /// A thread that starts in the agent's own terminal UI rather than in
    /// chat: recorded (idle, with its folder and teammate) but not run.
    /// Returns it with the first message to type into the terminal (the
    /// teammate's brief and the prompt), empty when there is no prompt.
    pub fn create_thread(&self, project: &Project, new: NewTask) -> Result<(Task, String)> {
        self.program(new.harness)?;
        let (worktree, branch, base) = match new.join.clone() {
            Some(j) => (j.worktree, j.branch, j.base),
            None => (None, dove_git::current_branch(&project.path).ok(), dove_git::head_sha(&project.path).ok()),
        };
        let task = Task {
            id: TaskId::new(),
            project_id: project.id,
            title: new.title,
            harness: new.harness,
            model: new.run.model.clone(),
            status: TaskStatus::Idle,
            worktree,
            branch,
            native_session_id: None,
            base,
            pinned: false,
            updated_at: dove_store::now_ms(),
            prompt: None,
            unread: false,
            automation: None,
        };
        self.store.insert_task(&task)?;
        self.save_add_dirs(task.id, &new.run.add_dirs);
        let first = match &new.persona {
            Some(p) => {
                self.store.assign_persona(task.id, &p.id)?;
                let notes = self.store.persona_notes_path(&p.id);
                let keep = self.store.setting("team_notes").as_deref() != Some("false");
                let team = self.store.personas().unwrap_or_default();
                format!("{}{}", persona_preamble(p, keep.then_some(notes.as_path()), &team), new.prompt)
            }
            None => new.prompt,
        };
        Ok((task, first.trim().to_string()))
    }

    fn start_task_with(
        &self,
        project: &Project,
        new: NewTask,
        draft: Option<&Task>,
        automation: Option<&str>,
    ) -> Result<(Task, LiveSession)> {
        let program = self.program(new.harness)?;
        let id = draft.map(|d| d.id).unwrap_or_default();
        let (cwd, worktree, branch, base) = if let Some(j) = new.join.clone() {
            let cwd = j.worktree.clone().unwrap_or_else(|| project.path.clone());
            (cwd, j.worktree, j.branch, j.base)
        } else {
            // Any folder works. Git details are recorded when the folder is a
            // repository, and left empty when it is not.
            let base = dove_git::head_sha(&project.path).ok();
            (project.path.clone(), None, dove_git::current_branch(&project.path).ok(), base)
        };
        let task = Task {
            id,
            project_id: project.id,
            title: new.title,
            harness: new.harness,
            model: new.run.model.clone(),
            status: TaskStatus::Running,
            worktree,
            branch,
            native_session_id: None,
            base,
            pinned: draft.is_some_and(|d| d.pinned),
            updated_at: dove_store::now_ms(),
            prompt: None,
            unread: false,
            automation: automation.map(str::to_string),
        };
        if draft.is_some() {
            self.store.update_task(&task)?;
        } else {
            self.store.insert_task(&task)?;
        }
        self.save_add_dirs(task.id, &new.run.add_dirs);
        let prompt = match &new.persona {
            Some(p) => {
                self.store.assign_persona(task.id, &p.id)?;
                let notes = self.store.persona_notes_path(&p.id);
                if let Some(dir) = notes.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let keep = self.store.setting("team_notes").as_deref() != Some("false");
                let team = self.store.personas().unwrap_or_default();
                format!("{}{}", persona_preamble(p, keep.then_some(notes.as_path()), &team), new.prompt)
            }
            None => new.prompt,
        };
        let spec = SessionSpec {
            kind: new.harness,
            program,
            cwd,
            model: new.run.model.clone(),
            reasoning: new.run.reasoning,
            model_options: new.run.options(),
            permission_mode: new.run.mode,
            resume: None,
            prompt: Some(prompt),
            fork: false,
            session: dove_harness::new_session_key(),
        };
        if memory::recalls(&self.store, project.id) {
            let live = self.launch_deferred(&task, spec)?;
            return Ok((task, live));
        }
        // A matching pre-started session takes the prompt straight away.
        if let Some((handle, session)) = self.take_warm(&spec)
            && let Some(prompt) = spec.prompt.clone()
            && handle.send(Command::Send(prompt))
        {
            let live = self.attach(&task, handle, session)?;
            return Ok((task, live));
        }
        let live = self.launch(&task, spec)?;
        Ok((task, live))
    }

    /// Start the harness for a thread the user is about to send in `cwd`,
    /// so the first reply does not wait on the agent's cold start (process,
    /// settings, MCP servers). Cheap to call repeatedly: a warm session with
    /// the same settings is kept; a different one replaces it.
    pub fn prewarm(&self, kind: HarnessKind, cwd: &std::path::Path, run: &RunSettings) {
        let Ok(program) = self.program(kind) else { return };
        self.warm_up(SessionSpec {
            kind,
            program,
            cwd: cwd.to_path_buf(),
            model: run.model.clone(),
            reasoning: run.reasoning,
            model_options: run.options(),
            permission_mode: run.mode,
            resume: None,
            prompt: None,
            fork: false,
            session: dove_harness::new_session_key(),
        });
    }

    /// Pre-start a reopened thread's agent (resuming its session) while a
    /// follow-up is typed. No-op while the thread is already live.
    pub fn prewarm_resume(&self, task: &Task, project: &Project, run: &RunSettings) {
        if task.native_session_id.is_none() || self.drivers.lock().contains_key(&task.id) {
            return;
        }
        if let Ok(spec) = self.resume_spec(task, project, None, run) {
            self.warm_up(spec);
        }
    }

    fn warm_up(&self, spec: SessionSpec) {
        if !prewarms(spec.kind) || self.store.setting("prewarm").as_deref() == Some("false") {
            return;
        }
        let key = WarmKey::of(&spec);
        let mut warm = self.warm.lock();
        if warm.as_ref().is_some_and(|w| w.key == key && w.at.elapsed() < WARM_TTL && w.handle.is_alive()) {
            return;
        }
        // Dropping the old handle kills its process.
        *warm = None;
        let session = spec.session.clone();
        let Ok(handle) = dove_harness::spawn(self.rt.handle(), spec) else { return };
        let at = std::time::Instant::now();
        *warm = Some(Warm { key, handle, session, at });
        drop(warm);
        // Expire it if nobody sends.
        let slot = self.warm.clone();
        self.rt.spawn(async move {
            tokio::time::sleep(WARM_TTL).await;
            let mut w = slot.lock();
            if w.as_ref().is_some_and(|w| w.at == at) {
                *w = None;
            }
        });
    }

    /// Stop any pre-started session (settings changed, app quitting).
    pub fn cool(&self) {
        *self.warm.lock() = None;
    }

    /// Stop a pre-started session that resumed `native`, so another driver
    /// (the agent's terminal UI) can open that session file.
    pub fn cool_session(&self, native: &str) {
        let mut warm = self.warm.lock();
        if warm.as_ref().is_some_and(|w| w.key.resume.as_deref() == Some(native)) {
            *warm = None;
        }
    }

    /// Ask `task`'s chat session to stop, including one no view holds any
    /// more. Returns whether a session was there to ask.
    pub fn stop_chat(&self, task: TaskId) -> bool {
        self.stops.lock().get(&task).is_some_and(|s| s.send(()).is_ok())
    }

    /// Whether `kind` has a terminal UI Dove can hand a thread to.
    pub fn has_terminal_ui(&self, kind: HarnessKind) -> bool {
        kind == HarnessKind::Shell
            || self.program(kind).ok().is_some_and(|p| tui::interactive_program(kind, &p).is_some())
    }

    /// The pre-started session matching `spec`, with its session key.
    fn take_warm(&self, spec: &SessionSpec) -> Option<(SessionHandle, String)> {
        let mut warm = self.warm.lock();
        let ok = warm.as_ref().is_some_and(|w| {
            w.key == WarmKey::of(spec) && w.at.elapsed() < WARM_TTL && w.handle.is_alive() && w.handle.events.is_empty()
        });
        if ok { warm.take().map(|w| (w.handle, w.session)) } else { None }
    }

    /// The thread a tool call with session key `key` comes from.
    pub fn task_for_session(&self, key: &str) -> Option<TaskId> {
        if key.is_empty() {
            return None;
        }
        self.sessions.lock().get(key).copied()
    }

    fn register_session(&self, key: &str, task: TaskId) {
        if !key.is_empty() {
            self.sessions.lock().insert(key.to_string(), task);
        }
    }

    /// Re-attach to a task's harness session, optionally with a new message.
    pub fn resume_task(
        &self,
        task: &Task,
        project: &Project,
        prompt: Option<String>,
        run: RunSettings,
    ) -> Result<LiveSession> {
        let spec = self.resume_spec(task, project, prompt, &run)?;
        if spec.prompt.is_some() && memory::recalls(&self.store, project.id) {
            return self.launch_deferred(task, spec);
        }
        if let Some((handle, session)) = self.take_warm(&spec)
            && let Some(prompt) = spec.prompt.clone()
            && handle.send(Command::Send(prompt))
        {
            return self.attach(task, handle, session);
        }
        self.launch(task, spec)
    }

    fn resume_spec(
        &self,
        task: &Task,
        project: &Project,
        prompt: Option<String>,
        run: &RunSettings,
    ) -> Result<SessionSpec> {
        Ok(SessionSpec {
            kind: task.harness,
            program: self.program(task.harness)?,
            cwd: task.worktree.clone().unwrap_or_else(|| project.path.clone()),
            model: run.model.clone().or_else(|| task.model.clone()),
            reasoning: run.reasoning,
            model_options: run.options(),
            permission_mode: run.mode,
            resume: task.native_session_id.clone(),
            prompt,
            fork: false,
            session: dove_harness::new_session_key(),
        })
    }

    /// A Kanban draft: a thread with a prompt that has not started.
    pub fn create_draft(&self, project: &Project, title: &str, prompt: &str, harness: HarnessKind) -> Result<Task> {
        let task = Task {
            id: TaskId::new(),
            project_id: project.id,
            title: if title.trim().is_empty() { "Untitled draft".into() } else { title.trim().to_string() },
            harness,
            model: None,
            status: TaskStatus::Draft,
            worktree: None,
            branch: None,
            native_session_id: None,
            base: None,
            pinned: false,
            updated_at: dove_store::now_ms(),
            prompt: Some(prompt.to_string()),
            unread: false,
            automation: None,
        };
        self.store.insert_task(&task)?;
        Ok(task)
    }

    /// Start a draft with its saved prompt.
    pub fn start_draft(&self, draft: &Task, project: &Project, run: RunSettings) -> Result<(Task, LiveSession)> {
        let prompt = draft.prompt.clone().filter(|p| !p.trim().is_empty()).context("this draft has no prompt yet")?;
        let new = NewTask {
            title: draft.title.clone(),
            prompt,
            harness: draft.harness,
            run,
            join: None,
            // A draft handed to a teammate starts as that teammate.
            persona: self
                .store
                .persona_assignments()
                .unwrap_or_default()
                .into_iter()
                .find(|(t, _)| *t == draft.id)
                .and_then(|(_, id)| self.store.personas().ok()?.into_iter().find(|p| p.id == id)),
        };
        self.start_task_with(project, new, Some(draft), None)
    }

    /// Automations whose schedule fired since their last run.
    pub fn due_schedules(&self) -> Vec<Automation> {
        let now = chrono::Local::now();
        self.store
            .automations()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.enabled)
            .filter(|a| {
                automation::Trigger::parse(&a.trigger).is_some_and(|t| {
                    t.is_schedule() && automation::schedule_due(&t, a.last_run, dove_store::now_ms(), now)
                })
            })
            .collect()
    }

    /// Check event triggers (git and `gh` calls; run off the UI thread).
    /// Returns automations to run with the context that fired them. The first
    /// check only records what exists, so enabling never fires on old events.
    pub fn due_events(&self) -> Vec<(Automation, String)> {
        let mut out = Vec::new();
        let projects = self.store.projects().unwrap_or_default();
        for mut a in self.store.automations().unwrap_or_default().into_iter().filter(|a| a.enabled) {
            let Some(trigger) = automation::Trigger::parse(&a.trigger) else { continue };
            let Some(project) = projects.iter().find(|p| p.id == a.project_id) else { continue };
            let mut state: serde_json::Value = serde_json::from_str(&a.state).unwrap_or_default();
            if !state.is_object() {
                state = serde_json::json!({});
            }
            let first = state.get("seen").is_none();
            let fired: Option<String> = match &trigger {
                automation::Trigger::OnCommit { branch } => {
                    let head = dove_git::rev(&project.path, branch).ok();
                    let seen = state["seen"].as_str().map(str::to_string);
                    match head {
                        Some(h) if seen.as_deref() != Some(h.as_str()) => {
                            state["seen"] = serde_json::Value::String(h.clone());
                            (!first).then(|| format!("New commit {} on {branch}.", &h[..h.len().min(10)]))
                        }
                        _ => None,
                    }
                }
                automation::Trigger::OnPullRequest { review_requested } => {
                    let filter =
                        if *review_requested { git::github::PrFilter::Reviewing } else { git::github::PrFilter::All };
                    match git::github::pull_requests(&project.path, filter) {
                        Ok(prs) => {
                            let seen: Vec<u64> = state["seen"]
                                .as_array()
                                .map(|a| a.iter().filter_map(|v| v.as_u64()).collect())
                                .unwrap_or_default();
                            let new: Vec<_> = prs.iter().filter(|p| !seen.contains(&p.number)).collect();
                            state["seen"] = serde_json::json!(prs.iter().map(|p| p.number).collect::<Vec<_>>());
                            (!first && !new.is_empty()).then(|| {
                                new.iter()
                                    .map(|p| format!("PR #{} \"{}\" by {} ({})", p.number, p.title, p.author, p.url))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                        }
                        Err(e) => {
                            tracing::debug!("automation {}: {e:#}", a.name);
                            None
                        }
                    }
                }
                _ => continue,
            };
            a.state = state.to_string();
            let _ = self.store.save_automation(&a);
            if let Some(context) = fired {
                out.push((a, context));
            }
        }
        out
    }

    /// Start a thread for an automation (scheduled, event or "Run now").
    pub fn run_automation(&self, a: &Automation, context: Option<&str>) -> Result<(Task, LiveSession)> {
        let project = self
            .store
            .projects()?
            .into_iter()
            .find(|p| p.id == a.project_id)
            .context("the automation's project was removed")?;
        let prompt = match context {
            Some(c) => format!("{}\n\nContext:\n{c}", a.prompt),
            None => a.prompt.clone(),
        };
        let stamp = chrono::Local::now().format("%b %-d %H:%M");
        let persona = self
            .store
            .setting(&format!("automation_persona.{}", a.id))
            .filter(|id| !id.is_empty())
            .and_then(|id| self.store.personas().ok()?.into_iter().find(|p| p.id == id));
        let new = NewTask {
            title: format!("{} · {stamp}", a.name),
            prompt,
            // A teammate with a preferred agent brings it (and its model);
            // one without runs on the automation's agent.
            harness: persona.as_ref().and_then(|p| p.harness).unwrap_or(a.harness),
            run: RunSettings {
                model: persona
                    .as_ref()
                    .filter(|p| p.harness.is_some())
                    .and_then(|p| p.model.clone())
                    .or_else(|| a.model.clone()),
                ..Default::default()
            },
            join: None,
            persona,
        };
        let mut updated = a.clone();
        updated.last_run = Some(dove_store::now_ms());
        self.store.save_automation(&updated)?;
        let (task, live) = self.start_task_with(&project, new, None, Some(&a.id))?;
        self.store.record_run(&a.id, task.id)?;
        Ok((task, live))
    }

    /// Extra folders a task was started with.
    pub fn add_dirs(&self, task: TaskId) -> Vec<PathBuf> {
        self.store
            .setting(&format!("add_dirs.{task}"))
            .map(|s| s.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect())
            .unwrap_or_default()
    }

    fn save_add_dirs(&self, task: TaskId, dirs: &[PathBuf]) {
        if !dirs.is_empty() {
            let joined = dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join("\n");
            let _ = self.store.set_setting(&format!("add_dirs.{task}"), &joined);
        }
    }

    /// Whether `fork_task` works for this task's agent.
    pub fn can_fork(task: &Task) -> bool {
        task.harness == HarnessKind::ClaudeCode && task.native_session_id.is_some()
    }

    /// A new task that starts from a copy of `task`'s conversation, in the
    /// same worktree. The original continues untouched.
    pub fn fork_task(&self, task: &Task, project: &Project, run: RunSettings) -> Result<(Task, LiveSession)> {
        let native = task.native_session_id.clone().context("this session has not started yet")?;
        let forked = Task {
            id: TaskId::new(),
            title: format!("{} (fork)", task.title),
            status: TaskStatus::Running,
            native_session_id: None,
            pinned: false,
            updated_at: dove_store::now_ms(),
            unread: false,
            automation: None,
            ..task.clone()
        };
        self.store.insert_task(&forked)?;
        self.save_add_dirs(forked.id, &run.add_dirs);
        let spec = SessionSpec {
            kind: task.harness,
            program: self.program(task.harness)?,
            cwd: task.worktree.clone().unwrap_or_else(|| project.path.clone()),
            model: run.model.clone().or_else(|| task.model.clone()),
            reasoning: run.reasoning,
            model_options: run.options(),
            permission_mode: run.mode,
            resume: Some(native),
            prompt: None,
            fork: true,
            session: dove_harness::new_session_key(),
        };
        let live = self.launch(&forked, spec)?;
        Ok((forked, live))
    }

    /// Delete a task for good: its record, transcript, usage, and (for
    /// worktree tasks) the worktree and its branch.
    pub fn delete_task(&self, task: &Task, project: &Project) -> Result<()> {
        if let Some(wt) = &task.worktree
            && let Err(e) = dove_git::remove_worktree(&project.path, wt, task.branch.as_deref())
        {
            tracing::warn!("removing worktree {}: {e:#}", wt.display());
        }
        self.store.delete_task(task.id)
    }

    /// The binary Dove would launch for `kind` (with any override).
    pub fn program_for(&self, kind: HarnessKind) -> Option<PathBuf> {
        self.program(kind).ok()
    }

    /// An AI title for a thread whose first prompt is `prompt` (Zeron's
    /// restricted title run). The thread's own agent titles when it can,
    /// else the first installed agent that can (Claude Code, Codex), on its
    /// cheapest model. The receiver gets `None` when no agent can title or
    /// every attempt failed; the caller keeps its own title then.
    pub fn generate_title(&self, kind: HarnessKind, prompt: String) -> tokio::sync::oneshot::Receiver<Option<String>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let installed: Vec<HarnessKind> = self.installed.read().iter().map(|i| i.kind).collect();
        let pick = std::iter::once(kind)
            .chain(installed)
            .filter(|k| dove_harness::title::supports(*k))
            .find_map(|k| self.program(k).ok().map(|p| (k, p)));
        let Some((kind, program)) = pick else {
            let _ = tx.send(None);
            return rx;
        };
        let model = dove_harness::title::cheapest_model(&self.models(kind));
        self.rt.spawn(async move {
            let title = dove_harness::title::generate(kind, &program, &prompt, model).await;
            let _ = tx.send(title);
        });
        rx
    }

    /// A short answer from a restricted, tool-less run on `kind` (or the
    /// first installed agent that has one), on its cheapest model. Voice
    /// Mode's command router uses it; `None` when no agent could answer.
    pub fn complete_restricted(
        &self,
        kind: Option<HarnessKind>,
        prompt: String,
    ) -> tokio::sync::oneshot::Receiver<Option<String>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let installed: Vec<HarnessKind> = self.installed.read().iter().map(|i| i.kind).collect();
        let pick = kind
            .into_iter()
            .chain(installed)
            .filter(|k| dove_harness::title::supports(*k))
            .find_map(|k| self.program(k).ok().map(|p| (k, p)));
        let Some((kind, program)) = pick else {
            let _ = tx.send(None);
            return rx;
        };
        let model = dove_harness::title::cheapest_model(&self.models(kind));
        self.rt.spawn(async move {
            let answer =
                dove_harness::title::complete(kind, &program, &prompt, model, std::time::Duration::from_secs(25))
                    .await
                    .map_err(|e| tracing::info!("{} router run failed: {e:#}", kind.label()))
                    .ok();
            let _ = tx.send(answer);
        });
        rx
    }

    /// A plain-language walkthrough of a pull request, on the same agent
    /// pick as [`Self::generate_title`]. With `cache_key` (see
    /// [`walkthrough_cache_key`]) a cached AI walkthrough is sent at once and
    /// a fresh one is cached. The receiver always gets a walkthrough: the
    /// stats-only fallback when no agent can write one, every attempt
    /// failed, or the `pr_walkthrough_ai` setting is off.
    pub fn generate_walkthrough(
        &self,
        kind: HarnessKind,
        input: dove_harness::walkthrough::WalkthroughInput,
        cache_key: Option<String>,
    ) -> tokio::sync::oneshot::Receiver<dove_harness::walkthrough::Walkthrough> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if let Some(cached) = cache_key.as_deref().and_then(|k| self.cached_walkthrough(k)) {
            let _ = tx.send(cached);
            return rx;
        }
        // The thread's own agent first, then every other installed agent
        // that can make a restricted run: one that is signed out or fails
        // hands over to the next before the stats-only fallback.
        let installed: Vec<HarnessKind> = self.installed.read().iter().map(|i| i.kind).collect();
        let mut picks: Vec<(HarnessKind, PathBuf, Option<String>)> = Vec::new();
        if self.store.flag(PR_WALKTHROUGH_AI, true) {
            for k in std::iter::once(kind).chain(installed) {
                if picks.iter().any(|(p, ..)| *p == k) || !dove_harness::title::supports(k) {
                    continue;
                }
                if let Ok(program) = self.program(k) {
                    let model = dove_harness::title::cheapest_model(&self.models(k));
                    picks.push((k, program, model));
                }
            }
        }
        if picks.is_empty() {
            let _ = tx.send(dove_harness::walkthrough::fallback(&input));
            return rx;
        }
        let store = self.store.clone();
        self.rt.spawn(async move {
            let fallback = dove_harness::walkthrough::fallback(&input);
            let mut walkthrough = None;
            for (kind, program, model) in picks {
                if let Some(w) = dove_harness::walkthrough::generate(kind, &program, model, input.clone()).await {
                    walkthrough = Some(w);
                    break;
                }
            }
            let walkthrough = match walkthrough {
                Some(w) => {
                    if let Some(key) = &cache_key {
                        store_walkthrough(&store, key, &w);
                    }
                    w
                }
                None => fallback,
            };
            let _ = tx.send(walkthrough);
        });
        rx
    }

    /// The walkthrough cached under `key`, if any.
    pub fn cached_walkthrough(&self, key: &str) -> Option<dove_harness::walkthrough::Walkthrough> {
        serde_json::from_str(&self.store.setting(key)?).ok()
    }

    /// Cache `walkthrough` under `key`.
    pub fn store_walkthrough(&self, key: &str, walkthrough: &dove_harness::walkthrough::Walkthrough) {
        store_walkthrough(&self.store, key, walkthrough);
    }

    /// The driver running `task`, if any.
    pub fn driver(&self, task: TaskId) -> Option<Driver> {
        self.drivers.lock().get(&task).copied()
    }

    /// The agent's TUI command for `task`, resuming its session.
    pub fn tui_command(&self, task: &Task, project: &Project, run: &RunSettings) -> Result<tui::TuiCommand> {
        let cwd = task.worktree.clone().unwrap_or_else(|| project.path.clone());
        if task.harness == HarnessKind::Shell {
            return Ok(tui::shell_command(&cwd));
        }
        let program = self.program(task.harness)?;
        let Some(program) = tui::interactive_program(task.harness, &program) else {
            anyhow::bail!("{} has no terminal UI for this session", task.harness.label());
        };
        // The terminal's agent gets its own key, so its tool calls know
        // their thread like a chat session's do.
        let session = dove_harness::new_session_key();
        self.register_session(&session, task.id);
        let spec = tui::TuiSpec {
            native_session_id: task.native_session_id.as_deref(),
            model: run.model.as_deref().or(task.model.as_deref()),
            mode: run.mode,
            add_dirs: &run.add_dirs,
            session: &session,
        };
        Ok(tui::command(task.harness, &program, &cwd, &spec))
    }

    /// An agent's TUI for Flock: a fresh session in `cwd`, with Dove's tools
    /// (browser, computer use, memory, integrations) where the agent can
    /// load them at launch, and the composer's permission mode. It belongs
    /// to no thread, so a tool that needs one uses the thread in view.
    pub fn flock_command(&self, kind: HarnessKind, cwd: &std::path::Path) -> Result<tui::TuiCommand> {
        if kind == HarnessKind::Shell {
            return Ok(tui::shell_command(cwd));
        }
        let program = self.program(kind)?;
        let Some(program) = tui::interactive_program(kind, &program) else {
            anyhow::bail!("{} has no terminal UI", kind.label());
        };
        let session = dove_harness::new_session_key();
        let mode = match self.store.setting("permission_mode").as_deref() {
            Some("accept_edits") => PermissionMode::AcceptEdits,
            Some("plan") => PermissionMode::Plan,
            Some("bypass") => PermissionMode::Bypass,
            _ => PermissionMode::Default,
        };
        let spec = tui::TuiSpec { native_session_id: None, model: None, mode, add_dirs: &[], session: &session };
        Ok(tui::command(kind, &program, cwd, &spec))
    }

    /// Hand `task` to the terminal. Fails while the chat driver still runs
    /// it; stop that first (it is idle between turns, so nothing is lost).
    pub fn claim_terminal(&self, task: TaskId) -> Result<()> {
        let mut d = self.drivers.lock();
        if d.get(&task) == Some(&Driver::Chat) {
            anyhow::bail!("the chat session is still running");
        }
        d.insert(task, Driver::Terminal);
        Ok(())
    }

    /// The terminal let go of `task`.
    pub fn release_terminal(&self, task: TaskId) {
        let mut d = self.drivers.lock();
        if d.get(&task) == Some(&Driver::Terminal) {
            d.remove(&task);
        }
    }

    /// Mirror the TUI's session file into the chat (and the transcript)
    /// from its current end. `None` when the agent keeps no file Dove reads.
    pub fn mirror(&self, task: &Task, project: &Project) -> Option<mirror::MirrorSession> {
        let native = task.native_session_id.as_deref()?;
        let cwd = task.worktree.clone().unwrap_or_else(|| project.path.clone());
        let kind = task.harness;
        if !matches!(kind, HarnessKind::ClaudeCode | HarnessKind::Codex) {
            return None;
        }
        let path = tui::transcript_path(kind, &cwd, native).unwrap_or_default();
        let from = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let since = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let native = native.to_string();
        let resolve = move || tui::transcript_path(kind, &cwd, &native);
        let mut recorder = Recorder {
            store: self.store.clone(),
            task: task.id,
            project: task.project_id,
            harness: task.harness,
            model: task.model.clone(),
            status: TaskStatus::Idle,
            shared: self.shared.clone(),
            drivers: self.drivers.clone(),
            memory: self.memory_session(task),
        };
        let mut writer = self.store.transcript_writer(task.id).ok();
        let parser = mirror::LineParser::new(kind).since(since);
        Some(mirror::spawn_tail_resolving(self.rt.handle(), path, from, Some(resolve), parser, move |ev| {
            recorder.record(ev, writer.as_mut());
            if let Some(w) = writer.as_mut() {
                let _ = w.flush();
            }
        }))
    }

    fn launch(&self, task: &Task, spec: SessionSpec) -> Result<LiveSession> {
        {
            let mut d = self.drivers.lock();
            if d.get(&task.id) == Some(&Driver::Terminal) {
                anyhow::bail!("this thread is open in the terminal; switch back to chat first");
            }
            d.insert(task.id, Driver::Chat);
        }
        self.register_session(&spec.session, task.id);
        let handle = match dove_harness::spawn(self.rt.handle(), spec) {
            Ok(h) => h,
            Err(e) => {
                self.drivers.lock().remove(&task.id);
                return Err(e);
            }
        };
        Ok(self.record(task, handle))
    }

    /// Start (or adopt a pre-started session for) `task` without its first
    /// prompt, then send that prompt through the session like any other, so
    /// memory recall runs on it in the recorder instead of blocking here.
    /// Every driver waits for its first `Send` when it has no prompt.
    fn launch_deferred(&self, task: &Task, mut spec: SessionSpec) -> Result<LiveSession> {
        let prompt = spec.prompt.take();
        let live = match self.take_warm(&spec) {
            Some((handle, session)) => self.attach(task, handle, session)?,
            None => self.launch(task, spec)?,
        };
        if let Some(p) = prompt {
            live.send(Command::Send(p));
        }
        Ok(live)
    }

    /// Adopt an already running (pre-started) session for `task`; its
    /// session key now names `task`.
    fn attach(&self, task: &Task, handle: SessionHandle, session: String) -> Result<LiveSession> {
        {
            let mut d = self.drivers.lock();
            if d.get(&task.id) == Some(&Driver::Terminal) {
                anyhow::bail!("this thread is open in the terminal; switch back to chat first");
            }
            d.insert(task.id, Driver::Chat);
        }
        self.register_session(&session, task.id);
        Ok(self.record(task, handle))
    }

    fn record(&self, task: &Task, handle: SessionHandle) -> LiveSession {
        let capabilities = handle.capabilities;
        let (tx, rx) = flume::unbounded();
        let (cmd_tx, cmd_rx) = flume::unbounded::<Command>();
        let (stop_tx, stop_rx) = flume::bounded::<()>(1);
        self.stops.lock().insert(task.id, stop_tx.clone());
        let recorder = Recorder {
            store: self.store.clone(),
            task: task.id,
            project: task.project_id,
            harness: task.harness,
            model: task.model.clone(),
            status: TaskStatus::Running,
            shared: self.shared.clone(),
            drivers: self.drivers.clone(),
            memory: self.memory_session(task),
        };
        let stops = self.stops.clone();
        let id = task.id;
        self.rt.spawn(async move {
            recorder.run(handle, tx, cmd_rx, stop_rx).await;
            // A newer session for the same task keeps its own handle.
            let mut s = stops.lock();
            if s.get(&id).is_some_and(|t| t.same_channel(&stop_tx)) {
                s.remove(&id);
            }
        });
        LiveSession { task: task.id, events: rx, commands: cmd_tx, capabilities }
    }

    fn memory_session(&self, task: &Task) -> memory::Shared {
        let cwd = task.worktree.clone().unwrap_or_else(|| {
            self.store
                .projects()
                .ok()
                .and_then(|ps| ps.into_iter().find(|p| p.id == task.project_id).map(|p| p.path))
                .unwrap_or_default()
        });
        memory::Session::new(self.store.clone(), task.id, task.project_id, task.harness, cwd)
    }
}

struct Recorder {
    store: Arc<Store>,
    task: TaskId,
    project: dove_proto::ProjectId,
    harness: HarnessKind,
    model: Option<String>,
    status: TaskStatus,
    shared: Arc<RwLock<Shared>>,
    drivers: Arc<parking_lot::Mutex<HashMap<TaskId, Driver>>>,
    /// Memory recall/capture for this session.
    memory: memory::Shared,
}

impl Recorder {
    async fn run(
        mut self,
        handle: SessionHandle,
        out: flume::Sender<AgentEvent>,
        commands: flume::Receiver<Command>,
        stop: flume::Receiver<()>,
    ) {
        let mut writer = match self.store.transcript_writer(self.task) {
            Ok(w) => Some(w),
            Err(e) => {
                tracing::warn!("transcript for {} not writable: {e:#}", self.task);
                None
            }
        };
        // UI commands pass through memory on their way to the agent: recall
        // is prepended to sends, denied permissions are learned from. In
        // order, so a send never overtakes an earlier one.
        let (fwd_tx, fwd_rx) = flume::unbounded::<Command>();
        let mem = self.memory.clone();
        tokio::spawn(async move {
            while let Ok(cmd) = commands.recv_async().await {
                let cmd = match cmd {
                    Command::Send(text) => Command::Send(memory::before_send(&mem, text).await),
                    Command::Permission { request_id, decision } => {
                        mem.lock().decided(&request_id, decision);
                        Command::Permission { request_id, decision }
                    }
                    other => other,
                };
                if fwd_tx.send(cmd).is_err() {
                    break;
                }
            }
        });
        let mut ui_open = true;
        let mut stop_open = true;
        loop {
            tokio::select! {
                // Stop from outside the view (the terminal is taking over).
                asked = stop.recv_async(), if stop_open => match asked {
                    Ok(()) => { handle.send(Command::Kill); }
                    Err(_) => stop_open = false,
                },
                ev = handle.events.recv_async() => {
                    let Ok(ev) = ev else { break };
                    self.record(&ev, writer.as_mut());
                    let exited = matches!(ev, AgentEvent::Exited { .. });
                    // Free the chat driver before the UI sees Exited, so the
                    // terminal can claim this same session on the next tick.
                    if exited {
                        self.release_chat_driver();
                    }
                    if ui_open && out.send(ev).is_err() {
                        // Nobody is watching; keep recording until the agent stops.
                        ui_open = false;
                    }
                    if exited { break; }
                }
                cmd = fwd_rx.recv_async(), if ui_open => match cmd {
                    Ok(c) => { handle.send(c); }
                    // The UI dropped its session: let the agent finish the turn
                    // in the background rather than killing it mid-edit.
                    Err(_) => ui_open = false,
                },
            }
        }
        if let Some(w) = writer.as_mut() {
            let _ = w.flush();
        }
        self.release_chat_driver();
    }

    fn release_chat_driver(&self) {
        let mut d = self.drivers.lock();
        if d.get(&self.task) == Some(&Driver::Chat) {
            d.remove(&self.task);
        }
    }

    fn record(&mut self, ev: &AgentEvent, writer: Option<&mut dove_store::TranscriptWriter>) {
        self.memory.lock().observe(ev);
        if let Some(w) = writer {
            if let Err(e) = w.append(ev) {
                tracing::warn!("transcript append failed: {e:#}");
            }
            if matches!(ev, AgentEvent::TurnEnded { .. } | AgentEvent::PermissionRequested(_)) {
                let _ = w.flush();
            }
        }
        match ev {
            AgentEvent::SessionStarted { native_session_id: Some(id), model } => {
                let _ = self.store.set_native_session(self.task, id);
                if self.model.is_none() {
                    self.model = model.clone();
                }
            }
            AgentEvent::RateLimits { windows } => {
                // Providers report one window at a time; merge by kind.
                let mut shared = self.shared.write();
                let list = shared.rate_limits.entry(self.harness).or_default();
                for w in windows {
                    match list.iter_mut().find(|x| x.kind == w.kind) {
                        Some(x) => *x = w.clone(),
                        None => list.push(w.clone()),
                    }
                }
                if let Ok(json) = serde_json::to_string(list) {
                    let _ = self.store.set_setting(&format!("rate_limits.{}", self.harness.id()), &json);
                }
            }
            AgentEvent::Commands { commands } => {
                self.shared.write().commands.insert(self.harness, commands.clone());
            }
            AgentEvent::TurnEnded { usage: Some(u), .. } if u.total_tokens() > 0 || u.cost_usd.is_some() => {
                if let Err(e) = self.store.record_usage(self.task, self.project, self.harness, self.model.as_deref(), u)
                {
                    tracing::warn!("usage not recorded: {e:#}");
                }
            }
            _ => {}
        }
        // Exiting mid-turn is a crash; exiting between turns (app closed,
        // session idled out) leaves the task as it was.
        let next = match ev {
            AgentEvent::Exited { .. } if self.status == TaskStatus::Running => Some(TaskStatus::Failed),
            _ => next_status(ev),
        };
        if let Some(s) = next.filter(|s| *s != self.status) {
            self.status = s;
            let _ = self.store.set_status(self.task, s);
            if matches!(s, TaskStatus::Done | TaskStatus::Failed | TaskStatus::NeedsInput) {
                let _ = self.store.set_unread(self.task, true);
            }
        }
    }
}

// ------------------------------------------------------------ live models

/// Each agent's last fetched model list, plus when it was last asked.
struct LiveModels {
    /// Shared, so a picker reading its catalog every frame copies nothing.
    lists: RwLock<HashMap<HarnessKind, Arc<Vec<dove_proto::Model>>>>,
    /// When a fetch last started, for throttling.
    asked: parking_lot::Mutex<HashMap<HarnessKind, std::time::Instant>>,
    /// Bumped whenever a list changes.
    version: std::sync::atomic::AtomicU64,
    /// Wakes pickers when `version` moves, so nothing polls.
    changed: tokio::sync::watch::Sender<u64>,
    /// Fetches start agent CLIs: at most two at once.
    slots: tokio::sync::Semaphore,
    /// The background sweep has been scheduled this run.
    sweep: std::sync::atomic::AtomicBool,
}

impl Default for LiveModels {
    fn default() -> Self {
        Self {
            lists: Default::default(),
            asked: Default::default(),
            version: Default::default(),
            changed: tokio::sync::watch::channel(0).0,
            slots: tokio::sync::Semaphore::new(2),
            sweep: Default::default(),
        }
    }
}

impl LiveModels {
    fn bump(&self) {
        let v = self.version.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        self.changed.send_replace(v);
    }
}

impl Engine {
    /// `kind`'s models: the last live list (this run's, else the one saved
    /// last time), else the curated catalog. Returns at once; callers ask
    /// for a background refresh with [`Self::refresh_models_if_stale`],
    /// which bumps [`Self::models_version`] when it lands.
    pub fn models(&self, kind: HarnessKind) -> Vec<dove_proto::Model> {
        self.models_shared(kind).as_ref().clone()
    }

    /// [`Self::models`] without the copy: the live list itself.
    pub fn models_shared(&self, kind: HarnessKind) -> Arc<Vec<dove_proto::Model>> {
        let cached = self.live_models.lists.read().get(&kind).cloned();
        match cached {
            Some(list) => list,
            None => {
                let saved: Option<Vec<dove_proto::Model>> = self
                    .store
                    .setting(&models_key(kind))
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .filter(|l: &Vec<dove_proto::Model>| !l.is_empty());
                match saved {
                    Some(list) => {
                        let list = Arc::new(list);
                        self.live_models.lists.write().insert(kind, list.clone());
                        list
                    }
                    None => Arc::new(models::catalog(kind)),
                }
            }
        }
    }

    /// Changes whenever any live list does.
    pub fn models_version(&self) -> u64 {
        self.live_models.version.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Wakes whenever any live list changes, so a picker redraws without
    /// polling. Executor-agnostic (tokio's `watch` needs no runtime).
    pub fn models_watch(&self) -> tokio::sync::watch::Receiver<u64> {
        self.live_models.changed.subscribe()
    }

    /// Refresh `kind`'s list if its last successful fetch (this run or an
    /// earlier one) is older than the freshness window. A picker calls this
    /// for the agent it is showing.
    pub fn refresh_models_if_stale(&self, kind: HarnessKind) {
        if self.models_age(kind).is_none_or(|age| age > MODELS_FRESH_FOR) {
            self.refresh_models(kind);
        }
    }

    /// How long ago `kind`'s list was last fetched successfully.
    fn models_age(&self, kind: HarnessKind) -> Option<std::time::Duration> {
        let at: i64 = self.store.setting(&models_fetched_key(kind))?.parse().ok()?;
        let age = (dove_store::now_ms() - at).max(0) as u64;
        Some(std::time::Duration::from_millis(age))
    }

    /// Once per run, a couple of minutes after launch: refresh, one agent at
    /// a time, every installed agent whose list is a day old or was never
    /// fetched.
    fn schedule_model_sweep(&self) {
        if self.live_models.sweep.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let installed: Vec<(HarnessKind, std::path::PathBuf)> =
            self.installed.read().iter().map(|i| (i.kind, i.program.clone())).collect();
        let due: Vec<(HarnessKind, std::path::PathBuf)> = installed
            .into_iter()
            .filter(|(kind, _)| self.models_age(*kind).is_none_or(|age| age > MODELS_SWEEP_AFTER))
            .collect();
        if due.is_empty() {
            return;
        }
        let live = self.live_models.clone();
        let store = self.store.clone();
        self.rt.spawn(async move {
            tokio::time::sleep(MODELS_SWEEP_DELAY).await;
            for (kind, program) in due {
                {
                    let mut asked = live.asked.lock();
                    if asked.get(&kind).is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(60)) {
                        continue;
                    }
                    asked.insert(kind, std::time::Instant::now());
                }
                fetch_models(&live, &store, kind, &program).await;
            }
        });
    }

    /// Fetch `kind`'s live models now, unless one started in the last minute.
    pub fn refresh_models(&self, kind: HarnessKind) {
        let Some(program) = self.installed.read().iter().find(|i| i.kind == kind).map(|i| i.program.clone()) else {
            return;
        };
        {
            let mut asked = self.live_models.asked.lock();
            if asked.get(&kind).is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(60)) {
                return;
            }
            asked.insert(kind, std::time::Instant::now());
        }
        let live = self.live_models.clone();
        let store = self.store.clone();
        self.rt.spawn(async move {
            let Ok(_slot) = live.slots.acquire().await else { return };
            fetch_models(&live, &store, kind, &program).await;
        });
    }
}

/// Fetch one agent's live list, save it with the time it landed, and wake
/// pickers if it changed.
async fn fetch_models(live: &LiveModels, store: &dove_store::Store, kind: HarnessKind, program: &std::path::Path) {
    let fetched =
        tokio::time::timeout(std::time::Duration::from_secs(90), dove_harness::catalog::fetch(kind, program)).await;
    match fetched {
        Ok(Ok(list)) if !list.is_empty() => {
            let _ = store.set_setting(&models_fetched_key(kind), &dove_store::now_ms().to_string());
            if live.lists.read().get(&kind).map(|l| l.as_ref()) != Some(&list) {
                if let Ok(json) = serde_json::to_string(&list) {
                    let _ = store.set_setting(&models_key(kind), &json);
                }
                live.lists.write().insert(kind, Arc::new(list));
                live.bump();
            }
        }
        failed => {
            match failed {
                Ok(Ok(_)) => tracing::debug!("{} advertised no models", kind.label()),
                Ok(Err(e)) => tracing::info!("{} models not refreshed: {e:#}", kind.label()),
                Err(_) => tracing::info!("{} models not refreshed: timed out", kind.label()),
            }
            // `refresh_models` throttles to one start a minute; let a failed
            // fetch be retried once `MODELS_RETRY_AFTER` has passed.
            let throttle = std::time::Duration::from_secs(60);
            if let Some(at) = std::time::Instant::now().checked_sub(throttle.saturating_sub(MODELS_RETRY_AFTER)) {
                live.asked.lock().insert(kind, at);
            }
        }
    }
}

/// Setting: whether PR walkthroughs ask an agent (`"false"` keeps them to
/// the stats-only fallback). On by default.
pub const PR_WALKTHROUGH_AI: &str = "pr_walkthrough_ai";

/// The settings key a walkthrough of `repo` (`owner/name`) PR `number` at
/// `head_sha` is cached under; a new push changes the head and the key.
pub fn walkthrough_cache_key(repo: &str, number: u64, head_sha: &str) -> String {
    format!("walkthrough:{repo}#{number}@{head_sha}")
}

fn store_walkthrough(store: &Store, key: &str, walkthrough: &dove_harness::walkthrough::Walkthrough) {
    match serde_json::to_string(walkthrough) {
        Ok(json) => {
            if let Err(e) = store.set_setting(key, &json) {
                tracing::warn!("caching walkthrough {key}: {e:#}");
            }
        }
        Err(e) => tracing::warn!("serialising walkthrough {key}: {e}"),
    }
}

/// How long a fetched list counts as current when a picker shows it.
const MODELS_FRESH_FOR: std::time::Duration = std::time::Duration::from_secs(30 * 60);
/// How soon a failed fetch may be retried.
const MODELS_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(30);
/// The background sweep refreshes lists older than a day...
const MODELS_SWEEP_AFTER: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
/// ...starting this long after launch, so launch itself spawns nothing.
const MODELS_SWEEP_DELAY: std::time::Duration = std::time::Duration::from_secs(120);

fn models_key(kind: HarnessKind) -> String {
    format!("models.v2.{}", kind.id())
}

fn models_fetched_key(kind: HarnessKind) -> String {
    format!("models.fetched.{}", kind.id())
}

#[cfg(test)]
mod persona_tests {
    use super::*;

    #[test]
    fn preamble_round_trips_through_strip() {
        let p = dove_store::Persona {
            id: "ava".into(),
            name: "Ava".into(),
            role: "Reviewer".into(),
            avatar: "A".into(),
            hue: 0.5,
            harness: None,
            model: None,
            instructions: "Be terse.".into(),
            created_at: 0,
        };
        let team = vec![
            p.clone(),
            dove_store::Persona { id: "kai".into(), name: "Kai".into(), role: "Builder".into(), ..p.clone() },
        ];
        let sent =
            format!("{}Fix the login bug", persona_preamble(&p, Some(std::path::Path::new("/n/NOTES.md")), &team));
        assert!(sent.contains("NOTES.md"));
        assert!(!persona_preamble(&p, None, &[]).contains("notes file"));
        assert!(sent.contains("- Kai (Builder)"), "the roster lists the others");
        assert!(!sent.contains("- Ava"), "but not the teammate itself");
        assert!(sent.contains("team_message"));
        assert!(!persona_preamble(&p, None, std::slice::from_ref(&p)).contains("Your teammates"), "alone: no roster");
        assert!(sent.contains("You are Ava, Reviewer"));
        assert!(sent.contains("Be terse."));
        assert_eq!(strip_persona(&sent), "Fix the login bug");
        assert_eq!(strip_persona("plain"), "plain");
    }
}
