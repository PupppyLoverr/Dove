//! A buddy's computer in the cloud: a Boat sandbox (https://boat.dev), a
//! Linux machine with a desktop, made and shown through Boat's API with the
//! person's own key (src/kx/keys.rs keeps it; it never reaches a script).
//!
//!   kx.boat.check              whether Boat takes the key held
//!   kx.boat.list             → [{ id, name, state }] (the ones made here)
//!   kx.boat.create(name)       minutes; → its id
//!   kx.boat.start(id), .stop(id), .remove(id)
//!   kx.boat.view(id)         → an address showing its desktop, live
//!
//! Only sandboxes named `kx-…` are listed, started, stopped or removed
//! here: each is looked up and its name checked before anything is done to
//! it, so the person's other sandboxes on Boat are never touched. A sandbox
//! costs the person money while it runs, so each is made (and started) to
//! stop itself after an hour.
//!
//! The desktop's address carries a token and opens within ten minutes: it's
//! asked for each time the desktop is opened, and never written down.

use std::time::Duration;

use serde_json::{Value as Json, json};
use zgx_script::{ScriptHost, Value};

const API: &str = "https://boat.dev/api/v1";
/// How long a sandbox runs before it stops itself.
const TTL_SECONDS: u64 = 3600;

/// A sandbox id as Boat writes them: `bx_` and eight of its letters and digits.
fn is_id(id: &str) -> bool {
    id.len() == 11 && id.starts_with("bx_") && id[3..].chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// A name of ours: `kx-` and small letters, digits or dashes.
fn ours(name: &str) -> bool {
    name.len() > 3 && name.len() <= 48 && name.starts_with("kx-") && name[3..].chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Why Boat said no, in words for the person. (Nothing of the request, so
/// nothing of the key.)
fn refusal(status: u16, body: &Json) -> String {
    let said = body["message"].as_str().or_else(|| body["error"]["message"].as_str()).unwrap_or_default();
    let code = body["code"].as_str().or_else(|| body["error"]["code"].as_str()).unwrap_or_default();
    match (status, code) {
        (401, _) => "Boat didn't accept the key. Check it at boat.dev and save it again.".to_owned(),
        (402, _) => "Boat says this account can't run sandboxes now. Check its plan and billing at boat.dev.".to_owned(),
        (_, "machine_not_running") => "It's stopped. Start it, then open its desktop.".to_owned(),
        _ if said.is_empty() => format!("Boat answered {status}."),
        _ => format!("Boat: {}", said.chars().take(300).collect::<String>()),
    }
}

/// One call to Boat with the key held.
fn call(method: &str, path: &str, body: Option<Json>, confirm: Option<&str>) -> Result<Json, String> {
    let key = crate::kx::keys::get("boat").ok_or("There's no Boat key yet. Add one first.")?;
    let mut req = ureq::request(method, &format!("{API}{path}")).timeout(Duration::from_secs(40)).set("Authorization", &format!("Bearer {key}"));
    if let Some(id) = confirm {
        // (Boat's own guard on a delete: the id said twice.)
        req = req.set("X-Ascii-Confirm-Delete", id);
    }
    let answered = match body {
        Some(body) => req.send_json(body),
        None => req.call(),
    };
    match answered {
        Ok(resp) => Ok(resp.into_json::<Json>().unwrap_or(Json::Null)),
        Err(ureq::Error::Status(status, resp)) => Err(refusal(status, &resp.into_json::<Json>().unwrap_or(Json::Null))),
        Err(_) => Err("Boat couldn't be reached. Check the connection and try again.".to_owned()),
    }
}

fn row(s: &Json) -> Json {
    json!({ "id": s["id"], "name": s["name"], "state": s["state"] })
}

fn list() -> Result<Json, String> {
    let all = call("GET", "/sandboxes", None, None)?;
    Ok(Json::Array(all["sandboxes"].as_array().into_iter().flatten().filter(|s| s["name"].as_str().is_some_and(ours)).map(row).collect()))
}

/// One of ours, looked up: an id that isn't Boat's, or a sandbox that isn't
/// named as ours, is refused before anything is done to it.
fn mine(id: &str) -> Result<Json, String> {
    if !is_id(id) {
        return Err("That isn't a Boat sandbox.".into());
    }
    let got = call("GET", &format!("/sandboxes/{id}"), None, None)?;
    if !got["sandbox"]["name"].as_str().is_some_and(ours) {
        return Err("That isn't one of Keplorayx's computers.".into());
    }
    Ok(got["sandbox"].clone())
}

fn up(state: &str) -> bool {
    matches!(state, "ready" | "idle" | "running")
}

/// Waited for until it's up (a few minutes at most), or said why it isn't.
fn wait_up(id: &str) -> Result<(), String> {
    for _ in 0..90 {
        let now = mine(id)?;
        let state = now["state"].as_str().unwrap_or_default();
        if up(state) {
            return Ok(());
        }
        if matches!(state, "error" | "cancelled") {
            return Err("Boat couldn't start it.".into());
        }
        std::thread::sleep(Duration::from_secs(4));
    }
    Err("It's taking longer than a few minutes. Look again shortly.".into())
}

fn create(name: &str) -> Result<String, String> {
    if !ours(name) {
        return Err("A computer's name starts kx- and has only small letters, digits and dashes.".into());
    }
    // (One per buddy: the one it has is used again, not made twice and paid for twice.)
    if let Some(had) = list()?.as_array().into_iter().flatten().find(|s| s["name"] == name) {
        return had["id"].as_str().map(str::to_owned).ok_or_else(|| "Boat listed it without an id.".to_owned());
    }
    let made = call("POST", "/sandboxes", Some(json!({ "name": name, "ttlSeconds": TTL_SECONDS })), None)?;
    let id = made["sandbox"]["id"].as_str().filter(|id| is_id(id)).ok_or("Boat made no sandbox.")?.to_owned();
    wait_up(&id)?;
    Ok(id)
}

fn view(id: &str) -> Result<String, String> {
    mine(id)?;
    for _ in 0..20 {
        let got = call("POST", &format!("/sandboxes/{id}/desktop"), Some(json!({})), None)?;
        if let Some(url) = got["desktopUrl"].as_str().filter(|u| u.starts_with("https://")) {
            return Ok(url.to_owned());
        }
        if got["provisioning"] != true {
            break;
        }
        std::thread::sleep(Duration::from_secs(3));
    }
    Err("Boat gave no address for its desktop.".into())
}

fn text(v: &Value) -> String {
    crate::auth::text(v)
}

async fn away<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    crate::kx::off_thread(work).await.unwrap_or_else(|| Err("Boat's answer was lost.".into()))
}

pub fn install(host: &ScriptHost) {
    host.async_action("kx.boat.check", |_| async move {
        away(|| call("GET", "/sandboxes", None, None).map(|_| ())).await?;
        Ok(Value::Nil)
    });
    host.async_action("kx.boat.list", |_| async move { Ok(crate::kx::to_value(&away(list).await?)) });
    host.async_action("kx.boat.create", |name| {
        let name = text(&name);
        async move { Ok(Value::from(away(move || create(&name)).await?.as_str())) }
    });
    host.async_action("kx.boat.start", |id| {
        let id = text(&id);
        async move {
            away(move || {
                mine(&id)?;
                call("POST", &format!("/sandboxes/{id}/resume"), Some(json!({ "ttlSeconds": TTL_SECONDS })), None)?;
                wait_up(&id)
            })
            .await?;
            Ok(Value::Nil)
        }
    });
    host.async_action("kx.boat.stop", |id| {
        let id = text(&id);
        async move {
            away(move || {
                mine(&id)?;
                call("POST", &format!("/sandboxes/{id}/stop"), Some(json!({})), None).map(|_| ())
            })
            .await?;
            Ok(Value::Nil)
        }
    });
    host.async_action("kx.boat.remove", |id| {
        let id = text(&id);
        async move {
            away(move || {
                mine(&id)?;
                call("DELETE", &format!("/sandboxes/{id}"), None, Some(&id)).map(|_| ())
            })
            .await?;
            Ok(Value::Nil)
        }
    });
    host.async_action("kx.boat.view", |id| {
        let id = text(&id);
        async move { Ok(Value::from(away(move || view(&id)).await?.as_str())) }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_boats_ids_and_our_own_names() {
        assert!(is_id("bx_2a3b4c5d"));
        for no in ["", "bx_", "bx_2a3b4c5", "bx_2a3b4c5d6", "bx_2A3B4C5D", "bx_2a3b/c5d", "xx_2a3b4c5d", "bx_2a3b4c5?"] {
            assert!(!is_id(no), "{no}");
        }
        assert!(ours("kx-b7861b61a8708"));
        assert!(!ours("my-server") && !ours("kx-") && !ours("kx-A"));
    }

    #[test]
    fn a_refusal_says_what_to_do_and_nothing_of_the_request() {
        assert!(refusal(401, &json!({ "message": "invalid token boat_abc" })).contains("didn't accept the key"));
        assert!(!refusal(401, &json!({ "message": "invalid token boat_abc" })).contains("boat_abc"));
        assert!(refusal(402, &Json::Null).contains("billing"));
        assert!(refusal(400, &json!({ "error": { "code": "machine_not_running" } })).contains("Start it"));
        assert_eq!(refusal(500, &Json::Null), "Boat answered 500.");
        assert_eq!(refusal(409, &json!({ "message": "busy" })), "Boat: busy");
    }
}
