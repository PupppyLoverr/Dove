//! Local NEXI drivers from [NEXI-Models-Drivers](https://github.com/PupppyLoverr/NEXI-Models-Drivers).
//!
//! Nothing here is linked into the Dove binary. Speech models and the computer-use
//! runtime stay on disk and are started as their own processes, so the app stays
//! inside its size budget. Set `DOVE_NEXI_ROOT` to the checkout, or clone it next
//! to Dove (for example `~/Downloads/NEXI-Models-Drivers`).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug)]
pub struct Report {
    pub root: Option<PathBuf>,
    pub tts_python: Option<PathBuf>,
    pub tts_model: bool,
    pub stt_python: Option<PathBuf>,
    pub stt_model: bool,
    pub cua: Option<PathBuf>,
}

impl Report {
    pub fn probe() -> Self {
        let root = find_root();
        let tts_python = python_in(&root, "nexi-tts");
        let stt_python = python_in(&root, "nexi-stt");
        let tts_model = root
            .as_ref()
            .is_some_and(|r| r.join("nexi-tts/.models/nano-int8/kitten_tts_nano_v0_8.onnx").is_file());
        let stt_model = root.as_ref().is_some_and(|r| r.join("nexi-stt/models/tiny.en/model.bin").is_file());
        let cua = find_cua(root.as_deref());
        Self { root, tts_python, tts_model, stt_python, stt_model, cua }
    }

    pub fn tts_line(&self) -> String {
        match (&self.tts_python, self.tts_model) {
            (Some(_), true) => "NEXI TTS is ready (Kitten Nano, on this device).".into(),
            (Some(_), false) => "Python env is ready. Download the Nano voice with the setup command.".into(),
            (None, _) => "Not set up. Copy the setup command and run it once.".into(),
        }
    }

    pub fn stt_line(&self) -> String {
        match (&self.stt_python, self.stt_model) {
            (Some(_), true) => "NEXI STT is ready (Whisper tiny.en, on this device).".into(),
            (Some(_), false) => "Python env is ready. Download tiny.en with the setup command.".into(),
            (None, _) => "Not set up. Copy the setup command and run it once.".into(),
        }
    }

    pub fn cua_line(&self) -> String {
        if let Some(bin) = &self.cua {
            return bin.display().to_string();
        }
        if self.root.as_ref().is_some_and(|r| r.join("nexi-cua").is_dir()) {
            return "Source is on disk. Create the nexi-cua venv to turn it on.".into();
        }
        "Not installed. Clone NEXI-Models-Drivers and install nexi-cua.".into()
    }
}

pub fn stt_ready() -> bool {
    let r = Report::probe();
    r.stt_python.is_some() && r.stt_model
}

pub fn cua_bin() -> Option<PathBuf> {
    Report::probe().cua
}

/// Commands a person runs once, from the NEXI checkout, to fetch the drivers.
pub fn setup_script(root: Option<&Path>) -> String {
    let root = root.map(|p| p.display().to_string()).unwrap_or_else(|| "NEXI-Models-Drivers".into());
    if cfg!(windows) {
        format!(
            r#"# Speech to text (Whisper tiny.en, local)
cd "{root}\nexi-stt"
py -3.12 -m venv .venv
.venv\Scripts\python -m pip install -r requirements.txt
.venv\Scripts\python -m nexi_stt download --preset tiny.en --destination models\tiny.en

# Text to speech (Kitten Nano, local)
cd "{root}\nexi-tts"
py -3.12 -m venv .venv
.venv\Scripts\python -m pip install -r requirements.txt
.venv\Scripts\python -m nexi_tts download --profile nano

# Computer and browser use
cd "{root}\nexi-cua"
py -3.12 -m venv .venv
.venv\Scripts\python -m pip install -e ".[windows]"
.venv\Scripts\nexi-cua doctor
"#
        )
    } else {
        let extra = if cfg!(target_os = "macos") { "macos" } else { "linux" };
        format!(
            r#"# Speech to text (Whisper tiny.en, local)
cd "{root}/nexi-stt"
python3.12 -m venv .venv
.venv/bin/python -m pip install -r requirements.txt
.venv/bin/python -m nexi_stt download --preset tiny.en --destination models/tiny.en

# Text to speech (Kitten Nano, local)
cd "{root}/nexi-tts"
python3.12 -m venv .venv
.venv/bin/python -m pip install -r requirements.txt
.venv/bin/python -m nexi_tts download --profile nano

# Computer and browser use
cd "{root}/nexi-cua"
python3.12 -m venv .venv
.venv/bin/python -m pip install -e ".[{extra}]"
.venv/bin/nexi-cua doctor
"#
        )
    }
}

fn find_root() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(p) = std::env::var_os("DOVE_NEXI_ROOT") {
        candidates.push(PathBuf::from(p));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("NEXI-Models-Drivers"));
        if let Some(parent) = cwd.parent() {
            candidates.push(parent.join("NEXI-Models-Drivers"));
        }
    }
    if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        let home = PathBuf::from(home);
        candidates.push(home.join("Downloads/NEXI-Models-Drivers"));
        candidates.push(home.join("src/NEXI-Models-Drivers"));
        candidates.push(home.join("code/NEXI-Models-Drivers"));
    }
    candidates.into_iter().find(|p| p.join("nexi-tts").is_dir() && p.join("nexi-cua").is_dir())
}

fn python_in(root: &Option<PathBuf>, folder: &str) -> Option<PathBuf> {
    let root = root.as_ref()?;
    let venv = root.join(folder).join(".venv");
    let win = venv.join("Scripts/python.exe");
    let unix = venv.join("bin/python");
    if win.is_file() {
        Some(win)
    } else if unix.is_file() {
        Some(unix)
    } else {
        None
    }
}

fn find_cua(root: Option<&Path>) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "nexi-cua.exe" } else { "nexi-cua" };
    let mut dirs: Vec<_> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("nexi-cua/Scripts"));
    }
    if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from) {
        dirs.push(home.join(".nexi-cua/bin"));
        dirs.push(home.join(".local/bin"));
    }
    if let Some(root) = root {
        let venv = root.join("nexi-cua/.venv");
        dirs.push(venv.join("Scripts"));
        dirs.push(venv.join("bin"));
    }
    dirs.into_iter().map(|d| d.join(exe)).find(|p| p.is_file())
}

fn silence_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = cmd;
}

fn run_bounded(mut cmd: Command, limit: Duration) -> Result<std::process::Output, String> {
    silence_window(&mut cmd);
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("NEXI did not finish within {} s", limit.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => return Err(e.to_string()),
        }
    }
    child.wait_with_output().map_err(|e| e.to_string())
}

fn output_text(out: &std::process::Output) -> String {
    let mut text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        text = String::from_utf8_lossy(&out.stderr).trim().to_string();
    }
    if text.chars().count() > 6000 {
        text = text.chars().take(6000).collect::<String>() + "…";
    }
    text
}

/// Run `nexi-cua` and return its stdout (or a short error).
pub fn cua_exec(args: &[String]) -> Result<String, String> {
    let bin = cua_bin().ok_or_else(|| {
        "nexi-cua is not installed. Open Settings → Integrations → NEXI computer use and run the setup command."
            .to_string()
    })?;
    let mut cmd = Command::new(&bin);
    cmd.args(args);
    if let Some(root) = find_root() {
        cmd.current_dir(root.join("nexi-cua"));
    }
    let out = run_bounded(cmd, Duration::from_secs(90))?;
    let text = output_text(&out);
    if out.status.success() || out.status.code() == Some(2) {
        // `run` exits 2 when the loop stops short of done (abstain, confirm).
        Ok(if text.is_empty() { format!("nexi-cua finished ({})", out.status) } else { text })
    } else {
        Err(if text.is_empty() { format!("nexi-cua exited {}", out.status) } else { text })
    }
}

pub fn synthesize_wav(text: &str, voice: &str, out: &Path) -> Result<(), String> {
    let report = Report::probe();
    let python = report.tts_python.ok_or("NEXI TTS is not set up. Open Settings → Voice and copy the setup command.")?;
    if !report.tts_model {
        return Err("The NEXI voice model is not downloaded yet. Run `python -m nexi_tts download --profile nano`.".into());
    }
    let root = report.root.ok_or("NEXI checkout was not found.")?;
    let dir = std::env::temp_dir().join("dove-voice");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let src = dir.join("say.txt");
    std::fs::write(&src, text).map_err(|e| e.to_string())?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let voice = if voice.is_empty() { "Jasper" } else { voice };
    let mut cmd = Command::new(python);
    cmd.current_dir(root.join("nexi-tts"))
        .args([
            "-m",
            "nexi_tts",
            "speak",
            "--profile",
            "nano",
            "--text-file",
            &src.display().to_string(),
            "--voice",
            voice,
            "--language",
            "en",
            "--speed",
            "1",
            "--threads",
            "2",
            "--output",
            &out.display().to_string(),
        ])
        .env("PYTHONNOUSERSITE", "1");
    let result = run_bounded(cmd, Duration::from_secs(120))?;
    if !result.status.success() {
        return Err(output_text(&result));
    }
    if !out.is_file() {
        return Err("NEXI TTS finished without writing audio.".into());
    }
    Ok(())
}

/// Fish Audio's HTTP TTS. The key is the user's; Dove does not ship one.
pub fn fish_wav(api_key: &str, text: &str, out: &Path) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Add a Fish Audio API key in Settings → Voice.".into());
    }
    let body = serde_json::json!({
        "text": text,
        "format": "wav",
        "latency": "balanced"
    })
    .to_string();
    let bytes = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?.block_on(
        async move {
            let resp = reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .map_err(|e| e.to_string())?
                .post("https://api.fish.audio/v1/tts")
                .header("Authorization", format!("Bearer {key}"))
                .header("Content-Type", "application/json")
                .header("model", "s1")
                .body(body)
                .send()
                .await
                .map_err(|e| format!("Fish Audio: {e}"))?;
            let status = resp.status();
            let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
            if !status.is_success() {
                let msg = String::from_utf8_lossy(&bytes);
                return Err(format!("Fish Audio {status}: {}", msg.chars().take(240).collect::<String>()));
            }
            Ok::<_, String>(bytes)
        },
    )?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(out, &bytes).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn write_wav(path: &Path, samples: &[f32], rate: u32) -> std::io::Result<()> {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let v = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
        pcm.extend_from_slice(&v.to_le_bytes());
    }
    let data_len = pcm.len() as u32;
    let mut header = Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&(36 + data_len).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&rate.to_le_bytes());
    header.extend_from_slice(&(rate * 2).to_le_bytes());
    header.extend_from_slice(&2u16.to_le_bytes());
    header.extend_from_slice(&16u16.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data_len.to_le_bytes());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(path)?;
    file.write_all(&header)?;
    file.write_all(&pcm)?;
    Ok(())
}

pub fn transcribe_samples(samples: &[f32], rate: u32) -> Result<String, String> {
    if samples.len() < rate as usize / 5 || samples.iter().all(|s| s.abs() < 0.0008) {
        return Ok(String::new());
    }
    let report = Report::probe();
    let python = report
        .stt_python
        .ok_or("NEXI speech-to-text is not set up. Open Settings → Voice and copy the setup command.")?;
    if !report.stt_model {
        return Err("The NEXI speech model is not downloaded yet (models/tiny.en).".into());
    }
    let root = report.root.ok_or("NEXI checkout was not found.")?;
    let dir = std::env::temp_dir().join("dove-voice");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let wav = dir.join(format!("hear-{}.wav", std::process::id()));
    let capped = samples.len().min(rate as usize * 60);
    write_wav(&wav, &samples[..capped], rate).map_err(|e| e.to_string())?;
    let model = root.join("nexi-stt/models/tiny.en");
    let mut cmd = Command::new(python);
    cmd.current_dir(root.join("nexi-stt"))
        .args([
            "-m",
            "nexi_stt",
            "transcribe",
            &wav.display().to_string(),
            "--model",
            &model.display().to_string(),
            "--format",
            "text",
        ])
        .env("PYTHONNOUSERSITE", "1");
    let out = run_bounded(cmd, Duration::from_secs(120))?;
    let _ = std::fs::remove_file(&wav);
    if !out.status.success() {
        return Err(output_text(&out));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A live microphone. Drop it, or call [`Capture::into_pcm`], to stop.
pub struct Capture {
    samples: Arc<Mutex<Vec<f32>>>,
    level: Arc<AtomicU32>,
    rate: u32,
    _stream: cpal::Stream,
}

impl Capture {
    pub fn start(device_name: Option<&str>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = pick_input(&host, device_name)?;
        let config = device.default_input_config().map_err(|e| e.to_string())?;
        let rate = config.sample_rate();
        let channels = config.channels() as usize;
        let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
        let level = Arc::new(AtomicU32::new(0));
        let full = Arc::new(AtomicBool::new(false));
        let cap = rate as usize * 60;
        let samples_cb = samples.clone();
        let level_cb = level.clone();
        let full_cb = full.clone();
        let err = |e: cpal::StreamError| eprintln!("dove mic: {e}");
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device
                .build_input_stream(
                    &config.into(),
                    move |data: &[f32], _| push_frames(data, channels, cap, &samples_cb, &level_cb, &full_cb),
                    err,
                    None,
                )
                .map_err(|e| e.to_string())?,
            cpal::SampleFormat::I16 => device
                .build_input_stream(
                    &config.into(),
                    move |data: &[i16], _| {
                        let floats: Vec<f32> = data.iter().map(|s| *s as f32 / 32768.0).collect();
                        push_frames(&floats, channels, cap, &samples_cb, &level_cb, &full_cb);
                    },
                    err,
                    None,
                )
                .map_err(|e| e.to_string())?,
            other => return Err(format!("This microphone's format ({other:?}) is not supported")),
        };
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self { samples, level, rate, _stream: stream })
    }

    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    pub fn into_pcm(self) -> (Vec<f32>, u32) {
        drop(self._stream);
        let samples = self.samples.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
        (samples, self.rate)
    }
}

fn push_frames(
    data: &[f32],
    channels: usize,
    cap: usize,
    samples: &Mutex<Vec<f32>>,
    level: &AtomicU32,
    full: &AtomicBool,
) {
    if full.load(Ordering::Relaxed) {
        return;
    }
    let Ok(mut buf) = samples.try_lock() else { return };
    let channels = channels.max(1);
    let mut energy = 0.0f32;
    let mut n = 0usize;
    for frame in data.chunks(channels) {
        if buf.len() >= cap {
            full.store(true, Ordering::Relaxed);
            break;
        }
        let s = frame.first().copied().unwrap_or(0.0);
        buf.push(s);
        energy += s * s;
        n += 1;
    }
    if n > 0 {
        level.store((energy / n as f32).sqrt().to_bits(), Ordering::Relaxed);
    }
}

fn pick_input(host: &cpal::Host, want: Option<&str>) -> Result<cpal::Device, String> {
    if let Some(want) = want.filter(|s| !s.is_empty())
        && let Ok(devices) = host.input_devices()
    {
        for device in devices {
            if device_label(&device).is_some_and(|n| n == want || n.contains(want)) {
                return Ok(device);
            }
        }
    }
    host.default_input_device().ok_or_else(|| "No microphone found".into())
}

#[allow(deprecated)]
fn device_label(device: &cpal::Device) -> Option<String> {
    device.name().ok()
}

#[cfg(test)]
mod tests {
    use super::write_wav;

    #[test]
    fn wav_header_is_pcm16() {
        let path = std::env::temp_dir().join("dove-wav-test.wav");
        write_wav(&path, &[0.0, 0.5, -0.5], 16_000).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"RIFF"));
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(bytes.len(), 44 + 6);
        let _ = std::fs::remove_file(path);
    }
}
