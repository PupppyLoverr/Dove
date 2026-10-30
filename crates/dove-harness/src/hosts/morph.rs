//! Morph Cloud (morph.so): VM instances started from snapshots over a REST
//! API, reached through Morph's SSH gateway. Paths and fields come from the
//! official SDK (github.com/morph-labs/morph-python-sdk, `morphcloud/api.py`)
//! and https://cloud.morph.so/docs/documentation/instances/ssh.
//!
//! Connecting: `GET /instance/{id}/ssh/key` returns a per-instance key pair;
//! Keplorayx writes the private key 0600 into the box's folder and dials
//! `ssh <instance-id>@ssh.cloud.morph.so`. The SDK itself authenticates by
//! putting `<id>:<API key>` in the username, which would leak the key into
//! the process list, so it is not used. Unverified: the reply also has a
//! `password` that "may be null"; if Morph ever encrypts the key with it,
//! `BatchMode` fails and the route must switch to registering Keplorayx's
//! own key with `POST /user/ssh-keys`.
//!
//! Creating: a template is a snapshot id. Without one, Keplorayx makes (once)
//! a small snapshot of Morph's `morphvm-minimal` image, found again by its
//! `digest`. Whether that image ships `git` is not documented. Sleep is
//! `pause` (memory and disk kept); delete is `DELETE /instance/{id}`.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider, SshTarget};

const API: &str = "https://cloud.morph.so/api";
const GATEWAY: &str = "ssh.cloud.morph.so";
/// Identifies the default snapshot so every create reuses it.
const BASE_DIGEST: &str = "keplorayx-morphvm-minimal-1";

pub struct Morph;
pub static PROVIDER: Morph = Morph;

impl Morph {
    fn key<'a>(&self, keys: &'a Keys) -> Result<&'a str> {
        keys.get(self.id()).context("no Morph Cloud API key: add one in Settings › Machines")
    }

    fn request(&self, keys: &Keys, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let key = self.key(keys)?;
        let mut req = super::http().request(method, format!("{API}{path}")).bearer_auth(key);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().with_context(|| format!("calling Morph Cloud {path}"))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            bail!("Morph Cloud {path}: {}", api_error(status.as_u16(), &value));
        }
        Ok(value)
    }

    fn instance(&self, keys: &Keys, id: &str) -> Result<Value> {
        self.request(keys, reqwest::Method::GET, &format!("/instance/{id}"), None)
    }

    /// Poll `path` until its `status` is `want`.
    fn wait_status(&self, keys: &Keys, path: &str, want: &str, limit: Duration) -> Result<Value> {
        let deadline = Instant::now() + limit;
        loop {
            let v = self.request(keys, reqwest::Method::GET, path, None)?;
            let status = v["status"].as_str().unwrap_or("unknown");
            if status == want {
                return Ok(v);
            }
            if status == "error" {
                bail!(
                    "Morph Cloud {} {} failed",
                    v["object"].as_str().unwrap_or("object"),
                    v["id"].as_str().unwrap_or(path)
                );
            }
            if Instant::now() >= deadline {
                bail!("Morph Cloud {path} was still {status} after {}s", limit.as_secs());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    /// The small default snapshot, made on first use.
    fn base_snapshot(&self, keys: &Keys) -> Result<String> {
        let found = self.request(keys, reqwest::Method::GET, &format!("/snapshot?digest={BASE_DIGEST}"), None)?;
        if let Some(id) = found["data"].as_array().into_iter().flatten().find_map(|s| s["id"].as_str()) {
            return Ok(id.to_string());
        }
        let snap = self.request(
            keys,
            reqwest::Method::POST,
            "/snapshot",
            Some(json!({
                "image_id": "morphvm-minimal", "vcpus": 1, "memory": 4096, "disk_size": 16384,
                "digest": BASE_DIGEST, "metadata": { "created-by": "keplorayx" }
            })),
        )?;
        let id = snap["id"].as_str().context("Morph Cloud did not say which snapshot it made")?.to_string();
        self.wait_status(keys, &format!("/snapshot/{id}"), "ready", Duration::from_secs(300))?;
        Ok(id)
    }
}

impl Provider for Morph {
    fn id(&self) -> &'static str {
        "morph"
    }

    fn label(&self) -> &'static str {
        "Morph Cloud"
    }

    fn key_url(&self) -> &'static str {
        "https://cloud.morph.so/web/keys"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, keys: &Keys) -> Probe {
        if !keys.has(self.id()) {
            return Probe::NeedsSetup("Add your Morph Cloud API key".into());
        }
        match self.request(keys, reqwest::Method::GET, "/instance", None) {
            Ok(_) => Probe::Ready,
            Err(err) if err.to_string().contains("HTTP 401") || err.to_string().contains("HTTP 403") => {
                Probe::NeedsSetup("Morph Cloud rejected the key".into())
            }
            Err(err) => Probe::Unavailable(format!("{err:#}")),
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        let page = self.request(keys, reqwest::Method::GET, "/instance", None)?;
        Ok(page["data"].as_array().into_iter().flatten().filter_map(box_info).collect())
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let snapshot = match &spec.template {
            Some(t) => t.clone(),
            None => self.base_snapshot(keys)?,
        };
        let mut metadata = serde_json::Map::new();
        metadata.insert("created-by".into(), json!("keplorayx"));
        if let Some(n) = &spec.name {
            metadata.insert("name".into(), json!(n));
        }
        let inst = self.request(
            keys,
            reqwest::Method::POST,
            &format!("/instance?snapshot_id={snapshot}"),
            Some(json!({ "metadata": metadata })),
        )?;
        let id = inst["id"].as_str().context("Morph Cloud did not say which instance it started")?.to_string();
        let inst = self.wait_status(keys, &format!("/instance/{id}"), "ready", Duration::from_secs(300))?;
        box_info(&inst).context("Morph Cloud returned an unreadable instance")
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        let inst = self.instance(keys, id)?;
        match inst["status"].as_str().unwrap_or("unknown") {
            "ready" => return Ok(()),
            "paused" => {
                self.request(keys, reqwest::Method::POST, &format!("/instance/{id}/resume"), None)?;
            }
            "error" => bail!("Morph Cloud instance {id} is in error"),
            _ => {}
        }
        self.wait_status(keys, &format!("/instance/{id}"), "ready", Duration::from_secs(300)).map(|_| ())
    }

    fn sleep(&self, keys: &Keys, id: &str) -> Result<()> {
        self.request(keys, reqwest::Method::POST, &format!("/instance/{id}/pause"), None).map(|_| ())
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        self.request(keys, reqwest::Method::DELETE, &format!("/instance/{id}"), None).map(|_| ())
    }

    fn connect(&self, keys: &Keys, id: &str, dir: &Path) -> Result<Endpoint> {
        let reply = self.request(keys, reqwest::Method::GET, &format!("/instance/{id}/ssh/key"), None)?;
        let identity = dir.join("id_morph");
        super::write_private(&identity, private_key(&reply)?.as_bytes())?;
        Ok(Endpoint::Ssh(SshTarget {
            host: GATEWAY.into(),
            port: None,
            user: id.to_string(),
            identity: Some(identity),
            certificate: None,
            known_host: None,
        }))
    }
}

/// The PEM from an `instance_ssh_key` reply, newline-terminated (OpenSSH
/// rejects a key file without one).
fn private_key(reply: &Value) -> Result<String> {
    let key = reply["private_key"].as_str().filter(|k| !k.is_empty()).context("Morph Cloud gave no SSH key")?;
    Ok(if key.ends_with('\n') { key.to_string() } else { format!("{key}\n") })
}

fn box_info(inst: &Value) -> Option<BoxInfo> {
    let id = inst["id"].as_str()?.to_string();
    let state = inst["status"].as_str().unwrap_or("unknown").to_string();
    Some(BoxInfo {
        name: inst["metadata"]["name"].as_str().unwrap_or(&id).to_string(),
        running: state == "ready",
        state,
        id,
    })
}

/// FastAPI's `{detail}` (a string, or a list of `{msg}`) → `detail (HTTP n)`.
fn api_error(status: u16, v: &Value) -> String {
    let msg = match &v["detail"] {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().filter_map(|i| i["msg"].as_str()).collect::<Vec<_>>().join("; "),
        _ => v["message"].as_str().unwrap_or("request failed").to_string(),
    };
    format!("{msg} (HTTP {status})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instances_become_boxes() {
        let b = box_info(&json!({
            "id": "morphvm_abc", "object": "instance", "status": "ready",
            "metadata": {"name": "agent box", "created-by": "keplorayx"}
        }))
        .unwrap();
        assert_eq!(
            b,
            BoxInfo { id: "morphvm_abc".into(), name: "agent box".into(), state: "ready".into(), running: true }
        );
        let b = box_info(&json!({"id": "morphvm_x", "status": "paused", "metadata": {}})).unwrap();
        assert_eq!((b.name.as_str(), b.running), ("morphvm_x", false));
        assert!(box_info(&json!({"status": "ready"})).is_none());
    }

    #[test]
    fn reads_an_ssh_key_reply() {
        let reply = json!({"object": "instance_ssh_key", "private_key": "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----", "public_key": "ssh-ed25519 AAAA", "password": null});
        assert!(private_key(&reply).unwrap().ends_with("-----END OPENSSH PRIVATE KEY-----\n"));
        assert!(private_key(&json!({"public_key": "ssh-ed25519 AAAA"})).is_err());
    }

    #[test]
    fn errors_read_like_sentences() {
        assert_eq!(api_error(404, &json!({"detail": "Instance not found"})), "Instance not found (HTTP 404)");
        let v = json!({"detail": [{"loc": ["query", "snapshot_id"], "msg": "field required"}]});
        assert_eq!(api_error(422, &v), "field required (HTTP 422)");
    }

    #[test]
    fn no_key_needs_setup() {
        assert!(matches!(PROVIDER.probe(&Keys::default()), Probe::NeedsSetup(_)));
    }
}
