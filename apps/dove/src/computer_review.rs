//! Dove Computer Use in the app: the approval cards a thread shows above
//! its input when its agent wants to use a desktop app, the take-over lease
//! (Take over / Hand back, and Ctrl+Alt+Shift+S on Windows), and the strip
//! that says which app an agent is using.
//!
//! The `computer` tool runs in each agent's `dove --mcp-browser` process
//! (`computer_host.rs`); it reaches this module through the loopback
//! endpoint as the `computer_review`, `computer_lease` and
//! `computer_activity` requests, routed here by the shell.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use dove_computer::{Decision, Lease, Review, Scope};
use dove_core::proto::TaskId;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::assets::{icon, icons};
use crate::browser::agent::Outcome;
use crate::computer_host::{ACTIVITY, LEASE, REVIEW};
use crate::design::ty;
use crate::kit::controls::{self as ctl, Tone};
use crate::theme::Theme;

/// How long a card waits for the user before it counts as declined.
pub const REVIEW_TIMEOUT_SECS: u64 = 300;
/// How long the strip keeps showing the last thing an agent did.
const ACTIVITY_SHOWN: Duration = Duration::from_secs(20);

pub fn is_gui_tool(name: &str) -> bool {
    matches!(name, REVIEW | LEASE | ACTIVITY)
}

// ------------------------------------------------------------- lease

static LEASE_STATE: Mutex<Lease> = Mutex::new(Lease { epoch: 0, human_control: false });

pub fn lease() -> Lease {
    *LEASE_STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Take control (`true`) or hand it back. Every change moves the epoch, so
/// a result an agent produced across it is discarded.
pub fn set_human(on: bool) -> Lease {
    let mut l = LEASE_STATE.lock().unwrap_or_else(|e| e.into_inner());
    if l.human_control != on {
        l.human_control = on;
        l.epoch += 1;
    }
    *l
}

fn toggle_control(cx: &mut App) {
    let now = set_human(!lease().human_control);
    let (title, body) = if now.human_control {
        ("You have control of the computer", "Agents using desktop apps wait until you hand it back.")
    } else {
        ("Agents may use the computer again", "Computer use resumes on their next step.")
    };
    crate::kit::Toasts::push(cx, title, Some(body.into()), crate::kit::Tone::Info, None);
    cx.refresh_windows();
}

// ------------------------------------------------------------- state

/// The cards waiting for the user, and the activity strip.
#[derive(Default)]
pub struct ComputerUse {
    cards: Vec<Entity<ReviewCard>>,
    strip: Option<Entity<Strip>>,
}

impl gpui::Global for ComputerUse {}

/// The cards thread `id` shows above its input.
pub fn cards_for(id: TaskId, cx: &App) -> Vec<Entity<ReviewCard>> {
    cx.try_global::<ComputerUse>()
        .map(|g| g.cards.iter().filter(|c| c.read(cx).thread == id).cloned().collect())
        .unwrap_or_default()
}

/// The strip for thread `id`: while its agent is using an app, or while
/// the user has control.
pub fn strip_for(id: TaskId, cx: &App) -> Option<Entity<Strip>> {
    let strip = cx.try_global::<ComputerUse>()?.strip.clone()?;
    let s = strip.read(cx);
    let recent = s.thread == Some(id) && s.at.is_some_and(|t| t.elapsed() < ACTIVITY_SHOWN);
    (recent || (lease().human_control && s.thread == Some(id))).then_some(strip)
}

/// What a request from the hub answers: now, or once the user decides.
pub enum Reply {
    Now(Outcome),
    Wait(tokio::sync::oneshot::Receiver<Decision>),
}

fn decision_text(d: &Decision) -> String {
    serde_json::to_string(d).unwrap_or_default()
}

pub fn decision_outcome(d: &Decision) -> Outcome {
    Outcome::ok(decision_text(d))
}

/// Answer one computer-use request for thread `thread` (None when Dove
/// could not tell which thread the agent belongs to).
pub fn route(tool: &str, args: &serde_json::Value, thread: Option<TaskId>, cx: &mut App) -> Reply {
    match tool {
        LEASE => Reply::Now(Outcome::ok(serde_json::to_string(&lease()).unwrap_or_default())),
        ACTIVITY => {
            let app = args.get("app").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let summary = args.get("summary").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let strip = match cx.try_global::<ComputerUse>().and_then(|g| g.strip.clone()) {
                Some(s) => s,
                None => {
                    let s = cx.new(|_| Strip::default());
                    cx.default_global::<ComputerUse>().strip = Some(s.clone());
                    s
                }
            };
            strip.update(cx, |s, cx| {
                s.thread = thread;
                s.app = app;
                s.summary = summary;
                s.at = Some(Instant::now());
                cx.notify();
            });
            cx.refresh_windows();
            Reply::Now(Outcome::ok("ok"))
        }
        REVIEW => {
            let Some(thread) = thread else {
                return Reply::Now(decision_outcome(&Decision::Unavailable {
                    reason: "Dove couldn't tell which thread this agent belongs to; start it from a Dove thread".into(),
                }));
            };
            let review: Review = match serde_json::from_value(args.clone()) {
                Ok(r) => r,
                Err(e) => return Reply::Now(Outcome::err(format!("bad review request: {e}"))),
            };
            if lease().human_control {
                return Reply::Now(decision_outcome(&Decision::Deny { reason: "the user has control of the computer".into() }));
            }
            let (tx, rx) = tokio::sync::oneshot::channel();
            let app = review.app.clone();
            let card = cx.new(|_| ReviewCard { thread, review, reply: Some(tx) });
            cx.default_global::<ComputerUse>().cards.push(card.clone());
            // Unanswered cards decline on their own.
            let weak = card.downgrade();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_secs(REVIEW_TIMEOUT_SECS)).await;
                let _ = weak.update(cx, |c, cx| {
                    c.decide(Decision::Deny { reason: "nobody answered within 5 minutes".into() }, cx)
                });
            })
            .detach();
            crate::kit::Toasts::push(
                cx,
                "An agent wants to use a desktop app",
                Some(format!("{app}: review it above the input in its thread.")),
                crate::kit::Tone::Info,
                Some(thread),
            );
            cx.refresh_windows();
            Reply::Wait(rx)
        }
        _ => Reply::Now(Outcome::err("unknown computer-use request")),
    }
}

// ------------------------------------------------------------- card

pub struct ReviewCard {
    pub thread: TaskId,
    review: Review,
    reply: Option<tokio::sync::oneshot::Sender<Decision>>,
}

pub enum ReviewEvent {
    Decided,
}

impl EventEmitter<ReviewEvent> for ReviewCard {}

impl ReviewCard {
    fn decide(&mut self, d: Decision, cx: &mut Context<Self>) {
        if let Some(tx) = self.reply.take() {
            let _ = tx.send(d);
        }
        let me = cx.entity_id();
        if let Some(g) = cx.try_global::<ComputerUse>() {
            let left: Vec<Entity<ReviewCard>> = g.cards.iter().filter(|c| c.entity_id() != me).cloned().collect();
            cx.default_global::<ComputerUse>().cards = left;
        }
        cx.emit(ReviewEvent::Decided);
        cx.refresh_windows();
    }
}

fn scope_text(scope: Scope) -> &'static str {
    match scope {
        Scope::Background => "In the background: your mouse, keyboard and focus stay yours.",
        Scope::Foreground => "Brings the window to the front briefly; the pointer may move. Best when you are not typing.",
        Scope::Consequential => "This sends, deletes, buys or confirms something, so Dove asks every time.",
    }
}

impl Render for ReviewCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(cx).clone();
        let id = cx.entity_id().as_u64() as usize;
        let r = &self.review;
        let consequential = r.scope == Scope::Consequential;
        let shown: Vec<String> = r.actions.iter().take(6).cloned().collect();
        let more = r.actions.len().saturating_sub(shown.len());
        let accent = if consequential { t.danger } else { t.accent };
        div()
            .id(("computer-review", id))
            .occlude()
            .w_full()
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .rounded(px(14.0))
            .bg(t.composer_bg())
            .border_1()
            .border_color(accent.opacity(0.35))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(icon(icons::MONITOR).size(px(14.0)).text_color(accent))
                    .child(div().text_size(ty::CAPTION).text_color(t.text_muted).child(format!(
                        "This thread wants to use {}",
                        if r.app.is_empty() { "a desktop app" } else { r.app.as_str() }
                    ))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .children(shown.into_iter().map(|a| div().text_size(ty::SMALL).text_color(t.text).truncate().child(a)))
                    .when(more > 0, |d| {
                        d.child(div().text_size(ty::CAPTION).text_color(t.text_faint).child(format!("…and {more} more")))
                    }),
            )
            .child(div().text_size(ty::CAPTION).text_color(t.text_faint).child(scope_text(r.scope)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().flex_1())
                    .child(ctl::button_sm(&t, format!("cu-deny-{id}"), "Deny", Tone::Quiet).on_click(cx.listener(
                        |this, _, _, cx| this.decide(Decision::Deny { reason: String::new() }, cx),
                    )))
                    .when(!consequential, |d| {
                        d.child(
                            ctl::button_sm(&t, format!("cu-session-{id}"), "Allow in this thread", Tone::Filled)
                                .tooltip(crate::ui::tip("Allow this and later actions of the same kind on this app until the agent's session ends"))
                                .on_click(cx.listener(|this, _, _, cx| this.decide(Decision::Session, cx))),
                        )
                    })
                    .child(
                        ctl::button_sm(&t, format!("cu-once-{id}"), "Allow once", Tone::Solid)
                            .on_click(cx.listener(|this, _, _, cx| this.decide(Decision::Once, cx))),
                    ),
            )
    }
}

// ------------------------------------------------------------- strip

#[derive(Default)]
pub struct Strip {
    thread: Option<TaskId>,
    app: String,
    summary: String,
    at: Option<Instant>,
}

impl Render for Strip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(cx).clone();
        let human = lease().human_control;
        let text = if human {
            "You have control of the computer. Agents wait until you hand it back.".to_string()
        } else {
            let what = if self.summary.is_empty() { String::new() } else { format!(": {}", self.summary) };
            format!("Using {} in the background{what}", if self.app.is_empty() { "an app" } else { &self.app })
        };
        div()
            .id("computer-strip")
            .occlude()
            .w_full()
            .px(px(12.0))
            .py(px(6.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(10.0))
            .bg(t.composer_bg())
            .border_1()
            .border_color(t.border.opacity(0.6))
            .child(icon(icons::MONITOR).size(px(13.0)).text_color(if human { t.accent } else { t.text_muted }))
            .child(div().flex_1().text_size(ty::CAPTION).text_color(t.text_muted).truncate().child(text))
            .child(
                ctl::button_sm(&t, "cu-take", if human { "Hand back" } else { "Take over" }, if human { Tone::Solid } else { Tone::Quiet })
                    .tooltip(crate::ui::tip(if cfg!(windows) {
                        "Stop agents using the computer until you hand it back (Ctrl+Alt+Shift+S)"
                    } else {
                        "Stop agents using the computer until you hand it back"
                    }))
                    .on_click(cx.listener(|_, _, _, cx| toggle_control(cx))),
            )
    }
}

// ------------------------------------------------------------- hotkey

/// Ctrl+Alt+Shift+S, system-wide on Windows: take control or hand it back.
pub fn hotkey_events() -> flume::Receiver<()> {
    static CH: std::sync::OnceLock<flume::Receiver<()>> = std::sync::OnceLock::new();
    CH.get_or_init(|| {
        let (tx, rx) = flume::unbounded();
        start_hotkey(tx);
        rx
    })
    .clone()
}

/// Called from the shell when the hotkey fires.
pub fn on_hotkey(cx: &mut App) {
    toggle_control(cx);
}

#[cfg(windows)]
fn start_hotkey(tx: flume::Sender<()>) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};
    // A distinct id from Voice Mode's registration (1).
    const ID: i32 = 0x0C17;
    let _ = std::thread::Builder::new().name("dove-computer-hotkey".into()).spawn(move || unsafe {
        if RegisterHotKey(std::ptr::null_mut(), ID, MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT, u32::from(b'S')) == 0 {
            tracing::info!("computer use: Ctrl+Alt+Shift+S is taken by another app; use Take over in the thread");
            return;
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if msg.message == WM_HOTKEY && msg.wParam == ID as usize {
                let _ = tx.send(());
            }
        }
        UnregisterHotKey(std::ptr::null_mut(), ID);
    });
}

#[cfg(not(windows))]
fn start_hotkey(_tx: flume::Sender<()>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lease_moves_its_epoch_on_every_change() {
        let start = set_human(false);
        let taken = set_human(true);
        assert!(taken.human_control && taken.epoch == start.epoch + 1);
        assert_eq!(set_human(true), taken, "no change, no new epoch");
        let back = set_human(false);
        assert!(!back.human_control && back.epoch == start.epoch + 2);
    }

    #[test]
    fn decisions_round_trip_to_the_hub() {
        for d in [Decision::Once, Decision::Session, Decision::Deny { reason: "x".into() }] {
            let back: Decision = serde_json::from_str(&decision_text(&d)).unwrap();
            assert_eq!(back, d);
        }
        assert!(is_gui_tool(REVIEW) && is_gui_tool(LEASE) && is_gui_tool(ACTIVITY) && !is_gui_tool("chrome_tabs"));
    }
}
