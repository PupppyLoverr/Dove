//! The island: Dove's always-on helper process (`dove --island`).
//!
//! It owns the notch and the tray, starts at login, and stays when Dove's
//! window closes or Dove quits, so music, the clipboard, notes and the timer
//! keep working and agents can still be watched and answered while the
//! engine runs headless. It loads no engine, no webview and no voice: just
//! GPUI for the notch, the settings store and a one-thread runtime for the
//! line to the engine ([`ipc`]).
//!
//! One island per data folder (a named mutex on Windows, a lock file
//! elsewhere). Dove starts it if it isn't running ([`ensure_running`]); the
//! tray's **Quit Dove completely** stops both.

pub mod host;
pub mod ipc;
pub mod login;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dove_core::store::Store;
use gpui::{App, QuitMode};

/// Whether this platform runs an island (it needs the notch and a tray).
pub const SUPPORTED: bool = cfg!(windows);

/// `dove --island`.
pub fn run() {
    let root = dove_core::store::default_root();
    if !claim(&root) {
        tracing::info!("island: already running for {}", root.display());
        return;
    }
    let store = match Store::open(&root) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("Dove's island could not open its data: {e:#}");
            return;
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("island-io")
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Dove's island could not start: {e:#}");
            return;
        }
    };
    let handle = rt.handle().clone();
    gpui_platform::application().with_assets(crate::assets::Assets).run(move |cx: &mut App| {
        // The island has no main window; it runs until told to quit.
        cx.set_quit_mode(QuitMode::Explicit);
        gpui_base::init(cx);
        crate::assets::load_fonts(cx);
        crate::motion::init(&store, cx);
        crate::design::load_preferences(&store);
        let notch = crate::notch::init(store.clone(), handle.clone(), cx);
        tray::start(store.clone(), handle.clone(), notch, cx);
        login::apply(&store);
    });
    drop(rt);
}

/// Holds this data folder's island slot for the life of the process.
/// False when another island has it.
fn claim(root: &Path) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
        use windows_sys::Win32::System::Threading::CreateMutexW;
        let name: Vec<u16> = mutex_name(root).encode_utf16().chain(Some(0)).collect();
        // SAFETY: a named mutex, kept (never closed) for the process's life.
        unsafe {
            let h = CreateMutexW(std::ptr::null(), 1, name.as_ptr());
            !h.is_null() && GetLastError() != ERROR_ALREADY_EXISTS
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd as _;
        let path = root.join("run").join("island.lock");
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(root));
        let Ok(file) = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path) else {
            return false;
        };
        // SAFETY: flock on an fd we own; the file is leaked so the lock lives
        // as long as the process.
        let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
        if locked {
            std::mem::forget(file);
        }
        locked
    }
}

#[cfg(windows)]
fn mutex_name(root: &Path) -> String {
    let addr = ipc::address(root);
    format!(r"Local\{}", addr.rsplit('\\').next().unwrap_or("dove-island"))
}

/// Whether an island already runs for `root`.
fn running(root: &Path) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
        let name: Vec<u16> = mutex_name(root).encode_utf16().chain(Some(0)).collect();
        // SAFETY: opens (and closes) a handle to a named mutex, if it exists.
        unsafe {
            let h = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr());
            if h.is_null() {
                return false;
            }
            CloseHandle(h);
            true
        }
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        false
    }
}

fn exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Starts a detached copy of this executable with `args`.
fn spawn_detached(args: &[&str]) -> bool {
    let Some(exe) = exe() else { return false };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        /// DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: it outlives us and
        /// gets no console.
        const FLAGS: u32 = 0x0000_0008 | 0x0000_0200;
        cmd.creation_flags(FLAGS);
    }
    match cmd.spawn() {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("could not start {args:?}: {e}");
            false
        }
    }
}

/// From Dove: start the island if this data folder has none running.
pub fn ensure_running(root: &Path) {
    if !SUPPORTED || std::env::var("DOVE_NOTCH").ok().as_deref() == Some("0") || running(root) {
        return;
    }
    spawn_detached(&["--island"]);
}

/// From the island: start Dove, with its window or headless.
pub fn launch_dove(window: bool) {
    if window {
        spawn_detached(&[]);
    } else {
        spawn_detached(&["--headless"]);
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod tray {
    use std::sync::Arc;
    use std::time::Duration;

    use dove_core::store::Store;
    use gpui::{App, Entity};
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use super::ipc::ToEngine;
    use crate::notch::Notch;

    enum Click {
        Open,
        Login,
        QuitAll,
    }

    fn icon() -> Option<Icon> {
        let image = crate::assets::brand_icon_square(32)?;
        let (w, h) = image.dimensions();
        Icon::from_rgba(image.into_raw(), w, h).ok()
    }

    /// The tray icon and its menu. Its events arrive through callbacks on
    /// the UI thread (no polling) and are handled on GPUI's executor.
    pub fn start(store: Arc<Store>, rt: tokio::runtime::Handle, notch: Option<Entity<Notch>>, cx: &mut App) {
        let menu = Menu::new();
        let open = MenuItem::new("Open Dove", true, None);
        let login = CheckMenuItem::new("Start at login", super::login::allowed(), store.flag(super::login::KEY, true), None);
        let quit = MenuItem::new("Quit Keplorayx completely", true, None);
        let _ = menu.append(&open);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&login);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&quit);
        let Some(image) = icon() else { return };
        let tray: TrayIcon = match TrayIconBuilder::new()
            .with_tooltip("Keplorayx")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_icon(image)
            .build()
        {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("tray icon unavailable: {e}");
                return;
            }
        };
        let (tx, rx) = flume::unbounded::<Click>();
        let (open_id, login_id, quit_id) = (open.id().clone(), login.id().clone(), quit.id().clone());
        {
            let tx = tx.clone();
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                let click = if e.id == open_id {
                    Click::Open
                } else if e.id == login_id {
                    Click::Login
                } else if e.id == quit_id {
                    Click::QuitAll
                } else {
                    return;
                };
                let _ = tx.send(click);
            }));
        }
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: tray_icon::MouseButtonState::Up, .. } = e
            {
                let _ = tx.send(Click::Open);
            }
        }));
        cx.spawn(async move |cx| {
            // Kept alive (and its menu) for as long as the island runs.
            let _tray = tray;
            let _login_item = login.clone();
            while let Ok(click) = rx.recv_async().await {
                let quit = cx
                    .update(|cx| match click {
                        Click::Open => {
                            send(notch.as_ref(), &store, &rt, ToEngine::Open { task: None }, cx);
                            false
                        }
                        Click::Login => {
                            let on = !store.flag(super::login::KEY, true);
                            let _ = store.set_setting(super::login::KEY, if on { "true" } else { "false" });
                            super::login::apply(&store);
                            login.set_checked(on);
                            false
                        }
                        Click::QuitAll => {
                            let running = notch.as_ref().map(|n| n.read(cx).running_agents()).unwrap_or(0);
                            if running > 0 && !confirm_quit(running) {
                                return false;
                            }
                            send(notch.as_ref(), &store, &rt, ToEngine::QuitAll, cx);
                            true
                        }
                    });
                if quit {
                    // Let the goodbye reach Dove before the island goes.
                    cx.background_executor().timer(Duration::from_millis(400)).await;
                    let _ = cx.update(|cx| cx.quit());
                    break;
                }
            }
        })
        .detach();
    }

    /// Through the notch's line when there is one, else a one-off connection
    /// (or, to open, a fresh Dove).
    fn send(notch: Option<&Entity<Notch>>, store: &Store, rt: &tokio::runtime::Handle, msg: ToEngine, cx: &mut App) {
        if let Some(n) = notch
            && n.read(cx).connected()
        {
            n.update(cx, |n, _| n.command(msg));
            return;
        }
        match super::ipc::connect(rt, store.root()) {
            Some(link) => {
                let _ = link.outbound.send(msg);
                // The link's writer runs on `rt`; keep it alive long enough.
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(500));
                    drop(link);
                });
            }
            None if matches!(msg, ToEngine::Open { .. }) => super::launch_dove(true),
            None => {}
        }
    }

    #[cfg(windows)]
    fn confirm_quit(running: usize) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            IDYES, MB_ICONWARNING, MB_SETFOREGROUND, MB_YESNO, MessageBoxW,
        };
        let text = format!(
            "{running} agent{} still working or waiting for you. Quit Dove completely and stop {}?",
            if running == 1 { " is" } else { "s are" },
            if running == 1 { "it" } else { "them" }
        );
        let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "Keplorayx".encode_utf16().chain(Some(0)).collect();
        // SAFETY: two NUL-terminated strings that outlive the call.
        unsafe {
            MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONWARNING | MB_SETFOREGROUND)
                == IDYES
        }
    }

    #[cfg(not(windows))]
    fn confirm_quit(_: usize) -> bool {
        true
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod tray {
    use std::sync::Arc;

    use dove_core::store::Store;
    use gpui::{App, Entity};

    use crate::notch::Notch;

    pub fn start(_: Arc<Store>, _: tokio::runtime::Handle, _: Option<Entity<Notch>>, _: &mut App) {}
}
