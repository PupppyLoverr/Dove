//! The notch: a capsule at the top centre of the screen, over every window.
//! It shows your agents at a glance (each a star in its own light) and lights
//! its lip warm when one needs you, with Allow and Deny right there. A click
//! anywhere else on it brings the main window forward on that thread.
//!
//! It is a window of this process (not Dove's separate `--island`), so it
//! shares the engine's hub (src/host.rs): the same live sessions and
//! approvals the main window sees. Off with `KX_NOTCH=0`.

use std::sync::Arc;

use zgx_desktop::{WindowBounds, WindowContext, WindowLevel, WindowOptions};

/// The capsule's window, in points: room for its widest state.
pub const SIZE: (f64, f64) = (520., 72.);

/// Whether the notch opens beside the main window.
pub fn enabled() -> bool {
    std::env::var_os("KX_NOTCH").is_none_or(|v| v != "0")
}

/// Its window: frameless, transparent, always on top, at the top centre of
/// the primary display.
pub fn window() -> WindowOptions {
    WindowOptions {
        title: "Keplorayx notch".into(),
        width: SIZE.0,
        height: SIZE.1,
        transparent: true,
        decorations: false,
        resizable: false,
        shadow: false,
        level: WindowLevel::AlwaysOnTop,
        bounds: top_centre(),
        ..Default::default()
    }
}

#[cfg(windows)]
fn top_centre() -> Option<WindowBounds> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN};
    // SAFETY: a plain query of the primary display's width in pixels.
    let screen = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    if screen <= 0 {
        return None;
    }
    let scale = system_scale();
    let (w, h) = ((SIZE.0 * scale) as u32, (SIZE.1 * scale) as u32);
    Some(WindowBounds { position: Some(((screen - w as i32) / 2, 0)), size: (w, h) })
}

/// The primary display's scale (1.25 at 120 dpi), for placing a window in
/// pixels before it exists.
#[cfg(windows)]
pub fn system_scale() -> f64 {
    // SAFETY: a plain query.
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForSystem() };
    if dpi == 0 { 1. } else { dpi as f64 / 96. }
}

#[cfg(not(windows))]
pub fn system_scale() -> f64 {
    1.
}

#[cfg(not(windows))]
fn top_centre() -> Option<WindowBounds> {
    // macOS and Linux place it where the platform does until the island's
    // own placement (the notch on a MacBook) is ported.
    None
}

/// The notch's content in `cx`: the engine's host and notch.luau. `focus`
/// brings the main window forward.
pub fn open(
    cx: &mut WindowContext,
    engine: Arc<dove_core::Engine>,
    focus: impl Fn(Option<String>) + 'static,
) -> zgx_script::ScriptHost {
    let host = crate::script_host(cx, "notch.luau");
    crate::host::install(cx, &host, engine);
    crate::clock::install(cx, &host);
    // At rest, a click asks: the launcher, over everything (src/launcher.rs).
    host.action("notch.ask", |_| crate::launcher::summon());
    host.action("notch.focus", move |thread| {
        let id = match thread {
            zgx_script::Value::Str(s) => Some(s.to_string()),
            _ => None,
        };
        focus(id);
    });
    reveal(cx, &host);
    crate::serve(cx, host.clone(), "notch.luau", false);
    host
}

/// The notch shows only when you reach for it: the pointer at the screen's
/// top edge, over where it sits (`notch.near`, true), and while the pointer
/// stays on it; it goes again soon after the pointer leaves. Hidden, its
/// window lets every click through to what's under it. A thread watches the
/// pointer (a plain query, ten times a second).
fn reveal(cx: &mut WindowContext, host: &zgx_script::ScriptHost) {
    let near = host.shared("notch.near", false);
    let window = cx.window.clone();
    window.set_pointer_passthrough(true);
    let (tell, heard) = flume::unbounded::<bool>();
    std::thread::Builder::new()
        .name("kx-notch-reveal".into())
        .spawn(move || watch(tell))
        .ok();
    cx.tasks.spawn(async move {
        while let Ok(now) = heard.recv_async().await {
            window.set_pointer_passthrough(!now);
            near.set(zgx_script::Value::Bool(now));
        }
    });
}

#[cfg(windows)]
fn watch(tell: flume::Sender<bool>) {
    use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::{GetCursorPos, GetSystemMetrics, SM_CXSCREEN}};
    let scale = system_scale();
    let (w, h) = (SIZE.0 * scale, SIZE.1 * scale);
    let mut shown = false;
    let mut away = 0u32;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: plain queries.
        let (ok, screen) = unsafe { (GetCursorPos(&mut p), GetSystemMetrics(SM_CXSCREEN)) };
        if ok == 0 || screen <= 0 {
            continue;
        }
        let left = (screen as f64 - w) / 2.;
        let (x, y) = (p.x as f64, p.y as f64);
        // Reaching for it: the top few pixels, over it (a little wider).
        let reach = y <= 4. * scale && x >= left - 40. * scale && x <= left + w + 40. * scale;
        // On it, once it shows (with a little room round it).
        let over = y <= h + 12. * scale && x >= left - 12. * scale && x <= left + w + 12. * scale;
        let now = if shown {
            if over { away = 0; true } else { away += 1; away < 5 }
        } else {
            reach
        };
        if now != shown {
            shown = now;
            if tell.send(now).is_err() {
                break;
            }
        }
    }
}

#[cfg(not(windows))]
fn watch(tell: flume::Sender<bool>) {
    // Elsewhere: always shown, as before.
    let _ = tell.send(true);
}
