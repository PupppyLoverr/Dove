//! Pinned model files: where they come from, what they weigh, their licence,
//! and the SHA-256 they must match before ONNX Runtime ever sees them.
//!
//! Parakeet (speech-to-text) keeps its own manifest in `zeron-voice`; these
//! are the small models Voice Mode adds. Nothing downloads without the user
//! asking from Settings › Voice.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy)]
pub struct ModelFile {
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub licence: &'static str,
    pub purpose: &'static str,
}

/// Silero VAD v5, ONNX export (onnx-community/silero-vad at e71cae9).
pub const SILERO: ModelFile = ModelFile {
    id: "silero-vad",
    name: "silero-vad.onnx",
    url: "https://huggingface.co/onnx-community/silero-vad/resolve/e71cae966052b992a7eca6b17738916ce0eca4ec/onnx/model.onnx",
    size: 2_243_022,
    sha256: "a4a068cd6cf1ea8355b84327595838ca748ec29a25bc91fc82e6c299ccdc5808",
    licence: "MIT",
    purpose: "Hears when you start and stop talking",
};

/// Smart Turn v3.2 (CPU build), pipecat-ai/smart-turn-v3 at f766f81.
pub const SMART_TURN: ModelFile = ModelFile {
    id: "smart-turn",
    name: "smart-turn-v3.2-cpu.onnx",
    url: "https://huggingface.co/pipecat-ai/smart-turn-v3/resolve/f766f81d3cfdf7737ac64aad813d91bbfd56bf93/smart-turn-v3.2-cpu.onnx",
    size: 8_679_182,
    sha256: "2bb026316b14a660486a75b1733cd3fbab8c2fd0314dc9af7be49f8cca967e4f",
    licence: "BSD-2-Clause",
    purpose: "Tells a finished sentence from a pause mid-thought",
};

// Moonshine base (English), int8 ONNX export: onnx-community/moonshine-base-ONNX at b1e9b6a.

pub const MOONSHINE_ENCODER: ModelFile = ModelFile {
    id: "moonshine-encoder",
    name: "moonshine-base-encoder-int8.onnx",
    url: concat!(
        "https://huggingface.co/onnx-community/moonshine-base-ONNX/resolve/b1e9b6aae3c3c7298f10c3798393fdf38e8fbbad",
        "/onnx/encoder_model_int8.onnx"
    ),
    size: 20_488_801,
    sha256: "4edb38db96b52ddee5a3b25d2211bb6394f9a9a2d95f67bce0e8c861da018a4d",
    licence: "MIT",
    purpose: "Fast local speech recognition (encoder)",
};

pub const MOONSHINE_DECODER: ModelFile = ModelFile {
    id: "moonshine-decoder",
    name: "moonshine-base-decoder-merged-int8.onnx",
    url: concat!(
        "https://huggingface.co/onnx-community/moonshine-base-ONNX/resolve/b1e9b6aae3c3c7298f10c3798393fdf38e8fbbad",
        "/onnx/decoder_model_merged_int8.onnx"
    ),
    size: 42_427_261,
    sha256: "45febab0347ffbe1326459e092ae20a2cfda10ca33ab018b3c299247284de61b",
    licence: "MIT",
    purpose: "Fast local speech recognition (decoder)",
};

pub const MOONSHINE_TOKENIZER: ModelFile = ModelFile {
    id: "moonshine-tokenizer",
    name: "moonshine-tokenizer.json",
    url: concat!(
        "https://huggingface.co/onnx-community/moonshine-base-ONNX/resolve/b1e9b6aae3c3c7298f10c3798393fdf38e8fbbad",
        "/tokenizer.json"
    ),
    size: 3_761_754,
    sha256: "7b913404bdd039af4756783218af4440bc07fb7d6d8258d677e34f95b3ec416f",
    licence: "MIT",
    purpose: "Fast local speech recognition (vocabulary)",
};

// Moonshine tiny (English), int8: onnx-community/moonshine-tiny-ONNX at a6da124.
pub const MOONSHINE_TINY_ENCODER: ModelFile = ModelFile {
    id: "moonshine-tiny-encoder",
    name: "moonshine-tiny-encoder-int8.onnx",
    url: "https://huggingface.co/onnx-community/moonshine-tiny-ONNX/resolve/a6da1241cd305dcd64eab1edbd615f2bb9aabb95/onnx/encoder_model_int8.onnx",
    size: 7_919_835,
    sha256: "2981b85cfd6a4647ec37fe139e268cbc2c8a24102bf1fbb70dedeb7964995be8",
    licence: "MIT",
    purpose: "Fastest local speech recognition (encoder)",
};

pub const MOONSHINE_TINY_DECODER: ModelFile = ModelFile {
    id: "moonshine-tiny-decoder",
    name: "moonshine-tiny-decoder-merged-int8.onnx",
    url: "https://huggingface.co/onnx-community/moonshine-tiny-ONNX/resolve/a6da1241cd305dcd64eab1edbd615f2bb9aabb95/onnx/decoder_model_merged_int8.onnx",
    size: 20_189_512,
    sha256: "bd126e9e95c62b2236116f676f80877ddde488599631014149f92196cd8ae3a7",
    licence: "MIT",
    purpose: "Fastest local speech recognition (decoder)",
};

/// Moonshine's sizes. Both share one tokenizer (identical file upstream).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoonshineSize {
    /// 27M parameters, about 32 MB.
    Tiny,
    /// 61M parameters, about 67 MB.
    Base,
}

impl MoonshineSize {
    pub fn files(self) -> [ModelFile; 3] {
        match self {
            Self::Tiny => [MOONSHINE_TINY_ENCODER, MOONSHINE_TINY_DECODER, MOONSHINE_TOKENIZER],
            Self::Base => [MOONSHINE_ENCODER, MOONSHINE_DECODER, MOONSHINE_TOKENIZER],
        }
    }

    pub fn layers(self) -> usize {
        match self {
            Self::Tiny => 6,
            Self::Base => 8,
        }
    }

    pub fn head_dim(self) -> usize {
        match self {
            Self::Tiny => 36,
            Self::Base => 52,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Tiny => "moonshine-tiny",
            Self::Base => "moonshine-base",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "moonshine-tiny" => Some(Self::Tiny),
            "moonshine-base" => Some(Self::Base),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Tiny => "Moonshine tiny",
            Self::Base => "Moonshine base",
        }
    }
}

pub const ALL: &[ModelFile] = &[
    SILERO,
    SMART_TURN,
    MOONSHINE_ENCODER,
    MOONSHINE_DECODER,
    MOONSHINE_TINY_ENCODER,
    MOONSHINE_TINY_DECODER,
    MOONSHINE_TOKENIZER,
];

pub fn dir(root: &Path) -> PathBuf {
    root.join("voice").join("call")
}

pub fn path(root: &Path, m: &ModelFile) -> PathBuf {
    dir(root).join(m.name)
}

/// Size matches and a sidecar records the verified hash. Cheap: no hashing.
pub fn installed(root: &Path, m: &ModelFile) -> bool {
    let p = path(root, m);
    std::fs::metadata(&p).is_ok_and(|md| md.len() == m.size)
        && std::fs::read_to_string(p.with_extension("verified")).is_ok_and(|s| s.trim() == m.sha256)
}

fn sha256_file(p: &Path) -> Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(format!("{:x}", h.finalize()))
}

/// Hash the file again before a native runtime loads it.
pub fn verify(p: &Path, m: &ModelFile) -> Result<()> {
    let got = sha256_file(p).with_context(|| format!("reading {}", p.display()))?;
    if got != m.sha256 {
        bail!("{} is damaged (checksum mismatch). Remove it in Settings › Voice and download it again.", m.name);
    }
    Ok(())
}

/// Download `m` into `root`, verifying size and hash before it is renamed
/// into place. A cancelled or failed download leaves nothing behind.
pub fn download(root: &Path, m: &ModelFile, cancel: &AtomicBool, mut progress: impl FnMut(u64)) -> Result<()> {
    let dest = path(root, m);
    std::fs::create_dir_all(dest.parent().unwrap())?;
    let part = dest.with_extension("part");
    let result = (|| -> Result<()> {
        if cancel.load(Ordering::Acquire) {
            bail!("Download cancelled");
        }
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(600))
            .build()?;
        let mut resp = client.get(m.url).send()?.error_for_status()?;
        let mut out = std::fs::File::create(&part)?;
        let mut hash = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            if cancel.load(Ordering::Acquire) {
                bail!("Download cancelled");
            }
            let n = resp.read(&mut buf)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > m.size {
                bail!("{} is larger than expected", m.name);
            }
            hash.update(&buf[..n]);
            out.write_all(&buf[..n])?;
            progress(total);
        }
        if total != m.size || format!("{:x}", hash.finalize()) != m.sha256 {
            bail!("{} failed checksum verification", m.name);
        }
        out.sync_all()?;
        drop(out);
        std::fs::rename(&part, &dest)?;
        std::fs::write(dest.with_extension("verified"), m.sha256)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

pub fn remove(root: &Path, m: &ModelFile) {
    let p = path(root, m);
    let _ = std::fs::remove_file(p.with_extension("verified"));
    let _ = std::fs::remove_file(&p);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_well_formed() {
        for m in ALL {
            assert_eq!(m.sha256.len(), 64);
            assert!(m.url.starts_with("https://huggingface.co/"));
            assert!(m.url.contains("/resolve/") && !m.url.contains("/resolve/main/"), "{} must pin a revision", m.id);
            assert!(m.size > 0);
        }
    }

    #[test]
    fn a_wrong_file_is_neither_installed_nor_verified() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(dir.path(), &SILERO);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"not a model").unwrap();
        assert!(!installed(dir.path(), &SILERO));
        assert!(verify(&p, &SILERO).is_err());
        // A sidecar claiming verification does not help a wrong size.
        std::fs::write(p.with_extension("verified"), SILERO.sha256).unwrap();
        assert!(!installed(dir.path(), &SILERO));
    }

    #[test]
    fn a_cancelled_download_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let r = download(dir.path(), &SILERO, &AtomicBool::new(true), |_| {});
        assert!(r.is_err());
        assert!(!path(dir.path(), &SILERO).exists());
        assert!(!path(dir.path(), &SILERO).with_extension("part").exists());
    }
}
