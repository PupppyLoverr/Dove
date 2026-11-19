//! A machine you already own, reached over your own `ssh`: cloud mode with
//! bring-your-own-key, tier one. No provider, no account, no relay of ours
//! (docs/cloud-byok.md has the trust boundary this derives from).
//!
//! One `ssh -N -L 127.0.0.1:<local>:127.0.0.1:<engine>` per machine stands
//! between this app and the engine running there (`keplorayx-engine
//! headless`), started over the same channel when nothing answers; the
//! engine's per-start bearer is read over it (`cat <data>/ipc-token`) and
//! kept in memory only, and the app's ordinary IPC client dials the
//! forwarded port with it. Because the far side is a whole engine, threads,
//! streaming, panels and approvals are the ones this app already speaks.
//!
//! Two trust anchors, neither of them ours to mint:
//!
//! - **The host key.** Offered on the first connect as a fingerprint for the
//!   person to accept, then pinned in this app's own `known_hosts` for that
//!   machine. A later mismatch is refused before anything is sent, loudly.
//! - **The engine's bearer.** Minted by the remote engine on every start,
//!   read through the encrypted channel, never written to this machine.
//!
//! Nothing secret is kept here: `machines.json` holds an address, a port, a
//! path to the person's own key (or none, for the ssh agent) and the public
//! host key. The provider keys a remote engine needs are written to *that*
//! machine (0600) over this channel and not kept here at all.
//!
//!   kx.remote                          { hosts, active } (shared)
//!   kx.remote.add({ target, … })      → { id, fingerprint } (the ask)
//!   kx.remote.trust({ id, accept })    ← the answer to it
//!   kx.remote.connect(id), .disconnect(), .remove(id), .status()

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::{Value as Json, json};
use zgx_script::{ScriptHost, Value};

/// The remote engine's own IPC port, out of the way of every local one
/// (27654 is the shipping engine's, 27731 Keplorayx's, 27871 staging).
const DEFAULT_ENGINE_PORT: u16 = 27741;
/// Its data, where an engine started over ssh keeps its threads.
const DEFAULT_DATA_DIR: &str = "~/.keplorayx";
/// How long `ssh` may take to answer before it is called unreachable.
const CONNECT_TIMEOUT: u16 = 8;
/// How long a forward may take to carry bytes.
const FORWARD_WAIT: Duration = Duration::from_secs(15);
/// How long an engine started over ssh may take to write its bearer.
const START_WAIT: Duration = Duration::from_secs(45);

/// Why a machine isn't usable, in the words the person sees, with the state
/// its row shows.
#[derive(Debug, Clone)]
pub enum Fault {
    /// Nothing answered on that name and port.
    Unreachable(String),
    /// The host refused this computer's key.
    RefusedKey,
    /// The host's key isn't the one accepted before.
    HostKeyChanged(String),
    /// The person hasn't accepted this machine's host key yet.
    Untrusted(String),
    /// `ssh` isn't on this computer.
    NoSsh,
    /// The machine answered, but has no engine binary.
    NoEngine(String),
    /// The engine is there and isn't running, or wrote no bearer.
    Engine(String),
    /// The forward didn't come up.
    NoForward(String),
    Other(String),
}

impl Fault {
    fn state(&self) -> &'static str {
        match self {
            Self::Unreachable(_) => "unreachable",
            Self::RefusedKey => "refused_key",
            Self::HostKeyChanged(_) => "host_key_changed",
            Self::Untrusted(_) => "untrusted",
            Self::NoSsh => "no_ssh",
            Self::NoEngine(_) => "no_engine",
            Self::Engine(_) => "engine_stopped",
            Self::NoForward(_) => "no_forward",
            Self::Other(_) => "failed",
        }
    }

    fn say(&self) -> String {
        match self {
            Self::Unreachable(why) => why.clone(),
            Self::RefusedKey => "The machine refused this computer's key. Add its public key to \
                                 the machine's ~/.ssh/authorized_keys, or load your key in ssh-agent."
                .into(),
            Self::HostKeyChanged(fp) => format!(
                "This machine's host key is not the one you accepted ({fp}). Nothing was sent. \
                 If the machine was rebuilt, remove it here and add it again — but if it wasn't, \
                 someone may be impersonating it."
            ),
            Self::Untrusted(fp) => format!("Accept this machine's host key first: {fp}."),
            Self::NoSsh => "`ssh` isn't on this computer. Windows' optional OpenSSH Client, or \
                            Git for Windows, provides it."
                .into(),
            Self::NoEngine(where_to_get) => format!(
                "That machine has no `keplorayx-engine` on its PATH ({where_to_get}). Install it \
                 there and connect again."
            ),
            Self::Engine(why) => why.clone(),
            Self::NoForward(why) => format!("The ssh forward didn't come up: {why}"),
            Self::Other(why) => why.clone(),
        }
    }
}

/// One machine the person added.
#[derive(Debug, Clone)]
pub struct Host {
    pub id: String,
    pub name: String,
    /// `user@host` as typed.
    pub target: String,
    pub port: u16,
    /// A private key on this computer, or none for the agent and ~/.ssh.
    pub identity: Option<String>,
    /// The engine's data on *that* machine (its threads live here).
    pub data_dir: String,
    /// The engine's IPC port on that machine.
    pub engine_port: u16,
    /// The host key accepted for it, as the line trusted in our known_hosts.
    pub known_host: Option<String>,
    /// That line's SHA256 fingerprint, for the person to recognise.
    pub fingerprint: Option<String>,
    /// Whether this is a loopback engine on this same computer, standing in
    /// for a remote one (the development seam, `KX_MACHINE_LOOPBACK=1`).
    pub local: bool,
}

impl Host {
    fn address(&self) -> String {
        format!("{}:{}", self.target, self.port)
    }

    fn trusted(&self) -> bool {
        self.local || (self.known_host.is_some() && self.fingerprint.is_some())
    }

    /// Where that machine's engine keeps its threads, expanded here only for
    /// the record the person reads.
    fn read_dir(&self) -> &str {
        &self.data_dir
    }

    fn to_json(&self) -> Json {
        json!({
            "id": self.id,
            "name": self.name,
            "target": self.target,
            "port": self.port,
            "identity": self.identity,
            "data_dir": self.data_dir,
            "engine_port": self.engine_port,
            "known_host": self.known_host,
            "fingerprint": self.fingerprint,
            "here": self.local,
        })
    }

    fn from_json(v: &Json) -> Option<Self> {
        Some(Self {
            id: v["id"].as_str()?.to_owned(),
            name: v["name"].as_str()?.to_owned(),
            target: v["target"].as_str()?.to_owned(),
            port: v["port"].as_u64().unwrap_or(22) as u16,
            identity: v["identity"].as_str().map(str::to_owned),
            data_dir: v["data_dir"].as_str().unwrap_or(DEFAULT_DATA_DIR).to_owned(),
            engine_port: v["engine_port"].as_u64().unwrap_or(DEFAULT_ENGINE_PORT as u64) as u16,
            known_host: v["known_host"].as_str().map(str::to_owned),
            fingerprint: v["fingerprint"].as_str().map(str::to_owned),
            local: v["here"].as_bool().unwrap_or(false),
        })
    }
}

fn dir() -> PathBuf {
    crate::kx_account::data_dir()
}

fn machines_file() -> PathBuf {
    dir().join("machines.json")
}

/// Where this app keeps the host key it pinned for one machine: its own
/// file, so a changed key is not merged with anything else's.
fn known_hosts_file(id: &str) -> PathBuf {
    dir().join("machines").join(format!("{id}.known_hosts"))
}

fn hosts() -> Vec<Host> {
    let kept: Vec<Host> = std::fs::read_to_string(machines_file())
        .ok()
        .and_then(|s| serde_json::from_str::<Json>(&s).ok())
        .map(|j| j["hosts"].as_array().cloned().unwrap_or_default())
        .unwrap_or_default()
        .iter()
        .filter_map(Host::from_json)
        .collect();
    match loopback_host() {
        Some(stand_in) if !kept.iter().any(|h| h.local) => {
            let mut all = kept;
            all.push(stand_in);
            all
        }
        _ => kept,
    }
}

/// A loopback engine standing in for a remote one: only in a development
/// build, and only when asked (`KX_MACHINE_LOOPBACK=1`), because the whole
/// point of the feature is a machine that is not this one.
fn loopback_host() -> Option<Host> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let asked = std::env::var_os("KX_MACHINE_LOOPBACK").is_some_and(|v| v != "0");
    asked.then(|| Host {
        id: "loopback".into(),
        name: "This PC (loopback engine)".into(),
        target: "loopback".into(),
        port: 22,
        identity: None,
        data_dir: std::env::var("KX_MACHINE_LOOPBACK_DATA").unwrap_or_else(|_| {
            dir().parent().map(|p| p.join("keplorayx-loopback").display().to_string()).unwrap_or_else(|| "keplorayx-loopback".into())
        }),
        engine_port: std::env::var("KX_MACHINE_LOOPBACK_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(27761),
        known_host: Some("loopback".into()),
        fingerprint: Some("loopback".into()),
        local: true,
    })
}

fn host(id: &str) -> Option<Host> {
    hosts().into_iter().find(|h| h.id == id)
}

fn write_hosts(all: &[Host]) {
    let path = machines_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let hosts: Vec<Json> = all.iter().filter(|h| !h.local).map(Host::to_json).collect();
    let active = active().filter(|id| all.iter().any(|h| !h.local && &h.id == id));
    let kept = json!({ "hosts": hosts, "active": active });
    let _ = std::fs::write(path, kept.to_string());
}

/// The machine the app's engine is attached to, across launches.
fn active() -> Option<String> {
    std::fs::read_to_string(machines_file())
        .ok()
        .and_then(|s| serde_json::from_str::<Json>(&s).ok())
        .and_then(|j| j["active"].as_str().map(str::to_owned))
}

fn set_active(id: Option<&str>) {
    let all = hosts();
    let mut kept: Vec<Host> = all.into_iter().filter(|h| !h.local).collect();
    if kept.is_empty() {
        kept = Vec::new();
    }
    let path = machines_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let hosts: Vec<Json> = kept.iter().map(Host::to_json).collect();
    let _ = std::fs::write(path, json!({ "hosts": hosts, "active": id }).to_string());
}

// ── runtime state ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct State {
    state: &'static str,
    detail: String,
    version: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self { state: "stopped", detail: "Not connected.".into(), version: None }
    }
}

fn states() -> &'static Mutex<BTreeMap<String, State>> {
    static STATES: OnceLock<Mutex<BTreeMap<String, State>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn set_state(id: &str, state: &'static str, detail: impl Into<String>) {
    let mut all = states().lock().unwrap();
    let held = all.entry(id.to_owned()).or_default();
    held.state = state;
    held.detail = detail.into();
}

fn set_version(id: &str, version: Option<String>) {
    states().lock().unwrap().entry(id.to_owned()).or_default().version = version;
}

fn state_of(id: &str) -> State {
    states().lock().unwrap().get(id).cloned().unwrap_or_default()
}

/// The token the app's engine dials with, kept in memory for the life of the
/// link and replaced whenever the link comes back.
type Token = crate::engine::rpc::Token;

/// A live link: the forward, the bearer, and what stops it.
struct Live {
    host: String,
    local_port: u16,
    token: Token,
    stop: Arc<AtomicBool>,
    since: Instant,
}

fn live() -> &'static Mutex<Option<Live>> {
    static LIVE: OnceLock<Mutex<Option<Live>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(None))
}

fn live_host() -> Option<String> {
    live().lock().unwrap().as_ref().map(|l| l.host.clone())
}

// ── the channel ────────────────────────────────────────────────────────────

/// What one machine says about its engine.
struct Facts {
    /// The engine's binary there.
    bin: String,
    /// Its build, when it says (`--build-id`).
    build: Option<String>,
}

/// The far side of a link: a machine over ssh, or (the development seam) an
/// engine on this same computer. Both answer the same five questions, so the
/// link below is written once.
enum Channel {
    Ssh(Ssh),
    Local(Local),
}

struct Ssh {
    host: Host,
}

struct Local {
    host: Host,
    /// The engine this link started, if it started one.
    started: Mutex<Option<Child>>,
}

impl Channel {
    fn of(host: &Host) -> Self {
        if host.local {
            Self::Local(Local { host: host.clone(), started: Mutex::new(None) })
        } else {
            Self::Ssh(Ssh { host: host.clone() })
        }
    }

    fn reachable(&self) -> Result<(), Fault> {
        match self {
            Self::Ssh(s) => s
                .run("true", Duration::from_secs(CONNECT_TIMEOUT as u64 + 4))
                .map(|_| ())
                .map_err(|why| s.fault(why)),
            Self::Local(_) => Ok(()),
        }
    }

    /// Whether that machine can run an engine, and which build it is.
    fn engine(&self) -> Result<Facts, Fault> {
        match self {
            Self::Ssh(s) => s.engine(),
            Self::Local(l) => l.engine(),
        }
    }

    /// The bearer the engine there wrote, if it has one.
    fn token(&self) -> Result<Option<String>, Fault> {
        match self {
            Self::Ssh(s) => s.token(),
            Self::Local(l) => l.token(),
        }
    }

    /// Start the engine there, detached from this channel.
    fn start_engine(&self, bin: &str) -> Result<(), Fault> {
        match self {
            Self::Ssh(s) => s.start_engine(bin),
            Self::Local(l) => l.start_engine(bin),
        }
    }

    /// A loopback port on this computer that reaches the engine there.
    fn forward(&self, local: u16) -> Result<Forward, Fault> {
        match self {
            Self::Ssh(s) => s.forward(local),
            Self::Local(l) => l.forward(local),
        }
    }
}

impl Ssh {
    /// `ssh` with this machine's own options: batch mode (never a prompt this
    /// app can't answer: a passphrase belongs in ssh-agent), the pinned host
    /// key file, and a connect timeout.
    fn command(&self) -> Result<Command, Fault> {
        let exe = ssh_exe().ok_or(Fault::NoSsh)?;
        let mut c = Command::new(exe);
        c.arg("-p").arg(self.host.port.to_string());
        c.arg("-o").arg("BatchMode=yes");
        c.arg("-o").arg(format!("ConnectTimeout={CONNECT_TIMEOUT}"));
        c.arg("-o").arg("StrictHostKeyChecking=yes");
        c.arg("-o").arg(format!("UserKnownHostsFile={}", known_hosts_file(&self.host.id).display()));
        c.arg("-o").arg("UpdateHostKeys=no");
        if let Some(identity) = &self.host.identity {
            c.arg("-i").arg(identity);
            c.arg("-o").arg("IdentitiesOnly=yes");
        }
        hidden(&mut c);
        c.arg(&self.host.target);
        Ok(c)
    }

    /// A shell command run there, its stdout, or what ssh said went wrong.
    fn run(&self, script: &str, wait: Duration) -> Result<String, String> {
        let mut command = self.command().map_err(|f| f.say())?;
        command.arg(script).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let child = command.spawn().map_err(|e| format!("ssh couldn't be started: {e}"))?;
        let (out, err, code) = wait_on(child, wait)?;
        if code == Some(0) {
            return Ok(out);
        }
        Err(if err.trim().is_empty() { format!("ssh stopped with {code:?}") } else { err.trim().to_owned() })
    }

    fn engine(&self) -> Result<Facts, Fault> {
        let script = "b=$(command -v keplorayx-engine || command -v zeron-engine) || exit 127; \
                      printf '%s\\n' \"$b\"; \"$b\" --build-id 2>/dev/null || true";
        match self.run(script, Duration::from_secs(CONNECT_TIMEOUT as u64 + 6)) {
            Ok(out) => parse_facts(&out).ok_or(Fault::NoEngine(format!("{} asked, nothing answered", self.host.target))),
            Err(why) if is_missing_engine(&why) => Err(Fault::NoEngine(why)),
            Err(why) => Err(self.fault(why)),
        }
    }

    fn token(&self) -> Result<Option<String>, Fault> {
        let file = format!("{}/ipc-token", shell_quote(&self.host.data_dir));
        let out = self.run(&format!("cat {file} 2>/dev/null || true"), Duration::from_secs(20)).map_err(|why| self.fault(why))?;
        Ok(clean_token(&out))
    }

    fn start_engine(&self, bin: &str) -> Result<(), Fault> {
        let start = start_command(bin, &self.host.data_dir, self.host.engine_port);
        self.run(&start, Duration::from_secs(CONNECT_TIMEOUT as u64 + 6)).map(|_| ()).map_err(|why| self.fault(why))
    }

    fn forward(&self, local: u16) -> Result<Forward, Fault> {
        let mut command = self.command()?;
        command
            .arg("-N")
            .arg("-o")
            .arg("ExitOnForwardFailure=yes")
            .arg("-o")
            .arg("ServerAliveInterval=10")
            .arg("-o")
            .arg("ServerAliveCountMax=3")
            .arg("-L")
            .arg(format!("127.0.0.1:{local}:127.0.0.1:{}", self.host.engine_port))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let child = command.spawn().map_err(|e| Fault::NoForward(format!("ssh couldn't be started: {e}")))?;
        Ok(Forward::Ssh(child))
    }

    /// What ssh's own words mean, as the fault the person reads.
    fn fault(&self, why: String) -> Fault {
        let lower = why.to_ascii_lowercase();
        if lower.contains("host key verification failed") {
            return match &self.host.fingerprint {
                Some(fp) => Fault::HostKeyChanged(fp.clone()),
                None => Fault::Untrusted("no host key accepted yet".into()),
            };
        }
        if lower.contains("permission denied") || lower.contains("no such identity") || lower.contains("too many authentication failures") {
            return Fault::RefusedKey;
        }
        if lower.contains("could not resolve hostname") || lower.contains("name or service not known") {
            return Fault::Unreachable(format!("`{}` isn't a name this computer can find.", self.host.target));
        }
        if lower.contains("connection refused") {
            return Fault::Unreachable(format!("Nothing answered on {}:{}.", self.host.target, self.host.port));
        }
        if lower.contains("timed out") || lower.contains("timeout") || lower.contains("operation timed out") {
            return Fault::Unreachable(format!("{}:{} didn't answer within {CONNECT_TIMEOUT} s.", self.host.target, self.host.port));
        }
        Fault::Other(why)
    }
}

impl Local {
    fn engine(&self) -> Result<Facts, Fault> {
        let bin = engine_beside_app().ok_or_else(|| Fault::NoEngine("this checkout's target/debug".into()))?;
        let build = crate::engine::manager::build_of(&bin);
        Ok(Facts { bin: bin.display().to_string(), build })
    }

    fn token(&self) -> Result<Option<String>, Fault> {
        Ok(clean_token(&std::fs::read_to_string(Path::new(&self.host.data_dir).join("ipc-token")).unwrap_or_default()))
    }

    fn start_engine(&self, bin: &str) -> Result<(), Fault> {
        let data = PathBuf::from(&self.host.data_dir);
        std::fs::create_dir_all(&data).map_err(|e| Fault::Engine(format!("Couldn't make {}: {e}", data.display())))?;
        let mut command = Command::new(bin);
        command
            .env("ZERON_DATA_DIR", &data)
            .env("ZERON_IPC_PORT", self.host.engine_port.to_string())
            .env("ZERON_WORKTREES_DIR", data.join("worktrees"))
            .env_remove("ZERON_ORIGIN")
            .env_remove("ZERON_IPC_TOKEN")
            .env_remove("ZERON_IPC_TOKEN_FILE")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hidden(&mut command);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command.spawn().map_err(|e| Fault::Engine(format!("The engine here didn't start: {e}")))?;
        *self.started.lock().unwrap() = Some(child);
        Ok(())
    }

    fn forward(&self, local: u16) -> Result<Forward, Fault> {
        Forward::relay(local, self.host.engine_port)
    }
}

/// A forward, as what stops it.
enum Forward {
    Ssh(Child),
    Relay(Relay),
}

struct Relay {
    stop: Arc<AtomicBool>,
    streams: Arc<Mutex<Vec<TcpStream>>>,
    /// The accepting thread: ended and joined when the link is taken down,
    /// so the port is free before anything rebinds it.
    accepting: Option<std::thread::JoinHandle<()>>,
}

impl Relay {
    /// A loopback port on this computer that carries bytes to `remote` here
    /// (the loopback stand-in for an ssh `-L`).
    fn start(local: u16, remote: u16, stop: Arc<AtomicBool>) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", local))?;
        listener.set_nonblocking(true)?;
        let streams: Arc<Mutex<Vec<TcpStream>>> = Arc::default();
        let held = streams.clone();
        let running = stop.clone();
        let accepting = std::thread::Builder::new().name("kx-forward".into()).spawn(move || {
            while !running.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((client, _)) => {
                        let Ok(peer) = TcpStream::connect(("127.0.0.1", remote)) else { continue };
                        // (Windows hands on the listener's non-blocking mode
                        // with an accepted socket: a forwarding socket that
                        // reads instead of waiting would drop the link.)
                        let _ = client.set_nonblocking(false);
                        let _ = client.set_nodelay(true);
                        let _ = peer.set_nodelay(true);
                        if let (Ok(a), Ok(b)) = (client.try_clone(), peer.try_clone()) {
                            held.lock().unwrap().extend([a, b]);
                        }
                        let (from, to) = (client.try_clone().ok(), peer.try_clone().ok());
                        std::thread::spawn(move || carry(from, Some(peer)));
                        std::thread::spawn(move || carry(to, Some(client)));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
        })?;
        Ok(Self { stop, streams, accepting: Some(accepting) })
    }

    /// Takes the link down: no more accepts, the port free, every carried
    /// connection closed.
    fn kill(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(accepting) = self.accepting.take() {
            let _ = accepting.join();
        }
        for stream in self.streams.lock().unwrap().drain(..) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// Bytes one way until either end is gone.
fn carry(mut from: Option<TcpStream>, mut to: Option<TcpStream>) {
    let (Some(mut from), Some(mut to)) = (from.take(), to.take()) else { return };
    let mut buf = [0u8; 32 * 1024];
    loop {
        match from.read(&mut buf) {
            Ok(0) => break,
            // (Nothing yet: wait rather than treat it as the end.)
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break,
            Ok(n) => {
                if to.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
    let _ = to.shutdown(std::net::Shutdown::Both);
}

impl Forward {
    /// A loopback port of our own that carries bytes to `remote` here: the
    /// stand-in for an ssh `-L`, used by the loopback machine.
    fn relay(local: u16, remote: u16) -> Result<Self, Fault> {
        let stop = Arc::new(AtomicBool::new(false));
        let relay = Relay::start(local, remote, stop).map_err(|e| Fault::NoForward(e.to_string()))?;
        Ok(Self::Relay(relay))
    }

    fn kill(&mut self) {
        match self {
            Self::Ssh(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            Self::Relay(r) => r.kill(),
        }
    }

    fn died(&mut self) -> bool {
        match self {
            Self::Ssh(child) => matches!(child.try_wait(), Ok(Some(_))),
            Self::Relay(r) => r.stop.load(Ordering::Relaxed),
        }
    }
}

// ── linking ────────────────────────────────────────────────────────────────

/// What a link being up means for the app.
struct Up {
    forward: Forward,
    token: String,
    local_port: u16,
    version: Option<String>,
}

/// Everything up to a forward that carries bytes to the engine there: the
/// machine answers, its engine is there, running and has written its bearer,
/// and a loopback port on this computer reaches it.
fn link_up(host: &Host, local_port: u16) -> Result<Up, Fault> {
    let channel = Channel::of(host);
    channel.reachable()?;
    let facts = channel.engine()?;
    set_version(&host.id, facts.build.clone());
    let mut token = channel.token()?;
    if token.is_none() {
        set_state(&host.id, "starting", "Starting the engine there…");
        channel.start_engine(&facts.bin)?;
        let until = Instant::now() + START_WAIT;
        while token.is_none() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(400));
            token = channel.token().unwrap_or(None);
        }
        if token.is_none() {
            return Err(Fault::Engine(format!(
                "The engine on {} was started but wrote no bearer in {} s. Its log is {}/logs/headless.log.",
                host.name,
                START_WAIT.as_secs(),
                host.data_dir
            )));
        }
    }
    let mut forward = channel.forward(local_port)?;
    let until = Instant::now() + FORWARD_WAIT;
    loop {
        if TcpStream::connect_timeout(&([127, 0, 0, 1], local_port).into(), Duration::from_millis(300)).is_ok() {
            break;
        }
        if forward.died() {
            let mut forward = forward;
            forward.kill();
            return Err(Fault::NoForward("ssh stopped with the forward refused".into()));
        }
        if Instant::now() >= until {
            forward.kill();
            return Err(Fault::NoForward(format!("nothing answered on 127.0.0.1:{local_port} in {} s", FORWARD_WAIT.as_secs())));
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    Ok(Up { forward, token: token.unwrap_or_default(), local_port, version: facts.build })
}

/// Connects `host` (off this thread), then hands the app's engine the
/// forwarded port and the bearer.
async fn connect(id: String) -> Result<Json, String> {
    let Some(host) = host(&id) else {
        return Err("That machine isn't here.".into());
    };
    if !host.trusted() {
        let fp = host.fingerprint.clone().unwrap_or_else(|| "unknown".into());
        let fault = Fault::Untrusted(fp);
        set_state(&id, fault.state(), fault.say());
        return Err(fault.say());
    }
    disconnect(false);
    set_state(&id, "connecting", "Connecting…");

    let (host_for_thread, id_for_thread) = (host.clone(), id.clone());
    let outcome = crate::kx::off_thread(move || {
        let port = free_port().ok_or(Fault::NoForward("no loopback port free".into()))?;
        link_up(&host_for_thread, port)
    })
    .await
    .ok_or_else(|| "The link didn't start.".to_owned())?;

    match outcome {
        Ok(up) => {
            let token = Token::default();
            token.set(Some(up.token.clone()));
            let stop = Arc::new(AtomicBool::new(false));
            *live().lock().unwrap() = Some(Live { host: id.clone(), local_port: up.local_port, token: token.clone(), stop: stop.clone(), since: Instant::now() });
            set_state(&id, "live", format!("Connected to {}.", host.name));
            // The app's own engine connection now points at that machine.
            crate::engine::get().use_machine(host.name.clone(), up.local_port, token.clone());
            let (host_for_watch, ticker) = (host.clone(), token);
            std::thread::Builder::new()
                .name("kx-link".into())
                .spawn(move || watch(host_for_watch, id_for_thread, up.forward, up.local_port, ticker, stop))
                .map_err(|e| e.to_string())?;
            publish();
            Ok(json!({ "id": id, "state": "live", "port": up.local_port, "version": up.version }))
        }
        Err(fault) => {
            set_state(&id, fault.state(), fault.say());
            publish();
            Err(fault.say())
        }
    }
}

/// Keeps `host` linked: when the forward dies the engine there is still
/// running its threads, so the link is put back and the app dials again on
/// the same port — the session was never ours to lose.
fn watch(host: Host, id: String, mut forward: Forward, local_port: u16, token: Token, stop: Arc<AtomicBool>) {
    let mut backoff = Duration::from_millis(500);
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(400));
        if stop.load(Ordering::Relaxed) {
            return;
        }
        if !forward.died() {
            continue;
        }
        forward.kill();
        set_state(&id, "reconnecting", "The link dropped. Reconnecting…");
        publish();
        let mut back = None;
        for _ in 0..3 {
            match relink(&host, local_port) {
                Ok((fresh, bearer)) => {
                    token.set(Some(bearer));
                    back = Some(fresh);
                    break;
                }
                Err(why) => {
                    set_state(&id, "reconnecting", format!("Reconnecting… {}", why.say()));
                    publish();
                    std::thread::sleep(backoff);
                    backoff = (backoff * 2).min(Duration::from_secs(5));
                }
            }
            if stop.load(Ordering::Relaxed) {
                return;
            }
        }
        match back {
            Some(fresh) => {
                forward = fresh;
                backoff = Duration::from_millis(500);
                set_state(&id, "live", format!("Connected to {}.", host.name));
                publish();
            }
            None => {
                // Three tries on the same port: say so rather than pretend,
                // then keep trying slowly, because the machine may only be
                // asleep.
                set_state(&id, "failed", "The link dropped and didn't come back. Connect again when the machine is reachable.");
                publish();
                let until = Instant::now() + Duration::from_secs(15);
                while Instant::now() < until && !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
    }
}

/// The forward again, on the same loopback port, with the bearer as it is
/// now (the engine there may have restarted and minted a new one).
fn relink(host: &Host, local_port: u16) -> Result<(Forward, String), Fault> {
    let channel = Channel::of(host);
    channel.reachable()?;
    let bearer = channel.token()?.ok_or_else(|| Fault::Engine(format!("The engine on {} has no bearer: it isn't running.", host.name)))?;
    let forward = channel.forward(local_port)?;
    let until = Instant::now() + FORWARD_WAIT;
    while Instant::now() < until {
        if TcpStream::connect_timeout(&([127, 0, 0, 1], local_port).into(), Duration::from_millis(300)).is_ok() {
            return Ok((forward, bearer));
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    Err(Fault::NoForward(format!("nothing answered on 127.0.0.1:{local_port} in {} s", FORWARD_WAIT.as_secs())))
}

fn disconnect(publish_after: bool) {
    let taken = live().lock().unwrap().take();
    if let Some(live) = taken {
        live.stop.store(true, Ordering::Relaxed);
        set_state(&live.host, "stopped", "Disconnected.");
        set_active(None);
        crate::engine::get().use_local();
    }
    if publish_after {
        publish();
    }
}

// ── ssh's own bits ─────────────────────────────────────────────────────────

/// `ssh` (and its siblings) as this computer has them.
fn ssh_exe() -> Option<PathBuf> {
    if cfg!(windows) {
        if let Some(root) = std::env::var_os("SystemRoot") {
            let at = PathBuf::from(root).join("System32").join("OpenSSH").join("ssh.exe");
            if at.is_file() {
                return Some(at);
            }
        }
    }
    which("ssh")
}

fn sibling(tool: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{tool}.exe") } else { tool.to_owned() };
    if cfg!(windows) {
        if let Some(root) = std::env::var_os("SystemRoot") {
            let at = PathBuf::from(root).join("System32").join("OpenSSH").join(&exe);
            if at.is_file() {
                return Some(at);
            }
        }
    }
    which(&exe)
}

fn which(exe: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).map(|dir| dir.join(exe)).find(|at| at.is_file())
}

fn hidden(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
}

fn free_port() -> Option<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    Some(port)
}

fn clean_token(raw: &str) -> Option<String> {
    let token = raw.trim();
    (!token.is_empty() && !token.contains('\n') && token.len() <= 512).then(|| token.to_owned())
}

/// The first line is the engine's path, the rest its build (when it says).
fn parse_facts(out: &str) -> Option<Facts> {
    let mut lines = out.lines().map(str::trim).filter(|l| !l.is_empty());
    let bin = lines.next()?.to_owned();
    if bin.contains(' ') || bin.len() > 512 {
        return None;
    }
    let build = lines.next().map(str::to_owned).filter(|b| !b.contains(' '));
    Some(Facts { bin, build })
}

fn is_missing_engine(why: &str) -> bool {
    why.contains("exit 127") || why.contains("127") && why.contains("stopped")
}

/// A path for a POSIX shell: single-quoted, its own quotes escaped.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// How the engine there is started, from a shell on that machine: its data
/// and port in its environment, its output in its own log, and `setsid`
/// where there is one so it outlives the ssh call.
fn start_command(bin: &str, data_dir: &str, port: u16) -> String {
    let data = shell_quote(data_dir);
    let log = shell_quote(&format!("{data_dir}/logs/headless.log"));
    let bin = shell_quote(bin);
    format!(
        "mkdir -p {data} {logdir} && cd {data} && \
         if command -v setsid >/dev/null 2>&1; then \
           setsid nohup env ZERON_DATA_DIR={data} ZERON_IPC_PORT={port} {bin} headless >> {log} 2>&1 < /dev/null & \
         else \
           nohup env ZERON_DATA_DIR={data} ZERON_IPC_PORT={port} {bin} headless >> {log} 2>&1 < /dev/null & \
         fi; echo started",
        data = data,
        logdir = shell_quote(&format!("{data_dir}/logs")),
        log = log,
        port = port,
        bin = bin,
    )
}

/// The engine this app ships beside itself (`ZERON_ZGX_ENGINE_BIN`, as
/// manager.rs reads it, or the one this checkout built).
fn engine_beside_app() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("ZERON_ZGX_ENGINE_BIN") {
        let named = PathBuf::from(named);
        if named.is_file() {
            return Some(named);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let beside = exe.parent()?.join(format!("keplorayx-engine{}", std::env::consts::EXE_SUFFIX));
    if beside.is_file() {
        return Some(beside);
    }
    let built = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug")
        .join(format!("keplorayx-engine{}", std::env::consts::EXE_SUFFIX));
    built.is_file().then_some(built)
}

/// Waits on a child for its stdout, stderr and exit code (its pipes read on
/// threads, so neither can fill and stop it).
fn wait_on(mut child: Child, wait: Duration) -> Result<(String, String, Option<i32>), String> {
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let read = |from: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut said = String::new();
            if let Some(mut from) = from {
                let _ = from.read_to_string(&mut said);
            }
            said
        })
    };
    let (out_thread, err_thread) = (read(out.take().map(|o| Box::new(o) as Box<dyn Read + Send>)), read(err.take().map(|e| Box::new(e) as Box<dyn Read + Send>)));
    let until = Instant::now() + wait;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = out_thread.join().unwrap_or_default();
                let err = err_thread.join().unwrap_or_default();
                return Ok((out, err, status.code()));
            }
            Ok(None) if Instant::now() >= until => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("nothing answered in {} s", wait.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(error) => return Err(error.to_string()),
        }
    }
}

// ── adding a machine ───────────────────────────────────────────────────────

/// What `ssh-keyscan` says a machine's host key is: the line to pin and the
/// fingerprint to show.
fn scan(target: &str, port: u16) -> Result<(String, String), Fault> {
    let exe = sibling("ssh-keyscan").ok_or(Fault::NoSsh)?;
    let mut command = Command::new(exe);
    command.arg("-p").arg(port.to_string()).arg("-T").arg(CONNECT_TIMEOUT.to_string()).arg("-t").arg("ed25519,ecdsa,rsa").arg(target);
    hidden(&mut command);
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let child = command.spawn().map_err(|e| Fault::Unreachable(format!("ssh-keyscan couldn't be started: {e}")))?;
    let (out, _, _) = wait_on(child, Duration::from_secs(CONNECT_TIMEOUT as u64 + 6)).map_err(Fault::Unreachable)?;
    let line = out
        .lines()
        .map(str::trim)
        .find(|l| !l.starts_with('#') && (l.contains("ssh-ed25519") || l.contains("ecdsa-sha2") || l.contains("ssh-rsa")))
        .ok_or_else(|| Fault::Unreachable(format!("Nothing answered on {target}:{port}. Is sshd running there, and is the port open?")))?;
    let fingerprint = fingerprint_of(&target.to_owned(), port, line)?;
    Ok((line.to_owned(), fingerprint))
}

/// The fingerprint of one host key line, in ssh's own words.
fn fingerprint_of(target: &str, port: u16, line: &str) -> Result<String, Fault> {
    let keygen = sibling("ssh-keygen").ok_or(Fault::NoSsh)?;
    let name = if port == 22 { target.to_owned() } else { format!("[{target}]:{port}") };
    let path = std::env::temp_dir().join(format!("kx-key-{}-{port}", std::process::id()));
    let _ = std::fs::create_dir_all(&path);
    let file = path.join("known_hosts");
    std::fs::write(&file, format!("{line}\n")).map_err(|e| Fault::Other(e.to_string()))?;
    let mut command = Command::new(keygen);
    command.arg("-l").arg("-F").arg(&name).arg("-f").arg(&file).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    hidden(&mut command);
    let out = command.output().map_err(|e| Fault::Other(e.to_string()))?;
    let _ = std::fs::remove_dir_all(&path);
    let said = String::from_utf8_lossy(&out.stdout);
    said.split_whitespace()
        .find(|part| part.starts_with("SHA256:"))
        .map(str::to_owned)
        .ok_or_else(|| Fault::Other("ssh-keygen didn't say what that key's fingerprint is.".into()))
}

/// `user@host` (or `host`) as typed, split.
fn split_target(target: &str) -> Option<(String, String)> {
    let target = target.trim();
    if target.is_empty() || target.len() > 255 || target.contains(char::is_whitespace) {
        return None;
    }
    match target.split_once('@') {
        Some((user, host)) => {
            let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-._:@[]".contains(c));
            (ok(user) && ok(host)).then(|| (user.to_owned(), host.to_owned()))
        }
        None => {
            let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_else(|_| "root".into());
            Some((user, target.to_owned()))
        }
    }
}

fn slug(name: &str) -> String {
    let kept: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let trimmed = kept.trim_matches('-').to_owned();
    if trimmed.is_empty() { format!("m-{}", std::process::id()) } else { trimmed[..trimmed.len().min(40)].to_owned() }
}

// ── what the scripts see ───────────────────────────────────────────────────

fn snapshot() -> Json {
    let held = live().lock().unwrap();
    let live_id = held.as_ref().map(|l| l.host.clone());
    let link = held.as_ref().map(|l| {
        json!({
            "host": l.host,
            "port": l.local_port,
            "for_seconds": l.since.elapsed().as_secs(),
        })
    });
    drop(held);
    let hosts: Vec<Json> = hosts()
        .into_iter()
        .map(|host| {
            let state = state_of(&host.id);
            json!({
                "id": host.id,
                "name": host.name,
                "target": host.target,
                "address": host.address(),
                "port": host.port,
                "identity": host.identity,
                "data_dir": host.read_dir(),
                "engine_port": host.engine_port,
                "fingerprint": host.fingerprint,
                "trusted": host.trusted(),
                "state": if live_id.as_deref() == Some(host.id.as_str()) && state.state == "live" { "live" } else { state.state },
                "detail": state.detail,
                "version": state.version,
                "active": live_id.as_deref() == Some(host.id.as_str()),
                "here": host.local,
            })
        })
        .collect();
    let active_name = live_id.as_deref().and_then(|id| hosts.iter().find(|h| h["id"] == id).map(|h| h["name"].clone()));
    json!({ "hosts": hosts, "active": live_id, "active_name": active_name, "link": link })
}

thread_local! {
    static SHARED: std::cell::RefCell<Option<zgx_script::Shared>> = const { std::cell::RefCell::new(None) };
}

fn publish() {
    let now = snapshot();
    SHARED.with(|s| {
        if let Some(shared) = s.borrow().as_ref() {
            shared.set(crate::kx::to_value(&now));
        }
    });
}

fn id_for(target: &str) -> String {
    format!("m-{}", slug(target))
}

// ── the scripts' surface ───────────────────────────────────────────────────

pub fn install(cx: &mut zgx_desktop::WindowContext, host: &ScriptHost) {
    let _ = cx;
    let shared = host.shared("kx.remote", crate::kx::to_value(&snapshot()));
    SHARED.with(|s| *s.borrow_mut() = Some(shared));

    host.async_action("kx.remote.status", |_| async move { Ok(crate::kx::to_value(&snapshot())) });

    host.async_action("kx.remote.add", |req| {
        let req = crate::kx::widgets::value_json(&req);
        async move {
            let target = req["target"].as_str().unwrap_or_default().to_owned();
            let (user, name) =
                split_target(&target).ok_or("That isn't `user@machine` or a machine's name.")?;
            let port = req["port"].as_u64().unwrap_or(22).clamp(1, 65535) as u16;
            let address = format!("{user}@{name}");
            let scanned = crate::kx::off_thread({
                let (address, name) = (address.clone(), name.clone());
                move || scan(&name, port).map(|(line, fp)| (line, fp, address))
            })
            .await
            .ok_or_else(|| "The host key check didn't finish.".to_owned())?;
            let (known_host, fingerprint, target) = scanned.map_err(|fault| match fault {
                Fault::Unreachable(why) | Fault::Engine(why) | Fault::NoForward(why) | Fault::Other(why) => why,
                other => other.say(),
            })?;
            let identity = req["identity"].as_str().map(str::to_owned).filter(|p| !p.trim().is_empty());
            let data_dir = req["data_dir"].as_str().map(str::to_owned).filter(|d| !d.trim().is_empty()).unwrap_or_else(|| DEFAULT_DATA_DIR.to_owned());
            let engine_port = req["engine_port"].as_u64().unwrap_or(DEFAULT_ENGINE_PORT as u64).clamp(1024, 65535) as u16;
            let id = id_for(&format!("{target}-{port}"));
            let mut all = hosts();
            all.retain(|h| h.id != id);
            all.push(Host {
                id: id.clone(),
                name: req["name"].as_str().map(str::to_owned).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| name.clone()),
                target,
                port,
                identity,
                data_dir,
                engine_port,
                known_host: Some(known_host.clone()),
                fingerprint: Some(fingerprint.clone()),
                local: false,
            });
            // The key is offered, not yet trusted: `kx.remote.trust` is the
            // person's answer, and nothing dials before it.
            let mut offered = all.clone();
            if let Some(entry) = offered.iter_mut().find(|h| h.id == id) {
                entry.known_host = None;
            }
            write_hosts(&offered);
            states().lock().unwrap().insert(id.clone(), State { state: "untrusted", detail: format!("Accept this machine's host key: {fingerprint}"), version: None });
            pending().lock().unwrap().insert(id.clone(), known_host);
            publish();
            Ok(crate::kx::to_value(&json!({ "id": id, "fingerprint": fingerprint, "name": name })))
        }
    });

    host.async_action("kx.remote.trust", |req| {
        let req = crate::kx::widgets::value_json(&req);
        async move {
            let id = req["id"].as_str().unwrap_or_default().to_owned();
            let accept = req["accept"].as_bool().unwrap_or(false);
            if !accept {
                let mut all = hosts();
                all.retain(|h| h.id != id);
                write_hosts(&all);
                states().lock().unwrap().remove(&id);
                pending().lock().unwrap().remove(&id);
                let _ = std::fs::remove_file(known_hosts_file(&id));
                publish();
                return Ok(Value::Nil);
            }
            let Some(line) = pending().lock().unwrap().remove(&id) else {
                return Err("That machine isn't waiting to be accepted.".into());
            };
            let mut all = hosts();
            let Some(entry) = all.iter_mut().find(|h| h.id == id) else {
                return Err("That machine isn't here any more.".into());
            };
            entry.known_host = Some(line.clone());
            write_hosts(&all);
            // Pinned in this app's own file: from here a changed key is a
            // refusal, never a silent new trust.
            let path = known_hosts_file(&id);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&path, format!("{line}\n")).map_err(|e| format!("The host key couldn't be kept: {e}"))?;
            states().lock().unwrap().insert(id.clone(), State { state: "stopped", detail: "Accepted. Connect when you're ready.".into(), version: None });
            publish();
            Ok(Value::Nil)
        }
    });

    host.async_action("kx.remote.connect", |req| {
        let id = machine_id(&req);
        async move {
            let answer = connect(id.clone()).await?;
            set_active(Some(&id));
            Ok(crate::kx::to_value(&answer))
        }
    });

    host.action("kx.remote.disconnect", |_| {
        disconnect(true);
    });

    host.action("kx.remote.remove", |req| {
        let id = machine_id(&req);
        if live_host().as_deref() == Some(id.as_str()) {
            disconnect(false);
        }
        let mut all = hosts();
        all.retain(|h| h.id != id);
        write_hosts(&all);
        states().lock().unwrap().remove(&id);
        let _ = std::fs::remove_file(known_hosts_file(&id));
        publish();
    });

    // Attached to a machine last run: link again without being asked.
    if let Some(id) = active() {
        if crate::kx::remote::host(&id).is_some_and(|h| h.trusted()) {
            crate::engine::get().spawn(async move {
                if let Err(error) = connect(id).await {
                    eprintln!("zeron-zgx: machine: didn't reconnect: {error}");
                }
            });
        }
    }
}

/// The host key waiting for the person's answer, by machine.
fn pending() -> &'static Mutex<BTreeMap<String, String>> {
    static PENDING: OnceLock<Mutex<BTreeMap<String, String>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// A machine's id, from `{ id = … }` or a bare id.
fn machine_id(arg: &Value) -> String {
    match arg {
        Value::Map(fields) => fields.iter().find(|(k, _)| k.as_ref() == "id").map(|(_, v)| crate::auth::text(v)).unwrap_or_default(),
        Value::Str(id) => id.to_string(),
        other => crate::auth::text(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_is_user_at_machine_or_just_a_machine() {
        assert_eq!(split_target("ann@box.example"), Some(("ann".into(), "box.example".into())));
        assert_eq!(split_target(" box ").map(|(_, h)| h), Some("box".into()));
        assert_eq!(split_target(""), None);
        assert_eq!(split_target("a@b c"), None);
        assert_eq!(split_target("a@b@c"), Some(("a".into(), "b@c".into())));
    }

    #[test]
    fn a_shell_path_is_quoted_once_and_its_own_quotes_escaped() {
        assert_eq!(shell_quote("~/.keplorayx"), "'~/.keplorayx'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn the_engine_start_command_sets_its_data_and_port_and_detaches() {
        let command = start_command("/usr/local/bin/keplorayx-engine", "/home/ann/.keplorayx", 27741);
        assert!(command.contains("ZERON_DATA_DIR='/home/ann/.keplorayx'"));
        assert!(command.contains("ZERON_IPC_PORT=27741"));
        assert!(command.contains("setsid"));
        assert!(command.contains("</dev/null") || command.contains("< /dev/null"));
        assert!(command.contains("/home/ann/.keplorayx/logs/headless.log"));
    }

    #[test]
    fn an_engines_facts_are_its_path_and_its_build() {
        let facts = parse_facts("/usr/local/bin/keplorayx-engine\nabc123.456\n").expect("facts");
        assert_eq!(facts.bin, "/usr/local/bin/keplorayx-engine");
        assert_eq!(facts.build.as_deref(), Some("abc123.456"));
        // One that doesn't say which build it is: its path, no build.
        assert_eq!(parse_facts("/opt/zeron-engine\n").and_then(|f| f.build), None);
        assert!(parse_facts("").is_none());
        assert!(parse_facts("not a path at all, a sentence\n").is_none());
    }

    #[test]
    fn a_bearer_is_a_token_and_nothing_else() {
        assert_eq!(clean_token("  abc123  \n"), Some("abc123".into()));
        assert_eq!(clean_token(""), None);
        assert_eq!(clean_token("a\nb"), None);
        assert_eq!(clean_token(&"x".repeat(600)), None);
    }

    /// ssh's own words, as the fault the person reads.
    fn fault_of(why: &str, fingerprint: Option<&str>) -> Fault {
        let host = Host {
            id: "m-x".into(),
            name: "x".into(),
            target: "ann@x".into(),
            port: 22,
            identity: None,
            data_dir: DEFAULT_DATA_DIR.into(),
            engine_port: DEFAULT_ENGINE_PORT,
            known_host: Some("line".into()),
            fingerprint: fingerprint.map(str::to_owned),
            local: false,
        };
        Ssh { host }.fault(why.to_owned())
    }

    #[test]
    fn each_way_a_machine_refuses_has_its_own_words() {
        let changed = fault_of("@@@@@@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! host key verification failed.", Some("SHA256:abc"));
        assert!(matches!(changed, Fault::HostKeyChanged(_)));
        assert!(changed.say().contains("Nothing was sent"));
        assert!(matches!(fault_of("ann@x: Permission denied (publickey).", Some("SHA256:abc")), Fault::RefusedKey));
        assert!(matches!(fault_of("ssh: Could not resolve hostname nope: Name or service not known", Some("SHA256:abc")), Fault::Unreachable(_)));
        assert!(matches!(fault_of("ssh: connect to host x port 22: Connection refused", Some("SHA256:abc")), Fault::Unreachable(_)));
        assert!(matches!(fault_of("ssh: connect to host x port 22: Connection timed out", Some("SHA256:abc")), Fault::Unreachable(_)));
        assert!(matches!(fault_of("something else entirely", Some("SHA256:abc")), Fault::Other(_)));
    }

    fn machine(local: bool) -> Host {
        Host {
            id: "m-x".into(),
            name: "x".into(),
            target: "ann@x".into(),
            port: 2222,
            identity: Some("C:/Users/ann/.ssh/id_ed25519".into()),
            data_dir: DEFAULT_DATA_DIR.into(),
            engine_port: DEFAULT_ENGINE_PORT,
            known_host: Some("[x]:2222 ssh-ed25519 AAAA".into()),
            fingerprint: Some("SHA256:abc".into()),
            local,
        }
    }

    /// The forward carries bytes both ways and keeps carrying them: an echo
    /// server at one end, a client at the other, several round trips.
    #[test]
    fn a_relay_carries_bytes_both_ways() {
        let echo = TcpListener::bind(("127.0.0.1", 0)).expect("an echo port");
        let echo_port = echo.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut socket, _) = echo.accept().expect("a client");
            let mut buf = [0u8; 64];
            while let Ok(n) = socket.read(&mut buf) {
                if n == 0 || socket.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        });
        let local = free_port().expect("a port");
        let stop = Arc::new(AtomicBool::new(false));
        let mut relay = Relay::start(local, echo_port, stop).expect("a relay");
        std::thread::sleep(Duration::from_millis(100));
        let mut client = TcpStream::connect(("127.0.0.1", local)).expect("the relay");
        let _ = client.set_read_timeout(Some(Duration::from_secs(5)));
        for round in 0..3 {
            client.write_all(format!("ping {round}").as_bytes()).expect("written");
            let mut back = [0u8; 16];
            let n = client.read(&mut back).expect("an answer");
            assert_eq!(&back[..n], format!("ping {round}").as_bytes());
        }
        relay.kill();
    }

    #[test]
    fn an_unaccepted_machine_is_never_dialled() {
        let mut host = machine(false);
        assert!(host.trusted());
        host.known_host = None;
        assert!(!host.trusted(), "a key offered but not accepted");
        host.known_host = Some("line".into());
        assert!(machine(true).trusted(), "a loopback engine has no host key");
    }

    /// ssh is told exactly what this app trusts and nothing more: no
    /// prompting it cannot answer, one pinned known_hosts of its own, and
    /// the person's key only if they named one.
    #[test]
    fn the_ssh_command_pins_the_host_key_and_never_prompts() {
        let host = machine(false);
        let ssh = Ssh { host: host.clone() };
        let command = ssh.command().expect("ssh");
        let args: Vec<String> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        let joined = args.join(" ");
        assert!(joined.contains("BatchMode=yes"));
        assert!(joined.contains("StrictHostKeyChecking=yes"));
        assert!(joined.contains("UpdateHostKeys=no"));
        assert!(joined.contains(&format!("UserKnownHostsFile={}", known_hosts_file(&host.id).display())));
        assert!(joined.contains("-p 2222"));
        assert!(joined.contains("-i C:/Users/ann/.ssh/id_ed25519"));
        assert!(joined.contains("IdentitiesOnly=yes"));
        assert_eq!(args.last().map(String::as_str), Some("ann@x"));
    }

    /// The forward is a loopback port of its own on the engine's port there,
    /// and it fails loudly rather than half-open.
    #[test]
    fn the_forward_is_one_loopback_port_to_the_engines_own() {
        let ssh = Ssh { host: machine(false) };
        // (Its child can't start without a machine: the argv is what's read.)
        let command = ssh.command().expect("ssh");
        let base: Vec<String> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(!base.iter().any(|a| a == "-N"), "a run, not a forward");
        assert!(command_line_for_forward(&ssh, 27999).contains("-L 127.0.0.1:27999:127.0.0.1:27741"));
    }

    /// `forward`'s argument list, without starting a process.
    fn command_line_for_forward(ssh: &Ssh, local: u16) -> String {
        let mut command = ssh.command().expect("ssh");
        command
            .arg("-N")
            .arg("-o")
            .arg("ExitOnForwardFailure=yes")
            .arg("-o")
            .arg("ServerAliveInterval=10")
            .arg("-o")
            .arg("ServerAliveCountMax=3")
            .arg("-L")
            .arg(format!("127.0.0.1:{local}:127.0.0.1:{}", ssh.host.engine_port));
        let args: Vec<String> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        args.join(" ")
    }

    /// What the engine at the far end says it is, waiting as the app's
    /// supervisor waits: an engine that has bound its port but not yet
    /// assembled its core answers, then drops the connection ("Handshake
    /// not finished", then a reset), so a connection that ends is made again.
    fn engine_identity(port: u16, token: &str) -> Result<String, String> {
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        let until = Instant::now() + Duration::from_secs(90);
        let mut said = String::new();
        while Instant::now() < until {
            let mut connecting = std::pin::pin!(crate::engine::rpc::connect(port, Some(token.to_owned())));
            let rpc = loop {
                if let std::task::Poll::Ready(answer) = connecting.as_mut().poll(&mut cx) {
                    match answer {
                        Ok(rpc) => break Some(rpc),
                        Err(error) => {
                            said = format!("{error:?}");
                            break None;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            };
            if let Some(rpc) = rpc {
                let mut pump = std::pin::pin!(rpc.pump());
                let mut call = std::pin::pin!(rpc.call("EngineInfo", json!({})));
                loop {
                    if pump.as_mut().poll(&mut cx).is_ready() {
                        break;
                    }
                    if let std::task::Poll::Ready(answer) = call.as_mut().poll(&mut cx) {
                        rpc.close();
                        match answer {
                            Ok(info) => return Ok(info["deviceId"].as_str().unwrap_or_default().to_owned()),
                            Err(error) => said = error,
                        }
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        Err(format!("no engine answered in 90 s (last: {said})"))
    }

    /// The whole path with an engine at the far end: the bearer read over
    /// the channel, a forward that carries the app's own IPC, an engine
    /// answering on it — and the same engine still there after the forward
    /// is killed and put back.
    ///
    /// (A loopback stand-in for the far machine: it proves the transport,
    /// not the ssh trust boundary. `KX_TEST_ENGINE=1` runs it.)
    #[test]
    #[ignore = "starts a real engine"]
    fn a_link_carries_a_whole_engine_and_survives_losing_the_forward() {
        if std::env::var("KX_TEST_ENGINE").ok().as_deref() != Some("1") {
            return;
        }
        let data = std::env::temp_dir().join(format!("kx-remote-e2e-{}", std::process::id()));
        let port = free_port().expect("a port");
        let host = Host {
            id: "e2e".into(),
            name: "loopback".into(),
            target: "loopback".into(),
            port: 22,
            identity: None,
            data_dir: data.display().to_string(),
            engine_port: port,
            known_host: Some("loopback".into()),
            fingerprint: Some("loopback".into()),
            local: true,
        };
        let local = free_port().expect("a port");
        let first = link_up(&host, local).expect("a link");
        assert!(!first.token.is_empty(), "a bearer read over the channel");
        let identity = engine_identity(first.local_port, &first.token).expect("the engine answers");
        assert!(!identity.is_empty(), "the engine said who it is");

        // The link dies; the engine does not.
        let mut forward = first.forward;
        forward.kill();
        let gone = (0..20).all(|_| {
            std::thread::sleep(Duration::from_millis(100));
            TcpStream::connect_timeout(&([127, 0, 0, 1], first.local_port).into(), Duration::from_millis(300)).is_err()
        });
        assert!(gone, "the forward's port is gone with it");
        let (back, bearer) = relink(&host, local).expect("the link again");
        let mut forward = back;
        let again = engine_identity(local, &bearer).expect("the same engine answers");
        assert_eq!(again, identity, "the engine never restarted");
        forward.kill();
        let _ = std::fs::remove_dir_all(&data);
    }
}
