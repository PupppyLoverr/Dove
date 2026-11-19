//! The notch: a capsule at the top centre of the screen, over every window.
//! It shows your agents at a glance (each a star in its own light) and lights
//! its lip warm when one needs you, with Allow and Deny right there. A click
//! anywhere else on it brings the main window forward on that thread.
//!
//! It is a window of this process (not Dove's separate `--island`), so it
//! shares the engine's hub (src/host.rs): the same live sessions and
//! approvals the main window sees. Off with `KX_NOTCH=0`.

use std::sync::Arc;

use zgx_desktop::{WindowBounds, WindowContext, WindowHandle, WindowLevel, WindowOptions};

/// The island's window, in points: room for its largest shape, which a
/// sprung region cuts out of it (see `shape`). It never moves or resizes.
pub const SIZE: (f64, f64) = (680., 460.);

/// The island's spring, the same as notch.luau's `ui.spring` for the pill
/// it draws (stiffness, damping; a mass of one): a little overshoot, as the
/// Dynamic Island's.
pub const SPRING: (f64, f64) = (360., 26.);

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

#[cfg(target_os = "macos")]
pub fn system_scale() -> f64 {
    use core_graphics::display::CGDisplay;
    // (Retina reads 2: the main display's pixels against the points it
    // reports for itself.)
    let display = CGDisplay::main();
    let pixels = display.pixels_wide() as f64;
    let points = display.bounds().size.width;
    if pixels > 0. && points > 0. { pixels / points } else { 1. }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn system_scale() -> f64 {
    1.
}

/// The island's window starts at the top centre of the primary display on a
/// Mac, as on Windows. (AppKit's screens are points, the window's bounds
/// pixels; `CGDisplay::main()` reports the primary with its origin at 0,0.)
#[cfg(target_os = "macos")]
fn top_centre() -> Option<WindowBounds> {
    use core_graphics::display::CGDisplay;
    let frame = CGDisplay::main().bounds();
    if frame.size.width <= 0. {
        return None;
    }
    let scale = system_scale();
    let (w, h) = ((SIZE.0 * scale) as u32, (SIZE.1 * scale) as u32);
    let left = (((frame.size.width - SIZE.0) / 2.) * scale).round() as i32;
    Some(WindowBounds { position: Some((left, 0)), size: (w, h) })
}

#[cfg(not(any(windows, target_os = "macos")))]
fn top_centre() -> Option<WindowBounds> {
    // (Linux places it where the compositor does until the island's own
    // placement is ported.)
    None
}

thread_local! {
    /// The open island's `notch.pin`, so Settings can tell it.
    static PIN: std::cell::RefCell<Option<zgx_script::Shared>> = const { std::cell::RefCell::new(None) };
}

/// Settings › General's "Keep the island out while something happens":
/// the island stays at the top while agents work, music plays or a timer
/// runs (`notch.pin` in notch.luau), or tucks away until you reach for it.
pub fn set_pin(on: bool) {
    PIN.with(|p| {
        if let Some(pin) = &*p.borrow() {
            pin.set(zgx_script::Value::from(on));
        }
    });
}

/// The notch's content in `cx`: the engine's host and notch.luau. `focus`
/// brings the main window forward.
pub fn open(
    cx: &mut WindowContext,
    focus: impl Fn(Option<String>) + 'static,
    inspect: bool,
) -> zgx_script::ScriptHost {
    let host = crate::script_host(cx, "notch.luau");
    crate::kx::engine_for(cx, &host);
    crate::kx::clock::install(cx, &host);
    let pin = host.shared("notch.pin", crate::kx::start::flag("notch_pin", true));
    PIN.with(|p| *p.borrow_mut() = Some(pin));
    // At rest, a click asks: the launcher, over everything (src/launcher.rs).
    host.action("notch.ask", |_| crate::kx::launcher::summon());
    // The Tray's Show: a file in its folder.
    host.action("notch.reveal", |path| {
        if let zgx_script::Value::Str(path) = path {
            #[cfg(windows)]
            let _ = std::process::Command::new("explorer").arg(format!("/select,{path}")).spawn();
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("open").arg("-R").arg(&*path).spawn();
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
    crate::kx::media::install(cx, &host);
    // The Music tab's keys and what's playing (src/widgets.rs).
    host.action("widget.media", |what| {
        if let zgx_script::Value::Str(what) = what {
            crate::kx::widgets::press_media(&what);
        }
    });
    host.function("widget.playing", |_| zgx_script::Value::from(crate::kx::launcher::now_playing().as_str()));
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
    behind(cx, host);
    let (aim, aimed) = flume::unbounded::<[f64; 6]>();
    let shaped = cx.window.clone();
    std::thread::Builder::new()
        .name("kx-notch-shape".into())
        .spawn(move || shape(shaped, aimed))
        .ok();
    let resize = cx.window.clone();
    host.action("notch.size", move |req| {
        let n = |k: &str| match req.field(k) {
            Some(zgx_script::Value::Num(v)) => *v,
            _ => 0.,
        };
        let (w, h, r, y) = (n("w").max(0.), n("h").max(0.), n("r").max(0.), n("y"));
        // (The bubble beside it, as the Dynamic Island splits in two: its
        // width, and the gap between; both 0 while there's none.)
        let (b, g) = (n("b").max(0.), n("g").max(0.));
        let interactive = matches!(req.field("interactive"), Some(zgx_script::Value::Bool(true)));
        let scale = system_scale();
        let (pw, ph) = (((w + g + b) * scale).round(), (h * scale).round());
        let left = (screen_width() - pw) / 2.;
        if let Ok(mut rect) = RECT.lock() {
            *rect = (left, (y * scale).max(0.), pw, ph);
        }
        let shown = h > 6. && y >= 0.;
        // (Coming into view: over every window, those kept on top too.)
        if shown && !SHOWN.load(std::sync::atomic::Ordering::Relaxed) {
            to_top();
        }
        SHOWN.store(shown, std::sync::atomic::Ordering::Relaxed);
        resize.set_pointer_passthrough(!interactive);
        let _ = aim.send([w, h, r, y, b, g]);
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
fn shape(_window: WindowHandle, aimed: flume::Receiver<[f64; 6]>) {
    use windows_sys::Win32::{
        Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute},
        Graphics::Gdi::{CreateRectRgn, SetWindowRgn},
    };
    let scale = system_scale();
    let (k, c) = SPRING;
    // Now and how fast, each of w, h, the corner, how far down, and the
    // bubble's width and its gap.
    let mut now = [0f64; 6];
    let mut speed = [0f64; 6];
    let mut aim = [0f64; 6];
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
                // (Left out of every capture, ours of what's behind it
                // first; KX_NOTCH_CAPTURABLE=1 keeps it in, for its PNGs.)
                if std::env::var_os("KX_NOTCH_CAPTURABLE").is_none() {
                    windows_sys::Win32::UI::WindowsAndMessaging::SetWindowDisplayAffinity(h, 0x11);
                }
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
        let settled = (0..6).all(|i| (now[i] - aim[i]).abs() < 0.25 && speed[i].abs() < 0.5);
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
                now = aim;
            }
        }
        let started = std::time::Instant::now();
        for i in 0..6 {
            // (Semi-implicit Euler, at 120 a second.)
            speed[i] += (k * (aim[i] - now[i]) - c * speed[i]) * step;
            now[i] += speed[i] * step;
        }
        if (0..6).all(|i| (now[i] - aim[i]).abs() < 0.25 && speed[i].abs() < 0.5) {
            now = aim;
            speed = [0.; 6];
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
        let (b, g) = ((now[4].max(0.) * scale).round(), (now[5].max(0.) * scale).round());
        // (The pill and its bubble, centred together.)
        let (left, top) = (((full - (w + g + b)) / 2.).round(), (now[3] * scale).round());
        // (The region is where clicks land, and room for the pill's soft
        // shadow round it: the pill grown by `pad` on every side, its corners
        // round about the pill's own centres, so the region's hard edge
        // keeps clear of the pill's smooth one, the corners' too. It used to
        // start at the pill's top, its corners shaving the pill's.)
        let pad = (SHADOW * scale).round();
        let round = |l: f64, w: f64, r: f64| {
            [
                (l - pad) as i32,
                (top - pad) as i32,
                (l + w + pad) as i32 + 1,
                (top + h + pad * 1.5) as i32 + 1,
                (2. * (r + pad)) as i32,
            ]
        };
        let cut = if w < 1. || h < 1. || top + h <= 0. { None } else { Some(round(left, w, r)) };
        let bubble = if cut.is_some() && b >= 1. { Some(round(left + w + g, b, (b.min(h) / 2.).round())) } else { None };
        if let Ok(mut c) = BUBBLE.lock() {
            *c = bubble;
        }
        if let Ok(mut c) = CUT.lock() {
            *c = Some((hwnd as isize, cut));
        }
        cut_to(hwnd, cut, bubble);
        let spent = started.elapsed();
        if let Some(rest) = std::time::Duration::from_secs_f64(step).checked_sub(spent) {
            std::thread::sleep(rest);
        }
    }
}

/// The island's shape, sprung (macOS). AppKit has no window region to cut, so
/// the window is the pill: its top stays on the screen's edge — the script
/// places the pill `y` points below the top of its own window, which is what
/// `ui.spring(aim_y)` means there — and its width and height follow the pill
/// (with its bubble, and the room the shadow needs) as it springs. Nothing of
/// the window is left over the desktop to eat a click.
#[cfg(target_os = "macos")]
fn shape(window: WindowHandle, aimed: flume::Receiver<[f64; 6]>) {
    let scale = system_scale();
    let (k, c) = SPRING;
    let mut now = [0f64; 6];
    let mut speed = [0f64; 6];
    let mut aim = [0f64; 6];
    let step = 1. / 120.;
    let mut first = true;
    loop {
        // Asleep until there's somewhere to go.
        let settled = (0..6).all(|i| (now[i] - aim[i]).abs() < 0.25 && speed[i].abs() < 0.5);
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
                now = aim;
            }
        }
        let started = std::time::Instant::now();
        for i in 0..6 {
            // (Semi-implicit Euler, at 120 a second.)
            speed[i] += (k * (aim[i] - now[i]) - c * speed[i]) * step;
            now[i] += speed[i] * step;
        }
        if (0..6).all(|i| (now[i] - aim[i]).abs() < 0.25 && speed[i].abs() < 0.5) {
            now = aim;
            speed = [0.; 6];
        }
        // The pill and its bubble, centred together; the window grows round
        // them by the shadow's room on every side.
        let wide = now[0].max(0.) + now[4].max(0.) + now[5].max(0.);
        let tall = now[3].max(0.) + now[1].max(0.);
        let points = screen_width() / scale;
        let w = ((wide + SHADOW * 2.) * scale).round() as u32;
        let h = ((tall + SHADOW * 2.) * scale).round() as u32;
        let x = (((points - wide) / 2. - SHADOW).max(0.) * scale).round() as i32;
        if w > 1 && h > 1 {
            window.set_bounds(WindowBounds { position: Some((x, 0)), size: (w, h) });
        }
        let spent = started.elapsed();
        if let Some(rest) = std::time::Duration::from_secs_f64(step).checked_sub(spent) {
            std::thread::sleep(rest);
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn shape(_window: WindowHandle, aimed: flume::Receiver<[f64; 6]>) {
    while aimed.recv().is_ok() {}
}

/// The island's window over every other, those kept on top included,
/// without taking the keys.
#[cfg(windows)]
fn to_top() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos};
    let Some(hwnd) = CUT.lock().ok().and_then(|c| *c).map(|(h, _)| h) else { return };
    // SAFETY: our own window; its place in the order only.
    unsafe {
        SetWindowPos(hwnd as _, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

#[cfg(not(windows))]
fn to_top() {}

/// Whether the island shows (more than its sliver): what's behind it is
/// copied only then.
static SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The island's glass: what's on the screen behind its window, copied a
/// few times a second while it shows (the island itself left out of the
/// copy: Windows excludes it from capture), a quarter size, for notch.luau
/// to draw blurred inside the pill (`host:notch/behind`, `notch.behind`
/// counting each copy). Windows' own blur fills the whole window past any
/// shape, so the island makes its own.
fn behind(cx: &mut WindowContext, host: &zgx_script::ScriptHost) {
    let copies = host.shared("notch.behind", 0.);
    // How light what's behind the pill is (0–1), so the tint thins over a
    // bright window and smokes a little more over a dark one.
    let light = host.shared("notch.behind.light", 0.5);
    let (tell, heard) = flume::bounded::<(u32, u32, Vec<u8>, f64)>(1);
    std::thread::Builder::new()
        .name("kx-notch-behind".into())
        .spawn(move || {
            let capturable = std::env::var_os("KX_NOTCH_CAPTURABLE").is_some();
            let mut was = false;
            let mut hidden_for = 0u32;
            loop {
                let shown = SHOWN.load(std::sync::atomic::Ordering::Relaxed);
                // Hidden, there is nothing behind it to copy: the screen is
                // left alone and only the flag is looked at, which is a load
                // and a sleep. (This loop copied the island's rectangle and
                // blurred it 2.5 times a second whether or not the island
                // was there, forever: the largest thing the app did at rest.
                // M2c, debug: the notch window alone, 38.9 % of one core.)
                if !shown && !capturable {
                    was = false;
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
                // The first copy is taken as it starts showing, so its glass
                // already has a backdrop; after that, a few times a second
                // while it shows. (In a capture, slower, and only hidden.)
                if !(shown && !was) {
                    std::thread::sleep(std::time::Duration::from_millis(if shown { 110 } else { 400 }));
                }
                if !shown && was {
                    was = false;
                    continue;
                }
                was = shown;
                // (Kept in captures for its PNGs, it would copy itself: it
                // copies only while hidden then.)
                if capturable {
                    hidden_for = if shown { 0 } else { hidden_for + 1 };
                    // (And once it's had time to go: not a ghost of it.)
                    if hidden_for < 3 {
                        continue;
                    }
                }
                if let Some((w, h, mut rgba)) = copy_behind() {
                    // Frosted here, not by a blur on the image in notch.luau:
                    // a blurred node draws from a layer of its own that the
                    // pill's rounded clip doesn't hold, the whole copy seen
                    // round the island as a box.
                    let l = brightness(&rgba, w, h);
                    frost(&mut rgba, w, h, ISLAND_BLUR * system_scale() / 4., 1.9, 0.92 + l.clamp(0., 1.) * 0.2);
                    if tell.send((w, h, rgba, l)).is_err() {
                        break;
                    }
                }
            }
        })
        .ok();
    let host = host.clone();
    cx.tasks.spawn(async move {
        let mut n = 0.;
        while let Ok((w, h, rgba, l)) = heard.recv_async().await {
            if let Ok(image) = zgx::image::ImageData::new(w, h, rgba) {
                host.image("notch/behind", Some(Arc::new(image)));
                n += 1.;
                copies.set(zgx_script::Value::from(n));
                light.set(zgx_script::Value::from(l));
            }
        }
    });
}

/// What's behind the island's window: its rectangle of the screen, a
/// quarter size (it's blurred anyway), RGBA.
#[cfg(windows)]
fn copy_behind() -> Option<(u32, u32, Vec<u8>)> {
    use windows_sys::Win32::{Foundation::RECT, UI::WindowsAndMessaging::GetWindowRect};
    let hwnd = CUT.lock().ok().and_then(|c| *c).map(|(h, _)| h)?;
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: a plain query of our own window.
    if unsafe { GetWindowRect(hwnd as _, &mut r) } == 0 {
        return None;
    }
    copy_screen(r.left, r.top, r.right - r.left, r.bottom - r.top)
}

/// That rectangle of the screen (pixels), a quarter size (it's blurred
/// anyway), RGBA: the glass's backdrop for the island and the launcher.
#[cfg(windows)]
pub fn copy_screen(left: i32, top: i32, w: i32, h: i32) -> Option<(u32, u32, Vec<u8>)> {
    use windows_sys::Win32::{
        Foundation::RECT,
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
            DeleteDC, DeleteObject, GetDC, ReleaseDC, SRCCOPY, SelectObject,
        },
    };
    let r = RECT { left, top, right: left + w, bottom: top + h };
    // SAFETY: GDI objects made here and released before returning; the
    // DIB's pixels read only while it lives.
    unsafe {
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        if w <= 0 || h <= 0 {
            return None;
        }
        let screen = GetDC(std::ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        let mut out = None;
        if !dib.is_null() && !bits.is_null() {
            let old = SelectObject(dc, dib);
            if BitBlt(dc, 0, 0, w, h, screen, r.left, r.top, SRCCOPY | CAPTUREBLT) != 0 {
                let px = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
                out = Some(quarter(px, w as usize, h as usize));
            }
            SelectObject(dc, old);
            DeleteObject(dib);
        }
        DeleteDC(dc);
        ReleaseDC(std::ptr::null_mut(), screen);
        out
    }
}

#[cfg(not(windows))]
fn copy_behind() -> Option<(u32, u32, Vec<u8>)> {
    None
}

#[cfg(not(windows))]
pub fn copy_screen(_left: i32, _top: i32, _w: i32, _h: i32) -> Option<(u32, u32, Vec<u8>)> {
    None
}

/// How light the pill's part of a capture is, 0–1. The pill sits in the
/// middle of the top of the island's window.
fn brightness(px: &[u8], w: u32, h: u32) -> f64 {
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 || px.len() < w * h * 4 {
        return 0.5;
    }
    let x0 = w / 5;
    let x1 = w.saturating_sub(x0).max(x0 + 1);
    let y1 = (h * 3 / 5).max(1);
    let mut sum = 0u64;
    let mut n = 0u64;
    let mut y = 0;
    while y < y1 {
        let mut x = x0;
        while x < x1 {
            let i = (y * w + x) * 4;
            if i + 2 < px.len() {
                sum += px[i] as u64 * 3 + px[i + 1] as u64 * 6 + px[i + 2] as u64;
                n += 1;
            }
            x += 3;
        }
        y += 3;
    }
    (sum as f64 / n.max(1) as f64) / (255.0 * 10.0)
}

/// How far the island's glass blurs what's behind it, in points.
pub const ISLAND_BLUR: f64 = 26.;
/// How far Spotlight's glass blurs what's behind it, in points.
pub const SPOTLIGHT_BLUR: f64 = 40.;

/// Liquid glass on a copy (RGBA, opaque): blurred by `sigma` texels (three
/// box blurs across and down, a Gaussian near enough), its colour deepened
/// by `saturate` and lifted by `brightness`, as CSS's filters. Done to the
/// image itself, so it draws as a plain image the glass's rounded clip
/// holds.
pub fn frost(px: &mut [u8], w: u32, h: u32, sigma: f64, saturate: f64, brightness: f64) {
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 || px.len() < w * h * 4 {
        return;
    }
    if sigma > 0.3 {
        // Three boxes of this radius make a Gaussian of `sigma`.
        let r = (((12. * sigma * sigma / 3. + 1.).sqrt() - 1.) / 2.).round().max(1.) as usize;
        let mut tmp = vec![0u8; px.len()];
        for _ in 0..3 {
            box_pass(px, &mut tmp, w, h, r, true);
            box_pass(&tmp, px, w, h, r, false);
        }
    }
    let s = saturate as f32;
    let b = brightness as f32;
    for p in px.chunks_exact_mut(4) {
        let (r, g, bl) = (p[0] as f32, p[1] as f32, p[2] as f32);
        // CSS's saturate() matrix.
        let nr = (0.213 + 0.787 * s) * r + (0.715 - 0.715 * s) * g + (0.072 - 0.072 * s) * bl;
        let ng = (0.213 - 0.213 * s) * r + (0.715 + 0.285 * s) * g + (0.072 - 0.072 * s) * bl;
        let nb = (0.213 - 0.213 * s) * r + (0.715 - 0.715 * s) * g + (0.072 + 0.928 * s) * bl;
        p[0] = (nr * b).clamp(0., 255.) as u8;
        p[1] = (ng * b).clamp(0., 255.) as u8;
        p[2] = (nb * b).clamp(0., 255.) as u8;
    }
}

/// One box blur of radius `r` from `src` into `dst`, across or down, its
/// edges held (what's past them taken as the edge's own colour).
fn box_pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, across: bool) {
    let (lines, len) = if across { (h, w) } else { (w, h) };
    let at = |line: usize, i: usize| if across { (line * w + i) * 4 } else { (i * w + line) * 4 };
    let n = (2 * r + 1) as u32;
    for line in 0..lines {
        // The window round the first texel, -r..=r, its ends held.
        let mut sum = [0u32; 3];
        for k in 0..=2 * r {
            let i = (k as isize - r as isize).clamp(0, len as isize - 1) as usize;
            let o = at(line, i);
            for c in 0..3 {
                sum[c] += src[o + c] as u32;
            }
        }
        for i in 0..len {
            let o = at(line, i);
            for c in 0..3 {
                dst[o + c] = (sum[c] / n) as u8;
            }
            dst[o + 3] = 255;
            let out = at(line, i.saturating_sub(r));
            let inn = at(line, (i + r + 1).min(len - 1));
            for c in 0..3 {
                sum[c] = sum[c] + src[inn + c] as u32 - src[out + c] as u32;
            }
        }
    }
}

/// BGRA at full size to RGBA at a quarter (each pixel the average of 4×4).
fn quarter(px: &[u8], w: usize, h: usize) -> (u32, u32, Vec<u8>) {
    let (qw, qh) = ((w / 4).max(1), (h / 4).max(1));
    let mut out = vec![0u8; qw * qh * 4];
    for y in 0..qh {
        for x in 0..qw {
            let mut sum = [0u32; 3];
            let mut n = 0;
            for dy in 0..4 {
                for dx in 0..4 {
                    let (sx, sy) = (x * 4 + dx, y * 4 + dy);
                    if sx < w && sy < h {
                        let i = (sy * w + sx) * 4;
                        sum[0] += px[i + 2] as u32;
                        sum[1] += px[i + 1] as u32;
                        sum[2] += px[i] as u32;
                        n += 1;
                    }
                }
            }
            let o = (y * qw + x) * 4;
            for c in 0..3 {
                out[o + c] = (sum[c] / n.max(1)) as u8;
            }
            out[o + 3] = 255;
        }
    }
    (qw as u32, qh as u32, out)
}

/// The shape last cut (the window, and left, top, right, bottom, corner in
/// pixels, or None for nothing): the pointer's watcher checks the window
/// still has it. (The window loses its region when the system or zgx
/// re-applies its glass or shows it, and uncut it's a box of glass the size
/// of the largest island.)
static CUT: std::sync::Mutex<Option<(isize, Option<[i32; 5]>)>> = std::sync::Mutex::new(None);
/// The bubble beside it, cut with it (the same left, top, right, bottom,
/// corner), or None while there's none.
static BUBBLE: std::sync::Mutex<Option<[i32; 5]>> = std::sync::Mutex::new(None);

/// Cuts `hwnd` to `cut`.
#[cfg(windows)]
fn cut_to(hwnd: windows_sys::Win32::Foundation::HWND, cut: Option<[i32; 5]>, bubble: Option<[i32; 5]>) {
    use windows_sys::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, RGN_OR, SetWindowRgn};
    // SAFETY: regions made here; the bubble's merged into the pill's and
    // freed, the pill's handed to our own window, which owns it from then on.
    unsafe {
        let region = match cut {
            None => CreateRectRgn(0, 0, 0, 0),
            Some([l, t, r, b, c]) => CreateRoundRectRgn(l, t, r, b, c, c),
        };
        if let (Some(_), Some([l, t, r, b, c])) = (cut, bubble) {
            let other = CreateRoundRectRgn(l, t, r, b, c, c);
            CombineRgn(region, region, other, RGN_OR);
            DeleteObject(other as _);
        }
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
    let bubble = BUBBLE.lock().ok().and_then(|b| *b);
    let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
    let mut b = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: a plain query of our own window.
    let kind = unsafe { GetWindowRgnBox(hwnd, &mut b) };
    let holds = match cut {
        None => kind == NULLREGION,
        // (The box of a round rect: its sides, give or take a pixel.)
        Some([l, t, r, bottom, _]) => {
            // (With the bubble, the box reaches to its right side.)
            let r = bubble.map_or(r, |b| b[2].max(r));
            kind != 0
                && (b.left - l.max(0)).abs() <= 1
                && (b.top - t.max(0)).abs() <= 1
                && (b.right - r).abs() <= 2
                && (b.bottom - bottom).abs() <= 2
        }
    };
    if !holds {
        cut_to(hwnd, cut, bubble);
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

#[cfg(target_os = "macos")]
fn screen_width() -> f64 {
    use core_graphics::display::CGDisplay;
    // (Pixels, as the Windows one is: every caller here works in pixels.)
    let frame = CGDisplay::main().bounds();
    frame.size.width * system_scale()
}

#[cfg(not(any(windows, target_os = "macos")))]
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

/// Where the pointer is, on a Mac: the island shows while it reaches for the
/// top edge over the middle of the screen, and while it's over the opened
/// island, and goes 450 ms after it leaves — the same as the Windows thread.
#[cfg(target_os = "macos")]
fn watch(tell: flume::Sender<bool>) {
    use core_graphics::{
        event::CGEvent,
        event_source::{CGEventSource, CGEventSourceStateID},
    };
    let scale = system_scale();
    let reach_w = 460. * scale;
    let (mut shown, mut rested, mut away) = (false, 0u32, 0u32);
    loop {
        std::thread::sleep(std::time::Duration::from_millis(if shown { 50 } else { 90 }));
        // (A null event carries the pointer's place; reading it needs no
        // permission.)
        let Ok(source) = CGEventSource::new(CGEventSourceStateID::HIDSystemState) else {
            continue;
        };
        let Ok(event) = CGEvent::new(source) else { continue };
        let p = event.location();
        let screen = screen_width();
        let (x, y) = (p.x, p.y);
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

#[cfg(not(any(windows, target_os = "macos")))]
fn watch(tell: flume::Sender<bool>) {
    // Elsewhere: always shown, as before.
    let _ = tell.send(true);
}
