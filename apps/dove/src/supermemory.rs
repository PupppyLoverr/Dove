//! Supermemory as Dove's memory service ([`dove_core::memory::Backend`]).
//!
//! It uses the token Dove signed in for (Settings › Integrations or ›
//! Memory), so no separate API key is needed; a pasted `sm_…` key in the
//! server's headers works the same. Two surfaces, one credential:
//!
//! - Supermemory's MCP server for extracted memories (`search_memory`,
//!   `get_profile`, `list_memories`, `forget_memory`, `who_am_i`).
//! - The REST API (`/v3/documents`, `/v3/search`) for saves with real
//!   metadata and an idempotent `customId`, and for document search. Memory
//!   extraction runs later on Supermemory's side (minutes, sometimes much
//!   longer), so recall also searches documents: a turn captured a moment
//!   ago is found on the very next prompt.
//!
//! One worker thread owns the connection and serves calls in order; a call
//! that fails reconnects once.

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
    /// REST calls run on the caller's thread, in parallel with MCP ones.
    rest: Arc<Rest>,
}

/// Supermemory's REST API with the server's credential.
struct Rest {
    def: McpServer,
    http: reqwest::Client,
}

/// The live connection and what its tools accept.
struct Conn {
    def: McpServer,
    server: Option<upstream::Server>,
    /// Tool name → its input properties.
    tools: Vec<(String, Vec<String>)>,
    /// The server asks every call to pass back the `conversation_id` its
    /// last answer gave.
    conversation: Option<String>,
}

/// Supermemory's REST API.
const API: &str = "https://api.supermemory.ai";
/// The account's default space: where the MCP tools put memories saved
/// without a namespace, so the user scope over REST.
const DEFAULT_SPACE: &str = "sm_project_default";

/// How long one call may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// How long recall waits for the MCP side (extracted memories, profile).
/// Calls share one connection and queue, so a slow one must not cost the
/// prompt its REST results: recall as a whole has a few seconds.
const RECALL_MCP_WAIT: Duration = Duration::from_millis(2500);

impl Supermemory {
    fn start(def: McpServer) -> Self {
        let (jobs, rx) = mpsc::channel::<Job>();
        let http = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().unwrap_or_default();
        let rest = Arc::new(Rest { def: def.clone(), http });
        let _ = std::thread::Builder::new().name("dove-supermemory".into()).spawn(move || {
            let mut conn = Conn { def, server: None, tools: Vec::new(), conversation: None };
            while let Ok(job) = rx.recv() {
                job(&mut conn);
            }
        });
        Self { jobs, rest }
    }

    fn run<T: Send + 'static>(&self, f: impl FnOnce(&mut Conn) -> Result<T, String> + Send + 'static) -> Result<T, String> {
        self.run_for(CALL_TIMEOUT, f)
    }

    /// [`Self::run`], waiting at most `wait` (the call itself still finishes
    /// on the worker; its answer is dropped).
    fn run_for<T: Send + 'static>(
        &self,
        wait: Duration,
        f: impl FnOnce(&mut Conn) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (tx, rx) = mpsc::channel();
        self.jobs
            .send(Box::new(move |c| {
                let _ = tx.send(f(c));
            }))
            .map_err(|_| "the Supermemory connection stopped".to_string())?;
        rx.recv_timeout(wait).unwrap_or_else(|_| Err("Supermemory did not answer in time".into()))
    }
}

impl Rest {
    /// The credential for the REST API: the OAuth token, or a pasted key.
    fn bearer(&self) -> Option<String> {
        match &self.def.transport {
            Transport::Http { oauth: true, .. } => crate::mcp_oauth::token(&self.def.id),
            Transport::Http { headers, .. } => headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("authorization") || k.eq_ignore_ascii_case("x-api-key"))
                .map(|(_, v)| v.trim().trim_start_matches("Bearer ").trim().to_string()),
            Transport::Stdio { .. } => None,
        }
    }

    /// POST `body` to `path` on the REST API. Refreshes the sign-in once on 401.
    fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        self.send(reqwest::Method::POST, path, Some(body))
    }

    fn send(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value, String> {
        let mut token = self.bearer().ok_or("no Supermemory credential")?;
        for attempt in 0..2 {
            let mut req = self.http.request(method.clone(), format!("{API}{path}")).bearer_auth(&token);
            if let Some(b) = &body {
                req = req.json(b);
            }
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
            let (status, text) = rt.block_on(async {
                let resp = req.send().await.map_err(|e| format!("could not reach Supermemory: {e}"))?;
                let status = resp.status();
                Ok::<_, String>((status, resp.text().await.unwrap_or_default()))
            })?;
            if status == reqwest::StatusCode::UNAUTHORIZED
                && attempt == 0
                && matches!(self.def.transport, Transport::Http { oauth: true, .. })
            {
                token = crate::mcp_oauth::refresh(&self.def.id)?;
                continue;
            }
            if !status.is_success() {
                return Err(format!("Supermemory {path} answered {status}: {}", text.chars().take(200).collect::<String>()));
            }
            if text.trim().is_empty() {
                return Ok(Value::Null);
            }
            return serde_json::from_str(&text).map_err(|e| format!("Supermemory {path}: {e}"));
        }
        Err("Supermemory refused the credentials".into())
    }

    /// One page of a container's documents with their text (newest first).
    /// Memory extraction lags behind saves, so listing falls back to this.
    fn documents(&self, tag: &str, page: usize, limit: usize) -> Result<Vec<Memory>, String> {
        let list = self.post("/v3/documents/list", json!({"containerTags": [tag], "page": page, "limit": limit}))?;
        let ids: Vec<String> = list
            .get("memories")
            .or_else(|| list.get("documents"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|d| d.get("id")?.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let mut out = Vec::new();
        for id in ids {
            let doc = self.send(reqwest::Method::GET, &format!("/v3/documents/{id}"), None)?;
            let text = ["content", "raw", "summary", "title"]
                .iter()
                .find_map(|k| doc.get(*k)?.as_str().filter(|t| !t.trim().is_empty()))
                .unwrap_or_default();
            if !text.is_empty() {
                let at = doc.get("createdAt").and_then(Value::as_str).map(str::to_string);
                out.push(Memory { id: Some(id), text: text.trim().to_string(), score: None, at });
            }
        }
        Ok(out)
    }

    fn delete_document(&self, id: &str) -> Result<(), String> {
        self.send(reqwest::Method::DELETE, &format!("/v3/documents/{id}"), None).map(|_| ())
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
            let mut arguments = args(&props);
            if let (Some(c), Some(a)) = (&self.conversation, arguments.as_object_mut())
                && props.iter().any(|p| p == "conversation_id")
            {
                a.insert("conversation_id".into(), json!(c));
            }
            let server = self.server.as_mut().expect("connected");
            match server.request("tools/call", json!({"name": tool, "arguments": arguments})) {
                Ok(result) => {
                    if let Some(c) = conversation_of(&result) {
                        self.conversation = Some(c);
                    }
                    if result.get("isError").and_then(Value::as_bool) == Some(true) {
                        return Err(text_of(&result).unwrap_or_else(|| "Supermemory reported an error".into()));
                    }
                    return Ok(result);
                }
                Err(e) if attempt == 0 => {
                    tracing::debug!("supermemory {tool}: {e}; reconnecting");
                    self.server = None;
                    self.conversation = None;
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
        let tag = scope.container().unwrap_or(DEFAULT_SPACE);
        let body = json!({"q": query, "limit": limit, "onlyMatchingChunks": true, "containerTags": [tag]});
        let rest = self.rest.clone();
        let documents = std::thread::spawn(move || rest.post("/v3/search", body).map(|r| relevant_documents(&r)));
        let memories = self.run_for(RECALL_MCP_WAIT, move |c| {
            c.call(&["search_memory", "recall", "search"], |props| {
                let mut a = serde_json::Map::new();
                put(&mut a, props, &["query", "q", "informationToGet"], json!(query));
                put(&mut a, props, &["limit"], json!(limit));
                with_scope(&mut a, props, &scope, false);
                Value::Object(a)
            })
            .map(|r| memories_of(&r))
        });
        let documents = documents.join().unwrap_or_else(|_| Err("document search stopped".into()));
        {
            let mut out = match (memories, documents) {
                (Err(a), Err(b)) => return Err(format!("{a}; {b}")),
                (m, d) => {
                    if let Err(e) = &m {
                        tracing::debug!("supermemory memory search: {e}");
                    }
                    if let Err(e) = &d {
                        tracing::debug!("supermemory document search: {e}");
                    }
                    let mut all = m.unwrap_or_default();
                    all.extend(d.unwrap_or_default());
                    all
                }
            };
            out.sort_by(|a, b| b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)));
            let mut seen = std::collections::HashSet::new();
            out.retain(|m| seen.insert(m.text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()));
            out.truncate(limit);
            Ok(out)
        }
    }

    fn standing(&self, scope: &Scope, query: &str, limit: usize) -> Result<Vec<Memory>, String> {
        let tag = scope.container().unwrap_or(DEFAULT_SPACE);
        // Every saved correction and stated rule, ranked by the prompt but
        // never dropped for scoring low: they apply to everything.
        let body = json!({
            "q": if query.trim().is_empty() { "rules" } else { query },
            "limit": limit,
            "onlyMatchingChunks": false,
            "containerTags": [tag],
            "filters": {"OR": [{"key": "kind", "value": "feedback"}, {"key": "kind", "value": "rule"}]},
        });
        let mut out = documents_of(&self.rest.post("/v3/search", body)?);
        out.sort_by(|a, b| b.at.cmp(&a.at));
        Ok(out)
    }

    fn profile(&self, scope: &Scope) -> Result<Vec<Memory>, String> {
        let scope = scope.clone();
        self.run_for(RECALL_MCP_WAIT, move |c| {
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
        let tag = doc.scope.container().unwrap_or(DEFAULT_SPACE);
        let mut body = json!({"content": doc.content, "metadata": doc.metadata, "containerTags": [tag]});
        if let Some(id) = &doc.custom_id {
            body["customId"] = json!(id);
        }
        match self.rest.post("/v3/documents", body) {
            Ok(_) => return Ok(()),
            Err(e) => tracing::debug!("supermemory REST save: {e}; trying MCP"),
        }
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
        let tag = scope.container().unwrap_or(DEFAULT_SPACE).to_string();
        let scope = scope.clone();
        let memories = self.run(move |c| {
            let r = c.call(&["list_memories"], |props| {
                let mut a = serde_json::Map::new();
                put(&mut a, props, &["page"], json!(page));
                put(&mut a, props, &["limit"], json!(limit.min(50)));
                with_scope(&mut a, props, &scope, false);
                Value::Object(a)
            })?;
            Ok(memories_of(&r))
        });
        match memories {
            Ok(list) if !list.is_empty() => Ok(list),
            // Nothing extracted yet: the saved documents themselves.
            other => self.rest.documents(&tag, page, limit.min(50)).or_else(|e| other.map_err(|m| format!("{m}; {e}"))),
        }
    }

    fn forget(&self, m: &Memory, scope: &Scope) -> Result<(), String> {
        // A document (from document search or listing) is deleted outright;
        // an extracted memory goes through `forget_memory`.
        if let Some(id) = &m.id
            && self.rest.delete_document(id).is_ok()
        {
            return Ok(());
        }
        let (m, scope) = (m.clone(), scope.clone());
        self.run(move |c| {
            c.call(&["forget_memory", "memory"], |props| {
                let mut a = serde_json::Map::new();
                match &m.id {
                    Some(id) if props.iter().any(|p| p == "ids") => {
                        a.insert("ids".into(), json!([id]));
                    }
                    _ => put(&mut a, props, &["content", "thingToForget", "memory", "text"], json!(m.text)),
                }
                if !props.iter().any(|p| p == "ids") {
                    put(&mut a, props, &["action"], json!("forget"));
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

/// A text part that only carries the server's `conversation_id`.
fn conversation_part(text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(text.trim()).ok()?;
    let o = v.as_object().filter(|o| o.len() == 1)?;
    o.get("conversation_id")?.as_str().map(str::to_string)
}

fn conversation_of(result: &Value) -> Option<String> {
    result
        .get("content")?
        .as_array()?
        .iter()
        .filter_map(|c| c.get("text")?.as_str())
        .find_map(conversation_part)
}

/// Document hits from `/v3/search`: the matching chunks (or the title).
fn documents_of(reply: &Value) -> Vec<Memory> {
    let Some(results) = reply.get("results").and_then(Value::as_array) else { return Vec::new() };
    results
        .iter()
        .filter_map(|r| {
            let chunks: Vec<&str> = r
                .get("chunks")
                .and_then(Value::as_array)
                .map(|cs| {
                    cs.iter()
                        .filter(|c| c.get("isRelevant").and_then(Value::as_bool) != Some(false))
                        .filter_map(|c| c.get("content")?.as_str())
                        .collect()
                })
                .unwrap_or_default();
            let text = if chunks.is_empty() {
                r.get("title").and_then(Value::as_str)?.to_string()
            } else {
                chunks.join(" … ")
            };
            Some(Memory {
                id: r.get("documentId").and_then(Value::as_str).map(str::to_string),
                text: text.trim().to_string(),
                score: r.get("score").and_then(Value::as_f64).map(|s| s as f32),
                at: r.get("createdAt").and_then(Value::as_str).map(str::to_string),
            })
        })
        .filter(|m| !m.text.is_empty())
        .collect()
}

/// [`documents_of`] without the noise: hits under [`MIN_DOC_SCORE`] dropped.
fn relevant_documents(reply: &Value) -> Vec<Memory> {
    documents_of(reply).into_iter().filter(|m| m.score.is_none_or(|s| s >= MIN_DOC_SCORE)).collect()
}

/// Document hits below this hybrid score are noise for recall: unrelated
/// notes land around 0.5 for a plain question, real matches above 0.65.
const MIN_DOC_SCORE: f32 = 0.6;

/// All text parts of a tool result, joined (without the conversation id).
fn text_of(result: &Value) -> Option<String> {
    let parts: Vec<&str> = result
        .get("content")?
        .as_array()?
        .iter()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|c| c.get("text")?.as_str())
        .filter(|t| conversation_part(t).is_none())
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
        // An answer with a known (possibly empty) list is the whole truth;
        // its text part is only a human summary ("No matching memories").
        if !out.is_empty() || has_list(&v, 0) {
            return out;
        }
    }
    text_of(result).map(|t| memories_in_text(&t)).unwrap_or_default()
}

const TEXT_KEYS: &[&str] = &["memory", "content", "text", "summary", "chunk", "fact", "title"];
const LIST_KEYS: &[&str] = &[
    "results",
    "memories",
    "memoryEntries",
    "documents",
    "items",
    "data",
    "static",
    "dynamic",
    "profile",
    "chunks",
    "facts",
];

fn has_list(v: &Value, depth: usize) -> bool {
    depth < 3
        && v.as_object().is_some_and(|o| {
            LIST_KEYS.iter().any(|k| match o.get(*k) {
                Some(Value::Array(_)) => true,
                Some(child @ Value::Object(_)) => has_list(child, depth + 1),
                _ => false,
            })
        })
}

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
                                at: ["createdAt", "updatedAt"]
                                    .iter()
                                    .find_map(|k| o.get(*k)?.as_str().map(str::to_string)),
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
    if ["no memories", "no matching", "no results", "nothing found", "no active memories", "no profile facts"]
        .iter()
        .any(|p| lower.contains(p))
    {
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
    fn parses_real_supermemory_answers() {
        let empty = json!({
            "content": [
                {"type": "text", "text": "No matching memories found."},
                {"type": "text", "text": "{\"conversation_id\":\"01a1\"}"}
            ],
            "structuredContent": {"query": "q", "results": [], "total": 0}
        });
        assert!(memories_of(&empty).is_empty());
        assert_eq!(conversation_of(&empty).as_deref(), Some("01a1"));
        let profile = json!({
            "content": [{"type": "text", "text": "No profile facts are available for this space yet."}],
            "structuredContent": {"profile": {"static": [], "dynamic": []}}
        });
        assert!(memories_of(&profile).is_empty());
        let listed = json!({"structuredContent": {"memoryEntries": [{"id": "a", "memory": "Uses just"}]}});
        assert_eq!(memories_of(&listed)[0].text, "Uses just");
        // Text only, with the trailer.
        let text_only = json!({"content": [
            {"type": "text", "text": "No active memories stored yet."},
            {"type": "text", "text": "{\"conversation_id\":\"x\"}"}
        ]});
        assert!(memories_of(&text_only).is_empty());
    }

    #[test]
    fn parses_document_search() {
        let r = json!({"results": [
            {"documentId": "d1", "score": 0.85, "title": "t", "chunks": [
                {"content": "build with just build-1", "isRelevant": true},
                {"content": "noise", "isRelevant": false}
            ]},
            {"documentId": "d2", "score": 0.7, "title": "Only a title", "chunks": []},
            {"documentId": "d3", "score": 0.52, "title": "Unrelated", "chunks": []}
        ], "total": 2});
        let m = relevant_documents(&r);
        assert_eq!(m.len(), 2);
        assert_eq!(documents_of(&r).len(), 3);
        assert_eq!(m[0].text, "build with just build-1");
        assert_eq!(m[0].id.as_deref(), Some("d1"));
        assert_eq!(m[1].text, "Only a title");
        assert!(documents_of(&json!({"results": []})).is_empty());
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

/// Against a signed-in account, with real agents (costs a few turns):
/// `DOVE_DATA=<home with mcp_auth.json> cargo test -p dove supermemory::e2e -- --ignored --nocapture --test-threads 1`
#[cfg(test)]
mod e2e {
    use std::time::{Duration, Instant};

    use dove_core::proto::{AgentEvent, HarnessKind};
    use dove_core::{Command, NewTask, RunSettings};

    use super::*;

    fn real() -> Arc<dyn memory::Backend> {
        refresh(&cfg::data_root());
        memory::backend().expect("sign in to Supermemory in DOVE_DATA first")
    }

    fn nonce() -> String {
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
        format!("{}", ms % 1_000_000_000)
    }

    /// Run one turn; returns (the prompt as the agent got it, the answer).
    fn turn(live: &dove_core::LiveSession) -> (String, String) {
        let (mut sent, mut answer) = (String::new(), String::new());
        let until = Instant::now() + Duration::from_secs(300);
        while Instant::now() < until {
            match live.events.recv_timeout(Duration::from_secs(5)) {
                Ok(AgentEvent::UserMessage { text }) if sent.is_empty() => sent = text.to_string(),
                Ok(AgentEvent::TextDelta { text }) => answer.push_str(&text),
                Ok(AgentEvent::Error { message }) => println!("agent error: {message}"),
                Ok(AgentEvent::TurnEnded { .. }) | Ok(AgentEvent::Exited { .. }) => break,
                _ => {}
            }
        }
        (sent, answer)
    }

    fn wait_for(what: &str, secs: u64, mut f: impl FnMut() -> bool) -> bool {
        let until = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < until {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_secs(5));
        }
        println!("timed out waiting for {what}");
        false
    }

    #[test]
    #[ignore]
    fn rest_probe() {
        let def = server(&cfg::load()).unwrap();
        let rest = Rest { def, http: reqwest::Client::new() };
        let path = std::env::var("PROBE_PATH").unwrap_or_else(|_| "/v3/documents/list".into());
        let body: Value = serde_json::from_str(&std::env::var("PROBE_BODY").unwrap_or_else(|_| "{}".into())).unwrap();
        let r = rest.post(&path, body);
        println!("{}", r.map(|v| serde_json::to_string_pretty(&v).unwrap()).unwrap_or_else(|e| e).chars().take(4000).collect::<String>());
    }

    #[test]
    #[ignore]
    fn raw_probe() {
        let def = server(&cfg::load()).unwrap();
        let mut s = upstream::make(&def.id, &def.transport)().unwrap();
        s.initialize().unwrap();
        for t in s.list_tools().unwrap() {
            let name = t["name"].as_str().unwrap_or_default().to_string();
            if ["search_memory", "get_profile", "list_memories", "list_documents"].contains(&name.as_str()) {
                println!("TOOL {name} {}", t.pointer("/inputSchema/properties").map(|p| p.to_string()).unwrap_or_default());
            }
        }
        let n = nonce();
        let mut call = |name: &str, args: Value| {
            let r = s.request("tools/call", json!({"name": name, "arguments": args}));
            println!("== {name} {args}
{}", serde_json::to_string_pretty(&r).unwrap_or_default());
        };
        if let Ok(tag) = std::env::var("PROBE_TAG") {
            let q = std::env::var("PROBE_Q").unwrap_or_default();
            call("search_memory", json!({"query": q, "namespace": tag}));
            call("list_memories", json!({"namespace": tag}));
            call("list_documents", json!({"namespace": tag}));
            return;
        }
        call("list_spaces", json!({}));
        let tag = format!("repo_dove_probe__{n}");
        call("add_memory", json!({"content": format!("Dove probe {n}: deploys go through the staging-{n} pipeline"), "namespace": tag}));
        call("add_memory", json!({"content": format!("Dove probe default {n}: prefers staging-{n} reviews")}));
        std::thread::sleep(Duration::from_secs(45));
        call("search_memory", json!({"query": format!("staging-{n} pipeline"), "namespace": tag}));
        call("search_memory", json!({"query": format!("staging-{n} pipeline"), "containerTag": tag}));
        call("search_memory", json!({"query": format!("staging-{n}")}));
        call("list_memories", json!({"namespace": tag}));
        call("list_documents", json!({"namespace": tag}));
        call("get_profile", json!({"namespace": tag}));
    }

    #[test]
    #[ignore]
    fn tools_and_round_trip() {
        let def = server(&cfg::load()).expect("Supermemory server in integrations.json");
        let mut s = upstream::make(&def.id, &def.transport)().unwrap();
        s.initialize().unwrap();
        for t in s.list_tools().unwrap() {
            let props = t.pointer("/inputSchema/properties").map(|p| p.to_string()).unwrap_or_default();
            println!("TOOL {} {props}", t["name"]);
        }
        let b = real();
        println!("WHO {:?}", b.who_am_i());
        let n = nonce();
        let scope = Scope::Project(format!("repo_dove_e2e__{}", memory::sha16(&n)));
        println!("PROFILE {:?}", b.profile(&scope));
        b.add(&Doc {
            content: format!("Dove e2e round trip {n}: the release checklist lives in RELEASING.md"),
            scope: scope.clone(),
            metadata: [("source".to_string(), "dove".to_string())].into(),
            custom_id: None,
            at: 0,
        })
        .unwrap();
        let found = wait_for("the memory to be searchable", 180, || {
            let r = b.search(&format!("release checklist {n}"), &scope, 5);
            println!("SEARCH {r:?}");
            r.is_ok_and(|l| l.iter().any(|m| m.text.contains(&n)))
        });
        println!("LIST {:?}", b.list(&scope, 1, 10));
        assert!(found);
    }

    /// The recorder's own calls (recall on send, observe each event) with
    /// simulated agent replies, against the real account.
    #[test]
    #[ignore]
    fn pipeline_against_account() {
        use dove_core::proto::{ProjectId, TaskId};
        let b = real();
        let n = nonce();
        let base = std::path::PathBuf::from(format!("C:/dvw/mem-e2e/pipe-{n}"));
        let repo = base.join(format!("pipetest{n}"));
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(&repo).status().unwrap();
        git(&["init", "-q"]);
        git(&["remote", "add", "origin", &format!("https://github.com/Dove-E2E/PipeTest{n}.git")]);
        let store = Arc::new(dove_core::store::Store::open(&base.join("dove")).unwrap());
        let project = ProjectId::new();
        let tag = memory::project_tag(&repo);
        assert_eq!(tag, memory::generated_tag(&format!("PipeTest{n}"), &format!("github.com/dove-e2e/pipetest{n}")));
        println!("TAG {tag}");
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let scope = Scope::Project(tag.clone());

        // Claude's session: first prompt (nothing to recall yet), then its turn.
        let claude = memory::Session::new(store.clone(), TaskId::new(), project, HarnessKind::ClaudeCode, repo.clone());
        let fake = format!("sk-e2eFAKE{n}abcdefghijkl");
        let prompt = format!("Remember: the build command here is `just build-{n}`, never npm. Key {fake} is fake.");
        let sent = rt.block_on(memory::before_send(&claude, prompt.clone()));
        println!("CLAUDE GOT {sent:?}");
        for ev in [
            AgentEvent::UserMessage { text: sent.clone().into() },
            AgentEvent::TextDelta { text: format!("Noted: build with `just build-{n}`.").into() },
            AgentEvent::ToolDiff { call_id: "1".into(), path: repo.join(".env").display().to_string(), old_text: None, new_text: "X=1".into() },
            AgentEvent::ToolDiff { call_id: "2".into(), path: repo.join("src/main.rs").display().to_string(), old_text: None, new_text: "fn main(){}".into() },
            AgentEvent::TurnEnded { usage: None, is_error: false },
        ] {
            claude.lock().observe(&ev);
        }
        let mut stored = Vec::new();
        let saved = wait_for("the captured turn", 180, || {
            stored = b.search(&format!("build command just build-{n}"), &scope, 5).unwrap_or_default();
            println!("SEARCH {stored:?}");
            stored.iter().any(|m| m.text.contains(&format!("build-{n}")))
        });
        assert!(saved, "capture never became searchable");
        let hit = stored.iter().find(|m| m.text.contains(&format!("build-{n}"))).unwrap();
        assert!(!hit.text.contains(&fake) && hit.text.contains("[REDACTED]"), "not redacted: {}", hit.text);
        assert!(memory::Outbox::at(store.root()).is_empty());

        // Codex's session, same repo: recall brings the convention in.
        let codex = memory::Session::new(store.clone(), TaskId::new(), project, HarnessKind::Codex, repo.clone());
        let started = Instant::now();
        let asked = rt.block_on(memory::before_send(&codex, "What is this project's build command?".into()));
        println!("CODEX GOT ({:?}) {asked}", started.elapsed());
        let recalled = memory::recalled(&asked).unwrap_or_default();
        assert!(recalled.iter().any(|m| m.contains(&format!("build-{n}"))), "recall missed it: {recalled:?}");
        assert!(asked.ends_with("What is this project's build command?"));
        assert_eq!(dove_core::strip_persona(&asked), "What is this project's build command?");
        // The same memory is not recalled twice in one session.
        let again = rt.block_on(memory::before_send(&codex, "And the build command again?".into()));
        assert!(memory::recalled(&again).unwrap_or_default().iter().all(|m| !m.contains(&format!("build-{n}"))));

        // A correction in Claude's session becomes feedback.
        rt.block_on(memory::before_send(&claude, format!("No, use `just build-{n} --release` instead.")));
        let learned = wait_for("the correction", 180, || {
            let r = b.search(&format!("user corrected the agent build-{n} release"), &scope, 5).unwrap_or_default();
            println!("FEEDBACK {r:?}");
            r.iter().any(|m| m.text.contains("corrected") && m.text.contains(&format!("build-{n}")))
        });
        assert!(learned);

        // Offline: a save fails into the outbox, then drains once back.
        let def = server(&cfg::load()).unwrap();
        let offline = McpServer {
            transport: Transport::Http { url: "http://127.0.0.1:9/mcp".into(), headers: Default::default(), oauth: false },
            ..def
        };
        memory::set_backend(Some(Arc::new(Supermemory::start(offline))));
        let outbox = memory::Outbox::at(store.root());
        memory::save_or_queue(
            store.root(),
            Doc { content: format!("Dove e2e offline {n}"), scope: scope.clone(), metadata: Default::default(), custom_id: None, at: 0 },
        );
        assert!(wait_for("the outbox to fill", 60, || outbox.len() == 1));
        memory::set_backend(Some(b.clone()));
        memory::flush_outbox(store.root());
        assert!(wait_for("the outbox to drain", 60, || outbox.is_empty()));
        assert!(wait_for("the drained save", 180, || {
            b.search(&format!("Dove e2e offline {n}"), &scope, 5).is_ok_and(|l| l.iter().any(|m| m.text.contains(&n)))
        }));
        println!("PIPELINE OK");
    }

    #[test]
    #[ignore]
    fn claude_teaches_codex() {
        let b = real();
        let n = nonce();
        let base = std::path::PathBuf::from(format!("C:/dvw/mem-e2e/run-{n}"));
        let repo = base.join(format!("memtest{n}"));
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(&repo).status().unwrap();
        git(&["init", "-q"]);
        git(&["remote", "add", "origin", &format!("git@github.com:dove-e2e/memtest{n}.git")]);
        std::fs::write(repo.join("README.md"), "# memtest\n").unwrap();
        let engine = dove_core::Engine::new(base.join("dove")).unwrap();
        engine.rescan_harnesses();
        let project = engine.store.add_project(&repo).unwrap();
        let tag = memory::project_tag(&repo);
        println!("PROJECT TAG {tag}");
        assert!(memory::recalls(&engine.store, project.id));

        // 1. Claude learns a convention (with a fake key that must not leave).
        let fake = format!("sk-e2eFAKE{n}abcdefghijkl");
        let (_t1, live) = engine
            .start_task(
                &project,
                NewTask {
                    title: "teach".into(),
                    prompt: format!(
                        "Remember this project convention: the build command here is `just build-{n}`, never npm. \
                         (Ignore this test key: {fake}.) Reply with just OK."
                    ),
                    harness: HarnessKind::ClaudeCode,
                    run: RunSettings::default(),
                    join: None,
                    persona: None,
                },
            )
            .unwrap();
        let (sent, answer) = turn(&live);
        println!("CLAUDE GOT: {sent}\nCLAUDE SAID: {answer}");
        live.send(Command::Kill);
        let scope = Scope::Project(tag.clone());
        let saved = wait_for("the captured turn in memory", 240, || {
            let r = b.search(&format!("build command just build-{n}"), &scope, 5);
            println!("SEARCH {r:?}");
            r.is_ok_and(|l| l.iter().any(|m| m.text.contains(&format!("build-{n}"))))
        });
        let listed = b.list(&scope, 1, 20).unwrap_or_default();
        println!("STORED {listed:?}");
        let outbox = memory::Outbox::at(engine.store.root());
        println!("OUTBOX {}", outbox.len());
        assert!(saved, "capture never reached Supermemory");
        assert!(listed.iter().all(|m| !m.text.contains(&fake)), "the fake key was not redacted");
        assert!(outbox.is_empty(), "capture queued instead of sending");

        // 2. Codex, same project, new session: recall must bring it in.
        let (_t2, live2) = engine
            .start_task(
                &project,
                NewTask {
                    title: "ask".into(),
                    prompt: "What is this project's build command? Answer with the command only, no tools.".into(),
                    harness: HarnessKind::Codex,
                    run: RunSettings::default(),
                    join: None,
                    persona: None,
                },
            )
            .unwrap();
        let (sent2, answer2) = turn(&live2);
        println!("CODEX GOT: {sent2}\nCODEX SAID: {answer2}");
        live2.send(Command::Kill);
        let recalled = memory::recalled(&sent2).unwrap_or_default();
        println!("RECALLED {recalled:?}");
        assert!(recalled.iter().any(|m| m.contains(&format!("build-{n}"))), "recall missed the convention");
        assert!(answer2.contains(&format!("build-{n}")), "Codex did not use the recalled convention");

        // 3. Offline: a save fails into the outbox, then drains once back.
        let def = server(&cfg::load()).unwrap();
        let offline = McpServer {
            transport: Transport::Http {
                url: "http://127.0.0.1:9/mcp".into(),
                headers: Default::default(),
                oauth: false,
            },
            ..def
        };
        memory::set_backend(Some(Arc::new(Supermemory::start(offline))));
        let doc = Doc {
            content: format!("Dove e2e offline {n}"),
            scope: scope.clone(),
            metadata: Default::default(),
            custom_id: None,
            at: 0,
        };
        memory::save_or_queue(engine.store.root(), doc);
        assert!(wait_for("the outbox to fill", 60, || outbox.len() == 1));
        memory::set_backend(Some(b.clone()));
        memory::flush_outbox(engine.store.root());
        assert!(wait_for("the outbox to drain", 60, || outbox.is_empty()));
        println!("E2E OK");
    }
}
