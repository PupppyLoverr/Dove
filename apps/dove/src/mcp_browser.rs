//! `dove --mcp-browser`: a stdio MCP server that hands an agent Dove's
//! in-app browser.
//!
//! Dove injects it into every harness it starts (Claude Code, Codex, ACP
//! agents, …). Each `tools/call` is forwarded over loopback to the running
//! app (`browser::control`), which routes it to the browser of the thread
//! whose working directory matches. No async runtime: one blocking request
//! at a time is all an agent's tool loop needs.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{Value, json};

const INSTRUCTIONS: &str = "Tools from Dove, the app the user is chatting with you in.\n\
Browser: Dove has an in-app browser the user watches in the side panel. Start with browser_navigate \
(localhost dev servers work), then browser_snapshot for an outline whose elements carry refs like \
[ref=e12]; pass refs to browser_click, browser_type, browser_hover and friends. Actions return a fresh \
snapshot; browser_screenshot shows you the pixels.\n\
Visual answers: call show_ui to put an interactive card in the chat (stats, bar/line charts, tables, \
progress, checklists, toggles, steppers, choices and action buttons) whenever something is clearer \
shown than told, or when you want the user to pick or tune values. Their choice comes back as their \
next message. Dove's chat also renders ```mermaid diagrams and $$ LaTeX $$ math.";

/// `show_ui`: rendered by Dove's chat straight from the tool call.
fn show_ui_tool() -> Value {
    json!({
        "name": "show_ui",
        "description": "Show the user an interactive card inline in the chat. Use it for dashboards, comparisons, charts, plans to tick off, settings to tune, or choices to make. If you add actions, the user's pick plus all control values arrive as their next message.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": {"type": "string"},
                "body": {"type": "string", "description": "Short intro text."},
                "blocks": {
                    "type": "array",
                    "description": "Content, top to bottom. Each item has a `type`: text {text}; callout {tone: info|success|warning|danger, title?, text}; stats {items: [{label, value, delta?}]}; bars {title?, items: [{label, value}], unit?, horizontal?}; line {title?, values: [number], labels?: [string], unit?}; table {columns: [string], rows: [[any]]}; progress {label, value 0-1}; kv {items: [[key, value]]}; code {lang?, text}; image {src, alt?}; divider {}. Controls the user can change: choice {id, label, choices: [string], value?}; toggle {id, label, value: bool}; number {id, label, value, min?, max?, step?}; checklist {id, label?, items: [{label, checked?}]}.",
                    "items": {"type": "object", "properties": {"type": {"type": "string"}}, "required": ["type"]}
                },
                "actions": {
                    "type": "array",
                    "description": "Buttons; the first is primary.",
                    "items": {"type": "object", "properties": {"id": {"type": "string"}, "label": {"type": "string"}}, "required": ["label"]}
                }
            },
            "required": ["title"]
        }
    })
}

pub fn run() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            let _ = writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}}));
            let _ = out.flush();
            continue;
        };
        let Some(id) = msg.get("id").cloned() else { continue }; // notification
        let method = msg.get("method").and_then(Value::as_str).unwrap_or_default();
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        let reply = match method {
            "initialize" => Ok(json!({
                "protocolVersion": params.get("protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "dove", "title": "Dove browser", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            })),
            "ping" => Ok(json!({})),
            "tools/list" => {
                let mut tools = crate::browser::agent::tools();
                if let Some(list) = tools.as_array_mut() {
                    list.push(show_ui_tool());
                }
                Ok(json!({"tools": tools}))
            }
            "tools/call" if params.get("name").and_then(Value::as_str) == Some("show_ui") => Ok(json!({
                "content": [{"type": "text", "text": "Shown to the user inline in the chat. If it has actions, their choice arrives as their next message."}],
                "isError": false
            })),
            "tools/call" => Ok(call(
                params.get("name").and_then(Value::as_str).unwrap_or_default(),
                params.get("arguments").cloned().unwrap_or(json!({})),
            )),
            "resources/list" => Ok(json!({"resources": []})),
            "prompts/list" => Ok(json!({"prompts": []})),
            _ => Err(json!({"code": -32601, "message": format!("Method not found: {method}")})),
        };
        let msg = match reply {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
        };
        if writeln!(out, "{msg}").is_err() || out.flush().is_err() {
            break;
        }
    }
}

fn call(tool: &str, args: Value) -> Value {
    match forward(tool, args) {
        Ok(v) => {
            let mut content = vec![json!({"type": "text", "text": v.get("text").and_then(Value::as_str).unwrap_or("")})];
            if let Some(image) = v.get("image").and_then(Value::as_str) {
                content.push(json!({"type": "image", "data": image, "mimeType": "image/png"}));
            }
            json!({"content": content, "isError": v.get("isError").and_then(Value::as_bool).unwrap_or(false)})
        }
        Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
    }
}

fn forward(tool: &str, args: Value) -> Result<Value, String> {
    let port: u16 = std::env::var("DOVE_BROWSER_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .ok_or("Dove's browser is not available (DOVE_BROWSER_PORT is unset).")?;
    let token = std::env::var("DOVE_BROWSER_TOKEN").unwrap_or_default();
    let cwd = std::env::var("DOVE_CWD")
        .ok()
        .or_else(|| std::env::current_dir().ok().map(|d| d.display().to_string()))
        .unwrap_or_default();
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .map_err(|_| "Dove is not running, so its browser is unavailable.".to_string())?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(150)));
    let request = json!({"token": token, "cwd": cwd, "tool": tool, "args": args});
    stream.write_all(format!("{request}\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream.take(64 * 1024 * 1024)).read_line(&mut line).map_err(|e| e.to_string())?;
    serde_json::from_str(&line).map_err(|_| "Dove closed the connection without answering.".to_string())
}
