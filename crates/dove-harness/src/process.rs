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
        "cmd" | "bat" if !is_broken_cmd_shim(path) => Some(1),
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

/// Bun's `~/.bun/bin/<tool>.exe` is a few-kilobyte trampoline that runs the
/// target named in the `<tool>.bunx` beside it, and exits with "could not
/// create process" when that target is gone. Every trampoline carries the
/// "Bun failed to remap" message, so the marker only says it is one; the
/// `.bunx` target says whether it still works.
fn is_broken_bun_stub(path: &Path) -> bool {
    const MARKER: &[u8] = b"Bun failed to remap";
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if meta.len() == 0 || meta.len() > 256 * 1024 {
        return false;
    }
    let Ok(bytes) = std::fs::read(path) else { return false };
    if !bytes.windows(MARKER.len()).any(|window| window == MARKER) {
        return false;
    }
    match bunx_target(&path.with_extension("bunx")) {
        Some(target) => !target.is_file(),
        None => true,
    }
}

/// The program a `.bunx` file points at. The file is UTF-16LE: a path
/// relative to the bin folder's parent, closed by `"`, then the launcher.
fn bunx_target(bunx: &Path) -> Option<PathBuf> {
    let bytes = std::fs::read(bunx).ok()?;
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let text = String::from_utf16_lossy(&units);
    let rel = text.split('"').next().filter(|r| !r.is_empty())?;
    Some(bunx.parent()?.parent()?.join(rel))
}

/// npm and pnpm `.cmd` shims run `node "%~dp0\..\<pkg>\bin\cli.js"`. When the
/// package is removed or moved (pnpm keeps a store per version), the shim
/// stays behind and node exits with "Cannot find module" before the CLI
/// starts. A shim whose script target is missing does not count.
fn is_broken_cmd_shim(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if meta.len() > 64 * 1024 {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    let Some(dir) = path.parent() else { return false };
    cmd_shim_targets(&text).iter().any(|rel| !dir.join(rel).is_file())
}

/// The `%~dp0`-relative script paths a shim runs, leaving out the optional
/// `node.exe` beside it (the shim checks that one with `IF EXIST`).
fn cmd_shim_targets(text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for chunk in text.split("\"%~dp0").skip(1) {
        let Some(rel) = chunk.split('"').next() else { continue };
        let rel = rel.trim_start_matches(['\\', '/']);
        let script = std::path::Path::new(rel).extension().and_then(|e| e.to_str()).unwrap_or("");
        if !rel.is_empty() && matches!(script.to_ascii_lowercase().as_str(), "js" | "cjs" | "mjs") {
            targets.push(rel.replace('/', "\\"));
        }
    }
    targets
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

    /// Bun's layout: `<root>/bin/<tool>.exe` + `<tool>.bunx`, whose path is
    /// relative to `<root>`. The stub works while that target exists.
    /// Windows only: the targets are backslash paths.
    #[cfg(windows)]
    #[test]
    fn bun_stub_counts_while_its_target_exists() {
        // `home/.bun/bin/codex.exe` -> `home/node_modules/...`, as Bun lays it out.
        let home = std::env::temp_dir().join(format!("dove-bunx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let bin = home.join(".bun").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let stub = bin.join("codex.exe");
        std::fs::write(&stub, b"MZ\nBun failed to remap this bin\n").unwrap();
        let bunx: Vec<u8> = "..\\node_modules\\@openai\\codex\\bin\\codex.js\"\0node \0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        std::fs::write(bin.join("codex.bunx"), bunx).unwrap();
        assert!(is_broken_bun_stub(&stub), "target missing");

        let target = home.join("node_modules").join("@openai").join("codex").join("bin");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("codex.js"), "").unwrap();
        assert!(!is_broken_bun_stub(&stub), "target present");
        assert_eq!(launch_rank(&stub), Some(0));

        let _ = std::fs::remove_dir_all(&home);
    }

    /// A pnpm shim left behind after its package moved is not launchable.
    /// Windows only: `.cmd` shims name their script with backslashes.
    #[cfg(windows)]
    #[test]
    fn cmd_shim_with_a_missing_script_is_skipped() {
        let dir = std::env::temp_dir().join(format!("dove-cmdshim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pkg")).unwrap();
        let shim = |target: &str| {
            format!(
                "@SETLOCAL\r\n@IF EXIST \"%~dp0\\node.exe\" (\r\n  \"%~dp0\\node.exe\"  \"%~dp0\\{target}\" %*\r\n) ELSE (\r\n  node  \"%~dp0\\{target}\" %*\r\n)\r\n"
            )
        };
        assert_eq!(cmd_shim_targets(&shim("pkg\\cli.js")), vec!["pkg\\cli.js".to_string(), "pkg\\cli.js".to_string()]);

        let broken = dir.join("gone.cmd");
        std::fs::write(&broken, shim("missing\\cli.js")).unwrap();
        let good = dir.join("ok.cmd");
        std::fs::write(&good, shim("pkg\\cli.js")).unwrap();
        std::fs::write(dir.join("pkg").join("cli.js"), "").unwrap();
        let plain = dir.join("plain.cmd");
        std::fs::write(&plain, "@echo off\r\nsomething.exe %*\r\n").unwrap();

        assert_eq!(launch_rank(&broken), None);
        assert_eq!(launch_rank(&good), Some(1));
        assert_eq!(launch_rank(&plain), Some(1));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Installed CLIs must be something CreateProcess can start.
    #[cfg(windows)]
    #[test]
    fn resolve_skips_scripts_and_bun_stubs() {
        for name in ["opencode", "npx", "cursor-agent", "kimi", "amp", "claude", "codex", "node"] {
            let Some(path) = resolve(name) else { continue };
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            assert!(matches!(ext.as_str(), "exe" | "cmd" | "bat" | "com"), "{name} resolved to {}", path.display());
            assert!(!is_broken_bun_stub(&path), "{name} resolved to broken stub {}", path.display());
            assert!(!is_broken_cmd_shim(&path), "{name} resolved to broken shim {}", path.display());
        }
    }
}
