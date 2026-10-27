//! The `computer` tool against the real, stock cua-driver: it opens windows
//! and sends input, so every test here is `#[ignore]` and runs on purpose:
//!
//! ```sh
//! DOVE_CUA_DRIVER=<path to cua-driver(.exe)> cargo test -p dove-computer --test live -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Without `DOVE_CUA_DRIVER` the newest Dove plugin install is used
//! (`~/.dove/plugins/computer-use/<version>/`). The Windows fixture is a
//! small WinForms window (PowerShell) with a name field, a password field, a
//! Save button, a status label and a canvas whose text exists only as pixels.

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use dove_computer::host::OcrLine;
use dove_computer::{AppIdentity, Approvals, Computer, Config, Decision, Driver, Host, Lease, Review};
use serde_json::{Value, json};

// ------------------------------------------------------------ driver

/// `cua-driver mcp` over stdio, with Dove's launch environment.
struct McpStdio {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
    _home: tempfile::TempDir,
}

impl McpStdio {
    fn start(bin: &PathBuf) -> Self {
        let home = tempfile::tempdir().unwrap();
        let mut child = Command::new(bin)
            .arg("mcp")
            .env("CUA_DRIVER_RS_HOME", home.path())
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
        let mut s = Self { child, stdin, stdout, next: 1, _home: home };
        s.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "dove-live-test", "version": "0"}}))
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

    fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for McpStdio {
    fn drop(&mut self) {
        let _ = self.child.kill();
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

fn driver_bin() -> PathBuf {
    if let Some(p) = std::env::var_os("DOVE_CUA_DRIVER") {
        return PathBuf::from(p);
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap();
    let root = home.join(".dove").join("plugins").join("computer-use");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&root).expect("no Dove Computer Use install").flatten().map(|e| e.path()).filter(|p| p.join("plugin.json").is_file()).collect();
    versions.sort();
    let dir = versions.pop().expect("no installed version");
    let record: Value = serde_json::from_slice(&std::fs::read(dir.join("plugin.json")).unwrap()).unwrap();
    dir.join(record["entrypoint"].as_str().unwrap())
}

// ------------------------------------------------------------ host

/// Approves everything, never takes over, reads OCR and secure fields from the OS.
struct LiveHost {
    dir: tempfile::TempDir,
    reviews: Vec<Review>,
}

impl Host for LiveHost {
    fn review(&mut self, review: &Review) -> Decision {
        self.reviews.push(review.clone());
        Decision::Session
    }
    fn lease(&mut self) -> Lease {
        Lease::default()
    }
    fn identify(&mut self, _pid: u32, app: &str) -> AppIdentity {
        AppIdentity { name: app.into(), ..Default::default() }
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
    fn activity(&mut self, _app: &str, _summary: &str) {}
    fn cache_dir(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }
}

fn call(c: &mut Computer<McpStdio>, h: &mut LiveHost, args: Value) -> (Value, dove_computer::Output) {
    let started = Instant::now();
    let out = c.handle(&args, h);
    eprintln!("{:>6} ms  {}", started.elapsed().as_millis(), args["action"]);
    let head = out.text.split("\n\n").next().unwrap_or("").to_string();
    (serde_json::from_str(&head).unwrap_or(Value::Null), out)
}

// ------------------------------------------------------------ fixture

#[cfg(windows)]
const FIXTURE: &str = r#"
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$f = New-Object Windows.Forms.Form
$f.Text = 'Dove CU Fixture'
$f.Size = New-Object Drawing.Size(420, 340)
$f.StartPosition = 'Manual'; $f.Location = New-Object Drawing.Point(40, 40)
$name = New-Object Windows.Forms.TextBox; $name.Name = 'Name'; $name.AccessibleName = 'Name'; $name.Location = New-Object Drawing.Point(20, 20); $name.Width = 200
$pw = New-Object Windows.Forms.TextBox; $pw.UseSystemPasswordChar = $true; $pw.AccessibleName = 'Secret'; $pw.Location = New-Object Drawing.Point(20, 60); $pw.Width = 200
$save = New-Object Windows.Forms.Button; $save.Text = 'Save'; $save.Location = New-Object Drawing.Point(240, 18)
$status = New-Object Windows.Forms.Label; $status.Text = 'idle'; $status.AccessibleName = 'Status'; $status.Location = New-Object Drawing.Point(20, 100); $status.Width = 300
$save.Add_Click({ $status.Text = 'saved:' + $name.Text })
$bmp = New-Object Drawing.Bitmap(360, 90)
$g = [Drawing.Graphics]::FromImage($bmp); $g.Clear([Drawing.Color]::White)
$g.DrawString('PIXEL ONLY TEXT', (New-Object Drawing.Font('Segoe UI', 22)), [Drawing.Brushes]::Black, 8, 20)
$pic = New-Object Windows.Forms.PictureBox; $pic.Image = $bmp; $pic.Location = New-Object Drawing.Point(20, 140); $pic.Size = New-Object Drawing.Size(360, 90)
$f.Controls.AddRange(@($name, $pw, $save, $status, $pic))
[void]$f.ShowDialog()
"#;

#[cfg(windows)]
/// The fixture process, and the folder holding its script (kept alive with it).
struct Fixture(Child, #[allow(dead_code)] tempfile::TempDir);

#[cfg(windows)]
impl Fixture {
    fn open() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fixture.ps1");
        std::fs::write(&script, FIXTURE).unwrap();
        let child = Command::new("powershell.exe")
            .args(["-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script)
            .spawn()
            .expect("start the fixture");
        std::thread::sleep(Duration::from_secs(4));
        Self(child, dir)
    }
}

#[cfg(windows)]
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn find(out: &str, needle: &str) -> u32 {
    out.lines()
        .find(|l| l.contains(needle))
        .and_then(|l| l.trim().strip_prefix('#'))
        .and_then(|l| l.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no row with {needle:?} in\n{out}"))
}

/// The foreground window and the pointer position: what background input
/// must leave alone.
#[cfg(windows)]
fn user_state() -> (isize, (i32, i32)) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetForegroundWindow() -> isize;
        fn GetCursorPos(p: *mut [i32; 2]) -> i32;
    }
    let mut p = [0i32; 2];
    // SAFETY: pure queries into a local buffer.
    unsafe {
        GetCursorPos(&mut p);
        (GetForegroundWindow(), (p[0], p[1]))
    }
}

/// The tick of the last real (hardware or SendInput) input on this seat.
/// Background delivery (UIA patterns, PostMessage) does not move it.
#[cfg(windows)]
fn last_input_tick() -> u32 {
    #[repr(C)]
    struct LastInputInfo {
        size: u32,
        time: u32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetLastInputInfo(info: *mut LastInputInfo) -> i32;
    }
    let mut info = LastInputInfo { size: 8, time: 0 };
    // SAFETY: fills the local struct.
    unsafe { GetLastInputInfo(&mut info) };
    info.time
}

// ------------------------------------------------------------ tests

#[cfg(windows)]
#[test]
#[ignore = "drives real windows with the real cua-driver"]
fn windows_fixture_end_to_end() {
    let fixture = Fixture::open();
    let driver = McpStdio::start(&driver_bin());
    let driver_pid = driver.pid();
    let mut c = Computer::new(driver, Config { approvals: Approvals::Ask, ..Config::default() });
    let mut h = LiveHost { dir: tempfile::tempdir().unwrap(), reviews: Vec::new() };

    // Find the fixture by its title.
    let (v, _) = call(&mut c, &mut h, json!({"action": "list_windows"}));
    let w = v["windows"].as_array().unwrap().iter().find(|w| w["title"] == "Dove CU Fixture").unwrap_or_else(|| panic!("fixture not listed: {v}")).clone();
    let target = json!({"action": "capture", "pid": w["pid"], "window_id": w["window_id"]});

    // Tree-first capture: rows, no pixels. WinForms shows the window a
    // moment before its controls reach the accessibility tree.
    let mut cap = call(&mut c, &mut h, target.clone()).1;
    for _ in 0..20 {
        if cap.text.contains("\"Name\"") {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
        cap = call(&mut c, &mut h, target.clone()).1;
    }
    eprintln!("{}", cap.text);
    assert!(cap.images.is_empty(), "a rich tree sends no screenshot:\n{}", cap.text);
    let name = find(&cap.text, "\"Name\"");
    let save = find(&cap.text, "\"Save\"");

    // Type and click in the background: the foreground window and the
    // pointer stay where they are.
    std::thread::sleep(Duration::from_millis(1500));
    let before = user_state();
    let input_before = last_input_tick();
    let (v, _) = call(&mut c, &mut h, json!({"action": "type", "element": name, "text": "Ada"}));
    assert_eq!(v["ok"], true, "{v}");
    let (v, _) = call(&mut c, &mut h, json!({"action": "click", "element": save}));
    assert_eq!(v["ok"], true, "{v}");
    let (v, _) = call(&mut c, &mut h, json!({"action": "verify", "expect": [{"text": "saved:Ada"}]}));
    assert_eq!(v["status"], "satisfied", "{v}");
    let after = user_state();
    assert_eq!(after.0, before.0, "background input never changes the foreground window");
    if last_input_tick() == input_before {
        // No real input happened, so neither Dove nor a person moved the pointer.
        assert_eq!(after.1, before.1, "background input never moves the pointer");
    } else {
        eprintln!("(someone used the mouse or keyboard during the actions; pointer check skipped)");
    }
    assert_eq!(h.reviews.len(), 1, "one approval for the app, then the session grant");

    // The password field: refused by the OS's IsPassword, before any input.
    let (_, cap) = call(&mut c, &mut h, target.clone());
    let secret = find(&cap.text, "\"Secret\"");
    let (v, _) = call(&mut c, &mut h, json!({"action": "type", "element": secret, "text": "hunter2"}));
    assert_eq!(v["code"], "handoff_required", "{v}");

    // One batch: set the name, save, check the result.
    let (_, cap) = call(&mut c, &mut h, json!({"action": "capture", "pid": w["pid"], "window_id": w["window_id"], "since": "latest"}));
    eprintln!("since read:\n{}", cap.text);
    assert!(cap.text.starts_with("capture mode=auto"), "the since read keeps the target:\n{}", cap.text);
    let (v, out) = call(&mut c, &mut h, json!({"action": "run", "steps": [
        {"action": "set_value", "element": name, "value": "Grace"},
        {"action": "click", "element": save, "expect": [{"text": "saved:Grace"}]},
    ]}));
    assert_eq!(v["ok"], true, "{}", out.text);

    // On-device grounding.
    call(&mut c, &mut h, target.clone());
    let (v, out) = call(&mut c, &mut h, json!({"action": "act", "intent": "click Save"}));
    assert_eq!(v["ok"], true, "{}", out.text);

    // OCR reads text that exists only as pixels.
    let (_, shot) = call(&mut c, &mut h, json!({"action": "capture", "pid": w["pid"], "window_id": w["window_id"], "mode": "som"}));
    assert_eq!(shot.images.len(), 1);
    use base64::Engine as _;
    let raw = base64::engine::general_purpose::STANDARD.decode(&shot.images[0].0).unwrap();
    let lines = dove_computer::native::ocr(&raw).expect("Windows OCR");
    assert!(lines.iter().any(|l| l.text.to_uppercase().contains("PIXEL ONLY")), "{lines:?}");

    // Speed: warm capture latency.
    let mut times = Vec::new();
    for _ in 0..10 {
        let t = Instant::now();
        call(&mut c, &mut h, json!({"action": "capture", "pid": w["pid"], "window_id": w["window_id"], "query": "Save"}));
        times.push(t.elapsed().as_millis());
    }
    times.sort();
    eprintln!("capture(query) p50 {} ms, p90 {} ms", times[5], times[9]);

    // No telemetry, no update checks: the driver holds no remote TCP connection.
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &format!("Get-NetTCPConnection -OwningProcess {driver_pid} -ErrorAction SilentlyContinue | Where-Object {{ $_.RemoteAddress -notin @('127.0.0.1','::1','0.0.0.0','::') }} | Measure-Object | Select-Object -ExpandProperty Count")])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "0", "cua-driver opened a remote connection");
    c.close();
    drop(fixture);
}

#[test]
#[ignore = "reads the real desktop with the real cua-driver"]
fn any_os_lists_and_captures() {
    let mut c = Computer::new(McpStdio::start(&driver_bin()), Config::default());
    let mut h = LiveHost { dir: tempfile::tempdir().unwrap(), reviews: Vec::new() };
    let (v, out) = call(&mut c, &mut h, json!({"action": "list_windows"}));
    assert!(v["count"].as_u64().unwrap_or(0) > 0, "{}", out.text);
    let (_, out) = call(&mut c, &mut h, json!({"action": "capture"}));
    assert!(out.text.starts_with("capture mode=auto"), "{}", out.text);
    assert!(h.reviews.is_empty(), "reading needs no approval");
    c.close();
}
