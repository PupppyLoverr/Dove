//! Agents in the in-app browser: what an agent asks of a page (the engine's
//! `browser_*` tools, zr/crates/mcp/src/tools/kx_browser.rs) reaches the
//! main window's scripts, and their answer goes back.
//!
//! A step is a file. The tool writes `<data>/browser/requests/<id>.json`;
//! this reads the folder a few times a second and keeps what waits in
//! `kx.browser.requests` (a list, oldest first), which
//! ui/kx/browser_agent.luau follows: it does each on the page (its cursor
//! moving there, the page's blue shimmer on) and calls `kx.browser.reply`
//! with `{ id, json }` (the page's answer, as JSON text) or `{ id, error }`.
//! The answer is written to `<data>/browser/replies/<id>.json`, where the
//! tool is waiting for it, and the request is taken away.

use std::{path::PathBuf, time::Duration};

use serde_json::{Value as Json, json};
use zgx_desktop::WindowContext;
use zgx_script::ScriptHost;

/// How often the folder is read.
const EVERY: Duration = Duration::from_millis(120);
/// A request nobody answered is let go after this (its tool gave up before).
const STALE_MS: u64 = 90_000;

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// An id as the tool makes them (hex), never a path.
fn safe(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Answers nobody took (their tool had given up waiting) are swept away.
fn sweep(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir.join("replies")) else { return };
    for entry in entries.flatten() {
        let old = entry.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_millis() as u64 > STALE_MS);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// What waits, oldest first. Stale ones are removed on the way.
fn pending(dir: &std::path::Path) -> Vec<Json> {
    let Ok(entries) = std::fs::read_dir(dir.join("requests")) else { return Vec::new() };
    let now = now_ms();
    let mut out: Vec<Json> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(request) = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Json>(&t).ok()) else { continue };
        if !request["id"].as_str().is_some_and(safe) {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        if now.saturating_sub(request["at"].as_u64().unwrap_or(0)) > STALE_MS {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        out.push(request);
    }
    out.sort_by_key(|r| r["at"].as_u64().unwrap_or(0));
    out
}

/// The scripts' answer, as the tool reads it: `{ ok, result }` or
/// `{ ok: false, error }`. A page's own `{ error }` is an error too.
fn answer(reply: &Json) -> Json {
    if let Some(error) = reply["error"].as_str() {
        return json!({ "ok": false, "error": error });
    }
    let result = reply["json"].as_str().and_then(|text| serde_json::from_str::<Json>(text).ok()).unwrap_or_else(|| json!({}));
    match result["error"].as_str() {
        Some(error) => json!({ "ok": false, "error": error }),
        None => json!({ "ok": true, "result": result }),
    }
}

pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    let dir: PathBuf = crate::kx_account::data_dir().join("browser");
    let _ = std::fs::create_dir_all(dir.join("requests"));
    let _ = std::fs::create_dir_all(dir.join("replies"));
    let requests = host.shared("kx.browser.requests", crate::kx::to_value(&Json::Array(Vec::new())));
    {
        let dir = dir.clone();
        host.action("kx.browser.reply", move |reply| {
            let reply = crate::kx::widgets::value_json(&reply);
            let Some(id) = reply["id"].as_str().filter(|id| safe(id)) else { return };
            // (Written whole, then named: the tool never reads half an answer.)
            let (half, whole) = (dir.join("replies").join(format!("{id}.tmp")), dir.join("replies").join(format!("{id}.json")));
            if std::fs::write(&half, answer(&reply).to_string()).is_ok() {
                let _ = std::fs::rename(&half, &whole);
            }
            let _ = std::fs::remove_file(dir.join("requests").join(format!("{id}.json")));
        });
    }
    cx.tasks.spawn(async move {
        let mut last = String::new();
        let mut rounds = 0u32;
        loop {
            zgx::timer::sleep(EVERY).await;
            // (Once a minute or so.)
            rounds = rounds.wrapping_add(1);
            if rounds % 500 == 1 {
                sweep(&dir);
            }
            let waiting = pending(&dir);
            // (Told only when what waits changes.)
            let ids = waiting.iter().filter_map(|r| r["id"].as_str()).collect::<Vec<_>>().join(",");
            if ids != last {
                last = ids;
                requests.set(crate::kx::to_value(&Json::Array(waiting)));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_never_paths() {
        assert!(safe("0a1b2c"));
        assert!(!safe("../x"));
        assert!(!safe(""));
    }

    #[test]
    fn a_pages_error_is_an_error() {
        assert_eq!(answer(&json!({ "id": "a", "error": "no tab" }))["ok"], false);
        assert_eq!(answer(&json!({ "id": "a", "json": "{\"error\":\"no such ref\"}" }))["error"], "no such ref");
        let ok = answer(&json!({ "id": "a", "json": "{\"url\":\"https://a.test/\"}" }));
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["result"]["url"], "https://a.test/");
    }
}
