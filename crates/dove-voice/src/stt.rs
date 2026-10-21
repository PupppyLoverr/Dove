//! Speech-to-text engines.
//!
//! - [`Moonshine`]: Moonshine base (MIT), int8, about 67 MB. The default:
//!   fast on short turns and light on memory. Runs on this machine.
//! - [`Parakeet`]: NVIDIA Parakeet TDT 0.6B v3 (int8) on this machine, the
//!   same model and download as hold-to-dictate: more accurate, about 700 MB
//!   of RAM while loaded. Nothing leaves the device.
//! - [`OpenAiCompatible`]: any `/v1/audio/transcriptions` endpoint the user
//!   configures (OpenAI, Groq, a local whisper server). Off by default; audio
//!   is uploaded, and Settings says so.

use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::engines::SpeechToText;

/// 16-bit WAV bytes of 16 kHz mono samples.
pub fn wav_bytes(samples: &[f32], rate: u32) -> Vec<u8> {
    let mut cur = std::io::Cursor::new(Vec::with_capacity(44 + samples.len() * 2));
    {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::new(&mut cur, spec).expect("in-memory wav");
        for s in samples {
            let _ = w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
        }
        let _ = w.finalize();
    }
    cur.into_inner()
}

/// Decode WAV bytes to mono f32.
pub fn decode_wav(bytes: &[u8]) -> Result<crate::engines::Pcm> {
    let mut r = hound::WavReader::new(std::io::Cursor::new(bytes)).context("reading WAV")?;
    let spec = r.spec();
    let ch = spec.channels.max(1) as usize;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().filter_map(Result::ok).collect(),
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample.saturating_sub(1))) as f32;
            r.samples::<i32>().filter_map(Result::ok).map(|s| s as f32 / scale).collect()
        }
    };
    let samples = raw.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect();
    Ok(crate::engines::Pcm { samples, rate: spec.sample_rate })
}

#[cfg(feature = "local")]
pub struct Parakeet {
    model: zeron_voice::Recognizer,
}

#[cfg(feature = "local")]
impl Parakeet {
    /// Load (and re-verify) the installed model. Takes a second or two.
    pub fn load(dir: &std::path::Path) -> Result<Self> {
        if !zeron_voice::installed(dir) {
            bail!("Local speech recognition is not installed. Install it in Settings › Voice.");
        }
        Ok(Self { model: zeron_voice::Recognizer::load(dir)? })
    }
}

#[cfg(feature = "local")]
impl SpeechToText for Parakeet {
    fn name(&self) -> String {
        "Parakeet TDT 0.6B v3".into()
    }

    fn partial(&mut self, audio: &[f32]) -> Result<String> {
        self.finish(audio)
    }

    fn finish(&mut self, audio: &[f32]) -> Result<String> {
        Ok(self.model.transcribe(audio.to_vec(), crate::RATE)?.trim().to_string())
    }
}

/// Moonshine base (English, MIT): a 61M-parameter encoder-decoder built for
/// live speech. Its compute scales with the clip length (no 30 s padding),
/// so a short turn transcribes in tens of milliseconds, and the int8 export
/// is about 67 MB on disk. The default local recognizer; Parakeet remains
/// the high-accuracy option.
#[cfg(feature = "local")]
pub struct Moonshine {
    encoder: ort::session::Session,
    decoder: ort::session::Session,
    /// Token id → the bytes it decodes to (special tokens decode to nothing).
    pieces: Vec<Vec<u8>>,
    size: crate::models::MoonshineSize,
    profile_on: bool,
    last_profile: (f32, usize, f32),
}

#[cfg(feature = "local")]
impl Moonshine {
    const HEADS: usize = 8;
    const BOS: i64 = 1;
    const EOS: i64 = 2;

    pub fn installed(root: &std::path::Path, size: crate::models::MoonshineSize) -> bool {
        size.files().iter().all(|m| crate::models::installed(root, m))
    }

    /// The tuning `voice_bench`'s stt_tune measured fastest here (see
    /// docs/VOICE.md); the call uses it.
    pub const ENCODER: crate::vad::OnnxTuning = crate::vad::OnnxTuning { threads: 4, spin: true, optimize: 3 };
    pub const DECODER: crate::vad::OnnxTuning = crate::vad::OnnxTuning { threads: 4, spin: true, optimize: 3 };

    pub fn load(root: &std::path::Path, size: crate::models::MoonshineSize) -> Result<Self> {
        Self::load_tuned(root, size, Self::ENCODER, Self::DECODER)
    }

    pub fn load_tuned(
        root: &std::path::Path,
        size: crate::models::MoonshineSize,
        enc: crate::vad::OnnxTuning,
        dec: crate::vad::OnnxTuning,
    ) -> Result<Self> {
        use crate::models::{path, verify};
        if !Self::installed(root, size) {
            bail!("{} is not installed. Download it in Settings › Voice.", size.label());
        }
        let [e, d, t] = size.files();
        // Verify and build both sessions at once: session creation is most of
        // a cold start, and the two graphs are independent.
        let (encoder, decoder) = std::thread::scope(|scope| {
            let enc = scope.spawn(|| {
                verify(&path(root, &e), &e)?;
                crate::vad::onnx_session_tuned(&path(root, &e), enc)
            });
            let dec = scope.spawn(|| {
                verify(&path(root, &d), &d)?;
                crate::vad::onnx_session_tuned(&path(root, &d), dec)
            });
            let encoder = enc.join().map_err(|_| anyhow::anyhow!("encoder load panicked"))?;
            let decoder = dec.join().map_err(|_| anyhow::anyhow!("decoder load panicked"))?;
            anyhow::Ok((encoder?, decoder?))
        })?;
        verify(&path(root, &t), &t)?;
        let pieces = Self::vocabulary(&std::fs::read_to_string(path(root, &t))?)?;
        Ok(Self { encoder, decoder, pieces, size, profile_on: false, last_profile: (0.0, 0, 0.0) })
    }

    /// SentencePiece BPE with byte fallback: `▁` is a space, `<0xNN>` a byte.
    fn vocabulary(json: &str) -> Result<Vec<Vec<u8>>> {
        let v: serde_json::Value = serde_json::from_str(json)?;
        let vocab = v.pointer("/model/vocab").and_then(|v| v.as_object()).context("tokenizer has no vocabulary")?;
        let mut pieces = vec![Vec::new(); vocab.len().max(32_768)];
        for (token, id) in vocab {
            let Some(id) = id.as_u64().map(|i| i as usize) else { continue };
            if id >= pieces.len() {
                pieces.resize(id + 1, Vec::new());
            }
            pieces[id] = if let Some(hex) = token.strip_prefix("<0x").and_then(|t| t.strip_suffix('>')) {
                u8::from_str_radix(hex, 16).map(|b| vec![b]).unwrap_or_default()
            } else if matches!(token.as_str(), "<unk>" | "<s>" | "</s>") {
                Vec::new()
            } else {
                token.replace('\u{2581}', " ").into_bytes()
            };
        }
        Ok(pieces)
    }

    pub fn decode(&self, ids: &[i64]) -> String {
        let bytes: Vec<u8> = ids.iter().filter_map(|&i| self.pieces.get(i as usize)).flatten().copied().collect();
        collapse_repeats(String::from_utf8_lossy(&bytes).trim())
    }

    /// Where the time goes: (encoder ms, decoder steps, decoder ms).
    pub fn profile(&mut self, audio: &[f32]) -> Result<(f32, usize, f32)> {
        self.profile_on = true;
        let r = self.transcribe(audio);
        self.profile_on = false;
        r.map(|_| self.last_profile)
    }

    fn transcribe(&mut self, audio: &[f32]) -> Result<String> {
        use ort::value::Tensor;
        let t_enc = std::time::Instant::now();
        let seconds = audio.len() as f32 / crate::RATE as f32;
        if seconds < 0.25 || audio.iter().all(|s| s.abs() < 1e-4) {
            return Ok(String::new());
        }
        let mut enc = self
            .encoder
            .run(ort::inputs!["input_values" => Tensor::from_array(([1usize, audio.len()], audio.to_vec()))?])?;
        let hidden = enc.remove("last_hidden_state").context("encoder output")?;
        drop(enc);
        let enc_ms = t_enc.elapsed().as_secs_f32() * 1000.0;
        let t_dec = std::time::Instant::now();
        // Moonshine's guidance: about 6.5 tokens per second of speech. The
        // cap counts speech, not the silence around it, so a short command
        // cannot run on into a loop.
        let active = active_seconds(audio);
        let max_tokens = ((active * 6.5).ceil() as usize + 2).clamp(3, 220);
        let mut run = 0usize;
        let (layers, head_dim) = (self.size.layers(), self.size.head_dim());
        let empty = |_: usize| Tensor::from_array(([1usize, Self::HEADS, 0, head_dim], Vec::<f32>::new()));
        let mut past: Vec<(String, ort::value::DynValue)> = Vec::new();
        let mut cross: Vec<(String, ort::value::DynValue)> = Vec::new();
        for l in 0..layers {
            for kind in ["decoder.key", "decoder.value"] {
                past.push((format!("past_key_values.{l}.{kind}"), empty(l)?.into_dyn()));
            }
            for kind in ["encoder.key", "encoder.value"] {
                cross.push((format!("past_key_values.{l}.{kind}"), empty(l)?.into_dyn()));
            }
        }
        let mut tokens: Vec<i64> = Vec::new();
        let mut last = Self::BOS;
        for step in 0..max_tokens {
            let mut inputs: Vec<(String, ort::session::SessionInputValue)> = Vec::with_capacity(past.len() + 3);
            inputs.push(("input_ids".into(), Tensor::from_array(([1usize, 1], vec![last]))?.into()));
            inputs.push(("encoder_hidden_states".into(), (&hidden).into()));
            inputs.push(("use_cache_branch".into(), Tensor::from_array(([1usize], vec![step > 0]))?.into()));
            for (name, value) in past.drain(..) {
                inputs.push((name, value.into()));
            }
            for (name, value) in &cross {
                inputs.push((name.clone(), value.into()));
            }
            let mut out = self.decoder.run(inputs)?;
            let (_, logits) = out["logits"].try_extract_tensor::<f32>()?;
            let next =
                logits.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i as i64).unwrap_or(Self::EOS);
            // The decoder's self-attention cache grows every step. The
            // cross-attention cache over the audio is computed on the first
            // step only; later steps return placeholders for it, so the
            // first step's tensors are kept.
            if step == 0 {
                cross.clear();
                for l in 0..layers {
                    for kind in ["encoder.key", "encoder.value"] {
                        let v = out.remove(format!("present.{l}.{kind}")).context("cross-attention cache")?;
                        cross.push((format!("past_key_values.{l}.{kind}"), v));
                    }
                }
            }
            for l in 0..layers {
                for kind in ["decoder.key", "decoder.value"] {
                    let v = out.remove(format!("present.{l}.{kind}")).context("decoder cache output")?;
                    past.push((format!("past_key_values.{l}.{kind}"), v));
                }
            }
            if next == Self::EOS {
                break;
            }
            // A third identical token in a row is the decoder looping
            // ("stop stop stop"), not speech.
            run = if next == last { run + 1 } else { 0 };
            if run >= 2 {
                tokens.pop();
                break;
            }
            tokens.push(next);
            last = next;
        }
        if self.profile_on {
            self.last_profile = (enc_ms, tokens.len() + 1, t_dec.elapsed().as_secs_f32() * 1000.0);
        }
        let text = self.decode(&tokens);
        // A short command with a word said twice in a row ("Stop stop") is
        // the decoder repeating itself far more often than the user.
        Ok(if active < 1.0 { collapse_doubles(&text) } else { text })
    }
}

#[cfg(feature = "local")]
impl SpeechToText for Moonshine {
    fn name(&self) -> String {
        self.size.label().into()
    }

    fn partial(&mut self, audio: &[f32]) -> Result<String> {
        self.transcribe(audio)
    }

    fn finish(&mut self, audio: &[f32]) -> Result<String> {
        self.transcribe(audio)
    }
}

/// "stop stop stop stop" → "stop stop": a word repeated more than twice in
/// a row is a decoding loop, and two is as many as anyone says on purpose.
pub fn collapse_repeats(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for w in text.split_whitespace() {
        let key = |s: &str| s.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        let n = out.len();
        if n >= 2 && key(out[n - 1]) == key(w) && key(out[n - 2]) == key(w) {
            continue;
        }
        out.push(w);
    }
    out.join(" ")
}

/// Seconds of `audio` (16 kHz) that carry sound: 10 ms windows louder than a
/// tenth of the loudest.
pub fn active_seconds(audio: &[f32]) -> f32 {
    let windows: Vec<f32> = audio.chunks(160).map(crate::vad::rms).collect();
    let peak = windows.iter().cloned().fold(0.0f32, f32::max);
    if peak <= 0.0 {
        return 0.0;
    }
    windows.iter().filter(|w| **w > peak * 0.1).count() as f32 * 0.01
}

/// "Stop stop" → "Stop": immediate word repeats removed (case-insensitive).
pub fn collapse_doubles(text: &str) -> String {
    let key = |s: &str| s.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    let mut out: Vec<&str> = Vec::new();
    for w in text.split_whitespace() {
        if out.last().is_some_and(|p| key(p) == key(w) && !key(w).is_empty()) {
            continue;
        }
        out.push(w);
    }
    out.join(" ")
}

/// A user-configured OpenAI-compatible transcription endpoint.
pub struct OpenAiCompatible {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    client: reqwest::blocking::Client,
}

impl OpenAiCompatible {
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Result<Self> {
        let base_url = base_url.trim_end_matches('/').to_string();
        if !(base_url.starts_with("https://")
            || base_url.starts_with("http://127.0.0.1")
            || base_url.starts_with("http://localhost"))
        {
            bail!("Use an https:// endpoint (plain http only for localhost).");
        }
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(30)).build()?;
        Ok(Self { base_url, api_key: api_key.to_string(), model: model.to_string(), client })
    }
}

impl SpeechToText for OpenAiCompatible {
    fn name(&self) -> String {
        format!("{} (cloud)", self.model)
    }

    fn partial(&mut self, _audio: &[f32]) -> Result<String> {
        // Partials would upload the same audio over and over.
        Ok(String::new())
    }

    fn finish(&mut self, audio: &[f32]) -> Result<String> {
        let part = reqwest::blocking::multipart::Part::bytes(wav_bytes(audio, crate::RATE))
            .file_name("turn.wav")
            .mime_str("audio/wav")?;
        let form = reqwest::blocking::multipart::Form::new().text("model", self.model.clone()).part("file", part);
        let resp = self
            .client
            .post(format!("{}/audio/transcriptions", self.base_url))
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .context("transcription request")?;
        let status = resp.status();
        if !status.is_success() {
            bail!("The transcription service answered {status}.");
        }
        let v: serde_json::Value = resp.json()?;
        Ok(v.get("text").and_then(|t| t.as_str()).unwrap_or_default().trim().to_string())
    }

    fn remote(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_round_trips() {
        let s: Vec<f32> = (0..1600).map(|i| ((i as f32) / 100.0).sin() * 0.5).collect();
        let pcm = decode_wav(&wav_bytes(&s, 16_000)).unwrap();
        assert_eq!(pcm.rate, 16_000);
        assert_eq!(pcm.samples.len(), s.len());
        assert!(pcm.samples.iter().zip(&s).all(|(a, b)| (a - b).abs() < 1e-3));
    }

    #[test]
    fn decoding_loops_collapse() {
        assert_eq!(collapse_repeats("Stop stop stop stop stop"), "Stop stop");
        assert_eq!(collapse_repeats("Stop."), "Stop.");
        assert_eq!(collapse_repeats("no no, I said no"), "no no, I said no");
        assert_eq!(collapse_repeats("go go go now"), "go go now");
        assert_eq!(collapse_doubles("Stop stop"), "Stop");
        assert_eq!(collapse_doubles("Stop. stop"), "Stop.");
        assert_eq!(collapse_doubles("Approve it"), "Approve it");
    }

    #[test]
    fn active_speech_ignores_the_silence_around_it() {
        let mut a = vec![0.0005f32; 16_000];
        a.extend((0..5_600).map(|i| (i as f32 * 0.05).sin() * 0.3));
        a.extend(vec![0.0005f32; 8_000]);
        let s = active_seconds(&a);
        assert!((0.3..0.4).contains(&s), "{s}");
    }

    #[test]
    fn cloud_endpoints_must_be_https_or_loopback() {
        assert!(OpenAiCompatible::new("http://example.com/v1", "k", "whisper-1").is_err());
        assert!(OpenAiCompatible::new("https://api.openai.com/v1/", "k", "whisper-1").is_ok());
        assert!(OpenAiCompatible::new("http://127.0.0.1:8080/v1", "", "whisper").is_ok());
    }
}
