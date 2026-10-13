//! Spawning agent CLIs the same way on every OS.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::process::Command;

/// Resolve a program name against PATH (plus the usual per-user install dirs
/// a GUI-launched process does not inherit).
///
/// Native executables win over script shims (`.cmd`, `.bat`, `.ps1`) found
/// earlier on PATH: they start faster and do not break when a package
/// manager's global store moves.
pub fn resolve(program: &str) -> Option<PathBuf> {
    let all: Vec<PathBuf> = which::which_in_all(program, Some(search_path()), ".").ok()?.collect();
    let is_shim = |p: &PathBuf| {
        let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        matches!(ext.as_str(), "cmd" | "bat" | "ps1")
    };
    all.iter().find(|p| !is_shim(p)).or(all.first()).cloned()
}

/// PATH extended with directories that installers use but that a process
/// launched from Finder, the Start menu or a desktop file may not see.
pub fn search_path() -> OsString {
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        let home = PathBuf::from(home);
        for rel in [".local/bin", "bin", ".cargo/bin", ".bun/bin", ".npm-global/bin", ".volta/bin"] {
            dirs.push(home.join(rel));
        }
    }
    if cfg!(target_os = "macos") {
        dirs.push("/opt/homebrew/bin".into());
        dirs.push("/usr/local/bin".into());
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

/// A command for `program` that works for native binaries and for the
/// `.cmd`/`.bat` shims npm and pnpm install on Windows.
pub fn command(program: &Path, args: &[String], cwd: &Path) -> Command {
    let ext = program.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let mut cmd = if cfg!(windows) && (ext == "cmd" || ext == "bat") {
        let mut c = Command::new("cmd.exe");
        c.arg("/d").arg("/s").arg("/c").arg(program).args(args);
        c
    } else {
        let mut c = Command::new(program);
        c.args(args);
        c
    };
    cmd.current_dir(cwd)
        .env("PATH", search_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: never flash a console for a child of a GUI app.
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// Spawn with a readable error naming the program.
pub fn spawn(mut cmd: Command, program: &Path) -> Result<tokio::process::Child> {
    cmd.spawn().with_context(|| format!("failed to start {}", program.display()))
}
