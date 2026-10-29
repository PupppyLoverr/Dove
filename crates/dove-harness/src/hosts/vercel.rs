//! Vercel Sandbox (vercel.com/docs/vercel-sandbox): persistent Firecracker
//! sandboxes. Each run is a "session"; stopping one snapshots the sandbox
//! and the next request resumes it (route: native, over the REST API).
//!
//! - REST on `https://vercel.com/api`, `Authorization: Bearer <token>`,
//!   `?teamId=` on every call. Paths are read from `@vercel/sandbox`'s
//!   `api-client.js` (https://www.npmjs.com/package/@vercel/sandbox, 3.9):
//!   `POST /v3/sandboxes`, `GET /v2/sandboxes?project=`,
//!   `GET /v2/sandboxes/{name}?projectId=&resume=true`,
//!   `POST /v2/sandboxes/sessions/{session}/stop`, `DELETE /v2/sandboxes/{name}`.
//! - Stdio: `POST …/sessions/{session}/cmd` with `attachStdin: true`, input
//!   as base64 to `POST …/cmd/{id}/stdin` (`offset` makes resends safe,
//!   `close: true` is EOF), output from `GET …/cmd/{id}/logs` (NDJSON
//!   `{"stream":"stdout"|"stderr","data":…}`), and the exit code from
//!   `GET …/cmd/{id}?wait=true`. The `sandbox` CLI's `exec -i` needs a real
//!   TTY and plain `exec` drops stdin, so the CLI can't carry an agent.
//!
//! Session caps (https://vercel.com/docs/vercel-sandbox/pricing): 45 minutes
//! on Hobby, 24 hours on Pro and Enterprise. The bridge extends the session
//! while an agent runs; at the plan's cap Vercel stops it (state is kept in
//! the snapshot). The image is Ubuntu with git; user `ubuntu`, home `/vercel`.
//!
//! The key is `VERCEL_TOKEN|TEAM_ID|PROJECT_ID` (the team may be empty for a
//! personal scope).

use std::io::BufRead as _;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::e2b::stdio;
use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

const API: &str = "https://vercel.com/api";
/// The longest a Hobby session may run; Pro accepts it too.
const SESSION_MS: u64 = 45 * 60 * 1000;

pub struct Vercel;
pub static PROVIDER: Vercel = Vercel;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Auth {
    token: String,
    team: Option<String>,
    project: String,
}

/// `TOKEN|TEAM_ID|PROJECT_ID`.
fn parse_key(raw: &str) -> Option<Auth> {
    let mut parts = raw.split('|').map(str::trim);
    let (token, team, project) = (parts.next()?, parts.next()?, parts.next()?);
    if token.is_empty() || project.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(Auth { token: token.into(), team: (!team.is_empty()).then(|| team.to_string()), project: project.into() })
}

impl Auth {
    /// `API + path`, with `teamId` added to the query.
    fn url(&self, path: &str) -> String {
        match &self.team {
            Some(t) => format!("{API}{path}{}teamId={t}", if path.contains('?') { '&' } else { '?' }),
            None => format!("{API}{path}"),
        }
    }

    fn send(
        &self,
        client: &reqwest::blocking::Client,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(u16, Value)> {
        let mut req = client.request(method, self.url(path)).bearer_auth(&self.token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().with_context(|| format!("calling Vercel {}", path.split('?').next().unwrap_or(path)))?;
        let status = resp.status().as_u16();
        let text = resp.text().unwrap_or_default();
        Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
    }

    fn request(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let (status, v) = self.send(&super::http(), method, path, body)?;
        if !(200..300).contains(&status) {
            bail!("Vercel {}: {}", path.split('?').next().unwrap_or(path), api_error(status, &v));
        }
        Ok(v)
    }

    /// The sandbox and, with `resume`, its running session.
    fn sandbox(&self, name: &str, resume: bool) -> Result<Value> {
        self.request(
            reqwest::Method::GET,
            &format!("/v2/sandboxes/{name}?projectId={}&resume={resume}", self.project),
            None,
        )
    }

    /// Resume `name` and wait for a running session; returns its id.
    fn running_session(&self, name: &str, limit: Duration) -> Result<String> {
        let deadline = Instant::now() + limit;
        loop {
            let v = self.sandbox(name, true)?;
            let session = &v["session"];
            let id = session["id"].as_str().or(v["sandbox"]["currentSessionId"].as_str()).unwrap_or_default();
            match session["status"].as_str().or(v["sandbox"]["status"].as_str()) {
                Some("running") if !id.is_empty() => return Ok(id.to_string()),
                Some("failed" | "aborted") => bail!("Vercel sandbox {name} failed to start"),
                _ => {}
            }
            if Instant::now() >= deadline {
                bail!("Vercel sandbox {name} was not running after {}s", limit.as_secs());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

impl Vercel {
    fn auth(&self, keys: &Keys) -> Result<Auth> {
        let raw = keys.get(self.id()).context("no Vercel key: add TOKEN|TEAM_ID|PROJECT_ID in Settings › Machines")?;
        parse_key(raw).context("the Vercel key must look like TOKEN|TEAM_ID|PROJECT_ID")
    }
}

impl Provider for Vercel {
    fn id(&self) -> &'static str {
        "vercel"
    }

    fn label(&self) -> &'static str {
        "Vercel Sandbox"
    }

    fn key_url(&self) -> &'static str {
        "https://vercel.com/account/tokens"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, keys: &Keys) -> Probe {
        let Some(raw) = keys.get(self.id()) else {
            return Probe::NeedsSetup("Add a Vercel token as TOKEN|TEAM_ID|PROJECT_ID".into());
        };
        let Some(auth) = parse_key(raw) else {
            return Probe::NeedsSetup("The Vercel key must look like TOKEN|TEAM_ID|PROJECT_ID".into());
        };
        let path = format!("/v2/sandboxes?project={}&limit=1", auth.project);
        match auth.send(&super::http(), reqwest::Method::GET, &path, None) {
            Ok((200..=299, _)) => Probe::Ready,
            Ok((401 | 403, _)) => Probe::NeedsSetup("Vercel rejected the token, team or project".into()),
            Ok((status, v)) => Probe::Unavailable(format!("Vercel: {}", api_error(status, &v))),
            Err(err) => Probe::Unavailable(format!("{err:#}")),
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        let auth = self.auth(keys)?;
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..5 {
            let mut path = format!("/v2/sandboxes?project={}&limit=100", auth.project);
            if let Some(c) = &cursor {
                path.push_str(&format!("&cursor={c}"));
            }
            let page = auth.request(reqwest::Method::GET, &path, None)?;
            out.extend(page["sandboxes"].as_array().into_iter().flatten().filter_map(box_info));
            cursor = page["pagination"]["next"].as_str().filter(|c| !c.is_empty()).map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        Ok(out)
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let auth = self.auth(keys)?;
        let mut body = json!({
            "projectId": auth.project,
            "timeout": SESSION_MS,
            "resources": { "vcpus": 2 },
            "persistent": true,
            "tags": { "created-by": "keplorayx" },
        });
        if let Some(n) = &spec.name {
            body["name"] = json!(n);
        }
        if let Some(img) = &spec.template {
            body["image"] = json!(img);
        }
        let v = auth.request(reqwest::Method::POST, "/v3/sandboxes", Some(body))?;
        let name = v["sandbox"]["name"].as_str().context("Vercel did not say which sandbox it created")?.to_string();
        auth.running_session(&name, Duration::from_secs(180))?;
        Ok(BoxInfo { id: name.clone(), name, state: "running".into(), running: true })
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        self.auth(keys)?.running_session(id, Duration::from_secs(180)).map(|_| ())
    }

    fn sleep(&self, keys: &Keys, id: &str) -> Result<()> {
        let auth = self.auth(keys)?;
        let v = auth.sandbox(id, false)?;
        if v["sandbox"]["status"] != "running" {
            return Ok(());
        }
        let session = v["sandbox"]["currentSessionId"].as_str().context("Vercel gave no session to stop")?;
        auth.request(reqwest::Method::POST, &format!("/v2/sandboxes/sessions/{session}/stop"), None).map(|_| ())
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        let auth = self.auth(keys)?;
        let path = format!("/v2/sandboxes/{id}?projectId={}", auth.project);
        match auth.send(&super::http(), reqwest::Method::DELETE, &path, None)? {
            (200..=299 | 404, _) => Ok(()),
            (status, v) => bail!("Vercel delete: {}", api_error(status, &v)),
        }
    }

    fn connect(&self, keys: &Keys, id: &str, _dir: &Path) -> Result<Endpoint> {
        self.wake(keys, id)?;
        Ok(Endpoint::Native)
    }

    fn bridge(&self, keys: &Keys, id: &str, argv: &[String]) -> Result<i32> {
        let (program, args) = argv.split_first().context("nothing to run")?;
        let auth = self.auth(keys)?;
        let session = auth.running_session(id, Duration::from_secs(180))?;
        let base = format!("/v2/sandboxes/sessions/{session}");
        let v = auth.request(
            reqwest::Method::POST,
            &format!("{base}/cmd"),
            Some(json!({ "command": program, "args": args, "attachStdin": true })),
        )?;
        let cmd = v["command"]["id"].as_str().context("Vercel did not return a command id")?.to_string();

        // Keep the session alive while the agent runs (until the plan's cap).
        {
            let auth = auth.clone();
            let path = format!("{base}/extend-timeout");
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(10 * 60));
                    let _ = auth.request(reqwest::Method::POST, &path, Some(json!({ "duration": 15 * 60 * 1000 })));
                }
            });
        }

        // Stdin in order with byte offsets, then close.
        {
            let auth = auth.clone();
            let path = format!("{base}/cmd/{cmd}/stdin");
            std::thread::spawn(move || {
                let client = super::http();
                let chunks = stdio::stdin_chunks();
                let mut offset = 0u64;
                while let Ok(Some(chunk)) = chunks.recv() {
                    let body = stdin_body(&chunk, offset, false);
                    if !matches!(auth.send(&client, reqwest::Method::POST, &path, Some(body)), Ok((200..=299, _))) {
                        return;
                    }
                    offset += chunk.len() as u64;
                }
                let _ = auth.send(&client, reqwest::Method::POST, &path, Some(stdin_body(&[], offset, true)));
            });
        }

        let streaming = stdio::streaming_client();
        let resp = streaming
            .get(auth.url(&format!("{base}/cmd/{cmd}/logs")))
            .bearer_auth(&auth.token)
            .send()
            .context("opening the Vercel log stream")?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let v: Value = serde_json::from_str(&resp.text().unwrap_or_default()).unwrap_or(Value::Null);
            bail!("Vercel log stream: {}", api_error(status, &v));
        }
        for line in std::io::BufReader::new(resp).split(b'\n') {
            match log_line(&line.context("reading the Vercel log stream")?) {
                Log::Stdout(s) => stdio::out(s.as_bytes())?,
                Log::Stderr(s) => stdio::err(s.as_bytes())?,
                Log::Error(why) => bail!("Vercel log stream: {why}"),
                Log::Ignore => {}
            }
        }
        let (status, v) = auth.send(&streaming, reqwest::Method::GET, &format!("{base}/cmd/{cmd}?wait=true"), None)?;
        if !(200..300).contains(&status) {
            bail!("Vercel command status: {}", api_error(status, &v));
        }
        Ok(v["command"]["exitCode"].as_i64().unwrap_or(1) as i32)
    }
}

fn stdin_body(chunk: &[u8], offset: u64, close: bool) -> Value {
    let mut b = json!({ "offset": offset });
    if !chunk.is_empty() {
        b["data"] = json!(stdio::b64_encode(chunk));
    }
    if close {
        b["close"] = json!(true);
    }
    b
}

#[derive(Debug, PartialEq, Eq)]
enum Log {
    Stdout(String),
    Stderr(String),
    Error(String),
    Ignore,
}

fn log_line(line: &[u8]) -> Log {
    let Ok(v) = serde_json::from_slice::<Value>(line) else { return Log::Ignore };
    let data = || v["data"].as_str().unwrap_or_default().to_string();
    match v["stream"].as_str() {
        Some("stdout") => Log::Stdout(data()),
        Some("stderr") => Log::Stderr(data()),
        Some("error") => Log::Error(format!(
            "{}: {}",
            v["data"]["code"].as_str().unwrap_or("error"),
            v["data"]["message"].as_str().unwrap_or("stream failed")
        )),
        _ => Log::Ignore,
    }
}

fn box_info(s: &Value) -> Option<BoxInfo> {
    let name = s["name"].as_str()?.to_string();
    let state = s["status"].as_str().unwrap_or("unknown").to_string();
    Some(BoxInfo { id: name.clone(), name, running: state == "running", state })
}

/// `{"error":{"code","message"}}` → `code: message (HTTP n)`.
fn api_error(status: u16, v: &Value) -> String {
    let e = &v["error"];
    let code = e["code"].as_str().unwrap_or("error");
    let msg = e["message"].as_str().unwrap_or("request failed");
    format!("{code}: {msg} (HTTP {status})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_has_three_parts() {
        let a = parse_key("tok|team_1|prj_1").unwrap();
        assert_eq!((a.token.as_str(), a.team.as_deref(), a.project.as_str()), ("tok", Some("team_1"), "prj_1"));
        assert_eq!(parse_key("tok||prj_1").unwrap().team, None);
        assert!(parse_key("tok|team").is_none());
        assert!(parse_key("tok|team|").is_none());
        assert!(parse_key("a|b|c|d").is_none());
        assert_eq!(a.url("/v2/sandboxes?project=p"), "https://vercel.com/api/v2/sandboxes?project=p&teamId=team_1");
        assert_eq!(a.url("/v3/sandboxes"), "https://vercel.com/api/v3/sandboxes?teamId=team_1");
    }

    #[test]
    fn stdin_bodies_carry_offsets() {
        assert_eq!(stdin_body(b"hi\n", 5, false), json!({"offset": 5, "data": "aGkK"}));
        assert_eq!(stdin_body(&[], 8, true), json!({"offset": 8, "close": true}));
    }

    #[test]
    fn reads_log_lines() {
        assert_eq!(log_line(br#"{"stream":"stdout","data":"hi"}"#), Log::Stdout("hi".into()));
        assert_eq!(log_line(br#"{"stream":"stderr","data":"x"}"#), Log::Stderr("x".into()));
        assert_eq!(
            log_line(br#"{"stream":"error","data":{"code":"gone","message":"session stopped"}}"#),
            Log::Error("gone: session stopped".into())
        );
        assert_eq!(log_line(b"not json"), Log::Ignore);
    }

    #[test]
    fn sandboxes_become_boxes() {
        let b = box_info(&json!({"name": "sbx", "status": "stopped", "currentSessionId": "s1"})).unwrap();
        assert_eq!(b, BoxInfo { id: "sbx".into(), name: "sbx".into(), state: "stopped".into(), running: false });
        assert_eq!(
            api_error(403, &json!({"error": {"code": "forbidden", "message": "Not authorized"}})),
            "forbidden: Not authorized (HTTP 403)"
        );
    }

    #[test]
    fn no_key_needs_setup() {
        assert!(matches!(PROVIDER.probe(&Keys::default()), Probe::NeedsSetup(_)));
    }
}
