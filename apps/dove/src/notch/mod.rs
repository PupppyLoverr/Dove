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
//! - **Expanded**: Home cards, Agents, Music, Tray (clipboard and shelf),
//!   Notes, Timer, Notifications and Dev servers, with a command bar.
//!
//! Nothing polls. The loop sleeps until a platform event (pointer, clipboard,
//! foreground, hotkey, display), a message from the engine, media, audio, or
//! its own next deadline (a hover delay, a banner's end, the timer).
//!
//! QA hooks: `DOVE_NOTCH_STATE=slim|expanded|alert`,
//! `DOVE_NOTCH_PAGE=home|agents|music|tray|notes|timer|alerts|dev|new`,
//! `DOVE_NOTCH_DEMO=1`, `DOVE_NOTCH_CAPTURABLE=1` (screenshots can see it),
//! `DOVE_NOTCH_CAPTURED_GLASS=1` (skip composition glass).

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

use dove_core::proto::{HarnessKind, ProjectId, TaskId, TaskStatus};
use dove_core::store::Store;
use gpui::{
    App, AppContext as _, Context, Entity, ExternalPaths, Render, Subscription, Window, WindowBackgroundAppearance,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, point, px, size,
};
use gpui_base::input::{InputEvent, InputState, TextareaState};

use crate::island::ipc::{self, ToEngine, ToIsland};
pub use bridge::{NotchHost, Snapshot};
use data::{Clip, Note, Timer, Undo};
use platform::{Display, Shape};
use spring::Springs;
use tokens as tk;

/// The pointer must rest this long before the island opens, so passing
/// through the top edge (to a browser tab, say) doesn't open it.
const HOVER_DELAY: Duration = Duration::from_millis(160);
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
const CLOSE_AFTER: Duration = Duration::from_secs(10);
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
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine {
    pub fn new() -> Self {
        Self { state: State::Hidden, pinned: false, drag_reveal: false, hot_since: None, away_since: None }
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
        Geo { y: a[0], w: a[1].max(1.0), h: a[2].max(1.0), r: a[3].max(0.0), dx: a[4], b1: a[5].max(0.0), b2: a[6].max(0.0) }
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
            glass: store.flag("notch_glass", true),
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

    pub fn tabs(&self) -> Vec<Tab> {
        Tab::ALL.into_iter().filter(|t| !self.hidden_tabs.contains(t)).collect()
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    Timer(u64),
    NewNote(String),
    Capture,
    Focus,
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
    pub message: Entity<InputState>,
    pub prompt: Entity<InputState>,
    pub note: Entity<TextareaState>,
    pub command: Entity<InputState>,
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
    context_menu: bool,

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
    editing_note: bool,
    timer: Timer,
    /// "Until <agent> finishes": the timer also ends when this agent does.
    timer_link: Option<TaskId>,
    new_agent: NewAgent,
    tray_filter: TrayFilter,
    tray_selected: Option<usize>,
    ports: Vec<win::Port>,
    ports_at: Option<Instant>,
    query: String,
    hit: usize,
    /// The hotkey opened the island: focus the command bar when it draws.
    focus_command: bool,
    hovered_tab: Option<Tab>,
    hovered_row: Option<TaskId>,
    /// A tray file pressed for a drag out: its index and where the press was.
    drag_from: Option<(usize, gpui::Point<gpui::Pixels>)>,
    /// Dragging the music scrubber, as a fraction.
    scrub: Option<f32>,
    /// The scrubber's left edge and width on screen, from its last paint.
    scrub_bounds: Option<(f32, f32)>,
    page_at: Instant,
    /// The tab the header's puck slides from.
    tab_from: Option<Tab>,

    /// The window that had the keyboard before the island took it.
    prev_focus: Option<isize>,
    /// The captured, blurred screen behind the canvas (when composition
    /// glass isn't available), and its bookkeeping.
    backdrop: Option<Arc<gpui::RenderImage>>,
    backdrop_key: Vec<u8>,
    backdrop_at: Option<Instant>,
    retired: Vec<Arc<gpui::RenderImage>>,
    capturing: bool,

    /// The line to Dove's engine (`None` while Dove isn't running), what
    /// waits to go over it, and when it was last tried.
    link: Option<ipc::Link>,
    queue: Vec<ToEngine>,
    link_at: Instant,
    launched: Option<Instant>,
    next_request: u64,
    msgs: flume::Sender<Msg>,
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
        let message = field("Message this agent…", window, cx);
        let prompt = field("What should it do?", window, cx);
        let command = field("Search or type a command…", window, cx);
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
            cx.subscribe_in(&message, window, |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    this.notch.update(cx, |n, cx| n.send_message(window, cx));
                }
            }),
            cx.subscribe_in(&prompt, window, |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    this.notch.update(cx, |n, cx| n.start_agent(window, cx));
                }
            }),
            cx.subscribe_in(&note, window, |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { secondary: true, .. } = ev {
                    this.notch.update(cx, |n, cx| n.save_note(window, cx));
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
        let inputs = Inputs { message, prompt, note, command };
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
    cx.spawn(async move |cx| {
        let started = Instant::now();
        let mut demoed = false;
        loop {
            let wait = cx.update(|cx| notch.read(cx).next_wake(Instant::now()));
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
                    n.step(cx);
                })
            });
        }
    })
    .detach();
}

impl Notch {
    fn new(store: Arc<Store>, rt: tokio::runtime::Handle, msgs: flume::Sender<Msg>) -> Self {
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
            springs: Springs::new(Geo::HIDDEN.to_array(), tk::SPRING_STIFFNESS, tk::SPRING_DAMPING),
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
            editing_note: false,
            timer: Timer::new(Duration::from_secs(25 * 60)),
            timer_link: None,
            new_agent: NewAgent::default(),
            tray_filter: TrayFilter::All,
            tray_selected: None,
            ports: Vec::new(),
            ports_at: None,
            query: String::new(),
            hit: 0,
            focus_command: false,
            hovered_tab: None,
            hovered_row: None,
            drag_from: None,
            scrub: None,
            scrub_bounds: None,
            page_at: Instant::now(),
            tab_from: None,
            prev_focus: None,
            backdrop: None,
            backdrop_key: Vec::new(),
            backdrop_at: None,
            retired: Vec::new(),
            capturing: false,
            link,
            queue: Vec::new(),
            link_at: Instant::now(),
            launched: None,
            next_request: 1,
            msgs,
            store,
            rt,
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
            at.extend(self.hidden_since.filter(|_| self.win.is_some() || !self.trimmed).map(|t| t + CLOSE_AFTER));
        }
        if self.shown && self.animating() {
            at.push(now + Duration::from_millis(100));
        }
        if self.shown && self.uses_captured_glass() {
            let every = if self.machine.state == State::Slim { 2000 } else { 600 };
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
                    cx.notify();
                }
            }
            Msg::Platform(win::Event::Hotkey) => self.hotkey(cx),
            Msg::Platform(win::Event::Display) => {
                self.display = None;
                self.transparency = win::transparency_on();
                self.audio.refresh();
            }
            Msg::Link(m) => self.on_link(m, cx),
            Msg::LinkGone => {
                tracing::info!("island: Dove went away");
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
        if let Some(d) = self.deny.take_if(|d| now >= d.at + DENY_GRACE) {
            self.command(ToEngine::Decide {
                task: d.task,
                request_id: d.request_id,
                decision: dove_core::proto::PermissionDecision::Deny,
            });
            dirty = true;
        }
        if self.timer.finished() {
            self.timer.reset();
            self.timer_link = None;
            self.flash(FlashKind::Timer, "Time's up".into(), None, None);
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
        if self.shown
            && self.page == Page::Tab(Tab::Dev)
            && self.machine.state == State::Expanded
            && self.ports_at.is_none_or(|t| now >= t + Duration::from_secs(3))
        {
            self.ports_at = Some(now);
            let ports = win::listening_ports();
            if ports != self.ports {
                self.ports = ports;
                dirty = true;
            }
        }
        if self.shown
            && self.uses_captured_glass()
            && self.backdrop_at.is_none_or(|t| {
                let every = if self.machine.state == State::Slim { 2000 } else { 600 };
                now >= t + Duration::from_millis(every)
            })
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
            let engaged = self.win.as_ref().and_then(|w| w.native).is_some_and(|h| h.is_foreground());
            let input = Input {
                display,
                cursor: platform::cursor(),
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
        if !self.shown && self.hidden_since.is_some_and(|t| now >= t + CLOSE_AFTER) {
            if self.win.is_some() {
                self.drop_window(cx);
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
        self.approval().is_some() || flash
    }

    /// The first approval nobody has dismissed (or is denying).
    fn approval(&self) -> Option<(&bridge::Agent, &bridge::Pending)> {
        self.snap.agents.iter().flat_map(|a| a.pending.iter().map(move |p| (a, p))).find(|(_, p)| {
            !self.dismissed.contains(&p.request_id) && self.deny.as_ref().is_none_or(|d| d.request_id != p.request_id)
        })
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
                let h = if self.approval().is_some() { self.banner_height() } else { tk::FLASH_H };
                Geo {
                    y: tk::TOP_GAP,
                    w: tk::BANNER_W,
                    h,
                    r: if h > 60.0 { 28.0 } else { h / 2.0 },
                    dx: 0.0,
                    b1: 0.0,
                    b2: 0.0,
                }
            }
            State::Expanded => Geo {
                y: tk::TOP_GAP,
                w: tk::EXPANDED_W,
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
        let now = Instant::now();
        let current = Geo::from_array(self.springs.values());
        if state == State::Hidden {
            self.springs.set_targets(Geo::DOT.to_array());
            self.pending_target = None;
            self.dot_until = None;
            self.lift_at = Some(now + Duration::from_millis(220));
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
            if opening {
                self.springs.set_targets(Geo::DOT.to_array());
                self.pending_target = Some(target);
                self.dot_until = Some(now + Duration::from_millis(tk::DOT_MS));
            } else {
                self.pending_target = None;
                self.dot_until = None;
                self.springs.set_targets(target.to_array());
            }
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
        let working =
            self.snap.agents.iter().any(|a| a.status == TaskStatus::Running) || self.snap.flock.iter().any(|f| f.working);
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
        self.prefs.glass && self.transparency && self.win.as_ref().is_some_and(|w| w.glass.is_none() && w.native.is_some())
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

    fn adopt_window(&mut self, handle: WindowHandle<NotchWindow>, native: Option<platform::Handle>, cx: &mut Context<Self>) {
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
        let task = cx.background_executor().spawn(async move { platform::capture_behind(x, y, w, h, 8).map(frost) });
        cx.spawn(async move |this, cx| {
            let frame = task.await;
            let _ = this.update(cx, |n, cx| {
                n.capturing = false;
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
                    self.retarget(cx);
                    cx.notify();
                }
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
            ToIsland::Bye => {
                self.link = None;
                self.link_at = Instant::now();
            }
        }
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
                    self.flash(FlashKind::Done, format!("{} finished · {}", a.kind.label(), a.title), Some(a.kind), Some(a.id));
                    if self.timer_link == Some(a.id) && self.timer.running() {
                        self.timer.reset();
                        self.timer_link = None;
                    }
                }
                (Some(p), TaskStatus::Failed) if p != TaskStatus::Failed => {
                    self.flash(FlashKind::Failed, format!("{} failed · {}", a.kind.label(), a.title), Some(a.kind), Some(a.id));
                }
                _ => {}
            }
        }
        // Forget dismissals of requests that are gone.
        let live: HashSet<&String> = snap.agents.iter().flat_map(|a| a.pending.iter().map(|p| &p.request_id)).collect();
        self.dismissed.retain(|r| live.contains(r));
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
    fn with_window(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Notch, &mut Window, &mut Context<Notch>) + 'static) {
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
        }
        self.page = page.clone();
        self.query.clear();
        self.refresh_reply();
        if page == Page::Tab(Tab::Alerts) {
            self.unread_notices = 0;
        }
        let focus = match page {
            Page::Agent(_) => Some(0),
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
                    if which == 0 {
                        i.message.update(cx, |s, cx| s.focus(window, cx));
                    } else {
                        i.prompt.update(cx, |s, cx| s.focus(window, cx));
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
        let text = inputs.message.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.command(ToEngine::Send { task: id, text });
        inputs.message.update(cx, |s, cx| s.set_value("", window, cx));
        self.toast(if self.connected() { "Sent" } else { "Starting Dove…" });
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

    pub(crate) fn save_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(inputs) = self.inputs.clone() else { return };
        let text = inputs.note.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        match self.selected_note.filter(|_| self.editing_note) {
            Some(id) => {
                if let Some(n) = self.notes.iter_mut().find(|n| n.id == id) {
                    n.text = text;
                }
            }
            None => {
                let note = Note::new(text);
                self.selected_note = Some(note.id);
                self.notes.insert(0, note);
            }
        }
        self.editing_note = false;
        inputs.note.update(cx, |s, cx| s.set_value("", window, cx));
        self.save();
        self.toast("Saved");
        self.retarget(cx);
        cx.notify();
    }

    pub(crate) fn delete_note(&mut self, id: i64, cx: &mut Context<Self>) {
        if let Some(i) = self.notes.iter().position(|n| n.id == id) {
            let note = self.notes.remove(i);
            self.undo = Some((Undo::Note(i, note), Instant::now()));
            if self.selected_note == Some(id) {
                self.selected_note = None;
                self.editing_note = false;
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
        if let Some(d) = self.deny.take() {
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
        self.page = Page::Tab(Tab::Tray);
        self.tray_filter = TrayFilter::All;
        self.save();
        self.retarget(cx);
        cx.notify();
    }

    /// Captures the window that was in front into the tray.
    pub(crate) fn capture(&mut self, send: bool, cx: &mut Context<Self>) {
        let skip = self.win.as_ref().and_then(|w| w.native).map(|h| h.raw());
        let dir = self.store.root().join("island").join("shots");
        match win::capture_front(&dir, skip) {
            Some((path, title)) => {
                self.shelf.insert(0, path.clone());
                self.save();
                if send {
                    self.send_to_agent(format!("Here's a screenshot of \"{title}\": {}", path.display()), cx);
                } else {
                    self.toast("Captured");
                }
                self.page = Page::Tab(Tab::Tray);
                self.retarget(cx);
            }
            None => self.toast("Nothing to capture"),
        }
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
        // Actions first: "timer 25", "note …", "new …", "capture", "focus".
        if let Some(rest) = q.strip_prefix("timer") {
            let mins = rest.trim().trim_end_matches('m').parse::<u64>().unwrap_or(25).clamp(1, 600);
            out.push((Hit::Timer(mins), format!("Start a {mins}-minute timer"), "Timer".into()));
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
        for t in self.prefs.tabs() {
            if t.label().to_lowercase().contains(&q) {
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
        if new_prompt.is_none() && out.is_empty() {
            out.push((Hit::NewAgent(self.query.trim().to_string()), format!("Ask an agent: {}", self.query.trim()), "Agents".into()));
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
                self.editing_note = false;
                self.open_page(Page::Tab(Tab::Notes), Some(window), cx);
            }
            Hit::Clip(i) => {
                if let Some(t) = self.clips.get(i).and_then(|c| c.as_text().map(str::to_string)) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(t));
                    self.toast("Copied");
                }
            }
            Hit::Tab(t) => self.open_page(Page::Tab(t), Some(window), cx),
            Hit::NewAgent(p) => {
                if let Some(i) = self.inputs.clone() {
                    i.prompt.update(cx, |s, cx| s.set_value(p, window, cx));
                }
                self.open_page(Page::NewAgent, Some(window), cx);
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
            Hit::Capture => self.capture(false, cx),
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
        Display { left: 0, top: 0, width: 2000, scale: 1.25 }
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
        assert_eq!(m.tick(&at(TOP), t + Duration::from_millis(100)), None);
        assert_eq!(m.tick(&at(TOP), t + HOVER_DELAY), Some(State::Expanded));
    }

    #[test]
    fn passing_through_the_edge_does_nothing() {
        let mut m = Machine::new();
        let t = Instant::now();
        m.tick(&at(TOP), t);
        m.tick(&at(AWAY), t + Duration::from_millis(100));
        assert_eq!(m.deadline(), None);
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
    }
}
