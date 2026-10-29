//! DeepInfra Sandboxes (docs.deepinfra.com/sandboxes/overview): VMs on a
//! REST API (route: native, with a file mailbox standing in for stdin).
//!
//! - `https://api.deepinfra.com`, `Authorization: Bearer <DEEPINFRA_API_KEY>`
//!   (OpenAPI: https://api.deepinfra.com/openapi.json): `POST /v1/sandboxes`,
//!   `GET /v1/sandboxes`, `GET`/`DELETE /v1/sandboxes/{id}`,
//!   `POST /v1/sandboxes/{id}/start|stop`, `PUT /v1/sandboxes/{id}/fs/content?path=`
//!   and `POST /v1/sandboxes/{id}/exec`
//!   (https://docs.deepinfra.com/api-reference/sandboxes/exec-command), which
//!   streams NDJSON `{"stdout":…}` / `{"stderr":…}` and ends with one
//!   `{"returncode":N}` or `{"error":…}` line.
//!
//! Exec has **no stdin** (DeepInfra's own Hermes plugin feeds commands
//! through a heredoc for that reason). An agent needs a live stdin, so the
//! bridge wraps the command in a small `sh` loop that reads numbered files
//! from `/workspace/.keplorayx/stdin-<tag>/` into a FIFO on the agent's
//! stdin; each local chunk is written as `<n>` followed by an empty
//! `<n>.ok` marker (so a half-written file is never read), and EOF is an
//! `eof` file. It costs two HTTP calls per chunk, which is fine for
//! newline-delimited JSON.
//!
//! Hard limit: an exec is capped at 1800 s, so an agent run on DeepInfra
//! ends after 30 minutes. Only `/workspace` survives stop/start, and a
//! stopped sandbox is deleted after 7 days. Uncertain: whether the default
//! image ships git.

use std::io::BufRead as _;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::e2b::stdio;
use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

const API: &str = "https://api.deepinfra.com/v1";
/// The longest an exec may run.
const EXEC_SECS: u64 = 1800;
const MAILBOX_ROOT: &str = "/workspace/.keplorayx";

pub struct Deepinfra;
pub static PROVIDER: Deepinfra = Deepinfra;

impl Deepinfra {
    fn key<'a>(&self, keys: &'a Keys) -> Result<&'a str> {
        keys.get(self.id()).context("no DeepInfra API key: add one in Settings › Machines")
    }

    fn send(&self, keys: &Keys, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<(u16, Value)> {
        let key = self.key(keys)?;
        let mut req = super::http().request(method, format!("{API}{path}")).bearer_auth(key);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp =
            req.send().with_context(|| format!("calling DeepInfra {}", path.split('?').next().unwrap_or(path)))?;
        let status = resp.status().as_u16();
        let text = resp.text().unwrap_or_default();
        Ok((status, serde_json::from_str(&text).unwrap_or_else(|_| json!({ "detail": text.trim() }))))
    }

    fn request(&self, keys: &Keys, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let (status, v) = self.send(keys, method, path, body)?;
        if !(200..300).contains(&status) {
            bail!("DeepInfra {}: {}", path.split('?').next().unwrap_or(path), api_error(status, &v));
        }
        Ok(v)
    }

    fn wait_running(&self, keys: &Keys, id: &str, limit: Duration) -> Result<Value> {
        let deadline = Instant::now() + limit;
        loop {
            let s = self.request(keys, reqwest::Method::GET, &format!("/sandboxes/{id}"), None)?;
            match state(&s).as_str() {
                "running" => return Ok(s),
                "failed" => {
                    bail!("DeepInfra sandbox {id} failed: {}", s["fail_reason"].as_str().unwrap_or("no reason given"))
                }
                _ => {}
            }
            if Instant::now() >= deadline {
                bail!("DeepInfra sandbox {id} was not running after {}s", limit.as_secs());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

impl Provider for Deepinfra {
    fn id(&self) -> &'static str {
        "deepinfra"
    }

    fn label(&self) -> &'static str {
        "DeepInfra Sandbox"
    }

    fn key_url(&self) -> &'static str {
        "https://deepinfra.com/dash/api_keys"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, keys: &Keys) -> Probe {
        if !keys.has(self.id()) {
            return Probe::NeedsSetup("Add your DeepInfra API key".into());
        }
        match self.send(keys, reqwest::Method::GET, "/sandboxes", None) {
            Ok((200..=299, _)) => Probe::Ready,
            Ok((401 | 403, _)) => Probe::NeedsSetup("DeepInfra rejected the key".into()),
            Ok((status, v)) => Probe::Unavailable(format!("DeepInfra: {}", api_error(status, &v))),
            Err(err) => Probe::Unavailable(format!("{err:#}")),
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        let v = self.request(keys, reqwest::Method::GET, "/sandboxes", None)?;
        Ok(v.as_array().into_iter().flatten().filter_map(box_info).collect())
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let mut tags = serde_json::Map::new();
        tags.insert("created-by".into(), json!("keplorayx"));
        if let Some(n) = &spec.name {
            tags.insert("name".into(), json!(n));
        }
        // `template` picks a plan from `GET /v1/sandboxes/catalog`; empty
        // means DeepInfra's default.
        let body = json!({ "plan": spec.template.as_deref().unwrap_or(""), "tags": tags });
        let v = self.request(keys, reqwest::Method::POST, "/sandboxes", Some(body))?;
        let id = v["sandbox_id"].as_str().context("DeepInfra did not say which sandbox it created")?.to_string();
        let s = self.wait_running(keys, &id, Duration::from_secs(300))?;
        box_info(&s).context("DeepInfra returned an unreadable sandbox")
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        let s = self.request(keys, reqwest::Method::GET, &format!("/sandboxes/{id}"), None)?;
        if state(&s) == "stopped" {
            self.request(keys, reqwest::Method::POST, &format!("/sandboxes/{id}/start"), None)?;
        }
        self.wait_running(keys, id, Duration::from_secs(240)).map(|_| ())
    }

    fn sleep(&self, keys: &Keys, id: &str) -> Result<()> {
        match self.send(keys, reqwest::Method::POST, &format!("/sandboxes/{id}/stop"), None)? {
            (200..=299 | 409, _) => Ok(()), // 409: already stopped or stopping
            (status, v) => bail!("DeepInfra stop: {}", api_error(status, &v)),
        }
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        match self.send(keys, reqwest::Method::DELETE, &format!("/sandboxes/{id}"), None)? {
            (200..=299 | 404, _) => Ok(()),
            (status, v) => bail!("DeepInfra delete: {}", api_error(status, &v)),
        }
    }

    fn connect(&self, keys: &Keys, id: &str, _dir: &Path) -> Result<Endpoint> {
        self.wait_running(keys, id, Duration::from_secs(60))?;
        Ok(Endpoint::Native)
    }

    fn bridge(&self, keys: &Keys, id: &str, argv: &[String]) -> Result<i32> {
        if argv.is_empty() {
            bail!("nothing to run");
        }
        let key = self.key(keys)?.to_string();
        let mailbox = format!("{MAILBOX_ROOT}/stdin-{}", uuid::Uuid::now_v7().simple());

        // Stdin: `<n>` then `<n>.ok`, then `eof`. The first writes retry
        // briefly in case the loop hasn't made the folder yet.
        {
            let (key, id, mailbox) = (key.clone(), id.to_string(), mailbox.clone());
            std::thread::spawn(move || {
                let client = super::http();
                let put = |name: &str, body: Vec<u8>| -> bool {
                    for attempt in 0..20 {
                        let ok = client
                            .put(format!("{API}/sandboxes/{id}/fs/content"))
                            .query(&[("path", format!("{mailbox}/{name}"))])
                            .bearer_auth(&key)
                            .header("Content-Type", "application/octet-stream")
                            .body(body.clone())
                            .send()
                            .is_ok_and(|r| r.status().is_success());
                        if ok {
                            return true;
                        }
                        std::thread::sleep(Duration::from_millis(100 * (attempt + 1)));
                    }
                    false
                };
                let chunks = stdio::stdin_chunks();
                let mut n = 0u64;
                while let Ok(Some(chunk)) = chunks.recv() {
                    if !put(&n.to_string(), chunk) || !put(&format!("{n}.ok"), Vec::new()) {
                        return;
                    }
                    n += 1;
                }
                put("eof", Vec::new());
            });
        }

        let body = json!({ "command": mailbox_argv(&mailbox, argv), "timeout_seconds": EXEC_SECS });
        let resp = stdio::streaming_client()
            .post(format!("{API}/sandboxes/{id}/exec"))
            .bearer_auth(&key)
            .json(&body)
            .send()
            .context("starting the command on the DeepInfra sandbox")?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let v: Value = serde_json::from_str(&resp.text().unwrap_or_default()).unwrap_or(Value::Null);
            bail!("DeepInfra exec: {}", api_error(status, &v));
        }
        for line in std::io::BufReader::new(resp).split(b'\n') {
            match exec_line(&line.context("reading the DeepInfra exec stream")?) {
                Line::Stdout(s) => stdio::out(s.as_bytes())?,
                Line::Stderr(s) => stdio::err(s.as_bytes())?,
                Line::Exit(code) => return Ok(code),
                Line::Error(why) => bail!("DeepInfra exec failed: {why}"),
                Line::Ignore => {}
            }
        }
        bail!("the DeepInfra exec stream ended before the command exited")
    }
}

/// `sh -c <loop> keplorayx-stdin <mailbox> <argv…>`: feeds the mailbox
/// into a FIFO on the command's stdin and returns the command's status.
fn mailbox_argv(mailbox: &str, argv: &[String]) -> Vec<String> {
    const FEED: &str = r#"d=$1; shift
mkdir -p "$d" && mkfifo "$d/in" || exit 125
( n=0
  while :; do
    if [ -e "$d/$n.ok" ]; then cat "$d/$n"; rm -f "$d/$n" "$d/$n.ok"; n=$((n+1))
    elif [ -e "$d/eof" ]; then break
    else sleep 0.05; fi
  done ) > "$d/in" &
feeder=$!
"$@" < "$d/in"
status=$?
kill "$feeder" 2>/dev/null
rm -rf "$d"
exit "$status""#;
    let mut a: Vec<String> = vec!["sh".into(), "-c".into(), FEED.into(), "keplorayx-stdin".into(), mailbox.into()];
    a.extend(argv.iter().cloned());
    a
}

#[derive(Debug, PartialEq, Eq)]
enum Line {
    Stdout(String),
    Stderr(String),
    Exit(i32),
    Error(String),
    Ignore,
}

fn exec_line(line: &[u8]) -> Line {
    let Ok(v) = serde_json::from_slice::<Value>(line) else { return Line::Ignore };
    if let Some(s) = v["stdout"].as_str() {
        return Line::Stdout(s.to_string());
    }
    if let Some(s) = v["stderr"].as_str() {
        return Line::Stderr(s.to_string());
    }
    if let Some(c) = v["returncode"].as_i64() {
        return Line::Exit(c as i32);
    }
    if !v["error"].is_null() {
        return Line::Error(v["error"].as_str().map(String::from).unwrap_or_else(|| v["error"].to_string()));
    }
    Line::Ignore
}

/// The state in lower case (the API documents both `RUNNING` and `running`).
fn state(s: &Value) -> String {
    s["state"].as_str().unwrap_or("unknown").to_ascii_lowercase()
}

fn box_info(s: &Value) -> Option<BoxInfo> {
    let id = s["sandbox_id"].as_str()?.to_string();
    let state = state(s);
    Some(BoxInfo {
        name: s["tags"]["name"].as_str().unwrap_or(&id).to_string(),
        running: state == "running",
        state,
        id,
    })
}

/// FastAPI errors: `{"detail": "…"}` or `{"detail": [{"msg": …}]}`.
fn api_error(status: u16, v: &Value) -> String {
    let d = &v["detail"];
    let msg = d
        .as_str()
        .map(String::from)
        .or_else(|| d[0]["msg"].as_str().map(String::from))
        .unwrap_or_else(|| v["error"].as_str().unwrap_or("request failed").to_string());
    format!("{msg} (HTTP {status})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_exec_lines() {
        assert_eq!(exec_line(br#"{"stdout":"hi\n"}"#), Line::Stdout("hi\n".into()));
        assert_eq!(exec_line(br#"{"stderr":"x"}"#), Line::Stderr("x".into()));
        assert_eq!(exec_line(br#"{"returncode":3}"#), Line::Exit(3));
        assert_eq!(exec_line(br#"{"error":"timed out"}"#), Line::Error("timed out".into()));
        assert_eq!(exec_line(b""), Line::Ignore);
    }

    #[test]
    fn mailbox_wraps_the_argv() {
        let a = mailbox_argv("/workspace/.keplorayx/stdin-1", &["sh".into(), "-lc".into(), "x".into()]);
        assert_eq!(&a[..2], ["sh", "-c"]);
        assert!(a[2].contains(r#""$@" < "$d/in""#));
        assert_eq!(&a[3..], ["keplorayx-stdin", "/workspace/.keplorayx/stdin-1", "sh", "-lc", "x"]);
    }

    #[test]
    fn sandboxes_become_boxes() {
        let b = box_info(&json!({"sandbox_id": "s1", "state": "RUNNING", "tags": {"name": "dev"}})).unwrap();
        assert_eq!(b, BoxInfo { id: "s1".into(), name: "dev".into(), state: "running".into(), running: true });
        assert!(box_info(&json!({"state": "running"})).is_none());
        assert_eq!(api_error(422, &json!({"detail": [{"msg": "field required"}]})), "field required (HTTP 422)");
        assert_eq!(api_error(409, &json!({"detail": "already stopped"})), "already stopped (HTTP 409)");
    }

    #[test]
    fn no_key_needs_setup() {
        assert!(matches!(PROVIDER.probe(&Keys::default()), Probe::NeedsSetup(_)));
    }
}
