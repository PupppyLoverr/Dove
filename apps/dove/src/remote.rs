//! Remote control: drive this desktop's sessions from the Dove phone apps.
//!
//! Spawns the `dove-mesh-host` sidecar (mesh/crates/host), which owns the
//! Woozlit account session and the Cosmos edge connections. Signed out or
//! switched off, nothing leaves the machine. Signed in and on, every project
//! and session is published to the account's registry room and each session's
//! transcript to its chat2 room; commands the phone queues come back here as
//! [`RemoteEvent::Command`] and run on the same session the desktop shows.

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dove_core::Engine;
use dove_core::proto::{AgentEvent, TaskId};
use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global};
use serde_json::{Value, json};

const ENABLED_KEY: &str = "remote_enabled";
pub(crate) const SYNC_OFFER_SEEN: &str = "sync_offer_seen";
pub(crate) const SYNC_CLAIM: &str = "sync_claim";
const REPUBLISH_EVERY: Duration = Duration::from_millis(600);
/// Sessions whose history is published on connect (newest first).
const HISTORY_SESSIONS: usize = 40;

#[derive(Debug, Clone, Default)]
pub struct RemoteStatus {
    pub running: bool,
    pub signed_in: bool,
    pub connected: bool,
    pub email: Option<String>,
    pub device_id: Option<String>,
    pub error: Option<String>,
    pub signing_in: bool,
}

#[derive(Debug, Clone)]
pub enum RemoteCommand {
    Run(String),
    Interrupt,
}

#[derive(Debug, Clone)]
pub enum RemoteEvent {
    Command {
        task: TaskId,
        id: String,
        command: RemoteCommand,
    },
    /// Chats already on the account, from the edge registry.
    AccountSessions {
        device: String,
        chats: Value,
    },
    /// Messages for one of those chats, from its room.
    AccountTranscript {
        task: TaskId,
        messages: Value,
    },
    Changed,
}

pub struct Remote {
    engine: Arc<Engine>,
    pub status: RemoteStatus,
    child: Option<Child>,
    stdin: Option<Arc<Mutex<ChildStdin>>>,
    last_publish: HashMap<TaskId, Instant>,
    pending: HashMap<TaskId, bool>,
    /// How far each session's transcript has been published.
    feeds: Feeds,
}

impl EventEmitter<RemoteEvent> for Remote {}

struct RemoteGlobal(Entity<Remote>);
impl Global for RemoteGlobal {}

/// The raw host error names every protocol command. The screen should not.
fn friendly_host_error(message: &str) -> String {
    if message.contains("unknown variant") || message.contains("list_sessions") {
        "Restart Keplorayx so the sync helper can read your account.".into()
    } else if message.chars().count() > 180 {
        message.chars().take(180).collect()
    } else {
        message.to_string()
    }
}

fn host_binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("DOVE_MESH_HOST").map(PathBuf::from).filter(|p| p.exists()) {
        return Some(p);
    }
    let name = if cfg!(windows) { "dove-mesh-host.exe" } else { "dove-mesh-host" };
    let mut candidates = Vec::new();
    // Dev checkout compiled from apps/dove. The mesh host is a separate
    // workspace, so it is not next to dove.exe when CARGO_TARGET_DIR points
    // outside the repo (the warm target dir).
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(repo) = manifest.parent().and_then(|p| p.parent()) {
        for profile in ["debug", "release"] {
            candidates.push(repo.join("mesh").join("target").join(profile).join(name));
        }
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.to_path_buf())) {
        // Same target directory (mesh built with the same CARGO_TARGET_DIR).
        candidates.push(dir.join(name));
        candidates.push(dir.join("../Resources").join(name));
        for ancestor in dir.ancestors().take(12) {
            for profile in ["debug", "release"] {
                candidates.push(ancestor.join("mesh").join("target").join(profile).join(name));
            }
        }
    }
    // The checkout's mesh/target copy can be older than the one built beside
    // dove.exe. The newest file is the one that understands list_sessions.
    candidates.into_iter().filter(|p| p.is_file()).max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
}

/// Whether this computer may upload `task`. Undecided (the offer is still
/// open) uploads nothing. "Start fresh" keeps the threads that already
/// lived here (`held`). A session owned by another device is never claimed.
pub(crate) fn may_publish_session(claim: Option<&str>, held: bool, owner: Option<&str>, mine: &str) -> bool {
    if held {
        return false;
    }
    if let Some(owner) = owner.map(str::trim).filter(|s| !s.is_empty())
        && (mine.is_empty() || owner != mine)
    {
        return false;
    }
    matches!(claim, Some("bring") | Some("fresh"))
}

impl Remote {
    pub fn init(engine: Arc<Engine>, cx: &mut App) -> Entity<Remote> {
        let enabled = engine.store.setting(ENABLED_KEY).as_deref() == Some("1");
        let e = cx.new(|_| Remote {
            engine,
            status: RemoteStatus::default(),
            child: None,
            stdin: None,
            last_publish: HashMap::new(),
            pending: HashMap::new(),
            feeds: Feeds::default(),
        });
        cx.set_global(RemoteGlobal(e.clone()));
        if enabled {
            e.update(cx, |r, cx| r.start(cx));
        }
        e
    }

    pub fn global(cx: &App) -> Option<Entity<Remote>> {
        cx.try_global::<RemoteGlobal>().map(|g| g.0.clone())
    }

    pub fn enabled(&self) -> bool {
        self.engine.store.setting(ENABLED_KEY).as_deref() == Some("1")
    }

    pub fn set_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        let _ = self.engine.store.set_setting(ENABLED_KEY, if on { "1" } else { "0" });
        if on {
            self.start(cx);
        } else {
            self.stop(cx);
        }
    }

    fn send(&self, value: Value) {
        if let Some(stdin) = &self.stdin {
            let Ok(mut w) = stdin.lock() else { return };
            let _ = writeln!(w, "{value}");
            let _ = w.flush();
        }
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        if self.child.is_some() {
            return;
        }
        let Some(bin) = host_binary() else {
            self.status.error = Some("dove-mesh-host is missing from this build".into());
            cx.emit(RemoteEvent::Changed);
            cx.notify();
            return;
        };
        let data = self.engine.store.root().join("mesh");
        let spawned = Command::new(bin)
            .arg("--data-dir")
            .arg(&data)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(e) => {
                self.status.error = Some(format!("couldn't start remote host: {e}"));
                cx.notify();
                return;
            }
        };
        let stdout = child.stdout.take().expect("piped");
        self.stdin = child.stdin.take().map(|s| Arc::new(Mutex::new(s)));
        self.child = Some(child);
        self.status = RemoteStatus { running: true, ..Default::default() };
        let (tx, rx) = flume::unbounded::<Value>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line)
                    && tx.send(v).is_err()
                {
                    break;
                }
            }
        });
        cx.spawn(async move |this, cx| {
            while let Ok(ev) = rx.recv_async().await {
                if this.update(cx, |r, cx| r.on_event(ev, cx)).is_err() {
                    break;
                }
            }
            let _ = this.update(cx, |r, cx| {
                r.child = None;
                r.stdin = None;
                r.status.running = false;
                r.status.connected = false;
                cx.emit(RemoteEvent::Changed);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.send(json!({ "op": "shutdown" }));
        self.stdin = None;
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(2));
                let _ = child.kill();
                let _ = child.wait();
            });
        }
        self.status = RemoteStatus::default();
        cx.emit(RemoteEvent::Changed);
        cx.notify();
    }

    pub fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.child.is_none() {
            self.set_enabled(true, cx);
        }
        if self.child.is_none() {
            self.status.signing_in = false;
            cx.notify();
            return;
        }
        self.status.signing_in = true;
        self.status.error = None;
        self.send(json!({ "op": "sign_in" }));
        cx.notify();
    }

    pub fn offer_seen(&self) -> bool {
        self.engine.store.setting(SYNC_OFFER_SEEN).as_deref() == Some("1")
    }

    /// Publish this computer's sessions, then ask for the account's list.
    pub fn bring_work(&mut self) {
        let _ = self.engine.store.set_setting(SYNC_CLAIM, "bring");
        let _ = self.engine.store.set_setting(SYNC_OFFER_SEEN, "1");
        self.publish_all();
        self.request_sessions();
    }

    /// Pull the account's sessions. Threads already on this computer stay
    /// here and are not uploaded.
    pub fn start_fresh(&mut self) {
        self.hold_existing();
        let _ = self.engine.store.set_setting(SYNC_CLAIM, "fresh");
        let _ = self.engine.store.set_setting(SYNC_OFFER_SEEN, "1");
        self.request_sessions();
    }

    /// Ask the host to send the registry's chats again.
    pub fn request_sessions(&self) {
        self.send(json!({ "op": "list_sessions" }));
    }

    fn hold_existing(&self) {
        let Ok(projects) = self.engine.store.projects() else { return };
        for project in projects {
            for task in self.engine.store.tasks(project.id).unwrap_or_default() {
                self.hold_task(task.id);
            }
        }
        for task in self.engine.store.archived_tasks(10_000).unwrap_or_default() {
            self.hold_task(task.id);
        }
    }

    fn hold_task(&self, id: TaskId) {
        let owner = format!("account_device.{id}");
        if self.engine.store.setting(&owner).is_some() {
            return;
        }
        let _ = self.engine.store.set_setting(&format!("account_held.{id}"), "1");
    }

    fn publishes(&self, id: TaskId) -> bool {
        may_publish_session(
            self.engine.store.setting(SYNC_CLAIM).as_deref(),
            self.engine.store.setting(&format!("account_held.{id}")).as_deref() == Some("1"),
            self.engine.store.setting(&format!("account_device.{id}")).as_deref(),
            self.status.device_id.as_deref().unwrap_or(""),
        )
    }

    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        self.send(json!({ "op": "sign_out" }));
        cx.notify();
    }

    fn on_event(&mut self, ev: Value, cx: &mut Context<Self>) {
        let s = |k: &str| ev.get(k).and_then(Value::as_str).map(str::to_owned);
        match ev.get("ev").and_then(Value::as_str).unwrap_or_default() {
            "status" => {
                self.status.signed_in = ev.get("signedIn").and_then(Value::as_bool).unwrap_or(false);
                self.status.connected = ev.get("connected").and_then(Value::as_bool).unwrap_or(false);
                self.status.email = s("email");
                self.status.device_id = s("deviceId");
                if self.status.signed_in {
                    self.status.signing_in = false;
                }
                // Uploading waits for the offer. Connecting alone must not
                // claim this computer's threads.
                // "fresh" still publishes sessions created after the choice.
                // Held threads (the ones that were already here) are skipped
                // inside publish_all.
                if self.status.connected
                    && matches!(self.engine.store.setting(SYNC_CLAIM).as_deref(), Some("bring") | Some("fresh"))
                {
                    self.publish_all();
                }
            }
            "connected" => {
                self.status.connected = true;
                self.status.error = None;
            }
            "sign_in_url" => {
                if let Some(url) = s("url") {
                    cx.open_url(&url);
                }
            }
            "error" => {
                self.status.signing_in = false;
                self.status.error = s("message").map(|message| friendly_host_error(&message));
            }
            "account_sessions" => {
                let device = s("deviceId").unwrap_or_default();
                let chats = ev.get("chats").cloned().unwrap_or(Value::Null);
                cx.emit(RemoteEvent::AccountSessions { device, chats });
            }
            "account_transcript" => {
                let Some(chat) = s("chat") else { return };
                let Some(task) = TaskId::parse(&chat) else { return };
                let messages = ev.get("messages").cloned().unwrap_or(Value::Null);
                cx.emit(RemoteEvent::AccountTranscript { task, messages });
            }
            "command" => {
                let (Some(chat), Some(id)) = (s("chat"), s("id")) else { return };
                let Some(task) = TaskId::parse(&chat) else {
                    self.command_done(&chat, &id, false, Some("Unknown session"));
                    return;
                };
                let command = match s("kind").as_deref() {
                    Some("interrupt") => RemoteCommand::Interrupt,
                    Some("run") | Some("steer") => RemoteCommand::Run(s("prompt").unwrap_or_default()),
                    _ => return,
                };
                cx.emit(RemoteEvent::Command { task, id, command });
            }
            _ => {}
        }
        cx.emit(RemoteEvent::Changed);
        cx.notify();
    }

    /// Ask the host for a session's messages. Used for chats that belong to
    /// another device on this account.
    pub fn pull_chat(&self, chat: &str) {
        self.send(json!({ "op": "pull_chat", "id": chat }));
    }

    pub fn command_done(&self, chat: &str, id: &str, ok: bool, resolution: Option<&str>) {
        self.send(json!({ "op": "command_done", "chat": chat, "id": id, "ok": ok, "resolution": resolution }));
    }

    /// Publish every project and session row, plus recent transcripts.
    fn publish_all(&mut self) {
        let Some(stdin) = self.stdin.clone() else { return };
        let engine = self.engine.clone();
        let mine = self.status.device_id.clone().unwrap_or_default();
        // A fresh connection gets every turn again (publishing is idempotent).
        if let Ok(mut f) = self.feeds.lock() {
            f.clear();
        }
        let feeds = self.feeds.clone();
        std::thread::spawn(move || {
            let Ok(projects) = engine.store.projects() else { return };
            let mut recent = Vec::new();
            for p in &projects {
                write_line(
                    &stdin,
                    json!({ "op": "space", "id": p.id.to_string(), "path": p.path.to_string_lossy(), "name": p.name }),
                );
                for t in engine.store.tasks(p.id).unwrap_or_default() {
                    let held = engine.store.setting(&format!("account_held.{}", t.id)).as_deref() == Some("1");
                    let owner = engine.store.setting(&format!("account_device.{}", t.id));
                    if !may_publish_session(Some("bring"), held, owner.as_deref(), &mine) {
                        continue;
                    }
                    recent.push((t.updated_at, t.id));
                    write_line(&stdin, chat_row(&t, p));
                }
            }
            recent.sort_by_key(|e| std::cmp::Reverse(e.0));
            for (_, id) in recent.into_iter().take(HISTORY_SESSIONS) {
                publish_transcript(&engine, &stdin, &feeds, id, false);
            }
        });
    }

    /// Called by a session view after it applies a batch of live events.
    pub fn observe(&mut self, task: TaskId, events: &[AgentEvent]) {
        if !self.status.connected || self.stdin.is_none() || !self.publishes(task) {
            return;
        }
        let finished = events.iter().any(|e| matches!(e, AgentEvent::TurnEnded { .. } | AgentEvent::Exited { .. }));
        let due = self.last_publish.get(&task).is_none_or(|at| at.elapsed() >= REPUBLISH_EVERY);
        if !(due || finished) {
            self.pending.insert(task, true);
            return;
        }
        self.pending.remove(&task);
        self.last_publish.insert(task, Instant::now());
        let (engine, stdin, feeds) = (self.engine.clone(), self.stdin.clone().expect("checked"), self.feeds.clone());
        std::thread::spawn(move || {
            if let Ok(Some(t)) = engine.store.task(task)
                && let Ok(projects) = engine.store.projects()
                && let Some(p) = projects.iter().find(|p| p.id == t.project_id)
            {
                write_line(&stdin, chat_row(&t, p));
            }
            publish_transcript(&engine, &stdin, &feeds, task, !finished);
        });
    }
}

fn write_line(stdin: &Arc<Mutex<ChildStdin>>, value: Value) {
    let Ok(mut w) = stdin.lock() else { return };
    let _ = writeln!(w, "{value}");
    let _ = w.flush();
}

fn chat_row(t: &dove_core::store::Task, p: &dove_core::store::Project) -> Value {
    json!({
        "op": "chat",
        "id": t.id.to_string(),
        "space_id": p.id.to_string(),
        "title": t.title,
        "cwd": t.worktree.as_ref().unwrap_or(&p.path).to_string_lossy(),
        "branch": t.branch,
        "preview": t.prompt.as_deref().map(|s| s.chars().take(140).collect::<String>()),
    })
}

/// One published chat turn: (stable id, role, text, still streaming).
pub type Turn = (String, &'static str, String, bool);

/// Folds a transcript into chat turns incrementally (P15): each turn is
/// keyed by the event index it starts at, so republishing is idempotent
/// across launches, and only the open assistant turn's text is kept.
#[derive(Default)]
pub struct Fold {
    /// Events folded so far (the next event's index).
    events: usize,
    /// The assistant turn still streaming: (id, text).
    open: Option<(String, String)>,
    /// Parts of the open assistant turn. Reasoning and tools are not prose.
    open_parts: Vec<Value>,
    /// Parts of turns already closed, keyed by the turn id.
    part_of: HashMap<String, Vec<Value>>,
}

impl Fold {
    /// Fold more events. Returns the turns they created or changed, each
    /// once, in order of first appearance, with their latest state.
    pub fn push(&mut self, events: &[AgentEvent]) -> Vec<Turn> {
        let mut out: Vec<Turn> = Vec::new();
        let put = |out: &mut Vec<Turn>, turn: Turn| match out.iter_mut().find(|t| t.0 == turn.0) {
            Some(t) => *t = turn,
            None => out.push(turn),
        };
        let mut touched = false;
        for (k, ev) in events.iter().enumerate() {
            let i = self.events + k;
            match ev {
                AgentEvent::UserMessage { text } => {
                    // A superseded answer is no longer streaming.
                    if let Some((id, text)) = self.open.take()
                        && touched
                    {
                        self.part_of.insert(id.clone(), std::mem::take(&mut self.open_parts));
                        put(&mut out, (id, "assistant", text, false));
                    }
                    touched = false;
                    put(&mut out, (format!("u{i}"), "user", text.to_string(), false));
                }
                AgentEvent::TextDelta { text } => {
                    self.open.get_or_insert_with(|| (format!("a{i}"), String::new())).1.push_str(text);
                    append_part(&mut self.open_parts, "text", text);
                    touched = true;
                }
                AgentEvent::ThinkingDelta { text } => {
                    self.open.get_or_insert_with(|| (format!("a{i}"), String::new()));
                    append_part(&mut self.open_parts, "reasoning", text);
                    touched = true;
                }
                AgentEvent::ToolCall { call_id, name, title, input, .. } => {
                    self.open.get_or_insert_with(|| (format!("a{i}"), String::new()));
                    self.open_parts.push(json!({
                        "kind": "tool",
                        "callId": call_id,
                        "name": name,
                        "title": title,
                        "input": input,
                    }));
                    touched = true;
                }
                AgentEvent::ToolResult { call_id, output, is_error } => {
                    if let Some(part) = self.open_parts.iter_mut().rev().find(|part| {
                        part.get("callId").and_then(Value::as_str) == Some(call_id.as_str())
                    }) {
                        part["output"] = json!(output.as_ref());
                        part["isError"] = json!(is_error);
                        touched = true;
                    }
                }
                AgentEvent::TurnEnded { .. } | AgentEvent::Exited { .. } => {
                    if let Some((id, text)) = self.open.take() {
                        self.part_of.insert(id.clone(), std::mem::take(&mut self.open_parts));
                        put(&mut out, (id, "assistant", text, false));
                    }
                    touched = false;
                }
                _ => {}
            }
        }
        self.events += events.len();
        if touched && let Some((id, text)) = &self.open {
            self.part_of.insert(id.clone(), self.open_parts.clone());
            put(&mut out, (id.clone(), "assistant", text.clone(), true));
        }
        out
    }

    fn parts_of(&self, id: &str) -> Vec<Value> {
        self.part_of.get(id).cloned().unwrap_or_default()
    }
}

fn append_part(parts: &mut Vec<Value>, kind: &str, text: &str) {
    let key = if kind == "reasoning" { "reasoning" } else { "text" };
    if let Some(last) = parts.last_mut()
        && last.get("kind").and_then(Value::as_str) == Some(kind)
        && let Some(existing) = last.get_mut(key)
    {
        if let Some(s) = existing.as_str() {
            *existing = json!(format!("{s}{text}"));
        }
        return;
    }
    parts.push(json!({ "kind": kind, key: text }));
}

/// A whole transcript folded into chat turns.
#[cfg(test)]
pub fn fold_turns(events: &[AgentEvent]) -> Vec<Turn> {
    Fold::default().push(events)
}

/// Where each session's publishing stands: the transcript byte offset read
/// so far and the fold state, so a publish reads and sends only what is new.
#[derive(Default)]
struct Feed {
    offset: u64,
    fold: Fold,
}

type Feeds = Arc<Mutex<HashMap<TaskId, Arc<Mutex<Feed>>>>>;

fn publish_transcript(engine: &Engine, stdin: &Arc<Mutex<ChildStdin>>, feeds: &Feeds, task: TaskId, live: bool) {
    let feed = match feeds.lock() {
        Ok(mut f) => f.entry(task).or_default().clone(),
        Err(_) => return,
    };
    // One publish per session at a time, so turns go out in order.
    let Ok(mut feed) = feed.lock() else { return };
    let (events, next) = engine.store.transcript_from(task, feed.offset);
    feed.offset = next;
    let chat = task.to_string();
    for (id, role, text, open) in feed.fold.push(&events) {
        let parts = feed.fold.parts_of(&id);
        let text = text.trim().to_owned();
        if text.is_empty() && parts.is_empty() {
            continue;
        }
        write_line(
            stdin,
            json!({ "op": "message", "chat": chat, "id": format!("{chat}-{id}"), "role": role, "text": text, "parts": parts, "streaming": open && live }),
        );
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_turns_with_stable_ids() {
        let events = vec![
            AgentEvent::UserMessage { text: "hi".into() },
            AgentEvent::TextDelta { text: "Hel".into() },
            AgentEvent::TextDelta { text: "lo".into() },
            AgentEvent::TurnEnded { usage: None, is_error: false },
            AgentEvent::UserMessage { text: "again".into() },
            AgentEvent::TextDelta { text: "Sure".into() },
        ];
        let turns = fold_turns(&events);
        assert_eq!(turns.len(), 4);
        assert_eq!(turns[0], ("u0".into(), "user", "hi".into(), false));
        assert_eq!(turns[1], ("a1".into(), "assistant", "Hello".into(), false));
        assert_eq!(turns[3], ("a5".into(), "assistant", "Sure".into(), true));
    }

    #[test]
    fn incremental_folds_send_only_what_changed() {
        let events = vec![
            AgentEvent::UserMessage { text: "hi".into() },
            AgentEvent::TextDelta { text: "Hel".into() },
            AgentEvent::TextDelta { text: "lo".into() },
            AgentEvent::TurnEnded { usage: None, is_error: false },
            AgentEvent::UserMessage { text: "again".into() },
            AgentEvent::TextDelta { text: "Sure".into() },
        ];
        let mut fold = Fold::default();
        let first = fold.push(&events[..2]);
        assert_eq!(
            first,
            vec![("u0".into(), "user", "hi".into(), false), ("a1".into(), "assistant", "Hel".into(), true)]
        );
        // The open answer is resent whole; finished turns are not resent.
        assert_eq!(fold.push(&events[2..3]), vec![("a1".into(), "assistant", "Hello".into(), true)]);
        assert_eq!(fold.push(&events[3..4]), vec![("a1".into(), "assistant", "Hello".into(), false)]);
        let rest = fold.push(&events[4..]);
        assert_eq!(
            rest,
            vec![("u4".into(), "user", "again".into(), false), ("a5".into(), "assistant", "Sure".into(), true)]
        );
        // Nothing new, nothing sent.
        assert!(fold.push(&[]).is_empty());
        // Ids match a fold of the whole transcript.
        let whole: Vec<String> = fold_turns(&events).into_iter().map(|t| t.0).collect();
        assert_eq!(whole, ["u0", "a1", "u4", "a5"]);
    }

    #[test]
    fn offer_gates_upload() {
        assert!(!may_publish_session(None, false, None, "desktop"));
        assert!(!may_publish_session(Some("fresh"), true, None, "desktop"));
        assert!(may_publish_session(Some("fresh"), false, None, "desktop"));
        assert!(may_publish_session(Some("bring"), false, None, "desktop"));
        assert!(!may_publish_session(Some("bring"), false, Some("phone"), "desktop"));
        assert!(may_publish_session(Some("bring"), false, Some("desktop"), "desktop"));
        assert!(!may_publish_session(Some("bring"), true, None, "desktop"));
    }
}
