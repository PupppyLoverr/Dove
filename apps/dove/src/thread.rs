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
    IntoElement, MouseButton, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    StyleRefinement, Styled as _, Subscription, Window, div, px,
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
    PullRequest(String),
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
    /// The terminal's side pill.
    pill: crate::term_pill::PillState,
    /// Text typed into the terminal as a draft (not yet entered), so it can
    /// be replaced or cleared.
    term_draft: Option<String>,
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
        chat.update(cx, |c, _| c.dock_outside = true);
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
                TaskViewEvent::PullRequest(url) => TaskViewEventKind::PullRequest(url.clone()),
                // Voice Mode subscribes to the chat directly.
                TaskViewEvent::VoiceText(_) | TaskViewEvent::VoiceTurnEnded { .. } => return,
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
            // Text typed into a terminal can't be revised in place, so live
            // dictation types only its final text there.
            DictationEvent::Text(text) | DictationEvent::LiveDone(text) => {
                if let Some(t) = &this.terminal
                    && !text.is_empty()
                {
                    let text = text.clone();
                    t.update(cx, |t, cx| t.type_text(&text, cx));
                }
            }
            DictationEvent::Live(_) => {}
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
            pill: Default::default(),
            term_draft: None,
            mirror: None,
            focus: cx.focus_handle(),
            _mirror_pump: None,
            _watch: None,
            _subs: subs,
        }
    }

    /// Open the agent's terminal UI for a thread that has not run yet, and
    /// type `first` into it once the UI settles (a voice launch).
    pub fn open_terminal(&mut self, first: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.request(ViewMode::Terminal, window, cx);
        let Some(first) = first.filter(|f| !f.trim().is_empty()) else { return };
        cx.spawn_in(window, async move |this, cx| {
            // Wait for the UI to draw and go quiet (at most 20 s).
            let start = std::time::Instant::now();
            loop {
                cx.background_executor().timer(Duration::from_millis(250)).await;
                let ready = this
                    .update(cx, |v, cx| {
                        v.terminal.as_ref().is_some_and(|t| {
                            let t = t.read(cx);
                            t.is_running() && t.is_quiet(Duration::from_millis(1200))
                        })
                    })
                    .unwrap_or(true);
                if ready && start.elapsed() > Duration::from_millis(1500) || start.elapsed() > Duration::from_secs(20) {
                    break;
                }
            }
            let _ = this.update(cx, |v, cx| {
                if let Some(t) = &v.terminal {
                    t.update(cx, |t, cx| {
                        t.type_text(&first, cx);
                        t.send_raw("\r");
                    });
                }
            });
        })
        .detach();
    }

    /// Put `text` in the thread's input without sending it: the composer
    /// in chat, the agent's prompt line in the terminal (replacing an
    /// earlier draft there).
    pub fn draft(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        match (&self.terminal, self.view) {
            (Some(term), ViewMode::Terminal) => {
                let term = term.clone();
                let old = self.term_draft.replace(text.to_string());
                term.update(cx, |t, cx| {
                    if let Some(old) = old {
                        t.send_raw(&"\x7f".repeat(old.chars().count()));
                    }
                    t.type_text(text, cx);
                });
            }
            _ => {
                let composer = self.chat.read(cx).composer.clone();
                crate::composer::set_text(&composer, text, window, cx);
                crate::composer::focus(&composer, window, cx);
            }
        }
        cx.notify();
    }

    /// Send what [`Self::draft`] put in the input.
    pub fn send_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match (&self.terminal, self.view) {
            (Some(term), ViewMode::Terminal) => {
                self.term_draft = None;
                term.update(cx, |t, _| t.send_raw("\r"));
                true
            }
            _ => {
                let composer = self.chat.read(cx).composer.clone();
                let Some(text) = crate::composer::take_text(&composer, window, cx) else { return false };
                self.chat.update(cx, |c, cx| c.submit(text, window, cx));
                true
            }
        }
    }

    /// Take back what [`Self::draft`] put in the input.
    pub fn clear_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match (&self.terminal, self.view) {
            (Some(term), ViewMode::Terminal) => {
                if let Some(old) = self.term_draft.take() {
                    term.update(cx, |t, _| t.send_raw(&"\x7f".repeat(old.chars().count())));
                }
            }
            _ => {
                let composer = self.chat.read(cx).composer.clone();
                let _ = crate::composer::take_text(&composer, window, cx);
            }
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
        // A pre-started copy of this session would write the same file.
        if let Some(native) = task.native_session_id.as_deref() {
            self.engine.cool_session(native);
        }
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
        // The view let go of its session but the agent still runs it in the
        // background: stop it there.
        if live.is_none() {
            engine.stop_chat(id);
        }
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
            // The recorder drops the chat driver when the agent exits; give
            // a slow exit (flushing its session file) up to three seconds.
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let mut claimed = engine.claim_terminal(id);
            while claimed.is_err() && std::time::Instant::now() < deadline {
                cx.background_executor().timer(Duration::from_millis(25)).await;
                claimed = engine.claim_terminal(id);
            }
            let claimed = claimed.map_err(|_| {
                anyhow::anyhow!(
                    "The chat session didn't stop in time, so the terminal didn't open. Try again in a moment."
                )
            });
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
        let term = cx.new(|cx| {
            TerminalView::for_command_env(cmd.cwd.clone(), cmd.program.clone(), cmd.args.clone(), cmd.env.clone(), cx)
        });
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
        // The store may know the native session id this view missed.
        let (task, project) = self.session_task(cx);
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

    /// Chat | Terminal segment in the thread's title bar (Ctrl+` and the
    /// command palette flip it too). Agents with no terminal UI get a
    /// switch that says so instead of failing.
    pub fn view_switch(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.view;
        let armed = self.armed.is_some();
        let harness = self.chat.read(cx).task.harness;
        let has_terminal = self.engine.has_terminal_ui(harness);
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
            .child(if has_terminal || mode == ViewMode::Terminal {
                item("view-terminal", icons::TERMINAL, "Terminal (Ctrl+`)", ViewMode::Terminal, cx).into_any_element()
            } else {
                div()
                    .id("view-terminal-off")
                    .h(px(22.0))
                    .w(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(0.4)
                    .tooltip(ui::tip(format!("{} has no terminal UI Dove can open", harness.label())))
                    .child(icon(icons::TERMINAL).size(px(13.5)).text_color(t.text_faint))
                    .into_any_element()
            })
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

        // The terminal gets every row: what used to be a header bar is the
        // side pill (see `term_pill`), opening into the thread, the way back
        // to chat, dictation, a voice call and the terminal's own actions.
        use crate::term_pill::{self, Item, Pill};
        let me = cx.entity().downgrade();
        let to_chat = me.clone();
        let mut items = vec![
            Item::new("term-to-chat", icons::CHAT_ROUND_LINE, "Back to chat", move |window, cx| {
                let _ = to_chat.update(cx, |v, cx| v.request(ViewMode::Chat, window, cx));
            })
            .hint("Ctrl+`"),
        ];
        items.push(if ready {
            let item = Item::hold(
                "term-mic",
                icons::MICROPHONE,
                if busy {
                    "Transcribing…"
                } else if recording {
                    "Listening… release to type"
                } else {
                    "Hold to dictate"
                },
                move |_, cx| press.update(cx, |v, cx| v.press(cx)),
                move |_, cx| release.update(cx, |v, cx| v.release(cx)),
            );
            if recording { item.tone(t.danger) } else { item }
        } else {
            Item::new("term-mic", icons::MICROPHONE, "Set up dictation in Settings › Voice", |window, cx| {
                window.dispatch_action(Box::new(crate::actions::OpenSettings), cx)
            })
        });
        items.push(
            Item::new("term-call", icons::PHONE, "Voice call with this thread", |window, cx| {
                window.dispatch_action(Box::new(crate::actions::ToggleVoiceCall), cx)
            })
            .hint("Ctrl+Shift+Space"),
        );
        items.push(Item::new("term-copy", icons::COPY, "Copy selection or screen", move |_, cx| {
            term_copy.update(cx, |v, cx| v.copy_selection_or_screen(cx));
        }));
        items.push(Item::new("term-clear", icons::REFRESH, "Clear screen", move |_, cx| {
            term_clear.update(cx, |v, cx| v.clear(cx));
        }));
        if !running {
            items.push(
                Item::new("term-restart", icons::RESTART, "Restart", move |_, cx| {
                    term_restart.update(cx, |v, cx| v.restart(cx));
                })
                .tone(t.warning),
            );
        }
        items.push(Item::new("term-ext", icons::ARROW_UP_RIGHT, "Open in system terminal", move |_, _| {
            crate::terminal::open_system_terminal(&cwd_for_ext)
        }));
        let detail = match &task.branch {
            Some(b) => format!("{} · {} · {b}", task.harness.label(), project.name),
            None => format!("{} · {}", task.harness.label(), project.name),
        };
        let pill = term_pill::render(
            Pill {
                id: "term-pill".into(),
                open: self.pill.open,
                live: running,
                glyph: ui::harness_icon(task.harness),
                tint: ui::harness_tint(task.harness, t, t.text_muted),
                title: task.title.clone().into(),
                detail: detail.into(),
                recording,
            },
            items,
            cx.listener(|this, hovered: &bool, _, cx| term_pill::hover(this, *hovered, |v: &mut Self| &mut v.pill, cx)),
            t,
        );

        div().relative().size_full().child(term.clone()).child(term_pill::overlay(pill)).into_any_element()
    }
}

impl Render for ThreadView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("ThreadView");
        let t = Theme::of(cx).clone();
        let body: AnyElement = match self.view {
            // The switch lives in the title bar, so the transcript gets the
            // whole card.
            // The transcript is cached; its composer dock is drawn here,
            // over its bottom edge, so a transcript tick leaves the
            // composer's last frame in place.
            ViewMode::Chat => {
                let dock = self.chat.update(cx, |c, cx| c.render_dock(window, cx));
                div()
                    .relative()
                    .size_full()
                    .child(self.chat.clone().cached(StyleRefinement::default().size_full()))
                    .children(dock)
                    .into_any_element()
            }
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
