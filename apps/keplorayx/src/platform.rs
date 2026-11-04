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
            Shell::Quit => "quit",
        }
    }
}

/// Starts the tray icon and the global keys; their events arrive on the
/// returned channel. The tray lives as long as the returned guard.
pub fn start(global_keys: bool) -> (flume::Receiver<Shell>, Tray) {
    let (tell, heard) = flume::unbounded();
    if global_keys {
        keys::start(tell.clone());
    }
    let tray = tray::start(tell);
    (heard, tray)
}

pub use tray::Tray;

#[cfg(any(windows, target_os = "macos"))]
mod tray {
    use super::Shell;
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
        menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    };

    pub struct Tray(#[allow(dead_code)] Option<TrayIcon>);

    pub fn start(tell: flume::Sender<Shell>) -> Tray {
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
        let quit = MenuItem::new("Quit Keplorayx", true, None);
        let _ = menu.append(&quit);
        let ids: Vec<_> = items
            .iter()
            .map(|(item, what)| (item.id().clone(), *what))
            .chain([(quit.id().clone(), Shell::Quit)])
            .collect();
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

    pub fn start(_tell: flume::Sender<Shell>) -> Tray {
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

    const KEYS: [(i32, u32, Shell); 3] =
        [(1, VK_SPACE as u32, Shell::Ask), (2, b'A' as u32, Shell::Allow), (3, b'D' as u32, Shell::Deny)];

    /// A thread of its own registers the keys and waits for them: a hot key
    /// registered with no window arrives in its thread's queue.
    pub fn start(tell: flume::Sender<Shell>) {
        std::thread::Builder::new()
            .name("kx-keys".into())
            .spawn(move || {
                for (id, vk, _) in KEYS {
                    // SAFETY: registering a hot key for this thread's queue.
                    if unsafe { RegisterHotKey(std::ptr::null_mut(), id, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, vk) } == 0 {
                        tracing::warn!("global key {id} is taken by another app");
                    }
                }
                // SAFETY: a zeroed MSG is a valid out-parameter.
                let mut msg: MSG = unsafe { std::mem::zeroed() };
                // SAFETY: this thread's own message queue.
                while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
                    if msg.message == WM_HOTKEY
                        && let Some((_, _, what)) = KEYS.iter().find(|(id, ..)| *id as usize == msg.wParam)
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
