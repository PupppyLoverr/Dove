//! ego lite detection: the app, its version, the `ego-browser` command it
//! registers during onboarding, and the runtime check ego lite documents
//! (`ego-browser nodejs -e "console.log('ego-browser ready')"`).
//!
//! Dove never installs or upgrades ego lite; it only finds it and says what
//! is missing.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The ego lite app on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub path: PathBuf,
    pub version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Health {
    Ready,
    Failed(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub app: Option<App>,
    /// The `ego-browser` command, when onboarding registered it.
    pub cli: Option<PathBuf>,
    /// ego lite's pointer to its active version names a folder that is gone:
    /// the app was uninstalled or moved.
    pub stale: Option<PathBuf>,
}

impl Status {
    pub fn version(&self) -> Option<&str> {
        self.app.as_ref().and_then(|a| a.version.as_deref())
    }
}

/// `0.6.0.16` and friends: digits and dots, at least two parts.
fn looks_like_version(name: &str) -> bool {
    let parts: Vec<&str> = name.split('.').collect();
    parts.len() >= 2 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Highest version-named folder inside `dir` (ego lite's `Application\<ver>`).
fn newest_version_dir(dir: &Path) -> Option<PathBuf> {
    let mut found: Vec<(Vec<u64>, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            looks_like_version(&name).then(|| (name.split('.').map(|p| p.parse().unwrap_or(0)).collect(), e.path()))
        })
        .collect();
    found.sort();
    found.pop().map(|(_, p)| p)
}

/// `CFBundleShortVersionString` from a macOS app's Info.plist (XML form).
fn plist_version(plist: &str) -> Option<String> {
    let key = plist.find("<key>CFBundleShortVersionString</key>")?;
    let rest = &plist[key..];
    let start = rest.find("<string>")? + "<string>".len();
    let end = rest[start..].find("</string>")? + start;
    Some(rest[start..end].trim().to_string()).filter(|v| !v.is_empty())
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Where ego lite is installed, and a dangling active-version pointer.
pub fn find_app(home: Option<&Path>) -> (Option<App>, Option<PathBuf>) {
    let mut stale = None;
    // ego lite keeps `~/.local/share/ego/active_version_dir` pointing at the
    // version it runs.
    if let Some(home) = home {
        let link = home.join(".local").join("share").join("ego").join("active_version_dir");
        if let Ok(target) = std::fs::read_link(&link) {
            if target.is_dir() {
                let version =
                    target.file_name().map(|n| n.to_string_lossy().to_string()).filter(|n| looks_like_version(n));
                return (Some(App { path: target, version }), None);
            }
            stale = Some(target);
        }
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var).map(PathBuf::from) {
                roots.push(base.join("Citro Labs").join("ego lite").join("Application"));
                roots.push(base.join("Programs").join("Citro Labs").join("ego lite").join("Application"));
            }
        }
        for root in roots {
            if let Some(dir) = newest_version_dir(&root) {
                let version = dir.file_name().map(|n| n.to_string_lossy().to_string());
                return (Some(App { path: dir, version }), stale);
            }
        }
    } else if cfg!(target_os = "macos") {
        let mut apps = vec![PathBuf::from("/Applications/ego lite.app")];
        if let Some(home) = home {
            apps.push(home.join("Applications").join("ego lite.app"));
        }
        for app in apps {
            if app.is_dir() {
                let version = std::fs::read_to_string(app.join("Contents").join("Info.plist"))
                    .ok()
                    .and_then(|p| plist_version(&p));
                return (Some(App { path: app, version }), stale);
            }
        }
    }
    (None, stale)
}

/// Everything that can be found without running anything.
pub fn detect() -> Status {
    let (app, stale) = find_app(home().as_deref());
    Status { app, cli: dove_core::harness_process::resolve("ego-browser"), stale }
}

/// ego lite's documented runtime check, with stdin closed (an open stdin
/// makes `ego-browser nodejs` wait) and a deadline.
pub fn health(cli: &Path, timeout: Duration) -> Health {
    let mut cmd = std::process::Command::new(cli);
    cmd.args(["nodejs", "-e", "console.log('ego-browser ready')"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Health::Failed(format!("could not run ego-browser: {e}")),
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Health::Failed(format!("no answer within {} s", timeout.as_secs()));
            }
            Err(e) => return Health::Failed(e.to_string()),
        }
    }
    match child.wait_with_output() {
        Ok(out) => {
            judge(&String::from_utf8_lossy(&out.stdout), &String::from_utf8_lossy(&out.stderr), out.status.success())
        }
        Err(e) => Health::Failed(e.to_string()),
    }
}

/// Ready only when the runtime printed the line ego lite promises. ego
/// lite 0.6 relays the script's console to stderr, so either stream counts.
fn judge(stdout: &str, stderr: &str, ok: bool) -> Health {
    if ok && stdout.lines().chain(stderr.lines()).any(|l| l.trim() == "ego-browser ready") {
        return Health::Ready;
    }
    let why = stderr.lines().chain(stdout.lines()).map(str::trim).find(|l| !l.is_empty()).unwrap_or(if ok {
        "it ran but did not print \"ego-browser ready\""
    } else {
        "it exited with an error"
    });
    Health::Failed(why.chars().take(200).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_dotted_numbers() {
        assert!(looks_like_version("0.6.0.16"));
        assert!(looks_like_version("1.2"));
        assert!(!looks_like_version("SetupMetrics"));
        assert!(!looks_like_version("0.6.x"));
        assert!(!looks_like_version("7"));
    }

    #[test]
    fn the_newest_version_folder_wins() {
        let dir = std::env::temp_dir().join(format!("dove-ego-{}", std::process::id()));
        for v in ["0.6.0.9", "0.6.0.16", "0.5.9.99", "SetupMetrics"] {
            std::fs::create_dir_all(dir.join(v)).unwrap();
        }
        assert_eq!(newest_version_dir(&dir).unwrap().file_name().unwrap(), "0.6.0.16");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plist_versions_parse() {
        let plist = "<dict><key>CFBundleName</key><string>ego lite</string>\
                     <key>CFBundleShortVersionString</key>\n  <string>0.6.1</string></dict>";
        assert_eq!(plist_version(plist).as_deref(), Some("0.6.1"));
        assert_eq!(plist_version("<dict></dict>"), None);
    }

    #[test]
    fn health_needs_the_promised_line() {
        assert_eq!(judge("ego-browser ready\n", "", true), Health::Ready);
        // ego lite 0.6.0.16 on Windows prints it on stderr.
        assert_eq!(judge("", "ego-browser ready\r\n", true), Health::Ready);
        assert!(matches!(judge("", "ego-browser ready", false), Health::Failed(_)), "a failed run is not ready");
        assert!(
            matches!(judge("", "Error: ego lite is not running", false), Health::Failed(e) if e.contains("not running"))
        );
        assert!(matches!(judge("hello", "", true), Health::Failed(_)));
    }

    #[test]
    fn a_missing_app_folder_is_reported_as_stale() {
        let home = std::env::temp_dir().join(format!("dove-ego-home-{}", std::process::id()));
        let share = home.join(".local").join("share").join("ego");
        std::fs::create_dir_all(&share).unwrap();
        let gone = home.join("removed").join("0.6.0.16");
        let link = share.join("active_version_dir");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&gone, &link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&gone, &link).is_ok();
        if made {
            let (app, stale) = find_app(Some(&home));
            assert!(app.is_none() || app.as_ref().is_some_and(|a| a.path != gone));
            assert_eq!(stale.as_deref(), Some(gone.as_path()));
            // Restored: the pointer now finds the app and its version.
            std::fs::create_dir_all(&gone).unwrap();
            let (app, stale) = find_app(Some(&home));
            assert_eq!(app.unwrap().version.as_deref(), Some("0.6.0.16"));
            assert_eq!(stale, None);
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
