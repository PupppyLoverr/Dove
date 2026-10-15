//! Jarvis mode: speak the assistant's finished turn.
//!
//! The composer toggle stores `jarvis=true`. When a live turn ends, the
//! transcript asks this module to speak. Audio comes from NEXI TTS on this
//! machine, or from Fish Audio when that backend and a key are set.

use std::process::{Command, Stdio};
use std::sync::Mutex;

use dove_core::Engine;

use crate::nexi;

/// Serializes playback so two replies cannot talk over each other.
static PLAYING: Mutex<()> = Mutex::new(());

/// Plain speech from a markdown reply. Code, links and images stay out.
pub fn for_speech(markdown: &str) -> String {
    let mut out = String::new();
    let mut code = false;
    for line in markdown.lines() {
        let trim = line.trim();
        if trim.starts_with("```") {
            code = !code;
            continue;
        }
        if code || trim.starts_with('|') || trim.starts_with("![") {
            continue;
        }
        let mut skip = false;
        let mut chars = trim.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '`' => skip = !skip,
                _ if skip => {}
                '*' | '_' | '#' | '>' => {}
                '[' => {
                    let rest: String = chars.clone().collect();
                    if let Some(end) = rest.find(']') {
                        out.push_str(&rest[..end]);
                        for _ in 0..=end {
                            chars.next();
                        }
                        if chars.peek() == Some(&'(') {
                            while chars.next().is_some_and(|c| c != ')') {}
                        }
                    }
                }
                _ => out.push(c),
            }
        }
        out.push(' ');
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut spoken: String = collapsed.chars().take(900).collect();
    if collapsed.chars().count() > 900 {
        spoken.push('…');
    }
    spoken
}

pub fn enabled(engine: &Engine) -> bool {
    engine.store.setting("jarvis").as_deref() == Some("true")
}

/// Speak `text` when Jarvis is on. Failures are printed; they do not block the chat.
pub fn maybe_speak(engine: &Engine, text: String) {
    if enabled(engine) {
        let prefs = prefs(engine);
        std::thread::spawn(move || {
            if let Err(e) = speak_with(&prefs, &text) {
                eprintln!("dove voice: {e}");
            }
        });
    }
}

/// Speak even when the toggle is off. Used by Settings → Voice → Test.
pub fn speak_now(engine: &Engine, text: String) -> Result<(), String> {
    speak_blocking(engine, &text)
}

struct Prefs {
    backend: String,
    voice: String,
    fish_key: String,
}

fn prefs(engine: &Engine) -> Prefs {
    Prefs {
        backend: engine.store.setting("tts_backend").unwrap_or_else(|| "nexi".into()),
        voice: engine.store.setting("tts_voice").filter(|v| !v.is_empty()).unwrap_or_else(|| "Jasper".into()),
        fish_key: engine
            .store
            .setting("fish_api_key")
            .filter(|k| !k.is_empty())
            .or_else(|| std::env::var("FISH_API_KEY").ok())
            .unwrap_or_default(),
    }
}

fn speak_blocking(engine: &Engine, text: &str) -> Result<(), String> {
    speak_with(&prefs(engine), text)
}

fn speak_with(prefs: &Prefs, text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    let dir = std::env::temp_dir().join("dove-voice");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let wav = dir.join(format!("jarvis-{}.wav", std::process::id()));
    if prefs.backend == "fish" {
        nexi::fish_wav(&prefs.fish_key, text, &wav)?;
    } else {
        nexi::synthesize_wav(text, &prefs.voice, &wav)?;
    }
    play_wav(&wav)
}

fn play_wav(path: &std::path::Path) -> Result<(), String> {
    let _guard = PLAYING.lock().map_err(|e| e.to_string())?;
    let mut cmd = if cfg!(windows) {
        let shown = path.display().to_string().replace('\'', "''");
        let script = format!("(New-Object System.Media.SoundPlayer '{shown}').PlaySync()");
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        cmd
    } else if cfg!(target_os = "macos") {
        let mut cmd = Command::new("afplay");
        cmd.arg(path);
        cmd
    } else {
        let program = if which("paplay") { "paplay" } else { "aplay" };
        let mut cmd = Command::new(program);
        cmd.arg(path);
        cmd
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000);
    }
    let status = cmd.stdout(Stdio::null()).stderr(Stdio::null()).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("Could not play audio ({status})")) }
}

fn which(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::for_speech;

    #[test]
    fn speech_strips_code_and_links() {
        let spoken = for_speech("Hello.\n```rust\nfn hidden() {}\n```\nSee [the docs](https://example.com). Done.");
        assert!(spoken.contains("Hello."));
        assert!(spoken.contains("the docs"));
        assert!(spoken.contains("Done."));
        assert!(!spoken.contains("hidden"));
        assert!(!spoken.contains("example.com"));
    }
}
