//! Loopback endpoint the `dove --mcp-browser` bridge calls into.
//!
//! One JSON line in (`{token, cwd, tool, args}`), one JSON line out
//! (`{text, image, isError}`). A random per-launch token keeps other local
//! processes out; the port and token reach agents through the MCP server's
//! environment.
//!
//! `chrome_*` tools never reach the shell: they go to the Dove for Chrome
//! extension (apps/extension) over a second loopback listener that speaks
//! WebSocket. That port and its pairing token persist in
//! `~/.dove/extension.json`, so the extension pairs once.
//!
//! Who may connect: only a `chrome-extension://` origin (web pages can open
//! loopback sockets too) that presents the pairing token, and after the
//! first pairing only that same extension. Settings › Plugins › New
//! pairing code rotates the token and forgets the paired extension.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt as _, AsyncWriteExt, BufReader};

use super::agent::Outcome;

pub use extension::{connected_browser, connection_watch, new_pairing_code, pairing_code};

pub struct Request {
    /// The agent's working directory: which thread is asking.
    pub cwd: Option<PathBuf>,
    /// The calling session's key (`DOVE_SESSION`), which names its thread
    /// exactly even when several threads share a folder. Empty when unknown.
    pub session: String,
    pub tool: String,
    pub args: Value,
    pub reply: tokio::sync::oneshot::Sender<Outcome>,
}

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
}

static REQUESTS: OnceLock<Mutex<Option<flume::Receiver<Request>>>> = OnceLock::new();

/// The request stream, once (the shell owns it).
pub fn take_requests() -> Option<flume::Receiver<Request>> {
    REQUESTS.get()?.lock().ok()?.take()
}

/// 64 hex characters (244 random bits) from the operating system's CSPRNG.
fn token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// A short label for the thread asking ("dove" for `C:/src/dove`), shown in
/// the extension's activity log so the user sees which agent acted.
fn thread_label(cwd: Option<&str>) -> Option<String> {
    let name = std::path::Path::new(cwd?).file_name()?.to_string_lossy().to_string();
    (!name.is_empty()).then(|| name.chars().take(60).collect())
}

/// Files an agent may hand to `chrome_upload`: existing files inside its
/// own working folder. Anything else (a path outside the project, a folder,
/// a missing file) is refused before the browser sees it.
pub fn check_upload_paths(cwd: Option<&std::path::Path>, paths: &[String]) -> Result<Vec<String>, String> {
    let root = cwd
        .and_then(|c| c.canonicalize().ok())
        .ok_or("chrome_upload needs the thread's working folder; upload files from inside the project.")?;
    if paths.is_empty() || paths.len() > 10 {
        return Err("chrome_upload takes 1 to 10 file paths.".into());
    }
    paths
        .iter()
        .map(|p| {
            let path = std::path::Path::new(p);
            let full = if path.is_absolute() { path.to_path_buf() } else { root.join(path) };
            let full = full.canonicalize().map_err(|_| format!("{p} does not exist."))?;
            if !full.starts_with(&root) {
                return Err(format!("{p} is outside this thread's folder; Dove only uploads project files."));
            }
            if !full.is_file() {
                return Err(format!("{p} is not a file."));
            }
            // The browser wants a plain path (no \\?\ prefix on Windows).
            Ok(full.display().to_string().trim_start_matches(r"\\?\").to_string())
        })
        .collect()
}

/// Bind 127.0.0.1 on a free port and start serving on `rt`.
pub fn start(rt: &tokio::runtime::Handle) -> Option<Endpoint> {
    let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?;
    std_listener.set_nonblocking(true).ok()?;
    let port = std_listener.local_addr().ok()?.port();
    let endpoint = Endpoint { port, token: token() };
    let (tx, rx) = flume::unbounded::<Request>();
    REQUESTS.get_or_init(|| Mutex::new(None)).lock().ok()?.replace(rx);
    let want = endpoint.token.clone();
    extension::start(rt);
    rt.spawn(async move {
        let Ok(listener) = tokio::net::TcpListener::from_std(std_listener) else { return };
        loop {
            let Ok((stream, _)) = listener.accept().await else { continue };
            let tx = tx.clone();
            let want = want.clone();
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read.take(4 * 1024 * 1024)).lines();
                let Ok(Some(line)) = lines.next_line().await else { return };
                let reply = match serde_json::from_str::<Value>(&line) {
                    Ok(req)
                        if req.get("token").and_then(Value::as_str) == Some(want.as_str())
                            && req.get("tool").and_then(Value::as_str).is_some_and(|t| t.starts_with("chrome_")) =>
                    {
                        let tool = req.get("tool").and_then(Value::as_str).unwrap_or_default();
                        let cwd = req.get("cwd").and_then(Value::as_str).filter(|s| !s.is_empty());
                        let mut args = req.get("args").cloned().filter(Value::is_object).unwrap_or(json!({}));
                        if let Some(label) = thread_label(cwd) {
                            args["context"] = json!(label);
                        }
                        let checked = if tool == "chrome_upload" {
                            let paths: Vec<String> = args
                                .get("paths")
                                .and_then(Value::as_array)
                                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                                .unwrap_or_default();
                            check_upload_paths(cwd.map(std::path::Path::new), &paths).map(|ok| {
                                args["paths"] = json!(ok);
                            })
                        } else {
                            Ok(())
                        };
                        match checked {
                            Ok(()) => extension::call(&tool["chrome_".len()..], args).await,
                            Err(e) => Outcome::err(e),
                        }
                    }
                    Ok(req) if req.get("token").and_then(Value::as_str) == Some(want.as_str()) => {
                        let (rtx, rrx) = tokio::sync::oneshot::channel();
                        let request = Request {
                            cwd: req.get("cwd").and_then(Value::as_str).filter(|s| !s.is_empty()).map(PathBuf::from),
                            session: req.get("session").and_then(Value::as_str).unwrap_or_default().to_string(),
                            tool: req.get("tool").and_then(Value::as_str).unwrap_or_default().to_string(),
                            args: req.get("args").cloned().unwrap_or(json!({})),
                            reply: rtx,
                        };
                        if tx.send(request).is_err() {
                            Outcome::err("Dove is shutting down")
                        } else {
                            rrx.await.unwrap_or_else(|_| Outcome::err("Dove dropped the request"))
                        }
                    }
                    Ok(_) => Outcome::err("Bad token"),
                    Err(e) => Outcome::err(format!("Bad request: {e}")),
                };
                let mut out = json!({"text": reply.text, "isError": reply.is_error});
                if let Some(image) = reply.image {
                    out["image"] = json!(image);
                }
                let mut bytes = out.to_string().into_bytes();
                bytes.push(b'\n');
                let _ = write.write_all(&bytes).await;
                let _ = write.shutdown().await;
            });
        }
    });
    Some(endpoint)
}

mod extension {
    //! A minimal RFC 6455 server for exactly one client: the extension's
    //! service worker. Text frames only; the extension's first message must
    //! carry the pairing token.

    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::{mpsc, oneshot};

    use super::Outcome;

    struct Conn {
        generation: u64,
        tx: mpsc::UnboundedSender<String>,
        browser: String,
    }

    #[derive(Default)]
    struct State {
        conn: Option<Conn>,
        pending: HashMap<u64, oneshot::Sender<Value>>,
        pairing: Option<(u16, String)>,
        /// The extension origin that paired first; others are refused.
        paired_origin: Option<String>,
    }

    /// Largest message accepted from the extension (a screenshot is a few MB).
    const MAX_MESSAGE: u64 = 16 * 1024 * 1024;

    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    static NEXT: AtomicU64 = AtomicU64::new(1);

    fn state() -> std::sync::MutexGuard<'static, State> {
        STATE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The code the user pastes into the extension: `dove-<port>-<token>`.
    pub fn pairing_code() -> Option<String> {
        state().pairing.as_ref().map(|(port, token)| format!("dove-{port}-{token}"))
    }

    /// The connected browser's user agent, when the extension is paired and live.
    pub fn connected_browser() -> Option<String> {
        state().conn.as_ref().map(|c| c.browser.clone())
    }

    fn changes() -> &'static tokio::sync::watch::Sender<u64> {
        static CHANGES: OnceLock<tokio::sync::watch::Sender<u64>> = OnceLock::new();
        CHANGES.get_or_init(|| tokio::sync::watch::channel(0).0)
    }

    /// Wakes when the extension connects or disconnects, so views that show
    /// its state redraw without polling.
    pub fn connection_watch() -> tokio::sync::watch::Receiver<u64> {
        changes().subscribe()
    }

    fn connection_changed() {
        changes().send_modify(|n| *n = n.wrapping_add(1));
    }

    fn config_path() -> std::path::PathBuf {
        dove_core::store::default_root().join("extension.json")
    }

    fn save(port: u16, token: &str, origin: Option<&str>) {
        let path = config_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut body = json!({"port": port, "token": token, "protocol": 1});
        if let Some(o) = origin {
            body["origin"] = json!(o);
        }
        if std::fs::write(&path, body.to_string()).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }

    /// Settings › Plugins › Dove for Chrome › New pairing code: a fresh token, the paired
    /// extension forgotten, and the current connection dropped.
    pub fn new_pairing_code() -> Option<String> {
        let mut st = state();
        let (port, _) = st.pairing.clone()?;
        let token = super::token();
        st.pairing = Some((port, token.clone()));
        st.paired_origin = None;
        if let Some(conn) = st.conn.take() {
            drop(conn.tx);
        }
        st.pending.clear();
        drop(st);
        save(port, &token, None);
        connection_changed();
        pairing_code()
    }

    fn bind() -> Option<(std::net::TcpListener, String)> {
        let saved: Value =
            std::fs::read_to_string(config_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        state().paired_origin = saved
            .get("origin")
            .and_then(Value::as_str)
            .filter(|o| o.starts_with("chrome-extension://"))
            .map(str::to_string);
        let token = saved
            .get("token")
            .and_then(Value::as_str)
            .filter(|t| t.len() >= 32 && t.chars().all(|c| c.is_ascii_alphanumeric()))
            .map(str::to_string)
            .unwrap_or_else(super::token);
        let saved_port = saved.get("port").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok());
        let listener = saved_port
            .and_then(|p| std::net::TcpListener::bind(("127.0.0.1", p)).ok())
            .or_else(|| std::net::TcpListener::bind(("127.0.0.1", 0)).ok())?;
        let port = listener.local_addr().ok()?.port();
        let origin = state().paired_origin.clone();
        save(port, &token, origin.as_deref());
        Some((listener, token))
    }

    pub fn start(rt: &tokio::runtime::Handle) {
        let Some((std_listener, token)) = bind() else { return };
        if std_listener.set_nonblocking(true).is_err() {
            return;
        }
        let port = std_listener.local_addr().map(|a| a.port()).unwrap_or_default();
        state().pairing = Some((port, token.clone()));
        rt.spawn(async move {
            let Ok(listener) = tokio::net::TcpListener::from_std(std_listener) else { return };
            loop {
                let Ok((stream, _)) = listener.accept().await else { continue };
                let token = token.clone();
                tokio::spawn(async move {
                    let _ = serve(stream, &token).await;
                });
            }
        });
    }

    /// Run one `chrome_*` tool in the user's browser.
    pub async fn call(tool: &str, args: Value) -> Outcome {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut st = state();
            let Some(conn) = st.conn.as_ref() else {
                return Outcome::err(
                    "Dove for Chrome is not connected. Ask the user to install the extension and pair it in \
                     Dove → Settings → Plugins, or use the in-app browser_* tools instead.",
                );
            };
            let msg = json!({"type": "call", "id": id, "tool": tool, "args": args}).to_string();
            if conn.tx.send(msg).is_err() {
                return Outcome::err("Dove for Chrome just disconnected.");
            }
            st.pending.insert(id, tx);
        }
        // The per-site allow prompt can wait up to two minutes for the user.
        let reply = tokio::time::timeout(Duration::from_secs(140), rx).await;
        state().pending.remove(&id);
        let Ok(Ok(reply)) = reply else {
            return Outcome::err("The browser did not answer in time (the extension disconnected or the tab hung).");
        };
        if reply.get("ok").and_then(Value::as_bool) != Some(true) {
            return Outcome::err(
                reply.get("error").and_then(Value::as_str).unwrap_or("The browser reported an error."),
            );
        }
        Outcome {
            text: reply.get("text").and_then(Value::as_str).unwrap_or_default().to_string(),
            image: reply.pointer("/image/data").and_then(Value::as_str).map(str::to_string),
            is_error: false,
        }
    }

    async fn serve(mut stream: tokio::net::TcpStream, token: &str) -> std::io::Result<()> {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if head.len() > 16 * 1024 || stream.read(&mut byte).await? == 0 {
                return Ok(());
            }
            head.push(byte[0]);
        }
        let head = String::from_utf8_lossy(&head).to_string();
        let header = |name: &str| {
            head.lines().find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
            })
        };
        let origin = header("origin").unwrap_or_default();
        let key = header("sec-websocket-key");
        // Web pages can open loopback WebSockets too; only extensions may talk here.
        let from_extension = origin.starts_with("chrome-extension://");
        let (Some(key), true, true) = (key, from_extension, head.starts_with("GET /extension")) else {
            stream.write_all(b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await?;
            return Ok(());
        };
        let accept = base64(&sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes()));
        stream
            .write_all(
                format!(
                    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await?;
        let (mut read, mut write) = stream.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel::<(u8, Vec<u8>)>();
        let writer = tokio::spawn(async move {
            loop {
                let (op, payload) = tokio::select! {
                    m = rx.recv() => match m { Some(m) => (1u8, m.into_bytes()), None => break },
                    c = ctl_rx.recv() => match c { Some(c) => c, None => break },
                };
                if write.write_all(&frame(op, &payload)).await.is_err() || op == 8 {
                    break;
                }
            }
        });
        let generation = NEXT.fetch_add(1, Ordering::Relaxed);
        let mut paired = false;
        let mut message = Vec::new();
        loop {
            let Ok(Some((fin, op, payload))) = read_frame(&mut read).await else { break };
            match op {
                8 => break,
                9 => {
                    let _ = ctl_tx.send((10, payload));
                    continue;
                }
                10 => continue,
                0..=2 => message.extend_from_slice(&payload),
                _ => break,
            }
            if !fin {
                continue;
            }
            let text = String::from_utf8_lossy(&std::mem::take(&mut message)).to_string();
            let Ok(msg) = serde_json::from_str::<Value>(&text) else { continue };
            match msg.get("type").and_then(Value::as_str) {
                Some("hello") if !paired => {
                    let current = state().pairing.as_ref().map(|(_, t)| t.clone()).unwrap_or_else(|| token.to_string());
                    let pinned = state().paired_origin.clone();
                    if pinned.as_deref().is_some_and(|p| p != origin) {
                        let _ = tx.send(json!({"type": "rejected", "reason": "This Dove is paired with a different browser extension. To switch, choose New pairing code in Dove → Settings → Plugins."}).to_string());
                        let _ = ctl_tx.send((8, Vec::new()));
                        break;
                    }
                    if msg.get("token").and_then(Value::as_str) != Some(current.as_str()) {
                        let _ = tx.send(json!({"type": "rejected", "reason": "Pairing code does not match this Dove. Copy it again from Settings → Plugins."}).to_string());
                        let _ = ctl_tx.send((8, Vec::new()));
                        break;
                    }
                    paired = true;
                    let browser = msg.get("agent").and_then(Value::as_str).unwrap_or("Chromium").to_string();
                    if pinned.is_none() {
                        state().paired_origin = Some(origin.clone());
                        if let Some((port, t)) = state().pairing.clone() {
                            save(port, &t, Some(&origin));
                        }
                    }
                    let mut st = state();
                    // A newer connection (another profile, a reload) replaces the old one.
                    st.conn = Some(Conn { generation, tx: tx.clone(), browser });
                    drop(st);
                    connection_changed();
                    let _ = tx.send(
                        json!({"type": "welcome", "app": format!("Dove {}", env!("CARGO_PKG_VERSION"))}).to_string(),
                    );
                }
                Some("result") if paired => {
                    let id = msg.get("id").and_then(Value::as_u64).unwrap_or_default();
                    if let Some(reply) = state().pending.remove(&id) {
                        let _ = reply.send(msg);
                    }
                }
                _ => {}
            }
        }
        {
            let mut st = state();
            if st.conn.as_ref().is_some_and(|c| c.generation == generation) {
                st.conn = None;
                st.pending.clear();
                drop(st);
                connection_changed();
            }
        }
        writer.abort();
        Ok(())
    }

    async fn read_frame(read: &mut tokio::net::tcp::OwnedReadHalf) -> std::io::Result<Option<(bool, u8, Vec<u8>)>> {
        let mut h = [0u8; 2];
        if read.read_exact(&mut h).await.is_err() {
            return Ok(None);
        }
        let fin = h[0] & 0x80 != 0;
        let op = h[0] & 0x0f;
        let masked = h[1] & 0x80 != 0;
        let mut len = u64::from(h[1] & 0x7f);
        if len == 126 {
            len = u64::from(read.read_u16().await?);
        } else if len == 127 {
            len = read.read_u64().await?;
        }
        if len > MAX_MESSAGE {
            return Ok(None);
        }
        let mut mask = [0u8; 4];
        if masked {
            read.read_exact(&mut mask).await?;
        }
        let mut payload = vec![0u8; len as usize];
        read.read_exact(&mut payload).await?;
        if masked {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
        }
        Ok(Some((fin, op, payload)))
    }

    fn frame(op: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0x80 | op];
        match payload.len() {
            n if n < 126 => out.push(n as u8),
            n if n <= u16::MAX as usize => {
                out.push(126);
                out.extend_from_slice(&(n as u16).to_be_bytes());
            }
            n => {
                out.push(127);
                out.extend_from_slice(&(n as u64).to_be_bytes());
            }
        }
        out.extend_from_slice(payload);
        out
    }

    pub(super) fn sha1(data: &[u8]) -> [u8; 20] {
        let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
        let mut msg = data.to_vec();
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
        for chunk in msg.chunks(64) {
            let mut w = [0u32; 80];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
            }
            for i in 16..80 {
                w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
            }
            let [mut a, mut b, mut c, mut d, mut e] = h;
            for (i, wi) in w.iter().enumerate() {
                let (f, k) = match i {
                    0..=19 => ((b & c) | (!b & d), 0x5A827999),
                    20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                    40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                    _ => (b ^ c ^ d, 0xCA62C1D6),
                };
                let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
                e = d;
                d = c;
                c = b.rotate_left(30);
                b = a;
                a = t;
            }
            for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
                *x = x.wrapping_add(y);
            }
        }
        let mut out = [0u8; 20];
        for (i, v) in h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
        }
        out
    }

    pub(super) fn base64(data: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in data.chunks(3) {
            let n = (u32::from(c[0]) << 16)
                | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
                | u32::from(*c.get(2).unwrap_or(&0));
            out.push(T[(n >> 18) as usize & 63] as char);
            out.push(T[(n >> 12) as usize & 63] as char);
            out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
            out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pairing_tokens_are_long_random_and_alphanumeric() {
        let (a, b) = (super::token(), super::token());
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn uploads_are_limited_to_files_in_the_thread_folder() {
        let dir = std::env::temp_dir().join(format!("dove-upload-check-{}", std::process::id()));
        let project = dir.join("project");
        std::fs::create_dir_all(project.join("docs")).unwrap();
        std::fs::write(project.join("docs/cv.pdf"), b"pdf").unwrap();
        std::fs::write(dir.join("secret.txt"), b"no").unwrap();
        let ok = super::check_upload_paths(Some(&project), &["docs/cv.pdf".into()]).unwrap();
        assert!(ok[0].ends_with("cv.pdf") && !ok[0].starts_with(r"\\?\"), "{ok:?}");
        for bad in
            [dir.join("secret.txt").display().to_string(), "../secret.txt".into(), "docs".into(), "missing.pdf".into()]
        {
            assert!(super::check_upload_paths(Some(&project), std::slice::from_ref(&bad)).is_err(), "{bad}");
        }
        assert!(super::check_upload_paths(None, &["docs/cv.pdf".into()]).is_err());
        assert!(super::check_upload_paths(Some(&project), &[]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn thread_labels_are_the_folder_name() {
        assert_eq!(super::thread_label(Some("C:/src/dove")).as_deref(), Some("dove"));
        assert_eq!(super::thread_label(None), None);
    }

    #[test]
    fn websocket_accept_matches_rfc6455() {
        // The example handshake from RFC 6455 §1.3.
        let key = "dGhlIHNhbXBsZSBub25jZQ==258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
        assert_eq!(super::extension::base64(&super::extension::sha1(key.as_bytes())), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }
}
