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
    #[serde(default)]
    pub on_top: bool,
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
    dove_store::default_root()
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
        on_top: args["on_top"].as_bool().unwrap_or_else(|| old.as_ref().is_some_and(|o| o.on_top)),
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
pub fn call(_engine: &dove_core::Engine, tool: &str, args: &Json) -> Result<String, String> {
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
    /// Each widget's window, by its id.
    static WINDOWS: RefCell<Vec<(String, WindowHandle)>> = const { RefCell::new(Vec::new()) };
    static OPENER: RefCell<Option<Box<dyn Fn(&Widget)>>> = const { RefCell::new(None) };
}

fn widgets_value() -> Value {
    crate::panels::to_value(&serde_json::to_value(widgets()).unwrap_or_default())
}
fn calendar_value() -> Value {
    crate::panels::to_value(&serde_json::to_value(calendar()).unwrap_or_default())
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

// ── The windows ────────────────────────────────────────────────────────

fn options(w: &Widget) -> WindowOptions {
    let (pw, ph) = size(&w.kind, &w.variant);
    let scale = crate::notch::system_scale();
    let bounds = match (w.x, w.y) {
        (Some(x), Some(y)) => Some(WindowBounds { position: Some((x, y)), size: ((pw * scale) as u32, (ph * scale) as u32) }),
        _ => None,
    };
    WindowOptions {
        title: format!("Keplorayx widget: {}", w.title.clone().unwrap_or_else(|| w.kind.clone())),
        width: pw,
        height: ph,
        transparent: true,
        decorations: false,
        resizable: false,
        shadow: false,
        level: if w.on_top { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal },
        bounds,
        ..Default::default()
    }
}

/// Starts the widgets' windows (once a process): one per widget, opened,
/// changed and closed as the list changes.
pub fn start(cx: &mut WindowContext, engine: Arc<dove_core::Engine>, main: impl Fn(Option<String>) + Clone + 'static) {
    let windows = cx.windows.clone();
    let opener = move |w: &Widget| {
        let (engine, w, main) = (engine.clone(), w.clone(), main.clone());
        windows.open(options(&w), move |cx| {
            let host = crate::script_host(cx, "widget.luau");
            crate::host::install(cx, &host, engine.clone());
            crate::clock::install(cx, &host);
            crate::visuals::install(cx, &host);
            install(&host);
            system::install(cx, &host);
            host.shared("widget.id", w.id.as_str());
            let window = cx.window.clone();
            host.action("widget.close", move |_| window.close());
            // A thread, in the main window (or the main window alone).
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
            // Where you dragged it, kept (checked every two seconds; moved,
            // written once it rests).
            let (bounds, id) = (cx.window_info.bounds.clone(), w.id.clone());
            cx.tasks.spawn(async move {
                let mut last = None;
                loop {
                    zgx::timer::sleep(std::time::Duration::from_secs(2)).await;
                    let at = bounds.with_untracked(|b| b.position);
                    if at.is_some() && last.is_some() && at != last {
                        if let Some((x, y)) = at {
                            let mut list = widgets();
                            if let Some(w) = list.iter_mut().find(|w| w.id == id) {
                                (w.x, w.y) = (Some(x), Some(y));
                                let _ = write("widgets.json", &list);
                            }
                        }
                    }
                    last = at;
                }
            });
            crate::serve(cx, host, "widget.luau", false);
        })
    };
    OPENER.with(|o| *o.borrow_mut() = Some(Box::new(move |w: &Widget| {
        let handle = opener(w);
        WINDOWS.with(|ws| ws.borrow_mut().push((w.id.clone(), handle)));
    })));
    sync_windows();
}

/// Each widget with its window, and no window without its widget. A widget
/// whose look changed (variant, layer) opens again.
pub fn sync_windows() {
    let list = widgets();
    thread_local! {
        static SHOWN: RefCell<Vec<Widget>> = const { RefCell::new(Vec::new()) };
    }
    let shown = SHOWN.with(|s| s.borrow().clone());
    WINDOWS.with(|ws| {
        let mut ws = ws.borrow_mut();
        ws.retain(|(id, handle)| {
            let now = list.iter().find(|w| &w.id == id);
            let was = shown.iter().find(|w| &w.id == id);
            let same = match (now, was) {
                (Some(n), Some(w)) => n.kind == w.kind && n.variant == w.variant && n.on_top == w.on_top,
                _ => false,
            };
            if !same {
                handle.close();
            }
            same
        });
    });
    let open: Vec<String> = WINDOWS.with(|ws| ws.borrow().iter().map(|(id, _)| id.clone()).collect());
    OPENER.with(|o| {
        if let Some(open_one) = &*o.borrow() {
            for w in list.iter().filter(|w| !open.contains(&w.id)) {
                open_one(w);
            }
        }
    });
    SHOWN.with(|s| *s.borrow_mut() = list);
}

/// CPU and RAM, for the system widget: `kx.system` = { cpu (0–100), ram
/// (0–100), ram_used_gb, ram_total_gb }, every two seconds.
mod system {
    use zgx_desktop::WindowContext;
    use zgx_script::ScriptHost;

    pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
        let shared = host.shared("kx.system", crate::panels::to_value(&sample(None).0));
        cx.tasks.spawn(async move {
            let mut last = None;
            loop {
                zgx::timer::sleep(std::time::Duration::from_secs(2)).await;
                let (value, times) = sample(last);
                last = times;
                shared.set(crate::panels::to_value(&value));
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
        crate::launcher::now_playing()
    }
}
