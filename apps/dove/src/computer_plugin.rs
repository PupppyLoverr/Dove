//! Dove Computer Use: the `dove-driver` plugin (Dove's fork of the
//! open-source cua-driver), installed only when the user clicks Install in
//! Settings → Integrations.
//!
//! The binary comes from Dove's own GitHub releases, at the version pinned
//! here, and is accepted only when its SHA-256 matches the manifest compiled
//! into Dove (`assets/computer-use-manifest.txt`, `sha256sum` format). It
//! lands in `<dove data>/plugins/computer-use/<version>/`, which the hub
//! launches as the `computer` upstream. Update replaces it with the pinned
//! version; Uninstall removes the folder, and with it the `computer__*`
//! tools.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest as _, Sha256};

/// The dove-driver release this build of Dove installs.
pub const VERSION: &str = "0.37.0";
/// Dove's repository, whose releases carry the driver assets.
const REPO: &str = "PupppyLoverr/Dove";
/// Release tag of the driver assets.
fn tag() -> String {
    format!("dove-driver-v{VERSION}")
}
/// `<sha256>  <asset>` lines, one per OS/arch, for [`VERSION`].
const MANIFEST: &str = include_str!("../assets/computer-use-manifest.txt");
/// Larger downloads are refused (the driver is about 30 MB).
const MAX_BYTES: usize = 256 * 1024 * 1024;
const EXE: &str = if cfg!(windows) { "dove-driver.exe" } else { "dove-driver" };

/// The release asset for this OS and CPU, as the driver's CI names it.
pub fn asset_name() -> Option<String> {
    let os = match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "macos",
        "linux" => "linux",
        _ => return None,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => return None,
    };
    Some(format!("dove-driver-{os}-{arch}{}", if os == "windows" { ".exe" } else { "" }))
}

/// The expected SHA-256 (lowercase hex) of `asset` in `manifest`.
pub fn expected_sha(manifest: &str, asset: &str) -> Option<String> {
    manifest.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).find_map(|l| {
        let (sha, name) = l.split_once(char::is_whitespace)?;
        // `sha256sum` marks binary mode with a leading `*`.
        let name = name.trim().trim_start_matches('*');
        (name == asset && sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| sha.to_ascii_lowercase())
    })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether this build can install the plugin on this machine.
pub fn available() -> bool {
    asset_name().is_some_and(|a| expected_sha(MANIFEST, &a).is_some())
}

fn plugin_root(data: &Path) -> PathBuf {
    data.join("plugins").join("computer-use")
}

pub fn root() -> PathBuf {
    plugin_root(&crate::pages::integrations::data_root())
}

/// An installed copy: its version and binary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub version: String,
    pub bin: PathBuf,
}

impl Installed {
    pub fn outdated(&self) -> bool {
        self.version != VERSION
    }
}

/// The newest installed version under `root`, if its binary is there.
fn installed_in(root: &Path) -> Option<Installed> {
    let mut best: Option<(Vec<u64>, Installed)> = None;
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let version = entry.file_name().to_string_lossy().to_string();
        let bin = entry.path().join(EXE);
        let Some(key) = parse_version(&version) else { continue };
        if !bin.is_file() {
            continue;
        }
        if best.as_ref().is_none_or(|(k, _)| key > *k) {
            best = Some((key, Installed { version, bin }));
        }
    }
    best.map(|(_, i)| i)
}

fn parse_version(v: &str) -> Option<Vec<u64>> {
    v.split('.').map(|p| p.parse().ok()).collect()
}

pub fn installed() -> Option<Installed> {
    installed_in(&root())
}

/// Write a verified binary into `root/<version>/` and drop older versions.
/// Nothing is written unless `bytes` matches `want`.
fn install_bytes(root: &Path, version: &str, bytes: &[u8], want: &str) -> Result<Installed, String> {
    let got = sha256_hex(bytes);
    if !got.eq_ignore_ascii_case(want) {
        return Err(format!("The download's checksum does not match Dove's manifest (got {}…). Nothing was installed.", &got[..12]));
    }
    let dir = root.join(version);
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let bin = dir.join(EXE);
    let part = dir.join(format!("{EXE}.part"));
    std::fs::write(&part, bytes).map_err(|e| format!("could not write {}: {e}", part.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755));
    }
    std::fs::rename(&part, &bin).map_err(|e| format!("could not install {}: {e}", bin.display()))?;
    let _ = std::fs::write(
        dir.join("plugin.json"),
        serde_json::json!({"name": "dove-driver", "version": version, "sha256": got}).to_string(),
    );
    remove_others(root, version);
    Ok(Installed { version: version.to_string(), bin })
}

/// Remove every version folder except `keep`. A running driver keeps its
/// folder locked on Windows; that one goes on the next update.
fn remove_others(root: &Path, keep: &str) {
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy() != keep && entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Download the pinned release for this machine, verify it and install it.
/// Blocking; run it off the UI thread.
pub fn install() -> Result<Installed, String> {
    let asset = asset_name().ok_or("Dove Computer Use is not built for this platform")?;
    let want = expected_sha(MANIFEST, &asset)
        .ok_or_else(|| format!("No Dove Computer Use {VERSION} release for this platform ({asset}) yet"))?;
    let url = format!("https://github.com/{REPO}/releases/download/{}/{asset}", tag());
    let bytes = download(&url)?;
    install_bytes(&root(), VERSION, &bytes, &want)
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .user_agent(concat!("Dove/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| e.to_string())?;
        let mut resp = client.get(url).send().await.map_err(|e| format!("could not download: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("the release download answered {}", resp.status()));
        }
        let mut out = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("download interrupted: {e}"))? {
            out.extend_from_slice(&chunk);
            if out.len() > MAX_BYTES {
                return Err("the download is larger than expected; stopped".into());
            }
        }
        Ok(out)
    })
}

/// Remove every installed version.
pub fn uninstall() -> Result<(), String> {
    let root = root();
    match std::fs::remove_dir_all(&root) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!(
            "could not remove {} ({e}). Quit agents using computer use and try again.",
            root.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"; // "hello"

    #[test]
    fn reads_sha256sum_manifest() {
        let manifest = format!(
            "# dove-driver 0.37.0\n{SHA_A}  dove-driver-linux-x86_64\n{}  *dove-driver-windows-x86_64.exe\nbad line\n",
            "AB".repeat(32)
        );
        assert_eq!(expected_sha(&manifest, "dove-driver-linux-x86_64").as_deref(), Some(SHA_A));
        assert_eq!(expected_sha(&manifest, "dove-driver-windows-x86_64.exe"), Some("ab".repeat(32)));
        assert_eq!(expected_sha(&manifest, "dove-driver-macos-aarch64"), None);
        assert_eq!(sha256_hex(b"hello"), SHA_A);
    }

    #[test]
    fn asset_names_follow_release_naming() {
        let name = asset_name().unwrap();
        assert!(name.starts_with("dove-driver-"));
        assert_eq!(name.ends_with(".exe"), cfg!(windows));
    }

    #[test]
    fn installs_only_verified_bytes_and_replaces_old_versions() {
        let root = std::env::temp_dir().join(format!("dove-cu-plugin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // A tampered download installs nothing.
        assert!(install_bytes(&root, "0.36.0", b"evil", SHA_A).is_err());
        assert!(installed_in(&root).is_none());
        let old = install_bytes(&root, "0.36.0", b"hello", SHA_A).unwrap();
        assert!(old.bin.is_file());
        assert_eq!(installed_in(&root), Some(old.clone()));
        let new = install_bytes(&root, "0.37.0", b"hello", SHA_A).unwrap();
        assert_eq!(installed_in(&root).map(|i| i.version), Some("0.37.0".to_string()));
        assert!(!old.bin.exists(), "the update removes the old version");
        assert_eq!(new.outdated(), VERSION != "0.37.0");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn newest_version_wins() {
        assert!(parse_version("0.37.0") > parse_version("0.9.12"));
        assert_eq!(parse_version("latest"), None);
    }
}
