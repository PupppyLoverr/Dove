//! Opt-in backend: Vercel Labs agent-desktop (Apache-2.0).
//!
//! It drives native apps through the accessibility tree. It is not the
//! default. cua-driver stays the fallback when this binary is missing.
//! An unsigned copy that the OS refuses to start is reported as blocked,
//! and is not silently replaced.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

static REFS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub enum DesktopError {
    /// No binary, or no hash for a copy that is not the spike path.
    Missing,
    /// The OS refused to start it (SmartScreen, Smart App Control).
    Blocked(String),
    Failed(String),
}

/// `DOVE_AGENT_DESKTOP`, then the plugins folder.
pub fn binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DOVE_AGENT_DESKTOP").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }
    let name = if cfg!(windows) { "agent-desktop.exe" } else { "agent-desktop" };
    let home = std::env::var_os("DOVE_HOME").map(PathBuf::from).or_else(|| {
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(|h| PathBuf::from(h).join(".dove"))
    })?;
    let path = home.join("plugins").join("agent-desktop").join(name);
    path.is_file().then_some(path)
}

/// A plugins-folder copy is used only when `expected_sha256` matches.
/// `DOVE_AGENT_DESKTOP` is the Windows spike and may omit the hash.
pub fn usable(expected_sha256: &str) -> bool {
    let Some(path) = binary() else { return false };
    let from_env = std::env::var_os("DOVE_AGENT_DESKTOP").is_some_and(|p| PathBuf::from(p) == path);
    if from_env && expected_sha256.is_empty() {
        return true;
    }
    if expected_sha256.is_empty() {
        return false;
    }
    hash_of(&path).is_some_and(|got| got.eq_ignore_ascii_case(expected_sha256.trim()))
}

fn hash_of(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(hex(&Sha256::digest(bytes)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Capture, click, and type. Other actions are [`DesktopError::Missing`] so
/// the caller keeps cua-driver for them.
pub fn try_action(action: &str, args: &Value) -> Result<Value, DesktopError> {
    let bin = binary().ok_or(DesktopError::Missing)?;
    match action {
        "capture" => snapshot(&bin),
        "click" => click(&bin, args),
        "type" => set_value(&bin, args),
        _ => Err(DesktopError::Missing),
    }
}

fn snapshot(bin: &Path) -> Result<Value, DesktopError> {
    let value = run(bin, &["snapshot", "--json"])?;
    let mut rows = Vec::new();
    collect(&value, &mut rows);
    if let Ok(mut refs) = REFS.lock() {
        *refs = rows.iter().map(|(r, _)| r.clone()).collect();
    }
    let elements: Vec<Value> =
        rows.iter().enumerate().map(|(i, (r#ref, name))| json!({"index": i, "ref": r#ref, "name": name})).collect();
    Ok(json!({
        "ok": true,
        "action": "capture",
        "backend": "agent-desktop",
        "elements": elements,
    }))
}

fn click(bin: &Path, args: &Value) -> Result<Value, DesktopError> {
    let r#ref = r#ref_of(args)?;
    let _ = run(bin, &["click", &r#ref])?;
    Ok(json!({"ok": true, "action": "click", "backend": "agent-desktop", "ref": r#ref}))
}

fn set_value(bin: &Path, args: &Value) -> Result<Value, DesktopError> {
    let text = args.get("text").and_then(Value::as_str).unwrap_or("");
    let r#ref = r#ref_of(args)?;
    let _ = run(bin, &["set-value", &r#ref, "--value", text])?;
    Ok(json!({"ok": true, "action": "type", "backend": "agent-desktop", "ref": r#ref}))
}

fn r#ref_of(args: &Value) -> Result<String, DesktopError> {
    if let Some(r#ref) = args.get("ref").and_then(Value::as_str) {
        return Ok(r#ref.to_string());
    }
    let index = args
        .get("index")
        .and_then(Value::as_u64)
        .or_else(|| args.get("element").and_then(Value::as_u64))
        .ok_or_else(|| DesktopError::Failed("capture first, then click or type by index".into()))?;
    REFS.lock()
        .ok()
        .and_then(|refs| refs.get(index as usize).cloned())
        .ok_or_else(|| DesktopError::Failed("that row is not in the last agent-desktop snapshot".into()))
}

fn run(bin: &Path, args: &[&str]) -> Result<Value, DesktopError> {
    let out = Command::new(bin).args(args).output().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound || err.raw_os_error() == Some(5) {
            DesktopError::Blocked(format!(
                "agent-desktop did not start ({err}). Windows may be blocking the unsigned plugin."
            ))
        } else {
            DesktopError::Failed(err.to_string())
        }
    })?;
    if !out.status.success() {
        let message = String::from_utf8_lossy(&out.stderr);
        let message = message.trim();
        if message.is_empty() {
            return Err(DesktopError::Failed(format!("agent-desktop exited {}", out.status)));
        }
        return Err(DesktopError::Failed(message.to_string()));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(serde_json::from_str(text.trim()).unwrap_or_else(|_| json!({"raw": text.trim()})))
}

fn collect(value: &Value, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            if let Some(r#ref) = map.get("ref").and_then(Value::as_str) {
                let name = map
                    .get("name")
                    .or_else(|| map.get("title"))
                    .or_else(|| map.get("label"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                out.push((r#ref.to_string(), name.to_string()));
            }
            for child in map.values() {
                collect(child, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect(child, out);
            }
        }
        _ => {}
    }
}
