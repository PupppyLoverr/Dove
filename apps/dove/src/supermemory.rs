//! Supermemory as Dove's memory service ([`dove_core::memory::Backend`]).
//!
//! It talks to Supermemory's own MCP server with the token Dove signed in
//! for (Settings › Integrations or › Memory), so no separate API key is
//! needed; a pasted `sm_…` key in the server's headers works the same. One
//! worker thread owns the connection and serves calls in order; a call that
//! fails reconnects once.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use dove_core::memory::{self, Doc, Memory, Scope};
use serde_json::{Value, json};

use crate::mcp_browser::upstream;
use crate::pages::integrations::{self as cfg, McpServer, Transport};

/// The catalog entry.
pub const CATALOG_ID: &str = "supermemory";

/// The user's Supermemory server, if added and enabled.
pub fn server(config: &cfg::Config) -> Option<McpServer> {
    config.servers.iter().find(|s| s.enabled && s.catalog.as_deref() == Some(CATALOG_ID)).cloned()
}

/// Whether `server` has credentials: an OAuth sign-in or a pasted key.
pub fn authorized(server: &McpServer) -> bool {
    match &server.transport {
        Transport::Http { oauth: true, .. } => crate::mcp_oauth::signed_in(&server.id),
        Transport::Http { headers, .. } => !headers.is_empty(),
        Transport::Stdio { .. } => true,
    }
}

/// Register the backend when Supermemory is connected (and clear it when
/// not). Called at startup and whenever integrations are saved.
pub fn refresh(data: &std::path::Path) {
    let config = cfg::load();
    match server(&config).filter(authorized) {
        Some(s) => {
            let mut current = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
            if current.as_ref() != Some(&s) {
                *current = Some(s.clone());
                memory::set_backend(Some(Arc::new(Supermemory::start(s))));
                memory::flush_outbox(data);
            }
        }
        None => {
            *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = None;
            memory::set_backend(None);
        }
    }
}

static CURRENT: std::sync::Mutex<Option<McpServer>> = std::sync::Mutex::new(None);

type Job = Box<dyn FnOnce(&mut Conn) + Send>;

pub struct Supermemory {
    jobs: mpsc::Sender<Job>,
}

/// The live connection and what its tools accept.
struct Conn {
    def: McpServer,
    server: Option<upstream::Server>,
    /// Tool name → its input properties.
    tools: Vec<(String, Vec<String>)>,
}

/// How long one call may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

impl Supermemory {
    fn start(def: McpServer) -> Self {
        let (jobs, rx) = mpsc::channel::<Job>();
        let _ = std::thread::Builder::new().name("dove-supermemory".into()).spawn(move || {
            let mut conn = Conn { def, server: None, tools: Vec::new() };
            while let Ok(job) = rx.recv() {
                job(&mut conn);
            }
        });
        Self { jobs }
    }

    fn run<T: Send + 'static>(&self, f: impl FnOnce(&mut Conn) -> Result<T, String> + Send + 'static) -> Result<T, String> {
        let (tx, rx) = mpsc::channel();
        self.jobs
            .send(Box::new(move |c| {
                let _ = tx.send(f(c));
            }))
            .map_err(|_| "the Supermemory connection stopped".to_string())?;
        rx.recv_timeout(CALL_TIMEOUT).unwrap_or_else(|_| Err("Supermemory did not answer in time".into()))
    }
}

impl Conn {
    fn connect(&mut self) -> Result<(), String> {
        if self.server.is_some() {
            return Ok(());
        }
        let mut server = upstream::make(&self.def.id, &self.def.transport)()?;
        server.initialize()?;
        self.tools = server
            .list_tools()?
            .into_iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?.to_string();
                let props = t
                    .pointer("/inputSchema/properties")
                    .and_then(Value::as_object)
                    .map(|o| o.keys().cloned().collect())
                    .unwrap_or_default();
                Some((name, props))
            })
            .collect();
        self.server = Some(server);
        Ok(())
    }

    /// The first of `names` the server offers, with its input properties.
    fn tool(&self, names: &[&str]) -> Option<(String, Vec<String>)> {
        names.iter().find_map(|n| self.tools.iter().find(|(t, _)| t.eq_ignore_ascii_case(n)).cloned())
    }

    /// Call the first tool of `names` the server has, building its arguments
    /// from what it accepts. Reconnects once on failure.
    fn call(&mut self, names: &[&str], args: impl Fn(&[String]) -> Value) -> Result<Value, String> {
        for attempt in 0..2 {
            self.connect()?;
            let (tool, props) =
                self.tool(names).ok_or_else(|| format!("Supermemory offers none of {}", names.join(", ")))?;
            let server = self.server.as_mut().expect("connected");
            match server.request("tools/call", json!({"name": tool, "arguments": args(&props)})) {
                Ok(result) => {
                    if result.get("isError").and_then(Value::as_bool) == Some(true) {
                        return Err(text_of(&result).unwrap_or_else(|| "Supermemory reported an error".into()));
                    }
                    return Ok(result);
                }
                Err(e) if attempt == 0 => {
                    tracing::debug!("supermemory {tool}: {e}; reconnecting");
                    self.server = None;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }
}

/// Put `value` under the first of `keys` the tool accepts (or the first key
/// when its schema lists no properties).
fn put(args: &mut serde_json::Map<String, Value>, props: &[String], keys: &[&str], value: Value) {
    let key = keys.iter().find(|k| props.iter().any(|p| p == *k)).or_else(|| props.is_empty().then(|| &keys[0]));
    if let Some(k) = key {
        args.insert((*k).to_string(), value);
    }
}

fn with_scope(args: &mut serde_json::Map<String, Value>, props: &[String], scope: &Scope, write: bool) {
    if let Some(tag) = scope.container() {
        // Writes take `namespace` (`containerTag` is deprecated there).
        let keys: &[&str] = if write { &["namespace", "containerTag"] } else { &["containerTag", "namespace"] };
        put(args, props, keys, json!(tag));
    }
}

impl memory::Backend for Supermemory {
    fn search(&self, query: &str, scope: &Scope, limit: usize) -> Result<Vec<Memory>, String> {
        let (query, scope) = (query.to_string(), scope.clone());
        self.run(move |c| {
            let r = c.call(&["search_memory", "recall", "search"], |props| {
                let mut a = serde_json::Map::new();
                put(&mut a, props, &["query", "q", "informationToGet"], json!(query));
                put(&mut a, props, &["limit"], json!(limit));
                with_scope(&mut a, props, &scope, false);
                Value::Object(a)
            })?;
            Ok(memories_of(&r).into_iter().take(limit).collect())
        })
    }

    fn profile(&self, scope: &Scope) -> Result<Vec<Memory>, String> {
        let scope = scope.clone();
        self.run(move |c| {
            let r = c.call(&["get_profile", "profile"], |props| {
                let mut a = serde_json::Map::new();
                with_scope(&mut a, props, &scope, false);
                Value::Object(a)
            })?;
            Ok(memories_of(&r))
        })
    }

    fn add(&self, doc: &Doc) -> Result<(), String> {
        let doc = doc.clone();
        self.run(move |c| {
            c.call(&["add_memory", "addMemory", "memory"], |props| {
                let mut a = serde_json::Map::new();
                put(&mut a, props, &["content", "thingToRemember", "memory", "text"], json!(doc.text_with_metadata()));
                put(&mut a, props, &["action"], json!("save"));
                put(&mut a, props, &["customId"], doc.custom_id.clone().map(Value::from).unwrap_or(Value::Null));
                put(&mut a, props, &["metadata"], json!(doc.metadata));
                with_scope(&mut a, props, &doc.scope, true);
                a.retain(|_, v| !v.is_null());
                Value::Object(a)
            })
            .map(|_| ())
        })
    }

    fn list(&self, scope: &Scope, page: usize, limit: usize) -> Result<Vec<Memory>, String> {
        let scope = scope.clone();
        self.run(move |c| {
            let r = c.call(&["list_memories", "list_documents"], |props| {
                let mut a = serde_json::Map::new();
                put(&mut a, props, &["page"], json!(page));
                put(&mut a, props, &["limit"], json!(limit.min(50)));
                with_scope(&mut a, props, &scope, false);
                Value::Object(a)
            })?;
            Ok(memories_of(&r))
        })
    }

    fn forget(&self, m: &Memory, scope: &Scope) -> Result<(), String> {
        let (m, scope) = (m.clone(), scope.clone());
        self.run(move |c| {
            c.call(&["forget_memory", "memory"], |props| {
                let mut a = serde_json::Map::new();
                put(&mut a, props, &["content", "thingToForget", "memory", "text"], json!(m.text));
                put(&mut a, props, &["action"], json!("forget"));
                if let Some(id) = &m.id {
                    put(&mut a, props, &["memoryId", "id"], json!(id));
                }
                with_scope(&mut a, props, &scope, true);
                Value::Object(a)
            })
            .map(|_| ())
        })
    }

    fn who_am_i(&self) -> Result<String, String> {
        self.run(|c| {
            let r = c.call(&["who_am_i", "whoAmI", "whoami"], |_| json!({}))?;
            let v = structured(&r);
            let pick = |k: &str| v.as_ref().and_then(|v| v.get(k)?.as_str().map(str::to_string));
            Ok(pick("email")
                .or_else(|| pick("name"))
                .or_else(|| pick("userId"))
                .or_else(|| text_of(&r).map(|t| t.lines().next().unwrap_or_default().to_string()))
                .unwrap_or_else(|| "Signed in".into()))
        })
    }
}

/// All text parts of a tool result, joined.
fn text_of(result: &Value) -> Option<String> {
    let parts: Vec<&str> = result
        .get("content")?
        .as_array()?
        .iter()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|c| c.get("text")?.as_str())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

/// `structuredContent`, or a text part that is JSON.
fn structured(result: &Value) -> Option<Value> {
    if let Some(s) = result.get("structuredContent").filter(|s| !s.is_null()) {
        return Some(s.clone());
    }
    let text = text_of(result)?;
    serde_json::from_str(text.trim()).ok()
}

/// The memories in a tool result, whatever shape it has.
pub fn memories_of(result: &Value) -> Vec<Memory> {
    if let Some(v) = structured(result) {
        let mut out = Vec::new();
        collect(&v, &mut out, 0);
        if !out.is_empty() {
            return out;
        }
    }
    text_of(result).map(|t| memories_in_text(&t)).unwrap_or_default()
}

const TEXT_KEYS: &[&str] = &["memory", "content", "text", "summary", "chunk", "fact", "title"];
const LIST_KEYS: &[&str] =
    &["results", "memories", "documents", "items", "data", "static", "dynamic", "profile", "chunks", "facts"];

fn collect(v: &Value, out: &mut Vec<Memory>, depth: usize) {
    if depth > 4 {
        return;
    }
    match v {
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(s) if !s.trim().is_empty() => out.push(Memory::new(s.trim())),
                    Value::Object(o) => {
                        let text = TEXT_KEYS.iter().find_map(|k| o.get(*k)?.as_str().filter(|s| !s.trim().is_empty()));
                        match text {
                            Some(t) => out.push(Memory {
                                id: ["id", "memoryId", "documentId", "customId"]
                                    .iter()
                                    .find_map(|k| o.get(*k)?.as_str().map(str::to_string)),
                                text: t.trim().to_string(),
                                score: ["similarity", "score", "relevance"]
                                    .iter()
                                    .find_map(|k| o.get(*k)?.as_f64())
                                    .map(|s| s as f32),
                            }),
                            None => collect(item, out, depth + 1),
                        }
                    }
                    other => collect(other, out, depth + 1),
                }
            }
        }
        Value::Object(o) => {
            for k in LIST_KEYS {
                if let Some(child) = o.get(*k) {
                    collect(child, out, depth + 1);
                }
            }
        }
        _ => {}
    }
}

/// Memories in a plain-text answer: list items, else paragraphs; headings
/// and empty-result notes are skipped.
pub fn memories_in_text(text: &str) -> Vec<Memory> {
    let bullet = |l: &str| -> Option<String> {
        let l = l.trim();
        let rest = l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")).or_else(|| l.strip_prefix("• ")).or_else(|| {
            let digits = l.chars().take_while(char::is_ascii_digit).count();
            (digits > 0).then(|| l[digits..].strip_prefix(". ").or_else(|| l[digits..].strip_prefix(") "))).flatten()
        })?;
        Some(rest.trim().to_string()).filter(|s| !s.is_empty())
    };
    let items: Vec<Memory> = text.lines().filter_map(bullet).map(Memory::new).collect();
    if !items.is_empty() {
        return items;
    }
    let lower = text.to_lowercase();
    if lower.contains("no memories") || lower.contains("no results") || lower.contains("nothing found") {
        return Vec::new();
    }
    text.split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty() && !(p.ends_with(':') && !p.contains('\n')))
        .map(Memory::new)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> Value {
        json!({"content": [{"type": "text", "text": t}]})
    }

    #[test]
    fn parses_json_results() {
        let r = text(r#"{"results":[{"id":"m1","memory":"Uses pnpm","similarity":0.9},{"content":"Tabs","score":0.4}]}"#);
        let m = memories_of(&r);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].id.as_deref(), Some("m1"));
        assert_eq!(m[0].score, Some(0.9));
        assert_eq!(m[1].text, "Tabs");
        let profile = json!({"structuredContent": {"profile": {"static": ["Name is Sam"], "dynamic": ["Working on Dove"]}}});
        assert_eq!(memories_of(&profile).len(), 2);
    }

    #[test]
    fn parses_text_results() {
        let m = memories_of(&text("Found 2 memories:\n- Uses pnpm\n- Runs cargo test -j 2"));
        assert_eq!(m.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["Uses pnpm", "Runs cargo test -j 2"]);
        assert!(memories_of(&text("No memories found.")).is_empty());
        let paras = memories_of(&text("Profile:\n\nPrefers dark mode.\n\nWorks in Rust."));
        assert_eq!(paras.len(), 2);
    }

    #[test]
    fn arguments_follow_the_schema() {
        let mut a = serde_json::Map::new();
        let props = vec!["content".to_string(), "namespace".to_string()];
        put(&mut a, &props, &["content", "thingToRemember"], json!("x"));
        with_scope(&mut a, &props, &Scope::Project("repo_a__1".into()), true);
        put(&mut a, &props, &["customId"], json!("c"));
        assert_eq!(Value::Object(a), json!({"content": "x", "namespace": "repo_a__1"}));
        let mut b = serde_json::Map::new();
        with_scope(&mut b, &["containerTag".to_string()], &Scope::User, false);
        assert!(b.is_empty());
    }
}
