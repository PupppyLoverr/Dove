//! Daytona (daytona.io): sandboxes over a REST API with an SSH gateway.
//! Built from the OpenAPI spec at https://www.daytona.io/docs/openapi.json
//! and the SSH guide at https://www.daytona.io/docs/en/ssh-access/.
//!
//! Connecting: `POST /sandbox/{id}/ssh-access` mints a short-lived token, and
//! the token *is* the SSH username (`ssh <token>@ssh.app.daytona.io`). There
//! is no key or password: the gateway accepts SSH "none" auth and checks the
//! token after the handshake (`NoClientAuth` in `apps/ssh-gateway/main.go`
//! at v0.190.0), so `ssh -o BatchMode=yes` should not prompt. Unverified
//! against the live gateway: a rejected token just closes the connection
//! (ssh exits 255 with no message). The token sits only in the box's 0600
//! `ssh_config`, never on a command line. Host and port are read from the
//! returned `sshCommand`, falling back to `ssh.app.daytona.io:22`.
//!
//! The default snapshot runs as user `daytona` (home `/home/daytona`);
//! whether it ships `git` is not documented.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider, SshTarget};

const API: &str = "https://app.daytona.io/api";
const GATEWAY: &str = "ssh.app.daytona.io";
/// The launcher mints a token before every run and the gateway checks it only
/// at connect, so a short life is enough.
const TOKEN_MINUTES: u32 = 120;

pub struct Daytona;
pub static PROVIDER: Daytona = Daytona;

impl Daytona {
    fn key<'a>(&self, keys: &'a Keys) -> Result<&'a str> {
        keys.get(self.id()).context("no Daytona API key: add one in Settings › Machines")
    }

    fn request(&self, keys: &Keys, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let key = self.key(keys)?;
        let mut req = super::http().request(method, format!("{API}{path}")).bearer_auth(key);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().with_context(|| format!("calling Daytona {path}"))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            bail!("Daytona {path}: {}", api_error(status.as_u16(), &value));
        }
        Ok(value)
    }

    fn sandbox(&self, keys: &Keys, id: &str) -> Result<Value> {
        self.request(keys, reqwest::Method::GET, &format!("/sandbox/{id}"), None)
    }

    /// Poll until the sandbox reaches `want`.
    fn wait_state(&self, keys: &Keys, id: &str, want: &str, limit: Duration) -> Result<Value> {
        let deadline = Instant::now() + limit;
        loop {
            let s = self.sandbox(keys, id)?;
            let state = s["state"].as_str().unwrap_or("unknown");
            if state == want {
                return Ok(s);
            }
            if matches!(state, "error" | "build_failed" | "destroyed") {
                let why = s["errorReason"].as_str().unwrap_or("no reason given");
                bail!("Daytona sandbox {id} is {state}: {why}");
            }
            if Instant::now() >= deadline {
                bail!("Daytona sandbox {id} was still {state} after {}s", limit.as_secs());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

impl Provider for Daytona {
    fn id(&self) -> &'static str {
        "daytona"
    }

    fn label(&self) -> &'static str {
        "Daytona"
    }

    fn key_url(&self) -> &'static str {
        "https://app.daytona.io/dashboard/keys"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, keys: &Keys) -> Probe {
        if !keys.has(self.id()) {
            return Probe::NeedsSetup("Add your Daytona API key (dtn_…)".into());
        }
        match self.request(keys, reqwest::Method::GET, "/sandbox?limit=1", None) {
            Ok(_) => Probe::Ready,
            Err(err) if err.to_string().contains("HTTP 401") || err.to_string().contains("HTTP 403") => {
                Probe::NeedsSetup("Daytona rejected the key".into())
            }
            Err(err) => Probe::Unavailable(format!("{err:#}")),
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..5 {
            let path = match &cursor {
                Some(c) => format!("/sandbox?limit=200&cursor={c}"),
                None => "/sandbox?limit=200".to_string(),
            };
            let page = self.request(keys, reqwest::Method::GET, &path, None)?;
            let (items, next) = sandbox_page(&page);
            out.extend(items.iter().filter_map(box_info));
            cursor = next;
            if cursor.is_none() {
                break;
            }
        }
        Ok(out)
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let mut body = serde_json::Map::new();
        body.insert("labels".into(), json!({ "created-by": "keplorayx" }));
        // Stop after 30 idle minutes; a stopped sandbox keeps its disk.
        body.insert("autoStopInterval".into(), json!(30));
        if let Some(n) = &spec.name {
            body.insert("name".into(), json!(n));
        }
        // A template is a snapshot id or name (`daytona-small`, your own…).
        if let Some(t) = &spec.template {
            body.insert("snapshot".into(), json!(t));
        }
        let s = self.request(keys, reqwest::Method::POST, "/sandbox", Some(Value::Object(body)))?;
        let id = s["id"].as_str().context("Daytona did not say which sandbox it created")?.to_string();
        let s = self.wait_state(keys, &id, "started", Duration::from_secs(300))?;
        box_info(&s).context("Daytona returned an unreadable sandbox")
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        let s = self.sandbox(keys, id)?;
        let state = s["state"].as_str().unwrap_or("unknown");
        match state {
            "started" => return Ok(()),
            // `start` also restores an archived sandbox.
            "stopped" | "archived" | "paused" => {
                self.request(keys, reqwest::Method::POST, &format!("/sandbox/{id}/start"), None)?;
            }
            // A start is refused mid-transition; let it settle first.
            "stopping" | "archiving" | "pausing" => {
                let settled = match state {
                    "stopping" => "stopped",
                    "archiving" => "archived",
                    _ => "paused",
                };
                self.wait_state(keys, id, settled, Duration::from_secs(300))?;
                self.request(keys, reqwest::Method::POST, &format!("/sandbox/{id}/start"), None)?;
            }
            "error" | "build_failed" | "destroyed" | "destroying" => bail!("Daytona sandbox {id} is {state}"),
            _ => {}
        }
        self.wait_state(keys, id, "started", Duration::from_secs(300)).map(|_| ())
    }

    fn sleep(&self, keys: &Keys, id: &str) -> Result<()> {
        self.request(keys, reqwest::Method::POST, &format!("/sandbox/{id}/stop"), None)?;
        self.wait_state(keys, id, "stopped", Duration::from_secs(120)).map(|_| ())
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        self.request(keys, reqwest::Method::DELETE, &format!("/sandbox/{id}"), None).map(|_| ())
    }

    fn connect(&self, keys: &Keys, id: &str, _dir: &Path) -> Result<Endpoint> {
        let reply = self.request(
            keys,
            reqwest::Method::POST,
            &format!("/sandbox/{id}/ssh-access?expiresInMinutes={TOKEN_MINUTES}"),
            None,
        )?;
        parse_ssh_access(&reply).map(Endpoint::Ssh)
    }
}

/// One page of `GET /sandbox`: `{items, nextCursor}`, or a bare array from
/// older API versions.
fn sandbox_page(page: &Value) -> (Vec<Value>, Option<String>) {
    match page {
        Value::Array(items) => (items.clone(), None),
        _ => (
            page["items"].as_array().cloned().unwrap_or_default(),
            page["nextCursor"].as_str().filter(|c| !c.is_empty()).map(String::from),
        ),
    }
}

fn box_info(s: &Value) -> Option<BoxInfo> {
    let id = s["id"].as_str()?.to_string();
    let state = s["state"].as_str().unwrap_or("unknown").to_string();
    Some(BoxInfo { name: s["name"].as_str().unwrap_or(&id).to_string(), running: state == "started", state, id })
}

/// NestJS-style `{statusCode, message, error}`, where `message` may be a
/// list → `message (HTTP n)`.
fn api_error(status: u16, v: &Value) -> String {
    let msg = match &v["message"] {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "),
        _ => v["error"].as_str().unwrap_or("request failed").to_string(),
    };
    format!("{msg} (HTTP {status})")
}

/// The SSH target from an `ssh-access` reply: the token is the user, and
/// host/port come from `sshCommand` (`ssh -p 2222 <token>@host`) when given.
fn parse_ssh_access(reply: &Value) -> Result<SshTarget> {
    let token = reply["token"].as_str().filter(|t| !t.is_empty()).context("Daytona gave no SSH token")?.to_string();
    let (host, port) = reply["sshCommand"].as_str().and_then(parse_ssh_command).unwrap_or((GATEWAY.to_string(), None));
    Ok(SshTarget { host, port, user: token, identity: None, certificate: None, known_host: None })
}

/// `ssh [-p N] user@host` → `(host, port)`.
fn parse_ssh_command(cmd: &str) -> Option<(String, Option<u16>)> {
    let words: Vec<&str> = cmd.split_whitespace().collect();
    let port = words.iter().position(|w| *w == "-p").and_then(|i| words.get(i + 1)).and_then(|p| p.parse().ok());
    let host = words.iter().rev().find_map(|w| w.split_once('@').map(|(_, h)| h))?;
    (!host.is_empty()).then(|| (host.to_string(), port.filter(|p| *p != 22)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_ssh_access_reply() {
        let reply = json!({
            "id": "a1", "sandboxId": "s1", "token": "tok123",
            "expiresAt": "2026-10-29T12:00:00Z",
            "sshCommand": "ssh -p 2222 tok123@ssh.app.daytona.io"
        });
        let t = parse_ssh_access(&reply).unwrap();
        assert_eq!((t.host.as_str(), t.port, t.user.as_str()), ("ssh.app.daytona.io", Some(2222), "tok123"));
        assert!(t.identity.is_none());
        let bare = parse_ssh_access(&json!({"token": "t"})).unwrap();
        assert_eq!((bare.host.as_str(), bare.port), (GATEWAY, None));
        assert!(parse_ssh_access(&json!({"sshCommand": "ssh x@y"})).is_err());
    }

    #[test]
    fn reads_ssh_commands() {
        assert_eq!(parse_ssh_command("ssh tok@ssh.app.daytona.io"), Some(("ssh.app.daytona.io".into(), None)));
        assert_eq!(parse_ssh_command("ssh -p 22 tok@h"), Some(("h".into(), None)));
        assert_eq!(parse_ssh_command("ssh -p 2222 tok@localhost"), Some(("localhost".into(), Some(2222))));
        assert_eq!(parse_ssh_command("nonsense"), None);
    }

    #[test]
    fn reads_both_list_shapes() {
        let paged = json!({"items": [{"id": "s1", "name": "a", "state": "started"}], "nextCursor": "c2"});
        let (items, next) = sandbox_page(&paged);
        assert_eq!((items.len(), next.as_deref()), (1, Some("c2")));
        let (items, next) = sandbox_page(&json!([{"id": "s1"}, {"id": "s2"}]));
        assert_eq!((items.len(), next), (2, None));
    }

    #[test]
    fn sandboxes_become_boxes() {
        let b = box_info(&json!({"id": "s1", "name": "box", "state": "started"})).unwrap();
        assert_eq!(b, BoxInfo { id: "s1".into(), name: "box".into(), state: "started".into(), running: true });
        assert!(!box_info(&json!({"id": "s1", "state": "archived"})).unwrap().running);
        assert!(box_info(&json!({"name": "x"})).is_none());
    }

    #[test]
    fn errors_read_like_sentences() {
        let v = json!({"statusCode": 400, "message": ["name must be a string", "cpu too big"], "error": "Bad Request"});
        assert_eq!(api_error(400, &v), "name must be a string; cpu too big (HTTP 400)");
        assert_eq!(api_error(401, &json!({"message": "Unauthorized"})), "Unauthorized (HTTP 401)");
    }

    #[test]
    fn no_key_needs_setup() {
        assert!(matches!(PROVIDER.probe(&Keys::default()), Probe::NeedsSetup(_)));
    }
}
