//! "Use this computer as a machine": pairing codes, the node's trusted
//! devices, the client's paired nodes, and the one-shot CLI modes
//! (`exec`, `pair`, `nodes`, `serve-node`, …).
//!
//! Files under the data dir (`~/.dove/mesh` by default), all 0600:
//! - `node.json`     `{enabled}` — node mode on/off (the sidecar polls it);
//! - `pairing.json`  the active pairing code (SHA-256 only), expiry, attempts;
//! - `trusted.json`  devices that paired with this node (SHA-256 of their proof);
//! - `nodes.json`    nodes this device paired with (id, name, proof).
//!
//! Every process (desktop sidecar, `serve-node`, one-shot CLIs) reads the
//! files on use, so a code made by one is redeemable through another and a
//! revoke takes effect on the next exec (and within seconds for running ones).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use zeron_rpc::node_exec::{self, Dial, ExecOpen, NodePolicy, PairGrant, PairRequest};

use crate::{Account, Bearer, Paths, device_id, hostname, write_private};

/// How long a pairing code stays valid.
pub const CODE_TTL: Duration = Duration::from_secs(10 * 60);
/// Wrong guesses before the active code is burned.
const MAX_ATTEMPTS: u32 = 5;
/// Crockford base32: no I, L, O, U.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn random32() -> [u8; 32] {
    // Two v4 UUIDs (OS CSPRNG, 244 random bits) folded through SHA-256.
    let mut hasher = Sha256::new();
    hasher.update(uuid::Uuid::new_v4().as_bytes());
    hasher.update(uuid::Uuid::new_v4().as_bytes());
    hasher.finalize().into()
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Constant-time equality for fixed-size digests.
fn ct_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    std::hint::black_box(diff) == 0
}

/// `ABCD-EFGH` (40 bits).
fn new_code() -> String {
    let bytes = random32();
    let chars: String = bytes[..8].iter().map(|b| ALPHABET[(*b % 32) as usize] as char).collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

/// Canonical form of a typed code: case, dashes and spaces ignored, and the
/// look-alikes Crockford folds (O→0, I/L→1, U→V).
pub fn normalize_code(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            'U' => 'V',
            c => c,
        })
        .collect()
}

pub fn pair_uri(device: &str, code: &str) -> String {
    format!("keplorayx://pair?device={}&code={}", crate::enc(device), crate::enc(code))
}

/// `keplorayx://pair?device=…&code=…` → (device, code).
pub fn parse_pair_uri(uri: &str) -> Option<(String, String)> {
    let query = uri.strip_prefix("keplorayx://pair?")?;
    let (mut device, mut code) = (None, None);
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some(("device", v)) => device = Some(crate::percent_decode(v)),
            Some(("code", v)) => code = Some(crate::percent_decode(v)),
            _ => {}
        }
    }
    Some((device?, code?))
}

fn valid_device_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    std::fs::read(path).ok().and_then(|raw| serde_json::from_slice(&raw).ok()).unwrap_or_default()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    write_private(path, &bytes).map_err(|e| format!("could not save {}: {e}", path.display()))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct NodeSettings {
    #[serde(default)]
    enabled: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pending {
    code_hash: String,
    expires_at: i64,
    #[serde(default)]
    attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trusted {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    proof_hash: String,
    pub paired_at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustedFile {
    #[serde(default)]
    devices: Vec<Trusted>,
}

/// A node this device paired with (client side, `nodes.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedNode {
    pub id: String,
    pub name: String,
    secret: String,
    pub paired_at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct NodesFile {
    #[serde(default)]
    nodes: Vec<PairedNode>,
}

pub struct Offer {
    pub code: String,
    pub uri: String,
    pub expires_at: i64,
}

type PairedHook = Box<dyn Fn(&str, &str) + Send + Sync>;

pub struct NodeStore {
    dir: PathBuf,
    device_id: String,
    name: String,
    /// Serializes read-modify-write of the files within this process.
    lock: Mutex<()>,
    on_paired: Mutex<Option<PairedHook>>,
}

impl NodeStore {
    pub fn new(dir: &Path, device_id: String, name: String) -> Arc<Self> {
        Arc::new(Self { dir: dir.to_path_buf(), device_id, name, lock: Mutex::new(()), on_paired: Mutex::new(None) })
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn on_paired(&self, hook: impl Fn(&str, &str) + Send + Sync + 'static) {
        *self.on_paired.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Box::new(hook));
    }

    pub fn enabled(&self) -> bool {
        read_json::<NodeSettings>(&self.dir.join("node.json")).enabled
    }

    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let _g = self.guard();
        write_json(&self.dir.join("node.json"), &NodeSettings { enabled })
    }

    /// A fresh single-use code (replaces any active one).
    pub fn offer(&self) -> Result<Offer, String> {
        let _g = self.guard();
        let code = new_code();
        let expires_at = now_ms() + CODE_TTL.as_millis() as i64;
        let pending = Pending { code_hash: hex(&sha256(normalize_code(&code).as_bytes())), expires_at, attempts: 0 };
        write_json(&self.dir.join("pairing.json"), &pending)?;
        Ok(Offer { uri: pair_uri(&self.device_id, &code), code, expires_at })
    }

    pub fn trusted(&self) -> Vec<Trusted> {
        let mut devices = read_json::<TrustedFile>(&self.dir.join("trusted.json")).devices;
        for d in &mut devices {
            d.proof_hash.clear();
        }
        devices
    }

    pub fn revoke(&self, device: &str) -> Result<bool, String> {
        let _g = self.guard();
        let path = self.dir.join("trusted.json");
        let mut file = read_json::<TrustedFile>(&path);
        let before = file.devices.len();
        file.devices.retain(|d| d.id != device);
        let removed = file.devices.len() != before;
        if removed {
            write_json(&path, &file)?;
        }
        Ok(removed)
    }

    // ── client side ──

    pub fn nodes(&self) -> Vec<PairedNode> {
        read_json::<NodesFile>(&self.dir.join("nodes.json")).nodes
    }

    fn save_node(&self, node: PairedNode) -> Result<(), String> {
        let _g = self.guard();
        let path = self.dir.join("nodes.json");
        let mut file = read_json::<NodesFile>(&path);
        file.nodes.retain(|n| n.id != node.id);
        file.nodes.push(node);
        write_json(&path, &file)
    }

    pub fn forget(&self, device: &str) -> Result<bool, String> {
        let _g = self.guard();
        let path = self.dir.join("nodes.json");
        let mut file = read_json::<NodesFile>(&path);
        let before = file.nodes.len();
        file.nodes.retain(|n| n.id != device);
        let removed = file.nodes.len() != before;
        if removed {
            write_json(&path, &file)?;
        }
        Ok(removed)
    }
}

impl NodePolicy for NodeStore {
    fn authorize(&self, device: &str, proof: &str) -> Result<(), String> {
        let devices = read_json::<TrustedFile>(&self.dir.join("trusted.json")).devices;
        let presented = sha256(proof.as_bytes());
        let ok = devices
            .iter()
            .find(|d| d.id == device)
            .and_then(|d| unhex(&d.proof_hash))
            .is_some_and(|stored| ct_eq(&stored, &presented));
        if ok {
            Ok(())
        } else {
            Err(format!("{} is not paired with {device}; pair it again from Settings → Machines", self.name))
        }
    }

    fn pair(&self, request: &PairRequest) -> Result<PairGrant, String> {
        if !valid_device_id(&request.device) {
            return Err("invalid device id".into());
        }
        let grant = {
            let _g = self.guard();
            let path = self.dir.join("pairing.json");
            let Some(mut pending) =
                std::fs::read(&path).ok().and_then(|raw| serde_json::from_slice::<Pending>(&raw).ok())
            else {
                return Err("no pairing code is active on this computer; make a new one".into());
            };
            if pending.expires_at < now_ms() {
                let _ = std::fs::remove_file(&path);
                return Err("this pairing code expired; make a new one".into());
            }
            pending.attempts += 1;
            let presented = sha256(normalize_code(&request.code).as_bytes());
            let matches = unhex(&pending.code_hash).is_some_and(|stored| ct_eq(&stored, &presented));
            if !matches {
                if pending.attempts >= MAX_ATTEMPTS {
                    let _ = std::fs::remove_file(&path);
                    return Err("wrong code; too many attempts, make a new code".into());
                }
                write_json(&path, &pending)?;
                return Err("wrong pairing code".into());
            }
            // Single use: burn it before granting anything.
            std::fs::remove_file(&path).map_err(|e| format!("could not consume the code: {e}"))?;
            let secret = hex(&random32());
            let trusted_path = self.dir.join("trusted.json");
            let mut file = read_json::<TrustedFile>(&trusted_path);
            file.devices.retain(|d| d.id != request.device);
            let name: String = request.name.trim().chars().filter(|c| !c.is_control()).take(80).collect();
            file.devices.push(Trusted {
                id: request.device.clone(),
                name: if name.is_empty() { request.device.clone() } else { name },
                proof_hash: hex(&sha256(secret.as_bytes())),
                paired_at: now_ms(),
            });
            write_json(&trusted_path, &file)?;
            PairGrant { device: self.device_id.clone(), name: self.name.clone(), secret }
        };
        if let Some(hook) = &*self.on_paired.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
            hook(&request.device, &request.name);
        }
        Ok(grant)
    }

    fn workdir(&self) -> PathBuf {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"))
    }
}

/// The terminal QR for `text` (dark-on-light blocks, quiet zone included).
pub fn qr(text: &str) -> String {
    use qrcode::render::unicode::Dense1x2;
    match qrcode::QrCode::new(text.as_bytes()) {
        Ok(code) => code.render::<Dense1x2>().dark_color(Dense1x2::Light).light_color(Dense1x2::Dark).build(),
        Err(_) => String::new(),
    }
}

// ── CLI ─────────────────────────────────────────────────────────────────────

pub struct Cli {
    pub paths: Paths,
    pub data_dir: PathBuf,
    pub edge: String,
    pub positionals: Vec<String>,
    pub flags: std::collections::HashMap<String, String>,
    pub argv: Vec<String>,
}

impl Cli {
    fn flag(&self, name: &str) -> Option<&str> {
        self.flags.get(name).map(String::as_str)
    }

    fn json(&self) -> bool {
        self.flags.contains_key("json")
    }

    fn store(&self) -> Result<Arc<NodeStore>, String> {
        let id = device_id(&self.paths).map_err(|e| e.to_string())?;
        Ok(NodeStore::new(&self.data_dir, id, hostname()))
    }

    fn account(&self) -> Option<Account> {
        std::fs::read(&self.paths.account).ok().and_then(|raw| serde_json::from_slice(&raw).ok())
    }

    fn bearer(&self) -> Option<Bearer> {
        let account = self.account()?;
        Some(Bearer { account: Arc::new(tokio::sync::Mutex::new(Some(account))), path: self.paths.account.clone() })
    }
}

fn fail(code: i32, message: impl std::fmt::Display) -> i32 {
    eprintln!("dove-mesh-host: {message}");
    code
}

/// Runs a one-shot command; returns the process exit code.
pub async fn run(cli: Cli) -> i32 {
    let command = cli.positionals.first().cloned().unwrap_or_default();
    let store = match cli.store() {
        Ok(store) => store,
        Err(e) => return fail(1, e),
    };
    match command.as_str() {
        "exec" => exec(&cli, &store).await,
        "pair" => pair(&cli, &store).await,
        "nodes" => nodes(&cli, &store).await,
        "serve-node" => serve_node(&cli, &store).await,
        "pair-code" => match store.offer() {
            Ok(offer) => {
                if cli.json() {
                    println!(
                        "{}",
                        json!({ "code": offer.code, "uri": offer.uri, "expiresAt": offer.expires_at,
                                "device": store.device_id(), "enabled": store.enabled() })
                    );
                } else {
                    println!("{}\n{}\n{}", offer.code, qr(&offer.uri), offer.uri);
                }
                0
            }
            Err(e) => fail(1, e),
        },
        "paired" => {
            let devices = store.trusted();
            if cli.json() {
                println!("{}", serde_json::to_string(&devices).unwrap_or_else(|_| "[]".into()));
            } else {
                for d in devices {
                    println!("{}\t{}", d.id, d.name);
                }
            }
            0
        }
        "revoke" | "forget" => {
            let Some(device) = cli.flag("device") else { return fail(2, format!("{command} --device <id>")) };
            let result = if command == "revoke" { store.revoke(device) } else { store.forget(device) };
            match result {
                Ok(_) => 0,
                Err(e) => fail(1, e),
            }
        }
        "node-mode" => {
            let enabled = match cli.positionals.get(1).map(String::as_str) {
                Some("on") => true,
                Some("off") => false,
                _ => {
                    println!("{}", if store.enabled() { "on" } else { "off" });
                    return 0;
                }
            };
            match store.set_enabled(enabled) {
                Ok(()) => 0,
                Err(e) => fail(1, e),
            }
        }
        other => fail(
            2,
            format!(
                "unknown command {other:?} (exec, pair, nodes, serve-node, pair-code, paired, revoke, forget, node-mode)"
            ),
        ),
    }
}

async fn token(cli: &Cli) -> Result<String, i32> {
    let Some(bearer) = cli.bearer() else {
        return Err(fail(node_exec::EXIT_REFUSED, "not signed in to Woozlit; sign in from Keplorayx first"));
    };
    bearer.get().await.map_err(|e| fail(node_exec::EXIT_LINK_LOST, format!("account token: {e}")))
}

async fn exec(cli: &Cli, store: &NodeStore) -> i32 {
    let Some(device) = cli.flag("device") else { return fail(2, "exec --device <id> -- <argv…>") };
    if cli.argv.is_empty() {
        return fail(2, "exec --device <id> -- <argv…>");
    }
    let Some(node) = store.nodes().into_iter().find(|n| n.id == device) else {
        return fail(node_exec::EXIT_REFUSED, format!("this computer is not paired with {device}"));
    };
    let token = match token(cli).await {
        Ok(token) => token,
        Err(code) => return code,
    };
    let dial = Dial {
        edge_url: cli.edge.clone(),
        device: device.to_owned(),
        token,
        direct_name: Some(node.name.clone()),
    };
    let open = ExecOpen {
        device: store.device_id().to_owned(),
        proof: node.secret,
        argv: cli.argv.clone(),
        cwd: cli.flag("cwd").map(str::to_owned),
    };
    match node_exec::exec(&dial, &open, tokio::io::stdin(), tokio::io::stdout(), tokio::io::stderr()).await {
        Ok(code) => code,
        Err(e) => fail(e.exit_code(), e),
    }
}

async fn pair(cli: &Cli, store: &NodeStore) -> i32 {
    let (device, code) = match (cli.flag("uri").and_then(parse_pair_uri), cli.flag("device"), cli.flag("code")) {
        (Some(parsed), _, _) => parsed,
        (None, Some(device), Some(code)) => (device.to_owned(), code.to_owned()),
        _ => return fail(2, "pair --device <id> --code <code>  (or --uri keplorayx://pair?…)"),
    };
    if device == store.device_id() {
        return fail(2, "that code belongs to this computer");
    }
    let token = match token(cli).await {
        Ok(token) => token,
        Err(code) => return code,
    };
    let dial = Dial { edge_url: cli.edge.clone(), device: device.clone(), token, direct_name: None };
    let request = PairRequest { code, device: store.device_id().to_owned(), name: store.name().to_owned() };
    match node_exec::pair(&dial, &request).await {
        Ok(grant) => {
            let node =
                PairedNode { id: device.clone(), name: grant.name.clone(), secret: grant.secret, paired_at: now_ms() };
            if let Err(e) = store.save_node(node) {
                return fail(1, e);
            }
            if cli.json() {
                println!("{}", json!({ "ok": true, "device": device, "name": grant.name }));
            } else {
                println!("Paired with {} ({device})", grant.name);
            }
            0
        }
        Err(e) => fail(e.exit_code(), e),
    }
}

async fn nodes(cli: &Cli, store: &NodeStore) -> i32 {
    let nodes = store.nodes();
    let timeout = Duration::from_millis(cli.flag("timeout-ms").and_then(|v| v.parse().ok()).unwrap_or(2_000));
    let token = match (nodes.is_empty(), cli.bearer()) {
        (false, Some(bearer)) => bearer.get().await.ok(),
        _ => None,
    };
    let probes = nodes.iter().map(|node| {
        let token = token.clone();
        let edge = cli.edge.clone();
        let id = node.id.clone();
        async move {
            match token {
                Some(token) => {
                    node_exec::probe(&Dial { edge_url: edge, device: id, token, direct_name: None }, timeout).await
                }
                None => false,
            }
        }
    });
    let online = futures::future::join_all(probes).await;
    let rows: Vec<_> = nodes
        .iter()
        .zip(online)
        .map(|(n, online)| json!({ "id": n.id, "name": n.name, "online": online, "pairedAt": n.paired_at }))
        .collect();
    if cli.json() {
        println!("{}", serde_json::Value::Array(rows));
    } else {
        for row in rows {
            println!(
                "{}\t{}\t{}",
                row["id"].as_str().unwrap_or(""),
                row["name"].as_str().unwrap_or(""),
                if row["online"].as_bool() == Some(true) { "online" } else { "offline" }
            );
        }
    }
    0
}

async fn serve_node(cli: &Cli, store: &Arc<NodeStore>) -> i32 {
    let Some(bearer) = cli.bearer() else {
        return fail(1, "not signed in to Woozlit; sign in from Keplorayx on this computer first");
    };
    store.on_paired(|device, name| println!("Paired with {name} ({device})."));
    let _relay = crate::spawn_node_relay(&cli.edge, store, bearer);
    println!("{} is serving as a Keplorayx machine (device {}).", store.name(), store.device_id());
    if !cli.flags.contains_key("no-code") {
        match store.offer() {
            Ok(offer) => {
                println!("On your laptop: Add a machine → Pair a device, then scan or type the code.");
                println!("\n  Code: {}   (valid {} minutes, single use)\n", offer.code, CODE_TTL.as_secs() / 60);
                println!("{}", qr(&offer.uri));
                println!("  {}\n", offer.uri);
            }
            Err(e) => eprintln!("dove-mesh-host: {e}"),
        }
    }
    let _ = tokio::signal::ctrl_c().await;
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_normalize_and_round_trip_through_uri() {
        assert_eq!(normalize_code("abcd-efgh"), "ABCDEFGH");
        assert_eq!(normalize_code(" o1l-i u"), "0111V");
        let code = new_code();
        assert_eq!(code.len(), 9);
        let uri = pair_uri("dove-desktop-1", &code);
        assert_eq!(parse_pair_uri(&uri), Some(("dove-desktop-1".into(), code)));
    }

    #[test]
    fn pairing_is_single_use_and_limited() {
        let dir = tempfile::tempdir().unwrap();
        let store = NodeStore::new(dir.path(), "node-1".into(), "Mac".into());
        let request = |code: &str| PairRequest { code: code.into(), device: "laptop-1".into(), name: "Laptop".into() };
        assert!(store.pair(&request("AAAA-AAAA")).is_err(), "no code active");
        let offer = store.offer().unwrap();
        let grant = store.pair(&request(&offer.code.to_lowercase())).unwrap();
        assert_eq!(grant.device, "node-1");
        assert!(store.authorize("laptop-1", &grant.secret).is_ok());
        assert!(store.authorize("laptop-1", "wrong").is_err());
        assert!(store.authorize("other", &grant.secret).is_err());
        assert!(store.pair(&request(&offer.code)).is_err(), "single use");
        let offer = store.offer().unwrap();
        for _ in 0..MAX_ATTEMPTS {
            assert!(store.pair(&request("0000-0000")).is_err());
        }
        assert!(store.pair(&request(&offer.code)).is_err(), "burned after too many attempts");
        assert!(store.revoke("laptop-1").unwrap());
        assert!(store.authorize("laptop-1", &grant.secret).is_err());
        assert!(!store.trusted().iter().any(|d| !d.proof_hash.is_empty()));
    }
}
