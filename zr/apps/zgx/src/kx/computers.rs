//! A buddy's computer: a Cua Space on this machine, checked and managed
//! through `cua spaces` (https://cua.ai). First checks if any local Space is
//! already present so a buddy can attach to it directly, or creates a new one
//! named `kx-<buddy_id>`.
//!
//!   kx.computer.ready            whether `cua` is installed (shared)
//!   kx.computer.list           → [{ id, name, display_name, status, kind, image, os, runtime, is_ours }]
//!   kx.computer.create({ name, image })   minutes; → its space id (`local:<name>`)
//!   kx.computer.start(target), .stop(target), .remove(target)
//!   kx.computer.view(target)   → an address showing its desktop, live
//!
//! The desktop is cua's own HTML5 viewer, served on a loopback port by a
//! `cua viewer` this starts once and finds again on later launches
//! (`<data>/cua-viewer.json`), so the Desktop tab is a page like any other.

use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    sync::Mutex,
};

use serde_json::{Value as Json, json};
use zgx_script::{ScriptHost, Value};

/// The images a buddy's Space may be made from (`cua spaces create <IMAGE>`).
fn space_image(image: &str) -> Option<&'static str> {
    match image {
        "linux" | "ubuntu" => Some("linux"),
        "omarchy" => Some("omarchy"),
        "windows" => Some("windows"),
        "macos" => Some("macos"),
        _ => None,
    }
}

/// A safe Space slug: lowercase letters, digits, dashes or underscores.
fn valid_slug(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// A name of ours: `kx-` followed by a valid slug.
fn ours(name: &str) -> bool {
    let bare = name.strip_prefix("local:").unwrap_or(name);
    bare.len() > 3 && bare.starts_with("kx-") && valid_slug(bare)
}

/// Normalize a Space target (`local:<name>` or `<name>`) into `(id, bare_name)`.
fn normalize_target(target: &str) -> Option<(String, String)> {
    let bare = target.strip_prefix("local:").unwrap_or(target);
    if !valid_slug(bare) {
        return None;
    }
    Some((format!("local:{bare}"), bare.to_owned()))
}

/// Where `cua` is: on the path, else where its installer puts it.
fn cua_path() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "cua.exe" } else { "cua" };
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let at = dir.join(exe);
            if at.is_file() {
                return Some(at);
            }
        }
    }
    let home = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    let at = home.join("Programs").join("cua").join("bin").join(exe);
    at.is_file().then_some(at)
}

fn command(args: &[&str]) -> Option<Command> {
    let mut c = Command::new(cua_path()?);
    c.args(args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // (No console window flashing up behind the app.)
        c.creation_flags(0x0800_0000);
    }
    Some(c)
}

/// `cua` run to its end: what it printed, or why it failed in its own words.
fn cua(args: &[&str]) -> Result<String, String> {
    let mut c = command(args).ok_or("cua isn't installed on this computer (https://cua.ai).")?;
    let out = c.output().map_err(|e| format!("cua couldn't be run: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let said = String::from_utf8_lossy(&out.stderr);
    let said = said.trim();
    Err(if said.is_empty() { format!("cua stopped with {}", out.status) } else { said.chars().take(600).collect() })
}

/// Local Cua Spaces on this machine (`cua spaces ls --json`, enriched with
/// live power state from `cua sandbox ls --json`).
fn list() -> Result<Json, String> {
    let raw = cua(&["spaces", "ls", "--json"])?;
    let parsed: Json = serde_json::from_str(&raw).map_err(|e| format!("cua spaces list couldn't be read: {e}"))?;
    let spaces = parsed["spaces"].as_array().cloned().unwrap_or_default();

    // Merge live status/runtime from `cua sandbox ls --json` when available.
    let mut sb_status: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
    if let Ok(sb_raw) = cua(&["sandbox", "ls", "--json"]) {
        if let Ok(Json::Array(arr)) = serde_json::from_str::<Json>(&sb_raw) {
            for s in arr {
                if s["location"] == "local" {
                    let name = s["name"].as_str().unwrap_or_default().to_owned();
                    let status = s["status"].as_str().or_else(|| s["state"].as_str()).unwrap_or("unknown").to_owned();
                    let runtime = s["runtime"].as_str().unwrap_or_default().to_owned();
                    if !name.is_empty() {
                        sb_status.insert(name, (status, runtime));
                    }
                }
            }
        }
    }

    let local: Vec<Json> = spaces
        .into_iter()
        .filter(|s| {
            s["provider"] == "local" || s["id"].as_str().is_some_and(|id| id.starts_with("local:"))
        })
        .map(|s| {
            let name = s["name"].as_str().unwrap_or_default().to_owned();
            let id = s["id"].as_str().map(str::to_owned).unwrap_or_else(|| format!("local:{name}"));
            let display_name = s["display_name"].as_str().unwrap_or(&name).to_owned();
            let power = s["resources"]["powerState"].as_str().filter(|p| !p.is_empty()).map(str::to_owned);
            let (sb_stat, sb_rt) = sb_status.get(&name).cloned().unwrap_or_default();
            let status = power.or_else(|| (!sb_stat.is_empty()).then_some(sb_stat)).unwrap_or_else(|| "stopped".to_owned());
            json!({
                "id": id,
                "name": name,
                "display_name": display_name,
                "status": status,
                "kind": s["kind"],
                "image": s["image"],
                "os": s["os_pretty_name"].as_str().or_else(|| s["os_name"].as_str()).or_else(|| s["os"].as_str()).unwrap_or("Linux"),
                "runtime": if sb_rt.is_empty() { "qemu" } else { sb_rt.as_str() },
                "is_ours": ours(&name),
            })
        })
        .collect();
    Ok(Json::Array(local))
}

/// The viewer that's serving: its address's start (`http://127.0.0.1:PORT`)
/// and its key, with the process when this run started it.
static VIEWER: Mutex<Option<(String, String, Option<std::process::Child>)>> = Mutex::new(None);

fn viewer_file() -> PathBuf {
    crate::kx_account::data_dir().join("cua-viewer.json")
}

/// Whether something answers at `base` (a viewer from an earlier launch).
fn answers(base: &str) -> bool {
    let Some(addr) = base.strip_prefix("http://").and_then(|a| a.parse::<std::net::SocketAddr>().ok()) else { return false };
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(400)).is_ok()
}

/// The viewer's address and key: the one serving, else one started now.
fn viewer() -> Result<(String, String), String> {
    let mut held = VIEWER.lock().map_err(|_| "the viewer's state is broken")?;
    if let Some((base, key, _)) = held.as_ref() {
        if answers(base) {
            return Ok((base.clone(), key.clone()));
        }
    }
    // One an earlier launch left serving.
    if let Some(kept) = std::fs::read_to_string(viewer_file()).ok().and_then(|s| serde_json::from_str::<Json>(&s).ok()) {
        if let (Some(base), Some(key)) = (kept["base"].as_str(), kept["key"].as_str()) {
            if answers(base) {
                *held = Some((base.to_owned(), key.to_owned(), None));
                return Ok((base.to_owned(), key.to_owned()));
            }
        }
    }
    let mut c = command(&["viewer", "--listen", "127.0.0.1:0", "--no-open"]).ok_or("cua isn't installed on this computer (https://cua.ai).")?;
    let mut child = c.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("cua's viewer couldn't be started: {e}"))?;
    let out = child.stdout.take().ok_or("cua's viewer said nothing")?;
    // Its first lines say where it serves: "Viewer server: http://127.0.0.1:PORT/?key=KEY".
    let mut found: Option<(String, String)> = None;
    for line in BufReader::new(out).lines().map_while(Result::ok).take(6) {
        if let Some(at) = line.find("http://") {
            let url = line[at..].trim();
            if let Some((base, key)) = url.split_once("/?key=") {
                found = Some((base.to_owned(), key.to_owned()));
                break;
            }
        }
    }
    let Some((base, key)) = found else {
        let _ = child.kill();
        return Err("cua's viewer didn't say where it serves.".into());
    };
    let _ = std::fs::write(viewer_file(), json!({ "base": base, "key": key }).to_string());
    *held = Some((base.clone(), key.clone(), Some(child)));
    Ok((base, key))
}

/// The address of one Space's desktop.
fn view(target: &str) -> Result<String, String> {
    let (_, bare) = normalize_target(target).ok_or_else(|| "That isn't a valid Space name.".to_owned())?;
    let (base, key) = viewer()?;
    Ok(format!("{base}/s/local%3A{bare}/open?key={key}"))
}

fn text(v: &Value) -> String {
    crate::auth::text(v)
}

pub fn install(host: &ScriptHost) {
    host.shared("kx.computer.ready", cua_path().is_some());
    host.async_action("kx.computer.list", |_| async move {
        let found = crate::kx::off_thread(list).await.unwrap_or_else(|| Err("cua stopped unexpectedly".into()))?;
        Ok(crate::kx::to_value(&found))
    });
    host.async_action("kx.computer.create", |req| {
        let req = crate::kx::widgets::value_json(&req);
        async move {
            let raw_name = req["name"].as_str().unwrap_or_default().to_owned();
            let (space_id, bare_name) = normalize_target(&raw_name)
                .filter(|(_, b)| ours(b))
                .ok_or_else(|| "A computer's name starts kx- and has only small letters, digits and dashes.".to_owned())?;
            let image = space_image(req["image"].as_str().unwrap_or("linux"))
                .ok_or("That isn't an image Keplorayx makes Spaces from.")?;
            let made_name = bare_name.clone();
            let made_id = space_id.clone();
            crate::kx::off_thread(move || {
                // (On Windows a local Space is a QEMU VM: left to choose, cua picks
                // a container, which needs Docker, and most PCs have none.)
                let mut args = vec!["spaces", "create", image, "--on", "local", "--name", &made_name, "--json"];
                if cfg!(windows) && image == "linux" {
                    args.extend(["--kind", "vm", "--runtime", "qemu"]);
                }
                cua(&args)
                    .map(|_| ())
                    .inspect_err(|_| {
                        let _ = cua(&["spaces", "delete", &made_id, "--force"]);
                    })
            })
            .await
            .unwrap_or_else(|| Err("cua stopped unexpectedly".into()))?;
            Ok(Value::from(space_id.as_str()))
        }
    });
    for (action, verb) in [("kx.computer.start", "start"), ("kx.computer.stop", "stop"), ("kx.computer.remove", "delete")] {
        host.async_action(action, move |target| {
            let target = text(&target);
            async move {
                let (space_id, bare) = normalize_target(&target).ok_or_else(|| "That isn't a valid Space name.".to_owned())?;
                // (Deleting is only ever for a Space Keplorayx made: one of the
                // person's own that a buddy was attached to is theirs to keep.)
                if verb == "delete" && !ours(&bare) {
                    return Err("That Space isn't one Keplorayx made, so it isn't deleted from here.".to_owned());
                }
                crate::kx::off_thread(move || {
                    let args: Vec<&str> = if verb == "delete" {
                        vec!["spaces", "delete", &space_id, "--force"]
                    } else {
                        vec!["spaces", verb, &space_id]
                    };
                    cua(&args).map(|_| ())
                })
                    .await
                    .unwrap_or_else(|| Err("cua stopped unexpectedly".into()))?;
                Ok(Value::Nil)
            }
        });
    }
    host.async_action("kx.computer.view", |name| {
        let name = text(&name);
        async move {
            let url = crate::kx::off_thread(move || view(&name)).await.unwrap_or_else(|| Err("cua stopped unexpectedly".into()))?;
            Ok(Value::from(url.as_str()))
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_our_own_names_and_known_images() {
        assert!(ours("kx-b7861b61a8708"));
        assert!(ours("kx-mina-2"));
        assert!(ours("local:kx-mina-2"));
        assert!(valid_slug("space-45dabf6720"));
        assert_eq!(normalize_target("space-45dabf6720"), Some(("local:space-45dabf6720".into(), "space-45dabf6720".into())));
        assert_eq!(normalize_target("local:space-45dabf6720"), Some(("local:space-45dabf6720".into(), "space-45dabf6720".into())));
        // Anything that isn't a plain slug or our kx- prefix.
        for no in ["space-1", "agentspace", "kx-", "kx-A", "kx-a b", "kx-a;rm", "local:kx-a", "kx-../x"] {
            assert!(!ours(no) || no == "local:kx-a", "{no}");
        }
        assert!(normalize_target("kx-../x").is_none());
        assert!(normalize_target("kx-a;rm").is_none());
        assert!(space_image("linux").is_some() && space_image("omarchy").is_some());
        assert!(space_image("ghcr.io/evil/image").is_none());
    }
}
