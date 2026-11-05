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
    // What's playing, from Windows' media controls, and its keys (src/media.rs).
    crate::media::install(cx, &host);
    // The Music tab's keys and what's playing (src/widgets.rs).
    host.action("widget.media", |what| {
        if let zgx_script::Value::Str(what) = what {
            crate::widgets::press_media(&what);
        }
    });
    host.function("widget.playing", |_| zgx_script::Value::from(crate::launcher::now_playing().as_str()));
    reveal(cx, &host);
    crate::serve(cx, host.clone(), "notch.luau", false);
    host
}

/// Where the notch is now, in screen pixels (left, top, width, height):
/// the pointer's watcher reads it to know whether the pointer is over it.
static RECT: std::sync::Mutex<(f64, f64, f64, f64)> = std::sync::Mutex::new((0., 0., 0., 0.));

/// The island's states, as notch.luau asks for them: each its size (in
/// points, centred at the screen's top) and whether it takes clicks
/// (`notch.size { w, h, interactive }`). Everything else lets clicks through.
///
/// And the pointer (`notch.near`): true once it rests at the screen's top
/// edge over where the island comes from (90 ms, so passing through to a
/// browser's tabs doesn't open it), or on the island once it shows; false
/// 450 ms after it leaves, as Dove's island did. A thread asks where the
/// pointer is ten times a second; nothing else runs while it's away.
fn reveal(cx: &mut WindowContext, host: &zgx_script::ScriptHost) {
    let near = host.shared("notch.near", false);
    let window = cx.window.clone();
    window.set_pointer_passthrough(true);
    let resize = cx.window.clone();
    host.action("notch.size", move |req| {
        let n = |k: &str| match req.field(k) {
            Some(zgx_script::Value::Num(v)) => *v,
            _ => 0.,
        };
        let (w, h) = (n("w").max(1.), n("h").max(1.));
        let interactive = matches!(req.field("interactive"), Some(zgx_script::Value::Bool(true)));
        let frosted = matches!(req.field("glass"), Some(zgx_script::Value::Bool(true)));
        let top = n("top");
        let scale = system_scale();
        let (pw, ph) = ((w * scale).round(), (h * scale).round());
        let (left, top) = ((screen_width() - pw) / 2., (top * scale).round());
        if let Ok(mut r) = RECT.lock() {
            *r = (left, top, pw, ph);
        }
        resize.set_bounds(WindowBounds { position: Some((left as i32, top as i32)), size: (pw as u32, ph as u32) });
        resize.set_pointer_passthrough(!interactive);
        // The island is the main window's frosted glass (a blur of what's
        // behind it, the theme's tint over it), its corners the system's
        // round; hidden, no glass, so nothing shows but the hint.
        resize.set_glass(frosted.then(|| zgx_desktop::Glass::new(zgx_desktop::GlassMaterial::Hud).plain()));
        round_corners();
    });
    let (tell, heard) = flume::unbounded::<bool>();
    std::thread::Builder::new()
        .name("kx-notch-reveal".into())
        .spawn(move || watch(tell))
        .ok();
    cx.tasks.spawn(async move {
        while let Ok(now) = heard.recv_async().await {
            near.set(zgx_script::Value::Bool(now));
        }
    });
}

/// Windows 11's rounded corners on the island's window.
#[cfg(windows)]
fn round_corners() {
    use windows_sys::Win32::{
        Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute},
        UI::WindowsAndMessaging::FindWindowW,
    };
    let title: Vec<u16> = "Keplorayx notch".encode_utf16().chain([0]).collect();
    // SAFETY: our own window; a plain attribute.
    unsafe {
        let h = FindWindowW(std::ptr::null(), title.as_ptr());
        if !h.is_null() {
            let pref = DWMWCP_ROUND;
            DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE as u32, (&pref as *const i32).cast(), 4);
        }
    }
}

#[cfg(not(windows))]
fn round_corners() {}

#[cfg(windows)]
fn screen_width() -> f64 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN};
    // SAFETY: a plain query.
    unsafe { GetSystemMetrics(SM_CXSCREEN) as f64 }
}

#[cfg(not(windows))]
fn screen_width() -> f64 {
    1440.
}

#[cfg(windows)]
fn watch(tell: flume::Sender<bool>) {
    use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};
    let scale = system_scale();
    // Where reaching opens it: the top edge, over the island's middle.
    let reach_w = 460. * scale;
    let (mut shown, mut rested, mut away) = (false, 0u32, 0u32);
    loop {
        std::thread::sleep(std::time::Duration::from_millis(if shown { 50 } else { 90 }));
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: a plain query.
        if unsafe { GetCursorPos(&mut p) } == 0 {
            continue;
        }
        let screen = screen_width();
        let (x, y) = (p.x as f64, p.y as f64);
        let left = (screen - reach_w) / 2.;
        let reach = y <= 3. * scale && x >= left && x <= left + reach_w;
        let (rx, ry, rw, rh) = RECT.lock().map(|r| *r).unwrap_or_default();
        let margin = 10. * scale;
        let over = x >= rx - margin && x <= rx + rw + margin && y >= ry - margin && y <= ry + rh + margin;
        let now = if shown {
            if over || reach {
                away = 0;
                true
            } else {
                away += 1;
                // (450 ms at 50 ms a look.)
                away < 9
            }
        } else if reach {
            rested += 1;
            // (Resting at the edge a moment, not passing through.)
            rested >= 1
        } else {
            rested = 0;
            false
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
