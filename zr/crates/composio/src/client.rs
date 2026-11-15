//! Composio's hosted MCP server over streamable HTTP: each JSON-RPC message
//! is a POST; the reply comes back as JSON or as a short event stream. The
//! session id Composio hands out on `initialize` rides every later message.

use anyhow::{Context as _, bail};
use serde_json::{Value, json};

/// Somewhere JSON-RPC messages go: Composio, or a stand-in in tests.
#[async_trait::async_trait]
pub trait Upstream: Send + Sync {
    /// Sends one message: a request's reply, or `None` for a notification.
    async fn send(&self, message: Value) -> anyhow::Result<Option<Value>>;
}

pub struct Http {
    http: reqwest::Client,
    url: String,
    key: String,
    session: tokio::sync::Mutex<Option<String>>,
    version: tokio::sync::Mutex<Option<String>>,
}

impl Http {
    pub fn new(url: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder().timeout(std::time::Duration::from_secs(120)).build().unwrap_or_default(),
            url: url.into(),
            key: key.into(),
            session: Default::default(),
            version: Default::default(),
        }
    }

    /// Opened for the app's own use: `initialize`d, as a client called
    /// "keplorayx", ready for [`Http::call_tool`].
    pub async fn open(url: impl Into<String>, key: impl Into<String>) -> anyhow::Result<Self> {
        let http = Self::new(url, key);
        http.send(json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "keplorayx", "version": env!("CARGO_PKG_VERSION") } }
        }))
        .await?
        .and_then(|reply| reply.get("result").cloned())
        .context("Composio didn't answer")?;
        http.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await?;
        Ok(http)
    }

    /// One tool, called: its result (`content`, `isError`, …).
    pub async fn call_tool(&self, name: &str, arguments: Value) -> anyhow::Result<Value> {
        let reply = self
            .send(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": arguments } }))
            .await?
            .context("Composio didn't answer")?;
        if let Some(error) = reply.get("error") {
            bail!("{}", error.get("message").and_then(Value::as_str).unwrap_or("Composio refused"));
        }
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }
}

/// The reply to `id` among an event stream's messages.
fn from_events(body: &str, id: Option<&Value>) -> Option<Value> {
    let mut found = None;
    for event in body.split("\n\n") {
        let data: String = event
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n");
        let Ok(message) = serde_json::from_str::<Value>(&data) else { continue };
        let answers = message.get("result").is_some() || message.get("error").is_some();
        if answers && (id.is_none() || message.get("id") == id) {
            found = Some(message);
        }
    }
    found
}

#[async_trait::async_trait]
impl Upstream for Http {
    async fn send(&self, message: Value) -> anyhow::Result<Option<Value>> {
        let mut request = self
            .http
            .post(&self.url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("x-consumer-api-key", &self.key);
        if let Some(session) = self.session.lock().await.clone() {
            request = request.header("mcp-session-id", session);
        }
        if let Some(version) = self.version.lock().await.clone() {
            request = request.header("mcp-protocol-version", version);
        }
        let response = request.json(&message).send().await.context("Composio couldn't be reached")?;
        if let Some(session) = response.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *self.session.lock().await = Some(session.to_owned());
        }
        let status = response.status();
        let events = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("event-stream"));
        let body = response.text().await.unwrap_or_default();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            bail!("Composio didn't take the key ({status})");
        }
        if !status.is_success() {
            bail!("Composio answered {status}: {}", body.chars().take(200).collect::<String>());
        }
        if message.get("id").is_none() || body.trim().is_empty() {
            return Ok(None);
        }
        let reply = if events { from_events(&body, message.get("id")) } else { serde_json::from_str(&body).ok() };
        if let Some(version) = reply.as_ref().and_then(|r| r.pointer("/result/protocolVersion")).and_then(Value::as_str) {
            *self.version.lock().await = Some(version.to_owned());
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_stream_gives_the_reply_to_its_request() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n";
        let reply = from_events(body, Some(&json!(7))).unwrap();
        assert_eq!(reply["result"]["ok"], json!(true));
        assert!(from_events(body, Some(&json!(8))).is_none());
    }
}
