//! Boat (boat.dev): cloud computers over a REST API, reached with plain SSH.
//! Built from the Boat Public API v1 OpenAPI spec (`https://boat.dev/api/v1`,
//! reference at https://boat.dev/docs/api).
//!
//! Connecting: Keplorayx keeps an ed25519 key per sandbox (system
//! `ssh-keygen`), authorizes it with `POST /sandboxes/{id}/sshkey`, and gets
//! back the host to dial (`machineIp`, or `sshEndpoint` `host:port` when the
//! machine has no public IPv4), the SSH user (`user`, home `/home/user`) and
//! the sandbox's host key, which is pinned. So `ssh` runs with `BatchMode`.
//!
//! Lifecycle: `stop` archives the disk to a snapshot (kept 14 days) and
//! `resume` restores it onto a fresh machine. A new sandbox keeps Boat's
//! default `ttlSeconds` (one hour before it auto-stops); `wake` resumes it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider, SshTarget};

const API: &str = "https://boat.dev/api/v1";

pub struct Boat;
pub static PROVIDER: Boat = Boat;

impl Boat {
    fn key<'a>(&self, keys: &'a Keys) -> Result<&'a str> {
        keys.get(self.id()).context("no Boat API key: add one in Settings › Machines")
    }

    fn request(&self, keys: &Keys, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let key = self.key(keys)?;
        let mut req = super::http().request(method.clone(), format!("{API}{path}")).bearer_auth(key);
        if method == reqwest::Method::POST && path == "/sandboxes" {
            // Makes a create safe to retry without a second billable box.
            req = req.header("Idempotency-Key", uuid::Uuid::now_v7().to_string());
        }
        if method == reqwest::Method::DELETE {
            // Boat refuses a delete unless this header names the sandbox.
            req = req.header("X-Ascii-Confirm-Delete", path.rsplit('/').next().unwrap_or_default());
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().with_context(|| format!("calling Boat {path}"))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            bail!("Boat {path}: {}", api_error(status.as_u16(), &value));
        }
        Ok(value)
    }

    fn sandbox(&self, keys: &Keys, id: &str) -> Result<Value> {
        Ok(self.request(keys, reqwest::Method::GET, &format!("/sandboxes/{id}"), None)?["sandbox"].clone())
    }

    /// Poll until the sandbox is in one of `want`.
    fn wait_state(&self, keys: &Keys, id: &str, want: &[&str], limit: Duration) -> Result<Value> {
        let deadline = Instant::now() + limit;
        loop {
            let s = self.sandbox(keys, id)?;
            let state = s["state"].as_str().unwrap_or("unknown");
            if want.contains(&state) {
                return Ok(s);
            }
            if matches!(state, "error" | "cancelled") {
                let why = s["error"].as_str().unwrap_or("no reason given");
                bail!("Boat sandbox {id} is {state}: {why}");
            }
            if Instant::now() >= deadline {
                bail!("Boat sandbox {id} was still {state} after {}s", limit.as_secs());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

/// States in which the machine is up and SSH answers.
const UP: &[&str] = &["ready", "idle", "running"];

impl Provider for Boat {
    fn id(&self) -> &'static str {
        "boat"
    }

    fn label(&self) -> &'static str {
        "Boat"
    }

    fn key_url(&self) -> &'static str {
        "https://boat.dev"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, keys: &Keys) -> Probe {
        if !keys.has(self.id()) {
            return Probe::NeedsSetup("Add your Boat API key (boat_…)".into());
        }
        match self.request(keys, reqwest::Method::GET, "/sandboxes?limit=1", None) {
            Ok(_) => Probe::Ready,
            Err(err) if err.to_string().contains("HTTP 401") => Probe::NeedsSetup("Boat rejected the key".into()),
            Err(err) => Probe::Unavailable(format!("{err:#}")),
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..5 {
            let path = match &cursor {
                Some(c) => format!("/sandboxes?limit=200&cursor={c}"),
                None => "/sandboxes?limit=200".to_string(),
            };
            let page = self.request(keys, reqwest::Method::GET, &path, None)?;
            out.extend(page["sandboxes"].as_array().into_iter().flatten().filter_map(box_info));
            cursor = page["pageInfo"]["nextCursor"].as_str().map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        Ok(out)
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let mut body = serde_json::Map::new();
        body.insert("labels".into(), json!({ "created-by": "keplorayx" }));
        if let Some(n) = &spec.name {
            body.insert("name".into(), json!(n));
        }
        // A template is a saved snapshot name or a share code (`bsh_…`).
        if let Some(t) = &spec.template {
            body.insert("from".into(), json!(t));
        }
        let reply = self.request(keys, reqwest::Method::POST, "/sandboxes", Some(Value::Object(body)))?;
        let id = reply["sandbox"]["id"].as_str().context("Boat did not say which sandbox it created")?.to_string();
        let s = self.wait_state(keys, &id, UP, Duration::from_secs(300))?;
        box_info(&s).context("Boat returned an unreadable sandbox")
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        let s = self.sandbox(keys, id)?;
        let state = s["state"].as_str().unwrap_or("unknown");
        if UP.contains(&state) {
            return Ok(());
        }
        match state {
            "archiving" => {
                // A resume cannot start until the snapshot is saved.
                self.wait_state(keys, id, &["archived"], Duration::from_secs(300))?;
                self.request(keys, reqwest::Method::POST, &format!("/sandboxes/{id}/resume"), Some(json!({})))?;
            }
            "archived" => {
                self.request(keys, reqwest::Method::POST, &format!("/sandboxes/{id}/resume"), Some(json!({})))?;
            }
            "error" | "cancelled" => bail!("Boat sandbox {id} is {state}"),
            _ => {}
        }
        self.wait_state(keys, id, UP, Duration::from_secs(300)).map(|_| ())
    }

    fn sleep(&self, keys: &Keys, id: &str) -> Result<()> {
        self.request(keys, reqwest::Method::POST, &format!("/sandboxes/{id}/stop"), Some(json!({})))?;
        self.wait_state(keys, id, &["archived"], Duration::from_secs(180)).map(|_| ())
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        // Accepted for background deletion; the sandbox leaves every list at once.
        self.request(keys, reqwest::Method::DELETE, &format!("/sandboxes/{id}"), None).map(|_| ())
    }

    fn connect(&self, keys: &Keys, id: &str, dir: &Path) -> Result<Endpoint> {
        let (identity, public) = ensure_keypair(dir)?;
        let reply = self.request(
            keys,
            reqwest::Method::POST,
            &format!("/sandboxes/{id}/sshkey"),
            Some(json!({ "key": public })),
        )?;
        parse_sshkey(&reply, &identity).map(Endpoint::Ssh)
    }
}

fn box_info(s: &Value) -> Option<BoxInfo> {
    let id = s["id"].as_str()?.to_string();
    let state = s["state"].as_str().unwrap_or("unknown").to_string();
    Some(BoxInfo {
        name: s["name"].as_str().unwrap_or(&id).to_string(),
        running: UP.contains(&state.as_str()),
        state,
        id,
    })
}

/// Boat's error envelope (`{code, message}` at the top or under `error`) →
/// `code: message (HTTP n)`.
fn api_error(status: u16, v: &Value) -> String {
    let code = v["code"].as_str().or(v["error"]["code"].as_str()).unwrap_or("error");
    let msg = v["message"].as_str().or(v["error"]["message"].as_str()).unwrap_or("request failed");
    format!("{code}: {msg} (HTTP {status})")
}

/// The SSH target from a `sshkey` reply. `sshEndpoint` (`host:port`) wins
/// over `machineIp` when Boat sets it; the host key is pinned.
fn parse_sshkey(reply: &Value, identity: &Path) -> Result<SshTarget> {
    let (host, port) = match reply["sshEndpoint"].as_str().filter(|e| !e.is_empty()) {
        Some(ep) => {
            let (h, p) = ep.rsplit_once(':').context("Boat gave an SSH endpoint without a port")?;
            (h.to_string(), Some(p.parse::<u16>().context("Boat gave an unreadable SSH port")?))
        }
        None => {
            (reply["machineIp"].as_str().filter(|h| !h.is_empty()).context("Boat gave no SSH host")?.to_string(), None)
        }
    };
    let user = reply["sshUser"].as_str().unwrap_or("user").to_string();
    let known_host = reply["hostKey"].as_str().filter(|k| !k.is_empty()).map(|key| {
        let pattern = match port {
            Some(p) if p != 22 => format!("[{host}]:{p}"),
            _ => host.clone(),
        };
        format!("{pattern} {}", key.trim())
    });
    Ok(SshTarget { host, port, user, identity: Some(identity.to_path_buf()), certificate: None, known_host })
}

/// The box's own ed25519 key pair in `dir` (made once with the system
/// `ssh-keygen`): the private key's path and the public key line.
pub(super) fn ensure_keypair(dir: &Path) -> Result<(PathBuf, String)> {
    super::create_private_dir(dir)?;
    let identity = dir.join("id_ed25519");
    if !identity.exists() {
        let mut cmd = std::process::Command::new("ssh-keygen");
        cmd.args(["-q", "-t", "ed25519", "-N", "", "-C", "keplorayx", "-f"]).arg(&identity);
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let out = cmd.output().context("running ssh-keygen (is OpenSSH installed?)")?;
        if !out.status.success() {
            bail!("ssh-keygen failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
    }
    let public = std::fs::read_to_string(identity.with_extension("pub")).context("reading the box's public key")?;
    Ok((identity, public.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_sshkey_reply_with_a_public_ip() {
        let reply = json!({
            "ok": true, "type": "ssh_key.configured", "success": true,
            "machineIp": "203.0.113.10", "sshUser": "user", "sshEndpoint": null,
            "hostKey": "ssh-ed25519 AAAAHOST"
        });
        let t = parse_sshkey(&reply, Path::new("/d/id_ed25519")).unwrap();
        assert_eq!((t.host.as_str(), t.port, t.user.as_str()), ("203.0.113.10", None, "user"));
        assert_eq!(t.known_host.as_deref(), Some("203.0.113.10 ssh-ed25519 AAAAHOST"));
        assert_eq!(t.identity.as_deref(), Some(Path::new("/d/id_ed25519")));
    }

    #[test]
    fn an_ssh_endpoint_wins_and_pins_with_its_port() {
        let reply = json!({
            "machineIp": null, "sshUser": "user", "sshEndpoint": "203.0.113.10:22001",
            "hostKey": "ssh-ed25519 AAAAHOST\n"
        });
        let t = parse_sshkey(&reply, Path::new("/d/k")).unwrap();
        assert_eq!((t.host.as_str(), t.port), ("203.0.113.10", Some(22001)));
        assert_eq!(t.known_host.as_deref(), Some("[203.0.113.10]:22001 ssh-ed25519 AAAAHOST"));
        assert!(parse_sshkey(&json!({"sshUser": "user"}), Path::new("/d/k")).is_err());
    }

    #[test]
    fn sandboxes_become_boxes() {
        let b = box_info(&json!({"id": "bx_23456789", "name": "Sandbox 1", "state": "idle"})).unwrap();
        assert_eq!(
            b,
            BoxInfo { id: "bx_23456789".into(), name: "Sandbox 1".into(), state: "idle".into(), running: true }
        );
        let b = box_info(&json!({"id": "bx_23456789", "state": "archived"})).unwrap();
        assert!(!b.running);
        assert_eq!(b.name, "bx_23456789");
        assert!(box_info(&json!({"name": "x"})).is_none());
    }

    #[test]
    fn errors_read_like_sentences() {
        let v = json!({"ok": false, "code": "invalid_key", "message": "API key is invalid",
            "error": {"code": "invalid_key", "message": "API key is invalid", "status": 401}});
        assert_eq!(api_error(401, &v), "invalid_key: API key is invalid (HTTP 401)");
        assert_eq!(api_error(500, &Value::Null), "error: request failed (HTTP 500)");
    }

    #[test]
    fn no_key_needs_setup() {
        assert!(matches!(PROVIDER.probe(&Keys::default()), Probe::NeedsSetup(_)));
    }
}
