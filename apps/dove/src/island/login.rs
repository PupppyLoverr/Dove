//! Start the island at login (on by default; the tray and Settings › Notch
//! turn it off). Windows uses the user's Run key.
//!
//! Only an installed Dove registers itself: a development build (an exe
//! under a cargo `target` folder) or a run on another data folder
//! (`DOVE_HOME`, QA copies) never touches the Run key, unless
//! `DOVE_ISLAND_LOGIN=1` asks for it.

use dove_core::store::Store;

/// The setting (bool, default on).
pub const KEY: &str = "island_login";

#[cfg(windows)]
const VALUE: &str = "Dove Island";

/// Whether this copy of Dove may register itself.
pub fn allowed() -> bool {
    if std::env::var("DOVE_ISLAND_LOGIN").ok().as_deref() == Some("1") {
        return true;
    }
    if std::env::var_os("DOVE_HOME").is_some() {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    !exe.components().any(|c| c.as_os_str().to_string_lossy().starts_with("target"))
}

/// Registers or unregisters the island to match the setting.
pub fn apply(store: &Store) {
    if !allowed() {
        return;
    }
    set(store.flag(KEY, true));
}

#[cfg(windows)]
fn set(on: bool) {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW};
    let subkey: Vec<u16> = r"Software\Microsoft\Windows\CurrentVersion\Run".encode_utf16().chain(Some(0)).collect();
    let name: Vec<u16> = VALUE.encode_utf16().chain(Some(0)).collect();
    // SAFETY: NUL-terminated strings that outlive each call; the data length
    // is the string's byte length including its NUL.
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else { return };
            let command = format!("\"{}\" --island", exe.display());
            let data: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            );
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey.as_ptr(), name.as_ptr());
        }
    }
}

#[cfg(not(windows))]
fn set(_: bool) {
    // macOS (SMAppService) and Linux (XDG autostart) come with their ports.
}
