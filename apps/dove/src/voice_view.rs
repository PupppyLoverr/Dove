//! The call view: while a voice call runs, the chat area becomes a calm,
//! living call screen over the same frosted wallpaper as the chat. In the
//! middle, a round matrix of soft dots: it breathes while Dove listens,
//! follows your voice while it hears you, sweeps while the agent works and
//! pulses outward while Dove speaks. Under it, the words: what you are
//! saying as you say it, then the reply as it is read. A rail lists the
//! threads the call touches (what each is doing, which one needs your
//! approval), and a glass dock mutes, stops the reading, flips to the chat,
//! or ends the call.
//!
//! "Chat" flips back to the thread (the compact strip above the composer
//! keeps the call in view there, with a button to come back). The matrix
//! animates only while the view is on screen, at 20 frames a second while
//! the call is active and 10 while it only breathes; a flipped or ended
//! call draws nothing.

use std::time::{Duration, Instant};

use dove_core::proto::TaskId;
use dove_voice::pipeline::State;
use dove_voice::snapshot::{Snapshot, ThreadStatus};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription,
    Window, canvas, div, point, px, size,
};

use crate::assets::{icon, icons};
use crate::design::ty;
use crate::theme::Theme;
use crate::voice_call::VoiceCall;

/// One frame every 50 ms while the view is on screen and the call is
/// active (hearing, working, speaking); every other one while it only
/// breathes. The dots change brightness, not place, so 20 and 10 frames a
/// second read as smooth, at about half the processor time of 30.
const FRAME: Duration = Duration::from_millis(50);
/// Threads on the rail at most.
const RAIL_MAX: usize = 6;

/// A thread on the rail.
#[derive(Clone, Debug, PartialEq)]
pub struct RailItem {
    pub id: String,
    pub title: String,
    pub agent: String,
    pub status: ThreadStatus,
    /// What it is doing now ("Editing src/auth.rs").
    pub doing: Option<String>,
    /// A permission it waits on.
    pub pending: Option<String>,
    /// The thread the call talks to.
    pub current: bool,
    /// The teammate running it.
    pub teammate: Option<String>,
}

/// The threads worth a glance during a call: those waiting on the user
/// first, then the one the call talks to, then the ones working.
pub fn rail(snap: &Snapshot) -> Vec<RailItem> {
    let current = snap.current.as_deref();
    let mut items: Vec<(u8, RailItem)> = snap
        .threads
        .iter()
        .filter_map(|t| {
            let pending = t.pending.first().map(|p| p.summary.clone());
            let is_current = current == Some(t.id.as_str());
            let rank = if pending.is_some() {
                0
            } else if is_current {
                1
            } else if t.status == ThreadStatus::Running {
                2
            } else {
                return None;
            };
            Some((
                rank,
                RailItem {
                    id: t.id.clone(),
                    title: t.title.clone(),
                    agent: t.agent.clone(),
                    status: t.status,
                    doing: t.activity.clone().filter(|a| !a.trim().is_empty()),
                    pending,
                    current: is_current,
                    teammate: t.teammate.clone(),
                },
            ))
        })
        .collect();
    items.sort_by_key(|(rank, _)| *rank);
    items.into_iter().map(|(_, i)| i).take(RAIL_MAX).collect()
}

/// Dots across the matrix (an odd count keeps one dot at the centre).
pub const MATRIX: usize = 17;

/// How lit a matrix dot is, 0 to 1, at grid position (`x`, `y`) (each -1
/// to 1, the centre at 0), `secs` into the call, with the voice `level`
/// (0 to 1) and the call's `state`.
pub fn matrix_glow(x: f32, y: f32, secs: f32, level: f32, state: State) -> f32 {
    use std::f32::consts::TAU;
    let d = (x * x + y * y).sqrt().min(1.0);
    let level = level.clamp(0.0, 1.0);
    let wave = |speed: f32, k: f32| 0.5 + 0.5 * (secs * speed - d * k).sin();
    let v = match state {
        // A slow shimmer turning around the edge while engines load.
        State::Loading => {
            let a = y.atan2(x);
            let sweep = (a - secs * 2.0).rem_euclid(TAU);
            0.08 + 0.32 * (-(sweep - 1.0).powi(2) * 1.6).exp() * d
        }
        // A slow breath from the centre; your voice brightens it.
        State::Listening => 0.14 + 0.2 * wave(1.6, 4.0) * (1.0 - 0.45 * d) + 0.45 * level * (1.0 - d),
        // Your voice, from the centre out, with a quick ripple on top.
        State::Hearing => 0.16 + level * (0.72 * (1.0 - 0.7 * d) + 0.2 * wave(9.0, 11.0)) + 0.1 * wave(3.0, 5.0),
        // A comet sweeping around: the agent is working.
        State::Thinking => {
            let a = y.atan2(x);
            let behind = (secs * 2.6 - a).rem_euclid(TAU);
            0.1 + 0.78 * (-behind * 1.25).exp() * (0.35 + 0.65 * d)
        }
        // Rings travelling outward, stronger with the voice.
        State::Speaking => {
            let ring = (secs * 4.5 - d * 8.5).sin().max(0.0).powi(2);
            0.12 + ring * (0.5 + 0.45 * level) * (1.0 - 0.25 * d) + 0.12 * (1.0 - d)
        }
        State::Muted => 0.07 + 0.04 * wave(0.8, 3.0),
    };
    v.clamp(0.0, 1.0)
}

/// "3:07" from seconds.
pub fn clock(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

pub enum CallViewEvent {
    /// Show the normal window (the chat), keeping the call.
    ShowChat,
    /// Open a thread in the chat (to approve something there).
    OpenThread(TaskId),
    /// Point the call at a thread (a rail row was clicked).
    TalkTo(TaskId),
    /// Answer the call as if spoken ("yes", "send it", "never mind").
    Say(&'static str),
}

pub struct CallView {
    call: Entity<VoiceCall>,
    rail: Vec<RailItem>,
    /// On screen: the matrix animates only then.
    visible: bool,
    /// Ticks since shown, for the calm frame rate.
    ticks: u64,
    born: Instant,
    focus: FocusHandle,
    _tick: Option<gpui::Task<()>>,
    _observe: Subscription,
    /// The "What you can say" sheet shows.
    help: bool,
}

impl EventEmitter<CallViewEvent> for CallView {}

impl Focusable for CallView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl CallView {
    pub fn new(call: Entity<VoiceCall>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&call, |_, _, cx| cx.notify());
        Self {
            call,
            rail: Vec::new(),
            visible: false,
            ticks: 0,
            born: Instant::now(),
            focus: cx.focus_handle(),
            _tick: None,
            _observe: observe,
            help: false,
        }
    }

    /// Show or hide the view; frames run only while it shows.
    pub fn set_visible(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.visible == on {
            return;
        }
        self.visible = on;
        self._tick = on.then(|| {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(FRAME).await;
                    let go = this
                        .update(cx, |v, cx| {
                            v.ticks += 1;
                            let call = v.call.read(cx);
                            let calm = call.muted
                                || (matches!(call.state, State::Listening | State::Loading | State::Muted)
                                    && call.level < 0.02);
                            if v.visible && (!calm || v.ticks % 2 == 0) {
                                cx.notify();
                            }
                            v.visible
                        })
                        .unwrap_or(false);
                    if !go {
                        break;
                    }
                }
            })
        });
        cx.notify();
    }

    pub fn set_rail(&mut self, rail: Vec<RailItem>, cx: &mut Context<Self>) {
        if self.rail != rail {
            self.rail = rail;
            cx.notify();
        }
    }

    fn render_rail(&self, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.rail.is_empty() {
            return None;
        }
        let rows = self.rail.iter().enumerate().map(|(i, item)| {
            let id = TaskId::parse(&item.id);
            let waiting = item.pending.is_some();
            let (glyph, tint): (gpui::AnyElement, Hsla) = match (waiting, item.status) {
                (true, _) => {
                    (icon(icons::DANGER_TRIANGLE).size(px(13.0)).text_color(t.warning).into_any_element(), t.warning)
                }
                (false, ThreadStatus::Running) => {
                    (crate::kit::spinner(("call-rail-spin", i), 12.0, t.busy).into_any_element(), t.busy)
                }
                (false, ThreadStatus::Failed) => {
                    (icon(icons::CLOSE_CIRCLE).size(px(13.0)).text_color(t.danger).into_any_element(), t.danger)
                }
                (false, ThreadStatus::Done) => {
                    (icon(icons::CHECK).size(px(13.0)).text_color(t.success).into_any_element(), t.success)
                }
                _ => (
                    icon(icons::CHAT_ROUND_LINE).size(px(13.0)).text_color(t.text_muted).into_any_element(),
                    t.text_muted,
                ),
            };
            let line: SharedString = match (&item.pending, &item.doing) {
                (Some(p), _) => format!("Needs your approval: {p}").into(),
                (None, Some(d)) => d.clone().into(),
                (None, None) => match item.status {
                    ThreadStatus::Running => "Working".into(),
                    ThreadStatus::Done => "Finished".into(),
                    ThreadStatus::Failed => "Stopped with an error".into(),
                    ThreadStatus::NeedsInput => "Waiting for you".into(),
                    ThreadStatus::Idle => "Ready".into(),
                },
            };
            div()
                .id(("call-rail", i))
                .w_full()
                .min_h(px(44.0))
                .px(px(12.0))
                .py(px(8.0))
                .flex()
                .items_center()
                .gap(px(10.0))
                .rounded(px(14.0))
                .bg(t.composer_bg())
                .border_1()
                .border_color(if waiting {
                    t.warning.opacity(0.4)
                } else if item.current {
                    t.accent.opacity(0.4)
                } else {
                    t.composer_border()
                })
                // Any other thread: click to talk to it instead.
                .when_some(id.filter(|_| !item.current), |d, id| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(t.glass_hover()))
                        .tooltip(crate::ui::tip("Talk to this thread"))
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(CallViewEvent::TalkTo(id))))
                })
                .child(div().size(px(16.0)).flex_none().flex().items_center().justify_center().child(glyph))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .min_w_0()
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(ty::SMALL)
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(t.text)
                                        .child(item.title.clone()),
                                )
                                .child(div().flex_none().text_size(ty::CAPTION).text_color(t.text_faint).child({
                                    let who = match &item.teammate {
                                        Some(m) => format!("{m} · {}", item.agent),
                                        None => item.agent.clone(),
                                    };
                                    if item.current { format!("{who} · talking to") } else { who }
                                })),
                        )
                        .child(div().min_w_0().truncate().text_size(ty::CAPTION).text_color(tint).child(line)),
                )
                .when_some(id.filter(|_| waiting), |d, id| {
                    d.child(
                        div()
                            .id(("call-rail-open", i))
                            .flex_none()
                            .h(px(28.0))
                            .px(px(12.0))
                            .flex()
                            .items_center()
                            .rounded_full()
                            .bg(t.warning.opacity(0.16))
                            .text_size(ty::CAPTION)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.text)
                            .cursor_pointer()
                            .hover(|s| s.bg(t.warning.opacity(0.24)))
                            .child("Approve in chat")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.stop_propagation();
                                cx.emit(CallViewEvent::OpenThread(id))
                            })),
                    )
                })
        });
        Some(div().w_full().max_w(px(520.0)).flex().flex_col().gap(px(6.0)).children(rows).into_any_element())
    }

    /// What the call is waiting on an answer for: several threads to open,
    /// or a draft sitting in an input. Its buttons answer as speech would.
    fn render_pending(&self, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let call = self.call.read(cx);
        let (glyph, head, lines, yes, no): (&'static str, String, Vec<String>, &'static str, &'static str) =
            if let Some(dove_voice::intent::Action::Launch { items, .. }) = call.conductor.pending_plan() {
                let first = items.first()?;
                let agent = dove_core::proto::HarnessKind::from_id(&first.agent).map(|k| k.label()).unwrap_or("agent");
                let kind = if first.terminal { "terminals" } else { "chats" };
                let lines = items.iter().filter_map(|i| i.prompt.clone()).collect::<Vec<_>>();
                let mut lines = lines;
                lines.dedup();
                (icons::WIDGET_ADD, format!("Open {} {agent} {kind}?", items.len()), lines, "Open them", "Not now")
            } else if let Some((thread, text)) = call.conductor.pending_draft() {
                let title = self.rail.iter().find(|r| r.id == thread).map(|r| r.title.clone());
                let head = match title {
                    Some(t) => format!("Draft in {t}"),
                    None => "Draft".to_string(),
                };
                (icons::PEN, head, vec![text.to_string()], "Send it", "Clear")
            } else {
                return None;
            };
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .h(px(30.0))
                .px(px(14.0))
                .flex()
                .items_center()
                .rounded_full()
                .cursor_pointer()
                .text_size(ty::SMALL)
                .font_weight(gpui::FontWeight::MEDIUM)
                .when(primary, |d| d.bg(t.accent).text_color(gpui::white()).hover(|s| s.opacity(0.9)))
                .when(!primary, |d| d.bg(t.wash(0.07)).text_color(t.text).hover(|s| s.bg(t.wash(0.11))))
                .child(label)
        };
        Some(
            glass(t, 18.0)
                .border_color(t.accent.opacity(0.4))
                .w_full()
                .max_w(px(520.0))
                .p(px(14.0))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(icon(glyph).size(px(14.0)).text_color(t.accent))
                        .child(
                            div()
                                .text_size(ty::BODY)
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(t.text)
                                .child(head),
                        ),
                )
                .children(lines.into_iter().take(4).map(|l| {
                    div().text_size(ty::SMALL).line_height(ty::lh::BODY).text_color(t.text_muted).line_clamp(2).child(l)
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            button("call-pending-yes", yes, true)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(CallViewEvent::Say("yes")))),
                        )
                        .child(
                            button("call-pending-no", no, false)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(CallViewEvent::Say("no")))),
                        )
                        .child(div().flex_1())
                        .child(div().text_size(ty::CAPTION).text_color(t.text_faint).child("or just say it")),
                )
                .into_any_element(),
        )
    }
}

/// A filled circle.
fn circle(window: &mut Window, c: (f32, f32), r: f32, color: Hsla) {
    if r <= 0.0 {
        return;
    }
    let b = Bounds::new(point(px(c.0 - r), px(c.1 - r)), size(px(2.0 * r), px(2.0 * r)));
    window.paint_quad(gpui::fill(b, color).corner_radii(px(r)));
}

/// The matrix: rounded dots on a square grid, inside a circle, each lit by
/// [`matrix_glow`]. Brighter dots also grow a little, so it reads as soft
/// light rather than a chart.
fn paint_matrix(bounds: Bounds<Pixels>, secs: f32, level: f32, state: State, color: Hsla, window: &mut Window) {
    let w = f32::from(bounds.size.width);
    let h = f32::from(bounds.size.height);
    let side = w.min(h);
    let (cx, cy) = (f32::from(bounds.origin.x) + w / 2.0, f32::from(bounds.origin.y) + h / 2.0);
    let step = side / MATRIX as f32;
    let half = (MATRIX as f32 - 1.0) / 2.0;
    // A soft halo behind the dots, brighter as the call gets busier.
    let busy = matrix_glow(0.0, 0.0, secs, level, state);
    circle(window, (cx, cy), side * 0.5, color.opacity(0.03 + 0.05 * busy));
    circle(window, (cx, cy), side * 0.36, color.opacity(0.03 + 0.05 * busy));
    for i in 0..MATRIX {
        for j in 0..MATRIX {
            let (x, y) = ((i as f32 - half) / half, (j as f32 - half) / half);
            if x * x + y * y > 1.02 {
                continue;
            }
            let g = matrix_glow(x, y, secs, level, state);
            let r = step * (0.22 + 0.17 * g);
            circle(window, (cx + x * half * step, cy + y * half * step), r, color.opacity(0.2 + 0.8 * g));
        }
    }
}

/// The surface the call view's cards sit on: the chat composer's glass.
fn glass(t: &Theme, radius: f32) -> gpui::Div {
    div()
        .rounded(px(radius))
        .bg(t.composer_bg())
        .border_1()
        .border_color(t.composer_border())
        .shadow(t.composer_shadows(false))
}

impl Render for CallView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("CallView");
        let t = Theme::of(cx).clone();
        let call = self.call.read(cx);
        let state = if call.muted { State::Muted } else { call.state };
        let awaiting = call.awaiting_click.is_some();
        let muted = call.muted;
        let speaking = call.state == State::Speaking && !muted;
        let level = if muted { 0.0 } else { (call.level.clamp(0.0, 0.3) / 0.3).clamp(0.0, 1.0) };
        let (label, tint) = if awaiting {
            ("Approve it in the chat to go on", t.warning)
        } else {
            match state {
                State::Loading => ("Starting the call…", t.text_muted),
                State::Listening => ("Listening", t.accent),
                State::Hearing => ("Hearing you", t.accent),
                State::Thinking => ("Working on it", t.busy),
                State::Speaking => ("Speaking · talk to interrupt", t.accent),
                State::Muted => ("Muted · hold the dots or Space to talk", t.text_muted),
            }
        };
        let dot_color = match state {
            State::Muted => t.text_faint,
            State::Thinking => t.busy,
            _ if awaiting => t.warning,
            _ => t.accent,
        };
        // The words: a live partial while you talk, else the last line.
        let (words, words_color): (Option<SharedString>, Hsla) = match (&call.error, &call.partial, &call.caption) {
            (Some(e), _, _) => (Some(e.clone()), t.danger),
            (None, Some(p), _) => (Some(format!("{p}…").into()), t.text_muted),
            (None, None, Some(c)) => (Some(c.clone()), t.text),
            (None, None, None) if call.state == State::Loading => (None, t.text),
            (None, None, None) if call.new_thread => {
                (Some("Say what you'd like done. A new thread starts on your picked agent.".into()), t.text_faint)
            }
            (None, None, None) => (
                Some(
                    "Ask what's happening, give the next step, check on the team, open a few chats at once, or draft into any thread."
                        .into(),
                ),
                t.text_faint,
            ),
        };
        let history: Vec<(bool, String)> = {
            let n = call.lines.len();
            // The lines before the one shown big, as a short conversation.
            call.lines.iter().take(n.saturating_sub(1)).rev().take(4).rev().map(|l| (l.you, l.text.clone())).collect()
        };
        let wide = window.viewport_size().width > px(1040.0);
        let help = self.help;
        let target = call.target.clone();
        let elapsed = (call.state != State::Loading).then(|| clock(call.started().elapsed().as_secs()));
        let engines = call.engines.clone();
        let secs = self.born.elapsed().as_secs_f32();

        let matrix_px = if wide { 300.0 } else { 240.0 };
        let matrix = div()
            .id("call-matrix")
            .size(px(matrix_px))
            .flex_none()
            .cursor_pointer()
            .tooltip(crate::ui::tip(if muted { "Hold to talk" } else { "Hold to talk while muted" }))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|v, _, _, cx| v.call.update(cx, |c, _| c.push_to_talk(true))),
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|v, _, _, cx| v.call.update(cx, |c, _| c.push_to_talk(false))),
            )
            .child(
                canvas(|_, _, _| (), move |b, _, window, _| paint_matrix(b, secs, level, state, dot_color, window))
                    .size_full(),
            );

        let chip =
            |id: &'static str| glass(&t, 999.0).id(id).h(px(34.0)).px(px(14.0)).flex().items_center().gap(px(8.0));
        let target_chip = chip("call-top")
            .text_size(ty::SMALL)
            .child(icon(icons::PHONE).size(px(13.0)).text_color(t.accent))
            .child(
                div()
                    .max_w(px(420.0))
                    .truncate()
                    .text_color(t.text)
                    .child(target.unwrap_or_else(|| "Voice call".into())),
            )
            .when_some(elapsed, |d, e| {
                d.child(div().text_color(t.text_faint).child("·")).child(div().text_color(t.text_muted).child(e))
            })
            .when_some(engines, |d, e| d.tooltip(crate::ui::tip(e)));
        let help_toggle = chip("call-help")
            .cursor_pointer()
            .when(help, |d| d.border_color(t.accent.opacity(0.5)))
            .hover(|s| s.bg(t.glass_hover()))
            .text_size(ty::SMALL)
            .text_color(if help { t.text } else { t.text_muted })
            .child(icon(icons::INFO_CIRCLE).size(px(13.0)).text_color(if help { t.accent } else { t.text_muted }))
            .child("What you can say")
            .on_click(cx.listener(|v, _, _, cx| {
                v.help = !v.help;
                cx.notify();
            }));
        let top = div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(div().flex_1())
            .child(target_chip)
            .child(div().flex_1().flex().justify_end().child(help_toggle));

        // The state, as a small glass pill with its own light.
        let status = glass(&t, 999.0)
            .h(px(30.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(div().size(px(7.0)).rounded_full().bg(tint))
            .child(div().text_size(ty::SMALL).font_weight(gpui::FontWeight::MEDIUM).text_color(tint).child(label));

        let controls = glass(&t, 999.0)
            .p(px(8.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(
                control("call-mute", if muted { "Unmute microphone" } else { "Mute microphone" }, &t)
                    .when(muted, |d| d.bg(t.danger.opacity(0.16)))
                    .child(
                        icon(if muted { icons::MICROPHONE_OFF } else { icons::MICROPHONE })
                            .size(px(20.0))
                            .text_color(if muted { t.danger } else { t.text }),
                    )
                    .on_click(cx.listener(move |v, _, _, cx| v.call.update(cx, |c, cx| c.set_muted(!muted, cx)))),
            )
            .when(speaking, |d| {
                d.child(
                    control("call-silence", "Stop speaking (the agent keeps working)", &t)
                        .child(icon(icons::STOP).size(px(18.0)).text_color(t.text))
                        .on_click(cx.listener(|v, _, _, cx| v.call.update(cx, |c, cx| c.silence(cx)))),
                )
            })
            .child(
                control("call-chat", "Show the chat (Esc)", &t)
                    .child(icon(icons::CHAT_ROUND_LINE).size(px(20.0)).text_color(t.text))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(CallViewEvent::ShowChat))),
            )
            .child(
                control("call-end", "End call", &t)
                    .bg(t.danger)
                    .child(icon(icons::PHONE_HANG_UP).size(px(20.0)).text_color(gpui::white()))
                    .on_click(cx.listener(|v, _, _, cx| v.call.update(cx, |c, cx| c.end(cx)))),
            );

        // The conversation so far, as bubbles (you on the right), then the
        // line being heard or said, large.
        let bubbles = div().w_full().max_w(px(560.0)).flex().flex_col().gap(px(6.0)).children(history.into_iter().map(
            |(you, text)| {
                div().w_full().flex().when(you, |d| d.justify_end()).child(
                    div()
                        .max_w(px(420.0))
                        .px(px(12.0))
                        .py(px(7.0))
                        .rounded(px(14.0))
                        .text_size(ty::SMALL)
                        .line_height(ty::lh::BODY)
                        .line_clamp(2)
                        .border_1()
                        .when(you, |d| {
                            d.bg(t.accent.opacity(0.18)).border_color(t.accent.opacity(0.22)).text_color(t.text)
                        })
                        .when(!you, |d| {
                            d.bg(t.composer_bg()).border_color(t.composer_border()).text_color(t.text_muted)
                        })
                        .child(text),
                )
            },
        ));
        let words_block =
            div().w_full().max_w(px(640.0)).flex().flex_col().items_center().gap(px(14.0)).child(bubbles).when_some(
                words,
                |d, w| {
                    d.child(
                        div()
                            .text_center()
                            .text_size(px(21.0))
                            .line_height(px(29.0))
                            .text_color(words_color)
                            .line_clamp(3)
                            .child(w),
                    )
                },
            );
        let stage = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .child(matrix)
            .child(status)
            .child(words_block);
        // What needs an answer and who's working: beside the stage when the
        // window is wide, under it otherwise.
        let pending = self.render_pending(&t, cx);
        let rail = self.render_rail(&t, cx);
        let side = (pending.is_some() || rail.is_some()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .when(wide, |d| d.w(px(380.0)).flex_none().justify_center())
                .when(!wide, |d| d.w_full().items_center())
                .children(pending)
                .when_some(rail, |d, r| {
                    d.child(
                        div()
                            .w_full()
                            .max_w(px(520.0))
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .px(px(4.0))
                                    .text_size(ty::CAPTION)
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(t.text_faint)
                                    .child("Threads · click one to talk to it"),
                            )
                            .child(r),
                    )
                })
        });
        let body = if wide {
            div().flex_1().min_h_0().w_full().flex().gap(px(28.0)).child(stage).children(side).into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(stage)
                .children(side.map(|s| s.pb(px(8.0))))
                .into_any_element()
        };

        div()
            .id("call-view")
            .key_context("CallView")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .occlude()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.0))
            .px(px(28.0))
            .py(px(20.0))
            .on_key_down(cx.listener(|v, e: &gpui::KeyDownEvent, _, cx| match e.keystroke.key.as_str() {
                "escape" if v.help => {
                    v.help = false;
                    cx.notify();
                }
                "escape" => cx.emit(CallViewEvent::ShowChat),
                "space" if !e.is_held => v.call.update(cx, |c, _| c.push_to_talk(true)),
                _ => {}
            }))
            .on_key_up(cx.listener(|v, e: &gpui::KeyUpEvent, _, cx| {
                if e.keystroke.key == "space" {
                    v.call.update(cx, |c, _| c.push_to_talk(false));
                }
            }))
            .child(top)
            .child(body)
            .child(controls)
            .when(help, |d| d.child(render_help(&t, cx)))
    }
}

/// Everything the call understands, grouped, with a phrase to try for
/// each. Spoken in any wording close to these.
pub const CAPABILITIES: &[(&str, &str, &[&str])] = &[
    (
        "Threads",
        "Hear how work stands and steer it",
        &[
            "What's happening?",
            "What's everyone doing?",
            "Talk to the tests thread",
            "Tell Codex to also run clippy",
            "Stop",
            "Take me back",
        ],
    ),
    (
        "Start work",
        "One thread, or several at once (Dove asks before opening more than one)",
        &[
            "Start a Claude task on the login page in a new worktree",
            "Launch three Claude chats to review the auth code",
            "Open two Codex terminals",
            "Spin up a terminal as the Builder",
        ],
    ),
    (
        "Team",
        "Your teammates' progress, and questions for them",
        &["What's the team doing?", "How's the Reviewer doing?", "Ask the Reviewer to look at the login diff"],
    ),
    (
        "Prompt any chat or terminal",
        "Words go in its input unsent; then send, change or clear them",
        &[
            "Draft for Codex: run the tests on Windows",
            "Type cargo test into the codex terminal",
            "Send it · Change it to … · Add … · Never mind",
        ],
    ),
    (
        "Layout and context",
        "Open things where you can see them",
        &["Open the tests thread beside this", "Show the diff", "Take a look at this"],
    ),
    (
        "Approvals and the call",
        "Risky approvals always need your click",
        &["Approve", "Approve for this session", "Deny", "Talk faster", "Mute", "Be quiet", "Repeat that", "End call"],
    ),
];

/// The "What you can say" sheet over the call view.
fn render_help(t: &Theme, cx: &mut Context<CallView>) -> gpui::AnyElement {
    let groups = CAPABILITIES.iter().map(|(title, blurb, phrases)| {
        div()
            .w(px(300.0))
            .flex_grow(1.0)
            .p(px(14.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .rounded(px(16.0))
            .bg(t.composer_bg())
            .border_1()
            .border_color(t.composer_border())
            .child(div().text_size(ty::BODY).font_weight(gpui::FontWeight::MEDIUM).text_color(t.text).child(*title))
            .child(div().text_size(ty::CAPTION).text_color(t.text_faint).child(*blurb))
            .children(
                phrases.iter().map(|p| div().text_size(ty::SMALL).text_color(t.text_muted).child(format!("“{p}”"))),
            )
    });
    div()
        .id("call-help-sheet")
        .absolute()
        .inset_0()
        .bg(t.bg.opacity(0.82))
        .flex()
        .justify_center()
        .overflow_y_scroll()
        .on_click(cx.listener(|v, _, _, cx| {
            v.help = false;
            cx.notify();
        }))
        .child(
            div()
                .max_w(px(980.0))
                .w_full()
                .px(px(28.0))
                .py(px(36.0))
                .flex()
                .flex_col()
                .gap(px(16.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(20.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(t.text)
                                .child("What you can say"),
                        )
                        .child(div().text_size(ty::SMALL).text_color(t.text_faint).child("Esc or click to close")),
                )
                .child(
                    div()
                        .text_size(ty::SMALL)
                        .text_color(t.text_muted)
                        .child("Talk naturally: these are examples, not exact commands. Anything else goes to the thread you're talking to as your message. Ctrl+Shift+Space in another thread moves the call there."),
                )
                .child(div().flex().flex_wrap().gap(px(12.0)).children(groups)),
        )
        .into_any_element()
}

/// A call control: a 52px round button in the dock.
fn control(id: &'static str, label: &'static str, t: &Theme) -> gpui::Stateful<gpui::Div> {
    crate::ui::round_button_sized(id, label, 52.0, t).bg(t.wash(0.06))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dove_voice::snapshot::{Pending, Thread};

    fn thread(id: &str, title: &str, agent: &str, agent_id: &str, status: ThreadStatus) -> Thread {
        Thread {
            id: id.into(),
            title: title.into(),
            agent: agent.into(),
            agent_id: agent_id.into(),
            project: "dove".into(),
            status,
            live: true,
            steers: true,
            interrupts: true,
            session_approvals: true,
            pending: Vec::new(),
            activity: None,
            todos: None,
            diff: None,
            cwd: None,
            error: None,
            teammate: None,
            terminal: false,
        }
    }

    #[test]
    fn the_rail_puts_waiting_threads_first_and_skips_idle_ones() {
        let mut waiting = thread("a", "Deploy", "Codex", "codex", ThreadStatus::NeedsInput);
        waiting.pending.push(Pending {
            request_id: "r1".into(),
            tool: "Bash".into(),
            summary: "Run npm publish".into(),
            input: serde_json::json!({}),
            question: false,
        });
        let snap = Snapshot {
            threads: vec![
                thread("b", "Idle one", "Claude Code", "claude", ThreadStatus::Idle),
                thread("c", "Busy", "Claude Code", "claude", ThreadStatus::Running),
                thread("d", "Current", "Claude Code", "claude", ThreadStatus::Done),
                waiting,
            ],
            current: Some("d".into()),
            ..Default::default()
        };
        let r = rail(&snap);
        let ids: Vec<&str> = r.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "d", "c"]);
        assert_eq!(r[0].pending.as_deref(), Some("Run npm publish"));
        assert!(r[1].current && !r[2].current);
    }

    #[test]
    fn the_matrix_follows_the_call() {
        let states = [State::Loading, State::Listening, State::Hearing, State::Thinking, State::Speaking, State::Muted];
        for st in states {
            for i in 0..40 {
                let g = matrix_glow(0.3, -0.5, i as f32 * 0.13, 0.7, st);
                assert!((0.0..=1.0).contains(&g), "{st:?} glows within 0..1");
            }
        }
        // Your voice lights the centre while it hears you.
        assert!(
            matrix_glow(0.0, 0.0, 1.0, 1.0, State::Hearing) > matrix_glow(0.0, 0.0, 1.0, 0.0, State::Hearing) + 0.4
        );
        // Muted is dimmer than any active state.
        let muted = matrix_glow(0.0, 0.0, 1.0, 1.0, State::Muted);
        assert!(muted < 0.12 && muted < matrix_glow(0.0, 0.0, 1.0, 1.0, State::Listening));
        // Working moves: the same dot changes over time.
        let a = matrix_glow(0.9, 0.0, 0.0, 0.0, State::Thinking);
        let b = matrix_glow(0.9, 0.0, 1.2, 0.0, State::Thinking);
        assert!((a - b).abs() > 0.2, "the sweep moves");
    }

    #[test]
    fn clock_reads_minutes_and_seconds() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(187), "3:07");
    }
}
