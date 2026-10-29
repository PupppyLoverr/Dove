//! E2B (e2b.dev): Firecracker sandboxes over a REST control plane, with
//! stdio streamed through the in-sandbox daemon `envd` (route: native).
//!
//! - Control plane, `https://api.e2b.app`, header `X-API-Key`
//!   (https://e2b.dev/docs/api-reference, spec:
//!   https://github.com/e2b-dev/E2B/blob/main/spec/openapi.yml):
//!   `POST /v2/sandboxes` (template `base`, `timeout`, `autoPause`),
//!   `GET /v2/sandboxes?state=running,paused` (paged by `X-Next-Token`),
//!   `POST /v2/sandboxes/{id}/connect` (resumes a paused box and returns the
//!   envd token), `POST /sandboxes/{id}/pause`, `POST /sandboxes/{id}/timeout`,
//!   `DELETE /sandboxes/{id}`.
//! - Stdio: envd's Connect-RPC process service on
//!   `https://49983-<sandboxID>.<domain>` (spec:
//!   https://github.com/e2b-dev/E2B/blob/main/spec/envd/process/process.proto).
//!   `process.Process/Start` is a server stream (Connect protocol, JSON,
//!   envelopes of 1 flag byte + 4-byte big-endian length) started with
//!   `stdin: true`; input goes in with unary `SendInput` calls and EOF with
//!   `CloseStdin` (envd ≥ 0.5.2). No PTY, so bytes pass through untouched.
//!
//! Boxes are made with `autoPause`, so when their timeout runs out they pause
//! (keeping files and memory) instead of dying; the bridge keeps pushing the
//! timeout forward while an agent runs. The Hobby plan caps one continuous
//! session at 1 hour and Pro at 24 hours (https://e2b.dev/pricing).

use std::io::Read as _;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

const API: &str = "https://api.e2b.app";
const DEFAULT_DOMAIN: &str = "e2b.app";
const ENVD_PORT: u16 = 49983;
/// Seconds a box stays up after the last push; it auto-pauses after that.
const IDLE_SECS: u64 = 900;

pub struct E2b;
pub static PROVIDER: E2b = E2b;

impl E2b {
    fn key<'a>(&self, keys: &'a Keys) -> Result<&'a str> {
        keys.get(self.id()).context("no E2B API key: add one in Settings › Machines")
    }

    /// One control-plane call: the status, the JSON body and the response
    /// headers we page by.
    fn send(
        &self,
        keys: &Keys,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(u16, Value, Option<String>)> {
        let key = self.key(keys)?;
        let mut req = super::http().request(method, format!("{API}{path}")).header("X-API-Key", key);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().with_context(|| format!("calling E2B {path}"))?;
        let status = resp.status().as_u16();
        let next = resp.headers().get("x-next-token").and_then(|v| v.to_str().ok()).map(String::from);
        let text = resp.text().unwrap_or_default();
        Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null), next))
    }

    fn request(&self, keys: &Keys, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let (status, value, _) = self.send(keys, method, path, body)?;
        if !(200..300).contains(&status) {
            bail!("E2B {path}: {}", api_error(status, &value));
        }
        Ok(value)
    }

    /// Resume (if paused) and return the box's envd address and token.
    fn connect_envd(&self, keys: &Keys, id: &str) -> Result<Envd> {
        let reply = self.request(
            keys,
            reqwest::Method::POST,
            &format!("/v2/sandboxes/{id}/connect"),
            Some(json!({ "timeout": IDLE_SECS })),
        )?;
        Ok(Envd::from_reply(id, &reply))
    }
}

impl Provider for E2b {
    fn id(&self) -> &'static str {
        "e2b"
    }

    fn label(&self) -> &'static str {
        "E2B"
    }

    fn key_url(&self) -> &'static str {
        "https://e2b.dev/dashboard?tab=keys"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, keys: &Keys) -> Probe {
        if !keys.has(self.id()) {
            return Probe::NeedsSetup("Add your E2B API key (e2b_…)".into());
        }
        match self.send(keys, reqwest::Method::GET, "/v2/sandboxes?limit=1", None) {
            Ok((200..=299, _, _)) => Probe::Ready,
            Ok((401 | 403, _, _)) => Probe::NeedsSetup("E2B rejected the key".into()),
            Ok((status, v, _)) => Probe::Unavailable(format!("E2B: {}", api_error(status, &v))),
            Err(err) => Probe::Unavailable(format!("{err:#}")),
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..5 {
            let mut path = "/v2/sandboxes?state=running,paused&limit=100".to_string();
            if let Some(t) = &token {
                path.push_str(&format!("&nextToken={}", urlencode(t)));
            }
            let (status, page, next) = self.send(keys, reqwest::Method::GET, &path, None)?;
            if !(200..300).contains(&status) {
                bail!("E2B /v2/sandboxes: {}", api_error(status, &page));
            }
            out.extend(page.as_array().into_iter().flatten().filter_map(box_info));
            token = next.filter(|t| !t.is_empty());
            if token.is_none() {
                break;
            }
        }
        Ok(out)
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let mut metadata = serde_json::Map::new();
        metadata.insert("createdBy".into(), json!("keplorayx"));
        if let Some(n) = &spec.name {
            metadata.insert("name".into(), json!(n));
        }
        let body = json!({
            "templateID": spec.template.as_deref().unwrap_or("base"),
            "timeout": IDLE_SECS,
            "autoPause": true,
            "metadata": metadata,
        });
        let reply = self.request(keys, reqwest::Method::POST, "/v2/sandboxes", Some(body))?;
        let id = reply["sandboxID"].as_str().context("E2B did not say which sandbox it created")?.to_string();
        Ok(BoxInfo {
            name: spec.name.clone().unwrap_or_else(|| id.clone()),
            id,
            state: "running".into(),
            running: true,
        })
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        self.connect_envd(keys, id).map(|_| ())
    }

    fn sleep(&self, keys: &Keys, id: &str) -> Result<()> {
        let (status, v, _) = self.send(keys, reqwest::Method::POST, &format!("/sandboxes/{id}/pause"), None)?;
        match status {
            200..=299 | 409 => Ok(()), // 409: already paused
            _ => bail!("E2B pause: {}", api_error(status, &v)),
        }
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        let (status, v, _) = self.send(keys, reqwest::Method::DELETE, &format!("/sandboxes/{id}"), None)?;
        match status {
            200..=299 | 404 => Ok(()),
            _ => bail!("E2B delete: {}", api_error(status, &v)),
        }
    }

    fn connect(&self, keys: &Keys, id: &str, _dir: &Path) -> Result<Endpoint> {
        self.connect_envd(keys, id)?;
        Ok(Endpoint::Native)
    }

    fn bridge(&self, keys: &Keys, id: &str, argv: &[String]) -> Result<i32> {
        let (program, args) = argv.split_first().context("nothing to run")?;
        let envd = self.connect_envd(keys, id)?;
        let tag = format!("keplorayx-{}", uuid::Uuid::now_v7().simple());

        // Keep the box from pausing under a running agent.
        {
            let key = self.key(keys)?.to_string();
            let id = id.to_string();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(120));
                    let _ = super::http()
                        .post(format!("{API}/sandboxes/{id}/timeout"))
                        .header("X-API-Key", &key)
                        .json(&json!({ "timeout": IDLE_SECS }))
                        .send();
                }
            });
        }

        let start = json!({ "process": { "cmd": program, "args": args }, "tag": tag, "stdin": true });
        let mut resp = envd
            .post(&stdio::streaming_client(), "process.Process/Start")
            .header("Content-Type", "application/connect+json")
            .header("Keepalive-Ping-Interval", "50")
            .body(envelope(&start))
            .send()
            .context("starting the command on the E2B sandbox")?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let v: Value = serde_json::from_str(&resp.text().unwrap_or_default()).unwrap_or(Value::Null);
            bail!("E2B could not start the command: {}", api_error(status, &v));
        }

        // Stdin waits for the start event (it needs the pid), then sends each
        // chunk in order and closes stdin at EOF.
        let (started_tx, started_rx) = mpsc::channel::<u64>();
        {
            let envd = envd.clone();
            std::thread::spawn(move || {
                let Ok(pid) = started_rx.recv() else { return };
                let client = super::http();
                let chunks = stdio::stdin_chunks();
                while let Ok(Some(chunk)) = chunks.recv() {
                    let body = json!({ "process": { "pid": pid }, "input": { "stdin": stdio::b64_encode(&chunk) } });
                    let ok = envd.unary(&client, "process.Process/SendInput", &body).is_ok();
                    if !ok {
                        return;
                    }
                }
                let _ = envd.unary(&client, "process.Process/CloseStdin", &json!({ "process": { "pid": pid } }));
            });
        }

        let mut frames = Frames::default();
        let mut buf = vec![0u8; 64 * 1024];
        let mut started = Some(started_tx);
        loop {
            let n = match resp.read(&mut buf) {
                Ok(0) => bail!("the E2B stream ended before the command exited"),
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e).context("reading the E2B stream"),
            };
            frames.push(&buf[..n]);
            while let Some((flags, payload)) = frames.next_frame() {
                let msg: Value = serde_json::from_slice(&payload).unwrap_or(Value::Null);
                if flags & END_STREAM != 0 {
                    if msg["error"].is_object() {
                        bail!("E2B stream failed: {}", connect_error(&msg["error"]));
                    }
                    bail!("the E2B stream ended before the command exited");
                }
                match event(&msg) {
                    Event::Start(pid) => {
                        if let Some(tx) = started.take() {
                            let _ = tx.send(pid);
                        }
                    }
                    Event::Stdout(b) => stdio::out(&b)?,
                    Event::Stderr(b) => stdio::err(&b)?,
                    Event::End(code) => return Ok(code),
                    Event::Other => {}
                }
            }
        }
    }
}

/// Where a box's envd listens and how to authenticate to it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Envd {
    id: String,
    base: String,
    token: Option<String>,
}

impl Envd {
    fn from_reply(id: &str, reply: &Value) -> Self {
        let domain = reply["domain"].as_str().filter(|d| !d.is_empty()).unwrap_or(DEFAULT_DOMAIN);
        Envd {
            id: id.to_string(),
            base: format!("https://{ENVD_PORT}-{id}.{domain}"),
            token: reply["envdAccessToken"].as_str().filter(|t| !t.is_empty()).map(String::from),
        }
    }

    fn post(&self, client: &reqwest::blocking::Client, procedure: &str) -> reqwest::blocking::RequestBuilder {
        let mut req = client
            .post(format!("{}/{procedure}", self.base))
            .header("Connect-Protocol-Version", "1")
            // The sandbox's default user ("user", home /home/user).
            .header("Authorization", "Basic dXNlcjo=")
            .header("E2b-Sandbox-Id", &self.id)
            .header("E2b-Sandbox-Port", ENVD_PORT.to_string());
        if let Some(t) = &self.token {
            req = req.header("X-Access-Token", t);
        }
        req
    }

    fn unary(&self, client: &reqwest::blocking::Client, procedure: &str, body: &Value) -> Result<Value> {
        let resp = self
            .post(client, procedure)
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .with_context(|| format!("calling E2B {procedure}"))?;
        let status = resp.status().as_u16();
        let v: Value = serde_json::from_str(&resp.text().unwrap_or_default()).unwrap_or(Value::Null);
        if !(200..300).contains(&status) {
            bail!("E2B {procedure}: {} (HTTP {status})", connect_error(&v));
        }
        Ok(v)
    }
}

/// Connect envelope flag for the stream's closing message.
const END_STREAM: u8 = 0x02;

/// One Connect streaming envelope: flags, 4-byte big-endian length, JSON.
fn envelope(msg: &Value) -> Vec<u8> {
    let body = msg.to_string().into_bytes();
    let mut out = Vec::with_capacity(body.len() + 5);
    out.push(0);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// Reassembles Connect envelopes from a byte stream.
#[derive(Default)]
struct Frames {
    buf: Vec<u8>,
}

impl Frames {
    fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    fn next_frame(&mut self) -> Option<(u8, Vec<u8>)> {
        if self.buf.len() < 5 {
            return None;
        }
        let len = u32::from_be_bytes([self.buf[1], self.buf[2], self.buf[3], self.buf[4]]) as usize;
        if self.buf.len() < 5 + len {
            return None;
        }
        let flags = self.buf[0];
        let payload = self.buf[5..5 + len].to_vec();
        self.buf.drain(..5 + len);
        Some((flags, payload))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Event {
    Start(u64),
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    End(i32),
    Other,
}

/// A `StartResponse` (proto3 JSON: defaults omitted, bytes in base64).
fn event(msg: &Value) -> Event {
    let e = &msg["event"];
    if let Some(start) = e.get("start") {
        return Event::Start(start["pid"].as_u64().unwrap_or(0));
    }
    if let Some(data) = e.get("data") {
        if let Some(b) = data["stdout"].as_str() {
            return Event::Stdout(stdio::b64_decode(b).unwrap_or_default());
        }
        if let Some(b) = data["stderr"].as_str() {
            return Event::Stderr(stdio::b64_decode(b).unwrap_or_default());
        }
        return Event::Other;
    }
    if let Some(end) = e.get("end") {
        let code = end["exitCode"].as_i64().unwrap_or(0) as i32;
        // Killed by a signal: `exited` is false and there is no real code.
        let exited = end["exited"].as_bool().unwrap_or(false);
        return Event::End(if exited || code != 0 { code } else { 1 });
    }
    Event::Other
}

fn connect_error(e: &Value) -> String {
    let code = e["code"].as_str().unwrap_or("unknown");
    match e["message"].as_str() {
        Some(m) => format!("{code}: {m}"),
        None => code.to_string(),
    }
}

fn box_info(s: &Value) -> Option<BoxInfo> {
    let id = s["sandboxID"].as_str()?.to_string();
    let state = s["state"].as_str().unwrap_or("unknown").to_string();
    let name = s["metadata"]["name"].as_str().or(s["alias"].as_str()).unwrap_or(&id).to_string();
    Some(BoxInfo { name, running: state == "running", state, id })
}

/// `{"code":409,"message":"…"}` → `message (HTTP n)`.
fn api_error(status: u16, v: &Value) -> String {
    let msg = v["message"].as_str().unwrap_or("request failed");
    format!("{msg} (HTTP {status})")
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Helpers every native-stream provider shares (E2B, Fly Sprites,
/// Northflank, Blaxel, Vercel, DeepInfra): this process's stdio, base64,
/// shell quoting and a WebSocket that can be polled between stdin writes.
pub(super) mod stdio {
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    use std::sync::mpsc;
    use std::time::Duration;

    use anyhow::{Context as _, Result};

    /// Read this process's stdin on a thread. Each message is a chunk;
    /// `None` is EOF (also sent when stdin fails).
    pub fn stdin_chunks() -> mpsc::Receiver<Option<Vec<u8>>> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(Some(buf[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            let _ = tx.send(None);
        });
        rx
    }

    pub fn out(bytes: &[u8]) -> Result<()> {
        let mut o = std::io::stdout().lock();
        o.write_all(bytes).and_then(|_| o.flush()).context("writing to stdout")
    }

    pub fn err(bytes: &[u8]) -> Result<()> {
        let mut e = std::io::stderr().lock();
        e.write_all(bytes).and_then(|_| e.flush()).context("writing to stderr")
    }

    /// A client for long-lived streams: a connect timeout but no overall one.
    pub fn streaming_client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .timeout(None)
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("Keplorayx/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new())
    }

    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    /// Standard base64 with padding.
    pub fn b64_encode(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for c in data.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            out.push(B64[(n >> 18) as usize & 63] as char);
            out.push(B64[(n >> 12) as usize & 63] as char);
            out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
            out.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
        }
        out
    }

    /// Standard or URL-safe base64, padded or not; whitespace is skipped.
    pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(s.len() * 3 / 4);
        let (mut acc, mut bits) = (0u32, 0u32);
        for b in s.bytes() {
            let v = match b {
                b'A'..=b'Z' => b - b'A',
                b'a'..=b'z' => b - b'a' + 26,
                b'0'..=b'9' => b - b'0' + 52,
                b'+' | b'-' => 62,
                b'/' | b'_' => 63,
                b'=' => break,
                b' ' | b'\n' | b'\r' | b'\t' => continue,
                _ => return None,
            };
            acc = acc << 6 | v as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        Some(out)
    }

    /// POSIX single-quoting, for providers that take one command string.
    pub fn sh_quote(s: &str) -> String {
        if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./=:,@%+".contains(&b)) {
            return s.to_string();
        }
        format!("'{}'", s.replace('\'', r"'\''"))
    }

    pub fn sh_join(argv: &[String]) -> String {
        argv.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" ")
    }

    pub type Ws = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>;

    /// Open a WebSocket with extra headers, then give reads a short timeout
    /// so one thread can alternate between reading and sending stdin.
    pub fn ws_connect(url: &str, headers: &[(&str, &str)]) -> Result<Ws> {
        use tungstenite::client::IntoClientRequest as _;
        let mut req = url.into_client_request().context("building the WebSocket request")?;
        for (k, v) in headers {
            req.headers_mut().insert(
                tungstenite::http::HeaderName::from_bytes(k.as_bytes())?,
                tungstenite::http::HeaderValue::from_str(v)?,
            );
        }
        let (ws, _) = tungstenite::connect(req).map_err(|e| anyhow::anyhow!("opening the exec stream: {e}"))?;
        let tcp = match ws.get_ref() {
            tungstenite::stream::MaybeTlsStream::Plain(s) => Some(s),
            tungstenite::stream::MaybeTlsStream::Rustls(s) => Some(s.get_ref()),
            _ => None,
        };
        if let Some(tcp) = tcp {
            tcp.set_read_timeout(Some(Duration::from_millis(25)))?;
        }
        Ok(ws)
    }

    /// A read that only timed out (try again after sending stdin).
    pub fn ws_timed_out(e: &tungstenite::Error) -> bool {
        matches!(e, tungstenite::Error::Io(io)
            if matches!(io.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
    }

    /// Splits byte chunks into UTF-8 text without breaking a character
    /// across two chunks (for APIs whose stdin is a JSON string).
    #[derive(Default)]
    pub struct Utf8Carry {
        pending: Vec<u8>,
    }

    impl Utf8Carry {
        pub fn push(&mut self, chunk: &[u8]) -> String {
            self.pending.extend_from_slice(chunk);
            let mut out = String::new();
            loop {
                match std::str::from_utf8(&self.pending) {
                    Ok(s) => {
                        out.push_str(s);
                        self.pending.clear();
                        return out;
                    }
                    Err(e) => {
                        let good = e.valid_up_to();
                        out.push_str(std::str::from_utf8(&self.pending[..good]).unwrap_or_default());
                        match e.error_len() {
                            // Truly invalid bytes: replace and go on.
                            Some(bad) => {
                                out.push('\u{FFFD}');
                                self.pending.drain(..good + bad);
                            }
                            // An incomplete character at the end: keep it.
                            None => {
                                self.pending.drain(..good);
                                return out;
                            }
                        }
                    }
                }
            }
        }

        /// Whatever is left at EOF.
        pub fn finish(&mut self) -> String {
            let s = String::from_utf8_lossy(&self.pending).into_owned();
            self.pending.clear();
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelopes_round_trip_across_chunks() {
        let msg = json!({"process": {"cmd": "sh"}, "stdin": true});
        let bytes = envelope(&msg);
        assert_eq!(bytes[0], 0);
        assert_eq!(u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize, bytes.len() - 5);
        let mut f = Frames::default();
        f.push(&bytes[..3]);
        assert!(f.next_frame().is_none());
        f.push(&bytes[3..]);
        let mut end = vec![END_STREAM, 0, 0, 0, 2];
        end.extend_from_slice(b"{}");
        f.push(&end);
        let (flags, payload) = f.next_frame().unwrap();
        assert_eq!(flags, 0);
        assert_eq!(serde_json::from_slice::<Value>(&payload).unwrap(), msg);
        assert_eq!(f.next_frame().unwrap(), (END_STREAM, b"{}".to_vec()));
        assert!(f.next_frame().is_none());
    }

    #[test]
    fn reads_process_events() {
        assert_eq!(event(&json!({"event": {"start": {"pid": 42}}})), Event::Start(42));
        assert_eq!(event(&json!({"event": {"data": {"stdout": "aGkK"}}})), Event::Stdout(b"hi\n".to_vec()));
        assert_eq!(event(&json!({"event": {"data": {"stderr": "b29wcw=="}}})), Event::Stderr(b"oops".to_vec()));
        assert_eq!(event(&json!({"event": {"keepalive": {}}})), Event::Other);
        assert_eq!(event(&json!({"event": {"end": {"exitCode": 3, "exited": true}}})), Event::End(3));
        // Proto3 JSON omits zero values.
        assert_eq!(event(&json!({"event": {"end": {"exited": true, "status": "exit status 0"}}})), Event::End(0));
        assert_eq!(event(&json!({"event": {"end": {"status": "signal: killed"}}})), Event::End(1));
    }

    #[test]
    fn envd_address_uses_the_reply_domain() {
        let e = Envd::from_reply("sb1", &json!({"envdAccessToken": "tok", "domain": null}));
        assert_eq!(e.base, "https://49983-sb1.e2b.app");
        assert_eq!(e.token.as_deref(), Some("tok"));
        let e = Envd::from_reply("sb1", &json!({"domain": "eu.example.dev"}));
        assert_eq!((e.base.as_str(), e.token), ("https://49983-sb1.eu.example.dev", None));
    }

    #[test]
    fn sandboxes_become_boxes() {
        let b = box_info(&json!({"sandboxID": "i1", "state": "paused", "metadata": {"name": "work"}})).unwrap();
        assert_eq!(b, BoxInfo { id: "i1".into(), name: "work".into(), state: "paused".into(), running: false });
        let b = box_info(&json!({"sandboxID": "i2", "state": "running", "alias": "base"})).unwrap();
        assert!(b.running && b.name == "base");
        assert!(box_info(&json!({"state": "running"})).is_none());
        assert_eq!(api_error(409, &json!({"code": 409, "message": "already paused"})), "already paused (HTTP 409)");
    }

    #[test]
    fn base64_round_trips() {
        for s in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0, 255, 128, 7]] {
            assert_eq!(stdio::b64_decode(&stdio::b64_encode(s)).unwrap(), s);
        }
        assert_eq!(stdio::b64_encode(b"hi\n"), "aGkK");
        assert_eq!(stdio::b64_decode("aGk").unwrap(), b"hi");
        assert!(stdio::b64_decode("a*b").is_none());
    }

    #[test]
    fn utf8_carry_never_splits_a_character() {
        let mut c = stdio::Utf8Carry::default();
        let s = "héllo→".as_bytes();
        let (a, b) = s.split_at(2); // inside 'é'
        assert_eq!(c.push(a), "h");
        assert_eq!(c.push(b), "héllo→".trim_start_matches('h'));
        assert_eq!(c.push(&[0xff, b'x']), "\u{FFFD}x");
        assert_eq!(c.push(&[0xe2]), "");
        assert_eq!(c.finish(), "\u{FFFD}");
    }

    #[test]
    fn shell_quoting() {
        assert_eq!(stdio::sh_quote("plain-arg_1.txt"), "plain-arg_1.txt");
        assert_eq!(stdio::sh_quote(""), "''");
        assert_eq!(stdio::sh_quote("it's"), r"'it'\''s'");
        assert_eq!(stdio::sh_join(&["sh".into(), "-lc".into(), "cd \"$0\"".into()]), "sh -lc 'cd \"$0\"'");
        assert_eq!(urlencode("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn no_key_needs_setup() {
        assert!(matches!(PROVIDER.probe(&Keys::default()), Probe::NeedsSetup(_)));
    }
}
