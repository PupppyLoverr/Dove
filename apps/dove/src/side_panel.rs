//! The thread's side panel, on Zeron's model: a strip of surface tabs in the
//! titlebar band over the panel (Diffs · History · Files · Plan · Browser ·
//! Terminal, any number of browsers and terminals), a `+` menu, an expand
//! ("takeover") toggle that gives the panel the conversation's width, and an
//! empty state that lists the surfaces to open.
//!
//! Every thread keeps its own tabs; browsers and terminals live as long as
//! their thread (scrollback and page state survive switching away) and are
//! closed with it.
//!
//! The panel splits freely into panes ([`crate::workspace`]): each pane
//! shows one of the tabs, so a browser, a terminal and a side chat can sit
//! side by side next to the conversation. Tab clicks and the pickers act on
//! the focused pane.
//!
//! Tabs drag anywhere: onto a pane's middle to show there, onto one of its
//! edges to split that way. Other threads (a teammate's, any chat in the
//! sidebar) drag in the same way and run in a pane of their own, composer
//! and all, beside the conversation.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use dove_core::Engine;
use dove_core::proto::{TaskId, TaskStatus, TodoItem, TodoStatus};
use dove_core::store::{Project, Task};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, anchored, canvas, deferred, div, px, relative,
};
use std::sync::Arc;

use crate::assets::{icon, icons};
use crate::browser::{BrowserContext, BrowserEvent, BrowserSurface};
use crate::changes::{Changes, ChangesEvent};
use crate::design::{radius, ty};
use crate::files_panel::{FilesEvent, FilesPanel};
use crate::history::{GitHistory, HistoryEvent};
use crate::kit;
use crate::pr_view::{PrTarget, PrView, PrViewEvent};
use crate::side_chat::{SideChat, SideChatEvent};
use crate::surface_chrome;
use crate::terminal::TerminalView;
use crate::theme::{Theme, ink};
use crate::thread::ThreadView;
use crate::transcript::{Subagent, TaskView};
use crate::ui;
use crate::workspace::{self, Axis, Dir, Node, PaneId, SplitPath, Zone};

/// A chip's widest; chips shrink toward [`CHIP_MIN`] so more tabs fit
/// before the strip scrolls.
const CHIP_W: f32 = 132.0;
const CHIP_MIN: f32 = 76.0;
/// A pane header's height (split panes, and every thread pane).
const PANE_HEAD: f32 = 34.0;

/// Web views live at once; a browser pane beyond these pauses (each
/// WebView2 costs tens of megabytes) until it is clicked.
const MAX_LIVE_BROWSERS: usize = 3;

/// What the pane shortcuts ask of the panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaneCommand {
    Split(Axis),
    Close,
    Maximize,
    Focus(Dir),
}

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
    /// The thread's pull request.
    PullRequest,
    /// Another thread, whole (chat or terminal, with its composer).
    Thread(TaskId),
}

/// A thread being dragged (from the sidebar, the Team section, a tab):
/// dropped on the conversation or a pane, it opens beside them.
#[derive(Clone)]
pub struct ThreadDrag {
    pub id: TaskId,
    pub title: SharedString,
}

impl Render for ThreadDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("ThreadDrag");
        let t = Theme::of(cx);
        div()
            .h(px(28.0))
            .max_w(px(260.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(7.0))
            .rounded(px(radius::fit(28.0)))
            .bg(t.surface_raised)
            .border_1()
            .border_color(t.border_strong)
            .shadow_md()
            .text_size(ty::SMALL)
            .child(icon(icons::CHAT_ROUND_LINE).size(px(13.0)).text_color(t.text_muted))
            .child(div().truncate().child(self.title.clone()))
    }
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
    /// Offered only while the thread has a pull request.
    PullRequest,
    /// Another thread (dragged in; never offered by the pickers).
    Thread,
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
        Tab::ALL.into_iter().chain([Tab::PullRequest]).find(|t| t.label().to_ascii_lowercase().replace(' ', "-") == id)
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
            Tab::PullRequest => "Pull request",
            Tab::Thread => "Thread",
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
            Tab::PullRequest => "This thread's pull request, explained",
            Tab::Thread => "Another thread, beside this one",
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
            Tab::PullRequest => icons::PULL_REQUEST,
            Tab::Thread => icons::CHAT_ROUND_LINE,
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
            Surface::PullRequest => Tab::PullRequest,
            Surface::Thread(_) => Tab::Thread,
        }
    }
}

pub enum SidePanelEvent {
    SendToAgent(String),
    /// Put this prompt in the thread's composer for the user to send.
    StageForAgent(String),
    /// Start a thread on a pull request's branch.
    CheckoutPr {
        project: Project,
        number: u64,
        title: String,
    },
    /// The thread's pull request loaded or changed state (titlebar and
    /// info card mirror it).
    PrChanged,
    /// The panel's own close button.
    Close,
    /// Expand / collapse toggled.
    Layout,
    /// Scroll the conversation to a transcript row (a subagent's call).
    RevealRow(usize),
    /// Open another thread (a side chat).
    OpenThread(TaskId),
    /// A thread was dropped on a pane: the shell supplies its view and
    /// calls [`SidePanel::place_thread`].
    DropThread {
        id: TaskId,
        pane: PaneId,
        zone: Zone,
    },
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

struct ThreadPanel {
    tabs: Vec<Surface>,
    /// The panes, and the tab each shows (a pane missing here shows the
    /// picker). A tab is on screen in one pane at most.
    layout: Node,
    shown: HashMap<PaneId, Surface>,
    /// The pane tab clicks and keys act on, and the panes by recency (most
    /// recent first) for folding squeezed panes and pausing browsers.
    focus: PaneId,
    recent: Vec<PaneId>,
    maximized: Option<PaneId>,
    next_pane: PaneId,
    browsers: HashMap<u64, (Entity<BrowserSurface>, Subscription)>,
    terminals: HashMap<u64, Entity<TerminalView>>,
    chats: HashMap<u64, (Entity<SideChat>, Subscription)>,
    subagents: HashMap<u64, (Entity<TaskView>, Subscription)>,
    /// Other threads shown in panes, and saved ones waiting for their view.
    threads: HashMap<TaskId, Entity<ThreadView>>,
    wanted: Vec<(PaneId, TaskId)>,
}

impl Default for ThreadPanel {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            layout: Node::Leaf(0),
            shown: HashMap::new(),
            focus: 0,
            recent: vec![0],
            maximized: None,
            next_pane: 1,
            browsers: HashMap::new(),
            terminals: HashMap::new(),
            chats: HashMap::new(),
            subagents: HashMap::new(),
            threads: HashMap::new(),
            wanted: Vec::new(),
        }
    }
}

impl ThreadPanel {
    /// The focused pane's tab.
    fn active(&self) -> Option<Surface> {
        self.shown.get(&self.focus).copied()
    }

    fn pane_of(&self, s: Surface) -> Option<PaneId> {
        self.shown.iter().find(|(_, x)| **x == s).map(|(id, _)| *id)
    }

    fn focus_pane(&mut self, id: PaneId) {
        self.focus = id;
        self.recent.retain(|x| *x != id);
        self.recent.insert(0, id);
    }

    /// Show `s`: focus the pane already showing it, else show it in the
    /// focused pane.
    fn set_active(&mut self, s: Surface) {
        match self.pane_of(s) {
            Some(p) => {
                self.focus_pane(p);
                if self.maximized.is_some() {
                    self.maximized = Some(p);
                }
            }
            None => {
                self.shown.insert(self.focus, s);
            }
        }
    }

    /// Put `s` in `pane`; a pane already showing it takes this pane's old
    /// tab (the two swap).
    fn place(&mut self, pane: PaneId, s: Surface) {
        let old = self.shown.insert(pane, s);
        if let Some(q) = self.shown.iter().find(|(id, x)| **id != pane && **x == s).map(|(id, _)| *id) {
            match old {
                Some(o) => self.shown.insert(q, o),
                None => self.shown.remove(&q),
            };
        }
        self.focus_pane(pane);
    }

    /// `s` left the tabs (it was at `ix`): a pane showing it shows the
    /// nearest tab that is not on screen, or the picker.
    fn drop_surface(&mut self, s: Surface, ix: usize) {
        let Some(pane) = self.pane_of(s) else { return };
        self.shown.remove(&pane);
        let n = self.tabs.len();
        let next = (ix..n).chain((0..ix.min(n)).rev()).map(|i| self.tabs[i]).find(|t| self.pane_of(*t).is_none());
        if let Some(t) = next {
            self.shown.insert(pane, t);
        }
    }

    fn close_pane(&mut self, id: PaneId) {
        if !self.layout.remove(id) {
            return;
        }
        self.shown.remove(&id);
        self.recent.retain(|x| *x != id);
        if self.maximized == Some(id) {
            self.maximized = None;
        }
        if self.focus == id {
            let next = self.recent.first().copied().unwrap_or_else(|| self.layout.leaves()[0]);
            self.focus_pane(next);
        }
    }
}

/// A thread's panes as saved in settings (`layout.<thread>`): the tree,
/// the focused pane, and what each pane shows. Side chats and subagent
/// chats are conversations of their own, so their panes come back empty.
#[derive(serde::Serialize, serde::Deserialize)]
struct SavedLayout {
    tree: Node,
    focus: PaneId,
    panes: Vec<(PaneId, String)>,
}

/// A surface as saved: its kind, and a browser's address.
fn surface_spec(s: Surface, p: &ThreadPanel, cx: &gpui::App) -> Option<String> {
    Some(match s {
        Surface::Changes => "changes".into(),
        Surface::History => "history".into(),
        Surface::Files => "files".into(),
        Surface::Plan => "plan".into(),
        Surface::Subagents => "subagents".into(),
        Surface::Chats => "chats".into(),
        Surface::PullRequest => "pull-request".into(),
        Surface::Terminal(_) => "terminal".into(),
        Surface::Browser(id) => match p.browsers.get(&id)?.0.read(cx).page.url.clone() {
            Some(url) => format!("browser:{url}"),
            None => "browser".into(),
        },
        Surface::Thread(id) => format!("thread:{id}"),
        Surface::SideChat(_) | Surface::Subagent(_) => return None,
    })
}

/// The kind (and a browser's address) for a saved surface.
fn spec_tab(spec: &str) -> Option<(Tab, Option<String>)> {
    if let Some(url) = spec.strip_prefix("browser:") {
        return Some((Tab::Browser, Some(url.to_string())));
    }
    Some((
        match spec {
            "changes" => Tab::Changes,
            "history" => Tab::History,
            "files" => Tab::Files,
            "plan" => Tab::Plan,
            "subagents" => Tab::Subagents,
            "chats" => Tab::Chats,
            "pull-request" => Tab::PullRequest,
            "terminal" => Tab::Terminal,
            "browser" => Tab::Browser,
            _ => return None,
        },
        None,
    ))
}

/// The payload of a divider being dragged.
#[derive(Clone)]
struct DividerDrag {
    path: SplitPath,
}

impl Render for DividerDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
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

/// The side panel, for views that list its browser tabs (`@` mentions).
pub struct PanelHandle(pub gpui::WeakEntity<SidePanel>);

impl gpui::Global for PanelHandle {}

/// Every in-app browser tab across threads, as (title, url).
pub fn browser_tabs(cx: &gpui::App) -> Vec<(String, String)> {
    let Some(panel) = cx.try_global::<PanelHandle>().and_then(|h| h.0.upgrade()) else { return Vec::new() };
    let p = panel.read(cx);
    let mut out: Vec<(String, String)> = Vec::new();
    for tp in p.panels.values() {
        for (b, _) in tp.browsers.values() {
            let page = &b.read(cx).page;
            if let Some(url) = page.url.clone().filter(|u| !out.iter().any(|(_, x)| x == u)) {
                out.push((page.label().to_string(), url));
            }
        }
    }
    out
}

/// What "take a look at this" can point at in the browser tab.
impl SidePanel {
    /// The title and URL of the shown thread's browser, if it has one.
    pub fn browser_context(&self, cx: &gpui::App) -> Option<(String, String)> {
        let panel = self.panels.get(&self.thread?)?;
        let (b, _) = panel.browsers.values().next()?;
        let page = &b.read(cx).page;
        Some((page.label(), page.url.clone()?))
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
    /// One view for whichever thread is shown, like Changes.
    pr_view: Entity<PrView>,
    /// Each thread's pull request.
    prs: HashMap<TaskId, PrTarget>,
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
    /// The panel's size last frame, for what fits when splitting.
    bounds: Rc<Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    /// The shown thread's panes changed since they were last saved.
    layout_dirty: bool,
    /// Where a tab or thread being dragged would land.
    drop_hint: Option<(PaneId, Zone)>,
    /// Each dock terminal's side pill.
    pills: HashMap<u64, crate::term_pill::PillState>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl EventEmitter<SidePanelEvent> for SidePanel {}

impl SidePanel {
    pub fn new(engine: Arc<Engine>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let changes = cx.new(|cx| Changes::new(window, cx));
        let files = cx.new(|cx| FilesPanel::new(window, cx));
        let git_history = cx.new(|cx| GitHistory::new(window, cx));
        let pr_view = cx.new(|cx| PrView::new(engine.clone(), true, window, cx));
        let mut subs = vec![
            cx.subscribe(&pr_view, |_, _, ev: &PrViewEvent, cx| match ev {
                PrViewEvent::HandOff(text) => cx.emit(SidePanelEvent::StageForAgent(text.clone())),
                PrViewEvent::Checkout { project, number, title } => cx.emit(SidePanelEvent::CheckoutPr {
                    project: project.clone(),
                    number: *number,
                    title: title.clone(),
                }),
                PrViewEvent::Loaded => {
                    cx.emit(SidePanelEvent::PrChanged);
                    cx.notify();
                }
                PrViewEvent::Back => {}
            }),
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
            pr_view,
            prs: HashMap::new(),
            expanded: false,
            plus_open: false,
            plus_hi: None,
            plus_focus: cx.focus_handle(),
            next_id: 1,
            browser_context: BrowserContext::default(),
            visible: false,
            bounds: Rc::new(Cell::new(None)),
            layout_dirty: false,
            drop_hint: None,
            pills: HashMap::new(),
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
        self.panel_ref().and_then(|p| p.active())
    }

    /// The panel's area as the layout sees it (roomy until first drawn).
    fn dock_rect(&self) -> workspace::Rect {
        match self.bounds.get() {
            Some(b) => workspace::Rect::new(0.0, 0.0, f32::from(b.size.width), f32::from(b.size.height)),
            None => workspace::Rect::new(0.0, 0.0, 1200.0, 900.0),
        }
    }

    /// Panes the dock may hold: the chat column counts as one unless the
    /// panel has taken over the window.
    fn max_panes(&self) -> usize {
        if self.expanded { workspace::MAX_PANES } else { workspace::MAX_PANES - 1 }
    }

    /// The panes on screen: the maximized one, or every pane that fits.
    fn visible_layout(&self, p: &ThreadPanel) -> Node {
        match p.maximized.filter(|m| p.layout.contains(*m)) {
            Some(m) => Node::Leaf(m),
            None => workspace::fit(&p.layout, self.dock_rect(), &p.recent),
        }
    }

    /// The tabs on screen, one per visible pane.
    fn on_screen(&self) -> Vec<Surface> {
        let Some(p) = self.panel_ref() else { return Vec::new() };
        self.visible_layout(p).leaves().iter().filter_map(|id| p.shown.get(id).copied()).collect()
    }

    /// Whether `s` is on screen in some pane.
    pub fn showing(&self, s: Surface) -> bool {
        self.on_screen().contains(&s)
    }

    /// The browsers that get a live web view: those on screen, most
    /// recently focused first, at most [`MAX_LIVE_BROWSERS`].
    fn live_browsers(&self) -> Vec<u64> {
        let Some(p) = self.panel_ref() else { return Vec::new() };
        let on = self.visible_layout(p).leaves();
        let mut order: Vec<PaneId> = p.recent.iter().copied().filter(|x| on.contains(x)).collect();
        order.extend(on.iter().copied().filter(|x| !p.recent.contains(x)));
        order
            .iter()
            .filter_map(|pane| match p.shown.get(pane) {
                Some(Surface::Browser(id)) => Some(*id),
                _ => None,
            })
            .take(MAX_LIVE_BROWSERS)
            .collect()
    }

    /// Save the shown thread's panes (`layout.<thread>`).
    fn save_layout(&mut self, cx: &gpui::App) {
        self.layout_dirty = false;
        let Some(thread) = self.thread else { return };
        let Some(p) = self.panels.get(&thread) else { return };
        let mut panes: Vec<(PaneId, String)> =
            p.shown.iter().filter_map(|(id, s)| Some((*id, surface_spec(*s, p, cx)?))).collect();
        panes.sort();
        let saved = SavedLayout { tree: p.layout.clone(), focus: p.focus, panes };
        if let Ok(json) = serde_json::to_string(&saved) {
            let _ = self.engine.store.set_setting(&format!("layout.{thread}"), &json);
        }
    }

    /// Rebuild a thread's saved panes the first time it is shown.
    fn restore_layout(&mut self, thread: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(saved) = self
            .engine
            .store
            .setting(&format!("layout.{thread}"))
            .and_then(|j| serde_json::from_str::<SavedLayout>(&j).ok())
        else {
            return;
        };
        let leaves = saved.tree.leaves();
        let Some(&top) = leaves.iter().max() else { return };
        if leaves.len() > workspace::MAX_PANES {
            return;
        }
        {
            let p = self.panels.entry(thread).or_default();
            p.layout = saved.tree;
            p.next_pane = top + 1;
            p.recent = leaves.clone();
        }
        for (pane, spec) in saved.panes {
            // Threads come back once the shell hands over their views.
            if let Some(id) = spec.strip_prefix("thread:").and_then(TaskId::parse)
                && leaves.contains(&pane)
            {
                if let Some(p) = self.panels.get_mut(&thread) {
                    p.wanted.push((pane, id));
                }
                continue;
            }
            let Some((kind, url)) = spec_tab(&spec).filter(|_| leaves.contains(&pane)) else { continue };
            if let Some(p) = self.panels.get_mut(&thread) {
                p.focus = pane;
            }
            self.open(kind, url, window, cx);
        }
        if let Some(p) = self.panels.get_mut(&thread) {
            p.focus_pane(if leaves.contains(&saved.focus) { saved.focus } else { leaves[0] });
        }
        self.layout_dirty = false;
    }

    /// Whether `pane` (the focused one when None) can split along `axis`.
    fn can_split(&self, pane: Option<PaneId>, axis: Axis) -> bool {
        let Some(p) = self.panel_ref() else { return false };
        workspace::can_split(&p.layout, pane.unwrap_or(p.focus), axis, self.dock_rect(), self.max_panes())
    }

    /// Split a pane (the focused one when None); the new pane opens on the
    /// picker and takes focus.
    pub fn split(&mut self, pane: Option<PaneId>, axis: Axis, cx: &mut Context<Self>) -> bool {
        if !self.can_split(pane, axis) {
            return false;
        }
        let Some(p) = self.panel() else { return false };
        let at = pane.unwrap_or(p.focus);
        let new = p.next_pane;
        p.next_pane += 1;
        p.layout.split(at, axis, new);
        p.maximized = None;
        p.focus_pane(new);
        self.plus_open = false;
        self.layout_dirty = true;
        cx.notify();
        true
    }

    fn close_pane(&mut self, pane: PaneId, cx: &mut Context<Self>) {
        if let Some(p) = self.panel() {
            p.close_pane(pane);
        }
        self.layout_dirty = true;
        cx.notify();
    }

    fn toggle_maximize(&mut self, pane: PaneId, cx: &mut Context<Self>) {
        if let Some(p) = self.panel() {
            p.maximized = if p.maximized == Some(pane) { None } else { Some(pane) };
            p.focus_pane(pane);
        }
        cx.notify();
    }

    fn focus_pane(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.panel() else { return };
        if p.focus == pane {
            return;
        }
        p.focus_pane(pane);
        if let Some(Surface::Terminal(id)) = p.shown.get(&pane).copied()
            && let Some(t) = p.terminals.get(&id).cloned()
        {
            let f = t.read(cx).focus_handle(cx);
            window.focus(&f, cx);
        }
        cx.notify();
    }

    /// A new pane on side `dir` of `pane`, focused; None when there is no
    /// room (the cap, or a pane too small to halve).
    fn split_toward(&mut self, pane: PaneId, dir: Dir) -> Option<PaneId> {
        let axis = if matches!(dir, Dir::Left | Dir::Right) { Axis::Row } else { Axis::Column };
        if !self.can_split(Some(pane), axis) {
            return None;
        }
        let p = self.panel()?;
        let new = p.next_pane;
        p.next_pane += 1;
        if !p.layout.split_side(pane, dir, new) {
            return None;
        }
        p.maximized = None;
        p.focus_pane(new);
        Some(new)
    }

    /// Where a drop on `pane` with `zone` shows its content: the pane, or a
    /// new one beside it (the pane itself when the split has no room).
    fn drop_target(&mut self, pane: PaneId, zone: Zone) -> PaneId {
        match zone {
            Zone::Center => pane,
            Zone::Side(dir) => self.split_toward(pane, dir).unwrap_or(pane),
        }
    }

    /// Track the drop zone under a drag over `pane` (drawn as a preview).
    fn hover_drop(&mut self, pane: PaneId, bounds: gpui::Bounds<gpui::Pixels>, pos: gpui::Point<gpui::Pixels>) -> bool {
        let rect = workspace::Rect::new(
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        );
        let (x, y) = (f32::from(pos.x), f32::from(pos.y));
        if x < rect.x || y < rect.y || x > rect.x + rect.w || y > rect.y + rect.h {
            return false;
        }
        let hint = Some((pane, workspace::zone_at(rect, x, y)));
        if self.drop_hint != hint {
            self.drop_hint = hint;
            return true;
        }
        false
    }

    /// A tab chip dropped on `pane`: shown there, or in a new pane on the
    /// side it was dropped on.
    fn drop_tab(&mut self, from: usize, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let zone = self.drop_hint.take().filter(|(p, _)| *p == pane).map_or(Zone::Center, |(_, z)| z);
        let Some(s) = self.panel_ref().and_then(|p| p.tabs.get(from).copied()) else { return };
        // Dropping a pane's own tab on its own middle does nothing.
        if zone == Zone::Center && self.panel_ref().and_then(|p| p.shown.get(&pane)) == Some(&s) {
            cx.notify();
            return;
        }
        let target = self.drop_target(pane, zone);
        if let Some(p) = self.panel() {
            // A tab moving to a new pane leaves its old one.
            if zone != Zone::Center
                && let Some(old) = p.pane_of(s)
            {
                p.shown.remove(&old);
                if p.layout.count() > 1 {
                    p.close_pane(old);
                }
            }
            p.place(target, s);
        }
        self.shown(s, window, cx);
    }

    /// Show thread `id` (its view is `view`) in a pane: dropped on `pane`
    /// with `zone`, or (None) beside the conversation, splitting the panel's
    /// leftmost pane unless it is empty. A thread already in a pane moves.
    pub fn place_thread(
        &mut self,
        id: TaskId,
        view: Entity<ThreadView>,
        at: Option<(PaneId, Zone)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.thread == Some(id) {
            return;
        }
        self.drop_hint = None;
        let s = Surface::Thread(id);
        let target = match at {
            Some((pane, zone)) => self.drop_target(pane, zone),
            None => {
                let Some(p) = self.panel_ref() else { return };
                let first = p.layout.leaves()[0];
                let empty = p.tabs.is_empty() || !p.shown.contains_key(&first);
                if empty { first } else { self.split_toward(first, Dir::Left).unwrap_or(first) }
            }
        };
        view.update(cx, |v, cx| v.set_shown(true, cx));
        if let Some(p) = self.panel() {
            if let Some(old) = p.pane_of(s).filter(|o| *o != target) {
                p.shown.remove(&old);
                if p.layout.count() > 1 {
                    p.close_pane(old);
                }
            }
            p.threads.insert(id, view);
            if !p.tabs.contains(&s) {
                p.tabs.push(s);
            }
            p.place(target, s);
        }
        self.shown(s, window, cx);
    }

    /// Saved thread panes of the shown thread still waiting for a view.
    pub fn take_wanted(&mut self) -> Vec<(PaneId, TaskId)> {
        self.panel().map(|p| std::mem::take(&mut p.wanted)).unwrap_or_default()
    }

    /// Every thread shown in some thread's panes (they don't hibernate).
    pub fn holds_thread(&self, id: TaskId) -> bool {
        self.panels.values().any(|p| p.threads.contains_key(&id))
    }

    fn set_divider(&mut self, path: SplitPath, ratio: f32, cx: &mut Context<Self>) {
        if let Some(p) = self.panel() {
            p.layout.set_ratio(path, ratio);
        }
        self.layout_dirty = true;
        cx.notify();
    }

    /// A pane shortcut (split, close, maximize, move focus).
    pub fn command(&mut self, cmd: PaneCommand, window: &mut Window, cx: &mut Context<Self>) {
        let Some(focus) = self.panel_ref().map(|p| p.focus) else { return };
        match cmd {
            PaneCommand::Split(axis) => {
                self.split(None, axis, cx);
            }
            PaneCommand::Close => self.close_pane(focus, cx),
            PaneCommand::Maximize => self.toggle_maximize(focus, cx),
            PaneCommand::Focus(dir) => {
                let rect = self.dock_rect();
                let next =
                    self.panel_ref().and_then(|p| workspace::neighbor(&self.visible_layout(p), focus, dir, rect));
                if let Some(n) = next {
                    self.focus_pane(n, window, cx);
                }
            }
        }
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
        if self.layout_dirty {
            self.save_layout(cx);
        }
        self.thread = thread;
        self.dir = dir.clone();
        self.plus_open = false;
        let fresh = thread.is_some_and(|id| !self.panels.contains_key(&id));
        let _ = self.panel();
        if let Some(id) = thread.filter(|_| fresh) {
            self.restore_layout(id, window, cx);
        }
        self.changes.update(cx, |c, cx| c.set_target(dir.clone(), base, title, window, cx));
        self.files.update(cx, |f, cx| f.set_root(dir.clone(), cx));
        self.git_history.update(cx, |h, cx| h.set_root(dir, cx));
        if let Some(target) = thread.and_then(|id| self.prs.get(&id)).cloned() {
            let harness = self.parent.as_ref().map(|(task, _)| task.harness);
            self.pr_view.update(cx, |v, cx| {
                if let Some(h) = harness {
                    v.set_harness(h);
                }
                v.set_target(target, cx)
            });
        }
        cx.notify();
    }

    /// Link `thread` to its pull request. The first time a thread gets one,
    /// a Pull request tab joins its strip (without taking focus).
    pub fn set_pr(&mut self, thread: TaskId, target: Option<PrTarget>, cx: &mut Context<Self>) {
        let same = match (&target, self.prs.get(&thread)) {
            (Some(a), Some(b)) => a.repo == b.repo && a.number == b.number,
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        let panel = self.panels.entry(thread).or_default();
        match target {
            Some(target) => {
                if !panel.tabs.contains(&Surface::PullRequest) {
                    panel.tabs.push(Surface::PullRequest);
                }
                if self.thread == Some(thread) {
                    let t = target.clone();
                    let harness = self.parent.as_ref().map(|(task, _)| task.harness);
                    self.pr_view.update(cx, |v, cx| {
                        if let Some(h) = harness {
                            v.set_harness(h);
                        }
                        v.set_target(t, cx)
                    });
                }
                self.prs.insert(thread, target);
            }
            None => {
                if let Some(ix) = panel.tabs.iter().position(|s| *s == Surface::PullRequest) {
                    panel.tabs.remove(ix);
                    panel.drop_surface(Surface::PullRequest, ix);
                }
                self.prs.remove(&thread);
            }
        }
        cx.emit(SidePanelEvent::PrChanged);
        cx.notify();
    }

    /// The shown thread's pull request and, once loaded, its details.
    pub fn pr(&self, cx: &gpui::App) -> Option<(PrTarget, Option<dove_core::git::github::PrDetail>)> {
        let target = self.prs.get(&self.thread?)?.clone();
        let view = self.pr_view.read(cx);
        let detail = view.target().filter(|t| t.number == target.number).and_then(|_| view.detail().cloned());
        Some((target, detail))
    }

    /// Show `url` in the Pull request tab (a link clicked in chat). It
    /// becomes the thread's PR when the thread has none yet.
    pub fn show_pr(&mut self, target: PrTarget, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(thread) = self.thread
            && !self.prs.contains_key(&thread)
        {
            self.set_pr(thread, Some(target.clone()), cx);
        }
        self.pr_view.update(cx, |v, cx| v.set_target(target, cx));
        if let Some(p) = self.panel()
            && !p.tabs.contains(&Surface::PullRequest)
        {
            p.tabs.push(Surface::PullRequest);
        }
        self.activate(Surface::PullRequest, window, cx);
    }

    /// The kinds the `+` menu offers for the shown thread.
    fn offered(&self) -> Vec<Tab> {
        let mut out = Tab::ALL.to_vec();
        if self.thread.is_some_and(|id| self.prs.contains_key(&id)) {
            out.insert(2, Tab::PullRequest);
        }
        out
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
            // Threads come in by drag, with their view (`place_thread`).
            Tab::Thread => return,
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
            Tab::PullRequest => {
                if !self.thread.is_some_and(|id| self.prs.contains_key(&id)) {
                    return;
                }
                Surface::PullRequest
            }
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
            p.set_active(surface);
        }
        self.shown(surface, window, cx);
    }

    /// Show `surface` in the focused pane even when another pane has it
    /// (the two swap): what a picker in an empty pane means.
    fn place_here(&mut self, surface: Surface, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.panel() {
            let focus = p.focus;
            p.place(focus, surface);
        }
        self.shown(surface, window, cx);
    }

    /// A surface came on screen: refresh what goes stale, focus shells.
    fn shown(&mut self, surface: Surface, window: &mut Window, cx: &mut Context<Self>) {
        self.layout_dirty = true;
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
            Surface::Thread(id) => {
                if let Some(v) = p.threads.remove(&id) {
                    v.update(cx, |v, cx| v.set_shown(false, cx));
                }
            }
            _ => {}
        }
        let Some(p) = self.panel() else { return };
        p.drop_surface(surface, ix);
        self.layout_dirty = true;
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
        // Panes showing it beside other threads let it go.
        let s = Surface::Thread(thread);
        for p in self.panels.values_mut() {
            if p.threads.remove(&thread).is_some()
                && let Some(ix) = p.tabs.iter().position(|x| *x == s)
            {
                p.tabs.remove(ix);
                p.drop_surface(s, ix);
            }
        }
        cx.notify();
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
        let dragging = dragging || cx.has_active_drag();
        if self.layout_dirty && !dragging {
            self.save_layout(cx);
        }
        let live_ids = if visible { self.live_browsers() } else { Vec::new() };
        let urls = self.detected_urls(cx);
        let current = self.thread;
        let combos: Vec<String> = crate::keymap::shortcuts().iter().map(|s| s.default.to_string()).collect();
        for (thread, p) in &self.panels {
            for (id, (b, _)) in &p.browsers {
                let live = Some(*thread) == current && live_ids.contains(id);
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
            let pick = match p.active() {
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
                p.set_active(Surface::Browser(id));
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
        p.set_active(Surface::Browser(id));
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
                    if p.active() == Some(*s) { "* " } else { "" },
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
                        p.set_active(Surface::Browser(id));
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
            Surface::Thread(id) => self
                .panel_ref()
                .and_then(|p| p.threads.get(&id))
                .map(|v| SharedString::from(v.read(cx).chat.read(cx).task.title.clone()))
                .unwrap_or_else(|| "Thread".into()),
            Surface::Subagents if !self.subagents.is_empty() => format!("Subagents · {}", self.subagents.len()).into(),
            Surface::Chats if !self.side_chats.is_empty() => format!("Chats · {}", self.side_chats.len()).into(),
            Surface::PullRequest => match self.thread.and_then(|id| self.prs.get(&id)) {
                Some(t) => format!("PR #{}", t.number).into(),
                None => "Pull request".into(),
            },
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
        let on_screen = self.on_screen();
        let single = self.panel_ref().is_none_or(|p| p.layout.count() == 1);

        let mut strip =
            div().id("panel-strip").flex().items_center().gap(px(4.0)).min_w_0().overflow_x_scroll().occlude();
        for (ix, s) in tabs.iter().copied().enumerate() {
            let selected = on_screen.contains(&s);
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
            } else if let Surface::Thread(id) = s
                && let Some(kind) =
                    self.panel_ref().and_then(|p| p.threads.get(&id)).map(|v| v.read(cx).chat.read(cx).task.harness)
            {
                icon(ui::harness_icon(kind))
                    .size(px(12.0))
                    .text_color(ui::harness_tint(kind, &t, t.text_muted))
                    .into_any_element()
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
                    .flex_1()
                    .min_w(px(CHIP_MIN))
                    .max_w(px(CHIP_W))
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
            .when(!tabs.is_empty() && single, |d| {
                d.child(self.split_button("panel-split-right", None, Axis::Row, &t, cx)).child(self.split_button(
                    "panel-split-down",
                    None,
                    Axis::Column,
                    &t,
                    cx,
                ))
            })
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
        let existing = self
            .panel_ref()
            .and_then(|p| p.tabs.iter().copied().find(|s| s.kind() == k && !matches!(s, Surface::Subagent(_))));
        match existing {
            Some(s) if !matches!(k, Tab::Browser | Tab::Terminal | Tab::SideChat) => self.place_here(s, window, cx),
            _ => self.open(k, None, window, cx),
        }
    }

    /// A split button (the strip's, or a pane header's), quiet when there
    /// is no room.
    fn split_button(
        &self,
        id: impl Into<gpui::ElementId>,
        pane: Option<PaneId>,
        axis: Axis,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let room = self.can_split(pane, axis);
        let (glyph, tip) = match (axis, room) {
            (Axis::Row, true) => (icons::SPLIT_COLUMNS, "Split right"),
            (Axis::Row, false) => (icons::SPLIT_COLUMNS, "No room to split right: widen the panel or close a pane"),
            (Axis::Column, true) => (icons::SPLIT_ROWS, "Split down"),
            (Axis::Column, false) => (icons::SPLIT_ROWS, "No room to split down: close a pane first"),
        };
        surface_chrome::toolbar_button(id, tip)
            .child(icon(glyph).size(px(12.5)).text_color(if room { t.text_muted } else { t.text_faint.opacity(0.5) }))
            .when(room, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    this.split(pane, axis, cx);
                }))
            })
    }

    /// The `+` surface picker: a two-column grid of cards (glyph, name and
    /// what it is for). Arrow keys move the highlight across the grid,
    /// Enter or Space opens, Esc closes.
    fn render_surface_grid(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        const COLS: usize = 2;
        let offered = self.offered();
        let count = offered.len();
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
                        if let Some(k) = this.plus_hi.and_then(|i| this.offered().get(i).copied()) {
                            this.pick(k, window, cx);
                        }
                    }
                    ("escape", _) => this.plus_open = false,
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }));
        for (i, k) in offered.into_iter().enumerate() {
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
        let has_pr = self.thread.is_some_and(|id| self.prs.contains_key(&id));
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
                    .children(
                        tabs.iter()
                            .copied()
                            .chain((title == "Review" && has_pr).then_some(Tab::PullRequest))
                            .map(|k| row(k, cx)),
                    ),
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

impl SidePanel {
    /// One surface's body (the picker when the pane is empty).
    fn render_surface(&mut self, surface: Option<Surface>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let t = t.clone();
        match surface {
            None => self.render_picker(&t, cx),
            Some(Surface::Subagents) => self.render_subagents(&t, cx),
            Some(Surface::Subagent(id)) => match self.subagent_view(id) {
                Some(v) => self.render_subagent(v.clone(), &t, cx),
                None => self.render_picker(&t, cx),
            },
            Some(Surface::Chats) => self.render_chats(&t, cx),
            Some(Surface::PullRequest) => div()
                .id("dock-pr")
                .size_full()
                .overflow_y_scroll()
                .child(div().px(px(14.0)).pt(px(12.0)).pb(px(24.0)).child(self.pr_view.clone()))
                .into_any_element(),
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
                    // The same side pill as a thread's terminal: the shell
                    // gets every row, its actions open on hover.
                    use crate::term_pill::{self, Item, Pill};
                    let me = cx.entity().downgrade();
                    let key = |s: &str| format!("dock-term-{s}-{id}");
                    let mut items = vec![
                        Item::new(key("new"), icons::PLUS, "New shell tab", move |window, cx| {
                            let _ = me.update(cx, |this, cx| this.open(Tab::Terminal, None, window, cx));
                        }),
                        Item::new(key("copy"), icons::COPY, "Copy selection or screen", move |_, cx| {
                            term_copy.update(cx, |v, cx| v.copy_selection_or_screen(cx));
                        }),
                        Item::new(key("clear"), icons::REFRESH, "Clear screen", move |_, cx| {
                            term_clear.update(cx, |v, cx| v.clear(cx));
                        }),
                    ];
                    if !running {
                        let restart = Item::new(key("restart"), icons::RESTART, "Restart shell", move |_, cx| {
                            term_restart.update(cx, |v, cx| v.restart(cx));
                        });
                        items.push(restart.tone(t.warning));
                    }
                    items.push(Item::new(
                        format!("dock-term-ext-{id}"),
                        icons::ARROW_UP_RIGHT,
                        "Open in system terminal",
                        move |_, _| crate::terminal::open_system_terminal(&cwd_ext),
                    ));
                    let pill = term_pill::render(
                        Pill {
                            id: SharedString::from(format!("dock-term-pill-{id}")),
                            open: self.pills.get(&id).is_some_and(|p| p.open),
                            live: running,
                            glyph: icons::TERMINAL,
                            tint: t.text_muted,
                            title: cwd_name.into(),
                            detail: cwd.display().to_string().into(),
                            recording: false,
                        },
                        items,
                        cx.listener(move |this, hovered: &bool, _, cx| {
                            term_pill::hover(this, *hovered, move |p: &mut Self| p.pills.entry(id).or_default(), cx)
                        }),
                        &t,
                    );
                    div().relative().size_full().child(term).child(term_pill::overlay(pill)).into_any_element()
                }
                None => self.render_picker(&t, cx),
            },
            Some(Surface::SideChat(id)) => match self.panel_ref().and_then(|p| p.chats.get(&id)) {
                Some((c, _)) => c.clone().into_any_element(),
                None => self.render_picker(&t, cx),
            },
            Some(Surface::Thread(id)) => match self.panel_ref().and_then(|p| p.threads.get(&id)) {
                Some(v) => v.clone().into_any_element(),
                None => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(kit::spinner(SharedString::from(format!("pane-thread-{id}")), 16.0, t.text_muted))
                    .into_any_element(),
            },
        }
    }

    /// A layout node: a pane, or two halves around a draggable divider.
    /// Dividers drag only when every pane is on screen (folded or maximized
    /// layouts have other paths).
    fn render_node(
        &mut self,
        node: &Node,
        path: SplitPath,
        multi: bool,
        draggable: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Node::Split { axis, ratio, a, b } = node else {
            let Node::Leaf(pane) = node else { unreachable!() };
            return self.render_pane(*pane, multi, t, cx);
        };
        let (axis, ratio) = (*axis, *ratio);
        let row = axis == Axis::Row;
        let first = self.render_node(a, Node::child_path(path, false), multi, draggable, t, cx);
        let second = self.render_node(b, Node::child_path(path, true), multi, draggable, t, cx);
        let line = div().bg(t.border).when(row, |d| d.w(px(1.0)).h_full()).when(!row, |d| d.h(px(1.0)).w_full());
        let divider = div()
            .id(("pane-divider", path as usize))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .when(row, |d| d.w(px(5.0)).h_full())
            .when(!row, |d| d.h(px(5.0)).w_full())
            .child(line)
            .when(draggable, |d| {
                d.when(row, |d| d.cursor_col_resize())
                    .when(!row, |d| d.cursor_row_resize())
                    .hover(|s| s.bg(t.wash(0.08)))
                    .on_drag(DividerDrag { path }, |d, _, _, cx| cx.new(|_| d.clone()))
            });
        div()
            .id(("pane-split", path as usize))
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .when(row, |d| d.flex_row())
            .when(!row, |d| d.flex_col())
            .on_drag_move(cx.listener(move |this, e: &gpui::DragMoveEvent<DividerDrag>, _, cx| {
                if e.drag(cx).path != path {
                    return;
                }
                let b = e.bounds;
                let r = match axis {
                    Axis::Row => workspace::drag_ratio(
                        f32::from(e.event.position.x),
                        f32::from(b.origin.x),
                        f32::from(b.size.width),
                        workspace::MIN_W,
                    ),
                    Axis::Column => workspace::drag_ratio(
                        f32::from(e.event.position.y),
                        f32::from(b.origin.y),
                        f32::from(b.size.height),
                        workspace::MIN_H,
                    ),
                };
                this.set_divider(path, r, cx);
            }))
            .child(
                div()
                    .flex_none()
                    .overflow_hidden()
                    .when(row, |d| d.h_full().w(relative(ratio)))
                    .when(!row, |d| d.w_full().h(relative(ratio)))
                    .child(first),
            )
            .child(divider)
            .child(div().flex_1().min_w_0().min_h_0().overflow_hidden().child(second))
            .into_any_element()
    }

    /// One pane: its tab's body under a slim header when the panel is split
    /// (name, split right/down, maximize, close). A click anywhere focuses
    /// it; a tab chip dropped on it shows there.
    fn render_pane(&mut self, pane: PaneId, multi: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let surface = self.panel_ref().and_then(|p| p.shown.get(&pane).copied());
        let focused = self.panel_ref().is_some_and(|p| p.focus == pane);
        let maximized = self.panel_ref().is_some_and(|p| p.maximized == Some(pane));
        let paused = match surface {
            Some(Surface::Browser(id)) => self.visible && !self.live_browsers().contains(&id),
            _ => false,
        };
        let body = self.render_surface(surface, t, cx);
        let ix = pane as usize;
        let thread = match surface {
            Some(Surface::Thread(id)) => self.panel_ref().and_then(|p| p.threads.get(&id)).cloned(),
            _ => None,
        };
        let header = (multi || thread.is_some()).then(|| {
            let title: SharedString = surface.map(|s| self.surface_title(s, cx)).unwrap_or_else(|| "New pane".into());
            let glyph = surface.map(|s| s.kind().glyph()).unwrap_or(icons::WIDGET_ADD);
            let ink_on = if focused { t.text } else { t.text_muted };
            // A thread pane leads with its agent and status, and carries its
            // own Chat/Terminal switch and a way to make it the main thread.
            let (lead, switch, open_main): (AnyElement, Option<AnyElement>, Option<AnyElement>) = match &thread {
                Some(view) => {
                    let task = view.read(cx).chat.read(cx).task.clone();
                    let status = match task.status {
                        TaskStatus::Running => Some(t.busy),
                        TaskStatus::NeedsInput => Some(t.warning),
                        TaskStatus::Failed => Some(t.danger),
                        _ => None,
                    };
                    let lead = div()
                        .relative()
                        .flex_none()
                        .child(icon(ui::harness_icon(task.harness)).size(px(14.0)).text_color(ui::harness_tint(
                            task.harness,
                            t,
                            ink_on,
                        )))
                        .children(
                            status.map(|c| div().absolute().right(px(-3.0)).bottom(px(-2.0)).child(ui::dot(c, 6.0))),
                        )
                        .into_any_element();
                    let switch = view.update(cx, |v, cx| v.view_switch(t, cx));
                    let id = task.id;
                    let open = surface_chrome::toolbar_button(("pane-open-main", ix), "Open as the main thread")
                        .child(icon(icons::ARROW_UP_RIGHT).size(px(12.5)).text_color(t.text_muted))
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(SidePanelEvent::OpenThread(id))))
                        .into_any_element();
                    (lead, Some(switch), Some(open))
                }
                None => (icon(glyph).size(px(12.5)).flex_none().text_color(ink_on).into_any_element(), None, None),
            };
            div()
                .id(("pane-head", ix))
                .h(px(PANE_HEAD))
                .flex_none()
                .pl(px(10.0))
                .pr(px(6.0))
                .flex()
                .items_center()
                .gap(px(7.0))
                .border_b_1()
                .border_color(t.border)
                .when(focused && multi, |d| d.bg(t.wash(0.035)))
                .when_some(surface, |d, s| {
                    // The header drags the pane's tab, like its chip.
                    let from = self.panel_ref().and_then(|p| p.tabs.iter().position(|x| *x == s));
                    d.when_some(from, |d, from| {
                        d.on_drag(ChipDrag { from, title: title.clone() }, |d, _, _, cx| cx.new(|_| d.clone()))
                    })
                })
                .child(lead)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(ty::SMALL)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(ink_on)
                        .child(title),
                )
                .children(switch)
                .children(open_main)
                .when(!maximized && multi, |d| {
                    d.child(self.split_button(("pane-split-right", ix), Some(pane), Axis::Row, t, cx))
                        .child(self.split_button(("pane-split-down", ix), Some(pane), Axis::Column, t, cx))
                })
                .when(multi, |d| {
                    d.child(
                        surface_chrome::toolbar_button(
                            ("pane-max", ix),
                            if maximized { "Restore the panes" } else { "Maximize this pane" },
                        )
                        .child(
                            icon(if maximized { icons::WINDOW_RESTORE } else { icons::WINDOW_MAXIMIZE })
                                .size(px(12.5))
                                .text_color(t.text_muted),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_maximize(pane, cx))),
                    )
                })
                .when(!maximized, |d| match surface.filter(|_| !multi) {
                    // A lone thread pane closes its tab; split panes close.
                    Some(s) => d.child(
                        surface_chrome::toolbar_button(("pane-close-tab", ix), "Close")
                            .child(icon(icons::CLOSE).size(px(12.0)).text_color(t.text_muted))
                            .on_click(cx.listener(move |this, _, window, cx| this.close(s, window, cx))),
                    ),
                    None => d.child(
                        surface_chrome::toolbar_button(("pane-close", ix), "Close pane (its tab stays in the strip)")
                            .child(icon(icons::CLOSE).size(px(12.0)).text_color(t.text_muted))
                            .on_click(cx.listener(move |this, _, _, cx| this.close_pane(pane, cx))),
                    ),
                })
        });
        // While a tab or thread is dragged over this pane: where it lands.
        let preview = self.drop_hint.filter(|(p, _)| *p == pane && cx.has_active_drag()).map(|(_, zone)| {
            let full = workspace::Rect::new(0.0, 0.0, 1.0, 1.0);
            let r = zone.preview(full);
            div().absolute().left(relative(r.x)).top(relative(r.y)).w(relative(r.w)).h(relative(r.h)).p(px(6.0)).child(
                div()
                    .size_full()
                    .rounded(px(radius::CARD))
                    .border_1()
                    .border_color(t.accent.opacity(0.6))
                    .bg(t.accent.opacity(0.10)),
            )
        });
        div()
            .id(("pane", ix))
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .capture_any_mouse_down(cx.listener(move |this, _, window, cx| this.focus_pane(pane, window, cx)))
            .on_drag_move(cx.listener(move |this, e: &gpui::DragMoveEvent<ChipDrag>, _, cx| {
                if this.hover_drop(pane, e.bounds, e.event.position) {
                    cx.notify();
                }
            }))
            .on_drag_move(cx.listener(move |this, e: &gpui::DragMoveEvent<ThreadDrag>, _, cx| {
                if this.hover_drop(pane, e.bounds, e.event.position) {
                    cx.notify();
                }
            }))
            .on_drop(cx.listener(move |this, d: &ChipDrag, window, cx| this.drop_tab(d.from, pane, window, cx)))
            .on_drop(cx.listener(move |this, d: &ThreadDrag, _, cx| {
                let zone = this.drop_hint.take().filter(|(p, _)| *p == pane).map_or(Zone::Center, |(_, z)| z);
                cx.emit(SidePanelEvent::DropThread { id: d.id, pane, zone });
                cx.notify();
            }))
            .children(header)
            .child(div().flex_1().min_h_0().child(body))
            .children(preview)
            .when(paused, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .top(px(if multi { PANE_HEAD } else { 0.0 }))
                        .bg(t.bg)
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(6.0))
                        .child(icon(icons::GLOBE).size(px(18.0)).text_color(t.text_faint))
                        .child(div().text_size(ty::SMALL).text_color(t.text_muted).child("Paused to save memory"))
                        .child(div().text_size(ty::CAPTION).text_color(t.text_faint).child("Click to show this page")),
                )
            })
            .into_any_element()
    }
}

impl Render for SidePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("SidePanel");
        let t = Theme::of(cx).clone();
        let (shown, whole) = match self.panel_ref() {
            Some(p) => {
                let shown = self.visible_layout(p);
                let whole = shown == p.layout;
                let multi = p.layout.count() > 1;
                (shown, whole && multi)
            }
            None => (Node::Leaf(0), false),
        };
        let multi = self.panel_ref().is_some_and(|p| p.layout.count() > 1);
        let body = self.render_node(&shown, workspace::ROOT, multi, whole, &t, cx);
        // Measure the panel for what fits; a new size redraws once.
        let cell = self.bounds.clone();
        let me = cx.entity().downgrade();
        let probe = canvas(
            move |b, _, cx| {
                if cell.get().map(|o| o.size) != Some(b.size) {
                    cell.set(Some(b));
                    let _ = me.update(cx, |_, cx| cx.notify());
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();
        div()
            .id("side-panel")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(probe)
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
