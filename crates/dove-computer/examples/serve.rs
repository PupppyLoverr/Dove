//! Drive the `computer` tool by hand: a loopback server that runs one
//! action per connection through the same dispatcher Dove's hub uses, over
//! a real `cua-driver mcp`, and keeps its state (target window, row
//! indices, grants) between requests.
//!
//! ```sh
//! DOVE_CUA_DRIVER=<cua-driver(.exe)> cargo run -p dove-computer --example serve -- <port> <out-dir>
//! ```
//!
//! Send one JSON line (the tool's arguments) to 127.0.0.1:<port>; the reply
//! is the text result, then the paths of any screenshots saved to <out-dir>.
//! Approvals are answered "allow in this session" and logged: run it only
//! for a task the user asked for.

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use base64::Engine as _;
use dove_computer::host::OcrLine;
use dove_computer::{AppIdentity, Computer, Config, Decision, Driver, Host, Lease, Review};
use serde_json::{Value, json};

struct McpStdio {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl McpStdio {
    fn start(bin: &str, home: &std::path::Path) -> Self {
        let mut child = Command::new(bin)
            .arg("mcp")
            .env("CUA_DRIVER_RS_HOME", home)
            .env("CUA_DRIVER_PERMISSION_MODE", "standard")
            .env("CUA_DRIVER_RS_TELEMETRY_ENABLED", "0")
            .env("DO_NOT_TRACK", "1")
            .env("CUA_DRIVER_RS_UPDATE_CHECK", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start cua-driver mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut s = Self { _child: child, stdin, stdout, next: 1 };
        s.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "dove-serve", "version": "0"}}))
            .expect("initialize");
        writeln!(s.stdin, "{}", json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).unwrap();
        s
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next;
        self.next += 1;
        writeln!(self.stdin, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        loop {
            line.clear();
            if self.stdout.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("cua-driver exited".into());
            }
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if v.get("method").is_some() {
                if let Some(rid) = v.get("id") {
                    writeln!(self.stdin, "{}", json!({"jsonrpc": "2.0", "id": rid, "result": {}})).ok();
                }
                continue;
            }
            if v.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(e) = v.get("error") {
                    return Err(e.to_string());
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }
}

impl Driver for McpStdio {
    fn call_tool(&mut self, name: &str, args: Value) -> Result<Value, String> {
        self.request("tools/call", json!({"name": name, "arguments": args}))
    }
    fn list_tools(&mut self) -> Result<Vec<Value>, String> {
        let r = self.request("tools/list", json!({}))?;
        Ok(r.get("tools").and_then(Value::as_array).cloned().unwrap_or_default())
    }
}

struct LocalHost {
    cache: PathBuf,
}

impl Host for LocalHost {
    fn review(&mut self, review: &Review) -> Decision {
        eprintln!("approved for this session: {:?} {} {:?}", review.scope, review.app, review.actions);
        Decision::Session
    }
    fn lease(&mut self) -> Lease {
        Lease::default()
    }
    fn identify(&mut self, _pid: u32, app: &str) -> AppIdentity {
        AppIdentity { name: app.into(), ..AppIdentity::default() }
    }
    fn secure_target(&mut self, _pid: u32, window_id: u64, label: Option<&str>) -> Option<bool> {
        dove_computer::native::secure_target(window_id, label)
    }
    fn ocr(&mut self, image: &[u8]) -> Result<Vec<OcrLine>, String> {
        dove_computer::native::ocr(image)
    }
    fn idle_ms(&mut self) -> Option<u64> {
        None
    }
    fn activity(&mut self, app: &str, summary: &str) {
        eprintln!("activity: {app}: {summary}");
    }
    fn cache_dir(&self) -> PathBuf {
        self.cache.clone()
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let port: u16 = args.next().and_then(|p| p.parse().ok()).unwrap_or(47811);
    let out = PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&out).unwrap();
    let bin = std::env::var("DOVE_CUA_DRIVER").expect("DOVE_CUA_DRIVER");
    let home = out.join("driver-home");
    std::fs::create_dir_all(&home).unwrap();
    let mut computer = Computer::new(McpStdio::start(&bin, &home), Config::default());
    let mut host = LocalHost { cache: out.join("cache") };
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    eprintln!("dove-computer serve on 127.0.0.1:{port}");
    let mut n = 0u32;
    for stream in listener.incoming().flatten() {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            continue;
        }
        let mut stream = stream;
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(v) if v["action"] == "quit" => {
                computer.close();
                let _ = writeln!(stream, "bye");
                break;
            }
            Ok(v) => {
                let started = std::time::Instant::now();
                let o = computer.handle(&v, &mut host);
                let mut text = format!("[{} ms]{}\n{}", started.elapsed().as_millis(), if o.is_error { " ERROR" } else { "" }, o.text);
                for (data, mime) in &o.images {
                    n += 1;
                    let ext = if mime == "image/jpeg" { "jpg" } else { "png" };
                    let path = out.join(format!("shot-{n:03}.{ext}"));
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) {
                        let _ = std::fs::write(&path, bytes);
                        text.push_str(&format!("\n[image saved: {}]", path.display()));
                    }
                }
                text
            }
            Err(e) => format!("bad request: {e}"),
        };
        let _ = stream.write_all(reply.as_bytes());
    }
}
