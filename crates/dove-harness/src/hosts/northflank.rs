//! Northflank (northflank.com): a box is a deployment service (Northflank
//! runs sandboxes as services, as microVMs on CPU plans), reached through
//! the command-exec WebSocket (route: native).
//!
//! - REST on `https://api.northflank.com/v1`, `Authorization: Bearer <token>`
//!   (https://northflank.com/docs/v1/api/introduction): `GET /projects`,
//!   `GET /projects/{p}/services`, `POST /projects/{p}/services/deployment`
//!   (external image + custom command), `GET`/`DELETE /projects/{p}/services/{s}`,
//!   `POST …/pause`, `POST …/resume`
//!   (https://northflank.com/docs/v1/application/sandboxes).
//! - Exec: `wss://api.northflank.com/v1/command-exec/projects/{p}/services/{s}`.
//!   The protocol is read from `@northflank/js-client` (`ExecCommandStandard`,
//!   https://unpkg.com/@northflank/js-client/dist/api-client.js): every frame
//!   is JSON; the first client message carries the token
//!   (`{"type":"init","data":{"auth":…,"context":{"command":[…],"shell":"none","tty":false}}}`),
//!   stdin is `{"type":"payload","data":"…"}` (UTF-8 text), EOF is
//!   `{"type":"stdInEnd"}`; the server sends `stdOut`/`stdErr`/`completion`
//!   (`exitCode`) and `error`. Output can trail `completion`, so the bridge
//!   reads on until the socket closes or stays quiet.
//!
//! The key is the API token, optionally `token|project-id` to pin the project
//! new boxes go in (otherwise the first project). Box ids are
//! `<project>/<service>`. Team-scoped tokens (`teams/{t}/` routes) are not
//! handled. Uncertain: the paused-state field name (`servicePaused`) and
//! whether the exec server also accepts text frames (the bridge sends binary,
//! as the official client does).

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};
use tungstenite::Message;

use super::e2b::stdio;
use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

const API: &str = "https://api.northflank.com/v1";
const WS: &str = "wss://api.northflank.com/v1";
/// Ubuntu 24.04 with git and curl (an agent needs sh + git).
const DEFAULT_IMAGE: &str = "buildpack-deps:noble-scm";
/// The smallest plan.
const PLAN: &str = "nf-compute-20";

pub struct Northflank;
pub static PROVIDER: Northflank = Northflank;

/// `token` or `token|project`.
fn parse_key(raw: &str) -> (&str, Option<&str>) {
    match raw.split_once('|') {
        Some((t, p)) if !p.trim().is_empty() => (t.trim(), Some(p.trim())),
        Some((t, _)) => (t.trim(), None),
        None => (raw.trim(), None),
    }
}

/// `<project>/<service>`.
fn split_id(id: &str) -> Result<(&str, &str)> {
    id.split_once('/')
        .filter(|(p, s)| !p.is_empty() && !s.is_empty())
        .with_context(|| format!("{id} is not a Northflank box id (project/service)"))
}

impl Northflank {
    fn token<'a>(&self, keys: &'a Keys) -> Result<&'a str> {
        let raw = keys.get(self.id()).context("no Northflank API token: add one in Settings › Machines")?;
        Ok(parse_key(raw).0)
    }

    fn send(&self, keys: &Keys, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<(u16, Value)> {
        let token = self.token(keys)?;
        let mut req = super::http().request(method, format!("{API}{path}")).bearer_auth(token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().with_context(|| format!("calling Northflank {path}"))?;
        let status = resp.status().as_u16();
        let text = resp.text().unwrap_or_default();
        Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
    }

    fn request(&self, keys: &Keys, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let (status, v) = self.send(keys, method, path, body)?;
        if !(200..300).contains(&status) {
            bail!("Northflank {path}: {}", api_error(status, &v));
        }
        Ok(v)
    }

    fn projects(&self, keys: &Keys) -> Result<Vec<String>> {
        let v = self.request(keys, reqwest::Method::GET, "/projects?per_page=100", None)?;
        Ok(v["data"]["projects"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["id"].as_str().map(String::from))
            .collect())
    }

    fn service(&self, keys: &Keys, id: &str) -> Result<Value> {
        let (p, s) = split_id(id)?;
        let v = self.request(keys, reqwest::Method::GET, &format!("/projects/{p}/services/{s}"), None)?;
        Ok(v["data"].clone())
    }

    fn wait_running(&self, keys: &Keys, id: &str, limit: Duration) -> Result<()> {
        let deadline = Instant::now() + limit;
        loop {
            let s = self.service(keys, id)?;
            match state(&s).as_str() {
                "completed" => return Ok(()),
                "failed" => bail!("Northflank service {id} failed to deploy: {}", s["status"]["deployment"]["reason"]),
                _ => {}
            }
            if Instant::now() >= deadline {
                bail!("Northflank service {id} was not running after {}s", limit.as_secs());
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    }
}

impl Provider for Northflank {
    fn id(&self) -> &'static str {
        "northflank"
    }

    fn label(&self) -> &'static str {
        "Northflank"
    }

    fn key_url(&self) -> &'static str {
        "https://app.northflank.com/s/account/settings/api/tokens"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, keys: &Keys) -> Probe {
        if !keys.has(self.id()) {
            return Probe::NeedsSetup("Add a Northflank API token (optionally token|project-id)".into());
        }
        match self.send(keys, reqwest::Method::GET, "/projects?per_page=1", None) {
            Ok((200..=299, _)) => Probe::Ready,
            Ok((401 | 403, _)) => Probe::NeedsSetup("Northflank rejected the token (it needs project access)".into()),
            Ok((status, v)) => Probe::Unavailable(format!("Northflank: {}", api_error(status, &v))),
            Err(err) => Probe::Unavailable(format!("{err:#}")),
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        let mut out = Vec::new();
        for p in self.projects(keys)?.iter().take(20) {
            let v = self.request(keys, reqwest::Method::GET, &format!("/projects/{p}/services?per_page=100"), None)?;
            out.extend(v["data"]["services"].as_array().into_iter().flatten().filter_map(|s| box_info(p, s)));
        }
        Ok(out)
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let pinned = keys.get(self.id()).and_then(|raw| parse_key(raw).1).map(String::from);
        let project = match pinned {
            Some(p) => p,
            None => self
                .projects(keys)?
                .into_iter()
                .next()
                .context("create a project in Northflank first (or add it to the key as token|project-id)")?,
        };
        let name = service_name(spec.name.as_deref());
        let body = create_body(&name, spec.template.as_deref().unwrap_or(DEFAULT_IMAGE));
        let v =
            self.request(keys, reqwest::Method::POST, &format!("/projects/{project}/services/deployment"), Some(body))?;
        let service = v["data"]["id"].as_str().unwrap_or(&name).to_string();
        let id = format!("{project}/{service}");
        self.wait_running(keys, &id, Duration::from_secs(300))?;
        Ok(BoxInfo { id, name, state: "completed".into(), running: true })
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        let s = self.service(keys, id)?;
        if s["servicePaused"] == true {
            let (p, sv) = split_id(id)?;
            self.request(keys, reqwest::Method::POST, &format!("/projects/{p}/services/{sv}/resume"), Some(json!({})))?;
        }
        self.wait_running(keys, id, Duration::from_secs(240))
    }

    fn sleep(&self, keys: &Keys, id: &str) -> Result<()> {
        let (p, s) = split_id(id)?;
        self.request(keys, reqwest::Method::POST, &format!("/projects/{p}/services/{s}/pause"), Some(json!({})))
            .map(|_| ())
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        let (p, s) = split_id(id)?;
        match self.send(keys, reqwest::Method::DELETE, &format!("/projects/{p}/services/{s}"), None)? {
            (200..=299 | 404, _) => Ok(()),
            (status, v) => bail!("Northflank delete: {}", api_error(status, &v)),
        }
    }

    fn connect(&self, keys: &Keys, id: &str, _dir: &Path) -> Result<Endpoint> {
        self.service(keys, id)?;
        Ok(Endpoint::Native)
    }

    fn bridge(&self, keys: &Keys, id: &str, argv: &[String]) -> Result<i32> {
        let token = self.token(keys)?;
        let (p, s) = split_id(id)?;
        let mut ws = stdio::ws_connect(&format!("{WS}/command-exec/projects/{p}/services/{s}"), &[])?;
        ws.send(Message::Binary(init_message(token, argv).to_string().into_bytes().into()))?;

        let stdin = stdio::stdin_chunks();
        let mut stdin_open = false; // until the server accepts the token
        let mut text = stdio::Utf8Carry::default();
        let mut exit: Option<i32> = None;
        let mut quiet_since = Instant::now();
        loop {
            while stdin_open {
                match stdin.try_recv() {
                    Ok(Some(chunk)) => {
                        let data = text.push(&chunk);
                        if !data.is_empty() {
                            ws.send(Message::Binary(
                                json!({"type": "payload", "data": data}).to_string().into_bytes().into(),
                            ))?;
                        }
                    }
                    Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        let rest = text.finish();
                        if !rest.is_empty() {
                            ws.send(Message::Binary(
                                json!({"type": "payload", "data": rest}).to_string().into_bytes().into(),
                            ))?;
                        }
                        ws.send(Message::Binary(json!({"type": "stdInEnd"}).to_string().into_bytes().into()))?;
                        stdin_open = false;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                }
            }
            let msg = match ws.read() {
                Ok(m) => m,
                Err(e) if stdio::ws_timed_out(&e) => {
                    // Output may trail `completion`; stop once it goes quiet.
                    if let Some(code) = exit
                        && quiet_since.elapsed() > Duration::from_secs(2)
                    {
                        let _ = ws.close(None);
                        return Ok(code);
                    }
                    continue;
                }
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => break,
                Err(e) if exit.is_some() => {
                    tracing::debug!("Northflank exec stream after completion: {e}");
                    break;
                }
                Err(e) => return Err(e).context("reading the Northflank exec stream"),
            };
            quiet_since = Instant::now();
            match server_message(&msg) {
                Server::AuthOk => stdin_open = exit.is_none(),
                Server::AuthFailed(why) => bail!("Northflank refused the exec session: {why}"),
                Server::Stdout(s) => stdio::out(s.as_bytes())?,
                Server::Stderr(s) => stdio::err(s.as_bytes())?,
                Server::Completion(code) => exit = Some(code),
                Server::Error(why) => bail!("Northflank exec failed: {why}"),
                Server::Closed => break,
                Server::Ignore => {}
            }
        }
        exit.context("Northflank closed the exec stream before the command finished")
    }
}

fn init_message(token: &str, argv: &[String]) -> Value {
    json!({
        "type": "init",
        "data": {
            "auth": { "type": "apiToken", "apiToken": token },
            "context": { "command": argv, "shell": "none", "encoding": "utf8", "tty": false },
        },
    })
}

#[derive(Debug, PartialEq, Eq)]
enum Server {
    AuthOk,
    AuthFailed(String),
    Stdout(String),
    Stderr(String),
    Completion(i32),
    Error(String),
    Closed,
    Ignore,
}

fn server_message(msg: &Message) -> Server {
    let v: Value = match msg {
        Message::Text(t) => serde_json::from_str(t.as_str()).unwrap_or(Value::Null),
        Message::Binary(b) => serde_json::from_slice(b).unwrap_or(Value::Null),
        Message::Close(_) => return Server::Closed,
        _ => return Server::Ignore,
    };
    let data = &v["data"];
    let text = |d: &Value| d.as_str().map(String::from).unwrap_or_else(|| d.to_string());
    match v["type"].as_str() {
        Some("init") => match data["auth"].as_str() {
            Some("successful") => Server::AuthOk,
            other => Server::AuthFailed(data["message"].as_str().or(other).unwrap_or("token not accepted").to_string()),
        },
        Some("stdOut") => Server::Stdout(text(data)),
        Some("stdErr") => Server::Stderr(text(data)),
        Some("completion") => Server::Completion(data["exitCode"].as_i64().unwrap_or(1) as i32),
        Some("error") => Server::Error(data["message"].as_str().map(String::from).unwrap_or_else(|| text(data))),
        _ => Server::Ignore,
    }
}

fn create_body(name: &str, image: &str) -> Value {
    json!({
        "name": name,
        "billing": { "deploymentPlan": PLAN },
        "deployment": {
            "instances": 1,
            "external": { "imagePath": image },
            "docker": {
                "configType": "customEntrypointCustomCommand",
                "customEntrypoint": "/bin/sh",
                "customCommand": "-c 'sleep infinity'",
            },
        },
    })
}

/// `paused`, or the deployment status in lower case (`completed` = up).
fn state(s: &Value) -> String {
    if s["servicePaused"] == true {
        return "paused".into();
    }
    s["status"]["deployment"]["status"].as_str().unwrap_or("unknown").to_ascii_lowercase()
}

fn box_info(project: &str, s: &Value) -> Option<BoxInfo> {
    let service = s["id"].as_str()?;
    if s["serviceType"].as_str().is_some_and(|t| t == "build") {
        return None;
    }
    let state = state(s);
    Some(BoxInfo {
        id: format!("{project}/{service}"),
        name: s["name"].as_str().unwrap_or(service).to_string(),
        running: state == "completed",
        state,
    })
}

/// Service names: lowercase letters, digits and dashes.
fn service_name(wanted: Option<&str>) -> String {
    let cleaned: String = wanted
        .unwrap_or_default()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(39)
        .collect();
    if cleaned.is_empty() {
        let id = uuid::Uuid::now_v7().simple().to_string();
        format!("keplorayx-{}", &id[id.len() - 8..])
    } else {
        cleaned
    }
}

/// `{"error":{"status","message"}}` → `message (HTTP n)`.
fn api_error(status: u16, v: &Value) -> String {
    let msg = v["error"]["message"].as_str().or(v["message"].as_str()).unwrap_or("request failed");
    format!("{msg} (HTTP {status})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_ids_split() {
        assert_eq!(parse_key("nf_tok"), ("nf_tok", None));
        assert_eq!(parse_key("nf_tok | proj-1"), ("nf_tok", Some("proj-1")));
        assert_eq!(parse_key("nf_tok|"), ("nf_tok", None));
        assert_eq!(split_id("proj/svc").unwrap(), ("proj", "svc"));
        assert!(split_id("svc").is_err());
    }

    #[test]
    fn reads_server_messages() {
        let m = |s: &str| Message::Binary(s.as_bytes().to_vec().into());
        assert_eq!(server_message(&m(r#"{"type":"init","data":{"auth":"successful"}}"#)), Server::AuthOk);
        assert!(matches!(server_message(&m(r#"{"type":"init","data":{"auth":"failed"}}"#)), Server::AuthFailed(_)));
        assert_eq!(
            server_message(&Message::Text(r#"{"type":"stdOut","data":"hi\n"}"#.into())),
            Server::Stdout("hi\n".into())
        );
        assert_eq!(server_message(&m(r#"{"type":"stdErr","data":"x"}"#)), Server::Stderr("x".into()));
        assert_eq!(
            server_message(&m(r#"{"type":"completion","data":{"exitCode":4,"status":"Failure"}}"#)),
            Server::Completion(4)
        );
        assert_eq!(
            server_message(&m(r#"{"type":"error","data":{"message":"no pod"}}"#)),
            Server::Error("no pod".into())
        );
        assert_eq!(server_message(&Message::Close(None)), Server::Closed);
    }

    #[test]
    fn init_carries_argv_without_a_shell_or_tty() {
        let v = init_message("tok", &["sh".into(), "-lc".into(), "x".into()]);
        assert_eq!(v["data"]["auth"]["apiToken"], "tok");
        assert_eq!(v["data"]["context"]["command"], json!(["sh", "-lc", "x"]));
        assert_eq!(
            (v["data"]["context"]["shell"].as_str(), v["data"]["context"]["tty"].as_bool()),
            (Some("none"), Some(false))
        );
    }

    #[test]
    fn services_become_boxes() {
        let s = json!({"id": "svc", "name": "Box", "serviceType": "deployment", "status": {"deployment": {"status": "COMPLETED"}}});
        assert_eq!(
            box_info("p", &s).unwrap(),
            BoxInfo { id: "p/svc".into(), name: "Box".into(), state: "completed".into(), running: true }
        );
        let paused = json!({"id": "svc", "servicePaused": true, "status": {"deployment": {"status": "COMPLETED"}}});
        assert!(!box_info("p", &paused).unwrap().running);
        assert!(box_info("p", &json!({"id": "b", "serviceType": "build"})).is_none());
        assert_eq!(create_body("n", "img")["deployment"]["external"]["imagePath"], "img");
        assert_eq!(service_name(Some("My_Box")), "my-box");
        assert_eq!(api_error(403, &json!({"error": {"status": 403, "message": "Forbidden"}})), "Forbidden (HTTP 403)");
    }

    #[test]
    fn no_key_needs_setup() {
        assert!(matches!(PROVIDER.probe(&Keys::default()), Probe::NeedsSetup(_)));
    }
}
