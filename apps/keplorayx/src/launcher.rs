//! The launcher: Alt+Space from any app, a window of its own over
//! everything (as Raycast's, Spotlight's, PowerToys' Command Palette). One
//! field finds an app, a window, a file, something you copied, a thread, a
//! setting, a command; works out sums and units; searches the web; and asks
//! an agent, its answer streaming in place (launcher.luau, on Zeron's
//! command palette). Notes on what it learned from the others:
//! docs/design/launcher-study.md.
//!
//! It is made once, hidden, beside the main window and summoned by the key
//! (src/platform.rs), the tray or the notch at rest; Escape, or a click
//! elsewhere, hides it again. Hidden, it draws nothing.
//!
//! What it finds on this computer comes from here, off the window's thread:
//!
//!   launcher.search({ q, only? })  → { { kind, id, label, sub?, target } }, best
//!                             first: apps (the Start menu's shortcuts),
//!                             windows open now, what you copied (this run
//!                             only, never written down), files (your
//!                             Desktop, Documents, Downloads and projects)
//!   launcher.run({ kind, target })  open it: an app or file opened, a window
//!                             brought forward, a copy put back, a command run
//!   launcher.copy(text), launcher.hide(), launcher.main(thread?)

use std::{
    cell::RefCell,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use serde_json::{Value as Json, json};
use zgx_desktop::{WindowBounds, WindowContext, WindowHandle, WindowLevel, WindowOptions};
use zgx_script::{Shared, Value};

/// The launcher's window, in points: the palette's card and the room its
/// shadow and an answer need.
const SIZE: (f64, f64) = (720., 620.);

thread_local! {
    static LAUNCHER: RefCell<Option<(WindowHandle, Shared)>> = const { RefCell::new(None) };
}

pub fn window() -> WindowOptions {
    WindowOptions {
        title: "Keplorayx launcher".into(),
        width: SIZE.0,
        height: SIZE.1,
        transparent: true,
        decorations: false,
        resizable: false,
        shadow: false,
        level: WindowLevel::AlwaysOnTop,
        // Shown, but parked off the screen until it's summoned: zgx keeps a
        // window made hidden hidden, whatever asks to show it.
        bounds: parked(),
        ..Default::default()
    }
}

#[cfg(windows)]
fn centred() -> Option<WindowBounds> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    // SAFETY: plain queries of the primary display's size in pixels.
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    if w <= 0 || h <= 0 {
        return None;
    }
    // A little above the middle, as the others sit; in pixels at the
    // system's scale.
    let scale = crate::notch::system_scale();
    let (lw, lh) = ((SIZE.0 * scale) as i32, (SIZE.1 * scale) as i32);
    Some(WindowBounds { position: Some(((w - lw) / 2, ((h - lh) / 3).max(0))), size: (lw as u32, lh as u32) })
}

#[cfg(not(windows))]
fn centred() -> Option<WindowBounds> {
    None
}

/// Off every screen, its size kept.
fn parked() -> Option<WindowBounds> {
    let size = centred().map(|b| b.size).unwrap_or((SIZE.0 as u32, SIZE.1 as u32));
    Some(WindowBounds { position: Some((-20000, -20000)), size })
}

/// Out of sight (off the screen), and not in your way.
fn park(window: &WindowHandle) {
    if let Some(b) = parked() {
        window.set_bounds(b);
    }
}

/// Brings the launcher forward, its field empty and focused.
pub fn summon() {
    if std::env::var_os("KX_MCP_ANNOUNCE").is_some() {
        eprintln!("launcher: summon (made: {})", LAUNCHER.with(|l| l.borrow().is_some()));
    }
    LAUNCHER.with(|l| {
        if let Some((window, summoned)) = &*l.borrow() {
            let n = match summoned.get() {
                Value::Num(n) => n + 1.,
                _ => 1.,
            };
            summoned.set(Value::Num(n));
            if let Some(b) = centred() {
                window.set_bounds(b);
            }
            window.request_focus();
            // Windows lets a background process take the foreground only
            // with the foreground window's leave: attach to its input, as
            // launchers do, once the window has shown.
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(60));
                windows::to_front("Keplorayx launcher");
            });
        }
    });
}

/// Makes the launcher, hidden, once a process. `main` brings the main window
/// forward (on a thread, when given one).
pub fn open(cx: &mut WindowContext, engine: Arc<dove_core::Engine>, main: impl Fn(Option<String>) + Clone + 'static) {
    if LAUNCHER.with(|l| l.borrow().is_some()) {
        return;
    }
    clipboard::watch();
    cx.windows.open(window(), move |cx| {
        build(cx, engine.clone(), main.clone(), false);
    });
}

/// The launcher's content in `cx` (`keplorayx launcher`: alone, shown, for
/// development and its PNGs).
pub fn build(cx: &mut WindowContext, engine: Arc<dove_core::Engine>, main: impl Fn(Option<String>) + Clone + 'static, inspect: bool) {
    {
        let host = crate::script_host(cx, "launcher.luau");
        crate::host::install(cx, &host, engine.clone());
        crate::clock::install(cx, &host);
        host.window_state(&cx.ui);
        let summoned = host.shared("launcher.summoned", 0.);
        LAUNCHER.with(|l| *l.borrow_mut() = Some((cx.window.clone(), summoned)));
        // No taskbar button or Alt+Tab entry while it waits off the screen.
        cx.tasks.spawn(async {
            zgx::timer::sleep(std::time::Duration::from_millis(300)).await;
            windows::tool_window("Keplorayx launcher");
        });
        let window = cx.window.clone();
        host.action("launcher.hide", move |why| {
            if std::env::var_os("KX_MCP_ANNOUNCE").is_some() {
                eprintln!("launcher: hide ({why:?})");
            }
            park(&window);
        });
        let (window, to_main) = (cx.window.clone(), main.clone());
        host.action("launcher.main", move |thread| {
            park(&window);
            to_main(match thread {
                Value::Str(s) if !s.is_empty() => Some(s.to_string()),
                _ => None,
            });
        });
        host.action("launcher.copy", |text| {
            if let Value::Str(s) = text {
                clipboard::put(&s);
            }
        });
        let window = cx.window.clone();
        host.action("launcher.run", move |req| {
            let kind = text(&req, "kind");
            let target = text(&req, "target");
            park(&window);
            run(&kind, &target);
        });
        let roots = projects(&engine);
        // The apps and files indexed now, so the first search is quick.
        {
            let roots = roots.clone();
            std::thread::spawn(move || {
                apps();
                files(&roots);
            });
        }
        host.async_action("launcher.search", move |req| {
            let (q, only, roots) = (text(&req, "q"), text(&req, "only"), roots.clone());
            async move {
                let found = crate::host::off_thread(move || match only.as_str() {
                    "" => search(&q, &roots),
                    kind => only_of(kind, &q),
                })
                .await
                .ok_or("failed")?;
                Ok::<_, String>(crate::panels::to_value(&found))
            }
        });
        crate::serve(cx, host, "launcher.luau", inspect);
    }
}

fn text(v: &Value, key: &str) -> String {
    match v.field(key) {
        Some(Value::Str(s)) => s.to_string(),
        _ => String::new(),
    }
}

fn projects(engine: &dove_core::Engine) -> Vec<PathBuf> {
    engine.store.projects().map(|ps| ps.into_iter().map(|p| p.path).collect()).unwrap_or_default()
}

// ── Finding ─────────────────────────────────────────────────────────────

/// How well `label` answers `q`: its letters in order, a word's start and a
/// run counting most; `None` when some letter isn't there.
pub fn score(q: &str, label: &str) -> Option<f64> {
    if q.is_empty() {
        return Some(0.);
    }
    let (q, l): (Vec<char>, Vec<char>) = (q.to_lowercase().chars().collect(), label.to_lowercase().chars().collect());
    let lower = label.to_lowercase();
    let mut s = 0.;
    if lower.starts_with(&q.iter().collect::<String>()) {
        s += 100.;
    } else if lower.contains(&q.iter().collect::<String>()) {
        s += 60.;
    }
    let contained = s > 0.;
    let (mut at, mut run, mut last, mut starts, mut letters) = (0usize, 0., usize::MAX, 0usize, 0usize);
    for c in &q {
        if *c == ' ' {
            continue;
        }
        let found = l[at..].iter().position(|x| x == c)?;
        let i = at + found;
        let start = i == 0 || !l[i - 1].is_alphanumeric();
        run = if last != usize::MAX && i == last + 1 { run + 1. } else { 0. };
        s += 1. + run * 2. + if start { 6. } else { 0. };
        starts += start as usize;
        letters += 1;
        last = i;
        at = i + 1;
    }
    // Letters scattered through a long name are no match: it must hold
    // what's typed, or most of it start its words ("vsc", Visual Studio Code).
    if !contained && starts * 2 < letters {
        return None;
    }
    // Shorter labels a little ahead of long ones that happen to hold it.
    Some(s - l.len() as f64 * 0.05)
}

fn search(q: &str, roots: &[PathBuf]) -> Json {
    let q = q.trim();
    let mut out: Vec<(f64, Json)> = Vec::new();
    // Nothing typed: what you copied lately, the open windows' first few.
    if q.is_empty() {
        for (i, text) in clipboard::recent().into_iter().take(5).enumerate() {
            out.push((-(i as f64), clip_item(&text)));
        }
        return Json::Array(unique(out));
    }
    for app in apps() {
        if let Some(s) = score(q, &app.name) {
            out.push((s + 20., json!({ "kind": "app", "id": format!("app:{}", app.path.display()), "label": app.name, "sub": "Application", "target": app.path.to_string_lossy() })));
        }
    }
    for w in windows::list() {
        if let Some(s) = score(q, &w.title) {
            out.push((s + 10., json!({ "kind": "window", "id": format!("window:{}", w.id), "label": w.title, "sub": w.app, "target": w.id.to_string() })));
        }
    }
    for text in clipboard::recent() {
        if text.to_lowercase().contains(&q.to_lowercase()) {
            out.push((30., clip_item(&text)));
        }
    }
    if q.chars().count() >= 2 {
        for f in files(roots) {
            let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if let Some(s) = score(q, &name) {
                let sub = f.parent().map(|p| p.display().to_string()).unwrap_or_default();
                out.push((s, json!({ "kind": "file", "id": format!("file:{}", f.display()), "label": name, "sub": sub, "target": f.to_string_lossy() })));
            }
        }
    }
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    Json::Array(unique(out).into_iter().take(40).collect())
}

/// Each row once, by its id (the palette's list keys its rows by id: a row
/// twice would abort zgx).
fn unique(rows: Vec<(f64, Json)>) -> Vec<Json> {
    let mut seen = std::collections::HashSet::new();
    rows.into_iter().map(|(_, j)| j).filter(|j| seen.insert(j["id"].as_str().unwrap_or_default().to_string())).collect()
}

/// One kind alone, all of it that answers `q` (nothing typed: all of it):
/// the Clipboard history page's (`clip`), Switch window's (`window`).
fn only_of(kind: &str, q: &str) -> Json {
    let q = q.trim();
    let mut out: Vec<(f64, Json)> = Vec::new();
    match kind {
        "clip" => {
            for (i, text) in clipboard::recent().into_iter().enumerate() {
                if q.is_empty() || text.to_lowercase().contains(&q.to_lowercase()) {
                    out.push((-(i as f64), clip_item(&text)));
                }
            }
        }
        "window" => {
            for w in windows::list() {
                if let Some(s) = score(q, &w.title) {
                    out.push((s, json!({ "kind": "window", "id": format!("window:{}", w.id), "label": w.title, "sub": w.app, "target": w.id.to_string() })));
                }
            }
        }
        _ => {}
    }
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    Json::Array(unique(out))
}

fn clip_item(text: &str) -> Json {
    let line: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(90).collect();
    json!({ "kind": "clip", "id": format!("clip:{}", text.len() as u64 ^ hash(text)), "label": line, "sub": "Copied", "target": text })
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn run(kind: &str, target: &str) {
    match kind {
        "app" if target.starts_with("shell:") => {
            #[cfg(windows)]
            let _ = std::process::Command::new("explorer").arg(target).spawn();
        }
        "app" | "file" => crate::open_external(target),
        "reveal" => reveal(target),
        "url" => crate::open_external(target),
        "window" => {
            if let Ok(id) = target.parse::<isize>() {
                windows::bring(id);
            }
        }
        "clip" => clipboard::put(target),
        "system" => system(target),
        _ => tracing::warn!("launcher: nothing runs a `{kind}`"),
    }
}

fn reveal(path: &str) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(format!("/select,{path}")).spawn();
    #[cfg(not(windows))]
    if let Some(dir) = std::path::Path::new(path).parent() {
        crate::open_external(&dir.to_string_lossy());
    }
}

/// The system's commands, by name. Only ones that lose nothing: no shut
/// down or restart from a stray Enter.
fn system(what: &str) {
    #[cfg(windows)]
    {
        let spawn = |exe: &str, args: &[&str]| {
            let _ = std::process::Command::new(exe).args(args).spawn();
        };
        match what {
            // SAFETY: a plain request to lock this session.
            "lock" => unsafe {
                windows_sys::Win32::System::Shutdown::LockWorkStation();
            },
            "task_manager" => spawn("taskmgr", &[]),
            "screenshot" => spawn("explorer", &["ms-screenclip:"]),
            "recycle_bin" => spawn("explorer", &["shell:RecycleBinFolder"]),
            "downloads" => spawn("explorer", &["shell:Downloads"]),
            s if s.starts_with("ms-settings:") => spawn("explorer", &[s]),
            other => tracing::warn!("launcher: no system command `{other}`"),
        }
    }
    #[cfg(not(windows))]
    let _ = what;
}

// ── Apps: the Start menu's shortcuts ────────────────────────────────────

struct App {
    name: String,
    path: PathBuf,
}

fn apps() -> &'static [App] {
    static APPS: OnceLock<Vec<App>> = OnceLock::new();
    APPS.get_or_init(|| {
        let mut out = store_apps();
        let mut dirs = Vec::new();
        #[cfg(windows)]
        {
            if let Some(p) = std::env::var_os("ProgramData") {
                dirs.push(PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs"));
            }
            if let Some(p) = std::env::var_os("APPDATA") {
                dirs.push(PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs"));
            }
        }
        for dir in dirs {
            for entry in walkdir::WalkDir::new(dir).max_depth(4).into_iter().flatten() {
                let path = entry.path();
                let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                if !matches!(ext.as_str(), "lnk" | "url" | "appref-ms") {
                    continue;
                }
                let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                let low = name.to_lowercase();
                // (Uninstallers and help files are shortcuts too.)
                if low.contains("uninstall") || low.ends_with("readme") || low.contains("documentation") {
                    continue;
                }
                if !out.iter().any(|a| a.name.eq_ignore_ascii_case(&name)) {
                    out.push(App { name, path: path.to_path_buf() });
                }
            }
        }
        out
    })
}

/// Every app the Start menu lists, packaged ones (Notepad, Calculator,
/// Settings) among them, as Windows' own `Get-StartApps` names them; each
/// opened through `shell:AppsFolder`.
#[cfg(windows)]
fn store_apps() -> Vec<App> {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", "Get-StartApps | ForEach-Object { $_.Name + \"`t\" + $_.AppID }"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let Ok(out) = out else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let (name, id) = line.trim_end().split_once('\t')?;
            let low = name.to_lowercase();
            if name.is_empty() || low.contains("uninstall") {
                return None;
            }
            Some(App { name: name.to_string(), path: PathBuf::from(format!(r"shell:AppsFolder\{id}")) })
        })
        .collect()
}

#[cfg(not(windows))]
fn store_apps() -> Vec<App> {
    Vec::new()
}

// ── Files: a small index of where your things are ───────────────────────

fn files(roots: &[PathBuf]) -> Vec<PathBuf> {
    static INDEX: Mutex<Option<(Instant, Vec<PathBuf>)>> = Mutex::new(None);
    let mut index = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, list)) = &*index
        && at.elapsed() < Duration::from_secs(600)
    {
        return list.clone();
    }
    let mut places: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from) {
        for d in ["Desktop", "Documents", "Downloads", "Pictures"] {
            places.push(home.join(d));
        }
    }
    places.extend(roots.iter().cloned());
    const SKIP: &[&str] = &["node_modules", "target", ".git", "dist", "build", ".venv", "__pycache__", ".next"];
    // (A project inside one of these, or one inside another, is walked
    // once: each file listed once, as a list's rows must be.)
    let mut list = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for place in places {
        let walk = walkdir::WalkDir::new(&place).max_depth(5).into_iter().filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !(n.starts_with('.') && e.depth() > 0) && !SKIP.contains(&n.as_ref())
        });
        for entry in walk.flatten().take(8000) {
            if entry.depth() > 0 && seen.insert(entry.path().to_path_buf()) {
                list.push(entry.into_path());
            }
        }
    }
    *index = Some((Instant::now(), list.clone()));
    list
}

// ── What you copied ─────────────────────────────────────────────────────

pub mod clipboard {
    use std::sync::Mutex;

    static RECENT: Mutex<Vec<String>> = Mutex::new(Vec::new());
    const KEEP: usize = 60;

    /// Newest first; this run only.
    pub fn recent() -> Vec<String> {
        RECENT.lock().map(|r| r.clone()).unwrap_or_default()
    }

    pub fn put(text: &str) {
        if let Ok(mut c) = arboard::Clipboard::new() {
            let _ = c.set_text(text.to_string());
        }
    }

    /// Notices each new copy (Windows says when the clipboard changes by a
    /// number; it is read only then). Off with `kx.clipboard_history` false
    /// (read at the start).
    pub fn watch() {
        #[cfg(windows)]
        std::thread::Builder::new()
            .name("kx-clipboard".into())
            .spawn(|| {
                use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;
                let mut seen = 0u32;
                let mut board = None;
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(600));
                    // SAFETY: a plain query.
                    let now = unsafe { GetClipboardSequenceNumber() };
                    if now == seen {
                        continue;
                    }
                    seen = now;
                    if board.is_none() {
                        board = arboard::Clipboard::new().ok();
                    }
                    let Some(b) = board.as_mut() else { continue };
                    let Ok(text) = b.get_text() else { continue };
                    if text.trim().is_empty() || text.len() > 20_000 {
                        continue;
                    }
                    if let Ok(mut r) = RECENT.lock() {
                        r.retain(|t| *t != text);
                        r.insert(0, text);
                        r.truncate(KEEP);
                    }
                }
            })
            .ok();
    }
}

// ── Windows open now ────────────────────────────────────────────────────

pub use windows::now_playing;

mod windows {
    pub struct Open {
        pub id: isize,
        pub title: String,
        pub app: String,
    }

    #[cfg(windows)]
    pub fn list() -> Vec<Open> {
        use windows_sys::Win32::{
            Foundation::{HWND, LPARAM},
            UI::WindowsAndMessaging::{
                EnumWindows, GWL_EXSTYLE, GetWindow, GetWindowLongPtrW, GetWindowTextW, GetWindowThreadProcessId,
                GW_OWNER, IsWindowVisible, WS_EX_TOOLWINDOW,
            },
        };
        unsafe extern "system" fn each(hwnd: HWND, out: LPARAM) -> i32 {
            // SAFETY: `out` is the Vec this call was given; `hwnd` a live window.
            unsafe {
                let out = &mut *(out as *mut Vec<Open>);
                if IsWindowVisible(hwnd) == 0 || !GetWindow(hwnd, GW_OWNER).is_null() {
                    return 1;
                }
                if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0 {
                    return 1;
                }
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, &mut pid);
                if pid == std::process::id() {
                    return 1;
                }
                let mut buf = [0u16; 256];
                let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
                if n <= 0 {
                    return 1;
                }
                let title = String::from_utf16_lossy(&buf[..n as usize]);
                if title == "Program Manager" {
                    return 1;
                }
                let app = title.rsplit(" - ").next().unwrap_or("").to_string();
                out.push(Open { id: hwnd as isize, title, app });
                1
            }
        }
        let mut out: Vec<Open> = Vec::new();
        // SAFETY: the callback only reads windows and pushes to `out`.
        unsafe { EnumWindows(Some(each), &mut out as *mut _ as LPARAM) };
        out
    }

    /// A music player's window title while it plays ("Artist - Song"): the
    /// players whose window says so (Spotify, foobar2000, VLC, AIMP,
    /// MusicBee, iTunes).
    #[cfg(windows)]
    pub fn now_playing() -> String {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, HWND},
            System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW},
            UI::WindowsAndMessaging::GetWindowThreadProcessId,
        };
        const PLAYERS: &[&str] = &["spotify.exe", "foobar2000.exe", "vlc.exe", "aimp.exe", "musicbee.exe", "itunes.exe"];
        for w in list() {
            if !w.title.contains(" - ") {
                continue;
            }
            let mut pid = 0u32;
            // SAFETY: a window we just listed.
            unsafe { GetWindowThreadProcessId(w.id as HWND, &mut pid) };
            // SAFETY: a query-only handle, closed below.
            let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            if process.is_null() {
                continue;
            }
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            // SAFETY: a buffer and its length in characters.
            let ok = unsafe { QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len) };
            // SAFETY: the handle opened above.
            unsafe { CloseHandle(process) };
            if ok == 0 {
                continue;
            }
            let exe = String::from_utf16_lossy(&buf[..len as usize]).to_lowercase();
            if PLAYERS.iter().any(|p| exe.ends_with(p)) {
                // (VLC's "Song - VLC media player".)
                return w.title.trim_end_matches(" - VLC media player").to_string();
            }
        }
        String::new()
    }

    #[cfg(not(windows))]
    pub fn now_playing() -> String {
        String::new()
    }

    /// Our window titled `title`, to the front and focused.
    #[cfg(windows)]
    pub fn to_front(title: &str) {
        use windows_sys::Win32::{
            System::Threading::{AttachThreadInput, GetCurrentThreadId},
            UI::WindowsAndMessaging::{
                BringWindowToTop, FindWindowW, GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible, SW_SHOW,
                SetForegroundWindow, ShowWindow,
            },
        };
        let wide: Vec<u16> = title.encode_utf16().chain([0]).collect();
        // SAFETY: a valid title; the threads attached are detached again.
        unsafe {
            let ours = FindWindowW(std::ptr::null(), wide.as_ptr());
            if ours.is_null() {
                return;
            }
            let front = GetForegroundWindow();
            let theirs = GetWindowThreadProcessId(front, std::ptr::null_mut());
            let me = GetCurrentThreadId();
            let attached = theirs != 0 && theirs != me && AttachThreadInput(me, theirs, 1) != 0;
            if std::env::var_os("KX_MCP_ANNOUNCE").is_some() {
                eprintln!("launcher: to front, visible before: {}", IsWindowVisible(ours));
            }
            if IsWindowVisible(ours) == 0 {
                ShowWindow(ours, SW_SHOW);
            }
            BringWindowToTop(ours);
            SetForegroundWindow(ours);
            if attached {
                AttachThreadInput(me, theirs, 0);
            }
        }
    }

    #[cfg(not(windows))]
    pub fn to_front(_title: &str) {}

    /// Our window titled `title`: a tool window (no taskbar button, no
    /// Alt+Tab entry).
    #[cfg(windows)]
    pub fn tool_window(title: &str) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            FindWindowW, GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
        };
        let wide: Vec<u16> = title.encode_utf16().chain([0]).collect();
        // SAFETY: our own window's extended styles.
        unsafe {
            let h = FindWindowW(std::ptr::null(), wide.as_ptr());
            if h.is_null() {
                return;
            }
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
            SetWindowLongPtrW(h, GWL_EXSTYLE, ((ex | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW) as isize);
        }
    }

    #[cfg(not(windows))]
    pub fn tool_window(_title: &str) {}

    #[cfg(windows)]
    pub fn bring(id: isize) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow};
        let hwnd = id as windows_sys::Win32::Foundation::HWND;
        // SAFETY: a window handle we listed; a stale one is ignored by Windows.
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
    }

    #[cfg(not(windows))]
    pub fn list() -> Vec<Open> {
        Vec::new()
    }

    #[cfg(not(windows))]
    pub fn bring(_id: isize) {}
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn words_starts_match_and_letters_scattered_dont() {
        assert!(score("vsc", "Visual Studio Code").is_some());
        assert!(score("vs", "Previous").is_none());
    }

    #[test]
    fn a_missing_letter_is_no_match() {
        assert!(score("xyz", "Firefox").is_none());
    }

    #[test]
    fn a_prefix_beats_a_match_inside() {
        assert!(score("note", "Notepad").unwrap() > score("note", "OneNote").unwrap());
    }
}
