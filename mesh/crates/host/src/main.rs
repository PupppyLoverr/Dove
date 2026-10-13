//! dove-mesh-host — the desktop half of Dove remote control.
//!
//! The Dove desktop app spawns this sidecar and talks to it over stdin/stdout
//! JSON lines. It signs in with Woozlit (loopback callback, same flow as the
//! Cosmos desktop), then publishes the desktop's projects and sessions to the
//! Cosmos edge exactly the way a Cosmos host does: rows in the account's
//! registry room (`reg1/{org}/{user}`) and one Loro session doc per chat in its
//! chat2 room. The Dove phone apps read those rooms and queue `run` / `steer` /
//! `interrupt` commands into the session doc; the host forwards each command to
//! the desktop, which runs it on its own harness and reports back.
//!
//! Local sessions are only published while the user is signed in and has
//! turned remote control on; nothing is uploaded otherwise.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use futures::future::BoxFuture;
use loro::{LoroDoc, VersionVector};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use zeron_client::auth as woozlit;
use zeron_doc::{
    MessagePart, MessageRole, MessageStatus, RegistryDoc, SegmentWriter, SessionCommandPayload, SessionCommandStatus,
    SessionDoc, SessionMessageEntry,
};
use zeron_sync::chat_client::{ChatClient, ChatDocSink, ChatEvent, CheckpointFetcher, RowImportOutcome};
use zeron_sync::registry::RegistryClient;
use zeron_sync::{SyncError, UrlProvider};

const DEFAULT_EDGE: &str = zeron_client::auth::PRODUCTION_EDGE_URL;
const PRESENCE_EVERY: Duration = Duration::from_secs(20);

// ── wire (desktop ⇄ host) ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Op {
    Status,
    SignIn,
    SignOut,
    Space {
        id: String,
        path: String,
        name: Option<String>,
    },
    Chat {
        id: String,
        space_id: Option<String>,
        title: Option<String>,
        cwd: Option<String>,
        branch: Option<String>,
        preview: Option<String>,
        #[serde(default)]
        archived: bool,
    },
    Message {
        chat: String,
        id: String,
        role: String,
        text: String,
        #[serde(default)]
        streaming: bool,
    },
    CommandDone {
        chat: String,
        id: String,
        ok: bool,
        resolution: Option<String>,
    },
    Shutdown,
}

fn emit(value: Value) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

fn emit_error(context: &str, err: impl std::fmt::Display) {
    emit(json!({ "ev": "error", "context": context, "message": err.to_string() }));
}

// ── persisted account session ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Account {
    user_id: String,
    email: Option<String>,
    access_token: String,
    refresh_token: String,
}

struct Paths {
    account: PathBuf,
    device: PathBuf,
}

impl Paths {
    fn new(dir: &Path) -> Self {
        Self { account: dir.join("account.json"), device: dir.join("device.json") }
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(tmp, path)?;
    Ok(())
}

fn device_id(paths: &Paths) -> Result<String> {
    if let Ok(raw) = std::fs::read_to_string(&paths.device)
        && let Ok(v) = serde_json::from_str::<Value>(&raw)
        && let Some(id) = v.get("id").and_then(Value::as_str)
    {
        return Ok(id.to_owned());
    }
    let id = format!("dove-desktop-{}", uuid::Uuid::new_v4());
    write_private(&paths.device, json!({ "id": id }).to_string().as_bytes())?;
    Ok(id)
}

// ── bearer (Woozlit / Firebase id token, refreshed on expiry) ───────────────

#[derive(Clone)]
struct Bearer {
    account: Arc<tokio::sync::Mutex<Option<Account>>>,
    path: PathBuf,
}

impl Bearer {
    async fn get(&self) -> Result<String, SyncError> {
        let mut guard = self.account.lock().await;
        let Some(account) = guard.as_mut() else {
            return Err(SyncError::JoinRefused("signed out".into()));
        };
        let now = chrono::Utc::now().timestamp() + 60;
        if woozlit::jwt_expired(&account.access_token, now) && !account.refresh_token.is_empty() {
            let tokens = woozlit::woozlit_refresh(&account.refresh_token)
                .await
                .map_err(|e| SyncError::JoinRefused(format!("refresh failed: {e}")))?;
            account.access_token = tokens.access_token;
            if !tokens.refresh_token.is_empty() {
                account.refresh_token = tokens.refresh_token;
            }
            let _ = write_private(&self.path, &serde_json::to_vec(account).unwrap_or_default());
        }
        Ok(account.access_token.clone())
    }
}

fn ws_base(edge: &str) -> String {
    let edge = edge.trim_end_matches('/');
    if let Some(rest) = edge.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = edge.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        edge.to_owned()
    }
}

fn enc(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

struct RoomUrl {
    bearer: Bearer,
    edge: String,
    /// `registry/{org}` or `chat2/{chat}`.
    room: String,
    device: String,
}

impl UrlProvider for RoomUrl {
    fn url(&self) -> BoxFuture<'static, Result<String, SyncError>> {
        let bearer = self.bearer.clone();
        let (base, room, device) = (ws_base(&self.edge), self.room.clone(), enc(&self.device));
        Box::pin(async move {
            let token = bearer.get().await?;
            Ok(format!("{base}/{room}/ws?token={}&device={device}", enc(&token)))
        })
    }
}

struct Checkpoint {
    bearer: Bearer,
    url: String,
    http: reqwest::Client,
}

impl CheckpointFetcher for Checkpoint {
    fn fetch(&self) -> BoxFuture<'static, Result<Vec<u8>, SyncError>> {
        let (bearer, url, http) = (self.bearer.clone(), self.url.clone(), self.http.clone());
        Box::pin(async move {
            let token = bearer.get().await?;
            let response = http
                .get(&url)
                .bearer_auth(token)
                .timeout(Duration::from_secs(120))
                .send()
                .await
                .map_err(|e| SyncError::WebSocket(e.to_string()))?;
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(Vec::new());
            }
            if !response.status().is_success() {
                return Err(SyncError::WebSocket(format!("checkpoint {}", response.status())));
            }
            let bytes = response.bytes().await.map_err(|e| SyncError::WebSocket(e.to_string()))?;
            Ok(bytes.to_vec())
        })
    }
}

// ── chat room: one Loro session doc per Dove session ────────────────────────

struct Sink {
    doc: Arc<SessionDoc>,
    cursor: AtomicU64,
    applied: mpsc::UnboundedSender<String>,
    chat: String,
}

impl Sink {
    fn import(&self, bytes: &[u8], cursor: u64) -> RowImportOutcome {
        match self.doc.doc().import(bytes) {
            Ok(status) if status.pending.is_some() => RowImportOutcome::PendingDependencies,
            Ok(_) | Err(_) => {
                self.cursor.fetch_max(cursor, Ordering::AcqRel);
                let _ = self.applied.send(self.chat.clone());
                RowImportOutcome::Applied
            }
        }
    }
}

impl ChatDocSink for Sink {
    fn apply_row(&self, bytes: &[u8], cursor: u64) -> RowImportOutcome {
        self.import(bytes, cursor)
    }
    fn apply_checkpoint(&self, bytes: &[u8], cursor: u64) -> Result<(), String> {
        if bytes.is_empty() {
            return Ok(());
        }
        match self.import(bytes, cursor) {
            RowImportOutcome::Applied => Ok(()),
            RowImportOutcome::PendingDependencies => Err("checkpoint missing dependencies".into()),
        }
    }
    fn contains_frontier(&self, frontier: &[u8]) -> bool {
        match VersionVector::decode(frontier) {
            Ok(vv) if !vv.is_empty() => self.doc.doc().oplog_vv().includes_vv(&vv),
            _ => false,
        }
    }
    fn advance_cursor(&self, cursor: u64) {
        self.cursor.fetch_max(cursor, Ordering::AcqRel);
    }
}

struct Room {
    doc: Arc<SessionDoc>,
    client: Arc<Mutex<Option<ChatClient>>>,
    _sub: loro::Subscription,
    /// Streaming assistant entries: message id → (entry index, parts written).
    writers: HashMap<String, (usize, Vec<MessagePart>)>,
    forwarded: HashSet<String>,
}

// ── host ────────────────────────────────────────────────────────────────────

struct Host {
    edge: String,
    paths: Paths,
    device_id: String,
    device_name: String,
    bearer: Bearer,
    http: reqwest::Client,
    registry: Arc<Mutex<RegistryDoc>>,
    registry_client: Option<RegistryClient>,
    rooms: HashMap<String, Room>,
    applied_tx: mpsc::UnboundedSender<String>,
    pending_sign_in: Option<String>,
    sign_in_tx: mpsc::UnboundedSender<(String, String)>,
}

impl Host {
    async fn signed_in(&self) -> Option<Account> {
        self.bearer.account.lock().await.clone()
    }

    async fn status(&self) {
        let account = self.signed_in().await;
        emit(json!({
            "ev": "status",
            "signedIn": account.is_some(),
            "email": account.as_ref().and_then(|a| a.email.clone()),
            "userId": account.as_ref().map(|a| a.user_id.clone()),
            "deviceId": self.device_id,
            "edge": self.edge,
            "connected": self.registry_client.is_some(),
            "chats": self.rooms.len(),
        }));
    }

    async fn sign_in(&mut self) -> Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let state = uuid::Uuid::new_v4().to_string();
        self.pending_sign_in = Some(state.clone());
        let callback = format!("http://127.0.0.1:{port}/callback?state={state}");
        let url = format!("{}/login?desktop=1&callback={}", woozlit::WOOZLIT_ORIGIN, enc(&callback));
        let tx = self.sign_in_tx.clone();
        tokio::spawn(async move {
            // One-shot loopback: the browser lands here with ?state=…&token=….
            let deadline = tokio::time::sleep(Duration::from_secs(600));
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    _ = &mut deadline => return,
                    accepted = listener.accept() => {
                        let Ok((mut socket, _)) = accepted else { continue };
                        let mut buf = vec![0u8; 16 * 1024];
                        let n = socket.read(&mut buf).await.unwrap_or(0);
                        let head = String::from_utf8_lossy(&buf[..n]);
                        let target = head.split_whitespace().nth(1).unwrap_or("");
                        let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
                        let params: HashMap<String, String> = query
                            .split('&')
                            .filter_map(|kv| kv.split_once('='))
                            .map(|(k, v)| (k.to_owned(), percent_decode(v)))
                            .collect();
                        let (state, token) = (
                            params.get("state").cloned().unwrap_or_default(),
                            params.get("token").or_else(|| params.get("code")).cloned().unwrap_or_default(),
                        );
                        let body = if token.is_empty() {
                            "Sign-in did not complete. Return to Dove and try again."
                        } else {
                            "Signed in to Dove. You can close this tab."
                        };
                        let page = format!(
                            "<!doctype html><meta charset=utf-8><title>Dove</title><body style=\"font:15px -apple-system,system-ui;display:grid;place-items:center;height:90vh;color:#111\"><p>{body}</p>"
                        );
                        let response = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
                            page.len()
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        if !token.is_empty() {
                            let _ = tx.send((state, token));
                            return;
                        }
                    }
                }
            }
        });
        emit(json!({ "ev": "sign_in_url", "url": url }));
        Ok(())
    }

    async fn complete_sign_in(&mut self, state: String, token: String) -> Result<()> {
        if self.pending_sign_in.take().as_deref() != Some(state.as_str()) {
            return Err(anyhow!("sign-in state mismatch — start again from Dove"));
        }
        let session = woozlit::woozlit_exchange(&token).await.context("Woozlit exchange")?;
        let account = Account {
            user_id: session.user_id,
            email: session.email,
            access_token: session.tokens.access_token,
            refresh_token: session.tokens.refresh_token,
        };
        write_private(&self.paths.account, &serde_json::to_vec(&account)?)?;
        *self.bearer.account.lock().await = Some(account);
        self.connect_registry().await?;
        self.status().await;
        Ok(())
    }

    async fn sign_out(&mut self) {
        if let Some(client) = self.registry_client.take() {
            client.shutdown().await;
        }
        for (_, room) in self.rooms.drain() {
            if let Some(client) = room.client.lock().unwrap().take() {
                tokio::spawn(client.shutdown());
            }
        }
        *self.bearer.account.lock().await = None;
        let _ = std::fs::remove_file(&self.paths.account);
        self.status().await;
    }

    async fn connect_registry(&mut self) -> Result<()> {
        let Some(account) = self.signed_in().await else {
            return Ok(());
        };
        if self.registry_client.is_some() {
            return Ok(());
        }
        let org = account.user_id.clone();
        let provider = Arc::new(RoomUrl {
            bearer: self.bearer.clone(),
            edge: self.edge.clone(),
            room: format!("registry/{}", enc(&org)),
            device: self.device_id.clone(),
        });
        let client = RegistryClient::connect_via(provider, self.registry.clone(), &self.device_id)
            .await
            .map_err(|e| anyhow!("registry: {e}"))?;
        {
            let mut doc = self.registry.lock().unwrap();
            let device: zeron_proto::Device = serde_json::from_value(json!({
                "id": self.device_id,
                "name": self.device_name,
                "platform": std::env::consts::OS,
                "lastSeenAt": chrono::Utc::now(),
                "createdAt": chrono::Utc::now(),
                "version": env!("CARGO_PKG_VERSION"),
                "capabilities": ["dove", "run", "interrupt"],
            }))?;
            doc.upsert_device(&device)?;
        }
        client.nudge();
        client.set_presence(chrono::Utc::now().timestamp_millis());
        self.registry_client = Some(client);
        emit(json!({ "ev": "connected", "deviceId": self.device_id }));
        Ok(())
    }

    fn mutate_registry(&self, f: impl FnOnce(&mut RegistryDoc) -> Result<()>) -> Result<()> {
        f(&mut self.registry.lock().unwrap())?;
        if let Some(client) = &self.registry_client {
            client.nudge();
        }
        Ok(())
    }

    async fn upsert_space(&mut self, id: String, path: String, name: Option<String>) -> Result<()> {
        if self.registry_client.is_none() {
            return Ok(());
        }
        let git = Path::new(&path).join(".git").exists();
        let space: zeron_proto::Space = serde_json::from_value(json!({
            "id": id, "deviceId": self.device_id, "path": path, "name": name,
            "gitDetected": git, "createdAt": chrono::Utc::now(),
        }))?;
        self.mutate_registry(|doc| Ok(doc.upsert_space(&space)?))
    }

    #[allow(clippy::too_many_arguments)]
    async fn upsert_chat(
        &mut self,
        id: String,
        space_id: Option<String>,
        title: Option<String>,
        cwd: Option<String>,
        branch: Option<String>,
        preview: Option<String>,
        archived: bool,
    ) -> Result<()> {
        if self.registry_client.is_none() {
            return Ok(());
        }
        let existing = self.registry.lock().unwrap().chat(&id)?;
        let created = existing.as_ref().map(|c| c.created_at).unwrap_or_else(chrono::Utc::now);
        let chat: zeron_proto::Chat = serde_json::from_value(json!({
            "id": id, "deviceId": self.device_id, "title": title, "archived": archived,
            "cwd": cwd, "branch": branch, "spaceId": space_id,
            "lastMessagePreview": preview, "lastMessageAt": chrono::Utc::now(),
            "createdAt": created, "config": null, "checkoutId": null,
        }))?;
        self.mutate_registry(|doc| Ok(doc.upsert_chat(&chat)?))?;
        self.ensure_room(&id).await?;
        Ok(())
    }

    async fn ensure_room(&mut self, chat: &str) -> Result<()> {
        if self.rooms.contains_key(chat) {
            return Ok(());
        }
        let doc = Arc::new(SessionDoc::from_doc(LoroDoc::new()));
        let client_slot: Arc<Mutex<Option<ChatClient>>> = Arc::new(Mutex::new(None));
        let pending: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
        let sub = {
            let slot = client_slot.clone();
            let pending = pending.clone();
            doc.doc().subscribe_local_update(Box::new(move |bytes: &Vec<u8>| {
                match &*slot.lock().unwrap() {
                    Some(client) => client.enqueue_update(bytes.clone()),
                    None => pending.lock().unwrap().push(bytes.clone()),
                }
                true
            }))
        };
        let sink = Arc::new(Sink {
            doc: doc.clone(),
            cursor: AtomicU64::new(0),
            applied: self.applied_tx.clone(),
            chat: chat.to_owned(),
        });
        let provider = Arc::new(RoomUrl {
            bearer: self.bearer.clone(),
            edge: self.edge.clone(),
            room: format!("chat2/{}", enc(chat)),
            device: self.device_id.clone(),
        });
        let fetcher = Arc::new(Checkpoint {
            bearer: self.bearer.clone(),
            url: format!("{}/chat2/{}/checkpoint", self.edge.trim_end_matches('/'), enc(chat)),
            http: self.http.clone(),
        });
        let client = ChatClient::connect_via(provider, sink, fetcher, &self.device_id, 0)
            .await
            .map_err(|e| anyhow!("chat2 {chat}: {e}"))?;
        // Join the room's history before writing so restarts never fork it.
        let mut events = client.events();
        let _ = tokio::time::timeout(Duration::from_secs(8), async {
            if client.caught_up() {
                return;
            }
            while let Ok(event) = events.recv().await {
                if matches!(event, ChatEvent::CaughtUp { .. }) {
                    return;
                }
            }
        })
        .await;
        for bytes in pending.lock().unwrap().drain(..) {
            client.enqueue_update(bytes);
        }
        *client_slot.lock().unwrap() = Some(client);
        if doc.chat_id().is_none() {
            let meta = doc.doc().get_map("meta");
            meta.insert("chatId", chat)?;
            meta.insert("schemaVersion", zeron_doc::SESSION_SCHEMA_VERSION as i64)?;
            doc.doc().commit();
        }
        let forwarded = doc
            .read_commands()
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.status != SessionCommandStatus::Pending)
            .map(|c| c.id)
            .collect();
        self.rooms
            .insert(chat.to_owned(), Room { doc, client: client_slot, _sub: sub, writers: HashMap::new(), forwarded });
        self.forward_commands(chat);
        Ok(())
    }

    async fn message(&mut self, chat: String, id: String, role: String, text: String, streaming: bool) -> Result<()> {
        if self.registry_client.is_none() {
            return Ok(());
        }
        self.ensure_room(&chat).await?;
        let device = self.device_id.clone();
        let room = self.rooms.get_mut(&chat).expect("room");
        let parts = vec![MessagePart::Text { id: format!("{id}:text"), text }];
        if role == "user" {
            if room.doc.find_entry(&id).is_none() {
                room.doc.push_message(&SessionMessageEntry {
                    id,
                    role: MessageRole::User,
                    parts,
                    created_at: chrono::Utc::now().timestamp_millis(),
                    device_id: device,
                    status: Some(MessageStatus::Complete),
                    continuation_of: None,
                    duration_ms: None,
                })?;
            }
            return Ok(());
        }
        if !streaming
            && !room.writers.contains_key(&id)
            && room.doc.find_entry(&id).is_some_and(|(_, written)| written == parts)
        {
            return Ok(());
        }
        let mut writer = match room.writers.remove(&id) {
            Some((index, written)) => SegmentWriter::resume(&room.doc, index, written),
            None => match room.doc.find_entry(&id) {
                Some((index, written)) => SegmentWriter::resume(&room.doc, index, written),
                None => SegmentWriter::begin(&room.doc, &id, &device, chrono::Utc::now().timestamp_millis())?,
            },
        };
        if streaming {
            writer.sync(&parts)?;
            room.writers.insert(id, writer.into_state());
        } else {
            writer.finish(&parts, MessageStatus::Complete)?;
        }
        Ok(())
    }

    fn forward_commands(&mut self, chat: &str) {
        let Some(room) = self.rooms.get_mut(chat) else { return };
        let now = chrono::Utc::now().timestamp_millis();
        for command in room.doc.read_commands().unwrap_or_default() {
            if command.status != SessionCommandStatus::Pending || room.forwarded.contains(&command.id) {
                continue;
            }
            room.forwarded.insert(command.id.clone());
            if command.effective_expiry() < now {
                let _ = room.doc.set_command_status(&command.id, SessionCommandStatus::Expired, None);
                continue;
            }
            let event = match &command.payload {
                SessionCommandPayload::Run { request, message_id } => json!({
                    "ev": "command", "chat": chat, "id": command.id, "kind": "run",
                    "prompt": request.prompt, "messageId": message_id, "model": request.model,
                }),
                SessionCommandPayload::Steer { prompt, message_id } => json!({
                    "ev": "command", "chat": chat, "id": command.id, "kind": "steer",
                    "prompt": prompt, "messageId": message_id,
                }),
                SessionCommandPayload::Interrupt {} => json!({
                    "ev": "command", "chat": chat, "id": command.id, "kind": "interrupt",
                }),
                SessionCommandPayload::RespondInput { .. } => {
                    let _ = room.doc.set_command_status(
                        &command.id,
                        SessionCommandStatus::Rejected,
                        Some("Answer this prompt on the desktop"),
                    );
                    continue;
                }
            };
            emit(event);
        }
    }

    fn command_done(&mut self, chat: &str, id: &str, ok: bool, resolution: Option<&str>) -> Result<()> {
        let Some(room) = self.rooms.get(chat) else { return Ok(()) };
        let status = if ok { SessionCommandStatus::Applied } else { SessionCommandStatus::Rejected };
        room.doc.set_command_status(id, status, resolution)?;
        Ok(())
    }
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() + 0 && i + 2 <= bytes.len() - 1 => {
                match u8::from_str_radix(&value[i + 1..i + 3], 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hostname() -> String {
    std::process::Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "Dove desktop".into())
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut data_dir = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".dove/mesh");
    let mut edge = std::env::var("DOVE_EDGE_URL").unwrap_or_else(|_| DEFAULT_EDGE.into());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => data_dir = args.next().map(PathBuf::from).context("--data-dir")?,
            "--edge" => edge = args.next().context("--edge")?,
            _ => {}
        }
    }
    let paths = Paths::new(&data_dir);
    let account: Option<Account> = std::fs::read(&paths.account).ok().and_then(|raw| serde_json::from_slice(&raw).ok());
    let device_id = device_id(&paths)?;
    let (applied_tx, mut applied_rx) = mpsc::unbounded_channel();
    let (sign_in_tx, mut sign_in_rx) = mpsc::unbounded_channel();
    let mut host = Host {
        edge,
        bearer: Bearer { account: Arc::new(tokio::sync::Mutex::new(account)), path: paths.account.clone() },
        paths,
        registry: Arc::new(Mutex::new(RegistryDoc::new(device_id.clone()))),
        device_id,
        device_name: hostname(),
        http: reqwest::Client::new(),
        registry_client: None,
        rooms: HashMap::new(),
        applied_tx,
        pending_sign_in: None,
        sign_in_tx,
    };
    if let Err(err) = host.connect_registry().await {
        emit_error("connect", err);
    }
    host.status().await;

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut presence = tokio::time::interval(PRESENCE_EVERY);
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { break };
                if line.trim().is_empty() { continue; }
                let op: Op = match serde_json::from_str(&line) {
                    Ok(op) => op,
                    Err(err) => { emit_error("parse", err); continue; }
                };
                let result = match op {
                    Op::Status => { host.status().await; Ok(()) }
                    Op::SignIn => host.sign_in().await,
                    Op::SignOut => { host.sign_out().await; Ok(()) }
                    Op::Space { id, path, name } => host.upsert_space(id, path, name).await,
                    Op::Chat { id, space_id, title, cwd, branch, preview, archived } =>
                        host.upsert_chat(id, space_id, title, cwd, branch, preview, archived).await,
                    Op::Message { chat, id, role, text, streaming } =>
                        host.message(chat, id, role, text, streaming).await,
                    Op::CommandDone { chat, id, ok, resolution } =>
                        host.command_done(&chat, &id, ok, resolution.as_deref()),
                    Op::Shutdown => break,
                };
                if let Err(err) = result { emit_error("op", err); }
            }
            Some(chat) = applied_rx.recv() => host.forward_commands(&chat),
            Some((state, token)) = sign_in_rx.recv() => {
                if let Err(err) = host.complete_sign_in(state, token).await {
                    emit_error("sign_in", format!("{err:#}"));
                }
            }
            _ = presence.tick() => {
                if let Some(client) = &host.registry_client {
                    client.set_presence(chrono::Utc::now().timestamp_millis());
                }
            }
        }
    }
    if let Some(client) = host.registry_client.take() {
        client.shutdown().await;
    }
    Ok(())
}
