//! Paired-device exec over the device relay ("connect your Mac").
//!
//! Two stream kinds ride the same `DeviceRoom` relay as RPC (frames
//! `{s, k, to, from}`, see [`crate::device_room`]):
//!
//! - `pair`: one JSON request, one JSON reply. A client presents the node's
//!   short pairing code; the node answers with its id, name and a fresh
//!   per-device secret, which the client keeps and presents on every exec.
//! - `exec`: one process per stream. Payloads are a one-byte tag plus data:
//!
//!   | tag | direction      | data                                   |
//!   |-----|----------------|----------------------------------------|
//!   | `o` | client → node  | JSON [`ExecOpen`] (first frame)        |
//!   | `i` | client → node  | stdin bytes                            |
//!   | `e` | client → node  | stdin EOF                              |
//!   | `k` | client → node  | hang up the process group              |
//!   | `c` | both           | credit: `u32` LE bytes consumed        |
//!   | `s` | node → client  | process started                        |
//!   | `r` | node → client  | refused: JSON `{error}`                |
//!   | `1` | node → client  | stdout bytes                           |
//!   | `2` | node → client  | stderr bytes                           |
//!   | `x` | node → client  | exit: JSON `{code}` (last frame)       |
//!
//! Flow control is credit based in both directions: a sender may have at
//! most [`WINDOW`] data bytes unacknowledged, and the receiver returns
//! credit only after the bytes reached their sink (child stdin, local
//! stdout). Nothing buffers without bound.
//!
//! Trust is the node's [`NodePolicy`]: the relay already scopes rooms to one
//! account, and on top of that a node runs processes only for devices that
//! paired with it and present the secret it issued.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Semaphore, mpsc};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::device_room::{
    DeviceFrameHeader, ECHO_KIND, HOST_CLOSED, HOST_OFFLINE, PING_INTERVAL, RELAY_KIND,
    RelayStreamSession, RelayStreams, SILENCE_LEASE, binary_frame, decode_device_frame,
    device_room_ws_url, encode_device_frame, relay_error_code,
};

pub const EXEC_KIND: &str = "exec";
pub const PAIR_KIND: &str = "pair";

/// Exit code for "the relay or the connection failed": callers reconnect.
pub const EXIT_LINK_LOST: i32 = 255;
/// Exit code for "this computer refused": not paired, revoked, bad request.
pub const EXIT_REFUSED: i32 = 77;

/// Largest data payload per frame.
pub const CHUNK: usize = 32 * 1024;
/// Unacknowledged data bytes allowed per direction per stream.
pub const WINDOW: usize = 256 * 1024;
/// Concurrent exec streams one host socket serves.
const MAX_STREAMS: usize = 64;
/// How often a running stream re-checks that its device is still trusted.
const RECHECK: Duration = Duration::from_secs(5);
/// After the process exits, how long to wait for its pipes to drain.
const DRAIN_GRACE: Duration = Duration::from_secs(2);
/// Time from dial to the node's `started` (or `refused`) reply.
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);
/// No echo from the node for this long (after one was seen) = dead path.
const ECHO_DEADLINE: Duration = Duration::from_secs(30);
const DIAL_TIMEOUT: Duration = Duration::from_secs(15);

const T_OPEN: u8 = b'o';
const T_STDIN: u8 = b'i';
const T_EOF: u8 = b'e';
const T_KILL: u8 = b'k';
const T_CREDIT: u8 = b'c';
const T_STARTED: u8 = b's';
const T_REFUSED: u8 = b'r';
const T_STDOUT: u8 = b'1';
const T_STDERR: u8 = b'2';
const T_EXIT: u8 = b'x';

/// The first frame of an exec stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecOpen {
    /// The calling device's id (as paired).
    pub device: String,
    /// The secret the node issued to `device` when they paired.
    pub proof: String,
    pub argv: Vec<String>,
    /// Working directory on the node; the node's home when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairRequest {
    pub code: String,
    /// The requesting device's id and display name.
    pub device: String,
    pub name: String,
}

/// What a successful pairing hands the client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairGrant {
    /// The node's device id and display name.
    pub device: String,
    pub name: String,
    /// The proof to present in [`ExecOpen::proof`].
    pub secret: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PairReply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    grant: Option<PairGrant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// The node's trust decisions. Implementations do small blocking file I/O.
pub trait NodePolicy: Send + Sync + 'static {
    /// `Ok` only for a device that paired with this node and presents the
    /// secret it was issued. The message of `Err` is shown to the caller.
    fn authorize(&self, device: &str, proof: &str) -> Result<(), String>;
    /// Redeem a pairing code (single use, expiring).
    fn pair(&self, request: &PairRequest) -> Result<PairGrant, String>;
    /// Where processes start when the caller names no directory.
    fn workdir(&self) -> PathBuf;
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn tagged(tag: u8, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 1);
    out.push(tag);
    out.extend_from_slice(data);
    out
}

fn error_json(message: &str) -> Vec<u8> {
    serde_json::json!({ "error": message })
        .to_string()
        .into_bytes()
}

// ---------------------------------------------------------------------------
// Node side
// ---------------------------------------------------------------------------

/// Serves `exec` and `pair` streams through a [`crate::HostRelay`]
/// (`HostRelayConfig::streams`).
pub struct ExecStreams {
    policy: Arc<dyn NodePolicy>,
}

impl ExecStreams {
    pub fn new(policy: Arc<dyn NodePolicy>) -> Arc<Self> {
        Arc::new(Self { policy })
    }
}

impl RelayStreams for ExecStreams {
    fn session(&self, out: mpsc::Sender<Vec<u8>>) -> Box<dyn RelayStreamSession> {
        Box::new(ExecSession {
            policy: self.policy.clone(),
            out,
            streams: Arc::new(Mutex::new(HashMap::new())),
        })
    }
}

enum Input {
    Stdin(Vec<u8>),
    Eof,
    Credit(usize),
    Kill,
}

type Streams = Arc<Mutex<HashMap<(String, String), mpsc::UnboundedSender<Input>>>>;

/// One per host socket. Dropping it (socket ended) drops every stream's
/// input channel, which hangs up the processes.
struct ExecSession {
    policy: Arc<dyn NodePolicy>,
    out: mpsc::Sender<Vec<u8>>,
    streams: Streams,
}

#[derive(Clone)]
struct Reply {
    out: mpsc::Sender<Vec<u8>>,
    kind: &'static str,
    conn: String,
    stream: String,
}

impl Reply {
    async fn send(&self, payload: &[u8]) -> bool {
        let header =
            DeviceFrameHeader::new(self.stream.clone(), self.kind).with_to(self.conn.clone());
        match encode_device_frame(&header, payload) {
            Ok(frame) => self.out.send(frame).await.is_ok(),
            Err(_) => false,
        }
    }

    async fn tag(&self, tag: u8, data: &[u8]) -> bool {
        self.send(&tagged(tag, data)).await
    }
}

impl RelayStreamSession for ExecSession {
    fn handles(&self, kind: &str) -> bool {
        kind == EXEC_KIND || kind == PAIR_KIND
    }

    fn frame(&mut self, header: DeviceFrameHeader, payload: Vec<u8>) {
        let Some(conn) = header.from else { return };
        if header.k == PAIR_KIND {
            let reply = Reply {
                out: self.out.clone(),
                kind: PAIR_KIND,
                conn,
                stream: header.s,
            };
            let policy = self.policy.clone();
            tokio::spawn(async move {
                let result = match serde_json::from_slice::<PairRequest>(&payload) {
                    Ok(request) => tokio::task::spawn_blocking(move || policy.pair(&request))
                        .await
                        .unwrap_or_else(|_| Err("pairing failed".into())),
                    Err(_) => Err("malformed pairing request".into()),
                };
                let body = match result {
                    Ok(grant) => PairReply {
                        grant: Some(grant),
                        error: None,
                    },
                    Err(error) => PairReply {
                        grant: None,
                        error: Some(error),
                    },
                };
                reply
                    .send(&serde_json::to_vec(&body).unwrap_or_default())
                    .await;
            });
            return;
        }
        let Some((&tag, data)) = payload.split_first() else {
            return;
        };
        let key = (conn, header.s);
        if tag == T_OPEN {
            let mut streams = lock(&self.streams);
            if streams.contains_key(&key) {
                return;
            }
            let reply = Reply {
                out: self.out.clone(),
                kind: EXEC_KIND,
                conn: key.0.clone(),
                stream: key.1.clone(),
            };
            if streams.len() >= MAX_STREAMS {
                tokio::spawn(async move {
                    reply
                        .tag(T_REFUSED, &error_json("too many sessions on this computer"))
                        .await
                });
                return;
            }
            let (tx, rx) = mpsc::unbounded_channel();
            streams.insert(key.clone(), tx);
            let registry = Arc::downgrade(&self.streams);
            let policy = self.policy.clone();
            let open = data.to_vec();
            tokio::spawn(async move {
                serve_stream(policy, reply, open, rx).await;
                forget(&registry, &key);
            });
            return;
        }
        let input = match tag {
            T_STDIN => Input::Stdin(data.to_vec()),
            T_EOF => Input::Eof,
            T_KILL => Input::Kill,
            T_CREDIT => match <[u8; 4]>::try_from(data) {
                Ok(n) => Input::Credit(u32::from_le_bytes(n) as usize),
                Err(_) => return,
            },
            _ => return,
        };
        if let Some(tx) = lock(&self.streams).get(&key) {
            let _ = tx.send(input);
        }
    }

    fn conn_closed(&mut self, conn_id: &str) {
        lock(&self.streams).retain(|(conn, _), _| conn != conn_id);
    }
}

fn forget(
    registry: &Weak<Mutex<HashMap<(String, String), mpsc::UnboundedSender<Input>>>>,
    key: &(String, String),
) {
    if let Some(map) = registry.upgrade() {
        lock(&map).remove(key);
    }
}

/// Hang up a process group: SIGHUP now, SIGKILL shortly after.
fn hang_up(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid.and_then(|p| i32::try_from(p).ok()).filter(|p| *p > 1) {
        // SAFETY: plain syscalls on a process group we created.
        unsafe {
            libc::kill(-pid, libc::SIGHUP);
        }
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            // SAFETY: as above.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        });
    }
    #[cfg(not(unix))]
    let _ = pid;
}

fn exit_code(status: &std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    1
}

async fn pump_output(
    mut reader: impl AsyncRead + Unpin,
    tag: u8,
    credit: Arc<Semaphore>,
    reply: Reply,
    done: mpsc::Sender<()>,
) {
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        match credit.acquire_many(n as u32).await {
            Ok(permit) => permit.forget(),
            Err(_) => break,
        }
        if !reply.tag(tag, &buf[..n]).await {
            break;
        }
    }
    let _ = done.send(()).await;
}

async fn serve_stream(
    policy: Arc<dyn NodePolicy>,
    reply: Reply,
    open: Vec<u8>,
    mut rx: mpsc::UnboundedReceiver<Input>,
) {
    let open: ExecOpen = match serde_json::from_slice(&open) {
        Ok(open) => open,
        Err(_) => {
            reply
                .tag(T_REFUSED, &error_json("malformed exec request"))
                .await;
            return;
        }
    };
    if let Err(error) = policy.authorize(&open.device, &open.proof) {
        reply.tag(T_REFUSED, &error_json(&error)).await;
        return;
    }
    let Some(program) = open.argv.first() else {
        reply.tag(T_REFUSED, &error_json("empty command")).await;
        return;
    };
    let mut command = tokio::process::Command::new(program);
    command
        .args(&open.argv[1..])
        .current_dir(
            open.cwd
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| policy.workdir()),
        )
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            reply.tag(T_STARTED, &[]).await;
            reply
                .tag(
                    T_STDERR,
                    format!("dove-mesh-host: {program}: {err}\n").as_bytes(),
                )
                .await;
            reply.tag(T_EXIT, br#"{"code":127}"#).await;
            return;
        }
    };
    if !reply.tag(T_STARTED, &[]).await {
        return;
    }
    let pid = child.id();
    let credit = Arc::new(Semaphore::new(WINDOW));
    let (done_tx, mut done_rx) = mpsc::channel::<()>(2);
    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(tokio::spawn(pump_output(
            stdout,
            T_STDOUT,
            credit.clone(),
            reply.clone(),
            done_tx.clone(),
        )));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(tokio::spawn(pump_output(
            stderr,
            T_STDERR,
            credit.clone(),
            reply.clone(),
            done_tx.clone(),
        )));
    }
    drop(done_tx);
    let mut open_readers = readers.len();

    // stdin: bytes land in the child's pipe before their credit goes back.
    let unacked = Arc::new(AtomicUsize::new(0));
    let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let mut stdin_tx = Some(stdin_tx);
    let writer = child.stdin.take().map(|mut stdin| {
        let (reply, unacked) = (reply.clone(), unacked.clone());
        tokio::spawn(async move {
            while let Some(bytes) = stdin_rx.recv().await {
                let ok = stdin.write_all(&bytes).await.is_ok() && stdin.flush().await.is_ok();
                unacked.fetch_sub(bytes.len(), Ordering::AcqRel);
                if !ok
                    || !reply
                        .tag(T_CREDIT, &(bytes.len() as u32).to_le_bytes())
                        .await
                {
                    break;
                }
            }
            // Dropping `stdin` closes the pipe: the child sees EOF.
        })
    });

    let mut recheck = tokio::time::interval(RECHECK);
    recheck.tick().await;
    let mut status = None;
    let mut grace: Option<tokio::time::Instant> = None;
    enum End {
        Exited,
        Gone,
        Refused(String),
    }
    let end = loop {
        if status.is_some() && open_readers == 0 {
            break End::Exited;
        }
        tokio::select! {
            input = rx.recv() => match input {
                None => break End::Gone,
                Some(Input::Kill) => hang_up(pid),
                Some(Input::Credit(n)) => {
                    // A credit beyond what was ever sent is a confused peer; cap it.
                    if credit.available_permits() + n <= WINDOW {
                        credit.add_permits(n);
                    }
                }
                Some(Input::Stdin(bytes)) => {
                    if let Some(tx) = &stdin_tx {
                        let len = bytes.len();
                        if unacked.fetch_add(len, Ordering::AcqRel) + len > WINDOW + CHUNK {
                            break End::Refused("stdin flow-control violation".into());
                        }
                        if tx.send(bytes).is_err() {
                            unacked.fetch_sub(len, Ordering::AcqRel);
                        }
                    }
                }
                Some(Input::Eof) => stdin_tx = None,
            },
            exited = child.wait(), if status.is_none() => {
                status = Some(exited.map(|s| exit_code(&s)).unwrap_or(1));
                grace = Some(tokio::time::Instant::now() + DRAIN_GRACE);
            }
            Some(()) = done_rx.recv() => open_readers = open_readers.saturating_sub(1),
            _ = tokio::time::sleep_until(grace.unwrap_or_else(tokio::time::Instant::now)), if grace.is_some() => {
                break End::Exited;
            }
            _ = recheck.tick() => {
                if let Err(error) = policy.authorize(&open.device, &open.proof) {
                    break End::Refused(error);
                }
            }
        }
    };
    for reader in &readers {
        reader.abort();
    }
    if let Some(writer) = writer {
        writer.abort();
    }
    match end {
        End::Exited => {
            let code = status.unwrap_or(1);
            reply
                .tag(
                    T_EXIT,
                    serde_json::json!({ "code": code }).to_string().as_bytes(),
                )
                .await;
        }
        End::Gone => hang_up(pid),
        End::Refused(error) => {
            hang_up(pid);
            reply.tag(T_REFUSED, &error_json(&error)).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Client side
// ---------------------------------------------------------------------------

/// Which device to reach, and the account bearer for the relay.
#[derive(Clone)]
pub struct Dial {
    pub edge_url: String,
    pub device: String,
    pub token: String,
    /// Hostname to try on the LAN / Tailscale before the relay. Empty skips
    /// the direct attempt.
    pub direct_name: Option<String>,
}

impl std::fmt::Debug for Dial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the token.
        f.debug_struct("Dial")
            .field("edge_url", &self.edge_url)
            .field("device", &self.device)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExecError {
    /// The node answered and said no (not paired, revoked, bad request).
    #[error("{0}")]
    Refused(String),
    /// The relay or the connection failed; retrying may help.
    #[error("{0}")]
    Link(String),
}

impl ExecError {
    pub fn exit_code(&self) -> i32 {
        match self {
            ExecError::Refused(_) => EXIT_REFUSED,
            ExecError::Link(_) => EXIT_LINK_LOST,
        }
    }
}

struct Link {
    tx: mpsc::Sender<Vec<u8>>,
    rx: mpsc::Receiver<Vec<u8>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Link {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Port the node listens on for a direct LAN or Tailscale connection.
/// The relay is used when nothing answers here. The pairing secret is still
/// required; the port does not accept anonymous commands.
pub const DIRECT_PORT: u16 = 53721;

async fn dial(target: &Dial) -> Result<Link, ExecError> {
    if let Some(link) = dial_direct(target).await {
        return Ok(link);
    }
    let conn = uuid::Uuid::new_v4().to_string();
    let url = device_room_ws_url(
        &target.edge_url,
        &target.device,
        "client",
        Some(&conn),
        &target.token,
    );
    let ws = match tokio::time::timeout(DIAL_TIMEOUT, zeron_sync::dial::connect_ws(&url)).await {
        Ok(Ok(ws)) => ws,
        // The URL carries the token: report the error, never the URL.
        Ok(Err(err)) => return Err(ExecError::Link(format!("could not reach the relay: {err}"))),
        Err(_) => return Err(ExecError::Link("timed out reaching the relay".into())),
    };
    Ok(link_from_socket(ws))
}

fn link_from_socket(ws: zeron_sync::dial::WsStream) -> Link {
    let (tx, wire_rx) = mpsc::channel::<Vec<u8>>(64);
    let (wire_in, rx) = mpsc::channel::<Vec<u8>>(64);
    let task = tokio::spawn(zeron_sync::socket::pump_with_timing(
        ws,
        wire_rx,
        wire_in,
        WsMessage::Binary,
        binary_frame,
        PING_INTERVAL,
        SILENCE_LEASE,
    ));
    Link { tx, rx, task }
}

/// Tailscale peers whose hostname matches `name`, as IP strings.
fn tailscale_ips(name: &str) -> Vec<String> {
    let mut cmd = std::process::Command::new("tailscale");
    cmd.args(["status", "--json"]).stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let Ok(out) = cmd.output() else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else { return Vec::new() };
    let Some(peers) = v.get("Peer").and_then(|p| p.as_object()) else { return Vec::new() };
    let want = name.to_ascii_lowercase();
    let mut ips = Vec::new();
    for peer in peers.values() {
        let host = peer.get("HostName").and_then(|h| h.as_str()).unwrap_or("");
        let dns = peer.get("DNSName").and_then(|h| h.as_str()).unwrap_or("");
        let host_l = host.to_ascii_lowercase();
        let dns_l = dns.to_ascii_lowercase();
        if host_l != want && !dns_l.starts_with(&format!("{want}.")) {
            continue;
        }
        if let Some(list) = peer.get("TailscaleIPs").and_then(|i| i.as_array()) {
            for ip in list.iter().filter_map(|i| i.as_str()) {
                if ip.contains('.') {
                    ips.push(ip.to_string());
                }
            }
        }
    }
    ips
}

async fn dial_direct(target: &Dial) -> Option<Link> {
    let name = target.direct_name.as_deref().filter(|s| !s.is_empty())?;
    let name = name.to_string();
    let ips = tokio::time::timeout(
        Duration::from_secs(1),
        tokio::task::spawn_blocking(move || tailscale_ips(&name)),
    )
    .await
    .ok()?
    .ok()?;
    for ip in ips {
        let url = format!("ws://{ip}:{DIRECT_PORT}/dove");
        let Ok(Ok(ws)) =
            tokio::time::timeout(Duration::from_millis(700), zeron_sync::dial::connect_ws(&url)).await
        else {
            continue;
        };
        return Some(link_from_socket(ws));
    }
    None
}

/// Listen for a direct connection. A bind failure leaves the relay as the
/// only path. Dropping the handle stops accepting.
pub fn spawn_direct(policy: Arc<dyn NodePolicy>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(("0.0.0.0", DIRECT_PORT)).await {
            Ok(listener) => listener,
            Err(err) => {
                tracing::info!(%err, "paired-device direct path is not listening; the relay still works");
                return;
            }
        };
        tracing::info!(port = DIRECT_PORT, "paired-device direct path listening");
        loop {
            let Ok((stream, _)) = listener.accept().await else { break };
            let policy = policy.clone();
            tokio::spawn(async move {
                let _ = serve_direct_one(stream, policy).await;
            });
        }
    })
}

async fn serve_direct_one(stream: tokio::net::TcpStream, policy: Arc<dyn NodePolicy>) -> Result<(), String> {
    use futures::{SinkExt, StreamExt};
    let mut ws = tokio_tungstenite::accept_async(stream).await.map_err(|e| e.to_string())?;
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(64);
    let streams = ExecStreams::new(policy);
    let mut session = streams.session(out_tx);
    loop {
        tokio::select! {
            incoming = ws.next() => {
                let Some(Ok(msg)) = incoming else { break };
                let Some(bytes) = binary_frame(msg) else { continue };
                let Ok((mut header, payload)) = decode_device_frame(&bytes) else { continue };
                if header.from.is_none() {
                    header.from = Some("direct".into());
                }
                if session.handles(&header.k) {
                    session.frame(header, payload);
                }
            }
            bytes = out_rx.recv() => {
                let Some(bytes) = bytes else { break };
                if ws.send(WsMessage::Binary(bytes.into())).await.is_err() {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn frame(kind: &str, stream: &str, payload: &[u8]) -> Vec<u8> {
    encode_device_frame(&DeviceFrameHeader::new(stream, kind), payload).unwrap_or_default()
}

fn relay_failure(payload: &[u8], device: &str) -> ExecError {
    match relay_error_code(payload).as_deref() {
        Some(HOST_OFFLINE) => ExecError::Link(format!(
            "{device} is offline (Keplorayx is not serving it as a machine)"
        )),
        Some(HOST_CLOSED) => ExecError::Link(format!("{device} disconnected")),
        Some(code) => ExecError::Link(format!("relay: {code}")),
        None => ExecError::Link("relay error".into()),
    }
}

/// Redeem `code` with the node `target.device`.
pub async fn pair(target: &Dial, request: &PairRequest) -> Result<PairGrant, ExecError> {
    let mut link = dial(target).await?;
    let stream = uuid::Uuid::new_v4().to_string();
    let body = serde_json::to_vec(request).map_err(|e| ExecError::Refused(e.to_string()))?;
    if link
        .tx
        .send(frame(PAIR_KIND, &stream, &body))
        .await
        .is_err()
    {
        return Err(ExecError::Link("connection lost".into()));
    }
    let wait = async {
        while let Some(bytes) = link.rx.recv().await {
            let Ok((header, payload)) = decode_device_frame(&bytes) else {
                continue;
            };
            if header.k == RELAY_KIND {
                return Err(relay_failure(&payload, &target.device));
            }
            if header.k != PAIR_KIND || header.s != stream {
                continue;
            }
            let reply: PairReply = serde_json::from_slice(&payload)
                .map_err(|_| ExecError::Link("malformed pairing reply".into()))?;
            return match (reply.grant, reply.error) {
                (Some(grant), _) => Ok(grant),
                (None, error) => Err(ExecError::Refused(
                    error.unwrap_or_else(|| "pairing refused".into()),
                )),
            };
        }
        Err(ExecError::Link("connection lost".into()))
    };
    tokio::time::timeout(Duration::from_secs(20), wait)
        .await
        .unwrap_or_else(|_| Err(ExecError::Link("timed out waiting for the computer".into())))
}

/// Whether `target.device` is serving its relay right now (one echo round
/// trip, bounded by `timeout`).
pub async fn probe(target: &Dial, timeout: Duration) -> bool {
    let attempt = async {
        let mut link = dial(target).await.ok()?;
        link.tx.send(frame(ECHO_KIND, ECHO_KIND, &[])).await.ok()?;
        while let Some(bytes) = link.rx.recv().await {
            match decode_device_frame(&bytes) {
                Ok((header, _)) if header.k == ECHO_KIND => return Some(true),
                Ok((header, _)) if header.k == RELAY_KIND => return Some(false),
                _ => {}
            }
        }
        Some(false)
    };
    tokio::time::timeout(timeout, attempt)
        .await
        .ok()
        .flatten()
        .unwrap_or(false)
}

async fn write_out(
    mut sink: impl AsyncWrite + Unpin,
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
    credit_to: mpsc::Sender<Vec<u8>>,
    stream: String,
) {
    while let Some(bytes) = rx.recv().await {
        if sink.write_all(&bytes).await.is_err() || sink.flush().await.is_err() {
            // The local reader is gone; keep granting credit so the node
            // never stalls on us (output is discarded, like a closed pipe).
            while let Some(bytes) = rx.recv().await {
                let _ = credit_to
                    .send(frame(
                        EXEC_KIND,
                        &stream,
                        &tagged(T_CREDIT, &(bytes.len() as u32).to_le_bytes()),
                    ))
                    .await;
            }
            return;
        }
        let _ = credit_to
            .send(frame(
                EXEC_KIND,
                &stream,
                &tagged(T_CREDIT, &(bytes.len() as u32).to_le_bytes()),
            ))
            .await;
    }
}

/// Run `open.argv` on `target.device` with live stdio. Returns the remote
/// exit code once all output has been written locally.
pub async fn exec<I, O, E>(
    target: &Dial,
    open: &ExecOpen,
    stdin: I,
    stdout: O,
    stderr: E,
) -> Result<i32, ExecError>
where
    I: AsyncRead + Unpin + Send + 'static,
    O: AsyncWrite + Unpin + Send + 'static,
    E: AsyncWrite + Unpin + Send + 'static,
{
    let mut link = dial(target).await?;
    let stream = uuid::Uuid::new_v4().to_string();
    let body = serde_json::to_vec(open).map_err(|e| ExecError::Refused(e.to_string()))?;
    if link
        .tx
        .send(frame(EXEC_KIND, &stream, &tagged(T_OPEN, &body)))
        .await
        .is_err()
    {
        return Err(ExecError::Link("connection lost".into()));
    }
    let _ = link.tx.send(frame(ECHO_KIND, ECHO_KIND, &[])).await;

    // stdin → node, under the node's credit.
    let credit = Arc::new(Semaphore::new(WINDOW));
    let stdin_task = {
        let (tx, credit, stream) = (link.tx.clone(), credit.clone(), stream.clone());
        let mut stdin = stdin;
        tokio::spawn(async move {
            let mut buf = vec![0u8; CHUNK];
            loop {
                let n = match stdin.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                match credit.acquire_many(n as u32).await {
                    Ok(permit) => permit.forget(),
                    Err(_) => return,
                }
                if tx
                    .send(frame(EXEC_KIND, &stream, &tagged(T_STDIN, &buf[..n])))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let _ = tx.send(frame(EXEC_KIND, &stream, &[T_EOF])).await;
        })
    };
    struct Abort(tokio::task::JoinHandle<()>);
    impl Drop for Abort {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _stdin_task = Abort(stdin_task);

    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let (err_tx, err_rx) = mpsc::unbounded_channel();
    let out_writer = tokio::spawn(write_out(stdout, out_rx, link.tx.clone(), stream.clone()));
    let err_writer = tokio::spawn(write_out(stderr, err_rx, link.tx.clone(), stream.clone()));
    let (mut out_tx, mut err_tx) = (Some(out_tx), Some(err_tx));

    let open_deadline = tokio::time::Instant::now() + OPEN_TIMEOUT;
    let mut started = false;
    let mut last_echo = tokio::time::Instant::now();
    let mut echo_seen = false;
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.tick().await;
    let code = loop {
        tokio::select! {
            bytes = link.rx.recv() => {
                let Some(bytes) = bytes else {
                    return Err(ExecError::Link(format!("lost the connection to {}", target.device)));
                };
                let Ok((header, payload)) = decode_device_frame(&bytes) else { continue };
                if header.k == RELAY_KIND {
                    return Err(relay_failure(&payload, &target.device));
                }
                if header.k == ECHO_KIND {
                    last_echo = tokio::time::Instant::now();
                    echo_seen = true;
                    continue;
                }
                if header.k != EXEC_KIND || header.s != stream {
                    continue;
                }
                last_echo = tokio::time::Instant::now();
                let Some((&tag, data)) = payload.split_first() else { continue };
                match tag {
                    T_STARTED => started = true,
                    T_REFUSED => {
                        let error = serde_json::from_slice::<serde_json::Value>(data)
                            .ok()
                            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned))
                            .unwrap_or_else(|| "refused".into());
                        return Err(ExecError::Refused(error));
                    }
                    T_STDOUT => if let Some(tx) = &out_tx { let _ = tx.send(data.to_vec()); },
                    T_STDERR => if let Some(tx) = &err_tx { let _ = tx.send(data.to_vec()); },
                    T_CREDIT => {
                        if let Ok(n) = <[u8; 4]>::try_from(data) {
                            let n = u32::from_le_bytes(n) as usize;
                            if credit.available_permits() + n <= WINDOW {
                                credit.add_permits(n);
                            }
                        }
                    }
                    T_EXIT => {
                        break serde_json::from_slice::<serde_json::Value>(data)
                            .ok()
                            .and_then(|v| v.get("code").and_then(|c| c.as_i64()))
                            .map(|c| c as i32)
                            .unwrap_or(1);
                    }
                    _ => {}
                }
            }
            _ = ping.tick() => {
                if link.tx.send(frame(ECHO_KIND, ECHO_KIND, &[])).await.is_err() {
                    return Err(ExecError::Link("connection lost".into()));
                }
            }
            _ = tokio::time::sleep_until(last_echo + ECHO_DEADLINE), if echo_seen => {
                return Err(ExecError::Link(format!("{} stopped answering", target.device)));
            }
            _ = tokio::time::sleep_until(open_deadline), if !started => {
                return Err(ExecError::Link(format!("timed out waiting for {}", target.device)));
            }
        }
    };
    // All output arrived before `exit`; let the writers finish it.
    out_tx.take();
    err_tx.take();
    let _ = out_writer.await;
    let _ = err_writer.await;
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_errors_map_to_exit_codes() {
        assert_eq!(ExecError::Link("x".into()).exit_code(), 255);
        assert_eq!(ExecError::Refused("x".into()).exit_code(), 77);
    }

    #[test]
    fn dial_debug_hides_token() {
        let dial = Dial {
            edge_url: "https://e".into(),
            device: "d".into(),
            token: "secret-token".into(),
            direct_name: None,
        };
        assert!(!format!("{dial:?}").contains("secret-token"));
    }
}
