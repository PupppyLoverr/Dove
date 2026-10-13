//! Terminals for Dove: a pure emulator (bytes in, grid out) and a PTY
//! session running the user's shell. No UI types here.
//!
//! `emulator` is adapted from Zeron (MIT, https://github.com/zeronsh/zeron).

pub mod emulator;

#[cfg(windows)]
mod win;

use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, MasterPty, PtySize};

pub use emulator::Emulator;

/// A shell running on a PTY. Output arrives on [`Pty::output`]; dropping the
/// session kills the shell.
pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    pub output: flume::Receiver<Vec<u8>>,
}

/// The user's interactive shell for this platform.
pub fn default_shell() -> CommandBuilder {
    if cfg!(windows) {
        // PowerShell 7 when installed, else Windows PowerShell.
        let pwsh = std::env::var_os("PATH")
            .into_iter()
            .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .map(|d| d.join("pwsh.exe"))
            .find(|p| p.is_file());
        let mut c = CommandBuilder::new(pwsh.map(|p| p.into_os_string()).unwrap_or_else(|| "powershell.exe".into()));
        c.arg("-NoLogo");
        c
    } else {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        let mut c = CommandBuilder::new(shell);
        c.arg("-l");
        c
    }
}

/// `program args…`, wrapped in `cmd.exe /d /c` for Windows batch shims
/// (npm installs agents as `claude.cmd`).
pub fn program_command(program: &Path, args: &[String]) -> CommandBuilder {
    let ext = program.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let mut c = if cfg!(windows) && (ext == "cmd" || ext == "bat") {
        let mut c = CommandBuilder::new("cmd.exe");
        c.arg("/d");
        c.arg("/c");
        c.arg(program);
        c
    } else {
        CommandBuilder::new(program)
    };
    for a in args {
        c.arg(a);
    }
    // Dove may itself run under an agent; its child-session markers must not
    // leak into the user's own sessions (Claude stops saving transcripts).
    for var in ["CLAUDE_CODE_CHILD_SESSION", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SSE_PORT"] {
        c.env_remove(var);
    }
    c
}

impl Pty {
    /// The user's shell in `cwd`.
    pub fn spawn(cwd: &Path, cols: u16, rows: u16) -> Result<Self> {
        Self::spawn_with(default_shell(), cwd, cols, rows)
    }

    /// Any program (an agent's TUI) in `cwd`.
    pub fn spawn_command(program: &Path, args: &[String], cwd: &Path, cols: u16, rows: u16) -> Result<Self> {
        Self::spawn_with(program_command(program, args), cwd, cols, rows)
    }

    fn spawn_with(mut cmd: CommandBuilder, cwd: &Path, cols: u16, rows: u16) -> Result<Self> {
        cmd.cwd(cwd);
        // Advertise a color terminal. NO_COLOR in the parent environment (some
        // agent hosts set it) would otherwise force a monochrome renderer even
        // after the child is attached to a real console.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "Dove");
        cmd.env_remove("NO_COLOR");
        cmd.env("FORCE_COLOR", "1");
        cmd.env("CLICOLOR", "1");
        cmd.env("CLICOLOR_FORCE", "1");
        let size = PtySize { rows: rows.max(1), cols: cols.max(2), pixel_width: 0, pixel_height: 0 };
        let (master, child) = open_session(&cmd, size)?;
        let mut reader = master.try_clone_reader().context("reading the pty")?;
        let writer = master.take_writer().context("writing the pty")?;
        let (tx, rx) = flume::unbounded();
        std::thread::Builder::new()
            .name("dove-pty".into())
            .spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                    }
                }
            })
            .context("spawning the pty reader")?;
        Ok(Self { master, writer, child, output: rx })
    }

    pub fn write(&mut self, bytes: &[u8]) {
        if let Err(e) = self.writer.write_all(bytes).and_then(|_| self.writer.flush()) {
            tracing::debug!("pty write failed: {e}");
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.master.resize(PtySize { rows: rows.max(1), cols: cols.max(2), pixel_width: 0, pixel_height: 0 });
    }

    pub fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

/// Windows uses a ConPTY with the child's standard handles left invalid, so
/// the pseudoconsole is the console (`isatty` is true and programs emit VT
/// color and Unicode). portable-pty's launcher points those handles at the
/// pipes, and programs then fall back to a dumb monochrome stream.
fn open_session(
    cmd: &CommandBuilder,
    size: PtySize,
) -> Result<(Box<dyn MasterPty + Send>, Box<dyn portable_pty::Child + Send + Sync>)> {
    #[cfg(windows)]
    {
        win::open(cmd, size).context("starting the terminal")
    }
    #[cfg(not(windows))]
    {
        let pair = portable_pty::native_pty_system().openpty(size).context("opening a pseudo-terminal")?;
        let child = pair.slave.spawn_command(cmd.clone()).context("starting the program")?;
        drop(pair.slave);
        Ok((pair.master, child))
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_round_trip() {
        let dir = std::env::temp_dir();
        let mut pty = Pty::spawn(&dir, 80, 24).expect("spawn shell");
        let mut emu = Emulator::new(80, 24);
        pty.write(b"echo dove-$((40+2))-ok\r");
        if cfg!(windows) {
            pty.write(b"echo dove-42-ok\r");
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut seen = false;
        while std::time::Instant::now() < deadline && !seen {
            if let Ok(bytes) = pty.output.recv_timeout(std::time::Duration::from_millis(200)) {
                let replies = emu.feed(&bytes);
                if !replies.is_empty() {
                    pty.write(&replies);
                }
                seen = (0..24).any(|r| emu.row_text(r).trim_start().starts_with("dove-42-ok"));
            }
        }
        assert!(seen, "shell output never showed the marker");
    }

    /// A real console emits VT color and Unicode. A pipe-backed spawn prints
    /// neither: PowerShell reports the output as redirected and Write-Host
    /// stays on the default foreground.
    #[cfg(windows)]
    #[test]
    fn console_keeps_color_and_symbols() {
        let mut cmd = CommandBuilder::new("powershell.exe");
        cmd.arg("-NoLogo");
        cmd.arg("-NoProfile");
        cmd.arg("-Command");
        cmd.arg(
            "if ([Console]::IsOutputRedirected) { Write-Host 'REDIRECTED' } else { Write-Host 'TTY-MARK' }; Write-Host 'RED' -ForegroundColor Red; Write-Host ([char]0x2713)",
        );
        let dir = std::env::temp_dir();
        let mut pty = Pty::spawn_with(cmd, &dir, 80, 24).expect("spawn powershell");
        let mut emu = Emulator::new(80, 24);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut saw_tty = false;
        let mut saw_redirected = false;
        let mut saw_red = false;
        let mut saw_mark = false;
        while std::time::Instant::now() < deadline && !(saw_tty && saw_red && saw_mark) {
            if let Ok(bytes) = pty.output.recv_timeout(std::time::Duration::from_millis(200)) {
                let replies = emu.feed(&bytes);
                if !replies.is_empty() {
                    pty.write(&replies);
                }
                for row in 0..24 {
                    let text = emu.row_text(row);
                    if text.contains("TTY-MARK") {
                        saw_tty = true;
                    }
                    if text.contains("REDIRECTED") {
                        saw_redirected = true;
                    }
                    if text.contains('✓') {
                        saw_mark = true;
                    }
                }
                saw_red |= emu.lines().iter().any(|row| row_has_red_word(row));
            }
        }
        let screen: String = (0..24).map(|r| emu.row_text(r)).collect::<Vec<_>>().join("\n");
        assert!(!saw_redirected, "child did not see a console:\n{screen}");
        assert!(saw_tty, "console marker never arrived:\n{screen}");
        assert!(saw_red, "RED was not painted red:\n{screen}");
        assert!(saw_mark, "check mark never arrived:\n{screen}");
    }

    #[cfg(windows)]
    fn row_has_red_word(row: &[emulator::CellSnapshot]) -> bool {
        let text: String = row.iter().map(|c| c.ch).collect();
        let Some(at) = text.find("RED") else {
            return false;
        };
        let col = text[..at].chars().count();
        (0..3).all(|i| row.get(col + i).is_some_and(|c| is_red(c.display_colors().0)))
    }

    #[cfg(windows)]
    fn is_red(color: emulator::CellColor) -> bool {
        match color {
            emulator::CellColor::Indexed(1 | 9) => true,
            emulator::CellColor::Rgb(r, g, b) => r > 80 && r > g.saturating_add(20) && r > b.saturating_add(20),
            _ => false,
        }
    }
}
