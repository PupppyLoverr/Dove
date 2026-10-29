//! The notch: Dove's Dynamic Island (docs/NOTCH-V2.md). It runs in the
//! island process (`dove --island`) and reaches agents over the line to the
//! engine (`crate::island::ipc`).
//!
//! - **Hidden**: no window at all after 10 s (and the process trims its
//!   memory). Rest the pointer at the top middle, or press Ctrl+Alt+Space,
//!   and a dot drops from the edge and springs into the island.
//! - **Slim** (compact): a 36 px capsule with the primary live activity
//!   (agents, music, the timer, a pinned note) and up to two bubbles for the
//!   rest. Scroll cycles the primary; middle-click plays or pauses; right-
//!   click hides it for an hour or opens settings.
//! - **Alert** (banner): approvals with a preview of the change, finishes,
//!   failures, the timer. Never takes focus.
//! - **Expanded**: five tabs (Home, Agents, Music, Tray, Notifications) and
//!   three pages reached from them (Notes, Timer, Dev servers), with a
//!   command bar that reaches everything (docs/NOTCH-V2.md §11).
//!
//! Nothing polls. The loop sleeps until a platform event (pointer, clipboard,
//! foreground, hotkey, display), a message from the engine, media, audio, or
//! its own next deadline (a hover delay, a banner's end, the timer).
//!
//! QA hooks: `DOVE_NOTCH_STATE=slim|expanded|alert`,
//! `DOVE_NOTCH_PAGE=home|agents|music|tray|notes|timer|alerts|dev|new|call|plan|view:<lens>`,
//! `DOVE_NOTCH_DEMO=1`, `DOVE_NOTCH_CAPTURABLE=1` (screenshots can see it),
//! `DOVE_NOTCH_CAPTURED_GLASS=1` (skip composition glass), `DOVE_NOTCH_TIMING=1`
//! (log transition timings), `DOVE_NOTCH_CYCLE=1` (open and close every 1.6 s).

pub mod bridge;
mod data;
pub mod glass;
mod media;
pub mod platform;
mod spring;
mod tokens;
mod views;
mod win;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dove_core::proto::{HarnessKind, PermissionDecision, ProjectId, TaskId, TaskStatus};
use dove_core::store::Store;
use gpui::{
    App, AppContext as _, Context, Entity, ExternalPaths, Render, ScrollHandle, Subscription, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, point, px, size,
};
use gpui_base::input::{InputEvent, InputState, TextareaState};

use crate::island::ipc::{self, ToEngine, ToIsland};
use crate::island::view::{CallInfo, NotchView, ShowWhat};
pub use bridge::{NotchHost, Snapshot};
use data::{Clip, Note, Timer, Undo};
use platform::{Display, Shape};
use spring::Springs;
use tokens as tk;
pub use views::v3::{Asking, Lens, PlanEdit};

/// The pointer must rest this long before the island opens, so passing
/// through the top edge (to a browser tab, say) doesn't open it.
const HOVER_DELAY: Duration = Duration::from_millis(90);
/// Moving up toward the edge at least this fast (device pixels per ms) opens
/// it at once: the pointer is clearly headed there.
const INTENT_SPEED: f32 = 1.2;
/// The window is made (hidden) when the pointer comes this close to the hot
/// zone (logical pixels), so opening never waits for it.
const WARM_BAND: f32 = 48.0;
/// With a button held (dragging a file up to the tray) it waits longer, so
/// dragging a window to the top for Snap Layouts doesn't open it.
const DRAG_DELAY: Duration = Duration::from_millis(380);
/// The pointer may leave the island this long before it closes.
const LEAVE_DELAY: Duration = Duration::from_millis(450);
/// How long a finished-task or timer banner stays.
const FLASH_FOR: Duration = Duration::from_secs(5);
/// The slim island's "Done" moment.
const MOMENT_FOR: Duration = Duration::from_millis(2400);
/// Undo stays offered this long.
const UNDO_FOR: Duration = Duration::from_secs(5);
/// A denial waits this long (with Undo) before it reaches the agent.
const DENY_GRACE: Duration = Duration::from_millis(3000);
/// After this long hidden the window closes and memory is trimmed.
const CLOSE_AFTER: Duration = Duration::from_secs(60);
/// With no window to keep warm, memory is trimmed this long after start.
const TRIM_AFTER: Duration = Duration::from_secs(10);
/// Stop waits this long (with Undo) before it reaches the agent.
const STOP_GRACE: Duration = Duration::from_secs(3);
/// A note saves itself this long after the last keystroke.
const NOTE_SAVE_AFTER: Duration = Duration::from_millis(600);
/// Suggestions (a copied error, dropped files, a stopped server, checks)
/// stay this long unless handled.
const SUGGEST_FOR: Duration = Duration::from_secs(9);
/// How often listening ports are checked while dev servers are around.
const PORTS_EVERY: Duration = Duration::from_secs(10);
/// Ports are watched this long after one was last seen.
const PORTS_WATCH: Duration = Duration::from_secs(600);
/// Hot zone at the top edge, logical pixels.
const HOT_WIDTH: f32 = 220.0;
const HOT_HEIGHT: f32 = 3.0;
/// Slack around the island before the pointer counts as gone.
const LEAVE_MARGIN: f32 = 14.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Hidden,
    Slim,
    Alert,
    Expanded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    Home,
    Agents,
    Music,
    Tray,
    Notes,
    Timer,
    Alerts,
    Dev,
}

impl Tab {
    pub const ALL: [Tab; 8] =
        [Tab::Home, Tab::Agents, Tab::Music, Tab::Tray, Tab::Notes, Tab::Timer, Tab::Alerts, Tab::Dev];
    /// The tab bar; the rest are pages reached from Home, the slim island and
    /// the command bar.
    pub const BAR: [Tab; 5] = [Tab::Home, Tab::Agents, Tab::Music, Tab::Tray, Tab::Alerts];

    pub fn in_bar(self) -> bool {
        Tab::BAR.contains(&self)
    }

    pub fn id(self) -> &'static str {
        match self {
            Tab::Home => "home",
            Tab::Agents => "agents",
            Tab::Music => "music",
            Tab::Tray => "tray",
            Tab::Notes => "notes",
            Tab::Timer => "timer",
            Tab::Alerts => "alerts",
            Tab::Dev => "dev",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tab::Home => "Home",
            Tab::Agents => "Agents",
            Tab::Music => "Music",
            Tab::Tray => "Tray",
            Tab::Notes => "Notes",
            Tab::Timer => "Timer",
            Tab::Alerts => "Notifications",
            Tab::Dev => "Dev servers",
        }
    }

    fn from_id(id: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|t| t.id() == id || (id == "clipboard" && *t == Tab::Tray))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    Tab(Tab),
    Agent(TaskId),
    NewAgent,
    /// Talking to K22 (V3).
    Call,
    /// An agent's plan to review.
    Plan(TaskId),
    /// A view an agent composed, by id.
    View(u64),
    /// A built-in lens.
    Lens(Lens),
}

impl Page {
    /// The wide pages (V3): the island grows to `tk::WIDE_W`.
    pub fn wide(&self) -> bool {
        matches!(self, Page::Call | Page::Plan(_) | Page::View(_) | Page::Lens(_))
    }

    /// The header's title for a page without a tab.
    pub fn title(&self) -> Option<&'static str> {
        match self {
            Page::Call => Some("K22"),
            Page::Plan(_) => Some("Plan"),
            Page::View(_) => Some("View"),
            Page::Lens(l) => Some(l.title()),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

/// One reading of the world, in device pixels.
#[derive(Clone, Copy, Debug)]
pub struct Input {
    pub display: Display,
    pub cursor: Option<(i32, i32)>,
    pub button_down: bool,
    pub fullscreen: bool,
    /// The island has the keyboard (the user is typing in it).
    pub engaged: bool,
    /// Where the island rests when nobody uses it: Hidden or Slim.
    pub rest: State,
    /// A banner wants to be seen.
    pub alert: bool,
    /// The island's bounds on screen (left, top, right, bottom).
    pub island: (i32, i32, i32, i32),
}

/// The state machine, kept apart from the window so it can be tested.
#[derive(Debug)]
pub struct Machine {
    pub state: State,
    pub pinned: bool,
    /// The island opened under a held button: show the tray to drop on.
    pub drag_reveal: bool,
    hot_since: Option<(Instant, bool)>,
    away_since: Option<Instant>,
    /// The last pointer reading, for its speed toward the edge.
    last_pointer: Option<((i32, i32), Instant)>,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine {
    pub fn new() -> Self {
        Self {
            state: State::Hidden,
            pinned: false,
            drag_reveal: false,
            hot_since: None,
            away_since: None,
            last_pointer: None,
        }
    }

    fn in_hot_zone(input: &Input) -> bool {
        let Some((x, y)) = input.cursor else { return false };
        let d = input.display;
        let half = (HOT_WIDTH / 2.0 * d.scale) as i32;
        let height = ((HOT_HEIGHT * d.scale) as i32).max(1);
        (x - d.center_x()).abs() <= half && y >= d.top && y < d.top + height
    }

    fn over_island(input: &Input) -> bool {
        let Some((x, y)) = input.cursor else { return false };
        let m = (LEAVE_MARGIN * input.display.scale) as i32;
        let (l, t, r, b) = input.island;
        // The strip above the island counts too, so moving up to the edge
        // doesn't close it.
        x >= l - m && x <= r + m && y >= t.min(input.display.top) && y <= b + m
    }

    /// When the machine next needs a look even if nothing happens.
    pub fn deadline(&self) -> Option<Instant> {
        let hot = self.hot_since.map(|(t, held)| t + if held { DRAG_DELAY } else { HOVER_DELAY });
        let away = self.away_since.map(|t| t + LEAVE_DELAY);
        hot.into_iter().chain(away).min()
    }

    /// Opens the island (a click, the hotkey, an alert).
    pub fn open(&mut self) -> Option<State> {
        (self.state != State::Expanded).then(|| self.go(State::Expanded))
    }

    /// Closes it to where it rests.
    pub fn close(&mut self, rest: State, alert: bool) -> Option<State> {
        self.pinned = false;
        let to = if alert { State::Alert } else { rest };
        (self.state != to).then(|| self.go(to))
    }

    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
        self.away_since = None;
    }

    /// Advances on a new reading. Returns the new state if it changed.
    pub fn tick(&mut self, input: &Input, now: Instant) -> Option<State> {
        // Moving up fast into the hot zone is intent enough to skip the wait.
        let fast = match (self.last_pointer, input.cursor) {
            (Some(((_, py), at)), Some((_, y))) => {
                let ms = now.saturating_duration_since(at).as_secs_f32() * 1000.0;
                ms > 0.0 && ms < 120.0 && (py - y) as f32 / ms >= INTENT_SPEED
            }
            _ => false,
        };
        self.last_pointer = input.cursor.map(|c| (c, now));
        if input.fullscreen {
            self.hot_since = None;
            return (self.state != State::Hidden).then(|| self.go(State::Hidden));
        }
        let resting = if input.alert { State::Alert } else { input.rest };
        match self.state {
            State::Hidden | State::Slim | State::Alert => {
                // A banner is interactive: it opens on a click, not a hover.
                let trigger = match self.state {
                    State::Hidden => Self::in_hot_zone(input),
                    State::Slim => Self::over_island(input) || Self::in_hot_zone(input),
                    _ => false,
                };
                if trigger && fast && !input.button_down && self.state == State::Hidden {
                    self.drag_reveal = false;
                    return Some(self.go(State::Expanded));
                }
                if trigger {
                    let (since, held) = *self.hot_since.get_or_insert((now, input.button_down));
                    let wait = if held || input.button_down { DRAG_DELAY } else { HOVER_DELAY };
                    if now.duration_since(since) >= wait {
                        self.drag_reveal = input.button_down;
                        return Some(self.go(State::Expanded));
                    }
                } else {
                    self.hot_since = None;
                }
                (self.state != resting).then(|| self.go(resting))
            }
            State::Expanded => {
                if self.pinned || input.engaged || Self::over_island(input) {
                    self.away_since = None;
                    return None;
                }
                let since = *self.away_since.get_or_insert(now);
                (now.duration_since(since) >= LEAVE_DELAY).then(|| self.go(resting))
            }
        }
    }

    fn go(&mut self, state: State) -> State {
        if state != State::Expanded {
            self.drag_reveal = false;
        }
        self.state = state;
        self.hot_since = None;
        self.away_since = None;
        state
    }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// The island's shape in the canvas, logical pixels: its top, size, corner
/// radius, the horizontal lean toward the pointer, and its bubbles (0 = none).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geo {
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub r: f32,
    pub dx: f32,
    pub b1: f32,
    pub b2: f32,
}

impl Geo {
    /// Tucked above the screen's top edge.
    pub const HIDDEN: Geo = Geo { y: -18.0, w: 12.0, h: 12.0, r: 6.0, dx: 0.0, b1: 0.0, b2: 0.0 };
    /// The dot that drops out of the edge before the island opens.
    pub const DOT: Geo = Geo { y: tk::TOP_GAP + 2.0, w: 14.0, h: 14.0, r: 7.0, dx: 0.0, b1: 0.0, b2: 0.0 };

    fn to_array(self) -> [f32; 7] {
        [self.y, self.w, self.h, self.r, self.dx, self.b1, self.b2]
    }

    fn from_array(a: [f32; 7]) -> Geo {
        Geo {
            y: a[0],
            w: a[1].max(1.0),
            h: a[2].max(1.0),
            r: a[3].max(0.0),
            dx: a[4],
            b1: a[5].max(0.0),
            b2: a[6].max(0.0),
        }
    }

    pub fn x(&self) -> f32 {
        (tk::CANVAS_W - self.w) / 2.0 + self.dx
    }

    /// The island, then its bubbles (each born from the island's edge).
    pub fn shapes(&self) -> Vec<Shape> {
        let mut v = vec![Shape { x: self.x(), y: self.y, w: self.w, h: self.h, r: self.r.min(self.h / 2.0) }];
        let mut x = self.x() + self.w + tk::BUBBLE_GAP;
        for b in [self.b1, self.b2] {
            if b > 1.0 {
                v.push(Shape { x, y: self.y + (self.h - b) / 2.0, w: b, h: b, r: b / 2.0 });
                x += b + tk::BUBBLE_GAP;
            }
        }
        v
    }

    /// The whole group's width (island plus bubbles), for hit testing.
    pub fn span(&self) -> f32 {
        let mut w = self.w;
        for b in [self.b1, self.b2] {
            if b > 1.0 {
                w += tk::BUBBLE_GAP + b;
            }
        }
        w
    }
}

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayPick {
    Primary,
    Pointer,
}

/// Preferences, re-read every few seconds (Dove's Settings writes them).
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub enabled: bool,
    pub slim_always: bool,
    pub live: bool,
    pub clipboard: bool,
    pub notifications: bool,
    /// Real-time glass (composition), else captured glass.
    pub glass: bool,
    /// Proactive nudges (an agent waiting on you for a while). Off by default.
    pub nudges: bool,
    pub display: DisplayPick,
    pub focus: bool,
    pub hidden_tabs: HashSet<Tab>,
    /// Which activities may show in the slim island.
    pub slim_agents: bool,
    pub slim_music: bool,
    pub slim_timer: bool,
    pub slim_note: bool,
}

impl Prefs {
    fn read(store: &Store) -> Self {
        let forced = std::env::var("DOVE_NOTCH").ok().as_deref() == Some("1");
        let hidden_tabs = store
            .setting("notch_hidden_tabs")
            .unwrap_or_default()
            .split(',')
            .filter_map(|t| Tab::from_id(t.trim()))
            .filter(|t| *t != Tab::Home)
            .collect();
        Prefs {
            enabled: forced || store.flag("notch", true),
            slim_always: store.flag("notch_slim", false)
                || std::env::var("DOVE_NOTCH_STATE").ok().as_deref() == Some("slim"),
            live: store.flag("notch_live", true),
            clipboard: store.flag("notch_clipboard", true),
            notifications: store.flag("notch_notifications", true),
            glass: store.flag("notch_glass", false),
            nudges: store.flag("notch_nudges", false),
            display: if store.setting("notch_display").as_deref() == Some("pointer") {
                DisplayPick::Pointer
            } else {
                DisplayPick::Primary
            },
            focus: store.flag("notch_focus", false),
            hidden_tabs,
            slim_agents: store.flag("notch_slim_agents", true),
            slim_music: store.flag("notch_slim_music", true),
            slim_timer: store.flag("notch_slim_timer", true),
            slim_note: store.flag("notch_slim_note", true),
        }
    }

    /// The tab bar, without the tabs Settings hides.
    pub fn tabs(&self) -> Vec<Tab> {
        Tab::BAR.into_iter().filter(|t| !self.hidden_tabs.contains(t)).collect()
    }
}

// ---------------------------------------------------------------------------
// Things the island shows
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlashKind {
    Done,
    Failed,
    Timer,
}

#[derive(Clone, Debug)]
pub struct Flash {
    pub kind: FlashKind,
    pub text: String,
    pub agent: Option<HarnessKind>,
    pub task: Option<TaskId>,
    pub at: Instant,
    pub until: Instant,
}

/// One line in Notifications.
#[derive(Clone, Debug)]
pub struct Notice {
    pub at_ms: i64,
    pub kind: FlashKind,
    pub text: String,
    pub agent: Option<HarnessKind>,
    pub task: Option<TaskId>,
}

/// The new-agent form's choices.
#[derive(Clone, Debug, Default)]
pub struct NewAgent {
    pub kind: Option<HarnessKind>,
    pub project: Option<ProjectId>,
    pub worktree: bool,
    pub error: Option<String>,
}

/// A denial waiting out its grace period (Undo cancels it).
#[derive(Clone, Debug)]
pub struct PendingDeny {
    pub task: TaskId,
    pub request_id: String,
    pub at: Instant,
}

/// The tray's filter chips.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrayFilter {
    All,
    Files,
    Text,
    Images,
    Links,
    Pinned,
}

/// What a command-bar result does.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Agent(TaskId),
    Note(i64),
    Clip(usize),
    Tab(Tab),
    NewAgent(String),
    /// Ask the default agent and show the answer here.
    Ask(String),
    Timer(u64),
    NewNote(String),
    Capture,
    Focus,
    Pin,
    Stop(TaskId),
    OpenInDove(TaskId),
    ClearClips,
    Shuffle,
    LinkTimer(TaskId),
    RunAutomation(String),
    Merge(TaskId, u64),
    /// Talk to K22.
    Call,
    Lens(Lens),
    /// A view from history.
    View(u64),
    Plan(TaskId),
}

/// What a menu item does.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuAction {
    Pin,
    Settings,
    HideHour,
    AgentMessage(TaskId),
    AgentStop(TaskId),
    AgentOpen(TaskId),
    AllowAlways(TaskId, String),
    AllowAll(TaskId, Vec<String>),
    Filter(TrayFilter),
    Capture,
    ClearClips,
    TileSend(usize),
    TileFix(usize),
    ClipPin(usize),
    ShelfCopy(usize),
    ShelfRemove(usize),
    ShelfStart(usize),
    NotePin(i64),
    NoteDelete(i64),
    NoteSend(i64),
    LinkTimer(Option<TaskId>),
    ResetTimer,
    ClearNotices,
    PortCopy(u16),
    PortStop(u32),
    NewKind(HarnessKind),
    NewProject(ProjectId),
    NewWorktree(bool),
    Merge(TaskId, u64),
    OpenUrl(String),
    /// K22's agent and model.
    VoiceAgent(HarnessKind),
    VoiceModel(Option<String>),
    Talk,
    Lens(Lens),
    OpenView(u64),
    Glass,
    Nudges,
    Focus,
    TalkAbout(TaskId),
}

/// A popup banner: it waits until handled or dismissed (suggestions time
/// out after [`SUGGEST_FOR`]).
#[derive(Clone, Debug)]
pub enum Popup {
    /// A plan passed 80 % of a limit; `key` remembers the dismissal.
    Usage { key: String, meter: bridge::Meter },
    /// Copied text that reads like an error.
    FixError { text: String },
    /// Files dropped on the island.
    Dropped { paths: Vec<PathBuf> },
    /// A dev server stopped listening and its process is gone.
    ServerDown { port: u16, process: String },
    /// A pull request's checks finished.
    Checks { pr: bridge::PrInfo },
    /// Approvals that waited out a focus timer.
    Held { count: usize },
    /// A screenshot to mark up and send.
    Shot(Shot),
}

impl Popup {
    /// Suggestions go away by themselves; the rest wait for you.
    fn expires(&self) -> bool {
        matches!(self, Popup::FixError { .. } | Popup::Dropped { .. } | Popup::ServerDown { .. } | Popup::Checks { .. })
    }

    /// Shown even in Focus mode (you asked for it, or it's about money).
    fn breaks_focus(&self) -> bool {
        matches!(self, Popup::Dropped { .. } | Popup::Shot(_) | Popup::Held { .. } | Popup::Usage { .. })
    }
}

/// A screenshot being marked up.
#[derive(Clone, Debug)]
pub struct Shot {
    pub path: PathBuf,
    pub image: Arc<gpui::RenderImage>,
    pub size: (u32, u32),
    pub title: String,
    pub marks: Vec<Mark>,
    pub tool: ShotTool,
    /// The mark being drawn.
    pub drawing: Option<Mark>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotTool {
    Arrow,
    Box,
    Highlight,
}

/// One mark on a screenshot, in fractions of its size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub tool: ShotTool,
    pub from: (f32, f32),
    pub to: (f32, f32),
}

/// A message the island sent that the agent hasn't shown as a turn yet.
#[derive(Clone, Debug)]
pub struct Sent {
    pub task: TaskId,
    pub text: String,
    pub at: Instant,
}

/// A question request's answers so far.
#[derive(Clone, Debug, Default)]
pub struct QAnswer {
    pub page: usize,
    pub picks: Vec<Vec<String>>,
}

/// An element flying between two views (album art, the timer ring), in
/// canvas coordinates: x, y, size.
#[derive(Clone, Copy, Debug)]
pub struct Hero {
    pub kind: HeroKind,
    pub from: (f32, f32, f32),
    pub to: (f32, f32, f32),
    pub at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeroKind {
    Art,
    Ring,
}

/// `DOVE_NOTCH_TIMING=1`: how long a transition took to show and settle.
#[derive(Clone, Debug)]
pub struct Timing {
    pub what: String,
    pub at: Instant,
    pub first: Option<Instant>,
    pub last_frame: Option<Instant>,
    pub max_gap: Duration,
}

/// Something that woke the loop.
enum Msg {
    Platform(win::Event),
    Link(ToIsland),
    LinkGone,
    Media(Option<media::Media>),
    Audio(win::AudioState),
}

/// The island's window while it exists, and what lives with it.
struct Win {
    handle: WindowHandle<NotchWindow>,
    native: Option<platform::Handle>,
    glass: Option<glass::Glass>,
}

/// The text fields (they belong to the window that draws them).
#[derive(Clone)]
pub struct Inputs {
    pub message: Entity<TextareaState>,
    pub prompt: Entity<InputState>,
    pub note: Entity<TextareaState>,
    pub command: Entity<InputState>,
    /// The wide pages' one-line field: a plan step, typing into the call, a
    /// follow-up to K22.
    pub line: Entity<InputState>,
}

pub struct Notch {
    store: Arc<Store>,
    rt: tokio::runtime::Handle,
    prefs: Prefs,
    prefs_at: Instant,
    machine: Machine,
    display: Option<Display>,
    fullscreen: bool,
    transparency: bool,
    hide_until: Option<Instant>,

    /// The island's shape, springing toward its target.
    springs: Springs<7>,
    /// The real target, applied once the dot has dropped (`dot_until`).
    pending_target: Option<Geo>,
    dot_until: Option<Instant>,
    /// Closing: shrink to the dot, then lift it away at `lift_at`.
    lift_at: Option<Instant>,
    hovered: bool,
    pressed: bool,
    magnet: f32,
    shapes: Vec<Shape>,
    epoch: Instant,

    win: Option<Win>,
    opening: bool,
    inputs: Option<Inputs>,
    /// The window is on screen.
    shown: bool,
    hidden_since: Option<Instant>,
    trimmed: bool,

    page: Page,
    snap: Snapshot,
    statuses: HashMap<TaskId, TaskStatus>,
    dismissed: HashSet<String>,
    flash: Option<Flash>,
    moment: Option<(String, Instant)>,
    notices: Vec<Notice>,
    unread_notices: usize,
    reply: String,
    /// The agent the tray and notes send to.
    last_agent: Option<TaskId>,
    toast: Option<(String, Instant)>,
    undo: Option<(Undo, Instant)>,
    deny: Option<PendingDeny>,
    /// Which slim activity leads (scroll cycles it).
    primary: usize,
    /// The slim island's right-click actions are showing.
    context_menu: bool,
    /// The open menu (one at a time), drawn inside the island.
    menu: Option<views::Menu>,
    /// Popup banners waiting (the first shows), with when each came.
    popups: Vec<(Popup, Instant)>,
    /// Usage warnings dismissed, by `kind|label|resets_at`.
    usage_dismissed: HashSet<String>,
    /// The last copied text offered for fixing (offered once).
    offered_error: Option<u64>,
    /// Each PR's checks as last seen, to notice them finish.
    pr_checks: HashMap<(TaskId, u64), String>,
    /// The state before the last transition.
    prev_state: State,

    media: Option<media::Media>,
    media_src: media::Source,
    audio: win::Audio,
    audio_state: win::AudioState,
    volume_shown_until: Option<Instant>,
    clips: Vec<Clip>,
    shelf: Vec<PathBuf>,
    icons: HashMap<PathBuf, Option<Arc<gpui::RenderImage>>>,
    notes: Vec<Note>,
    selected_note: Option<i64>,
    /// The note field changed; it saves itself shortly after.
    pub(crate) note_dirty: Option<Instant>,
    timer: Timer,
    /// "Until <agent> finishes": the timer also ends when this agent does.
    timer_link: Option<TaskId>,
    new_agent: NewAgent,
    tray_filter: TrayFilter,
    tray_selected: Option<usize>,
    ports: Vec<win::Port>,
    ports_at: Option<Instant>,
    /// When a dev server was last seen (they're watched for a while after).
    ports_seen: Option<Instant>,
    query: String,
    hit: usize,
    /// The hotkey opened the island: focus the command bar when it draws.
    focus_command: bool,
    /// Where Esc and the back arrow go from a page without a tab.
    back: Option<Page>,
    /// A tray file pressed for a drag out: its index and where the press was.
    drag_from: Option<(usize, gpui::Point<gpui::Pixels>)>,
    /// Dragging the music scrubber, as a fraction.
    scrub: Option<f32>,
    /// The scrubber's left edge and width on screen, from its last paint.
    scrub_bounds: Option<(f32, f32)>,
    page_at: Instant,
    /// The tab the header's puck slides from.
    tab_from: Option<Tab>,

    // The agent view.
    /// The agent whose text streams to us (the open agent view).
    watching: Option<TaskId>,
    /// Messages sent from here that the agent hasn't shown yet.
    sent: Vec<Sent>,
    /// Stop waiting out its grace period (Undo cancels it).
    stopping: Option<(TaskId, Instant)>,
    /// Answers to question requests, by request id.
    answers: HashMap<String, QAnswer>,
    reply_scroll: ScrollHandle,
    /// The reply follows the stream until you scroll up.
    stick: bool,
    /// Turns whose steps are unfolded.
    unfolded: HashSet<usize>,
    /// Files attached with @ for the next message.
    attachments: Vec<PathBuf>,
    /// The @ picker is open.
    picking: bool,
    /// Push-to-talk: the composer's text before dictation began.
    dictation_base: Option<String>,
    /// The composer's placeholder as last set ("Steer…", "Message…").
    composer_hint: &'static str,
    /// Ctrl+Alt+A / D are registered (an approval shows).
    approval_keys: bool,
    hero: Option<Hero>,
    timing: Option<Timing>,

    /// The window that had the keyboard before the island took it.
    prev_focus: Option<isize>,
    /// The captured, blurred screen behind the canvas (when composition
    /// glass isn't available), and its bookkeeping.
    backdrop: Option<Arc<gpui::RenderImage>>,
    backdrop_key: Vec<u8>,
    backdrop_at: Option<Instant>,
    retired: Vec<Arc<gpui::RenderImage>>,
    capturing: bool,
    /// How bright the backdrop under the island is (0–1), sampled with the
    /// backdrop (composition glass too), and the value the tint shows, which
    /// eases toward it.
    pub(crate) glass_luma: f32,
    pub(crate) luma_shown: f32,
    pub(crate) luma_frame: Instant,

    /// The line to Dove's engine (`None` while Dove isn't running), what
    /// waits to go over it, and when it was last tried.
    link: Option<ipc::Link>,
    queue: Vec<ToEngine>,
    link_at: Instant,
    launched: Option<Instant>,
    next_request: u64,
    msgs: flume::Sender<Msg>,

    // V3.
    /// The Voice Mode call (from the engine).
    pub(crate) call: CallInfo,
    /// Views shown, newest first, and the ones pinned to Home.
    pub(crate) views: Vec<NotchView>,
    pub(crate) widgets: Vec<NotchView>,
    pub(crate) asking: Option<Asking>,
    pub(crate) plan_edit: Option<PlanEdit>,
    pub(crate) call_typing: bool,
    /// The last view action that changes something, against double clicks.
    pub(crate) last_action: Option<(crate::island::view::ViewAction, Instant)>,
    /// Plans already announced (by task and request).
    plans_seen: HashSet<String>,
    /// Nudges offered (once each), and when an agent started waiting.
    nudged: HashSet<String>,
}

/// The window's root view: draws the notch, owns the text fields.
pub struct NotchWindow {
    notch: Entity<Notch>,
    _subs: Vec<Subscription>,
}

fn input_style() -> gpui_base::input::InputEditorStyle {
    gpui_base::input::InputEditorStyle {
        foreground: tk::TEXT,
        muted_foreground: tk::FAINT,
        background: gpui::transparent_black(),
        border: gpui::transparent_black(),
        selection: gpui::white().opacity(0.25),
        caret: gpui::white(),
        ..Default::default()
    }
}

impl NotchWindow {
    fn new(notch: Entity<Notch>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut s = InputState::new(window, cx).placeholder(placeholder);
                s.set_editor_style(input_style());
                s
            })
        };
        let message = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx).auto_grow(1, 6).submit_on_enter(true).placeholder("Message…");
            s.set_editor_style(input_style());
            s
        });
        let prompt = field("What should it do?", window, cx);
        let command = field("Ask K22, or search…", window, cx);
        let line = field("…", window, cx);
        let note = cx.new(|cx| {
            let mut s =
                TextareaState::new(window, cx).auto_grow(3, 8).placeholder("Write a note… (- [ ] for a checklist)");
            s.set_editor_style(input_style());
            s
        });
        // Carry over what was typed before the window last closed.
        let carried = notch.read(cx).inputs.as_ref().map(|i| {
            (
                i.message.read(cx).value().to_string(),
                i.prompt.read(cx).value().to_string(),
                i.note.read(cx).value().to_string(),
            )
        });
        if let Some((m, p, n)) = carried {
            message.update(cx, |s, cx| s.set_value(m, window, cx));
            prompt.update(cx, |s, cx| s.set_value(p, window, cx));
            note.update(cx, |s, cx| s.set_value(n, window, cx));
        }
        let subs = vec![
            cx.observe(&notch, |_, _, cx| cx.notify()),
            cx.subscribe_in(&message, window, |this, input, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { shift: false, .. } => {
                    this.notch.update(cx, |n, cx| n.send_message(window, cx))
                }
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    this.notch.update(cx, |n, cx| n.composer_changed(&text, cx));
                }
                _ => {}
            }),
            cx.subscribe_in(&prompt, window, |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    this.notch.update(cx, |n, cx| n.start_agent(window, cx));
                }
            }),
            cx.subscribe_in(&note, window, |this, _, ev: &InputEvent, _, cx| match ev {
                InputEvent::PressEnter { secondary: true, .. } => this.notch.update(cx, |n, cx| n.save_note(cx)),
                InputEvent::Change => this.notch.update(cx, |n, _| n.note_dirty = Some(Instant::now())),
                _ => {}
            }),
            cx.subscribe_in(&line, window, |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    this.notch.update(cx, |n, cx| n.line_enter(window, cx));
                }
            }),
            cx.subscribe_in(&command, window, |this, input, ev: &InputEvent, window, cx| {
                let text = input.read(cx).value().to_string();
                match ev {
                    InputEvent::Change => this.notch.update(cx, |n, cx| {
                        n.query = text;
                        n.hit = 0;
                        n.retarget(cx);
                        cx.notify();
                    }),
                    InputEvent::PressEnter { .. } => this.notch.update(cx, |n, cx| n.run_hit(None, window, cx)),
                    _ => {}
                }
            }),
        ];
        let inputs = Inputs { message, prompt, note, command, line };
        notch.update(cx, |n, _| n.inputs = Some(inputs));
        NotchWindow { notch, _subs: subs }
    }
}

impl Render for NotchWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        crate::perf_trace::count_render("notch");
        self.notch.update(cx, |n, cx| n.render_canvas(window, cx))
    }
}

/// Starts the notch where the platform has one (in the island process).
pub fn init(store: Arc<Store>, rt: tokio::runtime::Handle, cx: &mut App) -> Option<Entity<Notch>> {
    if !platform::SUPPORTED || std::env::var("DOVE_NOTCH").ok().as_deref() == Some("0") {
        return None;
    }
    let (tx, rx) = flume::unbounded::<Msg>();
    let notch = cx.new(|_| Notch::new(store, rt, tx.clone()));
    // Every source forwards into one channel the loop sleeps on.
    let platform_events = win::events();
    let t = tx.clone();
    cx.background_executor()
        .spawn(async move {
            while let Ok(e) = platform_events.recv_async().await {
                if t.send(Msg::Platform(e)).is_err() {
                    break;
                }
            }
        })
        .detach();
    let (media_rx, audio_rx) = {
        let n = notch.read(cx);
        (n.media_src.updates.clone(), n.audio.updates.clone())
    };
    let t = tx.clone();
    cx.background_executor()
        .spawn(async move {
            while let Ok(m) = media_rx.recv_async().await {
                if t.send(Msg::Media(m)).is_err() {
                    break;
                }
            }
        })
        .detach();
    let t = tx;
    cx.background_executor()
        .spawn(async move {
            while let Ok(a) = audio_rx.recv_async().await {
                if t.send(Msg::Audio(a)).is_err() {
                    break;
                }
            }
        })
        .detach();
    notch.update(cx, |n, cx| {
        if let Some(link) = n.link.take() {
            n.adopt_link(link, cx);
        }
        if let Ok(page) = std::env::var("DOVE_NOTCH_PAGE") {
            match page.as_str() {
                "new" => n.page = Page::NewAgent,
                p => {
                    if let Some(tab) = Tab::from_id(p) {
                        n.page = Page::Tab(tab);
                    } else if let Some(page) = n.qa_page(p) {
                        n.page = page;
                    }
                }
            }
        }
        match std::env::var("DOVE_NOTCH_STATE").as_deref() {
            Ok("slim") => n.force(State::Slim, cx),
            Ok("expanded") | Ok("open") => n.force(State::Expanded, cx),
            Ok("alert") => {
                n.flash(
                    FlashKind::Done,
                    "Claude Code finished · Fix the login redirect".into(),
                    Some(HarnessKind::ClaudeCode),
                    None,
                );
                n.force(State::Alert, cx);
            }
            _ => {}
        }
    });
    run(notch.clone(), rx, cx);
    Some(notch)
}

/// The loop: sleep until a message or the next deadline, then step.
fn run(notch: Entity<Notch>, rx: flume::Receiver<Msg>, cx: &mut App) {
    let demo = std::env::var_os("DOVE_NOTCH_DEMO").is_some();
    // QA: open and close every 1.6 s with a warm window, so the timing trace
    // (DOVE_NOTCH_TIMING) measures the transitions a hover makes.
    let cycle = std::env::var_os("DOVE_NOTCH_CYCLE").is_some();
    cx.spawn(async move |cx| {
        let started = Instant::now();
        let mut demoed = false;
        let mut cycled = Instant::now();
        loop {
            let mut wait = cx.update(|cx| notch.read(cx).next_wake(Instant::now()));
            if cycle {
                wait = wait.min(Duration::from_millis(100));
            }
            let timer = cx.background_executor().timer(wait);
            let msg = crate::island::host::race_value(rx.recv_async(), timer).await;
            cx.update(|cx| {
                notch.update(cx, |n, cx| {
                    if let Some(Ok(m)) = msg {
                        n.handle(m, cx);
                        // Take everything else that arrived together.
                        while let Ok(m) = rx.try_recv() {
                            n.handle(m, cx);
                        }
                    }
                    if demo && !demoed && started.elapsed() > Duration::from_millis(1500) {
                        demoed = true;
                        n.flash(
                            FlashKind::Done,
                            "Claude Code finished · Fix the login redirect".into(),
                            Some(HarnessKind::ClaudeCode),
                            None,
                        );
                    }
                    if cycle
                        && started.elapsed() > Duration::from_secs(3)
                        && cycled.elapsed() > Duration::from_millis(1600)
                    {
                        cycled = Instant::now();
                        let changed = if n.machine.state == State::Expanded {
                            n.machine.close(State::Hidden, false)
                        } else {
                            n.machine.open().inspect(|_| n.machine.set_pinned(true))
                        };
                        if changed.is_some() {
                            n.transition(cx);
                        }
                    }
                    n.step(cx);
                })
            });
        }
    })
    .detach();
}

impl Notch {
    fn new(store: Arc<Store>, rt: tokio::runtime::Handle, msgs: flume::Sender<Msg>) -> Self {
        let widgets = Self::load_widgets(&store);
        let prefs = Prefs::read(&store);
        let data = data::Saved::load(&store);
        let link = ipc::connect(&rt, store.root());
        Notch {
            prefs,
            prefs_at: Instant::now(),
            machine: Machine::new(),
            display: None,
            fullscreen: false,
            transparency: win::transparency_on(),
            hide_until: None,
            springs: Springs::new(Geo::HIDDEN.to_array(), tk::SPRING_OPEN.0, tk::SPRING_OPEN.1),
            pending_target: None,
            dot_until: None,
            lift_at: None,
            hovered: false,
            pressed: false,
            magnet: 0.0,
            shapes: Vec::new(),
            epoch: Instant::now(),
            win: None,
            opening: false,
            inputs: None,
            shown: false,
            hidden_since: Some(Instant::now()),
            trimmed: false,
            page: Page::Tab(Tab::Home),
            snap: Snapshot::default(),
            statuses: HashMap::new(),
            dismissed: HashSet::new(),
            flash: None,
            moment: None,
            notices: Vec::new(),
            unread_notices: 0,
            reply: String::new(),
            last_agent: None,
            toast: None,
            undo: None,
            deny: None,
            primary: 0,
            context_menu: false,
            menu: None,
            popups: Vec::new(),
            usage_dismissed: store
                .setting("notch_usage_dismissed")
                .unwrap_or_default()
                .split(',')
                .filter(|k| !k.is_empty())
                .map(str::to_string)
                .collect(),
            offered_error: None,
            pr_checks: HashMap::new(),
            prev_state: State::Hidden,
            media: None,
            media_src: media::start(),
            audio: win::Audio::start(),
            audio_state: win::AudioState::default(),
            volume_shown_until: None,
            clips: data.clips,
            shelf: data.shelf,
            icons: HashMap::new(),
            notes: data.notes,
            selected_note: None,
            note_dirty: None,
            timer: Timer::new(Duration::from_secs(25 * 60)),
            timer_link: None,
            new_agent: NewAgent::default(),
            tray_filter: TrayFilter::All,
            tray_selected: None,
            ports: Vec::new(),
            ports_at: None,
            ports_seen: None,
            query: String::new(),
            hit: 0,
            focus_command: false,
            back: None,
            drag_from: None,
            scrub: None,
            scrub_bounds: None,
            page_at: Instant::now(),
            tab_from: None,
            watching: None,
            sent: Vec::new(),
            stopping: None,
            answers: HashMap::new(),
            reply_scroll: ScrollHandle::new(),
            stick: true,
            unfolded: HashSet::new(),
            attachments: Vec::new(),
            picking: false,
            dictation_base: None,
            composer_hint: "",
            approval_keys: false,
            hero: None,
            timing: std::env::var_os("DOVE_NOTCH_TIMING").map(|_| Timing {
                what: String::new(),
                at: Instant::now(),
                first: Some(Instant::now()),
                last_frame: None,
                max_gap: Duration::ZERO,
            }),
            prev_focus: None,
            backdrop: None,
            backdrop_key: Vec::new(),
            backdrop_at: None,
            retired: Vec::new(),
            capturing: false,
            glass_luma: 0.3,
            luma_shown: 0.3,
            luma_frame: Instant::now(),
            link,
            queue: Vec::new(),
            link_at: Instant::now(),
            launched: None,
            next_request: 1,
            msgs,
            call: CallInfo::default(),
            views: Vec::new(),
            widgets,
            asking: None,
            plan_edit: None,
            call_typing: false,
            last_action: None,
            plans_seen: HashSet::new(),
            nudged: HashSet::new(),
            store,
            rt,
        }
    }

    /// A V3 page from `DOVE_NOTCH_PAGE` (`call`, `plan`, `view:<lens>`),
    /// with demo data under `DOVE_NOTCH_DEMO` so every page can be seen.
    fn qa_page(&mut self, page: &str) -> Option<Page> {
        let demo = std::env::var_os("DOVE_NOTCH_DEMO").is_some();
        match page {
            "call" => {
                if demo {
                    self.call = CallInfo {
                        live: true,
                        phase: crate::island::view::CallPhase::Speaking,
                        target: Some("K22".into()),
                        lines: vec![
                            (true, "What's everyone doing?".into()),
                            (false, "Two agents are working: Claude is fixing the login redirect and Codex is running the tests. One plan is waiting on you.".into()),
                            (true, "Show me the plan".into()),
                        ],
                        partial: Some("and then approve".into()),
                        level: 0.6,
                        elapsed_s: 94,
                        agent: Some(HarnessKind::ClaudeCode),
                        ..Default::default()
                    };
                }
                Some(Page::Call)
            }
            "plan" => {
                let task = TaskId::new();
                if demo {
                    let text = "1. Add a `redirect_to` check in `src/auth/login.rs`\n   - keep the existing session flow\n   - reject absolute URLs to other hosts\n2. Cover it with tests in `tests/login_redirect.rs`\n3. Update docs/AUTH.md with the new rule\n4. Run `cargo test -p auth`";
                    self.snap.plans.push(crate::island::view::PendingPlan {
                        task,
                        request_id: Some("demo".into()),
                        kind: HarnessKind::ClaudeCode,
                        title: "Fix the login redirect".into(),
                        project: "dove".into(),
                        text: text.into(),
                        steps: crate::island::view::plan_steps(text),
                        files: crate::island::view::plan_files(text),
                    });
                }
                Some(Page::Plan(task))
            }
            p => {
                let lens = Lens::from_id(p.strip_prefix("view:")?)?;
                if demo && matches!(lens, Lens::Status | Lens::Today) {
                    // A K22 view next to the lens, for the history.
                    self.views.push(NotchView {
                        id: 7,
                        title: "Release status".into(),
                        subtitle: Some("3 blockers left".into()),
                        blocks: vec![crate::island::view::Block::Progress {
                            label: "Release checklist".into(),
                            value: 0.7,
                            tone: crate::island::view::Tone::Good,
                        }],
                        ..Default::default()
                    });
                }
                Some(Page::Lens(lens))
            }
        }
    }

    // ----- the loop -----------------------------------------------------------

    /// How long the loop may sleep.
    fn next_wake(&self, now: Instant) -> Duration {
        let mut at: Vec<Instant> = Vec::new();
        at.extend(self.machine.deadline());
        at.extend(self.dot_until);
        at.extend(self.lift_at);
        at.extend(self.flash.as_ref().map(|f| f.until));
        at.extend(self.moment.as_ref().map(|m| m.1 + MOMENT_FOR));
        at.extend(self.toast.as_ref().map(|t| t.1 + Duration::from_millis(1600)));
        at.extend(self.undo.as_ref().map(|u| u.1 + UNDO_FOR));
        at.extend(self.deny.as_ref().map(|d| d.at + DENY_GRACE));
        at.extend(self.stopping.map(|(_, t)| t + STOP_GRACE));
        at.extend(self.note_dirty.map(|t| t + NOTE_SAVE_AFTER));
        at.extend(self.popups.iter().filter(|(p, _)| p.expires()).map(|(_, t)| *t + SUGGEST_FOR));
        if self.ports_seen.is_some_and(|t| now < t + PORTS_WATCH) {
            at.push(self.ports_at.map_or(now, |t| t + PORTS_EVERY));
        }
        at.extend(self.volume_shown_until);
        if self.timer.running() {
            at.push(now + self.timer.remaining());
            if self.shown {
                // The time on screen ticks once a second.
                let ms = 1000 - (self.timer.remaining().subsec_millis() as u64).min(999);
                at.push(now + Duration::from_millis(ms));
            }
        }
        if !self.shown {
            if self.win.is_some() {
                at.extend(self.hidden_since.map(|t| t + CLOSE_AFTER));
            } else if !self.trimmed {
                at.extend(self.hidden_since.map(|t| t + TRIM_AFTER));
            }
        }
        if self.shown && self.animating() {
            at.push(now + Duration::from_millis(100));
        }
        if self.shown && self.samples_backdrop() {
            let every = self.backdrop_every();
            at.push(self.backdrop_at.map_or(now, |t| t + Duration::from_millis(every)));
        }
        if self.shown && self.page == Page::Tab(Tab::Dev) && self.machine.state == State::Expanded {
            at.push(self.ports_at.map_or(now, |t| t + Duration::from_secs(3)));
        }
        if self.link.is_none() {
            at.push(self.link_at + Duration::from_secs(3));
        }
        at.push(self.prefs_at + Duration::from_secs(5));
        at.extend(self.hide_until);
        at.into_iter()
            .min()
            .map(|t| t.saturating_duration_since(now))
            .unwrap_or(Duration::from_secs(5))
            .max(Duration::from_millis(4))
    }

    fn handle(&mut self, msg: Msg, cx: &mut Context<Self>) {
        match msg {
            Msg::Platform(win::Event::Pointer) => {}
            Msg::Platform(win::Event::Foreground) => {
                if let Some(d) = self.display {
                    self.fullscreen = platform::fullscreen(d);
                }
            }
            Msg::Platform(win::Event::Clipboard) => {
                if self.poll_clipboard(cx) {
                    self.offer_fix();
                    self.retarget(cx);
                    cx.notify();
                }
            }
            Msg::Platform(win::Event::Hotkey) => self.hotkey(cx),
            Msg::Platform(win::Event::Approve(allow)) => self.approve_key(allow, cx),
            Msg::Platform(win::Event::Display) => {
                self.display = None;
                self.transparency = win::transparency_on();
                self.audio.refresh();
            }
            Msg::Link(m) => self.on_link(m, cx),
            Msg::LinkGone => {
                tracing::info!("island: Dove went away");
                self.watching = None;
                self.link = None;
                self.link_at = Instant::now();
                self.snap.agents.clear();
                self.snap.flock.clear();
                self.retarget(cx);
                cx.notify();
            }
            Msg::Media(m) => {
                self.media = m;
                self.retarget(cx);
                cx.notify();
            }
            Msg::Audio(a) => {
                self.audio_state = a;
                if self.shown {
                    cx.notify();
                }
            }
        }
    }

    /// Looks at everything once: expiries, the pointer, the state, the shape.
    fn step(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let mut dirty = false;
        if self.display.is_none() || self.prefs.display == DisplayPick::Pointer {
            let d = platform::display_for(self.prefs.display == DisplayPick::Pointer);
            if d != self.display {
                self.display = d;
                if let Some(d) = d {
                    self.fullscreen = platform::fullscreen(d);
                }
                if self.shown {
                    self.native(cx, |h, d| h.place(d, tk::CANVAS_W, tk::CANVAS_H));
                }
            }
        }
        if now >= self.prefs_at + Duration::from_secs(5) {
            self.prefs_at = now;
            let prefs = Prefs::read(&self.store);
            if prefs != self.prefs {
                self.prefs = prefs;
                dirty = true;
            }
        }
        if self.hide_until.is_some_and(|t| now >= t) {
            self.hide_until = None;
            dirty = true;
        }
        if self.flash.as_ref().is_some_and(|f| now >= f.until) {
            self.flash = None;
            dirty = true;
        }
        if self.moment.as_ref().is_some_and(|m| now >= m.1 + MOMENT_FOR) {
            self.moment = None;
            dirty = true;
        }
        if self.toast.as_ref().is_some_and(|t| now >= t.1 + Duration::from_millis(1600)) {
            self.toast = None;
            dirty = true;
        }
        if self.undo.as_ref().is_some_and(|u| now >= u.1 + UNDO_FOR) {
            self.undo = None;
            dirty = true;
        }
        if self.volume_shown_until.is_some_and(|t| now >= t) {
            self.volume_shown_until = None;
            dirty = true;
        }
        if let Some((task, _)) = self.stopping.take_if(|(_, t)| now >= *t + STOP_GRACE) {
            self.command(ToEngine::Stop { task });
            dirty = true;
        }
        if self.note_dirty.is_some_and(|t| now >= t + NOTE_SAVE_AFTER) {
            self.flush_note(cx);
        }
        let before = self.popups.len();
        self.popups.retain(|(p, t)| !p.expires() || now < *t + SUGGEST_FOR);
        if self.popups.len() != before {
            dirty = true;
        }
        if let Some(d) = self.deny.take_if(|d| now >= d.at + DENY_GRACE) {
            self.command(ToEngine::Decide {
                task: d.task,
                request_id: d.request_id,
                decision: dove_core::proto::PermissionDecision::Deny,
            });
            dirty = true;
        }
        if self.timer.finished() {
            // Approvals held for the focus timer arrive together now.
            let held = self.held_count();
            self.timer.reset();
            self.timer_link = None;
            self.flash(FlashKind::Timer, "Time's up".into(), None, None);
            if held > 0 {
                self.push_popup(Popup::Held { count: held });
            }
            dirty = true;
        }
        if self.link.is_none() && now >= self.link_at + Duration::from_secs(3) {
            self.link_at = now;
            if let Some(link) = ipc::connect(&self.rt, self.store.root()) {
                tracing::info!("island: connected to Dove");
                self.adopt_link(link, cx);
                dirty = true;
            }
        }
        // Dev servers: checked while their page shows, and for a while after
        // one was seen, so a crash can be noticed.
        let looking = self.shown && self.page == Page::Tab(Tab::Dev) && self.machine.state == State::Expanded;
        let watching = self.ports_seen.is_some_and(|t| now < t + PORTS_WATCH);
        let every = if looking { Duration::from_secs(3) } else { PORTS_EVERY };
        if (looking || watching) && self.ports_at.is_none_or(|t| now >= t + every) {
            self.ports_at = Some(now);
            let ports = win::listening_ports();
            if ports != self.ports {
                for gone in self.ports.iter().filter(|p| !ports.iter().any(|q| q.port == p.port)) {
                    if !win::process_alive(gone.pid) {
                        let popup = Popup::ServerDown { port: gone.port, process: gone.process.clone() };
                        self.popups.push((popup, now));
                    }
                }
                self.ports = ports;
                dirty = true;
            }
            if !self.ports.is_empty() {
                self.ports_seen = Some(now);
            }
        }
        // Ctrl+Alt+A / D answer the approval on show, from any app.
        let want_keys = self.approval().is_some_and(|(_, p)| p.safe && !p.question);
        if want_keys != self.approval_keys {
            self.approval_keys = want_keys;
            win::approval_keys(want_keys);
        }
        // The open agent view streams its agent's text.
        let watch = match self.page {
            Page::Agent(id) | Page::Plan(id) | Page::Lens(Lens::Agent(id)) if self.machine.state == State::Expanded => {
                Some(id)
            }
            // A view streams what its agent says with it (K22's answer).
            Page::View(v) if self.machine.state == State::Expanded => self
                .views
                .iter()
                .find(|x| x.id == v)
                .and_then(|x| x.task)
                .or(self.asking.as_ref().filter(|a| a.id == v).and_then(|a| a.task)),
            _ => None,
        };
        if watch != self.watching && self.connected() {
            self.watching = watch;
            self.command(ToEngine::Watch { task: watch });
        }
        if self.shown
            && self.samples_backdrop()
            && self.backdrop_at.is_none_or(|t| now >= t + Duration::from_millis(self.backdrop_every()))
        {
            self.refresh_backdrop(cx);
        }
        // The dot has dropped: spring into the real shape. / Closing: lift
        // the dot away.
        if self.dot_until.is_some_and(|t| now >= t) {
            self.dot_until = None;
            if let Some(g) = self.pending_target.take() {
                self.springs.set_targets(g.to_array());
                dirty = true;
            }
        }
        if self.lift_at.is_some_and(|t| now >= t) {
            self.lift_at = None;
            if self.machine.state == State::Hidden {
                self.springs.set_targets(Geo::HIDDEN.to_array());
                dirty = true;
            }
        }
        // The state machine.
        if let Some(display) = self.display {
            let cursor = platform::cursor();
            // Heading for the top: make the window now, so opening never
            // waits for it.
            if !self.shown
                && self.win.is_none()
                && !self.opening
                && self.prefs.enabled
                && !self.fullscreen
                && let Some((x, y)) = cursor
            {
                let s = display.scale;
                let near = (x - display.center_x()).abs() as f32 <= (HOT_WIDTH / 2.0 + WARM_BAND) * s
                    && y >= display.top
                    && ((y - display.top) as f32) < WARM_BAND * s;
                if near {
                    self.ensure_window(cx);
                }
            }
            let engaged = self.win.as_ref().and_then(|w| w.native).is_some_and(|h| h.is_foreground());
            let input = Input {
                display,
                cursor,
                button_down: platform::button_down(),
                fullscreen: self.fullscreen,
                engaged,
                rest: self.rest(),
                alert: self.alerting(),
                island: self.island_on_screen(),
            };
            self.update_magnet(&input);
            if self.machine.tick(&input, now).is_some() {
                self.transition(cx);
                dirty = true;
            }
        }
        if dirty {
            self.retarget(cx);
            cx.notify();
        } else if self.shown && self.animating() {
            cx.notify();
        }
        // Long hidden: close the window and give memory back.
        if !self.shown && self.hidden_since.is_some_and(|t| now >= t + TRIM_AFTER) {
            if self.win.is_some() {
                if self.hidden_since.is_some_and(|t| now >= t + CLOSE_AFTER) {
                    self.drop_window(cx);
                }
            } else if !self.trimmed {
                // Never opened since launch: still give start-up memory back.
                self.trimmed = true;
                win::trim_working_set();
            }
        }
    }

    // ----- state ----------------------------------------------------------------

    fn rest(&self) -> State {
        if !self.prefs.enabled || self.hide_until.is_some() {
            return State::Hidden;
        }
        if self.prefs.slim_always || (self.prefs.live && !self.slim_items().is_empty()) {
            State::Slim
        } else {
            State::Hidden
        }
    }

    fn alerting(&self) -> bool {
        if !self.prefs.enabled || !self.prefs.live || self.hide_until.is_some() {
            return false;
        }
        let flash = self.flash.as_ref().is_some_and(|f| !self.prefs.focus || f.kind == FlashKind::Timer);
        self.approval().is_some() || flash || self.popup().is_some()
    }

    /// The first approval nobody has dismissed (or is denying), unless a
    /// focus timer holds it (safe requests wait for the timer to end).
    pub(crate) fn approval(&self) -> Option<(&bridge::Agent, &bridge::Pending)> {
        self.snap.agents.iter().flat_map(|a| a.pending.iter().map(move |p| (a, p))).find(|(_, p)| {
            !self.dismissed.contains(&p.request_id)
                && self.deny.as_ref().is_none_or(|d| d.request_id != p.request_id)
                && !self.held(p)
        })
    }

    /// A focus timer runs and this request can wait for it.
    pub(crate) fn held(&self, p: &bridge::Pending) -> bool {
        self.prefs.focus && self.timer.running() && p.safe && !p.question
    }

    fn held_count(&self) -> usize {
        self.snap.agents.iter().flat_map(|a| a.pending.iter()).filter(|p| self.held(p)).count()
    }

    /// The popup on show: the first one Focus mode lets through.
    pub(crate) fn popup(&self) -> Option<&Popup> {
        self.popups.iter().map(|(p, _)| p).find(|p| !self.prefs.focus || p.breaks_focus())
    }

    pub(crate) fn push_popup(&mut self, p: Popup) {
        self.popups.push((p, Instant::now()));
    }

    /// Dismisses the popup on show (remembering a usage warning's).
    pub(crate) fn dismiss_popup(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.popups.iter().position(|(p, _)| !self.prefs.focus || p.breaks_focus()) else { return };
        match self.popups.remove(i).0 {
            Popup::Usage { key, .. } => {
                self.usage_dismissed.insert(key);
                let keys: Vec<&str> = self.usage_dismissed.iter().map(String::as_str).collect();
                let _ = self.store.set_setting("notch_usage_dismissed", &keys.join(","));
            }
            Popup::Shot(shot) => {
                let _ = std::fs::remove_file(&shot.path);
            }
            _ => {}
        }
        self.retarget(cx);
        cx.notify();
    }

    fn flash(&mut self, kind: FlashKind, text: String, agent: Option<HarnessKind>, task: Option<TaskId>) {
        let now = Instant::now();
        self.notices.insert(0, Notice { at_ms: dove_core::store::now_ms(), kind, text: text.clone(), agent, task });
        self.notices.truncate(60);
        self.unread_notices += 1;
        // In the slim island a finish is a moment, not a banner.
        if kind == FlashKind::Done && self.machine.state == State::Slim {
            self.moment = Some(("Done".into(), now));
            return;
        }
        if self.prefs.focus && kind != FlashKind::Timer {
            return;
        }
        self.flash = Some(Flash { kind, text, agent, task, at: now, until: now + FLASH_FOR });
    }

    pub(crate) fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }

    fn force(&mut self, state: State, cx: &mut Context<Self>) {
        self.machine.go(state);
        self.machine.set_pinned(true);
        self.transition(cx);
    }

    fn island_on_screen(&self) -> (i32, i32, i32, i32) {
        let Some(d) = self.display else { return (0, 0, 0, 0) };
        let s = d.scale;
        let g = Geo::from_array(self.springs.values());
        let left = d.center_x() - ((tk::CANVAS_W / 2.0 - g.x()) * s) as i32;
        (left, d.top + (g.y * s) as i32, left + (g.span() * s) as i32, d.top + ((g.y + g.h) * s) as i32)
    }

    fn update_magnet(&mut self, input: &Input) {
        let mut target = 0.0;
        if self.machine.state == State::Slim
            && let Some((x, y)) = input.cursor
        {
            let d = input.display;
            let (l, t, r, b) = input.island;
            let range = (tk::MAGNET_RANGE * d.scale) as i32;
            if y >= t - range && y <= b + range && x >= l - range && x <= r + range {
                let center = (l + r) / 2;
                let half = ((r - l) / 2 + range).max(1) as f32;
                target = ((x - center) as f32 / half).clamp(-1.0, 1.0) * tk::MAGNET_PULL;
            }
        }
        if (target - self.magnet).abs() > 0.2 {
            self.magnet = target;
            if self.pending_target.is_none() && self.machine.state == State::Slim {
                let mut t = self.springs.targets();
                t[4] = target;
                self.springs.set_targets(t);
            }
        }
    }

    /// The shape the current state and content want.
    fn target(&self) -> Geo {
        let mut g = match self.machine.state {
            State::Hidden => return Geo::HIDDEN,
            State::Slim => {
                let (w, bubbles) = self.slim_size();
                Geo {
                    y: tk::TOP_GAP,
                    w,
                    h: tk::SLIM_H,
                    r: tk::SLIM_H / 2.0,
                    dx: self.magnet,
                    b1: if bubbles >= 1 { tk::BUBBLE } else { 0.0 },
                    b2: if bubbles >= 2 { tk::BUBBLE } else { 0.0 },
                }
            }
            State::Alert => {
                let (w, h) = self.banner_size();
                Geo { y: tk::TOP_GAP, w, h, r: if h > 60.0 { 28.0 } else { h / 2.0 }, dx: 0.0, b1: 0.0, b2: 0.0 }
            }
            State::Expanded => Geo {
                y: tk::TOP_GAP,
                w: self.page_width(),
                h: self.page_height(),
                r: tk::R_ISLAND,
                dx: 0.0,
                b1: 0.0,
                b2: 0.0,
            },
        };
        if self.machine.state == State::Slim && self.hovered {
            g.w += 6.0;
            g.h += 2.0;
            g.r += 1.0;
        }
        if self.pressed {
            g.w *= 1.0 - tk::SQUISH;
            g.h *= 1.0 - tk::SQUISH;
        }
        g
    }

    /// Opening from nothing drops a dot first; closing shrinks to it and
    /// lifts it away; anything else springs straight to the new shape.
    fn transition(&mut self, cx: &mut Context<Self>) {
        let state = self.machine.state;
        tracing::info!(target: "dove::notch", "notch: {state:?}");
        self.context_menu = false;
        self.menu = None;
        let now = Instant::now();
        let current = Geo::from_array(self.springs.values());
        if let Some(t) = &mut self.timing {
            *t = Timing {
                what: format!("{:?} → {state:?}", self.prev_state),
                at: now,
                first: None,
                last_frame: None,
                max_gap: Duration::ZERO,
            };
        }
        if state == State::Expanded && self.prev_state == State::Slim {
            self.hero = self.hero_for_open();
        }
        self.prev_state = state;
        if state == State::Hidden {
            self.springs.tune(tk::SPRING_CLOSE);
            self.springs.set_targets(Geo::DOT.to_array());
            self.pending_target = None;
            self.dot_until = None;
            self.lift_at = Some(now + Duration::from_millis(90));
            self.release_focus(cx);
        } else {
            self.ensure_window(cx);
            self.hidden_since = None;
            self.trimmed = false;
            if !self.shown {
                self.shown = true;
                self.backdrop_at = None;
                self.native(cx, |h, d| h.place(d, tk::CANVAS_W, tk::CANVAS_H));
            }
            let opening = current.h <= Geo::DOT.h + 2.0;
            let target = self.target();
            self.pending_target = None;
            self.dot_until = None;
            if opening {
                // The dot drops as it grows: the spring starts just above the
                // edge, so the drop is the first part of the motion, not a
                // step before it.
                let mut seed = Geo::DOT;
                seed.y = tk::TOP_GAP - 10.0;
                self.springs.place(seed.to_array());
                self.springs.tune(tk::SPRING_OPEN);
            } else {
                let grows = target.h > current.h + 1.0 || target.w > current.w + 1.0;
                self.springs.tune(if grows { tk::SPRING_OPEN } else { tk::SPRING_CLOSE });
            }
            self.springs.set_targets(target.to_array());
            self.lift_at = None;
            if state == State::Expanded {
                if self.machine.drag_reveal {
                    self.page = Page::Tab(Tab::Tray);
                }
                if self.page == Page::Tab(Tab::Alerts) {
                    self.unread_notices = 0;
                }
                self.refresh_reply();
            } else {
                self.release_focus(cx);
            }
        }
        if crate::motion::reduced_now() {
            if let Some(g) = self.pending_target.take() {
                self.springs.set_targets(g.to_array());
            }
            if state == State::Hidden {
                self.springs.set_targets(Geo::HIDDEN.to_array());
            }
            self.dot_until = None;
            self.lift_at = None;
            self.springs.snap();
        }
        self.audio.sample(self.wants_level());
        cx.notify();
    }

    /// The content changed size within the same state: spring to it.
    pub(crate) fn retarget(&mut self, cx: &mut Context<Self>) {
        if self.machine.state == State::Hidden {
            return;
        }
        let target = self.target();
        if self.pending_target.is_some() {
            self.pending_target = Some(target);
            return;
        }
        if Geo::from_array(self.springs.targets()) != target {
            // A change of size within a state: quick and no bounce (an
            // opening still in flight keeps its own spring).
            if self.springs.settled() {
                self.springs.tune(tk::SPRING_RESIZE);
            }
            self.springs.set_targets(target.to_array());
            if crate::motion::reduced_now() {
                self.springs.snap();
            }
            cx.notify();
        }
        self.audio.sample(self.wants_level());
    }

    /// The equalizer is on screen, so the real output level is worth reading.
    fn wants_level(&self) -> bool {
        let playing = self.media.as_ref().is_some_and(|m| m.playing);
        playing
            && match self.machine.state {
                State::Slim => self.slim_items().first() == Some(&views::SlimItem::Music),
                State::Expanded => matches!(self.page, Page::Tab(Tab::Music | Tab::Home)),
                _ => false,
            }
    }

    /// Something on the island moves on its own (a pulse, a clock), so it
    /// repaints at a low rate while visible.
    fn animating(&self) -> bool {
        if !self.shown {
            return false;
        }
        let needs = self.snap.agents.iter().any(|a| a.needs_you());
        let working = self.snap.agents.iter().any(|a| a.status == TaskStatus::Running)
            || self.snap.flock.iter().any(|f| f.working);
        if self.hero.is_some() {
            return true;
        }
        match self.machine.state {
            State::Slim => needs || working || self.moment.is_some(),
            State::Alert => true,
            State::Expanded => match self.page {
                Page::Tab(Tab::Home) | Page::Tab(Tab::Agents) | Page::Agent(_) => needs || working,
                Page::Tab(Tab::Music) => self.media.as_ref().is_some_and(|m| m.playing),
                _ => false,
            },
            State::Hidden => false,
        }
    }

    fn hotkey(&mut self, cx: &mut Context<Self>) {
        if self.machine.state == State::Expanded {
            if self.machine.close(self.rest(), self.alerting()).is_some() {
                self.transition(cx);
            }
            return;
        }
        if self.machine.open().is_some() {
            self.machine.set_pinned(true);
            self.transition(cx);
        }
        self.take_focus(cx);
        self.focus_command = true;
        cx.notify();
    }

    // ----- the window -------------------------------------------------------------

    fn uses_captured_glass(&self) -> bool {
        self.prefs.glass
            && self.transparency
            && self.win.as_ref().is_some_and(|w| w.glass.is_none() && w.native.is_some())
    }

    /// Whether the screen behind is sampled: for captured glass's image, or
    /// (with composition glass) just for its brightness, which sets the tint.
    fn samples_backdrop(&self) -> bool {
        self.prefs.glass && self.transparency && self.win.as_ref().is_some_and(|w| w.native.is_some())
    }

    /// How often the backdrop is sampled (ms): captured glass needs fresh
    /// frames; the brightness alone can lag.
    fn backdrop_every(&self) -> u64 {
        match (self.uses_captured_glass(), self.machine.state == State::Slim) {
            (true, true) => 2000,
            (true, false) => 600,
            (false, true) => 3000,
            (false, false) => 1200,
        }
    }

    /// Opens the window (once) outside the current update: opening draws a
    /// first frame, and that reads this notch.
    fn ensure_window(&mut self, cx: &mut Context<Self>) {
        if self.win.is_some() || self.opening {
            return;
        }
        self.opening = true;
        let this = cx.entity();
        cx.spawn(async move |_, cx| {
            let options = WindowOptions {
                // Placed for real by `platform::Handle::place`.
                window_bounds: Some(WindowBounds::Windowed(gpui::Bounds::new(
                    point(px(0.0), px(0.0)),
                    size(px(tk::CANVAS_W), px(tk::CANVAS_H)),
                ))),
                titlebar: None,
                focus: false,
                show: false,
                kind: WindowKind::PopUp,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                window_background: WindowBackgroundAppearance::Transparent,
                app_id: Some("dev.dove.Dove.notch".into()),
                ..Default::default()
            };
            let notch = this.clone();
            let opened = cx.update(|cx| {
                cx.open_window(options, move |window, cx| {
                    crate::design::apply_to_window(window);
                    cx.new(|cx| NotchWindow::new(notch, window, cx))
                })
            });
            cx.update(|cx| match opened {
                Ok(handle) => {
                    let native = handle.update(cx, |_, window, _| platform::Handle::of(window)).ok().flatten();
                    this.update(cx, |n, cx| n.adopt_window(handle, native, cx));
                }
                Err(e) => {
                    tracing::warn!("notch: could not open its window: {e:#}");
                    this.update(cx, |n, _| n.opening = false);
                }
            });
        })
        .detach();
    }

    fn adopt_window(
        &mut self,
        handle: WindowHandle<NotchWindow>,
        native: Option<platform::Handle>,
        cx: &mut Context<Self>,
    ) {
        self.opening = false;
        let glass = native.filter(|_| self.prefs.glass && self.transparency).and_then(|h| glass::Glass::new(h.raw()));
        if let Some(h) = native {
            h.prepare();
            if !self.shown {
                h.hide();
            }
        }
        self.win = Some(Win { handle, native, glass });
        self.shapes.clear();
        self.springs.rest();
        if self.shown {
            self.native(cx, |h, d| h.place(d, tk::CANVAS_W, tk::CANVAS_H));
        }
        cx.notify();
    }

    /// Closes the window (it's been hidden a while) and trims memory.
    fn drop_window(&mut self, cx: &mut Context<Self>) {
        let Some(win) = self.win.take() else { return };
        drop(win.glass);
        self.backdrop = None;
        self.backdrop_key.clear();
        self.retired.clear();
        self.icons.clear();
        self.shapes.clear();
        let handle = win.handle;
        cx.spawn(async move |_, cx| {
            cx.update(|cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
            cx.background_executor().timer(Duration::from_millis(500)).await;
            win::trim_working_set();
        })
        .detach();
        self.trimmed = true;
    }

    /// Moves, shapes or hides the native window once the current update is
    /// over: those calls send window messages synchronously, and GPUI's
    /// handlers can't borrow a window in the middle of an update.
    fn native(&self, cx: &mut Context<Self>, op: impl FnOnce(platform::Handle, Display) + 'static) {
        let (Some(handle), Some(display)) = (self.win.as_ref().and_then(|w| w.native), self.display) else { return };
        cx.spawn(async move |_, _| op(handle, display)).detach();
    }

    /// The shape changed: clip the window (and its glass) to it.
    fn apply_shapes(&mut self, shapes: Vec<Shape>, cx: &mut Context<Self>) {
        if shapes == self.shapes {
            return;
        }
        let scale = self.display.map(|d| d.scale).unwrap_or(1.0);
        if let Some(glass) = self.win.as_ref().and_then(|w| w.glass.as_ref()) {
            glass.shape(&shapes, scale);
        }
        self.shapes = shapes.clone();
        self.native(cx, move |h, _| h.shape(&shapes, scale));
    }

    /// The island finished closing: hide the window (it's closed later).
    fn settled_hidden(&mut self, cx: &mut Context<Self>) {
        if self.machine.state == State::Hidden && self.shown {
            self.shown = false;
            self.hidden_since = Some(Instant::now());
            self.audio.sample(false);
            self.native(cx, |h, _| h.hide());
        }
    }

    /// Captures and blurs the screen behind the canvas off the UI thread
    /// (when composition glass isn't available). A frame equal to the last
    /// one changes nothing, so a still desktop costs no repaint.
    fn refresh_backdrop(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.display else { return };
        if self.capturing {
            return;
        }
        self.capturing = true;
        self.backdrop_at = Some(Instant::now());
        let (w, h) = ((tk::CANVAS_W * d.scale).round() as i32, (tk::CANVAS_H * d.scale).round() as i32);
        let (x, y) = (d.center_x() - w / 2, d.top);
        // The island's target shape, in the shrunk capture's pixels.
        const SHRINK: i32 = 8;
        let k = d.scale / SHRINK as f32;
        let g = Geo::from_array(self.springs.targets());
        let under = [g.x() * k, g.y * k, (g.x() + g.w) * k, (g.y + g.h) * k];
        // QA screenshots leave the island in the capture: read around it.
        let around = std::env::var_os("DOVE_NOTCH_CAPTURABLE").is_some();
        let image = self.uses_captured_glass();
        let task = cx.background_executor().spawn(async move {
            platform::capture_behind(x, y, w, h, SHRINK).map(|small| {
                let luma = luma_of(&small, under, around);
                (luma, image.then(|| frost(small)))
            })
        });
        cx.spawn(async move |this, cx| {
            let frame = task.await;
            let _ = this.update(cx, |n, cx| {
                n.capturing = false;
                let Some((luma, frame)) = frame else { return };
                if (luma - n.glass_luma).abs() > 0.02 {
                    n.glass_luma = luma;
                    cx.notify();
                }
                if let Some((key, image)) = frame
                    && key != n.backdrop_key
                {
                    n.backdrop_key = key;
                    if let Some(old) = n.backdrop.replace(image) {
                        n.retired.push(old);
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Takes the keyboard, so a field in the island can be typed in.
    pub(crate) fn take_focus(&mut self, cx: &mut Context<Self>) {
        let Some(handle) = self.win.as_ref().and_then(|w| w.native) else { return };
        let this = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let prev = handle.activate();
            if prev.is_some() {
                let _ = this.update(cx, |n, _| n.prev_focus = prev);
            }
        })
        .detach();
    }

    fn release_focus(&mut self, cx: &mut Context<Self>) {
        let prev = self.prev_focus.take();
        self.native(cx, move |handle, _| handle.deactivate(prev));
    }

    // ----- the engine ---------------------------------------------------------------

    fn adopt_link(&mut self, link: ipc::Link, cx: &mut Context<Self>) {
        let inbound = link.inbound.clone();
        let tx = self.msgs.clone();
        cx.background_executor()
            .spawn(async move {
                while let Ok(m) = inbound.recv_async().await {
                    if tx.send(Msg::Link(m)).is_err() {
                        return;
                    }
                }
                let _ = tx.send(Msg::LinkGone);
            })
            .detach();
        self.link = Some(link);
        for msg in std::mem::take(&mut self.queue) {
            self.command(msg);
        }
    }

    fn on_link(&mut self, m: ToIsland, cx: &mut Context<Self>) {
        match m {
            ToIsland::Snapshot { snap } => {
                if self.apply_snapshot(snap) {
                    if let Page::Agent(id) = self.page
                        && let Some(a) = self.snap.agents.iter().find(|a| a.id == id)
                    {
                        self.reply = a.reply.clone();
                    }
                    self.check_usage();
                    self.check_prs();
                    self.check_plans();
                    self.check_nudges();
                    self.retarget(cx);
                    cx.notify();
                }
            }
            ToIsland::Dictation { text, done, error } => {
                if let Some(e) = error {
                    self.dictation_base = None;
                    self.toast(e);
                } else if let Some(base) = self.dictation_base.clone() {
                    let joined = if base.trim().is_empty() { text } else { format!("{} {}", base.trim_end(), text) };
                    if done {
                        self.dictation_base = None;
                    }
                    self.with_window(cx, move |n, window, cx| {
                        if let Some(i) = n.inputs.clone() {
                            i.message.update(cx, |s, cx| s.set_value(joined, window, cx));
                        }
                    });
                }
                cx.notify();
            }
            ToIsland::Merged { result, .. } => {
                match result {
                    Ok(()) => self.toast("Merged"),
                    Err(e) => self.toast(format!("Couldn't merge: {}", e.lines().next().unwrap_or(""))),
                }
                cx.notify();
            }
            ToIsland::Started { id, result } => {
                if id + 1 == self.next_request {
                    match result {
                        Ok(task) => {
                            self.with_window(cx, |n, window, cx| {
                                if let Some(i) = &n.inputs {
                                    i.prompt.update(cx, |s, cx| s.set_value("", window, cx));
                                }
                            });
                            self.toast("Started");
                            self.last_agent = Some(task);
                            self.open_page(Page::Agent(task), None, cx);
                        }
                        Err(e) => self.new_agent.error = Some(e),
                    }
                    cx.notify();
                }
            }
            ToIsland::Call { call } => {
                let started = call.live && !self.call.live;
                let ended = !call.live && self.call.live;
                if let Some(e) = call.error.clone().filter(|_| !call.live) {
                    self.toast(e.chars().take(80).collect::<String>());
                }
                let keep_error = !call.live && call.error.is_some();
                self.call = call;
                if started && self.page != Page::Call {
                    self.unfold(Page::Call, cx);
                }
                if ended {
                    self.call_typing = false;
                }
                if keep_error {
                    self.unfold(Page::Call, cx);
                }
                self.retarget(cx);
                cx.notify();
            }
            ToIsland::View { view } => {
                let id = view.id;
                let answers = self.asking.as_ref().is_some_and(|a| a.id == id);
                let from = view.kind.map(|k| k.label()).unwrap_or("K22");
                let title = view.title.clone();
                self.remember_view(view);
                if answers {
                    self.asking = None;
                }
                if answers || self.call.live || self.page == Page::View(id) {
                    self.unfold(Page::View(id), cx);
                } else {
                    self.flash(FlashKind::Done, format!("{from} · {title}"), None, None);
                }
                cx.notify();
            }
            ToIsland::Asked { id, result } => {
                if let Some(a) = self.asking.as_mut().filter(|a| a.id == id) {
                    match result {
                        Ok(task) => {
                            a.task = Some(task);
                            if let Some(v) = self.views.iter_mut().find(|v| v.id == id) {
                                v.task = Some(task);
                            }
                        }
                        Err(e) => {
                            self.asking = None;
                            self.views.retain(|v| v.id != id);
                            self.toast(e.chars().take(90).collect::<String>());
                            if self.page == Page::View(id) {
                                self.open_page(Page::Tab(Tab::Home), None, cx);
                            }
                        }
                    }
                }
                cx.notify();
            }
            ToIsland::Show { what } => {
                let page = match what {
                    ShowWhat::Call => Page::Call,
                    ShowWhat::Plan { task } => Page::Plan(task),
                    ShowWhat::Agent { task } => Page::Agent(task),
                    ShowWhat::Lens { id } => Page::Lens(Lens::from_id(&id).unwrap_or(Lens::Status)),
                    ShowWhat::View { id } => Page::View(id),
                };
                self.unfold(page, cx);
            }
            ToIsland::Bye => {
                self.link = None;
                self.link_at = Instant::now();
                self.watching = None;
                if self.call.live {
                    self.call = CallInfo::default();
                }
            }
        }
    }

    /// A plan that just arrived: a banner offers to review it, once.
    fn check_plans(&mut self) {
        let fresh: Vec<_> = self
            .snap
            .plans
            .iter()
            .filter(|p| p.request_id.is_some())
            .filter(|p| !self.plans_seen.contains(&format!("{}|{:?}", p.task, p.request_id)))
            .cloned()
            .collect();
        for p in fresh {
            self.plans_seen.insert(format!("{}|{:?}", p.task, p.request_id));
            if !self.prefs.focus {
                self.flash(
                    FlashKind::Done,
                    format!("{} has a plan · {} steps · {}", p.kind.label(), p.steps.len(), p.title),
                    Some(p.kind),
                    Some(p.task),
                );
            }
        }
    }

    /// Proactive nudges (Settings › Notifications › Nudges, off by default):
    /// an agent waiting on you for five minutes, once each.
    fn check_nudges(&mut self) {
        if !self.prefs.nudges || self.prefs.focus {
            return;
        }
        let now = dove_core::store::now_ms();
        let waiting: Vec<_> = self
            .snap
            .agents
            .iter()
            .filter(|a| a.needs_you() && a.ended_ms.or(a.started_ms).is_some_and(|t| now - t > 5 * 60_000))
            .map(|a| (a.id, a.kind, a.title.clone()))
            .collect();
        for (id, kind, title) in waiting {
            let key = format!("wait|{id}");
            if self.nudged.insert(key) {
                self.flash(
                    FlashKind::Done,
                    format!("{} has waited 5 min on you · {title}", kind.label()),
                    Some(kind),
                    Some(id),
                );
            }
        }
    }

    pub(crate) fn start_timer_mins(&mut self, m: u64, cx: &mut Context<Self>) {
        self.timer.set(Duration::from_secs(m * 60));
        self.timer.toggle();
        self.toast(format!("{m}-minute timer started"));
        self.open_page(Page::Tab(Tab::Timer), None, cx);
    }

    pub(crate) fn connected(&self) -> bool {
        self.link.is_some()
    }

    /// Agents working or waiting on the user right now.
    pub(crate) fn running_agents(&self) -> usize {
        self.snap.agents.iter().filter(|a| a.live && a.active()).count()
    }

    /// Sends a command to Dove. If Dove isn't running it's launched (with its
    /// window for "open", headless otherwise) and the command waits for it.
    pub(crate) fn command(&mut self, msg: ToEngine) {
        if let Some(link) = &self.link
            && link.outbound.send(msg.clone()).is_ok()
        {
            return;
        }
        if matches!(msg, ToEngine::QuitAll) {
            return;
        }
        let window = matches!(msg, ToEngine::Open { .. } | ToEngine::OpenFlock | ToEngine::OpenSettings);
        self.queue.push(msg);
        if self.launched.is_none_or(|t| t.elapsed() > Duration::from_secs(15)) {
            self.launched = Some(Instant::now());
            crate::island::launch_dove(window);
        }
        self.link_at = Instant::now() - Duration::from_secs(2);
    }

    /// Takes a new snapshot of the agents, noticing what's new.
    fn apply_snapshot(&mut self, snap: Snapshot) -> bool {
        if snap == self.snap {
            return false;
        }
        let first = self.snap.agents.is_empty() && self.statuses.is_empty();
        for a in &snap.agents {
            let prev = self.statuses.insert(a.id, a.status);
            if first {
                continue;
            }
            match (prev, a.status) {
                (Some(TaskStatus::Running), TaskStatus::Done | TaskStatus::Idle) => {
                    self.flash(
                        FlashKind::Done,
                        format!("{} finished · {}", a.kind.label(), a.title),
                        Some(a.kind),
                        Some(a.id),
                    );
                    if self.timer_link == Some(a.id) && self.timer.running() {
                        self.timer.reset();
                        self.timer_link = None;
                    }
                }
                (Some(p), TaskStatus::Failed) if p != TaskStatus::Failed => {
                    self.flash(
                        FlashKind::Failed,
                        format!("{} failed · {}", a.kind.label(), a.title),
                        Some(a.kind),
                        Some(a.id),
                    );
                }
                _ => {}
            }
        }
        // Forget dismissals and answers of requests that are gone.
        let live: HashSet<&String> = snap.agents.iter().flat_map(|a| a.pending.iter().map(|p| &p.request_id)).collect();
        self.dismissed.retain(|r| live.contains(r));
        self.answers.retain(|r, _| live.contains(r));
        // A message the agent now shows as a turn isn't "sending" anymore.
        if let Some(f) = &snap.focus {
            self.sent.retain(|m| m.task != f.id || !f.turns.iter().any(|t| t.user.trim() == m.text.trim()));
        }
        self.sent.retain(|m| m.at.elapsed() < Duration::from_secs(600));
        self.snap = snap;
        if let Page::Agent(id) = self.page
            && !self.snap.agents.iter().any(|a| a.id == id)
        {
            self.page = Page::Tab(Tab::Agents);
        }
        true
    }

    fn refresh_reply(&mut self) {
        if let Page::Agent(id) = self.page {
            self.reply = self.snap.agents.iter().find(|a| a.id == id).map(|a| a.reply.clone()).unwrap_or_default();
        }
    }

    // ----- the clipboard and tray -------------------------------------------------------

    fn poll_clipboard(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.prefs.clipboard || platform::clipboard_concealed() {
            return false;
        }
        let Some(item) = cx.read_from_clipboard() else { return false };
        let clip = if let Some(text) = item.text().filter(|t| !t.trim().is_empty()) {
            Clip::text(text)
        } else if let Some(image) = item.entries().iter().find_map(|e| match e {
            gpui::ClipboardEntry::Image(i) => Some(i.clone()),
            _ => None,
        }) {
            Clip::image(Arc::new(image))
        } else {
            return false;
        };
        data::push_clip(&mut self.clips, clip);
        true
    }

    /// A file's shell icon, made once and kept while the window lives.
    pub(crate) fn icon_for(&mut self, path: &std::path::Path) -> Option<Arc<gpui::RenderImage>> {
        if let Some(i) = self.icons.get(path) {
            return i.clone();
        }
        let icon = win::file_icon(path).map(|img| Arc::new(gpui::RenderImage::new([image::Frame::new(img)])));
        self.icons.insert(path.to_path_buf(), icon.clone());
        icon
    }

    // ----- actions --------------------------------------------------------------------

    /// Runs `f` with the island's window (for its text fields), when there is
    /// one, after the current update.
    fn with_window(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Notch, &mut Window, &mut Context<Notch>) + 'static,
    ) {
        let Some(handle) = self.win.as_ref().map(|w| w.handle) else { return };
        let this = cx.entity();
        cx.spawn(async move |_, cx| {
            cx.update(|cx| {
                let _ = handle.update(cx, |_, window, cx| this.update(cx, |n, cx| f(n, window, cx)));
            });
        })
        .detach();
    }

    pub(crate) fn open_page(&mut self, page: Page, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if let Page::Agent(id) = page {
            self.last_agent = Some(id);
        }
        if page != self.page {
            self.page_at = Instant::now();
            self.tab_from = Some(views::tab_of(&self.page));
            // Pages without a tab remember where they were opened from.
            let to_bar = matches!(page, Page::Tab(t) if t.in_bar());
            self.back = if to_bar { None } else { Some(self.page.clone()).filter(|p| *p != page) };
            self.stick = true;
            self.unfolded.clear();
            self.picking = false;
        }
        self.menu = None;
        self.page = page.clone();
        self.query.clear();
        self.refresh_reply();
        if page == Page::Tab(Tab::Alerts) {
            self.unread_notices = 0;
        }
        let focus = match page {
            Page::Agent(_) => Some(0),
            Page::Tab(Tab::Notes) => Some(2),
            Page::NewAgent => {
                if self.new_agent.kind.is_none() {
                    self.new_agent.kind = self.snap.default_agent.or(self.snap.installed.first().copied());
                }
                if self.new_agent.project.is_none() {
                    self.new_agent.project = self.snap.default_project.or(self.snap.projects.first().map(|p| p.0));
                }
                self.new_agent.error = None;
                Some(1)
            }
            _ => None,
        };
        if let Some(which) = focus {
            self.take_focus(cx);
            let focus_input = move |n: &mut Notch, window: &mut Window, cx: &mut Context<Notch>| {
                if let Some(i) = n.inputs.clone() {
                    match which {
                        0 => i.message.update(cx, |s, cx| s.focus(window, cx)),
                        1 => i.prompt.update(cx, |s, cx| s.focus(window, cx)),
                        _ => {
                            // The note on show goes into the editor.
                            let text = n
                                .selected_note
                                .and_then(|id| n.notes.iter().find(|x| x.id == id))
                                .map(|x| x.text.clone());
                            i.note.update(cx, |s, cx| {
                                s.set_value(text.unwrap_or_default(), window, cx);
                                s.focus(window, cx);
                            });
                            n.note_dirty = None;
                        }
                    }
                }
            };
            match window {
                Some(w) => focus_input(self, w, cx),
                None => self.with_window(cx, focus_input),
            }
        }
        if self.machine.state != State::Expanded {
            if self.machine.open().is_some() {
                self.transition(cx);
            }
        } else {
            self.retarget(cx);
        }
        cx.notify();
    }

    pub(crate) fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Page::Agent(id) = self.page else { return };
        let Some(inputs) = self.inputs.clone() else { return };
        let typed = inputs.message.read(cx).value().trim().to_string();
        // A question on show takes typed text as its answer.
        if !typed.is_empty()
            && let Some(req) = self.question_of(id)
        {
            let rid = req.request_id.clone();
            inputs.message.update(cx, |s, cx| s.set_value("", window, cx));
            self.answer_pick(id, rid, typed, true, cx);
            return;
        }
        let mut text = typed;
        for p in std::mem::take(&mut self.attachments) {
            text.push_str(&format!("\n@{}", p.display()));
        }
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.sent.push(Sent { task: id, text: text.clone(), at: Instant::now() });
        self.command(ToEngine::Send { task: id, text });
        inputs.message.update(cx, |s, cx| s.set_value("", window, cx));
        self.picking = false;
        self.stick = true;
        if !self.connected() {
            self.toast("Starting Dove…");
        }
        self.retarget(cx);
        cx.notify();
    }

    /// The composer's text changed: `@` opens the tray picker.
    pub(crate) fn composer_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        let picking = text.ends_with('@') && (text.len() == 1 || text[..text.len() - 1].ends_with(char::is_whitespace));
        if picking != self.picking {
            self.picking = picking;
            cx.notify();
        }
    }

    /// Attaches a tray file (from the @ picker) to the next message.
    pub(crate) fn attach(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.attachments.contains(&path) {
            self.attachments.push(path);
        }
        self.picking = false;
        // Drop the "@" that opened the picker.
        self.with_window(cx, |n, window, cx| {
            if let Some(i) = n.inputs.clone() {
                let text = i.message.read(cx).value().to_string();
                let text = text.strip_suffix('@').unwrap_or(&text).to_string();
                i.message.update(cx, |s, cx| {
                    s.set_value(text, window, cx);
                    s.focus(window, cx);
                });
            }
        });
        cx.notify();
    }

    /// The first question request of `task` (answered in the agent view).
    pub(crate) fn question_of(&self, task: TaskId) -> Option<&bridge::Pending> {
        self.snap.agents.iter().find(|a| a.id == task)?.pending.iter().find(|p| p.question && !p.questions.is_empty())
    }

    /// Picks (or types) an answer for the question on show; the last one
    /// sends them all.
    pub(crate) fn answer_pick(
        &mut self,
        task: TaskId,
        rid: String,
        choice: String,
        typed: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(req) = self.question_of(task).filter(|r| r.request_id == rid).cloned() else { return };
        let n = req.questions.len();
        let a = self.answers.entry(rid.clone()).or_insert_with(|| QAnswer { page: 0, picks: vec![Vec::new(); n] });
        let page = a.page.min(n.saturating_sub(1));
        let multi = req.questions.get(page).is_some_and(|q| q.multi) && !typed;
        if let Some(picks) = a.picks.get_mut(page) {
            if multi {
                match picks.iter().position(|p| *p == choice) {
                    Some(i) => {
                        picks.remove(i);
                    }
                    None => picks.push(choice),
                }
                cx.notify();
                return;
            }
            *picks = vec![choice];
        }
        self.answer_next(task, rid, cx);
    }

    /// Moves to the next question, or sends the answers after the last.
    pub(crate) fn answer_next(&mut self, task: TaskId, rid: String, cx: &mut Context<Self>) {
        let Some(n) = self.question_of(task).filter(|r| r.request_id == rid).map(|r| r.questions.len()) else { return };
        let a = self.answers.entry(rid.clone()).or_insert_with(|| QAnswer { page: 0, picks: vec![Vec::new(); n] });
        if a.page + 1 < n {
            a.page += 1;
        } else {
            let answers = a.picks.clone();
            self.answers.remove(&rid);
            self.dismissed.insert(rid.clone());
            self.command(ToEngine::Answer { task, request_id: rid, answers });
            self.toast("Answered");
        }
        self.retarget(cx);
        cx.notify();
    }

    /// Allows or denies the approval on show (Ctrl+Alt+A / D from any app).
    fn approve_key(&mut self, allow: bool, cx: &mut Context<Self>) {
        let Some((agent, req)) = self.approval() else { return };
        let (id, rid, safe) = (agent.id, req.request_id.clone(), req.safe && !req.question);
        if allow {
            if safe {
                self.decide(id, rid, PermissionDecision::AllowOnce, cx);
            }
        } else {
            self.deny(id, rid, cx);
        }
    }

    pub(crate) fn decide(&mut self, task: TaskId, rid: String, decision: PermissionDecision, cx: &mut Context<Self>) {
        self.dismissed.insert(rid.clone());
        self.command(ToEngine::Decide { task, request_id: rid, decision });
        self.toast(if decision == PermissionDecision::AllowAlways { "Always allowed" } else { "Allowed" });
        self.retarget(cx);
        cx.notify();
    }

    /// Stops an agent after a short grace period (Undo cancels it).
    pub(crate) fn stop_agent(&mut self, task: TaskId, cx: &mut Context<Self>) {
        if let Some((t, _)) = self.stopping.take()
            && t != task
        {
            self.command(ToEngine::Stop { task: t });
        }
        self.stopping = Some((task, Instant::now()));
        self.retarget(cx);
        cx.notify();
    }

    /// Push-to-talk on the mic: pressed or released.
    pub(crate) fn dictate(&mut self, on: bool, cx: &mut Context<Self>) {
        if on && self.dictation_base.is_none() {
            self.dictation_base =
                Some(self.inputs.as_ref().map(|i| i.message.read(cx).value().to_string()).unwrap_or_default());
        }
        self.command(ToEngine::Dictate { on });
        cx.notify();
    }

    /// The agent the tray and notes send to: the one last opened, else the
    /// first working one.
    pub(crate) fn target_agent(&self) -> Option<TaskId> {
        self.last_agent
            .filter(|id| self.snap.agents.iter().any(|a| a.id == *id))
            .or_else(|| self.snap.agents.iter().find(|a| a.active()).map(|a| a.id))
    }

    pub(crate) fn send_to_agent(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(id) = self.target_agent() else {
            self.toast("No agent to send to");
            cx.notify();
            return;
        };
        let name = self.snap.agents.iter().find(|a| a.id == id).map(|a| a.kind.label()).unwrap_or("agent");
        self.command(ToEngine::Send { task: id, text });
        self.toast(format!("Sent to {name}"));
        cx.notify();
    }

    pub(crate) fn start_agent(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(inputs) = self.inputs.clone() else { return };
        let prompt = inputs.prompt.read(cx).value().trim().to_string();
        if prompt.is_empty() {
            return;
        }
        let (Some(kind), Some(project)) = (self.new_agent.kind, self.new_agent.project) else {
            self.new_agent.error = Some("Pick an agent and a project.".into());
            cx.notify();
            return;
        };
        let id = self.next_request;
        self.next_request += 1;
        self.new_agent.error = None;
        // The answer (`Started`) arrives over the link.
        self.command(ToEngine::Start { id, agent: kind, project, prompt, worktree: self.new_agent.worktree });
        self.toast(format!("Starting {}…", kind.label()));
        cx.notify();
    }

    /// Ctrl+Enter in a note: save now.
    pub(crate) fn save_note(&mut self, cx: &mut Context<Self>) {
        self.flush_note(cx);
        self.toast("Saved");
        cx.notify();
    }

    /// Writes the editor into the note on show (a new one on first text).
    pub(crate) fn flush_note(&mut self, cx: &mut Context<Self>) {
        self.note_dirty = None;
        let Some(inputs) = self.inputs.clone() else { return };
        let text = inputs.note.read(cx).value().trim_end().to_string();
        match self.selected_note.and_then(|id| self.notes.iter_mut().find(|n| n.id == id)) {
            Some(n) => {
                if n.text == text {
                    return;
                }
                n.text = text;
            }
            None => {
                if text.trim().is_empty() {
                    return;
                }
                let note = Note::new(text);
                self.selected_note = Some(note.id);
                self.notes.insert(0, note);
            }
        }
        self.save();
        self.retarget(cx);
        cx.notify();
    }

    /// Ticks or unticks a checklist line of the note on show, in the note
    /// and in the editor.
    pub(crate) fn toggle_check(&mut self, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_note(cx);
        let Some(n) = self.selected_note.and_then(|id| self.notes.iter_mut().find(|n| n.id == id)) else { return };
        n.toggle(line);
        let text = n.text.clone();
        if let Some(i) = self.inputs.clone() {
            i.note.update(cx, |s, cx| s.set_value(text, window, cx));
        }
        self.note_dirty = None;
        self.save();
        cx.notify();
    }

    /// Shows `id` in the editor (None: a fresh note).
    pub(crate) fn select_note(&mut self, id: Option<i64>, window: &mut Window, cx: &mut Context<Self>) {
        self.flush_note(cx);
        self.selected_note = id;
        let text = id.and_then(|id| self.notes.iter().find(|n| n.id == id)).map(|n| n.text.clone()).unwrap_or_default();
        if let Some(i) = self.inputs.clone() {
            i.note.update(cx, |s, cx| {
                s.set_value(text, window, cx);
                s.focus(window, cx);
            });
        }
        self.note_dirty = None;
        self.take_focus(cx);
        cx.notify();
    }

    pub(crate) fn delete_note(&mut self, id: i64, cx: &mut Context<Self>) {
        if let Some(i) = self.notes.iter().position(|n| n.id == id) {
            let note = self.notes.remove(i);
            self.undo = Some((Undo::Note(i, note), Instant::now()));
            if self.selected_note == Some(id) {
                self.selected_note = None;
                self.with_window(cx, |n, window, cx| {
                    if let Some(i) = n.inputs.clone() {
                        i.note.update(cx, |s, cx| s.set_value("", window, cx));
                    }
                    n.note_dirty = None;
                });
            }
            self.save();
            self.retarget(cx);
            cx.notify();
        }
    }

    pub(crate) fn clear_clips(&mut self, cx: &mut Context<Self>) {
        let removed: Vec<Clip> = self.clips.iter().filter(|c| !c.pinned).cloned().collect();
        if removed.is_empty() {
            return;
        }
        self.clips.retain(|c| c.pinned);
        self.undo = Some((Undo::Clips(removed), Instant::now()));
        self.retarget(cx);
        cx.notify();
    }

    pub(crate) fn remove_shelf(&mut self, i: usize, cx: &mut Context<Self>) {
        if i < self.shelf.len() {
            let p = self.shelf.remove(i);
            self.undo = Some((Undo::Shelf(i, p), Instant::now()));
            self.save();
            self.retarget(cx);
            cx.notify();
        }
    }

    pub(crate) fn undo_last(&mut self, cx: &mut Context<Self>) {
        if self.stopping.take().is_some() {
            self.toast("Kept running");
        } else if let Some(d) = self.deny.take() {
            self.dismissed.remove(&d.request_id);
            self.toast("Kept waiting");
        } else if let Some((undo, _)) = self.undo.take() {
            match undo {
                Undo::Note(i, note) => self.notes.insert(i.min(self.notes.len()), note),
                Undo::Clips(clips) => {
                    for c in clips.into_iter().rev() {
                        data::push_clip(&mut self.clips, c);
                    }
                }
                Undo::Shelf(i, p) => self.shelf.insert(i.min(self.shelf.len()), p),
            }
            self.save();
            self.toast("Restored");
        }
        self.retarget(cx);
        cx.notify();
    }

    /// Deny with a short grace period (Undo cancels it).
    pub(crate) fn deny(&mut self, task: TaskId, request_id: String, cx: &mut Context<Self>) {
        // A second denial sends the first at once.
        if let Some(d) = self.deny.take() {
            self.command(ToEngine::Decide {
                task: d.task,
                request_id: d.request_id,
                decision: dove_core::proto::PermissionDecision::Deny,
            });
        }
        self.deny = Some(PendingDeny { task, request_id, at: Instant::now() });
        self.toast("Denied");
        self.retarget(cx);
        cx.notify();
    }

    pub(crate) fn add_paths(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        for p in paths.paths() {
            if !self.shelf.contains(p) {
                self.shelf.insert(0, p.clone());
            }
        }
        self.shelf.truncate(60);
        self.tray_filter = TrayFilter::All;
        self.save();
        if self.machine.state == State::Expanded {
            self.page = Page::Tab(Tab::Tray);
        }
        // Offer to hand them to an agent.
        self.push_popup(Popup::Dropped { paths: paths.paths().to_vec() });
        self.retarget(cx);
        cx.notify();
    }

    /// Files dropped on an agent's row go straight to it.
    pub(crate) fn drop_on_agent(&mut self, task: TaskId, paths: &ExternalPaths, cx: &mut Context<Self>) {
        let list: Vec<String> = paths.paths().iter().map(|p| format!("@{}", p.display())).collect();
        if list.is_empty() {
            return;
        }
        let name = self.snap.agents.iter().find(|a| a.id == task).map(|a| a.kind.label()).unwrap_or("the agent");
        self.sent.push(Sent { task, text: list.join("\n"), at: Instant::now() });
        self.command(ToEngine::Send { task, text: list.join("\n") });
        self.toast(format!("Sent to {name}"));
        cx.notify();
    }

    /// Starts a new agent with `text` waiting in its prompt.
    pub(crate) fn new_agent_with(&mut self, text: String, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let set = move |n: &mut Notch, window: &mut Window, cx: &mut Context<Notch>| {
            if let Some(i) = n.inputs.clone() {
                i.prompt.update(cx, |s, cx| {
                    s.set_value(text, window, cx);
                    s.focus(window, cx);
                });
            }
        };
        match window {
            Some(w) => {
                set(self, w, cx);
                self.open_page(Page::NewAgent, Some(w), cx);
            }
            None => {
                self.with_window(cx, set);
                self.open_page(Page::NewAgent, None, cx);
            }
        }
    }

    /// The newest clip reads like an error: offer to have it fixed (once).
    fn offer_fix(&mut self) {
        let Some(text) = self.clips.first().and_then(|c| c.as_text()).map(str::to_string) else { return };
        if !data::looks_like_error(&text) {
            return;
        }
        use std::hash::{Hash as _, Hasher as _};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        let key = h.finish();
        if self.offered_error == Some(key) {
            return;
        }
        self.offered_error = Some(key);
        self.popups.retain(|(p, _)| !matches!(p, Popup::FixError { .. }));
        self.push_popup(Popup::FixError { text });
    }

    /// Hands copied error text to the agent you're working with, or to a
    /// new one.
    pub(crate) fn fix_error(&mut self, text: String, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let prompt = format!("Fix this error:\n\n```\n{}\n```", text.trim());
        if let Some(id) = self.target_agent() {
            self.sent.push(Sent { task: id, text: prompt.clone(), at: Instant::now() });
            self.command(ToEngine::Send { task: id, text: prompt });
            self.open_page(Page::Agent(id), window, cx);
        } else {
            self.new_agent_with(prompt, window, cx);
        }
    }

    /// A plan passed 80 % of a limit: say so once per limit window.
    fn check_usage(&mut self) {
        for m in self.snap.limits.clone() {
            if m.used < 80.0 {
                continue;
            }
            let key = format!("{}|{}|{}", m.kind.id(), m.label, m.resets_at.unwrap_or(0));
            let shown = self.popups.iter().any(|(p, _)| matches!(p, Popup::Usage { key: k, .. } if *k == key));
            if !shown && !self.usage_dismissed.contains(&key) {
                self.push_popup(Popup::Usage { key, meter: m });
            }
        }
    }

    /// A pull request's checks finished: say how they went.
    fn check_prs(&mut self) {
        let first = self.pr_checks.is_empty();
        for pr in self.snap.prs.clone() {
            let prev = self.pr_checks.insert((pr.task, pr.number), pr.checks.clone());
            if !first && prev.as_deref() == Some("pending") && matches!(pr.checks.as_str(), "passing" | "failing") {
                self.push_popup(Popup::Checks { pr });
            }
        }
    }

    /// Captures the window that was in front and pops it up to mark up.
    pub(crate) fn capture(&mut self, cx: &mut Context<Self>) {
        let skip = self.win.as_ref().and_then(|w| w.native).map(|h| h.raw());
        let dir = self.store.root().join("island").join("shots");
        let shot = win::capture_front(&dir, skip).and_then(|(path, title)| {
            let img = image::open(&path).ok()?.to_rgba8();
            let size = img.dimensions();
            let mut bgra = img;
            for px in bgra.pixels_mut() {
                px.0.swap(0, 2);
            }
            let image = Arc::new(gpui::RenderImage::new([image::Frame::new(bgra)]));
            Some(Shot { path, image, size, title, marks: Vec::new(), tool: ShotTool::Arrow, drawing: None })
        });
        match shot {
            Some(shot) => {
                self.popups.retain(|(p, _)| !matches!(p, Popup::Shot(_)));
                self.push_popup(Popup::Shot(shot));
                if self.machine.state == State::Expanded && self.machine.close(self.rest(), true).is_some() {
                    self.transition(cx);
                }
                self.retarget(cx);
            }
            None => self.toast("Nothing to capture"),
        }
        cx.notify();
    }

    /// The marked-up shot on show: burn the marks in, then send it to an
    /// agent or keep it in the tray.
    pub(crate) fn finish_shot(&mut self, send: bool, cx: &mut Context<Self>) {
        let Some(i) = self.popups.iter().position(|(p, _)| matches!(p, Popup::Shot(_))) else { return };
        let (Popup::Shot(shot), _) = self.popups.remove(i) else { return };
        if !shot.marks.is_empty()
            && let Ok(img) = image::open(&shot.path)
        {
            let mut img = img.to_rgba8();
            burn_marks(&mut img, &shot.marks);
            let _ = img.save(&shot.path);
        }
        self.shelf.insert(0, shot.path.clone());
        self.save();
        if send {
            self.send_to_agent(format!("Here's a screenshot of \"{}\":\n@{}", shot.title, shot.path.display()), cx);
        } else {
            self.toast("Kept in the tray");
        }
        self.retarget(cx);
        cx.notify();
    }

    /// Copies a tray item and pastes it into the app the island came from.
    pub(crate) fn paste_back(&mut self, text: Option<String>, image: Option<Arc<gpui::Image>>, cx: &mut Context<Self>) {
        let item = match (text, image) {
            (Some(t), _) => gpui::ClipboardItem::new_string(t),
            (None, Some(i)) => gpui::ClipboardItem::new_image(&i),
            _ => return,
        };
        cx.write_to_clipboard(item);
        let prev = self.prev_focus.take();
        if self.machine.close(self.rest(), self.alerting()).is_some() {
            self.transition(cx);
        }
        win::paste_into(prev);
    }

    pub(crate) fn hide_for_an_hour(&mut self, cx: &mut Context<Self>) {
        self.hide_until = Some(Instant::now() + Duration::from_secs(3600));
        self.context_menu = false;
        if self.machine.close(State::Hidden, false).is_some() {
            self.transition(cx);
        }
        cx.notify();
    }

    /// Runs a menu item.
    pub(crate) fn run_menu(&mut self, action: MenuAction, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        match action {
            MenuAction::Pin => {
                let pinned = !self.machine.pinned;
                self.machine.set_pinned(pinned);
                self.toast(if pinned { "Pinned open" } else { "Unpinned" });
            }
            MenuAction::Settings => self.command(ToEngine::OpenSettings),
            MenuAction::HideHour => self.hide_for_an_hour(cx),
            MenuAction::AgentMessage(id) => self.open_page(Page::Agent(id), Some(window), cx),
            MenuAction::AgentStop(id) => self.stop_agent(id, cx),
            MenuAction::AgentOpen(id) => self.command(ToEngine::Open { task: Some(id) }),
            MenuAction::AllowAlways(task, rid) => self.decide(task, rid, PermissionDecision::AllowAlways, cx),
            MenuAction::AllowAll(task, rids) => {
                let n = rids.len();
                for rid in rids {
                    self.dismissed.insert(rid.clone());
                    self.command(ToEngine::Decide { task, request_id: rid, decision: PermissionDecision::AllowOnce });
                }
                self.toast(format!("Allowed {n}"));
            }
            MenuAction::Filter(f) => {
                self.tray_filter = f;
                self.tray_selected = None;
            }
            MenuAction::Capture => self.capture(cx),
            MenuAction::ClearClips => self.clear_clips(cx),
            MenuAction::TileSend(i) => {
                if let Some(t) = self.clips.get(i).and_then(|c| c.as_text()).map(str::to_string) {
                    self.send_to_agent(t, cx);
                }
            }
            MenuAction::TileFix(i) => {
                if let Some(t) = self.clips.get(i).and_then(|c| c.as_text()).map(str::to_string) {
                    self.fix_error(t, Some(window), cx);
                }
            }
            MenuAction::ClipPin(i) => {
                if let Some(c) = self.clips.get_mut(i) {
                    c.pinned = !c.pinned;
                }
                self.save();
            }
            MenuAction::ShelfCopy(i) => {
                if let Some(p) = self.shelf.get(i) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(p.display().to_string()));
                    self.toast("Path copied");
                }
            }
            MenuAction::ShelfRemove(i) => self.remove_shelf(i, cx),
            MenuAction::ShelfStart(i) => {
                if let Some(p) = self.shelf.get(i).cloned() {
                    match self.target_agent() {
                        Some(id) => {
                            let text = format!("@{}", p.display());
                            self.sent.push(Sent { task: id, text: text.clone(), at: Instant::now() });
                            self.command(ToEngine::Send { task: id, text });
                            self.toast("Sent");
                        }
                        None => self.new_agent_with(format!("@{} ", p.display()), Some(window), cx),
                    }
                }
            }
            MenuAction::NotePin(id) => {
                for n in &mut self.notes {
                    n.pinned = n.id == id && !n.pinned;
                }
                self.save();
            }
            MenuAction::NoteDelete(id) => self.delete_note(id, cx),
            MenuAction::NoteSend(id) => {
                self.flush_note(cx);
                if let Some(t) = self.notes.iter().find(|n| n.id == id).map(|n| n.text.clone()) {
                    match self.target_agent() {
                        Some(_) => self.send_to_agent(t, cx),
                        None => self.new_agent_with(t, Some(window), cx),
                    }
                }
            }
            MenuAction::LinkTimer(id) => {
                self.timer_link = id;
                if id.is_some() {
                    self.toast("Ends when the agent finishes");
                }
            }
            MenuAction::ResetTimer => {
                self.timer.reset();
                self.timer_link = None;
            }
            MenuAction::ClearNotices => {
                self.notices.clear();
                self.unread_notices = 0;
            }
            MenuAction::PortCopy(port) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(format!("http://localhost:{port}")));
                self.toast("URL copied");
            }
            MenuAction::PortStop(pid) => {
                if win::stop_process(pid) {
                    self.ports.retain(|p| p.pid != pid);
                    self.toast("Stopped");
                } else {
                    self.toast("Couldn't stop it");
                }
            }
            MenuAction::NewKind(k) => self.new_agent.kind = Some(k),
            MenuAction::NewProject(p) => self.new_agent.project = Some(p),
            MenuAction::NewWorktree(on) => self.new_agent.worktree = on,
            MenuAction::Merge(task, number) => {
                self.command(ToEngine::Merge { task, number });
                self.toast("Merging…");
            }
            MenuAction::OpenUrl(url) => cx.open_url(&url),
            MenuAction::VoiceAgent(kind) => {
                self.snap.voice.agent = Some(kind);
                self.snap.voice.model = None;
                self.command(ToEngine::SetVoiceAgent { agent: Some(kind), model: None });
                self.toast(format!("K22 runs on {}", kind.label()));
            }
            MenuAction::VoiceModel(model) => {
                self.snap.voice.model = model.clone();
                self.command(ToEngine::SetVoiceAgent { agent: self.snap.voice.agent, model });
                self.toast("Model set");
            }
            MenuAction::Talk => self.talk(None, cx),
            MenuAction::TalkAbout(id) => self.talk(Some(id), cx),
            MenuAction::Focus => {
                let on = !self.prefs.focus;
                self.set_focus_mode(on, cx);
            }
            MenuAction::Lens(l) => self.open_page(Page::Lens(l), Some(window), cx),
            MenuAction::OpenView(id) => self.open_page(Page::View(id), Some(window), cx),
            MenuAction::Glass => {
                let on = !self.prefs.glass;
                self.prefs.glass = on;
                let _ = self.store.set_setting("notch_glass", if on { "true" } else { "false" });
                self.toast(if on { "Frosted glass" } else { "Black island" });
            }
            MenuAction::Nudges => {
                let on = !self.prefs.nudges;
                self.prefs.nudges = on;
                let _ = self.store.set_setting("notch_nudges", if on { "true" } else { "false" });
                self.toast(if on { "Nudges on" } else { "Nudges off" });
            }
        }
        self.retarget(cx);
        cx.notify();
    }

    /// Opening from the slim island: the art or the timer ring flies from
    /// the capsule to its place in the page.
    fn hero_for_open(&self) -> Option<Hero> {
        if crate::motion::reduced_now() {
            return None;
        }
        let first = self.slim_items().into_iter().next()?;
        let (slim_w, _) = self.slim_size();
        let slim_x = (tk::CANVAS_W - slim_w) / 2.0;
        let kind = match (&first, &self.page) {
            (views::SlimItem::Music, Page::Tab(Tab::Music)) if self.media.as_ref().is_some_and(|m| m.art.is_some()) => {
                HeroKind::Art
            }
            (views::SlimItem::Timer, Page::Tab(Tab::Timer)) => HeroKind::Ring,
            _ => return None,
        };
        let (from, to) = match kind {
            HeroKind::Art => ((slim_x + 8.0, tk::TOP_GAP + 7.0, 22.0), views::music_art_rect()),
            HeroKind::Ring => ((slim_x + 8.0, tk::TOP_GAP + 6.0, 24.0), views::timer_ring_rect()),
        };
        Some(Hero { kind, from, to, at: Instant::now() })
    }

    pub(crate) fn set_focus_mode(&mut self, on: bool, cx: &mut Context<Self>) {
        self.prefs.focus = on;
        let _ = self.store.set_setting("notch_focus", if on { "true" } else { "false" });
        self.toast(if on { "Focus on: only what needs you" } else { "Focus off" });
        cx.notify();
    }

    /// The command bar's results for the current query.
    pub(crate) fn hits(&self) -> Vec<(Hit, String, String)> {
        let q = self.query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<(Hit, String, String)> = Vec::new();
        let raw = self.query.trim();
        // "? question" or "ask question": answered here by the default agent.
        let ask = raw.strip_prefix('?').or_else(|| raw.strip_prefix("ask ")).map(str::trim).filter(|r| !r.is_empty());
        if let Some(question) = ask {
            out.push((Hit::Ask(question.to_string()), format!("Ask K22: {question}"), "K22".into()));
        }
        // Talk, the lenses, views, plans.
        if ["call", "talk", "voice", "k22"].iter().any(|w| q.starts_with(w)) {
            let label = if self.call.live { "End the call with K22" } else { "Talk to K22" };
            out.push((Hit::Call, label.into(), "Voice".into()));
        }
        if let Some(lens) = views::v3::Lens::for_question(&q) {
            out.push((Hit::Lens(lens), format!("Show {}", lens.title().to_lowercase()), "Lens".into()));
        }
        for l in [Lens::Status, Lens::Needs, Lens::Today] {
            if q.len() >= 3 && l.title().to_lowercase().starts_with(&q) && !out.iter().any(|h| h.0 == Hit::Lens(l)) {
                out.push((Hit::Lens(l), format!("Show {}", l.title().to_lowercase()), "Lens".into()));
            }
        }
        if q.starts_with("plan") {
            for p in self.snap.plans.iter().take(3) {
                out.push((Hit::Plan(p.task), format!("Review {}'s plan · {}", p.kind.label(), p.title), "Plan".into()));
            }
        }
        for v in self.views.iter().filter(|v| !v.pending && v.title.to_lowercase().contains(&q)).take(3) {
            out.push((Hit::View(v.id), v.title.clone(), "View".into()));
        }
        // Actions: "timer 25", "note …", "new …", "capture", "focus", "pin",
        // "stop …", "open …", "ports", "clear clips", "shuffle", "run …".
        if let Some(rest) = q.strip_prefix("timer") {
            let mins = rest.trim().trim_end_matches('m').parse::<u64>().unwrap_or(25).clamp(1, 600);
            out.push((Hit::Timer(mins), format!("Start a {mins}-minute timer"), "Timer".into()));
            if let Some(a) = self.snap.agents.iter().find(|a| a.status == TaskStatus::Running) {
                out.push((Hit::LinkTimer(a.id), format!("Timer until “{}” finishes", a.title), "Timer".into()));
            }
        }
        if q == "pin" || q == "unpin" {
            let label = if self.machine.pinned { "Unpin the island" } else { "Pin the island open" };
            out.push((Hit::Pin, label.into(), "Island".into()));
        }
        if "clear clips clipboard".starts_with(q.as_str()) && q.len() >= 3 && q.starts_with("clear") {
            out.push((Hit::ClearClips, "Clear the clipboard history (pinned stay)".into(), "Tray".into()));
        }
        if q.starts_with("shuf") && self.media.as_ref().is_some_and(|m| m.shuffle.is_some()) {
            out.push((Hit::Shuffle, "Toggle shuffle".into(), "Music".into()));
        }
        for (verb, stop) in [("stop ", true), ("open ", false)] {
            if let Some(name) = q.strip_prefix(verb).map(str::trim).filter(|n| !n.is_empty()) {
                for a in
                    self.snap.agents.iter().filter(|a| {
                        a.title.to_lowercase().contains(name) || a.kind.label().to_lowercase().contains(name)
                    })
                {
                    if stop && a.status == TaskStatus::Running {
                        out.push((Hit::Stop(a.id), format!("Stop “{}”", a.title), "Agents".into()));
                    } else if !stop {
                        out.push((Hit::OpenInDove(a.id), format!("Open “{}” in Keplorayx", a.title), "Agents".into()));
                    }
                }
            }
        }
        if q.starts_with("port") || q.starts_with("localhost") || q.starts_with("dev") || q.starts_with("server") {
            out.push((Hit::Tab(Tab::Dev), "Dev servers".into(), "Go to".into()));
        }
        if let Some(name) = q.strip_prefix("run ").map(str::trim).filter(|n| !n.is_empty()) {
            for a in self.snap.automations.iter().filter(|a| a.name.to_lowercase().contains(name)) {
                out.push((Hit::RunAutomation(a.id.clone()), format!("Run “{}” now", a.name), "Automation".into()));
            }
        }
        if q.starts_with("merge") {
            for pr in self.snap.prs.iter().filter(|p| p.state == "open" && p.mergeable) {
                out.push((
                    Hit::Merge(pr.task, pr.number),
                    format!("Merge #{} {}", pr.number, pr.title),
                    "Pull request".into(),
                ));
            }
        }
        if let Some(rest) = self.query.trim().strip_prefix("note ").filter(|r| !r.trim().is_empty()) {
            out.push((Hit::NewNote(rest.trim().to_string()), format!("Save note “{}”", rest.trim()), "Notes".into()));
        }
        let new_prompt = self.query.trim().strip_prefix("new ").map(str::trim).filter(|r| !r.is_empty());
        if let Some(p) = new_prompt {
            out.push((Hit::NewAgent(p.to_string()), format!("Start an agent: {p}"), "Agents".into()));
        }
        if "capture screenshot window".contains(q.as_str()) || q.starts_with("capture") {
            out.push((Hit::Capture, "Capture the window behind into the tray".into(), "Tray".into()));
        }
        if "focus mode".contains(q.as_str()) || q.starts_with("focus") {
            let label = if self.prefs.focus { "Turn focus off" } else { "Turn focus on" };
            out.push((Hit::Focus, label.into(), "Focus".into()));
        }
        for t in Tab::ALL {
            if t.label().to_lowercase().contains(&q) && !out.iter().any(|(h, _, _)| *h == Hit::Tab(t)) {
                out.push((Hit::Tab(t), format!("Open {}", t.label()), "Go to".into()));
            }
        }
        for a in &self.snap.agents {
            if a.title.to_lowercase().contains(&q) || a.kind.label().to_lowercase().contains(&q) {
                out.push((Hit::Agent(a.id), a.title.clone(), a.kind.label().to_string()));
            }
        }
        for n in &self.notes {
            if n.text.to_lowercase().contains(&q) {
                out.push((Hit::Note(n.id), n.title(), "Note".into()));
            }
        }
        for (i, c) in self.clips.iter().enumerate() {
            if let Some(t) = c.as_text()
                && t.to_lowercase().contains(&q)
            {
                out.push((Hit::Clip(i), t.lines().next().unwrap_or("").chars().take(80).collect(), "Clipboard".into()));
            }
        }
        if new_prompt.is_none() && ask.is_none() && out.is_empty() {
            out.push((Hit::Ask(raw.to_string()), format!("Ask K22: {raw}"), "K22".into()));
            out.push((Hit::NewAgent(raw.to_string()), format!("Start an agent: {raw}"), "Agents".into()));
        }
        out.truncate(8);
        out
    }

    /// Runs the selected (or given) command-bar result.
    pub(crate) fn run_hit(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let hits = self.hits();
        let Some((hit, _, _)) = hits.get(index.unwrap_or(self.hit)).cloned() else { return };
        if let Some(i) = self.inputs.clone() {
            i.command.update(cx, |s, cx| s.set_value("", window, cx));
        }
        self.query.clear();
        match hit {
            Hit::Agent(id) => self.open_page(Page::Agent(id), Some(window), cx),
            Hit::Note(id) => {
                self.selected_note = Some(id);
                self.open_page(Page::Tab(Tab::Notes), Some(window), cx);
            }
            Hit::Clip(i) => {
                if let Some(t) = self.clips.get(i).and_then(|c| c.as_text().map(str::to_string)) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(t));
                    self.toast("Copied");
                }
            }
            Hit::Tab(t) => self.open_page(Page::Tab(t), Some(window), cx),
            Hit::NewAgent(p) => self.new_agent_with(p, Some(window), cx),
            // K22 answers in a view (a lens when one fits).
            Hit::Ask(question) => self.ask_k22(question, cx),
            Hit::Call => self.talk(None, cx),
            Hit::Lens(l) => self.open_page(Page::Lens(l), Some(window), cx),
            Hit::View(id) => self.open_page(Page::View(id), Some(window), cx),
            Hit::Plan(task) => self.open_page(Page::Plan(task), Some(window), cx),
            Hit::Pin => {
                let pinned = !self.machine.pinned;
                self.machine.set_pinned(pinned);
                self.toast(if pinned { "Pinned open" } else { "Unpinned" });
            }
            Hit::Stop(id) => self.stop_agent(id, cx),
            Hit::OpenInDove(id) => self.command(ToEngine::Open { task: Some(id) }),
            Hit::ClearClips => self.clear_clips(cx),
            Hit::Shuffle => {
                if let Some(on) = self.media.as_ref().and_then(|m| m.shuffle) {
                    self.media_src.send(media::Control::Shuffle(!on));
                }
            }
            Hit::LinkTimer(id) => {
                if !self.timer.running() {
                    self.timer.toggle();
                }
                self.timer_link = Some(id);
                self.toast("Ends when the agent finishes");
                self.open_page(Page::Tab(Tab::Timer), Some(window), cx);
            }
            Hit::RunAutomation(id) => {
                self.command(ToEngine::RunAutomation { id });
                self.toast("Running");
            }
            Hit::Merge(task, number) => {
                self.command(ToEngine::Merge { task, number });
                self.toast("Merging…");
            }
            Hit::Timer(m) => {
                self.timer.set(Duration::from_secs(m * 60));
                self.timer.toggle();
                self.toast(format!("{m}-minute timer started"));
                self.open_page(Page::Tab(Tab::Timer), Some(window), cx);
            }
            Hit::NewNote(text) => {
                self.notes.insert(0, Note::new(text));
                self.save();
                self.toast("Note saved");
            }
            Hit::Capture => self.capture(cx),
            Hit::Focus => {
                let on = !self.prefs.focus;
                self.set_focus_mode(on, cx);
            }
        }
        self.retarget(cx);
        cx.notify();
    }

    pub(crate) fn save(&self) {
        data::Saved::store(&self.store, &self.clips, &self.shelf, &self.notes);
    }
}

/// Draws a screenshot's marks into it: arrows and boxes in red, highlights
/// as a translucent yellow wash.
fn burn_marks(img: &mut image::RgbaImage, marks: &[Mark]) {
    let (w, h) = img.dimensions();
    let px = |f: (f32, f32)| (f.0 * w as f32, f.1 * h as f32);
    let thick = (w.max(h) as f32 / 300.0).clamp(3.0, 9.0);
    const RED: [u8; 4] = [255, 59, 48, 255];
    let dot = |img: &mut image::RgbaImage, x: f32, y: f32| {
        let r = thick / 2.0;
        for dy in -(r as i32)..=(r as i32) {
            for dx in -(r as i32)..=(r as i32) {
                let (cx, cy) = (x as i32 + dx, y as i32 + dy);
                if (dx * dx + dy * dy) as f32 <= r * r && cx >= 0 && cy >= 0 && (cx as u32) < w && (cy as u32) < h {
                    img.put_pixel(cx as u32, cy as u32, image::Rgba(RED));
                }
            }
        }
    };
    let line = |img: &mut image::RgbaImage, a: (f32, f32), b: (f32, f32)| {
        let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()) / (thick / 3.0)).ceil().max(1.0) as usize;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            dot(img, a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        }
    };
    for m in marks {
        let (a, b) = (px(m.from), px(m.to));
        match m.tool {
            ShotTool::Arrow => {
                line(img, a, b);
                let ang = (b.1 - a.1).atan2(b.0 - a.0);
                let len = (thick * 5.0).max(14.0);
                for side in [-0.5f32, 0.5] {
                    let t = ang + std::f32::consts::PI + side;
                    line(img, b, (b.0 + len * t.cos(), b.1 + len * t.sin()));
                }
            }
            ShotTool::Box => {
                line(img, a, (b.0, a.1));
                line(img, (b.0, a.1), b);
                line(img, b, (a.0, b.1));
                line(img, (a.0, b.1), a);
            }
            ShotTool::Highlight => {
                let (x0, x1) = (a.0.min(b.0).max(0.0) as u32, a.0.max(b.0).min(w as f32 - 1.0) as u32);
                let (y0, y1) = (a.1.min(b.1).max(0.0) as u32, a.1.max(b.1).min(h as f32 - 1.0) as u32);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let p = img.get_pixel_mut(x, y);
                        p.0[0] = ((p.0[0] as u16 * 3 + 255) / 4) as u8;
                        p.0[1] = ((p.0[1] as u16 * 3 + 214) / 4) as u8;
                        p.0[2] = ((p.0[2] as u16 * 3) / 4) as u8;
                    }
                }
            }
        }
    }
}

/// How bright a capture (BGRA) is, 0–1, under the island's rectangle
/// `under` (or, `around`, outside it): mostly its brighter pixels, since a
/// bright patch is what makes white type hard to read.
fn luma_of(small: &image::RgbaImage, under: [f32; 4], around: bool) -> f32 {
    let mut lumas: Vec<f32> = Vec::new();
    for (x, y, p) in small.enumerate_pixels() {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let inside = fx >= under[0] && fx < under[2] && fy >= under[1] && fy < under[3];
        if inside != around {
            let [b, g, r, _] = p.0;
            lumas.push((0.0722 * b as f32 + 0.7152 * g as f32 + 0.2126 * r as f32) / 255.0);
        }
    }
    if lumas.is_empty() {
        return 0.3;
    }
    lumas.sort_by(f32::total_cmp);
    let mean = lumas.iter().sum::<f32>() / lumas.len() as f32;
    let p90 = lumas[(lumas.len() * 9 / 10).min(lumas.len() - 1)];
    0.5 * mean + 0.5 * p90
}

/// Blurs a capture (already 1/8 size; GPUI scales it back up smoothly) into
/// frosted glass, with bright backgrounds pulled down so white type stays
/// readable over them.
fn frost(small: image::RgbaImage) -> (Vec<u8>, Arc<gpui::RenderImage>) {
    let mut small = image::imageops::blur(&small, 2.6);
    for px in small.pixels_mut() {
        for c in &mut px.0[..3] {
            *c = (*c as f32 * 0.78) as u8;
        }
    }
    let key = small.as_raw().clone();
    (key, Arc::new(gpui::RenderImage::new([image::Frame::new(small)])))
}

/// Local time as HH:MM.
pub(crate) fn clock_text() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let local = secs as i64 + utc_offset_secs();
    format!("{:02}:{:02}", (local / 3600).rem_euclid(24), (local / 60).rem_euclid(60))
}

#[cfg(windows)]
fn utc_offset_secs() -> i64 {
    use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    /// `TIME_ZONE_ID_DAYLIGHT`, which windows-sys doesn't export.
    const TIME_ZONE_ID_DAYLIGHT: u32 = 2;
    // SAFETY: one out-parameter.
    unsafe {
        let mut tz: TIME_ZONE_INFORMATION = std::mem::zeroed();
        let id = GetTimeZoneInformation(&mut tz);
        let bias = tz.Bias + if id == TIME_ZONE_ID_DAYLIGHT { tz.DaylightBias } else { tz.StandardBias };
        -(bias as i64) * 60
    }
}

#[cfg(not(windows))]
fn utc_offset_secs() -> i64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display() -> Display {
        Display { left: 0, top: 0, width: 2000, height: 1300, scale: 1.25 }
    }

    fn at(cursor: Option<(i32, i32)>) -> Input {
        Input {
            display: display(),
            cursor,
            button_down: false,
            fullscreen: false,
            engaged: false,
            rest: State::Hidden,
            alert: false,
            island: (700, 9, 1300, 400),
        }
    }

    const TOP: Option<(i32, i32)> = Some((1000, 0));
    const AWAY: Option<(i32, i32)> = Some((1000, 900));

    #[test]
    fn rests_in_hot_zone_then_opens() {
        let mut m = Machine::new();
        let t = Instant::now();
        assert_eq!(m.tick(&at(TOP), t), None);
        assert_eq!(m.deadline(), Some(t + HOVER_DELAY));
        assert_eq!(m.tick(&at(TOP), t + Duration::from_millis(50)), None);
        assert_eq!(m.tick(&at(TOP), t + HOVER_DELAY), Some(State::Expanded));
    }

    #[test]
    fn heading_up_fast_opens_at_once() {
        let mut m = Machine::new();
        let t = Instant::now();
        assert_eq!(m.tick(&at(Some((1000, 160))), t), None);
        assert_eq!(m.tick(&at(TOP), t + Duration::from_millis(40)), Some(State::Expanded));
    }

    #[test]
    fn sliding_along_the_edge_still_waits() {
        let mut m = Machine::new();
        let t = Instant::now();
        assert_eq!(m.tick(&at(Some((900, 0))), t), None);
        assert_eq!(m.tick(&at(Some((1000, 0))), t + Duration::from_millis(20)), None);
    }

    #[test]
    fn marks_burn_into_the_shot() {
        let mut img = image::RgbaImage::from_pixel(200, 100, image::Rgba([0, 0, 0, 255]));
        let marks = [
            Mark { tool: ShotTool::Box, from: (0.1, 0.1), to: (0.5, 0.9) },
            Mark { tool: ShotTool::Highlight, from: (0.6, 0.1), to: (0.9, 0.5) },
        ];
        burn_marks(&mut img, &marks);
        assert_eq!(img.get_pixel(20, 50).0[0], 255, "the box's left edge is red");
        assert!(img.get_pixel(150, 30).0[0] > 50, "the highlight washes yellow");
        assert_eq!(img.get_pixel(60, 50).0[0], 0, "inside the box is untouched");
    }

    #[test]
    fn passing_through_the_edge_does_nothing() {
        let mut m = Machine::new();
        let t = Instant::now();
        m.tick(&at(TOP), t);
        m.tick(&at(AWAY), t + Duration::from_millis(100));
        assert_eq!(m.deadline(), None);
        // Coming back slowly restarts the wait (a fast flick up would be
        // intent, see `heading_up_fast_opens_at_once`).
        m.tick(&at(Some((1000, 10))), t + Duration::from_millis(150));
        assert_eq!(m.tick(&at(TOP), t + Duration::from_millis(200)), None);
    }

    #[test]
    fn corners_of_the_top_edge_do_nothing() {
        let mut m = Machine::new();
        let t = Instant::now();
        let corner = at(Some((10, 0)));
        m.tick(&corner, t);
        assert_eq!(m.tick(&corner, t + Duration::from_secs(1)), None);
    }

    #[test]
    fn a_held_button_waits_longer_and_reveals_the_tray() {
        let mut m = Machine::new();
        let t = Instant::now();
        let dragging = Input { button_down: true, ..at(TOP) };
        m.tick(&dragging, t);
        assert_eq!(m.tick(&dragging, t + HOVER_DELAY), None);
        assert_eq!(m.tick(&dragging, t + DRAG_DELAY), Some(State::Expanded));
        assert!(m.drag_reveal);
    }

    #[test]
    fn closes_after_the_delay_unless_pinned_or_typing() {
        let mut m = Machine::new();
        let t = Instant::now();
        m.go(State::Expanded);
        assert_eq!(m.tick(&at(Some((1000, 100))), t), None);
        assert_eq!(m.tick(&at(AWAY), t), None);
        assert_eq!(m.deadline(), Some(t + LEAVE_DELAY));
        assert_eq!(m.tick(&at(AWAY), t + LEAVE_DELAY), Some(State::Hidden));

        m.go(State::Expanded);
        let typing = Input { engaged: true, ..at(AWAY) };
        assert_eq!(m.tick(&typing, t + Duration::from_secs(10)), None);

        m.set_pinned(true);
        assert_eq!(m.tick(&at(AWAY), t + Duration::from_secs(10)), None);
    }

    #[test]
    fn rests_slim_and_opens_on_hover() {
        let mut m = Machine::new();
        let t = Instant::now();
        let slim = Input { rest: State::Slim, ..at(AWAY) };
        assert_eq!(m.tick(&slim, t), Some(State::Slim));
        let over = Input { rest: State::Slim, ..at(Some((1000, 20))) };
        m.tick(&over, t);
        assert_eq!(m.tick(&over, t + HOVER_DELAY), Some(State::Expanded));
    }

    #[test]
    fn alerts_pop_up_and_stay_until_handled() {
        let mut m = Machine::new();
        let t = Instant::now();
        let alert = Input { alert: true, ..at(AWAY) };
        assert_eq!(m.tick(&alert, t), Some(State::Alert));
        // Hovering a banner doesn't open it: its buttons are clicked.
        let over = Input { alert: true, ..at(Some((1000, 20))) };
        m.tick(&over, t);
        assert_eq!(m.tick(&over, t + Duration::from_secs(1)), None);
        assert_eq!(m.tick(&at(AWAY), t + Duration::from_secs(2)), Some(State::Hidden));
    }

    #[test]
    fn full_screen_hides_everything() {
        let mut m = Machine::new();
        m.go(State::Expanded);
        let full = Input { fullscreen: true, alert: true, rest: State::Slim, ..at(TOP) };
        assert_eq!(m.tick(&full, Instant::now()), Some(State::Hidden));
        assert_eq!(m.tick(&full, Instant::now() + Duration::from_secs(1)), None);
    }

    #[test]
    fn a_bright_patch_under_the_island_reads_bright() {
        // Dark all round, white under the island's rectangle.
        let img = image::RgbaImage::from_fn(20, 10, |x, y| {
            if (5..15).contains(&x) && (2..8).contains(&y) {
                image::Rgba([255, 255, 255, 255])
            } else {
                image::Rgba([10, 10, 10, 255])
            }
        });
        let under = [5.0, 2.0, 15.0, 8.0];
        assert!(luma_of(&img, under, false) > 0.95);
        assert!(luma_of(&img, under, true) < 0.1);
    }

    #[test]
    fn shapes_place_bubbles_after_the_island() {
        let g = Geo { y: 8.0, w: 200.0, h: 36.0, r: 18.0, dx: 0.0, b1: 36.0, b2: 36.0 };
        let s = g.shapes();
        assert_eq!(s.len(), 3);
        assert_eq!(s[1].x, g.x() + 200.0 + tk::BUBBLE_GAP);
        assert_eq!(s[2].x, s[1].x + 36.0 + tk::BUBBLE_GAP);
        assert_eq!(g.span(), 200.0 + 2.0 * (36.0 + tk::BUBBLE_GAP));
    }

    #[test]
    fn tabs_round_trip_their_ids() {
        for t in Tab::ALL {
            assert_eq!(Tab::from_id(t.id()), Some(t));
        }
        assert_eq!(Tab::from_id("clipboard"), Some(Tab::Tray));
        assert!(Tab::Alerts.in_bar() && !Tab::Notes.in_bar() && !Tab::Timer.in_bar() && !Tab::Dev.in_bar());
    }
}
