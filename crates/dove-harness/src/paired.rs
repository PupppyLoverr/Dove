//! Paired devices: other computers on the user's Woozlit account that run
//! Keplorayx with "Use this computer as a machine" on (or `dove-mesh-host
//! serve-node` headless). Agents reach them through the account's device
//! relay on the Cloudflare edge, so neither side opens a port.
//!
//! The relay client lives in the `dove-mesh-host` sidecar (mesh workspace);
//! this module only builds one-shot invocations of it:
//! - `exec --device <id> -- argv…` runs argv on the device with raw stdio and
//!   exits with the remote code (255: connection lost, 77: not paired);
//! - `nodes --json`, `pair`, `pair-code --json`, `paired --json`, `revoke`,
//!   `forget`, `node-mode on|off` manage pairing. State lives in files under
//!   `$DOVE_HOME/mesh`, so the long-running sidecar sees these changes.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::host::Invocation;

/// A paired device as the chip and the Machines page show it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub online: bool,
}

const SIDECAR: &str = if cfg!(windows) { "dove-mesh-host.exe" } else { "dove-mesh-host" };
const CACHE_FOR: Duration = Duration::from_secs(10);

/// The sidecar binary, found the way `apps/dove/src/remote.rs` finds it.
fn sidecar() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("DOVE_MESH_HOST").map(PathBuf::from).filter(|p| p.exists()) {
        return Some(p);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let mut candidates = vec![dir.join(SIDECAR), dir.join("../Resources").join(SIDECAR)];
    // Dev checkout: target/<profile>/dove → mesh/target/<profile>/dove-mesh-host.
    for ancestor in dir.ancestors().take(4) {
        for profile in ["release", "debug"] {
            candidates.push(ancestor.join("mesh/target").join(profile).join(SIDECAR));
        }
    }
    candidates.into_iter().find(|p| p.exists())
}

/// The sidecar's data dir, as the desktop passes it (`$DOVE_HOME/mesh`).
fn data_dir() -> PathBuf {
    crate::hosts::dove_home().join("mesh")
}

fn exec_args(data_dir: &str, device: &str, argv: &[String]) -> Vec<String> {
    let mut args: Vec<String> =
        vec!["--data-dir".into(), data_dir.into(), "exec".into(), "--device".into(), device.into(), "--".into()];
    args.extend(argv.iter().cloned());
    args
}

/// The local command that runs `argv` on `device` with live stdio.
pub fn exec_invocation(device: &str, argv: &[String]) -> Invocation {
    let program = sidecar().unwrap_or_else(|| PathBuf::from(SIDECAR));
    Invocation { program, args: exec_args(&data_dir().to_string_lossy(), device, argv), local_cwd: None }
}

/// Runs a one-shot sidecar command; stdout on success, the sidecar's own
/// message on failure.
fn sidecar_run(args: &[&str]) -> anyhow::Result<String> {
    let bin = sidecar().ok_or_else(|| anyhow::anyhow!("dove-mesh-host is missing from this build"))?;
    let out = Command::new(bin).arg("--data-dir").arg(data_dir()).args(args).stdin(Stdio::null()).output()?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    Err(anyhow::anyhow!(failure(&String::from_utf8_lossy(&out.stderr))))
}

fn failure(stderr: &str) -> String {
    let line = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("dove-mesh-host failed");
    line.trim().trim_start_matches("dove-mesh-host: ").to_string()
}

fn parse_devices(json: &str) -> Vec<Device> {
    serde_json::from_str(json.trim()).unwrap_or_default()
}

static CACHE: Mutex<Option<(Instant, Vec<Device>)>> = Mutex::new(None);
static REFRESHING: AtomicBool = AtomicBool::new(false);

fn fetch_devices() -> Vec<Device> {
    let devices =
        sidecar_run(&["nodes", "--json", "--timeout-ms", "1500"]).map(|s| parse_devices(&s)).unwrap_or_default();
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), devices.clone()));
    devices
}

fn invalidate() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Computers this one paired with, with relay presence. Cached ~10 s; a
/// stale cache is returned at once and refreshed in the background. Empty
/// (and instant) when the sidecar is missing.
pub fn devices() -> Vec<Device> {
    if sidecar().is_none() {
        return Vec::new();
    }
    let cached = CACHE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match cached {
        Some((at, devices)) if at.elapsed() < CACHE_FOR => devices,
        Some((_, devices)) => {
            if !REFRESHING.swap(true, Ordering::AcqRel) {
                std::thread::spawn(|| {
                    fetch_devices();
                    REFRESHING.store(false, Ordering::Release);
                });
            }
            devices
        }
        None => fetch_devices(),
    }
}

/// A device's display name, falling back to its id.
pub fn name(device: &str) -> String {
    devices().into_iter().find(|d| d.id == device).map(|d| d.name).unwrap_or_else(|| device.to_string())
}

/// Whether this build can pair devices (the relay client is present).
pub fn pairing_available() -> bool {
    sidecar().is_some()
}

/// Pair with the computer `device` using the code it shows. Both sides
/// remember the pairing; needs the Woozlit sign-in.
pub fn pair(device: &str, code: &str) -> anyhow::Result<()> {
    let result = sidecar_run(&["pair", "--device", device, "--code", code, "--json"]).map(|_| ());
    invalidate();
    result
}

/// Pair from a scanned `keplorayx://pair?device=…&code=…` URI.
pub fn pair_uri(uri: &str) -> anyhow::Result<()> {
    let result = sidecar_run(&["pair", "--uri", uri, "--json"]).map(|_| ());
    invalidate();
    result
}

/// Make this computer pairable: turns node mode on and returns a fresh
/// single-use code (valid 10 minutes) and the `keplorayx://pair` URI to show
/// as a QR. Node mode is served by the running sidecar (Remote control on)
/// or by `dove-mesh-host serve-node`.
pub fn pairing_offer() -> anyhow::Result<(String, String)> {
    set_node_enabled(true)?;
    let out = sidecar_run(&["pair-code", "--json"])?;
    let v: serde_json::Value = serde_json::from_str(out.trim())?;
    match (v["code"].as_str(), v["uri"].as_str()) {
        (Some(code), Some(uri)) => Ok((code.to_string(), uri.to_string())),
        _ => Err(anyhow::anyhow!("dove-mesh-host returned no pairing code")),
    }
}

/// Whether this computer accepts runs from devices paired with it.
pub fn node_enabled() -> bool {
    sidecar_run(&["node-mode"]).is_ok_and(|s| s.trim() == "on")
}

pub fn set_node_enabled(on: bool) -> anyhow::Result<()> {
    sidecar_run(&["node-mode", if on { "on" } else { "off" }]).map(|_| ())
}

/// Devices allowed to run on this computer (`online` is not tracked here).
pub fn trusted_devices() -> Vec<Device> {
    sidecar_run(&["paired", "--json"]).map(|s| parse_devices(&s)).unwrap_or_default()
}

/// Stop trusting `device` on this computer; its running sessions end.
pub fn revoke(device: &str) -> anyhow::Result<()> {
    sidecar_run(&["revoke", "--device", device]).map(|_| ())
}

/// Forget a computer this one paired with.
pub fn forget(device: &str) -> anyhow::Result<()> {
    let result = sidecar_run(&["forget", "--device", device]).map(|_| ());
    invalidate();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_args_put_argv_after_the_separator_verbatim() {
        let argv: Vec<String> =
            ["sh", "-lc", "cd \"$1\" && shift && exec \"$@\"", "--", "~/p", "codex"].map(String::from).to_vec();
        let args = exec_args("/h/.dove/mesh", "dove-desktop-1", &argv);
        assert_eq!(&args[..6], ["--data-dir", "/h/.dove/mesh", "exec", "--device", "dove-desktop-1", "--"]);
        assert_eq!(&args[6..], argv.as_slice());
    }

    #[test]
    fn invocation_runs_the_sidecar_without_a_local_cwd() {
        let inv = exec_invocation("d1", &["true".to_string()]);
        assert!(inv.program.to_string_lossy().contains("dove-mesh-host"));
        assert!(inv.local_cwd.is_none());
        assert_eq!(inv.args.last().map(String::as_str), Some("true"));
    }

    #[test]
    fn parses_node_lists_and_errors() {
        let devices = parse_devices(r#"[{"id":"a","name":"Mac","online":true,"pairedAt":1},{"id":"b","name":"PC"}]"#);
        assert_eq!(
            devices,
            vec![
                Device { id: "a".into(), name: "Mac".into(), online: true },
                Device { id: "b".into(), name: "PC".into(), online: false },
            ]
        );
        assert!(parse_devices("not json").is_empty());
        assert_eq!(failure("dove-mesh-host: wrong pairing code\n"), "wrong pairing code");
    }
}

/// `dove node serve` / `dove node pair`: run this computer as a machine in
/// the foreground (no window), printing a pairing code and a terminal QR.
/// Needs a prior sign-in (Settings › Remote on a desktop).
pub fn serve_foreground(show_code: bool) -> anyhow::Result<i32> {
    let bin = sidecar().ok_or_else(|| anyhow::anyhow!("dove-mesh-host is missing from this build"))?;
    let mut cmd = Command::new(bin);
    cmd.arg("--data-dir").arg(data_dir()).arg("serve-node");
    if !show_code {
        cmd.arg("--no-code");
    }
    Ok(cmd.status()?.code().unwrap_or(1))
}
