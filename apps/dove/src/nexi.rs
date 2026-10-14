//! NEXI local models: speech-to-text, text-to-speech and computer use.
//!
//! The NEXI drivers (github.com/PupppyLoverr/NEXI-Models-Drivers) are Python
//! packages with small loopback HTTP servers. Dove installs each one on
//! demand into its own virtualenv under `~/.dove/nexi/<component>` (so the
//! app itself stays small: computer use alone pulls in PyTorch), starts its
//! server when needed, and talks to it over 127.0.0.1. Nothing here leaves
//! the machine except the explicit install step (pip and the pinned model
//! downloads the drivers verify by SHA-256).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};

pub const REPO: &str = "https://github.com/PupppyLoverr/NEXI-Models-Drivers";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Component {
    Stt,
    Tts,
    Cua,
}

impl Component {
    pub const ALL: [Component; 3] = [Component::Stt, Component::Tts, Component::Cua];

    pub fn id(self) -> &'static str {
        match self {
            Component::Stt => "stt",
            Component::Tts => "tts",
            Component::Cua => "cua",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Component::Stt => "NEXI-STT",
            Component::Tts => "NEXI-TTS",
            Component::Cua => "NEXI-CUA",
        }
    }

    /// Folder inside the NEXI repository.
    fn folder(self) -> &'static str {
        match self {
            Component::Stt => "nexi-stt",
            Component::Tts => "nexi-tts",
            Component::Cua => "nexi-cua",
        }
    }

    /// The driver's default port, used to find a server the user started
    /// themselves. Servers Dove starts get a free port (see [`port`]).
    pub fn default_port(self) -> u16 {
        match self {
            Component::Stt => 8767,
            Component::Tts => 8765,
            Component::Cua => 8090,
        }
    }
}

/// `~/.dove/nexi`.
pub fn root(dove_root: &Path) -> PathBuf {
    dove_root.join("nexi")
}

fn source_dir(root: &Path) -> PathBuf {
    root.join("src")
}

fn venv(root: &Path, c: Component) -> PathBuf {
    root.join(c.id()).join("venv")
}

fn venv_python(root: &Path, c: Component) -> PathBuf {
    let v = venv(root, c);
    if cfg!(windows) { v.join("Scripts").join("python.exe") } else { v.join("bin").join("python") }
}

fn models_dir(root: &Path, c: Component) -> PathBuf {
    root.join(c.id()).join("models")
}

fn marker(root: &Path, c: Component) -> PathBuf {
    root.join(c.id()).join("installed")
}

pub fn log_path(root: &Path, c: Component) -> PathBuf {
    root.join(c.id()).join("install.log")
}

pub fn installed(root: &Path, c: Component) -> bool {
    marker(root, c).exists() && venv_python(root, c).exists()
}

fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // CREATE_NO_WINDOW: no console flashing up for each step.
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// A Python 3.10+ to build the virtualenvs with.
fn system_python() -> Result<Vec<String>> {
    let candidates: Vec<Vec<&str>> = if cfg!(windows) {
        vec![vec!["py", "-3.12"], vec!["py", "-3.11"], vec!["py", "-3.10"], vec!["python"], vec!["python3"]]
    } else {
        vec![vec!["python3.12"], vec!["python3.11"], vec!["python3.10"], vec!["python3"], vec!["python"]]
    };
    for c in candidates {
        let out =
            quiet(Command::new(c[0]).args(&c[1..]).args(["-c", "import sys; print(sys.version_info >= (3, 10))"]))
                .output();
        if let Ok(o) = out
            && o.status.success()
            && String::from_utf8_lossy(&o.stdout).trim() == "True"
        {
            return Ok(c.into_iter().map(str::to_string).collect());
        }
    }
    bail!("Python 3.10 or newer is needed for NEXI models (python.org or your package manager)")
}

struct Log(std::fs::File);

impl Log {
    fn open(path: &Path) -> Result<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        Ok(Self(std::fs::OpenOptions::new().create(true).append(true).open(path)?))
    }

    fn line(&mut self, s: &str) {
        let _ = writeln!(self.0, "{s}");
    }

    /// Run one step, its output appended to the log.
    fn run(&mut self, what: &str, cmd: &mut Command) -> Result<()> {
        self.line(&format!("\n== {what}"));
        let out = quiet(cmd).stdin(Stdio::null()).output().with_context(|| format!("could not start: {what}"))?;
        let _ = self.0.write_all(&out.stdout);
        let _ = self.0.write_all(&out.stderr);
        if !out.status.success() {
            let tail: String =
                String::from_utf8_lossy(&out.stderr).lines().rev().take(4).collect::<Vec<_>>().join(" | ");
            bail!("{what} failed: {tail}");
        }
        Ok(())
    }
}

/// Fetch (or update) the NEXI repository into `root/src`.
fn sync_source(root: &Path, log: &mut Log) -> Result<PathBuf> {
    let src = source_dir(root);
    if src.join(".git").exists() {
        let _ = log.run("update NEXI sources", Command::new("git").args(["-C"]).arg(&src).args(["pull", "--ff-only"]));
    } else {
        std::fs::create_dir_all(root)?;
        log.run(
            "download NEXI sources",
            Command::new("git").args(["-c", "core.longpaths=true", "clone", "--depth", "1", REPO]).arg(&src),
        )?;
    }
    Ok(src)
}

/// Add `pkg` to the virtualenv's import path with a `.pth` file.
fn link_source(vpy: &Path, pkg: &Path, c: Component, log: &mut Log) -> Result<()> {
    let out = Command::new(vpy).args(["-c", "import sysconfig; print(sysconfig.get_paths()['purelib'])"]).output()?;
    if !out.status.success() {
        bail!("could not locate the virtualenv's site-packages");
    }
    let site = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let pth = site.join(format!("dove-{}.pth", c.id()));
    std::fs::write(&pth, format!("{}\n", pkg.display()))?;
    let _ = writeln!(log.0, "\n== linked {} into {}", pkg.display(), pth.display());
    Ok(())
}

/// Install one component end to end: sources, virtualenv, packages, and
/// the pinned model download. Blocking; run it off the UI thread.
pub fn install(root: &Path, c: Component, progress: &dyn Fn(&str)) -> Result<()> {
    let mut log = Log::open(&log_path(root, c))?;
    progress("Downloading NEXI sources…");
    let src = sync_source(root, &mut log)?;
    let pkg = src.join(c.folder());
    if !pkg.exists() {
        bail!("{} is missing from the NEXI repository", c.folder());
    }
    progress("Creating a Python environment…");
    let py = system_python()?;
    let venv_dir = venv(root, c);
    if !venv_python(root, c).exists() {
        log.run("create virtualenv", Command::new(&py[0]).args(&py[1..]).args(["-m", "venv"]).arg(&venv_dir))?;
    }
    let vpy = venv_python(root, c);
    let pip = |log: &mut Log, what: &str, args: &[&str]| -> Result<()> {
        log.run(what, Command::new(&vpy).args(["-m", "pip", "install", "--disable-pip-version-check"]).args(args))
    };
    progress("Installing packages (this can take a few minutes)…");
    pip(&mut log, "upgrade pip", &["--upgrade", "pip"])?;
    // Python 3.12 virtualenvs ship without setuptools; the drivers install
    // in editable mode without build isolation, which needs it. Below 81:
    // CTranslate2 (NEXI-STT) still imports `pkg_resources`, which later
    // setuptools removed. NEXI itself pins 80.9.0.
    pip(&mut log, "install build tools", &["setuptools>=70,<81", "wheel"])?;
    match c {
        Component::Stt | Component::Tts => {
            // The pinned, hashed lock targets Linux; fall back to the direct
            // requirements on other platforms.
            let lock = pkg.join("requirements.txt").display().to_string();
            if pip(&mut log, "install pinned requirements", &["-r", &lock]).is_err() {
                let direct = pkg.join("requirements.in").display().to_string();
                pip(&mut log, "install direct requirements", &["-r", &direct])?;
            }
            let installable = pkg.join("setup.py").exists()
                || std::fs::read_to_string(pkg.join("pyproject.toml")).is_ok_and(|t| t.contains("[project]"));
            if installable {
                let path = pkg.display().to_string();
                pip(&mut log, "install driver", &["--no-deps", "--no-build-isolation", "-e", &path])?;
            } else {
                // Not a pip project (NEXI-TTS runs from its checkout): put the
                // checkout on the venv's import path instead.
                link_source(&vpy, &pkg, c, &mut log)?;
            }
        }
        Component::Cua => {
            // CPU PyTorch: a fraction of the default CUDA download.
            pip(&mut log, "install PyTorch (CPU)", &["torch", "--index-url", "https://download.pytorch.org/whl/cpu"])?;
            let wheel = std::fs::read_dir(pkg.join("releases"))?
                .flatten()
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|e| e == "whl"))
                .ok_or_else(|| anyhow!("no NEXI-CUA wheel in releases/"))?;
            let extra = if cfg!(windows) {
                "[windows]"
            } else if cfg!(target_os = "macos") {
                "[macos]"
            } else {
                "[linux]"
            };
            let spec = format!("{}{extra}", wheel.display());
            pip(&mut log, "install NEXI-CUA", &[&spec])?;
            // The learned ranker's weights ship in the repository.
            let dest = models_dir(root, c);
            std::fs::create_dir_all(&dest)?;
            for f in std::fs::read_dir(pkg.join("weights"))?.flatten() {
                std::fs::copy(f.path(), dest.join(f.file_name()))?;
            }
        }
    }
    progress("Downloading the model…");
    let models = models_dir(root, c);
    match c {
        Component::Stt => log.run(
            "download tiny.en",
            Command::new(&vpy)
                .args(["-m", "nexi_stt", "download", "--preset", "tiny.en", "--destination"])
                .arg(models.join("tiny.en")),
        )?,
        Component::Tts => log.run(
            "download Kitten Nano",
            Command::new(&vpy).args(["-m", "nexi_tts", "download", "--model-dir"]).arg(models.join("nano")),
        )?,
        Component::Cua => {}
    }
    std::fs::write(marker(root, c), b"ok")?;
    progress("Installed");
    Ok(())
}

/// Remove a component's environment and models.
pub fn uninstall(root: &Path, c: Component) -> Result<()> {
    stop(c);
    let dir = root.join(c.id());
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

/// Servers Dove started, with the port each listens on.
static SERVERS: Mutex<Vec<(Component, Child, u16)>> = Mutex::new(Vec::new());

/// Where a component answers: the server Dove started, else the default.
pub fn port(c: Component) -> u16 {
    let servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
    servers.iter().find(|(k, ..)| *k == c).map(|(.., p)| *p).unwrap_or(c.default_port())
}

/// A port nothing listens on right now. Fixed ports are not safe on
/// Windows: Python servers bind with `SO_REUSEADDR`, so two can share one
/// and requests land on whichever the system picks.
fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

/// Start a component's loopback server if it is not answering yet, and
/// wait (up to `wait`) until it is healthy.
pub fn ensure_running(root: &Path, c: Component, wait: Duration) -> Result<()> {
    if healthy(c) {
        return Ok(());
    }
    if !installed(root, c) {
        bail!("{} is not installed (Settings › Voice, or Integrations)", c.label());
    }
    {
        let mut servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
        servers.retain_mut(|(_, child, _)| child.try_wait().ok().flatten().is_none());
        if !servers.iter().any(|(k, ..)| *k == c) {
            let vpy = venv_python(root, c);
            let port_n = free_port()?;
            let port = port_n.to_string();
            let models = models_dir(root, c);
            let mut cmd = Command::new(&vpy);
            match c {
                Component::Stt => {
                    cmd.args(["-m", "nexi_stt", "serve", "--model"]).arg(models.join("tiny.en")).args(["--port", &port])
                }
                Component::Tts => cmd
                    .args(["-m", "nexi_tts", "serve", "--model-dir"])
                    .arg(models.join("nano"))
                    .args(["--port", &port]),
                Component::Cua => {
                    cmd.args(["-m", "nexi_cua.cli", "serve", "--port", &port]).env("NEXI_CUA_WEIGHTS", &models)
                }
            };
            let log =
                std::fs::OpenOptions::new().create(true).append(true).open(root.join(c.id()).join("server.log")).ok();
            let child = quiet(&mut cmd)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(log.map(Stdio::from).unwrap_or_else(Stdio::null))
                .spawn()
                .with_context(|| format!("could not start {}", c.label()))?;
            servers.push((c, child, port_n));
        }
    }
    let deadline = std::time::Instant::now() + wait;
    while std::time::Instant::now() < deadline {
        if healthy(c) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    stop(c);
    bail!("{} did not start; see {}", c.label(), root.join(c.id()).join("server.log").display())
}

/// Stop a server Dove started.
pub fn stop(c: Component) {
    let mut servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
    for (_, child, _) in servers.iter_mut().filter(|(k, ..)| *k == c) {
        kill_tree(child);
    }
    servers.retain(|(k, ..)| *k != c);
}

/// Kill a server and its children. On Windows a virtualenv's `python.exe`
/// is a launcher that runs the real interpreter as a child, so killing the
/// launcher alone would leave the server running.
fn kill_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        let _ = quiet(&mut Command::new("taskkill"))
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Stop every server Dove started (app exit).
pub fn stop_all() {
    for c in Component::ALL {
        stop(c);
    }
}

/// `GET /health` answers 200 with the NEXI driver's own health JSON (so an
/// unrelated server on the same port does not count).
pub fn healthy(c: Component) -> bool {
    let Ok((200, body)) = http(port(c), "GET", "/health", None, &[], Duration::from_millis(600)) else {
        return false;
    };
    is_nexi_health(&body)
}

/// NEXI-TTS answers `{"status":"ok"}`, NEXI-STT `{"status":"ready"}` and
/// NEXI-CUA `{"ok":true}`.
fn is_nexi_health(body: &[u8]) -> bool {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    matches!(v["status"].as_str(), Some("ok" | "ready")) || v["ok"] == serde_json::Value::Bool(true)
}

/// A minimal HTTP/1.1 client for the loopback servers (no TLS, no chunked
/// request bodies; chunked responses are decoded).
pub fn http(
    port: u16,
    method: &str,
    path: &str,
    body: Option<(&str, &[u8])>,
    headers: &[(&str, &str)],
    timeout: Duration,
) -> Result<(u16, Vec<u8>)> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(500).min(timeout))?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some((ct, b)) = body {
        req.push_str(&format!("Content-Type: {ct}\r\nContent-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes())?;
    if let Some((_, b)) = body {
        s.write_all(b)?;
    }
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| anyhow!("malformed HTTP response"))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status: u16 = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut body = raw[split + 4..].to_vec();
    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        body = dechunk(&body);
    }
    Ok((status, body))
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(nl) = b.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(String::from_utf8_lossy(&b[..nl]).trim(), 16).unwrap_or(0);
        if size == 0 || b.len() < nl + 2 + size {
            break;
        }
        out.extend_from_slice(&b[nl + 2..nl + 2 + size]);
        b = &b[(nl + 4 + size).min(b.len())..];
    }
    out
}

// ------------------------------------------------------------------ audio

/// Mono f32 samples → 16-bit PCM WAV.
pub fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        w.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    w
}

/// 16-bit PCM WAV → mono f32 samples and rate.
pub fn parse_wav(b: &[u8]) -> Result<(Vec<f32>, u32)> {
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        bail!("not a WAV file");
    }
    let (mut rate, mut channels, mut bits) = (0u32, 1u16, 16u16);
    let mut i = 12;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let len = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        let body = &b[i + 8..(i + 8 + len).min(b.len())];
        if id == b"fmt " && body.len() >= 16 {
            channels = u16::from_le_bytes([body[2], body[3]]);
            rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            if bits != 16 {
                bail!("only 16-bit WAV is supported");
            }
            let ch = channels.max(1) as usize;
            let frames: Vec<f32> = body
                .chunks_exact(2 * ch)
                .map(|f| {
                    let sum: f32 = f.chunks_exact(2).map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0).sum();
                    sum / ch as f32
                })
                .collect();
            return Ok((frames, rate.max(1)));
        }
        i += 8 + len + (len & 1);
    }
    bail!("WAV has no data")
}

/// Resample mono audio linearly (dictation-grade; fine for 16/24/48 kHz).
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let n = (input.len() as f64 / ratio) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * ratio;
            let j = x as usize;
            let f = (x - j as f64) as f32;
            let a = input[j.min(input.len() - 1)];
            let b = input[(j + 1).min(input.len() - 1)];
            a + (b - a) * f
        })
        .collect()
}

// ------------------------------------------------------------- endpoints

/// NEXI-STT: transcribe mono samples (resampled to 16 kHz).
pub fn transcribe(samples: &[f32], rate: u32) -> Result<String> {
    let pcm = resample(samples, rate, 16_000);
    let body = wav(&pcm, 16_000);
    let (status, out) = http(
        port(Component::Stt),
        "POST",
        "/transcribe?vad=1&profile=fast",
        Some(("audio/wav", &body)),
        &[],
        Duration::from_secs(60),
    )?;
    let v: serde_json::Value = serde_json::from_slice(&out).context("NEXI-STT returned non-JSON")?;
    if status != 200 {
        bail!("NEXI-STT: {}", v["error"].as_str().unwrap_or("transcription failed"));
    }
    Ok(v["text"].as_str().unwrap_or_default().trim().to_string())
}

/// NEXI-TTS: synthesize `text` with a built-in voice.
pub fn synthesize(text: &str, voice: &str, speed: f32) -> Result<(Vec<f32>, u32)> {
    let body = serde_json::json!({"text": text, "voice": voice, "language": "en", "speed": speed}).to_string();
    let (status, out) = http(
        port(Component::Tts),
        "POST",
        "/synthesize",
        Some(("application/json", body.as_bytes())),
        &[],
        Duration::from_secs(120),
    )?;
    if status != 200 {
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap_or_default();
        bail!("NEXI-TTS: {}", v["error"].as_str().unwrap_or("synthesis failed"));
    }
    parse_wav(&out)
}

pub const TTS_VOICES: &[&str] = &["Jasper", "Bella", "Luna", "Bruno", "Rosie", "Hugo", "Kiki", "Leo"];

/// NEXI-CUA: one call to its observe / decide / act API.
pub fn cua(path: &str, goal: &str) -> Result<serde_json::Value> {
    let body = serde_json::json!({"goal": goal}).to_string();
    let (status, out) = http(
        port(Component::Cua),
        "POST",
        path,
        Some(("application/json", body.as_bytes())),
        &[],
        Duration::from_secs(90),
    )?;
    let v: serde_json::Value = serde_json::from_slice(&out).context("NEXI-CUA returned non-JSON")?;
    if status != 200 {
        bail!("NEXI-CUA: {}", v["error"].as_str().unwrap_or("request failed"));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_round_trips() {
        let s: Vec<f32> = (0..480).map(|i| ((i as f32) / 20.0).sin() * 0.5).collect();
        let (back, rate) = parse_wav(&wav(&s, 24_000)).unwrap();
        assert_eq!(rate, 24_000);
        assert_eq!(back.len(), s.len());
        assert!(back.iter().zip(&s).all(|(a, b)| (a - b).abs() < 1e-3));
    }

    #[test]
    fn resample_keeps_duration() {
        let s = vec![0.0f32; 48_000];
        assert_eq!(resample(&s, 48_000, 16_000).len(), 16_000);
    }

    #[test]
    fn dechunks_bodies() {
        assert_eq!(dechunk(b"3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n"), b"abcde");
    }

    #[test]
    fn components_have_distinct_ports() {
        let mut ports: Vec<u16> = Component::ALL.iter().map(|c| c.default_port()).collect();
        ports.dedup();
        assert_eq!(ports.len(), 3);
    }

    #[test]
    fn recognizes_only_nexi_health() {
        assert!(is_nexi_health(br#"{"status": "ok", "local": true}"#));
        assert!(is_nexi_health(br#"{"status": "ready", "offline": true}"#));
        assert!(is_nexi_health(br#"{"ok": true}"#));
        assert!(!is_nexi_health(b"<!DOCTYPE HTML><html>OK</html>"));
        assert!(!is_nexi_health(br#"{"ok": false}"#));
    }
}
