//! Dove Computer Use: stock cua-driver (trycua/cua, MIT, signed by Cua AI),
//! installed only when the user clicks Install in Settings → Plugins.
//!
//! The driver comes from trycua's GitHub release at the version pinned in
//! the manifest compiled into Dove (`assets/computer-use-release.json`,
//! written from the release's archives), and is accepted only when the
//! archive's size and SHA-256 and every file in it match that manifest.
//! Before each launch the hub re-hashes the driver and its helpers. It
//! lands in `<dove data>/plugins/computer-use/<version>/`, which the hub
//! launches as the `computer` upstream with [`launch_env`]: its own state
//! folder, no telemetry, no update checks. Agents never see the driver's
//! tools: the hub wraps it in the one `computer` tool (crate
//! `dove-computer`). Update replaces it with the pinned version; Uninstall
//! removes the plugin folder.

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
/// The installer's record in each version folder.
const RECORD: &str = "plugin.json";
/// The manifest a local build keeps beside its files (a break-glass driver
/// build packaged with dove-driver's `dev-install.py`).
const DEV_MANIFEST: &str = "dove-driver-manifest.json";
/// Driver state (`CUA_DRIVER_RS_HOME`) lives next to the version folders,
/// so an update keeps it.
const STATE_DIR: &str = "state";

/// Settings › Plugins › Computer use, stored in `integrations.json` as
/// `computer`. Older files carry only `enabled`; unknown fields are ignored.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    /// Rank `act` and intent steps with the built-in NEXI-CUA-1 network;
    /// off, the deterministic resolver decides alone.
    pub ranker: bool,
    /// The permissions walkthrough was finished or dismissed.
    pub walkthrough_done: bool,
    /// Apps agents may never control: names (`1Password`), or
    /// `signer:<publisher or Team ID>`, `bundle:<id>`, `path:<prefix>`.
    pub denied_apps: Vec<String>,
    /// `ask`: approve background input once per app per thread (default);
    /// `auto`: background input runs without asking. Foreground input and
    /// consequential controls (send, buy, delete…) always ask.
    pub approvals: String,
    /// Expose the driver's own tools (`computer__*`) instead of the one
    /// `computer` tool, for debugging the driver itself.
    pub raw_tools: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            ranker: true,
            walkthrough_done: false,
            denied_apps: Vec::new(),
            approvals: "ask".into(),
            raw_tools: false,
        }
    }
}

impl Settings {
    /// The never-control rules, parsed.
    pub fn rules(&self) -> Vec<dove_computer::Rule> {
        self.denied_apps.iter().filter_map(|l| dove_computer::Rule::parse(l)).collect()
    }

    pub fn auto_background(&self) -> bool {
        self.approvals == "auto"
    }
}

/// Whether `app` (a process name, path or app name) is on the user's
/// never-control list. Matches the file stem without `.exe`, any case.
pub fn denied(app: &str, list: &[String]) -> bool {
    let stem = app.trim().rsplit(['/', '\\']).next().unwrap_or("").to_lowercase();
    let stem = stem.trim_end_matches(".exe");
    if stem.is_empty() {
        return false;
    }
    list.iter()
        .map(|d| d.trim().to_lowercase())
        .map(|d| d.trim_end_matches(".exe").to_string())
        .filter(|d| !d.is_empty())
        .any(|d| stem == d || stem.contains(&d))
}

/// Every app name or pid a driver call targets, including the steps of a
/// `run_steps` batch: `app`, `app_name`, `process_name`, `bundle_id` and
/// `pid` keys, at any depth.
pub fn targets(args: &serde_json::Value) -> (Vec<String>, Vec<u32>) {
    let mut names = Vec::new();
    let mut pids = Vec::new();
    let mut stack = vec![args];
    while let Some(v) = stack.pop() {
        match v {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    match (k.as_str(), v) {
                        ("app" | "app_name" | "process_name" | "bundle_id", serde_json::Value::String(s)) => {
                            names.push(s.clone())
                        }
                        ("pid", v) => {
                            if let Some(p) = v.as_u64().and_then(|p| u32::try_from(p).ok()) {
                                pids.push(p);
                            }
                        }
                        _ => stack.push(v),
                    }
                }
            }
            serde_json::Value::Array(a) => stack.extend(a.iter()),
            _ => {}
        }
    }
    (names, pids)
}

/// The executable name of process `pid`, if it is still running.
pub fn process_name(pid: u32) -> Option<String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        };
        // SAFETY: plain Win32 calls on a handle we open and close here.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
            CloseHandle(h);
            if ok == 0 {
                return None;
            }
            Some(String::from_utf16_lossy(&buf[..len as usize]))
        }
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|s| s.trim().to_string())
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("ps").args(["-p", &pid.to_string(), "-o", "comm="]).output().ok()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!s.is_empty()).then_some(s)
    }
}

/// Why a driver call must be refused, if it targets a never-control app.
pub fn refusal(args: &serde_json::Value, list: &[String]) -> Option<String> {
    if list.is_empty() {
        return None;
    }
    let (names, pids) = targets(args);
    let hit = names.into_iter().chain(pids.into_iter().filter_map(process_name)).find(|n| denied(n, list))?;
    let app = hit.rsplit(['/', '\\']).next().unwrap_or(&hit).to_string();
    Some(format!(
        "The user does not let agents control {app}. It is on the never-control list in Dove → Settings → Plugins → Computer use. Ask the user to do this step themselves."
    ))
}

/// The driver's permission mode (`CUA_DRIVER_PERMISSION_MODE`). Dove's own
/// `computer` layer asks the user before input; the driver's `standard`
/// mode adds its gates on process control and existing browser profiles.
/// `bounded` refuses to start without a capability manifest and
/// `unrestricted` needs a separate risk acknowledgement: Dove uses neither.
pub const PERMISSION_MODE: &str = "standard";

// ---------------------------------------------------------------- manifest

/// The release manifest (`manifest_version` 1): one pinned version and an
/// archive per platform with every file's hash, written from the release's
/// archives. Unknown fields are ignored.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Manifest {
    pub manifest_version: u32,
    pub version: String,
    /// Release tag, `cua-driver-rs-v<version>`.
    pub tag: String,
    /// `owner/name` of the repository whose releases carry the assets.
    pub repository: String,
    /// The oldest Dove that may install this release.
    pub min_dove_version: String,
    /// By platform key: `windows-x86_64`, `windows-arm64`, `macos-universal`,
    /// `linux-x86_64`, `linux-arm64`. A missing key: no build for it.
    pub platforms: BTreeMap<String, Platform>,
}

/// One platform's archive.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Platform {
    /// File name on the release.
    pub name: String,
    pub url: String,
    /// Lowercase hex SHA-256 and exact size of the archive.
    pub sha256: String,
    pub size: u64,
    /// `zip` or `tar.gz`.
    pub archive: String,
    /// The archive's single top-level folder.
    pub root: String,
    /// The driver, relative to `root`.
    pub entrypoint: String,
    /// Every regular file under `root`, with its hash.
    pub files: Vec<FileEntry>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub executable: bool,
    /// `driver`, `uia-sidecar`, `app-bundle-driver`, `license`, `notice`.
    pub role: String,
}

fn is_sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A file name or relative path with no way out of its folder.
fn safe_rel(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with(['/', '\\'])
        && !p.contains(':')
        && p.split(['/', '\\']).all(|c| c != ".." && !c.is_empty())
}

impl Platform {
    fn valid(&self) -> bool {
        safe_rel(&self.name)
            && !self.name.contains(['/', '\\'])
            && is_sha(&self.sha256)
            && self.size > 0
            && matches!(self.archive.as_str(), "zip" | "tar.gz")
            && safe_rel(&self.root)
            && !self.root.contains(['/', '\\'])
            && self.files.iter().all(|f| safe_rel(&f.path) && is_sha(&f.sha256))
            && self.files.iter().any(|f| f.path == self.entrypoint)
    }

    /// The files checked before every launch: the driver and its helpers.
    fn launch_files(&self) -> Vec<FileEntry> {
        self.files
            .iter()
            .filter(|f| {
                f.path == self.entrypoint || matches!(f.role.as_str(), "driver" | "uia-sidecar" | "app-bundle-driver")
            })
            .cloned()
            .collect()
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let m: Manifest = serde_json::from_str(text).map_err(|e| format!("bad release manifest: {e}"))?;
        if m.manifest_version != 1 {
            return Err(format!("unsupported release manifest version {}", m.manifest_version));
        }
        if parse_version(&m.version).is_none() {
            return Err(format!("bad release manifest: version {:?}", m.version));
        }
        Ok(m)
    }

    /// The archive for platform `key`, if the release has a valid one.
    pub fn platform(&self, key: &str) -> Option<&Platform> {
        self.platforms.get(key).filter(|p| p.valid())
    }

    /// The archive for this machine.
    pub fn asset(&self) -> Option<&Platform> {
        self.platform(platform_key()?)
    }

    /// Whether this Dove is new enough for the release.
    pub fn dove_supported(&self) -> bool {
        parse_version(&self.min_dove_version).is_none_or(|min| parse_version(env!("CARGO_PKG_VERSION")) >= Some(min))
    }

    fn url(&self, name: &str, url: &str) -> String {
        match release_base() {
            Some(base) => format!("{}/{name}", base.trim_end_matches('/')),
            None if url.starts_with("https://") => url.to_string(),
            None => format!("https://github.com/{}/releases/download/{}/{name}", self.repository, self.tag),
        }
    }
}

/// This machine's platform key in the manifest.
pub fn platform_key() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x86_64",
        ("windows", "aarch64") => "windows-arm64",
        ("macos", _) => "macos-universal",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-arm64",
        _ => return None,
    })
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
/// over a scratch folder). The size and SHA-256 checks are the same.
fn release_base() -> Option<String> {
    cfg!(debug_assertions).then(|| std::env::var("DOVE_COMPUTER_USE_RELEASE_BASE").ok()).flatten()
}

/// The pinned driver version.
pub fn version() -> &'static str {
    &manifest().version
}

/// Whether this build can install the plugin on this machine.
pub fn available() -> bool {
    let m = manifest();
    m.asset().is_some() && m.dove_supported()
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

/// What the installer records next to the files (`plugin.json`).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct Record {
    name: String,
    version: String,
    asset: String,
    sha256: String,
    entrypoint: String,
    /// The driver and helpers, re-hashed before each launch.
    launch_files: Vec<FileEntry>,
}

fn record_in(dir: &Path) -> Option<Record> {
    if let Some(r) = std::fs::read(dir.join(RECORD)).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
        return Some(r);
    }
    record_from_manifest(dir)
}

/// The record of a local install made by dove-driver's
/// `scripts/dove/dev-install.py`, which leaves the release manifest it
/// verified (every file's SHA-256) beside the files instead of `plugin.json`.
fn record_from_manifest(dir: &Path) -> Option<Record> {
    let text = std::fs::read_to_string(dir.join(DEV_MANIFEST)).ok()?;
    let m = Manifest::parse(&text).ok()?;
    let p = m.platform(platform_key()?)?;
    Some(Record {
        name: "cua-driver".into(),
        version: m.version.clone(),
        asset: p.name.clone(),
        sha256: p.sha256.to_ascii_lowercase(),
        entrypoint: p.entrypoint.clone(),
        launch_files: p.launch_files(),
    })
}

/// The newest installed version under `root`, if its binary is there.
fn installed_in(root: &Path) -> Option<Installed> {
    let mut best: Option<(Vec<u64>, Installed)> = None;
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let version = entry.file_name().to_string_lossy().to_string();
        let Some(key) = parse_version(&version) else { continue };
        let Some(record) = record_in(&entry.path()) else { continue };
        if !safe_rel(&record.entrypoint) {
            continue;
        }
        let bin = entry.path().join(&record.entrypoint);
        if !bin.is_file() {
            continue;
        }
        if best.as_ref().is_none_or(|(k, _)| key > *k) {
            best = Some((key, Installed { version, bin }));
        }
    }
    best.map(|(_, i)| i)
}

/// Numeric parts plus a final 1 for a release or 0 for a pre-release
/// (`0.37.0-dev`), so a local build ranks below the same released version.
fn parse_version(v: &str) -> Option<Vec<u64>> {
    let (core, pre) = match v.split_once('-') {
        Some((core, tag)) if !tag.is_empty() => (core, true),
        Some(_) => return None,
        None => (v, false),
    };
    let parts: Option<Vec<u64>> = core.split('.').map(|p| p.parse().ok()).collect();
    let mut parts = parts.filter(|p| !p.is_empty())?;
    parts.push(u64::from(!pre));
    Some(parts)
}

pub fn installed() -> Option<Installed> {
    installed_in(&root())
}

/// Re-hash the driver and its helpers against what was installed. The hub
/// runs this before every launch; a changed file means no launch.
pub fn verify_installed(i: &Installed) -> Result<(), String> {
    let record = record_in(i.dir()).ok_or("the plugin's install record is missing; reinstall it")?;
    if record.launch_files.is_empty() {
        return Err("the plugin's install record lists no files; reinstall it".into());
    }
    for f in &record.launch_files {
        let path = i.dir().join(&f.path);
        let bytes = std::fs::read(&path).map_err(|e| format!("{} is missing ({e}); reinstall the plugin", f.path))?;
        if bytes.len() as u64 != f.size || !sha256_hex(&bytes).eq_ignore_ascii_case(&f.sha256) {
            return Err(format!("{} changed since it was installed; reinstall the plugin", f.path));
        }
    }
    Ok(())
}

/// The driver the hub launches: the installed plugin when its files check
/// out, otherwise a developer build on `PATH`.
pub fn launch_bin() -> Result<PathBuf, String> {
    match installed() {
        Some(i) => verify_installed(&i).map(|()| i.bin),
        None => crate::pages::integrations::dev_driver().ok_or_else(|| "Dove Computer Use is not installed".into()),
    }
}

/// The environment the hub (and the Settings health check) starts the
/// driver with: its own state under Dove's data folder (never shared with
/// a cua-driver the user installed), the permission mode, no telemetry and
/// no update checks (Dove's plugin manager owns updates).
pub fn launch_env(_settings: &Settings) -> BTreeMap<String, String> {
    launch_env_in(&root())
}

fn launch_env_in(root: &Path) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("CUA_DRIVER_RS_HOME".into(), root.join(STATE_DIR).display().to_string());
    env.insert("CUA_DRIVER_PERMISSION_MODE".into(), PERMISSION_MODE.into());
    env.insert("CUA_DRIVER_RS_TELEMETRY_ENABLED".into(), "0".into());
    env.insert("DO_NOT_TRACK".into(), "1".into());
    env.insert("CUA_DRIVER_RS_UPDATE_CHECK".into(), "0".into());
    env
}

// ----------------------------------------------------------------- install

/// Download the pinned release for this machine, verify it and install it.
/// Blocking; run it off the UI thread.
pub fn install() -> Result<Installed, String> {
    install_in(&root(), manifest())
}

fn install_in(root: &Path, m: &Manifest) -> Result<Installed, String> {
    let key = platform_key().ok_or("Dove Computer Use is not built for this platform")?;
    let p =
        m.platform(key).ok_or_else(|| format!("cua-driver {} has no release for this platform ({key})", m.version))?;
    if !m.dove_supported() {
        return Err(format!("cua-driver {} needs Dove {} or later", m.version, m.min_dove_version));
    }
    let bytes = download(&m.url(&p.name, &p.url), p.size)?;
    install_bytes(root, &m.version, p, &bytes)
}

/// Unpack a verified download into `root/<version>/` and drop older
/// versions. Nothing is written unless the archive matches the manifest,
/// and nothing replaces the installed version unless every file does.
fn install_bytes(root: &Path, version: &str, p: &Platform, bytes: &[u8]) -> Result<Installed, String> {
    verify(&p.sha256, p.size, bytes)?;
    std::fs::create_dir_all(root).map_err(|e| format!("could not create {}: {e}", root.display()))?;
    let stage = root.join(format!(".stage-{version}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    let result = (|| -> Result<Installed, String> {
        std::fs::create_dir_all(&stage).map_err(|e| format!("could not create {}: {e}", stage.display()))?;
        match p.archive.as_str() {
            "zip" => unzip(bytes, &stage, p)?,
            _ => untar(bytes, &stage, p)?,
        }
        let top = stage.join(&p.root);
        check_tree(&top, p)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for f in p.files.iter().filter(|f| f.executable || f.path == p.entrypoint) {
                let _ = std::fs::set_permissions(top.join(&f.path), std::fs::Permissions::from_mode(0o755));
            }
        }
        let record = Record {
            name: "cua-driver".into(),
            version: version.into(),
            asset: p.name.clone(),
            sha256: p.sha256.to_ascii_lowercase(),
            entrypoint: p.entrypoint.clone(),
            launch_files: p.launch_files(),
        };
        let json = serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?;
        write_file(&top.join(RECORD), &json)?;
        let dir = root.join(version);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| busy(&dir, e))?;
        }
        std::fs::rename(&top, &dir).map_err(|e| format!("could not install {}: {e}", dir.display()))?;
        Ok(Installed { version: version.to_string(), bin: dir.join(&p.entrypoint) })
    })();
    let _ = std::fs::remove_dir_all(&stage);
    let installed = result?;
    remove_others(root, version);
    Ok(installed)
}

fn verify(sha256: &str, size: u64, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 != size {
        return Err(format!(
            "The download is {} bytes, not the {size} in Dove's manifest. Nothing was installed.",
            bytes.len()
        ));
    }
    let got = sha256_hex(bytes);
    if got.eq_ignore_ascii_case(sha256) {
        Ok(())
    } else {
        Err(format!(
            "The download's checksum does not match Dove's manifest (got {}…). Nothing was installed.",
            &got[..12]
        ))
    }
}

/// The unpacked `root/` holds exactly the manifest's files, each with its
/// size and hash, and no links.
fn check_tree(top: &Path, p: &Platform) -> Result<(), String> {
    let mut seen = Vec::new();
    let mut stack = vec![top.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).map_err(|e| format!("bad release archive: {e}"))?.flatten() {
            let meta = std::fs::symlink_metadata(e.path()).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                return Err("the release archive contains a link".into());
            }
            if meta.is_dir() {
                stack.push(e.path());
                continue;
            }
            let rel = e.path().strip_prefix(top).unwrap_or(&e.path()).to_string_lossy().replace('\\', "/");
            let f = p
                .files
                .iter()
                .find(|f| f.path == rel)
                .ok_or_else(|| format!("the release has an unlisted file {rel}"))?;
            let bytes = std::fs::read(e.path()).map_err(|e| e.to_string())?;
            if bytes.len() as u64 != f.size || !sha256_hex(&bytes).eq_ignore_ascii_case(&f.sha256) {
                return Err(format!("{rel} does not match Dove's manifest"));
            }
            seen.push(rel);
        }
    }
    match p.files.iter().find(|f| !seen.contains(&f.path)) {
        Some(f) => Err(format!("the release is missing {}", f.path)),
        None => Ok(()),
    }
}

fn busy(path: &Path, e: std::io::Error) -> String {
    format!("could not remove {} ({e}). Quit agents using computer use and try again.", path.display())
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// Whether an archive entry belongs: under `root/`, relative, no `..`.
fn entry_ok(name: &str, p: &Platform) -> bool {
    let name = name.replace('\\', "/");
    let name = name.trim_end_matches('/');
    safe_rel(name) && (name == p.root || name.starts_with(&format!("{}/", p.root)))
}

fn unzip(bytes: &[u8], dest: &Path, p: &Platform) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("bad release archive: {e}"))?;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|e| format!("bad release archive: {e}"))?;
        if file.is_symlink() {
            return Err("the release archive contains a link".into());
        }
        // `enclosed_name` also refuses absolute paths and `..`.
        let Some(rel) = file.enclosed_name().filter(|_| entry_ok(file.name(), p)) else {
            return Err(format!("the release archive has an unexpected path {}", file.name()));
        };
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
/// its listing is checked; [`check_tree`] then refuses links and strays.
fn untar(bytes: &[u8], dest: &Path, p: &Platform) -> Result<(), String> {
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
    if let Some(bad) = listing.lines().find(|l| !entry_ok(l, p)) {
        return Err(format!("the release archive has an unexpected path {bad}"));
    }
    run(&["-xzf".as_ref(), archive.as_os_str(), "-C".as_ref(), dest.as_os_str()])?;
    let _ = std::fs::remove_file(&archive);
    Ok(())
}

/// Remove every version folder except `keep`, leaving the driver state. A
/// running driver keeps its folder locked on Windows; that one goes on the
/// next update.
fn remove_others(root: &Path, keep: &str) {
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name != keep && parse_version(&name).is_some() && entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Remove the plugin: every version and the driver's state.
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

/// Download `url`, stopping as soon as more than `size` bytes arrive.
fn download(url: &str, size: u64) -> Result<Vec<u8>, String> {
    let limit = size.min(MAX_BYTES);
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
        let ver = self.driver_version.as_deref().map(|v| format!("cua-driver {v} · ")).unwrap_or_default();
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
                body: "Lets CuaDriver read windows and press buttons. Turn on CuaDriver in Privacy & Security › Accessibility. If it already shows on but the checks fail, turn it off and on again: a driver update changes its signature.",
                action: Some((
                    "Open Accessibility",
                    StepAction::Open("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"),
                )),
            },
            Step {
                title: "Screen Recording",
                body: "Lets CuaDriver take window screenshots when the accessibility tree is not enough. Turn on CuaDriver in Privacy & Security › Screen & System Audio Recording.",
                action: Some((
                    "Open Screen Recording",
                    StepAction::Open("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"),
                )),
            },
        ]
    } else if cfg!(windows) {
        &[Step {
            title: "Same integrity level",
            body: "cua-driver runs as you, at the same integrity level as Dove. Windows stops it from reading or controlling apps started as administrator (elevated). Run those apps normally, or start Dove as administrator only when you need to automate them. Its UIA helper runs from Dove's per-user folder without UIAccess, so it cannot type into elevated windows either. UAC prompts always stay with you.",
            action: None,
        }]
    } else {
        &[
            Step {
                title: "Accessibility bus (AT-SPI)",
                body: "cua-driver reads apps through AT-SPI. GNOME and KDE turn it on by default; elsewhere enable it, then restart the apps you want to automate.",
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

    #[test]
    fn never_control_list() {
        let list = vec!["1Password".to_string(), "keepass.exe".to_string()];
        assert!(denied(r"C:\Program Files\1Password\1Password.exe", &list));
        assert!(denied("KeePass", &list));
        assert!(!denied("notepad.exe", &list));
        assert!(!denied("", &list));
        let args = serde_json::json!({"steps": [{"tool": "click", "args": {"app": "1Password"}}]});
        assert!(refusal(&args, &list).is_some());
        assert!(refusal(&serde_json::json!({"app": "Notepad"}), &list).is_none());
        assert!(refusal(&args, &[]).is_none());
        let (names, pids) = targets(&serde_json::json!({"pid": 42, "x": {"app_name": "Mail"}}));
        assert_eq!((names, pids), (vec!["Mail".to_string()], vec![42]));
    }
    use std::io::Write as _;

    const EXE: &str = if cfg!(windows) { "cua-driver.exe" } else { "cua-driver" };
    const UIA: &str = "cua-driver-uia.exe";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dove-cu-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(path: &str, bytes: &[u8], role: &str) -> FileEntry {
        FileEntry {
            path: path.into(),
            sha256: sha256_hex(bytes),
            size: bytes.len() as u64,
            executable: role == "driver" || role == "uia-sidecar",
            role: role.into(),
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

    const ROOT: &str = "cua-driver-rs-0.38.0-windows-x86_64";

    /// A release archive in R0.6's layout (one top folder, driver, UIA
    /// sidecar, a notice) and its manifest entry.
    fn release() -> (Platform, Vec<u8>) {
        let files: [(&str, &[u8], &str); 3] =
            [(EXE, b"driver", "driver"), (UIA, b"uia", "uia-sidecar"), ("LICENSE", b"MIT", "license")];
        let zipped: Vec<(String, &[u8])> = files.iter().map(|(p, b, _)| (format!("{ROOT}/{p}"), *b)).collect();
        let bytes = zip_of(&zipped.iter().map(|(p, b)| (p.as_str(), *b)).collect::<Vec<_>>());
        let p = Platform {
            name: format!("{ROOT}.zip"),
            url: format!("https://github.com/trycua/cua/releases/download/cua-driver-rs-v0.38.0/{ROOT}.zip"),
            sha256: sha256_hex(&bytes),
            size: bytes.len() as u64,
            archive: "zip".into(),
            root: ROOT.into(),
            entrypoint: EXE.into(),
            files: files.iter().map(|(p, b, r)| entry(p, b, r)).collect(),
        };
        (p, bytes)
    }

    /// A one-response-per-connection HTTP server over `dir`: the fake release.
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
        assert_eq!(m.manifest_version, 1);
        assert!(m.repository.contains('/'));
        assert_eq!(m.tag, format!("cua-driver-rs-v{}", m.version));
        assert_eq!(m.repository, "trycua/cua");
        // Every listed platform is well formed; a bad entry would silently hide it.
        assert!(m.platforms.values().all(Platform::valid), "{:?}", m.platforms);
        assert!(m.dove_supported());
        // All five platforms ship, each from trycua's tag, with its driver
        // (inside the signed CuaDriver.app on macOS, for its TCC identity).
        for key in ["windows-x86_64", "windows-arm64", "macos-universal", "linux-x86_64", "linux-arm64"] {
            let p = m.platform(key).unwrap_or_else(|| panic!("{key} missing"));
            assert!(p.url.starts_with(&format!("https://github.com/trycua/cua/releases/download/{}/", m.tag)), "{key}");
            assert!(p.files.iter().any(|f| f.path == p.entrypoint && f.role == "driver"), "{key}");
        }
        assert_eq!(m.platform("macos-universal").unwrap().entrypoint, "CuaDriver.app/Contents/MacOS/cua-driver");
        let win = m.platform("windows-x86_64").unwrap();
        assert!(win.launch_files().iter().any(|f| f.path == "cua-driver-uia.exe"), "the UIA sidecar is re-hashed too");
    }

    #[test]
    fn reads_a_release_manifest_and_refuses_bad_entries() {
        let (p, _) = release();
        let text = serde_json::json!({
            "manifest_version": 1, "name": "cua-driver", "version": "0.38.0", "tag": "cua-driver-rs-v0.38.0",
            "repository": "trycua/cua", "min_dove_version": "0.1.0", "extra": {"ignored": true},
            "platforms": {
                "windows-x86_64": {"name": p.name, "url": p.url, "sha256": p.sha256, "size": p.size,
                    "archive": "zip", "root": ROOT, "entrypoint": EXE, "files": p.files},
                "linux-arm64": {"name": "../x.tar.gz", "sha256": p.sha256, "size": 1, "archive": "tar.gz",
                    "root": "r", "entrypoint": "cua-driver", "files": []}
            }
        })
        .to_string();
        let m = Manifest::parse(&text).unwrap();
        assert_eq!(m.platform("windows-x86_64"), Some(&p));
        assert!(m.platform("linux-arm64").is_none(), "a path in the name is refused");
        assert!(m.platform("macos-universal").is_none());
        assert_eq!(m.url(&p.name, &p.url), p.url);
        assert!(Manifest::parse(&text.replace("\"manifest_version\":1", "\"manifest_version\":2")).is_err());
        let newer = text.replace("\"min_dove_version\":\"0.1.0\"", "\"min_dove_version\":\"999.0.0\"");
        assert!(!Manifest::parse(&newer).unwrap().dove_supported());
        assert!(install_in(&scratch("too-old"), &Manifest::parse(&newer).unwrap()).is_err());
    }

    #[test]
    fn platform_keys_match_the_release() {
        let key = platform_key().unwrap();
        assert!(["windows-x86_64", "windows-arm64", "macos-universal", "linux-x86_64", "linux-arm64"].contains(&key));
    }

    #[test]
    fn installs_only_verified_archives_and_replaces_old_versions() {
        let root = scratch("bytes");
        let (p, bytes) = release();
        // A tampered or truncated download installs nothing.
        let mut evil = bytes.clone();
        evil[10] ^= 1;
        assert!(install_bytes(&root, "0.36.0", &p, &evil).is_err());
        assert!(install_bytes(&root, "0.36.0", &p, &bytes[..bytes.len() - 1]).is_err());
        assert!(installed_in(&root).is_none());
        let old = install_bytes(&root, "0.36.0", &p, &bytes).unwrap();
        assert_eq!(std::fs::read(&old.bin).unwrap(), b"driver");
        assert!(old.dir().join(UIA).is_file(), "the UIA sidecar sits next to the driver");
        assert_eq!(installed_in(&root), Some(old.clone()));
        verify_installed(&old).unwrap();
        std::fs::create_dir_all(root.join(STATE_DIR)).unwrap();
        install_bytes(&root, "0.37.0", &p, &bytes).unwrap();
        let new = installed_in(&root).unwrap();
        assert_eq!(new.version, "0.37.0");
        assert!(!old.bin.exists(), "the update removes the old version");
        assert!(root.join(STATE_DIR).is_dir(), "the update keeps the driver's state");
        // The pre-launch check catches a changed driver or sidecar.
        std::fs::write(new.dir().join(UIA), b"swapped").unwrap();
        assert!(verify_installed(&new).unwrap_err().contains(UIA));
        uninstall_in(&root).unwrap();
        assert!(installed_in(&root).is_none() && !root.exists());
    }

    #[test]
    fn archives_that_differ_from_the_manifest_are_refused() {
        let root = scratch("strict");
        let (p, _) = release();
        let check = |files: &[(&str, &[u8])]| {
            let bytes = zip_of(files);
            let p = Platform { sha256: sha256_hex(&bytes), size: bytes.len() as u64, ..p.clone() };
            install_bytes(&root, "0.37.0", &p, &bytes)
        };
        let exe = format!("{ROOT}/{EXE}");
        let uia = format!("{ROOT}/{UIA}");
        let lic = format!("{ROOT}/LICENSE");
        // Outside root, `..`, an unlisted file, a missing file, a changed file.
        assert!(check(&[("../evil.exe", b"x"), (&exe, b"driver")]).is_err());
        assert!(check(&[("other/cua-driver.exe", b"driver")]).is_err());
        assert!(
            check(&[(&exe, b"driver"), (&uia, b"uia"), (&lic, b"MIT"), (&format!("{ROOT}/extra.dll"), b"x")])
                .unwrap_err()
                .contains("unlisted")
        );
        assert!(check(&[(&exe, b"driver"), (&lic, b"MIT")]).unwrap_err().contains("missing"));
        assert!(check(&[(&exe, b"driver"), (&uia, b"evil"), (&lic, b"MIT")]).is_err());
        assert!(installed_in(&root).is_none());
        assert!(check(&[(&exe, b"driver"), (&uia, b"uia"), (&lic, b"MIT")]).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn installs_from_a_fake_release_server() {
        let release_dir = scratch("release");
        let root = scratch("root");
        let (p, bytes) = release();
        std::fs::write(release_dir.join(&p.name), &bytes).unwrap();
        let base = serve(release_dir);
        let got =
            install_bytes(&root, "0.37.0", &p, &download(&format!("{base}/{}", p.name), p.size).unwrap()).unwrap();
        assert_eq!(installed_in(&root), Some(got));
        // The manifest's size caps the download; a missing file fails.
        assert!(download(&format!("{base}/{}", p.name), 3).is_err());
        assert!(download(&format!("{base}/missing.zip"), 10).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn launch_env_isolates_state_and_never_phones_home() {
        let root = PathBuf::from("plugins").join("computer-use");
        let env = launch_env_in(&root);
        assert_eq!(env["CUA_DRIVER_RS_HOME"], root.join(STATE_DIR).display().to_string());
        assert_eq!(env["CUA_DRIVER_PERMISSION_MODE"], "standard");
        assert_eq!(env["CUA_DRIVER_RS_TELEMETRY_ENABLED"], "0");
        assert_eq!(env["DO_NOT_TRACK"], "1");
        assert_eq!(env["CUA_DRIVER_RS_UPDATE_CHECK"], "0");
    }

    #[test]
    fn settings_read_older_files() {
        let s: Settings = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(s.enabled && s.ranker && s.approvals == "ask" && !s.raw_tools, "older files get today's defaults");
        let s: Settings =
            serde_json::from_str(r#"{"enabled":true,"permission_mode":"bounded","grounder":true,"approvals":"auto"}"#).unwrap();
        assert!(s.enabled && s.auto_background(), "stray old fields are ignored");
        let s = Settings { denied_apps: vec!["1Password".into(), "signer:AgileBits".into(), " ".into()], ..Settings::default() };
        assert_eq!(s.rules().len(), 2);
    }

    #[test]
    fn health_report_summary() {
        let ok = serde_json::json!({"schema_version":"1","overall":"ok","driver_version":"0.37.0",
            "checks":[{"name":"binary_version","status":"pass","message":"v"},{"name":"ax","status":"pass","message":"a"}]});
        let h = Health::from_report(&ok);
        assert!(h.ok());
        assert_eq!(h.summary(), "cua-driver 0.37.0 · Ready: all 2 checks passed");
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
        assert!(parse_version("0.37.0") > parse_version("0.37.0-dev"));
        assert!(parse_version("0.37.0-dev") > parse_version("0.36.9"));
        assert_eq!(parse_version("0.37.0-"), None);
        assert_eq!(parse_version(STATE_DIR), None);
        let i = Installed { version: "0.1.0".into(), bin: PathBuf::from(EXE) };
        assert!(i.outdated() || version().is_empty());
    }

    #[test]
    fn every_os_has_a_walkthrough() {
        assert!(!walkthrough().is_empty());
        assert!(walkthrough().iter().all(|s| !s.title.is_empty() && !s.body.is_empty()));
    }

    /// A `dev-install.py` install (no `plugin.json`, the verified manifest
    /// beside the files, a `-dev` version) is found, checks out, and is
    /// refused once a driver file changes.
    #[test]
    fn finds_and_verifies_a_dev_install() {
        let root = scratch("dev-install");
        let dir = root.join("0.37.0-dev");
        std::fs::create_dir_all(&dir).unwrap();
        let (p, _) = release();
        std::fs::write(dir.join(EXE), b"driver").unwrap();
        std::fs::write(dir.join(UIA), b"uia").unwrap();
        std::fs::write(dir.join("LICENSE"), b"MIT").unwrap();
        let key = platform_key().expect("a supported platform");
        let manifest = serde_json::json!({
            "manifest_version": 1, "name": "dove-driver", "version": "0.37.0-dev",
            "tag": "dove-driver-v0.37.0-dev", "repository": "o/r", "min_dove_version": "0.1.0",
            "platforms": {key: {
                "name": p.name, "url": p.url, "sha256": p.sha256, "size": p.size, "archive": p.archive,
                "root": p.root, "entrypoint": p.entrypoint, "files": p.files,
            }},
        });
        std::fs::write(dir.join(DEV_MANIFEST), manifest.to_string()).unwrap();
        let i = installed_in(&root).expect("dev install found");
        assert_eq!(i.version, "0.37.0-dev");
        verify_installed(&i).unwrap();
        std::fs::write(dir.join(EXE), b"tampered").unwrap();
        assert!(verify_installed(&i).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// QA against a fake release served locally: installs the release that
    /// `DOVE_COMPUTER_USE_MANIFEST` describes from `DOVE_COMPUTER_USE_RELEASE_BASE`
    /// into `DOVE_CU_QA_ROOT`, then runs the pre-launch check. Run with
    /// `--ignored qa_fake_release` and those three variables set.
    #[test]
    #[ignore]
    fn qa_fake_release() {
        let root = PathBuf::from(std::env::var("DOVE_CU_QA_ROOT").expect("DOVE_CU_QA_ROOT"));
        let m = manifest();
        assert!(available(), "no release for this platform in the QA manifest");
        let i = install_in(&root, m).unwrap();
        verify_installed(&i).unwrap();
        eprintln!("installed {} at {}", i.version, i.bin.display());
    }
}
