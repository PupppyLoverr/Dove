//! A system-wide shortcut for Voice Mode: start or end a call while another
//! app has focus. Windows registers it with `RegisterHotKey` on a small
//! thread of its own; the shell listens on [`events`]. Other platforms keep
//! the in-app shortcut only.

use std::sync::{Mutex, OnceLock};

/// Presses of the global shortcut.
fn channel() -> &'static (flume::Sender<()>, flume::Receiver<()>) {
    static CH: OnceLock<(flume::Sender<()>, flume::Receiver<()>)> = OnceLock::new();
    CH.get_or_init(flume::unbounded)
}

pub fn events() -> flume::Receiver<()> {
    channel().1.clone()
}

/// The thread holding the current registration: its id (to post WM_QUIT
/// to) and handle (to wait until it has unregistered).
static ACTIVE: Mutex<Option<(u32, std::thread::JoinHandle<()>)>> = Mutex::new(None);

/// Modifier bits and virtual-key code for a binding such as
/// `secondary-shift-space` (keymap syntax).
pub fn parse(binding: &str) -> Option<(u32, u32)> {
    const ALT: u32 = 0x1;
    const CONTROL: u32 = 0x2;
    const SHIFT: u32 = 0x4;
    const WIN: u32 = 0x8;
    let mut mods = 0;
    let mut key = None;
    for part in binding.split('-') {
        match part {
            "ctrl" | "secondary" => mods |= CONTROL,
            "shift" => mods |= SHIFT,
            "alt" => mods |= ALT,
            "cmd" | "platform" | "win" => mods |= WIN,
            k => key = Some(k),
        }
    }
    let vk = match key? {
        "space" => 0x20,
        "enter" => 0x0D,
        "tab" => 0x09,
        "escape" => 0x1B,
        "`" => 0xC0,
        "\\" => 0xDC,
        "," => 0xBC,
        "." => 0xBE,
        "/" => 0xBF,
        ";" => 0xBA,
        k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => {
            k.to_ascii_uppercase().as_bytes()[0] as u32
        }
        k if k.starts_with('f') && k[1..].parse::<u32>().is_ok_and(|n| (1..=24).contains(&n)) => {
            0x6F + k[1..].parse::<u32>().unwrap()
        }
        _ => return None,
    };
    // A global shortcut without a modifier would swallow a key everywhere.
    (mods != 0).then_some((mods, vk))
}

/// Register `binding` system-wide (replacing any earlier one), or drop the
/// registration with `None`.
pub fn set(binding: Option<&str>) -> Result<(), String> {
    stop();
    let result = match binding {
        None => Ok(()),
        Some(binding) => parse(binding)
            .ok_or_else(|| format!("\"{binding}\" can't be a global shortcut"))
            .and_then(|(mods, vk)| start(mods, vk)),
    };
    *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = result.clone().err();
    result
}

static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// Why the last registration failed, for Settings.
pub fn last_error() -> Option<String> {
    LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Register Voice Mode's shortcut system-wide when the setting is on.
pub fn apply(store: &dove_core::store::Store) {
    let on = store.flag("voice_global_hotkey", true);
    let binding = crate::keymap::shortcuts()
        .into_iter()
        .find(|s| s.id == "voice_call")
        .map(|s| crate::keymap::binding(store, &s));
    match (on, binding) {
        (true, Some(b)) => {
            if let Err(e) = set(Some(&b)) {
                tracing::info!("voice call shortcut outside Keplorayx: {e}");
            }
        }
        _ => {
            let _ = set(None);
        }
    }
}

#[cfg(windows)]
fn stop() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
    if let Some((thread, handle)) = ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).take() {
        unsafe {
            PostThreadMessageW(thread, WM_QUIT, 0, 0);
        }
        let _ = handle.join();
    }
}

#[cfg(windows)]
fn start(mods: u32, vk: u32) -> Result<(), String> {
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<u32, String>>();
    let tx = channel().0.clone();
    let handle = std::thread::Builder::new()
        .name("dove-hotkey".into())
        .spawn(move || unsafe {
            if RegisterHotKey(std::ptr::null_mut(), 1, mods | MOD_NOREPEAT, vk) == 0 {
                let _ = ready_tx.send(Err("another app already uses this shortcut".into()));
                return;
            }
            let _ = ready_tx.send(Ok(GetCurrentThreadId()));
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                if msg.message == WM_HOTKEY {
                    let _ = tx.send(());
                }
            }
            UnregisterHotKey(std::ptr::null_mut(), 1);
        })
        .map_err(|e| e.to_string())?;
    let thread = ready_rx.recv().map_err(|_| "the shortcut thread stopped".to_string())??;
    *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = Some((thread, handle));
    Ok(())
}

#[cfg(not(windows))]
fn stop() {
    let _ = ACTIVE.lock().map(|mut a| a.take());
}

#[cfg(not(windows))]
fn start(_mods: u32, _vk: u32) -> Result<(), String> {
    Err("A shortcut that works outside Keplorayx is available on Windows; elsewhere use it inside Keplorayx.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_map_to_modifiers_and_keys() {
        assert_eq!(parse("secondary-shift-space"), Some((0x2 | 0x4, 0x20)));
        assert_eq!(parse("ctrl-alt-v"), Some((0x2 | 0x1, b'V' as u32)));
        assert_eq!(parse("shift-f9"), Some((0x4, 0x78)));
        assert_eq!(parse("space"), None, "no modifier: never global");
        assert_eq!(parse("ctrl-shift-mystery"), None);
    }

    #[cfg(windows)]
    #[test]
    fn a_shortcut_registers_and_unregisters() {
        // An unusual chord nothing else holds.
        let binding = "ctrl-alt-shift-f23";
        set(Some(binding)).expect("registered");
        // Registering the same chord again replaces our own registration.
        set(Some(binding)).expect("re-registered");
        set(None).unwrap();
        assert!(ACTIVE.lock().unwrap().is_none());
    }
}
