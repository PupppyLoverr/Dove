//! Streaming cloud voices: audio plays from the first chunk the service
//! sends, not after the whole clause has rendered.
//!
//! Every provider is asked for raw 16-bit mono PCM at [`crate::RATE`], so
//! nothing needs decoding and the echo canceller's reference stays exact.
//! Keys come from the environment (see [`Provider::from_env`]); none is ever
//! stored by Dove or written to a log. Each provider speaks with its own
//! voice setting; the system voice name a call carries is not sent to it.
//!
//! | Provider | Default model / voice | Why it's here |
//! | --- | --- | --- |
//! | [`FishAudio`] | `s2.1-pro-free` | Fish Audio's S2.1 Pro is free for developers through the API |
//! | [`Deepgram`] (Aura-2) | `aura-2-thalia-en` | lowest latency of the REST voices; $200 of credit on sign-up |
//! | [`Cartesia`] (Sonic) | `sonic-3.6` | built for real-time agents; 20k free credits a month (non-commercial) |
//! | [`Inworld`] | `inworld-tts-2`, Ashley | high quality per dollar |
//!
//! ElevenLabs streams through [`crate::tts::ElevenLabs`] with the same reader.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::engines::{Pcm, SpeechOptions, TextToSpeech};

/// The first chunk is handed over as soon as this much audio has arrived,
/// later ones in larger pieces (fewer, cheaper hand-offs to the player).
const FIRST_CHUNK_MS: u32 = 40;
const CHUNK_MS: u32 = 120;

/// What a streamed clause cost, for metrics and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StreamStats {
    pub chunks: usize,
    pub samples: usize,
    pub first_chunk_ms: Option<f32>,
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()?)
}

/// A failed response as one plain sentence, never echoing the key.
pub(crate) fn http_error(provider: &str, status: reqwest::StatusCode, body: &str) -> anyhow::Error {
    let detail: String = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            ["message", "detail", "error", "err_msg", "reason"].iter().find_map(|k| match &v[*k] {
                Value::String(s) => Some(s.clone()),
                Value::Object(o) => o.get("message").and_then(Value::as_str).map(str::to_string),
                _ => None,
            })
        })
        .unwrap_or_else(|| body.trim().chars().take(200).collect());
    let what = match status.as_u16() {
        401 | 403 => "rejected the API key",
        402 => "says the account is out of credits",
        429 => "is rate limiting requests",
        500..=599 => "is having trouble",
        _ => "refused the request",
    };
    if detail.is_empty() {
        anyhow::anyhow!("{provider} {what} ({status}).")
    } else {
        anyhow::anyhow!("{provider} {what} ({status}): {detail}")
    }
}

pub(crate) fn checked(provider: &str, resp: reqwest::blocking::Response) -> Result<reqwest::blocking::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let mut body = String::new();
    let _ = resp.take(4096).read_to_string(&mut body);
    Err(http_error(provider, status, &body))
}

/// Turns a byte stream of 16-bit little-endian PCM (optionally behind a WAV
/// header) into [`Pcm`] chunks.
#[derive(Debug)]
pub struct Pcm16Chunker {
    rate: u32,
    header: Vec<u8>,
    header_done: bool,
    odd: Option<u8>,
    pending: Vec<f32>,
    sent_first: bool,
}

impl Pcm16Chunker {
    pub fn new(rate: u32) -> Self {
        Self { rate, header: Vec::new(), header_done: false, odd: None, pending: Vec::new(), sent_first: false }
    }

    /// Feed bytes; returns chunks ready to play.
    pub fn push(&mut self, mut bytes: &[u8]) -> Vec<Pcm> {
        if !self.header_done {
            self.header.extend_from_slice(bytes);
            if self.header.len() < 12 {
                return Vec::new();
            }
            if &self.header[..4] != b"RIFF" {
                self.header_done = true;
                let all = std::mem::take(&mut self.header);
                return self.push_samples(&all);
            }
            // Walk the RIFF chunks to "data", reading the rate from "fmt ".
            let mut at = 12;
            loop {
                if self.header.len() < at + 8 {
                    return Vec::new();
                }
                let id = &self.header[at..at + 4];
                let size = u32::from_le_bytes(self.header[at + 4..at + 8].try_into().unwrap()) as usize;
                if id != b"data" && self.header.len() < at + 8 + size {
                    return Vec::new();
                }
                if id == b"fmt " && size >= 8 {
                    self.rate = u32::from_le_bytes(self.header[at + 12..at + 16].try_into().unwrap());
                }
                if id == b"data" {
                    let rest = self.header.split_off(at + 8);
                    self.header_done = true;
                    self.header.clear();
                    return self.push_samples(&rest);
                }
                at += 8 + size + (size & 1);
            }
        }
        let mut out_bytes = Vec::new();
        if let Some(b) = self.odd.take()
            && let Some((first, rest)) = bytes.split_first()
        {
            out_bytes.extend_from_slice(&[b, *first]);
            bytes = rest;
        }
        out_bytes.extend_from_slice(bytes);
        self.push_samples(&out_bytes)
    }

    fn push_samples(&mut self, bytes: &[u8]) -> Vec<Pcm> {
        let mut chunks = bytes.chunks_exact(2);
        for c in &mut chunks {
            self.pending.push(i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0);
        }
        if let [b] = chunks.remainder() {
            self.odd = Some(*b);
        }
        let ms = if self.sent_first { CHUNK_MS } else { FIRST_CHUNK_MS };
        let want = (self.rate as usize * ms as usize / 1000).max(1);
        if self.pending.len() >= want {
            self.sent_first = true;
            vec![Pcm { samples: std::mem::take(&mut self.pending), rate: self.rate }]
        } else {
            Vec::new()
        }
    }

    /// Whatever is left at the end of the stream.
    pub fn finish(&mut self) -> Option<Pcm> {
        (!self.pending.is_empty()).then(|| Pcm { samples: std::mem::take(&mut self.pending), rate: self.rate })
    }
}

/// Read `resp` to the end (or until `cancel`, or `out` says stop), handing
/// over PCM as it arrives.
pub fn stream_pcm16(
    mut resp: impl Read,
    rate: u32,
    cancel: &AtomicBool,
    out: &mut dyn FnMut(Pcm) -> bool,
) -> Result<StreamStats> {
    let started = std::time::Instant::now();
    let mut chunker = Pcm16Chunker::new(rate);
    let mut stats = StreamStats::default();
    let mut buf = vec![0u8; 8192];
    let mut emit = |pcm: Pcm, stats: &mut StreamStats| -> bool {
        stats.chunks += 1;
        stats.samples += pcm.samples.len();
        stats.first_chunk_ms.get_or_insert(started.elapsed().as_secs_f32() * 1000.0);
        out(pcm)
    };
    loop {
        if cancel.load(Ordering::Acquire) {
            return Ok(stats);
        }
        let n = match resp.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e).context("reading the audio stream"),
        };
        for pcm in chunker.push(&buf[..n]) {
            if cancel.load(Ordering::Acquire) || !emit(pcm, &mut stats) {
                return Ok(stats);
            }
        }
    }
    if let Some(pcm) = chunker.finish()
        && !cancel.load(Ordering::Acquire)
    {
        emit(pcm, &mut stats);
    }
    Ok(stats)
}

fn pick(value: &str, default: &str) -> String {
    if value.trim().is_empty() { default.to_string() } else { value.trim().to_string() }
}

fn need_key(key: &str, env: &str, provider: &str) -> Result<String> {
    if key.trim().is_empty() {
        bail!("Set {env} to use {provider}.");
    }
    Ok(key.trim().to_string())
}

/// Shared shape of the clause-at-a-time providers: build a request, stream it.
macro_rules! streaming_engine {
    ($ty:ty, $label:expr) => {
        impl TextToSpeech for $ty {
            fn name(&self) -> String {
                format!("{} {} (cloud)", $label, self.model_label())
            }

            fn synthesize(&mut self, text: &str, opts: &SpeechOptions, cancel: &AtomicBool) -> Result<Pcm> {
                let mut all = Pcm { samples: Vec::new(), rate: crate::RATE };
                self.synthesize_stream(text, opts, cancel, &mut |p| {
                    all.rate = p.rate;
                    all.samples.extend_from_slice(&p.samples);
                    true
                })?;
                Ok(all)
            }

            fn synthesize_stream(
                &mut self,
                text: &str,
                opts: &SpeechOptions,
                cancel: &AtomicBool,
                out: &mut dyn FnMut(Pcm) -> bool,
            ) -> Result<()> {
                if cancel.load(Ordering::Acquire) || text.trim().is_empty() {
                    return Ok(());
                }
                let resp = self.request(text, opts)?.send().with_context(|| format!("{} request", $label))?;
                let resp = checked($label, resp)?;
                self.read(resp, cancel, out)?;
                Ok(())
            }

            fn remote(&self) -> bool {
                true
            }
        }
    };
}

/// Fish Audio (`POST /v1/tts`, model in a header). With the default
/// `s2.1-pro-free` model the API costs nothing for developers.
pub struct FishAudio {
    pub api_key: String,
    pub model: String,
    /// A voice model id (`reference_id`); empty uses Fish Audio's default.
    pub voice: String,
    pub base_url: String,
    client: reqwest::blocking::Client,
}

impl FishAudio {
    pub const FREE_MODEL: &'static str = "s2.1-pro-free";

    pub fn new(api_key: &str, model: &str, voice: &str) -> Result<Self> {
        Ok(Self {
            api_key: need_key(api_key, "FISH_AUDIO_API_KEY", "Fish Audio")?,
            model: pick(model, Self::FREE_MODEL),
            voice: voice.trim().to_string(),
            base_url: "https://api.fish.audio".into(),
            client: client()?,
        })
    }

    pub fn body(&self, text: &str, opts: &SpeechOptions) -> Value {
        let mut body = json!({
            "text": text,
            "format": "pcm",
            "sample_rate": crate::RATE,
            // "balanced" trades a little quality for a much earlier first chunk.
            "latency": "balanced",
            "normalize": true,
            "prosody": { "speed": opts.rate.clamp(0.5, 2.0) },
        });
        if !self.voice.is_empty() {
            body["reference_id"] = json!(self.voice);
        }
        body
    }

    fn request(&self, text: &str, opts: &SpeechOptions) -> Result<reqwest::blocking::RequestBuilder> {
        Ok(self
            .client
            .post(format!("{}/v1/tts", self.base_url))
            .bearer_auth(&self.api_key)
            .header("model", &self.model)
            .json(&self.body(text, opts)))
    }

    fn read(
        &self,
        resp: reqwest::blocking::Response,
        cancel: &AtomicBool,
        out: &mut dyn FnMut(Pcm) -> bool,
    ) -> Result<StreamStats> {
        stream_pcm16(resp, crate::RATE, cancel, out)
    }

    fn model_label(&self) -> String {
        self.model.clone()
    }
}
streaming_engine!(FishAudio, "Fish Audio");

/// Deepgram Aura (`POST /v1/speak`); the voice is the model name.
pub struct Deepgram {
    pub api_key: String,
    /// `aura-2-thalia-en`, `aura-2-andromeda-en`, …
    pub voice: String,
    pub base_url: String,
    client: reqwest::blocking::Client,
}

impl Deepgram {
    pub const DEFAULT_VOICE: &'static str = "aura-2-thalia-en";

    pub fn new(api_key: &str, voice: &str) -> Result<Self> {
        Ok(Self {
            api_key: need_key(api_key, "DEEPGRAM_API_KEY", "Deepgram")?,
            voice: pick(voice, Self::DEFAULT_VOICE),
            base_url: "https://api.deepgram.com".into(),
            client: client()?,
        })
    }

    pub fn url(&self, opts: &SpeechOptions) -> String {
        let mut url = format!(
            "{}/v1/speak?model={}&encoding=linear16&sample_rate={}&container=none",
            self.base_url,
            self.voice,
            crate::RATE
        );
        // Speed isn't available for every voice; only ask when it matters.
        if (opts.rate - 1.0).abs() > 0.01 {
            url.push_str(&format!("&speed={:.2}", opts.rate.clamp(0.7, 1.5)));
        }
        url
    }

    fn request(&self, text: &str, opts: &SpeechOptions) -> Result<reqwest::blocking::RequestBuilder> {
        Ok(self
            .client
            .post(self.url(opts))
            .header("Authorization", format!("Token {}", self.api_key))
            .json(&json!({ "text": text })))
    }

    fn read(
        &self,
        resp: reqwest::blocking::Response,
        cancel: &AtomicBool,
        out: &mut dyn FnMut(Pcm) -> bool,
    ) -> Result<StreamStats> {
        stream_pcm16(resp, crate::RATE, cancel, out)
    }

    fn model_label(&self) -> String {
        self.voice.clone()
    }
}
streaming_engine!(Deepgram, "Deepgram");

/// Cartesia Sonic (`POST /tts/bytes`, raw PCM).
pub struct Cartesia {
    pub api_key: String,
    pub model: String,
    pub voice: String,
    pub base_url: String,
    client: reqwest::blocking::Client,
}

impl Cartesia {
    pub const VERSION: &'static str = "2026-08-14";
    pub const DEFAULT_MODEL: &'static str = "sonic-3.6";
    /// The voice in Cartesia's API reference example.
    pub const DEFAULT_VOICE: &'static str = "db6b0ed5-d5d3-463d-ae85-518a07d3c2b4";

    pub fn new(api_key: &str, model: &str, voice: &str) -> Result<Self> {
        Ok(Self {
            api_key: need_key(api_key, "CARTESIA_API_KEY", "Cartesia")?,
            model: pick(model, Self::DEFAULT_MODEL),
            voice: pick(voice, Self::DEFAULT_VOICE),
            base_url: "https://api.cartesia.ai".into(),
            client: client()?,
        })
    }

    pub fn body(&self, text: &str, opts: &SpeechOptions) -> Value {
        json!({
            "model_id": self.model,
            "transcript": text,
            "voice": { "id": self.voice },
            "output_format": { "container": "raw", "encoding": "pcm_s16le", "sample_rate": crate::RATE },
            "language": "en",
            "generation_config": { "speed": opts.rate.clamp(0.6, 1.5) },
        })
    }

    fn request(&self, text: &str, opts: &SpeechOptions) -> Result<reqwest::blocking::RequestBuilder> {
        Ok(self
            .client
            .post(format!("{}/tts/bytes", self.base_url))
            .bearer_auth(&self.api_key)
            .header("Cartesia-Version", Self::VERSION)
            .json(&self.body(text, opts)))
    }

    fn read(
        &self,
        resp: reqwest::blocking::Response,
        cancel: &AtomicBool,
        out: &mut dyn FnMut(Pcm) -> bool,
    ) -> Result<StreamStats> {
        stream_pcm16(resp, crate::RATE, cancel, out)
    }

    fn model_label(&self) -> String {
        self.model.clone()
    }
}
streaming_engine!(Cartesia, "Cartesia");

/// Inworld (`POST /tts/v1/voice:stream`): newline-delimited JSON, each line
/// carrying base64 audio.
pub struct Inworld {
    /// The portal's Base64 credential, sent as given.
    pub api_key: String,
    pub model: String,
    pub voice: String,
    pub base_url: String,
    client: reqwest::blocking::Client,
}

impl Inworld {
    pub const DEFAULT_MODEL: &'static str = "inworld-tts-2";
    pub const DEFAULT_VOICE: &'static str = "Ashley";

    pub fn new(api_key: &str, model: &str, voice: &str) -> Result<Self> {
        Ok(Self {
            api_key: need_key(api_key, "INWORLD_API_KEY", "Inworld")?,
            model: pick(model, Self::DEFAULT_MODEL),
            voice: pick(voice, Self::DEFAULT_VOICE),
            base_url: "https://api.inworld.ai".into(),
            client: client()?,
        })
    }

    pub fn body(&self, text: &str, opts: &SpeechOptions) -> Value {
        json!({
            "text": text,
            "voiceId": self.voice,
            "modelId": self.model,
            // PCM: raw samples, no WAV header in each chunk (LINEAR16 has one).
            "audioConfig": { "audioEncoding": "PCM", "sampleRateHertz": crate::RATE, "speakingRate": opts.rate.clamp(0.5, 1.5) },
        })
    }

    fn request(&self, text: &str, opts: &SpeechOptions) -> Result<reqwest::blocking::RequestBuilder> {
        Ok(self
            .client
            .post(format!("{}/tts/v1/voice:stream", self.base_url))
            .header("Authorization", format!("Basic {}", self.api_key))
            .json(&self.body(text, opts)))
    }

    fn read(
        &self,
        resp: reqwest::blocking::Response,
        cancel: &AtomicBool,
        out: &mut dyn FnMut(Pcm) -> bool,
    ) -> Result<StreamStats> {
        read_ndjson_audio(resp, cancel, out)
    }

    fn model_label(&self) -> String {
        self.model.clone()
    }
}
streaming_engine!(Inworld, "Inworld");

/// Inworld's stream: one JSON object per line, audio in `result.audioContent`.
/// A line with `error` ends the clause with that error, even after a 200.
pub fn read_ndjson_audio(
    resp: impl Read,
    cancel: &AtomicBool,
    out: &mut dyn FnMut(Pcm) -> bool,
) -> Result<StreamStats> {
    use base64::Engine as _;
    let started = std::time::Instant::now();
    let mut stats = StreamStats::default();
    let mut chunker = Pcm16Chunker::new(crate::RATE);
    let reader = std::io::BufReader::new(resp);
    for line in std::io::BufRead::lines(reader) {
        if cancel.load(Ordering::Acquire) {
            return Ok(stats);
        }
        let line = line.context("reading the audio stream")?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = serde_json::from_str(&line).context("Inworld sent a line that isn't JSON")?;
        if let Some(err) = msg.get("error") {
            let text = err.get("message").and_then(Value::as_str).unwrap_or("unknown error");
            bail!("Inworld stopped the stream: {text}");
        }
        let Some(audio) = msg.pointer("/result/audioContent").and_then(Value::as_str) else { continue };
        let bytes = base64::engine::general_purpose::STANDARD.decode(audio).context("Inworld audio isn't base64")?;
        for pcm in chunker.push(&bytes) {
            stats.chunks += 1;
            stats.samples += pcm.samples.len();
            stats.first_chunk_ms.get_or_insert(started.elapsed().as_secs_f32() * 1000.0);
            if !out(pcm) {
                return Ok(stats);
            }
        }
    }
    if let Some(pcm) = chunker.finish() {
        stats.chunks += 1;
        stats.samples += pcm.samples.len();
        stats.first_chunk_ms.get_or_insert(started.elapsed().as_secs_f32() * 1000.0);
        out(pcm);
    }
    Ok(stats)
}

/// Which streaming voice a call uses, with the key already read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provider {
    FishAudio { api_key: String, model: String, voice: String },
    Deepgram { api_key: String, voice: String },
    Cartesia { api_key: String, model: String, voice: String },
    Inworld { api_key: String, model: String, voice: String },
}

impl Provider {
    /// Ids used in settings, in the order Settings lists them.
    pub const IDS: &'static [(&'static str, &'static str)] = &[
        ("fish", "Fish Audio"),
        ("deepgram", "Deepgram Aura-2"),
        ("cartesia", "Cartesia Sonic"),
        ("inworld", "Inworld"),
    ];

    /// The environment variable(s) holding each provider's key, first match wins.
    pub fn key_vars(id: &str) -> &'static [&'static str] {
        match id {
            "fish" => &["FISH_AUDIO_API_KEY", "FISH_API_KEY"],
            "deepgram" => &["DEEPGRAM_API_KEY"],
            "cartesia" => &["CARTESIA_API_KEY"],
            "inworld" => &["INWORLD_API_KEY"],
            _ => &[],
        }
    }

    /// Whether a key for `id` is set (the value is never returned here).
    pub fn has_key(id: &str) -> bool {
        Self::key_vars(id).iter().any(|v| std::env::var(v).is_ok_and(|k| !k.trim().is_empty()))
    }

    /// A provider by settings id, reading its key from the environment.
    pub fn from_env(id: &str, model: &str, voice: &str) -> Option<Self> {
        let api_key = Self::key_vars(id).iter().find_map(|v| std::env::var(v).ok()).unwrap_or_default();
        let (model, voice) = (model.to_string(), voice.to_string());
        Some(match id {
            "fish" => Self::FishAudio { api_key, model, voice },
            "deepgram" => Self::Deepgram { api_key, voice },
            "cartesia" => Self::Cartesia { api_key, model, voice },
            "inworld" => Self::Inworld { api_key, model, voice },
            _ => return None,
        })
    }

    pub fn build(&self) -> Result<Box<dyn TextToSpeech>> {
        Ok(match self {
            Self::FishAudio { api_key, model, voice } => Box::new(FishAudio::new(api_key, model, voice)?),
            Self::Deepgram { api_key, voice } => Box::new(Deepgram::new(api_key, voice)?),
            Self::Cartesia { api_key, model, voice } => Box::new(Cartesia::new(api_key, model, voice)?),
            Self::Inworld { api_key, model, voice } => Box::new(Inworld::new(api_key, model, voice)?),
        })
    }
}

/// A cloud voice that falls back to the system voice when the service fails
/// (offline, out of credits, key revoked), so a call never goes mute. The
/// first failure is reported once through `on_error`.
pub struct WithFallback {
    pub primary: Box<dyn TextToSpeech>,
    backup: Option<Box<dyn TextToSpeech>>,
    make_backup: fn() -> Box<dyn TextToSpeech>,
    reported: bool,
    on_error: Box<dyn FnMut(String) + Send>,
}

impl WithFallback {
    pub fn new(primary: Box<dyn TextToSpeech>, on_error: Box<dyn FnMut(String) + Send>) -> Self {
        Self::with_backup(primary, || Box::new(crate::tts::System::new()), on_error)
    }

    pub fn with_backup(
        primary: Box<dyn TextToSpeech>,
        make_backup: fn() -> Box<dyn TextToSpeech>,
        on_error: Box<dyn FnMut(String) + Send>,
    ) -> Self {
        Self { primary, backup: None, make_backup, reported: false, on_error }
    }
}

impl TextToSpeech for WithFallback {
    fn name(&self) -> String {
        self.primary.name()
    }

    fn synthesize(&mut self, text: &str, opts: &SpeechOptions, cancel: &AtomicBool) -> Result<Pcm> {
        let mut all = Pcm { samples: Vec::new(), rate: crate::RATE };
        self.synthesize_stream(text, opts, cancel, &mut |p| {
            all.rate = p.rate;
            all.samples.extend_from_slice(&p.samples);
            true
        })?;
        Ok(all)
    }

    fn synthesize_stream(
        &mut self,
        text: &str,
        opts: &SpeechOptions,
        cancel: &AtomicBool,
        out: &mut dyn FnMut(Pcm) -> bool,
    ) -> Result<()> {
        let mut played = false;
        let first = self.primary.synthesize_stream(text, opts, cancel, &mut |p| {
            played = true;
            out(p)
        });
        match first {
            Ok(()) => Ok(()),
            // Half a clause already played: don't repeat it in another voice.
            Err(e) if played || cancel.load(Ordering::Acquire) => Err(e),
            Err(e) => {
                if !self.reported {
                    self.reported = true;
                    (self.on_error)(format!("{e:#} Speaking with the system voice instead."));
                }
                // The system voice's own options (its voice names differ).
                let opts = SpeechOptions { rate: opts.rate, voice: None };
                let make = self.make_backup;
                self.backup.get_or_insert_with(make).synthesize_stream(text, &opts, cancel, out)
            }
        }
    }

    fn voices(&self) -> Vec<String> {
        self.primary.voices()
    }

    fn remote(&self) -> bool {
        self.primary.remote()
    }
}

#[cfg(test)]
pub(crate) mod mock {
    //! A one-request HTTP server that records the request and replies with
    //! a chunked body, pausing between chunks like a real streaming voice.

    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    #[derive(Debug, Clone)]
    pub struct Seen {
        pub request_line: String,
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    impl Seen {
        pub fn header(&self, name: &str) -> Option<&str> {
            self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
        }
        pub fn json(&self) -> serde_json::Value {
            serde_json::from_str(&self.body).unwrap()
        }
    }

    /// Serve once: `status`, then `chunks` with `gap` between them.
    pub fn serve(status: u16, chunks: Vec<Vec<u8>>, gap: Duration) -> (String, mpsc::Receiver<Seen>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut headers = Vec::new();
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let line = line.trim_end().to_string();
                if line.is_empty() {
                    break;
                }
                if let Some((k, v)) = line.split_once(':') {
                    if k.eq_ignore_ascii_case("content-length") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap();
            let _ = tx.send(Seen {
                request_line: request_line.trim_end().into(),
                headers,
                body: String::from_utf8_lossy(&body).into(),
            });
            let mut out = stream;
            let reason = if status == 200 { "OK" } else { "Error" };
            let _ =
                write!(out, "HTTP/1.1 {status} {reason}\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n");
            for c in chunks {
                if write!(out, "{:x}\r\n", c.len()).is_err()
                    || out.write_all(&c).is_err()
                    || out.write_all(b"\r\n").is_err()
                    || out.flush().is_err()
                {
                    return;
                }
                std::thread::sleep(gap);
            }
            let _ = out.write_all(b"0\r\n\r\n");
        });
        (url, rx)
    }

    /// `n` samples of a 16-bit tone as little-endian bytes.
    pub fn tone(n: usize) -> Vec<u8> {
        (0..n).flat_map(|i| (((i as f32 * 0.1).sin() * 8000.0) as i16).to_le_bytes()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::mock::{serve, tone};
    use super::*;
    use std::time::{Duration, Instant};

    fn opts() -> SpeechOptions {
        SpeechOptions::default()
    }

    #[test]
    fn chunker_strips_wav_headers_across_reads_and_keeps_odd_bytes() {
        let pcm = crate::stt::wav_bytes(&[0.5, -0.5, 0.25], 24_000);
        let mut c = Pcm16Chunker::new(16_000);
        // Split in awkward places, including inside the header and a sample.
        let mut out = Vec::new();
        for piece in [&pcm[..7], &pcm[7..30], &pcm[30..45], &pcm[45..]] {
            out.extend(c.push(piece));
        }
        out.extend(c.finish());
        let samples: Vec<f32> = out.iter().flat_map(|p| p.samples.clone()).collect();
        assert_eq!(out[0].rate, 24_000, "rate read from the fmt chunk");
        assert_eq!(samples.len(), 3);
        assert!((samples[0] - 0.5).abs() < 0.001 && (samples[1] + 0.5).abs() < 0.001);

        let mut raw = Pcm16Chunker::new(16_000);
        let bytes = tone(2000);
        let mut n = 0;
        for piece in bytes.chunks(333) {
            n += raw.push(piece).iter().map(|p| p.samples.len()).sum::<usize>();
        }
        n += raw.finish().map_or(0, |p| p.samples.len());
        assert_eq!(n, 2000, "an odd split never drops a sample");
    }

    #[test]
    fn first_chunk_is_small_so_playback_starts_early() {
        let mut c = Pcm16Chunker::new(16_000);
        let first = c.push(&tone(700));
        assert_eq!(first.len(), 1, "40 ms (640 samples) is enough for the first chunk");
        assert!(c.push(&tone(700)).is_empty(), "later chunks wait for 120 ms");
    }

    #[test]
    fn fish_audio_streams_with_the_free_model_and_raw_pcm() {
        let chunks = vec![tone(1600), tone(1600), tone(1600)];
        let (url, seen) = serve(200, chunks, Duration::from_millis(150));
        let mut fish = FishAudio::new("fk_test", "", "").unwrap();
        fish.base_url = url;
        let t = Instant::now();
        let mut arrivals = Vec::new();
        let mut total = 0;
        fish.synthesize_stream(
            "Hello from Dove.",
            &SpeechOptions { rate: 1.2, voice: None },
            &AtomicBool::new(false),
            &mut |p| {
                arrivals.push(t.elapsed());
                total += p.samples.len();
                true
            },
        )
        .unwrap();
        assert_eq!(total, 4800);
        assert!(arrivals.len() >= 2);
        assert!(arrivals[0] < Duration::from_millis(250), "first audio before the stream ends: {:?}", arrivals[0]);
        let req = seen.recv().unwrap();
        assert_eq!(req.request_line, "POST /v1/tts HTTP/1.1");
        assert_eq!(req.header("authorization"), Some("Bearer fk_test"));
        assert_eq!(req.header("model"), Some(FishAudio::FREE_MODEL));
        let body = req.json();
        assert_eq!(body["format"], "pcm");
        assert_eq!(body["sample_rate"], 16_000);
        assert_eq!(body["latency"], "balanced");
        assert!((body["prosody"]["speed"].as_f64().unwrap() - 1.2).abs() < 0.01);
        assert!(body.get("reference_id").is_none(), "no voice set: Fish Audio's default");
    }

    #[test]
    fn deepgram_asks_for_raw_linear16_with_a_token() {
        let (url, seen) = serve(200, vec![tone(800)], Duration::ZERO);
        let mut dg = Deepgram::new("dg_test", "").unwrap();
        dg.base_url = url;
        let pcm = dg.synthesize("Hi.", &opts(), &AtomicBool::new(false)).unwrap();
        assert_eq!(pcm.samples.len(), 800);
        let req = seen.recv().unwrap();
        assert!(
            req.request_line.starts_with(
                "POST /v1/speak?model=aura-2-thalia-en&encoding=linear16&sample_rate=16000&container=none "
            )
        );
        assert_eq!(req.header("authorization"), Some("Token dg_test"));
        assert_eq!(req.json()["text"], "Hi.");
    }

    #[test]
    fn cartesia_sends_version_voice_and_raw_format() {
        let (url, seen) = serve(200, vec![tone(320)], Duration::ZERO);
        let mut c = Cartesia::new("sk_car_test", "", "").unwrap();
        c.base_url = url;
        c.synthesize("Hi.", &opts(), &AtomicBool::new(false)).unwrap();
        let req = seen.recv().unwrap();
        assert_eq!(req.request_line, "POST /tts/bytes HTTP/1.1");
        assert_eq!(req.header("cartesia-version"), Some(Cartesia::VERSION));
        assert_eq!(req.header("authorization"), Some("Bearer sk_car_test"));
        let body = req.json();
        assert_eq!(body["model_id"], Cartesia::DEFAULT_MODEL);
        assert_eq!(body["voice"]["id"], Cartesia::DEFAULT_VOICE);
        assert_eq!(body["output_format"]["encoding"], "pcm_s16le");
    }

    #[test]
    fn inworld_reads_ndjson_audio_and_surfaces_mid_stream_errors() {
        use base64::Engine as _;
        let b64 = |n| base64::engine::general_purpose::STANDARD.encode(tone(n));
        let lines = vec![
            format!("{{\"result\":{{\"audioContent\":\"{}\"}}}}\n", b64(1000)).into_bytes(),
            b"{\"result\":{\"timestampInfo\":{}}}\n".to_vec(),
            format!("{{\"result\":{{\"audioContent\":\"{}\"}}}}\n", b64(1000)).into_bytes(),
        ];
        let (url, seen) = serve(200, lines, Duration::from_millis(20));
        let mut iw = Inworld::new("aW53b3JsZA==", "", "").unwrap();
        iw.base_url = url;
        let pcm = iw.synthesize("Hi.", &opts(), &AtomicBool::new(false)).unwrap();
        assert_eq!(pcm.samples.len(), 2000);
        let req = seen.recv().unwrap();
        assert_eq!(req.header("authorization"), Some("Basic aW53b3JsZA=="));
        assert_eq!(req.json()["audioConfig"]["audioEncoding"], "PCM");

        let err = vec![b"{\"error\":{\"code\":8,\"message\":\"quota exceeded\"}}\n".to_vec()];
        let (url, _seen) = serve(200, err, Duration::ZERO);
        iw.base_url = url;
        let e = iw.synthesize("Hi.", &opts(), &AtomicBool::new(false)).unwrap_err();
        assert!(format!("{e}").contains("quota exceeded"), "{e}");
    }

    #[test]
    fn errors_name_the_problem_without_the_key() {
        let (url, _seen) =
            serve(402, vec![br#"{"status":402,"message":"Insufficient balance"}"#.to_vec()], Duration::ZERO);
        let mut fish = FishAudio::new("fk_secret_value", "", "").unwrap();
        fish.base_url = url;
        let e = format!("{:#}", fish.synthesize("Hi.", &opts(), &AtomicBool::new(false)).unwrap_err());
        assert!(e.contains("out of credits") && e.contains("Insufficient balance"), "{e}");
        assert!(!e.contains("fk_secret_value"));
        let (url, _seen) = serve(401, vec![b"unauthorized".to_vec()], Duration::ZERO);
        fish.base_url = url;
        let e = format!("{:#}", fish.synthesize("Hi.", &opts(), &AtomicBool::new(false)).unwrap_err());
        assert!(e.contains("rejected the API key"), "{e}");
    }

    #[test]
    fn cancel_and_stop_end_the_stream_early() {
        let chunks: Vec<Vec<u8>> = (0..20).map(|_| tone(1600)).collect();
        let (url, _seen) = serve(200, chunks, Duration::from_millis(50));
        let mut fish = FishAudio::new("k", "", "").unwrap();
        fish.base_url = url;
        let t = Instant::now();
        let mut got = 0;
        fish.synthesize_stream("A long clause.", &opts(), &AtomicBool::new(false), &mut |_| {
            got += 1;
            got < 2
        })
        .unwrap();
        assert_eq!(got, 2, "out() returning false stops reading");
        assert!(t.elapsed() < Duration::from_millis(600), "{:?}", t.elapsed());
        let cancelled = AtomicBool::new(true);
        let mut called = false;
        fish.synthesize_stream("x", &opts(), &cancelled, &mut |_| {
            called = true;
            true
        })
        .unwrap();
        assert!(!called, "a cancelled clause sends nothing");
    }

    #[test]
    fn keys_are_required_and_providers_build_from_settings_ids() {
        assert!(FishAudio::new("", "", "").is_err());
        assert!(format!("{}", Deepgram::new(" ", "").err().unwrap()).contains("DEEPGRAM_API_KEY"));
        for (id, _) in Provider::IDS {
            assert!(!Provider::key_vars(id).is_empty());
            let p = Provider::from_env(id, "", "").unwrap();
            let with_key = match p {
                Provider::FishAudio { model, voice, .. } => Provider::FishAudio { api_key: "k".into(), model, voice },
                Provider::Deepgram { voice, .. } => Provider::Deepgram { api_key: "k".into(), voice },
                Provider::Cartesia { model, voice, .. } => Provider::Cartesia { api_key: "k".into(), model, voice },
                Provider::Inworld { model, voice, .. } => Provider::Inworld { api_key: "k".into(), model, voice },
            };
            assert!(with_key.build().unwrap().remote());
        }
        assert!(Provider::from_env("nope", "", "").is_none());
    }

    struct Failing;
    impl TextToSpeech for Failing {
        fn name(&self) -> String {
            "failing".into()
        }
        fn synthesize(&mut self, _: &str, _: &SpeechOptions, _: &AtomicBool) -> Result<Pcm> {
            bail!("Fish Audio says the account is out of credits.")
        }
    }
    struct Beep;
    impl TextToSpeech for Beep {
        fn name(&self) -> String {
            "beep".into()
        }
        fn synthesize(&mut self, _: &str, _: &SpeechOptions, _: &AtomicBool) -> Result<Pcm> {
            Ok(Pcm { samples: vec![0.1; 160], rate: 16_000 })
        }
    }

    #[test]
    fn a_failing_cloud_voice_falls_back_and_reports_once() {
        let reports = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let r2 = reports.clone();
        let mut tts = WithFallback::with_backup(
            Box::new(Failing),
            || Box::new(Beep),
            Box::new(move |m| r2.lock().unwrap().push(m)),
        );
        for _ in 0..3 {
            let pcm = tts.synthesize("Hello.", &opts(), &AtomicBool::new(false)).unwrap();
            assert_eq!(pcm.samples.len(), 160, "the backup voice spoke");
        }
        let reports = reports.lock().unwrap();
        assert_eq!(reports.len(), 1, "reported once, not per clause");
        assert!(reports[0].contains("out of credits") && reports[0].contains("system voice"));
    }

    /// A live request against the real service, when its key is set:
    /// `FISH_AUDIO_API_KEY=… cargo test -p dove-voice live_ -- --ignored`
    #[test]
    #[ignore = "uses the network and a real API key"]
    fn live_providers_stream_the_first_chunk_quickly() {
        for (id, label) in Provider::IDS {
            if !Provider::has_key(id) {
                eprintln!("{label}: no key set, skipped");
                continue;
            }
            let mut tts = Provider::from_env(id, "", "").unwrap().build().unwrap();
            let t = Instant::now();
            let mut first = None;
            let mut n = 0;
            tts.synthesize_stream(
                "Dove here. The tests passed on Windows.",
                &opts(),
                &AtomicBool::new(false),
                &mut |p| {
                    first.get_or_insert(t.elapsed());
                    n += p.samples.len();
                    true
                },
            )
            .unwrap();
            eprintln!("{label}: first audio {:?}, {} ms of speech, total {:?}", first.unwrap(), n / 16, t.elapsed());
            assert!(n > 16_000 / 2);
        }
    }
}
