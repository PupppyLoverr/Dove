//! Text-to-speech engines.
//!
//! - [`System`]: the operating system's own voices (Windows speech
//!   synthesis, macOS `say`, Linux `espeak-ng`). No download, no network,
//!   a few megabytes of RAM; the default.
//! - [`OpenAiCompatible`]: a user-configured `/v1/audio/speech` endpoint.
//!   Off by default; the text is uploaded, and Settings says so.
//!
//! Every engine renders one clause at a time into PCM that Dove plays
//! itself, so barge-in can stop it mid-word and the echo canceller knows
//! exactly what was played.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::engines::{Pcm, SpeechOptions, TextToSpeech};

#[derive(Default)]
pub struct System {
    #[cfg(windows)]
    synth: Option<windows::Media::SpeechSynthesis::SpeechSynthesizer>,
}

// The WinRT synthesizer is agile (free-threaded); it is only ever used from
// the speech thread that owns this engine.
#[cfg(windows)]
unsafe impl Send for System {}

impl System {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(windows)]
impl System {
    fn synth(&mut self) -> Result<&windows::Media::SpeechSynthesis::SpeechSynthesizer> {
        if self.synth.is_none() {
            self.synth =
                Some(windows::Media::SpeechSynthesis::SpeechSynthesizer::new().context("Windows speech synthesis")?);
        }
        Ok(self.synth.as_ref().unwrap())
    }

    fn render(&mut self, text: &str, opts: &SpeechOptions) -> Result<Vec<u8>> {
        use windows::Media::SpeechSynthesis::SpeechSynthesizer;
        use windows::Storage::Streams::DataReader;
        use windows::core::HSTRING;
        let synth = self.synth()?.clone();
        if let Some(name) = &opts.voice
            && let Ok(all) = SpeechSynthesizer::AllVoices()
        {
            for v in all {
                if v.DisplayName().is_ok_and(|n| n.to_string_lossy() == *name) {
                    let _ = synth.SetVoice(&v);
                    break;
                }
            }
        }
        // SpeakingRate is 0.5-6.0, 1.0 the voice's default.
        if let Ok(o) = synth.Options() {
            let _ = o.SetSpeakingRate(f64::from(opts.rate.clamp(0.5, 3.0)));
        }
        let stream = synth.SynthesizeTextToStreamAsync(&HSTRING::from(text))?.get()?;
        let size = stream.Size()? as u32;
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
        reader.LoadAsync(size)?.get()?;
        let mut buf = vec![0u8; size as usize];
        reader.ReadBytes(&mut buf)?;
        Ok(buf)
    }
}

#[cfg(not(windows))]
impl System {
    fn render(&mut self, text: &str, opts: &SpeechOptions) -> Result<Vec<u8>> {
        // A spoken clause, never a shell string: arguments go straight to the program.
        let wpm = (175.0 * opts.rate.clamp(0.5, 3.0)).round() as u32;
        if cfg!(target_os = "macos") {
            let dir = std::env::temp_dir().join(format!("dove-say-{}", std::process::id()));
            std::fs::create_dir_all(&dir)?;
            let out = dir.join("clause.wav");
            let mut cmd = std::process::Command::new("/usr/bin/say");
            cmd.arg("-r").arg(wpm.to_string()).arg("--data-format=LEI16@22050").arg("-o").arg(&out);
            if let Some(v) = &opts.voice {
                cmd.arg("-v").arg(v);
            }
            let status = cmd.arg("--").arg(text).status().context("running say")?;
            if !status.success() {
                bail!("macOS speech synthesis failed");
            }
            let bytes = std::fs::read(&out)?;
            let _ = std::fs::remove_file(&out);
            Ok(bytes)
        } else {
            let program = ["espeak-ng", "espeak"]
                .into_iter()
                .find(|p| std::process::Command::new(p).arg("--version").output().is_ok())
                .context("No system voice found. Install espeak-ng, or choose another voice in Settings › Voice.")?;
            let mut cmd = std::process::Command::new(program);
            cmd.arg("--stdout").arg("-s").arg(wpm.to_string());
            if let Some(v) = &opts.voice {
                cmd.arg("-v").arg(v);
            }
            let out = cmd.arg("--").arg(text).output().context("running espeak-ng")?;
            if !out.status.success() {
                bail!("espeak-ng failed");
            }
            Ok(out.stdout)
        }
    }
}

impl TextToSpeech for System {
    fn name(&self) -> String {
        if cfg!(windows) {
            "Windows voice".into()
        } else if cfg!(target_os = "macos") {
            "macOS voice".into()
        } else {
            "espeak-ng".into()
        }
    }

    fn synthesize(&mut self, text: &str, opts: &SpeechOptions, cancel: &AtomicBool) -> Result<Pcm> {
        if cancel.load(Ordering::Acquire) || text.trim().is_empty() {
            return Ok(Pcm::default());
        }
        let bytes = self.render(text, opts)?;
        if cancel.load(Ordering::Acquire) {
            return Ok(Pcm::default());
        }
        crate::stt::decode_wav(&bytes)
    }

    fn voices(&self) -> Vec<String> {
        #[cfg(windows)]
        {
            windows::Media::SpeechSynthesis::SpeechSynthesizer::AllVoices()
                .map(|all| all.into_iter().filter_map(|v| v.DisplayName().ok().map(|n| n.to_string_lossy())).collect())
                .unwrap_or_default()
        }
        #[cfg(not(windows))]
        {
            Vec::new()
        }
    }
}

/// A user-configured OpenAI-compatible speech endpoint.
pub struct OpenAiCompatible {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub voice: String,
    client: reqwest::blocking::Client,
}

impl OpenAiCompatible {
    pub fn new(base_url: &str, api_key: &str, model: &str, voice: &str) -> Result<Self> {
        let base_url = base_url.trim_end_matches('/').to_string();
        if !(base_url.starts_with("https://")
            || base_url.starts_with("http://127.0.0.1")
            || base_url.starts_with("http://localhost"))
        {
            bail!("Use an https:// endpoint (plain http only for localhost).");
        }
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(30)).build()?;
        Ok(Self { base_url, api_key: api_key.into(), model: model.into(), voice: voice.into(), client })
    }
}

impl TextToSpeech for OpenAiCompatible {
    fn name(&self) -> String {
        format!("{} (cloud)", self.model)
    }

    fn synthesize(&mut self, text: &str, opts: &SpeechOptions, cancel: &AtomicBool) -> Result<Pcm> {
        if cancel.load(Ordering::Acquire) || text.trim().is_empty() {
            return Ok(Pcm::default());
        }
        let body = serde_json::json!({
            "model": self.model,
            "input": text,
            "voice": opts.voice.clone().unwrap_or_else(|| self.voice.clone()),
            "response_format": "wav",
            "speed": opts.rate.clamp(0.25, 4.0),
        });
        let resp = self
            .client
            .post(format!("{}/audio/speech", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .context("speech request")?;
        let status = resp.status();
        if !status.is_success() {
            bail!("The speech service answered {status}.");
        }
        let bytes = resp.bytes()?;
        if cancel.load(Ordering::Acquire) {
            return Ok(Pcm::default());
        }
        crate::stt::decode_wav(&bytes)
    }

    fn remote(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_or_empty_text_renders_nothing() {
        let mut s = System::new();
        assert_eq!(s.synthesize("hello", &SpeechOptions::default(), &AtomicBool::new(true)).unwrap(), Pcm::default());
        assert_eq!(s.synthesize("  ", &SpeechOptions::default(), &AtomicBool::new(false)).unwrap(), Pcm::default());
    }

    /// Needs a system voice; on CI machines without one, run with --ignored.
    #[test]
    #[ignore = "uses the operating system's speech synthesizer"]
    fn system_voice_renders_a_clause() {
        let mut s = System::new();
        let pcm = s.synthesize("Dove voice check.", &SpeechOptions::default(), &AtomicBool::new(false)).unwrap();
        assert!(pcm.rate >= 8000);
        assert!(pcm.duration_ms() > 300, "{} ms", pcm.duration_ms());
        assert!(pcm.samples.iter().any(|s| s.abs() > 0.05));
    }
}
