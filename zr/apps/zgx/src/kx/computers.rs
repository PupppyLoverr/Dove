//! A buddy's computer: a sandbox of its own on this machine, made and shown
//! through cua (the `cua` command line, https://cua.ai), which the person
//! has installed. A sandbox is a real desktop (a virtual machine or a
//! container) the buddy's agent can work in without touching the person's.
//!
//!   kx.computer.ready            whether `cua` is there to ask (shared)
//!   kx.computer.list           → [{ name, status, kind, runtime }] (local ones)
//!   kx.computer.create({ name, image })   minutes; → its name
//!   kx.computer.start(name), .stop(name), .remove(name)
//!   kx.computer.view(name)     → an address showing its desktop, live
//!
//! Only sandboxes whose name starts `kx-` are made, stopped or removed
//! here: the person's own are listed by nothing here and touched by nothing.
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

/// The images a buddy's computer may be made from, by the name the app uses:
/// what `cua sb create` is given for each.
fn image_args(image: &str) -> Option<&'static [&'static str]> {
    match image {
        // (Arch with Hyprland: the image already on a machine that has used cua's.)
        "omarchy" => Some(&["omarchy"]),
        "linux" => Some(&["linux", "--tier", "slim", "--kind", "vm"]),
        "linux-full" => Some(&["linux", "--kind", "vm"]),
        "windows" => Some(&["windows"]),
        _ => None,
    }
}

/// A name of ours: `kx-` and letters, digits or dashes.
fn ours(name: &str) -> bool {
    name.len() > 3 && name.len() <= 48 && name.starts_with("kx-") && name[3..].chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
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

/// The local sandboxes that are ours.
fn list() -> Result<Json, String> {
    let all: Json = serde_json::from_str(&cua(&["sandbox", "ls", "--json"])?).map_err(|e| format!("cua's list couldn't be read: {e}"))?;
    let mine: Vec<Json> = all
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["location"] == "local" && s["name"].as_str().is_some_and(ours))
        .map(|s| json!({ "name": s["name"], "status": s["status"], "kind": s["kind"], "runtime": s["runtime"] }))
        .collect();
    Ok(Json::Array(mine))
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

/// The address of one sandbox's desktop.
fn view(name: &str) -> Result<String, String> {
    if !ours(name) {
        return Err("That isn't one of Keplorayx's computers.".into());
    }
    let (base, key) = viewer()?;
    Ok(format!("{base}/s/local%3A{name}/open?key={key}"))
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
            let name = req["name"].as_str().unwrap_or_default().to_owned();
            if !ours(&name) {
                return Err("A computer's name starts kx- and has only small letters, digits and dashes.".to_owned());
            }
            let args = image_args(req["image"].as_str().unwrap_or("omarchy")).ok_or("That isn't an image Keplorayx makes computers from.")?;
            let made = name.clone();
            crate::kx::off_thread(move || {
                let mut all: Vec<&str> = vec!["sandbox", "create"];
                all.extend_from_slice(args);
                all.extend_from_slice(&["--name", &made, "--wait", "desktop"]);
                cua(&all).map(|_| ()).inspect_err(|_| {
                    // (What didn't come up isn't left running, or listed as if it had.)
                    let _ = cua(&["sandbox", "rm", &format!("local:{made}"), "--force"]);
                })
            })
            .await
            .unwrap_or_else(|| Err("cua stopped unexpectedly".into()))?;
            Ok(Value::from(name.as_str()))
        }
    });
    for (action, verb) in [("kx.computer.start", "resume"), ("kx.computer.stop", "suspend"), ("kx.computer.remove", "rm")] {
        host.async_action(action, move |name| {
            let name = text(&name);
            async move {
                if !ours(&name) {
                    return Err("That isn't one of Keplorayx's computers.".to_owned());
                }
                crate::kx::off_thread(move || {
                    let which = format!("local:{name}");
                    // (cua asks before it deletes, and there is nobody at its prompt: the
                    // person said yes in the app.)
                    let args: Vec<&str> = if verb == "rm" { vec!["sandbox", verb, &which, "--force"] } else { vec!["sandbox", verb, &which] };
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
        // The person's own sandboxes, and anything that isn't a plain name.
        for no in ["space-1", "agentspace", "kx-", "kx-A", "kx-a b", "kx-a;rm", "local:kx-a", "kx-../x"] {
            assert!(!ours(no), "{no}");
        }
        assert!(image_args("omarchy").is_some() && image_args("linux").is_some());
        assert!(image_args("ghcr.io/evil/image").is_none());
    }
}
