//! Newline-delimited JSON-RPC over a child's stdio, shared by the Codex
//! app-server and ACP adapters.

use std::collections::HashMap;

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::process::ChildStdin;

/// One decoded line from the server.
pub enum Incoming {
    /// Reply to a request we sent.
    Response {
        id: u64,
        result: Result<Value, String>,
    },
    /// A request the server is making of us (approvals, fs access, …).
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

pub fn decode(line: &str) -> Option<Incoming> {
    let v: Value = serde_json::from_str(line).ok()?;
    let method = v.get("method").and_then(Value::as_str).map(str::to_string);
    let id = v.get("id").filter(|i| !i.is_null()).cloned();
    Some(match (method, id) {
        (Some(method), Some(id)) => Incoming::Request { id, method, params: v["params"].clone() },
        (Some(method), None) => Incoming::Notification { method, params: v["params"].clone() },
        (None, Some(id)) => {
            let result = match v.get("error") {
                Some(e) if !e.is_null() => Err(e["message"].as_str().unwrap_or("request failed").to_string()),
                _ => Ok(v["result"].clone()),
            };
            Incoming::Response { id: id.as_u64()?, result }
        }
        (None, None) => return None,
    })
}

/// Writes requests with increasing ids and remembers what each was for.
pub struct Client<P> {
    stdin: ChildStdin,
    next: u64,
    /// Whether to include `"jsonrpc":"2.0"` (ACP does, Codex omits it).
    tagged: bool,
    pending: HashMap<u64, P>,
}

impl<P> Client<P> {
    pub fn new(stdin: ChildStdin, tagged: bool) -> Self {
        Self { stdin, next: 1, tagged, pending: HashMap::new() }
    }

    pub async fn request(&mut self, method: &str, params: Value, purpose: P) -> std::io::Result<u64> {
        let id = self.next;
        self.next += 1;
        self.pending.insert(id, purpose);
        let mut msg = json!({"id": id, "method": method, "params": params});
        self.tag(&mut msg);
        self.write(&msg).await?;
        Ok(id)
    }

    pub async fn notify(&mut self, method: &str, params: Option<Value>) -> std::io::Result<()> {
        let mut msg = json!({"method": method});
        if let Some(p) = params {
            msg["params"] = p;
        }
        self.tag(&mut msg);
        self.write(&msg).await
    }

    pub async fn respond(&mut self, id: Value, result: Value) -> std::io::Result<()> {
        let mut msg = json!({"id": id, "result": result});
        self.tag(&mut msg);
        self.write(&msg).await
    }

    pub async fn respond_error(&mut self, id: Value, code: i64, message: &str) -> std::io::Result<()> {
        let mut msg = json!({"id": id, "error": {"code": code, "message": message}});
        self.tag(&mut msg);
        self.write(&msg).await
    }

    pub fn take(&mut self, id: u64) -> Option<P> {
        self.pending.remove(&id)
    }

    fn tag(&self, msg: &mut Value) {
        if self.tagged {
            msg["jsonrpc"] = json!("2.0");
        }
    }

    async fn write(&mut self, msg: &Value) -> std::io::Result<()> {
        let mut line = msg.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await
    }
}
