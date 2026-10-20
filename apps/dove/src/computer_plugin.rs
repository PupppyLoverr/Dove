//! Dove Computer Use: the `dove-driver` plugin (Dove's fork of the
//! open-source cua-driver), installed only when the user clicks Install in
//! Settings → Integrations.
//!
//! The driver comes from the dove-driver GitHub releases, at the version
//! pinned in the release manifest compiled into Dove
//! (`assets/computer-use-release.json`), and is accepted only when its
//! SHA-256 matches that manifest. It lands in
//! `<dove data>/plugins/computer-use/<version>/`, which the hub launches as
//! the `computer` upstream with [`launch_env`]. Update replaces it with the
//! pinned version; Uninstall removes the plugin folder, and with it the
//! `computer__*` tools. The optional local grounder (the `intent` step and
//! `act` tool) is a second asset in the same manifest, downloaded only when
//! the user turns it on.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The release manifest this build of Dove installs from.
const MANIFEST: &str = include_str!("../assets/computer-use-release.json");
/// Larger downloads are refused (the driver is about 30 MB).
const MAX_BYTES: u64 = 512 * 1024 * 1024;
const EXE: &str = if cfg!(windows) { "dove-driver.exe" } else { "dove-driver" };
/// Driver state (`DOVE_DRIVER_HOME`) and the grounder live next to the
/// version folders, so an update keeps them.
const STATE_DIR: &str = "state";
const GROUNDER_DIR: &str = "grounder";

/// Settings › Integrations › Computer use, stored in `integrations.json` as
/// `computer`. Older files carry only `enabled`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    /// How much the driver may do without asking (`DOVE_DRIVER_PERMISSION_MODE`).
    pub permission_mode: PermissionMode,
    /// Use the local grounder for `intent` steps (downloads its weights).
    pub grounder: bool,
    /// The permissions walkthrough was finished or dismissed.
    pub walkthrough_done: bool,
}

/// The driver's own permission modes. `unrestricted` needs a separate
/// risk acknowledgement and is not offered in Settings.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    /// The driver asks before acting on an app the first time.
    #[default]
    Standard,
    /// Agents act within the driver's capability manifest without asking.
    Bounded,
}

impl PermissionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Bounded => "bounded",
        }
    }
}

// ---------------------------------------------------------------- manifest

/// The release manifest: one pinned dove-driver version, its per-platform
/// assets and the optional grounder weights.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Manifest {
    pub version: String,
    /// `owner/name` of the repository whose releases carry the assets.
    pub repo: String,
    /// Release tag, `dove-driver-v<version>` by convention.
    pub tag: String,
    pub assets: Vec<Asset>,
    pub grounder: Option<Asset>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Asset {
    /// File name on the release.
    pub name: String,
    /// `windows` | `macos` | `linux` (empty for the grounder).
    pub os: String,
    /// `x86_64` | `aarch64` (empty for the grounder).
    pub arch: String,
    /// Lowercase hex SHA-256 of the file.
    pub sha256: String,
    /// Size in bytes, when known; shown before downloading.
    pub size: Option<u64>,
    /// The file name the driver expects (grounder only).
    pub file: Option<String>,
}

impl Asset {
    fn valid(&self) -> bool {
        !self.name.is_empty()
            && !self.name.contains(['/', '\\'])
            && self.sha256.len() == 64
            && self.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    }

    fn kind(&self) -> Kind {
        let n = self.name.to_ascii_lowercase();
        if n.ends_with(".zip") {
            Kind::Zip
        } else if n.ends_with(".tar.gz") || n.ends_with(".tgz") {
            Kind::TarGz
        } else {
            Kind::Binary
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Binary,
    Zip,
    TarGz,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let m: Manifest = serde_json::from_str(text).map_err(|e| format!("bad release manifest: {e}"))?;
        if parse_version(&m.version).is_none() {
            return Err(format!("bad release manifest: version {:?}", m.version));
        }
        Ok(m)
    }

    /// The asset for `os`/`arch`, if the release has a valid one.
    pub fn asset_for(&self, os: &str, arch: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.os == os && a.arch == arch && a.valid())
    }

    /// The asset for this machine.
    pub fn asset(&self) -> Option<&Asset> {
        let (os, arch) = platform()?;
        self.asset_for(os, arch)
    }

    pub fn grounder(&self) -> Option<&Asset> {
        self.grounder.as_ref().filter(|a| a.valid())
    }

    fn url(&self, asset: &Asset) -> String {
        match release_base() {
            Some(base) => format!("{}/{}", base.trim_end_matches('/'), asset.name),
            None => format!("https://github.com/{}/releases/download/{}/{}", self.repo, self.tag, asset.name),
        }
    }
}

/// This OS and CPU in the manifest's names.
pub fn platform() -> Option<(&'static str, &'static str)> {
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
    Some((os, arch))
}

/// The compiled-in manifest. Debug builds may point
/// `DOVE_COMPUTER_USE_MANIFEST` at a fixture for QA against a fake release.
pub fn manifest() -> &'static Manifest {
    static M: OnceLock<Manifest> = OnceLock::new();
    M.get_or_init(|| {
        let text = cfg!(debug_assertions)
            .then(|| std::env::var_os("DOVE_COMPUTER_USE_MANIFEST"))
            .flatten()
            .and_then(|p| std::fs::read_to_string(p).ok());
        Manifest::parse(text.as_deref().unwrap_or(MANIFEST)).unwrap_or_default()
    })
}

/// Where release files are fetched from instead of GitHub, in debug
/// builds: `DOVE_COMPUTER_USE_RELEASE_BASE` (a URL; for QA, a local server
/// over a scratch folder). The SHA-256 check is the same either way.
fn release_base() -> Option<String> {
    cfg!(debug_assertions).then(|| std::env::var("DOVE_COMPUTER_USE_RELEASE_BASE").ok()).flatten()
}

/// The pinned driver version.
pub fn version() -> &'static str {
    &manifest().version
}

/// Whether this build can install the plugin on this machine.
pub fn available() -> bool {
    manifest().asset().is_some()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

// ------------------------------------------------------------------ layout

pub fn root() -> PathBuf {
    root_in(&crate::pages::integrations::data_root())
}

fn root_in(data: &Path) -> PathBuf {
    data.join("plugins").join("computer-use")
}

/// An installed copy: its version and binary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub version: String,
    pub bin: PathBuf,
}

impl Installed {
    pub fn outdated(&self) -> bool {
        parse_version(&self.version) < parse_version(version())
    }

    pub fn dir(&self) -> &Path {
        self.bin.parent().unwrap_or(&self.bin)
    }
}

/// The newest installed version under `root`, if its binary is there.
fn installed_in(root: &Path) -> Option<Installed> {
    let mut best: Option<(Vec<u64>, Installed)> = None;
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let version = entry.file_name().to_string_lossy().to_string();
        let Some(key) = parse_version(&version) else { continue };
        let Some(bin) = bin_in(&entry.path()) else { continue };
        if best.as_ref().is_none_or(|(k, _)| key > *k) {
            best = Some((key, Installed { version, bin }));
        }
    }
    best.map(|(_, i)| i)
}

/// The driver binary in a version folder: where `plugin.json` says, else
/// at the top.
fn bin_in(dir: &Path) -> Option<PathBuf> {
    let recorded = std::fs::read(dir.join("plugin.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("bin")?.as_str().map(|s| dir.join(s)));
    recorded.into_iter().chain([dir.join(EXE)]).find(|p| p.is_file())
}

fn parse_version(v: &str) -> Option<Vec<u64>> {
    let parts: Option<Vec<u64>> = v.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| !p.is_empty())
}

pub fn installed() -> Option<Installed> {
    installed_in(&root())
}

/// The installed grounder weights, if any.
pub fn grounder_installed() -> Option<PathBuf> {
    grounder_in(&root(), manifest())
}

fn grounder_in(root: &Path, m: &Manifest) -> Option<PathBuf> {
    let g = m.grounder()?;
    let path = root.join(GROUNDER_DIR).join(&g.sha256[..12]).join(g.file.as_deref().unwrap_or(&g.name));
    path.is_file().then_some(path)
}

/// The environment the hub (and the Settings health check) starts the
/// driver with: its own state under Dove's data folder, the user's
/// permission mode, lazy tool disclosure, and the grounder when installed.
pub fn launch_env(settings: &Settings) -> BTreeMap<String, String> {
    launch_env_in(&root(), settings, grounder_installed())
}

fn launch_env_in(root: &Path, settings: &Settings, grounder: Option<PathBuf>) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("DOVE_DRIVER_HOME".into(), root.join(STATE_DIR).display().to_string());
    env.insert("DOVE_DRIVER_PERMISSION_MODE".into(), settings.permission_mode.as_str().into());
    env.insert("DOVE_TOOL_DISCLOSURE".into(), "lazy".into());
    if let Some(model) = grounder.filter(|_| settings.grounder) {
        env.insert("DOVE_DRIVER_GROUNDER_MODEL".into(), model.display().to_string());
    }
    env
}

// ----------------------------------------------------------------- install

/// Download the pinned release for this machine, verify it and install it.
/// Blocking; run it off the UI thread.
pub fn install() -> Result<Installed, String> {
    install_in(&root(), manifest())
}

fn install_in(root: &Path, m: &Manifest) -> Result<Installed, String> {
    let (os, arch) = platform().ok_or("Dove Computer Use is not built for this platform")?;
    let asset = m
        .asset_for(os, arch)
        .ok_or_else(|| format!("No dove-driver {} release for this platform ({os}-{arch}) yet", m.version))?;
    let bytes = download(&m.url(asset), asset.size)?;
    install_bytes(root, &m.version, asset, &bytes)
}

/// Unpack a verified download into `root/<version>/` and drop older
/// versions. Nothing is written unless `bytes` matches the manifest.
fn install_bytes(root: &Path, version: &str, asset: &Asset, bytes: &[u8]) -> Result<Installed, String> {
    verify(asset, bytes)?;
    std::fs::create_dir_all(root).map_err(|e| format!("could not create {}: {e}", root.display()))?;
    let stage = root.join(format!(".stage-{version}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    let result = (|| -> Result<Installed, String> {
        std::fs::create_dir_all(&stage).map_err(|e| format!("could not create {}: {e}", stage.display()))?;
        match asset.kind() {
            Kind::Binary => write_file(&stage.join(EXE), bytes)?,
            Kind::Zip => unzip(bytes, &stage)?,
            Kind::TarGz => untar(bytes, &stage)?,
        }
        let bin = find_exe(&stage).ok_or_else(|| format!("the release has no {EXE}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for exe in executables(&stage) {
                let _ = std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o755));
            }
        }
        let rel = bin.strip_prefix(&stage).unwrap_or(&bin).to_string_lossy().replace('\\', "/");
        let meta = serde_json::json!({"name": "dove-driver", "version": version, "asset": asset.name,
            "sha256": asset.sha256.to_ascii_lowercase(), "bin": rel});
        write_file(&stage.join("plugin.json"), meta.to_string().as_bytes())?;
        let dir = root.join(version);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| busy(&dir, e))?;
        }
        std::fs::rename(&stage, &dir).map_err(|e| format!("could not install {}: {e}", dir.display()))?;
        Ok(Installed { version: version.to_string(), bin: dir.join(rel) })
    })();
    let _ = std::fs::remove_dir_all(&stage);
    let installed = result?;
    remove_others(root, version);
    Ok(installed)
}

fn verify(asset: &Asset, bytes: &[u8]) -> Result<(), String> {
    let got = sha256_hex(bytes);
    if got.eq_ignore_ascii_case(&asset.sha256) {
        Ok(())
    } else {
        Err(format!(
            "The download's checksum does not match Dove's manifest (got {}…). Nothing was installed.",
            &got[..12]
        ))
    }
}

fn busy(path: &Path, e: std::io::Error) -> String {
    format!("could not remove {} ({e}). Quit agents using computer use and try again.", path.display())
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// The driver binary in an unpacked release: at the top, or inside a
/// single top-level folder.
fn find_exe(dir: &Path) -> Option<PathBuf> {
    let top = dir.join(EXE);
    if top.is_file() {
        return Some(top);
    }
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path().join(EXE)).find(|p| p.is_file())
}

#[cfg(unix)]
fn executables(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("dove-driver")) {
                out.push(p);
            }
        }
    }
    out
}

fn unzip(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("bad release archive: {e}"))?;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|e| format!("bad release archive: {e}"))?;
        // `enclosed_name` refuses absolute paths and `..`.
        let Some(rel) = file.enclosed_name() else { return Err("the release archive has an unsafe path".into()) };
        let out = dest.join(rel);
        if file.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut buf = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut buf).map_err(|e| format!("bad release archive: {e}"))?;
        write_file(&out, &buf)?;
    }
    Ok(())
}

/// `.tar.gz` releases (macOS, Linux) unpack with the system `tar`, after
/// its listing is checked for absolute or `..` paths.
fn untar(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let archive = dest.join(".release.tar.gz");
    write_file(&archive, bytes)?;
    let run = |args: &[&std::ffi::OsStr]| {
        std::process::Command::new("tar").args(args).output().map_err(|e| format!("could not run tar: {e}")).and_then(
            |o| {
                if o.status.success() {
                    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
                } else {
                    Err(format!("tar failed: {}", String::from_utf8_lossy(&o.stderr).trim()))
                }
            },
        )
    };
    let listing = run(&["-tzf".as_ref(), archive.as_os_str()])?;
    if listing.lines().any(|l| l.starts_with('/') || l.split(['/', '\\']).any(|c| c == "..")) {
        return Err("the release archive has an unsafe path".into());
    }
    run(&["-xzf".as_ref(), archive.as_os_str(), "-C".as_ref(), dest.as_os_str()])?;
    let _ = std::fs::remove_file(&archive);
    Ok(())
}

/// Remove every version folder except `keep`, leaving the driver state and
/// the grounder. A running driver keeps its folder locked on Windows; that
/// one goes on the next update.
fn remove_others(root: &Path, keep: &str) {
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name != keep && parse_version(&name).is_some() && entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Remove the plugin: every version, the driver's state and the grounder.
pub fn uninstall() -> Result<(), String> {
    uninstall_in(&root())
}

fn uninstall_in(root: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(root) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(busy(root, e)),
    }
}

/// Download and verify the grounder weights. Blocking.
pub fn install_grounder() -> Result<PathBuf, String> {
    install_grounder_in(&root(), manifest())
}

fn install_grounder_in(root: &Path, m: &Manifest) -> Result<PathBuf, String> {
    let g = m.grounder().ok_or_else(|| format!("dove-driver {} has no local grounder release yet", m.version))?;
    if let Some(path) = grounder_in(root, m) {
        return Ok(path);
    }
    let bytes = download(&m.url(g), g.size)?;
    verify(g, &bytes)?;
    let base = root.join(GROUNDER_DIR);
    let dir = base.join(&g.sha256[..12]);
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let path = dir.join(g.file.as_deref().unwrap_or(&g.name));
    let part = dir.join(".download.part");
    write_file(&part, &bytes)?;
    std::fs::rename(&part, &path).map_err(|e| format!("could not install {}: {e}", path.display()))?;
    // Older weights go.
    for e in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        if e.path() != dir {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    Ok(path)
}

pub fn remove_grounder() -> Result<(), String> {
    let dir = root().join(GROUNDER_DIR);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(busy(&dir, e)),
    }
}

fn download(url: &str, size: Option<u64>) -> Result<Vec<u8>, String> {
    let limit = size.map_or(MAX_BYTES, |s| s.min(MAX_BYTES));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .user_agent(concat!("Dove/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(900))
            .build()
            .map_err(|e| e.to_string())?;
        let mut resp = client.get(url).send().await.map_err(|e| format!("could not download: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("the release download answered {}", resp.status()));
        }
        let mut out = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("download interrupted: {e}"))? {
            out.extend_from_slice(&chunk);
            if out.len() as u64 > limit {
                return Err("the download is larger than the release manifest says; stopped".into());
            }
        }
        Ok(out)
    })
}

// ------------------------------------------------------------------ health

/// The driver's `health_report` (schema_version 1), condensed for the plate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Health {
    /// `ok` | `degraded` | `failed`, or empty when the driver did not say.
    pub overall: String,
    pub driver_version: Option<String>,
    /// Failing checks: their message and, when given, the fix.
    pub problems: Vec<(String, Option<String>)>,
    pub passed: usize,
    /// Why there is no report (the driver did not start or answer).
    pub error: Option<String>,
}

impl Health {
    pub fn failed(error: impl Into<String>) -> Self {
        Self { error: Some(error.into()), ..Self::default() }
    }

    pub fn from_report(report: &serde_json::Value) -> Self {
        let checks = report.get("checks").and_then(|c| c.as_array()).cloned().unwrap_or_default();
        let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        let mut h = Health {
            overall: s(report, "overall").unwrap_or_default(),
            driver_version: s(report, "driver_version"),
            ..Self::default()
        };
        for c in &checks {
            match c.get("status").and_then(|x| x.as_str()) {
                Some("pass") => h.passed += 1,
                Some("fail") => h.problems.push((
                    s(c, "message").or_else(|| s(c, "name")).unwrap_or_else(|| "A check failed".into()),
                    s(c, "hint"),
                )),
                _ => {}
            }
        }
        if h.overall.is_empty() && checks.is_empty() {
            h.error = Some("The driver answered without a health report".into());
        }
        h
    }

    pub fn ok(&self) -> bool {
        self.error.is_none() && self.overall == "ok"
    }

    /// One line for the plate.
    pub fn summary(&self) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        let ver = self.driver_version.as_deref().map(|v| format!("dove-driver {v} · ")).unwrap_or_default();
        match (self.overall.as_str(), self.problems.first()) {
            ("ok", _) => format!("{ver}Ready: all {} checks passed", self.passed),
            (_, Some((msg, hint))) => {
                let more = if self.problems.len() > 1 {
                    format!(" (+{} more)", self.problems.len() - 1)
                } else {
                    String::new()
                };
                format!("{ver}{msg}{more}{}", hint.as_deref().map(|h| format!(". {h}")).unwrap_or_default())
            }
            (o, None) => format!("{ver}Status: {o}"),
        }
    }
}

// ------------------------------------------------------------- walkthrough

/// One step of the per-OS permissions walkthrough.
pub struct Step {
    pub title: &'static str,
    pub body: &'static str,
    /// A button: its label and what it opens or runs.
    pub action: Option<(&'static str, StepAction)>,
}

#[derive(Clone, Copy)]
pub enum StepAction {
    Open(&'static str),
    /// Turn on the AT-SPI accessibility bus for GNOME apps.
    EnableAtSpi,
}

pub fn walkthrough() -> &'static [Step] {
    if cfg!(target_os = "macos") {
        &[
            Step {
                title: "Accessibility",
                body: "Lets dove-driver read windows and press buttons. Turn on dove-driver in Privacy & Security › Accessibility.",
                action: Some((
                    "Open Accessibility",
                    StepAction::Open("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"),
                )),
            },
            Step {
                title: "Screen Recording",
                body: "Lets dove-driver take window screenshots when the accessibility tree is not enough. Turn on dove-driver in Privacy & Security › Screen & System Audio Recording.",
                action: Some((
                    "Open Screen Recording",
                    StepAction::Open("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"),
                )),
            },
        ]
    } else if cfg!(windows) {
        &[Step {
            title: "Same integrity level",
            body: "dove-driver runs as you, at the same integrity level as Dove. Windows stops it from reading or controlling apps started as administrator (elevated). Run those apps normally, or start Dove as administrator only when you need to automate them. UAC prompts always stay with you.",
            action: None,
        }]
    } else {
        &[
            Step {
                title: "Accessibility bus (AT-SPI)",
                body: "dove-driver reads apps through AT-SPI. GNOME and KDE turn it on by default; elsewhere enable it, then restart the apps you want to automate.",
                action: Some(("Enable AT-SPI", StepAction::EnableAtSpi)),
            },
            Step {
                title: "Wayland",
                body: "On Wayland, screenshots go through the desktop portal: approve the screen-share prompt the first time. X11 sessions need nothing extra.",
                action: None,
            },
        ]
    }
}

/// Linux: `gsettings set org.gnome.desktop.interface toolkit-accessibility true`.
pub fn enable_at_spi() -> Result<(), String> {
    let out = std::process::Command::new("gsettings")
        .args(["set", "org.gnome.desktop.interface", "toolkit-accessibility", "true"])
        .output()
        .map_err(|e| format!("could not run gsettings: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    const SHA_HELLO: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dove-cu-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn asset(name: &str, bytes: &[u8]) -> Asset {
        let (os, arch) = platform().unwrap();
        Asset {
            name: name.into(),
            os: os.into(),
            arch: arch.into(),
            sha256: sha256_hex(bytes),
            size: Some(bytes.len() as u64),
            file: None,
        }
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in files {
            out.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            out.write_all(bytes).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    /// A one-shot-per-request HTTP server over `dir`: the "fake release".
    fn serve(dir: PathBuf) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut req = [0u8; 2048];
                let n = stream.read(&mut req).unwrap_or(0);
                let line = String::from_utf8_lossy(&req[..n]).lines().next().unwrap_or("").to_string();
                let path = line.split_whitespace().nth(1).unwrap_or("/").trim_start_matches('/').to_string();
                match std::fs::read(dir.join(&path)) {
                    Ok(body) => {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(&body);
                    }
                    Err(_) => {
                        let _ = stream
                            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn compiled_manifest_parses() {
        let m = Manifest::parse(MANIFEST).expect("assets/computer-use-release.json");
        assert!(m.repo.contains('/'));
        assert_eq!(m.tag, format!("dove-driver-v{}", m.version));
        // Every listed asset is well formed; a bad line would silently hide a platform.
        assert!(m.assets.iter().all(Asset::valid), "{:?}", m.assets);
    }

    #[test]
    fn manifest_picks_this_platform_and_skips_bad_entries() {
        let text = format!(
            r#"{{"version":"0.38.0","repo":"o/r","tag":"dove-driver-v0.38.0","assets":[
              {{"name":"dove-driver-0.38.0-linux-x86_64.tar.gz","os":"linux","arch":"x86_64","sha256":"{SHA_HELLO}"}},
              {{"name":"bad","os":"windows","arch":"x86_64","sha256":"nothex"}},
              {{"name":"../evil.zip","os":"macos","arch":"aarch64","sha256":"{SHA_HELLO}"}}],
              "grounder":{{"name":"grounder.onnx","sha256":"{SHA_HELLO}","size":5}}}}"#
        );
        let m = Manifest::parse(&text).unwrap();
        assert_eq!(m.asset_for("linux", "x86_64").map(|a| a.kind()), Some(Kind::TarGz));
        assert!(m.asset_for("windows", "x86_64").is_none(), "bad sha refused");
        assert!(m.asset_for("macos", "aarch64").is_none(), "path in name refused");
        assert!(m.grounder().is_some());
        assert!(
            m.url(m.asset_for("linux", "x86_64").unwrap())
                .starts_with("https://github.com/o/r/releases/download/dove-driver-v0.38.0/")
        );
        assert!(Manifest::parse(r#"{"version":"latest"}"#).is_err());
    }

    #[test]
    fn installs_only_verified_bytes_and_replaces_old_versions() {
        let root = scratch("bytes");
        let a = asset(EXE, b"hello");
        // A tampered download installs nothing.
        assert!(install_bytes(&root, "0.36.0", &a, b"evil").is_err());
        assert!(installed_in(&root).is_none());
        let old = install_bytes(&root, "0.36.0", &a, b"hello").unwrap();
        assert!(old.bin.is_file());
        assert_eq!(installed_in(&root), Some(old.clone()));
        std::fs::create_dir_all(root.join(STATE_DIR)).unwrap();
        install_bytes(&root, "0.37.0", &a, b"hello").unwrap();
        assert_eq!(installed_in(&root).map(|i| i.version), Some("0.37.0".to_string()));
        assert!(!old.bin.exists(), "the update removes the old version");
        assert!(root.join(STATE_DIR).is_dir(), "the update keeps the driver's state");
        uninstall_in(&root).unwrap();
        assert!(installed_in(&root).is_none() && !root.exists());
    }

    #[test]
    fn zip_release_with_helper_installs_from_a_fake_release() {
        let release = scratch("release");
        let root = scratch("root");
        let uia = if cfg!(windows) { "dove-driver-uia.exe" } else { "dove-driver-uia" };
        let bytes = zip_of(&[(&format!("dove-driver/{EXE}"), b"driver"), (&format!("dove-driver/{uia}"), b"uia")]);
        std::fs::write(release.join("dove-driver.zip"), &bytes).unwrap();
        let grounder_bytes = b"weights";
        std::fs::write(release.join("grounder.onnx"), grounder_bytes).unwrap();
        let base = serve(release);
        let m = Manifest {
            version: "0.37.0".into(),
            repo: "o/r".into(),
            tag: "dove-driver-v0.37.0".into(),
            assets: vec![asset("dove-driver.zip", &bytes)],
            grounder: Some(Asset { file: Some("model.onnx".into()), ..asset("grounder.onnx", grounder_bytes) }),
        };
        let fetch = |a: &Asset| download(&format!("{base}/{}", a.name), a.size);
        let got = install_bytes(&root, &m.version, &m.assets[0], &fetch(&m.assets[0]).unwrap()).unwrap();
        assert_eq!(std::fs::read(&got.bin).unwrap(), b"driver");
        assert!(got.dir().join(uia).is_file(), "the UIA helper sits next to the driver");
        assert_eq!(installed_in(&root), Some(got));
        // The manifest's size caps the download.
        let small = Asset { size: Some(3), ..m.assets[0].clone() };
        assert!(fetch(&small).is_err());
        assert!(download(&format!("{base}/missing.zip"), None).is_err());

        // Grounder: verified, stored under its hash, passed to the driver only when on.
        let g = m.grounder().unwrap().clone();
        let bytes = fetch(&g).unwrap();
        verify(&g, &bytes).unwrap();
        let dir = root.join(GROUNDER_DIR).join(&g.sha256[..12]);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.onnx"), &bytes).unwrap();
        let path = grounder_in(&root, &m).unwrap();
        let on = Settings { enabled: true, grounder: true, ..Settings::default() };
        let env = launch_env_in(&root, &on, Some(path.clone()));
        assert_eq!(env["DOVE_DRIVER_GROUNDER_MODEL"], path.display().to_string());
        let off = launch_env_in(&root, &Settings { grounder: false, ..on }, Some(path));
        assert!(!off.contains_key("DOVE_DRIVER_GROUNDER_MODEL"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn zip_with_unsafe_paths_is_refused() {
        let root = scratch("unsafe");
        let bytes = zip_of(&[("../outside.exe", b"x"), (EXE, b"driver")]);
        assert!(install_bytes(&root, "0.37.0", &asset("d.zip", &bytes), &bytes).is_err());
        assert!(installed_in(&root).is_none());
        let bytes = zip_of(&[("README.md", b"no driver here")]);
        assert!(install_bytes(&root, "0.37.0", &asset("d.zip", &bytes), &bytes).unwrap_err().contains("has no"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn launch_env_sets_home_mode_and_lazy_tools() {
        let root = PathBuf::from("plugins").join("computer-use");
        let env =
            launch_env_in(&root, &Settings { permission_mode: PermissionMode::Bounded, ..Settings::default() }, None);
        assert_eq!(env["DOVE_DRIVER_HOME"], root.join(STATE_DIR).display().to_string());
        assert_eq!(env["DOVE_DRIVER_PERMISSION_MODE"], "bounded");
        assert_eq!(env["DOVE_TOOL_DISCLOSURE"], "lazy");
    }

    #[test]
    fn settings_read_older_files() {
        let s: Settings = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(s.enabled && !s.grounder && s.permission_mode == PermissionMode::Standard);
        let s: Settings =
            serde_json::from_str(r#"{"enabled":true,"permission_mode":"bounded","grounder":true}"#).unwrap();
        assert_eq!(s.permission_mode, PermissionMode::Bounded);
    }

    #[test]
    fn health_report_summary() {
        let ok = serde_json::json!({"schema_version":"1","overall":"ok","driver_version":"0.37.0",
            "checks":[{"name":"binary_version","status":"pass","message":"v"},{"name":"ax","status":"pass","message":"a"}]});
        let h = Health::from_report(&ok);
        assert!(h.ok());
        assert_eq!(h.summary(), "dove-driver 0.37.0 · Ready: all 2 checks passed");
        let bad = serde_json::json!({"overall":"degraded","checks":[
            {"name":"tcc_accessibility","status":"fail","message":"Accessibility is off","hint":"Turn it on in System Settings"},
            {"name":"tcc_screen_recording","status":"fail","message":"Screen Recording is off"},
            {"name":"binary_version","status":"skip","message":"-"}]});
        let h = Health::from_report(&bad);
        assert!(!h.ok());
        assert_eq!(h.problems.len(), 2);
        assert_eq!(h.summary(), "Accessibility is off (+1 more). Turn it on in System Settings");
        assert!(Health::from_report(&serde_json::json!("text")).error.is_some());
    }

    #[test]
    fn newest_version_wins() {
        assert!(parse_version("0.37.0") > parse_version("0.9.12"));
        assert_eq!(parse_version("latest"), None);
        assert_eq!(parse_version(STATE_DIR), None);
        assert_eq!(parse_version(GROUNDER_DIR), None);
        let i = Installed { version: "0.1.0".into(), bin: PathBuf::from(EXE) };
        assert!(i.outdated() || version().is_empty());
    }

    #[test]
    fn every_os_has_a_walkthrough() {
        assert!(!walkthrough().is_empty());
        assert!(walkthrough().iter().all(|s| !s.title.is_empty() && !s.body.is_empty()));
    }
}
