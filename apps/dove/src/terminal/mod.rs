//! Terminals: in-app shells, an agent's own TUI for Chat ⇄ Terminal, and a
//! launcher for the platform terminal.

pub mod view;

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use dove_term::emulator::SelectionType;
use dove_term::{Emulator, Pty};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Bounds, ClipboardItem, Context, EntityInputHandler, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement as _, Pixels, Point, Render,
    ScrollWheelEvent, Styled as _, UTF16Selection, Window, div, point, px, size,
};

use crate::theme::Theme;
use view::{GridGeometry, TerminalElement, cell_at, keystroke_bytes, paste_bytes};

/// Batch PTY output per frame.
const FRAME: Duration = Duration::from_millis(8);

/// What a terminal runs.
#[derive(Clone, Debug)]
pub enum Program {
    Shell,
    /// An agent's TUI: `program args…`.
    Command {
        program: PathBuf,
        args: Vec<String>,
    },
}

pub enum TerminalEvent {
    /// The program exited.
    Exited,
}

pub struct TerminalView {
    cwd: PathBuf,
    program: Program,
    /// When the program last printed anything.
    pub last_output: Instant,
    pty: Option<Pty>,
    pub(crate) emu: Emulator,
    geometry: GridGeometry,
    focus: FocusHandle,
    error: Option<String>,
    selecting: bool,
    _pump: Option<gpui::Task<()>>,
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl TerminalView {
    pub fn new(cwd: PathBuf, cx: &mut Context<Self>) -> Self {
        Self::with_program(cwd, Program::Shell, cx)
    }

    /// Run an agent's TUI instead of a shell.
    pub fn for_command(cwd: PathBuf, program: PathBuf, args: Vec<String>, cx: &mut Context<Self>) -> Self {
        Self::with_program(cwd, Program::Command { program, args }, cx)
    }

    fn with_program(cwd: PathBuf, program: Program, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            cwd,
            program,
            last_output: Instant::now(),
            pty: None,
            emu: Emulator::new(80, 24),
            geometry: GridGeometry::default(),
            focus: cx.focus_handle(),
            error: None,
            selecting: false,
            _pump: None,
        };
        this.start(cx);
        this
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let (cols, rows) = (self.emu.cols() as u16, self.emu.rows() as u16);
        let spawned = match &self.program {
            Program::Shell => Pty::spawn(&self.cwd, cols, rows),
            Program::Command { program, args } => Pty::spawn_command(program, args, &self.cwd, cols, rows),
        };
        match spawned {
            Ok(pty) => {
                let rx = pty.output.clone();
                self.pty = Some(pty);
                self.error = None;
                self._pump = Some(cx.spawn(async move |this, cx| {
                    while let Ok(first) = rx.recv_async().await {
                        cx.background_executor().timer(FRAME).await;
                        let mut bytes = first;
                        for more in rx.try_iter() {
                            bytes.extend(more);
                        }
                        let alive = this.update(cx, |v, cx| {
                            v.last_output = Instant::now();
                            let replies = v.emu.feed(&bytes);
                            if !replies.is_empty()
                                && let Some(p) = v.pty.as_mut()
                            {
                                p.write(&replies);
                            }
                            cx.notify();
                        });
                        if alive.is_err() {
                            return;
                        }
                    }
                    // The reader ended: the program exited.
                    let _ = this.update(cx, |v, cx| {
                        v.pty = None;
                        cx.emit(TerminalEvent::Exited);
                        cx.notify();
                    });
                }));
            }
            Err(e) => self.error = Some(format!("{e:#}")),
        }
    }

    /// Called from prepaint with the measured grid. Resizes immediately; the
    /// shell sees SIGWINCH / a ConPTY resize.
    pub(crate) fn on_grid_metrics(&mut self, g: GridGeometry) {
        let changed = g.cols != self.geometry.cols || g.rows != self.geometry.rows;
        self.geometry = g;
        if changed && g.cols > 0 && g.rows > 0 {
            self.emu.resize(g.cols, g.rows);
            if let Some(p) = &self.pty {
                p.resize(g.cols, g.rows);
            }
        }
    }

    pub fn is_running(&self) -> bool {
        self.pty.is_some()
    }

    /// Whether the program has printed nothing for `quiet`.
    pub fn is_quiet(&self, quiet: Duration) -> bool {
        self.last_output.elapsed() >= quiet
    }

    /// Type text (dictation, a scripted exit) as if from the keyboard.
    pub fn type_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let bytes = paste_bytes(text, self.emu.bracketed_paste_mode());
        self.write(&bytes);
        cx.notify();
    }

    /// Send raw input (control sequences, an exit command).
    pub fn send_raw(&mut self, text: &str) {
        self.write(text.as_bytes());
    }

    /// Stop the program now.
    pub fn kill(&mut self, cx: &mut Context<Self>) {
        self._pump = None;
        self.pty = None;
        cx.notify();
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Clear screen and reset scrollback.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.emu = Emulator::new(self.geometry.cols.max(2), self.geometry.rows.max(1));
        if let Some(p) = self.pty.as_mut() {
            p.write(b"\x0c");
        }
        cx.notify();
    }

    /// Copy the active selection, or all visible screen text if nothing is selected.
    pub fn copy_selection_or_screen(&self, cx: &mut Context<Self>) {
        let text = self.emu.selection_text().unwrap_or_else(|| {
            self.emu
                .lines()
                .into_iter()
                .map(|row| row.into_iter().map(|c| c.ch).collect::<String>().trim_end().to_string())
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string()
        });
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Restart the shell or command in place.
    pub fn restart(&mut self, cx: &mut Context<Self>) {
        self._pump = None;
        self.pty = None;
        self.emu = Emulator::new(self.geometry.cols.max(2), self.geometry.rows.max(1));
        self.start(cx);
        cx.notify();
    }

    fn write(&mut self, bytes: &[u8]) {
        if let Some(p) = self.pty.as_mut() {
            p.write(bytes);
        }
        self.emu.scroll_to_bottom();
    }

    fn key_down(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &e.keystroke;
        let m = &ks.modifiers;
        // Ctrl+Shift+C / V: copy and paste, as in most terminals.
        if m.control && m.shift && ks.key == "c" {
            if let Some(text) = self.emu.selection_text() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            cx.stop_propagation();
            return;
        }
        if m.control && m.shift && ks.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                let bytes = paste_bytes(&text, self.emu.bracketed_paste_mode());
                self.write(&bytes);
            }
            cx.stop_propagation();
            return;
        }
        if self.pty.is_none() && ks.key == "enter" && matches!(self.program, Program::Shell) {
            self.emu = Emulator::new(self.geometry.cols.max(2), self.geometry.rows.max(1));
            self.start(cx);
            cx.notify();
            return;
        }
        // Committed text (letters, AltGr and dead-key characters, IME) arrives
        // through the input handler; handling it here as well would type it twice.
        let printable = ks.key_char.as_deref().is_some_and(|c| !c.is_empty() && !c.chars().any(char::is_control));
        if printable && ((!m.control && !m.alt && !m.platform) || e.prefer_character_input) {
            return;
        }
        if let Some(bytes) = keystroke_bytes(&ks.key, ks.key_char.as_deref(), m, self.emu.app_cursor_mode()) {
            self.emu.clear_selection();
            self.write(&bytes);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn hit(&self, pos: gpui::Point<gpui::Pixels>) -> view::CellHit {
        let g = self.geometry;
        cell_at(
            f32::from(pos.x - g.origin.x),
            f32::from(pos.y - g.origin.y),
            g.cell_w,
            g.line_h,
            g.cols as usize,
            g.rows as usize,
        )
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let h = self.hit(e.position);
        let ty = match e.click_count {
            2 => SelectionType::Semantic,
            3 => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        let point = self.emu.grid_point(h.row, h.col);
        self.emu.start_selection(ty, point, h.side);
        self.selecting = true;
        cx.notify();
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting && e.pressed_button == Some(MouseButton::Left) {
            let h = self.hit(e.position);
            let point = self.emu.grid_point(h.row, h.col);
            self.emu.update_selection(point, h.side);
            cx.notify();
        }
    }

    fn scroll(&mut self, e: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let line_h = self.geometry.line_h.max(1.0);
        let dy = f32::from(e.delta.pixel_delta(gpui::px(line_h)).y);
        let lines = (dy / line_h).round() as i32;
        if lines != 0 {
            self.emu.scroll(lines);
            cx.notify();
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(cx).clone();
        let focused = self.focus.is_focused(window);
        let entity = cx.entity();
        div()
            .id("terminal")
            .track_focus(&self.focus)
            .key_context("Terminal")
            .size_full()
            .relative()
            .bg(if t.is_frost() { t.terminal_bg.opacity(0.55) } else { t.terminal_bg })
            .cursor(gpui::CursorStyle::IBeam)
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.selecting = false))
            .on_scroll_wheel(cx.listener(Self::scroll))
            .child(TerminalElement::new(entity, self.focus.clone(), focused))
            .when(self.pty.is_none(), |d| {
                d.child(
                    div()
                        .absolute()
                        .bottom(px(10.0))
                        .left(px(12.0))
                        .text_size(px(12.0))
                        .text_color(t.text_faint)
                        .child(match (&self.error, &self.program) {
                            (Some(e), _) => format!("Terminal failed: {e}"),
                            (None, Program::Shell) => "Shell exited · press Enter to restart".into(),
                            (None, Program::Command { .. }) => "The agent exited".into(),
                        }),
                )
            })
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: 0..0, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        self.emu.clear_selection();
        self.write(text.as_bytes());
        cx.notify();
    }

    /// IME composition in progress: nothing reaches the shell until commit.
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        _: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    /// Place the IME candidate window at the cursor.
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let g = self.geometry;
        let c = self.emu.cursor()?;
        Some(Bounds::new(
            point(g.origin.x + px(g.cell_w * c.col as f32), g.origin.y + px(g.line_h * c.row as f32)),
            size(px(g.cell_w), px(g.line_h)),
        ))
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}

/// Open the platform's terminal in `dir`. Best effort: failures are logged.
pub fn open_system_terminal(dir: &Path) {
    let result = if cfg!(windows) {
        Command::new("wt.exe")
            .arg("-d")
            .arg(dir)
            .spawn()
            .or_else(|_| Command::new("cmd.exe").args(["/c", "start", "cmd.exe"]).current_dir(dir).spawn())
            .map(|_| ())
    } else if cfg!(target_os = "macos") {
        Command::new("open").args(["-a", "Terminal"]).arg(dir).spawn().map(|_| ())
    } else {
        ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"]
            .iter()
            .find_map(|t| Command::new(t).current_dir(dir).spawn().ok())
            .map(|_| ())
            .ok_or_else(|| std::io::Error::other("no terminal emulator found"))
    };
    if let Err(e) = result {
        tracing::warn!("could not open a terminal in {}: {e}", dir.display());
    }
}
