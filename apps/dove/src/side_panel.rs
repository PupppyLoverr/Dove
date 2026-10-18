//! The thread's side panel, on Zeron's model: a strip of surface tabs in the
//! titlebar band over the panel (Diffs · History · Files · Plan · Browser ·
//! Terminal, any number of browsers and terminals), a `+` menu, an expand
//! ("takeover") toggle that gives the panel the conversation's width, and an
//! empty state that lists the surfaces to open.
//!
//! Every thread keeps its own tabs; browsers and terminals live as long as
//! their thread (scrollback and page state survive switching away) and are
//! closed with it.

use std::collections::HashMap;
use std::path::PathBuf;

use dove_core::Engine;
use dove_core::proto::{TaskId, TaskStatus, TodoItem, TodoStatus};
use dove_core::store::{Project, Task};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, anchored, deferred, div, px,
};
use std::sync::Arc;

use crate::assets::{icon, icons};
use crate::browser::{BrowserContext, BrowserEvent, BrowserSurface};
use crate::changes::{Changes, ChangesEvent};
use crate::design::{radius, ty};
use crate::files_panel::{FilesEvent, FilesPanel};
use crate::history::{GitHistory, HistoryEvent};
use crate::kit;
use crate::side_chat::{SideChat, SideChatEvent};
use crate::surface_chrome;
use crate::terminal::TerminalView;
use crate::theme::{Theme, ink};
use crate::transcript::{Subagent, TaskView};
use crate::ui;

/// Fixed chip width, as in Zeron, so drag reordering is uniform.
const CHIP_W: f32 = 112.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Surface {
    Changes,
    History,
    Files,
    Plan,
    Browser(u64),
    Terminal(u64),
    SideChat(u64),
    Subagents,
    /// One subagent's own chat, read-only.
    Subagent(u64),
    Chats,
}

/// The kinds the `+` menu and the empty state offer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Changes,
    History,
    Files,
    Plan,
    Browser,
    Terminal,
    SideChat,
    Subagents,
    Chats,
}

impl Tab {
    pub const ALL: [Tab; 9] = [
        Tab::Browser,
        Tab::Terminal,
        Tab::Changes,
        Tab::Files,
        Tab::History,
        Tab::Plan,
        Tab::Subagents,
        Tab::Chats,
        Tab::SideChat,
    ];

    /// A tab by its short name (`DOVE_PANEL`).
    pub fn from_id(id: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.label().to_ascii_lowercase().replace(' ', "-") == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Tab::Changes => "Diffs",
            Tab::History => "History",
            Tab::Files => "Files",
            Tab::Plan => "Plan",
            Tab::Browser => "Browser",
            Tab::Terminal => "Terminal",
            Tab::SideChat => "Side chat",
            Tab::Subagents => "Subagents",
            Tab::Chats => "Chats",
        }
    }

    /// What the surface is for (picker cards and tooltips).
    pub fn blurb(self) -> &'static str {
        match self {
            Tab::Browser => "Preview the app or any page",
            Tab::Terminal => "A shell in this checkout",
            Tab::Changes => "What changed, file by file",
            Tab::Files => "Browse and edit the project",
            Tab::History => "Commits on this branch",
            Tab::Plan => "The agent's to-do list",
            Tab::Subagents => "Work handed off by this thread",
            Tab::Chats => "Side conversations in this thread",
            Tab::SideChat => "Ask something without derailing",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Tab::Changes => icons::LIST,
            Tab::History => icons::GIT_BRANCH,
            Tab::Files => icons::FOLDER_TREE,
            Tab::Plan => icons::CHECKLIST,
            Tab::Browser => icons::GLOBE,
            Tab::Terminal => icons::TERMINAL,
            Tab::SideChat => icons::CHAT_ROUND_LINE,
            Tab::Subagents => icons::FORK,
            Tab::Chats => icons::BOT,
        }
    }
}

impl Surface {
    fn kind(self) -> Tab {
        match self {
            Surface::Changes => Tab::Changes,
            Surface::History => Tab::History,
            Surface::Files => Tab::Files,
            Surface::Plan => Tab::Plan,
            Surface::Browser(_) => Tab::Browser,
            Surface::Terminal(_) => Tab::Terminal,
            Surface::SideChat(_) => Tab::SideChat,
            Surface::Subagents | Surface::Subagent(_) => Tab::Subagents,
            Surface::Chats => Tab::Chats,
        }
    }
}

pub enum SidePanelEvent {
    SendToAgent(String),
    /// The panel's own close button.
    Close,
    /// Expand / collapse toggled.
    Layout,
    /// Scroll the conversation to a transcript row (a subagent's call).
    RevealRow(usize),
    /// Open another thread (a side chat).
    OpenThread(TaskId),
}

/// A side chat sharing this thread's branch.
#[derive(Clone, PartialEq)]
pub struct SideChatInfo {
    pub id: TaskId,
    pub title: String,
    pub status: TaskStatus,
    pub unread: bool,
    pub updated_at: i64,
}

#[derive(Default)]
struct ThreadPanel {
    tabs: Vec<Surface>,
    active: Option<Surface>,
    browsers: HashMap<u64, (Entity<BrowserSurface>, Subscription)>,
    terminals: HashMap<u64, Entity<TerminalView>>,
    chats: HashMap<u64, (Entity<SideChat>, Subscription)>,
    subagents: HashMap<u64, (Entity<TaskView>, Subscription)>,
}

#[derive(Clone)]
struct ChipDrag {
    from: usize,
    title: SharedString,
}

impl Render for ChipDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("ChipDrag");
        let t = Theme::of(cx);
        div()
            .h(px(24.0))
            .w(px(CHIP_W))
            .px(px(8.0))
            .flex()
            .items_center()
            .rounded(px(radius::RADIUS))
            .bg(t.surface_raised)
            .border_1()
            .border_color(t.border_strong)
            .shadow_md()
            .text_size(ty::META)
            .child(self.title.clone())
    }
}

pub struct SidePanel {
    pub changes: Entity<Changes>,
    pub files: Entity<FilesPanel>,
    git_history: Entity<GitHistory>,
    engine: Arc<Engine>,
    parent: Option<(Task, Project)>,
    thread: Option<TaskId>,
    dir: Option<PathBuf>,
    panels: HashMap<TaskId, ThreadPanel>,
    todos: Vec<TodoItem>,
    subagents: Vec<Subagent>,
    side_chats: Vec<SideChatInfo>,
    /// The panel has the conversation's width.
    pub expanded: bool,
    plus_open: bool,
    /// The highlighted card of the `+` surface picker, and its focus.
    plus_hi: Option<usize>,
    plus_focus: FocusHandle,
    next_id: u64,
    browser_context: BrowserContext,
    /// Whether the shell is showing the panel (drives native browser views).
    visible: bool,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl EventEmitter<SidePanelEvent> for SidePanel {}

impl SidePanel {
    pub fn new(engine: Arc<Engine>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let changes = cx.new(|cx| Changes::new(window, cx));
        let files = cx.new(|cx| FilesPanel::new(window, cx));
        let git_history = cx.new(|cx| GitHistory::new(window, cx));
        let mut subs = vec![
            cx.subscribe(&changes, |_, _, ev: &ChangesEvent, cx| match ev {
                ChangesEvent::SendToAgent(text) => cx.emit(SidePanelEvent::SendToAgent(text.clone())),
            }),
            cx.subscribe(&files, |_, _, ev: &FilesEvent, cx| match ev {
                FilesEvent::AskAgent(text) => cx.emit(SidePanelEvent::SendToAgent(text.clone())),
            }),
        ];
        subs.push(cx.subscribe_in(&git_history, window, |this, _, ev: &HistoryEvent, window, cx| match ev {
            HistoryEvent::OpenCommit { sha, subject } => {
                let Some(dir) = this.dir.clone() else { return };
                this.changes.update(cx, |c, cx| c.show_commit(dir, sha.clone(), subject, window, cx));
                this.show(Tab::Changes, window, cx);
            }
        }));
        Self {
            changes,
            files,
            git_history,
            engine,
            parent: None,
            thread: None,
            dir: None,
            panels: HashMap::new(),
            todos: Vec::new(),
            subagents: Vec::new(),
            side_chats: Vec::new(),
            expanded: false,
            plus_open: false,
            plus_hi: None,
            plus_focus: cx.focus_handle(),
            next_id: 1,
            browser_context: BrowserContext::default(),
            visible: false,
            focus: cx.focus_handle(),
            _subs: subs,
        }
    }

    fn panel(&mut self) -> Option<&mut ThreadPanel> {
        let id = self.thread?;
        Some(self.panels.entry(id).or_default())
    }

    fn panel_ref(&self) -> Option<&ThreadPanel> {
        self.panels.get(&self.thread?)
    }

    pub fn active(&self) -> Option<Surface> {
        self.panel_ref().and_then(|p| p.active)
    }

    /// Point the panel at a thread.
    pub fn set_thread(
        &mut self,
        thread: Option<TaskId>,
        dir: Option<PathBuf>,
        base: Option<String>,
        title: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.thread = thread;
        self.dir = dir.clone();
        self.plus_open = false;
        let _ = self.panel();
        self.changes.update(cx, |c, cx| c.set_target(dir.clone(), base, title, window, cx));
        self.files.update(cx, |f, cx| f.set_root(dir.clone(), cx));
        self.git_history.update(cx, |h, cx| h.set_root(dir, cx));
        cx.notify();
    }

    pub fn set_parent(&mut self, task: Task, project: Project) {
        self.parent = Some((task, project));
    }

    pub fn set_todos(&mut self, todos: Vec<TodoItem>, cx: &mut Context<Self>) {
        if self.todos != todos {
            self.todos = todos;
            cx.notify();
        }
    }

    pub fn set_subagents(&mut self, subagents: Vec<Subagent>, cx: &mut Context<Self>) {
        if self.subagents != subagents {
            self.subagents = subagents;
            cx.notify();
        }
    }

    pub fn set_side_chats(&mut self, chats: Vec<SideChatInfo>, cx: &mut Context<Self>) {
        if self.side_chats != chats {
            self.side_chats = chats;
            cx.notify();
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.changes.update(cx, |c, cx| c.refresh(cx));
        self.files.update(cx, |f, cx| f.refresh(cx));
        self.git_history.update(cx, |h, cx| h.refresh(cx));
    }

    /// Show a surface of `kind`, opening one if the thread has none.
    pub fn show(&mut self, kind: Tab, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self
            .panel_ref()
            .and_then(|p| p.tabs.iter().copied().find(|s| s.kind() == kind && !matches!(s, Surface::Subagent(_))));
        match existing {
            Some(s) => self.activate(s, window, cx),
            None => self.open(kind, None, window, cx),
        }
    }

    /// Open a new surface of `kind` (browsers and terminals can repeat).
    pub fn open(&mut self, kind: Tab, url: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.plus_open = false;
        let surface = match kind {
            Tab::Changes => Surface::Changes,
            Tab::History => Surface::History,
            Tab::Files => Surface::Files,
            Tab::Plan => Surface::Plan,
            Tab::Browser => {
                let id = self.next_id;
                self.next_id += 1;
                let ctx = self.browser_context.clone();
                let browser = cx.new(|cx| BrowserSurface::new(ctx, false, window, cx));
                let sub = cx.subscribe_in(&browser, window, move |this, _, ev: &BrowserEvent, window, cx| match ev {
                    BrowserEvent::Changed => cx.notify(),
                    BrowserEvent::NewTab(url) => this.open(Tab::Browser, url.clone(), window, cx),
                    BrowserEvent::Close => this.close(Surface::Browser(id), window, cx),
                });
                browser.update(cx, |b, cx| match &url {
                    Some(u) => b.navigate(u, window, cx),
                    None => b.focus_address(window, cx),
                });
                if let Some(p) = self.panel() {
                    p.browsers.insert(id, (browser, sub));
                }
                Surface::Browser(id)
            }
            Tab::Terminal => {
                let Some(dir) = self.dir.clone() else { return };
                let id = self.next_id;
                self.next_id += 1;
                let term = cx.new(|cx| TerminalView::new(dir, cx));
                let focus = term.read(cx).focus_handle(cx);
                if let Some(p) = self.panel() {
                    p.terminals.insert(id, term);
                }
                window.focus(&focus, cx);
                Surface::Terminal(id)
            }
            Tab::Subagents => Surface::Subagents,
            Tab::Chats => Surface::Chats,
            Tab::SideChat => {
                let Some((task, project)) = self.parent.clone() else { return };
                let id = self.next_id;
                self.next_id += 1;
                let engine = self.engine.clone();
                let chat = cx.new(|cx| SideChat::new(engine, task, project, window, cx));
                if let Some(seed) = url.filter(|s| !s.trim().is_empty()) {
                    chat.update(cx, |c, cx| c.submit(seed, window, cx));
                }
                let sub = cx.subscribe(&chat, |_, _, ev: &SideChatEvent, cx| match ev {
                    SideChatEvent::Started(_) | SideChatEvent::Changed => {
                        cx.emit(SidePanelEvent::Layout);
                        cx.notify();
                    }
                    SideChatEvent::OpenThread(tid) => {
                        cx.emit(SidePanelEvent::OpenThread(*tid));
                    }
                });
                if let Some(p) = self.panel() {
                    p.chats.insert(id, (chat, sub));
                }
                Surface::SideChat(id)
            }
        };
        if let Some(p) = self.panel()
            && !p.tabs.contains(&surface)
        {
            p.tabs.push(surface);
        }
        self.activate(surface, window, cx);
    }

    fn activate(&mut self, surface: Surface, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.panel() {
            p.active = Some(surface);
        }
        match surface {
            Surface::Changes => self.changes.update(cx, |c, cx| c.refresh(cx)),
            Surface::History => self.git_history.update(cx, |h, cx| h.refresh(cx)),
            Surface::Terminal(id) => {
                if let Some(t) = self.panel_ref().and_then(|p| p.terminals.get(&id)).cloned() {
                    let f = t.read(cx).focus_handle(cx);
                    window.focus(&f, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    pub fn close(&mut self, surface: Surface, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.panel() else { return };
        let Some(ix) = p.tabs.iter().position(|s| *s == surface) else { return };
        p.tabs.remove(ix);
        match surface {
            Surface::Browser(id) => {
                if let Some((b, _)) = p.browsers.remove(&id) {
                    b.update(cx, |b, cx| b.close(cx));
                }
            }
            Surface::Terminal(id) => {
                if let Some(t) = p.terminals.remove(&id) {
                    t.update(cx, |t, cx| t.kill(cx));
                }
            }
            Surface::SideChat(id) => {
                p.chats.remove(&id);
            }
            Surface::Subagent(id) => {
                p.subagents.remove(&id);
            }
            _ => {}
        }
        if p.active == Some(surface) {
            p.active = p.tabs.get(ix).or_else(|| p.tabs.get(ix.saturating_sub(1))).copied();
        }
        cx.notify();
    }

    fn reorder(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if let Some(p) = self.panel()
            && from < p.tabs.len()
            && to < p.tabs.len()
            && from != to
        {
            let s = p.tabs.remove(from);
            p.tabs.insert(to, s);
        }
        cx.notify();
    }

    pub fn new_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open(Tab::Terminal, None, window, cx);
    }

    pub fn open_path(&mut self, path: &str, line: Option<u32>, window: &mut Window, cx: &mut Context<Self>) {
        self.show(Tab::Files, window, cx);
        self.files.update(cx, |f, cx| f.open_path(path, line, cx));
    }

    /// Show the chat of the subagent spawned by `call` in `source` (the
    /// thread's transcript): its tab if open, else a new one.
    pub fn open_subagent(
        &mut self,
        source: &Entity<TaskView>,
        call: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = self.panel_ref().and_then(|p| {
            p.subagents
                .iter()
                .find(|(_, (v, _))| v.read(cx).mirrored().is_some_and(|(c, _)| c == call))
                .map(|(id, _)| *id)
        });
        let id = match existing {
            Some(id) => id,
            None => {
                let id = self.next_id;
                self.next_id += 1;
                let view = cx.new(|cx| TaskView::subagent(source, call, window, cx));
                let sub = cx.observe(&view, |_, _, cx| cx.notify());
                let Some(p) = self.panel() else { return };
                p.subagents.insert(id, (view, sub));
                p.tabs.push(Surface::Subagent(id));
                id
            }
        };
        self.activate(Surface::Subagent(id), window, cx);
    }

    fn subagent_view(&self, id: u64) -> Option<&Entity<TaskView>> {
        self.panel_ref().and_then(|p| p.subagents.get(&id)).map(|(v, _)| v)
    }

    pub fn toggle_expanded(&mut self, cx: &mut Context<Self>) {
        self.expanded = !self.expanded;
        cx.emit(SidePanelEvent::Layout);
        cx.notify();
    }

    /// Close a deleted thread's browsers and shells.
    pub fn forget(&mut self, thread: TaskId, cx: &mut Context<Self>) {
        if let Some(p) = self.panels.remove(&thread) {
            for (_, (b, _)) in p.browsers {
                b.update(cx, |b, cx| b.close(cx));
            }
            for (_, t) in p.terminals {
                t.update(cx, |t, cx| t.kill(cx));
            }
            let _ = p.chats;
        }
    }

    /// Drop the native web views of thread `id`'s browser tabs (its tab is
    /// closed, so all are hidden). Each reloads its URL when shown again.
    pub fn hibernate_thread(&mut self, id: TaskId, cx: &mut Context<Self>) {
        let Some(p) = self.panels.get(&id) else { return };
        for (b, _) in p.browsers.values() {
            b.update(cx, |b, cx| b.hibernate(true, cx));
        }
    }

    /// The memory watchdog: hibernate every web view not on screen, in any
    /// thread.
    pub fn hibernate_hidden(&mut self, cx: &mut Context<Self>) {
        for p in self.panels.values() {
            for (b, _) in p.browsers.values() {
                if b.read(cx).presentation() == crate::browser::model::Presentation::Hidden {
                    b.update(cx, |b, cx| b.hibernate(true, cx));
                }
            }
        }
    }

    /// Native browser views follow what is on screen: only the active
    /// browser of the visible panel is live. Call every shell frame.
    pub fn sync(&mut self, visible: bool, dragging: bool, cx: &mut Context<Self>) {
        self.visible = visible;
        let active = self.active();
        let urls = self.detected_urls(cx);
        let current = self.thread;
        let combos: Vec<String> = crate::keymap::shortcuts().iter().map(|s| s.default.to_string()).collect();
        for (thread, p) in &self.panels {
            for (id, (b, _)) in &p.browsers {
                let live = visible && Some(*thread) == current && active == Some(Surface::Browser(*id));
                let presentation = crate::browser::model::presentation(live, dragging);
                let urls = urls.clone();
                let combos = combos.clone();
                b.update(cx, |b, cx| {
                    b.set_presentation(presentation, cx);
                    if live {
                        b.set_previews(urls, cx);
                        b.set_shortcuts(combos);
                    }
                });
            }
        }
    }

    /// The browser an agent in `thread` drives: that thread's active (or
    /// most recent) browser tab, opened quietly if it has none. `reveal`
    /// brings the tab to the front of the panel.
    pub fn agent_browser(
        &mut self,
        thread: TaskId,
        reveal: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<BrowserSurface> {
        let existing = self.panels.get(&thread).and_then(|p| {
            let pick = match p.active {
                Some(Surface::Browser(id)) => Some(id),
                _ => p.tabs.iter().rev().find_map(|s| match s {
                    Surface::Browser(id) => Some(*id),
                    _ => None,
                }),
            };
            pick.and_then(|id| p.browsers.get(&id).map(|(b, _)| (id, b.clone())))
        });
        let (id, browser) = match existing {
            Some(found) => found,
            None => self.new_browser(thread, window, cx),
        };
        // An agent drives a hibernated tab as it was: reload its page first.
        browser.update(cx, |b, cx| b.wake(window, cx));
        if reveal {
            if let Some(p) = self.panels.get_mut(&thread) {
                p.active = Some(Surface::Browser(id));
            }
            cx.notify();
        }
        browser
    }

    fn new_browser(
        &mut self,
        thread: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (u64, Entity<BrowserSurface>) {
        let id = self.next_id;
        self.next_id += 1;
        let ctx = self.browser_context.clone();
        let browser = cx.new(|cx| BrowserSurface::new(ctx, false, window, cx));
        let sub = cx.subscribe_in(&browser, window, move |this, _, ev: &BrowserEvent, window, cx| match ev {
            BrowserEvent::Changed => cx.notify(),
            BrowserEvent::NewTab(url) => this.open(Tab::Browser, url.clone(), window, cx),
            BrowserEvent::Close => this.close(Surface::Browser(id), window, cx),
        });
        let p = self.panels.entry(thread).or_default();
        p.browsers.insert(id, (browser.clone(), sub));
        p.tabs.push(Surface::Browser(id));
        p.active = Some(Surface::Browser(id));
        cx.notify();
        (id, browser)
    }

    /// `browser_tabs` for an agent in `thread`.
    pub fn agent_tabs(
        &mut self,
        thread: TaskId,
        action: &str,
        index: Option<usize>,
        url: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        let list = |this: &Self, cx: &gpui::App| -> String {
            let Some(p) = this.panels.get(&thread) else { return "No browser tabs.".into() };
            let mut out = Vec::new();
            for (i, s) in p.tabs.iter().filter(|s| matches!(s, Surface::Browser(_))).enumerate() {
                let Surface::Browser(id) = s else { continue };
                let Some((b, _)) = p.browsers.get(id) else { continue };
                let b = b.read(cx);
                out.push(format!(
                    "{i}: {}{} — {}",
                    if p.active == Some(*s) { "* " } else { "" },
                    b.title(),
                    b.page.url.clone().unwrap_or_else(|| "(blank)".into())
                ));
            }
            if out.is_empty() {
                "No browser tabs.".into()
            } else {
                out.join(
                    "
",
                )
            }
        };
        let browser_ids: Vec<u64> = self
            .panels
            .get(&thread)
            .map(|p| {
                p.tabs.iter().filter_map(|s| if let Surface::Browser(id) = s { Some(*id) } else { None }).collect()
            })
            .unwrap_or_default();
        match action {
            "list" => Ok(list(self, cx)),
            "new" => {
                let (_, b) = self.new_browser(thread, window, cx);
                if let Some(u) = url.filter(|u| !u.trim().is_empty()) {
                    b.update(cx, |b, cx| b.navigate_quiet(&u, window, cx));
                }
                Ok(list(self, cx))
            }
            "select" | "close" => {
                let i = index.ok_or("Give the tab index")?;
                let id = *browser_ids.get(i).ok_or_else(|| format!("No browser tab {i}"))?;
                if action == "select" {
                    if let Some(p) = self.panels.get_mut(&thread) {
                        p.active = Some(Surface::Browser(id));
                    }
                    cx.notify();
                } else {
                    let saved = self.thread;
                    self.thread = Some(thread);
                    self.close(Surface::Browser(id), window, cx);
                    self.thread = saved;
                }
                Ok(list(self, cx))
            }
            other => Err(format!("Unknown action {other}; use list, new, select or close")),
        }
    }

    /// Drive the visible in-app browser the way a person would: open it if
    /// needed, then navigate, click, type, or snapshot through the page.
    pub fn drive_browser(&mut self, action: crate::browser::Drive, window: &mut Window, cx: &mut Context<Self>) {
        let current = match self.active() {
            Some(Surface::Browser(id)) => self.panel_ref().and_then(|p| p.browsers.get(&id)).map(|(b, _)| b.clone()),
            _ => self.panel_ref().and_then(|p| {
                p.tabs.iter().rev().find_map(|s| match s {
                    Surface::Browser(id) => p.browsers.get(id).map(|(b, _)| b.clone()),
                    _ => None,
                })
            }),
        };
        if current.is_none() {
            self.open(Tab::Browser, None, window, cx);
        }
        let browser = match self.active() {
            Some(Surface::Browser(id)) => self.panel_ref().and_then(|p| p.browsers.get(&id)).map(|(b, _)| b.clone()),
            _ => None,
        };
        if let Some(b) = browser {
            b.update(cx, |b, cx| {
                b.wake(window, cx);
                b.drive(action, window, cx)
            });
        }
    }

    /// Local dev-server URLs the thread's terminals have printed.
    fn detected_urls(&self, cx: &gpui::App) -> Vec<String> {
        let Some(p) = self.panel_ref() else { return Vec::new() };
        let mut out: Vec<String> = Vec::new();
        for t in p.terminals.values() {
            let emu = &t.read(cx).emu;
            for r in 0..emu.rows() {
                for url in find_local_urls(&emu.row_text(r)) {
                    if !out.contains(&url) {
                        out.push(url);
                    }
                }
            }
        }
        out
    }

    fn surface_title(&self, s: Surface, cx: &gpui::App) -> SharedString {
        match s {
            Surface::Browser(id) => self
                .panel_ref()
                .and_then(|p| p.browsers.get(&id))
                .map(|(b, _)| b.read(cx).title())
                .unwrap_or_else(|| "Browser".into()),
            Surface::Terminal(id) => {
                let n = self
                    .panel_ref()
                    .map(|p| p.tabs.iter().filter(|t| matches!(t, Surface::Terminal(x) if *x <= id)).count())
                    .unwrap_or(1);
                format!("Terminal {n}").into()
            }
            Surface::SideChat(id) => {
                let n = self
                    .panel_ref()
                    .map(|p| p.tabs.iter().filter(|t| matches!(t, Surface::SideChat(x) if *x <= id)).count())
                    .unwrap_or(1);
                format!("Chat {n}").into()
            }
            Surface::Subagent(id) => self
                .subagent_view(id)
                .and_then(|v| v.read(cx).mirrored().and_then(|(_, info)| info.map(|i| i.title.clone())))
                .unwrap_or_else(|| "Subagent".into())
                .into(),
            Surface::Subagents if !self.subagents.is_empty() => format!("Subagents · {}", self.subagents.len()).into(),
            Surface::Chats if !self.side_chats.is_empty() => format!("Chats · {}", self.side_chats.len()).into(),
            Surface::Changes => {
                let n = self.changes.read(cx).summary().0;
                if n > 0 { format!("Diffs · {n}").into() } else { "Diffs".into() }
            }
            Surface::Plan if !self.todos.is_empty() => {
                let done = self.todos.iter().filter(|x| x.status == TodoStatus::Completed).count();
                format!("Plan · {done}/{}", self.todos.len()).into()
            }
            other => other.kind().label().into(),
        }
    }

    /// The titlebar strip over the panel: one 112px chip per surface, then `+`,
    /// expand, and close — Zeron's RightPanelTabs bar.
    pub fn render_strip(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let tabs = self.panel_ref().map(|p| p.tabs.clone()).unwrap_or_default();
        let active = self.active();

        let mut strip =
            div().id("panel-strip").flex().items_center().gap(px(4.0)).min_w_0().overflow_x_scroll().occlude();
        for (ix, s) in tabs.iter().copied().enumerate() {
            let selected = Some(s) == active;
            let title = self.surface_title(s, cx);
            let group = SharedString::from(format!("panel-chip-{ix}"));
            let loading = match s {
                Surface::Browser(id) => {
                    self.panel_ref().and_then(|p| p.browsers.get(&id)).is_some_and(|(b, _)| b.read(cx).page.loading)
                }
                Surface::Subagent(id) => self
                    .subagent_view(id)
                    .is_some_and(|v| v.read(cx).mirrored().and_then(|(_, info)| info).is_some_and(|i| i.running)),
                _ => false,
            };
            let favicon = match s {
                Surface::Browser(id) => {
                    self.panel_ref().and_then(|p| p.browsers.get(&id)).and_then(|(b, _)| b.read(cx).favicon.clone())
                }
                _ => None,
            };
            let lead: AnyElement = if loading {
                kit::spinner(SharedString::from(format!("chip-spin-{ix}")), 12.0, t.text_muted).into_any_element()
            } else if let Some(img) = favicon {
                gpui::img(img).size(px(12.0)).into_any_element()
            } else {
                icon(s.kind().glyph())
                    .size(px(12.0))
                    .text_color(if selected { t.text_muted } else { t.text_muted.opacity(0.7) })
                    .into_any_element()
            };
            strip = strip.child(
                surface_chrome::tab(("panel-chip", ix), selected, &t)
                    .group(group.clone())
                    .h(px(24.0))
                    .w(px(CHIP_W))
                    .flex_none()
                    .px(px(4.0))
                    .rounded(px(radius::RADIUS))
                    .gap(px(3.0))
                    .aria_label(title.clone())
                    .tooltip(ui::tip(title.clone()))
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(cx.listener(move |this, _, window, cx| this.activate(s, window, cx)))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| this.close(s, window, cx)),
                    )
                    .on_drag(ChipDrag { from: ix, title: title.clone() }, |d, _, _, cx| cx.new(|_| d.clone()))
                    .drag_over::<ChipDrag>(|st, _, _, _| st.bg(ink(0.10)))
                    .on_drop(cx.listener(move |this, d: &ChipDrag, _, cx| this.reorder(d.from, ix, cx)))
                    .child(div().size(px(18.0)).flex_none().flex().items_center().justify_center().child(lead))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(ty::CAPTION)
                            .text_color(if selected { t.text } else { t.text_muted })
                            .child(title),
                    )
                    .child(
                        div()
                            .id(("panel-chip-x", ix))
                            .size(px(18.0))
                            .flex_none()
                            .rounded(px(radius::RADIUS))
                            .relative()
                            .role(gpui::Role::Button)
                            .aria_label("Close tab")
                            .hover(|st| st.bg(t.wash(0.12)))
                            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close(s, window, cx);
                            }))
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .opacity(0.0)
                                    .group_hover(group, |st| st.opacity(1.0))
                                    .child(icon(icons::CLOSE).size(px(12.0)).text_color(t.text_muted)),
                            ),
                    ),
            );
        }
        let plus_open = self.plus_open;
        let plus = div()
            .relative()
            .child(
                surface_chrome::toolbar_button("panel-plus", "New tab")
                    .child(icon(icons::PLUS).size(px(13.0)).text_color(t.text_muted))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.plus_open = !this.plus_open;
                        this.plus_hi = None;
                        if this.plus_open {
                            window.focus(&this.plus_focus, cx);
                        }
                        cx.notify();
                    })),
            )
            .when(plus_open, |d| {
                let grid = self.render_surface_grid(&t, cx);
                d.child(deferred(
                    anchored().anchor(gpui::Anchor::TopRight).snap_to_window_with_margin(px(8.0)).child(
                        div()
                            .mt(px(28.0))
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.plus_open = false;
                                cx.notify();
                            }))
                            .child(ui::menu_card(&t, grid)),
                    ),
                ))
            });
        div()
            .h_full()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(8.0))
            .child(div().flex_1().min_w_0().child(strip))
            .when(!tabs.is_empty(), |d| d.child(plus))
            .child(
                surface_chrome::toolbar_button(
                    "panel-expand",
                    if self.expanded { "Collapse panel" } else { "Expand panel" },
                )
                .child(
                    icon(if self.expanded { icons::COLLAPSE_ARROWS } else { icons::EXPAND_ARROWS })
                        .size(px(13.0))
                        .text_color(t.text_muted),
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_expanded(cx))),
            )
            .child(
                surface_chrome::toolbar_button("panel-close", "Close panel")
                    .child(icon(icons::CLOSE).size(px(12.0)).text_color(t.text_muted))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SidePanelEvent::Close))),
            )
            .into_any_element()
    }

    /// Open a surface picked from a picker: browsers, terminals and side
    /// chats always open a new one; the rest show the existing surface.
    fn pick(&mut self, k: Tab, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(k, Tab::Browser | Tab::Terminal | Tab::SideChat) {
            self.open(k, None, window, cx);
        } else {
            self.show(k, window, cx);
        }
    }

    /// The `+` surface picker: a two-column grid of cards (glyph, name and
    /// what it is for). Arrow keys move the highlight across the grid,
    /// Enter or Space opens, Esc closes.
    fn render_surface_grid(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        const COLS: usize = 2;
        let count = Tab::ALL.len();
        let hi = self.plus_hi;
        let mut grid = div()
            .id("surface-grid")
            .w(px(340.0))
            .flex()
            .flex_wrap()
            .gap(px(4.0))
            .track_focus(&self.plus_focus)
            .on_key_down(cx.listener(move |this, e: &gpui::KeyDownEvent, window, cx| {
                let step: Option<isize> = match e.keystroke.key.as_str() {
                    "left" => Some(-1),
                    "right" => Some(1),
                    "up" => Some(-(COLS as isize)),
                    "down" => Some(COLS as isize),
                    _ => None,
                };
                match (e.keystroke.key.as_str(), step) {
                    (_, Some(d)) => {
                        this.plus_hi = Some(match this.plus_hi {
                            None => 0,
                            Some(at) => (at as isize + d).clamp(0, count as isize - 1) as usize,
                        });
                    }
                    ("home", _) => this.plus_hi = Some(0),
                    ("end", _) => this.plus_hi = Some(count - 1),
                    ("enter" | "space", _) => {
                        if let Some(k) = this.plus_hi.and_then(|i| Tab::ALL.get(i).copied()) {
                            this.pick(k, window, cx);
                        }
                    }
                    ("escape", _) => this.plus_open = false,
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }));
        for (i, k) in Tab::ALL.into_iter().enumerate() {
            let on = hi == Some(i);
            grid = grid.child(
                div()
                    .id(SharedString::from(format!("plus-{}", k.label())))
                    .w(px(166.0))
                    .h(px(62.0))
                    .p(px(10.0))
                    .flex()
                    .flex_col()
                    .justify_between()
                    .rounded(px(kit::popover::MENU_ITEM_RADIUS))
                    .border_1()
                    .border_color(if on { t.border_strong } else { t.border })
                    .bg(if on { kit::popover::row_active_bg(t) } else { ink(0.02) })
                    .cursor_pointer()
                    .role(gpui::Role::MenuItem)
                    .aria_label(k.label())
                    .hover(|s| s.bg(kit::popover::row_active_bg(t)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(icon(k.glyph()).size(px(14.0)).flex_none().text_color(t.text_muted))
                            .child(
                                div()
                                    .text_size(ty::BODY)
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(t.text)
                                    .child(k.label()),
                            ),
                    )
                    .child(div().truncate().text_size(ty::MICRO).text_color(t.text_faint).child(k.blurb()))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if *hovered && this.plus_hi != Some(i) {
                            this.plus_hi = Some(i);
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| this.pick(k, window, cx))),
            );
        }
        grid
    }

    /// Empty-pane picker: Zeron's compact vertical list of surface rows.
    fn render_picker(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        // Three quiet groups, top-aligned: what the agent changed, where it
        // runs, and who else is working. Each row says what it is for.
        const GROUPS: [(&str, &[Tab]); 3] = [
            ("Run", &[Tab::Browser, Tab::Terminal]),
            ("Review", &[Tab::Changes, Tab::Files, Tab::History, Tab::Plan]),
            ("Agents", &[Tab::Subagents, Tab::Chats, Tab::SideChat]),
        ];
        // Zeron's surface row: a quiet bordered plate, icon and name. What
        // each one is for lives in its tooltip, so the list stays a list.
        let (border, border_strong) = (t.border, t.border_strong);
        let row = |k: Tab, cx: &mut Context<Self>| {
            let blurb = k.blurb();
            div()
                .id(SharedString::from(format!("surface-card-{}", k.label())))
                .group("surface-row")
                .w_full()
                .h(px(44.0))
                .px(px(14.0))
                .rounded(px(radius::RADIUS))
                .border_1()
                .border_color(border)
                .bg(ink(0.02))
                .flex()
                .items_center()
                .gap(px(10.0))
                .cursor_pointer()
                .hover(move |s| s.bg(ink(0.05)).border_color(border_strong))
                .tooltip(ui::tip(blurb))
                .child(icon(k.glyph()).size(px(15.0)).flex_none().text_color(t.text_muted))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(ty::BODY)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(t.text)
                        .child(k.label()),
                )
                .child(
                    icon(icons::ALT_ARROW_RIGHT)
                        .size(px(12.0))
                        .flex_none()
                        .text_color(t.text_faint)
                        .invisible()
                        .group_hover("surface-row", |s| s.visible()),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.pick(k, window, cx)))
        };
        let mut col = div().w_full().max_w(px(300.0)).flex().flex_col().gap(px(18.0));
        for (title, tabs) in GROUPS {
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .px(px(2.0))
                            .text_size(ty::MICRO)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.text_faint)
                            .child(title),
                    )
                    .children(tabs.iter().map(|k| row(*k, cx))),
            );
        }
        // Centered in the pane when it fits, scrolling from the top when not.
        div()
            .id("surface-picker")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .w_full()
                    .min_h(gpui::relative(1.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .p(px(16.0))
                    .child(col)
                    .child(
                        div()
                            .mt(px(18.0))
                            .text_size(ty::CAPTION)
                            .text_color(t.text_faint)
                            .child("Ctrl+\\ hides the panel"),
                    ),
            )
            .into_any_element()
    }

    fn render_subagents(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.subagents.is_empty() {
            return kit::empty_state(
                icons::FORK,
                "No subagents yet",
                "When the agent delegates research, verification, or implementation to a helper, it appears here with its live brief, tool steps, and result.",
                t,
            )
            .into_any_element();
        }
        let running = self.subagents.iter().filter(|s| s.running).count();
        let done = self.subagents.iter().filter(|s| !s.running && !s.error).count();
        let total_steps: usize = self.subagents.iter().map(|s| s.steps).sum();
        div()
            .id("subagents-list")
            .size_full()
            .overflow_y_scroll()
            .p(px(14.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .h(px(32.0))
                    .px(px(10.0))
                    .rounded(px(radius::RADIUS))
                    .bg(ink(0.03))
                    .border_1()
                    .border_color(t.border)
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(ty::META)
                    .text_color(t.text_muted)
                    .child(icon(icons::FORK).size(px(13.0)).text_color(t.text_muted))
                    .child(format!("{} spawned", self.subagents.len()))
                    .when(running > 0, |d| {
                        d.child(div().text_color(t.text_faint).child("·")).child(
                            div()
                                .text_color(t.busy)
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(format!("{running} active")),
                        )
                    })
                    .when(done > 0, |d| {
                        d.child(div().text_color(t.text_faint).child("·"))
                            .child(div().text_color(t.success).child(format!("{done} done")))
                    })
                    .child(div().flex_1())
                    .when(total_steps > 0, |d| {
                        d.child(
                            div()
                                .text_size(ty::MICRO)
                                .text_color(t.text_faint)
                                .child(format!("{total_steps} tool steps")),
                        )
                    }),
            )
            .children(self.subagents.iter().enumerate().map(|(i, a)| {
                let row = a.row;
                let call = a.id.clone();
                let card = SharedString::from(format!("sub-card-{i}"));
                let (glyph, color): (AnyElement, _) = if a.running {
                    (kit::spinner(SharedString::from(format!("sub-spin-{i}")), 12.0, t.busy).into_any_element(), t.busy)
                } else if a.error {
                    (icon(icons::CLOSE).size(px(12.0)).text_color(t.danger).into_any_element(), t.danger)
                } else {
                    (icon(icons::CHECK).size(px(12.0)).text_color(t.success).into_any_element(), t.success)
                };
                let result = a.output.as_deref().map(|o| o.trim().lines().take(4).collect::<Vec<_>>().join("\n"));
                div()
                    .id(SharedString::from(format!("sub-{i}")))
                    .p(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .rounded(px(radius::RADIUS))
                    .border_1()
                    .border_color(t.border)
                    .bg(ink(0.02))
                    .group(card.clone())
                    .cursor_pointer()
                    .hover(|s| s.bg(ink(0.045)))
                    .tooltip(ui::tip(if a.running { "Watch it work" } else { "Open its chat" }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .size(px(22.0))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(radius::RADIUS))
                                    .bg(color.opacity(0.12))
                                    .child(glyph),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(ty::BODY)
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(t.text)
                                    .child(a.title.clone()),
                            )
                            .when(!a.kind.is_empty(), |d| d.child(kit::badge(a.kind.clone(), t.text_muted)))
                            .when(!a.running && a.steps > 0, |d| {
                                d.child(
                                    div()
                                        .text_size(ty::MICRO)
                                        .text_color(t.text_faint)
                                        .child(format!("{} steps", a.steps)),
                                )
                            })
                            .when_some(a.took, |d, took| {
                                d.child(
                                    div()
                                        .text_size(ty::CAPTION)
                                        .text_color(t.text_faint)
                                        .child(format!("{:.0}s", took.as_secs_f32())),
                                )
                            })
                            .child(
                                div()
                                    .id(SharedString::from(format!("sub-reveal-{i}")))
                                    .size(px(22.0))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(radius::RADIUS))
                                    .invisible()
                                    .group_hover(card, |s| s.visible())
                                    .hover(|s| s.bg(ink(0.08)))
                                    .tooltip(ui::tip("Show in the conversation"))
                                    .child(icon(icons::RETURN).size(px(12.0)).text_color(t.text_muted))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.emit(SidePanelEvent::RevealRow(row));
                                    })),
                            ),
                    )
                    .when(a.running, |d| {
                        let doing = a.doing.clone().unwrap_or_else(|| "Starting".into());
                        let steps = a.steps;
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .text_size(ty::META)
                                .text_color(t.busy)
                                .child(div().min_w_0().truncate().child(doing))
                                .when(steps > 0, |d| {
                                    d.child(
                                        div()
                                            .flex_none()
                                            .text_color(t.text_faint)
                                            .child(format!("· {steps} {}", if steps == 1 { "step" } else { "steps" })),
                                    )
                                }),
                        )
                    })
                    .when(!a.prompt.is_empty(), |d| {
                        d.child(
                            div().text_size(ty::SMALL).text_color(t.text_muted).line_clamp(3).child(a.prompt.clone()),
                        )
                    })
                    .when_some(result.filter(|r| !r.is_empty()), |d, r| {
                        d.child(
                            div()
                                .px(px(10.0))
                                .py(px(8.0))
                                .rounded(px(radius::RADIUS))
                                .bg(ink(0.035))
                                .text_size(ty::META)
                                .text_color(t.text_faint)
                                .line_clamp(4)
                                .child(r),
                        )
                    })
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(crate::actions::OpenSubagent { call: call.clone() }), cx);
                    })
            }))
            .into_any_element()
    }

    /// A subagent's chat under a slim header: who it is and how it is doing.
    fn render_subagent(&self, view: Entity<TaskView>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let info = view.read(cx).mirrored().and_then(|(_, i)| i.cloned());
        let status: AnyElement = match &info {
            Some(i) if i.running => kit::spinner("sub-head-spin", 12.0, t.busy).into_any_element(),
            Some(i) if i.error => icon(icons::CLOSE_CIRCLE).size(px(13.0)).text_color(t.danger).into_any_element(),
            Some(_) => icon(icons::CHECK).size(px(13.0)).text_color(t.success).into_any_element(),
            None => icon(icons::BOT).size(px(13.0)).text_color(t.text_muted).into_any_element(),
        };
        let state = match &info {
            Some(i) if i.running => match i.steps {
                0 => "Working".to_string(),
                1 => "Working · 1 step".to_string(),
                n => format!("Working · {n} steps"),
            },
            Some(i) if i.error => "Failed".into(),
            Some(i) => match i.took {
                Some(d) => format!("Finished in {}", crate::transcript::duration(d)),
                None => "Finished".into(),
            },
            None => String::new(),
        };
        let row = info.as_ref().map(|i| i.row);
        let header = surface_chrome::toolbar(t)
            .px(px(12.0))
            .gap(px(8.0))
            .child(div().size(px(16.0)).flex_none().flex().items_center().justify_center().child(status))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ty::SMALL)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(t.text)
                    .child(info.as_ref().map(|i| i.title.clone()).unwrap_or_else(|| "Subagent".into())),
            )
            .when_some(info.as_ref().map(|i| i.kind.clone()).filter(|k| !k.is_empty()), |d, k| {
                d.child(kit::badge(k, t.text_muted))
            })
            .child(div().flex_none().text_size(ty::CAPTION).text_color(t.text_faint).child(state))
            .when_some(row, |d, row| {
                d.child(
                    surface_chrome::toolbar_button("sub-head-reveal", "Show in the conversation")
                        .child(icon(icons::RETURN).size(px(13.0)).text_color(t.text_muted))
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(SidePanelEvent::RevealRow(row)))),
                )
            });
        div().size_full().flex().flex_col().child(header).child(div().flex_1().min_h_0().child(view)).into_any_element()
    }

    fn render_chats(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let new_chat = ui::button_primary("chats-new", "New side chat", t)
            .on_click(cx.listener(|this, _, window, cx| this.open(Tab::SideChat, None, window, cx)));
        let starters: &[(&str, &str)] = &[
            ("Review diff", "Review the uncommitted changes in this worktree and point out bugs or edge cases."),
            ("Write tests", "Write focused unit/integration tests for the recent changes in this worktree."),
            ("Explain changes", "Summarize what changed in this worktree and how the pieces fit together."),
        ];
        let quick =
            div().flex().flex_wrap().gap(px(6.0)).children(starters.iter().enumerate().map(|(i, (label, prompt))| {
                let prompt = (*prompt).to_string();
                div()
                    .id(SharedString::from(format!("chats-quick-{i}")))
                    .h(px(26.0))
                    .px(px(9.0))
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .rounded(px(radius::RADIUS))
                    .border_1()
                    .border_color(t.border)
                    .bg(ink(0.025))
                    .cursor_pointer()
                    .text_size(ty::CAPTION)
                    .text_color(t.text_muted)
                    .hover(|s| s.bg(ink(0.06)).text_color(t.text))
                    .child(icon(icons::MAGIC_STICK_3).size(px(11.0)).text_color(t.text_faint))
                    .child(*label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open(Tab::SideChat, Some(prompt.clone()), window, cx);
                    }))
            }));
        let body: AnyElement = if self.side_chats.is_empty() {
            kit::empty_state(
                icons::BOT,
                "No side chats",
                "A side chat shares this thread's branch and files: ask a question or try an idea without derailing the main conversation.",
                t,
            )
            .into_any_element()
        } else {
            kit::group(t)
                .children(self.side_chats.iter().enumerate().map(|(i, c)| {
                    let id = c.id;
                    div()
                        .id(SharedString::from(format!("chat-row-{id}")))
                        .h(px(42.0))
                        .px(px(12.0))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .when(i > 0, |d| d.border_t_1().border_color(t.border))
                        .cursor_pointer()
                        .hover(|s| s.bg(ink(0.04)))
                        .child(div().w(px(12.0)).flex().justify_center().child(kit::status_glyph(
                            SharedString::from(format!("chat-st-{id}")),
                            c.status,
                            c.unread,
                            t,
                        )))
                        .child(div().flex_1().min_w_0().truncate().text_size(ty::BODY).child(c.title.clone()))
                        .child(
                            div()
                                .text_size(ty::CAPTION)
                                .text_color(t.text_faint)
                                .child(ui::relative_time(c.updated_at)),
                        )
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(SidePanelEvent::OpenThread(id))))
                }))
                .into_any_element()
        };
        div()
            .id("chats-list")
            .size_full()
            .overflow_y_scroll()
            .p(px(14.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(div().flex().items_center().gap(px(8.0)).child(quick).child(div().flex_1()).child(new_chat))
            .child(body)
            .into_any_element()
    }

    fn render_plan(&self, t: &Theme, _cx: &mut Context<Self>) -> AnyElement {
        if self.todos.is_empty() {
            return kit::empty_state(
                icons::CHECKLIST,
                "No plan yet",
                "When the agent writes a todo list, it shows here.",
                t,
            )
            .into_any_element();
        }
        let done = self.todos.iter().filter(|x| x.status == TodoStatus::Completed).count();
        let frac = done as f32 / self.todos.len() as f32;
        div()
            .id("plan-list")
            .size_full()
            .overflow_y_scroll()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(div().flex().items_center().gap(px(8.0)).child(kit::ring(frac, 16.0, t.accent, t.wash(0.12))).child(
                div().text_size(ty::BODY).text_color(t.text).child(format!("{done} of {} done", self.todos.len())),
            ))
            .children(self.todos.iter().map(|item| {
                let (glyph, color) = match item.status {
                    TodoStatus::Completed => (Some(icons::CHECK), t.success),
                    TodoStatus::InProgress => (None, t.busy),
                    TodoStatus::Pending => (None, t.text_faint),
                };
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.0))
                    .text_size(ty::BODY)
                    .child(
                        div().mt(px(3.0)).size(px(14.0)).flex_none().flex().items_center().justify_center().child(
                            match glyph {
                                Some(g) => icon(g).size(px(14.0)).text_color(color).into_any_element(),
                                None => div()
                                    .size(px(12.0))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(color)
                                    .when(item.status == TodoStatus::InProgress, |d| d.bg(color.opacity(0.3)))
                                    .into_any_element(),
                            },
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_color(if item.status == TodoStatus::Completed { t.text_faint } else { t.text })
                            .when(item.status == TodoStatus::Completed, |d| d.line_through())
                            .child(item.text.clone()),
                    )
            }))
            .into_any_element()
    }
}

pub fn find_local_urls(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    for scheme in ["http://", "https://"] {
        let mut rest = line;
        while let Some(i) = rest.find(scheme) {
            let tail = &rest[i..];
            let end = tail
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | '>' | ','))
                .unwrap_or(tail.len());
            let url = tail[..end].trim_end_matches(['.', ';']);
            let host = &url[scheme.len()..];
            if ["localhost", "127.0.0.1", "0.0.0.0", "[::1]"].iter().any(|h| host.starts_with(h)) {
                out.push(url.replace("0.0.0.0", "localhost"));
            }
            rest = &tail[end.max(scheme.len())..];
        }
    }
    out
}

impl Render for SidePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("SidePanel");
        let t = Theme::of(cx).clone();
        let body: AnyElement = match self.active() {
            None => self.render_picker(&t, cx),
            Some(Surface::Subagents) => self.render_subagents(&t, cx),
            Some(Surface::Subagent(id)) => match self.subagent_view(id) {
                Some(v) => self.render_subagent(v.clone(), &t, cx),
                None => self.render_picker(&t, cx),
            },
            Some(Surface::Chats) => self.render_chats(&t, cx),
            Some(Surface::Changes) => self.changes.clone().into_any_element(),
            Some(Surface::Files) => self.files.clone().into_any_element(),
            Some(Surface::History) => self.git_history.clone().into_any_element(),
            Some(Surface::Plan) => self.render_plan(&t, cx),
            Some(Surface::Browser(id)) => match self.panel_ref().and_then(|p| p.browsers.get(&id)) {
                Some((b, _)) => b.clone().into_any_element(),
                None => self.render_picker(&t, cx),
            },
            Some(Surface::Terminal(id)) => match self.panel_ref().and_then(|p| p.terminals.get(&id)).cloned() {
                Some(term) => {
                    let running = term.read(cx).is_running();
                    let cwd = term.read(cx).cwd().to_path_buf();
                    let cwd_name = cwd
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| cwd.display().to_string());
                    let cwd_ext = cwd.clone();
                    let term_copy = term.clone();
                    let term_clear = term.clone();
                    let term_restart = term.clone();
                    let header = surface_chrome::toolbar(&t)
                        .px(px(12.0))
                        .gap(px(8.0))
                        .child(ui::dot(if running { t.success } else { t.text_faint }, 6.0))
                        .child(icon(icons::TERMINAL).size(px(12.5)).text_color(t.text_muted))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(t.font_mono.clone())
                                .text_size(ty::CAPTION)
                                .text_color(t.text)
                                .child(cwd_name),
                        )
                        .child(
                            surface_chrome::toolbar_button("dock-term-copy", "Copy selection or screen")
                                .child(icon(icons::COPY).size(px(12.5)).text_color(t.text_muted))
                                .on_click(move |_, _, cx| {
                                    term_copy.update(cx, |v, cx| v.copy_selection_or_screen(cx));
                                }),
                        )
                        .child(
                            surface_chrome::toolbar_button("dock-term-clear", "Clear screen")
                                .child(icon(icons::REFRESH).size(px(12.5)).text_color(t.text_muted))
                                .on_click(move |_, _, cx| {
                                    term_clear.update(cx, |v, cx| v.clear(cx));
                                }),
                        )
                        .when(!running, |d| {
                            d.child(
                                surface_chrome::toolbar_button("dock-term-restart", "Restart shell")
                                    .child(icon(icons::RESTART).size(px(12.5)).text_color(t.warning))
                                    .on_click(move |_, _, cx| {
                                        term_restart.update(cx, |v, cx| v.restart(cx));
                                    }),
                            )
                        })
                        .child(
                            surface_chrome::toolbar_button("dock-term-ext", "Open in system terminal")
                                .child(icon(icons::ARROW_UP_RIGHT).size(px(12.5)).text_color(t.text_muted))
                                .on_click(move |_, _, _| crate::terminal::open_system_terminal(&cwd_ext)),
                        )
                        .child(
                            surface_chrome::toolbar_button("dock-term-new", "New shell tab")
                                .child(icon(icons::PLUS).size(px(13.0)).text_color(t.text_muted))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.open(Tab::Terminal, None, window, cx)),
                                ),
                        );
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .child(header)
                        .child(div().flex_1().min_h_0().child(term))
                        .into_any_element()
                }
                None => self.render_picker(&t, cx),
            },
            Some(Surface::SideChat(id)) => match self.panel_ref().and_then(|p| p.chats.get(&id)) {
                Some((c, _)) => c.clone().into_any_element(),
                None => self.render_picker(&t, cx),
            },
        };
        div()
            .id("side-panel")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_local_dev_urls() {
        assert_eq!(find_local_urls("  ➜  Local:   http://localhost:5173/"), vec!["http://localhost:5173/"]);
        assert_eq!(find_local_urls("Listening on http://0.0.0.0:8080."), vec!["http://localhost:8080"]);
        assert!(find_local_urls("see https://example.com").is_empty());
    }
}
