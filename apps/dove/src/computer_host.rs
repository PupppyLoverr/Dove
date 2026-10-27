//! What the `computer` tool asks of Dove, from inside `dove --mcp-browser`
//! (the per-agent MCP server): the user's approvals, the take-over lease
//! and the activity line go to the app over its loopback endpoint; process
//! identity, password-field checks, OCR and idle time come from the OS.

use std::path::PathBuf;

use dove_computer::{AppIdentity, Decision, Host, Lease, OcrLine, Review};
use serde_json::{Value, json};

/// The app's GUI tools the hub calls (handled in `computer_review.rs`).
pub const REVIEW: &str = "computer_review";
pub const LEASE: &str = "computer_lease";
pub const ACTIVITY: &str = "computer_activity";

pub struct AppHost {
    cache: PathBuf,
}

impl AppHost {
    pub fn new() -> Self {
        let data = std::env::var_os("DOVE_DATA").map(PathBuf::from).unwrap_or_else(crate::pages::integrations::data_root);
        Self { cache: data.join("cache").join("computer") }
    }

    /// Whether this server can reach a Dove window at all.
    fn app() -> bool {
        std::env::var_os("DOVE_BROWSER_PORT").is_some()
    }
}

impl Host for AppHost {
    fn review(&mut self, review: &Review) -> Decision {
        if !Self::app() {
            return Decision::Unavailable { reason: "no Dove window is attached to this agent".into() };
        }
        let args = serde_json::to_value(review).unwrap_or(Value::Null);
        match crate::mcp_browser::forward(REVIEW, args) {
            Ok(v) if v.get("isError").and_then(Value::as_bool) != Some(true) => {
                let text = v.get("text").and_then(Value::as_str).unwrap_or("");
                serde_json::from_str::<Decision>(text)
                    .unwrap_or(Decision::Unavailable { reason: format!("Dove answered {text:?}") })
            }
            Ok(v) => Decision::Unavailable {
                reason: v.get("text").and_then(Value::as_str).unwrap_or("Dove refused the request").to_string(),
            },
            Err(e) => Decision::Unavailable { reason: e },
        }
    }

    fn lease(&mut self) -> Lease {
        if !Self::app() {
            return Lease::default();
        }
        crate::mcp_browser::forward(LEASE, json!({}))
            .ok()
            .and_then(|v| v.get("text").and_then(Value::as_str).and_then(|t| serde_json::from_str(t).ok()))
            .unwrap_or_default()
    }

    fn identify(&mut self, pid: u32, app: &str) -> AppIdentity {
        identify(pid, app)
    }

    fn secure_target(&mut self, _pid: u32, window_id: u64, label: Option<&str>) -> Option<bool> {
        dove_computer::native::secure_target(window_id, label)
    }

    fn ocr(&mut self, image: &[u8]) -> Result<Vec<OcrLine>, String> {
        dove_computer::native::ocr(image)
    }

    fn idle_ms(&mut self) -> Option<u64> {
        idle_ms()
    }

    fn activity(&mut self, app: &str, summary: &str) {
        if Self::app() {
            let _ = crate::mcp_browser::forward(ACTIVITY, json!({"app": app, "summary": summary}));
        }
    }

    fn cache_dir(&self) -> PathBuf {
        self.cache.clone()
    }
}

/// What process `pid` is: name and path everywhere, plus the verified code
/// signer (Windows Authenticode subject, macOS Developer ID and Team ID)
/// and macOS bundle id.
pub fn identify(pid: u32, app: &str) -> AppIdentity {
    let path = crate::computer_plugin::process_name(pid).unwrap_or_default();
    let mut id = AppIdentity { name: app.to_string(), path: path.clone(), ..AppIdentity::default() };
    if path.is_empty() {
        return id;
    }
    #[cfg(windows)]
    {
        if let Some((signer, signed)) = authenticode_signer(&path) {
            id.signer = signer;
            id.signed = signed;
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some((bundle, signer, signed)) = codesign(&path) {
            id.bundle_id = bundle;
            id.signer = signer;
            id.signed = signed;
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) {
            id.path = exe.display().to_string();
        }
    }
    id
}

/// The embedded Authenticode signature's signer and whether it verifies.
/// Catalog-signed system binaries have no embedded signature: they report
/// unsigned, and name or path rules still cover them.
#[cfg(windows)]
fn authenticode_signer(path: &str) -> Option<(String, bool)> {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Security::Cryptography::{CERT_NAME_SIMPLE_DISPLAY_TYPE, CertGetNameStringW};
    use windows_sys::Win32::Security::WinTrust::{
        WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_FILE_INFO, WTD_CACHE_ONLY_URL_RETRIEVAL,
        WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
        WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust,
    };
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: WinVerifyTrust reads the structures built here for the length
    // of the calls; the state handle is closed before returning.
    unsafe {
        let mut file: WINTRUST_FILE_INFO = std::mem::zeroed();
        file.cbStruct = std::mem::size_of::<WINTRUST_FILE_INFO>() as u32;
        file.pcwszFilePath = wide.as_ptr();
        let mut data: WINTRUST_DATA = std::mem::zeroed();
        data.cbStruct = std::mem::size_of::<WINTRUST_DATA>() as u32;
        data.dwUIChoice = WTD_UI_NONE;
        data.fdwRevocationChecks = WTD_REVOKE_NONE;
        data.dwUnionChoice = WTD_CHOICE_FILE;
        data.Anonymous.pFile = &mut file;
        data.dwStateAction = WTD_STATEACTION_VERIFY;
        data.dwProvFlags = WTD_CACHE_ONLY_URL_RETRIEVAL;
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let hwnd = INVALID_HANDLE_VALUE as _;
        let status = WinVerifyTrust(hwnd, &mut action, (&mut data as *mut WINTRUST_DATA).cast());
        let mut signer = String::new();
        let prov = WTHelperProvDataFromStateData(data.hWVTStateData);
        if !prov.is_null() {
            let sgnr = WTHelperGetProvSignerFromChain(prov, 0, 0, 0);
            if !sgnr.is_null() && (*sgnr).csCertChain > 0 && !(*sgnr).pasCertChain.is_null() {
                let cert = (*(*sgnr).pasCertChain).pCert;
                if !cert.is_null() {
                    let mut buf = [0u16; 512];
                    let n = CertGetNameStringW(cert, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, std::ptr::null(), buf.as_mut_ptr(), buf.len() as u32);
                    if n > 1 {
                        signer = String::from_utf16_lossy(&buf[..(n - 1) as usize]);
                    }
                }
            }
        }
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        WinVerifyTrust(hwnd, &mut action, (&mut data as *mut WINTRUST_DATA).cast());
        (!signer.is_empty() || status == 0).then_some((signer, status == 0))
    }
}

/// `codesign -dv`: bundle identifier, signer (Authority and Team ID) and
/// whether the signature checks out (`codesign --verify`).
#[cfg(target_os = "macos")]
fn codesign(path: &str) -> Option<(String, String, bool)> {
    let out = std::process::Command::new("/usr/bin/codesign").args(["-dv", "--verbose=2", path]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)).map(|v| v.trim().to_string());
    let bundle = field("Identifier=").unwrap_or_default();
    let authority = field("Authority=").unwrap_or_default();
    let team = field("TeamIdentifier=").filter(|t| t != "not set").unwrap_or_default();
    let verified = std::process::Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict", path])
        .output()
        .is_ok_and(|o| o.status.success());
    let signer = [authority, team].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    Some((bundle, signer, verified && out.status.success()))
}

/// Milliseconds since the last real mouse or keyboard input on this seat.
fn idle_ms() -> Option<u64> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::GetTickCount;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
        // SAFETY: GetLastInputInfo fills the struct it is given.
        unsafe {
            let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
            (GetLastInputInfo(&mut info) != 0).then(|| u64::from(GetTickCount().wrapping_sub(info.dwTime)))
        }
    }
    #[cfg(target_os = "macos")]
    {
        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            fn CGEventSourceSecondsSinceLastEventType(state: i32, event: u32) -> f64;
        }
        // kCGEventSourceStateHIDSystemState (1), kCGAnyInputEventType (~0):
        // hardware input only, so the driver's own synthetic events do not count.
        // SAFETY: a pure query with constant arguments.
        let s = unsafe { CGEventSourceSecondsSinceLastEventType(1, u32::MAX) };
        (s.is_finite() && s >= 0.0).then(|| (s * 1000.0) as u64)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_identifies_itself() {
        let me = std::process::id();
        let id = identify(me, "dove-test");
        assert_eq!(id.name, "dove-test");
        assert!(!id.path.is_empty(), "the test binary's own path is readable");
    }

    #[cfg(windows)]
    #[test]
    fn a_signed_system_binary_reports_its_signer() {
        // explorer.exe carries an embedded Microsoft signature.
        let explorer = std::path::Path::new(&std::env::var("WINDIR").unwrap_or("C:\\Windows".into())).join("explorer.exe");
        let (signer, signed) = authenticode_signer(&explorer.display().to_string()).expect("explorer is signed");
        assert!(signed);
        assert!(signer.contains("Microsoft"), "{signer}");
    }

    #[test]
    fn idle_time_is_readable_or_absent() {
        if let Some(ms) = idle_ms() {
            assert!(ms < 365 * 24 * 3600 * 1000);
        }
    }
}
