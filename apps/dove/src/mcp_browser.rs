//! `dove --mcp-browser`: a stdio MCP server that hands an agent Dove's
//! in-app browser.
//!
//! Dove injects it into every harness it starts (Claude Code, Codex, ACP
//! agents, …). Each `tools/call` is forwarded over loopback to the running
//! app (`browser::control`), which routes it to the browser of the thread
//! whose working directory matches. `chrome_*` tools take the same path and
//! the app hands them to the Dove for Chrome extension instead.
//!
//! It also fronts the optional integrations (Settings → Integrations):
//! Composio, Paper and cua-driver run as upstream MCP servers whose tools are
//! re-exported here as `composio__*`, `paper__*` and `computer__*`, so every
//! harness Dove starts gets them through the one server Dove already injects.
//! One blocking request at a time is all an agent's tool loop needs.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{Value, json};

const BASE_INSTRUCTIONS: &str = "Tools from Dove, the app the user is chatting with you in.\n\
Browser: Dove has an in-app browser the user watches in the side panel. Start with browser_navigate \
(localhost dev servers work), then browser_snapshot for an outline whose elements carry refs like \
[ref=e12]; pass refs to browser_click, browser_type, browser_hover and friends. Actions return a fresh \
snapshot; browser_screenshot shows you the pixels.\n\
Visual answers: call show_ui to put an interactive card in the chat (stats, bar/line charts, tables, \
progress, checklists, toggles, steppers, choices and action buttons) whenever something is clearer \
shown than told, or when you want the user to pick or tune values. Their choice comes back as their \
next message. Dove's chat also renders ```mermaid diagrams and $$ LaTeX $$ math.\n\
Designs: call show_canvas to show a live, interactive design in Dove's canvas (a framed webview the user can \
switch between desktop, tablet and phone): web pages, app screens, mobile mockups, dashboards, components. Send \
kind html (a full document or a fragment; Tailwind is loaded for fragments) or react (JSX/TSX defining `App`; \
React hooks and Tailwind are available, no imports needed) or svg. Reuse the same id to iterate in place. Make \
it real: realistic content, a consistent spacing scale, accessible contrast, responsive layout. To let the user \
answer from the design, call `dove.send(text)` from a button or give an element data-dove-send=\"choice\"; it \
arrives as their next message.\n\
Office documents: for .pdf, .pptx, .xlsx or .docx work, call dove_skills with {\"name\": \"pdf\"|\"pptx\"|\"xlsx\"|\"docx\"} \
first. Each skill ships a ready script (python-docx, python-pptx, openpyxl, reportlab, pypdf) you run with \
`uv run <script>`; it gives real layouts, styles, formulas and charts. The user previews and edits the result \
in Dove's file panel.";

/// `dove_skills`: Dove's bundled office skills (`skills/` in the repo).
fn skills_tool() -> Value {
    json!({
        "name": "dove_skills",
        "description": "Dove's office skills: pdf (create, merge, split, fill forms, extract), pptx (decks from outlines on real layouts, edit slides), xlsx (workbooks with formulas, formatting, charts), docx (Word documents with styles, tables, images). Without a name it lists them; with a name it returns that skill's full instructions and the folder holding its script.",
        "inputSchema": {
            "type": "object",
            "properties": {"name": {"type": "string", "enum": ["pdf", "pptx", "xlsx", "docx"]}}
        }
    })
}

const CHROME_INSTRUCTIONS: &str = "\nUser's own browser: chrome_* tools act in the user's real Chrome (their logins, \
cookies and open tabs) through the Dove for Chrome extension, only on sites they allow. Use them when the task \
needs the user's signed-in session (their inbox, dashboards, internal tools); use browser_* for everything else, \
like local dev servers. chrome_tabs lists tabs; chrome_navigate or chrome_tab_open loads a page; chrome_snapshot \
returns an outline with refs like [ref=e12] for chrome_click, chrome_type, chrome_select, chrome_hover; take a new \
snapshot after the page changes. Never submit payments, send messages or delete data without the user's go-ahead.";

const PAPER_INSTRUCTIONS: &str = "\nPaper: paper__* tools read and edit the design file open in Paper Desktop. Use \
them when the user mentions Paper, a design, a mockup or a screen to build: read the current selection or \
artboard before implementing UI so code matches the design, and write back to Paper when asked to explore or \
iterate on visuals.";

const COMPOSIO_INSTRUCTIONS: &str = "\nComposio: composio__* tools reach the user's third-party apps (GitHub, Gmail, \
Slack, Notion, …) with accounts they connect through Composio. Search for the right tool first; if an app is not \
connected, give the user the authorization link it returns.";

const COMPUTER_INSTRUCTIONS: &str = "\nComputer use: computer__* tools (the open-source cua-driver) see and operate \
native desktop apps. Prefer files, CLIs and browser tools; use computer use only for apps with no other interface, \
observe the window before acting, and confirm destructive actions with the user.";

/// `show_canvas`: a live design, rendered by Dove's canvas from the call.
fn show_canvas_tool() -> Value {
    json!({
        "name": "show_canvas",
        "description": "Show the user a live, interactive design in Dove's canvas: a website, an app or mobile screen, a dashboard, a component, a diagram. It opens in a framed preview they can switch between desktop, tablet and phone. Calling again with the same id updates the design in place. Pages can answer back: dove.send(text) or an element with data-dove-send=\"…\" sends the user's choice as their next message.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Stable id; reuse it to iterate on the same design."},
                "title": {"type": "string"},
                "description": {"type": "string", "description": "One line on what this design is."},
                "kind": {"type": "string", "enum": ["html", "react", "svg"], "description": "html: a full document or a body fragment (Tailwind loaded for fragments). react: JSX/TSX that defines a component named App (React hooks and Tailwind available, no imports needed). svg: an SVG document."},
                "device": {"type": "string", "enum": ["desktop", "tablet", "mobile"], "description": "The frame it opens in."},
                "code": {"type": "string", "description": "The source."}
            },
            "required": ["title", "code"]
        }
    })
}

/// `show_ui`: rendered by Dove's chat straight from the tool call.
fn show_ui_tool() -> Value {
    json!({
        "name": "show_ui",
        "description": "Show the user an interactive card inline in the chat. Use it for dashboards, comparisons, charts, plans to tick off, settings to tune, or choices to make. If you add actions, the user's pick plus all control values arrive as their next message.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Optional stable id. Calling show_ui again with the same id updates that card in place (live dashboards, progress)."},
                "title": {"type": "string"},
                "body": {"type": "string", "description": "Short intro text."},
                "blocks": {
                    "type": "array",
                    "description": "Content, top to bottom. Each item has a `type`: text {text}; callout {tone: info|success|warning|danger, title?, text}; stats {items: [{label, value, delta?}]}; bars {title?, items: [{label, value}], unit?, horizontal?}; line {title?, values: [number], labels?: [string], unit?}; table {columns: [string], rows: [[any]]}; progress {label, value 0-1}; kv {items: [[key, value]]}; code {lang?, text}; image {src, alt?}; divider {}; timeline {title?, steps: [{title, detail?, status: done|active|todo|failed}]}; breakdown {title?, items: [{label, value}], unit?} (100% stacked bar); tabs {id, tabs: [{label, blocks: [...]}]}. Controls the user can change: cards {id, label?, items: [{id, title, body?, badge?}]} (pick one option card); choice {id, label, choices: [string], value?}; toggle {id, label, value: bool}; number {id, label, value, min?, max?, step?}; checklist {id, label?, items: [{label, checked?}]}; slider {id, label, value, min?, max?, step?, unit?}; rating {id, label, value?, max?}; replies {label?, items: [{id?, label}]} (buttons that answer at once). Design & app blocks: heading {text}; hero {badge?, title, subtitle?, cta?, secondary?}; navbar {id?, brand?, items: [string], cta?}; pricing {id?, plans: [{id?, name, price, badge?, features: [string]}]}; split {left: [...], right: [...]}; donut {title?, items: [{label, value}], unit?}; stats items may add spark: [number] for a trend line; swatches {title?, items: [{name, hex}]}; gallery {items: [{src, caption?}]}; board {columns: [{title, items: [string]}]}; link {label, url}; mockup {device: phone|tablet|browser|app, title?, nav?: [string], blocks: [...]} (an app screen or web page drawn natively from other blocks). For a full page or a React/TSX component use show_canvas instead.",
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
    let config = crate::pages::integrations::load();
    let mut upstream = upstream::Hub::new(&config);
    let mut instructions = format!("{BASE_INSTRUCTIONS}{CHROME_INSTRUCTIONS}");
    for (on, text) in [
        (upstream.has("composio"), COMPOSIO_INSTRUCTIONS),
        (upstream.has("paper"), PAPER_INSTRUCTIONS),
        (upstream.has("computer"), COMPUTER_INSTRUCTIONS),
    ] {
        if on {
            instructions.push_str(text);
        }
    }
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            let _ = writeln!(
                out,
                "{}",
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}})
            );
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
                "instructions": instructions,
            })),
            "ping" => Ok(json!({})),
            "tools/list" => {
                let mut tools = crate::browser::agent::tools();
                if let Some(list) = tools.as_array_mut() {
                    list.push(show_ui_tool());
                    list.push(show_canvas_tool());
                    list.extend(chrome_tools());
                    list.extend(upstream.tools());
                    list.push(skills_tool());
                }
                Ok(json!({"tools": tools}))
            }
            "tools/call" if params.get("name").and_then(Value::as_str) == Some("show_canvas") => Ok(json!({
                "content": [{"type": "text", "text": "The design is open in Dove's canvas. The user can switch frames, view the code and save it. Anything the page sends with dove.send arrives as their next message."}],
                "isError": false
            })),
            "tools/call" if params.get("name").and_then(Value::as_str) == Some("show_ui") => Ok(json!({
                "content": [{"type": "text", "text": "Shown to the user inline in the chat. If it has actions, their choice arrives as their next message."}],
                "isError": false
            })),
            "tools/call" if params.get("name").and_then(Value::as_str).is_some_and(|n| upstream.owns(n)) => {
                Ok(upstream.call(
                    params.get("name").and_then(Value::as_str).unwrap_or_default(),
                    params.get("arguments").cloned().unwrap_or(json!({})),
                ))
            }
            "tools/call" if params.get("name").and_then(Value::as_str) == Some("dove_skills") => {
                let name = params.pointer("/arguments/name").and_then(Value::as_str);
                Ok(match dove_core::office_skills::describe(name) {
                    Ok(text) => json!({"content": [{"type": "text", "text": text}], "isError": false}),
                    Err(text) => json!({"content": [{"type": "text", "text": text}], "isError": true}),
                })
            }
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
            let mut content =
                vec![json!({"type": "text", "text": v.get("text").and_then(Value::as_str).unwrap_or("")})];
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

/// `chrome_*`: the user's own browser, through the Dove for Chrome extension.
fn chrome_tools() -> Vec<Value> {
    let tab = json!({"type": "integer", "description": "Tab id from chrome_tabs. Defaults to the tab Dove last used, else the active tab."});
    let reference = json!({"type": "string", "description": "Element ref from chrome_snapshot (e12), or a CSS selector, or visible text."});
    let tool = |name: &str, description: &str, mut props: Value, required: &[&str]| {
        if let Some(p) = props.as_object_mut() {
            p.entry("tab").or_insert(tab.clone());
        }
        json!({"name": name, "description": description, "inputSchema": {"type": "object", "properties": props, "required": required}})
    };
    vec![
        tool("chrome_tabs", "List the user's open Chrome tabs (id, title, URL).", json!({}), &[]),
        tool(
            "chrome_tab_open",
            "Open a new tab in the user's Chrome, optionally at a URL, and wait for it to load.",
            json!({"url": {"type": "string"}, "background": {"type": "boolean"}}),
            &[],
        ),
        tool("chrome_tab_close", "Close a tab.", json!({}), &[]),
        tool("chrome_tab_activate", "Bring a tab to the front.", json!({}), &["tab"]),
        tool(
            "chrome_navigate",
            "Load a URL in the tab and wait for it to finish loading.",
            json!({"url": {"type": "string"}, "timeout_ms": {"type": "integer"}}),
            &["url"],
        ),
        tool(
            "chrome_snapshot",
            "Accessibility-style outline of the page; actionable elements carry [ref=eN] for the other chrome_* tools. Pass ref to outline one region, full for long pages.",
            json!({"ref": {"type": "string"}, "full": {"type": "boolean"}}),
            &[],
        ),
        tool(
            "chrome_click",
            "Click an element (real mouse events via the debugger) and wait for any navigation.",
            json!({"ref": reference.clone(), "double": {"type": "boolean"}}),
            &["ref"],
        ),
        tool("chrome_hover", "Move the mouse over an element.", json!({"ref": reference.clone()}), &["ref"]),
        tool(
            "chrome_type",
            "Focus a field and type text (replaces its value unless clear is false). submit presses Enter.",
            json!({"ref": reference.clone(), "text": {"type": "string"}, "clear": {"type": "boolean"}, "submit": {"type": "boolean"}}),
            &["ref", "text"],
        ),
        tool(
            "chrome_press",
            "Press a key or chord (Enter, Escape, Tab, ArrowDown, Meta+A, Control+Shift+K), optionally after focusing ref.",
            json!({"key": {"type": "string"}, "ref": reference.clone()}),
            &["key"],
        ),
        tool(
            "chrome_scroll",
            "Scroll the page (direction up/down/left/right, amount in px) or scroll ref into view.",
            json!({"direction": {"type": "string", "enum": ["up", "down", "left", "right"]}, "amount": {"type": "integer"}, "ref": reference.clone()}),
            &[],
        ),
        tool(
            "chrome_select",
            "Choose an option in a <select> by value or visible label.",
            json!({"ref": reference, "value": {"type": "string"}}),
            &["ref", "value"],
        ),
        tool("chrome_screenshot", "PNG of the tab's visible area (brings the tab to the front).", json!({}), &[]),
        tool(
            "chrome_read",
            "The page's readable text, for summarizing or extracting data.",
            json!({"full": {"type": "boolean"}}),
            &[],
        ),
        tool(
            "chrome_wait",
            "Wait for the page to finish loading, for text or a CSS selector to appear (timeout_ms), or for ms milliseconds.",
            json!({"text": {"type": "string"}, "selector": {"type": "string"}, "timeout_ms": {"type": "integer"}, "ms": {"type": "integer"}}),
            &[],
        ),
        tool(
            "chrome_release",
            "Hand the browser back to the user: detach the debugger and remove the control indicator. Call when done.",
            json!({}),
            &[],
        ),
    ]
}

/// The driver's own health report, condensed for Settings.
pub fn cua_health(bin: &std::path::Path) -> String {
    let bin = bin.to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = (|| {
            let mut server = upstream::Server::stdio("computer", &bin.display().to_string(), &["mcp".into()])?;
            server.initialize()?;
            server.request("tools/call", json!({"name": "health_report", "arguments": {}}))
        })();
        let _ = tx.send(result);
    });
    let result = match rx.recv_timeout(Duration::from_secs(20)) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return format!("Driver check failed: {e}"),
        Err(_) => return "The driver did not answer within 20 s".into(),
    };
    let report = result.get("structuredContent").cloned().or_else(|| {
        let text = result.pointer("/content/0/text").and_then(Value::as_str)?;
        serde_json::from_str::<Value>(text).ok().or_else(|| Some(Value::String(text.to_string())))
    });
    summarize_health(report.unwrap_or(Value::Null))
}

fn summarize_health(report: Value) -> String {
    if let Some(text) = report.as_str() {
        return text.lines().take(2).collect::<Vec<_>>().join(" · ").chars().take(160).collect();
    }
    let mut parts = Vec::new();
    let mut walk = vec![("", &report)];
    while let Some((key, v)) = walk.pop() {
        match v {
            Value::Object(map) => walk.extend(map.iter().map(|(k, v)| (k.as_str(), v))),
            Value::Bool(b)
                if key.to_ascii_lowercase().contains("accessibility")
                    || key.to_ascii_lowercase().contains("screen") =>
            {
                parts.push(format!("{}: {}", key.replace('_', " "), if *b { "granted" } else { "missing" }));
            }
            Value::String(s) if matches!(key, "verdict" | "status" | "overall") => parts.insert(0, s.clone()),
            _ => {}
        }
    }
    if parts.is_empty() { "Driver answered; no permission fields reported".into() } else { parts.join(" · ") }
}

mod upstream {
    //! Minimal MCP clients for the upstream servers: stdio (cua-driver) and
    //! streamable HTTP (Composio, Paper Desktop).

    use std::io::{BufRead as _, BufReader, Write as _};
    use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
    use std::sync::OnceLock;

    use serde_json::{Value, json};

    use crate::pages::integrations::{self as cfg, ComposioKey, Config};

    const PROTOCOL: &str = "2025-06-18";

    fn runtime() -> &'static tokio::runtime::Runtime {
        static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        RT.get_or_init(|| tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime"))
    }

    enum Transport {
        Stdio { _child: Child, stdin: ChildStdin, stdout: BufReader<ChildStdout> },
        Http { client: reqwest::Client, url: String, headers: Vec<(String, String)>, session: Option<String> },
    }

    pub struct Server {
        pub name: &'static str,
        transport: Transport,
        next: u64,
    }

    impl Server {
        pub fn stdio(name: &'static str, command: &str, args: &[String]) -> Result<Self, String> {
            let mut child = Command::new(command)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("could not start {command}: {e}"))?;
            let stdin = child.stdin.take().ok_or("no stdin")?;
            let stdout = BufReader::new(child.stdout.take().ok_or("no stdout")?);
            Ok(Self { name, transport: Transport::Stdio { _child: child, stdin, stdout }, next: 1 })
        }

        pub fn http(name: &'static str, url: String, headers: Vec<(String, String)>) -> Result<Self, String> {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| e.to_string())?;
            Ok(Self { name, transport: Transport::Http { client, url, headers, session: None }, next: 1 })
        }

        pub fn initialize(&mut self) -> Result<Value, String> {
            let result = self.request(
                "initialize",
                json!({"protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": {"name": "dove", "version": env!("CARGO_PKG_VERSION")}}),
            )?;
            self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}), None)?;
            Ok(result)
        }

        pub fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
            let id = self.next;
            self.next += 1;
            let reply = self
                .send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}), Some(id))?
                .ok_or_else(|| format!("{} sent no reply to {method}", self.name))?;
            if let Some(err) = reply.get("error") {
                return Err(err.get("message").and_then(Value::as_str).unwrap_or("error").to_string());
            }
            Ok(reply.get("result").cloned().unwrap_or(Value::Null))
        }

        fn send(&mut self, msg: Value, want: Option<u64>) -> Result<Option<Value>, String> {
            match &mut self.transport {
                Transport::Stdio { stdin, stdout, .. } => {
                    writeln!(stdin, "{msg}").and_then(|_| stdin.flush()).map_err(|e| e.to_string())?;
                    let Some(want) = want else { return Ok(None) };
                    let mut line = String::new();
                    loop {
                        line.clear();
                        if stdout.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                            return Err("the server exited".into());
                        }
                        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                        if v.get("method").is_some() {
                            // A server-to-client request: answer the harmless ones.
                            if let Some(id) = v.get("id") {
                                let result = match v.get("method").and_then(Value::as_str) {
                                    Some("ping") => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
                                    Some("roots/list") => json!({"jsonrpc": "2.0", "id": id, "result": {"roots": []}}),
                                    _ => {
                                        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "not supported"}})
                                    }
                                };
                                let _ = writeln!(stdin, "{result}");
                                let _ = stdin.flush();
                            }
                            continue;
                        }
                        if v.get("id").and_then(Value::as_u64) == Some(want) {
                            return Ok(Some(v));
                        }
                    }
                }
                Transport::Http { client, url, headers, session } => runtime().block_on(async {
                    let mut req = client
                        .post(url.as_str())
                        .header("content-type", "application/json")
                        .header("accept", "application/json, text/event-stream")
                        .header("mcp-protocol-version", PROTOCOL)
                        .body(msg.to_string());
                    for (k, v) in headers.iter() {
                        req = req.header(k.as_str(), v.as_str());
                    }
                    if let Some(s) = session.as_deref() {
                        req = req.header("mcp-session-id", s);
                    }
                    let resp = req.send().await.map_err(|e| format!("could not reach {url}: {e}"))?;
                    if let Some(s) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
                        *session = Some(s.to_string());
                    }
                    let status = resp.status();
                    let sse = resp
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|c| c.starts_with("text/event-stream"));
                    let body = resp.text().await.map_err(|e| e.to_string())?;
                    if !status.is_success() {
                        return Err(format!("{url} answered {status}: {}", body.chars().take(200).collect::<String>()));
                    }
                    let Some(want) = want else { return Ok(None) };
                    let messages: Vec<Value> = if sse {
                        body.split("\n\n")
                            .filter_map(|event| {
                                let data: Vec<&str> = event
                                    .lines()
                                    .filter_map(|l| l.strip_prefix("data:"))
                                    .map(str::trim_start)
                                    .collect();
                                serde_json::from_str(&data.join("\n")).ok()
                            })
                            .collect()
                    } else {
                        serde_json::from_str::<Value>(&body).map(|v| vec![v]).unwrap_or_default()
                    };
                    Ok(messages
                        .into_iter()
                        .flat_map(|m| if let Value::Array(a) = m { a } else { vec![m] })
                        .find(|m| m.get("id").and_then(Value::as_u64) == Some(want)))
                }),
            }
        }
    }

    enum Spec {
        Ready(Box<dyn FnOnce() -> Result<Server, String>>),
    }

    pub struct Hub {
        specs: Vec<(&'static str, Option<Spec>)>,
        live: Vec<(Server, Vec<Value>)>,
        started: bool,
    }

    impl Hub {
        pub fn new(config: &Config) -> Self {
            let mut specs: Vec<(&'static str, Option<Spec>)> = Vec::new();
            let c = config.composio.clone();
            if c.enabled && !c.api_key.is_empty() {
                specs.push(("composio", Some(Spec::Ready(Box::new(move || composio(&c))))));
            }
            if config.paper.enabled {
                specs.push((
                    "paper",
                    Some(Spec::Ready(Box::new(|| Server::http("paper", cfg::PAPER_MCP_URL.into(), Vec::new())))),
                ));
            }
            if config.computer.enabled
                && let Some(bin) = cfg::cua_driver()
            {
                specs.push((
                    "computer",
                    Some(Spec::Ready(Box::new(move || {
                        Server::stdio("computer", &bin.display().to_string(), &["mcp".into()])
                    }))),
                ));
            }
            Self { specs, live: Vec::new(), started: false }
        }

        pub fn has(&self, name: &str) -> bool {
            self.specs.iter().any(|(n, _)| *n == name)
        }

        pub fn owns(&self, tool: &str) -> bool {
            tool.split_once("__").is_some_and(|(prefix, _)| self.has(prefix))
        }

        fn start(&mut self) {
            if std::mem::replace(&mut self.started, true) {
                return;
            }
            for (name, spec) in &mut self.specs {
                let Some(Spec::Ready(make)) = spec.take() else { continue };
                let result = make().and_then(|mut server| {
                    server.initialize()?;
                    let mut tools = Vec::new();
                    let mut cursor: Option<Value> = None;
                    loop {
                        let page = server
                            .request("tools/list", cursor.as_ref().map_or(json!({}), |c| json!({"cursor": c})))?;
                        tools.extend(page.get("tools").and_then(Value::as_array).cloned().unwrap_or_default());
                        cursor = page.get("nextCursor").filter(|c| !c.is_null()).cloned();
                        if cursor.is_none() || tools.len() > 2000 {
                            break;
                        }
                    }
                    Ok((server, tools))
                });
                match result {
                    Ok(live) => self.live.push(live),
                    Err(e) => eprintln!("dove: {name} integration unavailable: {e}"),
                }
            }
        }

        pub fn tools(&mut self) -> Vec<Value> {
            self.start();
            self.live
                .iter()
                .flat_map(|(server, tools)| {
                    tools.iter().filter_map(move |t| {
                        let mut t = t.clone();
                        let name = t.get("name")?.as_str()?.to_string();
                        t["name"] = json!(format!("{}__{name}", server.name));
                        Some(t)
                    })
                })
                .collect()
        }

        pub fn call(&mut self, tool: &str, args: Value) -> Value {
            self.start();
            let Some((prefix, name)) = tool.split_once("__") else { return error("Unknown tool") };
            let Some((server, _)) = self.live.iter_mut().find(|(s, _)| s.name == prefix) else {
                return error(&format!(
                    "The {prefix} integration is not connected. Check Dove → Settings → Integrations."
                ));
            };
            server.request("tools/call", json!({"name": name, "arguments": args})).unwrap_or_else(|e| error(&e))
        }
    }

    fn error(text: &str) -> Value {
        json!({"content": [{"type": "text", "text": text}], "isError": true})
    }

    /// Consumer keys use Composio Connect directly; project and user keys
    /// open a v3.1 tool-router session (toolkit-scoped when the user picked
    /// any) and use the MCP URL and headers it returns.
    fn composio(c: &cfg::Composio) -> Result<Server, String> {
        let kind = ComposioKey::of(&c.api_key);
        let auth = vec![(kind.header().to_string(), c.api_key.clone())];
        if kind == ComposioKey::Consumer {
            return Server::http("composio", cfg::COMPOSIO_CONNECT_URL.into(), auth);
        }
        let mut body = json!({"user_id": "dove"});
        if !c.toolkits.is_empty() {
            body["toolkits"] = json!({"enable": c.toolkits});
        }
        let session: Value = runtime().block_on(async {
            let mut req =
                reqwest::Client::new().post(cfg::COMPOSIO_SESSION_URL).header("content-type", "application/json");
            for (k, v) in &auth {
                req = req.header(k.as_str(), v.as_str());
            }
            let resp = req.body(body.to_string()).send().await.map_err(|e| format!("Composio: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.map_err(|e| e.to_string())?;
            if !status.is_success() {
                return Err(format!("Composio session: {status}"));
            }
            serde_json::from_str(&text).map_err(|e| e.to_string())
        })?;
        let url = session
            .pointer("/mcp/url")
            .or_else(|| session.get("mcp_url"))
            .or_else(|| session.get("url"))
            .and_then(Value::as_str)
            .ok_or("Composio session has no MCP URL")?
            .to_string();
        // Never send the key to a host other than Composio's.
        let host = reqwest::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        if !(host == "composio.dev" || host.ends_with(".composio.dev")) {
            return Err(format!("Composio returned an MCP URL on {host}; refusing to send the key there"));
        }
        let headers = session
            .pointer("/mcp/headers")
            .and_then(Value::as_object)
            .map(|h| h.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect::<Vec<_>>())
            .filter(|h| !h.is_empty())
            .unwrap_or(auth);
        Server::http("composio", url, headers)
    }
}
