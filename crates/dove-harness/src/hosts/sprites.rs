//! Fly Sprites (sprites.dev): persistent Linux VMs that sleep when idle and
//! wake on the next request (route: native WebSocket exec).
//!
//! - REST on `https://api.sprites.dev`, `Authorization: Bearer <SPRITES_TOKEN>`
//!   (OpenAPI: https://docs.fly.io/sprites/api/openapi.json):
//!   `POST /v1/sprites {name}`, `GET /v1/sprites?max_results=&continuation_token=`,
//!   `GET`/`DELETE /v1/sprites/{name}`. There is no start or stop call: a
//!   sprite goes `cold` on its own and wakes on exec, so [`Provider::wake`]
//!   only checks it exists and [`Provider::sleep`] has nothing to do.
//! - Exec: `wss://api.sprites.dev/v1/sprites/{name}/exec?cmd=…&cmd=…&path=…&stdin=true`
//!   (https://docs.fly.io/sprites/api/exec; framing read from the Go SDK,
//!   https://github.com/superfly/sprites-go `exec.go`/`websocket.go`).
//!   Without a TTY every binary frame is one stream byte then payload:
//!   `0` stdin, `1` stdout, `2` stderr, `3` exit (code in the next byte),
//!   `4` stdin EOF. Text frames are JSON notices (`session_info`, port
//!   events, and `{"type":"exit","exit_code":N}`).
//!
//! Uncertain: whether a non-TTY exec survives a dropped socket (TTY sessions
//! are documented as detachable); the bridge assumes it does not.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};
use tungstenite::Message;

use super::e2b::stdio;
use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

const API: &str = "https://api.sprites.dev";
const WS: &str = "wss://api.sprites.dev";

pub struct Sprites;
pub static PROVIDER: Sprites = Sprites;

impl Sprites {
    fn key<'a>(&self, keys: &'a Keys) -> Result<&'a str> {
        keys.get(self.id()).context("no Sprites token: add one in Settings › Machines")
    }

    fn send(&self, keys: &Keys, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<(u16, Value)> {
        let key = self.key(keys)?;
        let mut req = super::http().request(method, format!("{API}{path}")).bearer_auth(key);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().with_context(|| format!("calling Sprites {path}"))?;
        let status = resp.status().as_u16();
        let text = resp.text().unwrap_or_default();
        Ok((status, serde_json::from_str(&text).unwrap_or_else(|_| json!({ "error": text.trim() }))))
    }

    fn request(&self, keys: &Keys, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let (status, v) = self.send(keys, method, path, body)?;
        if !(200..300).contains(&status) {
            bail!("Sprites {path}: {}", api_error(status, &v));
        }
        Ok(v)
    }
}

impl Provider for Sprites {
    fn id(&self) -> &'static str {
        "sprites"
    }

    fn label(&self) -> &'static str {
        "Fly Sprites"
    }

    fn key_url(&self) -> &'static str {
        "https://sprites.dev"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, keys: &Keys) -> Probe {
        if !keys.has(self.id()) {
            return Probe::NeedsSetup("Add your Sprites token (SPRITES_TOKEN)".into());
        }
        match self.send(keys, reqwest::Method::GET, "/v1/sprites?max_results=1", None) {
            Ok((200..=299, _)) => Probe::Ready,
            Ok((401 | 403, _)) => Probe::NeedsSetup("Sprites rejected the token".into()),
            Ok((status, v)) => Probe::Unavailable(format!("Sprites: {}", api_error(status, &v))),
            Err(err) => Probe::Unavailable(format!("{err:#}")),
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..10 {
            let mut path = "/v1/sprites?max_results=100".to_string();
            if let Some(t) = &token {
                path.push_str(&format!("&continuation_token={}", query_escape(t)));
            }
            let page = self.request(keys, reqwest::Method::GET, &path, None)?;
            out.extend(page["sprites"].as_array().into_iter().flatten().filter_map(box_info));
            token = page["next_continuation_token"].as_str().filter(|t| !t.is_empty()).map(String::from);
            if page["has_more"] != true || token.is_none() {
                break;
            }
        }
        Ok(out)
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let name = sprite_name(spec.name.as_deref());
        let reply = self.request(keys, reqwest::Method::POST, "/v1/sprites", Some(json!({ "name": name })))?;
        Ok(box_info(&reply).unwrap_or(BoxInfo { id: name.clone(), name, state: "cold".into(), running: false }))
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        // Sprites wake on the next exec; just make sure this one exists.
        self.request(keys, reqwest::Method::GET, &format!("/v1/sprites/{}", path_escape(id)), None).map(|_| ())
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        // No API for it: an idle sprite goes cold by itself.
        Ok(())
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        match self.send(keys, reqwest::Method::DELETE, &format!("/v1/sprites/{}", path_escape(id)), None)? {
            (200..=299 | 404, _) => Ok(()),
            (status, v) => bail!("Sprites delete: {}", api_error(status, &v)),
        }
    }

    fn connect(&self, keys: &Keys, id: &str, _dir: &Path) -> Result<Endpoint> {
        self.wake(keys, id)?;
        Ok(Endpoint::Native)
    }

    fn bridge(&self, keys: &Keys, id: &str, argv: &[String]) -> Result<i32> {
        let key = self.key(keys)?;
        let url = exec_url(id, argv)?;
        let auth = format!("Bearer {key}");
        let mut ws = stdio::ws_connect(&url, &[("Authorization", &auth)])?;
        let stdin = stdio::stdin_chunks();
        let mut stdin_open = true;
        loop {
            while stdin_open {
                match stdin.try_recv() {
                    Ok(Some(chunk)) => ws.send(Message::Binary(stdin_frame(&chunk).into()))?,
                    Ok(None) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        ws.send(Message::Binary(vec![STDIN_EOF].into()))?;
                        stdin_open = false;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                }
            }
            let msg = match ws.read() {
                Ok(m) => m,
                Err(e) if stdio::ws_timed_out(&e) => continue,
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    bail!("Sprites closed the exec stream before the command exited")
                }
                Err(e) => return Err(e).context("reading the Sprites exec stream"),
            };
            match frame(&msg) {
                Frame::Stdout(b) => stdio::out(b)?,
                Frame::Stderr(b) => stdio::err(b)?,
                Frame::Exit(code) => {
                    let _ = ws.close(None);
                    let _ = ws.flush();
                    return Ok(code);
                }
                Frame::Closed => bail!("Sprites closed the exec stream before the command exited"),
                Frame::Ignore => {}
            }
        }
    }
}

const STDIN: u8 = 0;
const STDOUT: u8 = 1;
const STDERR: u8 = 2;
const EXIT: u8 = 3;
const STDIN_EOF: u8 = 4;

fn stdin_frame(chunk: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(chunk.len() + 1);
    f.push(STDIN);
    f.extend_from_slice(chunk);
    f
}

#[derive(Debug, PartialEq, Eq)]
enum Frame<'a> {
    Stdout(&'a [u8]),
    Stderr(&'a [u8]),
    Exit(i32),
    Closed,
    Ignore,
}

fn frame(msg: &Message) -> Frame<'_> {
    match msg {
        Message::Binary(b) => match b.split_first() {
            Some((&STDOUT, rest)) => Frame::Stdout(rest),
            Some((&STDERR, rest)) => Frame::Stderr(rest),
            Some((&EXIT, rest)) => Frame::Exit(rest.first().copied().unwrap_or(0) as i32),
            _ => Frame::Ignore,
        },
        Message::Text(t) => {
            let v: Value = serde_json::from_str(t.as_str()).unwrap_or(Value::Null);
            if v["type"] == "exit" { Frame::Exit(v["exit_code"].as_i64().unwrap_or(0) as i32) } else { Frame::Ignore }
        }
        Message::Close(_) => Frame::Closed,
        _ => Frame::Ignore,
    }
}

/// `cmd` once per argv element, `path` = argv[0], stdin on, no TTY.
fn exec_url(name: &str, argv: &[String]) -> Result<String> {
    let program = argv.first().context("nothing to run")?;
    let mut url = format!("{WS}/v1/sprites/{}/exec?", path_escape(name));
    for a in argv {
        url.push_str(&format!("cmd={}&", query_escape(a)));
    }
    url.push_str(&format!("path={}&stdin=true", query_escape(program)));
    Ok(url)
}

fn box_info(s: &Value) -> Option<BoxInfo> {
    let name = s["name"].as_str()?.to_string();
    let state = s["status"].as_str().unwrap_or("unknown").to_string();
    Some(BoxInfo { id: name.clone(), name, running: state == "running", state })
}

/// Sprite names: lowercase letters, digits and dashes.
fn sprite_name(wanted: Option<&str>) -> String {
    let cleaned: String = wanted
        .unwrap_or_default()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
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

fn query_escape(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn path_escape(s: &str) -> String {
    query_escape(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_exec_frames() {
        let out = Message::Binary(vec![1, b'h', b'i'].into());
        assert_eq!(frame(&out), Frame::Stdout(b"hi"));
        let err = Message::Binary(vec![2, b'x'].into());
        assert_eq!(frame(&err), Frame::Stderr(b"x"));
        assert_eq!(frame(&Message::Binary(vec![3, 7].into())), Frame::Exit(7));
        assert_eq!(frame(&Message::Binary(vec![3].into())), Frame::Exit(0));
        assert_eq!(frame(&Message::Text(r#"{"type":"exit","exit_code":2}"#.into())), Frame::Exit(2));
        assert_eq!(frame(&Message::Text(r#"{"type":"session_info","session_id":"s"}"#.into())), Frame::Ignore);
        assert_eq!(frame(&Message::Binary(vec![].into())), Frame::Ignore);
        assert_eq!(frame(&Message::Close(None)), Frame::Closed);
        assert_eq!(stdin_frame(b"ls\n"), vec![0, b'l', b's', b'\n']);
    }

    #[test]
    fn exec_url_repeats_cmd_per_argument() {
        let argv: Vec<String> = ["sh", "-lc", "cd \"$0\" && exec \"$@\"", "proj", "claude"].map(String::from).to_vec();
        let url = exec_url("my box", &argv).unwrap();
        assert!(url.starts_with("wss://api.sprites.dev/v1/sprites/my%20box/exec?cmd=sh&cmd=-lc&cmd=cd%20%22%240%22"));
        assert!(url.ends_with("&cmd=proj&cmd=claude&path=sh&stdin=true"));
        assert!(exec_url("b", &[]).is_err());
    }

    #[test]
    fn sprites_become_boxes() {
        let b = box_info(&json!({"id": "sprite-1", "name": "dev", "status": "warm"})).unwrap();
        assert_eq!(b, BoxInfo { id: "dev".into(), name: "dev".into(), state: "warm".into(), running: false });
        assert!(box_info(&json!({"status": "running"})).is_none());
        assert_eq!(sprite_name(Some("My Box!")), "my-box");
        assert!(sprite_name(None).starts_with("keplorayx-"));
        assert_eq!(api_error(404, &json!({"error": "not found"})), "not found (HTTP 404)");
    }

    #[test]
    fn no_key_needs_setup() {
        assert!(matches!(PROVIDER.probe(&Keys::default()), Probe::NeedsSetup(_)));
    }
}
