//! Desktop widgets (as macOS's, Rainmeter's, uWidgets'): each a small zgx
//! window of its own, on the desktop or floating over it, drawn by
//! widget.luau with the app's design system. Kinds: clock, calendar,
//! agents, usage, ask, threads, system (CPU and RAM), music, note, and
//! custom (a Luau view, as `show_visual`'s); each in variants (small,
//! medium, large, and a kind's own).
//!
//! You add, move and change them (Settings › Widgets, a widget's menu), and
//! so can agents (src/mcp.rs: `set_widget`, `remove_widget`, `list_widgets`).
//! The calendar is Keplorayx's own, kept here; agents add to it and edit it
//! (`calendar_add`, `calendar_remove`, `calendar_list`), and the calendar
//! widget shows it.
//!
//! Kept in `$DOVE_HOME/widgets.json` and `$DOVE_HOME/calendar.json`; scripts
//! see `kx.widgets` and `kx.calendar`, and `kx.system` (CPU and RAM, every
//! two seconds while a system widget shows).

use std::{cell::RefCell, path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use zgx_desktop::{WindowBounds, WindowContext, WindowHandle, WindowLevel, WindowOptions};
use zgx_script::{ScriptHost, Shared, Value};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Widget {
    pub id: String,
    pub kind: String,
    #[serde(default = "medium")]
    pub variant: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub settings: Json,
    #[serde(default)]
    pub script: Option<String>,
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
}

fn medium() -> String {
    "medium".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub date: String,
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub minutes: Option<u32>,
    #[serde(default)]
    pub notes: Option<String>,
}

pub const KINDS: &[&str] = &["clock", "calendar", "agents", "usage", "ask", "threads", "system", "music", "note", "custom"];

/// Each kind's variants, the first its default.
pub fn variants(kind: &str) -> &'static [&'static str] {
    match kind {
        "clock" => &["digital", "analog", "world", "small"],
        "calendar" => &["agenda", "month", "small"],
        "system" => &["bars", "small", "graph"],
        "music" => &["medium", "small"],
        _ => &["medium", "small", "large"],
    }
}

/// A variant's size, in points.
pub fn size(kind: &str, variant: &str) -> (f64, f64) {
    match (kind, variant) {
        (_, "small") => (164., 164.),
        (_, "large") => (344., 344.),
        ("calendar", "month") => (344., 300.),
        ("clock", "world") => (344., 164.),
        ("ask", _) => (344., 120.),
        ("system", "graph") => (344., 164.),
        _ => (344., 164.),
    }
}

fn home() -> PathBuf {
    crate::kx_account::data_dir()
}

fn read<T: for<'a> Deserialize<'a> + Default>(name: &str) -> T {
    std::fs::read_to_string(home().join(name)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write<T: Serialize>(name: &str, value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(home().join(name), text).map_err(|e| e.to_string())
}

pub fn widgets() -> Vec<Widget> {
    read("widgets.json")
}
pub fn calendar() -> Vec<Event> {
    let mut list: Vec<Event> = read("calendar.json");
    list.sort_by(|a, b| (&a.date, &a.time).cmp(&(&b.date, &b.time)));
    list
}

fn new_id(prefix: &str) -> String {
    format!(
        "{prefix}-{:x}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_micros()
    )
}

fn valid_date(d: &str) -> bool {
    chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok()
}
fn valid_time(t: &str) -> bool {
    chrono::NaiveTime::parse_from_str(t, "%H:%M").is_ok()
}

/// Adds or changes a widget (`args` as `set_widget` takes them); its id.
pub fn set(args: &Json) -> Result<String, String> {
    let kind = args["kind"].as_str().unwrap_or_default();
    if !KINDS.contains(&kind) {
        return Err(format!("kind: one of {}", KINDS.join(", ")));
    }
    if kind == "custom" {
        let script = args["script"].as_str().unwrap_or_default();
        if script.trim().is_empty() {
            return Err("a custom widget needs `script`".into());
        }
        zeron_visual::check(script, zeron_visual::seed_for("widget")).map_err(|e| e.message)?;
    }
    let mut list = widgets();
    let id = args["id"].as_str().map(str::to_string).unwrap_or_else(|| new_id("w"));
    let variant = args["variant"].as_str().unwrap_or(variants(kind)[0]).to_string();
    if !variants(kind).contains(&variant.as_str()) {
        return Err(format!("variant of a {kind}: one of {}", variants(kind).join(", ")));
    }
    let at = list.iter().position(|w| w.id == id);
    let old = at.map(|i| list[i].clone());
    let n = |k: &str| args[k].as_f64().map(|v| v as i32);
    let widget = Widget {
        id: id.clone(),
        kind: kind.into(),
        variant,
        title: args["title"].as_str().map(str::to_string).or_else(|| old.as_ref().and_then(|o| o.title.clone())),
        settings: if args["settings"].is_object() { args["settings"].clone() } else { old.as_ref().map(|o| o.settings.clone()).unwrap_or(json!({})) },
        script: args["script"].as_str().map(str::to_string).or_else(|| old.as_ref().and_then(|o| o.script.clone())),
        x: n("x").or_else(|| old.as_ref().and_then(|o| o.x)),
        y: n("y").or_else(|| old.as_ref().and_then(|o| o.y)),
    };
    // A new one with no place: down the desktop's right side, under the
    // last there (points, from the desktop's top left).
    let widget = if widget.x.is_none() || widget.y.is_none() {
        let (sw, _) = desk::area_points();
        let (w, _) = size(&widget.kind, &widget.variant);
        let x = (sw - w - 28.) as i32;
        let y = list
            .iter()
            .filter(|o| o.id != widget.id && o.x.is_some_and(|ox| (ox - x).abs() < 120))
            .map(|o| o.y.unwrap_or(0) + size(&o.kind, &o.variant).1 as i32 + 12)
            .max()
            .unwrap_or(28);
        Widget { x: Some(x), y: Some(y), ..widget }
    } else {
        widget
    };
    match at {
        Some(i) => list[i] = widget,
        None => list.push(widget),
    }
    write("widgets.json", &list)?;
    Ok(id)
}

pub fn remove(id: &str) -> Result<(), String> {
    let mut list = widgets();
    let before = list.len();
    list.retain(|w| w.id != id);
    if list.len() == before {
        return Err(format!("no widget `{id}`"));
    }
    write("widgets.json", &list)
}

pub fn add_event(args: &Json) -> Result<String, String> {
    let title = args["title"].as_str().unwrap_or_default().trim().to_string();
    let date = args["date"].as_str().unwrap_or_default().to_string();
    if title.is_empty() || !valid_date(&date) {
        return Err("title, and date as YYYY-MM-DD".into());
    }
    let time = args["time"].as_str().map(str::to_string).filter(|t| !t.is_empty());
    if time.as_deref().is_some_and(|t| !valid_time(t)) {
        return Err("time as HH:MM, 24-hour".into());
    }
    let mut list = calendar();
    let id = args["id"].as_str().map(str::to_string).unwrap_or_else(|| new_id("e"));
    let event = Event {
        id: id.clone(),
        title,
        date,
        time,
        minutes: args["minutes"].as_u64().map(|m| m as u32),
        notes: args["notes"].as_str().map(str::to_string),
    };
    match list.iter().position(|e| e.id == id) {
        Some(i) => list[i] = event,
        None => list.push(event),
    }
    write("calendar.json", &list)?;
    Ok(id)
}

pub fn remove_event(id: &str) -> Result<(), String> {
    let mut list = calendar();
    let before = list.len();
    list.retain(|e| e.id != id);
    if list.len() == before {
        return Err(format!("no event `{id}`"));
    }
    write("calendar.json", &list)
}

/// The bridge's calls (src/mcp.rs), on the window's thread.
pub fn call(tool: &str, args: &Json) -> Result<String, String> {
    let answer = match tool {
        "list_widgets" => serde_json::to_string_pretty(&widgets()).map_err(|e| e.to_string()),
        "set_widget" => set(args).map(|id| format!("Widget `{id}` is on the desktop.")),
        "remove_widget" => remove(args["id"].as_str().unwrap_or_default()).map(|()| "Removed.".into()),
        "calendar_list" => {
            let today = chrono::Local::now().date_naive();
            let from = args["from"].as_str().and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()).unwrap_or(today);
            let to = args["to"]
                .as_str()
                .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                .unwrap_or(today + chrono::Days::new(14));
            let list: Vec<Event> = calendar()
                .into_iter()
                .filter(|e| chrono::NaiveDate::parse_from_str(&e.date, "%Y-%m-%d").is_ok_and(|d| d >= from && d <= to))
                .collect();
            serde_json::to_string_pretty(&list).map_err(|e| e.to_string())
        }
        "calendar_add" => add_event(args).map(|id| format!("On the calendar as `{id}`.")),
        "calendar_remove" => remove_event(args["id"].as_str().unwrap_or_default()).map(|()| "Removed.".into()),
        other => Err(format!("no tool `{other}`")),
    };
    publish();
    if answer.is_ok() && tool.ends_with("_widget") {
        sync_windows();
    }
    answer
}

// ── The scripts' side ──────────────────────────────────────────────────

thread_local! {
    static SHARED: RefCell<Vec<(Shared, Shared)>> = const { RefCell::new(Vec::new()) };
    /// The desktop surface every widget is drawn on.
    static SURFACE: RefCell<Option<WindowHandle>> = const { RefCell::new(None) };
}

fn widgets_value() -> Value {
    crate::kx::to_value(&serde_json::to_value(widgets()).unwrap_or_default())
}
fn calendar_value() -> Value {
    crate::kx::to_value(&serde_json::to_value(calendar()).unwrap_or_default())
}

/// Every window's `kx.widgets` and `kx.calendar`, told again.
pub fn publish() {
    let (w, c) = (widgets_value(), calendar_value());
    SHARED.with(|s| {
        for (ws, cs) in s.borrow().iter() {
            ws.set(w.clone());
            cs.set(c.clone());
        }
    });
}

/// The scripts' calls, on any window's host.
pub fn install(host: &ScriptHost) {
    let ws = host.shared("kx.widgets", widgets_value());
    let cs = host.shared("kx.calendar", calendar_value());
    SHARED.with(|s| s.borrow_mut().push((ws, cs)));
    host.action("kx.widget.set", |req| {
        let json = value_json(&req);
        if let Err(e) = set(&json) {
            tracing::warn!("widget: {e}");
        }
        publish();
        sync_windows();
    });
    host.action("kx.widget.remove", |id| {
        if let Value::Str(id) = id {
            let _ = remove(&id);
            publish();
            sync_windows();
        }
    });
    host.action("kx.event.set", |req| {
        if let Err(e) = add_event(&value_json(&req)) {
            tracing::warn!("calendar: {e}");
        }
        publish();
    });
    host.action("kx.event.remove", |id| {
        if let Value::Str(id) = id {
            let _ = remove_event(&id);
            publish();
        }
    });
}

/// A script's value as JSON.
pub fn value_json(v: &Value) -> Json {
    serde_json::from_str(&v.to_json()).unwrap_or(Json::Null)
}

// ── The desktop surface ────────────────────────────────────────────────
//
// Every widget on one surface (widgets.luau), not a window each: a
// transparent, frameless window over the desktop's work area, kept at the
// bottom of every window, with no taskbar button or Alt+Tab entry, never
// taking the keys from your apps (Windows' tool and no-activate styles). Its
// window region is the widgets' own boxes, so a click between them reaches
// the desktop's icons. Dragging a widget moves it on the surface (its place
// kept, in points from the desktop's top left).

/// Opens the surface (once a process), shown while there are widgets.
pub fn start(cx: &mut WindowContext, main: impl Fn(Option<String>) + Clone + 'static) {
    let (x, y, w, h) = desk::area_pixels();
    let scale = crate::kx::notch::system_scale();
    let options = WindowOptions {
        title: "Keplorayx widgets".into(),
        width: w as f64 / scale,
        height: h as f64 / scale,
        transparent: true,
        decorations: false,
        resizable: false,
        shadow: false,
        level: WindowLevel::Normal,
        bounds: Some(WindowBounds { position: Some((x, y)), size: (w as u32, h as u32) }),
        visible: !widgets().is_empty(),
        ..Default::default()
    };
    let handle = cx.windows.open(options, move |cx| {
        let host = crate::script_host(cx, "widgets.luau");
        crate::kx::engine_for(cx, &host);
        crate::kx::clock::install(cx, &host);
        crate::visuals::install(cx, &host);
        install(&host);
        system::install(cx, &host);
        crate::kx::media::install(cx, &host);
        wallpaper::install(cx, &host);
        let to_main = main.clone();
        host.action("widget.main", move |thread| {
            to_main(match thread {
                Value::Str(s) if !s.is_empty() => Some(s.to_string()),
                _ => None,
            });
        });
        host.action("widget.media", |what| {
            if let Value::Str(what) = what {
                media::press(&what);
            }
        });
        host.function("widget.playing", |_| Value::from(media::playing().as_str()));
        host.action("widget.ask", |_| crate::kx::launcher::summon());
        // A drag: the whole surface takes the pointer while it lasts, then
        // the widget's new place is kept and the region follows it.
        host.action("widgets.dragging", |on| desk::shape(matches!(on, Value::Bool(true))));
        host.action("widget.move", |req| {
            let (Some(Value::Str(id)), Some(Value::Num(x)), Some(Value::Num(y))) =
                (req.field("id"), req.field("x"), req.field("y"))
            else {
                return;
            };
            let mut list = widgets();
            if let Some(w) = list.iter_mut().find(|w| w.id == id.as_ref()) {
                (w.x, w.y) = (Some(*x as i32), Some(*y as i32));
                let _ = write("widgets.json", &list);
            }
            publish();
            desk::shape(false);
        });
        crate::serve(cx, host, "widgets.luau", false);
        // Its styles and place once the window exists.
        cx.tasks.spawn(async {
            zgx::timer::sleep(std::time::Duration::from_millis(400)).await;
            desk::settle();
            desk::shape(false);
        });
    });
    SURFACE.with(|s| *s.borrow_mut() = Some(handle));
    desk::keep_at_bottom();
}

/// The surface shown while there are widgets, its region their boxes.
pub fn sync_windows() {
    let any = !widgets().is_empty();
    SURFACE.with(|s| {
        if let Some(h) = &*s.borrow() {
            h.set_visible(any);
        }
    });
    desk::shape(false);
}

/// The desktop's side of it (Windows); elsewhere a plain window.
mod desk {
    use super::{size, widgets};

    /// The work area (the desktop less the taskbar), in pixels.
    #[cfg(windows)]
    pub fn area_pixels() -> (i32, i32, i32, i32) {
        use windows_sys::Win32::{
            Foundation::RECT,
            UI::WindowsAndMessaging::{SPI_GETWORKAREA, SystemParametersInfoW},
        };
        let mut r = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
        // SAFETY: a RECT to write into.
        unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut r as *mut RECT).cast(), 0) };
        (r.left, r.top, r.right - r.left, r.bottom - r.top)
    }

    #[cfg(not(windows))]
    pub fn area_pixels() -> (i32, i32, i32, i32) {
        (0, 0, 1440, 900)
    }

    /// The work area's size in points.
    pub fn area_points() -> (f64, f64) {
        let (_, _, w, h) = area_pixels();
        let scale = crate::kx::notch::system_scale();
        (w as f64 / scale, h as f64 / scale)
    }

    #[cfg(windows)]
    fn hwnd() -> Option<windows_sys::Win32::Foundation::HWND> {
        use windows_sys::Win32::{
            Foundation::{HWND, LPARAM},
            UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId},
        };
        unsafe extern "system" fn each(h: HWND, out: LPARAM) -> i32 {
            // SAFETY: `out` is the Option this call was given.
            unsafe {
                let mut pid = 0u32;
                GetWindowThreadProcessId(h, &mut pid);
                if pid != std::process::id() {
                    return 1;
                }
                let mut buf = [0u16; 64];
                let n = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
                if n > 0 && String::from_utf16_lossy(&buf[..n as usize]) == "Keplorayx widgets" {
                    *(out as *mut Option<HWND>) = Some(h);
                    return 0;
                }
                1
            }
        }
        let mut found: Option<HWND> = None;
        // SAFETY: the callback writes only `found`.
        unsafe { EnumWindows(Some(each), &mut found as *mut _ as LPARAM) };
        found
    }

    /// No taskbar button, no Alt+Tab entry, never activated; at the bottom.
    #[cfg(windows)]
    pub fn settle() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        };
        let Some(h) = hwnd() else { return };
        // SAFETY: our own window's extended styles.
        unsafe {
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
            let ex = (ex | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) & !WS_EX_APPWINDOW;
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex as isize);
        }
        lower(h);
    }

    #[cfg(windows)]
    fn lower(h: windows_sys::Win32::Foundation::HWND) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HWND_BOTTOM, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SetWindowPos,
        };
        // SAFETY: our own window.
        unsafe { SetWindowPos(h, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER) };
    }

    /// Keeps it under every window: a click on a widget would raise it.
    #[cfg(windows)]
    pub fn keep_at_bottom() {
        std::thread::Builder::new()
            .name("kx-widgets-desk".into())
            .spawn(|| {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(700));
                    if let Some(h) = hwnd() {
                        // Show desktop (Win+D) minimizes it with the rest:
                        // widgets stay on the desktop, so it comes back.
                        use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, SW_SHOWNOACTIVATE, ShowWindow};
                        // SAFETY: our own window.
                        unsafe {
                            if IsIconic(h) != 0 {
                                ShowWindow(h, SW_SHOWNOACTIVATE);
                            }
                        }
                        lower(h);
                    }
                }
            })
            .ok();
    }

    /// Its region: each widget's box, or, `whole`, the whole surface (a drag
    /// under way).
    #[cfg(windows)]
    pub fn shape(whole: bool) {
        use windows_sys::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, RGN_OR, SetWindowRgn};
        let Some(h) = hwnd() else { return };
        let scale = crate::kx::notch::system_scale();
        // SAFETY: regions made here, handed to the window (which owns the
        // one it's given) or deleted.
        unsafe {
            if whole {
                SetWindowRgn(h, std::ptr::null_mut(), 1);
                return;
            }
            let region = CreateRectRgn(0, 0, 0, 0);
            for w in widgets() {
                let (pw, ph) = size(&w.kind, &w.variant);
                let (x, y) = (w.x.unwrap_or(0) as f64, w.y.unwrap_or(0) as f64);
                let r = CreateRectRgn(
                    (x * scale) as i32,
                    (y * scale) as i32,
                    ((x + pw) * scale) as i32,
                    ((y + ph) * scale) as i32,
                );
                CombineRgn(region, region, r, RGN_OR);
                DeleteObject(r);
            }
            SetWindowRgn(h, region, 1);
        }
    }

    #[cfg(not(windows))]
    pub fn settle() {}
    #[cfg(not(windows))]
    pub fn keep_at_bottom() {}
    #[cfg(not(windows))]
    pub fn shape(_whole: bool) {}
}

/// CPU and RAM, for the system widget: `kx.system` = { cpu (0–100), ram
/// (0–100), ram_used_gb, ram_total_gb }, every two seconds.
mod system {
    use zgx_desktop::WindowContext;
    use zgx_script::ScriptHost;

    pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
        let shared = host.shared("kx.system", crate::kx::to_value(&sample(None).0));
        cx.tasks.spawn(async move {
            let mut last = None;
            loop {
                zgx::timer::sleep(std::time::Duration::from_secs(2)).await;
                let (value, times) = sample(last);
                last = times;
                shared.set(crate::kx::to_value(&value));
            }
        });
    }

    #[cfg(windows)]
    fn sample(last: Option<(u64, u64)>) -> (serde_json::Value, Option<(u64, u64)>) {
        use windows_sys::Win32::{
            Foundation::FILETIME,
            System::{
                SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
                Threading::GetSystemTimes,
            },
        };
        let zero = || FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut idle, mut kernel, mut user) = (zero(), zero(), zero());
        // SAFETY: three FILETIMEs to write into.
        unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) };
        let t = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
        let (idle, busy_total) = (t(idle), t(kernel) + t(user));
        let cpu = match last {
            Some((li, lt)) if busy_total > lt => {
                let total = busy_total - lt;
                let idle = idle.saturating_sub(li);
                100. * (total.saturating_sub(idle)) as f64 / total as f64
            }
            _ => 0.,
        };
        // SAFETY: a zeroed struct with its length set, as the call asks.
        let mut mem: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        mem.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        unsafe { GlobalMemoryStatusEx(&mut mem) };
        let gb = |b: u64| b as f64 / (1024. * 1024. * 1024.);
        let used = mem.ullTotalPhys.saturating_sub(mem.ullAvailPhys);
        (
            serde_json::json!({
                "cpu": cpu.round(),
                "ram": mem.dwMemoryLoad,
                "ram_used_gb": (gb(used) * 10.).round() / 10.,
                "ram_total_gb": (gb(mem.ullTotalPhys) * 10.).round() / 10.,
            }),
            Some((idle, busy_total)),
        )
    }

    #[cfg(not(windows))]
    fn sample(_: Option<(u64, u64)>) -> (serde_json::Value, Option<(u64, u64)>) {
        (serde_json::json!({ "cpu": 0, "ram": 0, "ram_used_gb": 0, "ram_total_gb": 0 }), None)
    }
}

/// The media keys, for the notch's Music tab too.
pub fn press_media(what: &str) {
    media::press(what);
}

/// The music widget's: the system's media keys, and what's playing as a
/// player's window says it (Spotify's "Artist - Song", a browser's tab).
mod media {
    #[cfg(windows)]
    pub fn press(what: &str) {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_MEDIA_NEXT_TRACK,
            VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK,
        };
        let vk = match what {
            "play" => VK_MEDIA_PLAY_PAUSE,
            "next" => VK_MEDIA_NEXT_TRACK,
            "previous" => VK_MEDIA_PREV_TRACK,
            _ => return,
        };
        let key = |up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 },
            },
        };
        let inputs = [key(false), key(true)];
        // SAFETY: two well-formed keyboard inputs.
        unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    }

    #[cfg(not(windows))]
    pub fn press(_what: &str) {}

    /// "Artist - Song" from a player's window, or "".
    pub fn playing() -> String {
        crate::kx::launcher::now_playing()
    }
}

/// The widgets' glass: the desktop's wallpaper as Windows draws it (its own
/// copy, `TranscodedWallpaper`, filled to the screen), cut to the work
/// area the surface covers and made a quarter size (it's blurred anyway),
/// as `host:widgets/wallpaper`; `widgets.wallpaper` counts each change (a
/// slideshow's next picture), and is 0 while there's none. Each widget
/// draws it blurred under its tint, pinned where it sits on the desktop, so
/// it reads as glass over the wallpaper, whatever window is in front.
mod wallpaper {
    use std::{sync::Arc, time::Duration};

    use zgx_desktop::WindowContext;
    use zgx_script::{ScriptHost, Value};

    fn file() -> Option<std::path::PathBuf> {
        let app = std::env::var_os("APPDATA")?;
        let p = std::path::PathBuf::from(app).join("Microsoft/Windows/Themes/TranscodedWallpaper");
        p.is_file().then_some(p)
    }

    #[cfg(windows)]
    fn screen() -> (u32, u32) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        // SAFETY: plain queries.
        unsafe { (GetSystemMetrics(SM_CXSCREEN).max(1) as u32, GetSystemMetrics(SM_CYSCREEN).max(1) as u32) }
    }

    #[cfg(not(windows))]
    fn screen() -> (u32, u32) {
        (1440, 900)
    }

    /// The work area's part of the wallpaper, a quarter size, RGBA.
    fn load() -> Option<(u32, u32, Vec<u8>)> {
        let picture = image::open(file()?).ok()?.to_rgba8();
        let (sw, sh) = screen();
        // Filled as Windows' default: covering the screen, centred.
        let (pw, ph) = (picture.width() as f64, picture.height() as f64);
        let k = (sw as f64 / pw).max(sh as f64 / ph);
        let (fw, fh) = ((pw * k).round() as u32, (ph * k).round() as u32);
        let filled = image::imageops::resize(&picture, fw / 4, fh / 4, image::imageops::FilterType::Triangle);
        let (ox, oy) = ((fw - sw) / 2, (fh - sh) / 2);
        let (ax, ay, aw, ah) = super::desk::area_pixels();
        let (x, y) = (((ox as i32 + ax).max(0) / 4) as u32, ((oy as i32 + ay).max(0) / 4) as u32);
        let (w, h) = ((aw.max(4) / 4) as u32, (ah.max(4) / 4) as u32);
        let (w, h) = (w.min(filled.width().saturating_sub(x)), h.min(filled.height().saturating_sub(y)));
        let cut = image::imageops::crop_imm(&filled, x, y, w, h).to_image();
        Some((cut.width(), cut.height(), cut.into_raw()))
    }

    pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
        let changes = host.shared("widgets.wallpaper", 0.);
        // (The surface's size in points: the wallpaper's, drawn under each
        // widget where it sits.)
        let (aw, ah) = super::desk::area_points();
        host.shared("widgets.area", Value::record([("w", Value::from(aw)), ("h", Value::from(ah))]));
        let host = host.clone();
        cx.tasks.spawn(async move {
            let mut seen = None;
            let mut n = 0.;
            loop {
                let stamp = file().and_then(|f| f.metadata().ok()).and_then(|m| m.modified().ok());
                if stamp != seen {
                    seen = stamp;
                    if let Some(Some((w, h, rgba))) = crate::kx::off_thread(load).await
                        && let Ok(image) = zgx::image::ImageData::new(w, h, rgba)
                    {
                        host.image("widgets/wallpaper", Some(Arc::new(image)));
                        n += 1.;
                        changes.set(Value::from(n));
                    }
                }
                // (A slideshow moves on every few minutes at most.)
                zgx::timer::sleep(Duration::from_secs(60)).await;
            }
        });
    }
}
