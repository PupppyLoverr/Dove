//! Settings → Connections: the apps the person connected on their own
//! Composio account, and who may use each (everywhere, Zeron's chats, every
//! buddy, or some buddies) and when an action waits for their OK.
//!
//!   kx.connections                 { key, on, scope, buddies, rule, apps = { [slug] = { scope, buddies, rule } } }
//!   kx.connections.set(policy)     replaces all but which chats are buddies'
//!   kx.composio.check()            the key works: how many tools Composio offers
//!   kx.composio.connect(slug)      opens the app's sign-in in the browser, waits
//!                                  for it, adds it to the list: "active" | "pending"
//!   kx.composio.forget(slug)       off the list here (still connected on Composio)
//!
//! The policy is `<data>/connections.json` (zeron_composio::policy), which the
//! engine reads to give a chat Composio's bridge or not. Which chats are
//! buddies' is kept in it here, from `buddies.json` ([`sync_chats`]). The
//! key is `kx.key.set { name = "composio" }` (kx/keys.rs).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value as Json, json};
use zeron_composio::client::{Http, Upstream as _};
use zeron_composio::policy::{App, Policy};
use zgx_script::{ScriptHost, Value};

fn dir() -> PathBuf {
    crate::kx_account::data_dir()
}

/// Which chats are buddies' (chat id → buddy id), from `buddies.json`, into
/// the policy: called whenever the buddies change.
pub fn sync_chats() {
    let dir = dir();
    let list: Vec<Json> = std::fs::read(dir.join("buddies.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let chats: BTreeMap<String, String> =
        list.iter().filter_map(|b| Some((b["chat"].as_str()?.to_owned(), b["id"].as_str()?.to_owned()))).collect();
    let mut policy = Policy::load(&dir);
    if policy.buddy_chats != chats {
        policy.buddy_chats = chats;
        let _ = policy.save(&dir);
    }
}

fn snapshot() -> Value {
    let dir = dir();
    let policy = Policy::load(&dir);
    let mut out = serde_json::to_value(&policy).unwrap_or_else(|_| json!({}));
    if let Some(o) = out.as_object_mut() {
        o.remove("buddy_chats");
        o.insert("key".into(), json!(zeron_composio::key::held(&dir)));
    }
    crate::kx::to_value(&out)
}

thread_local! {
    static SHARED: RefCell<Option<zgx_script::Shared>> = const { RefCell::new(None) };
}

fn publish() {
    SHARED.with(|s| {
        if let Some(shared) = s.borrow().as_ref() {
            shared.set(snapshot());
        }
    });
}

fn set(req: &Json) -> Result<(), String> {
    let dir = dir();
    let old = Policy::load(&dir);
    let mut next: Policy = serde_json::from_value(req.clone()).map_err(|e| format!("That isn't a policy: {e}"))?;
    next.buddy_chats = old.buddy_chats;
    next.apps = next.apps.into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect();
    next.save(&dir).map_err(|e| format!("It couldn't be saved: {e}"))
}

fn edit(f: impl FnOnce(&mut Policy)) {
    let dir = dir();
    let mut policy = Policy::load(&dir);
    f(&mut policy);
    let _ = policy.save(&dir);
}

/// Composio, opened with the person's key, for `work`, on a thread of its own.
async fn with_composio<T: Send + 'static>(
    work: impl FnOnce(Http) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<T>> + Send>> + Send + 'static,
) -> Result<T, String> {
    crate::kx::off_thread(move || -> Result<T, String> {
        let key = zeron_composio::key::read(&dir()).ok_or("There's no Composio key yet.")?;
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
        runtime.block_on(async move {
            let http = Http::open(zeron_composio::ENDPOINT, key).await.map_err(|e| e.to_string())?;
            work(http).await.map_err(|e| e.to_string())
        })
    })
    .await
    .unwrap_or_else(|| Err("It stopped before it finished.".into()))
}

/// The first `key` anywhere in a tool's result (its text parts parsed).
fn find(result: &Json, key: &str) -> Option<String> {
    fn walk(v: &Json, key: &str) -> Option<String> {
        match v {
            Json::Object(m) => m.get(key).and_then(|x| x.as_str().map(str::to_owned)).or_else(|| m.values().find_map(|x| walk(x, key))),
            Json::Array(a) => a.iter().find_map(|x| walk(x, key)),
            Json::String(s) => serde_json::from_str::<Json>(s).ok().and_then(|p| if p.is_object() || p.is_array() { walk(&p, key) } else { None }),
            _ => None,
        }
    }
    walk(result, key)
}

pub fn install(host: &ScriptHost) {
    sync_chats();
    let shared = host.shared("kx.connections", snapshot());
    SHARED.with(|s| *s.borrow_mut() = Some(shared));
    host.async_action("kx.connections.set", |req| {
        let done = set(&crate::kx::widgets::value_json(&req));
        publish();
        async move { done.map(|()| Value::Nil) }
    });
    host.action("kx.connections.refresh", |_| publish());
    host.async_action("kx.composio.check", |_| async move {
        let n = with_composio(|http| {
            Box::pin(async move {
                let reply = http.send(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).await?;
                Ok(reply.and_then(|r| r.pointer("/result/tools").and_then(Json::as_array).map(Vec::len)).unwrap_or(0))
            })
        })
        .await?;
        Ok(Value::from(n as f64))
    });
    host.async_action("kx.composio.connect", |slug| {
        let slug = crate::auth::text(&slug).trim().to_ascii_lowercase();
        async move {
            if slug.is_empty() || !slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err("That isn't an app's name.".to_owned());
            }
            let toolkit = slug.clone();
            let status = with_composio(move |http| {
                Box::pin(async move {
                    let asked = http.call_tool("COMPOSIO_MANAGE_CONNECTIONS", json!({ "toolkits": [toolkit] })).await?;
                    let Some(link) = find(&asked, "redirect_url") else {
                        return Ok("active".to_owned());
                    };
                    if link.starts_with("https://") {
                        crate::kx::open_external(&link);
                    }
                    let waited = http.call_tool("COMPOSIO_WAIT_FOR_CONNECTIONS", json!({ "toolkits": [toolkit] })).await?;
                    let said = waited.to_string().to_ascii_lowercase();
                    Ok(if said.contains("active") && !said.contains("not active") { "active" } else { "pending" }.to_owned())
                })
            })
            .await?;
            edit(|p| {
                p.apps.entry(slug).or_insert_with(App::default);
            });
            publish();
            Ok(Value::from(status.as_str()))
        }
    });
    host.action("kx.composio.forget", |slug| {
        let slug = crate::auth::text(&slug).to_ascii_lowercase();
        edit(|p| {
            p.apps.remove(&slug);
        });
        publish();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_is_found_inside_a_tools_text() {
        let result = json!({ "content": [{ "type": "text", "text": "{\"results\":{\"gmail\":{\"status\":\"initiated\",\"redirect_url\":\"https://x.y/z\"}}}" }] });
        assert_eq!(find(&result, "redirect_url").as_deref(), Some("https://x.y/z"));
        assert_eq!(find(&json!({ "content": [] }), "redirect_url"), None);
    }
}
