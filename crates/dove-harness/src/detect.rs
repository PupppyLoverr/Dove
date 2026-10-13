//! Finding which agent CLIs are installed.

use std::path::PathBuf;

use dove_proto::HarnessKind;

use crate::process;

/// Executable names to try for each harness, most specific first.
pub fn program_names(kind: HarnessKind) -> &'static [&'static str] {
    match kind {
        HarnessKind::ClaudeCode => &["claude"],
        HarnessKind::Codex => &["codex"],
        HarnessKind::Cursor => &["cursor-agent", "agent"],
        HarnessKind::Devin => &["devin"],
        HarnessKind::Grok => &["grok"],
        HarnessKind::Gemini => &["gemini"],
        HarnessKind::OpenCode => &["opencode"],
        HarnessKind::Pi => &["pi"],
        HarnessKind::Antigravity => &["agy_acp_server", "antigravity", "agy"],
        HarnessKind::Copilot => &["copilot"],
        HarnessKind::Amp => &["amp"],
        HarnessKind::Droid => &["droid"],
        HarnessKind::Goose => &["goose"],
        HarnessKind::Qwen => &["qwen"],
        HarnessKind::Kilo => &["kilo", "kilocode"],
        HarnessKind::Cline => &["cline"],
        HarnessKind::Hermes => &["hermes"],
        HarnessKind::Kimi => &["kimi"],
        HarnessKind::Shell => &[],
    }
}

pub fn find(kind: HarnessKind) -> Option<PathBuf> {
    if let Some(key) = env_override(kind)
        && let Some(path) = std::env::var_os(key).filter(|v| !v.is_empty())
    {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    program_names(kind).iter().find_map(|name| process::resolve(name).filter(|path| belongs_to(kind, name, path)))
}

/// Generic names other CLIs also ship: Grok installs an `agent` binary too,
/// so Cursor's `agent` only counts from Cursor's own install.
fn belongs_to(kind: HarnessKind, name: &str, path: &std::path::Path) -> bool {
    match (kind, name) {
        (HarnessKind::Cursor, "agent") => path.to_string_lossy().to_ascii_lowercase().contains("cursor"),
        _ => true,
    }
}

fn env_override(kind: HarnessKind) -> Option<&'static str> {
    Some(match kind {
        HarnessKind::Antigravity => "ANTIGRAVITY_ACP_EXECUTABLE",
        HarnessKind::Hermes => "HERMES_EXECUTABLE",
        HarnessKind::Grok => "GROK_EXECUTABLE",
        HarnessKind::Devin => "DEVIN_EXECUTABLE",
        _ => return None,
    })
}

#[derive(Debug, Clone)]
pub struct Installed {
    pub kind: HarnessKind,
    pub program: PathBuf,
}

/// Scan for every known harness. Pure PATH lookups: no process is started,
/// so this is cheap enough to run at startup on a background thread.
pub fn scan() -> Vec<Installed> {
    HarnessKind::ALL.iter().filter_map(|&kind| find(kind).map(|program| Installed { kind, program })).collect()
}
