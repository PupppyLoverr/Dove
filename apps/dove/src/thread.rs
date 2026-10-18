//! A thread: its chat, and the same session in the agent's own terminal UI.
//!
//! Chat ⇄ Terminal hands one session between two drivers (the engine allows
//! only one at a time):
//!
//! - To the terminal: if the chat session is idle, stop it (it is between
//!   turns, so nothing is lost) and start the agent's TUI in a PTY resuming
//!   the same session, in the same worktree, with the same model and
//!   permission level. Mid-turn, a sheet offers "switch at end of turn"
//!   (default) or "interrupt and switch now".
//! - While the TUI runs, the engine mirrors its session file into the chat
//!   (and the transcript), so chat, status and the side panel stay live. The
//!   chat is read-only with a banner.
//! - Back to chat: when the TUI is quiet, type its exit command and give it a
//!   moment to leave, then kill it. The next message resumes headless.

use std::sync::Arc;
use std::time::Duration;

use dove_core::Engine;
use dove_core::proto::{AgentEvent, TaskStatus};
use dove_core::store::{Project, Task};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div, px,
};

use crate::assets::{icon, icons};
use crate::design::{radius, ty};
use crate::kit;
use crate::terminal::{TerminalEvent, TerminalView};
use crate::theme::Theme;
use crate::transcript::{TaskView, TaskViewEvent};
use crate::ui;
use crate::voice::{Dictation, DictationEvent};

/// The TUI counts as idle after this long without output.
const QUIET: Duration = Duration::from_millis(1000);
/// How long `/exit` gets before the process is killed. The session file is
/// already on disk; this only covers the TUI's own shutdown.
const EXIT_WAIT: Duration = Duration::from_millis(900);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewMode {
    Chat,
    Terminal,
}

/// Which way a pending switch goes, for the mid-turn sheet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pending {
    ToTerminal,
    ToChat,
}

pub enum ThreadEvent {
    Chat(TaskViewEventKind),
    /// Chat or Terminal is now on screen.
    Mode,
}

/// The chat events the shell cares about, re-emitted.
#[derive(Clone)]
pub enum TaskViewEventKind {
    Status(TaskStatus),
    Changed,
    Native(String),
    Fork,
    ShowChanges,
    Title(String),
    AiTitle(String),
    DriveBrowser(crate::browser::Drive),
}

pub struct ThreadView {
    engine: Arc<Engine>,
    pub chat: Entity<TaskView>,
    terminal: Option<Entity<TerminalView>>,
    /// What is on screen.
    pub view: ViewMode,
    /// Who drives the session.
    pub driver: ViewMode,
    sheet: Option<Pending>,
    /// Keyboard focus of the switch sheet.
    sheet_focus: FocusHandle,
    armed: Option<Pending>,
    busy: Option<&'static str>,
    note: Option<SharedString>,
    voice: Entity<Dictation>,
    mirror: Option<dove_core::mirror::MirrorSession>,
    focus: FocusHandle,
    _mirror_pump: Option<gpui::Task<()>>,
    _watch: Option<gpui::Task<()>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<ThreadEvent> for ThreadView {}

impl Focusable for ThreadView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ThreadView {
    /// The open thread parses each token. Every other live thread coalesces
    /// its stream and paints the tail when it is opened.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.chat.update(cx, |c, cx| c.set_shown(shown, cx));
    }

    pub fn new(engine: Arc<Engine>, chat: Entity<TaskView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let voice = cx.new(|cx| Dictation::new(engine.clone(), cx));
        let mut subs = vec![cx.subscribe_in(&chat, window, |this, _, ev: &TaskViewEvent, window, cx| {
            let kind = match ev {
                TaskViewEvent::Status(s) => {
                    // An armed switch fires when the turn ends.
                    if this.armed == Some(Pending::ToTerminal) && *s != TaskStatus::Running {
                        this.armed = None;
                        this.switch_to_terminal_now(window, cx);
                    }
                    TaskViewEventKind::Status(*s)
                }
                TaskViewEvent::Changed => TaskViewEventKind::Changed,
                TaskViewEvent::Native(id) => TaskViewEventKind::Native(id.clone()),
                TaskViewEvent::Fork => TaskViewEventKind::Fork,
                TaskViewEvent::ShowChanges => TaskViewEventKind::ShowChanges,
                TaskViewEvent::Title(t) => TaskViewEventKind::Title(t.clone()),
                TaskViewEvent::AiTitle(t) => TaskViewEventKind::AiTitle(t.clone()),
                TaskViewEvent::DriveBrowser(d) => TaskViewEventKind::DriveBrowser(d.clone()),
                TaskViewEvent::SwitchToChat => {
                    this.request(ViewMode::Chat, window, cx);
                    return;
                }
                TaskViewEvent::ShowTerminal => {
                    this.view = ViewMode::Terminal;
                    this.focus_terminal(window, cx);
                    cx.emit(ThreadEvent::Mode);
                    cx.notify();
                    return;
                }
            };
            cx.emit(ThreadEvent::Chat(kind));
        })];
        subs.push(cx.subscribe(&voice, |this, _, ev: &DictationEvent, cx| match ev {
            DictationEvent::Text(text) => {
                if let Some(t) = &this.terminal {
                    let text = text.clone();
                    t.update(cx, |t, cx| t.type_text(&text, cx));
                }
            }
            DictationEvent::Changed => cx.notify(),
        }));
        Self {
            engine,
            chat,
            terminal: None,
            view: ViewMode::Chat,
            driver: ViewMode::Chat,
            sheet: None,
            sheet_focus: cx.focus_handle(),
            armed: None,
            busy: None,
            note: None,
            voice,
            mirror: None,
            focus: cx.focus_handle(),
            _mirror_pump: None,
            _watch: None,
            _subs: subs,
        }
    }

    fn task(&self, cx: &gpui::App) -> (Task, Project) {
        let v = self.chat.read(cx);
        (v.task.clone(), v.project.clone())
    }

    /// Ctrl+` or the view switch in the thread.
    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = if self.view == ViewMode::Chat { ViewMode::Terminal } else { ViewMode::Chat };
        self.request(next, window, cx);
    }

    /// Ask for a view. Switching drivers may need the end of a turn.
    pub fn request(&mut self, mode: ViewMode, window: &mut Window, cx: &mut Context<Self>) {
        self.note = None;
        match mode {
            ViewMode::Terminal if self.driver == ViewMode::Terminal => {
                self.view = ViewMode::Terminal;
                self.focus_terminal(window, cx);
            }
            ViewMode::Terminal => {
                if self.chat.read(cx).is_working() {
                    self.sheet = Some(Pending::ToTerminal);
                } else {
                    self.switch_to_terminal_now(window, cx);
                }
            }
            ViewMode::Chat if self.driver == ViewMode::Chat => {
                self.view = ViewMode::Chat;
                let composer = self.chat.read(cx).composer.clone();
                crate::composer::focus(&composer, window, cx);
            }
            ViewMode::Chat => {
                let quiet =
                    self.terminal.as_ref().is_none_or(|t| t.read(cx).is_quiet(QUIET) || !t.read(cx).is_running());
                if quiet {
                    self.switch_to_chat_now(false, window, cx);
                } else {
                    self.sheet = Some(Pending::ToChat);
                }
            }
        }
        cx.emit(ThreadEvent::Mode);
        cx.notify();
    }

    fn choose(&mut self, now: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.sheet.take() else { return };
        match (p, now) {
            (Pending::ToTerminal, true) => {
                self.chat.update(cx, |c, cx| c.interrupt(cx));
                self.armed = Some(Pending::ToTerminal);
                self.note = Some("Interrupting, then switching…".into());
            }
            (Pending::ToTerminal, false) => {
                self.armed = Some(Pending::ToTerminal);
                self.note = Some("Switches to the terminal when this turn ends".into());
            }
            (Pending::ToChat, true) => self.switch_to_chat_now(true, window, cx),
            (Pending::ToChat, false) => {
                self.armed = Some(Pending::ToChat);
                self.note = Some("Switches back to chat when the terminal goes quiet".into());
                self.watch_quiet(window, cx);
            }
        }
        cx.notify();
    }

    /// Poll the TUI until it is quiet, then hand back to chat.
    fn watch_quiet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._watch = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let quiet = this
                    .update(cx, |v, cx| {
                        v.armed == Some(Pending::ToChat)
                            && v.terminal.as_ref().is_none_or(|t| t.read(cx).is_quiet(Duration::from_millis(800)))
                    })
                    .unwrap_or(false);
                let still = this.update(cx, |v, _| v.armed == Some(Pending::ToChat)).unwrap_or(false);
                if !still {
                    break;
                }
                if quiet {
                    let _ = this.update_in(cx, |v, window, cx| {
                        v.armed = None;
                        v.finish_to_chat(false, window, cx);
                    });
                    break;
                }
            }
        }));
    }

    fn session_task(&self, cx: &gpui::App) -> (Task, Project) {
        let (mut task, project) = self.task(cx);
        // The store is updated on SessionStarted even if this view missed it.
        if task.native_session_id.is_none()
            && let Ok(Some(stored)) = self.engine.store.task(task.id)
        {
            task.native_session_id = stored.native_session_id;
        }
        (task, project)
    }

    fn switch_to_terminal_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (task, project) = self.session_task(cx);
        let run = self.chat.read(cx).composer.read(cx).run_settings(cx);
        let cmd = match self.engine.tui_command(&task, &project, &run) {
            Ok(c) => c,
            Err(e) => {
                self.note = Some(format!("{e:#}").into());
                cx.notify();
                return;
            }
        };
        let live = self.chat.update(cx, |c, cx| c.take_live(cx));
        // Nothing is driving the session: the TUI opens on this click.
        if live.is_none() && self.engine.claim_terminal(task.id).is_ok() {
            self.view = ViewMode::Terminal;
            self.attach_terminal(cmd, window, cx);
            return;
        }
        self.busy = Some("Opening terminal…");
        self.view = ViewMode::Terminal;
        let engine = self.engine.clone();
        let id = task.id;
        cx.spawn_in(window, async move |this, cx| {
            if let Some(live) = live {
                live.send(dove_core::Command::Kill);
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                loop {
                    if std::time::Instant::now() >= deadline {
                        break;
                    }
                    match live.events.try_recv() {
                        Ok(AgentEvent::Exited { .. }) | Err(flume::TryRecvError::Disconnected) => break,
                        Ok(_) => {}
                        Err(flume::TryRecvError::Empty) => {
                            cx.background_executor().timer(Duration::from_millis(10)).await;
                        }
                    }
                }
            }
            // The recorder drops the chat driver before it forwards Exited.
            let mut claimed = engine.claim_terminal(id);
            for _ in 0..8 {
                if claimed.is_ok() {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(15)).await;
                claimed = engine.claim_terminal(id);
            }
            let _ = this.update_in(cx, |v, window, cx| {
                v.busy = None;
                if let Err(e) = claimed {
                    v.note = Some(format!("{e:#}").into());
                    v.view = ViewMode::Chat;
                    cx.emit(ThreadEvent::Mode);
                    cx.notify();
                    return;
                }
                v.attach_terminal(cmd, window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// The terminal view for `cmd`, already claimed as this session's driver.
    fn attach_terminal(&mut self, cmd: dove_core::tui::TuiCommand, window: &mut Window, cx: &mut Context<Self>) {
        let term = cx.new(|cx| TerminalView::for_command(cmd.cwd.clone(), cmd.program.clone(), cmd.args.clone(), cx));
        let sub = cx.subscribe_in(&term, window, |v, _, ev: &TerminalEvent, window, cx| match ev {
            TerminalEvent::Exited => {
                if v.driver == ViewMode::Terminal {
                    v.note = Some("The agent's terminal exited; back to chat".into());
                    v.finish_to_chat(false, window, cx);
                }
            }
        });
        self._subs.push(sub);
        self.terminal = Some(term);
        self.driver = ViewMode::Terminal;
        self.view = ViewMode::Terminal;
        if !cmd.resumes {
            self.note = Some(
                "This agent can't reopen the chat's session in its own UI, so this is a new session in the same folder"
                    .into(),
            );
        }
        self.start_mirror(cx);
        self.chat.update(cx, |c, cx| c.set_terminal_banner(true, cx));
        self.focus_terminal(window, cx);
        cx.emit(ThreadEvent::Mode);
        cx.notify();
    }

    fn start_mirror(&mut self, cx: &mut Context<Self>) {
        let (task, project) = self.task(cx);
        let Some(mirror) = self.engine.mirror(&task, &project) else { return };
        let rx = mirror.events.clone();
        self.mirror = Some(mirror);
        let chat = self.chat.downgrade();
        self._mirror_pump = Some(cx.spawn(async move |_, cx| {
            while let Ok(first) = rx.recv_async().await {
                cx.background_executor().timer(Duration::from_millis(8)).await;
                let mut batch = vec![first];
                batch.extend(rx.try_iter());
                if chat.update(cx, |c, cx| c.apply_batch(batch, cx)).is_err() {
                    break;
                }
            }
        }));
    }

    fn switch_to_chat_now(&mut self, interrupt: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_to_chat(interrupt, window, cx);
    }

    /// Leave the TUI cleanly (exit command, then kill after a short grace).
    fn finish_to_chat(&mut self, interrupt: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (task, _) = self.task(cx);
        let term = self.terminal.clone();
        let running = term.as_ref().is_some_and(|t| t.read(cx).is_running());
        if !running {
            self.engine.release_terminal(task.id);
            self.land_on_chat(window, cx);
            return;
        }
        self.busy = Some("Closing terminal…");
        if let Some(t) = &term {
            t.update(cx, |t, _| {
                if interrupt {
                    t.send_raw("\u{1b}");
                }
                t.send_raw(dove_core::tui::exit_input(task.harness));
            });
        }
        let engine = self.engine.clone();
        cx.spawn_in(window, async move |this, cx| {
            let deadline = std::time::Instant::now() + EXIT_WAIT;
            loop {
                let running = term.as_ref().map(|t| t.read_with(cx, |t, _| t.is_running())).unwrap_or(false);
                if !running || std::time::Instant::now() >= deadline {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(20)).await;
            }
            if let Some(t) = &term {
                t.update(cx, |t, cx| t.kill(cx));
            }
            engine.release_terminal(task.id);
            let _ = this.update_in(cx, |v, window, cx| v.land_on_chat(window, cx));
        })
        .detach();
        cx.notify();
    }

    fn land_on_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = None;
        self.mirror = None;
        self._mirror_pump = None;
        self.terminal = None;
        self.driver = ViewMode::Chat;
        self.view = ViewMode::Chat;
        self.chat.update(cx, |c, cx| c.set_terminal_banner(false, cx));
        let composer = self.chat.read(cx).composer.clone();
        crate::composer::focus(&composer, window, cx);
        cx.emit(ThreadEvent::Mode);
        cx.notify();
    }

    fn focus_terminal(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = &self.terminal {
            let f = t.read(cx).focus_handle(cx);
            window.focus(&f, cx);
        }
    }

    /// Stop everything (thread deleted or archived).
    /// Whether this view can be dropped and rebuilt later from disk (tab
    /// hibernation): no agent session is attached, no terminal runs, and no
    /// hand-off between them is underway.
    pub fn can_hibernate(&self, cx: &gpui::App) -> bool {
        self.driver == ViewMode::Chat
            && self.terminal.as_ref().is_none_or(|t| !t.read(cx).is_running())
            && self.mirror.is_none()
            && self.sheet.is_none()
            && self.armed.is_none()
            && self.busy.is_none()
            && !self.chat.read(cx).is_live()
    }

    pub fn shutdown(&mut self, cx: &mut Context<Self>) {
        let (task, _) = self.task(cx);
        if let Some(t) = self.terminal.take() {
            t.update(cx, |t, cx| t.kill(cx));
        }
        self.mirror = None;
        self.engine.release_terminal(task.id);
        if let Some(live) = self.chat.update(cx, |c, cx| c.take_live(cx)) {
            live.send(dove_core::Command::Kill);
        }
    }

    fn render_sheet(&self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.sheet?;
        if !self.sheet_focus.is_focused(window) {
            window.focus(&self.sheet_focus, cx);
        }
        use crate::kit::controls::{self as ctl, Tone};
        use crate::kit::popover;
        let (title, body, later, now) = match p {
            Pending::ToTerminal => (
                "The agent is mid-turn",
                "Open its terminal when this turn ends, or interrupt it and switch now.",
                "Switch at end of turn",
                "Interrupt and switch now",
            ),
            Pending::ToChat => (
                "The terminal is busy",
                "Return to chat when the agent goes quiet, or interrupt it and switch now.",
                "Switch when it's quiet",
                "Interrupt and switch now",
            ),
        };
        // A kit dialog over the scrim: Enter waits for the turn, Esc or a
        // click outside cancels.
        let card = popover::dialog_card(t, 400.0)
            .id("switch-sheet")
            .role(gpui::Role::Dialog)
            .aria_label(title)
            .track_focus(&self.sheet_focus)
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| match e.keystroke.key.as_str() {
                "escape" => {
                    this.sheet = None;
                    cx.stop_propagation();
                    cx.notify();
                }
                "enter" => {
                    this.choose(false, window, cx);
                    cx.stop_propagation();
                }
                _ => {}
            }))
            .child(popover::dialog_body(t, title, body))
            .child(
                div()
                    .px(px(20.0))
                    .pt(px(12.0))
                    .pb(px(18.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        ctl::button(t, "sheet-later", later, Tone::Solid)
                            .w_full()
                            .on_click(cx.listener(|this, _, window, cx| this.choose(false, window, cx))),
                    )
                    .child(
                        ctl::button(t, "sheet-now", now, Tone::Outlined)
                            .w_full()
                            .on_click(cx.listener(|this, _, window, cx| this.choose(true, window, cx))),
                    )
                    .child(ctl::button(t, "sheet-cancel", "Cancel", Tone::Quiet).w_full().on_click(cx.listener(
                        |this, _, _, cx| {
                            this.sheet = None;
                            cx.notify();
                        },
                    ))),
            );
        let weak = cx.weak_entity();
        Some(popover::modal(
            "switch-sheet-layer",
            window.viewport_size(),
            t.scrim,
            None,
            card.into_any_element(),
            Some(Box::new(move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.sheet = None;
                    cx.notify();
                });
            })),
        ))
    }

    /// Chat | Terminal segment. The title bar no longer shows it; Ctrl+`
    /// and the command palette still flip the view.
    #[allow(dead_code)]
    pub fn view_switch(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.view;
        let armed = self.armed.is_some();
        // The selection plate slides between the two slots (the shell draws
        // this switch, so its frames carry the glide).
        let slot = crate::motion::state_t(
            &format!("view-switch-{}", cx.entity_id()),
            mode == ViewMode::Terminal,
            crate::motion::INDICATOR_SLIDE,
            cx.reduce_motion(),
        );
        let plate = div()
            .absolute()
            .top_0()
            .left(px(27.0 * slot))
            .h(px(22.0))
            .w(px(26.0))
            .rounded(px(radius::fit(22.0)))
            .bg(t.wash(0.10));
        let item =
            |id: &'static str, glyph: &'static str, label: &'static str, want: ViewMode, cx: &mut Context<Self>| {
                let on = mode == want;
                div()
                    .id(id)
                    .relative()
                    .h(px(22.0))
                    .w(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(radius::fit(22.0)))
                    .cursor_pointer()
                    .role(gpui::Role::Tab)
                    .aria_selected(on)
                    .aria_label(label)
                    .tooltip(ui::tip(label))
                    .child(icon(glyph).size(px(13.5)).text_color(if on { t.text } else { t.text_faint }))
                    .when(armed && want == ViewMode::Terminal && !on, |d| {
                        d.child(div().absolute().top(px(3.0)).right(px(3.0)).child(ui::dot(t.warning, 4.0)))
                    })
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.request(want, window, cx);
                    }))
            };
        div()
            .id("view-switch")
            .occlude()
            .relative()
            .flex()
            .items_center()
            .gap(px(1.0))
            .child(plate)
            .child(item("view-chat", icons::CHAT_ROUND_LINE, "Chat (Ctrl+`)", ViewMode::Chat, cx))
            .child(item("view-terminal", icons::TERMINAL, "Terminal (Ctrl+`)", ViewMode::Terminal, cx))
            .into_any_element()
    }

    fn render_terminal(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(term) = &self.terminal else {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.0))
                .child(kit::spinner("term-opening", 18.0, t.text_muted))
                .child(
                    div().text_size(ty::BODY).text_color(t.text_muted).child(self.busy.unwrap_or("Opening terminal…")),
                )
                .into_any_element();
        };
        let (task, project) = self.task(cx);
        let running = term.read(cx).is_running();
        let cwd = term.read(cx).cwd().to_path_buf();
        let cwd_for_ext = cwd.clone();
        let term_copy = term.clone();
        let term_clear = term.clone();
        let term_restart = term.clone();
        let voice = self.voice.read(cx);
        let recording = voice.is_recording();
        let busy = voice.is_transcribing();
        let ready = voice.is_ready();
        let (press, release) = (self.voice.clone(), self.voice.clone());

        let mic = div()
            .id("term-mic")
            .h(px(24.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(5.0))
            .rounded(px(radius::fit(24.0)))
            .border_1()
            .border_color(if recording { t.danger.opacity(0.5) } else { t.border })
            .bg(if recording { t.danger.opacity(0.16) } else { t.wash(0.05) })
            .cursor_pointer()
            .text_size(ty::CAPTION)
            .text_color(if recording { t.danger } else { t.text_muted })
            .tooltip(ui::tip(if ready {
                "Hold or click to dictate into the terminal"
            } else {
                "Install local voice in Settings › Voice"
            }))
            .child(if busy {
                kit::spinner("term-mic-busy", 12.0, t.text_muted).into_any_element()
            } else {
                icon(icons::MICROPHONE)
                    .size(px(12.5))
                    .text_color(if recording { t.danger } else { t.text_muted })
                    .into_any_element()
            })
            .child(if recording { "Listening…" } else { "Voice" })
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                press.update(cx, |v, cx| v.press(cx));
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| release.update(cx, |v, cx| v.release(cx)));

        let header = div()
            .h(px(34.0))
            .px(px(12.0))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.0))
            .border_b_1()
            .border_color(t.border)
            .bg(t.wash(0.025))
            .text_size(ty::META)
            .child(ui::dot(if running { t.success } else { t.text_faint }, 6.5))
            .child(icon(ui::harness_icon(task.harness)).size(px(13.0)).text_color(ui::harness_tint(
                task.harness,
                t,
                t.text_muted,
            )))
            .child(
                div()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(t.text)
                    .child(format!("{} TUI", task.harness.label())),
            )
            .child(div().text_color(t.text_faint).child("·"))
            .child(div().text_color(t.text_muted).truncate().child(project.name))
            .when_some(task.branch.clone(), |d, b| {
                d.child(
                    div()
                        .h(px(20.0))
                        .px(px(6.0))
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .rounded(px(radius::fit(20.0)))
                        .bg(t.wash(0.05))
                        .font_family(t.font_mono.clone())
                        .text_size(ty::MICRO)
                        .text_color(t.text_muted)
                        .child(icon(icons::GIT_BRANCH).size(px(10.5)).text_color(t.text_faint))
                        .child(b),
                )
            })
            .child(div().flex_1())
            .child(mic)
            .child(ui::row_button("term-copy", icons::COPY, "Copy selection or screen", t).on_click(move |_, _, cx| {
                term_copy.update(cx, |v, cx| v.copy_selection_or_screen(cx));
            }))
            .child(ui::row_button("term-clear", icons::REFRESH, "Clear screen", t).on_click(move |_, _, cx| {
                term_clear.update(cx, |v, cx| v.clear(cx));
            }))
            .when(!running, |d| {
                d.child(ui::button_ghost("term-restart", "Restart", t).h(px(24.0)).text_size(ty::CAPTION).on_click(
                    move |_, _, cx| {
                        term_restart.update(cx, |v, cx| v.restart(cx));
                    },
                ))
            })
            .child(
                ui::row_button("term-ext", icons::ARROW_UP_RIGHT, "Open in system terminal", t)
                    .on_click(move |_, _, _| crate::terminal::open_system_terminal(&cwd_for_ext)),
            );

        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(div().flex_1().min_h_0().child(term.clone()))
            .into_any_element()
    }
}

impl Render for ThreadView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("ThreadView");
        let t = Theme::of(cx).clone();
        let body: AnyElement = match self.view {
            // The switch lives in the title bar, so the transcript gets the
            // whole card.
            ViewMode::Chat => div().size_full().child(self.chat.clone()).into_any_element(),
            ViewMode::Terminal => self.render_terminal(&t, cx),
        };
        let note = self.note.clone().or(self.busy.map(SharedString::from));
        div()
            .id("thread")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .when_some(note, |d, n| {
                d.child(
                    div()
                        .id("thread-note")
                        .h(px(30.0))
                        .px(px(14.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .bg(t.accent.opacity(0.08))
                        .border_b_1()
                        .border_color(t.border)
                        .text_size(ty::SMALL)
                        .text_color(t.text_muted)
                        .child(icon(icons::INFO_CIRCLE).size(px(13.0)).text_color(t.accent))
                        .child(div().flex_1().truncate().child(n))
                        .child(
                            div()
                                .id("thread-note-x")
                                .cursor_pointer()
                                .child(icon(icons::CLOSE).size(px(11.0)).text_color(t.text_faint))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.note = None;
                                    if this.armed.is_some() {
                                        this.armed = None;
                                        this._watch = None;
                                    }
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(div().flex_1().min_h_0().child(body))
            .children(self.render_sheet(&t, window, cx))
    }
}
