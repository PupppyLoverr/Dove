//! The composer: everything between the user and an agent.
//!
//! ```text
//!  [Local ▾] [▣ dove ▾] [⎇ main ▾] [☑ Worktree] [+ folder]          ◔ 38%  ◔ 6%
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

pub mod picker;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use dove_core::harness_detect::Installed;
use dove_core::proto::{HarnessKind, PermissionMode, ProjectId, RateWindow};
use dove_core::store::{Project, Task};
use dove_core::{Engine, RunSettings};
use gpui::StyledImage as _;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, App, AppContext as _, ClipboardEntry, Context, Entity, EventEmitter, ExternalPaths,
    FocusHandle, Focusable, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, anchored, deferred, div,
    img, px,
};
use gpui_base::input::{Input, InputEvent, InputState, Textarea, TextareaState};

use crate::assets::{icon, icons};
use crate::kit;
use crate::theme::Theme;
use crate::ui;
use crate::voice::{Dictation, DictationEvent};
pub use picker::{Picker, PickerEvent};

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
    Device,
    Project,
    Branch,
    Mode,
    Mic,
    Attach,
    Folders,
}

#[derive(Clone)]
struct MentionItem {
    label: String,
    detail: String,
    insert: String,
}

/// An open `@` (files) or `/` (commands) completion.
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

pub struct Composer {
    engine: Arc<Engine>,
    pub input: Entity<TextareaState>,
    pub picker: Entity<Picker>,
    pub voice: Entity<Dictation>,
    pub mode: PermissionMode,
    /// Set for a thread's composer.
    thread: Option<(Task, Project)>,
    pub project: Option<Project>,
    projects: Vec<Project>,
    branches: Vec<String>,
    pub branch: Option<String>,
    pub worktree: bool,
    pub add_dirs: Vec<PathBuf>,
    attachments: Vec<PathBuf>,
    running: bool,
    queue: Vec<String>,
    context: (Option<u64>, Option<u64>),
    limits: Vec<RateWindow>,
    menu: Option<Menu>,
    bypass_confirm: bool,
    mention: Option<Mention>,
    files: Arc<Vec<String>>,
    files_root: Option<PathBuf>,
    clone_url: Option<Entity<InputState>>,
    error: Option<SharedString>,
    draft_gen: u64,
    dragging_files: bool,
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
            DictationEvent::Changed => cx.notify(),
        }));
        let worktree = engine.store.setting("use_worktree").as_deref() != Some("false");
        let mode = engine.store.setting("permission_mode").map(|m| mode_from_id(&m)).unwrap_or_default();
        Self {
            engine,
            input,
            picker,
            voice,
            mode,
            thread: None,
            project: None,
            projects: Vec::new(),
            branches: Vec::new(),
            branch: None,
            worktree,
            add_dirs: Vec::new(),
            attachments: Vec::new(),
            running: false,
            queue: Vec::new(),
            context: (None, None),
            limits: Vec::new(),
            menu: None,
            bypass_confirm: false,
            mention: None,
            files: Arc::new(Vec::new()),
            files_root: None,
            clone_url: None,
            error: None,
            draft_gen: 0,
            dragging_files: false,
            _subs: subs,
        }
    }

    /// The new-thread composer.
    pub fn home(engine: Arc<Engine>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::build(engine, "Do anything…", (1, 10), window, cx);
        // Bypass is confirmed per project; a new thread starts from the
        // safe default unless the user picks it again.
        if this.mode == PermissionMode::Bypass {
            this.mode = PermissionMode::Default;
        }
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
        let mut this = Self::build(engine.clone(), "Send follow-up", (1, 10), window, cx);
        this.picker.update(cx, |p, _| p.lock_to(task.harness, task.model.clone()));
        this.add_dirs = engine.add_dirs(task.id);
        this.thread = Some((task.clone(), project.clone()));
        this.project = Some(project.clone());
        this.branch = task.branch.clone();
        if let Some(m) = engine.store.setting(&format!("mode.{}", task.id)) {
            this.mode = mode_from_id(&m);
        }
        this.restore_draft(window, cx);
        this
    }

    fn draft_key(&self) -> String {
        match &self.thread {
            Some((t, _)) => format!("draft.{}", t.id),
            None => "draft.home".into(),
        }
    }

    fn restore_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.draft_key();
        if let Some(d) = self.engine.store.setting(&key).filter(|d| !d.is_empty()) {
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
                let value = if files.is_empty() { text } else { format!("{text}\n\u{1}\n{files}") };
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
        let changed = pick.as_ref().map(|p| p.id) != self.project.as_ref().map(|p| p.id);
        self.project = pick;
        if changed || self.branches.is_empty() {
            self.load_branches(cx);
        }
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
    /// reply skips the cold start. Worktree threads get their folder only
    /// at send time, so they start cold.
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
        if self.worktree {
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

    /// The next queued message, sent once the running turn ends.
    pub fn take_queued(&mut self, cx: &mut Context<Self>) -> Option<String> {
        if self.queue.is_empty() {
            return None;
        }
        cx.notify();
        Some(self.queue.remove(0))
    }

    fn load_branches(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.project.as_ref().map(|p| p.path.clone()) else { return };
        let load = cx.background_spawn(async move {
            let branches = dove_core::git::branches(&path).unwrap_or_default();
            let current = dove_core::git::current_branch(&path).ok();
            (branches, current)
        });
        cx.spawn(async move |this, cx| {
            let (branches, current) = load.await;
            let _ = this.update(cx, |c, cx| {
                c.branches = branches;
                c.branch = current;
                cx.notify();
            });
        })
        .detach();
    }

    fn root(&self) -> Option<PathBuf> {
        match &self.thread {
            Some((t, p)) => Some(t.worktree.clone().unwrap_or_else(|| p.path.clone())),
            None => self.project.as_ref().map(|p| p.path.clone()),
        }
    }

    // ------------------------------------------------------------ actions

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut text = self.input.read(cx).value().trim().to_string();
        if text.is_empty() && self.attachments.is_empty() {
            return;
        }
        if !self.attachments.is_empty() {
            let list = self.attachments.iter().map(|p| format!("- {}", p.display())).collect::<Vec<_>>().join("\n");
            text = format!("{text}\n\nAttached files (read them as needed):\n{list}").trim().to_string();
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

    fn set_mode(&mut self, mode: PermissionMode, cx: &mut Context<Self>) {
        if mode == PermissionMode::Bypass && !self.bypass_allowed() {
            self.bypass_confirm = true;
            cx.notify();
            return;
        }
        self.mode = mode;
        self.bypass_confirm = false;
        self.menu = None;
        let key = match &self.thread {
            Some((t, _)) => format!("mode.{}", t.id),
            None => "permission_mode".into(),
        };
        let _ = self.engine.store.set_setting(&key, mode_id(mode));
        cx.notify();
    }

    fn bypass_key(&self) -> Option<String> {
        self.project.as_ref().map(|p| format!("bypass_ok.{}", p.id))
    }

    fn bypass_allowed(&self) -> bool {
        self.bypass_key().is_some_and(|k| self.engine.store.setting(&k).as_deref() == Some("1"))
    }

    fn confirm_bypass(&mut self, cx: &mut Context<Self>) {
        if let Some(k) = self.bypass_key() {
            let _ = self.engine.store.set_setting(&k, "1");
        }
        self.set_mode(PermissionMode::Bypass, cx);
    }

    fn toggle_menu(&mut self, m: Menu, cx: &mut Context<Self>) {
        self.menu = if self.menu == Some(m) { None } else { Some(m) };
        self.bypass_confirm = false;
        self.clone_url = None;
        cx.notify();
    }

    fn pick_project(&mut self, id: ProjectId, cx: &mut Context<Self>) {
        self.project = self.projects.iter().find(|p| p.id == id).cloned();
        self.menu = None;
        self.files = Arc::new(Vec::new());
        self.files_root = None;
        self.load_branches(cx);
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
                self.ensure_files(cx);
                let q = token[1..].to_ascii_lowercase();
                let items: Vec<MentionItem> = self
                    .files
                    .iter()
                    .filter(|f| q.is_empty() || f.to_ascii_lowercase().contains(&q))
                    .take(MAX_MENTIONS)
                    .map(|f| MentionItem {
                        label: f.rsplit('/').next().unwrap_or(f).to_string(),
                        detail: f.clone(),
                        insert: format!("@{f}"),
                    })
                    .collect();
                Some(Mention { trigger: '@', start, items, highlight: 0 })
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
                    })
                    .collect();
                items.extend(TEMPLATES.iter().map(|(name, detail, body)| MentionItem {
                    label: format!("/{name}"),
                    detail: detail.to_string(),
                    insert: body.to_string(),
                }));
                items.retain(|i| q.is_empty() || i.label[1..].to_ascii_lowercase().starts_with(&q));
                items.truncate(MAX_MENTIONS);
                Some(Mention { trigger: '/', start, items, highlight: 0 })
            }
            _ => None,
        };
        self.mention = mention.filter(|m| !m.items.is_empty() || m.trigger == '@');
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
        self.input.update(cx, |s, cx| {
            let cursor = s.cursor();
            s.set_selected_range(m.start..cursor, cx);
            s.replace(format!("{} ", item.insert), window, cx);
        });
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

    fn render_context_row(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mut row = div().h(px(28.0)).flex().items_center().gap(px(6.0));
        match &self.thread {
            None => {
                row = row
                    .child(self.device_chip(t, cx))
                    .child(self.project_chip(t, cx))
                    .child(self.branch_chip(t, cx))
                    .child(
                        kit::checkbox("composer-worktree", self.worktree, "Worktree", t)
                            .ml(px(6.0))
                            .tooltip(ui::tip("Run in an isolated git worktree on its own branch"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.worktree = !this.worktree;
                                let _ = this
                                    .engine
                                    .store
                                    .set_setting("use_worktree", if this.worktree { "true" } else { "false" });
                                cx.notify();
                            })),
                    )
                    .child(self.folders_chip(t, cx));
            }
            Some((task, _)) => {
                row = row
                    .when_some(task.branch.clone(), |d, b| {
                        d.child(
                            div()
                                .h(px(24.0))
                                .px(px(8.0))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .text_size(px(12.5))
                                .text_color(t.text_muted)
                                .child(icon(icons::GIT_BRANCH).size(px(13.0)))
                                .child(b),
                        )
                    })
                    .when(task.worktree.is_some(), |d| {
                        d.child(
                            div()
                                .h(px(24.0))
                                .px(px(8.0))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .text_size(px(12.5))
                                .text_color(t.text_faint)
                                .child(icon(icons::WORKTREE).size(px(13.0)))
                                .child("Worktree"),
                        )
                    })
                    .child(self.folders_chip(t, cx));
            }
        }
        row.into_any_element()
    }

    fn device_chip(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let chip = ui::chip("composer-device", t)
            .child(icon(icons::LAPTOP).size(px(14.0)))
            .child("Local")
            .child(icon(icons::ALT_ARROW_DOWN).size(px(11.0)).text_color(t.text_faint))
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
                        .text_size(px(14.0))
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
        let chip = ui::chip("composer-project", t)
            .child(icon(icons::FOLDER).size(px(14.0)))
            .child(div().max_w(px(180.0)).truncate().text_color(t.text).child(name))
            .child(icon(icons::ALT_ARROW_DOWN).size(px(11.0)).text_color(t.text_faint))
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
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(t.border_strong)
                        .text_size(px(13.0))
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

    fn branch_chip(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let label = self.branch.clone().unwrap_or_else(|| "branch".into());
        let chip = ui::chip("composer-branch", t)
            .child(icon(icons::GIT_BRANCH).size(px(14.0)))
            .child(div().max_w(px(160.0)).truncate().child(label))
            .child(icon(icons::ALT_ARROW_DOWN).size(px(11.0)).text_color(t.text_faint))
            .tooltip(ui::tip(if self.worktree { "Branch the worktree starts from" } else { "Branch to check out" }))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Branch, cx)));
        let menu = (self.menu == Some(Menu::Branch)).then(|| {
            let current = self.branch.clone();
            let rows = div()
                .id("branch-list")
                .w(px(280.0))
                .max_h(px(320.0))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .when(self.branches.is_empty(), |d| {
                    d.child(
                        div()
                            .px(px(14.0))
                            .py(px(10.0))
                            .text_size(px(13.0))
                            .text_color(t.text_faint)
                            .child("No branches (not a git repository?)"),
                    )
                })
                .children(self.branches.iter().map(|b| {
                    let name = b.clone();
                    ui::menu_row(SharedString::from(format!("br-{b}")), icons::GIT_BRANCH, b.clone(), false, t)
                        .when(current.as_deref() == Some(b.as_str()), |d| {
                            d.child(icon(icons::CHECK).size(px(14.0)).text_color(t.accent))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.branch = Some(name.clone());
                            this.menu = None;
                            cx.notify();
                        }))
                }));
            self.popover(Anchor::BottomLeft, ui::menu_card(t, rows), cx)
        });
        div().relative().child(chip).children(menu).into_any_element()
    }

    fn folders_chip(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let supported = dove_core::supports_add_dirs(self.harness(cx));
        let n = self.add_dirs.len();
        let chip = ui::chip("composer-folders", t)
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
                .rounded(px(6.0))
                .text_size(px(11.5))
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
                        this.bypass_confirm = false;
                        cx.notify();
                    }))
                    .child(content),
            ),
        )
        .with_priority(2)
        .into_any_element()
    }

    fn attach_button(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let button = ui::icon_button(
            "composer-attach",
            icons::PLUS,
            "Attach files, images or a folder (or paste / drop)",
            t,
            |_, _, _| {},
        )
        .rounded_full()
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
                        .text_size(px(12.0))
                        .text_color(t.text_faint)
                        .child("Tip: paste an image or drop files on the composer."),
                );
            self.popover(Anchor::BottomLeft, ui::menu_card(t, rows), cx)
        });
        div().relative().child(button).children(menu).into_any_element()
    }

    fn mic_button(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let voice = self.voice.read(cx);
        let recording = voice.is_recording();
        let busy = voice.is_transcribing();
        let ready = voice.is_ready();
        let voice_entity = self.voice.clone();
        let hold = voice_entity.clone();
        let mic = div()
            .id("composer-mic")
            .size(px(30.0))
            .justify_center()
            .flex()
            .items_center()
            .gap(px(2.0))
            .rounded_full()
            .cursor_pointer()
            .when(recording, |d| d.bg(t.danger.opacity(0.14)))
            .hover(|s| s.bg(t.glass_hover()))
            .tooltip(ui::tip(if !ready {
                "Dictation: install local voice in Settings › Voice"
            } else if recording {
                "Release to transcribe (or click to stop)"
            } else {
                "Hold to talk, or click to toggle"
            }))
            .child(if busy {
                kit::spinner("mic-busy", 14.0, t.text_muted).into_any_element()
            } else {
                icon(if recording {
                    icons::MICROPHONE
                } else if ready {
                    icons::MICROPHONE
                } else {
                    icons::MICROPHONE_OFF
                })
                .size(px(16.0))
                .text_color(if recording { t.danger } else { t.text_muted })
                .into_any_element()
            })
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                hold.update(cx, |v, cx| v.press(cx));
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                voice_entity.update(cx, |v, cx| v.release(cx));
            });
        let caret = div()
            .id("composer-mic-menu")
            .h(px(28.0))
            .w(px(14.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .cursor_pointer()
            .hover(|s| s.bg(t.glass_hover()))
            .child(icon(icons::ALT_ARROW_DOWN).size(px(10.0)).text_color(t.text_faint))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Mic, cx)));
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
                        .child(div().text_size(px(13.0)).text_color(t.text).child("Dictation"))
                        .child(div().text_size(px(12.0)).text_color(t.text_faint).child(v.status_line())),
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
            .group("mic")
            .child(mic)
            .child(div().invisible().group_hover("mic", |s| s.visible()).child(caret))
            .children(menu)
            .into_any_element()
    }

    fn mode_button(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let bypass = self.mode == PermissionMode::Bypass;
        let chip = div()
            .id("composer-mode")
            .h(px(28.0))
            .px(px(9.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .rounded_full()
            .cursor_pointer()
            .text_size(px(12.5))
            .text_color(if bypass { t.danger } else { t.text_muted })
            .when(bypass, |d| d.bg(t.danger.opacity(0.12)))
            .hover(|s| s.bg(t.wash(0.08)))
            .child(icon(mode_icon(self.mode)).size(px(14.0)).text_color(if bypass { t.danger } else { t.text_faint }))
            .child(mode_label(self.mode))
            .tooltip(ui::tip("Permission level"))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Mode, cx)));
        let menu = (self.menu == Some(Menu::Mode)).then(|| {
            let modes = [
                (PermissionMode::Default, "Ask every time", "Approve edits and commands"),
                (PermissionMode::AcceptEdits, "Auto-edit", "Edits apply; commands ask"),
                (PermissionMode::Plan, "Plan", "Read-only until you approve a plan"),
                (PermissionMode::Bypass, "Bypass permissions", "Never ask. Best in a worktree"),
            ];
            let content: AnyElement = if self.bypass_confirm {
                let project = self.project.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "this project".into());
                div()
                    .w(px(300.0))
                    .p(px(14.0))
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(icon(icons::DANGER_TRIANGLE).text_color(t.danger))
                            .child(div().font_weight(gpui::FontWeight::MEDIUM).child("Bypass permissions?")),
                    )
                    .child(div().text_size(px(13.0)).text_color(t.text_muted).child(format!(
                        "In {project}, agents will run any command and edit any file without asking. Dove asks once per project."
                    )))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .child(ui::button_primary("bypass-yes", "Enable bypass", t).bg(t.danger).on_click(cx.listener(|this, _, _, cx| this.confirm_bypass(cx))))
                            .child(ui::button_ghost("bypass-no", "Cancel", t).on_click(cx.listener(|this, _, _, cx| {
                                this.bypass_confirm = false;
                                cx.notify();
                            }))),
                    )
                    .into_any_element()
            } else {
                div()
                    .w(px(280.0))
                    .flex()
                    .flex_col()
                    .children(modes.into_iter().map(|(m, label, desc)| {
                        let selected = m == self.mode;
                        let danger = m == PermissionMode::Bypass;
                        div()
                            .id(SharedString::from(format!("mode-{}", mode_id(m))))
                            .mx(px(4.0))
                            .px(px(10.0))
                            .py(px(6.0))
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .rounded(px(7.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.wash(0.10)))
                            .child(icon(mode_icon(m)).size(px(16.0)).text_color(if danger { t.danger } else { t.text_muted }))
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .child(div().text_size(px(14.0)).text_color(if danger { t.danger } else { t.text }).child(label))
                                    .child(div().text_size(px(12.0)).text_color(t.text_faint).child(desc)),
                            )
                            .when(selected, |d| d.child(icon(icons::CHECK).size(px(14.0)).text_color(t.accent)))
                            .on_click(cx.listener(move |this, _, _, cx| this.set_mode(m, cx)))
                    }))
                    .into_any_element()
            };
            self.popover(Anchor::BottomLeft, ui::menu_card(t, content), cx)
        });
        div().relative().child(chip).children(menu).into_any_element()
    }

    fn action_button(&self, has_text: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let steers = dove_core::steers(self.harness(cx));
        if self.running && !has_text {
            return div()
                .id("composer-stop")
                .size(px(32.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(t.solid)
                .cursor_pointer()
                .tooltip(ui::tip("Stop (Esc)"))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposerEvent::Stop)))
                .child(div().size(px(10.0)).rounded(px(2.0)).bg(t.on_solid))
                .into_any_element();
        }
        let (glyph, tip, label) = match (self.running, steers) {
            (true, true) => (icons::ARROW_UP, "Steer: send now, the agent adjusts mid-turn (Enter)", Some("Steer")),
            (true, false) => (icons::QUEUE_SEND, "Queue: send when this turn ends (Enter)", Some("Queue")),
            _ => (icons::ARROW_UP, "Send (Enter)", None),
        };
        let enabled = has_text || !self.attachments.is_empty();
        div()
            .id("composer-send")
            .h(px(32.0))
            .min_w(px(32.0))
            .when(label.is_some(), |d| d.px(px(10.0)))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(4.0))
            .rounded_full()
            .bg(if enabled { t.solid } else { t.wash(0.08) })
            .when(enabled, |d| d.hover(|s| s.opacity(0.88)))
            .text_color(if enabled { t.on_solid } else { t.text_faint })
            .text_size(px(12.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .when(enabled, |d| d.cursor_pointer())
            .tooltip(ui::tip(tip))
            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)))
            .child(icon(glyph).size(px(15.0)).text_color(if enabled { t.on_solid } else { t.text_faint }))
            .when_some(label, |d, l| d.child(l))
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
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.wash(0.04))
                        .text_size(px(12.5))
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

    fn render_attachments(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.attachments.is_empty() {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_wrap()
                .gap(px(6.0))
                .children(self.attachments.iter().enumerate().map(|(i, p)| {
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let is_image = matches!(
                        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
                        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
                    );
                    div()
                        .id(SharedString::from(format!("att-{i}")))
                        .h(px(30.0))
                        .pl(px(4.0))
                        .pr(px(8.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(t.border)
                        .bg(t.wash(0.04))
                        .text_size(px(12.5))
                        .text_color(t.text_muted)
                        .tooltip(ui::tip(p.display().to_string()))
                        .child(if is_image {
                            img(p.clone())
                                .size(px(22.0))
                                .rounded(px(4.0))
                                .object_fit(gpui::ObjectFit::Cover)
                                .into_any_element()
                        } else {
                            div()
                                .size(px(22.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon(icons::PAPERCLIP).size(px(13.0)))
                                .into_any_element()
                        })
                        .child(div().max_w(px(180.0)).truncate().child(name))
                        .child(
                            div()
                                .id(SharedString::from(format!("att-x-{i}")))
                                .cursor_pointer()
                                .child(icon(icons::CLOSE).size(px(11.0)).text_color(t.text_faint))
                                .on_click(cx.listener(move |this, _, _, cx| this.remove_attachment(i, cx))),
                        )
                }))
                .into_any_element(),
        )
    }

    fn render_mention(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let m = self.mention.as_ref()?;
        let empty = m.items.is_empty();
        let list = div()
            .w(px(420.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(14.0))
                    .pt(px(6.0))
                    .pb(px(4.0))
                    .text_size(px(11.0))
                    .text_color(t.text_faint)
                    .child(if m.trigger == '@' { "Files" } else { "Commands" }),
            )
            .when(empty, |d| {
                d.child(
                    div()
                        .px(px(14.0))
                        .py(px(8.0))
                        .text_size(px(13.0))
                        .text_color(t.text_faint)
                        .child(if self.files.is_empty() { "Loading files…" } else { "No matching files" }),
                )
            })
            .children(m.items.iter().enumerate().map(|(i, item)| {
                let selected = i == m.highlight;
                div()
                    .id(SharedString::from(format!("mention-{i}")))
                    .mx(px(4.0))
                    .px(px(10.0))
                    .h(px(32.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .rounded(px(7.0))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(t.wash(0.10)))
                    .hover(|s| s.bg(t.wash(0.08)))
                    .child(
                        icon(if m.trigger == '@' { icons::DOCUMENT } else { icons::COMMAND })
                            .size(px(14.0))
                            .text_color(t.text_faint),
                    )
                    .child(div().flex_none().text_size(px(13.0)).text_color(t.text).child(item.label.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.0))
                            .text_color(t.text_faint)
                            .child(item.detail.clone()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(m) = this.mention.as_mut() {
                            m.highlight = i;
                        }
                        this.accept_mention(window, cx);
                    }))
            }));
        Some(self.popover(Anchor::BottomLeft, ui::menu_card(t, list), cx))
    }
}

#[derive(Clone)]
struct QueueDrag(usize);

impl Render for QueueDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(cx);
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(6.0))
            .bg(t.surface_raised)
            .text_size(px(12.0))
            .child(format!("Message {}", self.0 + 1))
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

pub fn mode_from_id(id: &str) -> PermissionMode {
    match id {
        "accept_edits" => PermissionMode::AcceptEdits,
        "plan" => PermissionMode::Plan,
        "bypass" => PermissionMode::Bypass,
        _ => PermissionMode::Default,
    }
}

pub fn mode_label(m: PermissionMode) -> &'static str {
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

/// Focus a composer's input. Takes the entity so callers holding `cx`
/// mutably need no outstanding read borrow.
pub fn focus(composer: &Entity<Composer>, window: &mut Window, cx: &mut App) {
    let input = composer.read(cx).input.clone();
    focus_input(&input, window, cx);
}

pub fn set_text(composer: &Entity<Composer>, text: &str, window: &mut Window, cx: &mut App) {
    let input = composer.read(cx).input.clone();
    let text = text.to_string();
    input.update(cx, |s, cx| s.set_value(text, window, cx));
}

pub fn is_empty(composer: &Entity<Composer>, cx: &App) -> bool {
    composer.read(cx).input.read(cx).value().trim().is_empty()
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let t = &theme;
        let has_text = !self.input.read(cx).value().trim().is_empty();
        let bypass = self.mode == PermissionMode::Bypass;
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let home = self.thread.is_none();

        // One rounded surface: what you type on top, how it runs underneath
        // (attach, permission, model, voice, send), like the best chat boxes.
        let border = if bypass {
            t.danger.opacity(0.55)
        } else if self.dragging_files {
            t.accent
        } else if focused {
            t.wash(0.22)
        } else {
            t.wash(0.10)
        };
        let box_ = div()
            .id("composer-pill")
            .w_full()
            .rounded(px(20.0))
            .border_1()
            .border_color(border)
            .bg(t.surface_raised)
            .when(focused, |d| d.shadow_lg())
            .pt(px(10.0))
            .pb(px(8.0))
            .px(px(10.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .text_size(px(15.0))
            .cursor_text()
            .on_click(cx.listener(|this, _, window, cx| focus_input(&this.input, window, cx)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.dragging_files = false;
                this.attach(paths.paths().to_vec(), cx);
            }))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(gpui::transparent_black()))
            .children(self.render_queue(t, cx))
            .children(self.render_attachments(t, cx))
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .relative()
                    .px(px(6.0))
                    .child(Textarea::new(&self.input))
                    .children(self.render_mention(t, cx)),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .h(px(32.0))
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .child(self.attach_button(t, cx))
                    .child(self.mode_button(t, cx))
                    .child(div().flex_1().min_w(px(4.0)))
                    .child(div().min_w_0().overflow_hidden().child(self.picker.clone()))
                    .child(self.mic_button(t, cx))
                    .child(div().pl(px(4.0)).child(self.action_button(has_text, t, cx))),
            );

        // Under the box: where it runs (in a thread) and the meters.
        let meta = div()
            .w_full()
            .h(px(24.0))
            .px(px(6.0))
            .flex()
            .items_center()
            .gap(px(4.0))
            .when(!home, |d| d.child(div().flex().min_w_0().overflow_hidden().child(self.render_context_row(t, cx))))
            .child(div().flex_1())
            .child(self.rings(t, cx));

        div()
            .id("composer")
            .w_full()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .capture_key_down(cx.listener(Self::on_key))
            .when(home, |d| d.child(self.render_context_row(t, cx)))
            .child(box_)
            .child(meta)
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .id("composer-error")
                        .px(px(8.0))
                        .text_size(px(12.0))
                        .text_color(t.danger)
                        .cursor_pointer()
                        .child(e)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.error = None;
                            cx.notify();
                        })),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
