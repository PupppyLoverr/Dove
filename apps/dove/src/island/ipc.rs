//! The line between the island (`dove --island`) and the engine (`dove`,
//! with or without its window): newline-delimited JSON over a named pipe on
//! Windows, a Unix socket elsewhere. The engine serves; the island connects.
//!
//! - The address is per data folder (`DOVE_HOME`), so a QA copy on a scratch
//!   home never talks to the real one.
//! - The first frame is `hello` with the token the engine wrote to
//!   `<home>/run/island.token` (readable only by the user); anything else
//!   drops the connection.
//! - The engine pushes a full [`Snapshot`] whenever it changes; the island
//!   sends commands. Nobody polls.
//! - Owning the address also makes the engine single-instance: a second
//!   `dove` finds it taken, asks the first to show its window and exits.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dove_core::proto::{HarnessKind, PermissionDecision, ProjectId, TaskId};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::view::{CallInfo, NotchView, PlanVerdict, ShowWhat};
use crate::notch::Snapshot;

/// Island → engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ToEngine {
    Hello {
        token: String,
    },
    Send {
        task: TaskId,
        text: String,
    },
    Decide {
        task: TaskId,
        request_id: String,
        decision: PermissionDecision,
    },
    Stop {
        task: TaskId,
    },
    Start {
        id: u64,
        agent: HarnessKind,
        project: ProjectId,
        prompt: String,
        worktree: bool,
    },
    /// Shows Dove's window, on `task` if given (opening it if closed).
    Open {
        task: Option<TaskId>,
    },
    /// Opens Flock in Dove's window.
    OpenFlock,
    /// Opens Settings › Notifications (where the notch's settings are).
    OpenSettings,
    /// Answers a question request: per question, the chosen labels or text.
    Answer {
        task: TaskId,
        request_id: String,
        answers: Vec<Vec<String>>,
    },
    /// The island has this agent open: stream its text as it arrives.
    Watch {
        task: Option<TaskId>,
    },
    /// Merges an agent's pull request.
    Merge {
        task: TaskId,
        number: u64,
    },
    /// Runs an automation now.
    RunAutomation {
        id: String,
    },
    /// Push-to-talk: the mic pressed (`on`) or released.
    Dictate {
        on: bool,
    },
    /// Voice Mode: start a call (about `task`, else with K22) or end it.
    Call {
        on: bool,
        #[serde(default)]
        task: Option<TaskId>,
    },
    CallMute {
        on: bool,
    },
    /// Typed into the call, as if said.
    CallSay {
        text: String,
    },
    /// Talk to another thread (`None`: K22).
    CallTarget {
        task: Option<TaskId>,
    },
    /// Ask K22, typed in the notch; it answers in a view (`ToIsland::View`).
    Ask {
        id: u64,
        text: String,
    },
    /// Answers an agent's plan.
    Plan {
        task: TaskId,
        request_id: Option<String>,
        verdict: PlanVerdict,
    },
    /// K22's agent and model (Settings › Voice, or the call page).
    SetVoiceAgent {
        agent: Option<HarnessKind>,
        model: Option<String>,
    },
    /// Quits the engine (and the UI) for good.
    QuitAll,
}

/// Engine → island.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ToIsland {
    Snapshot {
        snap: Snapshot,
    },
    Started {
        id: u64,
        result: Result<TaskId, String>,
    },
    /// Push-to-talk: the words so far (`live`), the final words, or why it
    /// couldn't listen.
    Dictation {
        text: String,
        done: bool,
        error: Option<String>,
    },
    /// A merge finished.
    Merged {
        task: TaskId,
        result: Result<(), String>,
    },
    /// The Voice Mode call, whenever it changes (`live: false` once over).
    Call {
        call: CallInfo,
    },
    /// A view to show (a newer one with the same id replaces it).
    View {
        view: NotchView,
    },
    /// An `Ask`: the K22 thread it went to, or why it couldn't.
    Asked {
        id: u64,
        result: Result<TaskId, String>,
    },
    /// Open something (voice: "show me the plan").
    Show {
        what: ShowWhat,
    },
    /// The engine is going away (quit, or nothing left to run).
    Bye,
}

/// Where this data folder's engine listens.
pub fn address(root: &Path) -> String {
    use sha2::Digest as _;
    let hash = sha2::Sha256::digest(root.display().to_string().to_lowercase().as_bytes());
    let short: String = hash.iter().take(6).map(|b| format!("{b:02x}")).collect();
    if cfg!(windows) {
        format!(r"\\.\pipe\dove-island-{short}")
    } else {
        root.join("run").join("island.sock").display().to_string()
    }
}

fn token_path(root: &Path) -> PathBuf {
    root.join("run").join("island.token")
}

/// A fresh token for this engine run, written where only the user reads it.
pub fn new_token(root: &Path) -> String {
    let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let path = token_path(root);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, &token);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    token
}

pub fn read_token(root: &Path) -> Option<String> {
    std::fs::read_to_string(token_path(root)).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

// ---------------------------------------------------------------------------
// Engine side
// ---------------------------------------------------------------------------

/// Connected islands; each gets every frame.
#[derive(Clone, Default)]
pub struct Hub(Arc<Mutex<Vec<flume::Sender<String>>>>);

impl Hub {
    pub fn broadcast(&self, msg: &ToIsland) {
        let Ok(line) = serde_json::to_string(msg) else { return };
        self.0.lock().unwrap_or_else(|e| e.into_inner()).retain(|tx| tx.send(line.clone()).is_ok());
    }

    pub fn is_empty(&self) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    fn add(&self, tx: flume::Sender<String>) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(tx);
    }
}

/// Owns the address, or returns false when another engine already does.
/// Commands from islands arrive on `inbound`; `on_connect` runs for each new
/// island (to send it the current snapshot).
pub fn serve(
    rt: &tokio::runtime::Handle,
    root: &Path,
    token: String,
    hub: Hub,
    inbound: flume::Sender<ToEngine>,
    on_connect: flume::Sender<()>,
) -> bool {
    let addr = address(root);
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ServerOptions;
        let _guard = rt.enter();
        let Ok(first) = ServerOptions::new().first_pipe_instance(true).create(&addr) else {
            return false;
        };
        rt.spawn(async move {
            let mut server = first;
            loop {
                if server.connect().await.is_err() {
                    match ServerOptions::new().create(&addr) {
                        Ok(s) => server = s,
                        Err(_) => break,
                    }
                    continue;
                }
                let conn = server;
                server = match ServerOptions::new().create(&addr) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("island: pipe closed: {e}");
                        break;
                    }
                };
                tokio::spawn(session(conn, token.clone(), hub.clone(), inbound.clone(), on_connect.clone()));
            }
        });
        true
    }
    #[cfg(unix)]
    {
        let path = PathBuf::from(&addr);
        // A live engine answers; a stale socket file from a crash doesn't.
        if std::os::unix::net::UnixStream::connect(&path).is_ok() {
            return false;
        }
        let _ = std::fs::remove_file(&path);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _guard = rt.enter();
        let Ok(listener) = tokio::net::UnixListener::bind(&path) else { return false };
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        rt.spawn(async move {
            while let Ok((conn, _)) = listener.accept().await {
                tokio::spawn(session(conn, token.clone(), hub.clone(), inbound.clone(), on_connect.clone()));
            }
        });
        true
    }
}

async fn session<S>(conn: S, token: String, hub: Hub, inbound: flume::Sender<ToEngine>, on_connect: flume::Sender<()>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
{
    let (read, mut write) = tokio::io::split(conn);
    let mut lines = BufReader::new(read).lines();
    let Ok(Some(first)) = lines.next_line().await else { return };
    match serde_json::from_str::<ToEngine>(&first) {
        Ok(ToEngine::Hello { token: t }) if t == token => {}
        _ => {
            tracing::warn!("island: a connection without the token was dropped");
            return;
        }
    }
    let (tx, rx) = flume::unbounded::<String>();
    hub.add(tx);
    let _ = on_connect.send(());
    tokio::spawn(async move {
        while let Ok(line) = rx.recv_async().await {
            if write.write_all(line.as_bytes()).await.is_err() || write.write_all(b"\n").await.is_err() {
                break;
            }
            let _ = write.flush().await;
        }
    });
    while let Ok(Some(line)) = lines.next_line().await {
        match serde_json::from_str::<ToEngine>(&line) {
            Ok(msg) => {
                if inbound.send(msg).is_err() {
                    break;
                }
            }
            Err(e) => tracing::debug!("island: bad frame: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Island side
// ---------------------------------------------------------------------------

/// A live line to the engine.
pub struct Link {
    pub outbound: flume::Sender<ToEngine>,
    /// Closes when the engine goes away.
    pub inbound: flume::Receiver<ToIsland>,
}

/// Connects to this data folder's engine, if one is running.
pub fn connect(rt: &tokio::runtime::Handle, root: &Path) -> Option<Link> {
    let token = read_token(root)?;
    let addr = address(root);
    #[cfg(windows)]
    let conn = {
        use tokio::net::windows::named_pipe::ClientOptions;
        let _guard = rt.enter();
        ClientOptions::new().open(&addr).ok()?
    };
    #[cfg(unix)]
    let conn = {
        let std = std::os::unix::net::UnixStream::connect(&addr).ok()?;
        std.set_nonblocking(true).ok()?;
        let _guard = rt.enter();
        tokio::net::UnixStream::from_std(std).ok()?
    };
    let (out_tx, out_rx) = flume::unbounded::<ToEngine>();
    let (in_tx, in_rx) = flume::unbounded::<ToIsland>();
    let _ = out_tx.send(ToEngine::Hello { token });
    rt.spawn(async move {
        let (read, mut write) = tokio::io::split(conn);
        let writer = tokio::spawn(async move {
            while let Ok(msg) = out_rx.recv_async().await {
                let Ok(line) = serde_json::to_string(&msg) else { continue };
                if write.write_all(line.as_bytes()).await.is_err() || write.write_all(b"\n").await.is_err() {
                    break;
                }
                let _ = write.flush().await;
            }
        });
        let mut lines = BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            match serde_json::from_str::<ToIsland>(&line) {
                Ok(msg) => {
                    if in_tx.send(msg).is_err() {
                        break;
                    }
                }
                Err(e) => tracing::debug!("island: bad frame: {e}"),
            }
        }
        writer.abort();
    });
    Some(Link { outbound: out_tx, inbound: in_rx })
}

/// Asks a running engine to show its window (a second `dove` launch).
/// True when one answered.
pub fn ask_to_show(root: &Path) -> bool {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return false };
    let Some(link) = connect(rt.handle(), root) else { return false };
    let _ = link.outbound.send(ToEngine::Open { task: None });
    // Give the writer a moment to flush before the runtime goes away.
    rt.block_on(async { tokio::time::sleep(std::time::Duration::from_millis(300)).await });
    true
}
