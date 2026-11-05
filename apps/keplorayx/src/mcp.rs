//! Keplorayx's own tools for the agents it starts: an MCP server on each
//! agent's stdio (`keplorayx mcp`, which dove-harness hands every harness:
//! Claude Code's `--mcp-config`, ACP's `mcpServers`, …), relaying to the
//! app on a loopback port with a token (the control server here).
//!
//! What agents get:
//! - `show_visual`, `show_image`: generative UI in the thread (Zeron's;
//!   src/visuals.rs runs the script, ds/visual draws it).
//! - `create_plugin`, `write_plugin_file`, `read_plugin_file`,
//!   `list_plugins`, `reload_plugins`: agents make plugins (src/plugins.rs).
//! - Widgets: `list_widgets`, `set_widget`, `remove_widget`,
//!   `calendar_list`, `calendar_add`, `calendar_remove` (src/widgets.rs).
//! - Every allowed plugin's own MCP tools, named under the plugin's id
//!   (`computer_click`): the bridge starts each plugin's server and relays
//!   to it, so the tools reach whichever agent runs (Claude Code, Codex,
//!   Gemini, OpenCode) the same way.

use std::{
    io::{BufRead as _, BufReader, Write as _},
    net::{TcpListener, TcpStream},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::Arc,
};

use serde_json::{Value as Json, json};

const INSTRUCTIONS: &str = "\
Keplorayx runs you. When a chart, table, diagram or dashboard would say more than prose, write a \
Luau visual and show it with `show_visual` before your final reply; Keplorayx draws it with its own \
interface in the thread, so don't announce or restate it. To show an image file, use `show_image`. \
You can extend Keplorayx itself with plugins (`create_plugin`), and keep the person's desktop \
widgets and calendar (`list_widgets`, `set_widget`, `calendar_add`). Tools named after a plugin \
(`computer_…`) come from plugins the person installed.";

/// `show_visual`'s description: Zeron's (crates/mcp tools.rs, MIT), in our
/// words where it names the app; 3D scenes and shaders aren't drawn here yet.
const SHOW_VISUAL: &str = "\
Show an interactive visual (chart, table, dashboard, diagram, simulation) inline in this thread, \
above your final text reply; call it before writing that reply. Use it when a visual says more than \
prose. The reader already sees it, so the reply should not announce, locate, or restate it.\n\
\n\
A visual is a Luau script. Keplorayx runs it in a sandbox (no I/O or network; math.random is seeded) \
and renders what it returns with its own native interface. Build data and state at the top level, \
then return a view function; handlers just assign to state:\n\
\n\
local s = ui.state { metric = \"p95\" }\n\
local data = { { region = \"us-east\", p95 = 182, p99 = 240 }, { region = \"eu-west\", p95 = 140, p99 = 210 } }\n\
return function()\n\
  return ui.column { gap = 16,\n\
    ui.text { \"Latency by region\", size = \"lg\", weight = 600 },\n\
    ui.tabs { options = { \"p95\", \"p99\" }, value = s.metric, on_change = function(v) s.metric = v end },\n\
    ui.chart { type = \"bar\", height = 220, unit = \"ms\",\n\
      x = ui.map(data, function(d) return d.region end),\n\
      series = { { name = s.metric, values = ui.map(data, function(d) return d[s.metric] end) } } },\n\
  }\n\
end\n\
\n\
The script is run before it's shown (setup, render, every draw, each control once); the first error \
comes back with its line: fix it and call again.\n\
- ui.column { gap, align }, ui.row { gap, align, justify = start|center|end|between, wrap }, \
ui.grid { columns, gap }; width = px or grow = true; key = \"name\".\n\
- ui.text { \"plain \", ui.span { \"bold\", weight = 600, color = \"accent\", mono = true }, size = \
xs|sm|body|lg|xl|2xl, weight, color, mono, italic, align, truncate }.\n\
- ui.stat { value, label, delta, tone }; ui.table { columns, rows }; ui.chart { type = \
bar|line|area|scatter, height, x, series = { { name, values, color } }, stacked, horizontal, min, \
max, unit, prefix, decimals, legend }.\n\
- ui.rule {}, ui.space { size }, ui.link { label, url }.\n\
- Controls: ui.tabs { options, value, on_change }, ui.toggle { label, value, on_change }, \
ui.slider { label, value, min, max, step, unit, on_change }, ui.button { label, on_click, primary }.\n\
- ui.canvas { height, draw = function(g, w, h, t) end, animate, on_click }: g:line, g:polyline, \
g:polygon, g:rect, g:circle, g:arc, g:text; g.pointer while hovered.\n\
- Helpers: ui.scale, ui.ticks, ui.fmt, ui.map, ui.range, ui.clamp, ui.lerp, ui.series(i), \
ui.color(token, alpha), ui.mix.\n\
Colors are theme tokens only: text, muted, faint, accent, on-accent, success, warning, danger, border, \
chart1-chart6. No cards, fills or boxes: content lies directly on the thread. Heights 160-420px; \
the width is the reply column (360-720px).";

fn tools() -> Json {
    let obj = |props: Json, required: &[&str]| json!({ "type": "object", "properties": props, "required": required });
    json!([
        { "name": "show_visual", "description": SHOW_VISUAL, "inputSchema": obj(json!({
            "title": { "type": "string", "description": "A short title" },
            "script": { "type": "string", "description": "The Luau script" },
        }), &["title", "script"]) },
        { "name": "show_image", "description": "Show an image file (PNG, JPEG, WebP, GIF; at most 10 MB) inline in this thread, above your final reply. Keplorayx copies it, so it stays after the file changes.",
          "inputSchema": obj(json!({ "path": { "type": "string" } }), &["path"]) },
        { "name": "list_plugins", "description": "Keplorayx's plugins: installed (and whether the person allowed them) and the catalog's.",
          "inputSchema": obj(json!({}), &[]) },
        { "name": "create_plugin", "description": "Make a Keplorayx plugin: a folder with kx-plugin.json and init.luau, run by zgx in a sandbox in Keplorayx's window. The person allows it in Settings › Plugins before it runs. Permissions: ui.add, ui.change, sessions.read, messages.read, navigate, compose, network, clipboard, files, web, windows, terminals. init.luau uses zgx's `ui` (ui.contribute { target = \"#sidebar\", after = ui.text { … } }: a node, not a function; an element by its id, as #sidebar; a wrong target or code comes back from reload_plugins as its error), ui.shared(\"kx.threads\") with sessions.read, ui.shared(\"kx.now\")).",
          "inputSchema": obj(json!({
            "id": { "type": "string" }, "name": { "type": "string" }, "description": { "type": "string" },
            "permissions": { "type": "array", "items": { "type": "string" } },
            "code": { "type": "string", "description": "init.luau" },
            "replace": { "type": "boolean" },
          }), &["id", "name", "code"]) },
        { "name": "write_plugin_file", "description": "Write a file in a plugin's folder (init.luau, kx-plugin.json, a module). It reloads.",
          "inputSchema": obj(json!({ "id": { "type": "string" }, "path": { "type": "string" }, "content": { "type": "string" } }), &["id", "path", "content"]) },
        { "name": "read_plugin_file", "description": "Read a file in a plugin's folder.",
          "inputSchema": obj(json!({ "id": { "type": "string" }, "path": { "type": "string" } }), &["id", "path"]) },
        { "name": "reload_plugins", "description": "Load the plugins again; returns their list with any errors.",
          "inputSchema": obj(json!({}), &[]) },
        { "name": "list_widgets", "description": "The person's desktop widgets: each one's id, kind, variant, place and settings.",
          "inputSchema": obj(json!({}), &[]) },
        { "name": "set_widget", "description": "Add or change a desktop widget. kind: clock, calendar, agents, usage, ask, threads, system, music, note, or custom (with `script`: a Luau view as show_visual's). variant: small, medium or large (or a kind's own: clock's analog|digital|world). Widgets live on the desktop, under every window. settings: the kind's own (clock: { zones = {\"Europe/London\"} }; note: { text }).",
          "inputSchema": obj(json!({
            "id": { "type": "string", "description": "Omit to add one" }, "kind": { "type": "string" },
            "variant": { "type": "string" }, "title": { "type": "string" },
            "settings": { "type": "object" }, "script": { "type": "string" },
            "x": { "type": "number", "description": "Points from the desktop's top left" }, "y": { "type": "number" },
          }), &["kind"]) },
        { "name": "remove_widget", "description": "Remove a desktop widget by id.",
          "inputSchema": obj(json!({ "id": { "type": "string" } }), &["id"]) },
        { "name": "calendar_list", "description": "The person's Keplorayx calendar: events from `from` to `to` (YYYY-MM-DD; default the next 14 days).",
          "inputSchema": obj(json!({ "from": { "type": "string" }, "to": { "type": "string" } }), &[]) },
        { "name": "calendar_add", "description": "Add or change an event on the person's Keplorayx calendar (shown in the calendar widget). date YYYY-MM-DD, time HH:MM (24h, optional), minutes (length).",
          "inputSchema": obj(json!({
            "id": { "type": "string" }, "title": { "type": "string" }, "date": { "type": "string" },
            "time": { "type": "string" }, "minutes": { "type": "number" }, "notes": { "type": "string" },
          }), &["title", "date"]) },
        { "name": "calendar_remove", "description": "Remove an event from the person's Keplorayx calendar by id.",
          "inputSchema": obj(json!({ "id": { "type": "string" } }), &["id"]) },
    ])
}

// ── The app's side: the control server ─────────────────────────────────

/// A call from a bridge, to answer on the window's thread.
pub struct Call {
    pub tool: String,
    pub args: Json,
    /// The agent's folder (for tools that act where it works).
    #[allow(dead_code)]
    pub cwd: String,
    pub reply: flume::Sender<Result<String, String>>,
}


/// Listens on loopback for this process's bridges and offers the bridge to
/// every agent started from now on. Calls arrive on the returned channel.
pub fn start() -> Option<flume::Receiver<Call>> {
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    let token = token();
    let (tell, heard) = flume::unbounded::<Call>();
    let expected = token.clone();
    std::thread::Builder::new()
        .name("kx-mcp".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let (tell, expected) = (tell.clone(), expected.clone());
                std::thread::spawn(move || serve_bridge(stream, &expected, &tell));
            }
        })
        .ok()?;
    // (Development: say where, so a test can be the agent.)
    if std::env::var_os("KX_MCP_ANNOUNCE").is_some() {
        eprintln!("kx-mcp: port={port} token={token}");
    }
    let exe = std::env::current_exe().ok()?;
    dove_harness::set_tool_server(Some(dove_harness::ToolServer {
        name: "keplorayx".into(),
        command: exe.display().to_string(),
        args: vec!["mcp".into()],
        env: vec![
            ("KX_PORT".into(), port.to_string()),
            ("KX_TOKEN".into(), token),
            ("DOVE_HOME".into(), dove_store::default_root().display().to_string()),
        ],
    }));
    Some(heard)
}

fn token() -> String {
    use std::hash::{BuildHasher as _, Hasher as _};
    let mut out = String::new();
    for i in 0..4u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i ^ std::process::id() as u64);
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// One bridge's connection: a request a line, an answer a line.
fn serve_bridge(stream: TcpStream, token: &str, tell: &flume::Sender<Call>) {
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    for line in BufReader::new(read).lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Json>(&line) else { break };
        if req["token"].as_str() != Some(token) {
            break;
        }
        let tool = req["tool"].as_str().unwrap_or_default().to_string();
        let answer = if tool == "show_visual" {
            // (Checked here, off the window's thread: it runs the script.)
            show_visual(&req["args"])
        } else {
            let (reply, wait) = flume::bounded(1);
            let call = Call { tool, args: req["args"].clone(), cwd: req["cwd"].as_str().unwrap_or("").into(), reply };
            if tell.send(call).is_err() {
                break;
            }
            wait.recv().unwrap_or_else(|_| Err("Keplorayx closed".into()))
        };
        let line = match answer {
            Ok(text) => json!({ "ok": true, "text": text }),
            Err(error) => json!({ "ok": false, "error": error }),
        };
        if writeln!(out, "{line}").is_err() {
            break;
        }
    }
}

/// `show_visual`: run the script as the thread will (zeron-visual's
/// check), then keep it where the transcript finds it.
fn show_visual(args: &Json) -> Result<String, String> {
    let script = args["script"].as_str().unwrap_or_default();
    let title = args["title"].as_str().unwrap_or("Visual").replace(['|', '\n'], " ");
    if script.trim().is_empty() {
        return Err("script is empty".into());
    }
    let id = format!("v-{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    let report = zeron_visual::check(script, zeron_visual::seed_for(&id)).map_err(|e| e.message)?;
    let dir = dove_store::default_root().join("visuals");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{id}-visual.luau"));
    std::fs::write(&path, script).map_err(|e| e.to_string())?;
    let logs = if report.logs.is_empty() { String::new() } else { format!("\nprint: {}", report.logs.join("\n")) };
    // (The last line is the transcript's, which draws the visual in its place.)
    Ok(format!(
        "Shown above your reply ({} nodes, {} controls checked).{logs}\n[visual {id} | {title} | {}]",
        report.nodes,
        report.interactions,
        path.display()
    ))
}

/// `show_image`: a copy kept with the thread's images.
pub fn show_image(args: &Json) -> Result<String, String> {
    let from = std::path::PathBuf::from(args["path"].as_str().unwrap_or_default());
    let ext = from.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif") {
        return Err("a PNG, JPEG, WebP or GIF".into());
    }
    let size = std::fs::metadata(&from).map_err(|e| format!("{}: {e}", from.display()))?.len();
    if size > 10 * 1024 * 1024 {
        return Err("at most 10 MB".into());
    }
    let dir = dove_store::default_root().join("images");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let id = format!("i-{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    let to = dir.join(format!("{id}.{ext}"));
    std::fs::copy(&from, &to).map_err(|e| e.to_string())?;
    Ok(format!("Shown above your reply.\n[image {id} | {}]", to.display()))
}

/// The calls answered on the window's thread, as they come.
pub fn answer(engine: Arc<dove_core::Engine>, calls: flume::Receiver<Call>, tasks: &zgx_desktop::TaskSpawner) {
    tasks.spawn(async move {
        while let Ok(call) = calls.recv_async().await {
            let answer = match call.tool.as_str() {
                "show_image" => show_image(&call.args),
                "list_plugins" => Ok(crate::plugins::list_text(&engine)),
                "create_plugin" => crate::plugins::create(&call.args).inspect(|_| crate::plugins::reload_all(&engine)),
                "write_plugin_file" => {
                    crate::plugins::write_file(&call.args).inspect(|_| crate::plugins::reload_all(&engine))
                }
                "read_plugin_file" => crate::plugins::read_file(&call.args),
                "reload_plugins" => {
                    crate::plugins::reload_all(&engine);
                    Ok(crate::plugins::list_text(&engine))
                }
                "plugin_servers" => Ok(serde_json::to_string(
                    &crate::plugins::servers(&engine)
                        .into_iter()
                        .map(|(id, mcp, folder)| json!({ "id": id, "command": mcp.command, "args": mcp.args, "env": mcp.env, "cwd": folder, "instructions": mcp.instructions }))
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_default()),
                tool if tool.starts_with("calendar_") || tool.ends_with("_widget") || tool == "list_widgets" => {
                    crate::widgets::call(&engine, tool, &call.args)
                }
                other => Err(format!("no tool `{other}`")),
            };
            let _ = call.reply.send(answer);
        }
    });
}

// ── The bridge: `keplorayx mcp`, on an agent's stdio ───────────────────

struct Link {
    out: TcpStream,
    read: BufReader<TcpStream>,
    token: String,
}

impl Link {
    fn connect() -> Option<Self> {
        let port: u16 = std::env::var("KX_PORT").ok()?.parse().ok()?;
        let token = std::env::var("KX_TOKEN").ok()?;
        let out = TcpStream::connect(("127.0.0.1", port)).ok()?;
        let read = BufReader::new(out.try_clone().ok()?);
        Some(Self { out, read, token })
    }

    fn call(&mut self, tool: &str, args: &Json) -> Result<String, String> {
        let cwd = std::env::var("DOVE_CWD").unwrap_or_default();
        let req = json!({ "token": self.token, "tool": tool, "args": args, "cwd": cwd });
        writeln!(self.out, "{req}").map_err(|_| "Keplorayx isn't running".to_string())?;
        let mut line = String::new();
        self.read.read_line(&mut line).map_err(|_| "Keplorayx closed".to_string())?;
        let answer: Json = serde_json::from_str(&line).map_err(|_| "Keplorayx closed".to_string())?;
        if answer["ok"].as_bool() == Some(true) {
            Ok(answer["text"].as_str().unwrap_or_default().to_string())
        } else {
            Err(answer["error"].as_str().unwrap_or("failed").to_string())
        }
    }
}

/// A plugin's MCP server, started by the bridge: its tools, relayed.
struct Server {
    prefix: String,
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    tools: Vec<Json>,
    next: u64,
}

impl Server {
    fn start(spec: &Json) -> Option<Self> {
        let id = spec["id"].as_str()?;
        let mut cmd = Command::new(spec["command"].as_str()?);
        cmd.args(spec["args"].as_array().into_iter().flatten().filter_map(|a| a.as_str()));
        if let Some(env) = spec["env"].as_object() {
            for (k, v) in env {
                cmd.env(k, v.as_str().unwrap_or_default());
            }
        }
        if let Some(cwd) = spec["cwd"].as_str() {
            cmd.current_dir(cwd);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = match cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() {
            Ok(child) => child,
            Err(e) => {
                eprintln!("keplorayx mcp: {id}: {e}");
                return None;
            }
        };
        let stdin = child.stdin.take()?;
        let stdout = BufReader::new(child.stdout.take()?);
        let mut server =
            Self { prefix: id.replace('-', "_"), _child: child, stdin, stdout, tools: Vec::new(), next: 1 };
        let init = server.request("initialize", json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "keplorayx", "version": env!("CARGO_PKG_VERSION") },
        }));
        if init.is_none() {
            eprintln!("keplorayx mcp: {id}: no answer to initialize");
            return None;
        }
        let _ = writeln!(server.stdin, "{}", json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        let Some(listed) = server.request("tools/list", json!({})) else {
            eprintln!("keplorayx mcp: {id}: no answer to tools/list");
            return None;
        };
        server.tools = listed["tools"].as_array().cloned().unwrap_or_default();
        Some(server)
    }

    fn request(&mut self, method: &str, params: Json) -> Option<Json> {
        let id = self.next;
        self.next += 1;
        writeln!(self.stdin, "{}", json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).ok()?;
        loop {
            let mut line = String::new();
            if self.stdout.read_line(&mut line).ok()? == 0 {
                return None;
            }
            let Ok(msg) = serde_json::from_str::<Json>(&line) else { continue };
            if msg["id"].as_u64() == Some(id) {
                return Some(if msg.get("error").is_some() { json!({ "error": msg["error"] }) } else { msg["result"].clone() });
            }
        }
    }

    /// A tool's name as this bridge offers it: under its prefix (a name
    /// already under it, as cua's `computer_click`, kept as it is).
    fn exposed(&self, name: &str) -> String {
        let name = if name.starts_with(&format!("{}_", self.prefix)) { name.to_string() } else { format!("{}_{name}", self.prefix) };
        name.chars().take(64).collect()
    }

    /// Its tools, as this bridge offers them.
    fn listed(&self) -> Vec<Json> {
        self.tools
            .iter()
            .map(|t| {
                let mut t = t.clone();
                t["name"] = json!(self.exposed(t["name"].as_str().unwrap_or_default()));
                t
            })
            .collect()
    }

    /// The tool of its own an offered name stands for.
    fn original(&self, offered: &str) -> Option<String> {
        self.tools
            .iter()
            .filter_map(|t| t["name"].as_str())
            .find(|n| self.exposed(n) == offered)
            .map(str::to_string)
    }
}

/// `keplorayx mcp`: serves until the agent closes its stdin.
pub fn serve_stdio() -> i32 {
    let mut link = Link::connect();
    let mut servers: Vec<Server> = Vec::new();
    // Each plugin's word to the agent on its tools, after ours.
    let mut instructions = INSTRUCTIONS.to_string();
    if let Some(link) = link.as_mut()
        && let Ok(list) = link.call("plugin_servers", &json!({}))
        && let Ok(specs) = serde_json::from_str::<Vec<Json>>(&list)
    {
        servers = specs.iter().filter_map(Server::start).collect();
        for spec in &specs {
            if let Some(words) = spec["instructions"].as_str() {
                instructions.push_str("

");
                instructions.push_str(words);
            }
        }
    }
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Json>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { continue };
        let method = msg["method"].as_str().unwrap_or_default();
        let result = match method {
            "initialize" => Ok(json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "keplorayx", "version": env!("CARGO_PKG_VERSION") },
                "instructions": instructions,
            })),
            "ping" => Ok(json!({})),
            "tools/list" => {
                let mut list = tools().as_array().cloned().unwrap_or_default();
                for s in &servers {
                    list.extend(s.listed());
                }
                Ok(json!({ "tools": list }))
            }
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or_default().to_string();
                let args = msg["params"]["arguments"].clone();
                let plugin = servers.iter_mut().find_map(|s| s.original(&name).map(|tool| (s, tool)));
                if let Some((server, tool)) = plugin {
                    match server.request("tools/call", json!({ "name": tool, "arguments": args })) {
                        Some(r) if r.get("error").is_some() => Err(r["error"]["message"].as_str().unwrap_or("failed").to_string()),
                        Some(r) => Ok(r),
                        None => Err("the plugin's tool server stopped".into()),
                    }
                } else {
                    let answer = match link.as_mut() {
                        Some(link) => link.call(&name, &args),
                        None => Err("Keplorayx isn't running".into()),
                    };
                    Ok(match answer {
                        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
                        Err(error) => json!({ "content": [{ "type": "text", "text": error }], "isError": true }),
                    })
                }
            }
            _ => Err(format!("no method `{method}`")),
        };
        let reply = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(message) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": message } }),
        };
        if writeln!(stdout, "{reply}").is_err() || stdout.flush().is_err() {
            break;
        }
    }
    0
}
