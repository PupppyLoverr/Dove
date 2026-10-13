//! Loopback endpoint the `dove --mcp-browser` bridge calls into.
//!
//! One JSON line in (`{token, cwd, tool, args}`), one JSON line out
//! (`{text, image, isError}`). A random per-launch token keeps other local
//! processes out; the port and token reach agents through the MCP server's
//! environment.

use std::hash::{BuildHasher, Hasher};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt as _, AsyncWriteExt, BufReader};

use super::agent::Outcome;

pub struct Request {
    /// The agent's working directory: which thread is asking.
    pub cwd: Option<PathBuf>,
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

fn token() -> String {
    let mut out = String::new();
    for _ in 0..4 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        h.write_u32(std::process::id());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
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
                    Ok(req) if req.get("token").and_then(Value::as_str) == Some(want.as_str()) => {
                        let (rtx, rrx) = tokio::sync::oneshot::channel();
                        let request = Request {
                            cwd: req.get("cwd").and_then(Value::as_str).filter(|s| !s.is_empty()).map(PathBuf::from),
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

