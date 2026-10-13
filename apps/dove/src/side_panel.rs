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
use std::time::Duration;

use dove_core::proto::{TaskId, TodoItem, TodoStatus};
use dove_core::store::{Project, Task};
use dove_core::Engine;
use std::sync::Arc;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, anchored, deferred, div, px,
};

use crate::assets::{icon, icons};
use crate::browser::{BrowserContext, BrowserEvent, BrowserSurface};
use crate::changes::{Changes, ChangesEvent};
use crate::files_panel::{FilesEvent, FilesPanel};
use crate::history::{GitHistory, HistoryEvent};
use crate::side_chat::{SideChat, SideChatEvent};
use crate::kit;
use crate::surface_chrome;
use crate::terminal::TerminalView;
use crate::theme::{Theme, ink};
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
}

impl Tab {
    pub const ALL: [Tab; 7] = [Tab::Browser, Tab::Terminal, Tab::Changes, Tab::History, Tab::Files, Tab::Plan, Tab::SideChat];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Changes => "Diffs",
            Tab::History => "History",
            Tab::Files => "Files",
            Tab::Plan => "Plan",
            Tab::Browser => "Browser",
            Tab::Terminal => "Terminal",
            Tab::SideChat => "Side chat",
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
        }
    }
}

pub enum SidePanelEvent {
    SendToAgent(String),
    /// The panel's own close button.
    Close,
    /// Expand / collapse toggled.
    Layout,
}

#[derive(Default)]
struct ThreadPanel {
    tabs: Vec<Surface>,
    active: Option<Surface>,
    browsers: HashMap<u64, (Entity<BrowserSurface>, Subscription)>,
    terminals: HashMap<u64, Entity<TerminalView>>,
    chats: HashMap<u64, (Entity<SideChat>, Subscription)>,
}

#[derive(Clone)]
struct ChipDrag {
    from: usize,
    title: SharedString,
}

impl Render for ChipDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(cx);
        div()
            .h(px(24.0))
            .w(px(CHIP_W))
            .px(px(8.0))
            .flex()
            .items_center()
            .rounded(px(6.0))
            .bg(t.surface_raised)
            .border_1()
            .border_color(t.border_strong)
            .shadow_md()
            .text_size(px(12.0))
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
    /// The panel has the conversation's width.
    pub expanded: bool,
    plus_open: bool,
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
            expanded: false,
            plus_open: false,
            next_id: 1,
            browser_context: BrowserContext::default(),
            visible: false,
            focus: cx.focus_handle(),
            _subs: subs,
        }
    }

    fn panel(&mut self) -> Option<&mut ThreadPanel> {
        let id = self.thread?;
        Some(self.panels.entry(id).or_insert_with(ThreadPanel::default))
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

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.changes.update(cx, |c, cx| c.refresh(cx));
        self.files.update(cx, |f, cx| f.refresh(cx));
        self.git_history.update(cx, |h, cx| h.refresh(cx));
    }

    /// Show a surface of `kind`, opening one if the thread has none.
    pub fn show(&mut self, kind: Tab, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self.panel_ref().and_then(|p| p.tabs.iter().copied().find(|s| s.kind() == kind));
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
            Tab::SideChat => {
                let Some((task, project)) = self.parent.clone() else { return };
                let id = self.next_id;
                self.next_id += 1;
                let engine = self.engine.clone();
                let chat = cx.new(|cx| SideChat::new(engine, task, project, window, cx));
                let sub = cx.subscribe(&chat, |_, _, ev: &SideChatEvent, cx| match ev {
                    SideChatEvent::Started(_) | SideChatEvent::Changed => {
                        cx.emit(SidePanelEvent::Layout);
                        cx.notify();
                    }
                });
                if let Some(p) = self.panel() {
                    p.chats.insert(id, (chat, sub));
                }
                Surface::SideChat(id)
            }
        };
        if let Some(p) = self.panel() {
            if !p.tabs.contains(&surface) {
                p.tabs.push(surface);
            }
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
            _ => {}
        }
        if p.active == Some(surface) {
            p.active = p.tabs.get(ix).or_else(|| p.tabs.get(ix.saturating_sub(1))).copied();
        }
        cx.notify();
    }

    fn reorder(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if let Some(p) = self.panel() {
            if from < p.tabs.len() && to < p.tabs.len() && from != to {
                let s = p.tabs.remove(from);
                p.tabs.insert(to, s);
            }
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
        if reveal {
            if let Some(p) = self.panels.get_mut(&thread) {
                p.active = Some(Surface::Browser(id));
            }
            cx.notify();
        }
        browser
    }

    fn new_browser(&mut self, thread: TaskId, window: &mut Window, cx: &mut Context<Self>) -> (u64, Entity<BrowserSurface>) {
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
            if out.is_empty() { "No browser tabs.".into() } else { out.join("
") }
        };
        let browser_ids: Vec<u64> = self
            .panels
            .get(&thread)
            .map(|p| p.tabs.iter().filter_map(|s| if let Surface::Browser(id) = s { Some(*id) } else { None }).collect())
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
            _ => self.panel_ref().and_then(|p| p.tabs.iter().rev().find_map(|s| match s {
                Surface::Browser(id) => p.browsers.get(id).map(|(b, _)| b.clone()),
                _ => None,
            })),
        };
        if current.is_none() {
            self.open(Tab::Browser, None, window, cx);
        }
        let browser = match self.active() {
            Some(Surface::Browser(id)) => self.panel_ref().and_then(|p| p.browsers.get(&id)).map(|(b, _)| b.clone()),
            _ => None,
        };
        if let Some(b) = browser {
            b.update(cx, |b, cx| b.drive(action, window, cx));
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

    /// The titlebar strip over the panel: surface chips, `+`, expand, close.
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
            let loading = matches!(s, Surface::Browser(id) if self.panel_ref().and_then(|p| p.browsers.get(&id)).is_some_and(|(b, _)| b.read(cx).page.loading));
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
                    .size(px(13.0))
                    .text_color(if selected { t.text } else { t.text_muted })
                    .into_any_element()
            };
            strip = strip.child(
                surface_chrome::tab(("panel-chip", ix), selected, &t)
                    .group(group.clone())
                    .w(px(CHIP_W))
                    .px(px(4.0))
                    .gap(px(4.0))
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
                    // The icon swaps in place for ✕ while hovered (no width jump).
                    .child(
                        div()
                            .size(px(18.0))
                            .flex_none()
                            .relative()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(div().group_hover(group.clone(), |st| st.invisible()).child(lead))
                            .child(
                                div()
                                    .id(("panel-chip-x", ix))
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.0))
                                    .invisible()
                                    .group_hover(group.clone(), |st| st.visible())
                                    .hover(|st| st.bg(ink(0.10)))
                                    .role(gpui::Role::Button)
                                    .aria_label("Close tab")
                                    .child(icon(icons::CLOSE).size(px(10.0)).text_color(t.text_muted))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.close(s, window, cx);
                                    })),
                            ),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(title)),
            );
        }
        let plus = div()
            .relative()
            .child(
                surface_chrome::toolbar_button("panel-plus", "Open a surface")
                    .child(icon(icons::PLUS).size(px(14.0)).text_color(t.text_muted))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.plus_open = !this.plus_open;
                        cx.notify();
                    })),
            )
            .when(self.plus_open, |d| {
                let rows = div().w(px(200.0)).flex().flex_col().children(Tab::ALL.into_iter().map(|k| {
                    ui::menu_row(SharedString::from(format!("plus-{}", k.label())), k.glyph(), k.label(), false, &t)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if matches!(k, Tab::Browser | Tab::Terminal | Tab::SideChat) {
                                this.open(k, None, window, cx);
                            } else {
                                this.show(k, window, cx);
                            }
                        }))
                }));
                d.child(deferred(
                    anchored().anchor(gpui::Anchor::TopRight).snap_to_window_with_margin(px(8.0)).child(
                        div()
                            .mt(px(28.0))
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.plus_open = false;
                                cx.notify();
                            }))
                            .child(ui::menu_card(&t, rows)),
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
            .px(px(6.0))
            .child(div().flex_1().min_w_0().child(strip))
            .child(plus)
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

    /// The empty state: one row per surface (Zeron's surface picker).
    fn render_picker(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let row = |k: Tab, cx: &mut Context<Self>| {
            let border_strong = t.border_strong;
            div()
                .id(SharedString::from(format!("surface-card-{}", k.label())))
                .w_full()
                .h(px(44.0))
                .px(px(14.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(t.border)
                .bg(ink(0.02))
                .flex()
                .items_center()
                .gap(px(10.0))
                .cursor_pointer()
                .role(gpui::Role::Button)
                .aria_label(k.label())
                .hover(move |s| s.bg(ink(0.05)).border_color(border_strong))
                .child(icon(k.glyph()).size(px(15.0)).text_color(t.text_muted))
                .child(
                    div().text_size(px(13.0)).font_weight(gpui::FontWeight::MEDIUM).text_color(t.text).child(k.label()),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.open(k, None, window, cx)))
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(px(16.0))
            .child(
                div()
                    .w_full()
                    .max_w(px(280.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .children(Tab::ALL.into_iter().map(|k| row(k, cx))),
            )
            .into_any_element()
    }

    fn render_plan(&self, t: &Theme) -> AnyElement {
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
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(kit::ring(frac, 16.0, t.accent, t.wash(0.12)))
                    .child(div().text_size(px(13.0)).child(format!("{done} of {} done", self.todos.len()))),
            )
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
                    .text_size(px(13.5))
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

/// `http://localhost:5173/`, `http://127.0.0.1:8080`, … found in a line.
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
        let t = Theme::of(cx).clone();
        let body: AnyElement = match self.active() {
            None => self.render_picker(&t, cx),
            Some(Surface::Changes) => self.changes.clone().into_any_element(),
            Some(Surface::Files) => self.files.clone().into_any_element(),
            Some(Surface::History) => self.git_history.clone().into_any_element(),
            Some(Surface::Plan) => self.render_plan(&t),
            Some(Surface::Browser(id)) => match self.panel_ref().and_then(|p| p.browsers.get(&id)) {
                Some((b, _)) => b.clone().into_any_element(),
                None => self.render_picker(&t, cx),
            },
            Some(Surface::Terminal(id)) => match self.panel_ref().and_then(|p| p.terminals.get(&id)) {
                Some(term) => term.clone().into_any_element(),
                None => self.render_picker(&t, cx),
            },
            Some(Surface::SideChat(id)) => match self.panel_ref().and_then(|p| p.chats.get(&id)) {
                Some((c, _)) => c.clone().into_any_element(),
                None => self.render_picker(&t, cx),
            },
        };
        let _ = Duration::ZERO;
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
