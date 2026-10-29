//! Runloop (runloop.ai): devboxes over a REST API, reached by SSH through a
//! TLS proxy. Paths and fields come from the official Python client
//! (github.com/runloopai/api-client-python, `resources/devboxes/devboxes.py`)
//! and https://docs.runloop.ai/docs/devboxes/ssh.
//!
//! Connecting: `POST /v1/devboxes/{id}/create_ssh_key` returns the devbox's
//! private key, SSH user and a per-devbox `url`. Runloop's SSH listens behind
//! a TLS front door on `ssh.runloop.ai:443` that routes on SNI, so the config
//! uses the same `ProxyCommand openssl s_client -quiet -servername %h -connect
//! ssh.runloop.ai:443` that `rli devbox ssh --config-only` prints
//! (github.com/runloopai/rl-cli, `src/utils/ssh.ts`). That needs `openssl` on
//! this machine; the probe says so when it is missing. The key lives 0600 in
//! the box's folder and the host key is pinned on first use.
//!
//! Sleep is `suspend` (disk kept, processes not); new devboxes suspend after
//! 30 idle minutes. Whether the default image ships `git` is not documented.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider, SSH_ALIAS};

const API: &str = "https://api.runloop.ai";
const SSH_FRONT_DOOR: &str = "ssh.runloop.ai:443";

pub struct Runloop;
pub static PROVIDER: Runloop = Runloop;

impl Runloop {
    fn key<'a>(&self, keys: &'a Keys) -> Result<&'a str> {
        keys.get(self.id()).context("no Runloop API key: add one in Settings › Machines")
    }

    fn request(&self, keys: &Keys, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let key = self.key(keys)?;
        let mut req = super::http().request(method, format!("{API}{path}")).bearer_auth(key);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().with_context(|| format!("calling Runloop {path}"))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            bail!("Runloop {path}: {}", api_error(status.as_u16(), &value, &text));
        }
        Ok(value)
    }

    fn devbox(&self, keys: &Keys, id: &str) -> Result<Value> {
        self.request(keys, reqwest::Method::GET, &format!("/v1/devboxes/{id}"), None)
    }

    /// Poll until the devbox reaches `want`.
    fn wait_status(&self, keys: &Keys, id: &str, want: &str, limit: Duration) -> Result<Value> {
        let deadline = Instant::now() + limit;
        loop {
            let d = self.devbox(keys, id)?;
            let status = d["status"].as_str().unwrap_or("unknown");
            if status == want {
                return Ok(d);
            }
            if matches!(status, "failure" | "shutdown") {
                let why = d["failure_reason"].as_str().or(d["shutdown_reason"].as_str()).unwrap_or("no reason given");
                bail!("Runloop devbox {id} is {status}: {why}");
            }
            if Instant::now() >= deadline {
                bail!("Runloop devbox {id} was still {status} after {}s", limit.as_secs());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

impl Provider for Runloop {
    fn id(&self) -> &'static str {
        "runloop"
    }

    fn label(&self) -> &'static str {
        "Runloop"
    }

    fn key_url(&self) -> &'static str {
        "https://platform.runloop.ai/settings"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, keys: &Keys) -> Probe {
        if !keys.has(self.id()) {
            return Probe::NeedsSetup("Add your Runloop API key".into());
        }
        if which::which("openssl").is_err() {
            return Probe::NeedsSetup("Install OpenSSL: Runloop's SSH runs through `openssl s_client`".into());
        }
        match self.request(keys, reqwest::Method::GET, "/v1/devboxes?limit=1", None) {
            Ok(_) => Probe::Ready,
            Err(err) if err.to_string().contains("HTTP 401") => Probe::NeedsSetup("Runloop rejected the key".into()),
            Err(err) => Probe::Unavailable(format!("{err:#}")),
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        let mut out: Vec<BoxInfo> = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..5 {
            let path = match &after {
                Some(a) => format!("/v1/devboxes?limit=100&starting_after={a}"),
                None => "/v1/devboxes?limit=100".to_string(),
            };
            let page = self.request(keys, reqwest::Method::GET, &path, None)?;
            let items = page["devboxes"].as_array().cloned().unwrap_or_default();
            after = items.last().and_then(|d| d["id"].as_str()).map(String::from);
            // Shut-down devboxes are gone for good; leave them out.
            out.extend(items.iter().filter_map(box_info).filter(|b| b.state != "shutdown"));
            if page["has_more"] != true || after.is_none() {
                break;
            }
        }
        Ok(out)
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let mut body = serde_json::Map::new();
        body.insert("metadata".into(), json!({ "created-by": "keplorayx" }));
        body.insert(
            "launch_parameters".into(),
            json!({ "after_idle": { "idle_time_seconds": 1800, "on_idle": "suspend" } }),
        );
        if let Some(n) = &spec.name {
            body.insert("name".into(), json!(n));
        }
        // A template is a blueprint id (`bpt_…`) or name.
        if let Some(t) = &spec.template {
            let field = if t.starts_with("bpt_") { "blueprint_id" } else { "blueprint_name" };
            body.insert(field.into(), json!(t));
        }
        let d = self.request(keys, reqwest::Method::POST, "/v1/devboxes", Some(Value::Object(body)))?;
        let id = d["id"].as_str().context("Runloop did not say which devbox it created")?.to_string();
        let d = self.wait_status(keys, &id, "running", Duration::from_secs(300))?;
        box_info(&d).context("Runloop returned an unreadable devbox")
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        let d = self.devbox(keys, id)?;
        let status = d["status"].as_str().unwrap_or("unknown");
        match status {
            "running" => return Ok(()),
            "suspending" => {
                self.wait_status(keys, id, "suspended", Duration::from_secs(300))?;
                self.request(keys, reqwest::Method::POST, &format!("/v1/devboxes/{id}/resume"), None)?;
            }
            "suspended" => {
                self.request(keys, reqwest::Method::POST, &format!("/v1/devboxes/{id}/resume"), None)?;
            }
            "failure" | "shutdown" => bail!("Runloop devbox {id} is {status}"),
            _ => {}
        }
        self.wait_status(keys, id, "running", Duration::from_secs(300)).map(|_| ())
    }

    fn sleep(&self, keys: &Keys, id: &str) -> Result<()> {
        self.request(keys, reqwest::Method::POST, &format!("/v1/devboxes/{id}/suspend"), None)?;
        self.wait_status(keys, id, "suspended", Duration::from_secs(180)).map(|_| ())
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        self.request(keys, reqwest::Method::POST, &format!("/v1/devboxes/{id}/shutdown"), None).map(|_| ())
    }

    fn connect(&self, keys: &Keys, id: &str, dir: &Path) -> Result<Endpoint> {
        let reply = self.request(keys, reqwest::Method::POST, &format!("/v1/devboxes/{id}/create_ssh_key"), None)?;
        let key = reply["ssh_private_key"].as_str().filter(|k| !k.is_empty()).context("Runloop gave no SSH key")?;
        let identity = dir.join("id_runloop");
        let mut pem = key.to_string();
        if !pem.ends_with('\n') {
            pem.push('\n');
        }
        super::write_private(&identity, pem.as_bytes())?;
        // The user is in the key reply on current APIs; older ones only set
        // it on the devbox (`rli` falls back to `user`).
        let user = match reply["ssh_user"].as_str().filter(|u| !u.is_empty()) {
            Some(u) => u.to_string(),
            None => self.devbox(keys, id)?["launch_parameters"]["user_parameters"]["username"]
                .as_str()
                .unwrap_or("user")
                .to_string(),
        };
        let url = reply["url"].as_str().filter(|u| !u.is_empty()).context("Runloop gave no SSH address")?;
        Ok(Endpoint::SshConfig { config: ssh_config(url, &user, &identity, dir) })
    }
}

/// A `Host dove-box` block that tunnels through Runloop's TLS front door.
/// `%h` is the per-devbox `url`, which is also the SNI name the proxy routes on.
fn ssh_config(url: &str, user: &str, identity: &Path, dir: &Path) -> String {
    let known = dir.join("known_hosts");
    format!(
        "Host {SSH_ALIAS}\n  HostName {url}\n  User {user}\n  IdentityFile \"{}\"\n  IdentitiesOnly yes\n  \
         UserKnownHostsFile \"{}\"\n  StrictHostKeyChecking accept-new\n  \
         ProxyCommand openssl s_client -quiet -servername %h -connect {SSH_FRONT_DOOR}\n  \
         ServerAliveInterval 30\n  ServerAliveCountMax 4\n",
        identity.display(),
        known.display()
    )
}

fn box_info(d: &Value) -> Option<BoxInfo> {
    let id = d["id"].as_str()?.to_string();
    let state = d["status"].as_str().unwrap_or("unknown").to_string();
    Some(BoxInfo {
        name: d["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&id).to_string(),
        running: state == "running",
        state,
        id,
    })
}

/// Runloop errors carry `message` (sometimes `error`); otherwise the body.
fn api_error(status: u16, v: &Value, text: &str) -> String {
    let msg = v["message"]
        .as_str()
        .or(v["error"].as_str())
        .or(v["error"]["message"].as_str())
        .map(String::from)
        .unwrap_or_else(|| {
            let t = text.trim();
            if t.is_empty() { "request failed".into() } else { t.chars().take(200).collect() }
        });
    format!("{msg} (HTTP {status})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_tunnels_through_tls_and_pins_hosts() {
        let c = ssh_config("dbx_1.ssh.runloop.ai", "user", Path::new("/d/id_runloop"), Path::new("/d"));
        assert!(c.starts_with("Host dove-box\n  HostName dbx_1.ssh.runloop.ai\n  User user\n"));
        assert!(c.contains("IdentityFile \"/d/id_runloop\"\n  IdentitiesOnly yes"));
        assert!(c.contains("ProxyCommand openssl s_client -quiet -servername %h -connect ssh.runloop.ai:443\n"));
        assert!(c.contains(&format!("UserKnownHostsFile \"{}\"", Path::new("/d").join("known_hosts").display())));
        assert_eq!(c.matches("Host ").count(), 1);
    }

    #[test]
    fn devboxes_become_boxes() {
        let b = box_info(&json!({"id": "dbx_1", "name": "a", "status": "running"})).unwrap();
        assert_eq!(b, BoxInfo { id: "dbx_1".into(), name: "a".into(), state: "running".into(), running: true });
        let b = box_info(&json!({"id": "dbx_2", "name": null, "status": "suspended"})).unwrap();
        assert_eq!((b.name.as_str(), b.running), ("dbx_2", false));
        assert!(box_info(&json!({"status": "running"})).is_none());
    }

    #[test]
    fn errors_read_like_sentences() {
        assert_eq!(api_error(404, &json!({"message": "Devbox not found"}), ""), "Devbox not found (HTTP 404)");
        assert_eq!(api_error(502, &Value::Null, "Bad gateway"), "Bad gateway (HTTP 502)");
        assert_eq!(api_error(500, &Value::Null, ""), "request failed (HTTP 500)");
    }

    #[test]
    fn no_key_needs_setup() {
        assert!(matches!(PROVIDER.probe(&Keys::default()), Probe::NeedsSetup(_)));
    }
}
