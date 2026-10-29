//! Blaxel (blaxel.ai): microVM sandboxes that drop to standby when no
//! connection is open and resume in about 25 ms on the next request
//! (route: native, over the sandbox's own process API).
//!
//! - Control plane `https://api.blaxel.ai/v0`, `Authorization: Bearer <key>`
//!   and `X-Blaxel-Workspace` (https://docs.blaxel.ai/api-reference/introduction):
//!   `POST /sandboxes` (https://docs.blaxel.ai/api-reference/compute/create-sandbox),
//!   `GET /sandboxes`, `GET`/`DELETE /sandboxes/{name}`. There is no stop
//!   call (https://docs.blaxel.ai/Sandboxes/Standby-control): standby is
//!   automatic, so [`Provider::sleep`] does nothing.
//! - Process API on the sandbox URL (`metadata.url`, else
//!   `https://run.blaxel.ai/{workspace}/sandboxes/{name}`; spec:
//!   https://github.com/blaxel-ai/sandbox/blob/main/sandbox-api/docs/openapi.yml):
//!   `POST /process` with `stdin: true`, raw bytes to
//!   `POST /process/{name}/stdin`, EOF with `DELETE /process/{name}/stdin`,
//!   output from `GET /process/{name}/logs/stream` as NDJSON
//!   (`{"type":"stdout"|"stderr","data":…,"encoding"?:"base64"}`), and the
//!   exit code from `GET /process/{name}`. The process gets one command
//!   string, so the argv is shell-quoted.
//!
//! The key is `BL_API_KEY|BL_WORKSPACE`. Uncertain: whether the log stream
//! replays output written before it was opened (it is opened right after
//! the process starts) and whether `blaxel/base-image` ships git.

use std::io::BufRead as _;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::e2b::stdio;
use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

const API: &str = "https://api.blaxel.ai/v0";
const DEFAULT_IMAGE: &str = "blaxel/base-image:latest";

pub struct Blaxel;
pub static PROVIDER: Blaxel = Blaxel;

/// `BL_API_KEY|BL_WORKSPACE`.
fn parse_key(raw: &str) -> Option<(&str, &str)> {
    let (k, w) = raw.split_once('|')?;
    let (k, w) = (k.trim(), w.trim());
    (!k.is_empty() && !w.is_empty()).then_some((k, w))
}

struct Auth {
    key: String,
    workspace: String,
}

impl Auth {
    fn apply(&self, req: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        req.bearer_auth(&self.key).header("X-Blaxel-Workspace", &self.workspace)
    }
}

impl Blaxel {
    fn auth(&self, keys: &Keys) -> Result<Auth> {
        let raw = keys.get(self.id()).context("no Blaxel key: add BL_API_KEY|BL_WORKSPACE in Settings › Machines")?;
        let (key, workspace) = parse_key(raw).context("the Blaxel key must look like BL_API_KEY|BL_WORKSPACE")?;
        Ok(Auth { key: key.into(), workspace: workspace.into() })
    }

    fn send(&self, keys: &Keys, method: reqwest::Method, url: &str, body: Option<Value>) -> Result<(u16, Value)> {
        let auth = self.auth(keys)?;
        let mut req = auth.apply(super::http().request(method, url));
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().with_context(|| format!("calling Blaxel {}", url.trim_start_matches(API)))?;
        let status = resp.status().as_u16();
        let text = resp.text().unwrap_or_default();
        Ok((status, serde_json::from_str(&text).unwrap_or_else(|_| json!({ "message": text.trim() }))))
    }

    fn request(&self, keys: &Keys, method: reqwest::Method, url: &str, body: Option<Value>) -> Result<Value> {
        let (status, v) = self.send(keys, method, url, body)?;
        if !(200..300).contains(&status) {
            bail!("Blaxel {}: {}", url.trim_start_matches(API), api_error(status, &v));
        }
        Ok(v)
    }

    fn sandbox(&self, keys: &Keys, name: &str) -> Result<Value> {
        self.request(keys, reqwest::Method::GET, &format!("{API}/sandboxes/{name}"), None)
    }

    fn wait_deployed(&self, keys: &Keys, name: &str, limit: Duration) -> Result<Value> {
        let deadline = Instant::now() + limit;
        loop {
            let s = self.sandbox(keys, name)?;
            match s["status"].as_str().unwrap_or_default() {
                "DEPLOYED" => return Ok(s),
                "FAILED" | "TERMINATED" | "DELETING" => {
                    bail!("Blaxel sandbox {name} is {}", s["status"].as_str().unwrap_or("unavailable").to_lowercase())
                }
                _ => {}
            }
            if Instant::now() >= deadline {
                bail!("Blaxel sandbox {name} was not ready after {}s", limit.as_secs());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

impl Provider for Blaxel {
    fn id(&self) -> &'static str {
        "blaxel"
    }

    fn label(&self) -> &'static str {
        "Blaxel"
    }

    fn key_url(&self) -> &'static str {
        "https://app.blaxel.ai"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, keys: &Keys) -> Probe {
        let Some(raw) = keys.get(self.id()) else {
            return Probe::NeedsSetup("Add your Blaxel key as BL_API_KEY|BL_WORKSPACE".into());
        };
        if parse_key(raw).is_none() {
            return Probe::NeedsSetup("The Blaxel key must look like BL_API_KEY|BL_WORKSPACE".into());
        }
        match self.send(keys, reqwest::Method::GET, &format!("{API}/sandboxes"), None) {
            Ok((200..=299, _)) => Probe::Ready,
            Ok((401 | 403, _)) => Probe::NeedsSetup("Blaxel rejected the key or workspace".into()),
            Ok((status, v)) => Probe::Unavailable(format!("Blaxel: {}", api_error(status, &v))),
            Err(err) => Probe::Unavailable(format!("{err:#}")),
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        let v = self.request(keys, reqwest::Method::GET, &format!("{API}/sandboxes"), None)?;
        // Older API versions return an array; newer ones `{data, meta}`.
        let items = v.as_array().or(v["data"].as_array());
        Ok(items.into_iter().flatten().filter_map(box_info).collect())
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let name = sandbox_name(spec.name.as_deref());
        let body = json!({
            "metadata": { "name": name, "labels": { "created-by": "keplorayx" } },
            "spec": { "runtime": { "image": spec.template.as_deref().unwrap_or(DEFAULT_IMAGE), "memory": 4096 } },
        });
        self.request(keys, reqwest::Method::POST, &format!("{API}/sandboxes"), Some(body))?;
        let s = self.wait_deployed(keys, &name, Duration::from_secs(180))?;
        box_info(&s).context("Blaxel returned an unreadable sandbox")
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        // Standby resumes on the next request; wait out a deploy in progress.
        self.wait_deployed(keys, id, Duration::from_secs(180)).map(|_| ())
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        // Automatic: a sandbox with no open connection goes to standby.
        Ok(())
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        match self.send(keys, reqwest::Method::DELETE, &format!("{API}/sandboxes/{id}"), None)? {
            (200..=299 | 404, _) => Ok(()),
            (status, v) => bail!("Blaxel delete: {}", api_error(status, &v)),
        }
    }

    fn connect(&self, keys: &Keys, id: &str, _dir: &Path) -> Result<Endpoint> {
        self.wait_deployed(keys, id, Duration::from_secs(60))?;
        Ok(Endpoint::Native)
    }

    fn bridge(&self, keys: &Keys, id: &str, argv: &[String]) -> Result<i32> {
        if argv.is_empty() {
            bail!("nothing to run");
        }
        let auth = self.auth(keys)?;
        let s = self.sandbox(keys, id)?;
        let base = data_plane_url(&s, &auth.workspace, id);
        let name = format!("keplorayx-{}", uuid::Uuid::now_v7().simple());
        let start = json!({
            "command": stdio::sh_join(argv),
            "name": name,
            "waitForCompletion": false,
            "stdin": true,
            "keepAlive": true,
        });
        self.request(keys, reqwest::Method::POST, &format!("{base}/process"), Some(start))?;

        // Stdin: raw bytes in order, then EOF.
        {
            let auth = Auth { key: auth.key.clone(), workspace: auth.workspace.clone() };
            let url = format!("{base}/process/{name}/stdin");
            std::thread::spawn(move || {
                let client = super::http();
                let chunks = stdio::stdin_chunks();
                while let Ok(Some(chunk)) = chunks.recv() {
                    let sent = auth
                        .apply(client.post(&url))
                        .header("Content-Type", "application/octet-stream")
                        .body(chunk)
                        .send()
                        .is_ok_and(|r| r.status().is_success());
                    if !sent {
                        return;
                    }
                }
                let _ = auth.apply(client.delete(&url)).send();
            });
        }

        let resp = auth
            .apply(stdio::streaming_client().get(format!("{base}/process/{name}/logs/stream")))
            .header("Accept", "application/x-ndjson")
            .send()
            .context("opening the Blaxel log stream")?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let v: Value = serde_json::from_str(&resp.text().unwrap_or_default()).unwrap_or(Value::Null);
            bail!("Blaxel log stream: {}", api_error(status, &v));
        }
        for line in std::io::BufReader::new(resp).split(b'\n') {
            let line = line.context("reading the Blaxel log stream")?;
            match log_line(&line) {
                Log::Stdout(b) => stdio::out(&b)?,
                Log::Stderr(b) => stdio::err(&b)?,
                Log::Error(why) => bail!("Blaxel process stream: {why}"),
                Log::Ignore => {}
            }
        }

        // The stream closes when the process exits; read its exit code.
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let p = self.request(keys, reqwest::Method::GET, &format!("{base}/process/{name}"), None)?;
            if p["status"] != "running" || Instant::now() >= deadline {
                return Ok(exit_code(&p));
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }
}

fn data_plane_url(s: &Value, workspace: &str, name: &str) -> String {
    match s["metadata"]["url"].as_str().filter(|u| !u.is_empty()) {
        Some(u) => u.trim_end_matches('/').to_string(),
        None => format!("https://run.blaxel.ai/{workspace}/sandboxes/{name}"),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Log {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Error(String),
    Ignore,
}

fn log_line(line: &[u8]) -> Log {
    let Ok(v) = serde_json::from_slice::<Value>(line) else { return Log::Ignore };
    let bytes = || {
        let d = v["data"].as_str().unwrap_or_default();
        if v["encoding"] == "base64" { stdio::b64_decode(d).unwrap_or_default() } else { d.as_bytes().to_vec() }
    };
    match v["type"].as_str() {
        Some("stdout") => Log::Stdout(bytes()),
        Some("stderr") => Log::Stderr(bytes()),
        Some("error") => Log::Error(v["data"].as_str().map(String::from).unwrap_or_else(|| v["data"].to_string())),
        _ => Log::Ignore,
    }
}

fn exit_code(p: &Value) -> i32 {
    match p["exitCode"].as_i64() {
        Some(c) => c as i32,
        None if p["status"] == "completed" => 0,
        None => 1,
    }
}

fn box_info(s: &Value) -> Option<BoxInfo> {
    let name = s["metadata"]["name"].as_str()?.to_string();
    let status = s["status"].as_str().unwrap_or("UNKNOWN");
    // `state` (RUNNING/STANDBY) is finer than `status` once deployed.
    let state = match s["state"].as_str() {
        Some(st) if status == "DEPLOYED" => st,
        _ => status,
    }
    .to_ascii_lowercase();
    Some(BoxInfo {
        id: name.clone(),
        name: s["metadata"]["displayName"].as_str().unwrap_or(&name).to_string(),
        running: status == "DEPLOYED" && state != "standby",
        state,
    })
}

/// Names: lowercase letters, digits, dashes; at most 49 characters.
fn sandbox_name(wanted: Option<&str>) -> String {
    let cleaned: String = wanted
        .unwrap_or_default()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(49)
        .collect();
    if cleaned.is_empty() {
        let id = uuid::Uuid::now_v7().simple().to_string();
        format!("keplorayx-{}", &id[id.len() - 8..])
    } else {
        cleaned
    }
}

fn api_error(status: u16, v: &Value) -> String {
    let msg =
        v["error"]["message"].as_str().or(v["message"].as_str()).or(v["error"].as_str()).unwrap_or("request failed");
    format!("{msg} (HTTP {status})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_needs_workspace() {
        assert_eq!(parse_key("bl_k | ws"), Some(("bl_k", "ws")));
        assert_eq!(parse_key("bl_k"), None);
        assert_eq!(parse_key("bl_k|"), None);
    }

    #[test]
    fn reads_log_lines() {
        assert_eq!(log_line(br#"{"type":"stdout","data":"hi\n"}"#), Log::Stdout(b"hi\n".to_vec()));
        assert_eq!(log_line(br#"{"type":"stderr","data":"aGkK","encoding":"base64"}"#), Log::Stderr(b"hi\n".to_vec()));
        assert_eq!(log_line(br#"{"type":"keepalive"}"#), Log::Ignore);
        assert_eq!(log_line(br#"{"type":"error","data":"boom"}"#), Log::Error("boom".into()));
        assert_eq!(log_line(b""), Log::Ignore);
    }

    #[test]
    fn exit_codes_and_urls() {
        assert_eq!(exit_code(&json!({"status": "failed", "exitCode": 2})), 2);
        assert_eq!(exit_code(&json!({"status": "completed"})), 0);
        assert_eq!(exit_code(&json!({"status": "killed"})), 1);
        let s = json!({"metadata": {"name": "a", "url": "https://sbx-a.bl.run/"}});
        assert_eq!(data_plane_url(&s, "ws", "a"), "https://sbx-a.bl.run");
        assert_eq!(data_plane_url(&json!({}), "ws", "a"), "https://run.blaxel.ai/ws/sandboxes/a");
    }

    #[test]
    fn sandboxes_become_boxes() {
        let b = box_info(&json!({"metadata": {"name": "a"}, "status": "DEPLOYED", "state": "STANDBY"})).unwrap();
        assert_eq!(b, BoxInfo { id: "a".into(), name: "a".into(), state: "standby".into(), running: false });
        let b = box_info(&json!({"metadata": {"name": "a"}, "status": "DEPLOYED", "state": "RUNNING"})).unwrap();
        assert!(b.running);
        assert_eq!(box_info(&json!({"metadata": {"name": "a"}, "status": "DEPLOYING"})).unwrap().state, "deploying");
        assert_eq!(sandbox_name(Some("My Box")), "my-box");
    }

    #[test]
    fn no_key_needs_setup() {
        assert!(matches!(PROVIDER.probe(&Keys::default()), Probe::NeedsSetup(_)));
    }
}
