//! The composer: everything between the user and an agent.
//!
//! ```text
//!  [▣ dove ▾] [+ folder]                                          ◔ 38%  ◔ 6%
//! ╭──────────────────────────────────────────────────────────────────────────╮
//! │ [queued: "also add tests" ×]                                              │
//! │ [📎 screenshot.png ×] [@src/main.rs]                                      │
//! │ Ask anything, / for commands, @ for context                               │
//! │ ＋  🎙▾  ⛨ Ask every time ▾                     ✳ Opus · Medium ▾    (↑)  │
//! ╰──────────────────────────────────────────────────────────────────────────╯
//! ```
//!
//! One entity serves the new-thread canvas (`home`) and each thread
//! (`for_task`, where the agent is fixed and the context row collapses to
//! read-only chips). The agent · model · effort picker is its own entity
//! (`picker.rs`).

pub mod mentions;
pub mod picker;

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use dove_core::harness_detect::Installed;
use dove_core::proto::{HarnessKind, PermissionMode, ProjectId, RateWindow};
use dove_core::store::{Persona, Project, Task};
use dove_core::{Engine, RunSettings};
use gpui::StyledImage as _;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, App, AppContext as _, ClipboardEntry, Context, Entity, EntityId, EventEmitter, ExternalPaths,
    FocusHandle, Focusable, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, anchored, deferred, div,
    img, point, px, relative,
};
use gpui_base::input::{Input, InputEvent, InputState, Textarea, TextareaState};

use crate::assets::{icon, icons};
use crate::design::{radius, ty};
use crate::kit;
use crate::kit::popover;
use crate::theme::Theme;
use crate::ui;
use crate::voice::{Dictation, DictationEvent};
pub use picker::{Picker, PickerEvent};

/// Zeron's composer plate: at most 768 px wide, 76 px tall at rest, and it
/// grows with the prompt up to about 260 px before the text scrolls.
pub const PLATE_MAX_W: f32 = 840.0;
pub const PLATE_MIN_H: f32 = 76.0;
pub const PLATE_MAX_H: f32 = 260.0;

/// The composer's width: a little wider than the conversation column, so
/// the bar frames the messages above it, up to a cap.
pub fn plate_width() -> f32 {
    (crate::design::conversation_width() + 40.0).min(PLATE_MAX_W)
}
/// One prompt line at the plate's 15 px text.
const PLATE_LINE: f32 = 24.0;
/// The plate around the prompt: padding, border and the control row.
const PLATE_CHROME: f32 = PLATE_MIN_H - PLATE_LINE;
/// Prompt lines the thread plate grows through before the text scrolls.
const PLATE_ROWS: (usize, usize) = (1, ((PLATE_MAX_H - PLATE_CHROME) / PLATE_LINE) as usize);

pub enum ComposerEvent {
    Submit(String),
    Stop,
    /// The plan-usage ring was clicked.
    OpenUsage,
    /// "Install local voice…" was chosen.
    OpenVoiceSettings,
    /// The new-thread project changed.
    ProjectChanged(ProjectId),
    /// A project was added or cloned; the app should reload its list.
    ProjectsChanged,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Menu {
    Persona,
    Device,
    Project,
    /// Projects, extra folders and the device, in one menu (bottom bar).
    Place,
    /// Permission level. Only the bottom new-thread bar draws this.
    Mode,
    Mic,
    Attach,
    Folders,
}

/// Which usage meter the bottom bar is showing a detail card for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Meter {
    Plan,
}

/// Settings key for the new-thread composer. `center` is the current bar;
/// `dock` is the bottom-center bar.
pub const HOME_INPUT: &str = "home_input";

pub fn home_input_is_dock(value: Option<&str>) -> bool {
    value == Some("dock")
}

#[derive(Clone, Default)]
struct MentionItem {
    label: String,
    detail: String,
    insert: String,
    /// For a `#` skill that is off or not installed yet: what picking it
    /// does first.
    skill: Option<SkillPick>,
    /// Picking it adds this card instead of inserting text.
    card: Option<mentions::Card>,
    /// The menu section it is listed under ("Files", "Threads", …).
    section: &'static str,
}

impl MentionItem {
    fn card(section: &'static str, card: mentions::Card) -> Self {
        Self { label: card.label(), detail: card.detail(), card: Some(card), section, ..Default::default() }
    }
}

/// What picking a skill in the `#` menu does besides inserting `#name`.
#[derive(Clone, Debug, PartialEq)]
enum SkillPick {
    /// Turn an installed skill on.
    Enable(String),
    /// Install a recommended skill from GitHub.
    Install { repo: String, path: String },
}

/// An open `@` (files), `/` (commands) or `#` (skills) completion.
struct Mention {
    trigger: char,
    /// Byte offset of the trigger character.
    start: usize,
    items: Vec<MentionItem>,
    highlight: usize,
}

const MAX_MENTIONS: usize = 8;

/// Prompt templates offered under `/` next to the agent's own commands.
const TEMPLATES: &[(&str, &str, &str)] = &[
    (
        "review",
        "Review the changes on this branch",
        "Review the changes on this branch against its base. List bugs, risky changes and missing tests, most severe first.",
    ),
    (
        "test",
        "Run the tests and fix failures",
        "Run the project's tests. Fix any failures, then run them again until they pass.",
    ),
    (
        "design",
        "Design an interactive web or app screen on the Canvas",
        "Design a clean interactive screen in chat: realistic content, spacing, and the states a person would use. ",
    ),
    (
        "mobile",
        "Mock a mobile app flow in a phone frame",
        "Design a mobile app screen with realistic data, working tabs, and the interactions a person would use: ",
    ),
    (
        "dashboard",
        "Build an interactive analytics or admin dashboard",
        "Build an analytics or admin dashboard in chat: headline metrics, charts, and the tables a person would scan. ",
    ),
    (
        "simplify",
        "Simplify and clean up recent changes",
        "Audit the recently modified code and simplify complex logic, remove dead code, and tighten types without changing behavior.",
    ),
    (
        "explain",
        "Explain how this code works",
        "Explain how this part of the codebase works, with the key files and the flow between them.",
    ),
    (
        "plan",
        "Plan before changing anything",
        "Before changing anything, write a short plan: the files you will touch and why. Wait for my go-ahead.",
    ),
    ("commit", "Commit with a good message", "Commit the current changes with a clear, conventional commit message."),
];

/// How the composer is laid out: a roomy card on the new-thread page, a
/// one-line pill under a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Hero,
    Pill,
}

pub struct Composer {
    engine: Arc<Engine>,
    /// `/orchestrator` for this thread (or the next new one).
    orchestrate: crate::orchestrate::Orchestrate,
    variant: Variant,
    /// The teammate a new thread runs as (new-thread composer only).
    pub persona: Option<Persona>,
    personas: Vec<Persona>,
    pub input: Entity<TextareaState>,
    pub picker: Entity<Picker>,
    pub voice: Entity<Dictation>,
    /// Where live dictation's text sits in the input (byte range), so each
    /// update replaces it in place and the rest of the draft is untouched.
    live_dictation: Option<std::ops::Range<usize>>,
    pub mode: PermissionMode,
    /// Set for a thread's composer.
    thread: Option<(Task, Project)>,
    pub project: Option<Project>,
    projects: Vec<Project>,
    pub add_dirs: Vec<PathBuf>,
    attachments: Vec<PathBuf>,
    /// Mentioned threads, folders, windows and tabs, shown as cards.
    cards: Vec<mentions::Card>,
    /// Threads the `@` menu offers: (id, title, agent), read when it opens.
    mention_threads: Vec<(String, String, String)>,
    /// The paired browser's tabs, fetched when the `@` menu opens.
    chrome_tabs: Vec<(String, String)>,
    running: bool,
    queue: Vec<String>,
    context: (Option<u64>, Option<u64>),
    limits: Vec<RateWindow>,
    menu: Option<Menu>,
    mention: Option<Mention>,
    files: Arc<Vec<String>>,
    files_root: Option<PathBuf>,
    clone_url: Option<Entity<InputState>>,
    error: Option<SharedString>,
    draft_gen: u64,
    dragging_files: bool,
    /// The new-thread page is using the bottom-center bar.
    home_dock: bool,
    /// A thread's composer draws the same bottom bar as the new-thread dock.
    dock_bar: bool,
    /// The sidebar is open, so the bar stays tucked until the pointer is over it.
    dock_tuck: bool,
    /// The pointer is over the dock, which holds a tucked bar open.
    dock_hot: bool,
    /// The usage card under the pointer, if any.
    meter: Option<Meter>,
    /// Bumped when the pointer enters a meter, so a leave-timer cannot
    /// close a card the pointer has since re-entered.
    meter_gen: u64,
    /// Set when the host embeds this composer as a cached view: the height
    /// the host lays it out at, and the host to repaint when it changes.
    cache_slot: Option<(Rc<Cell<f32>>, EntityId)>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<ComposerEvent> for Composer {}

impl Focusable for Composer {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl Composer {
    fn build(
        engine: Arc<Engine>,
        placeholder: &'static str,
        rows: (usize, usize),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let prompt_style = ui::input_style(Theme::of(cx));
        let input = cx.new(|cx| {
            let mut s =
                TextareaState::new(window, cx).auto_grow(rows.0, rows.1).submit_on_enter(true).placeholder(placeholder);
            s.set_editor_style(prompt_style);
            s
        });
        let picker = cx.new(|cx| {
            let mut p = Picker::new(window, cx);
            p.attach_engine(engine.clone(), cx);
            p
        });
        let voice = cx.new(|cx| Dictation::new(engine.clone(), cx));
        let mut subs = Vec::new();
        subs.push(cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { shift: false, .. } => {
                if this.mention.is_some() {
                    this.accept_mention(window, cx);
                } else {
                    this.submit(window, cx);
                }
            }
            InputEvent::Change => {
                this.update_mention(cx);
                this.save_draft(cx);
                this.prewarm(cx);
                cx.notify();
            }
            _ => {}
        }));
        subs.push(cx.subscribe_in(&picker, window, |this, _, ev: &PickerEvent, window, cx| match ev {
            PickerEvent::Closed => focus_input(&this.input, window, cx),
            PickerEvent::Changed => {
                if this.mode == PermissionMode::Bypass && this.thread.is_none() {
                    // Nothing to re-check: bypass is per project, not per agent.
                }
                cx.notify();
            }
        }));
        subs.push(cx.subscribe_in(&voice, window, |this, _, ev: &DictationEvent, window, cx| match ev {
            DictationEvent::Text(text) => {
                let text = text.clone();
                this.input.update(cx, |s, cx| {
                    let before = s.value();
                    let sep = if before.is_empty() || before.ends_with(char::is_whitespace) { "" } else { " " };
                    s.insert(format!("{sep}{text}"), window, cx);
                });
                focus_input(&this.input, window, cx);
            }
            DictationEvent::Live(text) => {
                this.place_live_dictation(text, false, window, cx);
            }
            DictationEvent::LiveDone(text) => {
                this.place_live_dictation(text, true, window, cx);
                focus_input(&this.input, window, cx);
            }
            DictationEvent::Changed => cx.notify(),
        }));
        let mode = engine.store.setting("permission_mode").map(|m| mode_from_id(&m)).unwrap_or_default();
        Self {
            orchestrate: Default::default(),
            engine,
            variant: Variant::Pill,
            persona: None,
            personas: Vec::new(),
            input,
            picker,
            voice,
            live_dictation: None,
            mode,
            thread: None,
            project: None,
            projects: Vec::new(),
            add_dirs: Vec::new(),
            attachments: Vec::new(),
            cards: Vec::new(),
            mention_threads: Vec::new(),
            chrome_tabs: Vec::new(),
            running: false,
            queue: Vec::new(),
            context: (None, None),
            limits: Vec::new(),
            menu: None,
            mention: None,
            files: Arc::new(Vec::new()),
            files_root: None,
            clone_url: None,
            error: None,
            draft_gen: 0,
            dragging_files: false,
            home_dock: false,
            dock_bar: false,
            dock_tuck: false,
            dock_hot: false,
            meter: None,
            meter_gen: 0,
            cache_slot: None,
            _subs: subs,
        }
    }

    /// Draw for a host that caches this view at `height` (see
    /// [`Self::render`]); `host` re-renders when the height must change.
    pub fn set_cache_slot(&mut self, height: Rc<Cell<f32>>, host: EntityId) {
        self.cache_slot = Some((height, host));
    }

    /// Match the new-thread bar to Settings › Appearance. No-op unless the
    /// choice changed, so typing does not rebuild the field.
    pub fn apply_home_chrome(&mut self, dock: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.variant != Variant::Hero || self.home_dock == dock {
            return;
        }
        self.home_dock = dock;
        let placeholder = if dock { "Describe a task or ask a question" } else { "Do anything…" };
        let rows = if dock { (1, 8) } else { (2, 12) };
        self.input.update(cx, |s, cx| {
            s.set_placeholder(placeholder, window, cx);
            s.set_auto_grow(rows.0, rows.1, cx);
        });
        self.picker.update(cx, |p, _| p.set_text_trigger(dock));
        cx.notify();
    }

    /// The new-thread composer.
    pub fn home(engine: Arc<Engine>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::build(engine, "Do anything…", (2, 12), window, cx);
        this.variant = Variant::Hero;
        this.restore_draft(window, cx);
        this
    }

    /// A thread's composer, fixed to its agent.
    pub fn for_task(
        engine: Arc<Engine>,
        task: &Task,
        project: &Project,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::build(engine.clone(), "Send follow-up", (1, 8), window, cx);
        this.dock_bar = true;
        this.picker.update(cx, |p, _| {
            p.lock_to(task.harness, task.model.clone());
            p.set_text_trigger(true);
        });
        this.add_dirs = engine.add_dirs(task.id);
        this.thread = Some((task.clone(), project.clone()));
        this.project = Some(project.clone());
        if let Some(m) = engine.store.setting(&format!("mode.{}", task.id)) {
            this.mode = mode_from_id(&m);
        }
        this.restore_draft(window, cx);
        this
    }

    /// Lay a new-thread composer out as the one-line pill (side chats live
    /// in a narrow panel, beside a conversation).
    pub fn as_pill(&mut self, cx: &mut Context<Self>) {
        self.variant = Variant::Pill;
        self.home_dock = false;
        self.dock_bar = false;
        self.dock_tuck = false;
        self.picker.update(cx, |p, _| p.set_text_trigger(false));
        self.input.update(cx, |s, cx| s.set_auto_grow(PLATE_ROWS.0, PLATE_ROWS.1, cx));
        cx.notify();
    }

    fn draft_key(&self) -> String {
        match &self.thread {
            Some((t, _)) => format!("draft.{}", t.id),
            None => "draft.home".into(),
        }
    }

    fn restore_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.orchestrate = crate::orchestrate::get(&self.engine.store, self.thread.as_ref().map(|(t, _)| t.id));
        let key = self.draft_key();
        if let Some(d) = self.engine.store.setting(&key).filter(|d| !d.is_empty()) {
            let (d, cards) = d.split_once("\n\u{2}\n").unwrap_or((d.as_str(), ""));
            self.cards = mentions::from_json(cards);
            let mut lines = d.splitn(2, "\n\u{1}\n");
            let text = lines.next().unwrap_or_default().to_string();
            if let Some(files) = lines.next() {
                self.attachments = files.lines().map(PathBuf::from).filter(|p| p.exists()).collect();
            }
            self.input.update(cx, |s, cx| s.set_value(text, window, cx));
        }
    }

    /// Persist the draft (text and attachments) a moment after typing stops.
    fn save_draft(&mut self, cx: &mut Context<Self>) {
        self.draft_gen += 1;
        let generation = self.draft_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            let _ = this.update(cx, |c, cx| {
                if c.draft_gen != generation {
                    return;
                }
                let text = c.input.read(cx).value().to_string();
                let files = c.attachments.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
                let mut value = if files.is_empty() { text } else { format!("{text}\n\u{1}\n{files}") };
                if !c.cards.is_empty() {
                    value = format!("{value}\n\u{2}\n{}", mentions::to_json(&c.cards));
                }
                let _ = c.engine.store.set_setting(&c.draft_key(), &value);
            });
        })
        .detach();
    }

    // ------------------------------------------------------------ state in

    pub fn set_installed(&mut self, installed: Vec<Installed>, cx: &mut Context<Self>) {
        self.picker.update(cx, |p, cx| p.set_installed(installed, cx));
    }

    /// The projects the new-thread composer can pick from.
    pub fn set_projects(&mut self, projects: Vec<Project>, selected: Option<ProjectId>, cx: &mut Context<Self>) {
        self.projects = projects;
        let pick = selected
            .and_then(|id| self.projects.iter().find(|p| p.id == id))
            .or_else(|| self.projects.first())
            .cloned();
        self.project = pick;
        cx.notify();
    }

    /// The team the new-thread composer offers.
    pub fn set_personas(&mut self, personas: Vec<Persona>, cx: &mut Context<Self>) {
        if let Some(cur) = &self.persona {
            self.persona = personas.iter().find(|p| p.id == cur.id).cloned();
        }
        self.personas = personas;
        cx.notify();
    }

    /// Run the next thread as `persona` (or as no one). A teammate with a
    /// preferred agent brings it; one without keeps the current pick.
    pub fn use_persona(&mut self, persona: Option<Persona>, cx: &mut Context<Self>) {
        if let Some(p) = &persona
            && let Some(h) = p.harness
        {
            self.use_harness(h, p.model.clone(), cx);
        }
        self.persona = persona;
        self.menu = None;
        cx.notify();
    }

    pub fn use_harness(&mut self, kind: HarnessKind, model: Option<String>, cx: &mut Context<Self>) {
        self.picker.update(cx, |p, cx| {
            p.use_harness(kind, model);
            cx.notify();
        });
    }

    pub fn set_running(&mut self, running: bool, cx: &mut Context<Self>) {
        if self.running != running {
            self.running = running;
            cx.notify();
        }
    }

    pub fn set_meters(&mut self, context: (Option<u64>, Option<u64>), limits: Vec<RateWindow>, cx: &mut Context<Self>) {
        if self.context != context || self.limits != limits {
            self.context = context;
            self.limits = limits;
            cx.notify();
        }
    }

    pub fn harness(&self, cx: &App) -> HarnessKind {
        self.picker.read(cx).harness
    }

    /// While a new thread is being typed, start its agent so the first
    /// reply skips the cold start. The session runs in the project folder.
    fn prewarm(&self, cx: &App) {
        if self.input.read(cx).value().trim().is_empty() || self.running {
            return;
        }
        if let Some((task, project)) = &self.thread {
            if dove_core::prewarms(task.harness) {
                self.engine.prewarm_resume(task, project, &self.run_settings(cx));
            }
            return;
        }
        let Some(project) = self.project.as_ref() else { return };
        let harness = self.harness(cx);
        if !dove_core::prewarms(harness) {
            return;
        }
        self.engine.prewarm(harness, &project.path, &self.run_settings(cx));
    }

    /// How the next turn runs.
    pub fn run_settings(&self, cx: &App) -> RunSettings {
        let p = self.picker.read(cx);
        RunSettings {
            mode: self.mode,
            model: p.model.clone(),
            reasoning: p.reasoning,
            model_options: p.model_options.clone(),
            add_dirs: if dove_core::supports_add_dirs(p.harness) { self.add_dirs.clone() } else { Vec::new() },
        }
    }

    /// Send `text` once the running turn ends (another agent's message to
    /// a thread whose agent can't be steered).
    pub fn enqueue(&mut self, text: String, cx: &mut Context<Self>) {
        self.queue.push(text);
        cx.notify();
    }

    /// The next queued message, sent once the running turn ends.
    pub fn take_queued(&mut self, cx: &mut Context<Self>) -> Option<String> {
        if self.queue.is_empty() {
            return None;
        }
        cx.notify();
        Some(self.queue.remove(0))
    }

    fn root(&self) -> Option<PathBuf> {
        match &self.thread {
            Some((t, p)) => Some(t.worktree.clone().unwrap_or_else(|| p.path.clone())),
            None => self.project.as_ref().map(|p| p.path.clone()),
        }
    }

    // ------------------------------------------------------------ actions

    pub(crate) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut text = self.input.read(cx).value().trim().to_string();
        // `/orchestrator [off|auto]` is Dove's own: it isn't sent.
        if let Some(o) = crate::orchestrate::command(&text) {
            self.set_orchestrate(o, cx);
            self.input.update(cx, |s, cx| s.set_value("", window, cx));
            self.mention = None;
            return;
        }
        if text.is_empty() && self.attachments.is_empty() && self.cards.is_empty() {
            return;
        }
        if let Some(block) = skills_block(&text) {
            text = format!("{text}\n\n{block}");
        }
        if !self.cards.is_empty() {
            let computer_use = crate::pages::integrations::load().computer.enabled;
            if let Some(block) = mentions::block(&self.cards, &self.engine.store, computer_use) {
                text = format!("{text}\n\n{block}").trim().to_string();
            }
            self.cards.clear();
        }
        if !self.attachments.is_empty() {
            let list = self.attachments.iter().map(|p| format!("- {}", p.display())).collect::<Vec<_>>().join("\n");
            text = format!("{text}\n\n{ATTACH_HEADER}\n{list}").trim().to_string();
        }
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        self.attachments.clear();
        self.mention = None;
        let _ = self.engine.store.set_setting(&self.draft_key(), "");
        // Mid-turn: steer agents that take it, queue for the rest.
        if self.running && !dove_core::steers(self.harness(cx)) {
            self.queue.push(text);
            cx.notify();
            return;
        }
        cx.emit(ComposerEvent::Submit(text));
    }

    pub(crate) fn set_mode(&mut self, mode: PermissionMode, cx: &mut Context<Self>) {
        self.mode = mode;
        self.menu = None;
        let key = match &self.thread {
            Some((task, _)) => format!("mode.{}", task.id),
            None => "permission_mode".into(),
        };
        let _ = self.engine.store.set_setting(&key, mode_id(mode));
        cx.notify();
    }

    fn toggle_menu(&mut self, m: Menu, cx: &mut Context<Self>) {
        self.menu = if self.menu == Some(m) { None } else { Some(m) };
        self.clone_url = None;
        cx.notify();
    }

    fn pick_project(&mut self, id: ProjectId, cx: &mut Context<Self>) {
        self.project = self.projects.iter().find(|p| p.id == id).cloned();
        self.menu = None;
        self.files = Arc::new(Vec::new());
        self.files_root = None;
        cx.emit(ComposerEvent::ProjectChanged(id));
        cx.notify();
    }

    fn add_folder_project(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        let pick = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add project".into()),
        });
        let store = self.engine.store.clone();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = pick.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let root = dove_core::git::repo_root(&path).unwrap_or(path);
            let added = store.add_project(&root);
            let _ = this.update(cx, |c, cx| match added {
                Ok(p) => {
                    let _ = c.engine.store.set_setting("active_project", &p.id.to_string());
                    c.projects.push(p.clone());
                    c.pick_project(p.id, cx);
                    cx.emit(ComposerEvent::ProjectsChanged);
                }
                Err(e) => {
                    c.error = Some(format!("{e:#}").into());
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn start_clone(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = Theme::of(cx).clone();
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("https://github.com/owner/repo.git");
            s.set_editor_style(ui::input_style(&theme));
            s
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        let sub = cx.subscribe_in(&input, window, |this, input, ev: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                let url = input.read(cx).value().trim().to_string();
                if !url.is_empty() {
                    this.clone_repo(url, cx);
                }
            }
        });
        self._subs.push(sub);
        self.clone_url = Some(input);
        cx.notify();
    }

    fn clone_repo(&mut self, url: String, cx: &mut Context<Self>) {
        self.clone_url = None;
        self.menu = None;
        self.error = Some(format!("Cloning {url}…").into());
        let parent = self
            .engine
            .store
            .setting("clone_dir")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE")
                    .or_else(|| std::env::var_os("HOME"))
                    .map(|h| PathBuf::from(h).join("Dove"))
            })
            .unwrap_or_else(|| PathBuf::from("."));
        let store = self.engine.store.clone();
        let work = cx.background_spawn(async move {
            let dest = dove_core::git::clone(&url, &parent)?;
            store.add_project(&dest)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |c, cx| match result {
                Ok(p) => {
                    c.error = None;
                    c.projects.push(p.clone());
                    c.pick_project(p.id, cx);
                    cx.emit(ComposerEvent::ProjectsChanged);
                }
                Err(e) => {
                    c.error = Some(format!("Clone failed: {e:#}").into());
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn add_dir(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        let pick = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add folder".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = pick.await else { return };
            let _ = this.update(cx, |c, cx| {
                for p in paths {
                    if !c.add_dirs.contains(&p) {
                        c.add_dirs.push(p);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn attach_files(&mut self, images: bool, cx: &mut Context<Self>) {
        self.menu = None;
        let pick = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(if images { "Attach images".into() } else { "Attach files".into() }),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = pick.await else { return };
            let _ = this.update(cx, |c, cx| c.attach(paths, cx));
        })
        .detach();
        cx.notify();
    }

    fn attach(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        for p in paths {
            if !self.attachments.contains(&p) {
                self.attachments.push(p);
            }
        }
        self.save_draft(cx);
        cx.notify();
    }

    /// Ctrl+V with an image on the clipboard saves it and attaches it.
    fn paste_image(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else { return false };
        let Some(image) = item.entries().iter().find_map(|e| match e {
            ClipboardEntry::Image(i) => Some(i.clone()),
            _ => None,
        }) else {
            return false;
        };
        let dir = self.engine.store.root().join("attachments");
        let ext = match image.format {
            gpui::ImageFormat::Jpeg => "jpg",
            gpui::ImageFormat::Gif => "gif",
            gpui::ImageFormat::Webp => "webp",
            gpui::ImageFormat::Bmp => "bmp",
            gpui::ImageFormat::Svg => "svg",
            _ => "png",
        };
        let path = dir.join(format!("pasted-{}.{ext}", dove_core::store::now_ms()));
        let saved = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &image.bytes));
        match saved {
            Ok(()) => self.attach(vec![path], cx),
            Err(e) => self.error = Some(format!("Could not save the pasted image: {e}").into()),
        }
        true
    }

    fn remove_attachment(&mut self, i: usize, cx: &mut Context<Self>) {
        if i < self.attachments.len() {
            self.attachments.remove(i);
            self.save_draft(cx);
            cx.notify();
        }
    }

    // ------------------------------------------------------------ mentions

    /// Open, update or close the `@` / `/` menu for the token at the cursor.
    fn update_mention(&mut self, cx: &mut Context<Self>) {
        let (text, cursor) = {
            let s = self.input.read(cx);
            (s.value().to_string(), s.cursor())
        };
        let cursor = cursor.min(text.len());
        let before = &text[..cursor];
        let start = before.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
        let token = &before[start..];
        let trigger = token.chars().next();
        let mention = match trigger {
            Some('@') => {
                if self.mention.as_ref().is_none_or(|m| m.trigger != '@') {
                    self.load_mention_sources(cx);
                }
                self.ensure_files(cx);
                let q = token[1..].to_ascii_lowercase();
                Some(Mention { trigger: '@', start, items: self.at_items(&q, cx), highlight: 0 })
            }
            // Slash commands only at the start of the message.
            Some('/') if start == 0 => {
                let q = token[1..].to_ascii_lowercase();
                let mut items: Vec<MentionItem> = self
                    .engine
                    .commands(self.harness(cx))
                    .into_iter()
                    .map(|c| MentionItem {
                        insert: format!("/{}", c.name),
                        label: format!("/{}", c.name),
                        detail: c.description,
                        ..Default::default()
                    })
                    .collect();
                items.push(MentionItem {
                    label: "/orchestrator".into(),
                    detail: match self.orchestrate {
                        crate::orchestrate::Orchestrate::Off => {
                            "Let this thread start other agents (you approve each one)".into()
                        }
                        _ => "On · /orchestrator off stops it, /orchestrator auto skips approvals".into(),
                    },
                    insert: "/orchestrator".into(),
                    section: "Dove",
                    ..Default::default()
                });
                items.extend(TEMPLATES.iter().map(|(name, detail, body)| MentionItem {
                    label: format!("/{name}"),
                    detail: detail.to_string(),
                    insert: body.to_string(),
                    ..Default::default()
                }));
                items.retain(|i| q.is_empty() || i.label[1..].to_ascii_lowercase().starts_with(&q));
                items.truncate(MAX_MENTIONS);
                Some(Mention { trigger: '/', start, items, highlight: 0 })
            }
            // Skills: `#` alone lists them; `#12` with no match stays text.
            Some('#') => {
                let q = token[1..].to_ascii_lowercase();
                let mut items = skill_items(&q);
                // Apps and tabs can be mentioned with # too.
                items.extend(self.app_and_tab_items(&q, 3, cx));
                (!items.is_empty() || q.is_empty()).then_some(Mention { trigger: '#', start, items, highlight: 0 })
            }
            _ => None,
        };
        self.mention = mention.filter(|m| !m.items.is_empty() || m.trigger == '@' || m.trigger == '#');
    }

    /// Read what the `@` menu lists besides files: threads now, the paired
    /// browser's tabs in the background.
    fn load_mention_sources(&mut self, cx: &mut Context<Self>) {
        let own = self.thread.as_ref().map(|(t, _)| t.id);
        self.mention_threads = self
            .engine
            .store
            .all_tasks()
            .unwrap_or_default()
            .into_iter()
            .filter(|t| Some(t.id) != own)
            .map(|t| (t.id.to_string(), t.title.clone(), t.harness.label().to_string()))
            .collect();
        if crate::browser::control::connected_browser().is_some() {
            let rt = self.engine.runtime();
            let tabs = cx.background_spawn(async move {
                rt.spawn(crate::browser::control::chrome_tabs()).await.unwrap_or_default()
            });
            cx.spawn(async move |this, cx| {
                let tabs = tabs.await;
                let _ = this.update(cx, |c, cx| {
                    c.chrome_tabs = tabs;
                    if c.mention.is_some() {
                        c.update_mention(cx);
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    /// The `@` menu: files and folders, threads, apps' windows and tabs.
    fn at_items(&self, q: &str, cx: &App) -> Vec<MentionItem> {
        let hit = |s: &str| q.is_empty() || s.to_ascii_lowercase().contains(q);
        let mut items = Vec::new();
        if let Some(root) = self.root() {
            items.extend(
                self.files
                    .iter()
                    .filter(|f| hit(f))
                    .take(5)
                    .map(|f| MentionItem::card("Files", mentions::Card::File { path: root.join(f) })),
            );
            let mut dirs: Vec<&str> = Vec::new();
            for f in self.files.iter() {
                if let Some((d, _)) = f.rsplit_once('/')
                    && hit(d)
                    && !dirs.contains(&d)
                {
                    dirs.push(d);
                    if dirs.len() == 3 {
                        break;
                    }
                }
            }
            items.extend(
                dirs.into_iter().map(|d| MentionItem::card("Folders", mentions::Card::Folder { path: root.join(d) })),
            );
        }
        items.extend(self.mention_threads.iter().filter(|(_, title, agent)| hit(title) || hit(agent)).take(4).map(
            |(id, title, agent)| {
                MentionItem::card(
                    "Threads",
                    mentions::Card::Thread { id: id.clone(), title: title.clone(), agent: agent.clone() },
                )
            },
        ));
        items.extend(self.app_and_tab_items(q, 4, cx));
        items
    }

    /// Apps' windows and browser tabs matching `q`, at most `each` of each.
    fn app_and_tab_items(&self, q: &str, each: usize, cx: &App) -> Vec<MentionItem> {
        let hit = |s: &str| q.is_empty() || s.to_ascii_lowercase().contains(q);
        let windows =
            crate::device::windows_cached().into_iter().filter(|w| hit(&w.app) || hit(&w.title)).take(each).map(|w| {
                MentionItem::card(
                    "Apps & windows",
                    mentions::Card::Window { app: w.app, title: w.title, pid: w.pid, id: w.id },
                )
            });
        let in_app = crate::side_panel::browser_tabs(cx).into_iter().map(|(title, url)| mentions::Card::Tab {
            chrome: false,
            title,
            url,
        });
        let chrome =
            self.chrome_tabs.iter().cloned().map(|(title, url)| mentions::Card::Tab { chrome: true, title, url });
        let tabs = in_app
            .chain(chrome)
            .filter(|c| matches!(c, mentions::Card::Tab { title, url, .. } if hit(title) || hit(url)))
            .take(each)
            .map(|c| MentionItem::card("Browser tabs", c));
        windows.chain(tabs).collect()
    }

    fn remove_card(&mut self, i: usize, cx: &mut Context<Self>) {
        if i < self.cards.len() {
            self.cards.remove(i);
            self.save_draft(cx);
            cx.notify();
        }
    }

    fn ensure_files(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root() else { return };
        if self.files_root.as_ref() == Some(&root) {
            return;
        }
        self.files_root = Some(root.clone());
        let load = cx.background_spawn(async move { dove_core::git::list_files(&root).unwrap_or_default() });
        cx.spawn(async move |this, cx| {
            let files = load.await;
            let _ = this.update(cx, |c, cx| {
                c.files = Arc::new(files);
                c.update_mention(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn accept_mention(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(m) = self.mention.take() else { return };
        let Some(item) = m.items.get(m.highlight).cloned() else { return };
        // Dove's own `/orchestrator` acts at once instead of inserting text.
        if m.trigger == '/'
            && let Some(o) = crate::orchestrate::command(&item.insert)
        {
            self.input.update(cx, |s, cx| s.set_value("", window, cx));
            let next = if self.orchestrate == crate::orchestrate::Orchestrate::Off { o } else { self.orchestrate };
            self.set_orchestrate(next, cx);
            return;
        }
        // A card replaces the typed `@query`.
        if let Some(card) = item.card {
            self.input.update(cx, |s, cx| {
                let cursor = s.cursor();
                s.set_selected_range(m.start..cursor, cx);
                s.replace("", window, cx);
            });
            if !self.cards.iter().any(|c| c.same(&card)) {
                self.cards.push(card);
            }
            self.save_draft(cx);
            // A click in the menu took focus: keep typing where you were.
            focus_input(&self.input, window, cx);
            cx.notify();
            return;
        }
        if let Some(pick) = item.skill.clone() {
            // Turn the skill on (or install it) before the message goes.
            cx.background_spawn(async move {
                let done = match pick {
                    SkillPick::Enable(name) => dove_core::skills::set_skill_enabled(&name, true),
                    SkillPick::Install { repo, path } => dove_core::skills::install(&repo, &path).map(|_| ()),
                };
                if let Err(e) = done {
                    tracing::warn!("could not prepare the skill: {e}");
                }
            })
            .detach();
        }
        self.input.update(cx, |s, cx| {
            let cursor = s.cursor();
            s.set_selected_range(m.start..cursor, cx);
            s.replace(format!("{} ", item.insert), window, cx);
        });
        focus_input(&self.input, window, cx);
        cx.notify();
    }

    /// Keys the composer handles before the textarea: menu navigation,
    /// image paste, Esc to close or interrupt.
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let key = k.key.as_str();
        if let Some(m) = self.mention.as_mut() {
            match key {
                "down" if !m.items.is_empty() => {
                    m.highlight = (m.highlight + 1) % m.items.len();
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                "up" if !m.items.is_empty() => {
                    m.highlight = (m.highlight + m.items.len() - 1) % m.items.len();
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                "tab" => {
                    self.accept_mention(window, cx);
                    cx.stop_propagation();
                    return;
                }
                "escape" => {
                    self.mention = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }
        if key == "backspace"
            && self.mention.is_none()
            && !self.cards.is_empty()
            && self.input.read(cx).value().is_empty()
        {
            self.cards.pop();
            self.save_draft(cx);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if key == "v" && (k.modifiers.control || k.modifiers.platform) && self.paste_image(cx) {
            cx.stop_propagation();
            return;
        }
        if key == "escape" {
            if self.voice.read(cx).is_recording() {
                self.voice.update(cx, |v, cx| v.cancel(cx));
                cx.stop_propagation();
            } else if self.menu.is_some() {
                self.menu = None;
                cx.stop_propagation();
                cx.notify();
            } else if self.running && self.input.read(cx).value().trim().is_empty() {
                cx.emit(ComposerEvent::Stop);
                cx.stop_propagation();
            }
        }
    }

    // ------------------------------------------------------------ render

    /// Above the new-thread bar, right-aligned: who runs it, in which
    /// project, on which device.
    fn render_target_row(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .w_full()
            .h(px(22.0))
            .px(px(HERO_INSET - 8.0))
            .flex()
            .items_center()
            .justify_end()
            .gap(px(2.0))
            .when(!self.personas.is_empty(), |d| d.child(self.persona_chip(t, cx)))
            .child(self.project_chip(t, cx))
            .child(self.device_chip(t, cx))
            .into_any_element()
    }

    /// Below the new-thread bar, once an extra folder has been added.
    fn render_git_row(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .w_full()
            .h(px(22.0))
            .px(px(HERO_INSET - 8.0))
            .flex()
            .items_center()
            .gap(px(2.0))
            .child(self.folders_chip(t, cx))
            .into_any_element()
    }

    fn persona_chip(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let open = self.menu == Some(Menu::Persona);
        let label = self.persona.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "Default".into());
        let chip = ui::footer_chip("composer-persona", icons::BOT, label, open, t)
            .tooltip(ui::tip("Who runs this thread"))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Persona, cx)));
        let menu = open.then(|| {
            let selected = self.persona.as_ref().map(|p| p.id.clone());
            let rows = div()
                .w(px(280.0))
                .flex()
                .flex_col()
                .child(
                    ui::menu_row("persona-none", icons::BOT, "Default", false, t)
                        .when(selected.is_none(), |d| d.child(icon(icons::CHECK).size(px(14.0)).text_color(t.accent)))
                        .on_click(cx.listener(|this, _, _, cx| this.use_persona(None, cx))),
                )
                .children(self.personas.iter().map(|p| {
                    let pick = p.clone();
                    let on = selected.as_deref() == Some(p.id.as_str());
                    div()
                        .id(SharedString::from(format!("persona-{}", p.id)))
                        .mx(px(4.0))
                        .h(px(40.0))
                        .px(px(10.0))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .rounded(px(kit::popover::MENU_ITEM_RADIUS))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.wash(0.10)))
                        .child(crate::team::avatar(p, 22.0, t))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(div().text_size(ty::BODY).text_color(t.text).truncate().child(p.name.clone()))
                                .child(
                                    div()
                                        .text_size(ty::CAPTION)
                                        .text_color(t.text_faint)
                                        .truncate()
                                        .child(crate::team::subtitle(p)),
                                ),
                        )
                        .when(on, |d| d.child(icon(icons::CHECK).size(px(14.0)).text_color(t.accent)))
                        .on_click(cx.listener(move |this, _, _, cx| this.use_persona(Some(pick.clone()), cx)))
                }));
            self.popover(Anchor::BottomLeft, ui::menu_card(t, rows), cx)
        });
        div().relative().child(chip).children(menu).into_any_element()
    }

    fn render_context_row(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let home = self.thread.is_none();
        let mut row = div()
            .w_full()
            .min_h(px(26.0))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(if home { 2.0 } else { 6.0 }))
            .when(home, |d| d.justify_end());
        match &self.thread {
            None => {
                row = row
                    .child(self.device_chip(t, cx))
                    .when(!self.add_dirs.is_empty(), |d| d.child(self.folders_chip(t, cx)))
                    .child(self.project_chip(t, cx));
            }
            Some(_) => {
                // Extra folders show here once added; adding lives in the
                // attach menu.
                row = row.when(!self.add_dirs.is_empty(), |d| d.child(self.folders_chip(t, cx)));
            }
        }
        row.into_any_element()
    }

    fn device_chip(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let chip =
            ui::footer_chip("composer-device", icons::MONITOR, ui::host_name(), self.menu == Some(Menu::Device), t)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Device, cx)));
        let menu = (self.menu == Some(Menu::Device)).then(|| {
            let rows = div()
                .w(px(250.0))
                .flex()
                .flex_col()
                .child(
                    ui::menu_row("dev-local", icons::LAPTOP, format!("Local · {}", ui::host_name()), false, t)
                        .child(icon(icons::CHECK).size(px(14.0)).text_color(t.accent))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.menu = None;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .h(px(34.0))
                        .px(px(10.0))
                        .mx(px(4.0))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .text_size(ty::MD)
                        .text_color(t.text_faint)
                        .child(icon(icons::CLOUD).size(px(16.0)))
                        .child(div().flex_1().child("Cloud"))
                        .child(kit::badge("Coming soon", t.text_faint)),
                );
            self.popover(Anchor::BottomLeft, ui::menu_card(t, rows), cx)
        });
        div().relative().child(chip).children(menu).into_any_element()
    }

    fn project_chip(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let name = self.project.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "Add a project".into());
        let chip = ui::footer_chip("composer-project", icons::FOLDER, name, self.menu == Some(Menu::Project), t)
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Project, cx)));
        let menu = (self.menu == Some(Menu::Project)).then(|| {
            let selected = self.project.as_ref().map(|p| p.id);
            let mut rows = div().w(px(300.0)).max_h(px(380.0)).flex().flex_col();
            rows =
                rows.child(div().id("project-list").max_h(px(260.0)).overflow_y_scroll().flex().flex_col().children(
                    self.projects.iter().map(|p| {
                        let id = p.id;
                        ui::menu_row(SharedString::from(format!("proj-{id}")), icons::FOLDER, p.name.clone(), false, t)
                            .when(selected == Some(id), |d| {
                                d.child(icon(icons::CHECK).size(px(14.0)).text_color(t.accent))
                            })
                            .tooltip(ui::tip(p.path.display().to_string()))
                            .on_click(cx.listener(move |this, _, _, cx| this.pick_project(id, cx)))
                    }),
                ));
            rows = rows
                .child(ui::separator(t))
                .child(
                    ui::menu_row("proj-add", icons::ADD_CIRCLE, "Add folder…", false, t)
                        .on_click(cx.listener(|this, _, _, cx| this.add_folder_project(cx))),
                )
                .child(match &self.clone_url {
                    Some(input) => div()
                        .mx(px(8.0))
                        .my(px(4.0))
                        .h(px(32.0))
                        .px(px(10.0))
                        .flex()
                        .items_center()
                        .rounded(px(radius::RADIUS))
                        .border_1()
                        .border_color(t.border_strong)
                        .text_size(ty::BODY)
                        .child(div().flex_1().child(Input::new(input)))
                        .into_any_element(),
                    None => ui::menu_row("proj-clone", icons::GLOBAL, "Clone from URL…", false, t)
                        .on_click(cx.listener(|this, _, window, cx| this.start_clone(window, cx)))
                        .into_any_element(),
                });
            self.popover(Anchor::BottomLeft, ui::menu_card(t, rows), cx)
        });
        div().relative().child(chip).children(menu).into_any_element()
    }

    fn folders_chip(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let supported = dove_core::supports_add_dirs(self.harness(cx));
        let n = self.add_dirs.len();
        let chip =
            if self.thread.is_none() { ui::ghost_chip("composer-folders", t) } else { ui::chip("composer-folders", t) }
                .child(icon(icons::FOLDER_WITH_FILES).size(px(14.0)))
                .child(if n == 0 {
                    "Add folder".to_string()
                } else {
                    format!("{n} extra {}", if n == 1 { "folder" } else { "folders" })
                })
                .when(!supported, |d| d.opacity(0.5))
                .tooltip(ui::tip(if supported {
                    "Let the agent read and edit another folder"
                } else {
                    "Extra folders work with Claude Code"
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !supported {
                        return;
                    }
                    if this.add_dirs.is_empty() {
                        this.add_dir(cx);
                    } else {
                        this.toggle_menu(Menu::Folders, cx);
                    }
                }));
        let menu = (self.menu == Some(Menu::Folders)).then(|| {
            let rows = div()
                .w(px(320.0))
                .flex()
                .flex_col()
                .children(self.add_dirs.iter().enumerate().map(|(i, d)| {
                    ui::menu_row(
                        SharedString::from(format!("dir-{i}")),
                        icons::FOLDER,
                        d.display().to_string(),
                        false,
                        t,
                    )
                    .child(icon(icons::CLOSE).size(px(13.0)).text_color(t.text_faint))
                    .tooltip(ui::tip("Remove"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if i < this.add_dirs.len() {
                            this.add_dirs.remove(i);
                        }
                        if this.add_dirs.is_empty() {
                            this.menu = None;
                        }
                        cx.notify();
                    }))
                }))
                .child(ui::separator(t))
                .child(
                    ui::menu_row("dir-add", icons::ADD_CIRCLE, "Add folder…", false, t)
                        .on_click(cx.listener(|this, _, _, cx| this.add_dir(cx))),
                );
            self.popover(Anchor::BottomLeft, ui::menu_card(t, rows), cx)
        });
        div().relative().child(chip).children(menu).into_any_element()
    }

    /// Context-window and plan-usage rings.
    fn rings(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (tokens, window) = self.context;
        let ctx_frac = match (tokens, window) {
            (Some(u), Some(w)) if w > 0 => Some(u as f32 / w as f32),
            _ => None,
        };
        let model = self.picker.read(cx).model.clone().unwrap_or_default();
        let ctx_tip = match (tokens, window) {
            (Some(u), Some(w)) => format!(
                "Context {} / {} tokens ({:.0}%)\n{model}",
                ui::compact(u),
                ui::compact(w),
                u as f64 * 100.0 / w.max(1) as f64
            ),
            (Some(u), None) => format!("Context {} tokens used\n{model}", ui::compact(u)),
            _ => "Context window: nothing reported yet".into(),
        };
        let used = self
            .limits
            .iter()
            .filter_map(|w| w.used_percent)
            .fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.max(b))));
        let plan_tip = if self.limits.is_empty() {
            "Plan usage: the agent has not reported its limits yet".to_string()
        } else {
            self.limits
                .iter()
                .map(|w| {
                    let pct = w.used_percent.map(|p| format!("{p:.0}%")).unwrap_or_else(|| "—".into());
                    let reset = w.resets_at.map(|r| format!(" · resets {}", until(r))).unwrap_or_default();
                    format!("{} {pct}{reset}{}", w.label(), if w.blocked { " · blocked" } else { "" })
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let ring = |id: &'static str, frac: Option<f32>, label: String, tip: String| {
            let f = frac.unwrap_or(0.0);
            let color = kit::ring_color(f, t);
            div()
                .id(id)
                .h(px(24.0))
                .px(px(6.0))
                .flex()
                .items_center()
                .gap(px(5.0))
                .rounded(px(radius::fit(24.0)))
                .text_size(ty::CAPTION)
                .text_color(if frac.is_some() { color } else { t.text_faint })
                .hover(|s| s.bg(t.glass_hover()))
                .tooltip(ui::tip(tip))
                .child(kit::ring(f, 14.0, color, t.wash(0.12)))
                .child(label)
        };
        // Meters appear once the agent has reported something to show.
        div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .when(ctx_frac.is_some(), |d| {
                d.child(ring(
                    "ring-context",
                    ctx_frac,
                    ctx_frac.map(|f| format!("{:.0}%", f * 100.0)).unwrap_or_default(),
                    ctx_tip,
                ))
            })
            .when(used.is_some(), |d| {
                d.child(
                    ring(
                        "ring-plan",
                        used.map(|u| (u / 100.0) as f32),
                        used.map(|u| format!("{u:.0}%")).unwrap_or_default(),
                        plan_tip,
                    )
                    .cursor_pointer()
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposerEvent::OpenUsage))),
                )
            })
            .into_any_element()
    }

    fn popover(&self, anchor: Anchor, content: impl IntoElement, cx: &mut Context<Self>) -> AnyElement {
        deferred(
            anchored().anchor(anchor).snap_to_window_with_margin(px(8.0)).child(
                div()
                    .mb(px(30.0))
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.menu = None;
                        cx.notify();
                    }))
                    .child(content),
            ),
        )
        .with_priority(2)
        .into_any_element()
    }

    fn attach_button(&self, glyph: &'static str, size: f32, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let button =
            ui::round_button_sized("composer-attach", "Attach files, images or a folder (or paste / drop)", size, t)
                .child(
                    icon(glyph)
                        .size(px(if size < ui::ROUND_BUTTON { 15.0 } else { ui::ICON_LG }))
                        .text_color(t.text_muted),
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Attach, cx)));
        let menu = (self.menu == Some(Menu::Attach)).then(|| {
            let supported = dove_core::supports_add_dirs(self.harness(cx));
            let rows = div()
                .w(px(240.0))
                .flex()
                .flex_col()
                .child(
                    ui::menu_row("att-files", icons::PAPERCLIP, "Files…", false, t)
                        .on_click(cx.listener(|this, _, _, cx| this.attach_files(false, cx))),
                )
                .child(
                    ui::menu_row("att-images", icons::GALLERY, "Images…", false, t)
                        .on_click(cx.listener(|this, _, _, cx| this.attach_files(true, cx))),
                )
                .child(
                    ui::menu_row("att-folder", icons::FOLDER_WITH_FILES, "Folder as context…", false, t)
                        .when(!supported, |d| d.opacity(0.5).tooltip(ui::tip("Extra folders work with Claude Code")))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if supported {
                                this.add_dir(cx);
                            }
                        })),
                )
                .child(ui::separator(t))
                .child(
                    div()
                        .px(px(14.0))
                        .py(px(6.0))
                        .text_size(ty::META)
                        .text_color(t.text_faint)
                        .child("Tip: paste an image or drop files on the composer."),
                );
            self.popover(Anchor::BottomLeft, ui::menu_card(t, rows), cx)
        });
        div().relative().child(button).children(menu).into_any_element()
    }

    fn mic_button(&self, size: f32, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let voice = self.voice.read(cx);
        let recording = voice.is_recording();
        let busy = voice.is_transcribing();
        let ready = voice.is_ready();
        let voice_entity = self.voice.clone();
        let hold = voice_entity.clone();
        let mic = ui::round_button_sized(
            "composer-mic",
            if !ready {
                "Dictation: set up a speech model in Settings › Voice (right-click for options)"
            } else if recording {
                "Release to transcribe (or click to stop)"
            } else {
                "Hold to talk, or click to toggle. Right-click for voice options"
            },
            size,
            t,
        )
        .when(recording, |d| d.bg(t.danger.opacity(0.14)))
        .child(if busy {
            kit::spinner("mic-busy", 14.0, t.text_muted).into_any_element()
        } else {
            icon(icons::MICROPHONE)
                .size(px(if size < ui::ROUND_BUTTON { 15.0 } else { ui::ICON_LG }))
                .text_color(if recording {
                    t.danger
                } else if ready {
                    t.text_muted
                } else {
                    t.text_faint
                })
                .into_any_element()
        })
        .when(ready, |d| {
            d.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                hold.update(cx, |v, cx| v.press(cx));
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                voice_entity.update(cx, |v, cx| v.release(cx));
            })
        })
        .when(!ready, |d| d.on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Mic, cx))))
        // Voice options open on right-click, so the button keeps the same
        // footprint as its neighbours.
        .on_mouse_down(MouseButton::Right, cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Mic, cx)));
        let menu = (self.menu == Some(Menu::Mic)).then(|| {
            let v = self.voice.read(cx);
            let rows = div()
                .w(px(270.0))
                .flex()
                .flex_col()
                .child(
                    div()
                        .px(px(14.0))
                        .py(px(8.0))
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(div().text_size(ty::BODY).text_color(t.text).child("Dictation"))
                        .child(div().text_size(ty::META).text_color(t.text_faint).child(v.status_line())),
                )
                .child(ui::separator(t))
                .child(
                    ui::menu_row(
                        "mic-settings",
                        icons::SETTINGS,
                        if v.is_ready() { "Voice settings…" } else { "Install local voice…" },
                        false,
                        t,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.menu = None;
                        cx.emit(ComposerEvent::OpenVoiceSettings);
                        cx.notify();
                    })),
                );
            self.popover(Anchor::BottomLeft, ui::menu_card(t, rows), cx)
        });
        div()
            .relative()
            .flex()
            .items_center()
            .gap(px(2.0))
            .child(self.call_button(size, t, cx))
            .child(mic)
            .children(menu)
            .into_any_element()
    }

    /// Start (or end) a Voice Mode call with every thread.
    fn call_button(&self, size: f32, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let live = crate::voice_call::strip(cx).is_some();
        ui::round_button_sized(
            "composer-call",
            if live { "End the voice call" } else { "Voice call: talk with your agents hands-free (Ctrl Shift Space)" },
            size,
            t,
        )
        .when(live, |b| b.bg(t.accent.opacity(0.16)))
        .child(
            icon(icons::PHONE)
                .size(px(if size < ui::ROUND_BUTTON { 14.0 } else { ui::ICON_LG - 1.0 }))
                .text_color(if live { t.accent } else { t.text_muted }),
        )
        .on_click(|_, window, cx| window.dispatch_action(Box::new(crate::actions::ToggleVoiceCall), cx))
        .into_any_element()
    }

    fn action_button(&self, has_text: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let steers = dove_core::steers(self.harness(cx));
        let state = SendState::of(self.running, has_text, steers);
        // The glyph and label crossfade whenever the state changes; the
        // fade is keyed by the state, so it replays only on a change.
        let morph = |content: gpui::Div| {
            crate::motion::fade_quick(SharedString::from(format!("composer-send-{}", state.key())), content)
        };
        if state == SendState::Stop {
            return div()
                .id("composer-stop")
                .size(px(ui::ROUND_BUTTON))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(t.solid)
                .cursor_pointer()
                .tooltip(ui::tip("Stop (Esc)"))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposerEvent::Stop)))
                .child(morph(div().size(px(11.0)).rounded(px(3.0)).bg(t.on_solid)))
                .into_any_element();
        }
        let (glyph, tip, label) = match state {
            SendState::Steer => (icons::ARROW_UP, "Steer: send now, the agent adjusts mid-turn (Enter)", Some("Steer")),
            SendState::Queue => (icons::QUEUE_SEND, "Queue: send when this turn ends (Enter)", Some("Queue")),
            _ => (icons::ARROW_UP, "Send (Enter)", None),
        };
        let enabled = has_text || !self.attachments.is_empty() || !self.cards.is_empty();
        div()
            .id("composer-send")
            .h(px(ui::ROUND_BUTTON))
            .min_w(px(ui::ROUND_BUTTON))
            .when(label.is_some(), |d| d.px(px(10.0)))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(4.0))
            .rounded_full()
            .bg(if enabled || self.variant == Variant::Hero { t.solid } else { t.wash(0.08) })
            .when(enabled, |d| d.hover(|s| s.opacity(0.88)))
            .when(!enabled && self.variant == Variant::Hero, |d| d.opacity(0.9))
            .text_color(if enabled { t.on_solid } else { t.text_faint })
            .text_size(ty::META)
            .font_weight(gpui::FontWeight::MEDIUM)
            .when(enabled, |d| d.cursor_pointer())
            .tooltip(ui::tip(tip))
            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)))
            .child(morph(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(icon(glyph).size(px(ui::ICON_SM)).text_color(if enabled || self.variant == Variant::Hero {
                        t.on_solid
                    } else {
                        t.text_faint
                    }))
                    .when_some(label, |d, l| d.child(l)),
            ))
            .into_any_element()
    }

    fn render_queue(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.queue.is_empty() {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_wrap()
                .gap(px(6.0))
                .children(self.queue.iter().enumerate().map(|(i, q)| {
                    let preview: String = q.chars().take(60).collect();
                    div()
                        .id(SharedString::from(format!("queued-{i}")))
                        .h(px(26.0))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .rounded(px(radius::RADIUS))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.wash(0.04))
                        .text_size(ty::SMALL)
                        .text_color(t.text_muted)
                        .on_drag(QueueDrag(i), |d, _, _, cx| cx.new(|_| d.clone()))
                        .drag_over::<QueueDrag>(move |s, _, _, _| s.border_color(gpui::blue()))
                        .on_drop(cx.listener(move |this, from: &QueueDrag, _, cx| {
                            if from.0 < this.queue.len() && i < this.queue.len() {
                                let item = this.queue.remove(from.0);
                                this.queue.insert(i, item);
                                cx.notify();
                            }
                        }))
                        .child(icon(icons::QUEUE_DRAG_HANDLE).size(px(12.0)).text_color(t.text_faint))
                        .child(div().text_color(t.text_faint).child(format!("{}.", i + 1)))
                        .child(div().max_w(px(260.0)).truncate().child(preview))
                        .child(
                            div()
                                .id(SharedString::from(format!("queued-x-{i}")))
                                .cursor_pointer()
                                .child(icon(icons::CLOSE).size(px(11.0)).text_color(t.text_faint))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if i < this.queue.len() {
                                        this.queue.remove(i);
                                    }
                                    cx.notify();
                                })),
                        )
                }))
                .into_any_element(),
        )
    }

    /// Staged files as Zeron's thumbnail strip: 56 px rounded tiles, an
    /// image's picture (a click opens it full size) or a file's type icon and
    /// name, and a round remove button that shows on hover.
    fn render_attachments(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.attachments.is_empty() {
            return None;
        }
        const THUMB: f32 = 56.0;
        Some(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.0))
                .children(self.attachments.iter().enumerate().map(|(i, p)| {
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let is_image = is_image(p);
                    let group = SharedString::from(format!("att-g-{i}"));
                    let open = p.clone();
                    let frame = div()
                        .id(SharedString::from(format!("att-{i}")))
                        .size(px(THUMB))
                        .rounded(px(8.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(t.wash(0.10))
                        .bg(t.wash(0.04))
                        .cursor_pointer()
                        .tooltip(ui::tip(p.display().to_string()))
                        .on_click(move |_, _, cx| cx.open_with_system(&open));
                    let tile = if is_image {
                        frame
                            .child(
                                img(p.clone())
                                    .w(px(THUMB - 2.0))
                                    .h(px(THUMB - 2.0))
                                    .rounded(px(7.0))
                                    .object_fit(gpui::ObjectFit::Cover),
                            )
                            .into_any_element()
                    } else {
                        frame
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap(px(3.0))
                            .px(px(4.0))
                            .child(
                                crate::file_icons::icon(crate::file_icons::FileIconIdentity::file(&name), t.appearance)
                                    .size(px(22.0)),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .text_center()
                                    .text_size(ty::BADGE)
                                    .text_color(t.text_muted)
                                    .child(name.clone()),
                            )
                            .into_any_element()
                    };
                    div().group(group.clone()).relative().flex_none().child(tile).child(
                        div()
                            .id(SharedString::from(format!("att-x-{i}")))
                            .absolute()
                            .top(px(-6.0))
                            .right(px(-6.0))
                            .size(px(18.0))
                            .rounded_full()
                            .bg(t.bg)
                            .border_1()
                            .border_color(t.wash(0.12))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .shadow_sm()
                            .invisible()
                            .group_hover(group, |s| s.visible())
                            .tooltip(ui::tip(format!("Remove {name}")))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.remove_attachment(i, cx);
                            }))
                            .child(icon(icons::CLOSE).size(px(10.0)).text_color(t.text_muted)),
                    )
                }))
                .into_any_element(),
        )
    }

    /// Mentioned threads, folders, windows and tabs as Codex-style cards
    /// above the input: glyph and name, a remove button on hover.
    fn render_cards(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.cards.is_empty() {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_wrap()
                .gap(px(6.0))
                .children(self.cards.iter().enumerate().map(|(i, c)| {
                    let group = SharedString::from(format!("card-g-{i}"));
                    div()
                        .id(SharedString::from(format!("card-{i}")))
                        .group(group.clone())
                        .h(px(28.0))
                        .max_w(px(260.0))
                        .pl(px(8.0))
                        .pr(px(4.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .rounded(px(8.0))
                        .bg(t.wash(0.07))
                        .tooltip(ui::tip(c.detail()))
                        .child(icon(c.glyph()).size(px(13.0)).flex_none().text_color(t.accent))
                        .child(div().min_w_0().truncate().text_size(ty::SMALL).text_color(t.text).child(c.label()))
                        .child(
                            div()
                                .id(SharedString::from(format!("card-x-{i}")))
                                .size(px(18.0))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(5.0))
                                .cursor_pointer()
                                .opacity(0.0)
                                .group_hover(group, |s| s.opacity(1.0))
                                .hover(|s| s.bg(t.wash(0.10)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.remove_card(i, cx);
                                }))
                                .child(icon(icons::CLOSE).size(px(10.0)).text_color(t.text_muted)),
                        )
                }))
                .into_any_element(),
        )
    }

    fn render_mention(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let m = self.mention.as_ref()?;
        let empty = m.items.is_empty();
        let heading = |label: &'static str| {
            div().px(px(14.0)).pt(px(6.0)).pb(px(4.0)).text_size(ty::MICRO).text_color(t.text_faint).child(label)
        };
        let list = div()
            .w(px(440.0))
            .flex()
            .flex_col()
            .when(empty || m.items.first().is_some_and(|i| i.section.is_empty()), |d| {
                d.child(heading(match m.trigger {
                    '@' => "Files, threads, apps and tabs",
                    '#' => "Skills",
                    _ => "Commands",
                }))
            })
            .when(empty, |d| {
                d.child(div().px(px(14.0)).py(px(8.0)).text_size(ty::BODY).text_color(t.text_faint).child(
                    if m.trigger == '#' {
                        "No skills yet. Add some in Settings › Plugins › Skills."
                    } else if self.files.is_empty() && self.root().is_some() {
                        "Loading files…"
                    } else {
                        "Nothing matches"
                    },
                ))
            })
            .children(m.items.iter().enumerate().map(|(i, item)| {
                let selected = i == m.highlight;
                let new_section = !item.section.is_empty()
                    && (i == 0 || m.items.get(i - 1).is_none_or(|p| p.section != item.section));
                let row = div()
                    .id(SharedString::from(format!("mention-{i}")))
                    .mx(px(4.0))
                    .px(px(10.0))
                    .h(px(32.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .rounded(px(kit::popover::MENU_ITEM_RADIUS))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(t.wash(0.10)))
                    .hover(|s| s.bg(t.wash(0.08)))
                    .child(
                        icon(match (&item.card, m.trigger) {
                            (Some(c), _) => c.glyph(),
                            (None, '@') => icons::DOCUMENT,
                            (None, '#') => icons::MAGIC_STICK_3,
                            _ => icons::COMMAND,
                        })
                        .size(px(14.0))
                        .text_color(t.text_faint),
                    )
                    .child(div().flex_none().text_size(ty::BODY).text_color(t.text).child(item.label.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(ty::META)
                            .text_color(t.text_faint)
                            .child(item.detail.clone()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(m) = this.mention.as_mut() {
                            m.highlight = i;
                        }
                        this.accept_mention(window, cx);
                    }));
                div().flex().flex_col().when(new_section, |d| d.child(heading(item.section))).child(row)
            }));
        Some(self.popover(Anchor::BottomLeft, ui::menu_card(t, list), cx))
    }
}

#[derive(Clone)]
struct QueueDrag(usize);

impl Render for QueueDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("QueueDrag");
        let t = Theme::of(cx);
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(radius::RADIUS))
            .bg(t.surface_raised)
            .text_size(ty::META)
            .child(format!("Message {}", self.0 + 1))
    }
}

/// What the composer's round button does right now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SendState {
    Send,
    Steer,
    Queue,
    Stop,
}

impl SendState {
    fn of(running: bool, has_text: bool, steers: bool) -> Self {
        match (running, has_text, steers) {
            (false, _, _) => Self::Send,
            (true, false, _) => Self::Stop,
            (true, true, true) => Self::Steer,
            (true, true, false) => Self::Queue,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Send => "send",
            Self::Steer => "steer",
            Self::Queue => "queue",
            Self::Stop => "stop",
        }
    }
}

impl Composer {
    /// Put live dictation's `text` into the input: the first time at the
    /// cursor (after a space when needed), then over the same span each time
    /// it changes. `done` ends the span.
    fn place_live_dictation(&mut self, text: &str, done: bool, window: &mut Window, cx: &mut Context<Self>) {
        let range = self.live_dictation.clone();
        let text = text.trim().to_string();
        let new_range = self.input.update(cx, |s, cx| {
            let value = s.value().to_string();
            match range
                .filter(|r| r.end <= value.len() && value.is_char_boundary(r.start) && value.is_char_boundary(r.end))
            {
                Some(r) => {
                    // Keep the separating space only while there is text after it.
                    let lead = if value[r.clone()].starts_with(' ') && !text.is_empty() { " " } else { "" };
                    let piece = format!("{lead}{text}");
                    s.set_selected_range(r.clone(), cx);
                    s.replace(piece.clone(), window, cx);
                    r.start..r.start + piece.len()
                }
                None => {
                    if text.is_empty() {
                        return 0..0;
                    }
                    let at = s.cursor().min(value.len());
                    let before = &value[..at];
                    let sep = if before.is_empty() || before.ends_with(char::is_whitespace) { "" } else { " " };
                    let piece = format!("{sep}{text}");
                    s.insert(piece.clone(), window, cx);
                    at..at + piece.len()
                }
            }
        });
        self.live_dictation = if done || new_range.is_empty() && text.is_empty() { None } else { Some(new_range) };
        cx.notify();
    }
}

fn focus_input(input: &Entity<TextareaState>, window: &mut Window, cx: &mut App) {
    input.update(cx, |s, cx| s.focus(window, cx));
}

/// "in 2h 10m" for a unix-seconds reset time.
fn until(reset: i64) -> String {
    let now = (dove_core::store::now_ms() / 1000).max(0);
    let secs = (reset - now).max(0);
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("in {}m", secs / 60),
        3600..=86_399 => format!("in {}h {}m", secs / 3600, (secs % 3600) / 60),
        _ => format!("in {}d {}h", secs / 86_400, (secs % 86_400) / 3600),
    }
}

pub fn mode_id(m: PermissionMode) -> &'static str {
    match m {
        PermissionMode::Default => "default",
        PermissionMode::AcceptEdits => "accept_edits",
        PermissionMode::Plan => "plan",
        PermissionMode::Bypass => "bypass",
    }
}

fn mode_label(m: PermissionMode) -> &'static str {
    match m {
        PermissionMode::Default => "Ask every time",
        PermissionMode::AcceptEdits => "Auto-edit",
        PermissionMode::Plan => "Plan",
        PermissionMode::Bypass => "Bypass",
    }
}

fn mode_icon(m: PermissionMode) -> &'static str {
    match m {
        PermissionMode::Default => icons::KEY_MINIMALISTIC,
        PermissionMode::AcceptEdits => icons::PEN,
        PermissionMode::Plan => icons::CHECKLIST,
        PermissionMode::Bypass => icons::DANGER_TRIANGLE,
    }
}

pub fn mode_from_id(id: &str) -> PermissionMode {
    match id {
        "accept_edits" => PermissionMode::AcceptEdits,
        "plan" => PermissionMode::Plan,
        "bypass" => PermissionMode::Bypass,
        _ => PermissionMode::Default,
    }
}

/// Focus a composer's input. Takes the entity so callers holding `cx`
/// mutably need no outstanding read borrow.
/// The block a sent message carries for its staged files: the agent reads
/// the paths, and the transcript draws them back as thumbnails and pills.
pub const ATTACH_HEADER: &str = "Attached files (read them as needed):";

/// A sent message's own text and the files it carried (see
/// [`ATTACH_HEADER`]). Text without the block comes back whole.
pub fn split_attachments(text: &str) -> (String, Vec<PathBuf>) {
    let strip = |body: &str| {
        let cut = [body.find(SKILLS_HEADER), body.find(mentions::MENTIONS_HEADER)].into_iter().flatten().min();
        match cut {
            Some(s) => body[..s].trim_end().to_string(),
            None => body.to_string(),
        }
    };
    let Some(at) = text.find(ATTACH_HEADER) else { return (strip(text), Vec::new()) };
    let body = strip(text[..at].trim_end());
    let files = text[at + ATTACH_HEADER.len()..]
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- "))
        .filter(|p| !p.trim().is_empty())
        .map(|p| PathBuf::from(p.trim()))
        .collect();
    (body, files)
}

/// The block a sent message carries for the skills it names with `#`: the
/// agent loads each one, and the transcript leaves the block out.
pub const SKILLS_HEADER: &str = "Skills to use for this message (read each SKILL.md and follow it):";

/// The `#` menu for query `q`: installed skills that are on, then ones that
/// are off (picking turns them on), then recommended ones not installed yet
/// (picking installs them).
fn skill_items(q: &str) -> Vec<MentionItem> {
    use dove_core::skills::{self, Source};
    let reg = skills::load();
    let hit = |name: &str, detail: &str| q.is_empty() || name.contains(q) || detail.to_ascii_lowercase().contains(q);
    let mut on = Vec::new();
    let mut off = Vec::new();
    for s in &reg.skills {
        if !hit(&s.name, &s.description) {
            continue;
        }
        let from = match &s.source {
            Source::Bundled => String::new(),
            Source::Git { repo, .. } => format!(" · {repo}"),
            Source::Local { .. } => " · yours".to_string(),
        };
        let item = |detail: String, skill| MentionItem {
            label: format!("#{}", s.name),
            detail,
            insert: format!("#{}", s.name),
            skill,
            section: "Skills",
            ..Default::default()
        };
        if s.enabled {
            on.push(item(format!("{}{from}", s.description), None));
        } else {
            off.push(item(
                format!("Off, turns on when picked · {}", s.description),
                Some(SkillPick::Enable(s.name.clone())),
            ));
        }
    }
    let recommended = skills::RECOMMENDED
        .iter()
        .filter(|&(_repo, _path, name, blurb)| reg.get(name).is_none() && hit(name, blurb))
        .map(|(repo, path, name, blurb)| MentionItem {
            label: format!("#{name}"),
            detail: format!("Recommended, installs when picked · {blurb}"),
            insert: format!("#{name}"),
            skill: Some(SkillPick::Install { repo: repo.to_string(), path: path.to_string() }),
            section: "Skills",
            ..Default::default()
        });
    on.into_iter().chain(off).chain(recommended).take(MAX_SKILL_MENTIONS).collect()
}

const MAX_SKILL_MENTIONS: usize = 12;

/// The skills a message names as `#name` (installed and on), as the block
/// that tells the agent to load them.
fn skills_block(text: &str) -> Option<String> {
    let names = skill_tags(text);
    if names.is_empty() {
        return None;
    }
    let reg = dove_core::skills::load();
    let root = dove_core::skills::root();
    let lines: Vec<String> = names
        .iter()
        // A recommended skill picked a moment ago may still be installing;
        // its folder is where it will land.
        .filter(|n| {
            reg.get(n).map_or_else(
                || dove_core::skills::RECOMMENDED.iter().any(|(_, _, name, _)| *name == n.as_str()),
                |s| s.enabled,
            )
        })
        .map(|n| format!("- {n}: {}", root.join(n).join("SKILL.md").display()))
        .collect();
    (!lines.is_empty()).then(|| format!("{SKILLS_HEADER}\n{}", lines.join("\n")))
}

/// Every distinct `#name` in `text` (skill-name characters only).
fn skill_tags(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        let Some(tag) = word.strip_prefix('#') else { continue };
        let tag: String =
            tag.chars().take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-').collect();
        let tag = tag.trim_end_matches('-').to_string();
        if !tag.is_empty() && !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

/// Whether `path` is an image Dove can show as a thumbnail.
pub fn is_image(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
    )
}

/// Stage files in `composer`, as a drop would.
pub fn attach_paths(composer: &Entity<Composer>, paths: Vec<PathBuf>, cx: &mut App) {
    composer.update(cx, |c, cx| c.attach(paths, cx));
}

pub fn focus(composer: &Entity<Composer>, window: &mut Window, cx: &mut App) {
    let input = composer.read(cx).input.clone();
    focus_input(&input, window, cx);
}

pub fn set_text(composer: &Entity<Composer>, text: &str, window: &mut Window, cx: &mut App) {
    let input = composer.read(cx).input.clone();
    let text = text.to_string();
    input.update(cx, |s, cx| s.set_value(text, window, cx));
}

/// The composer's text, cleared from it (None when it is empty).
pub fn take_text(composer: &Entity<Composer>, window: &mut Window, cx: &mut App) -> Option<String> {
    let input = composer.read(cx).input.clone();
    let text = input.read(cx).value().to_string();
    if text.trim().is_empty() {
        return None;
    }
    input.update(cx, |s, cx| s.set_value(String::new(), window, cx));
    Some(text)
}

pub fn is_empty(composer: &Entity<Composer>, cx: &App) -> bool {
    composer.read(cx).input.read(cx).value().trim().is_empty()
}

/// The hero bar's left inset for its prompt text. The chips above and below
/// the bar and the attach glyph inside it all start on this line.
const HERO_INSET: f32 = 19.0;
/// The gap between the send disc and the hero bar's right and bottom edges.
const HERO_CORNER: f32 = 12.0;
/// The docked bar: its height, the prompt's line box (a button's height, so
/// text and controls share one centre), the row's side insets, and the
/// footer under it. Tucked or open, the bar keeps these.
const DOCK_H: f32 = 50.0;
const DOCK_LINE: f32 = ui::ROUND_BUTTON;
const DOCK_PAD_L: f32 = 4.0;
const DOCK_PAD_R: f32 = 6.0;
const DOCK_FOOTER_H: f32 = 26.0;
const DOCK_GAP: f32 = 4.0;

impl Composer {
    fn docked(&self) -> bool {
        self.dock_bar || (self.variant == Variant::Hero && self.home_dock)
    }

    fn note_dock_hover(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if self.dock_hot == hovered {
            return;
        }
        self.dock_hot = hovered;
        cx.notify();
    }

    /// Tuck the thread bar while the sidebar is open. No-op when unchanged.
    pub fn set_dock_tuck(&mut self, tuck: bool, cx: &mut Context<Self>) {
        if self.dock_tuck == tuck {
            return;
        }
        self.dock_tuck = tuck;
        if !tuck {
            self.dock_hot = false;
        }
        cx.notify();
    }

    /// The pointer entered or left a usage meter. Leaving waits briefly so
    /// the pointer can cross onto the card without the card vanishing.
    fn note_meter(&mut self, which: Option<Meter>, cx: &mut Context<Self>) {
        if which.is_some() {
            self.meter = which;
            self.meter_gen = self.meter_gen.wrapping_add(1);
            cx.notify();
            return;
        }
        let seen = self.meter_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(140)).await;
            let _ = this.update(cx, |this, cx| {
                if this.meter_gen == seen {
                    this.meter = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn menu_caption(&self, text: &str, t: &Theme) -> AnyElement {
        div()
            .px(px(12.0))
            .pt(px(8.0))
            .pb(px(2.0))
            .text_size(ty::MICRO)
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(t.text_faint)
            .child(text.to_string())
            .into_any_element()
    }

    /// Project, extra folders and device in one menu. The bottom bar's
    /// advantage over the center composer, which splits those controls.
    fn place_chip(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let name = self.project.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "Choose a folder".into());
        let open = self.menu == Some(Menu::Place);
        let chip = div()
            .id("composer-place")
            .h(px(24.0))
            .max_w(px(180.0))
            .px(px(6.0))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(5.0))
            .rounded(px(radius::fit(24.0)))
            .cursor_pointer()
            .text_size(ty::BODY)
            .text_color(if open { t.text.opacity(0.95) } else { t.text.opacity(0.72) })
            .when(open, |d| d.bg(t.element_hover))
            .hover(|s| s.bg(t.element_hover).text_color(t.text.opacity(0.95)))
            .tooltip(ui::tip("Folder and environment"))
            .child(icon(icons::FOLDER).size(px(12.0)).text_color(t.text.opacity(0.55)))
            .child(div().min_w_0().truncate().child(name))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Place, cx)));
        chip.into_any_element()
    }

    /// Folder rows, then one environment line. Anchored to the field's
    /// top center so the card sits above the bar rather than the label.
    fn place_card(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.project.as_ref().map(|p| p.id);
        let persona = self.persona.as_ref().map(|p| p.id.clone());
        let mut rows = div().w(px(280.0)).max_h(px(360.0)).flex().flex_col();
        if !self.personas.is_empty() {
            rows = rows.child(self.menu_caption("Teammate", t)).child(
                ui::menu_row("place-persona-none", icons::BOT, "Default", false, t)
                    .when(persona.is_none(), |d| d.child(icon(icons::CHECK).size(px(14.0)).text_color(t.accent)))
                    .on_click(cx.listener(|this, _, _, cx| this.use_persona(None, cx))),
            );
            for p in &self.personas {
                let pick = p.clone();
                let on = persona.as_deref() == Some(p.id.as_str());
                rows = rows.child(
                    ui::menu_row(
                        SharedString::from(format!("place-persona-{}", p.id)),
                        icons::BOT,
                        p.name.clone(),
                        false,
                        t,
                    )
                    .when(on, |d| d.child(icon(icons::CHECK).size(px(14.0)).text_color(t.accent)))
                    .on_click(cx.listener(move |this, _, _, cx| this.use_persona(Some(pick.clone()), cx))),
                );
            }
            rows = rows.child(ui::separator(t));
        }
        rows = rows.child(div().id("place-projects").max_h(px(200.0)).overflow_y_scroll().flex().flex_col().children(
            self.projects.iter().map(|p| {
                let id = p.id;
                ui::menu_row(SharedString::from(format!("place-{id}")), icons::FOLDER, p.name.clone(), false, t)
                    .when(selected == Some(id), |d| d.child(icon(icons::CHECK).size(px(14.0)).text_color(t.accent)))
                    .tooltip(ui::tip(p.path.display().to_string()))
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_project(id, cx)))
            }),
        ));
        rows = rows
            .child(
                ui::menu_row("place-add", icons::ADD_CIRCLE, "Add folder…", false, t)
                    .on_click(cx.listener(|this, _, _, cx| this.add_folder_project(cx))),
            )
            .child(match &self.clone_url {
                Some(input) => div()
                    .mx(px(8.0))
                    .my(px(4.0))
                    .h(px(32.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .rounded(px(radius::RADIUS))
                    .border_1()
                    .border_color(t.border_strong)
                    .text_size(ty::BODY)
                    .child(div().flex_1().child(Input::new(input)))
                    .into_any_element(),
                None => ui::menu_row("place-clone", icons::GLOBAL, "Clone from URL…", false, t)
                    .on_click(cx.listener(|this, _, window, cx| this.start_clone(window, cx)))
                    .into_any_element(),
            });
        let supported = dove_core::supports_add_dirs(self.harness(cx));
        if !self.add_dirs.is_empty() || supported {
            for (i, dir) in self.add_dirs.iter().enumerate() {
                rows = rows.child(
                    ui::menu_row(
                        SharedString::from(format!("place-dir-{i}")),
                        icons::FOLDER_WITH_FILES,
                        dir.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| dir.display().to_string()),
                        false,
                        t,
                    )
                    .tooltip(ui::tip(dir.display().to_string()))
                    .child(icon(icons::CLOSE).size(px(13.0)).text_color(t.text_faint))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if i < this.add_dirs.len() {
                            this.add_dirs.remove(i);
                        }
                        cx.notify();
                    })),
                );
            }
            rows = rows.child(
                ui::menu_row("place-dir-add", icons::ADD_CIRCLE, "Add another folder…", false, t)
                    .when(!supported, |d| d.opacity(0.5).tooltip(ui::tip("Extra folders work with Claude Code")))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if dove_core::supports_add_dirs(this.harness(cx)) {
                            this.add_dir(cx);
                        }
                    })),
            );
        }
        rows = rows.child(ui::separator(t)).child(
            div()
                .id("place-env")
                .h(px(34.0))
                .px(px(10.0))
                .mx(px(4.0))
                .my(px(4.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .rounded(px(kit::popover::MENU_ITEM_RADIUS))
                .cursor_pointer()
                .text_size(ty::MD)
                .text_color(t.text)
                .hover(|s| s.bg(t.element_hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.menu = None;
                    cx.notify();
                }))
                .child(icon(icons::LAPTOP).size(px(16.0)).flex_none().text_color(t.text_muted))
                .child(div().flex_1().min_w_0().truncate().child(format!("Local · {}", ui::host_name())))
                .child(icon(icons::CHECK).size(px(14.0)).flex_none().text_color(t.accent))
                .child(div().flex_none().text_size(ty::META).text_color(t.text_faint).child("Cloud · soon")),
        );
        deferred(
            anchored()
                .anchor(Anchor::BottomCenter)
                .offset(point(px(0.0), px(-8.0)))
                .snap_to_window_with_margin(px(8.0))
                .child(
                    div()
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.menu = None;
                            cx.notify();
                        }))
                        .child(ui::menu_card(t, rows)),
                ),
        )
        .with_priority(2)
        .into_any_element()
    }

    fn usage_bar(&self, fraction: f32, t: &Theme) -> AnyElement {
        let fraction = fraction.clamp(0.0, 1.0);
        let color = kit::ring_color(fraction, t);
        div()
            .w(px(96.0))
            .h(px(4.0))
            .flex_none()
            .rounded_full()
            .overflow_hidden()
            .bg(t.wash(0.10))
            .when(fraction > 0.0, |el| {
                el.child(
                    div().h_full().w(px((96.0 * fraction.max(0.04)).min(96.0))).rounded_full().bg(color.opacity(0.9)),
                )
            })
            .into_any_element()
    }

    fn plan_card(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let used = self
            .limits
            .iter()
            .filter_map(|w| w.used_percent)
            .fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.max(b))));
        let mut body = div().w(px(300.0)).px(px(8.0)).py(px(6.0)).flex().flex_col().gap(px(8.0)).child(
            div()
                .flex()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .text_size(ty::BODY)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(t.text)
                        .child("Plan usage"),
                )
                .when_some(used, |d, u| {
                    d.child(
                        div()
                            .text_size(ty::META)
                            .text_color(kit::ring_color((u / 100.0) as f32, t))
                            .child(format!("{u:.0}%")),
                    )
                }),
        );
        if self.limits.is_empty() {
            body = body.child(
                div().text_size(ty::META).text_color(t.text_muted).child("The agent has not reported its limits yet."),
            );
        } else {
            for window in &self.limits {
                let fraction = window.used_percent.map(|p| (p / 100.0) as f32).unwrap_or(0.0);
                let pct = window.used_percent.map(|p| format!("{p:.0}%")).unwrap_or_else(|| "—".into());
                let reset = window.resets_at.map(|r| format!("Resets {}", until(r)));
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .child(
                                    div()
                                        .w(px(108.0))
                                        .flex_none()
                                        .truncate()
                                        .text_size(ty::META)
                                        .text_color(t.text_muted)
                                        .child(window.label()),
                                )
                                .child(self.usage_bar(fraction, t))
                                .child(
                                    div()
                                        .w(px(36.0))
                                        .text_right()
                                        .text_size(ty::META)
                                        .text_color(if window.blocked {
                                            t.danger
                                        } else {
                                            kit::ring_color(fraction, t)
                                        })
                                        .child(pct),
                                ),
                        )
                        .when(window.blocked || reset.is_some(), |d| {
                            d.child(
                                div()
                                    .text_size(ty::MICRO)
                                    .text_color(if window.blocked { t.danger } else { t.text_faint })
                                    .child(match (window.blocked, reset.clone()) {
                                        (true, Some(r)) => format!("Blocked · {r}"),
                                        (true, None) => "Blocked".into(),
                                        (false, Some(r)) => r,
                                        (false, None) => String::new(),
                                    }),
                            )
                        }),
                );
            }
        }
        body = body.child(ui::separator(t)).child(
            div()
                .id("plan-open-usage")
                .h(px(28.0))
                .px(px(4.0))
                .flex()
                .items_center()
                .rounded(px(kit::popover::MENU_ITEM_RADIUS))
                .cursor_pointer()
                .text_size(ty::META)
                .text_color(t.text_muted)
                .hover(|s| s.bg(t.wash(0.08)).text_color(t.text))
                .child("Open usage")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposerEvent::OpenUsage))),
        );
        ui::menu_card(t, body).into_any_element()
    }

    fn meter_chip(&self, which: Meter, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let used = self
            .limits
            .iter()
            .filter_map(|w| w.used_percent)
            .fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.max(b))));
        let fraction = used.map(|u| (u / 100.0) as f32).unwrap_or(0.0);
        let color = if used.is_some() { kit::ring_color(fraction, t) } else { t.text_faint };
        let id = "dock-plan";
        let open = self.meter == Some(which);
        let trigger = div()
            .id(id)
            .size(px(22.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(radius::fit(22.0)))
            .cursor_pointer()
            .when(open, |d| d.bg(t.wash(0.08)))
            .hover(|s| s.bg(t.wash(0.08)))
            .child(kit::ring(fraction, 15.0, color, t.wash(0.16)))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.note_meter(if *hovered { Some(which) } else { None }, cx);
            }))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposerEvent::OpenUsage)));
        let card = open.then(|| {
            let content = self.plan_card(t, cx);
            deferred(
                anchored().anchor(Anchor::BottomRight).snap_to_window_with_margin(px(8.0)).child(
                    div()
                        .id("meter-card")
                        .mb(px(4.0))
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            this.note_meter(if *hovered { Some(which) } else { None }, cx);
                        }))
                        .child(content),
                ),
            )
            .with_priority(3)
            .into_any_element()
        });
        div().relative().child(trigger).children(card).into_any_element()
    }

    fn set_orchestrate(&mut self, o: crate::orchestrate::Orchestrate, cx: &mut Context<Self>) {
        self.orchestrate = o;
        crate::orchestrate::set(&self.engine.store, self.thread.as_ref().map(|(t, _)| t.id), o);
        let (title, body) = match o {
            crate::orchestrate::Orchestrate::Off => ("Orchestrator off", "This thread works alone."),
            crate::orchestrate::Orchestrate::Ask => (
                "Orchestrator on",
                "The agent can propose other agents; you review and approve each one above the input.",
            ),
            crate::orchestrate::Orchestrate::Auto => {
                ("Orchestrator on · auto", "The agent starts other agents without asking.")
            }
        };
        crate::kit::Toasts::push(cx, title, Some(body.to_string()), crate::kit::Tone::Info, None);
        cx.notify();
    }

    /// The `/orchestrator` chip while it is on; a click cycles ask, auto, off.
    fn orchestrate_chip(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let o = self.orchestrate;
        (o != crate::orchestrate::Orchestrate::Off).then(|| {
            div()
                .id("composer-orchestrate")
                .h(px(24.0))
                .px(px(6.0))
                .flex()
                .items_center()
                .gap(px(4.0))
                .rounded(px(radius::fit(24.0)))
                .cursor_pointer()
                .text_size(ty::BODY)
                .text_color(t.accent)
                .hover(|s| s.bg(t.element_hover))
                .tooltip(ui::tip("Orchestrator: ask approves each agent, auto starts them. Click to change"))
                .child(icon(icons::FORK).size(px(13.0)).text_color(t.accent))
                .child(if o == crate::orchestrate::Orchestrate::Auto { "Auto" } else { "Orchestrator" })
                .on_click(cx.listener(move |this, _, _, cx| this.set_orchestrate(o.next(), cx)))
                .into_any_element()
        })
    }

    /// Permission level. The dock, the thread composer, and the centered
    /// home composer all draw this chip. The stored default stays Ask.
    fn dock_mode(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let bypass = self.mode == PermissionMode::Bypass;
        let open = self.menu == Some(Menu::Mode);
        let label = if bypass { "Bypass permissions" } else { mode_label(self.mode) };
        let chip = div()
            .id("composer-mode")
            .h(px(24.0))
            .px(px(6.0))
            .flex()
            .items_center()
            .rounded(px(radius::fit(24.0)))
            .cursor_pointer()
            .text_size(ty::BODY)
            .text_color(t.text.opacity(0.62))
            .when(open, |d| d.bg(t.element_hover))
            .hover(|s| s.bg(t.element_hover).text_color(t.text.opacity(0.9)))
            .tooltip(ui::tip("Permission level"))
            .child(label)
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Mode, cx)));
        let menu = open.then(|| {
            let modes = [
                (PermissionMode::Default, "Ask every time", "Approve edits and commands"),
                (PermissionMode::AcceptEdits, "Auto-edit", "Edits apply; commands ask"),
                (PermissionMode::Plan, "Plan", "Read-only until you approve a plan"),
                (PermissionMode::Bypass, "Bypass permissions", "Never ask"),
            ];
            let content = div().w(px(280.0)).flex().flex_col().gap(px(popover::MENU_GAP)).children(
                modes.into_iter().map(|(m, label, desc)| {
                    let selected = m == self.mode;
                    let danger = m == PermissionMode::Bypass;
                    popover::menu_row_instant(t, format!("dock-mode-{}", mode_id(m)), selected)
                        .aria_label(label)
                        .py(px(6.0))
                        .child(icon(mode_icon(m)).size(px(16.0)).flex_none().text_color(if danger {
                            t.danger
                        } else {
                            t.text_muted
                        }))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(ty::BODY)
                                        .text_color(if danger { t.danger } else { t.text })
                                        .child(label),
                                )
                                .child(div().truncate().text_size(ty::META).text_color(t.text_faint).child(desc)),
                        )
                        .when(selected, |d| d.child(icon(icons::CHECK).size(px(14.0)).flex_none().text_color(t.accent)))
                        .on_click(cx.listener(move |this, _, _, cx| this.set_mode(m, cx)))
                }),
            );
            self.popover(Anchor::BottomLeft, ui::menu_card(t, content), cx)
        });
        div().relative().child(chip).children(menu).into_any_element()
    }

    /// The bottom-center bar: a short field, then folder, permissions,
    /// model and the two usage meters on the row beneath it.
    fn render_dock(
        &self,
        _window: &mut Window,
        t: &Theme,
        has_text: bool,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let staged = self.queue.len() + self.attachments.len() + self.cards.len() > 0;
        // A squircle: large corners, with a straight run along the top and bottom.
        let radius = radius::plate(DOCK_H);
        let edge = t.composer_border();
        let busy = has_text
            || !self.attachments.is_empty()
            || !self.cards.is_empty()
            || !self.queue.is_empty()
            || self.running
            || focused
            || self.menu.is_some()
            || self.meter.is_some();
        if self.dock_tuck && !self.dock_hot && !busy {
            // Tucked, the bar keeps its full size and the `+` and hint sit
            // exactly where the open bar draws them, with the footer's room
            // held empty, so opening it changes what is shown, never where.
            let hint = if self.thread.is_some() { "Send follow-up" } else { "Do anything…" };
            let bar = div()
                .w_full()
                .h(px(DOCK_H))
                .pl(px(DOCK_PAD_L))
                .pr(px(DOCK_PAD_R))
                .flex()
                .items_center()
                .gap(px(2.0))
                .rounded(px(radius))
                .border_1()
                .border_color(edge)
                .bg(t.composer_bg())
                .cursor_text()
                .text_size(ty::TITLE)
                .text_color(t.text_faint)
                .child(
                    div()
                        .size(px(ui::ROUND_BUTTON))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(icons::PLUS).size(px(ui::ICON_LG)).text_color(t.text_muted)),
                )
                .child(div().flex_1().min_w_0().px(px(2.0)).truncate().child(hint));
            return div()
                .id("composer")
                .w_full()
                .flex()
                .flex_col()
                .gap(px(DOCK_GAP))
                .child(bar)
                .child(div().h(px(DOCK_FOOTER_H)).flex_none())
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| this.note_dock_hover(*hovered, cx)))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.dock_hot = true;
                    focus_input(&this.input, window, cx);
                    cx.notify();
                }))
                .into_any_element();
        }
        // The plate looks the same in every permission mode; the mode chip
        // under it says which one is on.
        let border = if self.dragging_files {
            t.accent
        } else if focused {
            edge.opacity((edge.a * 2.0).min(1.0))
        } else {
            edge
        };
        let surface = div()
            .id("composer-pill")
            .w_full()
            .min_h(px(DOCK_H))
            .rounded(px(radius))
            .border_1()
            .border_color(border)
            .bg(t.composer_hero_bg())
            .shadow(t.composer_shadows(focused))
            .flex()
            .flex_col()
            .text_size(ty::TITLE)
            .cursor_text()
            .on_click(cx.listener(|this, _, window, cx| focus_input(&this.input, window, cx)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.dragging_files = false;
                this.attach(paths.paths().to_vec(), cx);
            }))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(gpui::transparent_black()));
        // A fixed line box: one line is exactly as tall as the buttons, so
        // the prompt's line and the controls share the bar's centre.
        let text = div()
            .flex_1()
            .min_w_0()
            .relative()
            .line_height(px(DOCK_LINE))
            .child(Textarea::new(&self.input))
            .children(self.render_mention(t, cx));
        let staged_rows = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .children(self.render_queue(t, cx))
            .children(self.render_cards(t, cx))
            .children(self.render_attachments(t, cx));
        let show_send = has_text || self.running || !self.attachments.is_empty() || !self.cards.is_empty();
        // The row fills the bar inside its border, so its content is centred
        // on the bar itself (equal room above and below).
        let bar = surface.when(staged, |d| d.child(staged_rows.px(px(12.0)).pt(px(12.0)).pb(px(2.0)))).child(
            div()
                .w_full()
                .min_h(px(DOCK_H - 2.0))
                .pl(px(DOCK_PAD_L))
                .pr(px(DOCK_PAD_R))
                .py(px((DOCK_H - 2.0 - DOCK_LINE) / 2.0))
                .flex()
                .items_center()
                .gap(px(2.0))
                .child(self.attach_button(icons::PLUS, 28.0, t, cx))
                .child(text.px(px(2.0)))
                .child(self.mic_button(28.0, t, cx))
                .when(show_send, |d| d.child(div().pl(px(2.0)).child(self.action_button(has_text, t, cx)))),
        );
        let bar = ui::frosted(radius, t.composer_blur().max(22.0), bar);
        // The footer's ends line up with the bar's: the folder glyph under the
        // `+`, the usage ring centred under the last button (mic or send).
        let footer = div()
            .w_full()
            .h(px(DOCK_FOOTER_H))
            .pl(px(DOCK_PAD_L + 1.0 + ui::ROUND_BUTTON / 2.0 - 12.0))
            .pr(px(DOCK_PAD_R + 1.0 + ui::ROUND_BUTTON / 2.0 - 11.0))
            .flex()
            .items_center()
            .gap(px(2.0))
            .child(self.place_chip(t, cx))
            .child(div().w(px(1.0)).h(px(10.0)).mx(px(6.0)).bg(t.border))
            .child(self.dock_mode(t, cx))
            .children(self.orchestrate_chip(t, cx))
            .child(div().flex_1())
            .child(div().flex_none().max_w(px(240.0)).child(self.picker.clone()))
            .child(self.meter_chip(Meter::Plan, t, cx));
        div()
            .id("composer")
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(DOCK_GAP))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| this.note_dock_hover(*hovered, cx)))
            .capture_key_down(cx.listener(Self::on_key))
            .when(self.menu == Some(Menu::Place), |d| {
                d.child(div().absolute().top_0().left(relative(0.5)).child(self.place_card(t, cx)))
            })
            .child(bar)
            .child(footer)
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .id("composer-error")
                        .px(px(8.0))
                        .text_size(ty::META)
                        .text_color(t.danger)
                        .cursor_pointer()
                        .child(e)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.error = None;
                            cx.notify();
                        })),
                )
            })
            .into_any_element()
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("Composer");
        let body = self.render_body(window, cx);
        let Some((height, host)) = self.cache_slot.clone() else { return body };
        // A cached view is laid out at the host's last measured height, so a
        // grown or shrunk composer keeps its bottom edge (it grows upward,
        // as it does uncached) and asks the host for its new height.
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_end()
            .child(
                div().relative().w_full().flex_none().child(body).child(
                    gpui::canvas(
                        move |b, _, cx| {
                            let h = f32::from(b.size.height);
                            if (height.get() - h).abs() > 0.5 {
                                height.set(h);
                                cx.defer(move |cx| cx.notify(host));
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                ),
            )
            .into_any_element()
    }
}

impl Composer {
    fn render_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let t = &theme;
        let has_text = !self.input.read(cx).value().trim().is_empty();
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let home = self.thread.is_none();
        if self.docked() {
            return self.render_dock(window, t, has_text, focused, cx);
        }

        let hero = self.variant == Variant::Hero;
        let staged = self.queue.len() + self.attachments.len() + self.cards.len() > 0;
        // Both are Zeron's 26 px plate; the hero's corner is also concentric
        // with its send disc (14 + 12).
        let radius = crate::design::radius::COMPOSER;
        let edge = t.composer_border();
        // The plate looks the same in every permission mode; the mode chip
        // under it says which one is on.
        let border = if self.dragging_files {
            t.accent
        } else if focused {
            edge.opacity((edge.a * 2.0).min(1.0))
        } else {
            edge
        };
        let fill = if hero { t.composer_hero_bg() } else { t.composer_bg() };
        // The material (glass, acrylic or opaque) is shared; only the
        // layout differs between the hero and the pill.
        let surface = div()
            .id("composer-pill")
            .w_full()
            .rounded(px(radius))
            .border_1()
            .border_color(border)
            .bg(fill)
            .shadow(t.composer_shadows(focused))
            .flex()
            .flex_col()
            .text_size(ty::TITLE)
            .cursor_text()
            .on_click(cx.listener(|this, _, window, cx| focus_input(&this.input, window, cx)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.dragging_files = false;
                this.attach(paths.paths().to_vec(), cx);
            }))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(gpui::transparent_black()));
        let text =
            div().flex_1().min_w_0().relative().child(Textarea::new(&self.input)).children(self.render_mention(t, cx));
        let staged_rows = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .children(self.render_queue(t, cx))
            .children(self.render_cards(t, cx))
            .children(self.render_attachments(t, cx));

        let box_ = if hero {
            // A short glass bar: the prompt on top, attach on the left, the
            // model picker and send on the right.
            surface
                .min_h(px(120.0))
                .when(staged, |d| d.child(staged_rows.px(px(14.0)).pt(px(12.0))))
                .child(
                    div()
                        .w_full()
                        .flex_1()
                        .min_h(px(64.0))
                        .px(px(HERO_INSET - 1.0))
                        .pt(px(16.0))
                        .pb(px(4.0))
                        .flex()
                        .child(text),
                )
                .child(
                    // Attach's glyph lines up with the prompt text; send
                    // sits as far from the right edge as from the bottom,
                    // concentric with the card's corner.
                    div()
                        .w_full()
                        .h(px(ui::ROUND_BUTTON))
                        .mb(px(HERO_CORNER - 1.0))
                        .pl(px(HERO_INSET - 1.0 - (ui::ROUND_BUTTON - ui::ICON_LG) / 2.0))
                        .pr(px(HERO_CORNER - 1.0))
                        .flex()
                        .items_center()
                        .gap(px(2.0))
                        .child(self.attach_button(icons::PAPERCLIP, ui::ROUND_BUTTON, t, cx))
                        .child(div().flex_none().child(self.dock_mode(t, cx)))
                        .children(self.orchestrate_chip(t, cx).map(|c| div().flex_none().child(c)))
                        .child(div().flex_1())
                        .child(div().min_w_0().max_w(px(240.0)).overflow_hidden().child(self.picker.clone()))
                        .child(self.mic_button(ui::ROUND_BUTTON, t, cx))
                        .child(div().pl(px(2.0)).child(self.action_button(has_text, t, cx))),
                )
        } else {
            // Zeron's plate: the prompt on top, the controls on a row under
            // it. It grows upward as you type (76 px to about 260 px).
            surface
                .min_h(px(PLATE_MIN_H))
                .pt(px(10.0))
                .pb(px(7.0))
                .pl(px(7.0))
                .pr(px(8.0))
                .gap(px(4.0))
                .when(staged, |d| d.child(staged_rows.px(px(4.0)).pt(px(2.0)).pb(px(4.0))))
                .child(div().w_full().min_w_0().flex().px(px(8.0)).child(text.min_w(px(72.0))))
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .h(px(ui::ROUND_BUTTON))
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .child(self.attach_button(icons::PAPERCLIP, ui::ROUND_BUTTON, t, cx))
                        .child(div().flex_1())
                        .child(
                            // In a narrow column the model label gives way
                            // first; voice and send always keep their size.
                            div()
                                .flex_shrink(1.0)
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .flex_shrink(1.0)
                                        .min_w_0()
                                        .max_w(px(220.0))
                                        .overflow_hidden()
                                        .child(self.picker.clone()),
                                )
                                .child(div().flex_none().child(self.mic_button(ui::ROUND_BUTTON, t, cx)))
                                .child(div().flex_none().pl(px(2.0)).child(self.action_button(has_text, t, cx))),
                        ),
                )
        };
        let blur = if hero { t.composer_blur().max(22.0) } else { t.composer_blur() };
        let box_ = ui::frosted(radius, blur, box_);

        // Under the pill: extra folders on an open thread, and the meters.
        let meta = (!hero).then(|| {
            div()
                .w_full()
                .h(px(28.0))
                .px(px(4.0))
                .flex()
                .items_center()
                .gap(px(4.0))
                .when(!home && !self.add_dirs.is_empty(), |d| {
                    d.child(div().flex().min_w_0().overflow_hidden().child(self.render_context_row(t, cx)))
                })
                .child(div().flex_1())
                .child(self.rings(t, cx))
        });

        div()
            .id("composer")
            .w_full()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .capture_key_down(cx.listener(Self::on_key))
            .when(home && hero, |d| d.child(self.render_target_row(t, cx)))
            .when(home && !hero, |d| d.child(self.render_context_row(t, cx)))
            .child(box_)
            .when(home && hero && !self.add_dirs.is_empty(), |d| d.child(self.render_git_row(t, cx)))
            .children(meta)
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .id("composer-error")
                        .px(px(8.0))
                        .text_size(ty::META)
                        .text_color(t.danger)
                        .cursor_pointer()
                        .child(e)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.error = None;
                            cx.notify();
                        })),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sent_files_split_from_the_words() {
        let sent = format!("look at these\n\n{ATTACH_HEADER}\n- C:/a/shot.png\n- C:/b/Cargo.toml");
        let (body, files) = split_attachments(&sent);
        assert_eq!(body, "look at these");
        assert_eq!(files, vec![PathBuf::from("C:/a/shot.png"), PathBuf::from("C:/b/Cargo.toml")]);
        // Files alone, and plain text, both come back sensibly.
        let with_skills =
            format!("#pdf make it\n\n{SKILLS_HEADER}\n- pdf: /s/pdf/SKILL.md\n\n{ATTACH_HEADER}\n- /a.png");
        let (body, files) = split_attachments(&with_skills);
        assert_eq!(body, "#pdf make it");
        assert_eq!(files, vec![PathBuf::from("/a.png")]);
        let (body, _) = split_attachments(&format!("hi #x\n\n{SKILLS_HEADER}\n- x: /x"));
        assert_eq!(body, "hi #x");
        assert_eq!(skill_tags("use #pdf, #web-app-testing. and #pdf #1 #"), vec!["pdf", "web-app-testing", "1"]);
        let (body, files) = split_attachments(&format!("{ATTACH_HEADER}\n- /tmp/x.jpg"));
        assert!(body.is_empty());
        assert_eq!(files.len(), 1);
        assert_eq!(split_attachments("just words"), ("just words".to_string(), Vec::new()));
        assert!(is_image(std::path::Path::new("A.PNG")) && !is_image(std::path::Path::new("a.toml")));
    }

    #[test]
    fn send_button_states() {
        assert_eq!(SendState::of(false, true, true), SendState::Send);
        assert_eq!(SendState::of(true, false, true), SendState::Stop);
        assert_eq!(SendState::of(true, true, true), SendState::Steer);
        assert_eq!(SendState::of(true, true, false), SendState::Queue);
        const { assert!(PLATE_MIN_H < PLATE_MAX_H) };
    }

    #[test]
    fn mode_ids_round_trip() {
        for m in [PermissionMode::Default, PermissionMode::AcceptEdits, PermissionMode::Plan, PermissionMode::Bypass] {
            assert_eq!(mode_from_id(mode_id(m)), m);
        }
    }

    #[test]
    fn reset_times_read_naturally() {
        let now = dove_core::store::now_ms() / 1000;
        assert_eq!(until(now + 30), "now");
        assert_eq!(until(now + 600), "in 10m");
        assert!(until(now + 7800).starts_with("in 2h"));
    }

    #[test]
    fn home_input_stays_centered_unless_dock_is_chosen() {
        assert!(!home_input_is_dock(None));
        assert!(!home_input_is_dock(Some("center")));
        assert!(!home_input_is_dock(Some("")));
        assert!(home_input_is_dock(Some("dock")));
    }
}
