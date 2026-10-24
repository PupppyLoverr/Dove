//! ONNX Runtime as a download, not a link. `ort` is built with
//! `load-dynamic`, so the app binary carries no inference engine; the
//! official Microsoft build is fetched next to the voice models, only when
//! the user downloads one, and checked against a pinned SHA-256 twice: the
//! release archive as it streams in, and the library before every load.
//!
//! Layout: `<voice root>/onnxruntime/<library>` plus a `.verified` sidecar,
//! where the voice root is the folder that holds each model folder
//! (`~/.dove/voice`).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

/// One platform's pinned ONNX Runtime release (microsoft/onnxruntime v1.28.0,
/// the version `ort` 2.0.0-rc.13 is built for).
#[derive(Debug, Clone, Copy)]
pub struct Pin {
    pub url: &'static str,
    /// Release archive size and SHA-256 (as GitHub publishes them).
    pub size: u64,
    pub sha256: &'static str,
    /// Path of the library inside the archive.
    pub member: &'static str,
    /// The library's own size and SHA-256.
    pub lib_size: u64,
    pub lib_sha256: &'static str,
    /// File name on disk.
    pub file: &'static str,
}

const BASE: &str = "https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/";

#[cfg(all(windows, target_arch = "x86_64"))]
pub const PIN: Option<Pin> = Some(Pin {
    url: "https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/onnxruntime-win-x64-1.28.0.zip",
    size: 78_796_801,
    sha256: "abef733dacbe2f571547a7150b479b5cb9cc0df22f96c24983a42cadb1b4f8bc",
    member: "onnxruntime-win-x64-1.28.0/lib/onnxruntime.dll",
    lib_size: 15_809_848,
    lib_sha256: "18370c375f07357fa5874344a9d9ac17e6b6fe1eb18b1dd209d79483b4470257",
    file: "onnxruntime.dll",
});

#[cfg(all(windows, target_arch = "aarch64"))]
pub const PIN: Option<Pin> = Some(Pin {
    url: "https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/onnxruntime-win-arm64-1.28.0.zip",
    size: 79_812_663,
    sha256: "cbe4547463ece092b505c3581376ed5896d22b5429f39d5e645e425ecdd369ad",
    member: "onnxruntime-win-arm64-1.28.0/lib/onnxruntime.dll",
    lib_size: 15_895_904,
    lib_sha256: "102342326311ad62879b808840b4e3fd1372c41d8c871b59536d89a3c64d53f2",
    file: "onnxruntime.dll",
});

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const PIN: Option<Pin> = Some(Pin {
    url: "https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/onnxruntime-osx-arm64-1.28.0.tgz",
    size: 32_396_562,
    sha256: "1268b359718099bde2cedb55787f182a130067bc4f31e8c88478c445b850d3d8",
    member: "onnxruntime-osx-arm64-1.28.0/lib/libonnxruntime.1.28.0.dylib",
    lib_size: 39_312_136,
    lib_sha256: "dc19bbcb2f5c9fb3c68b4f9248aa0a35065ff702c5dbeae75eac54a74da97b6d",
    file: "libonnxruntime.1.28.0.dylib",
});

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const PIN: Option<Pin> = Some(Pin {
    url: "https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/onnxruntime-linux-x64-1.28.0.tgz",
    size: 9_125_960,
    sha256: "a3e1b79d7bb1bf09696ce675f49e4064e6c81f6202b8225624fff0e93f8d6407",
    member: "onnxruntime-linux-x64-1.28.0/lib/libonnxruntime.so.1.28.0",
    lib_size: 24_268_848,
    lib_sha256: "1461ef7cc3d9e49982591721683cc3e3a55580aeca9a5254e7aac47b75ee4bab",
    file: "libonnxruntime.so.1.28.0",
});

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
pub const PIN: Option<Pin> = Some(Pin {
    url: "https://github.com/microsoft/onnxruntime/releases/download/v1.28.0/onnxruntime-linux-aarch64-1.28.0.tgz",
    size: 8_116_278,
    sha256: "e15ff8b5d85afe6c144d97c6fd432254bf76a219daaf17658087d6ecb3e8f0bb",
    member: "onnxruntime-linux-aarch64-1.28.0/lib/libonnxruntime.so.1.28.0",
    lib_size: 20_591_712,
    lib_sha256: "f1ec1a08eb99bd6e5401340f0a2b101381bf4694415480291dc13bcaa30f9ec7",
    file: "libonnxruntime.so.1.28.0",
});

#[cfg(not(any(
    all(windows, any(target_arch = "x86_64", target_arch = "aarch64")),
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")),
)))]
pub const PIN: Option<Pin> = None;

fn pin() -> Result<Pin> {
    PIN.context("Local voice is not available on this platform")
}

/// The runtime folder under a voice root.
pub fn dir(voice_root: &Path) -> PathBuf {
    voice_root.join("onnxruntime")
}

/// The runtime folder for a model folder (`<voice root>/<model>`) or a model
/// file in one: both share the voice root's runtime.
pub fn beside(model_dir: &Path) -> PathBuf {
    dir(model_dir.parent().unwrap_or(model_dir))
}

pub fn path(dir: &Path) -> Option<PathBuf> {
    PIN.map(|p| dir.join(p.file))
}

/// Size matches and a sidecar records the verified hash. Cheap: no hashing.
pub fn installed(dir: &Path) -> bool {
    let Some(p) = PIN else { return false };
    let lib = dir.join(p.file);
    std::fs::metadata(&lib).is_ok_and(|m| m.len() == p.lib_size)
        && std::fs::read_to_string(lib.with_extension("verified")).is_ok_and(|s| s.trim() == p.lib_sha256)
}

/// Bytes still to download for the runtime (0 once installed).
pub fn download_size(dir: &Path) -> u64 {
    match PIN {
        Some(p) if !installed(dir) => p.size,
        _ => 0,
    }
}

fn sha256_file(p: &Path) -> Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(format!("{:x}", h.finalize()))
}

/// One download at a time: two model buttons pressed together share it.
static DOWNLOAD: Mutex<()> = Mutex::new(());

/// Download the pinned release, verify it, extract the library, verify that,
/// and only then move it into place. Called only from a model download the
/// user started, on a worker. A no-op when already installed.
pub fn download(dir: &Path, cancel: &AtomicBool, mut progress: impl FnMut(u64)) -> Result<()> {
    let _one = DOWNLOAD.lock().unwrap_or_else(|e| e.into_inner());
    if installed(dir) {
        return Ok(());
    }
    let p = pin()?;
    if cancel.load(Ordering::Acquire) {
        bail!("Download cancelled");
    }
    std::fs::create_dir_all(dir)?;
    let archive = dir.join("archive.part");
    let lib = dir.join(p.file);
    let staged = lib.with_extension("part");
    let result = (|| -> Result<()> {
        fetch(&p, &archive, cancel, &mut progress)?;
        extract(&p, &archive, &staged)?;
        let got = sha256_file(&staged)?;
        if std::fs::metadata(&staged)?.len() != p.lib_size || got != p.lib_sha256 {
            bail!("ONNX Runtime failed checksum verification");
        }
        std::fs::rename(&staged, &lib)?;
        std::fs::write(lib.with_extension("verified"), p.lib_sha256)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(&archive);
    if result.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    result
}

fn fetch(p: &Pin, dest: &Path, cancel: &AtomicBool, progress: &mut impl FnMut(u64)) -> Result<()> {
    debug_assert!(p.url.starts_with(BASE));
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        let client = reqwest::Client::builder().connect_timeout(Duration::from_secs(20)).build()?;
        let mut response = tokio::select! {
            _ = super::cancelled(cancel) => bail!("Download cancelled"),
            r = tokio::time::timeout(Duration::from_secs(30), client.get(p.url).send()) => r??.error_for_status()?,
        };
        let mut out = std::fs::File::create(dest)?;
        let mut hash = Sha256::new();
        let mut size = 0u64;
        loop {
            let chunk = tokio::select! {
                _ = super::cancelled(cancel) => bail!("Download cancelled"),
                c = tokio::time::timeout(Duration::from_secs(30), response.chunk()) => c??,
            };
            let Some(chunk) = chunk else { break };
            size += chunk.len() as u64;
            if size > p.size {
                bail!("Unexpected ONNX Runtime size");
            }
            hash.update(&chunk);
            out.write_all(&chunk)?;
            progress(size);
        }
        if size != p.size || format!("{:x}", hash.finalize()) != p.sha256 {
            bail!("ONNX Runtime failed checksum verification");
        }
        out.sync_all()?;
        Ok(())
    })
}

#[cfg(windows)]
fn extract(p: &Pin, archive: &Path, dest: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
    let mut entry = zip.by_name(p.member).context("ONNX Runtime archive is missing its library")?;
    let mut out = std::fs::File::create(dest)?;
    std::io::copy(&mut (&mut entry).take(p.lib_size + 1), &mut out)?;
    out.sync_all()?;
    Ok(())
}

#[cfg(not(windows))]
fn extract(p: &Pin, archive: &Path, dest: &Path) -> Result<()> {
    let gz = flate2::read::GzDecoder::new(std::io::BufReader::new(std::fs::File::open(archive)?));
    let mut out = std::fs::File::create(dest)?;
    tar_member(gz, p.member, p.lib_size, &mut out)?;
    out.sync_all()?;
    Ok(())
}

/// Copy one regular file out of a tar stream. Enough of ustar and GNU tar
/// for a release archive: long names, pax headers and links are skipped.
#[cfg_attr(windows, allow(dead_code))]
fn tar_member(mut tar: impl Read, member: &str, limit: u64, out: &mut impl Write) -> Result<()> {
    let mut header = [0u8; 512];
    loop {
        tar.read_exact(&mut header).context("ONNX Runtime archive is missing its library")?;
        if header.iter().all(|&b| b == 0) {
            bail!("ONNX Runtime archive is missing its library");
        }
        let field = |r: std::ops::Range<usize>| {
            let f = &header[r];
            String::from_utf8_lossy(&f[..f.iter().position(|&b| b == 0).unwrap_or(f.len())]).into_owned()
        };
        let size = u64::from_str_radix(field(124..136).trim(), 8).context("damaged ONNX Runtime archive")?;
        let prefix = if &header[257..262] == b"ustar" { field(345..500) } else { String::new() };
        let name = if prefix.is_empty() { field(0..100) } else { format!("{prefix}/{}", field(0..100)) };
        let regular = matches!(header[156], b'0' | 0);
        let mut body = (&mut tar).take(size.div_ceil(512) * 512);
        if regular && name.trim_start_matches("./") == member {
            if size > limit {
                bail!("Unexpected ONNX Runtime size");
            }
            std::io::copy(&mut (&mut body).take(size), out)?;
            return Ok(());
        }
        std::io::copy(&mut body, &mut std::io::sink())?;
    }
}

/// Point `ort` at the downloaded library, once per process. The library is
/// hashed again first, so a file changed on disk never reaches the loader.
pub fn load(dir: &Path) -> Result<()> {
    static LOADED: Mutex<bool> = Mutex::new(false);
    let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    if *loaded {
        return Ok(());
    }
    let p = pin()?;
    let lib = dir.join(p.file);
    if !installed(dir) {
        bail!("The voice runtime is not installed. Download a voice model in Settings › Voice.");
    }
    if sha256_file(&lib).with_context(|| format!("reading {}", lib.display()))? != p.lib_sha256 {
        bail!("The voice runtime is damaged. Remove the voice models in Settings and download them again.");
    }
    let _ = ort::init_from(&lib).map_err(|e| anyhow::anyhow!("loading ONNX Runtime: {e}"))?.commit();
    *loaded = true;
    Ok(())
}

/// Delete the runtime once no model under `voice_root` needs it. A library
/// this process has loaded cannot be deleted on Windows; it then stays, still
/// verified, and goes on a later removal.
pub fn remove_if_unused(voice_root: &Path) {
    let rt = dir(voice_root);
    let mut models = std::fs::read_dir(voice_root).into_iter().flatten().flatten().filter(|e| e.path() != rt);
    let used = models.any(|d| {
        std::fs::read_dir(d.path())
            .into_iter()
            .flatten()
            .flatten()
            .any(|f| f.path().extension().is_some_and(|x| x == "onnx"))
    });
    if used {
        return;
    }
    if let Some(p) = PIN {
        let lib = rt.join(p.file);
        if std::fs::remove_file(&lib).is_ok() || !lib.exists() {
            let _ = std::fs::remove_file(lib.with_extension("verified"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_well_formed() {
        if let Some(p) = PIN {
            assert!(p.url.starts_with(BASE));
            assert!(p.member.ends_with(p.file));
            assert_eq!(p.sha256.len(), 64);
            assert_eq!(p.lib_sha256.len(), 64);
        }
    }

    #[test]
    fn a_wrong_library_is_neither_installed_nor_loaded() {
        let Some(p) = PIN else { return };
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join(p.file);
        std::fs::write(&lib, b"not a library").unwrap();
        std::fs::write(lib.with_extension("verified"), p.lib_sha256).unwrap();
        assert!(!installed(dir.path()));
        assert!(load(dir.path()).is_err());
    }

    #[test]
    fn a_cancelled_download_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let rt = dir.path().join("onnxruntime");
        assert!(download(&rt, &AtomicBool::new(true), |_| panic!("no progress after cancellation")).is_err());
        assert!(!installed(&rt));
        assert!(!rt.exists());
    }

    /// Network: `ZERON_VOICE_RUNTIME=<dir> cargo test -p zeron-voice -- --ignored`
    /// downloads the pinned release into `<dir>`, verifies it and loads it.
    #[test]
    #[ignore]
    fn downloads_verifies_and_loads_the_pinned_runtime() {
        let dir = PathBuf::from(std::env::var_os("ZERON_VOICE_RUNTIME").expect("set ZERON_VOICE_RUNTIME"));
        let mut seen = 0;
        download(&dir, &AtomicBool::new(false), |n| seen = n).unwrap();
        assert!(installed(&dir));
        assert!(seen == 0 || seen == PIN.unwrap().size, "progress ends at the archive size");
        assert!(!dir.join("archive.part").exists());
        load(&dir).unwrap();
        let _ = ort::session::Session::builder().unwrap();
    }

    #[test]
    fn tar_reader_finds_one_member() {
        fn entry(tar: &mut Vec<u8>, name: &str, kind: u8, body: &[u8]) {
            let mut h = [0u8; 512];
            h[..name.len()].copy_from_slice(name.as_bytes());
            h[124..135].copy_from_slice(format!("{:011o}", body.len()).as_bytes());
            h[156] = kind;
            h[257..262].copy_from_slice(b"ustar");
            tar.extend_from_slice(&h);
            tar.extend_from_slice(body);
            tar.resize(tar.len().div_ceil(512) * 512, 0);
        }
        let mut tar = Vec::new();
        entry(&mut tar, "r/lib/libx.so", b'2', b"");
        entry(&mut tar, "r/lib/other", b'0', &[7; 700]);
        entry(&mut tar, "r/lib/libx.so.1", b'0', b"payload");
        tar.extend_from_slice(&[0; 1024]);
        let mut out = Vec::new();
        tar_member(&tar[..], "r/lib/libx.so.1", 100, &mut out).unwrap();
        assert_eq!(out, b"payload");
        assert!(tar_member(&tar[..], "r/lib/missing", 100, &mut Vec::new()).is_err());
        assert!(tar_member(&tar[..], "r/lib/other", 100, &mut Vec::new()).is_err(), "over the size limit");
    }
}
