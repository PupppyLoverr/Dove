//! The stdio MCP server a chat gets for Composio: every message goes on to
//! Composio with the person's key added, and the chat is held to the policy
//! on the way (see [`crate::policy`]):
//!
//! - An app the chat may not use is refused, whichever of Composio's tools
//!   names it, and dropped from what its searches and connection lists say.
//! - Composio's remote sandboxes (Python, bash), which can reach any app,
//!   are only offered where every app is allowed.
//! - An action that waits under its app's rule (by default: anything that
//!   sends, posts, deletes or changes) is turned back until the agent has
//!   shown the person what it'll do and they've said yes: the agent then
//!   calls again with `"confirmed": true`.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

use crate::client::Upstream;
use crate::policy::{Place, Policy};

const MULTI: &str = "COMPOSIO_MULTI_EXECUTE_TOOL";
const SEARCH: &str = "COMPOSIO_SEARCH_TOOLS";
const MANAGE: &str = "COMPOSIO_MANAGE_CONNECTIONS";
const SANDBOXES: &[&str] = &["COMPOSIO_REMOTE_WORKBENCH", "COMPOSIO_REMOTE_BASH_TOOL"];

pub struct Bridge<U> {
    pub upstream: U,
    pub policy: Policy,
    pub place: Place,
}

/// Every string under a key that names a tool (`tool_slug`, `slug`) or an
/// app (`toolkit`, `toolkits`, `toolkit_slug`, `app`), wherever it sits.
fn named(value: &Value, out: &mut Vec<(bool, String)>) {
    fn strings(v: &Value, tool: bool, out: &mut Vec<(bool, String)>) {
        match v {
            Value::String(s) => out.push((tool, s.clone())),
            Value::Array(items) => items.iter().for_each(|i| strings(i, tool, out)),
            _ => {}
        }
    }
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let k = k.to_ascii_lowercase();
                // (What a tool is given is its own: a page's `slug` there
                // isn't a tool.)
                if k == "arguments" {
                    continue;
                }
                if k == "tool_slug" || k == "slug" || k == "tool" {
                    strings(v, true, out);
                } else if k.contains("toolkit") || k == "app" || k == "apps" {
                    strings(v, false, out);
                } else {
                    named(v, out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|i| named(i, out)),
        _ => {}
    }
}

fn result(id: Value, text: String, error: bool) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [{ "type": "text", "text": text }], "isError": error } })
}

impl<U: Upstream> Bridge<U> {
    fn app(&self, (tool, name): &(bool, String)) -> String {
        if *tool { self.policy.app_of(name) } else { name.to_ascii_lowercase() }
    }

    fn here(&self) -> &'static str {
        if self.place.buddy.is_some() { "this buddy" } else { "this chat" }
    }

    /// Why a call can't go on as it is, if it can't.
    fn check(&self, name: &str, args: &Value) -> Option<String> {
        if SANDBOXES.contains(&name) && !self.policy.unrestricted(&self.place) {
            return Some(format!("{name} isn't available to {}: some connected apps are kept from it.", self.here()));
        }
        let mut names = Vec::new();
        named(args, &mut names);
        let mut blocked: Vec<String> = names.iter().map(|n| self.app(n)).filter(|app| !self.policy.allows(app, &self.place)).collect();
        blocked.sort();
        blocked.dedup();
        if !blocked.is_empty() {
            return Some(format!(
                "{} isn't connected for {}. The person can change who may use it in Keplorayx: Settings → Connections.",
                blocked.join(", "),
                self.here()
            ));
        }
        if name == MULTI && args.get("confirmed").and_then(Value::as_bool) != Some(true) {
            let waiting: Vec<&str> = names.iter().filter(|(tool, _)| *tool).map(|(_, s)| s.as_str()).filter(|s| self.policy.waits(s)).collect();
            if !waiting.is_empty() {
                return Some(format!(
                    "Waiting for the person's OK: {} would change something. Tell them exactly what each will do (who it goes to, what it says or changes) and ask. Only after they say yes, call {MULTI} again with the same tools and \"confirmed\": true. Never set it without their yes.",
                    waiting.join(", ")
                ));
            }
        }
        None
    }

    /// Drops, from a search's or a connection list's JSON, every entry for
    /// an app this place may not use.
    fn prune(&self, value: &mut Value) {
        match value {
            Value::Array(items) => {
                items.retain(|item| {
                    let Value::Object(map) = item else { return true };
                    let mut own = Vec::new();
                    for (k, v) in map {
                        if let Value::String(s) = v {
                            let k = k.to_ascii_lowercase();
                            if k == "tool_slug" || k == "slug" {
                                own.push((true, s.clone()));
                            } else if k.contains("toolkit") || k == "app" {
                                own.push((false, s.clone()));
                            }
                        }
                    }
                    own.iter().all(|n| self.policy.allows(&self.app(n), &self.place))
                });
                items.iter_mut().for_each(|i| self.prune(i));
            }
            Value::Object(map) => map.values_mut().for_each(|v| self.prune(v)),
            _ => {}
        }
    }

    fn filter_result(&self, name: &str, mut reply: Value) -> Value {
        if name != SEARCH && name != MANAGE {
            return reply;
        }
        if let Some(content) = reply.pointer_mut("/result/content").and_then(Value::as_array_mut) {
            for part in content {
                let Some(text) = part.get("text").and_then(Value::as_str) else { continue };
                let Ok(mut parsed) = serde_json::from_str::<Value>(text) else { continue };
                self.prune(&mut parsed);
                part["text"] = Value::String(parsed.to_string());
            }
        }
        reply
    }

    fn filter_list(&self, mut reply: Value) -> Value {
        let unrestricted = self.policy.unrestricted(&self.place);
        if let Some(tools) = reply.pointer_mut("/result/tools").and_then(Value::as_array_mut) {
            tools.retain(|t| unrestricted || !t.get("name").and_then(Value::as_str).is_some_and(|n| SANDBOXES.contains(&n)));
            for tool in tools.iter_mut() {
                if tool.get("name").and_then(Value::as_str) == Some(MULTI) {
                    if let Some(props) = tool.pointer_mut("/inputSchema/properties").and_then(Value::as_object_mut) {
                        props.insert(
                            "confirmed".into(),
                            json!({ "type": "boolean", "description": "Set only after the person has seen what these actions will do and said yes. Actions that change something are turned back without it." }),
                        );
                    }
                }
            }
        }
        reply
    }

    async fn forward(&self, message: Value) -> Option<Value> {
        let id = message.get("id").cloned();
        match self.upstream.send(message).await {
            Ok(reply) => reply,
            Err(why) => id.map(|id| json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": why.to_string() } })),
        }
    }

    /// One message in; the reply to send back, if it wants one.
    pub async fn handle(&self, mut message: Value) -> Option<Value> {
        let id = message.get("id").cloned();
        match message.get("method").and_then(Value::as_str) {
            Some("tools/list") => self.forward(message).await.map(|r| self.filter_list(r)),
            Some("tools/call") => {
                let name = message.pointer("/params/name").and_then(Value::as_str).unwrap_or_default().to_owned();
                let args = message.pointer("/params/arguments").cloned().unwrap_or(Value::Null);
                if let Some(why) = self.check(&name, &args) {
                    return id.map(|id| result(id, why, true));
                }
                // (Ours, not Composio's: it doesn't go on.)
                if let Some(args) = message.pointer_mut("/params/arguments").and_then(Value::as_object_mut) {
                    args.remove("confirmed");
                }
                self.forward(message).await.map(|r| self.filter_result(&name, r))
            }
            _ => self.forward(message).await,
        }
    }
}

/// Serves `bridge` on stdin/stdout, a JSON-RPC message a line, until stdin
/// closes. Calls run side by side; replies go out as they come.
pub async fn serve_stdio<U: Upstream + 'static>(bridge: Arc<Bridge<U>>) -> anyhow::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        let mut out = tokio::io::stdout();
        while let Some(reply) = rx.recv().await {
            let mut line = reply.to_string();
            line.push('\n');
            if out.write_all(line.as_bytes()).await.is_err() || out.flush().await.is_err() {
                break;
            }
        }
    });
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        let (bridge, tx) = (bridge.clone(), tx.clone());
        tokio::spawn(async move {
            if let Some(reply) = bridge.handle(message).await {
                let _ = tx.send(reply);
            }
        });
    }
    drop(tx);
    let _ = writer.await;
    Ok(())
}

/// The bridge for `chat`, as the engine starts it: the policy and key from
/// `data_dir`.
pub async fn run(data_dir: &Path, chat: Option<&str>) -> anyhow::Result<()> {
    let policy = Policy::load(data_dir);
    let place = policy.place(chat);
    let key = crate::key::read(data_dir).unwrap_or_default();
    let upstream = crate::client::Http::new(crate::ENDPOINT, key);
    serve_stdio(Arc::new(Bridge { upstream, policy, place })).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{App, Rule, Scope};
    use std::sync::Mutex;

    /// Composio, played: answers `tools/list` with its 7 tools, a search
    /// with two apps' tools, and anything else with what it was sent.
    #[derive(Default)]
    struct Fake {
        seen: Mutex<Vec<Value>>,
    }

    #[async_trait::async_trait]
    impl Upstream for Fake {
        async fn send(&self, message: Value) -> anyhow::Result<Option<Value>> {
            self.seen.lock().unwrap().push(message.clone());
            let id = message.get("id").cloned();
            let Some(id) = id else { return Ok(None) };
            let tools: Vec<Value> = ["COMPOSIO_SEARCH_TOOLS", "COMPOSIO_MULTI_EXECUTE_TOOL", "COMPOSIO_REMOTE_WORKBENCH", "COMPOSIO_REMOTE_BASH_TOOL"]
                .iter()
                .map(|n| json!({ "name": n, "inputSchema": { "type": "object", "properties": {} } }))
                .collect();
            Ok(Some(match message["method"].as_str() {
                Some("tools/list") => json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools } }),
                Some("tools/call") if message["params"]["name"] == SEARCH => {
                    let found = json!({ "results": [{ "tool_slug": "GMAIL_SEND_EMAIL" }, { "tool_slug": "SLACK_SEND_MESSAGE" }] });
                    json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [{ "type": "text", "text": found.to_string() }] } })
                }
                _ => json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [{ "type": "text", "text": "done" }], "echo": message } }),
            }))
        }
    }

    fn bridge(policy: Policy, buddy: Option<&str>) -> Bridge<Fake> {
        Bridge { upstream: Fake::default(), policy, place: Place { buddy: buddy.map(str::to_owned) } }
    }

    fn call(name: &str, args: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } })
    }

    fn gmail_for_b1() -> Policy {
        let mut p = Policy::default();
        p.apps.insert("gmail".into(), App { scope: Scope::Some, buddies: vec!["b1".into()], rule: Rule::AskChanges });
        p
    }

    #[tokio::test]
    async fn an_app_kept_from_a_buddy_is_refused_and_never_sent() {
        let b = bridge(gmail_for_b1(), Some("b2"));
        let reply = b.handle(call(MULTI, json!({ "tools": [{ "tool_slug": "GMAIL_FETCH_EMAILS", "arguments": {} }] }))).await.unwrap();
        assert_eq!(reply["result"]["isError"], json!(true));
        assert!(reply["result"]["content"][0]["text"].as_str().unwrap().contains("gmail isn't connected"));
        assert!(b.upstream.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn reading_goes_through_and_a_send_waits_for_confirmed() {
        let b = bridge(gmail_for_b1(), Some("b1"));
        let read = b.handle(call(MULTI, json!({ "tools": [{ "tool_slug": "GMAIL_FETCH_EMAILS" }] }))).await.unwrap();
        assert_eq!(read["result"]["content"][0]["text"], json!("done"));
        let send = json!({ "tools": [{ "tool_slug": "GMAIL_SEND_EMAIL", "arguments": { "to": "a@b.c" } }] });
        let held = b.handle(call(MULTI, send.clone())).await.unwrap();
        assert!(held["result"]["content"][0]["text"].as_str().unwrap().starts_with("Waiting for the person's OK"));
        let mut ok = send;
        ok["confirmed"] = json!(true);
        let went = b.handle(call(MULTI, ok)).await.unwrap();
        // (It went, without our `confirmed`.)
        assert_eq!(went["result"]["content"][0]["text"], json!("done"));
        assert!(went["result"]["echo"]["params"]["arguments"].get("confirmed").is_none());
    }

    #[tokio::test]
    async fn searches_and_the_tool_list_leave_out_what_isnt_allowed_here() {
        let b = bridge(gmail_for_b1(), None);
        let found = b.handle(call(SEARCH, json!({ "queries": [{ "use_case": "send a message" }] }))).await.unwrap();
        let text = found["result"]["content"][0]["text"].as_str().unwrap();
        assert!(!text.contains("GMAIL") && text.contains("SLACK_SEND_MESSAGE"));
        let list = b.handle(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).await.unwrap();
        let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(!names.contains(&"COMPOSIO_REMOTE_WORKBENCH") && names.contains(&MULTI));
        let multi = list["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == MULTI).unwrap();
        assert!(multi["inputSchema"]["properties"]["confirmed"].is_object());
    }

    #[tokio::test]
    async fn with_everything_allowed_the_sandboxes_stay() {
        let b = bridge(Policy::default(), Some("b1"));
        let list = b.handle(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).await.unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 4);
        let sandbox = b.handle(call("COMPOSIO_REMOTE_BASH_TOOL", json!({ "command": "ls" }))).await.unwrap();
        assert_eq!(sandbox["result"]["content"][0]["text"], json!("done"));
    }

    #[tokio::test]
    async fn notifications_go_on_with_no_reply() {
        let b = bridge(Policy::default(), None);
        assert!(b.handle(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await.is_none());
        assert_eq!(b.upstream.seen.lock().unwrap().len(), 1);
    }
}
