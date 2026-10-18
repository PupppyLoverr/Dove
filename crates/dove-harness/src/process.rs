//! Spawning agent CLIs the same way on every OS.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::process::Command;

/// Resolve a program name against PATH (plus the usual per-user install dirs
/// a GUI-launched process does not inherit).
///
/// On Windows, a real `.exe` wins over a `.cmd` shim found earlier on PATH.
/// Extensionless shell scripts (`#!/bin/sh` npm shims) and broken Bun
/// trampolines are skipped: `CreateProcess` cannot run the scripts, and the
/// trampoline exits before the CLI starts. On Unix the first executable
/// match is enough.
pub fn resolve(program: &str) -> Option<PathBuf> {
    let all: Vec<PathBuf> = which::which_in_all(program, Some(search_path()), ".").ok()?.collect();
    if cfg!(windows) {
        return prefer_launchable(&all);
    }
    let is_shim = |p: &PathBuf| {
        let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        matches!(ext.as_str(), "cmd" | "bat" | "ps1")
    };
    all.iter().find(|p| !is_shim(p)).or(all.first()).cloned()
}

/// `Some(0)` is a native image. `Some(1)` is a batch shim `cmd.exe` can run.
/// `None` is a script or stub this process must not spawn.
fn launch_rank(path: &Path) -> Option<u8> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "exe" | "com" if !is_broken_bun_stub(path) => Some(0),
        "cmd" | "bat" => Some(1),
        _ => None,
    }
}

fn prefer_launchable(candidates: &[PathBuf]) -> Option<PathBuf> {
    let mut batch = None;
    for path in candidates {
        match launch_rank(path) {
            Some(0) => return Some(path.clone()),
            Some(1) if batch.is_none() => batch = Some(path.clone()),
            _ => {}
        }
    }
    batch
}

/// Bun's `~/.bun/bin/<tool>.exe` is sometimes a few-kilobyte trampoline that
/// exits with "could not create process" when its `node_modules` target is
/// gone. The real CLI is the npm `.cmd` beside it.
fn is_broken_bun_stub(path: &Path) -> bool {
    const MARKER: &[u8] = b"Bun failed to remap";
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if meta.len() == 0 || meta.len() > 256 * 1024 {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else { return false };
    bytes.windows(MARKER.len()).any(|window| window == MARKER)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_picker_skips_shell_scripts_and_broken_bun_stubs() {
        let dir = std::env::temp_dir().join(format!("dove-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("opencode");
        std::fs::write(&script, "#!/bin/sh\necho hi\n").unwrap();
        let stub = dir.join("opencode.exe");
        std::fs::write(&stub, b"MZ\nBun failed to remap this bin\n").unwrap();
        let cmd = dir.join("opencode.cmd");
        std::fs::write(&cmd, "@echo off\n").unwrap();
        let real = dir.join("claude.exe");
        std::fs::write(&real, b"MZ not a bun stub").unwrap();

        assert_eq!(launch_rank(&script), None);
        assert_eq!(launch_rank(&stub), None);
        assert_eq!(launch_rank(&cmd), Some(1));
        assert_eq!(launch_rank(&real), Some(0));
        assert_eq!(prefer_launchable(&[script.clone(), stub.clone(), cmd.clone()]).as_deref(), Some(cmd.as_path()));
        assert_eq!(prefer_launchable(&[cmd.clone(), real.clone()]).as_deref(), Some(real.as_path()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Installed CLIs must be something CreateProcess can start.
    #[cfg(windows)]
    #[test]
    fn resolve_skips_scripts_and_bun_stubs() {
        for name in ["opencode", "npx", "cursor-agent", "kimi", "amp", "claude", "node"] {
            let Some(path) = resolve(name) else { continue };
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            assert!(matches!(ext.as_str(), "exe" | "cmd" | "bat" | "com"), "{name} resolved to {}", path.display());
            assert!(!is_broken_bun_stub(&path), "{name} resolved to broken stub {}", path.display());
        }
    }
}
