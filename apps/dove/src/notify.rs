//! Attention outside the window: OS notifications when a thread needs you,
//! finishes or fails while Dove is in the background; the Needs-you count in
//! the window title; and an opt-in tray icon that keeps Dove (and its
//! automations) running with the window closed.

use std::sync::Arc;

use dove_core::Engine;
use gpui::{App, Window};

/// Settings keys.
pub const NOTIFICATIONS: &str = "notifications";
pub const SOUND: &str = "notification_sound";
pub const KEEP_RUNNING: &str = "keep_running";

/// Post an OS notification (off the UI thread; failures are logged).
pub fn os_notify(engine: &Engine, title: &str, body: &str) {
    // The island shows these itself while it's connected.
    if crate::island::host::serving() && engine.store.flag("notch_notifications", true) {
        return;
    }
    os_notify_store(&engine.store, title, body);
}

/// [`os_notify`] for a process without an engine (the island).
pub fn os_notify_store(store: &dove_core::store::Store, title: &str, body: &str) {
    if store.setting(NOTIFICATIONS).as_deref() == Some("false") {
        return;
    }
    let sound = store.setting(SOUND).as_deref() == Some("true");
    let (title, body) = (title.to_string(), body.to_string());
    std::thread::spawn(move || {
        let mut n = notify_rust::Notification::new();
        n.summary(&title).body(&body).appname("Dove");
        if sound {
            #[cfg(target_os = "macos")]
            n.sound_name("default");
            #[cfg(not(target_os = "macos"))]
            n.sound_name("message-new-instant");
        }
        if let Err(e) = n.show() {
            tracing::debug!("notification not shown: {e}");
        }
    });
}

/// "Dove (2)" while two threads need you.
pub fn set_badge(window: &mut Window, needs_you: usize) {
    let title = if needs_you == 0 { "Dove".to_string() } else { format!("Dove ({needs_you})") };
    window.set_window_title(&title);
}

/// Window-close behavior and the tray icon.
///
/// Closing the window frees it and everything it shows. The engine stays
/// (headless) while agents work or wait, or while automations are kept
/// running, and quits once that's done (`island::host`). Where the island
/// runs it owns the tray; elsewhere Dove keeps its own for automations.
pub fn install(engine: Arc<Engine>, window: &mut Window, cx: &mut App) {
    window.on_window_should_close(cx, move |_, cx| crate::island::host::Host::on_window_close(cx));
    if !crate::island::SUPPORTED && engine.store.setting(KEEP_RUNNING).as_deref() == Some("true") {
        tray::start(window, cx);
    }
}

/// Turn keep-running (and, without the island, its tray icon) on or off
/// from Settings.
pub fn set_keep_running(engine: &Engine, on: bool, window: &mut Window, cx: &mut App) {
    let _ = engine.store.set_setting(KEEP_RUNNING, if on { "true" } else { "false" });
    if crate::island::SUPPORTED {
        return;
    }
    if on {
        tray::start(window, cx);
    } else {
        tray::stop();
    }
}

pub fn tray_supported() -> bool {
    tray::SUPPORTED
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod tray {
    use std::cell::RefCell;
    use std::time::Duration;

    use gpui::{App, Window};
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

    pub const SUPPORTED: bool = true;

    thread_local! {
        static TRAY: RefCell<Option<(TrayIcon, String, String)>> = const { RefCell::new(None) };
    }

    /// The same dove as the exe, scaled for the tray.
    fn icon() -> Option<Icon> {
        let image = crate::assets::brand_icon_square(32)?;
        let (w, h) = image.dimensions();
        Icon::from_rgba(image.into_raw(), w, h).ok()
    }

    pub fn start(window: &mut Window, cx: &mut App) {
        if TRAY.with(|t| t.borrow().is_some()) {
            return;
        }
        let menu = Menu::new();
        let open = MenuItem::new("Open Dove", true, None);
        let quit = MenuItem::new("Quit Dove", true, None);
        let _ = menu.append(&open);
        let _ = menu.append(&quit);
        let built = TrayIconBuilder::new()
            .with_tooltip("Dove: automations keep running")
            .with_menu(Box::new(menu))
            .with_icon(match icon() {
                Some(i) => i,
                None => return,
            })
            .build();
        let tray = match built {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("tray icon unavailable: {e}");
                return;
            }
        };
        let (open_id, quit_id) = (open.id().0.clone(), quit.id().0.clone());
        TRAY.with(|t| *t.borrow_mut() = Some((tray, open_id, quit_id)));
        // Poll the tray's channels; only runs while the tray exists.
        let handle = window.window_handle();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(300)).await;
                if TRAY.with(|t| t.borrow().is_none()) {
                    break;
                }
                let mut open = false;
                let mut quit = false;
                while let Ok(ev) = MenuEvent::receiver().try_recv() {
                    TRAY.with(|t| {
                        if let Some((_, o, q)) = &*t.borrow() {
                            open |= ev.id.0 == *o;
                            quit |= ev.id.0 == *q;
                        }
                    });
                }
                while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                    if matches!(ev, TrayIconEvent::DoubleClick { .. }) {
                        open = true;
                    }
                }
                if quit {
                    cx.update(|cx| cx.quit());
                    break;
                }
                if open {
                    let _ = handle.update(cx, |_, window, _| window.activate_window());
                }
            }
        })
        .detach();
    }

    pub fn stop() {
        TRAY.with(|t| *t.borrow_mut() = None);
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod tray {
    use gpui::{App, Window};

    /// Linux keeps running minimized; a tray needs a desktop-specific host.
    pub const SUPPORTED: bool = false;

    pub fn start(_: &mut Window, _: &mut App) {}

    pub fn stop() {}
}
