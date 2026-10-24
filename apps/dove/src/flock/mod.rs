//! Flock: agents' own TUIs and shells side by side. Each tab is a tree of
//! split panes (a Claude Code TUI beside two shells, say) or an even grid of
//! up to 16. The sidebar's Agents roster lists every agent pane with whether
//! it is working, idle or waiting on you, read from its PTY. Broadcast
//! types into every selected pane at once; Kanban cards start here as a
//! fresh agent with the card's prompt; agents resume their last
//! conversation after a restart.
//!
//! Nothing spawns until the page is first opened: the saved layout is kept
//! as data and its panes start then.

pub mod pane_tree;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dove_core::Engine;
use dove_core::proto::HarnessKind;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, EventEmitter, Focusable as _, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Pixels, Render, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, anchored, canvas, deferred, div, px, relative,
};
use serde::{Deserialize, Serialize};

use crate::assets::{icon, icons};
use crate::design::{radius, ty};
use crate::terminal::{TerminalEvent, TerminalView};
use crate::theme::Theme;
use crate::ui;
use pane_tree::{Axis, Node, PaneId};

/// Output within this long reads as the agent working.
const WORKING_WINDOW: Duration = Duration::from_millis(1500);
const GAP: f32 = 6.0;
const SEAM: f32 = 6.0;
const SETTING: &str = "terminals_layout";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Working,
    /// Quiet on a question or approval prompt.
    NeedsInput,
    Idle,
    Exited,
}

impl AgentState {
    pub fn label(self) -> &'static str {
        match self {
            AgentState::Working => "Working",
            AgentState::NeedsInput => "Needs input",
            AgentState::Idle => "Idle",
            AgentState::Exited => "Exited",
        }
    }
}

/// One agent pane, for the sidebar roster.
#[derive(Clone)]
pub struct AgentRow {
    pub tab: usize,
    pub pane: PaneId,
    pub kind: HarnessKind,
    pub place: String,
    pub state: AgentState,
    /// When it last printed, in store milliseconds.
    pub last_ms: i64,
    pub selected: bool,
}

pub enum WorkspaceEvent {
    /// The roster changed (a pane came or went, an agent started or stopped working).
    AgentsChanged,
}

struct Pane {
    term: Entity<TerminalView>,
    /// `None` is a plain shell.
    kind: Option<HarnessKind>,
    cwd: PathBuf,
    /// A Kanban prompt to type once the agent has drawn and gone quiet.
    pending_prompt: Option<String>,
    _sub: Subscription,
}

struct Tab {
    tree: Node,
    focused: PaneId,
    /// Typing goes to every pane in `targets`.
    broadcast: bool,
    targets: HashSet<PaneId>,
}

impl Tab {
    fn new(tree: Node, focused: PaneId) -> Self {
        Self { tree, focused, broadcast: false, targets: HashSet::new() }
    }
}

/// Grid sizes offered under Layout.
pub const GRIDS: [usize; 5] = [2, 4, 6, 9, 16];

/// Screen text that means an agent is asking, not done.
const ASKING: [&str; 12] = [
    "do you want",
    "(y/n)",
    "[y/n]",
    "allow ",
    "approve",
    "press enter",
    "permission",
    "continue?",
    "proceed?",
    "\u{276f} 1.",
    "\u{203a} 1.",
    "waiting for your",
];

fn asking(tail: &str) -> bool {
    let tail = tail.to_lowercase();
    ASKING.iter().any(|p| tail.contains(p))
}

/// Arguments that reopen an agent's most recent conversation in its folder.
fn resume_args(kind: HarnessKind) -> Vec<String> {
    match kind {
        HarnessKind::ClaudeCode => vec!["--continue".into()],
        HarnessKind::Codex => vec!["resume".into(), "--last".into()],
        _ => Vec::new(),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Menu {
    Split,
}

/// The saved layout: trees plus what each leaf runs.
#[derive(Serialize, Deserialize, Default)]
struct Saved {
    tabs: Vec<SavedTab>,
    active: usize,
}

#[derive(Serialize, Deserialize)]
struct SavedTab {
    tree: Node,
    focused: PaneId,
    panes: Vec<SavedPane>,
}

#[derive(Serialize, Deserialize)]
struct SavedPane {
    id: PaneId,
    harness: Option<String>,
    cwd: PathBuf,
}

struct Drag {
    tab: usize,
    path: Vec<u8>,
    axis: Axis,
    area: Rc<Cell<Bounds<Pixels>>>,
}

pub struct Workspace {
    engine: Arc<Engine>,
    tabs: Vec<Tab>,
    panes: HashMap<PaneId, Pane>,
    pub active: usize,
    next_id: PaneId,
    /// Where new panes start (the home project, else the user's home).
    pub default_cwd: PathBuf,
    started: bool,
    menu: Option<Menu>,
    drag: Option<Drag>,
    /// The roster as last reported, to emit only on change.
    signature: Vec<(PaneId, AgentState)>,
    /// The roster as of the last report; render reads this instead of
    /// scanning every agent's screen.
    roster: Vec<AgentRow>,
    _ticker: Option<gpui::Task<()>>,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| ".".into())
}

/// "~" for the home folder, else the folder's name.
fn place(cwd: &Path) -> String {
    if cwd == home_dir() {
        return "~".into();
    }
    cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| cwd.display().to_string())
}

impl Workspace {
    pub fn new(engine: Arc<Engine>, _cx: &mut Context<Self>) -> Self {
        Self {
            engine,
            tabs: Vec::new(),
            panes: HashMap::new(),
            active: 0,
            next_id: 1,
            default_cwd: home_dir(),
            started: false,
            menu: None,
            drag: None,
            signature: Vec::new(),
            roster: Vec::new(),
            _ticker: None,
        }
    }

    /// Start the saved layout (or one shell) the first time the page shows.
    pub fn ensure_started(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.started {
            return;
        }
        self.started = true;
        let saved: Saved =
            self.engine.store.setting(SETTING).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        for st in saved.tabs {
            let mut ids = HashMap::new();
            for p in &st.panes {
                let kind = p.harness.as_deref().and_then(HarnessKind::from_id);
                let cwd = if p.cwd.is_dir() { p.cwd.clone() } else { self.default_cwd.clone() };
                let args = kind.map(resume_args).unwrap_or_default();
                if let Some(id) = self.spawn(kind, cwd, args, cx) {
                    ids.insert(p.id, id);
                }
            }
            // Re-key the tree onto the fresh ids, dropping panes that failed.
            let mut tree = Some(st.tree.clone());
            for old in st.tree.leaves() {
                if !ids.contains_key(&old) {
                    tree = tree.and_then(|t| t.remove(old));
                }
            }
            let Some(mut tree) = tree else { continue };
            rekey(&mut tree, &ids);
            let focused = ids.get(&st.focused).copied().unwrap_or_else(|| tree.leaves()[0]);
            self.tabs.push(Tab::new(tree, focused));
        }
        self.active = saved.active.min(self.tabs.len().saturating_sub(1));
        if self.tabs.is_empty() {
            self.new_tab(None, window, cx);
        }
        self.focus_active(window, cx);
        self.start_ticker(cx);
    }

    fn start_ticker(&mut self, cx: &mut Context<Self>) {
        self._ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                if this
                    .update(cx, |w, cx| {
                        w.deliver_prompts(cx);
                        w.report(cx)
                    })
                    .is_err()
                {
                    return;
                }
            }
        }));
    }

    /// Emit when the roster's shape or any agent's state changed.
    fn report(&mut self, cx: &mut Context<Self>) {
        let roster = self.scan_agents(cx);
        let sig: Vec<(PaneId, AgentState)> = roster.iter().map(|a| (a.pane, a.state)).collect();
        let selection_moved = roster.iter().map(|a| a.selected).ne(self.roster.iter().map(|a| a.selected));
        // Relative times move on their own; keep them fresh without a repaint.
        self.roster = roster;
        if sig != self.signature || selection_moved {
            self.signature = sig;
            cx.emit(WorkspaceEvent::AgentsChanged);
            cx.notify();
        }
    }

    /// Type each waiting Kanban prompt once its agent has drawn its screen
    /// and been quiet for a moment.
    fn deliver_prompts(&mut self, cx: &mut Context<Self>) {
        for p in self.panes.values_mut() {
            if p.pending_prompt.is_none() {
                continue;
            }
            let ready = {
                let term = p.term.read(cx);
                term.is_running() && term.is_quiet(Duration::from_secs(2)) && !term.tail_text(1).is_empty()
            };
            if ready && let Some(prompt) = p.pending_prompt.take() {
                p.term.update(cx, |t, cx| {
                    t.type_text(&prompt, cx);
                    t.send_raw("\r");
                });
            }
        }
    }

    fn spawn(
        &mut self,
        kind: Option<HarnessKind>,
        cwd: PathBuf,
        args: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        let term = match kind {
            // Each agent starts with Dove's tools, as a thread's terminal does.
            Some(k) => {
                let cmd = match self.engine.flock_command(k, &cwd) {
                    Ok(cmd) => cmd,
                    Err(e) => {
                        tracing::warn!("Flock could not start {}: {e:#}", k.label());
                        return None;
                    }
                };
                let mut all = cmd.args;
                all.extend(args);
                let dir = cwd.clone();
                cx.new(|cx| TerminalView::for_command_env(dir, cmd.program, all, cmd.env, cx))
            }
            None => {
                let dir = cwd.clone();
                cx.new(|cx| TerminalView::new(dir, cx))
            }
        };
        let id = self.next_id;
        self.next_id += 1;
        let sub = cx.subscribe(&term, |this, _, ev: &TerminalEvent, cx| match ev {
            TerminalEvent::Exited => this.report(cx),
        });
        self.panes.insert(id, Pane { term, kind, cwd, pending_prompt: None, _sub: sub });
        Some(id)
    }

    /// A new tab running `kind` (a shell for `None`).
    pub fn new_tab(&mut self, kind: Option<HarnessKind>, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.default_cwd.clone();
        self.open_tab(kind, cwd, None, window, cx);
    }

    fn open_tab(
        &mut self,
        kind: Option<HarnessKind>,
        cwd: PathBuf,
        prompt: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        let id = self.spawn(kind, cwd, Vec::new(), cx)?;
        if let Some(p) = self.panes.get_mut(&id) {
            p.pending_prompt = prompt;
        }
        self.tabs.push(Tab::new(Node::Leaf(id), id));
        self.active = self.tabs.len() - 1;
        self.menu = None;
        self.after_change(window, cx);
        Some(id)
    }

    /// Start a Kanban card here: a new tab running `kind` in `cwd`, typed
    /// `prompt` once it is ready. False when the agent could not start.
    pub fn dispatch(
        &mut self,
        kind: HarnessKind,
        cwd: PathBuf,
        prompt: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.ensure_started(window, cx);
        self.open_tab(Some(kind), cwd, Some(prompt), window, cx).is_some()
    }

    /// Fill the tab to an even grid of `n` panes: the ones it has keep their
    /// places, new ones are shells in the focused pane's folder.
    pub fn grid(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        if self.tabs.get(self.active).is_none() {
            self.new_tab(None, window, cx);
        }
        let Some(tab) = self.tabs.get(self.active) else { return };
        let mut ids = tab.tree.leaves();
        let cwd = self.panes.get(&tab.focused).map(|p| p.cwd.clone()).unwrap_or_else(|| self.default_cwd.clone());
        while ids.len() < n {
            let Some(id) = self.spawn(None, cwd.clone(), Vec::new(), cx) else { break };
            ids.push(id);
        }
        if let Some(tree) = Node::grid(&ids) {
            let tab = &mut self.tabs[self.active];
            tab.tree = tree;
            if tab.broadcast {
                tab.targets.extend(ids.iter().copied());
            }
        }
        self.wire_broadcast(cx);
        self.after_change(window, cx);
    }

    /// Broadcast on or off for the active tab; on, every pane is a target.
    pub fn toggle_broadcast(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else { return };
        tab.broadcast = !tab.broadcast;
        if tab.broadcast {
            tab.targets = tab.tree.leaves().into_iter().collect();
        }
        self.wire_broadcast(cx);
        cx.notify();
    }

    fn toggle_target(&mut self, tab: usize, id: PaneId, cx: &mut Context<Self>) {
        if let Some(t) = self.tabs.get_mut(tab)
            && !t.targets.remove(&id)
        {
            t.targets.insert(id);
        }
        self.wire_broadcast(cx);
        cx.notify();
    }

    /// Point each target pane's mirrors at the other targets in its tab.
    fn wire_broadcast(&mut self, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            for id in tab.tree.leaves() {
                let Some(p) = self.panes.get(&id) else { continue };
                let mirrors: Vec<_> = if tab.broadcast && tab.targets.contains(&id) {
                    tab.targets
                        .iter()
                        .filter(|o| **o != id)
                        .filter_map(|o| self.panes.get(o))
                        .map(|o| o.term.downgrade())
                        .collect()
                } else {
                    Vec::new()
                };
                p.term.update(cx, |t, _| t.mirrors = mirrors);
            }
        }
    }

    /// Split the focused pane, running the same thing in the same folder
    /// (a shell beside a shell, a shell beside an agent).
    pub fn split(&mut self, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return self.new_tab(None, window, cx) };
        let target = tab.focused;
        let cwd = self.panes.get(&target).map(|p| p.cwd.clone()).unwrap_or_else(|| self.default_cwd.clone());
        let Some(id) = self.spawn(None, cwd, Vec::new(), cx) else { return };
        let tab = &mut self.tabs[self.active];
        if tab.tree.split(target, axis, id) {
            tab.focused = id;
            if tab.broadcast {
                tab.targets.insert(id);
            }
        }
        self.wire_broadcast(cx);
        self.after_change(window, cx);
    }

    /// Close the focused pane; the tab goes with its last one.
    pub fn close_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let id = tab.focused;
        self.close(self.active, id, window, cx);
    }

    fn close(&mut self, tab_ix: usize, id: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.panes.remove(&id) {
            p.term.update(cx, |t, cx| t.kill(cx));
        }
        let mut tab = self.tabs.remove(tab_ix);
        let next = tab.tree.next(id, false);
        tab.targets.remove(&id);
        let Tab { tree, broadcast, targets, .. } = tab;
        if let Some(tree) = tree.remove(id) {
            let focused = next.filter(|n| tree.contains(*n)).unwrap_or_else(|| tree.leaves()[0]);
            self.tabs.insert(tab_ix, Tab { tree, focused, broadcast, targets });
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
        self.after_change(window, cx);
    }

    pub fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(ix);
        for id in tab.tree.leaves() {
            if let Some(p) = self.panes.remove(&id) {
                p.term.update(cx, |t, cx| t.kill(cx));
            }
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if self.active > ix {
            self.active -= 1;
        }
        self.after_change(window, cx);
    }

    /// Show a pane (from the roster): its tab, focused.
    pub fn focus_pane(&mut self, pane: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.tabs.iter().position(|t| t.tree.contains(pane)) {
            self.active = ix;
            self.tabs[ix].focused = pane;
            self.focus_active(window, cx);
            self.report(cx);
            cx.notify();
        }
    }

    fn focus_active(&self, window: &mut Window, cx: &mut App) {
        if let Some(p) = self.tabs.get(self.active).and_then(|t| self.panes.get(&t.focused)) {
            window.focus(&p.term.focus_handle(cx), cx);
        }
    }

    fn after_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_active(window, cx);
        self.save();
        self.report(cx);
        cx.notify();
    }

    fn save(&self) {
        let saved = Saved {
            active: self.active,
            tabs: self
                .tabs
                .iter()
                .map(|t| SavedTab {
                    tree: t.tree.clone(),
                    focused: t.focused,
                    panes: t
                        .tree
                        .leaves()
                        .into_iter()
                        .filter_map(|id| {
                            let p = self.panes.get(&id)?;
                            Some(SavedPane { id, harness: p.kind.map(|k| k.id().to_string()), cwd: p.cwd.clone() })
                        })
                        .collect(),
                })
                .collect(),
        };
        if let Ok(s) = serde_json::to_string(&saved) {
            let _ = self.engine.store.set_setting(SETTING, &s);
        }
    }

    /// Every agent pane, in tab order, as of the last report (at most half
    /// a second old).
    pub fn agents(&self) -> &[AgentRow] {
        &self.roster
    }

    fn scan_agents(&self, cx: &App) -> Vec<AgentRow> {
        let now = Instant::now();
        let now_ms = dove_core::store::now_ms();
        let mut out = Vec::new();
        for (tab_ix, tab) in self.tabs.iter().enumerate() {
            for id in tab.tree.leaves() {
                let Some(p) = self.panes.get(&id) else { continue };
                let Some(kind) = p.kind else { continue };
                let term = p.term.read(cx);
                let state = if !term.is_running() {
                    AgentState::Exited
                } else if term.is_quiet(WORKING_WINDOW) {
                    if asking(&term.tail_text(8)) { AgentState::NeedsInput } else { AgentState::Idle }
                } else {
                    AgentState::Working
                };
                let ago = now.saturating_duration_since(term.last_output).as_millis() as i64;
                out.push(AgentRow {
                    tab: tab_ix,
                    pane: id,
                    kind,
                    place: place(&p.cwd),
                    state,
                    last_ms: now_ms - ago,
                    selected: tab_ix == self.active && tab.focused == id,
                });
            }
        }
        out
    }

    /// Agents that can run here: installed, and not the bare-shell kind.
    fn launchable(&self) -> Vec<HarnessKind> {
        // Only agents with a terminal UI of their own (not ACP-only ones).
        let mut kinds: Vec<HarnessKind> = self
            .engine
            .installed()
            .into_iter()
            .map(|i| i.kind)
            .filter(|k| *k != HarnessKind::Shell && self.engine.has_terminal_ui(*k))
            .collect();
        kinds.dedup();
        kinds
    }

    fn pane_title(&self, id: PaneId) -> String {
        match self.panes.get(&id) {
            Some(Pane { kind: Some(k), .. }) => k.label().to_string(),
            Some(p) => place(&p.cwd),
            None => String::new(),
        }
    }

    fn pane_glyph(&self, id: PaneId, t: &Theme) -> AnyElement {
        match self.panes.get(&id).and_then(|p| p.kind) {
            Some(k) => icon(ui::harness_icon(k)).size(px(13.0)).text_color(ui::harness_tint(k, t, t.text_muted)),
            None => icon(icons::TERMINAL).size(px(13.0)).text_color(t.text_faint),
        }
        .into_any_element()
    }

    fn render_tabs(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let asking_tabs: HashSet<usize> =
            self.roster.iter().filter(|a| a.state == AgentState::NeedsInput).map(|a| a.tab).collect();
        let mut strip = div().id("ws-tabs").flex().items_center().gap(px(4.0)).min_w_0().overflow_x_scroll();
        for (ix, tab) in self.tabs.iter().enumerate() {
            let active = ix == self.active;
            let leaves = tab.tree.leaves();
            let mut glyphs = div().flex().items_center().gap(px(3.0));
            for id in leaves.iter().take(4) {
                glyphs = glyphs.child(self.pane_glyph(*id, t));
            }
            let group = format!("ws-tab-{ix}");
            strip = strip.child(
                div()
                    .id(gpui::ElementId::Name(group.clone().into()))
                    .group(group.clone())
                    .h(px(28.0))
                    .px(px(8.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(7.0))
                    .rounded(px(radius::RADIUS))
                    .cursor_pointer()
                    .text_size(ty::BODY)
                    .when(active, |d| d.bg(t.wash(0.08)).text_color(t.text))
                    .when(!active, |d| d.text_color(t.text_muted).hover(|s| s.bg(t.wash(0.04))))
                    .child(glyphs)
                    .child(div().max_w(px(180.0)).truncate().child(self.pane_title(tab.focused)))
                    .when(asking_tabs.contains(&ix), |d| d.child(ui::dot(t.warning, 6.0)))
                    .child(
                        div()
                            .id(gpui::ElementId::Name(format!("ws-tab-x-{ix}").into()))
                            .size(px(16.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.0))
                            .invisible()
                            .group_hover(group.clone(), |s| s.visible())
                            .hover(|s| s.bg(t.wash(0.08)))
                            .child(icon(icons::CLOSE).size(px(11.0)).text_color(t.text_faint))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_tab(ix, window, cx);
                            })),
                    )
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| this.close_tab(ix, window, cx)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.active = ix;
                        this.after_change(window, cx);
                    })),
            );
        }
        strip.into_any_element()
    }

    /// The Split menu: split the focused pane, lay the tab out as a grid,
    /// or open a new tab running a shell or an agent.
    fn render_menu(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let row = |id: String, glyph: AnyElement, label: String, hint: Option<&'static str>| {
            div()
                .id(gpui::ElementId::Name(id.into()))
                .h(px(30.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .gap(px(9.0))
                .rounded(px(radius::RADIUS))
                .cursor_pointer()
                .text_size(ty::BODY)
                .text_color(t.text)
                .hover(|s| s.bg(t.wash(0.06)))
                .child(glyph)
                .child(div().flex_1().min_w_0().truncate().child(label))
                .children(hint.map(|h| div().text_size(ty::CAPTION).text_color(t.text_faint).child(h)))
        };
        let section = |label: &'static str| {
            div().px(px(10.0)).pt(px(8.0)).pb(px(2.0)).text_size(ty::MICRO).text_color(t.text_faint).child(label)
        };
        let glyph = |g: &'static str| icon(g).size(px(14.0)).text_color(t.text_muted).into_any_element();
        let has_tab = self.tabs.get(self.active).is_some();
        let mut list = div()
            .w(px(250.0))
            .p(px(4.0))
            .flex()
            .flex_col()
            .rounded(px(10.0))
            .bg(t.surface_overlay)
            .border_1()
            .border_color(t.border)
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.menu = None;
                cx.notify();
            }));
        if has_tab {
            list = list
                .child(section("Split the focused pane"))
                .child(
                    row(
                        "ws-split-right".into(),
                        glyph(icons::SPLIT_COLUMNS),
                        "Split right".into(),
                        Some("Ctrl+Shift+D"),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.menu = None;
                        this.split(Axis::Row, window, cx)
                    })),
                )
                .child(
                    row("ws-split-down".into(), glyph(icons::SPLIT_ROWS), "Split down".into(), Some("Ctrl+Shift+E"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.menu = None;
                            this.split(Axis::Column, window, cx)
                        })),
                );
        }
        // Grids as a row of chips: 2 to 16 panes.
        let mut grids = div().px(px(6.0)).pb(px(4.0)).flex().flex_wrap().gap(px(4.0));
        for n in GRIDS {
            grids = grids.child(
                div()
                    .id(gpui::ElementId::Name(format!("ws-grid-{n}").into()))
                    .h(px(28.0))
                    .min_w(px(40.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(radius::RADIUS))
                    .bg(t.wash(0.04))
                    .cursor_pointer()
                    .text_size(ty::SMALL)
                    .text_color(t.text)
                    .hover(|s| s.bg(t.wash(0.09)))
                    .tooltip(ui::tip(format!("{n} panes in an even grid")))
                    .child(n.to_string())
                    .on_click(cx.listener(move |this, _, window, cx| this.grid(n, window, cx))),
            );
        }
        list = list.child(section("Grid")).child(grids).child(section("New tab"));
        list = list.child(
            row("ws-new-shell".into(), glyph(icons::TERMINAL), "Shell".into(), None)
                .on_click(cx.listener(|this, _, window, cx| this.new_tab(None, window, cx))),
        );
        for kind in self.launchable() {
            list = list.child(
                row(
                    format!("ws-new-{}", kind.id()),
                    icon(ui::harness_icon(kind))
                        .size(px(14.0))
                        .text_color(ui::harness_tint(kind, t, t.text_muted))
                        .into_any_element(),
                    kind.label().to_string(),
                    None,
                )
                .on_click(cx.listener(move |this, _, window, cx| this.new_tab(Some(kind), window, cx))),
            );
        }
        deferred(
            anchored()
                .anchor(gpui::Anchor::TopRight)
                .snap_to_window_with_margin(px(8.0))
                .child(div().mt(px(32.0)).child(list)),
        )
        .with_priority(2)
        .into_any_element()
    }

    /// One node of the tree: a pane card, or two children around a seam.
    fn render_node(&self, tab: usize, node: &Node, path: Vec<u8>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        match node {
            Node::Leaf(id) => self.render_pane(tab, *id, t, cx),
            Node::Split { axis, ratio, a, b } => {
                let axis = *axis;
                let area: Rc<Cell<Bounds<Pixels>>> = Rc::new(Cell::new(Bounds::default()));
                let mut pa = path.clone();
                pa.push(0);
                let mut pb = path.clone();
                pb.push(1);
                let first = div()
                    .flex_none()
                    .min_w_0()
                    .min_h_0()
                    .when(axis == Axis::Row, |d| d.h_full().w(relative(*ratio)))
                    .when(axis == Axis::Column, |d| d.w_full().h(relative(*ratio)))
                    .child(self.render_node(tab, a, pa, t, cx));
                let second = div().flex_1().min_w_0().min_h_0().child(self.render_node(tab, b, pb, t, cx));
                let seam_id = format!("ws-seam-{tab}-{}", path.iter().map(|s| s.to_string()).collect::<String>());
                let seam = div()
                    .id(gpui::ElementId::Name(seam_id.into()))
                    .flex_none()
                    .when(axis == Axis::Row, |d| d.w(px(SEAM)).h_full().cursor_col_resize())
                    .when(axis == Axis::Column, |d| d.h(px(SEAM)).w_full().cursor_row_resize())
                    .on_mouse_down(MouseButton::Left, {
                        let area = area.clone();
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.drag = Some(Drag { tab, path: path.clone(), axis, area: area.clone() });
                        })
                    });
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .when(axis == Axis::Column, |d| d.flex_col())
                    .child(
                        canvas(move |bounds, _, _| area.set(bounds), |_, _, _, _| {})
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                    )
                    .child(first)
                    .child(seam)
                    .child(second)
                    .into_any_element()
            }
        }
    }

    fn render_pane(&self, tab: usize, id: PaneId, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(p) = self.panes.get(&id) else { return div().into_any_element() };
        let focused = self.tabs.get(tab).is_some_and(|tb| tb.focused == id);
        let multi = self.tabs.get(tab).is_some_and(|tb| matches!(tb.tree, Node::Split { .. }));
        let broadcast = self.tabs.get(tab).is_some_and(|tb| tb.broadcast);
        let target = broadcast && self.tabs.get(tab).is_some_and(|tb| tb.targets.contains(&id));
        let group = format!("ws-pane-{id}");
        div()
            .id(gpui::ElementId::Name(group.clone().into()))
            .group(group.clone())
            .relative()
            .size_full()
            .rounded(px(10.0))
            .overflow_hidden()
            .border_1()
            .border_color(if target {
                t.warning.opacity(0.55)
            } else if focused && multi {
                t.wash(0.14)
            } else {
                t.wash(0.0)
            })
            .capture_any_mouse_down(cx.listener(move |this, _, _, cx| {
                if let Some(tb) = this.tabs.get_mut(tab)
                    && tb.focused != id
                {
                    tb.focused = id;
                    this.report(cx);
                    cx.notify();
                }
            }))
            .child(ui::rounded_clip(10.0, div().size_full().child(p.term.clone())))
            .when(broadcast, |d| {
                d.child(
                    div()
                        .id(gpui::ElementId::Name(format!("ws-pane-target-{id}").into()))
                        .absolute()
                        .bottom(px(8.0))
                        .right(px(8.0))
                        .h(px(20.0))
                        .px(px(7.0))
                        .flex()
                        .items_center()
                        .rounded(px(5.0))
                        .cursor_pointer()
                        .text_size(ty::MICRO)
                        .when(target, |d| d.bg(t.warning.opacity(0.18)).text_color(t.warning))
                        .when(!target, |d| d.bg(t.wash(0.06)).text_color(t.text_faint))
                        .tooltip(ui::tip("Include this pane in broadcast"))
                        .child(if target { "Broadcasting" } else { "Not included" })
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_target(tab, id, cx))),
                )
            })
            .child(
                div()
                    .id(gpui::ElementId::Name(format!("ws-pane-x-{id}").into()))
                    .absolute()
                    .top(px(6.0))
                    .right(px(6.0))
                    .size(px(20.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .cursor_pointer()
                    .invisible()
                    .group_hover(group, |s| s.visible())
                    .hover(|s| s.bg(t.wash(0.10)))
                    .tooltip(ui::tip("Close pane"))
                    .child(icon(icons::CLOSE).size(px(12.0)).text_color(t.text_faint))
                    .on_click(cx.listener(move |this, _, window, cx| this.close(tab, id, window, cx))),
            )
            .into_any_element()
    }
}

fn rekey(node: &mut Node, ids: &HashMap<PaneId, PaneId>) {
    match node {
        Node::Leaf(id) => {
            if let Some(n) = ids.get(id) {
                *id = *n;
            }
        }
        Node::Split { a, b, .. } => {
            rekey(a, ids);
            rekey(b, ids);
        }
    }
}

impl Workspace {
    /// Flock's tabs and tools, drawn in the window's title bar in place of
    /// a page title, so the page has no second bar of its own.
    pub fn render_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let open = self.menu.is_some();
        div()
            .h_full()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(4.0))
            .child(self.render_tabs(&t, cx))
            .child(div().flex_1())
            .child({
                let on = self.tabs.get(self.active).is_some_and(|tb| tb.broadcast);
                div()
                    .id("ws-broadcast")
                    .h(px(28.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .rounded(px(radius::RADIUS))
                    .cursor_pointer()
                    .text_size(ty::CAPTION)
                    .when(on, |d| d.bg(t.warning.opacity(0.16)).text_color(t.warning))
                    .when(!on, |d| d.text_color(t.text_muted).hover(|s| s.bg(t.wash(0.06))))
                    .tooltip(ui::tip("Type into every selected pane at once"))
                    .child(if on { "Broadcast on" } else { "Broadcast" })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_broadcast(cx)))
            })
            .child(
                div()
                    .relative()
                    .child(
                        div()
                            .id("ws-split")
                            .h(px(28.0))
                            .px(px(8.0))
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(5.0))
                            .rounded(px(radius::RADIUS))
                            .cursor_pointer()
                            .when(open, |d| d.bg(t.wash(0.08)))
                            .hover(|s| s.bg(t.wash(0.06)))
                            .tooltip(ui::tip("Split, grid or a new tab"))
                            .child(icon(icons::SPLIT_COLUMNS).size(px(15.0)).text_color(t.text_muted))
                            .child(icon(icons::ALT_ARROW_DOWN).size(px(11.0)).text_color(t.text_faint))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.menu = if this.menu.is_some() { None } else { Some(Menu::Split) };
                                cx.notify();
                            })),
                    )
                    .when(open, |d| d.child(self.render_menu(&t, cx))),
            )
            .into_any_element()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("Workspace");
        // Spawning panes and moving focus mid-frame trips GPUI's window
        // borrow; start after this frame instead.
        if !self.started {
            cx.defer_in(window, |this, window, cx| this.ensure_started(window, cx));
        }
        let t = Theme::of(cx).clone();
        let body = match self.tabs.get(self.active) {
            Some(tab) => {
                let tree = tab.tree.clone();
                self.render_node(self.active, &tree, Vec::new(), &t, cx)
            }
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(ty::BODY)
                .text_color(t.text_faint)
                .child("Your flock is empty. Open the split menu at the top right to start a shell or an agent.")
                .into_any_element(),
        };
        div()
            .id("workspace")
            .size_full()
            .flex()
            .flex_col()
            .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, _, cx| {
                let Some(d) = &this.drag else { return };
                if e.pressed_button != Some(MouseButton::Left) {
                    this.drag = None;
                    this.save();
                    return;
                }
                let b = d.area.get();
                let ratio = match d.axis {
                    Axis::Row => f32::from(e.position.x - b.origin.x) / f32::from(b.size.width).max(1.0),
                    Axis::Column => f32::from(e.position.y - b.origin.y) / f32::from(b.size.height).max(1.0),
                };
                let (tab, path) = (d.tab, d.path.clone());
                if let Some(tb) = this.tabs.get_mut(tab) {
                    tb.tree.set_ratio(&path, ratio);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    if this.drag.take().is_some() {
                        this.save();
                    }
                }),
            )
            .child(div().flex_1().min_h_0().p(px(GAP)).pt(px(2.0)).child(body))
    }
}
