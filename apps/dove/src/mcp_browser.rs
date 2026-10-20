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
snapshot; browser_screenshot shows you the pixels. Dove's chat also renders ```mermaid diagrams and $$ LaTeX $$ math.\n\
Visuals: call show_visual with a Luau script that returns a ui.* tree. ui.shader runs Shadertoy-style GLSL \
(void mainImage) on the GPU, including buffer passes. The visual appears inline in the chat; don't restate it.\n\
Office documents: for .pdf, .pptx, .xlsx or .docx work, call dove_skills with {\"name\": \"pdf\"|\"pptx\"|\"xlsx\"|\"docx\"} \
first. Each skill ships a ready script (python-docx, python-pptx, openpyxl, reportlab, pypdf) you run with \
`uv run <script>`; it gives real layouts, styles, formulas and charts. The user previews and edits the result \
in Dove's file panel.
Web tasks: before browsing, call dove_skills with {\"name\": \"dove-browser\"} (or read the dove-browser skill) and follow it; it prefers ego lite, the user's own browser, when installed.";

/// `dove_skills`: Dove's bundled office skills (`skills/` in the repo).
fn skills_tool() -> Value {
    json!({
        "name": "dove_skills",
        "description": "Dove's skills: pdf (create, merge, split, fill forms, extract), pptx (decks from outlines on real layouts, edit slides), xlsx (workbooks with formulas, formatting, charts), docx (Word documents with styles, tables, images), dove-browser (the playbook for any web task: ego lite or Dove's browser), dove-memory (long-term memory across sessions). Without a name it lists them; with a name it returns that skill's full instructions and its folder.",
        "inputSchema": {
            "type": "object",
            "properties": {"name": {"type": "string", "enum": ["pdf", "pptx", "xlsx", "docx", "dove-browser", "dove-memory"]}}
        }
    })
}

const EGO_INSTRUCTIONS: &str = "
Web tasks: browse with ego lite (the `ego-browser` command, the user's own Chromium). Before the first browser step call dove_skills with {\"name\": \"dove-browser\"} and follow it.";

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

fn show_visual_tool() -> Value {
    json!({
        "name": "show_visual",
        "description": SHOW_VISUAL,
        "inputSchema": {
            "type": "object",
            "properties": {
                "script": {"type": "string", "description": "Luau script: setup at the top level, ending with `return function() return ui.column { ... } end`."},
                "title": {"type": "string", "description": "Short name for the visual."},
                "assets": {
                    "type": "object",
                    "additionalProperties": {"type": "string"},
                    "description": "Files the script uses, by the name it uses them. Paths are on this machine, relative to the working directory."
                }
            },
            "required": ["script", "title"]
        }
    })
}

const SHOW_VISUAL: &str = "\
Show an interactive visual (chart, table, dashboard, diagram, simulation, 3D view, GPU shader) inline in this \
chat, above your final text reply; call it before writing that reply. Use it when a visual says more \
than prose. The reader already sees it, so the reply should not announce, locate, or restate it.\n\
\n\
A visual is a Luau script (Lua 5.1 plus `+=`, `continue`, and `if` expressions). Dove runs it in a \
sandbox (no I/O or network; math.random is seeded) and renders what it returns natively. Build data and \
state at the top level, then return a view function. Dove calls the view again after every handler:\n\
\n\
local s = ui.state { metric = \"p95\" }\n\
return function()\n\
  return ui.column { gap = 16,\n\
    ui.text { \"Latency\", size = \"lg\", weight = 600 },\n\
    ui.chart { type = \"bar\", height = 220, unit = \"ms\", x = { \"us\", \"eu\" }, series = { { name = s.metric, values = { 182, 140 } } } },\n\
  }\n\
end\n\
\n\
Nodes: ui.column, ui.row, ui.grid, ui.text, ui.stat, ui.table, ui.chart (bar|line|area|scatter), ui.rule, \
ui.space, ui.link, ui.tabs, ui.toggle, ui.slider, ui.button, ui.canvas (2D draw), ui.scene (3D, GPU), \
ui.shader. Colors are theme tokens (text, muted, faint, accent, success, warning, danger, border, chart1-6), \
not hex, except scene materials.\n\
\n\
ui.shader { height = 360, code = [[ GLSL ]], buffers = { [[ GLSL ]], ... }, inputs = { speed = s.speed }, \
blend = \"alpha|light|opaque\", bloom = 0-3, orbit = bool, animate = true }: each pass is Shadertoy-style \
void mainImage(out vec4 fragColor, in vec2 fragCoord), with iResolution, iTime, iTimeDelta, iFrame, iMouse, \
iCamera, iNoise, iChannel0-iChannel3, and iTexture0-iTexture3. Buffers run first. blend = \"light\" treats \
brightness as opacity (black is clear), which is what beams and a v-prism need. Don't paint a background. \
Scale by min(iResolution.x, iResolution.y). GLSL errors come back with the pass and line number.\n\
\n\
Assets: pass files in assets (name -> path). s:model { asset = \"prism.glb\" }, textures, and environment maps \
are named the same way. Dove checks the script before showing it and returns the first error with its line.";

/// With the agent browser off, register Dove's server for the integrations
/// alone, once any is on. Called at startup and whenever Settings saves.
pub fn register_without_browser(data: &std::path::Path) {
    if dove_core::tool_server(data).is_some() || !crate::pages::integrations::load().any_enabled() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    dove_core::set_tool_server(Some(dove_core::ToolServer {
        name: "dove".into(),
        command: exe.display().to_string(),
        args: vec!["--mcp-browser".into()],
        env: vec![("DOVE_DATA".into(), data.display().to_string())],
    }));
}

/// Start one server definition, list its tools and stop it again: the
/// "Test" button in Settings. Returns the tool count.
pub fn test_server(id: &str, transport: &crate::pages::integrations::Transport) -> Result<usize, String> {
    let make = upstream::make(id, transport);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = make().and_then(|mut server| {
            server.initialize()?;
            server.list_tools().map(|t| t.len())
        });
        let _ = tx.send(result);
    });
    rx.recv_timeout(Duration::from_secs(60)).unwrap_or_else(|_| Err("no answer within 60 s".into()))
}

/// What a connected server's tools are for, for the agent.
fn server_instructions(config: &crate::pages::integrations::Config, upstream: &upstream::Hub) -> String {
    let mut text = String::new();
    for s in config.servers.iter().filter(|s| s.enabled && upstream.has(&s.id)) {
        let hint = s.catalog.as_deref().and_then(crate::integrations_catalog::get).map(|e| e.hint).unwrap_or("");
        if hint.is_empty() {
            text.push_str(&format!("\n{}: {}__* tools come from the user's {} MCP server.", s.name, s.id, s.name));
        } else {
            text.push_str(&format!("\n{}: {hint}", s.name));
        }
    }
    text
}

pub fn run() {
    let config = crate::pages::integrations::load();
    let mut upstream = upstream::Hub::new(&config);
    // Without the app's browser endpoint (agent browser off), Dove's server
    // still carries the integrations, just not the browser tools.
    let browser = std::env::var_os("DOVE_BROWSER_PORT").is_some();
    // "Use ego lite only": agents browse with the ego-browser command.
    let ego_only = config.ego.enabled && crate::pages::integrations::ego_browser().is_some();
    let mut instructions = if browser {
        format!("{BASE_INSTRUCTIONS}{CHROME_INSTRUCTIONS}")
    } else {
        "Tools from Dove, the app the user is chatting with you in.".to_string()
    };
    if !browser || ego_only {
        instructions.push_str(EGO_INSTRUCTIONS);
    }
    for (on, text) in [
        (upstream.has("composio"), COMPOSIO_INSTRUCTIONS),
        (upstream.has("paper"), PAPER_INSTRUCTIONS),
        (upstream.has("computer"), COMPUTER_INSTRUCTIONS),
    ] {
        if on {
            instructions.push_str(text);
        }
    }
    instructions.push_str(&server_instructions(&config, &upstream));
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
                let mut tools = if browser && !ego_only { crate::browser::agent::tools() } else { json!([]) };
                if let Some(list) = tools.as_array_mut() {
                    list.push(show_visual_tool());
                    if browser {
                        list.extend(chrome_tools());
                    }
                    list.extend(upstream.tools());
                    list.push(skills_tool());
                }
                Ok(json!({"tools": tools}))
            }
            "tools/call" if params.get("name").and_then(Value::as_str) == Some("show_visual") => {
                let script = params.pointer("/arguments/script").and_then(Value::as_str).unwrap_or("");
                let title = params.pointer("/arguments/title").and_then(Value::as_str).unwrap_or("Visual");
                match zeron_visual::check(script, zeron_visual::seed_for(title)) {
                    Ok(_) => Ok(json!({
                        "content": [{"type": "text", "text": "Shown inline in the chat. Don't mention or describe the visual; reply with only what it doesn't already say."}],
                        "isError": false
                    })),
                    Err(err) => Ok(json!({
                        "content": [{"type": "text", "text": err.to_string()}],
                        "isError": true
                    })),
                }
            }
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
            let mut server = upstream::Server::stdio(
                "computer",
                &bin.display().to_string(),
                &["mcp".into()],
                &Default::default(),
                None,
            )?;
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
    //! Minimal MCP clients for the upstream servers: stdio (cua-driver,
    //! local catalog and custom servers) and streamable HTTP (Composio,
    //! Paper Desktop, remote catalog and custom servers).

    use std::collections::{BTreeMap, HashMap};
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
    use std::sync::OnceLock;
    use std::time::Duration;

    use serde_json::{Value, json};

    use crate::pages::integrations::{self as cfg, ComposioKey, Config, Transport};

    const PROTOCOL: &str = "2025-06-18";
    /// Claude Code and Codex cap a tool's full name (`mcp__dove__<name>`) at
    /// 64 characters; `mcp__dove__` takes 11.
    const MAX_TOOL_NAME: usize = 53;
    /// How long one upstream may take to start and list its tools.
    const START_TIMEOUT: Duration = Duration::from_secs(45);

    fn runtime() -> &'static tokio::runtime::Runtime {
        static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        RT.get_or_init(|| tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime"))
    }

    enum Link {
        Stdio {
            child: Child,
            stdin: ChildStdin,
            stdout: BufReader<ChildStdout>,
        },
        Http {
            client: reqwest::Client,
            url: String,
            headers: Vec<(String, String)>,
            session: Option<String>,
            /// The `mcp_oauth` key whose token goes in `Authorization`.
            oauth: Option<String>,
        },
    }

    pub struct Server {
        pub name: String,
        transport: Link,
        next: u64,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            if let Link::Stdio { child, .. } = &mut self.transport {
                let _ = child.kill();
            }
        }
    }

    impl Server {
        pub fn stdio(
            name: &str,
            command: &str,
            args: &[String],
            env: &BTreeMap<String, String>,
            cwd: Option<&str>,
        ) -> Result<Self, String> {
            // npm installs `npx` and friends as `.cmd` shims on Windows,
            // which only cmd.exe can start.
            let program = dove_core::harness_process::resolve(command).unwrap_or_else(|| command.into());
            let ext = program.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            let mut cmd = if cfg!(windows) && (ext == "cmd" || ext == "bat") {
                let mut c = Command::new("cmd.exe");
                c.args(["/d", "/s", "/c"]).arg(&program).args(args);
                c
            } else {
                let mut c = Command::new(&program);
                c.args(args);
                c
            };
            cmd.env("PATH", dove_core::harness_process::search_path())
                .envs(env)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            if let Some(dir) = cwd.filter(|d| !d.is_empty()) {
                cmd.current_dir(dir);
            } else if let Ok(dir) = std::env::var("DOVE_CWD") {
                cmd.current_dir(dir);
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt as _;
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }
            let mut child = cmd.spawn().map_err(|e| format!("could not start {command}: {e}"))?;
            let stdin = child.stdin.take().ok_or("no stdin")?;
            let stdout = BufReader::new(child.stdout.take().ok_or("no stdout")?);
            Ok(Self { name: name.to_string(), transport: Link::Stdio { child, stdin, stdout }, next: 1 })
        }

        pub fn http(
            name: &str,
            url: String,
            headers: Vec<(String, String)>,
            oauth: Option<String>,
        ) -> Result<Self, String> {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| e.to_string())?;
            Ok(Self {
                name: name.to_string(),
                transport: Link::Http { client, url, headers, session: None, oauth },
                next: 1,
            })
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

        /// Every tool, following `nextCursor`.
        pub fn list_tools(&mut self) -> Result<Vec<Value>, String> {
            let mut tools = Vec::new();
            let mut cursor: Option<Value> = None;
            loop {
                let page = self.request("tools/list", cursor.as_ref().map_or(json!({}), |c| json!({"cursor": c})))?;
                tools.extend(page.get("tools").and_then(Value::as_array).cloned().unwrap_or_default());
                cursor = page.get("nextCursor").filter(|c| !c.is_null()).cloned();
                if cursor.is_none() || tools.len() > 2000 {
                    return Ok(tools);
                }
            }
        }

        fn send(&mut self, msg: Value, want: Option<u64>) -> Result<Option<Value>, String> {
            let name = self.name.clone();
            match &mut self.transport {
                Link::Stdio { stdin, stdout, .. } => {
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
                Link::Http { client, url, headers, session, oauth } => {
                    let mut bearer = match oauth.as_deref() {
                        Some(id) => Some(crate::mcp_oauth::token(id).ok_or_else(|| {
                            format!("Not signed in to {name}. Sign in from Dove → Settings → Integrations.")
                        })?),
                        None => None,
                    };
                    for attempt in 0..2 {
                        let reply = runtime().block_on(post(client, url, headers, session, bearer.as_deref(), &msg, want))?;
                        match reply {
                            Reply::Done(v) => return Ok(v),
                            Reply::Unauthorized if attempt == 0 && oauth.is_some() => {
                                let id = oauth.as_deref().unwrap_or_default();
                                bearer = Some(crate::mcp_oauth::refresh(id).map_err(|_| {
                                    format!("{name} signed Dove out. Sign in again from Dove → Settings → Integrations.")
                                })?);
                            }
                            Reply::Unauthorized => {
                                return Err(format!(
                                    "{name} refused the credentials. Check them in Dove → Settings → Integrations."
                                ));
                            }
                        }
                    }
                    Err(format!("{name} refused the credentials"))
                }
            }
        }
    }

    enum Reply {
        Done(Option<Value>),
        Unauthorized,
    }

    async fn post(
        client: &reqwest::Client,
        url: &str,
        headers: &[(String, String)],
        session: &mut Option<String>,
        bearer: Option<&str>,
        msg: &Value,
        want: Option<u64>,
    ) -> Result<Reply, String> {
        let mut req = client
            .post(url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", PROTOCOL)
            .body(msg.to_string());
        for (k, v) in headers.iter() {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(token) = bearer {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        if let Some(s) = session.as_deref() {
            req = req.header("mcp-session-id", s);
        }
        let resp = req.send().await.map_err(|e| format!("could not reach {url}: {e}"))?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Ok(Reply::Unauthorized);
        }
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
        let Some(want) = want else { return Ok(Reply::Done(None)) };
        let messages: Vec<Value> = if sse {
            body.split("\n\n")
                .filter_map(|event| {
                    let data: Vec<&str> =
                        event.lines().filter_map(|l| l.strip_prefix("data:")).map(str::trim_start).collect();
                    serde_json::from_str(&data.join("\n")).ok()
                })
                .collect()
        } else {
            serde_json::from_str::<Value>(&body).map(|v| vec![v]).unwrap_or_default()
        };
        Ok(Reply::Done(
            messages
                .into_iter()
                .flat_map(|m| if let Value::Array(a) = m { a } else { vec![m] })
                .find(|m| m.get("id").and_then(Value::as_u64) == Some(want)),
        ))
    }

    type Make = Box<dyn FnOnce() -> Result<Server, String> + Send>;

    /// The upstream's tools as this server exposes them: `<prefix>__<tool>`,
    /// shortened (with a hash) when that would pass `MAX_TOOL_NAME`.
    struct Live {
        server: Server,
        tools: Vec<Value>,
        /// Exposed name → upstream name, for the shortened ones.
        renamed: HashMap<String, String>,
    }

    pub struct Hub {
        specs: Vec<(String, Option<Make>)>,
        live: Vec<Live>,
        started: bool,
    }

    /// A server definition the user entered, as a constructor.
    pub fn make(name: &str, transport: &Transport) -> Make {
        let name = name.to_string();
        match transport.clone() {
            Transport::Stdio { command, args, env, cwd } => {
                Box::new(move || Server::stdio(&name, &command, &args, &env, cwd.as_deref()))
            }
            Transport::Http { url, headers, oauth } => {
                let oauth = oauth.then(|| name.clone());
                Box::new(move || Server::http(&name, url, headers.into_iter().collect(), oauth))
            }
        }
    }

    impl Hub {
        pub fn new(config: &Config) -> Self {
            let mut specs: Vec<(String, Option<Make>)> = Vec::new();
            let c = config.composio.clone();
            if c.enabled && (c.account || !c.api_key.is_empty()) {
                specs.push(("composio".into(), Some(Box::new(move || composio(&c)))));
            }
            if config.paper.enabled {
                specs.push((
                    "paper".into(),
                    Some(Box::new(|| Server::http("paper", cfg::PAPER_MCP_URL.into(), Vec::new(), None))),
                ));
            }
            if config.computer.enabled
                && let Some(bin) = cfg::cua_driver()
            {
                specs.push((
                    "computer".into(),
                    Some(Box::new(move || {
                        Server::stdio("computer", &bin.display().to_string(), &["mcp".into()], &BTreeMap::new(), None)
                    })),
                ));
            }
            for s in config.servers.iter().filter(|s| s.enabled) {
                if specs.iter().any(|(n, _)| *n == s.id) {
                    continue;
                }
                specs.push((s.id.clone(), Some(make(&s.id, &s.transport))));
            }
            Self { specs, live: Vec::new(), started: false }
        }

        pub fn has(&self, name: &str) -> bool {
            self.specs.iter().any(|(n, _)| n == name)
        }

        pub fn owns(&self, tool: &str) -> bool {
            tool.split_once("__").is_some_and(|(prefix, _)| self.has(prefix))
        }

        /// Start every upstream at once, so one slow or dead server costs
        /// at most `START_TIMEOUT` instead of adding up.
        fn start(&mut self) {
            if std::mem::replace(&mut self.started, true) {
                return;
            }
            let (tx, rx) = std::sync::mpsc::channel();
            let mut pending = 0;
            for (name, spec) in &mut self.specs {
                let Some(make) = spec.take() else { continue };
                let tx = tx.clone();
                let name = name.clone();
                pending += 1;
                std::thread::spawn(move || {
                    let result = make().and_then(|mut server| {
                        server.initialize()?;
                        let tools = server.list_tools()?;
                        Ok((server, tools))
                    });
                    let _ = tx.send((name, result));
                });
            }
            drop(tx);
            let deadline = std::time::Instant::now() + START_TIMEOUT;
            while pending > 0 {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                match rx.recv_timeout(left) {
                    Ok((_, Ok((server, tools)))) => {
                        pending -= 1;
                        let (tools, renamed) = expose(&server.name, tools);
                        self.live.push(Live { server, tools, renamed });
                    }
                    Ok((name, Err(e))) => {
                        pending -= 1;
                        eprintln!("dove: {name} integration unavailable: {e}");
                    }
                    Err(_) => {
                        eprintln!("dove: {pending} integration(s) did not start in time");
                        break;
                    }
                }
            }
            // Keep the order the user set, not the order they answered in.
            let order: Vec<String> = self.specs.iter().map(|(n, _)| n.clone()).collect();
            self.live.sort_by_key(|l| order.iter().position(|n| *n == l.server.name).unwrap_or(usize::MAX));
        }

        pub fn tools(&mut self) -> Vec<Value> {
            self.start();
            self.live.iter().flat_map(|l| l.tools.iter().cloned()).collect()
        }

        pub fn call(&mut self, tool: &str, args: Value) -> Value {
            self.start();
            let Some((prefix, _)) = tool.split_once("__") else { return error("Unknown tool") };
            let Some(live) = self.live.iter_mut().find(|l| l.server.name == prefix) else {
                return error(&format!(
                    "The {prefix} integration is not connected. Check Dove → Settings → Integrations."
                ));
            };
            let name = live
                .renamed
                .get(tool)
                .cloned()
                .unwrap_or_else(|| tool.split_once("__").map(|(_, n)| n.to_string()).unwrap_or_default());
            live.server.request("tools/call", json!({"name": name, "arguments": args})).unwrap_or_else(|e| error(&e))
        }
    }

    /// Prefix each tool with its server and keep names within
    /// `MAX_TOOL_NAME` and `[A-Za-z0-9_-]`, remembering the originals.
    pub fn expose(prefix: &str, tools: Vec<Value>) -> (Vec<Value>, HashMap<String, String>) {
        let mut renamed = HashMap::new();
        let mut out = Vec::with_capacity(tools.len());
        for mut t in tools {
            let Some(name) = t.get("name").and_then(Value::as_str).map(str::to_string) else { continue };
            let exposed = exposed_name(prefix, &name);
            if exposed != format!("{prefix}__{name}") {
                renamed.insert(exposed.clone(), name);
            }
            t["name"] = json!(exposed);
            out.push(t);
        }
        (out, renamed)
    }

    pub fn exposed_name(prefix: &str, tool: &str) -> String {
        let clean: String =
            tool.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
        let full = format!("{prefix}__{clean}");
        if full.len() <= MAX_TOOL_NAME {
            return full;
        }
        // FNV-1a of the original keeps shortened names stable and distinct.
        let hash = tool.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
        let suffix = format!("_{:06x}", hash & 0xff_ffff);
        let keep = MAX_TOOL_NAME - suffix.len();
        format!("{}{suffix}", &full[..keep])
    }

    fn error(text: &str) -> Value {
        json!({"content": [{"type": "text", "text": text}], "isError": true})
    }

    /// Account sign-in and consumer keys use Composio Connect directly;
    /// project and user keys open a v3.1 tool-router session
    /// (toolkit-scoped when the user picked any) and use the MCP URL and
    /// headers it returns.
    fn composio(c: &cfg::Composio) -> Result<Server, String> {
        if c.account {
            return Server::http("composio", cfg::COMPOSIO_CONNECT_URL.into(), Vec::new(), Some("composio".into()));
        }
        let kind = ComposioKey::of(&c.api_key);
        let auth = vec![(kind.header().to_string(), c.api_key.clone())];
        if kind == ComposioKey::Consumer {
            return Server::http("composio", cfg::COMPOSIO_CONNECT_URL.into(), auth, None);
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
        Server::http("composio", url, headers, None)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn short_names_pass_through() {
            assert_eq!(exposed_name("linear", "list_issues"), "linear__list_issues");
        }

        #[test]
        fn long_names_are_shortened_stably() {
            let long = "a_really_long_tool_name_that_goes_on_and_on_forever_and_ever";
            let a = exposed_name("cloudflare", long);
            assert_eq!(a.len(), MAX_TOOL_NAME);
            assert_eq!(a, exposed_name("cloudflare", long));
            assert_ne!(a, exposed_name("cloudflare", &format!("{long}_2")));
            let (tools, renamed) = expose("cloudflare", vec![json!({"name": long})]);
            assert_eq!(renamed.get(tools[0]["name"].as_str().unwrap()).map(String::as_str), Some(long));
        }

        #[test]
        fn odd_characters_are_replaced() {
            let (tools, renamed) = expose("x", vec![json!({"name": "get.thing"})]);
            assert_eq!(tools[0]["name"], "x__get_thing");
            assert_eq!(renamed.get("x__get_thing").map(String::as_str), Some("get.thing"));
        }
    }
}
