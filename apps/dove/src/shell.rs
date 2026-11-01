//! The window: a top bar that never moves (thread tabs and panel toggles),
//! a collapsible sidebar, the routed page, and the thread's side panel.
//!
//! The shell owns app state (projects, threads, tabs, layout). Each thread is
//! its own cached entity, so a streaming thread re-renders only itself.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dove_core::harness_detect::Installed;
use dove_core::proto::{HarnessKind, ProjectId, TaskId, TaskStatus, Usage};
use dove_core::store::{Project, Task};
use dove_core::{Engine, Joined, NewTask};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, AnyView, AppContext as _, Bounds, ClipboardItem, Context, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, MouseButton, MouseMoveEvent, ParentElement as _, Pixels, Point, Render,
    SharedString, StatefulInteractiveElement as _, StyleRefinement, Styled as _, Subscription, Window,
    WindowControlArea, anchored, deferred, div, px, relative,
};
use gpui_base::input::{InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::composer::{Composer, ComposerEvent};
use crate::design::{radius, ty};
use crate::kit::{self, Toasts, Tone};
use crate::pages::PageState;
use crate::palette::{Palette, PaletteEvent, PaletteItem};
use crate::pr_view::PrTarget;
use crate::pull_requests::{PrEvent, PullRequestsPage};
use crate::remote::{Remote, RemoteCommand, RemoteEvent};
use crate::side_panel::{PaneCommand, SidePanel, SidePanelEvent, Tab};
use crate::theme::Theme;
use crate::thread::{TaskViewEventKind, ThreadEvent, ThreadView, ViewMode};
use crate::transcript::TaskView;
use crate::workspace::Zone;
use crate::workspace::{Axis, Dir};
use crate::{actions, notify, ui};

pub const SIDEBAR_W: f32 = 264.0;
/// The dock's default width (Zeron's surface host).
pub const PANEL_W: f32 = 520.0;
/// The chat column never gets narrower than this; the dock gives way first,
/// so the dock is at most the viewport less the sidebar and this.
const CENTER_MIN: f32 = 300.0;
/// Narrowest the dock gets.
const PANEL_FLOOR: f32 = 360.0;
/// The title bar's right-hand actions, as Zeron sizes them.
const HEADER_BUTTON: f32 = 28.0;
const HEADER_GAP: f32 = 4.0;
/// From the last action to the caption buttons or the window edge.
const BAR_EDGE_INSET: f32 = 6.0;
/// Zeron's session card: its width, its inset from the column's edge, the gap
/// it keeps from the reading column, and the gutters that column keeps.
const INFO_WIDTH: f32 = 272.0;
const INFO_INSET: f32 = 12.0;
const INFO_GAP: f32 = 24.0;
const READING_GUTTER: f32 = 48.0;

/// Room the chat holds free for the thread card, in a column `main` wide:
/// nothing while the centred reading column already clears it, else just
/// enough to shift the column left. `None` once that would narrow the column,
/// and the card floats over the chat instead. Zeron's `session_card_reserve`.
fn info_reserve(main: f32) -> Option<f32> {
    let reading = crate::design::conversation_width();
    let card_left = main - INFO_WIDTH - INFO_INSET;
    let reserve = (main + reading - 2.0 * (card_left - INFO_GAP)).max(0.0);
    (main - reserve - 2.0 * READING_GUTTER >= reading - 0.5).then_some(reserve)
}

fn harness_from_account(id: Option<&str>) -> HarnessKind {
    match id.unwrap_or("") {
        "claude" | "claude-code" => HarnessKind::ClaudeCode,
        "opencode" => HarnessKind::OpenCode,
        other => HarnessKind::from_id(other).unwrap_or(HarnessKind::ClaudeCode),
    }
}

fn synced_plain(message: &serde_json::Value) -> String {
    if let Some(parts) = message.get("parts").and_then(|p| p.as_array()) {
        let text: String = parts
            .iter()
            .filter(|part| part.get("kind").and_then(|k| k.as_str()) == Some("text"))
            .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("");
        if !text.is_empty() {
            return text;
        }
    }
    message.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn synced_has_structure(messages: &[serde_json::Value]) -> bool {
    messages.iter().any(|message| {
        message.get("parts").and_then(|p| p.as_array()).is_some_and(|parts| {
            parts.iter().any(|part| matches!(part.get("kind").and_then(|k| k.as_str()), Some("reasoning" | "tool")))
        })
    })
}

fn tool_from_part(part: &serde_json::Value) -> (String, String, serde_json::Value, String, Option<String>, bool) {
    if let Some(call) = part.get("call") {
        let kind = call.get("kind").and_then(|k| k.as_str()).unwrap_or("unknown");
        let title = match kind {
            "exec" => call.get("command"),
            "readFile" | "writeFile" | "editFile" | "applyPatch" => call.get("path"),
            "search" | "glob" => call.get("pattern"),
            "webFetch" => call.get("url"),
            "webSearch" => call.get("query"),
            "mcp" => call.get("tool"),
            _ => call.get("name"),
        }
        .and_then(|v| v.as_str())
        .unwrap_or(kind);
        let name = match kind {
            "exec" => "Execute",
            "readFile" => "Read",
            "writeFile" | "editFile" | "applyPatch" => "Edit",
            "search" | "glob" => "Search",
            "webFetch" | "webSearch" => "Fetch",
            "mcp" => call.get("tool").and_then(|v| v.as_str()).unwrap_or("tool"),
            _ => call.get("name").and_then(|v| v.as_str()).unwrap_or("tool"),
        };
        let id = part.get("id").and_then(|v| v.as_str()).unwrap_or("tool").to_string();
        let output = part.get("output").and_then(|v| v.as_str()).map(str::to_owned);
        let is_error = part.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
        return (name.to_string(), title.to_string(), call.clone(), id, output, is_error);
    }
    let name = part.get("name").and_then(|v| v.as_str()).unwrap_or("tool").to_string();
    let title = part.get("title").and_then(|v| v.as_str()).unwrap_or(&name).to_string();
    let id = part.get("callId").and_then(|v| v.as_str()).unwrap_or("tool").to_string();
    let input = part.get("input").cloned().unwrap_or(serde_json::Value::Null);
    let output = part.get("output").and_then(|v| v.as_str()).map(str::to_owned);
    let is_error = part.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
    (name, title, input, id, output, is_error)
}

fn events_from_synced(message: &serde_json::Value) -> Vec<dove_core::proto::AgentEvent> {
    use dove_core::proto::{AgentEvent, ToolKind};
    use std::sync::Arc;
    let role = message.get("role").and_then(|v| v.as_str()).unwrap_or("");
    if role == "user" {
        let text = synced_plain(message);
        if text.trim().is_empty() {
            return Vec::new();
        }
        return vec![AgentEvent::UserMessage { text: text.into() }];
    }
    if role != "assistant" {
        return Vec::new();
    }
    let mut events = Vec::new();
    if let Some(parts) = message.get("parts").and_then(|p| p.as_array()) {
        for part in parts {
            match part.get("kind").and_then(|k| k.as_str()) {
                Some("text") => {
                    let text: Arc<str> = part.get("text").and_then(|v| v.as_str()).unwrap_or("").into();
                    if !text.trim().is_empty() {
                        events.push(AgentEvent::TextDelta { text });
                    }
                }
                Some("reasoning") => {
                    let text: Arc<str> = part
                        .get("reasoning")
                        .or_else(|| part.get("text"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .into();
                    if !text.trim().is_empty() {
                        events.push(AgentEvent::ThinkingDelta { text });
                    }
                }
                Some("tool") => {
                    let (name, title, input, call_id, output, is_error) = tool_from_part(part);
                    events.push(AgentEvent::ToolCall {
                        call_id: call_id.clone(),
                        name: name.clone(),
                        kind: ToolKind::classify(&name),
                        title: title.into(),
                        input,
                    });
                    if let Some(output) = output {
                        events.push(AgentEvent::ToolResult { call_id, output: output.into(), is_error });
                    }
                }
                _ => {}
            }
        }
    } else {
        let text = synced_plain(message);
        if !text.trim().is_empty() {
            events.push(AgentEvent::TextDelta { text: text.into() });
        }
    }
    if !events.is_empty() {
        events.push(AgentEvent::BlockEnd);
        events.push(AgentEvent::TurnEnded { usage: None, is_error: false });
    }
    events
}

fn cloud_notice(state: &str) -> &'static str {
    match state {
        "sleeping" | "stopping" => "This session's Cloud machine is asleep — it wakes when you send",
        _ => "Starting a Cloud machine for this session…",
    }
}

/// Post-login sync wizard. The first-run card is a different question
/// (use this computer, or log in). This offer appears after sign-in.
#[derive(Clone, PartialEq)]
enum SyncOffer {
    Closed,
    /// Sign-in is open in the browser; this page moves on by itself.
    Waiting,
    Ready,
    Switching {
        bring: bool,
    },
    /// This computer's sessions going up to the account.
    Uploading {
        done: usize,
        total: usize,
        title: Option<String>,
    },
    Importing {
        done: usize,
        total: usize,
        title: Option<String>,
    },
    Done {
        imported: usize,
        skipped: usize,
        bring: bool,
    },
    Failed(String),
}

/// Shown once sign-in has reached the registry, until the user answers.
/// `first_run_seen` is not an input: staying local does not dismiss this.
fn sync_offer_due(signed_in: bool, connected: bool, seen: bool, postponed: bool) -> bool {
    signed_in && connected && !seen && !postponed
}

/// Zeron's offer line: what "Bring my work" would upload. `None` when this
/// computer has no local sessions, and the button becomes "Switch now".
fn local_work_phrase(sessions: usize) -> Option<String> {
    match sessions {
        0 => None,
        1 => Some("the 1 session".into()),
        n => Some(format!("the {n} sessions")),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AdoptKind {
    Imported,
    Skipped,
    Ignored,
}

/// A thread's picture is a baked frost (see `wallpaper_media`): blurred
/// until only colour remains, under a scrim that starts at this dim and
/// rises until text reads. Zeron's frosted-wallpaper recipe.
pub const CHAT_DIM_DARK: f32 = 0.78;
pub const CHAT_DIM_LIGHT: f32 = 0.82;
pub const CHAT_DIM_MIN: f32 = 0.55;
pub const CHAT_DIM_MAX: f32 = 0.95;
/// How much a GIF or video background is darkened: a light black veil so a
/// bright clip sits calmly behind the app, still plainly the clip itself.
pub const MOVING_DIM: f32 = 0.22;
/// The sidebar and dock over that picture: the same frost, a shade deeper.
const SIDE_ART_TINT: f32 = 0.24;
/// The sidebar's glass over the wallpaper's frost: enough to read as a
/// pane of tinted glass, light enough that the picture shows through.
const SIDE_GLASS_OVER_ART: f32 = 0.42;

/// Responsive fit, after Zeron's `horizontal_panel_fit`: which side columns
/// the window has room for, as (sidebar, dock). A column that does not fit
/// hides without closing (it keeps its width and its tabs) and returns once
/// the window is wide enough. The least recently opened column hides first,
/// except that an expanded dock (the user's takeover) outranks the sidebar.
/// Each column is `(open, opened_at)`.
#[cfg(test)]
pub(crate) fn panel_fit(
    viewport: f32,
    sidebar_w: f32,
    sidebar: (bool, u64),
    dock: (bool, u64),
    expanded: bool,
) -> (bool, bool) {
    panel_fit_floor(viewport, sidebar_w, sidebar, dock, PANEL_FLOOR, expanded)
}

/// [`panel_fit`] for a dock of `dock_floor`: the surfaces, the explorer or both.
pub(crate) fn panel_fit_floor(
    viewport: f32,
    sidebar_w: f32,
    sidebar: (bool, u64),
    dock: (bool, u64),
    dock_floor: f32,
    expanded: bool,
) -> (bool, bool) {
    let mut cols = [sidebar, dock];
    loop {
        let takeover = expanded && cols[1].0;
        let need = if cols[0].0 { sidebar_w } else { 0.0 }
            + if cols[1].0 { dock_floor } else { 0.0 }
            + if takeover { 0.0 } else { CENTER_MIN };
        if need <= viewport {
            return (cols[0].0, cols[1].0);
        }
        let victim = (0..2).filter(|&i| cols[i].0).min_by_key(|&i| (expanded && i == 1, cols[i].1));
        match victim {
            Some(i) => cols[i].0 = false,
            None => return (false, false),
        }
    }
}

/// A size-24 title-bar control (Zeron's `window_control_button`): the plate
/// fades in on hover.
fn quiet_bar_button(
    id: &'static str,
    label: &'static str,
    child: impl IntoElement,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(ui::BUTTON))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .bg(crate::motion::hover_blend(id, crate::theme::wash(0.0), crate::theme::wash(0.11)))
        .occlude()
        .role(gpui::Role::Button)
        .aria_label(label)
        .tooltip(ui::tip(label))
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_hover(crate::motion::hover_listener(id))
        .on_click(move |e, window, cx| {
            cx.stop_propagation();
            on_click(e, window, cx);
        })
        .child(child)
}

/// A size-28 action at the right of the title bar (Zeron's
/// `header_icon_button`): the thread card, files and panel toggles.
fn header_icon_button(
    id: &'static str,
    label: &'static str,
    child: impl IntoElement,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let key = format!("header-icon-{id}");
    div()
        .id(id)
        .size(px(HEADER_BUTTON))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .bg(crate::motion::hover_blend(&key, crate::theme::wash(0.0), crate::theme::wash(0.11)))
        .on_hover(crate::motion::hover_listener(key))
        .occlude()
        .role(gpui::Role::Button)
        .aria_label(label)
        .tooltip(ui::tip(label))
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(move |e, window, cx| {
            cx.stop_propagation();
            on_click(e, window, cx);
        })
        .child(child)
}

/// Recency of the side columns for [`panel_fit`].
#[derive(Default)]
struct Fit {
    seq: u64,
    sidebar_at: u64,
    panel_at: u64,
    /// Last frame's open flags, to stamp a column when it opens.
    sidebar_was: bool,
    panel_was: bool,
    /// This frame's verdict: (sidebar shown, dock shown).
    shown: (bool, bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Home,
    Thread(TaskId),
    /// Gone as a page: it opens Search on Threads (`navigate`).
    Threads,
    Artifacts,
    Kanban,
    Automations,
    PullRequests,
    Agents,
    Usage,
    Archived,
    Settings,
    Help,
    Team,
    Flock,
    /// Race and compare: past races, or one race's compare view.
    Races,
}

impl Route {
    /// A page by its short name (`DOVE_ROUTE`, deep links).
    pub fn from_id(id: &str) -> Option<Route> {
        Some(match id {
            "home" => Route::Home,
            "threads" => Route::Threads,
            "artifacts" => Route::Artifacts,
            "kanban" => Route::Kanban,
            "automations" => Route::Automations,
            "prs" | "pull-requests" => Route::PullRequests,
            "agents" => Route::Agents,
            "usage" => Route::Usage,
            "archived" => Route::Archived,
            "settings" => Route::Settings,
            "help" => Route::Help,
            "team" => Route::Team,
            "flock" | "terminals" => Route::Flock,
            "races" | "race" => Route::Races,
            _ => return None,
        })
    }
}

#[derive(Clone)]
pub struct ChatMenu {
    pub task: TaskId,
    pub at: Point<Pixels>,
    pub copy_open: bool,
    /// The delete row waiting for its second click.
    pub confirm_delete: Option<DeleteScope>,
}

/// Where a session is deleted from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeleteScope {
    /// This computer. A copy on the account stays there.
    Local,
    /// The account, on every device. This computer's copy stays.
    Cloud,
    Everywhere,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edge {
    Sidebar,
    Panel,
}

enum BrowserCall {
    Done(crate::browser::agent::Outcome),
    Run(Entity<crate::browser::BrowserSurface>),
}

pub struct Shell {
    pub engine: Arc<Engine>,
    pub projects: Vec<Project>,
    /// Live and draft threads (not archived).
    pub tasks: Vec<Task>,
    pub archived: Vec<Task>,
    /// The team, and which teammate runs each thread.
    pub personas: Vec<dove_core::store::Persona>,
    pub persona_of: HashMap<TaskId, String>,
    pub home_project: Option<ProjectId>,
    pub join: Option<(TaskId, Joined)>,
    pub route: Route,
    back: Vec<Route>,
    forward: Vec<Route>,
    pub threads: HashMap<TaskId, Entity<ThreadView>>,
    /// The voice call, while one runs (Voice Mode; see `voice_call.rs`).
    pub voice_call: Option<Entity<crate::voice_call::VoiceCall>>,
    pub(crate) voice_sub: Option<Subscription>,
    /// The full-window call view, and whether it shows (else the chat).
    pub(crate) call_view: Option<Entity<crate::voice_view::CallView>>,
    pub(crate) call_view_sub: Option<Subscription>,
    pub(crate) call_view_shown: bool,
    /// Open thread tabs, in order.
    pub tabs: Vec<TaskId>,
    pub sidebar_open: bool,
    pub sidebar_w: f32,
    pub panel_open: bool,
    /// The explorer column at the dock's right edge, toggled on its own.
    explorer_open: bool,
    pub panel_w: f32,
    resize: Option<(Edge, Pixels, f32)>,
    /// The limit the current drag presses against, and the seam's nudge
    /// when it first reaches one.
    resize_pressing: Option<crate::motion::ResizeEdge>,
    resize_bounce: Option<(Edge, crate::motion::ResizeEdge, Instant)>,
    pub panel: Entity<SidePanel>,
    pub composer: Entity<Composer>,
    pub composer_bounds: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
    /// The home composer's measured height, for embedding it cached.
    pub composer_height: std::rc::Rc<std::cell::Cell<f32>>,
    pub prs: Entity<PullRequestsPage>,
    /// Flock: agents' TUIs and shells in split panes and grids.
    pub workspace: Entity<crate::flock::Workspace>,
    /// Each thread's pull request URL (mirrors the store's `task_prs`).
    pr_links: HashMap<TaskId, String>,
    pub settings: Entity<crate::settings::SettingsPage>,
    pub palette: Option<Entity<Palette>>,
    pub menu: Option<ChatMenu>,
    pub rename: Option<(TaskId, Entity<InputState>)>,
    pub info_open: bool,
    /// The sidebar row under the pointer, and the one whose hover card is
    /// allowed to show (the card waits ~550 ms so a sweep does not flash).
    pub hover_task: Option<dove_core::proto::TaskId>,
    pub hover_ready: Option<dove_core::proto::TaskId>,
    pub more_open: bool,
    pub sidebar_options_open: bool,
    /// The sidebar footer's account menu.
    pub account_open: bool,
    /// The "+N" menu of tabs that do not fit the strip.
    pub tab_overflow_open: bool,
    /// The top bar's ⋯ menu of dock tabs.
    pub dock_more_open: bool,
    /// The wallpaper's dominant colour, per path (`None` inside while it is
    /// being worked out, or when the picture has none).
    wallpaper_hue: Option<(String, Option<gpui::Hsla>)>,
    /// Keyboard and exit state of the shell's own menus.
    pub menus: crate::shell_menu::Menus,
    pub agents_open: bool,
    pub installed: Vec<Installed>,
    pub versions: HashMap<HarnessKind, String>,
    /// Installed agents with a newer release.
    pub updates: Vec<crate::updates::Update>,
    pub today: Usage,
    pub error: Option<SharedString>,
    pub pages: PageState,
    /// Races: their records, the start sheet and the compare view's state.
    pub race: crate::race::RaceState,
    /// Unread automation runs, for the sidebar dot.
    pub inbox_unread: usize,
    pub(crate) focus: FocusHandle,
    titlebar_drag: bool,
    window_active: bool,
    /// The thread a background notification was about, opened when the
    /// window comes back within a minute.
    pending_focus: Option<(TaskId, Instant)>,
    /// Sidebar and dock open/close transitions.
    slides: Slides,
    /// Which side columns fit the window.
    fit: Fit,
    /// The thread list, as a cached view (made on first render).
    sidebar_view: Option<Entity<crate::sidebar::Sidebar>>,
    _subs: Vec<Subscription>,
    _scheduler: Option<gpui::Task<()>>,
    _browser_control: Option<gpui::Task<()>>,
    /// Presses of Voice Mode's system-wide shortcut.
    _voice_hotkey: Option<gpui::Task<()>>,
    /// Ctrl+Alt+Shift+S (Windows): take or hand back computer control.
    _computer_hotkey: Option<gpui::Task<()>>,
    /// Delegation and teammate messaging between threads (`orchestra.rs`).
    pub(crate) team: crate::orchestra::TeamState,
    /// Threads beside each open thread in the chat area (`split.rs`).
    pub(crate) companions: HashMap<TaskId, Vec<TaskId>>,
    /// The post-login "Sync is ready" wizard.
    sync_offer: SyncOffer,
    /// "Later" hides it until the next launch, or until the account menu reopens it.
    pub(crate) sync_postponed: bool,
    was_signed_in: bool,
    /// Account chats that arrived before the user answered the offer.
    pending_account: Option<(String, serde_json::Value)>,
    /// Sessions "Bring my work" sent up, for the wizard's last page.
    sync_uploaded: usize,
    sync_importing: bool,
    sync_import_gen: u64,
}

impl Shell {
    pub fn new(engine: Arc<Engine>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| Composer::home(engine.clone(), window, cx));
        let composer_height = std::rc::Rc::new(std::cell::Cell::new(0.0));
        let host = cx.entity_id();
        composer.update(cx, |c, _| c.set_cache_slot(composer_height.clone(), host));
        let panel = cx.new(|cx| {
            let mut p = SidePanel::new(engine.clone(), window, cx);
            p.expanded = engine.store.setting("panel_expanded").as_deref() == Some("true");
            p.dock_left = engine.store.setting("panel_side").as_deref() == Some("left");
            p
        });
        // Composers list the in-app browser's tabs for `@` mentions.
        cx.set_global(crate::side_panel::PanelHandle(panel.downgrade()));
        let prs = cx.new(|cx| PullRequestsPage::new(engine.clone(), window, cx));
        let workspace = cx.new(|cx| crate::flock::Workspace::new(engine.clone(), cx));
        let settings = cx.new(|cx| crate::settings::SettingsPage::new(engine.clone(), window, cx));
        let mut subs = Vec::new();
        subs.push(cx.subscribe(&workspace, |_, _, ev: &crate::flock::WorkspaceEvent, cx| match ev {
            crate::flock::WorkspaceEvent::AgentsChanged => cx.notify(),
        }));
        subs.push(cx.observe(&workspace, |this, _, cx| {
            if this.route == Route::Flock {
                cx.notify();
            }
        }));
        subs.push(cx.subscribe_in(&composer, window, |this, _, ev: &ComposerEvent, window, cx| match ev {
            ComposerEvent::Submit(text) => this.start(text.clone(), window, cx),
            ComposerEvent::OpenUsage => this.navigate(Route::Usage, window, cx),
            ComposerEvent::OpenVoiceSettings => this.open_settings_section("voice", window, cx),
            ComposerEvent::OpenMachines => this.open_settings_section("machines", window, cx),
            ComposerEvent::ProjectChanged(id) => this.set_home_project(*id, cx),
            ComposerEvent::ProjectsChanged => this.reload(cx),
            ComposerEvent::Race(text) => this.open_race_sheet(text.clone(), window, cx),
            ComposerEvent::Stop => {}
        }));
        subs.push(cx.subscribe_in(&panel, window, |this, _, ev: &SidePanelEvent, window, cx| match ev {
            SidePanelEvent::SendToAgent(text) => {
                if let Some(v) = this.active_thread() {
                    let chat = v.read(cx).chat.clone();
                    let text = text.clone();
                    chat.update(cx, |c, cx| c.submit(text, window, cx));
                }
            }
            SidePanelEvent::Dock(left) => {
                let _ = this.engine.store.set_setting("panel_side", if *left { "left" } else { "right" });
                cx.notify();
            }
            SidePanelEvent::ShowExplorer => this.set_explorer(true, cx),
            SidePanelEvent::ShowBody => {
                this.panel_open = true;
                let _ = this.engine.store.set_setting("panel_open", "true");
                cx.notify();
            }
            SidePanelEvent::Close => {
                this.panel_open = false;
                let _ = this.engine.store.set_setting("panel_open", "false");
                cx.notify();
            }
            SidePanelEvent::Layout => {
                let on = this.panel.read(cx).expanded;
                let _ = this.engine.store.set_setting("panel_expanded", if on { "true" } else { "false" });
                cx.notify();
            }
            SidePanelEvent::RevealRow(row) => {
                if let Some(v) = this.active_thread() {
                    let chat = v.read(cx).chat.clone();
                    let row = *row;
                    chat.update(cx, |c, cx| c.glide_to_row(row, cx));
                }
            }
            SidePanelEvent::OpenThread(id) => this.open_task(*id, window, cx),
            SidePanelEvent::StageForAgent(text) => {
                if let Some(v) = this.active_thread() {
                    let composer = v.read(cx).chat.read(cx).composer.clone();
                    crate::composer::set_text(&composer, text, window, cx);
                }
            }
            SidePanelEvent::CheckoutPr { project, number, title } => {
                this.checkout_pr(project.clone(), *number, title.clone(), window, cx)
            }
            SidePanelEvent::PrChanged => cx.notify(),
            SidePanelEvent::DropThread { id, pane, zone } => this.open_beside(*id, Some((*pane, *zone)), window, cx),
        }));
        subs.push(cx.subscribe_in(
            &settings,
            window,
            |this, _, ev: &crate::settings::SettingsEvent, window, cx| match ev {
                crate::settings::SettingsEvent::SignIn(kind) => this.sign_in(*kind, cx),
                crate::settings::SettingsEvent::Rescan => this.rescan(cx),
                crate::settings::SettingsEvent::Close => this.close_settings(window, cx),
                crate::settings::SettingsEvent::WipeSessions { local, cloud } => {
                    this.wipe_sessions(*local, *cloud, window, cx)
                }
            },
        ));
        // The shell draws the settings page's parts itself (its sections in
        // the sidebar slot, the section in the card), so it repaints when the
        // page changes.
        subs.push(cx.observe(&settings, |_, _, cx| cx.notify()));
        subs.push(cx.subscribe_in(&prs, window, |this, _, ev: &PrEvent, window, cx| match ev {
            PrEvent::Checkout { project, number, title } => {
                this.checkout_pr(project.clone(), *number, title.clone(), window, cx)
            }
            PrEvent::HandOff { project, prompt } => this.stage_new_thread(project.clone(), prompt, window, cx),
        }));
        if let Some(remote) = crate::remote::Remote::global(cx) {
            subs.push(cx.subscribe_in(&remote, window, |this, remote, ev: &RemoteEvent, window, cx| {
                this.on_remote(remote.clone(), ev, window, cx)
            }));
        }
        subs.push(cx.observe_window_activation(window, |this, window, cx| {
            this.window_active = window.is_window_active();
            crate::motion::window_activation_changed(this.window_active, cx);
            if !this.window_active {
                crate::memory::watchdog(this, Some(window), cx);
            }
            if this.window_active
                && let Some((id, at)) = this.pending_focus.take()
                && at.elapsed() < Duration::from_secs(60)
            {
                this.open_task(id, window, cx);
            }
        }));
        let store = &engine.store;
        let projects = store.projects().unwrap_or_default();
        let home_project = store
            .setting("active_project")
            .and_then(|id| projects.iter().find(|p| p.id.to_string() == id).map(|p| p.id))
            .or_else(|| projects.first().map(|p| p.id));
        let num = |key: &str, default: f32, lo: f32, hi: f32| {
            store.setting(key).and_then(|s| s.parse::<f32>().ok()).unwrap_or(default).clamp(lo, hi)
        };
        let pages = PageState::new(&engine, window, cx);
        let mut this = Self {
            sidebar_open: store.setting("sidebar_open").as_deref() != Some("false"),
            sidebar_w: num("sidebar_width", SIDEBAR_W, 208.0, 400.0),
            panel_open: store.setting("panel_open").as_deref() == Some("true"),
            // `DOVE_EXPLORER=1` opens the explorer column.
            explorer_open: std::env::var_os("DOVE_EXPLORER").is_some()
                || store.setting("explorer_open").as_deref() == Some("true"),
            panel_w: num("panel_width", PANEL_W, PANEL_FLOOR, 4000.0),
            resize: None,
            resize_pressing: None,
            resize_bounce: None,
            projects,
            tasks: Vec::new(),
            archived: Vec::new(),
            personas: Vec::new(),
            persona_of: HashMap::new(),
            home_project,
            join: None,
            route: Route::Home,
            back: Vec::new(),
            forward: Vec::new(),
            threads: HashMap::new(),
            voice_call: None,
            voice_sub: None,
            call_view: None,
            call_view_sub: None,
            call_view_shown: false,
            tabs: store
                .setting("open_tabs")
                .filter(|_| store.setting("restore_tabs").as_deref() != Some("false"))
                .map(|s| s.split(',').filter_map(uuid_task).collect())
                .unwrap_or_default(),
            panel,
            composer,
            composer_bounds: Default::default(),
            composer_height,
            prs,
            workspace,
            pr_links: store.task_prs().unwrap_or_default().into_iter().collect(),
            settings,
            palette: None,
            menu: None,
            rename: None,
            info_open: false,
            hover_task: None,
            hover_ready: None,
            more_open: false,
            sidebar_options_open: false,
            account_open: false,
            tab_overflow_open: false,
            dock_more_open: false,
            wallpaper_hue: None,
            menus: crate::shell_menu::Menus::new(cx),
            agents_open: false,
            installed: Vec::new(),
            versions: HashMap::new(),
            updates: Vec::new(),
            today: Usage::default(),
            error: None,
            pages,
            race: Default::default(),
            inbox_unread: 0,
            focus: cx.focus_handle(),
            titlebar_drag: false,
            window_active: true,
            pending_focus: None,
            slides: Slides::default(),
            fit: Fit { shown: (true, true), ..Fit::default() },
            sidebar_view: None,
            engine,
            _subs: subs,
            _scheduler: None,
            _browser_control: None,
            _voice_hotkey: None,
            _computer_hotkey: None,
            team: Default::default(),
            companions: HashMap::new(),
            sync_offer: SyncOffer::Closed,
            sync_postponed: false,
            was_signed_in: false,
            pending_account: None,
            sync_uploaded: 0,
            sync_importing: false,
            sync_import_gen: 0,
        };
        this.reload(cx);
        this.tabs.retain(|id| this.tasks.iter().any(|t| t.id == *id));
        this.scan_harnesses(cx);
        this.start_scheduler(cx);
        this.start_browser_control(window, cx);
        this.start_team_sweep(window, cx);
        this.start_voice_hotkey(window, cx);
        this.start_computer_hotkey(window, cx);
        crate::composer::focus(&this.composer, window, cx);
        this.consider_sync_offer(cx);
        // `DOVE_OPEN_THREAD=<id>`: start on a thread (QA and scripting).
        if let Some(id) = std::env::var("DOVE_OPEN_THREAD").ok().and_then(|s| uuid_task(&s)) {
            this.open_task(id, window, cx);
            // `DOVE_TERMINAL=1`: show it in the agent's terminal UI.
            if std::env::var("DOVE_TERMINAL").is_ok_and(|v| v == "1") {
                this.toggle_view_mode(window, cx);
            }
        }
        // `DOVE_STRESS_THREADS=<rounds>`: open and close threads in turn, to
        // watch memory across many switches (QA of leaks).
        if let Some(rounds) = std::env::var("DOVE_STRESS_THREADS").ok().and_then(|v| v.parse::<usize>().ok()) {
            let ids: Vec<TaskId> = this.tasks.iter().take(24).map(|t| t.id).collect();
            cx.spawn_in(window, async move |this, cx| {
                for _ in 0..rounds {
                    for id in &ids {
                        cx.background_executor().timer(Duration::from_millis(350)).await;
                        let _ = this.update_in(cx, |this, window, cx| this.open_task(*id, window, cx));
                        cx.background_executor().timer(Duration::from_millis(350)).await;
                        let _ = this.update_in(cx, |this, window, cx| this.close_tab(*id, window, cx));
                    }
                }
            })
            .detach();
        }
        // `DOVE_BESIDE=<id>[,<id>…]`: open threads beside the open one, as a
        // drop on the conversation would (QA of split view).
        if let Ok(list) = std::env::var("DOVE_BESIDE") {
            for id in list.split(',').filter_map(uuid_task) {
                this.split_with(id, window, cx);
            }
        }
        // `DOVE_ROUTE=<page>`: start on a page (QA and scripting).
        if let Some(route) = std::env::var("DOVE_ROUTE").ok().and_then(|r| Route::from_id(&r)) {
            this.navigate(route, window, cx);
        }
        // `DOVE_PANEL=<tab>`: open the side panel on a tab.
        if let Some(tab) = std::env::var("DOVE_PANEL").ok().and_then(|r| Tab::from_id(&r)) {
            this.show_panel(tab, window, cx);
        }
        if let Ok(section) = std::env::var("DOVE_SETTINGS") {
            this.open_settings_section(&section, window, cx);
        }
        // `DOVE_ATTACH=<path>[|<path>…]`: stage files in the visible
        // composer (QA of the attachment strip).
        if let Ok(spec) = std::env::var("DOVE_ATTACH") {
            let paths: Vec<PathBuf> = spec.split(['|', ';']).filter(|p| !p.is_empty()).map(PathBuf::from).collect();
            let composer = match this.active_thread() {
                Some(v) => v.read(cx).chat.read(cx).composer.clone(),
                None => this.composer.clone(),
            };
            crate::composer::attach_paths(&composer, paths, cx);
        }
        // `DOVE_SEED_TEAM=1`: add the starter team (QA of Team).
        if std::env::var("DOVE_SEED_TEAM").is_ok_and(|v| v == "1") {
            this.seed_starter_team(cx);
        }
        // `DOVE_PROMPT=<text>`: start a thread from Home on launch, on
        // `DOVE_PROMPT_AGENT` / `DOVE_PROMPT_MODEL` in `DOVE_PROMPT_MODE`
        // (QA of agent-driven features, like the team tools).
        if let Ok(prompt) = std::env::var("DOVE_PROMPT")
            && !prompt.trim().is_empty()
        {
            let agent = std::env::var("DOVE_PROMPT_AGENT").ok().and_then(|a| HarnessKind::from_id(&a));
            let model = std::env::var("DOVE_PROMPT_MODEL").ok().filter(|m| !m.is_empty());
            let mode = std::env::var("DOVE_PROMPT_MODE").ok().map(|m| crate::composer::mode_from_id(&m));
            this.composer.update(cx, |c, cx| {
                if let Some(kind) = agent {
                    c.use_harness(kind, model, cx);
                }
                if let Some(mode) = mode {
                    c.set_mode(mode, cx);
                }
            });
            crate::composer::set_text(&this.composer, &prompt, window, cx);
            this.composer.update(cx, |c, cx| c.submit(window, cx));
        }
        // `DOVE_INFO=1`: open the thread's info card (QA).
        if std::env::var("DOVE_INFO").is_ok_and(|v| v == "1") {
            this.info_open = true;
        }
        // `DOVE_QA_DELETE=wipe-local|wipe-cloud|wipe-both|local|cloud|both|menu`:
        // what Settings' mass delete and a session's menu do, on the first
        // listed thread where it takes one (QA, on a scratch home).
        if let Ok(what) = std::env::var("DOVE_QA_DELETE") {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(3)).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    let first = this.tasks.first().map(|t| t.id);
                    match (what.as_str(), first) {
                        ("wipe-local", _) => this.wipe_sessions(true, false, window, cx),
                        ("wipe-cloud", _) => this.wipe_sessions(false, true, window, cx),
                        ("wipe-both", _) => this.wipe_sessions(true, true, window, cx),
                        ("local", Some(id)) => this.delete_session(id, DeleteScope::Local, window, cx),
                        ("cloud", Some(id)) => this.delete_session(id, DeleteScope::Cloud, window, cx),
                        ("both", Some(id)) => this.delete_session(id, DeleteScope::Everywhere, window, cx),
                        ("menu", Some(id)) => {
                            this.menu = Some(ChatMenu {
                                task: id,
                                at: gpui::point(px(180.0), px(420.0)),
                                copy_open: false,
                                confirm_delete: None,
                            });
                            cx.notify();
                        }
                        _ => {}
                    }
                });
            })
            .detach();
        }
        // `DOVE_PICKER=1`: open the visible composer's model picker (QA).
        if std::env::var("DOVE_PICKER").is_ok_and(|v| v == "1") {
            let composer = match this.active_thread() {
                Some(v) => v.read(cx).chat.read(cx).composer.clone(),
                None => this.composer.clone(),
            };
            let picker = composer.read(cx).picker.clone();
            picker.update(cx, |p, cx| p.toggle(window, cx));
        }
        this
    }

    // ------------------------------------------------------------ data

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let store = &self.engine.store;
        self.projects = store.projects().unwrap_or_default();
        self.tasks = store.all_tasks().unwrap_or_default();
        self.archived = store.archived_tasks(200).unwrap_or_default();
        self.personas = store.personas().unwrap_or_default();
        self.persona_of = store.persona_assignments().unwrap_or_default().into_iter().collect();
        let personas = self.personas.clone();
        self.composer.update(cx, |c, cx| c.set_personas(personas, cx));
        // A live view knows its status better than the database row.
        for t in &mut self.tasks {
            if let Some(v) = self.threads.get(&t.id) {
                t.status = v.read(cx).chat.read(cx).status;
            }
            if self.route == Route::Thread(t.id) {
                t.unread = false;
            }
        }
        let midnight = dove_core::store::now_ms() - dove_core::store::now_ms() % 86_400_000;
        let mut total = Usage::default();
        for (_, u) in store.usage_by_harness(midnight).unwrap_or_default() {
            total += u;
        }
        self.today = total;
        self.inbox_unread = store.runs(100).unwrap_or_default().iter().filter(|r| r.unread).count();
        let preferred = self.home_project.and_then(|id| self.project(id)).cloned();
        let projects = self.projects.clone();
        self.prs.update(cx, |p, cx| p.set_projects(projects.clone(), preferred.as_ref(), cx));
        let home = self.home_project;
        self.composer.update(cx, |c, cx| c.set_projects(projects, home, cx));
        cx.notify();
    }

    fn scan_harnesses(&mut self, cx: &mut Context<Self>) {
        // PATH lookups are instant; show the agents right away. Versions come
        // from a cache keyed by each binary's size and modified time, so an
        // unchanged agent costs no process start; a new or updated one is
        // probed with `--version`, one at a time, once launch has settled.
        let engine = self.engine.clone();
        let scan = cx.background_spawn(async move { engine.rescan_harnesses() });
        cx.spawn(async move |this, cx| {
            let found = scan.await;
            let probes: Vec<_> = found.iter().map(|i| (i.kind, i.program.clone())).collect();
            let Ok(store) = this.update(cx, |s, cx| {
                s.installed = found.clone();
                let enabled: Vec<_> =
                    found.into_iter().filter(|i| crate::pages::agent_enabled(&s.engine.store, i.kind)).collect();
                s.composer.update(cx, |c, cx| c.set_installed(enabled, cx));
                cx.notify();
                s.engine.store.clone()
            }) else {
                return;
            };
            let mut known = Vec::new();
            let mut settled = false;
            for (kind, program) in probes {
                let cached = {
                    let (store, program) = (store.clone(), program.clone());
                    cx.background_spawn(async move { cached_version(&store, kind, &program) }).await
                };
                let version = match cached {
                    Some(v) => Some(v),
                    None => {
                        if !settled {
                            cx.background_executor().timer(VERSION_PROBE_DELAY).await;
                            settled = true;
                        }
                        let store = store.clone();
                        cx.background_spawn(async move {
                            let v = version_of(&program);
                            if let Some(v) = &v {
                                remember_version(&store, kind, &program, v);
                            }
                            v
                        })
                        .await
                    }
                };
                if let Some(v) = version {
                    known.push((kind, v.clone()));
                    let _ = this.update(cx, |s, cx| {
                        s.versions.insert(kind, v);
                        cx.notify();
                    });
                }
            }
            // With every version known, ask each agent's release channel
            // whether there is a newer one.
            let check = crate::browser::runtime().spawn(crate::updates::check(known));
            if let Ok(updates) = check.await {
                let _ = this.update(cx, |s, cx| {
                    s.updates = updates;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Update an agent in a terminal on the Agents page, where its output
    /// (and any prompt it asks) is visible.
    pub fn run_update(&mut self, kind: HarnessKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(program) = self.engine.program_for(kind) else { return };
        let (program, args) = crate::updates::command(kind, &program);
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(|| ".".into());
        let term = cx.new(|cx| crate::terminal::TerminalView::for_command(home, program, args, cx));
        self.pages.agent_terminal = Some((kind, term));
        self.updates.retain(|u| u.kind != kind);
        self.agents_open = false;
        // The update runs in the sign-in sheet, over whatever page is open.
        cx.notify();
        let _ = window;
    }

    /// Start or end a call from the system-wide shortcut (Settings › Voice).
    /// The presses go to the island host (`island::host`), which starts the
    /// notch's call while the notch runs and this window's otherwise.
    fn start_voice_hotkey(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        crate::hotkey::apply(&self.engine.store);
    }

    fn start_computer_hotkey(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let presses = crate::computer_review::hotkey_events();
        self._computer_hotkey = Some(cx.spawn_in(window, async move |this, cx| {
            while presses.recv_async().await.is_ok() {
                if this.update(cx, |_, cx| crate::computer_review::on_hotkey(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    /// `computer_*` from an agent's `computer` tool: approval cards in its
    /// thread, the take-over lease and the activity strip.
    fn route_computer_call(
        &mut self,
        req: &crate::browser::control::Request,
        cx: &mut Context<Self>,
    ) -> crate::computer_review::Reply {
        let visible = match self.route {
            Route::Thread(id) => Some(id),
            _ => None,
        };
        let thread = self.team_caller(req).ok().or(visible);
        crate::computer_review::route(&req.tool, &req.args, thread, cx)
    }

    /// Serve agents' browser tool calls (`dove --mcp-browser`): route each
    /// to the browser of the thread it came from, and run it there.
    fn start_browser_control(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(requests) = crate::browser::control::take_requests() else { return };
        self._browser_control = Some(cx.spawn_in(window, async move |this, cx| {
            while let Ok(req) = requests.recv_async().await {
                if crate::computer_review::is_gui_tool(&req.tool) {
                    match this.update(cx, |s, cx| s.route_computer_call(&req, cx)) {
                        Ok(crate::computer_review::Reply::Now(outcome)) => {
                            let _ = req.reply.send(outcome);
                        }
                        Ok(crate::computer_review::Reply::Wait(decided)) => {
                            cx.spawn(async move |_| {
                                let d = decided
                                    .await
                                    .unwrap_or(dove_computer::Decision::Deny { reason: "the card was closed".into() });
                                let _ = req.reply.send(crate::computer_review::decision_outcome(&d));
                            })
                            .detach();
                        }
                        Err(_) => break,
                    }
                    continue;
                }
                if crate::panel_tools::is_panel_tool(&req.tool) {
                    let reply = this.update_in(cx, |s, window, cx| s.route_panel_call(&req, window, cx));
                    match reply {
                        Ok(outcome) => {
                            let _ = req.reply.send(outcome);
                        }
                        Err(_) => break,
                    }
                    continue;
                }
                if crate::team_tools::is_team_tool(&req.tool) {
                    let reply = this.update_in(cx, |s, window, cx| s.route_team_call(&req, window, cx));
                    match reply {
                        Ok(crate::orchestra::TeamReply::Now(outcome)) => {
                            let _ = req.reply.send(outcome);
                        }
                        Ok(crate::orchestra::TeamReply::Wait(waiter)) => {
                            let weak = this.clone();
                            cx.spawn(async move |cx| {
                                let outcome = crate::orchestra::wait(weak, waiter, cx).await;
                                let _ = req.reply.send(outcome);
                            })
                            .detach();
                        }
                        Err(_) => break,
                    }
                    continue;
                }
                let routed = this.update_in(cx, |s, window, cx| s.route_browser_call(&req, window, cx));
                match routed {
                    Ok(Ok(BrowserCall::Done(outcome))) => {
                        let _ = req.reply.send(outcome);
                    }
                    Ok(Ok(BrowserCall::Run(browser))) => {
                        let label = crate::browser::agent::activity(&req.tool, &req.args);
                        browser.update(cx, |b, cx| b.set_agent_activity(Some(label), cx));
                        let weak = browser.downgrade();
                        cx.spawn(async move |cx| {
                            let outcome = crate::browser::agent::run(weak.clone(), &req.tool, req.args, cx).await;
                            let _ = weak.update(cx, |b, cx| b.set_agent_activity(None, cx));
                            let _ = req.reply.send(outcome);
                        })
                        .detach();
                    }
                    Ok(Err(e)) => {
                        let _ = req.reply.send(crate::browser::agent::Outcome::err(e));
                    }
                    Err(_) => break,
                }
            }
        }));
    }

    /// `panel_*` from an agent: its thread's side panel.
    fn route_panel_call(
        &mut self,
        req: &crate::browser::control::Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> crate::browser::agent::Outcome {
        use crate::browser::agent::Outcome;
        let visible = match self.route {
            Route::Thread(id) => Some(id),
            _ => None,
        };
        let Some(thread) = self.team_caller(req).ok().or(visible) else {
            return Outcome::err("Keplorayx couldn't tell which thread this call comes from.");
        };
        let shown = Some(thread) == visible;
        match req.tool.as_str() {
            "panel_open" => {
                if !self.engine.store.flag("agents_open_panels", true) {
                    return Outcome::err(
                        "The user turned off agents opening the side panel (Settings › General). Tell them what to look at instead.",
                    );
                }
                let (asks, layout) = match crate::panel_tools::parse_open(&req.args) {
                    Ok(v) => v,
                    Err(e) => return Outcome::err(e),
                };
                if shown {
                    self.panel_open = true;
                    let _ = self.engine.store.set_setting("panel_open", "true");
                }
                let text = self.panel.update(cx, |p, cx| p.agent_open(thread, asks, layout, shown, window, cx));
                cx.notify();
                Outcome::ok(text)
            }
            "panel_state" => {
                let open = self.panel_open;
                let state = self.panel.read(cx).agent_state(thread, open && shown, cx);
                Outcome::ok(serde_json::to_string_pretty(&state).unwrap_or_default())
            }
            "panel_close" => {
                if req.args.get("all").and_then(serde_json::Value::as_bool) == Some(true) {
                    if shown {
                        self.panel_open = false;
                        cx.notify();
                    }
                    return Outcome::ok("The side panel is hidden.");
                }
                let Some(index) = req.args.get("index").and_then(serde_json::Value::as_u64) else {
                    return Outcome::err("Give a tab \"index\" from panel_state, or all: true.");
                };
                match self.panel.update(cx, |p, cx| p.agent_close(thread, index as usize, window, cx)) {
                    Ok(t) => Outcome::ok(t),
                    Err(e) => Outcome::err(e),
                }
            }
            other => Outcome::err(format!("Unknown panel tool {other}.")),
        }
    }

    fn route_browser_call(
        &mut self,
        req: &crate::browser::control::Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<BrowserCall, String> {
        let visible = match self.route {
            Route::Thread(id) => Some(id),
            _ => None,
        };
        let same = |a: &std::path::Path, b: &std::path::Path| {
            let norm = |p: &std::path::Path| {
                let s = p.display().to_string().replace('\\', "/");
                let s = s.trim_end_matches('/').to_string();
                if cfg!(windows) { s.to_ascii_lowercase() } else { s }
            };
            norm(a) == norm(b)
        };
        let dir_of = |t: &Task| t.worktree.clone().or_else(|| self.project(t.project_id).map(|p| p.path.clone()));
        let thread = req
            .cwd
            .as_ref()
            .and_then(|cwd| {
                let matching: Vec<&Task> =
                    self.tasks.iter().filter(|t| dir_of(t).is_some_and(|d| same(&d, cwd))).collect();
                matching
                    .iter()
                    .find(|t| Some(t.id) == visible)
                    .or_else(|| matching.iter().find(|t| t.status == TaskStatus::Running))
                    .or_else(|| matching.iter().max_by_key(|t| t.updated_at))
                    .map(|t| t.id)
            })
            .or(visible)
            .ok_or("Open a Keplorayx thread to use its browser.")?;
        let shown = Some(thread) == visible;
        if req.tool == "browser_tabs" {
            let action = req.args.get("action").and_then(serde_json::Value::as_str).unwrap_or("list").to_string();
            let index = req.args.get("index").and_then(serde_json::Value::as_u64).map(|i| i as usize);
            let url = req.args.get("url").and_then(serde_json::Value::as_str).map(str::to_string);
            if shown && action != "list" {
                self.panel_open = true;
            }
            let text = self.panel.update(cx, |p, cx| p.agent_tabs(thread, &action, index, url, window, cx))?;
            cx.notify();
            return Ok(BrowserCall::Done(crate::browser::agent::Outcome::ok(text)));
        }
        if shown {
            // The user watches the agent work, like screen sharing.
            self.panel_open = true;
        }
        let browser = self.panel.update(cx, |p, cx| p.agent_browser(thread, shown, window, cx));
        cx.notify();
        Ok(BrowserCall::Run(browser))
    }

    /// Run automations: schedules every minute, event triggers every five.
    fn start_scheduler(&mut self, cx: &mut Context<Self>) {
        self._scheduler = Some(cx.spawn(async move |this, cx| {
            let mut tick = 0u64;
            loop {
                cx.background_executor().timer(Duration::from_secs(60)).await;
                tick += 1;
                let Ok(engine) = this.update(cx, |s, _| s.engine.clone()) else { break };
                let check_events = tick.is_multiple_of(5);
                let due = cx
                    .background_spawn(async move {
                        let mut due: Vec<(dove_core::store::Automation, Option<String>)> =
                            engine.due_schedules().into_iter().map(|a| (a, None)).collect();
                        if check_events {
                            due.extend(engine.due_events().into_iter().map(|(a, c)| (a, Some(c))));
                        }
                        due
                    })
                    .await;
                if due.is_empty() {
                    continue;
                }
                let _ = this.update_in_any_window(cx, move |s, window, cx| {
                    for (a, context) in due {
                        s.run_automation(&a, context.as_deref(), window, cx);
                    }
                });
            }
        }));
    }

    pub fn project(&self, id: ProjectId) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }

    pub fn task(&self, id: TaskId) -> Option<&Task> {
        self.tasks.iter().chain(self.archived.iter()).find(|t| t.id == id)
    }

    pub fn active_task(&self) -> Option<&Task> {
        match self.route {
            Route::Thread(id) => self.task(id),
            _ => None,
        }
    }

    pub fn active_thread(&self) -> Option<&Entity<ThreadView>> {
        match self.route {
            Route::Thread(id) => self.threads.get(&id),
            _ => None,
        }
    }

    /// Threads waiting on the user.
    pub fn needs_you(&self) -> Vec<&Task> {
        self.tasks.iter().filter(|t| t.status == TaskStatus::NeedsInput).collect()
    }

    // ------------------------------------------------------------ routing

    pub fn navigate(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        // Agents live in Settings › Agents (one place for them).
        if route == Route::Agents {
            self.open_settings_section("agents", window, cx);
            return;
        }
        // The Threads page gave way to Search, scoped to threads.
        if route == Route::Threads {
            self.open_search(crate::palette::Scope::Threads, window, cx);
            return;
        }
        if route == self.route {
            return;
        }
        self.back.push(self.route);
        self.forward.clear();
        self.go(route, window, cx);
    }

    fn go(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        let prev = self.route;
        self.route = route;
        self.info_open = false;
        self.close_menus();
        match route {
            Route::Thread(id) => self.attach_task(id, window, cx),
            Route::Home => crate::composer::focus(&self.composer, window, cx),
            Route::PullRequests => {
                let preferred = self.home_project.and_then(|id| self.project(id)).cloned();
                let projects = self.projects.clone();
                self.prs.update(cx, |p, cx| p.set_projects(projects, preferred.as_ref(), cx));
            }
            Route::Automations => self.pages.refresh_automations(&self.engine),
            Route::Kanban => self.load_kanban_meta(cx),
            Route::Flock => {
                let cwd = self.home_project.and_then(|id| self.project(id)).map(|p| p.path.clone());
                self.workspace.update(cx, |w, cx| {
                    if let Some(cwd) = cwd.filter(|c| c.is_dir()) {
                        w.default_cwd = cwd;
                    }
                    w.ensure_started(window, cx);
                });
            }
            Route::Usage => self.pages.refresh_usage(&self.engine, cx),
            Route::Races => self.races_shown(cx),
            _ => {}
        }
        if let Route::Thread(id) = prev
            && prev != route
        {
            self.mark_thread_shown(id, false, cx);
        }
        if let Route::Thread(id) = route {
            self.mark_thread_shown(id, true, cx);
        }
        cx.notify();
    }

    /// Background sessions stay live, but only the open thread re-parses
    /// tokens as they arrive.
    fn mark_thread_shown(&mut self, id: TaskId, shown: bool, cx: &mut Context<Self>) {
        if let Some(v) = self.threads.get(&id) {
            v.update(cx, |t, cx| t.set_shown(shown, cx));
        }
    }

    pub fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(r) = self.back.pop() {
            self.forward.push(self.route);
            self.go(r, window, cx);
        }
    }

    pub fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(r) = self.forward.pop() {
            self.back.push(self.route);
            self.go(r, window, cx);
        }
    }

    pub fn new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.join = None;
        if let Some(t) = self.active_task() {
            self.home_project = Some(t.project_id);
        }
        let home = self.home_project;
        let projects = self.projects.clone();
        self.composer.update(cx, |c, cx| c.set_projects(projects, home, cx));
        self.navigate(Route::Home, window, cx);
    }

    /// A new thread with `text` waiting in the composer for the user to finish and send.
    fn new_session_with(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.new_session(window, cx);
        crate::composer::set_text(&self.composer, text, window, cx);
    }

    pub fn open_task(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        if self.task(id).is_some_and(|t| t.status == TaskStatus::Draft) {
            self.navigate(Route::Kanban, window, cx);
            return;
        }
        self.navigate(Route::Thread(id), window, cx);
    }

    fn attach_task(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.task(id).cloned() else { return };
        let Some(project) = self.project(task.project_id).cloned() else { return };
        crate::memory::tab_opened(id, cx);
        // A call follows the user to the thread they open.
        if let Some(call) = self.voice_call.clone() {
            call.update(cx, |v, _| v.current = Some(id));
        }
        if !self.tabs.contains(&id) {
            self.tabs.push(id);
            self.save_tabs();
        }
        if !self.threads.contains_key(&id) {
            self.make_thread(task.clone(), project.clone(), None, window, cx);
        }
        if task.unread {
            let _ = self.engine.store.set_unread(id, false);
            let _ = self.engine.store.mark_run_read(id);
            for t in self.tasks.iter_mut().filter(|t| t.id == id) {
                t.unread = false;
            }
        }
        let dir = task.worktree.clone().unwrap_or_else(|| project.path.clone());
        let base = task.base.clone().or_else(|| Some("HEAD".into()));
        let wanted = self.panel.update(cx, |p, cx| {
            p.set_thread(Some(id), Some(dir), base, &task.title, window, cx);
            p.set_parent(task.clone(), project.clone());
            p.take_wanted()
        });
        // Threads saved in this thread's panes come back beside it.
        for (pane, other) in wanted {
            if let Some(view) = self.thread_view_for(other, window, cx) {
                self.panel.update(cx, |p, cx| p.place_thread(other, view, Some((pane, Zone::Center)), window, cx));
            }
        }
        self.sync_panel(cx);
        if let Some(v) = self.threads.get(&id)
            && v.read(cx).view == ViewMode::Chat
        {
            let composer = v.read(cx).chat.read(cx).composer.clone();
            crate::composer::focus(&composer, window, cx);
        }
    }

    /// A command queued from the phone, or the account's sessions arriving
    /// from the edge after sign-in.
    fn on_remote(&mut self, remote: Entity<Remote>, ev: &RemoteEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            RemoteEvent::AccountSessions { device, chats } => {
                self.receive_account_sessions(device.clone(), chats.clone(), cx);
                return;
            }
            RemoteEvent::AccountTranscript { task, messages } => {
                self.adopt_account_transcript(*task, messages.clone(), cx);
                return;
            }
            RemoteEvent::Upload { done, total, title } => {
                self.on_sync_upload(*done, *total, title.clone(), cx);
                return;
            }
            RemoteEvent::Changed => {
                self.consider_sync_offer(cx);
                return;
            }
            RemoteEvent::Command { .. } => {}
        }
        let RemoteEvent::Command { task, id, command } = ev else { return };
        let chat = task.to_string();
        let Some(t) = self.task(*task).cloned() else {
            remote.read(cx).command_done(&chat, id, false, Some("Session not found on this desktop"));
            return;
        };
        if !self.threads.contains_key(task) {
            let Some(project) = self.project(t.project_id).cloned() else {
                remote.read(cx).command_done(&chat, id, false, Some("Project not found on this desktop"));
                return;
            };
            self.make_thread(t, project, None, window, cx);
        }
        let Some(view) = self.threads.get(task).map(|v| v.read(cx).chat.clone()) else { return };
        match command {
            RemoteCommand::Run(prompt) if !prompt.trim().is_empty() => {
                let prompt = prompt.clone();
                view.update(cx, |v, cx| v.submit(prompt, window, cx));
            }
            RemoteCommand::Run(_) => {
                remote.read(cx).command_done(&chat, id, false, Some("Empty prompt"));
                return;
            }
            RemoteCommand::Interrupt => view.update(cx, |v, cx| v.interrupt(cx)),
        }
        remote.read(cx).command_done(&chat, id, true, None);
        cx.notify();
    }

    /// Open the offer once the registry is up, unless it was already answered
    /// or postponed this launch. `first_run_seen` does not apply.
    fn consider_sync_offer(&mut self, cx: &mut Context<Self>) {
        let Some(remote) = crate::remote::Remote::global(cx) else { return };
        let (status, seen) = {
            let remote = remote.read(cx);
            (remote.status.clone(), remote.offer_seen())
        };
        if self.was_signed_in && !status.signed_in {
            self.sync_postponed = false;
            if matches!(self.sync_offer, SyncOffer::Ready) {
                self.sync_offer = SyncOffer::Closed;
                cx.notify();
            }
        }
        self.was_signed_in = status.signed_in;
        if status.signing_in && matches!(self.sync_offer, SyncOffer::Closed) {
            self.sync_offer = SyncOffer::Waiting;
            cx.notify();
        }
        if matches!(self.sync_offer, SyncOffer::Waiting) {
            let finished = status.signed_in && status.connected;
            let gave_up = !status.signing_in && !status.signed_in && status.error.is_none();
            if finished || gave_up {
                self.sync_offer = SyncOffer::Closed;
            }
            cx.notify();
        }
        if matches!(self.sync_offer, SyncOffer::Switching { .. } | SyncOffer::Uploading { .. })
            && let Some(err) = status.error.clone()
        {
            self.sync_offer = SyncOffer::Failed(err);
            cx.notify();
            return;
        }
        if sync_offer_due(status.signed_in, status.connected, seen, self.sync_postponed)
            && matches!(self.sync_offer, SyncOffer::Closed)
        {
            self.open_sync_ready(cx);
        }
    }

    /// The offer, with the account's list on its way so it can say how much
    /// is coming down as well as how much would go up.
    fn open_sync_ready(&mut self, cx: &mut Context<Self>) {
        self.sync_offer = SyncOffer::Ready;
        if let Some(remote) = crate::remote::Remote::global(cx) {
            remote.read(cx).request_sessions();
        }
        cx.notify();
    }

    fn choose_sync(&mut self, bring: bool, cx: &mut Context<Self>) {
        self.sync_offer = SyncOffer::Switching { bring };
        self.sync_uploaded = 0;
        if let Some(remote) = crate::remote::Remote::global(cx) {
            remote.update(cx, |r, cx| {
                if bring {
                    r.bring_work(cx);
                } else {
                    r.start_fresh();
                }
            });
        }
        if !bring {
            match self.pending_account.take() {
                Some((device, chats)) => self.play_import(device, chats, bring, cx),
                None => self.arm_sync_timeout(bring, cx),
            }
        }
        cx.notify();
    }

    /// The registry list can be slow. The page must not sit on "Setting up
    /// sync" if that list never arrives.
    fn arm_sync_timeout(&mut self, bring: bool, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(8)).await;
            let _ = this.update(cx, |shell, cx| {
                if !matches!(shell.sync_offer, SyncOffer::Switching { .. }) {
                    return;
                }
                if let Some((device, chats)) = shell.pending_account.take() {
                    shell.play_import(device, chats, bring, cx);
                } else {
                    shell.sync_offer = SyncOffer::Done { imported: 0, skipped: 0, bring };
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// "Uploading session N of M", then on to whatever the account has.
    fn on_sync_upload(&mut self, done: usize, total: usize, title: Option<String>, cx: &mut Context<Self>) {
        if !matches!(self.sync_offer, SyncOffer::Switching { bring: true } | SyncOffer::Uploading { .. }) {
            return;
        }
        if done < total {
            self.sync_offer = SyncOffer::Uploading { done, total, title };
            cx.notify();
            return;
        }
        self.sync_uploaded = total;
        self.sync_offer = SyncOffer::Switching { bring: true };
        match self.pending_account.take() {
            Some((device, chats)) => self.play_import(device, chats, true, cx),
            None => self.arm_sync_timeout(true, cx),
        }
        cx.notify();
    }

    fn postpone_sync(&mut self, cx: &mut Context<Self>) {
        self.sync_postponed = true;
        self.sync_offer = SyncOffer::Closed;
        cx.notify();
    }

    pub(crate) fn reopen_sync_offer(&mut self, cx: &mut Context<Self>) {
        self.account_open = false;
        self.sync_postponed = false;
        self.open_sync_ready(cx);
    }

    /// Account chats from the registry. Before the offer is answered they
    /// wait. After it, they are written onto this computer.
    fn receive_account_sessions(&mut self, device: String, chats: serde_json::Value, cx: &mut Context<Self>) {
        let seen = self.engine.store.setting(crate::remote::SYNC_OFFER_SEEN).as_deref() == Some("1");
        let wizard = matches!(
            self.sync_offer,
            SyncOffer::Switching { .. } | SyncOffer::Importing { .. } | SyncOffer::Done { .. }
        );
        let asking = matches!(self.sync_offer, SyncOffer::Ready | SyncOffer::Waiting | SyncOffer::Uploading { .. });
        if (!seen && !wizard) || asking {
            self.pending_account = Some((device, chats));
            cx.notify();
            return;
        }
        if self.sync_importing {
            self.pending_account = Some((device, chats));
            return;
        }
        if let SyncOffer::Switching { bring } = self.sync_offer {
            self.play_import(device, chats, bring, cx);
            return;
        }
        if let SyncOffer::Done { imported: 0, bring, .. } = self.sync_offer
            && chats.as_array().is_some_and(|rows| !rows.is_empty())
        {
            self.play_import(device, chats, bring, cx);
            return;
        }
        if matches!(self.sync_offer, SyncOffer::Closed | SyncOffer::Done { .. } | SyncOffer::Failed(_)) {
            let _ = self.import_account_sessions(device, chats, cx);
        }
    }

    /// Walk the account list on screen: "Importing session N of M".
    fn play_import(&mut self, device: String, chats: serde_json::Value, bring: bool, cx: &mut Context<Self>) {
        let Some(list) = chats.as_array().cloned() else {
            self.sync_offer = SyncOffer::Done { imported: 0, skipped: 0, bring };
            cx.notify();
            return;
        };
        let total = list.len();
        self.sync_importing = true;
        self.sync_import_gen = self.sync_import_gen.wrapping_add(1);
        let generation = self.sync_import_gen;
        self.sync_offer = SyncOffer::Importing { done: 0, total, title: None };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let mut imported = 0usize;
            let mut skipped = 0usize;
            let mut pulls = Vec::new();
            let mut parents = Vec::new();
            for (i, chat) in list.into_iter().enumerate() {
                let title = chat.get("title").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_owned);
                let step = this.update(cx, |shell, _cx| {
                    if shell.sync_import_gen != generation {
                        return None;
                    }
                    shell.sync_offer = SyncOffer::Importing { done: i, total, title: title.clone() };
                    Some(shell.adopt_account_chat(&device, &chat))
                });
                let Ok(Some(adopted)) = step else { return };
                match adopted.0 {
                    AdoptKind::Imported => imported += 1,
                    AdoptKind::Skipped => skipped += 1,
                    AdoptKind::Ignored => {}
                }
                if let Some(id) = adopted.1 {
                    pulls.push(id);
                }
                if let Some(pair) = adopted.2 {
                    parents.push(pair);
                }
                cx.background_executor().timer(Duration::from_millis(30)).await;
            }
            let _ = this.update(cx, |shell, cx| {
                if shell.sync_import_gen != generation {
                    return;
                }
                shell.finish_account_links(&parents, &pulls, cx);
                shell.sync_importing = false;
                if imported > 0 || !pulls.is_empty() {
                    shell.reload(cx);
                }
                if let Some((device, chats)) = shell.pending_account.take() {
                    let more = chats.as_array().is_some_and(|rows| !rows.is_empty());
                    if imported == 0 && more {
                        shell.play_import(device, chats, bring, cx);
                        return;
                    }
                    let _ = shell.import_account_sessions(device, chats, cx);
                }
                shell.sync_offer = SyncOffer::Done { imported, skipped, bring };
                cx.notify();
            });
        })
        .detach();
    }

    /// Sessions already on the Woozlit account. Ones this computer does not
    /// have yet are recorded locally. Ones that live on another device are
    /// marked so this computer does not claim them, and their transcript is
    /// pulled from the chat room.
    fn import_account_sessions(
        &mut self,
        device: String,
        chats: serde_json::Value,
        cx: &mut Context<Self>,
    ) -> (usize, usize) {
        let Some(list) = chats.as_array() else { return (0, 0) };
        let mut imported = 0;
        let mut skipped = 0;
        let mut pulls = Vec::new();
        let mut parents = Vec::new();
        for chat in list {
            let (kind, pull, parent) = self.adopt_account_chat(&device, chat);
            match kind {
                AdoptKind::Imported => imported += 1,
                AdoptKind::Skipped => skipped += 1,
                AdoptKind::Ignored => {}
            }
            if let Some(id) = pull {
                pulls.push(id);
            }
            if let Some(pair) = parent {
                parents.push(pair);
            }
        }
        let changed = imported > 0 || !pulls.is_empty();
        self.finish_account_links(&parents, &pulls, cx);
        if changed {
            self.reload(cx);
        }
        (imported, skipped)
    }

    /// One registry chat. `(kind, pull this transcript, link child→parent)`.
    fn adopt_account_chat(
        &mut self,
        device: &str,
        chat: &serde_json::Value,
    ) -> (AdoptKind, Option<TaskId>, Option<(TaskId, TaskId)>) {
        let Some(id) = chat.get("id").and_then(|v| v.as_str()).and_then(TaskId::parse) else {
            return (AdoptKind::Ignored, None, None);
        };
        let store = self.engine.store.clone();
        // Deleted from this computer on purpose: the account's copy stays there.
        if store.setting(&format!("account_skip.{id}")).as_deref() == Some("1") {
            return (AdoptKind::Ignored, None, None);
        }
        let owner = chat.get("deviceId").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let remote_owned = !owner.is_empty() && owner != device;
        let archived = chat.get("archived").and_then(|v| v.as_bool()).unwrap_or(false);
        let parent =
            chat.get("parentChatId").and_then(|v| v.as_str()).and_then(TaskId::parse).map(|parent| (id, parent));
        if store.task(id).ok().flatten().is_some() {
            let mut pull = None;
            if remote_owned {
                let _ = store.set_setting(&format!("account_device.{id}"), &owner);
                if !archived && store.read_transcript(id).is_empty() {
                    pull = Some(id);
                }
            }
            return (AdoptKind::Skipped, pull, parent);
        }
        let cwd = chat.get("cwd").and_then(|v| v.as_str()).unwrap_or("");
        let path = if !cwd.is_empty() && std::path::Path::new(cwd).exists() {
            std::path::PathBuf::from(cwd)
        } else {
            let dir = dove_core::hosts::dove_home().join("account");
            let _ = std::fs::create_dir_all(&dir);
            dir
        };
        let Ok(project) = store.add_project(&path) else {
            return (AdoptKind::Ignored, None, None);
        };
        let title =
            chat.get("title").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("Session").to_string();
        let task = Task {
            id,
            project_id: project.id,
            title,
            harness: harness_from_account(chat.get("harness").and_then(|v| v.as_str())),
            model: None,
            status: if archived { TaskStatus::Archived } else { TaskStatus::Idle },
            worktree: None,
            branch: chat.get("branch").and_then(|v| v.as_str()).map(str::to_owned),
            native_session_id: None,
            base: None,
            pinned: false,
            updated_at: dove_core::store::now_ms(),
            prompt: chat.get("preview").and_then(|v| v.as_str()).map(str::to_owned),
            unread: !archived && remote_owned,
            automation: None,
        };
        if store.insert_task(&task).is_err() {
            return (AdoptKind::Ignored, None, None);
        }
        let pull = if remote_owned {
            let _ = store.set_setting(&format!("account_device.{id}"), &owner);
            (!archived).then_some(id)
        } else {
            None
        };
        (AdoptKind::Imported, pull, parent)
    }

    fn finish_account_links(&mut self, parents: &[(TaskId, TaskId)], pulls: &[TaskId], cx: &mut Context<Self>) {
        let store = self.engine.store.clone();
        for (child, parent) in parents {
            if store.task(*parent).ok().flatten().is_some() {
                let _ = store.link_task(*child, *parent, dove_core::store::LinkKind::Delegate, false);
            }
        }
        if pulls.is_empty() {
            return;
        }
        if let Some(remote) = crate::remote::Remote::global(cx) {
            remote.update(cx, |r, _| {
                for id in pulls {
                    let events = store.read_transcript(*id);
                    let flat = events.is_empty()
                        || !events.iter().any(|event| {
                            matches!(
                                event,
                                dove_core::proto::AgentEvent::ThinkingDelta { .. }
                                    | dove_core::proto::AgentEvent::ToolCall { .. }
                            )
                        });
                    if flat {
                        r.pull_chat(&id.to_string());
                    }
                }
            });
        }
    }

    /// Write an account transcript that this computer does not have yet, then
    /// show it if that session is already open.
    fn adopt_account_transcript(&mut self, task: TaskId, messages: serde_json::Value, cx: &mut Context<Self>) {
        let Some(list) = messages.as_array().cloned() else { return };
        if list.is_empty() {
            return;
        }
        let local = self.engine.store.read_transcript(task);
        let local_flat = !local.iter().any(|event| {
            matches!(
                event,
                dove_core::proto::AgentEvent::ThinkingDelta { .. } | dove_core::proto::AgentEvent::ToolCall { .. }
            )
        });
        if !local.is_empty() && !(local_flat && synced_has_structure(&list)) {
            return;
        }
        let store = self.engine.store.clone();
        cx.spawn(async move |this, cx| {
            let wrote = cx
                .background_spawn(async move {
                    use std::io::Write;
                    let path = store.transcript_path(task);
                    if let Some(dir) = path.parent() {
                        std::fs::create_dir_all(dir).ok()?;
                    }
                    let mut file = std::fs::File::create(&path).ok()?;
                    let mut any = false;
                    for message in &list {
                        for event in events_from_synced(message) {
                            serde_json::to_writer(&mut file, &event).ok()?;
                            file.write_all(b"\n").ok()?;
                            any = true;
                        }
                    }
                    file.flush().ok()?;
                    any.then_some(())
                })
                .await;
            if wrote.is_none() {
                return;
            }
            let _ = this.update(cx, |shell, cx| {
                if let Some(view) = shell.threads.get(&task) {
                    view.update(cx, |thread, cx| {
                        thread.chat.update(cx, |chat, cx| chat.reload_saved(cx));
                    });
                }
            });
        })
        .detach();
    }

    /// Whether thread `id`'s view must stay: its tab is open, it is the
    /// open thread, or it shows beside another (a column or a pane).
    pub(crate) fn keeps_thread(&self, id: TaskId, cx: &gpui::App) -> bool {
        self.tabs.contains(&id)
            || self.route == Route::Thread(id)
            || self.is_companion(id)
            || self.panel.read(cx).holds_thread(id)
    }

    /// Thread `id`'s view, made (without starting anything) if it is not
    /// open yet. None for drafts and unknown threads.
    pub(crate) fn thread_view_for(
        &mut self,
        id: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ThreadView>> {
        if let Some(v) = self.threads.get(&id) {
            return Some(v.clone());
        }
        let task = self.task(id).cloned().filter(|t| t.status != TaskStatus::Draft)?;
        let project = self.project(task.project_id).cloned()?;
        crate::memory::tab_opened(id, cx);
        Some(self.make_thread(task, project, None, window, cx))
    }

    /// Show thread `id` beside the open one: in a pane of the side panel,
    /// dropped on `at` or (None) next to the conversation. The panel opens
    /// if it was closed. Dropping the open thread on itself does nothing.
    pub(crate) fn open_beside(
        &mut self,
        id: TaskId,
        at: Option<(crate::workspace::PaneId, Zone)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Route::Thread(main) = self.route else {
            // Nothing open to sit beside: open it.
            self.open_task(id, window, cx);
            return;
        };
        if main == id {
            return;
        }
        let Some(view) = self.thread_view_for(id, window, cx) else { return };
        // A thread shows in one place: a column beside moves to the dock.
        if self.companions.get(&main).is_some_and(|c| c.contains(&id)) {
            self.close_companion(main, id, cx);
        }
        if !self.panel_open {
            self.panel_open = true;
            let _ = self.engine.store.set_setting("panel_open", "true");
        }
        self.panel.update(cx, |p, cx| p.place_thread(id, view, at, window, cx));
        cx.notify();
    }

    pub(crate) fn make_thread(
        &mut self,
        task: Task,
        project: Project,
        live: Option<dove_core::LiveSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ThreadView> {
        let id = task.id;
        let engine = self.engine.clone();
        let chat = cx.new(|cx| TaskView::new(engine.clone(), task, project, live, window, cx));
        let view = cx.new(|cx| ThreadView::new(engine, chat, window, cx));
        let sub = cx.subscribe_in(&view, window, move |this, view, ev: &ThreadEvent, window, cx| {
            this.on_thread_event(id, view.clone(), ev, window, cx)
        });
        // Dropped with the view when it hibernates.
        crate::memory::own_thread_sub(id, sub, cx);
        self.threads.insert(id, view.clone());
        view
    }

    fn on_thread_event(
        &mut self,
        id: TaskId,
        _view: Entity<ThreadView>,
        ev: &ThreadEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::memory::thread_event(self, id, ev, cx);
        match ev {
            ThreadEvent::Mode => cx.notify(),
            ThreadEvent::Chat(kind) => match kind {
                TaskViewEventKind::Status(s) => self.on_status(id, *s, window, cx),
                TaskViewEventKind::Native(native) => {
                    for t in self.tasks.iter_mut().filter(|t| t.id == id) {
                        t.native_session_id = Some(native.clone());
                    }
                }
                TaskViewEventKind::Changed => {
                    if self.route == Route::Thread(id) {
                        self.sync_panel(cx);
                    }
                }
                TaskViewEventKind::Fork => self.fork_task(id, window, cx),
                TaskViewEventKind::Rewind => self.rewind_turn(id, cx),
                TaskViewEventKind::ShowChanges => {
                    self.panel_open = true;
                    self.panel.update(cx, |p, cx| p.show(Tab::Changes, window, cx));
                    cx.notify();
                }
                TaskViewEventKind::Title(title) => {
                    // Quick titles never override a rename or an AI title.
                    let store = &self.engine.store;
                    let kept = store.setting(&format!("renamed.{id}")).is_some()
                        || store.setting(&format!("title_ai.{id}")).is_some();
                    if !kept {
                        self.apply_title(id, title, cx);
                    }
                }
                TaskViewEventKind::PullRequest(url) => self.link_pr(id, url, cx),
                TaskViewEventKind::AiTitle(title) => {
                    // An agent's title replaces the quick one; a rename wins.
                    if self.engine.store.setting(&format!("renamed.{id}")).is_none() {
                        let _ = self.engine.store.set_setting(&format!("title_ai.{id}"), "1");
                        self.apply_title(id, title, cx);
                    }
                }
                TaskViewEventKind::DriveBrowser(action) => {
                    self.panel_open = true;
                    let action = action.clone();
                    self.panel.update(cx, |p, cx| p.drive_browser(action, window, cx));
                    cx.notify();
                }
            },
        }
    }

    /// A thread's status moved: update the lists, the badge, notifications.
    fn on_status(&mut self, id: TaskId, s: TaskStatus, window: &mut Window, cx: &mut Context<Self>) {
        let was = self.task(id).map(|t| t.status);
        let visible = self.route == Route::Thread(id) && self.window_active;
        for t in self.tasks.iter_mut().filter(|t| t.id == id) {
            t.status = s;
            t.updated_at = dove_core::store::now_ms();
            if matches!(s, TaskStatus::Done | TaskStatus::Failed | TaskStatus::NeedsInput) && !visible {
                t.unread = true;
            }
        }
        self.team_on_status(id, s);
        self.race_on_status(id, s, cx);
        if s == TaskStatus::Running && was != Some(TaskStatus::Running) {
            self.checkpoint_turn(id, cx);
        }
        let title = self.task(id).map(|t| t.title.clone()).unwrap_or_default();
        if matches!(s, TaskStatus::Done | TaskStatus::NeedsInput | TaskStatus::Failed) {
            let headline = match s {
                TaskStatus::NeedsInput => "Needs you",
                TaskStatus::Failed => "Failed",
                _ => "Finished",
            };
            let note = format!("{headline}: {title}");
            cx.background_spawn(async move { dove_core::reach::deliver(&note, &note) }).detach();
        }
        if matches!(s, TaskStatus::Done | TaskStatus::NeedsInput | TaskStatus::Failed) {
            let (headline, tone) = match s {
                TaskStatus::NeedsInput => ("Needs you", Tone::Warning),
                TaskStatus::Failed => ("Failed", Tone::Danger),
                _ => ("Finished", Tone::Success),
            };
            if !self.window_active {
                notify::os_notify(&self.engine, &format!("{headline}: {title}"), "Open Keplorayx to continue.");
                self.pending_focus = Some((id, Instant::now()));
            } else if self.route != Route::Thread(id) {
                Toasts::push(cx, format!("{headline}: {title}"), None, tone, Some(id));
            }
            if self.route == Route::Thread(id) {
                self.panel.update(cx, |p, cx| p.refresh(cx));
            }
        }
        notify::set_badge(window, self.needs_you().len());
        cx.notify();
    }

    /// Save a shadow git ref of the worktree as a turn starts, so Rewind can
    /// put tracked files back. Runs off the UI thread.
    fn checkpoint_turn(&mut self, id: TaskId, cx: &mut Context<Self>) {
        let Some(task) = self.task(id).cloned() else { return };
        let dir = task.worktree.clone().or_else(|| self.project(task.project_id).map(|p| p.path.clone()));
        let Some(dir) = dir else { return };
        let label = id.to_string();
        let found = cx.background_spawn(async move { dove_core::git::checkpoint(&dir, &label) });
        cx.spawn(async move |this, cx| {
            let Ok(Some(sha)) = found.await else { return };
            let _ = this.update(cx, |s, _| {
                let _ = s.engine.store.set_setting(&format!("checkpoint.{id}"), &sha);
            });
        })
        .detach();
    }

    /// Two clicks: the first arms, the second restores tracked files from the
    /// checkpoint taken when the current turn started.
    pub fn rewind_turn(&mut self, id: TaskId, cx: &mut Context<Self>) {
        let key = format!("rewind-arm.{id}");
        let now = dove_core::store::now_ms();
        let armed =
            self.engine.store.setting(&key).and_then(|s| s.parse::<i64>().ok()).is_some_and(|at| now - at < 8_000);
        if !armed {
            let _ = self.engine.store.set_setting(&key, &now.to_string());
            Toasts::push(
                cx,
                "Click Rewind again to restore files from the last checkpoint.",
                None,
                Tone::Warning,
                Some(id),
            );
            return;
        }
        let _ = self.engine.store.set_setting(&key, "");
        let Some(sha) = self.engine.store.setting(&format!("checkpoint.{id}")).filter(|s| !s.is_empty()) else {
            Toasts::push(cx, "No checkpoint yet. One is saved when a turn starts.", None, Tone::Warning, Some(id));
            return;
        };
        let Some(task) = self.task(id).cloned() else { return };
        let dir = task.worktree.clone().or_else(|| self.project(task.project_id).map(|p| p.path.clone()));
        let Some(dir) = dir else { return };
        let work = cx.background_spawn(async move { dove_core::git::rewind_to(&dir, &sha) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |_, cx| match result {
                Ok(()) => Toasts::push(
                    cx,
                    "Restored tracked files from the checkpoint. New untracked files were left in place.",
                    None,
                    Tone::Success,
                    Some(id),
                ),
                Err(e) => Toasts::push(cx, format!("Couldn't rewind: {e:#}"), None, Tone::Danger, Some(id)),
            });
        })
        .detach();
    }

    /// Mirror the active thread's plan, subagents and side chats into the
    /// side panel.
    fn sync_panel(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.active_thread() else { return };
        let chat = v.read(cx).chat.clone();
        // Todos, subagents and designs move only when the chat says so.
        let fresh = crate::memory::panel_changed((chat.entity_id(), chat.read(cx).panel_rev()), cx);
        let (todos, subagents) = if fresh {
            let chat = chat.read(cx);
            (chat.todos().to_vec(), chat.subagents())
        } else {
            Default::default()
        };
        let chats = self.side_chats_of(self.active_task().map(|t| t.id));
        let pr = self.active_task().map(|t| (t.id, self.pr_target(t.id)));
        self.panel.update(cx, |p, cx| {
            if fresh {
                p.set_todos(todos, cx);
                p.set_subagents(subagents, cx);
            }
            p.set_side_chats(chats, cx);
            if let Some((id, target)) = pr {
                p.set_pr(id, target, cx);
            }
        });
    }

    /// Link thread `id` to the pull request at `url` (an agent opened it, or
    /// it is the branch's PR). The dock gains a Pull request tab.
    pub fn link_pr(&mut self, id: TaskId, url: &str, cx: &mut Context<Self>) {
        let Some((slug, n)) = dove_core::git::github::parse_pr_url(url) else { return };
        let url = format!("https://github.com/{slug}/pull/{n}");
        if self.pr_links.get(&id) == Some(&url) {
            return;
        }
        let _ = self.engine.store.set_task_pr(id, Some(&url));
        self.pr_links.insert(id, url);
        if self.active_task().is_some_and(|t| t.id == id) {
            self.sync_panel(cx);
        }
        cx.notify();
    }

    /// Thread `id`'s pull request as something a [`crate::pr_view::PrView`]
    /// can show. Driven through `gh -R owner/repo`, so it works whether or
    /// not the PR's repository is this thread's checkout.
    fn pr_target(&self, id: TaskId) -> Option<PrTarget> {
        let url = self.pr_links.get(&id)?;
        let (slug, number) = dove_core::git::github::parse_pr_url(url)?;
        let project = self.task(id).and_then(|t| self.project(t.project_id)).cloned();
        Some(PrTarget { repo: dove_core::git::github::PrRepo::Remote(slug), number, url: url.clone(), project })
    }

    /// Open the pull request at `url` in the dock's Pull request tab.
    pub fn open_pr_url(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some((slug, number)) = dove_core::git::github::parse_pr_url(url) else { return };
        let project = self.active_task().and_then(|t| self.project(t.project_id)).cloned();
        let target = PrTarget {
            repo: dove_core::git::github::PrRepo::Remote(slug.clone()),
            number,
            url: format!("https://github.com/{slug}/pull/{number}"),
            project,
        };
        if let Some(id) = self.active_task().map(|t| t.id)
            && !self.pr_links.contains_key(&id)
        {
            self.link_pr(id, &target.url, cx);
        }
        self.panel_open = true;
        let _ = self.engine.store.set_setting("panel_open", "true");
        self.panel.update(cx, |p, cx| p.show_pr(target, window, cx));
        cx.notify();
    }

    /// Put `prompt` in Home's composer, in `project`, for a new thread.
    fn stage_new_thread(
        &mut self,
        project: Option<Project>,
        prompt: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(project) = project {
            self.home_project = Some(project.id);
            let projects = self.projects.clone();
            self.composer.update(cx, |c, cx| {
                c.set_projects(projects, Some(project.id), cx);
                cx.notify();
            });
        }
        crate::composer::set_text(&self.composer, prompt, window, cx);
        self.navigate(Route::Home, window, cx);
    }

    /// Threads that share `id`'s worktree (its side chats), newest first.
    fn side_chats_of(&self, id: Option<TaskId>) -> Vec<crate::side_panel::SideChatInfo> {
        let Some(task) = id.and_then(|id| self.task(id)) else { return Vec::new() };
        let Some(wt) = task.worktree.clone() else { return Vec::new() };
        let mut out: Vec<_> = self
            .tasks
            .iter()
            .filter(|t| t.id != task.id && t.worktree.as_ref() == Some(&wt))
            .map(|t| crate::side_panel::SideChatInfo {
                id: t.id,
                title: t.title.clone(),
                status: t.status,
                unread: t.unread,
                updated_at: t.updated_at,
            })
            .collect();
        out.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        out
    }

    fn save_tabs(&self) {
        let s = self.tabs.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(",");
        let _ = self.engine.store.set_setting("open_tabs", &s);
    }

    pub fn close_tab(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.tabs.iter().position(|t| *t == id) else { return };
        self.tabs.remove(i);
        self.save_tabs();
        if self.route == Route::Thread(id) {
            let next = self.tabs.get(i).or_else(|| self.tabs.get(i.saturating_sub(1))).copied();
            match next {
                Some(n) => self.navigate(Route::Thread(n), window, cx),
                None => self.navigate(Route::Home, window, cx),
            }
        }
        crate::memory::tab_closed(self, id, window, cx);
        cx.notify();
    }

    fn cycle_tab(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let cur = match self.route {
            Route::Thread(id) => self.tabs.iter().position(|t| *t == id).map(|i| i as isize).unwrap_or(-1),
            _ => -1,
        };
        let n = self.tabs.len() as isize;
        let next = ((cur + delta) % n + n) % n;
        let id = self.tabs[next as usize];
        self.navigate(Route::Thread(id), window, cx);
    }

    // ------------------------------------------------------------ actions

    pub(crate) fn start(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let (project, run, harness, persona, cloud) = {
            let c = self.composer.read(cx);
            (c.project.clone(), c.run_settings(cx), c.harness(cx), c.persona.clone(), c.cloud)
        };
        if cloud {
            let Some(project) = project.or_else(|| self.home_project.and_then(|id| self.project(id)).cloned()) else {
                self.error = Some("Add a project first.".into());
                cx.notify();
                return;
            };
            let new = NewTask {
                title: ui::title_from_prompt(&text),
                prompt: text.clone(),
                harness,
                run,
                join: self.join.as_ref().map(|(_, j)| j.clone()),
                persona,
            };
            match self.engine.create_thread(&project, new) {
                Ok((task, _)) => {
                    let id = task.id;
                    let _ = self.engine.store.set_setting(&format!("cloud_state.{id}"), "starting");
                    let _ = self.engine.store.set_prompt(id, &text);
                    self.error = None;
                    self.join = None;
                    self.tasks.insert(0, task.clone());
                    self.make_thread(task, project, None, window, cx);
                    self.navigate(Route::Thread(id), window, cx);
                    Toasts::push(cx, cloud_notice("starting"), None, Tone::Info, Some(id));
                }
                Err(e) => {
                    crate::composer::set_text(&self.composer, &text, window, cx);
                    self.error = Some(format!("{e:#}").into());
                }
            }
            cx.notify();
            return;
        }
        let Some(project) = project.or_else(|| self.home_project.and_then(|id| self.project(id)).cloned()) else {
            self.error = Some("Add a project first.".into());
            cx.notify();
            return;
        };
        let new = NewTask {
            title: ui::title_from_prompt(&text),
            prompt: text.clone(),
            harness,
            run,
            join: self.join.as_ref().map(|(_, j)| j.clone()),
            persona,
        };
        match self.engine.start_task(&project, new) {
            Ok((task, live)) => {
                self.error = None;
                self.join = None;
                let id = task.id;
                // `/orchestrator` typed on the new-thread composer goes with it.
                let o = crate::orchestrate::get(&self.engine.store, None);
                if o != crate::orchestrate::Orchestrate::Off {
                    crate::orchestrate::set(&self.engine.store, Some(id), o);
                    crate::orchestrate::set(&self.engine.store, None, crate::orchestrate::Orchestrate::Off);
                }
                self.tasks.insert(0, task.clone());
                self.make_thread(task, project, Some(live), window, cx);
                self.navigate(Route::Thread(id), window, cx);
            }
            Err(e) => {
                // Keep what the user typed.
                crate::composer::set_text(&self.composer, &text, window, cx);
                self.error = Some(format!("{e:#}").into());
            }
        }
        cx.notify();
    }

    pub fn run_automation(
        &mut self,
        a: &dove_core::store::Automation,
        context: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.engine.run_automation(a, context) {
            Ok((task, live)) => {
                let Some(project) = self.project(task.project_id).cloned() else { return };
                let id = task.id;
                self.tasks.insert(0, task.clone());
                self.make_thread(task, project, Some(live), window, cx);
                self.inbox_unread += 1;
                Toasts::push(cx, format!("Automation started: {}", a.name), None, Tone::Info, Some(id));
                cx.notify();
            }
            Err(e) => Toasts::push(
                cx,
                format!("Automation {} failed to start", a.name),
                Some(format!("{e:#}")),
                Tone::Danger,
                None,
            ),
        }
    }

    /// Start a Kanban draft with its saved prompt.
    pub fn start_draft(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.task(id).cloned() else { return };
        let Some(project) = self.project(draft.project_id).cloned() else { return };
        let run = self.composer.read(cx).run_settings(cx);
        let run = if self.composer.read(cx).harness(cx) == draft.harness { run } else { Default::default() };
        match self.engine.start_draft(&draft, &project, run) {
            Ok((task, live)) => {
                self.tasks.retain(|t| t.id != id);
                self.tasks.insert(0, task.clone());
                self.make_thread(task, project, Some(live), window, cx);
                cx.notify();
            }
            Err(e) => Toasts::push(cx, "Could not start the draft", Some(format!("{e:#}")), Tone::Danger, None),
        }
    }

    pub fn add_project_dialog(&mut self, cx: &mut Context<Self>) {
        let pick = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add project".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = pick.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let _ = this.update(cx, |s, cx| s.add_project(path, cx));
        })
        .detach();
    }

    fn add_project(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let root = dove_core::git::repo_root(&path).unwrap_or(path);
        match self.engine.store.add_project(&root) {
            Ok(p) => {
                self.home_project = Some(p.id);
                let _ = self.engine.store.set_setting("active_project", &p.id.to_string());
                self.reload(cx);
            }
            Err(e) => self.error = Some(format!("{e:#}").into()),
        }
        cx.notify();
    }

    pub fn set_home_project(&mut self, id: ProjectId, cx: &mut Context<Self>) {
        self.home_project = Some(id);
        self.join = None;
        let _ = self.engine.store.set_setting("active_project", &id.to_string());
        cx.notify();
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        // Hidden only for want of room: bring it back (the dock gives way).
        if self.sidebar_open && !self.fit.shown.0 {
            self.reveal_sidebar();
            cx.notify();
            return;
        }
        self.sidebar_open = !self.sidebar_open;
        let _ = self.engine.store.set_setting("sidebar_open", if self.sidebar_open { "true" } else { "false" });
        cx.notify();
    }

    /// Show the sidebar, making it the most recent column so the fit keeps
    /// it over the dock.
    fn reveal_sidebar(&mut self) {
        self.sidebar_open = true;
        self.fit.seq += 1;
        self.fit.sidebar_at = self.fit.seq;
    }

    /// A drag sample's limit: reaching one (not holding it) nudges the seam.
    fn note_resize_edge(&mut self, edge: Edge, pressing: Option<crate::motion::ResizeEdge>) {
        if let Some(limit) = pressing
            && self.resize_pressing != pressing
            && !crate::motion::reduced_now()
        {
            self.resize_bounce = Some((edge, limit, Instant::now()));
        }
        self.resize_pressing = pressing;
    }

    /// The sidebar is open and the window has room for it.
    pub fn sidebar_shown(&self) -> bool {
        self.sidebar_open && self.fit.shown.0
    }

    /// Either of the dock's columns is open.
    fn right_open(&self) -> bool {
        self.panel_open || (self.explorer_open && matches!(self.route, Route::Thread(_)))
    }

    fn set_explorer(&mut self, open: bool, cx: &mut Context<Self>) {
        self.explorer_open = open;
        let _ = self.engine.store.set_setting("explorer_open", if open { "true" } else { "false" });
        if open {
            // The newest column: short of room, the others give way to it.
            self.fit.seq += 1;
            self.fit.panel_at = self.fit.seq;
            self.panel.update(cx, |p, cx| p.refresh(cx));
        }
        cx.notify();
    }

    /// The dock is open and the window has room for it.
    /// The side panel shows (it steps aside while the call view is up).
    fn panel_shown(&self) -> bool {
        self.right_open() && self.fit.shown.1 && !self.call_view_shown
    }

    /// Stamp columns that just opened and decide what fits this frame.
    fn update_fit(&mut self, viewport: f32, in_thread: bool, expanded: bool) {
        let f = &mut self.fit;
        if self.sidebar_open && !f.sidebar_was {
            f.seq += 1;
            f.sidebar_at = f.seq;
        }
        let right_open = self.panel_open || (self.explorer_open && matches!(self.route, Route::Thread(_)));
        if right_open && !f.panel_was {
            f.seq += 1;
            f.panel_at = f.seq;
        }
        f.sidebar_was = self.sidebar_open;
        f.panel_was = right_open;
        let explorer = self.explorer_open && matches!(self.route, Route::Thread(_));
        let dock_floor = if self.panel_open { PANEL_FLOOR } else { 0.0 }
            + if explorer { crate::side_panel::EXPLORER_MIN } else { 0.0 };
        f.shown = panel_fit_floor(
            viewport,
            self.sidebar_w,
            (self.sidebar_open, f.sidebar_at),
            (in_thread && right_open, f.panel_at),
            dock_floor,
            expanded,
        );
    }

    pub fn toggle_panel(&mut self, cx: &mut Context<Self>) {
        if self.right_open() && matches!(self.route, Route::Thread(_)) && !self.fit.shown.1 {
            // Hidden for want of room: bring it back (the sidebar gives way).
            self.fit.seq += 1;
            self.fit.panel_at = self.fit.seq;
            self.panel.update(cx, |p, cx| p.refresh(cx));
            cx.notify();
            return;
        }
        self.panel_open = !self.panel_open;
        let _ = self.engine.store.set_setting("panel_open", if self.panel_open { "true" } else { "false" });
        if self.panel_open {
            self.panel.update(cx, |p, cx| p.refresh(cx));
        } else {
            crate::memory::watchdog(self, None, cx);
        }
        cx.notify();
    }

    /// A pane shortcut, in a thread. Splitting with the panel closed opens
    /// it first; the other commands need it open.
    fn pane_command(&mut self, cmd: PaneCommand, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.route, Route::Thread(_)) {
            return;
        }
        if !self.panel_open {
            if !matches!(cmd, PaneCommand::Split(_)) {
                return;
            }
            self.toggle_panel(cx);
        }
        self.panel.update(cx, |p, cx| p.command(cmd, window, cx));
    }

    pub fn show_panel(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        self.panel_open = true;
        let _ = self.engine.store.set_setting("panel_open", "true");
        self.panel.update(cx, |p, cx| p.show(tab, window, cx));
        cx.notify();
    }

    /// Store a thread's new title and show it in the sidebar, tabs and pages.
    fn apply_title(&mut self, id: TaskId, title: &str, cx: &mut Context<Self>) {
        let _ = self.engine.store.rename_task(id, title);
        for t in self.tasks.iter_mut().filter(|t| t.id == id) {
            t.title = title.to_string();
        }
        if let Some(v) = self.threads.get(&id) {
            let chat = v.read(cx).chat.clone();
            let title = title.to_string();
            chat.update(cx, |c, _| c.task.title = title);
        }
        cx.notify();
    }

    pub fn set_pinned(&mut self, id: TaskId, pinned: bool, cx: &mut Context<Self>) {
        let _ = self.engine.store.set_pinned(id, pinned);
        self.menu = None;
        self.reload(cx);
    }

    pub fn set_archived(&mut self, id: TaskId, archived: bool, window: &mut Window, cx: &mut Context<Self>) {
        let status = if archived { TaskStatus::Archived } else { TaskStatus::Idle };
        let _ = self.engine.store.set_status(id, status);
        if archived {
            // Stop a running agent; the transcript stays.
            if let Some(v) = self.threads.remove(&id) {
                v.update(cx, |v, cx| v.shutdown(cx));
            }
            self.panel.update(cx, |p, cx| p.forget(id, cx));
            if self.tabs.contains(&id) {
                self.close_tab(id, window, cx);
            }
        }
        self.menu = None;
        self.reload(cx);
    }

    pub fn delete_task(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.task(id).cloned() else { return };
        let project = self.project(task.project_id).cloned();
        self.close_for_delete(id, window, cx);
        let engine = self.engine.clone();
        // Removing a worktree can take a moment; do it off the UI thread.
        cx.background_spawn(async move {
            let _slot = crate::git_slots::slot().await;
            if let Err(e) = engine.delete_task(&task, project.as_ref()) {
                tracing::warn!("delete: {e:#}");
            }
        })
        .detach();
        self.menu = None;
        cx.notify();
    }

    /// Stop a thread that is about to be deleted and take it off the screen.
    fn close_for_delete(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(v) = self.threads.remove(&id) {
            v.update(cx, |v, cx| v.shutdown(cx));
        }
        // Terminals leaked before; kill them with the thread.
        self.panel.update(cx, |p, cx| p.forget(id, cx));
        if self.tabs.contains(&id) {
            self.close_tab(id, window, cx);
        }
        self.back.retain(|r| *r != Route::Thread(id));
        self.forward.retain(|r| *r != Route::Thread(id));
        self.tasks.retain(|t| t.id != id);
        self.archived.retain(|t| t.id != id);
    }

    /// Delete one session from this computer, the account, or both.
    pub fn delete_session(&mut self, id: TaskId, scope: DeleteScope, window: &mut Window, cx: &mut Context<Self>) {
        let remote = crate::remote::Remote::global(cx);
        if let Some(remote) = remote {
            remote.update(cx, |r, _| {
                if scope != DeleteScope::Local {
                    r.delete_from_account(id);
                }
                if scope != DeleteScope::Cloud {
                    r.forget_locally(id);
                }
            });
        }
        if scope == DeleteScope::Cloud {
            self.menu = None;
            cx.notify();
        } else {
            self.delete_task(id, window, cx);
        }
    }

    /// Settings' mass delete: every session on this computer, on the account,
    /// or both.
    fn wipe_sessions(&mut self, local: bool, cloud: bool, window: &mut Window, cx: &mut Context<Self>) {
        let remote = crate::remote::Remote::global(cx);
        if cloud && let Some(remote) = &remote {
            remote.update(cx, |r, _| r.delete_all_from_account());
        }
        if !local {
            self.reload(cx);
            return;
        }
        // The Archived page lists 200; this takes them all.
        let store = self.engine.store.clone();
        let mut doomed = store.all_tasks().unwrap_or_default();
        doomed.extend(store.archived_tasks(usize::MAX >> 1).unwrap_or_default());
        if let Some(remote) = &remote {
            remote.update(cx, |r, _| doomed.iter().for_each(|t| r.forget_locally(t.id)));
        }
        let doomed: Vec<(Task, Option<Project>)> = doomed
            .into_iter()
            .map(|t| {
                let project = self.project(t.project_id).cloned();
                (t, project)
            })
            .collect();
        for (task, _) in &doomed {
            self.close_for_delete(task.id, window, cx);
        }
        self.tasks.clear();
        self.archived.clear();
        self.menu = None;
        // One after another on one thread, and the list reloads only once
        // nothing it would read is about to go.
        let engine = self.engine.clone();
        let work = cx.background_spawn(async move {
            for (task, project) in &doomed {
                if let Err(e) = engine.delete_task(task, project.as_ref()) {
                    tracing::warn!("delete: {e:#}");
                }
            }
        });
        cx.spawn(async move |this, cx| {
            work.await;
            let _ = this.update(cx, |this, cx| this.reload(cx));
        })
        .detach();
        cx.notify();
    }

    pub fn begin_rename(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.task(id) else { return };
        let title = task.title.clone();
        let style = ui::input_style(Theme::of(cx));
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_editor_style(style);
            s.set_value(title, window, cx);
            s
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        let sub = cx.subscribe_in(&input, window, move |this, input, ev: &InputEvent, _, cx| match ev {
            InputEvent::PressEnter { .. } | InputEvent::Blur => {
                let title = input.read(cx).value().trim().to_string();
                if !title.is_empty() {
                    let _ = this.engine.store.rename_task(id, &title);
                    let _ = this.engine.store.set_setting(&format!("renamed.{id}"), "1");
                }
                this.rename = None;
                this.reload(cx);
            }
            _ => {}
        });
        self._subs.push(sub);
        self.rename = Some((id, input));
        self.menu = None;
        if !self.sidebar_shown() {
            self.reveal_sidebar();
        }
        cx.notify();
    }

    pub fn side_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.active_task().cloned() else { return };
        self.home_project = Some(task.project_id);
        self.join = Some((task.id, Joined::of(&task)));
        let model = task.model.clone();
        let harness = task.harness;
        self.composer.update(cx, |c, cx| c.use_harness(harness, model, cx));
        self.info_open = false;
        self.back.push(self.route);
        self.forward.clear();
        self.go(Route::Home, window, cx);
    }

    pub fn fork_task(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.task(id).cloned() else { return };
        let Some(project) = self.project(task.project_id).cloned() else { return };
        self.info_open = false;
        // Keep the source thread's mode, model and effort (fixes the old drop).
        let run = self
            .threads
            .get(&id)
            .map(|v| v.read(cx).chat.read(cx).composer.read(cx).run_settings(cx))
            .unwrap_or_default();
        match self.engine.fork_task(&task, &project, run) {
            Ok((forked, live)) => {
                let fid = forked.id;
                self.tasks.insert(0, forked.clone());
                self.make_thread(forked, project, Some(live), window, cx);
                self.navigate(Route::Thread(fid), window, cx);
            }
            Err(e) => self.error = Some(format!("{e:#}").into()),
        }
        cx.notify();
    }

    /// Fetch a PR's head and open a new thread on it from the composer.
    fn checkout_pr(
        &mut self,
        project: Project,
        number: u64,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = project.path.clone();
        let work = cx.background_spawn(async move { dove_core::git::github::fetch_pr(&path, number) });
        cx.spawn_in(window, async move |this, cx| {
            let branch = work.await;
            let _ = this.update_in(cx, |s, window, cx| match branch {
                Ok(_) => {
                    s.home_project = Some(project.id);
                    let projects = s.projects.clone();
                    s.composer.update(cx, |c, cx| {
                        c.set_projects(projects, Some(project.id), cx);
                        cx.notify();
                    });
                    let prompt = format!(
                        "Review pull request #{number} \"{title}\". Summarize it, then list risks and missing tests."
                    );
                    crate::composer::set_text(&s.composer, &prompt, window, cx);
                    s.navigate(Route::Home, window, cx);
                }
                Err(e) => Toasts::push(
                    cx,
                    format!("Could not fetch PR #{number}"),
                    Some(format!("{e:#}")),
                    Tone::Danger,
                    None,
                ),
            });
        })
        .detach();
    }

    pub fn open_settings_section(&mut self, section: &str, window: &mut Window, cx: &mut Context<Self>) {
        let section = section.to_string();
        self.settings.update(cx, |s, cx| s.show_section(&section, cx));
        self.navigate(Route::Settings, window, cx);
    }

    /// Open Settings on one setting: its section, scrolled to it, glowing.
    pub fn reveal_setting(&mut self, row: &str, window: &mut Window, cx: &mut Context<Self>) {
        let row = row.to_string();
        self.settings.update(cx, |s, cx| s.reveal(&row, cx));
        self.navigate(Route::Settings, window, cx);
    }

    /// Leave Settings for wherever the user came from.
    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.route != Route::Settings {
            return;
        }
        if self.back.last().is_some_and(|r| *r != Route::Settings) {
            self.go_back(window, cx);
        } else {
            self.navigate(Route::Home, window, cx);
        }
    }

    /// The active thread's working directory.
    pub fn task_dir(&self) -> Option<PathBuf> {
        let task = self.active_task()?;
        task.worktree.clone().or_else(|| self.project(task.project_id).map(|p| p.path.clone()))
    }

    pub fn copy(&mut self, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.menu = None;
        cx.notify();
    }

    pub(crate) fn toggle_view_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(v) = self.active_thread().cloned() {
            v.update(cx, |v, cx| v.toggle(window, cx));
        }
    }

    // ------------------------------------------------------------ palette

    /// Search (the palette), opened on one scope.
    pub fn open_search(&mut self, scope: crate::palette::Scope, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_none() {
            self.open_palette(window, cx);
        }
        if let Some(p) = self.palette.clone() {
            p.update(cx, |p, cx| p.set_scope(scope, cx));
        }
    }

    /// The workspace of threads with no folder (`<data>/chats`).
    pub fn is_chats_project(&self, id: ProjectId) -> bool {
        let dir = crate::pages::artifacts::chats_dir();
        let norm = |p: &std::path::Path| p.to_string_lossy().trim_start_matches(r"\?\").to_lowercase();
        self.project(id).is_some_and(|p| norm(&p.path) == norm(&dir))
    }

    /// A new thread with no folder: it works in `<data>/chats`, and what it
    /// makes shows on the Artifacts page.
    pub fn new_chat_without_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = crate::pages::artifacts::chats_dir();
        let _ = std::fs::create_dir_all(&dir);
        match self.engine.store.add_project(&dir) {
            Ok(p) => {
                self.reload(cx);
                self.set_home_project(p.id, cx);
                self.new_session(window, cx);
            }
            Err(e) => {
                self.error = Some(format!("{e:#}").into());
                cx.notify();
            }
        }
    }

    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.palette = None;
            cx.notify();
            return;
        }
        let items = self.palette_items(cx);
        let root = self.task_dir();
        let store = self.engine.store.clone();
        let palette = cx.new(|cx| Palette::new(items, root, window, cx).with_messages(store));
        let sub = cx.subscribe_in(&palette, window, |this, _, ev: &PaletteEvent, window, cx| {
            this.palette = None;
            match ev {
                PaletteEvent::Pick(item) => this.run_palette(item.clone(), window, cx),
                PaletteEvent::Compose(text) => {
                    this.new_session(window, cx);
                    crate::composer::set_text(&this.composer, text, window, cx);
                    crate::composer::focus(&this.composer, window, cx);
                }
                PaletteEvent::Close => {}
            }
            cx.notify();
        });
        self._subs.push(sub);
        self.palette = Some(palette);
        cx.notify();
    }

    fn palette_items(&self, _cx: &Context<Self>) -> Vec<PaletteItem> {
        use PaletteItem as P;
        let mut items = vec![
            P::Action("New thread", icons::PEN_NEW_SQUARE, "new"),
            P::Action("All threads", icons::LIST, "threads"),
            P::Action("Team", icons::BOT, "team"),
            P::Action("Kanban", icons::WIDGET, "kanban"),
            P::Action("Automations", icons::CLOCK_CIRCLE, "automations"),
            P::Action("Pull requests", icons::PULL_REQUEST, "prs"),
            P::Action("Agents", icons::BOT, "agents"),
            P::Action("Usage", icons::CLOCK_CIRCLE, "usage"),
            P::Action("Archived threads", icons::ARCHIVE_MINIMALISTIC, "archived"),
            P::Action("Settings", icons::SETTINGS, "settings"),
            P::Action("Keyboard shortcuts", icons::KEYBOARD, "help"),
            P::Action("Toggle sidebar", icons::SIDEBAR, "sidebar"),
            P::Action("Toggle side panel", icons::PANEL_RIGHT_TOGGLE, "panel"),
            P::Action("Add project…", icons::DOCUMENT_ADD, "add_project"),
            P::Action("Switch theme (light / dark)", icons::MOON, "theme"),
            P::Action("Teach Keplorayx a skill…", icons::MAGIC_STICK_3, "teach"),
        ];
        if self.active_task().is_some() {
            items.extend([
                P::Action("Chat ⇄ Terminal", icons::TERMINAL, "toggle_terminal"),
                P::Action("Show changes", icons::FILE_CODE, "panel_changes"),
                P::Action("Show files", icons::FOLDER_TREE, "panel_files"),
                P::Action("Show subagents", icons::FORK, "panel_subagents"),
                P::Action("New shell", icons::TERMINAL, "panel_shell"),
                P::Action("Rename thread", icons::PEN, "rename"),
                P::Action("Fork thread", icons::FORK, "fork"),
                P::Action("New side chat", icons::PLUS, "side_chat"),
            ]);
        }
        items.extend(
            self.tasks
                .iter()
                .filter(|t| t.status != TaskStatus::Draft)
                .map(|t| P::Thread(t.id, t.title.clone(), t.status)),
        );
        items.extend(dove_core::teach::list().into_iter().map(|t| P::Taught(t.name, t.status.label())));
        items.extend(self.projects.iter().map(|p| P::Project(p.id, p.name.clone())));
        items.extend(self.installed.iter().filter(|i| dove_core::harness_supported(i.kind)).map(|i| P::Agent(i.kind)));
        items.extend(
            crate::settings::search_index().into_iter().map(|(_, row, title, section)| P::Setting(row, title, section)),
        );
        items
    }

    fn run_palette(&mut self, item: PaletteItem, window: &mut Window, cx: &mut Context<Self>) {
        match item {
            PaletteItem::Thread(id, _, _) => self.open_task(id, window, cx),
            PaletteItem::Message(hit) => self.open_task(hit.task, window, cx),
            PaletteItem::Project(id, _) => {
                self.set_home_project(id, cx);
                let projects = self.projects.clone();
                self.composer.update(cx, |c, cx| c.set_projects(projects, Some(id), cx));
                self.navigate(Route::Home, window, cx);
            }
            PaletteItem::Agent(kind) => {
                self.composer.update(cx, |c, cx| c.use_harness(kind, None, cx));
                self.new_session(window, cx);
            }
            PaletteItem::File(path) => {
                self.show_panel(Tab::Files, window, cx);
                self.panel.update(cx, |p, cx| p.open_path(&path, None, window, cx));
            }
            PaletteItem::Setting(row, _, _) => self.reveal_setting(row, window, cx),
            PaletteItem::Taught(name, _) => {
                if let Some(t) = dove_core::teach::list().into_iter().find(|t| t.name == name) {
                    self.new_session_with(&dove_core::teach::rehearse_prompt(&t), window, cx);
                }
            }
            PaletteItem::Action(_, _, id) => match id {
                "new" => self.new_session(window, cx),
                "threads" => self.navigate(Route::Threads, window, cx),
                "team" => self.open_teammate(None, window, cx),
                "kanban" => self.navigate(Route::Kanban, window, cx),
                "flock" => self.navigate(Route::Flock, window, cx),
                "automations" => self.navigate(Route::Automations, window, cx),
                "prs" => self.navigate(Route::PullRequests, window, cx),
                "agents" => self.navigate(Route::Agents, window, cx),
                "usage" => self.navigate(Route::Usage, window, cx),
                "archived" => self.navigate(Route::Archived, window, cx),
                "settings" => self.navigate(Route::Settings, window, cx),
                "help" => self.navigate(Route::Help, window, cx),
                "sidebar" => self.toggle_sidebar(cx),
                "panel" => self.toggle_panel(cx),
                "add_project" => self.add_project_dialog(cx),
                "theme" => {
                    let dark = Theme::of(cx).appearance.is_dark();
                    let _ = self.engine.store.set_setting("appearance", if dark { "light" } else { "dark" });
                    crate::apply_theme(&self.engine, cx);
                }
                "toggle_terminal" => self.toggle_view_mode(window, cx),
                "panel_changes" => self.show_panel(Tab::Changes, window, cx),
                "panel_files" => self.show_panel(Tab::Files, window, cx),
                "panel_subagents" => self.show_panel(Tab::Subagents, window, cx),
                "panel_shell" => {
                    self.show_panel(Tab::Terminal, window, cx);
                    self.panel.update(cx, |p, cx| p.new_shell(window, cx));
                }
                "rename" => {
                    if let Some(id) = self.active_task().map(|t| t.id) {
                        self.begin_rename(id, window, cx);
                    }
                }
                "fork" => {
                    if let Some(id) = self.active_task().map(|t| t.id) {
                        self.fork_task(id, window, cx);
                    }
                }
                "side_chat" => self.side_chat(window, cx),
                "teach" => self.new_session_with(&dove_core::teach::teach_prompt(), window, cx),
                _ => {}
            },
        }
    }

    // ------------------------------------------------------------ chrome

    fn drag_region(&self, id: &'static str, el: gpui::Div, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        el.id(id)
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, _| this.titlebar_drag = true))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.titlebar_drag = false))
            .on_mouse_down_out(cx.listener(|this, _, _, _| this.titlebar_drag = false))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, _| {
                if this.titlebar_drag && e.pressed_button == Some(MouseButton::Left) {
                    this.titlebar_drag = false;
                    window.start_window_move();
                }
            }))
            .on_click(|e, window, _| {
                if e.click_count() == 2 {
                    if cfg!(target_os = "macos") { window.titlebar_double_click() } else { window.zoom_window() }
                }
            })
    }

    /// Floating window chrome for the empty main page: sidebar and history,
    /// with no tab strip. The strip appears once a thread is open.
    fn render_bare_chrome(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let left_pad = if cfg!(target_os = "macos") { 78.0 } else { 10.0 };
        let can_back = !self.back.is_empty();
        let can_fwd = !self.forward.is_empty();
        // A history arrow with nowhere to go stays visible, just dim.
        let nav = |id: &'static str,
                   path: &'static str,
                   label: &'static str,
                   enabled: bool,
                   go: fn(&mut Shell, &mut Window, &mut Context<Shell>),
                   cx: &mut Context<Self>| {
            if enabled {
                ui::icon_button(id, path, label, &t, cx.listener(move |this, _, window, cx| go(this, window, cx)))
                    .into_any_element()
            } else {
                div()
                    .id(id)
                    .size(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(path).size(px(16.0)).text_color(t.text_muted.opacity(0.35)))
                    .into_any_element()
            }
        };
        let buttons = div()
            .flex()
            .items_center()
            .child(ui::window_button(
                "home-sidebar",
                ui::sidebar_glyph(
                    crate::motion::state_t(
                        "glyph-sidebar",
                        self.sidebar_shown(),
                        crate::motion::GLYPH_STATE,
                        cx.reduce_motion(),
                    ),
                    false,
                    16.0,
                    t.text_muted,
                ),
                "Toggle sidebar (Ctrl+B)",
                &t,
                cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)),
            ))
            .child(div().w(px(8.0)))
            .child(nav("home-back", icons::ARROW_LEFT, "Back (Alt+←)", can_back, |s, w, cx| s.go_back(w, cx), cx))
            .child(div().w(px(2.0)))
            .child(nav("home-fwd", icons::ARROW_RIGHT, "Forward (Alt+→)", can_fwd, |s, w, cx| s.go_forward(w, cx), cx));
        // Over the artwork (sidebar closed) the controls ride a small frosted
        // island, as in Zeron; with the sidebar open they sit on its top.
        // Zeron's rule: the frosted island only when the sidebar is closed
        // and there is a picture under the controls.
        let island_on = !self.sidebar_shown() && self.has_wallpaper();
        let island = div()
            .h(px(32.0))
            .px(px(6.0))
            .flex()
            .items_center()
            .rounded(px(radius::RADIUS))
            .when(island_on, |d| {
                d.bg(t.glass_overlay()).border_1().border_color(t.wash(0.07)).shadow(vec![gpui::BoxShadow {
                    color: gpui::hsla(0.0, 0.0, 0.0, 0.22),
                    offset: gpui::point(px(0.0), px(2.0)),
                    blur_radius: px(10.0),
                    spread_radius: px(0.0),
                    inset: false,
                }])
            })
            .child(buttons);
        let cluster = div().ml(px(if cfg!(target_os = "macos") { left_pad } else { 6.0 })).child(if island_on {
            ui::frosted(radius::RADIUS, 20.0, island).into_any_element()
        } else {
            island.into_any_element()
        });
        let bar = div()
            .relative()
            .h(px(Theme::TITLEBAR_HEIGHT))
            .pt(px(Theme::TITLEBAR_TOP_PAD))
            .flex()
            .items_center()
            .child(cluster);
        let bar = self.drag_region("home-chrome", bar, cx);
        div().absolute().top_0().left_0().right_0().child(bar).into_any_element()
    }

    /// The slim bar's middle: the open chat's mark and title (double-click
    /// to rename). Pages leave it empty: each one titles itself.
    fn render_bar_title(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Route::Thread(id) = self.route else { return div().into_any_element() };
        let Some(task) = self.task(id).cloned() else { return div().into_any_element() };
        let mark = match self.persona_for(id) {
            Some(p) => crate::team::avatar(p, 14.0, t).into_any_element(),
            None => icon(ui::harness_icon(task.harness))
                .size(px(14.0))
                .text_color(ui::harness_tint(task.harness, t, t.text_muted))
                .into_any_element(),
        };
        // Zeron's title: the agent's mark and the name. Double-click renames;
        // right-click opens the thread's menu, as its sidebar row does.
        div()
            .id("tb-title")
            .h(px(28.0))
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.0))
            .occlude()
            .tooltip(ui::tip(format!("{} (double-click to rename)", task.title)))
            .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                if e.click_count() == 2 {
                    this.begin_rename(id, window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.close_menus();
                    this.menu = Some(ChatMenu { task: id, at: e.position, copy_open: false, confirm_delete: None });
                    this.menu_opened(window, cx);
                    cx.notify();
                }),
            )
            .child(div().flex_none().flex().items_center().justify_center().child(mark))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(t.text.opacity(0.85))
                    .child(task.title.clone()),
            )
            .into_any_element()
    }

    fn render_topbar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let left_pad = if cfg!(target_os = "macos") { 78.0 } else { 10.0 };
        let can_back = !self.back.is_empty();
        let can_fwd = !self.forward.is_empty();
        // With the sidebar open the cluster is exactly the sidebar's width, so
        // the tabs start where the chat starts. The bar repeats the body's
        // columns (sidebar | chat | dock) instead of spilling across them.
        // Zeron's cluster: the sidebar toggle, a group gap, then both history
        // arrows (dim when there is nowhere to go, so nothing shifts).
        let arrow = |id: &'static str,
                     path: &'static str,
                     label: &'static str,
                     enabled: bool,
                     go: fn(&mut Shell, &mut Window, &mut Context<Shell>),
                     cx: &mut Context<Self>| {
            if enabled {
                let color = crate::motion::hover_blend(id, t.text_muted, t.text);
                quiet_bar_button(
                    id,
                    label,
                    icon(path).size(px(16.0)).text_color(color),
                    cx.listener(move |this, _, window, cx| go(this, window, cx)),
                )
                .into_any_element()
            } else {
                div()
                    .id(id)
                    .size(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(path).size(px(16.0)).text_color(t.text_muted.opacity(0.35)))
                    .into_any_element()
            }
        };
        // Tabs are optional (Settings › General): by default the bar is the
        // slim one, with the chat's title where the tabs would be.
        let tabs_on = self.engine.store.flag("topbar_tabs", false);
        let side = self.sidebar_shown() || self.route == Route::Settings;
        let cluster = div()
            .flex()
            .flex_none()
            .h_full()
            .items_center()
            .pt(px(Theme::TITLEBAR_TOP_PAD))
            .pl(px(left_pad))
            .when(side, |d| d.w(px(self.sidebar_w)))
            .when(!side, |d| d.pr(px(8.0)))
            .child({
                let color = crate::motion::hover_blend("tb-sidebar", t.text_muted, t.text);
                quiet_bar_button(
                    "tb-sidebar",
                    "Toggle sidebar (Ctrl+B)",
                    ui::sidebar_glyph(
                        crate::motion::state_t(
                            "glyph-sidebar",
                            self.sidebar_shown(),
                            crate::motion::GLYPH_STATE,
                            cx.reduce_motion(),
                        ),
                        false,
                        16.0,
                        color,
                    ),
                    cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)),
                )
            })
            .child(div().w(px(8.0)))
            .child(arrow("tb-back", icons::ARROW_LEFT, "Back (Alt+←)", can_back, |s, w, cx| s.go_back(w, cx), cx))
            .child(div().w(px(2.0)))
            .child(arrow("tb-fwd", icons::ARROW_RIGHT, "Forward (Alt+→)", can_fwd, |s, w, cx| s.go_forward(w, cx), cx))
            // The new-thread page is where `+` leads, so it shows neither.
            .when(!tabs_on && self.route != Route::Home, |d| {
                let color = crate::motion::hover_blend("tb-new", t.text_muted, t.text);
                d.child(div().w(px(8.0))).child(quiet_bar_button(
                    "tb-new",
                    "New thread (Ctrl+N)",
                    icon(icons::PLUS).size(px(16.0)).text_color(color),
                    cx.listener(|this, _, window, cx| this.new_session(window, cx)),
                ))
            });

        let in_thread = matches!(self.route, Route::Thread(_));
        let flock = self.route == Route::Flock;
        let panel_open = (in_thread || flock) && self.panel_shown();
        let body_open = panel_open && self.panel_open;
        let takeover = body_open && self.panel.read(cx).expanded;
        let dock_left = self.panel_left();
        let viewport = f32::from(window.viewport_size().width);
        // Zeron's trailing layout: the explorer and panel toggles are fixed
        // right-edge anchors (over the explorer column when it is open), and
        // the surface tabs fill from the panel's seam up to them.
        let right_pad = ui::caption_reserve() + BAR_EDGE_INSET;
        let slots = if in_thread {
            HEADER_BUTTON * 2.0 + HEADER_GAP
        } else if flock {
            HEADER_BUTTON
        } else {
            0.0
        };
        let explorer_w = if panel_open { self.explorer_width(window) } else { 0.0 };
        let controls_w = (explorer_w - right_pad).max(slots);
        // Toggle, group gap, back, 2 px, forward, group gap, `+`, trailing pad.
        let cluster_w = if side { self.sidebar_w } else { left_pad + ui::BUTTON * 3.0 + 8.0 + 2.0 + 8.0 + 8.0 };
        let tools_w = if in_thread { HEADER_BUTTON + 8.0 } else { 0.0 } + controls_w + right_pad;
        let room = viewport - cluster_w - tools_w;
        let tabs = if flock {
            self.workspace.update(cx, |w, cx| w.render_bar(cx))
        } else if tabs_on {
            self.render_tabs(room, &t, cx)
        } else {
            // The chat column's 16 px gutter, or a 12 px group gap past the
            // cluster when the sidebar is closed (the cluster pads 8).
            let title = self.render_bar_title(&t, cx);
            div().min_w_0().pl(px(if side { 16.0 } else { 4.0 })).child(title).into_any_element()
        };
        let strip = body_open.then(|| {
            let total =
                if takeover { viewport - if side { self.sidebar_w } else { 0.0 } } else { self.panel_width(window) };
            let width = if dock_left && !takeover { total } else { total - right_pad - controls_w };
            let width = width.min(viewport - cluster_w - right_pad - controls_w).max(0.0);
            div()
                .w(px(width))
                .h_full()
                .flex_none()
                .pt(px(Theme::TITLEBAR_TOP_PAD))
                .child(self.panel.update(cx, |panel, cx| panel.render_strip(cx)))
        });
        let running_lanes = {
            let (subagents, chats) = self.panel.read(cx).lanes();
            subagents.iter().any(|a| a.2) || chats.iter().any(|c| c.2)
        };
        // The thread card's toggle ends the chat's stretch of the bar.
        let info_toggle = (in_thread && !takeover).then(|| {
            let open = self.info_open;
            header_icon_button(
                "tb-info",
                if open { "Hide thread details" } else { "Show thread details" },
                ui::corner_card_glyph(16.0, t.text_muted),
                cx.listener(|this, _, _, cx| {
                    this.info_open = !this.info_open;
                    cx.notify();
                }),
            )
            .when(open, |d| d.bg(crate::theme::wash(0.09)))
            // Subagents and side chats at work, seen from the closed card too.
            .when(running_lanes, |d| {
                d.relative().child(div().absolute().top(px(5.0)).right(px(5.0)).child(ui::dot(t.busy, 5.0)))
            })
        });
        let files_toggle = in_thread.then(|| {
            let open = panel_open && explorer_w > 0.0;
            let label = if open { "Hide files panel" } else { "Show files panel" };
            header_icon_button(
                "tb-files",
                label,
                icon(icons::FOLDER_TREE).size(px(16.0)).text_color(t.text_muted),
                cx.listener(move |this, _, _, cx| this.set_explorer(!open, cx)),
            )
            .when(open, |d| d.bg(crate::theme::wash(0.09)))
        });
        let panel_toggle = (in_thread || flock).then(|| {
            header_icon_button(
                "tb-panel",
                "Toggle side panel (Ctrl+\\)",
                ui::sidebar_glyph(
                    crate::motion::state_t("tb-panel", body_open, crate::motion::GLYPH_STATE, cx.reduce_motion()),
                    true,
                    16.0,
                    t.text_muted,
                ),
                cx.listener(|this, _, _, cx| this.toggle_panel(cx)),
            )
        });
        // Over artwork the strip stays clear: the picture (and, in a thread,
        // its frost) is already painted for this column, so the tabs sit on
        // the same surface as the page. Everywhere else it is the solid bar.
        let home_art = matches!(self.route, Route::Home) && self.has_wallpaper();
        let thread_art = self.full_art_route() && self.has_wallpaper();
        // On glass the bar has no fill at all: the window's one surface runs
        // from the top edge down, with no band above the chat.
        let solid = !home_art && !thread_art && !t.is_frost();
        let surface = (!takeover).then(|| {
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .when(solid, |d| d.bg(t.bg))
                .pt(px(Theme::TITLEBAR_TOP_PAD))
                .pr(px(8.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(div().flex_1().min_w_0().h_full().flex().items_center().child(tabs))
                .children(info_toggle)
        });
        // The anchors and the caption clearance. Over the dock its own fill
        // shows through; over the chat they carry the chat's.
        let anchors = div()
            .w(px(controls_w + right_pad))
            .h_full()
            .flex_none()
            .when(solid && (!panel_open || dock_left), |d| d.bg(t.bg))
            .pt(px(Theme::TITLEBAR_TOP_PAD))
            .pr(px(right_pad))
            .flex()
            .items_center()
            .justify_end()
            .gap(px(HEADER_GAP))
            .children(files_toggle)
            .children(panel_toggle);
        let bar = div()
            .relative()
            .h(px(Theme::TITLEBAR_HEIGHT))
            .flex()
            .items_center()
            .when(!side && solid, |d| d.bg(t.bg))
            .child(cluster);
        let bar = if dock_left { bar.children(strip).children(surface) } else { bar.children(surface).children(strip) };
        let bar = self.drag_region("titlebar", bar.child(anchors), cx);
        div().relative().flex_none().child(bar).into_any_element()
    }

    /// Open threads as tabs: status glyph, title, close. Middle-click closes;
    /// drag to reorder.
    ///
    /// Tabs split `room` evenly, like a browser's, but never below a readable
    /// width (status, mark and about ten characters). What does not fit goes
    /// to a "+N" menu, and the active tab always stays on the strip. With no
    /// room for even one tab, the strip shows the active thread's title.
    fn render_tabs(&mut self, room: f32, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        const TAB_MAX: f32 = 210.0;
        const TAB_MIN: f32 = 112.0;
        const GAP: f32 = 2.0;
        const MORE_W: f32 = 40.0;
        let active = match self.route {
            Route::Thread(id) => Some(id),
            _ => None,
        };
        let all: Vec<(usize, TaskId, Task)> = self
            .tabs
            .clone()
            .into_iter()
            .enumerate()
            .filter_map(|(i, id)| self.task(id).cloned().map(|task| (i, id, task)))
            .collect();
        let total = all.len();
        // Room for the tabs once the `+` (and, when needed, "+N") is placed.
        let avail = room - ui::BUTTON - GAP;
        let fits = |k: usize| {
            let more = if k < total { MORE_W + GAP } else { 0.0 };
            k == 0 || (avail - more - GAP * k as f32) / k as f32 >= TAB_MIN
        };
        let mut shown_n = total;
        while !fits(shown_n) {
            shown_n -= 1;
        }
        // The first `shown_n` in order; the active tab takes the last slot
        // when it would otherwise overflow.
        let mut shown: Vec<(usize, TaskId, Task)> = all.iter().take(shown_n).cloned().collect();
        if shown_n > 0
            && let Some(act) = active
            && !shown.iter().any(|(_, id, _)| *id == act)
            && let Some(entry) = all.iter().find(|(_, id, _)| *id == act)
        {
            shown.pop();
            shown.push(entry.clone());
        }
        let hidden: Vec<(usize, TaskId, Task)> =
            all.iter().filter(|(_, id, _)| !shown.iter().any(|(_, s, _)| s == id)).cloned().collect();
        let k = shown.len().max(1) as f32;
        let more_w = if hidden.is_empty() { 0.0 } else { MORE_W + GAP };
        let width = ((avail - more_w - GAP * k) / k).clamp(TAB_MIN, TAB_MAX).floor();
        // Full: glyph, avatar, title, close. Narrow: avatar, title.
        let full = width >= 128.0;
        let mut strip = div().id("thread-tabs").flex().items_center().gap(px(GAP)).min_w_0().overflow_hidden();
        // No room for a tab: the active thread's title stands in.
        if shown.is_empty()
            && let Some(task) = active.and_then(|id| self.task(id)).cloned()
        {
            strip = strip.child(
                div()
                    .id("tab-title")
                    .min_w_0()
                    .px(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .tooltip(ui::tip(task.title.clone()))
                    .child(kit::status_glyph(
                        SharedString::from(format!("tab-st-{}", task.id)),
                        task.status,
                        task.unread,
                        t,
                    ))
                    .child(
                        div().min_w_0().truncate().text_size(ty::SMALL).text_color(t.text).child(task.title.clone()),
                    ),
            );
        }
        for (i, id, task) in shown {
            let selected = active == Some(id);
            let attention = matches!(task.status, TaskStatus::Running | TaskStatus::NeedsInput) || task.unread;
            let show_glyph = (task.status != TaskStatus::Idle || task.unread) && (full || attention);
            let show_close = full || selected;
            strip = strip.child(
                div()
                    .id(SharedString::from(format!("tab-{id}")))
                    .group(SharedString::from(format!("tab-{id}")))
                    .h(px(28.0))
                    .w(px(width))
                    .flex_none()
                    .pl(px(9.0))
                    .pr(px(if show_close { 4.0 } else { 9.0 }))
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .rounded(px(radius::RADIUS))
                    .occlude()
                    .cursor_pointer()
                    .when(selected && matches!(self.route, Route::Home) && self.has_wallpaper(), |d| {
                        d.bg(t.bg.opacity(0.42))
                    })
                    // The full title on hover wherever it may be cut short,
                    // and always on the active tab.
                    .when(!full || selected, |d| d.tooltip(ui::tip(task.title.clone())))
                    .relative()
                    .when(show_glyph, |d| {
                        d.child(kit::status_glyph(
                            SharedString::from(format!("tab-st-{id}")),
                            task.status,
                            task.unread,
                            t,
                        ))
                    })
                    .map(|d| {
                        d.child(match self.persona_for(id) {
                            Some(p) => crate::team::avatar(p, 15.0, t).into_any_element(),
                            None => icon(ui::harness_icon(task.harness))
                                .size(px(13.0))
                                .text_color(ui::harness_tint(task.harness, t, t.text_muted))
                                .into_any_element(),
                        })
                    })
                    .map(|d| {
                        d.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(ty::SMALL)
                                .when(task.unread && !selected, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                                .text_color(if selected { t.text } else { t.text_muted })
                                .group_hover(SharedString::from(format!("tab-{id}")), |s| s.text_color(t.text))
                                .child(task.title.clone()),
                        )
                    })
                    .when(show_close, |d| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("tab-x-{id}")))
                                .size(px(18.0))
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .group(SharedString::from(format!("tab-x-{id}")))
                                .when(!selected, |d| {
                                    d.invisible().group_hover(SharedString::from(format!("tab-{id}")), |s| s.visible())
                                })
                                .tooltip(ui::tip("Close tab (Ctrl+W)"))
                                .child(
                                    icon(icons::CLOSE)
                                        .size(px(10.0))
                                        .text_color(t.text_faint)
                                        .group_hover(SharedString::from(format!("tab-x-{id}")), |s| {
                                            s.text_color(t.text)
                                        }),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.close_tab(id, window, cx);
                                })),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| this.navigate(Route::Thread(id), window, cx)))
                    .on_mouse_up(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| this.close_tab(id, window, cx)),
                    )
                    .on_drag(TabDrag { id, title: task.title.clone().into() }, |d, _, _, cx| cx.new(|_| d.clone()))
                    .drag_over::<TabDrag>(move |s, _, _, cx| s.bg(Theme::of(cx).accent.opacity(0.12)))
                    .on_drop(cx.listener(move |this, from: &TabDrag, _, cx| {
                        if let Some(src) = this.tabs.iter().position(|t| *t == from.id) {
                            let item = this.tabs.remove(src);
                            let dst = i.min(this.tabs.len());
                            this.tabs.insert(dst, item);
                            this.save_tabs();
                            cx.notify();
                        }
                    })),
            );
        }
        if !hidden.is_empty() {
            strip = strip.child(self.render_tab_overflow(&hidden, t, cx));
        }
        strip = strip.child(
            quiet_bar_button(
                "tab-new",
                "New thread (Ctrl+N)",
                icon(icons::PLUS).size(px(16.0)).text_color(crate::motion::hover_blend(
                    "tab-new",
                    t.text_muted,
                    t.text,
                )),
                cx.listener(|this, _, window, cx| this.new_session(window, cx)),
            )
            .flex_none(),
        );
        strip.into_any_element()
    }

    /// The "+N" chip and its menu of the tabs that did not fit.
    fn render_tab_overflow(
        &mut self,
        hidden: &[(usize, TaskId, Task)],
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::shell_menu::{Item, act};
        let open = self.tab_overflow_open;
        let attention = hidden.iter().any(|(_, _, task)| {
            matches!(task.status, TaskStatus::NeedsInput) || (task.unread && task.status != TaskStatus::Running)
        });
        let chip = div()
            .id("tab-more")
            .h(px(28.0))
            .min_w(px(40.0))
            .px(px(8.0))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(5.0))
            .rounded(px(radius::RADIUS))
            .cursor_pointer()
            .occlude()
            .text_size(ty::SMALL)
            .role(gpui::Role::Button)
            .aria_label("More tabs")
            .tooltip(ui::tip("More tabs"))
            .when(open, |d| d.bg(ui::selected_bg(t)).text_color(t.text))
            .when(!open, |d| {
                crate::motion::hover_fade(d, "tab-more", (t.wash(0.0), t.wash(0.0)), Some((t.text_muted, t.text)))
            })
            .when(attention, |d| d.child(ui::dot(t.warning, 5.0)))
            .child(format!("+{}", hidden.len()))
            .on_click(cx.listener(|this, _, window, cx| {
                if this.tab_overflow_open {
                    this.close_menus();
                } else {
                    this.close_menus();
                    this.tab_overflow_open = true;
                    this.menu_opened(window, cx);
                }
                cx.notify();
            }));
        let exit = if open { None } else { self.menus.overflow_exit() };
        let menu = (open || exit.is_some()).then(|| {
            let items: Vec<Item> = hidden
                .iter()
                .map(|(_, id, task)| {
                    let id = *id;
                    let close = div()
                        .id(SharedString::from(format!("tab-more-x-{id}")))
                        .size(px(20.0))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .rounded(px(radius::RADIUS))
                        .hover(|s| s.bg(t.wash(0.14)))
                        .tooltip(ui::tip("Close tab"))
                        .child(icon(icons::CLOSE).size(px(10.0)).text_color(t.text_faint))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_tab(id, window, cx);
                        }));
                    Item::row(
                        SharedString::from(format!("tab-more-{id}")),
                        Some(ui::harness_icon(task.harness)),
                        task.title.clone(),
                        act(move |this, window, cx| this.navigate(Route::Thread(id), window, cx)),
                    )
                    .trailing(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .when(task.status != TaskStatus::Idle || task.unread, |d| {
                                d.child(kit::status_glyph(
                                    SharedString::from(format!("tab-more-st-{id}")),
                                    task.status,
                                    task.unread,
                                    t,
                                ))
                            })
                            .child(close),
                    )
                })
                .collect();
            let live = exit.is_none();
            let body = self.menu_list(items, 280.0, live, t, cx);
            let card = self.menu_surface(t, body, exit);
            deferred(
                anchored().anchor(Anchor::TopLeft).snap_to_window_with_margin(px(8.0)).child(
                    div()
                        .mt(px(32.0))
                        .when(live, |d| {
                            d.on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.close_menus();
                                cx.notify();
                            }))
                        })
                        .child(card),
                ),
            )
            .with_priority(2)
        });
        div().relative().flex_none().child(chip).children(menu).into_any_element()
    }

    /// The dock tabs on the bar: the main ones as buttons, the rest in a ⋯
    /// menu. A button opens the dock on that tab, or closes the dock when
    /// that tab is already what it shows.
    #[allow(dead_code)]
    fn render_dock_tabs(&mut self, panel_open: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use crate::shell_menu::{Item, act};
        const MAIN: [Tab; 4] = [Tab::Terminal, Tab::Changes, Tab::Browser, Tab::Files];
        const MORE: [Tab; 5] = [Tab::History, Tab::Plan, Tab::Subagents, Tab::Chats, Tab::SideChat];
        let on = |tab: Tab, cx: &gpui::App| panel_open && self.panel.read(cx).showing_kind(tab);
        let mut row = div().flex().items_center().gap(px(2.0));
        for tab in MAIN {
            let lit = on(tab, cx);
            let id = match tab {
                Tab::Terminal => "tb-tab-terminal",
                Tab::Changes => "tb-tab-diffs",
                Tab::Browser => "tb-tab-browser",
                _ => "tb-tab-files",
            };
            let color = if lit { t.text } else { crate::motion::hover_blend(id, t.text_muted, t.text) };
            row = row.child(
                quiet_bar_button(
                    id,
                    tab.label(),
                    icon(tab.glyph()).size(px(16.0)).text_color(color),
                    cx.listener(move |this, _, window, cx| {
                        if lit {
                            this.toggle_panel(cx);
                        } else {
                            this.show_panel(tab, window, cx);
                        }
                    }),
                )
                .when(lit, |d| d.bg(ui::selected_bg(t))),
            );
        }
        let open = self.dock_more_open;
        let lit_more = MORE.iter().any(|tab| on(*tab, cx));
        let color = if open || lit_more { t.text } else { crate::motion::hover_blend("tb-more", t.text_muted, t.text) };
        let button = quiet_bar_button(
            "tb-more",
            "More panels",
            icon(icons::MORE_HORIZONTAL).size(px(16.0)).text_color(color),
            cx.listener(|this, _, window, cx| {
                let was = this.dock_more_open;
                this.close_menus();
                if !was {
                    this.dock_more_open = true;
                    this.menu_opened(window, cx);
                }
                cx.notify();
            }),
        )
        .when(open, |d| d.bg(ui::selected_bg(t)));
        let exit = if open { None } else { self.menus.dock_more_exit() };
        let menu = (open || exit.is_some()).then(|| {
            let items: Vec<Item> = MORE
                .into_iter()
                .map(|tab| {
                    Item::row(
                        SharedString::from(format!("tb-more-{}", tab.label())),
                        Some(tab.glyph()),
                        tab.label(),
                        act(move |this, window, cx| this.show_panel(tab, window, cx)),
                    )
                    .trailing(div().text_size(ty::META).text_color(t.text_faint).child(tab.blurb()))
                })
                .collect();
            let live = exit.is_none();
            let body = self.menu_list(items, 300.0, live, t, cx);
            let card = self.menu_surface(t, body, exit);
            deferred(
                anchored().anchor(Anchor::TopRight).snap_to_window_with_margin(px(8.0)).child(
                    div()
                        .mt(px(32.0))
                        .when(live, |d| {
                            d.on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.close_menus();
                                cx.notify();
                            }))
                        })
                        .child(card),
                ),
            )
            .with_priority(2)
        });
        row.child(div().relative().flex_none().child(button).children(menu)).into_any_element()
    }

    /// Local sessions this computer would upload. Another device's sessions
    /// do not count; ones an earlier "Start fresh" held back do, since
    /// "Bring my work" releases them.
    pub(crate) fn local_sessions_to_bring(&self) -> usize {
        let counts = |tasks: &[Task]| {
            tasks
                .iter()
                .filter(|task| self.engine.store.setting(&format!("account_device.{}", task.id)).is_none())
                .count()
        };
        counts(&self.tasks) + counts(&self.archived)
    }

    /// Sessions on the account that this computer does not have yet.
    fn account_sessions_waiting(&self) -> usize {
        let Some((_, chats)) = &self.pending_account else { return 0 };
        let Some(rows) = chats.as_array() else { return 0 };
        rows.iter()
            .filter(|chat| {
                chat.get("id")
                    .and_then(|v| v.as_str())
                    .and_then(TaskId::parse)
                    .is_some_and(|id| self.engine.store.task(id).ok().flatten().is_none())
            })
            .count()
    }

    pub(crate) fn account_detail(&self, cx: &gpui::App) -> String {
        let Some(remote) = crate::remote::Remote::global(cx) else {
            return "Local".into();
        };
        let status = &remote.read(cx).status;
        if !status.signed_in {
            return "Local".into();
        }
        if self.sync_postponed || matches!(self.sync_offer, SyncOffer::Ready) {
            return "Sync is ready".into();
        }
        status.email.clone().unwrap_or_else(|| "Signed in".into())
    }

    /// Zeron's sign-in pages: finish in the browser, then bring this
    /// computer's sessions up or start fresh, with the counts and progress as
    /// it goes. A page of its own on the window's surface, not a dialog.
    fn render_sync_offer(&mut self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if matches!(self.sync_offer, SyncOffer::Closed) {
            return None;
        }
        let status = crate::remote::Remote::global(cx).map(|remote| remote.read(cx).status.clone()).unwrap_or_default();
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        let phrase = local_work_phrase(self.local_sessions_to_bring());
        let waiting = self.account_sessions_waiting();
        let uploaded = self.sync_uploaded;
        let signed_in_line =
            status.email.clone().map_or_else(|| "You're signed in".to_string(), |e| format!("Signed in as {e}"));
        let check = || icon(icons::CHECK).size(px(14.0)).text_color(t.success).into_any_element();
        let spin = |id: &'static str| kit::spinner(id, 12.0, t.text_muted).into_any_element();
        let line = |leading: AnyElement, text: String, strong: bool| sync_status_block(t, leading, text, strong);
        // The way back on the left, the way forward on the right.
        let ghost = |id: &'static str, label: &'static str, click: fn(&mut Shell, &mut Context<Shell>)| {
            sync_ghost(t, id, label).on_click(cx.listener(move |this, _, _, cx| click(this, cx))).into_any_element()
        };
        let primary = |id: &'static str, label: &'static str, click: fn(&mut Shell, &mut Context<Shell>)| {
            sync_primary(t, id, label).on_click(cx.listener(move |this, _, _, cx| click(this, cx))).into_any_element()
        };
        let bring = |this: &mut Shell, cx: &mut Context<Shell>| this.choose_sync(true, cx);
        let fresh = |this: &mut Shell, cx: &mut Context<Shell>| this.choose_sync(false, cx);
        let later = |this: &mut Shell, cx: &mut Context<Shell>| this.postpone_sync(cx);
        let done = |this: &mut Shell, cx: &mut Context<Shell>| {
            this.sync_offer = SyncOffer::Closed;
            cx.notify();
        };
        let retry = |this: &mut Shell, cx: &mut Context<Shell>| this.choose_sync(true, cx);
        let cancel = |this: &mut Shell, cx: &mut Context<Shell>| {
            this.sync_offer = SyncOffer::Closed;
            if let Some(remote) = crate::remote::Remote::global(cx) {
                remote.update(cx, |r, cx| r.cancel_sign_in(cx));
            }
            cx.notify();
        };
        let again = |_: &mut Shell, cx: &mut Context<Shell>| {
            if let Some(remote) = crate::remote::Remote::global(cx) {
                remote.update(cx, |r, cx| r.sign_in(cx));
            }
        };
        let progress = |fraction: f32, current: Option<String>| {
            div()
                .mt(px(28.0))
                .rounded(px(10.0))
                .bg(t.wash(0.04))
                .px(px(16.0))
                .py(px(12.0))
                .min_h(px(44.0))
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(10.0))
                .when_some(current, |d, current| {
                    d.child(div().truncate().text_size(px(12.5)).text_color(t.text_muted).child(current))
                })
                .child(
                    div()
                        .h(px(4.0))
                        .w_full()
                        .rounded(px(2.0))
                        .bg(t.wash(0.12))
                        .child(div().h_full().rounded(px(2.0)).bg(t.accent_strong).w(relative(fraction.max(0.04)))),
                )
                .into_any_element()
        };
        let share =
            |done: usize, total: usize| if total == 0 { 0.0 } else { (done as f32 / total as f32).clamp(0.0, 1.0) };

        let (key, title, subtitle, blocks, back, forward): (
            &'static str,
            &'static str,
            String,
            Vec<AnyElement>,
            Option<AnyElement>,
            Vec<AnyElement>,
        ) = match &self.sync_offer {
            SyncOffer::Closed => return None,
            SyncOffer::Waiting => {
                let block = match &status.error {
                    Some(error) => div()
                        .mt(px(28.0))
                        .px(px(8.0))
                        .text_size(px(12.0))
                        .line_height(px(17.0))
                        .text_color(t.danger)
                        .child(error.clone())
                        .into_any_element(),
                    None if status.signed_in => {
                        line(spin("sync-waiting-spin"), "Connecting to your account…".into(), false)
                    }
                    None => line(spin("sync-waiting-spin"), "Waiting for your browser…".into(), false),
                };
                (
                    "sync-waiting",
                    "Finish logging in",
                    "We opened your browser to log in to Woozlit. Come back when you're done, this page moves on by itself.".into(),
                    vec![block],
                    Some(ghost("sync-enable-cancel", "Cancel", cancel)),
                    vec![primary("sync-enable-open-browser", "Open browser again", again)],
                )
            }
            SyncOffer::Ready => {
                let mut blocks = vec![line(check(), signed_in_line, true)];
                if waiting > 0 {
                    blocks.push(
                        div()
                            // Stacked on the block above, not a section away.
                            .mt(px(-20.0))
                            .child(line(
                                icon(icons::CLOUD).size(px(14.0)).text_color(t.text_muted).into_any_element(),
                                format!(
                                    "{waiting} session{} on your account will come to this computer.",
                                    plural(waiting)
                                ),
                                false,
                            ))
                            .into_any_element(),
                    );
                }
                let subtitle = match &phrase {
                    Some(phrase) => {
                        format!("Bring {phrase} from this device into your synced workspace, or start it fresh.")
                    }
                    None => "Keplorayx can switch to your synced workspace now.".into(),
                };
                let forward = if phrase.is_some() {
                    vec![
                        ghost("sync-switch-fresh", "Start fresh", fresh),
                        primary("sync-switch-import", "Bring my work", bring),
                    ]
                } else {
                    vec![primary("sync-switch-now", "Switch now", fresh)]
                };
                (
                    "sync-ready",
                    "Sync is ready",
                    subtitle,
                    blocks,
                    Some(ghost("sync-switch-later", "Later", later)),
                    forward,
                )
            }
            SyncOffer::Switching { bring } => (
                "sync-switching",
                "Setting up sync",
                if *bring {
                    "Handing your sessions over to your account. They come along next.".into()
                } else {
                    "Connecting this computer to your account.".into()
                },
                vec![line(spin("sync-switching-spin"), "Switching to your synced workspace…".into(), false)],
                None,
                Vec::new(),
            ),
            SyncOffer::Uploading { done, total, title } => (
                "sync-uploading",
                "Bringing your work over",
                format!("Uploading session {} of {total}", (*done + 1).min(*total)),
                vec![progress(share(*done, *total), title.clone())],
                None,
                Vec::new(),
            ),
            SyncOffer::Importing { done, total, title } => (
                "sync-importing",
                "Bringing your work over",
                if *total == 0 {
                    "Looking for your sessions…".into()
                } else {
                    format!("Importing session {} of {total}", (*done + 1).min(*total))
                },
                vec![progress(share(*done, *total), title.clone())],
                None,
                Vec::new(),
            ),
            SyncOffer::Done { imported, skipped, .. } => {
                let (imported, skipped) = (*imported, *skipped);
                let subtitle = match (uploaded, imported) {
                    (0, 0) if skipped == 0 => "Your synced workspace is ready.".to_string(),
                    (0, 0) => format!("{skipped} session{} already here.", plural(skipped)),
                    (up, 0) => format!("{up} session{} moved into your synced workspace.", plural(up)),
                    (0, down) => format!("{down} session{} from your account now on this computer.", plural(down)),
                    (up, down) => format!(
                        "{up} session{} moved into your synced workspace, {down} came down from your account.",
                        plural(up)
                    ),
                };
                (
                    "sync-done",
                    "Your work is synced",
                    subtitle,
                    Vec::new(),
                    None,
                    vec![primary("sync-switch-done", "Continue", done)],
                )
            }
            SyncOffer::Failed(message) => (
                "sync-failed",
                "Import didn't finish",
                "Anything already imported is kept; retrying only copies what's missing.".into(),
                vec![
                    div()
                        .mt(px(12.0))
                        .px(px(8.0))
                        .text_size(px(12.0))
                        .line_height(px(17.0))
                        .text_color(t.danger)
                        .child(message.clone())
                        .into_any_element(),
                ],
                Some(ghost("import-failed-dismiss", "Later", later)),
                vec![primary("import-failed-retry", "Retry import", retry)],
            ),
        };
        let column = div()
            .w_full()
            .max_w(px(640.0))
            .px(px(40.0))
            .flex()
            .flex_col()
            .child(crate::motion::fade_quick(
                key,
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px(px(4.0))
                            .text_size(px(20.0))
                            .line_height(px(26.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.text)
                            .child(title),
                    )
                    .child(
                        div()
                            .mt(px(4.0))
                            .px(px(4.0))
                            .min_w_0()
                            .text_size(px(13.0))
                            .line_height(px(17.0))
                            .text_color(t.text_muted)
                            .child(subtitle),
                    )
                    .children(blocks),
            ))
            .child(
                div()
                    .flex_none()
                    .pt(px(24.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().children(back.map(|b| div().ml(px(-8.0)).child(b))))
                    .child(div().flex().items_center().gap(px(8.0)).children(forward)),
            );
        let strip = self.drag_region(
            "sync-titlebar",
            div().absolute().top_0().left_0().right_0().h(px(Theme::TITLEBAR_HEIGHT)),
            cx,
        );
        Some(
            div()
                .id("sync-page")
                .absolute()
                .inset_0()
                .occlude()
                .bg(t.surface)
                .child(strip)
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .pt(px(Theme::TITLEBAR_HEIGHT))
                        .pb(px(Theme::TITLEBAR_HEIGHT / 2.0))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .child(column),
                )
                .into_any_element(),
        )
    }

    /// Shown once, and only when no Woozlit session is stored. Use locally
    /// stays on this computer. Log in is what a phone or another computer needs.
    fn render_first_run(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let home = dove_core::hosts::dove_home();
        let signed_in = home.join("mesh").join("account.json").is_file()
            || home.join("account.json").is_file()
            || home.join("session.json").is_file();
        if signed_in || self.engine.store.setting("first_run_seen").as_deref() == Some("1") {
            return None;
        }
        let stay = div()
            .id("first-run-local")
            .px(px(14.0))
            .py(px(8.0))
            .rounded(px(radius::ROW))
            .bg(t.wash(0.08))
            .cursor_pointer()
            .child("Use locally")
            .on_click(cx.listener(|this, _, _, cx| {
                let _ = this.engine.store.set_setting("first_run_seen", "1");
                cx.notify();
            }));
        let login = div()
            .id("first-run-login")
            .px(px(14.0))
            .py(px(8.0))
            .rounded(px(radius::ROW))
            .bg(t.accent)
            .text_color(t.surface)
            .cursor_pointer()
            .child("Log in")
            .on_click(cx.listener(|this, _, _, cx| {
                let _ = this.engine.store.set_setting("first_run_seen", "1");
                if let Some(remote) = crate::remote::Remote::global(cx) {
                    remote.update(cx, |r, cx| r.sign_in(cx));
                }
                cx.notify();
            }));
        Some(
            div()
                .absolute()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::hsla(0.0, 0.0, 0.0, if t.appearance.is_dark() { 0.55 } else { 0.32 }))
                .child(
                    div()
                        .w(px(420.0))
                        .p(px(20.0))
                        .flex()
                        .flex_col()
                        .gap(px(12.0))
                        .rounded(px(radius::CARD))
                        .bg(t.surface)
                        .border_1()
                        .border_color(t.border)
                        .shadow_lg()
                        .child(div().text_size(ty::TITLE).child("Keplorayx"))
                        .child(div().text_size(ty::BODY).text_color(t.text_muted).child(
                            "Use this computer only, or log in with Woozlit so a phone and other devices can connect.",
                        ))
                        .child(div().flex().gap(px(8.0)).justify_end().child(stay).child(login)),
                )
                .into_any_element(),
        )
    }

    /// The sidebar hover card: the full name, harness, folder, branch, and
    /// worktree. It appears only after the pointer has rested on that row.
    fn render_session_hover(&self, t: &Theme, _cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.hover_ready?;
        if self.hover_task != Some(id) {
            return None;
        }
        let task = self.tasks.iter().find(|task| task.id == id)?;
        let project = self.project(task.project_id);
        let folder = project.map(|p| p.path.display().to_string()).unwrap_or_default();
        let branch = task.branch.clone().unwrap_or_else(|| "No branch".into());
        let worktree = task.worktree.as_ref().map(|p| p.display().to_string());
        let pr = self.pr_links.get(&id).cloned();
        Some(
            deferred(
                div()
                    .absolute()
                    .left(px(self.sidebar_w + 8.0))
                    .top(px(Theme::TITLEBAR_HEIGHT + 48.0))
                    .w(px(284.0))
                    .p(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .rounded(px(radius::CARD))
                    .bg(t.surface)
                    .border_1()
                    .border_color(t.border)
                    .shadow_md()
                    .text_size(ty::SMALL)
                    .child(div().text_size(ty::BODY).child(task.title.clone()))
                    .child(div().text_color(t.text_muted).child(task.harness.label()))
                    .child(div().text_color(t.text_faint).child(folder))
                    .child(div().text_color(t.text_faint).child(branch))
                    .when_some(worktree, |d, w| d.child(div().text_color(t.text_faint).child(w)))
                    .when_some(pr, |d, url| d.child(div().text_color(t.accent).child(url))),
            )
            .into_any_element(),
        )
    }

    fn render_info_card(&self, task: &Task, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let project = self.project(task.project_id).cloned();
        let pr = self.panel.read(cx).pr(cx).filter(|_| self.pr_links.contains_key(&task.id));
        let path = task.worktree.clone().or_else(|| project.as_ref().map(|p| p.path.clone()));
        let (usage, context, mode) = self
            .threads
            .get(&task.id)
            .map(|v| {
                let chat = v.read(cx).chat.read(cx);
                let mode = chat.composer.read(cx).mode;
                (chat.usage(), chat.context(), mode)
            })
            .unwrap_or_default();
        let (files, adds, dels) = self.panel.read(cx).changes.read(cx).summary();
        let can_fork = Engine::can_fork(task);
        let path_s = path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        let id = task.id;
        let context_line = match context {
            (Some(u), Some(w)) if w > 0 => {
                format!("{:.0}% context · {} / {}", u as f32 / w as f32 * 100.0, ui::compact(u), ui::compact(w))
            }
            (Some(u), _) => format!("{} used", ui::compact(u)),
            _ => "Context not reported yet".into(),
        };
        let mode_name = match mode {
            dove_core::proto::PermissionMode::Default => "Ask every time",
            dove_core::proto::PermissionMode::AcceptEdits => "Auto-edit",
            dove_core::proto::PermissionMode::Plan => "Plan",
            dove_core::proto::PermissionMode::Bypass => "Bypass",
        };
        let model = task
            .model
            .clone()
            .map(|m| format!("{} · {m}", task.harness.label()))
            .unwrap_or_else(|| task.harness.label().to_string());
        // Facts and actions share the menu grammar (30 px rows, an 8 px
        // inset, a 15 px glyph and a 10 px gap), so every icon and every
        // label sits on one column down the card.
        let fact = |glyph: &'static str, text: String| {
            div()
                .min_h(px(30.0))
                .px(px(8.0))
                .py(px(5.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .rounded(px(kit::popover::MENU_ITEM_RADIUS))
                .text_size(ty::BODY)
                .text_color(t.text)
                .child(icon(glyph).size(px(15.0)).flex_none().text_color(t.text_muted))
                .child(div().flex_1().min_w_0().truncate().child(text))
        };
        let context_pct = match context {
            (Some(u), Some(w)) if w > 0 => Some((u as f32 / w as f32).clamp(0.0, 1.0)),
            _ => None,
        };
        let meter = |p: f32| {
            div().flex_none().w(px(36.0)).h(px(4.0)).rounded_full().bg(t.wash(0.12)).child(
                div().h_full().w(relative(p)).rounded_full().bg(if p >= 0.95 {
                    t.danger
                } else if p >= 0.8 {
                    t.warning
                } else {
                    t.accent
                }),
            )
        };

        let mut env = vec![
            fact(icons::LAPTOP, format!("Local · {}", ui::host_name())).into_any_element(),
            fact(
                icons::FOLDER,
                match (&project, path.as_ref()) {
                    (Some(p), Some(dir)) => format!("{} · {}", p.name, dir.display()),
                    (Some(p), None) => p.name.clone(),
                    _ => "No folder".into(),
                },
            )
            .into_any_element(),
        ];
        if let Some(b) = task.branch.clone() {
            env.push(fact(icons::GIT_BRANCH, b).into_any_element());
        }
        let mut ctx_rows = vec![
            fact(icons::CLOCK_CIRCLE, context_line).children(context_pct.map(meter)).into_any_element(),
            fact(ui::harness_icon(task.harness), model).into_any_element(),
            fact(icons::KEY_MINIMALISTIC, format!("Permissions · {mode_name}")).into_any_element(),
        ];
        if usage.total_tokens() > 0 {
            ctx_rows.push(fact(icons::TAG, ui::usage_line(&usage)).into_any_element());
        }
        if let Some(cost) = usage.cost_usd {
            ctx_rows.push(fact(icons::TAG, format!("${cost:.2} this thread")).into_any_element());
        }
        let mut change_rows = Vec::new();
        if files > 0 {
            change_rows.push(
                div()
                    .id("info-open-changes")
                    .min_h(px(30.0))
                    .px(px(8.0))
                    .py(px(5.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .rounded(px(kit::popover::MENU_ITEM_RADIUS))
                    .cursor_pointer()
                    .text_size(ty::BODY)
                    .text_color(t.text)
                    .hover(|s| s.bg(kit::popover::row_active_bg(t)))
                    .tooltip(ui::tip("Show the changes"))
                    .on_click(cx.listener(|this, _, window, cx| this.show_panel(Tab::Changes, window, cx)))
                    .child(icon(icons::FILE_CODE).size(px(15.0)).flex_none().text_color(t.text_muted))
                    .child(div().flex_1().min_w_0().truncate().child(format!("{files} files changed")))
                    .child(div().flex_none().text_size(ty::META).text_color(t.success).child(format!("+{adds}")))
                    .child(div().flex_none().text_size(ty::META).text_color(t.danger).child(format!("−{dels}")))
                    .into_any_element(),
            );
        }
        if let Some((target, detail)) = pr {
            let (status, color) = match &detail {
                Some(d) => crate::pr_view::status_style(d, t),
                None => ("", t.text_muted),
            };
            let checks = detail.as_ref().map(|d| d.checks_summary()).unwrap_or("none");
            let title = detail.as_ref().map(|d| d.title.clone()).unwrap_or_default();
            change_rows.push(
                div()
                    .id("info-open-pr")
                    .min_h(px(30.0))
                    .px(px(8.0))
                    .py(px(5.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .rounded(px(kit::popover::MENU_ITEM_RADIUS))
                    .cursor_pointer()
                    .text_size(ty::BODY)
                    .text_color(t.text)
                    .hover(|s| s.bg(kit::popover::row_active_bg(t)))
                    .tooltip(ui::tip("Show the pull request"))
                    .on_click(cx.listener(|this, _, window, cx| this.show_panel(Tab::PullRequest, window, cx)))
                    .child(icon(icons::PULL_REQUEST).size(px(15.0)).flex_none().text_color(color))
                    .child(div().flex_1().min_w_0().truncate().child(if title.is_empty() {
                        format!("PR #{}", target.number)
                    } else {
                        format!("#{} · {title}", target.number)
                    }))
                    .when(!status.is_empty(), |d| {
                        d.child(div().flex_none().text_size(ty::META).text_color(color).child(status))
                    })
                    .when(checks != "none", |d| {
                        d.child(ui::dot(
                            match checks {
                                "passing" => t.success,
                                "failing" => t.danger,
                                _ => t.warning,
                            },
                            6.0,
                        ))
                    })
                    .into_any_element(),
            );
        }
        let actions = vec![
            ui::menu_row("info-view", icons::TERMINAL, "Switch Chat / Terminal (Ctrl+`)", false, t)
                .on_click(cx.listener(|this, _, window, cx| this.toggle_view_mode(window, cx)))
                .into_any_element(),
            ui::menu_row("info-side", icons::PLUS, "New side chat", false, t)
                .on_click(cx.listener(|this, _, window, cx| this.side_chat(window, cx)))
                .into_any_element(),
            if can_fork {
                ui::menu_row("info-fork", icons::FORK, "Fork thread", false, t)
                    .on_click(cx.listener(move |this, _, window, cx| this.fork_task(id, window, cx)))
                    .into_any_element()
            } else {
                ui::menu_row("info-fork", icons::FORK, "Fork thread", false, t)
                    .opacity(0.45)
                    .tooltip(ui::tip("Forking needs a Claude Code session that has started"))
                    .into_any_element()
            },
            {
                let p = path.clone();
                ui::menu_row("info-open", icons::FOLDER_WITH_FILES, "Open folder", false, t)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        if let Some(p) = &p {
                            cx.open_with_system(p);
                        }
                    }))
                    .into_any_element()
            },
            {
                let p = path;
                ui::menu_row("info-term", icons::TERMINAL, "Open in system terminal", false, t)
                    .on_click(cx.listener(move |_, _, _, _cx| {
                        if let Some(p) = &p {
                            crate::terminal::open_system_terminal(p);
                        }
                    }))
                    .into_any_element()
            },
            ui::menu_row("info-copy-path", icons::COPY, "Copy path", false, t)
                .on_click(cx.listener(move |this, _, _, cx| this.copy(path_s.clone(), cx)))
                .into_any_element(),
        ];
        // Zeron's lanes and ports: what runs alongside the thread, and the
        // servers its terminals started. Each one opens what it names.
        let (subagents, side_chats) = self.panel.read(cx).lanes();
        let mut lanes: Vec<AnyElement> = Vec::new();
        for (i, (call, title, running)) in subagents.into_iter().enumerate() {
            lanes.push(
                ui::menu_row(SharedString::from(format!("info-sub-{i}")), icons::FORK, title, false, t)
                    .when(running, |d| d.child(ui::dot(t.busy, 6.0)))
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(crate::actions::OpenSubagent { call: call.clone() }), cx);
                    })
                    .into_any_element(),
            );
        }
        for (chat, title, running) in side_chats {
            lanes.push(
                ui::menu_row(SharedString::from(format!("info-chat-{chat}")), icons::CHAT_ROUND_LINE, title, false, t)
                    .when(running, |d| d.child(ui::dot(t.busy, 6.0)))
                    .on_click(cx.listener(move |this, _, window, cx| this.navigate(Route::Thread(chat), window, cx)))
                    .into_any_element(),
            );
        }
        let ports: Vec<AnyElement> = self
            .panel
            .read(cx)
            .detected_urls(cx)
            .into_iter()
            .take(6)
            .enumerate()
            .map(|(i, url)| {
                let label = url.trim_start_matches("http://").trim_start_matches("https://").to_string();
                ui::menu_row(SharedString::from(format!("info-port-{i}")), icons::GLOBE, label, false, t)
                    .tooltip(ui::tip("Open in the browser panel"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.show_panel(Tab::Browser, window, cx);
                        let url = url.clone();
                        this.panel.update(cx, |p, cx| p.open(Tab::Browser, Some(url), window, cx));
                    }))
                    .into_any_element()
            })
            .collect();

        // Zeron's session card: one frosted card, plain rows, a hairline
        // between what it is and what you can do with it.
        let section = |rows: Vec<AnyElement>| div().flex().flex_col().gap(px(2.0)).children(rows);
        let body = div()
            .id("thread-info")
            .w(px(INFO_WIDTH))
            .max_h(px(620.0))
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .child(section(env))
            .child(ui::separator(t))
            .child(section(ctx_rows))
            .when(!change_rows.is_empty(), |d| d.child(ui::separator(t)).child(section(change_rows)))
            .when(!lanes.is_empty(), |d| d.child(ui::separator(t)).child(section(lanes)))
            .when(!ports.is_empty(), |d| d.child(ui::separator(t)).child(section(ports)))
            .child(ui::separator(t))
            .child(section(actions));
        ui::frosted(kit::popover::CARD_RADIUS, 16.0, kit::popover::card(t).child(body)).into_any_element()
    }

    fn render_menu_overlay(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use crate::shell_menu::{Item, act};
        let (menu, exit) = match self.menu.clone() {
            Some(menu) => (menu, None),
            None => self.menus.context_exit().map(|(m, p)| (m, Some(p)))?,
        };
        let task = self.task(menu.task).cloned()?;
        let t = Theme::of(cx).clone();
        let id = task.id;
        let archived = task.status == TaskStatus::Archived;
        let pinned = task.pinned;
        let project_path = self.project(task.project_id).map(|p| p.path.display().to_string()).unwrap_or_default();
        let mut items = vec![
            Item::row(
                "cm-rename",
                Some(icons::PEN),
                "Rename",
                act(move |this, window, cx| this.begin_rename(id, window, cx)),
            )
            .trailing(kit::kbd("F2", &t)),
            Item::row(
                "cm-pin",
                Some(icons::PIN),
                if pinned { "Unpin" } else { "Pin" },
                act(move |this, _, cx| this.set_pinned(id, !pinned, cx)),
            ),
            Item::row(
                "cm-archive",
                Some(if archived { icons::ARCHIVE_UP_MINIMALISTIC } else { icons::ARCHIVE_MINIMALISTIC }),
                if archived { "Unarchive" } else { "Archive" },
                act(move |this, window, cx| this.set_archived(id, !archived, window, cx)),
            ),
            Item::row(
                "cm-copy",
                Some(icons::COPY),
                "Copy",
                act(|this, _, _| {
                    if let Some(m) = &mut this.menu {
                        m.copy_open = !m.copy_open;
                    }
                }),
            )
            .keep_open()
            .trailing(
                icon(if menu.copy_open { icons::ALT_ARROW_DOWN } else { icons::ALT_ARROW_RIGHT })
                    .size(px(14.0))
                    .text_color(t.text_faint),
            ),
        ];
        if menu.copy_open {
            let copies: Vec<(&'static str, &'static str, String)> = vec![
                ("cc-title", "Title", task.title.clone()),
                ("cc-branch", "Branch name", task.branch.clone().unwrap_or_default()),
                ("cc-path", "Path", task.worktree.as_ref().map(|p| p.display().to_string()).unwrap_or(project_path)),
                ("cc-session", "Session ID", task.native_session_id.clone().unwrap_or_default()),
            ];
            for (cid, label, value) in copies {
                if value.is_empty() {
                    continue;
                }
                items.push(Item::row(cid, None, label, act(move |this, _, cx| this.copy(value.clone(), cx))).indent());
            }
        }
        items.push(Item::Separator);
        // Signed in, a session can live in two places, so delete says which.
        let signed_in = crate::remote::Remote::global(cx).is_some_and(|r| r.read(cx).status.signed_in);
        let scopes: &[(DeleteScope, &'static str, &'static str, &'static str, &'static str)] = if signed_in {
            &[
                (
                    DeleteScope::Local,
                    "cm-delete",
                    "Delete from this computer…",
                    "cm-delete-confirm",
                    "Click to delete from this computer",
                ),
                (
                    DeleteScope::Cloud,
                    "cm-delete-cloud",
                    "Delete from cloud…",
                    "cm-delete-cloud-confirm",
                    "Click to delete from your account",
                ),
                (
                    DeleteScope::Everywhere,
                    "cm-delete-all",
                    "Delete everywhere…",
                    "cm-delete-all-confirm",
                    "Click to delete everywhere",
                ),
            ]
        } else {
            &[(DeleteScope::Local, "cm-delete", "Delete…", "cm-delete-confirm", "Click to delete for good")]
        };
        for &(scope, row, label, sure_row, sure) in scopes {
            items.push(if menu.confirm_delete == Some(scope) {
                Item::row(
                    sure_row,
                    Some(icons::TRASH_BIN_MINIMALISTIC),
                    sure,
                    act(move |this, window, cx| this.delete_session(id, scope, window, cx)),
                )
                .danger()
            } else {
                Item::row(
                    row,
                    Some(icons::TRASH_BIN_MINIMALISTIC),
                    label,
                    act(move |this, _, _| {
                        if let Some(m) = &mut this.menu {
                            m.confirm_delete = Some(scope);
                        }
                    }),
                )
                .danger()
                .keep_open()
            });
        }
        let live = exit.is_none();
        let body = self.menu_list(items, 240.0, live, &t, cx);
        let card = self.menu_surface(&t, body, exit);
        Some(
            deferred(
                anchored().position(menu.at).snap_to_window_with_margin(px(8.0)).child(
                    div()
                        .when(live, |d| {
                            d.on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.close_menus();
                                cx.notify();
                            }))
                        })
                        .child(card),
                ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    /// Split the focused terminal, opening Flock first if elsewhere.
    fn split_terminal(&mut self, axis: crate::flock::pane_tree::Axis, window: &mut Window, cx: &mut Context<Self>) {
        if self.route != Route::Flock {
            self.navigate(Route::Flock, window, cx);
            return;
        }
        self.workspace.update(cx, |w, cx| w.split(axis, window, cx));
    }

    /// Run a Kanban draft as the agent's own TUI in a new Flock tab, typing
    /// its prompt once the agent is ready.
    pub fn dispatch_to_flock(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.task(id).cloned() else { return };
        let Some(cwd) = self.project(task.project_id).map(|p| p.path.clone()) else { return };
        let prompt = task.prompt.clone().unwrap_or_else(|| task.title.clone());
        self.navigate(Route::Flock, window, cx);
        let kind = task.harness;
        self.workspace.update(cx, |w, cx| w.dispatch(kind, cwd, prompt, window, cx));
    }

    /// Routes whose picture runs under the whole window as frosted glass.
    fn full_art_route(&self) -> bool {
        (matches!(self.route, Route::Thread(_) | Route::Flock) || self.call_view_shown)
            && self.engine.store.setting("wallpaper_chat").as_deref() != Some("false")
    }

    /// The side panel docks on the left of the chat instead of the right.
    pub fn panel_left(&self) -> bool {
        self.engine.store.setting("panel_side").as_deref() == Some("left")
    }

    fn has_wallpaper(&self) -> bool {
        self.wallpaper_path().is_some()
    }

    /// The wallpaper's dominant colour, worked out once per picture off the
    /// UI thread (the shell repaints when it lands).
    fn wallpaper_hue(&mut self, cx: &mut Context<Self>) -> Option<gpui::Hsla> {
        let path = self.wallpaper_path()?;
        if let Some((seen, hue)) = &self.wallpaper_hue
            && *seen == path
        {
            return *hue;
        }
        self.wallpaper_hue = Some((path.clone(), None));
        let work = cx.background_executor().spawn({
            let path = path.clone();
            async move { crate::wallpaper_media::dominant_color(std::path::Path::new(&path)) }
        });
        cx.spawn(async move |this, cx| {
            let hue = work.await;
            tracing::debug!(?hue, "wallpaper hue for {path}");
            let _ = this.update(cx, |this, cx| {
                if this.wallpaper_hue.as_ref().is_some_and(|(p, _)| *p == path) {
                    this.wallpaper_hue = Some((path, hue));
                    cx.notify();
                }
            });
        })
        .detach();
        None
    }

    /// The wallpaper image, when it is set and on disk. The disk check is
    /// cached for a second: a render asks several times.
    fn wallpaper_path(&self) -> Option<String> {
        thread_local! {
            static SEEN: std::cell::RefCell<Option<(String, bool, Instant)>> = const { std::cell::RefCell::new(None) };
        }
        let path = self.engine.store.setting("wallpaper_path").filter(|p| !p.is_empty())?;
        SEEN.with(|seen| {
            let mut seen = seen.borrow_mut();
            let fresh = seen.as_ref().is_some_and(|(p, _, at)| *p == path && at.elapsed() < Duration::from_secs(1));
            if !fresh {
                let exists = std::path::Path::new(&path).is_file();
                *seen = Some((path.clone(), exists, Instant::now()));
            }
            seen.as_ref().filter(|(_, exists, _)| *exists).map(|(p, ..)| p.clone())
        })
    }

    /// Artwork for the chat column only, from the top of the window. The
    /// sidebar and the files panel stay on the shell's own fill.
    fn wallpaper_layer(
        &mut self,
        left: f32,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let home = matches!(self.route, Route::Home);
        let thread = self.full_art_route();
        if !home && !thread || width < 8.0 {
            return None;
        }
        let path = self.wallpaper_path()?;
        let t = Theme::of(cx).clone();
        let viewport_h = f32::from(window.viewport_size().height);
        let kind = crate::wallpaper_media::kind_cached(std::path::Path::new(&path));
        // On Home a GIF or video loops as attached across the column: no
        // blur, fade, cut-out or effect, only a light dim (`MOVING_DIM`).
        // Behind chats and terminals every background is frosted under the
        // contrast scrim, moving ones still playing, so text stays readable.
        let moving = kind != crate::wallpaper_media::Kind::Still;
        let height = if thread || moving { viewport_h } else { crate::wallpaper::hero_height(viewport_h) };
        let scale = window.scale_factor();
        let playback = self.engine.store.setting("wallpaper_playback").unwrap_or_else(|| "always".into());
        let composer = if home { self.composer_bounds.clone() } else { std::rc::Rc::new(std::cell::Cell::new(None)) };
        let target = (width * scale, height * scale);
        let art = if moving && !thread {
            let props = crate::wallpaper_media::BackdropProps {
                path: path.into(),
                variant: crate::wallpaper_media::Variant::Hero,
                target,
                play: if thread { playback == "always" } else { playback != "off" },
                fade: false,
                composer: None,
                opacity: 1.0,
            };
            let view = window.use_keyed_state("shell-motion", cx, crate::wallpaper_media::Backdrop::new);
            let veil = gpui::black().opacity(MOVING_DIM);
            view.update(cx, |b, cx| b.show(props, cx)).then(|| (view.into_any_element(), Some(veil)))
        } else if thread {
            // Behind a chat: baked frost under a contrast-solved scrim.
            let blur = self.chat_blur().round() as u32;
            let variant = crate::wallpaper_media::Variant::Frost(blur);
            let props = crate::wallpaper_media::BackdropProps {
                path: path.clone().into(),
                variant,
                target,
                // Moving wallpaper plays on Home only. A chat keeps the still
                // frame so a video does not decode while the window sits idle.
                play: false,
                fade: false,
                composer: None,
                opacity: 1.0,
            };
            let view = window.use_keyed_state("shell-frost", cx, crate::wallpaper_media::Backdrop::new);
            let ready = view.update(cx, |b, cx| b.show(props, cx));
            let bright = crate::wallpaper_media::clip(std::path::Path::new(&path), variant, target, true, cx)
                .map_or(1.0, |c| c.bright);
            ready.then(|| (view.into_any_element(), Some(t.bg.opacity(self.chat_scrim(bright, &t)))))
        } else {
            let effect =
                crate::wallpaper::Effect::from_id(&self.engine.store.setting("wallpaper_effect").unwrap_or_default());
            let animate = self.engine.store.setting("wallpaper_animate").as_deref() == Some("true");
            let props = crate::wallpaper::HeroProps {
                path: path.into(),
                effect,
                light: !t.appearance.is_dark(),
                target,
                animate,
                frosted: t.is_frost(),
                blur: 0,
                fade: home,
                composer,
            };
            let view = window.use_keyed_state("shell-hero", cx, crate::wallpaper::Hero::new);
            view.update(cx, |hero, cx| hero.show(props, cx)).then(|| (view.into_any_element(), None))
        };
        // One column, from the title strip down. Home fades into the canvas.
        // A thread lays the baked frost and one scrim over the whole window,
        // titlebar included, so the bar is the page. While the art decodes
        // (or if it cannot) the column is the plain page: the chat skips its
        // own fill under a wallpaper, so without this the window glass would
        // show the desktop through it.
        Some(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(left))
                .w(px(width))
                .overflow_hidden()
                .bg(t.bg)
                .when_some(art, |d, (view, scrim)| {
                    d.child(div().absolute().top_0().left_0().right_0().h(px(height)).child(view))
                        .when_some(scrim, |d, veil| d.child(div().absolute().inset_0().bg(veil)))
                })
                .into_any_element(),
        )
    }

    /// The chat backdrop's blur, in screen pixels (Settings › Background).
    fn chat_blur(&self) -> f32 {
        use crate::wallpaper_media::FrostLook;
        self.engine
            .store
            .setting("wallpaper_chat_blur")
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(FrostLook::DEFAULT_BLUR)
            .clamp(FrostLook::MIN_BLUR, FrostLook::MAX_BLUR)
    }

    /// Scrim coverage over the chat frost: the user's dim as the floor,
    /// raised until body text holds 7:1 over the art's brightest region
    /// (its darkest, on a light theme).
    fn chat_scrim(&self, bright: f32, t: &Theme) -> f32 {
        let dark = t.appearance.is_dark();
        let floor = self
            .engine
            .store
            .setting("wallpaper_chat_dim")
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(if dark { CHAT_DIM_DARK } else { CHAT_DIM_LIGHT })
            .clamp(CHAT_DIM_MIN, CHAT_DIM_MAX);
        let backdrop = if dark { bright } else { 0.0 };
        crate::wallpaper_media::scrim_alpha(
            backdrop,
            crate::theme::luminance(crate::theme::flatten(t.bg, t.bg)),
            crate::theme::luminance(crate::theme::flatten(t.text, t.bg)),
            floor,
        )
    }

    fn render_center(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // A call shows in the chat's place, over the same frosted picture.
        if let Some(call) = self.call_view.clone().filter(|_| self.call_view_shown) {
            return div().size_full().child(call).into_any_element();
        }
        match self.route {
            Route::Home => self.render_home(window, cx),
            Route::Thread(id) => match self.threads.get(&id) {
                // A chat is a light frame around its cached transcript and
                // the composer (which GPUI could not reuse under a cached
                // view that re-renders), so it is drawn directly; a
                // terminal is cached whole.
                // The thread, and any threads beside it (`split.rs`).
                Some(v) => {
                    let v = v.clone();
                    self.render_split(id, v, window, cx)
                }
                None => div().into_any_element(),
            },
            Route::Threads => self.render_home(window, cx),
            Route::Artifacts => self.render_artifacts_page(window, cx),
            Route::Kanban => self.render_kanban(window, cx),
            Route::Automations => self.render_automations(window, cx),
            Route::PullRequests => self.prs.clone().into_any_element(),
            Route::Usage => self.render_usage_page(cx),
            Route::Races => self.render_races_page(cx),
            Route::Archived => self.render_archived_page(cx),
            Route::Settings | Route::Agents => match self.settings.update(cx, |s, cx| s.render_content(window, cx)) {
                Some(content) => content,
                // Settings › Team is the Team page, with Settings' options.
                None if self.settings.read(cx).section == crate::settings::Section::Team => {
                    let options = self.settings.update(cx, |s, cx| s.team_options(cx));
                    self.render_team_page(Some(options), cx)
                }
                // Settings › Archived is the Archived page itself.
                None => self.render_archived_page(cx),
            },
            Route::Help => self.render_help_page(cx),
            Route::Team => self.render_team_page(None, cx),
            Route::Flock => {
                AnyView::from(self.workspace.clone()).cached(StyleRefinement::default().size_full()).into_any_element()
            }
        }
    }
}

#[derive(Clone)]
struct TabDrag {
    id: TaskId,
    title: SharedString,
}

impl Render for TabDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("TabDrag");
        let t = Theme::of(cx);
        div()
            .px(px(10.0))
            .py(px(5.0))
            .rounded(px(radius::RADIUS))
            .bg(t.surface_raised)
            .border_1()
            .border_color(t.border)
            .text_size(ty::SMALL)
            .child(self.title.clone())
    }
}

fn uuid_task(s: &str) -> Option<TaskId> {
    TaskId::parse(s)
}

/// `DOVE_FRAME_STATS=1` logs how often the shell re-renders, to catch
/// accidental continuous repaint (idle should be zero renders per second).
fn frame_stats() {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicU64, Ordering};
    static ENABLED: OnceLock<bool> = OnceLock::new();
    static COUNT: AtomicU64 = AtomicU64::new(0);
    static WINDOW_START: AtomicU64 = AtomicU64::new(0);
    if !*ENABLED.get_or_init(|| std::env::var_os("DOVE_FRAME_STATS").is_some()) {
        return;
    }
    let now = dove_core::store::now_ms() as u64;
    let start = WINDOW_START.load(Ordering::Relaxed);
    let n = COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    if now.saturating_sub(start) >= 1000 {
        tracing::info!(target: "dove::frames", "shell renders in the last second: {n}");
        COUNT.store(0, Ordering::Relaxed);
        WINDOW_START.store(now, Ordering::Relaxed);
    }
}

/// Version probes start agent processes; launch waits this long first.
const VERSION_PROBE_DELAY: Duration = Duration::from_secs(10);

/// `size:modified` of a binary: changes whenever the agent is updated.
fn binary_stamp(program: &std::path::Path) -> Option<String> {
    let meta = std::fs::metadata(program).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(format!("{}:{modified}", meta.len()))
}

/// The version probed last time, if the binary has not changed since.
fn cached_version(store: &dove_core::store::Store, kind: HarnessKind, program: &std::path::Path) -> Option<String> {
    let saved = store.setting(&format!("agent_version.{}", kind.id()))?;
    let (stamp, version) = saved.rsplit_once('|')?;
    (Some(stamp) == binary_stamp(program).as_deref() && !version.is_empty()).then(|| version.to_string())
}

fn remember_version(store: &dove_core::store::Store, kind: HarnessKind, program: &std::path::Path, version: &str) {
    if let Some(stamp) = binary_stamp(program) {
        let _ = store.set_setting(&format!("agent_version.{}", kind.id()), &format!("{stamp}|{version}"));
    }
}

pub fn version_of(program: &std::path::Path) -> Option<String> {
    let mut cmd = std::process::Command::new(program);
    let ext = program.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if cfg!(windows) && (ext == "cmd" || ext == "bat") {
        cmd = std::process::Command::new("cmd.exe");
        cmd.arg("/d").arg("/c").arg(program);
    }
    cmd.arg("--version").stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    // "codex-cli 0.146.0", "2.1.270 (Claude Code)", "v1.18.32" → the version token.
    first
        .split_whitespace()
        .find(|w| w.trim_start_matches('v').chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|w| w.trim_start_matches('v').trim_end_matches(',').to_string())
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("Shell");
        crate::motion::set_chrome_view(cx.entity_id());
        self.tick_menus(window, cx);
        frame_stats();
        let t = Theme::of(cx).clone();
        let in_thread = matches!(self.route, Route::Thread(_));
        // Pages that carry the side panel.
        let docked = in_thread || self.route == Route::Flock;
        // No open threads: the main page is the wallpaper and the composer.
        // The tab strip only exists once a thread is open.
        // The new-thread page draws only its floating window controls (the
        // frosted island over the picture), unless thread tabs are on and
        // some are open.
        let bare = matches!(self.route, Route::Home)
            && (self.tabs.is_empty() || !self.engine.store.flag("topbar_tabs", false));
        let wants_expand = self.panel.read(cx).expanded && self.panel_open;
        self.update_fit(f32::from(window.viewport_size().width), docked, wants_expand);
        let panel_open = docked && self.panel_shown();
        let expanded = panel_open && wants_expand;
        let sidebar_shown = self.sidebar_shown();
        // Web views are native windows: the call view hides them.
        let panel_live = panel_open && self.panel_open && !self.call_view_shown;
        let body_on = self.panel_open;
        let explorer_w = if panel_open { self.explorer_width(window) } else { 0.0 };
        self.panel.update(cx, |p, cx| {
            p.set_columns(body_on, explorer_w, cx);
            p.sync(panel_live, self.resize.is_some(), cx)
        });
        let topbar = (!bare).then(|| self.render_topbar(window, cx));
        // With no title bar, the sidebar starts below the floating window
        // controls instead of running under them.
        let in_settings = self.route == Route::Settings;
        let view = self.sidebar_view(sidebar_shown && !in_settings, cx);
        let sidebar = if in_settings {
            // Settings swaps the thread list for its sections (with Back);
            // the thread list returns when Settings closes.
            let width = self.sidebar_w;
            let nav = self.settings.update(cx, |s, cx| s.render_nav(width, window, cx));
            Some(div().h_full().flex_none().child(nav).into_any_element())
        } else {
            sidebar_shown.then(|| {
                let sidebar = match view {
                    Some(v) => self.embed_sidebar(v, cx),
                    None => self.render_sidebar(cx),
                };
                if bare {
                    div().h_full().flex().flex_col().pt(px(Theme::TITLEBAR_HEIGHT)).child(sidebar).into_any_element()
                } else {
                    sidebar
                }
            })
        };
        let center = (!expanded).then(|| self.render_center(window, cx));
        let panel_w = self.panel_width(window);
        let now = Instant::now();
        self.slides.sidebar.step((!in_settings).then_some(sidebar_shown), self.sidebar_w, now);
        self.slides.panel.step(docked.then_some(panel_open && !expanded), panel_w, now);
        // A drag that reaches its limit nudges the column toward the drag
        // and lets it settle back.
        let bounce =
            self.resize_bounce.and_then(|(e, limit, at)| crate::motion::resize_bounce(limit, at).map(|o| (e, o)));
        if bounce.is_some() {
            window.request_animation_frame();
        } else {
            self.resize_bounce = None;
        }
        let sidebar_nudge = bounce.filter(|(e, _)| *e == Edge::Sidebar).map(|(_, o)| o);
        let panel_nudge = bounce.filter(|(e, _)| *e == Edge::Panel).map(|(_, o)| -o);
        let sidebar = sidebar.map(|sidebar| match sidebar_nudge {
            Some(o) => div().h_full().flex_none().relative().left(px(o)).child(sidebar).into_any_element(),
            None => sidebar,
        });
        let sidebar = sidebar.map(|sidebar| match self.slides.sidebar.entrance("sidebar-in") {
            Some(id) if !in_settings => {
                crate::motion::slide_in(id, self.sidebar_w, false, div().h_full().flex_none().child(sidebar))
                    .into_any_element()
            }
            // Entering Settings crossfades its sections into the slot.
            _ if in_settings => crate::motion::fade_quick("settings-nav-in", div().h_full().flex_none().child(sidebar))
                .into_any_element(),
            _ => sidebar,
        });
        // Closing ghosts: the sidebar's own rows and a plain dock surface.
        let sidebar_ghost = self.slides.sidebar.exit(now).map(|(p, w)| {
            let rows = self.render_sidebar(cx);
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(-24.0 * p))
                .w(px(w))
                .opacity(1.0 - p)
                .bg(t.glass())
                .when(bare, |d| d.pt(px(Theme::TITLEBAR_HEIGHT)))
                .child(rows)
        });
        let thread_art_now = self.full_art_route() && self.has_wallpaper();
        let panel_ghost = self.slides.panel.exit(now).map(|(p, w)| {
            div().absolute().top_0().bottom_0().right(px(-24.0 * p)).w(px(w)).opacity(1.0 - p).bg(if thread_art_now {
                t.bg.opacity(SIDE_ART_TINT)
            } else {
                t.glass()
            })
        });
        if sidebar_ghost.is_some() || panel_ghost.is_some() {
            window.request_animation_frame();
        }

        // The dock is the window's full height. Its fill is the same as the
        // chat, and it continues up through the titlebar strip (`dock_layer`).
        let panel_entrance = (!expanded).then(|| self.slides.panel.entrance("panel-in")).flatten();
        let dock_left = self.panel_left();
        let panel = panel_open.then(|| {
            div()
                .relative()
                .h_full()
                .when(!expanded, |d| d.w(px(panel_w)).flex_none())
                .when(expanded, |d| d.flex_1().min_w_0())
                .when(!expanded && body_on, |d| {
                    d.child(
                        div()
                            .absolute()
                            .when(!dock_left, |d| d.left(px(-3.0)))
                            .when(dock_left, |d| d.right(px(-3.0)))
                            .top_0()
                            .bottom_0()
                            .w(px(6.0))
                            .cursor_col_resize()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, e: &gpui::MouseDownEvent, window, cx| {
                                    this.resize = Some((Edge::Panel, e.position.x, this.panel_body_width(window)));
                                    cx.stop_propagation();
                                }),
                            ),
                    )
                })
                .child(AnyView::from(self.panel.clone()).cached(StyleRefinement::default().size_full()))
        });
        let panel = panel.map(|panel| match (panel_entrance, panel_nudge) {
            (Some(id), _) => crate::motion::slide_in(id, panel_w, !dock_left, panel).into_any_element(),
            (None, Some(o)) => panel.left(px(if dock_left { -o } else { o })).into_any_element(),
            (None, None) => panel.into_any_element(),
        });

        let home_art = matches!(self.route, Route::Home) && self.has_wallpaper();
        let thread_art = self.full_art_route() && self.has_wallpaper();
        // Beside the chat when it clears the reading column; over it when not.
        let chat_w = f32::from(window.viewport_size().width)
            - if sidebar_shown { self.sidebar_w } else { 0.0 }
            - if panel_open { panel_w } else { 0.0 };
        let info_gutter =
            if self.info_open && in_thread && !expanded { info_reserve(chat_w) } else { None }.unwrap_or(0.0);
        let info_floats = self.info_open && in_thread && !expanded && info_reserve(chat_w).is_none();
        if let Some(thread) = self.active_thread().cloned() {
            thread.update(cx, |view, cx| {
                view.chat.update(cx, |chat, _| chat.info_reserve = info_gutter);
            });
        }
        let card = if bare {
            Some(
                div().flex_1().min_w_0().h_full().overflow_hidden().child(ui::rounded_clip(
                    0.0,
                    div()
                        .size_full()
                        .flex()
                        .when_some(center, |d, center| d.child(div().flex_1().min_w_0().h_full().child(center))),
                )),
            )
        } else {
            center.map(|center| {
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .when(!home_art && !thread_art && !t.is_frost(), |d| d.bg(t.bg))
                    .overflow_hidden()
                    .child(div().size_full().flex().child(div().flex_1().min_w_0().h_full().child(center)))
            })
        };
        // The sidebar takes the wallpaper's colour when one is set.
        let hue = self.wallpaper_hue(cx);
        // Zeron's dock: its flush panel fill over the window glass. Over a
        // thread's frost it shares the chat's frost with nothing on top.
        let dock_fill = (!thread_art).then(|| t.panel_bg());
        let chrome = move |d: gpui::Div| d.when_some(dock_fill, |d, fill| d.bg(fill));
        let dock_layer = panel_open.then(|| {
            let inner = if sidebar_shown { self.sidebar_w } else { 0.0 };
            chrome(div())
                .absolute()
                .top_0()
                .bottom_0()
                .when(!dock_left || expanded, |d| d.right_0())
                .when(dock_left && !expanded, |d| d.left(px(inner)))
                .when(!expanded, |d| d.w(px(panel_w)))
                .when(expanded, |d| d.left(px(inner)))
                .when(!expanded, |d| if dock_left { d.border_r_1() } else { d.border_l_1() }.border_color(t.border))
                // The explorer's hairline runs up through the title bar too.
                .when(explorer_w > 0.0 && body_on, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .h(px(Theme::TITLEBAR_HEIGHT))
                            .w(px(explorer_w))
                            .border_l_1()
                            .border_color(t.border),
                    )
                })
        });
        // The left sidebar is Zeron's: the window glass with a 5% wash and a
        // hairline. Only a thread's full-window frost needs its own tint, in
        // the picture's hue, to read as a pane of glass over the art.
        let side_shown = sidebar_shown || in_settings;
        let side_fill = match hue {
            _ if !thread_art => t.wash(0.05),
            Some(h) => t.wallpaper_tinted(t.sidebar_glass(), h).opacity(SIDE_GLASS_OVER_ART),
            None => t.sidebar_glass().opacity(SIDE_GLASS_OVER_ART),
        };
        let side_layer = side_shown.then(|| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(px(self.sidebar_w))
                .bg(side_fill)
                .border_r_1()
                .border_color(t.border)
        });
        // The shell's glass: the whole window, sidebar included.
        let plane = div()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .left(px(if side_shown && thread_art { self.sidebar_w } else { 0.0 }))
            .bg(t.glass());

        let menu = self.render_menu_overlay(cx);
        // The palette draws its own scrim and card (kit modal layer).
        let palette = self.palette.clone().map(|p| div().absolute().inset_0().child(p));

        div()
            .id("shell")
            .track_focus(&self.focus)
            .key_context("Shell")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .text_color(t.text)
            .font({
                let mut font = gpui::font(t.font_sans.clone());
                font.fallbacks = Some(gpui_base::theme_tokens::sans_font_fallbacks());
                font
            })
            .text_size(ty::MD)
            .on_action(cx.listener(|this, _: &actions::NewSession, window, cx| this.new_session(window, cx)))
            .on_action(cx.listener(|this, _: &actions::AddProject, _, cx| this.add_project_dialog(cx)))
            .on_action(cx.listener(|this, _: &actions::ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &actions::ToggleRightPanel, _, cx| this.toggle_panel(cx)))
            .on_action(cx.listener(|this, _: &actions::SplitPaneRight, window, cx| {
                this.pane_command(PaneCommand::Split(Axis::Row), window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::SplitPaneDown, window, cx| {
                this.pane_command(PaneCommand::Split(Axis::Column), window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &actions::ClosePane, window, cx| {
                    this.pane_command(PaneCommand::Close, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &actions::MaximizePane, window, cx| {
                this.pane_command(PaneCommand::Maximize, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::FocusPaneLeft, window, cx| {
                this.pane_command(PaneCommand::Focus(Dir::Left), window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::FocusPaneRight, window, cx| {
                this.pane_command(PaneCommand::Focus(Dir::Right), window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::FocusPaneUp, window, cx| {
                this.pane_command(PaneCommand::Focus(Dir::Up), window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::FocusPaneDown, window, cx| {
                this.pane_command(PaneCommand::Focus(Dir::Down), window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::GoBack, window, cx| this.go_back(window, cx)))
            .on_action(cx.listener(|this, _: &actions::GoForward, window, cx| this.go_forward(window, cx)))
            .on_action(
                cx.listener(|this, _: &actions::OpenSettings, window, cx| this.navigate(Route::Settings, window, cx)),
            )
            .on_action(cx.listener(|this, _: &actions::ToggleTerminal, window, cx| this.toggle_view_mode(window, cx)))
            .on_action(cx.listener(|this, _: &actions::ToggleVoiceCall, window, cx| this.toggle_voice_call(window, cx)))
            .on_action(cx.listener(|this, _: &actions::ShowCallView, window, cx| this.show_call_view(true, window, cx)))
            .on_action(cx.listener(|this, _: &actions::CommandPalette, window, cx| this.open_palette(window, cx)))
            .on_action(
                cx.listener(|this, _: &actions::OpenThreads, window, cx| this.navigate(Route::Threads, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &actions::OpenKanban, window, cx| this.navigate(Route::Kanban, window, cx)),
            )
            .on_action(cx.listener(|this, _: &actions::OpenFlock, window, cx| this.navigate(Route::Flock, window, cx)))
            .on_action(cx.listener(|this, _: &actions::FlockSplitRight, window, cx| {
                this.split_terminal(crate::flock::pane_tree::Axis::Row, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::FlockSplitDown, window, cx| {
                this.split_terminal(crate::flock::pane_tree::Axis::Column, window, cx)
            }))
            .on_action(cx.listener(|this, _: &actions::FlockClosePane, window, cx| {
                if this.route == Route::Flock {
                    this.workspace.update(cx, |w, cx| w.close_pane(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &actions::NextTab, window, cx| this.cycle_tab(1, window, cx)))
            .on_action(cx.listener(|this, _: &actions::PrevTab, window, cx| this.cycle_tab(-1, window, cx)))
            .on_action(cx.listener(|this, a: &actions::GoToTab, window, cx| {
                if let Some(id) = this.tabs.get(a.index).copied() {
                    this.navigate(Route::Thread(id), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &actions::CloseTab, window, cx| {
                if let Route::Thread(id) = this.route {
                    this.close_tab(id, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &actions::RenameThread, window, cx| {
                if let Route::Thread(id) = this.route {
                    this.begin_rename(id, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &actions::FocusComposer, window, cx| match this.active_thread() {
                Some(v) => {
                    let composer = v.read(cx).chat.read(cx).composer.clone();
                    crate::composer::focus(&composer, window, cx);
                }
                None => crate::composer::focus(&this.composer, window, cx),
            }))
            .on_action(cx.listener(|this, a: &actions::OpenPath, window, cx| {
                this.show_panel(Tab::Files, window, cx);
                let (path, line) = (a.path.clone(), a.line);
                this.panel.update(cx, |p, cx| p.open_path(&path, line, window, cx));
            }))
            .on_action(cx.listener(|this, a: &actions::OpenPullRequest, window, cx| {
                this.open_pr_url(&a.url, window, cx);
            }))
            .on_action(cx.listener(|this, a: &actions::OpenSubagent, window, cx| {
                let Some(v) = this.active_thread() else { return };
                let source = v.read(cx).chat.clone();
                this.panel_open = true;
                let _ = this.engine.store.set_setting("panel_open", "true");
                let call = a.call.clone();
                this.panel.update(cx, |p, cx| p.open_subagent(&source, call, window, cx));
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                // Hold Space to talk while a call is muted, only when nothing
                // that takes text has focus (the shell itself does).
                if e.keystroke.key == "space"
                    && !e.is_held
                    && this.focus.is_focused(window)
                    && let Some(call) = this.voice_call.clone()
                    && call.read(cx).muted
                {
                    call.update(cx, |v, _| v.push_to_talk(true));
                    cx.stop_propagation();
                    return;
                }
                if e.keystroke.key == "escape" {
                    let menu = this.close_menus();
                    let open = menu || this.info_open || this.palette.is_some();
                    if open {
                        this.info_open = false;
                        this.palette = None;
                        cx.stop_propagation();
                        cx.notify();
                    } else if this.route == Route::Settings && !this.settings.read(cx).is_recording() {
                        this.close_settings(window, cx);
                        cx.stop_propagation();
                    }
                }
            }))
            .on_key_up(cx.listener(|this, e: &gpui::KeyUpEvent, _, cx| {
                if e.keystroke.key == "space"
                    && let Some(call) = this.voice_call.clone()
                {
                    call.update(cx, |v, _| v.push_to_talk(false));
                }
            }))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
                if let Some((edge, origin, width)) = this.resize {
                    let delta = f32::from(e.position.x - origin);
                    match edge {
                        Edge::Sidebar => {
                            let (w, pressing) = crate::motion::clamp_resize(width + delta, 208.0, 400.0);
                            this.sidebar_w = w;
                            this.note_resize_edge(edge, pressing);
                        }
                        // Stops where the chat column would be squeezed, so the
                        // saved width is the one you see.
                        Edge::Panel => {
                            let max = this.panel_room(window).max(PANEL_FLOOR);
                            let grow = if this.panel_left() { delta } else { -delta };
                            let (w, pressing) = crate::motion::clamp_resize(width + grow, PANEL_FLOOR, max);
                            this.panel_w = w;
                            this.note_resize_edge(edge, pressing);
                        }
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if let Some((edge, _, _)) = this.resize.take() {
                        this.resize_pressing = None;
                        let (key, w) = match edge {
                            Edge::Sidebar => ("sidebar_width", this.sidebar_w),
                            Edge::Panel => ("panel_width", this.panel_w),
                        };
                        let _ = this.engine.store.set_setting(key, &format!("{w:.0}"));
                        cx.notify();
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Navigate(gpui::NavigationDirection::Back),
                cx.listener(|this, _, window, cx| this.go_back(window, cx)),
            )
            .on_mouse_down(
                MouseButton::Navigate(gpui::NavigationDirection::Forward),
                cx.listener(|this, _, window, cx| this.go_forward(window, cx)),
            )
            .child(plane)
            .children({
                // A thread's picture runs under the whole window as one
                // frost (the sidebar's tinted glass sits on it); home keeps
                // it to the chat column, where it fades into the canvas, and
                // the sidebar gets its own strip of frost.
                let full = thread_art;
                let docked = panel_open && !expanded && !full;
                let art_left = if (sidebar_shown || in_settings) && !full { self.sidebar_w } else { 0.0 }
                    + if docked && dock_left { panel_w } else { 0.0 };
                let art_right = if docked && !dock_left { panel_w } else { 0.0 };
                let art_width = (f32::from(window.viewport_size().width) - art_left - art_right).max(1.0);
                self.wallpaper_layer(art_left, art_width, window, cx)
            })
            .children(side_layer)
            .children(dock_layer)
            .children(panel_ghost)
            .children(topbar)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .children(sidebar)
                    .map(
                        |d| if dock_left { d.children(panel).children(card) } else { d.children(card).children(panel) },
                    )
                    .children(sidebar_ghost),
            )
            .when(bare, |d| d.child(self.render_bare_chrome(window, cx)))
            .children(menu)
            .children(palette)
            // A page's confirmation (Threads, Archived, Kanban, Automations,
            // Team): drawn here so it stays modal whichever route is showing.
            .children(self.render_first_run(&t, cx))
            .children(self.render_sync_offer(&t, cx))
            .children(self.render_session_hover(&t, cx))
            .children(self.render_page_dialog(window, cx))
            .children(self.render_race_sheet(window, cx))
            // An agent's sign-in or update terminal, over any page.
            .children(self.render_agent_sheet(&t, window, cx))
            .children(self.render_voice_bar(cx))
            .children((self.info_open && in_thread && !expanded).then(|| self.active_task().cloned()).flatten().map(
                |task| {
                    let right = if panel_open && !dock_left { panel_w + INFO_INSET } else { INFO_INSET };
                    div()
                        .absolute()
                        .top(px(Theme::TITLEBAR_HEIGHT + 8.0))
                        .right(px(right))
                        // Floating over the chat, a click anywhere else closes it.
                        .when(info_floats, |d| {
                            d.on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.info_open = false;
                                cx.notify();
                            }))
                        })
                        .child(crate::motion::fade_quick(
                            "thread-card-in",
                            div().child(self.render_info_card(&task, &t, cx)),
                        ))
                },
            ))
            // Caption buttons sit on the window itself, above the titlebar's
            // padding, in the same even slots Zeron uses.
            .children(ui::windows_captions(window, &t))
            .children(ui::linux_captions(&t))
            .child(gpui::canvas(
                |_, _, _| {},
                |_, _, window, _| {
                    // Hover and state fades advance from wall time; keep frames
                    // coming only while one of them is still moving.
                    if crate::motion::fades_active() {
                        window.request_animation_frame();
                    }
                },
            ))
    }
}

impl Shell {
    /// The dock's laid-out width, its columns together. The titlebar strip
    /// and the dock both use it, so they stay one column.
    pub(crate) fn panel_width(&self, window: &Window) -> f32 {
        let explorer = self.explorer_width(window);
        let body = if self.panel_open || explorer <= 0.0 { self.panel_body_width(window) } else { 0.0 };
        body + explorer
    }

    /// The surfaces' width: the saved width, narrowed so the chat column
    /// keeps `CENTER_MIN`.
    fn panel_body_width(&self, window: &Window) -> f32 {
        self.panel_w.min(self.panel_room(window)).max(PANEL_FLOOR)
    }

    /// The explorer column, while it is open and fits beside the chat. Short
    /// of room the tree narrows before the chat or the surfaces do.
    fn explorer_width(&self, window: &Window) -> f32 {
        use crate::side_panel::{EXPLORER_MIN, EXPLORER_W};
        let room = self.room_beside_chat(window) - if self.panel_open { PANEL_FLOOR } else { 0.0 };
        if self.explorer_open && matches!(self.route, Route::Thread(_)) && room >= EXPLORER_MIN {
            room.min(EXPLORER_W)
        } else {
            0.0
        }
    }

    /// The widest the surfaces can be while the chat column keeps
    /// `CENTER_MIN` and the explorer its width.
    fn panel_room(&self, window: &Window) -> f32 {
        self.room_beside_chat(window) - self.explorer_width(window)
    }

    fn room_beside_chat(&self, window: &Window) -> f32 {
        let total = f32::from(window.viewport_size().width);
        let left = if self.sidebar_shown() || self.route == Route::Settings { self.sidebar_w } else { 0.0 };
        total - left - CENTER_MIN
    }

    /// The sidebar's view while it is on screen. Off screen, its fades are
    /// dropped (the shell's own would be pruned the same frame).
    fn sidebar_view(&mut self, shown: bool, cx: &mut Context<Self>) -> Option<Entity<crate::sidebar::Sidebar>> {
        if !shown {
            if let Some(v) = &self.sidebar_view {
                crate::motion::forget_owner(v.entity_id());
            }
            return None;
        }
        let shell = cx.entity();
        Some(self.sidebar_view.get_or_insert_with(|| cx.new(|cx| crate::sidebar::Sidebar::new(&shell, cx))).clone())
    }

    /// The sidebar, reusing its last frame unless it needs drawing. A shell
    /// change re-renders it (it observes the shell); a working thread's step
    /// line is checked here, since that comes from the thread's own view.
    fn embed_sidebar(&self, view: Entity<crate::sidebar::Sidebar>, cx: &mut Context<Self>) -> AnyElement {
        let steps = self.sidebar_steps(cx);
        if view.read(cx).steps != steps {
            return AnyView::from(view).into_any_element();
        }
        AnyView::from(view)
            .cached(StyleRefinement::default().w(px(self.sidebar_w)).h_full().flex_none())
            .into_any_element()
    }

    /// Start a sidebar resize drag (from the sidebar's edge).
    pub fn begin_sidebar_resize(&mut self, x: Pixels) {
        self.resize = Some((Edge::Sidebar, x, self.sidebar_w));
    }
}

/// Sidebar and dock transitions. Layout never animates: the chat column
/// takes its final width at once. An opening surface slides 24 px in and
/// fades up (a fresh generation id replays the entrance); a closing one
/// leaves a ghost that slides out and fades over the content.
#[derive(Default)]
struct Slides {
    sidebar: Slide,
    panel: Slide,
}

#[derive(Default)]
struct Slide {
    /// Last frame's state; `None` while the surface is not in play (the
    /// route has no dock, or Settings owns the sidebar slot), so coming back
    /// to it snaps instead of sliding.
    shown: Option<bool>,
    generation: u32,
    /// The exit in flight: when it started and the surface's width.
    out: Option<(Instant, f32)>,
}

impl Slide {
    fn step(&mut self, shown: Option<bool>, width: f32, now: Instant) {
        if let (Some(was), Some(is)) = (self.shown, shown)
            && was != is
            && !crate::motion::reduced_now()
        {
            if is {
                self.generation += 1;
            } else {
                self.out = Some((now, width));
            }
        }
        if shown != Some(false) {
            self.out = None;
        }
        self.shown = shown;
    }

    /// The entrance's element id, once the surface has been opened in this
    /// session (the first paint never slides).
    fn entrance(&self, name: &'static str) -> Option<gpui::ElementId> {
        (self.generation > 0).then(|| gpui::ElementId::NamedInteger(name.into(), self.generation as u64))
    }

    /// Exit progress (0 → 1) and width while the ghost is out.
    fn exit(&mut self, now: Instant) -> Option<(f32, f32)> {
        let (at, width) = self.out?;
        let t = crate::motion::panel_spec(width).since(at, now);
        if t >= 1.0 {
            self.out = None;
            return None;
        }
        Some((t, width))
    }
}

trait UpdateInAnyWindow {
    fn update_in_any_window(
        &self,
        cx: &mut gpui::AsyncApp,
        f: impl FnOnce(&mut Shell, &mut Window, &mut Context<Shell>),
    ) -> anyhow::Result<()>;
}

impl UpdateInAnyWindow for gpui::WeakEntity<Shell> {
    /// Run `f` with the shell's window (background tasks have none).
    fn update_in_any_window(
        &self,
        cx: &mut gpui::AsyncApp,
        f: impl FnOnce(&mut Shell, &mut Window, &mut Context<Shell>),
    ) -> anyhow::Result<()> {
        let window = cx.update(|cx| cx.windows().first().copied()).ok_or_else(|| anyhow::anyhow!("no window"))?;
        let this = self.clone();
        window.update(cx, |_, window, cx| {
            let _ = this.update(cx, |s, cx| f(s, window, cx));
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_fits_a_wide_window() {
        assert_eq!(panel_fit(1600.0, 264.0, (true, 1), (true, 2), false), (true, true));
    }

    #[test]
    fn least_recent_column_hides_first() {
        // 264 + 360 + 300 = 924 does not fit 900: the older column goes.
        assert_eq!(panel_fit(900.0, 264.0, (true, 1), (true, 2), false), (false, true));
        assert_eq!(panel_fit(900.0, 264.0, (true, 3), (true, 2), false), (true, false));
    }

    #[test]
    fn expanded_dock_outranks_the_sidebar() {
        assert_eq!(panel_fit(500.0, 264.0, (true, 5), (true, 1), true), (false, true));
    }

    #[test]
    fn closed_columns_stay_closed() {
        assert_eq!(panel_fit(2000.0, 264.0, (false, 0), (false, 0), false), (false, false));
        assert_eq!(panel_fit(200.0, 264.0, (true, 0), (true, 0), false), (false, false));
    }

    #[test]
    fn sync_offer_follows_sign_in_not_first_run() {
        assert!(!sync_offer_due(false, true, false, false));
        assert!(!sync_offer_due(true, false, false, false));
        assert!(sync_offer_due(true, true, false, false));
        assert!(!sync_offer_due(true, true, true, false));
        assert!(!sync_offer_due(true, true, false, true));
    }

    #[test]
    fn work_phrase_matches_the_offer_copy() {
        assert_eq!(local_work_phrase(0), None);
        assert_eq!(local_work_phrase(1).as_deref(), Some("the 1 session"));
        assert_eq!(local_work_phrase(3).as_deref(), Some("the 3 sessions"));
    }
}

/// One block holding a single quiet line, as on Zeron's sign-in pages: where
/// sync stands, what the sign-in is waiting on.
fn sync_status_block(t: &Theme, leading: AnyElement, text: String, strong: bool) -> AnyElement {
    div()
        .mt(px(28.0))
        .rounded(px(10.0))
        .bg(t.wash(0.04))
        .px(px(16.0))
        .py(px(10.0))
        .min_h(px(44.0))
        .flex()
        .items_center()
        .gap(px(10.0))
        .child(div().flex_none().flex().items_center().child(leading))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(12.5))
                .line_height(px(17.0))
                .text_color(if strong { t.text } else { t.text_muted })
                .child(text),
        )
        .into_any_element()
}

/// Zeron's primary page button: a near-white plate with a near-black label.
fn sync_primary(t: &Theme, id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px(px(14.0))
        .py(px(6.0))
        .rounded(px(8.0))
        .bg(t.text)
        .text_size(px(13.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(t.on_solid)
        .cursor_pointer()
        .hover(|s| s.opacity(0.9))
        .role(gpui::Role::Button)
        .aria_label(label)
        .child(label)
}

/// Zeron's ghost page button: muted text, a wash on hover.
fn sync_ghost(t: &Theme, id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px(px(12.0))
        .py(px(6.0))
        .rounded(px(8.0))
        .text_size(px(13.0))
        .text_color(crate::motion::hover_blend(id, t.text_muted, t.text))
        .bg(crate::motion::hover_blend(id, crate::theme::wash(0.0), crate::theme::ink(0.06)))
        .on_hover(crate::motion::hover_listener(id))
        .cursor_pointer()
        .role(gpui::Role::Button)
        .aria_label(label)
        .child(label)
}
