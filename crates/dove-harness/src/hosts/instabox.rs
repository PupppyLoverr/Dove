//! InstaBox (instabox.sh): cloud machines over a REST API with an SSH
//! gateway. Built from the OpenAPI spec at `api.instabox.sh/v1/openapi.json`.
//!
//! Connecting uses InstaBox's certificate flow: Keplorayx keeps an ed25519
//! key per machine (made with the system `ssh-keygen`), asks the API to sign
//! it (`POST /v1/machines/{id}/connections`), and pins the gateway through the
//! `@cert-authority` key in the same reply. So `ssh` runs with `BatchMode`,
//! no password and no token on any command line.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider, SshTarget};

const API: &str = "https://api.instabox.sh";
/// Certificates last a day (the API's maximum); the launcher signs a fresh
/// one before every run, so a long session never meets an expired one.
const CERT_TTL_SECS: u64 = 86_400;

pub struct InstaBox;
pub static INSTABOX: InstaBox = InstaBox;

impl InstaBox {
    fn key<'a>(&self, keys: &'a Keys) -> Result<&'a str> {
        keys.get(self.id()).context("no InstaBox API key: add one in Settings › Machines")
    }

    fn request(&self, keys: &Keys, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let key = self.key(keys)?;
        let mut req = super::http().request(method.clone(), format!("{API}{path}")).header("X-API-Key", key);
        // Lifecycle calls require an idempotency key; a fresh one per call is
        // right because each call here is a new intent.
        if method != reqwest::Method::GET {
            req = req.header("Idempotency-Key", uuid::Uuid::now_v7().to_string());
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().with_context(|| format!("calling InstaBox {path}"))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            bail!("InstaBox {path}: {}", api_error(status.as_u16(), &value));
        }
        Ok(value)
    }

    fn machine(&self, keys: &Keys, id: &str) -> Result<Value> {
        self.request(keys, reqwest::Method::GET, &format!("/v1/machines/{id}"), None)
    }

    /// Long-poll an operation until it settles (lifecycle calls are async).
    fn wait_operation(&self, keys: &Keys, op: &Value, limit: Duration) -> Result<Value> {
        let Some(id) = op["id"].as_str() else { return Ok(op.clone()) };
        let deadline = Instant::now() + limit;
        let mut current = op.clone();
        while Instant::now() < deadline {
            match current["state"].as_str() {
                Some("succeeded") => return Ok(current),
                Some("failed") => {
                    bail!("InstaBox {}: {}", current["type"].as_str().unwrap_or("operation"), api_error(0, &current))
                }
                _ => {}
            }
            current = self.request(keys, reqwest::Method::GET, &format!("/v1/operations/{id}?wait=25s"), None)?;
        }
        bail!("InstaBox operation {id} did not finish in {}s", limit.as_secs())
    }

    /// Poll until exec, files and SSH work (`runtime.ready`).
    fn wait_ready(&self, keys: &Keys, id: &str, limit: Duration) -> Result<Value> {
        let deadline = Instant::now() + limit;
        loop {
            let m = self.machine(keys, id)?;
            if is_ready(&m) {
                return Ok(m);
            }
            if m["state"] == "error" || m["state"] == "deleted" {
                bail!("InstaBox machine {id} is {}", m["state"].as_str().unwrap_or("unavailable"));
            }
            if Instant::now() >= deadline {
                bail!("InstaBox machine {id} was not ready after {}s", limit.as_secs());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

impl Provider for InstaBox {
    fn id(&self) -> &'static str {
        "instabox"
    }

    fn label(&self) -> &'static str {
        "InstaBox"
    }

    fn key_url(&self) -> &'static str {
        "https://instabox.sh"
    }

    fn verified(&self) -> bool {
        false
    }

    fn probe(&self, keys: &Keys) -> Probe {
        if !keys.has(self.id()) {
            return Probe::NeedsSetup("Add your InstaBox API key (ibk_…)".into());
        }
        match self.request(keys, reqwest::Method::GET, "/v1/machines?limit=1", None) {
            Ok(_) => Probe::Ready,
            Err(err) if err.to_string().contains("401") => Probe::NeedsSetup("InstaBox rejected the key".into()),
            Err(err) => Probe::Unavailable(format!("{err:#}")),
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..5 {
            let path = match &cursor {
                Some(c) => format!("/v1/machines?limit=100&cursor={c}"),
                None => "/v1/machines?limit=100".to_string(),
            };
            let page = self.request(keys, reqwest::Method::GET, &path, None)?;
            out.extend(page["machines"].as_array().into_iter().flatten().filter_map(box_info));
            cursor = page["nextCursor"].as_str().map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        Ok(out)
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let mut body = serde_json::Map::new();
        if let Some(n) = &spec.name {
            body.insert("name".into(), json!(n));
        }
        if let Some(t) = &spec.template {
            body.insert("templateId".into(), json!(t));
        }
        let op = self.request(keys, reqwest::Method::POST, "/v1/machines", Some(Value::Object(body)))?;
        let op = self.wait_operation(keys, &op, Duration::from_secs(300))?;
        let id = op["machineId"].as_str().context("InstaBox did not say which machine it created")?.to_string();
        let m = self.wait_ready(keys, &id, Duration::from_secs(180))?;
        box_info(&m).context("InstaBox returned an unreadable machine")
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        let m = self.machine(keys, id)?;
        if is_ready(&m) {
            return Ok(());
        }
        match m["state"].as_str() {
            // `start` also resumes a paused machine.
            Some("stopped" | "suspended") => {
                let op =
                    self.request(keys, reqwest::Method::POST, &format!("/v1/machines/{id}/start"), Some(json!({})))?;
                self.wait_operation(keys, &op, Duration::from_secs(240))?;
            }
            Some("error" | "deleted" | "deleting") => {
                bail!("InstaBox machine {id} is {}", m["state"].as_str().unwrap_or("unavailable"))
            }
            _ => {}
        }
        self.wait_ready(keys, id, Duration::from_secs(180)).map(|_| ())
    }

    fn sleep(&self, keys: &Keys, id: &str) -> Result<()> {
        let op = self.request(keys, reqwest::Method::POST, &format!("/v1/machines/{id}/suspend"), None)?;
        self.wait_operation(keys, &op, Duration::from_secs(120)).map(|_| ())
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        let key = self.key(keys)?;
        let resp = super::http()
            .delete(format!("{API}/v1/machines/{id}"))
            .header("X-API-Key", key)
            .header("Idempotency-Key", uuid::Uuid::now_v7().to_string())
            .header("X-Confirm-Delete", id)
            .send()
            .context("calling InstaBox delete")?;
        let status = resp.status();
        let value: Value = resp.json().unwrap_or(Value::Null);
        if !status.is_success() {
            bail!("InstaBox delete: {}", api_error(status.as_u16(), &value));
        }
        self.wait_operation(keys, &value, Duration::from_secs(120)).map(|_| ())
    }

    fn connect(&self, keys: &Keys, id: &str, dir: &Path) -> Result<Endpoint> {
        let identity = dir.join("id_ed25519");
        if !identity.exists() {
            keygen(&identity)?;
        }
        let public = std::fs::read_to_string(identity.with_extension("pub")).context("reading the box's public key")?;
        let reply = self.request(
            keys,
            reqwest::Method::POST,
            &format!("/v1/machines/{id}/connections"),
            Some(json!({ "publicKey": public.trim(), "ttlSeconds": CERT_TTL_SECS })),
        )?;
        let target = parse_connection(&reply, &identity)?;
        let cert = target.certificate.clone().context("no certificate path")?;
        super::write_private(&cert, format!("{}\n", reply["certificate"].as_str().unwrap_or_default()).as_bytes())?;
        Ok(Endpoint::Ssh(target))
    }
}

fn is_ready(m: &Value) -> bool {
    m["state"] == "running" && m["runtime"]["ready"] == true
}

fn box_info(m: &Value) -> Option<BoxInfo> {
    let id = m["id"].as_str()?.to_string();
    let state = m["state"].as_str().unwrap_or("unknown").to_string();
    Some(BoxInfo { name: m["name"].as_str().unwrap_or(&id).to_string(), running: state == "running", state, id })
}

/// `{"error":{"code","message"}}` → `code: message (HTTP n)`.
fn api_error(status: u16, v: &Value) -> String {
    let e = &v["error"];
    let code = e["code"].as_str().unwrap_or("error");
    let msg = e["message"].as_str().unwrap_or("request failed");
    if status == 0 { format!("{code}: {msg}") } else { format!("{code}: {msg} (HTTP {status})") }
}

/// The SSH target from a `connections` reply. The gateway is trusted only
/// through the CA key InstaBox returns.
fn parse_connection(reply: &Value, identity: &Path) -> Result<SshTarget> {
    let host = reply["host"].as_str().context("InstaBox gave no SSH host")?.to_string();
    let port = reply["port"].as_u64().and_then(|p| u16::try_from(p).ok());
    let user = reply["username"].as_str().context("InstaBox gave no SSH username")?.to_string();
    if reply["certificate"].as_str().is_none_or(str::is_empty) {
        bail!("InstaBox did not sign the key");
    }
    let known_host = reply["hostTrust"]["publicKey"].as_str().map(|ca| {
        let pattern = match port {
            Some(p) if p != 22 => format!("[{host}]:{p}"),
            _ => host.clone(),
        };
        format!("@cert-authority {pattern} {ca}")
    });
    Ok(SshTarget {
        host,
        port,
        user,
        identity: Some(identity.to_path_buf()),
        certificate: Some(identity.with_file_name("id_ed25519-cert.pub")),
        known_host,
    })
}

/// `ssh-keygen -t ed25519 -N "" -f <path>`, quietly.
fn keygen(path: &Path) -> Result<()> {
    let mut cmd = std::process::Command::new("ssh-keygen");
    cmd.args(["-q", "-t", "ed25519", "-N", "", "-C", "keplorayx", "-f"]).arg(path);
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_connection_reply() {
        let reply = json!({
            "id": "cn_1", "machineId": "m_abc", "transport": "ssh",
            "host": "api.instabox.sh", "port": 2222, "username": "m_abc",
            "certificate": "ssh-ed25519-cert-v01@openssh.com AAAA",
            "hostTrust": {"type": "ca", "publicKey": "ssh-ed25519 CAKEY", "principals": ["api.instabox.sh"]}
        });
        let t = parse_connection(&reply, Path::new("/d/id_ed25519")).unwrap();
        assert_eq!((t.host.as_str(), t.port, t.user.as_str()), ("api.instabox.sh", Some(2222), "m_abc"));
        assert_eq!(t.known_host.as_deref(), Some("@cert-authority [api.instabox.sh]:2222 ssh-ed25519 CAKEY"));
        assert_eq!(t.certificate.as_deref(), Some(Path::new("/d/id_ed25519-cert.pub")));
        assert!(parse_connection(&json!({"host": "h", "username": "u"}), Path::new("/d/k")).is_err());
    }

    #[test]
    fn readiness_needs_running_and_runtime_ready() {
        assert!(is_ready(&json!({"state": "running", "runtime": {"ready": true}})));
        assert!(!is_ready(&json!({"state": "running", "runtime": {"ready": false}})));
        assert!(!is_ready(&json!({"state": "suspended", "runtime": {"ready": true}})));
    }

    #[test]
    fn machines_become_boxes() {
        let b = box_info(&json!({"id": "m_1", "name": "box-1", "state": "suspended"})).unwrap();
        assert_eq!(b, BoxInfo { id: "m_1".into(), name: "box-1".into(), state: "suspended".into(), running: false });
        assert!(box_info(&json!({"name": "x"})).is_none());
    }

    #[test]
    fn errors_read_like_sentences() {
        let v = json!({"error": {"code": "invalid_key", "message": "API key is invalid"}});
        assert_eq!(api_error(401, &v), "invalid_key: API key is invalid (HTTP 401)");
    }

    #[test]
    fn no_key_needs_setup() {
        assert!(matches!(INSTABOX.probe(&Keys::default()), Probe::NeedsSetup(_)));
    }
}
