//! Keplorayx beside the desktop, outside its windows: an icon in the tray
//! (or the menu bar), keys that reach it from any app, and Windows' voice
//! typing. Each sends what happened as a [`Shell`] event to the main window,
//! which brings itself forward and acts (src/main.rs).
//!
//! - The tray: a click opens the window; its menu has New thread, the Inbox,
//!   Settings and Quit. Windows and macOS (Linux's tray needs GTK's loop,
//!   which zgx doesn't run: no icon there yet).
//! - Keys, from any app (Windows): Ctrl+Alt+Space brings Keplorayx forward
//!   with the composer focused; Ctrl+Alt+A and Ctrl+Alt+D allow or deny
//!   what an agent waits on (the notch's buttons), as Dove's notch had them.
//!   Off with the store's `kx.global_keys` = "false".
//! - Dictation: Win+H, Windows' own voice typing, into the field with focus.

/// What the desktop asked of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    /// Bring the main window forward.
    Open,
    /// Forward, on a new thread, the composer focused.
    Ask,
    Inbox,
    Settings,
    /// Answer the oldest request waiting on you.
    Allow,
    Deny,
    /// Show or hide the notch.
    Notch,
    /// Start Keplorayx with Windows (or stop), in the background.
    Login,
    /// The launcher (Alt+Space).
    Launcher,
    Quit,
}

impl Shell {
    pub fn word(self) -> &'static str {
        match self {
            Shell::Open => "open",
            Shell::Ask => "ask",
            Shell::Inbox => "inbox",
            Shell::Settings => "settings",
            Shell::Allow => "allow",
            Shell::Deny => "deny",
            Shell::Notch => "notch",
            Shell::Login => "login",
            Shell::Launcher => "launcher",
            Shell::Quit => "quit",
        }
    }
}

/// Starts the tray icon and the global keys; their events arrive on the
/// returned channel. The tray lives as long as the returned guard.
pub fn start(global_keys: bool, notch: bool) -> (flume::Receiver<Shell>, Tray) {
    let (tell, heard) = flume::unbounded();
    if global_keys {
        keys::start(tell.clone());
    }
    let tray = tray::start(tell, notch, autostart::on());
    (heard, tray)
}

/// Starting with the computer: a per-user Run entry (Windows) that starts
/// Keplorayx in the background (`--background`: the tray and the notch, no
/// window). Elsewhere, not yet.
pub mod autostart {
    #[cfg(windows)]
    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    #[cfg(windows)]
    const NAME: &str = "Keplorayx";

    #[cfg(windows)]
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    /// Whether it starts with the computer.
    pub fn on() -> bool {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
            let (key, name) = (wide(KEY), wide(NAME));
            let mut size = 0u32;
            // SAFETY: valid, NUL-terminated strings; a size query only.
            let found = unsafe {
                RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size)
            };
            found == 0
        }
        #[cfg(not(windows))]
        false
    }

    /// Start with the computer, or stop.
    pub fn set(on: bool) {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW};
            let (key, name) = (wide(KEY), wide(NAME));
            if on {
                let Ok(exe) = std::env::current_exe() else { return };
                let line = wide(&format!("\"{}\" --background", exe.display()));
                // SAFETY: valid strings; the value's size is in bytes, its NUL included.
                unsafe {
                    RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ, line.as_ptr().cast(), (line.len() * 2) as u32);
                }
            } else {
                // SAFETY: valid strings.
                unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
            }
        }
        #[cfg(not(windows))]
        let _ = on;
    }
}

pub use tray::Tray;

#[cfg(any(windows, target_os = "macos"))]
mod tray {
    use super::Shell;
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
        menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    };

    pub struct Tray(Option<TrayIcon>);

    impl Tray {
        /// Whether the icon is there (a window hidden to it can come back).
        pub fn live(&self) -> bool {
            self.0.is_some()
        }
    }

    pub fn start(tell: flume::Sender<Shell>, notch: bool, login: bool) -> Tray {
        let menu = Menu::new();
        let items = [
            (MenuItem::new("Open Keplorayx", true, None), Shell::Open),
            (MenuItem::new("New thread", true, None), Shell::Ask),
            (MenuItem::new("Inbox", true, None), Shell::Inbox),
            (MenuItem::new("Settings", true, None), Shell::Settings),
        ];
        for (item, _) in &items {
            let _ = menu.append(item);
        }
        let _ = menu.append(&PredefinedMenuItem::separator());
        // (Ticked as they stand; a click flips the tick itself.)
        let show_notch = CheckMenuItem::new("Show the notch", true, notch, None);
        let at_login = CheckMenuItem::new("Start with Windows", cfg!(windows), login, None);
        let _ = menu.append(&show_notch);
        let _ = menu.append(&at_login);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let quit = MenuItem::new("Quit Keplorayx", true, None);
        let _ = menu.append(&quit);
        let ids: Vec<_> = items
            .iter()
            .map(|(item, what)| (item.id().clone(), *what))
            .chain([
                (show_notch.id().clone(), Shell::Notch),
                (at_login.id().clone(), Shell::Login),
                (quit.id().clone(), Shell::Quit),
            ])
            .collect();
        // Kept alive with the menu.
        std::mem::forget((show_notch, at_login));
        let t = tell.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if let Some((_, what)) = ids.iter().find(|(id, _)| *id == e.id) {
                let _ = t.send(*what);
            }
        }));
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                let _ = tell.send(Shell::Open);
            }
        }));
        let icon = Icon::from_rgba(orbit_icon(32), 32, 32).ok();
        let mut builder =
            TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("Keplorayx").with_menu_on_left_click(false);
        if let Some(icon) = icon {
            builder = builder.with_icon(icon);
        }
        match builder.build() {
            Ok(tray) => Tray(Some(tray)),
            Err(e) => {
                tracing::warn!("tray: {e}");
                Tray(None)
            }
        }
    }

    /// Orbit's mark, drawn: a planet and its ring, white, `n` pixels square.
    fn orbit_icon(n: u32) -> Vec<u8> {
        let mut px = vec![0u8; (n * n * 4) as usize];
        let c = (n as f32 - 1.) / 2.;
        for y in 0..n {
            for x in 0..n {
                let (dx, dy) = (x as f32 - c, y as f32 - c);
                // The planet: a disc; the ring: an ellipse's outline, tilted.
                let disc = (dx * dx + dy * dy).sqrt() - n as f32 * 0.2;
                let (rx, ry) = (dx * 0.8 + dy * 0.6, -dx * 0.6 + dy * 0.8);
                let ring = ((rx / (n as f32 * 0.46)).powi(2) + (ry / (n as f32 * 0.2)).powi(2)).sqrt() - 1.;
                let a = (1. - disc.clamp(0., 1.)).max((1. - (ring.abs() * n as f32 * 0.22 - 0.4).clamp(0., 1.)) * 0.95);
                let i = ((y * n + x) * 4) as usize;
                px[i..i + 4].copy_from_slice(&[255, 255, 255, (a.clamp(0., 1.) * 255.) as u8]);
            }
        }
        px
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod tray {
    use super::Shell;

    pub struct Tray;

    impl Tray {
        pub fn live(&self) -> bool {
            false
        }
    }

    pub fn start(_tell: flume::Sender<Shell>, _notch: bool, _login: bool) -> Tray {
        Tray
    }
}

#[cfg(windows)]
mod keys {
    use super::Shell;
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, VK_SPACE},
        WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY},
    };

    // (id, modifiers, key, what.)
    const KEYS: [(i32, u32, u32, Shell); 3] = [
        (1, MOD_CONTROL | MOD_ALT, VK_SPACE as u32, Shell::Ask),
        (2, MOD_CONTROL | MOD_ALT, b'A' as u32, Shell::Allow),
        (3, MOD_CONTROL | MOD_ALT, b'D' as u32, Shell::Deny),
    ];

    /// Alt+Space, the launcher's. Windows keeps it for every window's system
    /// menu, so a registered hot key doesn't reliably get it: a low-level
    /// keyboard hook does, as PowerToys Run's does. It takes the press (the
    /// system menu doesn't open) and taps a key that means nothing, so the
    /// lone Alt left behind doesn't open a menu bar.
    mod launcher_key {
        use super::Shell;
        use std::sync::{
            OnceLock,
            atomic::{AtomicBool, Ordering},
        };
        use windows_sys::Win32::{
            Foundation::{LPARAM, LRESULT, WPARAM},
            UI::{
                Input::KeyboardAndMouse::{
                    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
                    VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SPACE,
                },
                WindowsAndMessaging::{
                    CallNextHookEx, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, SetWindowsHookExW, WH_KEYBOARD_LL,
                    WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
                },
            },
        };

        static TELL: OnceLock<flume::Sender<Shell>> = OnceLock::new();
        /// A Space taken: its release is taken too.
        static TAKEN: AtomicBool = AtomicBool::new(false);

        fn down(vk: u16) -> bool {
            // SAFETY: a plain query.
            unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
        }

        unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
            if code == HC_ACTION as i32 {
                // SAFETY: for WH_KEYBOARD_LL, lparam is a KBDLLHOOKSTRUCT.
                let k = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
                let msg = wparam as u32;
                if k.vkCode == VK_SPACE as u32 && k.flags & LLKHF_INJECTED == 0 {
                    if (msg == WM_SYSKEYDOWN || msg == WM_KEYDOWN)
                        && down(VK_MENU)
                        && !down(VK_CONTROL)
                        && !down(VK_LWIN)
                        && !down(VK_RWIN)
                    {
                        if !TAKEN.swap(true, Ordering::Relaxed) {
                            if let Some(tell) = TELL.get() {
                                let _ = tell.send(Shell::Launcher);
                            }
                            mask_alt();
                        }
                        return 1;
                    }
                    if (msg == WM_SYSKEYUP || msg == WM_KEYUP) && TAKEN.swap(false, Ordering::Relaxed) {
                        return 1;
                    }
                }
            }
            // SAFETY: passing the event on, as every hook must.
            unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
        }

        /// A press and release of 0xE8, a key that means nothing: Alt is no
        /// longer pressed alone.
        fn mask_alt() {
            let key = |up: bool| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT { wVk: 0xE8, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 },
                },
            };
            let inputs = [key(false), key(true)];
            // SAFETY: two well-formed keyboard inputs.
            unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
        }

        /// On the calling thread, which must pump messages.
        pub fn install(tell: flume::Sender<Shell>) {
            let _ = TELL.set(tell);
            // SAFETY: a low-level hook needs no module or thread.
            let h = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), std::ptr::null_mut(), 0) };
            if h.is_null() {
                tracing::warn!("the launcher's key (Alt+Space) couldn't be hooked");
            }
        }
    }

    /// A thread of its own registers the keys and waits for them: a hot key
    /// registered with no window arrives in its thread's queue.
    pub fn start(tell: flume::Sender<Shell>) {
        std::thread::Builder::new()
            .name("kx-keys".into())
            .spawn(move || {
                launcher_key::install(tell.clone());
                for (id, mods, vk, _) in KEYS {
                    // SAFETY: registering a hot key for this thread's queue.
                    if unsafe { RegisterHotKey(std::ptr::null_mut(), id, mods | MOD_NOREPEAT, vk) } == 0 {
                        tracing::warn!("global key {id} is taken by another app");
                    }
                }
                // SAFETY: a zeroed MSG is a valid out-parameter.
                let mut msg: MSG = unsafe { std::mem::zeroed() };
                // SAFETY: this thread's own message queue.
                while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
                    if msg.message == WM_HOTKEY
                        && let Some((.., what)) = KEYS.iter().find(|(id, ..)| *id as usize == msg.wParam)
                        && tell.send(*what).is_err()
                    {
                        break;
                    }
                }
            })
            .ok();
    }
}

#[cfg(not(windows))]
mod keys {
    use super::Shell;

    /// macOS and Linux: not yet (macOS needs Carbon's RegisterEventHotKey or
    /// an event tap; the shipping Dove's port belongs here).
    pub fn start(_tell: flume::Sender<Shell>) {}
}

/// Windows' voice typing: Win+H, typed as if by the person, into the field
/// that has focus.
#[cfg(windows)]
pub fn dictate() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_H, VK_LWIN,
    };
    let key = |vk: u16, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 },
        },
    };
    let inputs = [key(VK_LWIN, false), key(VK_H, false), key(VK_H, true), key(VK_LWIN, true)];
    // SAFETY: four well-formed keyboard inputs.
    unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
}

#[cfg(not(windows))]
pub fn dictate() {
    // macOS's dictation and Linux's have no key to press for them; the
    // shipping Dove's own dictation (dove-voice) is the way there.
}
