//! Keplorayx on zgx. The interface is Luau under `ui/`, read from the source
//! tree and reloaded as it's saved, keeping its state; this host makes the
//! windows, runs Dove's engine in this process and answers the scripts'
//! calls (src/host.rs).
//!
//! Run: cargo run -p keplorayx -- [main | gallery] [--inspect]
//!
//! A development build keeps its data apart from the shipping Dove's
//! (`~/.dove`): `%LOCALAPPDATA%\keplorayx-zgx-dev` unless `DOVE_HOME` says
//! otherwise, so its threads, agents and settings never meet a running Dove's.

mod host;
mod transcript;
mod transparency;

use std::{path::PathBuf, time::Duration};

use zgx::compose::component;
use zgx_desktop::{Application, FontData, Glass, GlassMaterial, Titlebar, WindowContext, WindowOptions};
use zgx_script::ScriptHost;

/// Where `ui.persist` keeps its files.
const APP_ID: &str = "keplorayx-zgx";

enum Screen {
    Main,
    Gallery,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // For scripts/perf: when main began, and (once the first view is built)
    // how long that took. `DOVE_STARTUP_TRACE=1`.
    let started = std::time::Instant::now();
    let started_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // Before any thread starts: the environment is still this thread's alone.
    // Windows has no HOME, only USERPROFILE, and the scripts' `~` reads HOME.
    #[cfg(windows)]
    if std::env::var_os("HOME").is_none()
        && let Some(profile) = std::env::var_os("USERPROFILE")
    {
        // SAFETY: one thread so far.
        unsafe { std::env::set_var("HOME", profile) };
    }
    // No type check after each reload unless asked: `zgx check ui` is the check.
    if std::env::var_os("ZGX_CHECK").is_none() {
        // SAFETY: one thread so far.
        unsafe { std::env::set_var("ZGX_CHECK", "0") };
    }
    if std::env::var_os("DOVE_HOME").is_none() {
        // SAFETY: one thread so far.
        unsafe { std::env::set_var("DOVE_HOME", dev_home()) };
    }

    let mut screen = Screen::Main;
    let mut inspect = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "main" => screen = Screen::Main,
            "gallery" => screen = Screen::Gallery,
            "--inspect" => inspect = true,
            other => return Err(format!("unknown argument `{other}`: expected main, gallery or --inspect").into()),
        }
    }

    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("KX_LOG").unwrap_or_else(|_| "warn".into()))
        .try_init();

    let root = dove_store::default_root();
    let engine = dove_core::Engine::new(root)?;
    // Which agents are installed: PATH lookups, off the window's way.
    {
        let engine = engine.clone();
        std::thread::spawn(move || {
            engine.rescan_harnesses();
        });
    }

    let entry = match screen {
        Screen::Main => "app.luau",
        Screen::Gallery => "gallery.luau",
    };
    zgx_script::compile_ahead(&ui_dir().join(entry), &[("kx", ui_dir().join("kx").as_path())]);

    let mut application = Application::new();
    for font in fonts() {
        application = application.font(font?);
    }
    application.window(window(&screen)).run(move |cx| {
        let host = script_host(cx, entry);
        if let Screen::Main = screen {
            host::install(cx, &host, engine.clone());
        }
        serve(cx, host, entry, inspect);
        if std::env::var_os("DOVE_STARTUP_TRACE").is_some() {
            eprintln!(
                "dove-startup: main_unix_ms={started_unix_ms} first_render_ms={:.1}",
                started.elapsed().as_secs_f64() * 1e3
            );
        }
    })?;
    Ok(())
}

/// Orbit's type (all OFL, `fonts/`): Inter for the interface, Inter Display
/// for headings, JetBrains Mono for agent output and code.
fn fonts() -> impl Iterator<Item = Result<FontData, Box<dyn std::error::Error>>> {
    [
        &include_bytes!("../fonts/Inter-Regular.ttf")[..],
        include_bytes!("../fonts/Inter-Italic.ttf"),
        include_bytes!("../fonts/Inter-Medium.ttf"),
        include_bytes!("../fonts/Inter-SemiBold.ttf"),
        include_bytes!("../fonts/Inter-Bold.ttf"),
        include_bytes!("../fonts/InterDisplay-SemiBold.ttf"),
        include_bytes!("../fonts/JetBrainsMono-Regular.ttf"),
        include_bytes!("../fonts/JetBrainsMono-Medium.ttf"),
        include_bytes!("../fonts/JetBrainsMono-Italic.ttf"),
    ]
    .into_iter()
    .map(|bytes| FontData::new(std::sync::Arc::<[u8]>::from(bytes)).map_err(Into::into))
}

/// A development build's data: apart from the shipping app's `~/.dove`.
fn dev_home() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("keplorayx-zgx-dev")
}

/// Keplorayx's window: an inset title bar with the system's caption buttons,
/// over the system backdrop (Mica on Windows 11), which the theme tints.
fn window(screen: &Screen) -> WindowOptions {
    let (title, autosave) = match screen {
        Screen::Main => ("Keplorayx", "keplorayx-main"),
        Screen::Gallery => ("Keplorayx · Gallery", "keplorayx-gallery"),
    };
    // Development: a fixed size for the design PNGs (`KX_SIZE=1240x820`).
    let (width, height) = std::env::var("KX_SIZE")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or((1240., 820.));
    WindowOptions {
        title: title.into(),
        width,
        height,
        min_size: Some((760., 500.)),
        frame_autosave: std::env::var_os("KX_SIZE").is_none().then(|| autosave.into()),
        decorations: cfg!(target_os = "macos"),
        titlebar: Titlebar::Inset,
        glass: Some(Glass::new(GlassMaterial::UnderWindowBackground).plain()),
        ..Default::default()
    }
}

/// The scripts: this crate's `ui/`, or `KX_UI` (a frozen copy for a test rig).
fn ui_dir() -> PathBuf {
    match std::env::var_os("KX_UI") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui"),
    }
}

/// A host for the script `entry` under `ui/`, with what every window gives
/// its scripts.
fn script_host(cx: &mut WindowContext, entry: &str) -> ScriptHost {
    let host = ScriptHost::new(&cx.ui.runtime, ui_dir().join(entry)).app_id(APP_ID);
    zgx_run::install_fetcher(&host);
    host.window_chrome(cx.window_info.titlebar_area.clone(), cx.window_info.controls.clone());
    host.window_zoom(&cx.ui);
    host.window_size(cx.viewport.clone());
    host.shared("platform", std::env::consts::OS);
    // Development: open on a gallery page, in a theme, or on a fixture
    // (tools/shot.ps1 sets these for the design PNGs).
    for (key, var) in [("dev.page", "KX_PAGE"), ("dev.theme", "KX_THEME"), ("dev.fixture", "KX_FIXTURE")] {
        host.shared(key, std::env::var(var).unwrap_or_default().as_str());
    }
    host.shared("dev.solid", std::env::var_os("KX_SOLID").is_some_and(|v| v == "1"));
    transparency::install(cx, &host);
    let window = cx.window.clone();
    host.action("close", move |_| window.close());
    host
}

/// Mounts the script, reloads it as it's saved, and serves devtools when asked.
fn serve(cx: &mut WindowContext, host: ScriptHost, entry: &str, inspect: bool) {
    if let Err(error) = host.reload() {
        eprintln!("keplorayx: {entry}: {error}");
    }
    cx.tasks.spawn(host.watch(Duration::from_millis(250)));
    if inspect {
        zgx_script::inspect::prepare();
    }
    let view = host.view();
    let handle = cx.render(component(move |_| view));
    if inspect {
        let session = zgx_script::inspect::Session::new(&cx.ui, handle, &host, Some(cx.inspector.clone()));
        let tasks = cx.tasks.clone();
        match zgx_script::inspect::serve(session, entry, move |task| tasks.spawn(task)) {
            Ok(server) => {
                eprintln!("keplorayx: devtools for {entry} at {}", server.path().display());
                // Kept for the window's life.
                std::mem::forget(server);
            }
            Err(error) => eprintln!("keplorayx: devtools: {error}"),
        }
    }
}
