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

/// The island's window, in points: room for its largest shape, which a
/// sprung region cuts out of it (see `shape`). It never moves or resizes.
pub const SIZE: (f64, f64) = (680., 460.);

/// The island's spring, the same as notch.luau's `ui.spring` for the pill
/// it draws (stiffness, damping; a mass of one): a little overshoot, as the
/// Dynamic Island's.
pub const SPRING: (f64, f64) = (340., 26.);

/// Room round the pill for its shadow, in points.
const SHADOW: f64 = 14.;

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
    inspect: bool,
) -> zgx_script::ScriptHost {
    let host = crate::script_host(cx, "notch.luau");
    crate::host::install(cx, &host, engine);
    crate::clock::install(cx, &host);
    // At rest, a click asks: the launcher, over everything (src/launcher.rs).
    host.action("notch.ask", |_| crate::launcher::summon());
    // The Tray's Show: a file in its folder.
    host.action("notch.reveal", |path| {
        if let zgx_script::Value::Str(path) = path {
            #[cfg(windows)]
            let _ = std::process::Command::new("explorer").arg(format!("/select,{path}")).spawn();
        }
    });
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
    crate::serve(cx, host.clone(), "notch.luau", inspect);
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
    // No system glass: Windows draws its blur over the whole window, past
    // any region (the box of glass round the island). notch.luau draws the
    // pill itself, its edges smooth, on a transparent window.
    window.set_glass(None);
    let (aim, aimed) = flume::unbounded::<[f64; 4]>();
    std::thread::Builder::new()
        .name("kx-notch-shape".into())
        .spawn(move || shape(aimed))
        .ok();
    let resize = cx.window.clone();
    host.action("notch.size", move |req| {
        let n = |k: &str| match req.field(k) {
            Some(zgx_script::Value::Num(v)) => *v,
            _ => 0.,
        };
        let (w, h, r, y) = (n("w").max(0.), n("h").max(0.), n("r").max(0.), n("y"));
        let interactive = matches!(req.field("interactive"), Some(zgx_script::Value::Bool(true)));
        let scale = system_scale();
        let (pw, ph) = ((w * scale).round(), (h * scale).round());
        let left = (screen_width() - pw) / 2.;
        if let Ok(mut rect) = RECT.lock() {
            *rect = (left, (y * scale).max(0.), pw, ph);
        }
        resize.set_pointer_passthrough(!interactive);
        let _ = aim.send([w, h, r, y]);
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

/// The island's shape, sprung: a pill floating a few points under the
/// screen's top edge, every corner round, cut out of the still window as
/// its region, so the glass drops in from above the edge and morphs between
/// sizes with the Dynamic Island's overshoot, and clicks outside it reach
/// what's under. Woken by each new aim (w, h, corner, how far down, in
/// points); asleep once it settles.
#[cfg(windows)]
fn shape(aimed: flume::Receiver<[f64; 4]>) {
    use windows_sys::Win32::{
        Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute},
        Graphics::Gdi::{CreateRectRgn, SetWindowRgn},
    };
    let scale = system_scale();
    let (k, c) = SPRING;
    // Now and how fast, each of w, h, the corner and how far down.
    let mut now = [0f64; 4];
    let mut speed = [0f64; 4];
    let mut aim = [0f64; 4];
    let step = 1. / 120.;
    // Nothing shows until the script says what: the window exists before
    // its first shape, and uncut it's a box of glass the size of the
    // largest island.
    let mut window = None;
    for _ in 0..200 {
        if let Some(h) = own_window() {
            // SAFETY: our own window; an empty region it then owns, and a
            // plain attribute (no system corners: the region is its shape).
            unsafe {
                SetWindowRgn(h as _, CreateRectRgn(0, 0, 0, 0), 1);
                if let Ok(mut c) = CUT.lock() {
                    *c = Some((h as isize, None));
                }
                let pref = DWMWCP_DONOTROUND;
                DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE as u32, (&pref as *const i32).cast(), 4);
            }
            window = Some(h);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let mut first = true;
    loop {
        // Asleep until there's somewhere to go.
        let settled = (0..4).all(|i| (now[i] - aim[i]).abs() < 0.25 && speed[i].abs() < 0.5);
        let next = if settled { aimed.recv().ok() } else { aimed.try_recv().ok() };
        if settled && next.is_none() {
            return;
        }
        if let Some(next) = next {
            aim = next;
            // (It starts from where it rests, above the edge, not from
            // nothing at the window's corner.)
            if first {
                first = false;
                now = [aim[0], aim[1], aim[2], aim[3]];
            }
        }
        let started = std::time::Instant::now();
        for i in 0..4 {
            // (Semi-implicit Euler, at 120 a second.)
            speed[i] += (k * (aim[i] - now[i]) - c * speed[i]) * step;
            now[i] += speed[i] * step;
        }
        if (0..4).all(|i| (now[i] - aim[i]).abs() < 0.25 && speed[i].abs() < 0.5) {
            now = aim;
            speed = [0.; 4];
        }
        let hwnd = match window {
            Some(h) => h,
            None => {
                let Some(h) = own_window() else {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    continue;
                };
                // SAFETY: our own window; a plain attribute (no system
                // corners: the region is its shape).
                unsafe {
                    let pref = DWMWCP_DONOTROUND;
                    DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE as u32, (&pref as *const i32).cast(), 4);
                }
                window = Some(h);
                h
            }
        };
        let full = SIZE.0 * scale;
        let (w, h) = ((now[0].max(0.) * scale).round(), (now[1].max(0.) * scale).round());
        let r = (now[2].max(0.) * scale).min(h / 2.).min(w / 2.).round();
        let (left, top) = (((full - w) / 2.).round(), (now[3] * scale).round());
        // (The region is where clicks land, and room for the pill's soft
        // shadow beside and under it.)
        let pad = (SHADOW * scale).round();
        let cut = if w < 1. || h < 1. || top + h <= 0. {
            None
        } else {
            Some([
                (left - pad) as i32,
                top as i32 - 1,
                (left + w + pad) as i32 + 1,
                (top + h + pad * 1.5) as i32 + 1,
                (2. * (r + pad)) as i32,
            ])
        };
        if let Ok(mut c) = CUT.lock() {
            *c = Some((hwnd as isize, cut));
        }
        cut_to(hwnd, cut);
        let spent = started.elapsed();
        if let Some(rest) = std::time::Duration::from_secs_f64(step).checked_sub(spent) {
            std::thread::sleep(rest);
        }
    }
}

#[cfg(not(windows))]
fn shape(aimed: flume::Receiver<[f64; 4]>) {
    while aimed.recv().is_ok() {}
}

/// The shape last cut (the window, and left, top, right, bottom, corner in
/// pixels, or None for nothing): the pointer's watcher checks the window
/// still has it. (The window loses its region when the system or zgx
/// re-applies its glass or shows it, and uncut it's a box of glass the size
/// of the largest island.)
static CUT: std::sync::Mutex<Option<(isize, Option<[i32; 5]>)>> = std::sync::Mutex::new(None);

/// Cuts `hwnd` to `cut`.
#[cfg(windows)]
fn cut_to(hwnd: windows_sys::Win32::Foundation::HWND, cut: Option<[i32; 5]>) {
    use windows_sys::Win32::Graphics::Gdi::{CreateRectRgn, CreateRoundRectRgn, SetWindowRgn};
    // SAFETY: a region made here and handed to our own window, which owns
    // it from then on.
    unsafe {
        let region = match cut {
            None => CreateRectRgn(0, 0, 0, 0),
            Some([l, t, r, b, c]) => CreateRoundRectRgn(l, t, r, b, c, c),
        };
        SetWindowRgn(hwnd as _, region, 1);
    }
}

/// Whether the window still has the shape last cut; cut again if not.
#[cfg(windows)]
fn keep_cut() {
    use windows_sys::Win32::{
        Foundation::RECT,
        Graphics::Gdi::{GetWindowRgnBox, NULLREGION},
    };
    let Some((hwnd, cut)) = CUT.lock().ok().and_then(|c| *c) else { return };
    let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
    let mut b = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: a plain query of our own window.
    let kind = unsafe { GetWindowRgnBox(hwnd, &mut b) };
    let holds = match cut {
        None => kind == NULLREGION,
        // (The box of a round rect: its sides, give or take a pixel.)
        Some([l, t, r, bottom, _]) => {
            kind != 0
                && (b.left - l.max(0)).abs() <= 1
                && (b.top - t.max(0)).abs() <= 1
                && (b.right - r).abs() <= 2
                && (b.bottom - bottom).abs() <= 2
        }
    };
    if !holds {
        cut_to(hwnd, cut);
    }
}

/// This process's island window (another Keplorayx's has the same title).
#[cfg(windows)]
fn own_window() -> Option<windows_sys::Win32::Foundation::HWND> {
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM},
        UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId},
    };
    unsafe extern "system" fn each(h: HWND, found: LPARAM) -> i32 {
        let mut pid = 0u32;
        let mut title = [0u16; 64];
        // SAFETY: plain queries of a window the system handed us; `found`
        // is the Option below, alive for EnumWindows' call.
        unsafe {
            GetWindowThreadProcessId(h, &mut pid);
            if pid != std::process::id() {
                return 1;
            }
            let n = GetWindowTextW(h, title.as_mut_ptr(), title.len() as i32);
            if String::from_utf16_lossy(&title[..n.max(0) as usize]) == "Keplorayx notch" {
                *(found as *mut Option<HWND>) = Some(h);
                return 0;
            }
        }
        1
    }
    let mut found: Option<HWND> = None;
    // SAFETY: the callback writes only `found`, which outlives the call.
    unsafe { EnumWindows(Some(each), &mut found as *mut _ as LPARAM) };
    found
}

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
        keep_cut();
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
