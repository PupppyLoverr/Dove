//! Apps and windows on this computer, for `@`-mentioning one in the
//! composer. On Windows the list comes straight from the window manager
//! (no driver needed); elsewhere it comes from Dove Computer Use's driver
//! when that plugin is on.

use std::time::{Duration, Instant};

/// One top-level window the user could mean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// The app, from its executable name ("Notepad", "Code").
    pub app: String,
    pub title: String,
    pub pid: u32,
    /// The window's id: an HWND on Windows, the driver's id elsewhere.
    pub id: u64,
}

/// The windows worth offering, front to back, with Dove's own left out.
pub fn windows() -> Vec<Window> {
    #[cfg(windows)]
    {
        windows_native()
    }
    #[cfg(not(windows))]
    {
        driver_windows()
    }
}

/// [`windows`], at most once every few seconds (the menu asks on every key).
pub fn windows_cached() -> Vec<Window> {
    use std::sync::Mutex;
    static CACHE: Mutex<Option<(Instant, Vec<Window>)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, list)) = c.as_ref()
        && at.elapsed() < Duration::from_secs(4)
    {
        return list.clone();
    }
    let list = windows();
    *c = Some((Instant::now(), list.clone()));
    list
}

/// "Notepad" from `C:\…\notepad.exe`; well-known apps by their own names.
fn app_name(exe: &str) -> String {
    let stem = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
    let stem = stem.strip_suffix(".exe").or_else(|| stem.strip_suffix(".EXE")).unwrap_or(stem);
    const KNOWN: &[(&str, &str)] = &[
        ("msedge", "Microsoft Edge"),
        ("chrome", "Google Chrome"),
        ("firefox", "Firefox"),
        ("brave", "Brave"),
        ("code", "VS Code"),
        ("windowsterminal", "Windows Terminal"),
        ("explorer", "File Explorer"),
        ("winword", "Word"),
        ("excel", "Excel"),
        ("powerpnt", "PowerPoint"),
        ("olk", "Outlook"),
        ("outlook", "Outlook"),
        ("ms-teams", "Teams"),
    ];
    if let Some((_, name)) = KNOWN.iter().find(|(k, _)| stem.eq_ignore_ascii_case(k)) {
        return (*name).to_string();
    }
    let mut c = stem.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Shell and system surfaces that are windows but not apps anyone means.
fn is_system(title: &str, app: &str) -> bool {
    const TITLES: &[&str] =
        &["Program Manager", "Windows Input Experience", "Settings", "Microsoft Text Input Application"];
    const APPS: &[&str] =
        &["File Explorer", "TextInputHost", "ShellExperienceHost", "StartMenuExperienceHost", "SearchHost"];
    (TITLES.contains(&title) && app != "Notepad") || (APPS.contains(&app) && title.is_empty())
}

#[cfg(windows)]
fn windows_native() -> Vec<Window> {
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GWL_EXSTYLE, GetWindow, GetWindowLongW, GetWindowRect, GetWindowTextLengthW,
        GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, WS_EX_TOOLWINDOW,
    };

    unsafe extern "system" fn each(hwnd: HWND, out: LPARAM) -> i32 {
        unsafe {
            let out = &mut *(out as *mut Vec<(isize, u32, String)>);
            if IsWindowVisible(hwnd) == 0 || !GetWindow(hwnd, GW_OWNER).is_null() {
                return 1;
            }
            if (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32) & WS_EX_TOOLWINDOW != 0 {
                return 1;
            }
            // Suspended UWP apps keep invisible ("cloaked") windows.
            let mut cloaked = 0u32;
            DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, (&mut cloaked as *mut u32).cast(), 4);
            if cloaked != 0 {
                return 1;
            }
            let mut r: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut r) == 0 || r.right - r.left < 80 || r.bottom - r.top < 40 {
                return 1;
            }
            let n = GetWindowTextLengthW(hwnd);
            if n <= 0 {
                return 1;
            }
            let mut buf = vec![0u16; n as usize + 1];
            let got = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            let title = String::from_utf16_lossy(&buf[..got.max(0) as usize]);
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            out.push((hwnd as isize, pid, title));
            1
        }
    }

    let mut found: Vec<(isize, u32, String)> = Vec::new();
    unsafe { EnumWindows(Some(each), &mut found as *mut _ as LPARAM) };
    let own = std::process::id();
    let exe_of = |pid: u32| -> String {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return String::new();
            }
            let mut buf = [0u16; 520];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
            CloseHandle(h);
            if ok == 0 { String::new() } else { String::from_utf16_lossy(&buf[..len as usize]) }
        }
    };
    found
        .into_iter()
        .filter(|(_, pid, title)| *pid != own && !title.trim().is_empty())
        .map(|(hwnd, pid, title)| Window { app: app_name(&exe_of(pid)), title, pid, id: hwnd as u64 })
        .filter(|w| !is_system(&w.title, &w.app))
        .collect()
}

/// Windows from Dove Computer Use's driver (`list_windows`), when it is on.
#[cfg(not(windows))]
fn driver_windows() -> Vec<Window> {
    let config = crate::pages::integrations::load();
    if !config.computer.enabled {
        return Vec::new();
    }
    let Some(bin) = crate::pages::integrations::cua_driver() else { return Vec::new() };
    let reply = crate::mcp_browser::driver_call(
        &bin,
        &config.computer,
        "list_windows",
        serde_json::json!({}),
        Duration::from_secs(8),
    )
    .ok();
    let Some(list) = reply
        .as_ref()
        .and_then(|v| v.get("structuredContent").and_then(|s| s.get("windows")).and_then(|w| w.as_array()).cloned())
    else {
        return Vec::new();
    };
    let own = std::process::id();
    list.iter()
        .filter_map(|w| {
            Some(Window {
                app: w.get("app_name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                title: w.get("title").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                pid: w.get("pid").and_then(|v| v.as_u64())? as u32,
                id: w.get("window_id").and_then(|v| v.as_u64())?,
            })
        })
        .filter(|w| w.pid != own && !w.title.trim().is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_names_read_like_apps() {
        assert_eq!(app_name(r"C:\Windows\System32\notepad.exe"), "Notepad");
        assert_eq!(app_name("/Applications/Safari.app/Contents/MacOS/Safari"), "Safari");
        assert_eq!(app_name(""), "");
        assert_eq!(app_name(r"C:\Program Files\Edge\msedge.exe"), "Microsoft Edge");
        assert!(is_system("Program Manager", "File Explorer"));
        assert!(!is_system("notes.txt - Notepad", "Notepad"));
    }

    #[cfg(windows)]
    #[test]
    fn lists_without_dove_and_without_panicking() {
        let own = std::process::id();
        assert!(windows().iter().all(|w| w.pid != own && !w.title.is_empty()));
    }
}
