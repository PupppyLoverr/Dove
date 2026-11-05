//! Keplorayx on zgx. The interface is Luau under `ui/`, read from the source
//! tree and reloaded as it's saved, keeping its state; this host makes the
//! windows, runs Dove's engine in this process and answers the scripts'
//! calls (src/host.rs).
//!
//! Run: cargo run -p keplorayx -- [main | gallery | intro | onboarding | notch] [--inspect]
//!
//! A first run (no `kx.onboarded` in the store) opens with First Light, the
//! intro and setup (src/intro.rs); `DOVE_SKIP_INTRO=1` goes straight to the
//! main window.
//!
//! A development build keeps its data apart from the shipping Dove's
//! (`~/.dove`): `%LOCALAPPDATA%\keplorayx-zgx-dev` unless `DOVE_HOME` says
//! otherwise, so its threads, agents and settings never meet a running Dove's.

mod account;
mod host;
mod intro;
mod notch;
mod panels;
mod platform;
mod preview;
mod transcript;
mod transparency;

use std::{path::PathBuf, rc::Rc, sync::Arc, time::Duration};

use zgx::compose::component;
use zgx_desktop::{Application, FontData, Glass, GlassMaterial, Titlebar, WindowContext, WindowOptions};
use zgx_script::{ScriptHost, Value};

/// Where `ui.persist` keeps its files.
const APP_ID: &str = "keplorayx-zgx";

#[derive(Clone, Copy, PartialEq)]
enum Screen {
    Main,
    Gallery,
    /// First Light: the overlay, handing off to setup.
    Intro,
    /// Setup on its own (development, or the intro skipped).
    Onboarding,
    /// The notch on its own (development: its fixtures and PNGs).
    Notch,
}

/// The store's word that setup has been finished once.
const ONBOARDED: &str = "kx.onboarded";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Started as a remote-host launcher (`dove-hosts/<host>/<agent>`): hop to
    // WSL, SSH or a Cua Space and exit, before any window or store exists.
    if let Some(code) = dove_core::host::run_launcher() {
        std::process::exit(code);
    }
    // `keplorayx node …`: the headless engine other machines run (session
    // daemon, provider bridge, port pipe). No window, no store.
    if std::env::args_os().nth(1).is_some_and(|a| a == "node") {
        let args: Vec<String> = std::env::args().skip(2).collect();
        std::process::exit(dove_core::node::main(&args));
    }
    // An agent started from inside another agent's terminal must not carry
    // its markers on to the agents Keplorayx starts (Claude would stop
    // saving transcripts, and resume would break), as Dove's shell clears.
    for var in ["CLAUDE_CODE_CHILD_SESSION", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SSE_PORT"] {
        // SAFETY: first thing in main, before any thread exists.
        unsafe { std::env::remove_var(var) };
    }
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

    let mut screen = None;
    let mut inspect = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "main" => screen = Some(Screen::Main),
            "gallery" => screen = Some(Screen::Gallery),
            "intro" => screen = Some(Screen::Intro),
            "onboarding" => screen = Some(Screen::Onboarding),
            "notch" => screen = Some(Screen::Notch),
            "--inspect" => inspect = true,
            other => {
                return Err(
                    format!("unknown argument `{other}`: expected main, gallery, intro, onboarding or --inspect").into()
                );
            }
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

    // No screen asked for: First Light the first time, the main window after.
    let screen = screen.unwrap_or_else(|| {
        let skip = std::env::var_os("DOVE_SKIP_INTRO").is_some_and(|v| v == "1");
        if skip || engine.store.setting(ONBOARDED).is_some() { Screen::Main } else { Screen::Intro }
    });

    let entry = entry(screen);
    zgx_script::compile_ahead(
        &ui_dir().join(entry),
        &[("kx", ui_dir().join("kx").as_path()), ("ds", ui_dir().join("ds").as_path())],
    );

    let mut application = Application::new();
    for font in fonts() {
        application = application.font(font?);
    }
    let options = match screen {
        Screen::Intro => intro::overlay_window(),
        Screen::Onboarding => intro::setup_window(),
        Screen::Notch => notch::window(),
        _ => window(screen),
    };
    application.window(options).run(move |cx| {
        match screen {
            Screen::Main => open_main(cx, engine.clone(), inspect),
            Screen::Gallery => {
                let host = script_host(cx, entry);
                serve(cx, host, entry, inspect);
            }
            Screen::Onboarding => {
                let host = open_onboarding(cx, engine.clone());
                host.shared("onboarding.arrived", true);
            }
            Screen::Notch => {
                notch::open(cx, engine.clone(), |_| {});
            }
            Screen::Intro => {
                let host = script_host(cx, entry);
                let engine = engine.clone();
                let setup: Rc<dyn Fn(&mut WindowContext) -> ScriptHost> =
                    Rc::new(move |cx| open_onboarding(cx, engine.clone()));
                intro::install(cx, &host, setup);
                serve(cx, host, entry, inspect);
            }
        }
        if std::env::var_os("DOVE_STARTUP_TRACE").is_some() {
            eprintln!(
                "dove-startup: main_unix_ms={started_unix_ms} first_render_ms={:.1}",
                started.elapsed().as_secs_f64() * 1e3
            );
        }
    })?;
    Ok(())
}

fn entry(screen: Screen) -> &'static str {
    match screen {
        Screen::Main => "app.luau",
        Screen::Gallery => "gallery.luau",
        Screen::Intro => "intro.luau",
        Screen::Onboarding => "onboarding.luau",
        Screen::Notch => "notch.luau",
    }
}

/// The main window's content in `cx`: the engine's host and app.luau.
fn open_main(cx: &mut WindowContext, engine: Arc<dove_core::Engine>, inspect: bool) {
    let host = script_host(cx, "app.luau");
    host::install(cx, &host, engine.clone());
    account::install(&host, &cx.tasks, engine.clone());
    host.shared("kx.data_dir", engine.store.root().to_string_lossy().as_ref());
    // The notch, over every window: a click on it brings this window forward
    // on the thread it showed (`notch.focus`, which app.luau follows).
    let focus_thread = host.shared("notch.focus", "");
    if notch::enabled() {
        let main_window = cx.window.clone();
        let engine = engine.clone();
        cx.windows.open(notch::window(), move |cx| {
            let (window, focus) = (main_window.clone(), focus_thread.clone());
            let asked = std::cell::Cell::new(0u32);
            notch::open(cx, engine.clone(), move |thread| {
                window.request_focus();
                if let Some(id) = thread {
                    asked.set(asked.get() + 1);
                    focus.set(Value::from(format!("{id}#{}", asked.get()).as_str()));
                }
            });
        });
    }
    // The tray and the keys that reach Keplorayx from any app, once a
    // process: what they ask comes to this window as `kx.shell`
    // ("<word>#<n>", n so the same ask twice is two), which app.luau follows.
    thread_local! {
        static SHELL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if !SHELL.with(|s| s.replace(true)) && std::env::var_os("KX_TRAY").is_none_or(|v| v != "0") {
        let (heard, tray) = platform::start(engine.store.flag("kx.global_keys", true));
        // It lives as long as the process.
        std::mem::forget(tray);
        let (window, shell) = (cx.window.clone(), host.shared("kx.shell", ""));
        cx.tasks.spawn(async move {
            let mut n = 0u32;
            while let Ok(what) = heard.recv_async().await {
                if what == platform::Shell::Quit {
                    std::process::exit(0);
                }
                // Answering from the keys leaves you where you are.
                if !matches!(what, platform::Shell::Allow | platform::Shell::Deny) {
                    window.request_focus();
                }
                n += 1;
                shell.set(Value::from(format!("{}#{n}", what.word()).as_str()));
            }
        });
    }
    // Settings › Appearance › Open setup: First Light's steps again, in a
    // window of their own.
    let windows = cx.windows.clone();
    host.action("app.onboarding", move |_| {
        let engine = engine.clone();
        windows.open(intro::setup_window(), move |cx| {
            let host = open_onboarding(cx, engine.clone());
            host.shared("onboarding.arrived", true);
        });
    });
    serve(cx, host, "app.luau", inspect);
}

/// Setup in `cx`: the engine's host (agents, projects) and onboarding.luau.
/// Finishing it records that it's done, opens the main window and closes
/// this one.
fn open_onboarding(cx: &mut WindowContext, engine: Arc<dove_core::Engine>) -> ScriptHost {
    let host = script_host(cx, "onboarding.luau");
    host::install(cx, &host, engine.clone());
    let (window, windows) = (cx.window.clone(), cx.windows.clone());
    host.action("onboarding.finish", move |_| {
        let _ = engine.store.set_setting(ONBOARDED, "1");
        let engine = engine.clone();
        windows.open(self::window(Screen::Main), move |cx| open_main(cx, engine.clone(), false));
        window.close();
    });
    let dev_step = std::env::var("KX_STEP").unwrap_or_default();
    host.shared("onboarding.step", dev_step.as_str());
    serve(cx, host.clone(), "onboarding.luau", false);
    host
}

/// Orbit's type (all OFL, `fonts/`, as Zeron sets it): Geist for the
/// interface, Geist Mono for agent output and code, Instrument Serif for
/// the few words said large (a greeting, First Light).
fn fonts() -> impl Iterator<Item = Result<FontData, Box<dyn std::error::Error>>> {
    [
        &include_bytes!("../fonts/Geist.ttf")[..],
        include_bytes!("../fonts/Geist-Italic.ttf"),
        include_bytes!("../fonts/Geist-Medium.ttf"),
        include_bytes!("../fonts/Geist-SemiBold.ttf"),
        include_bytes!("../fonts/Geist-Bold.ttf"),
        include_bytes!("../fonts/GeistMono.ttf"),
        include_bytes!("../fonts/GeistMono-Medium.ttf"),
        include_bytes!("../fonts/InstrumentSerif-Regular.ttf"),
        include_bytes!("../fonts/InstrumentSerif-Italic.ttf"),
    ]
    .into_iter()
    .map(|bytes| FontData::new(std::sync::Arc::<[u8]>::from(bytes)).map_err(Into::into))
}

/// A development build's data: apart from the shipping app's `~/.dove`.
/// Opens `target` (an http(s) address or a file's path) in the system's app
/// for it. Nothing else: no other scheme is handed to the shell.
pub fn open_external(target: &str) {
    let web = target.starts_with("https://") || target.starts_with("http://");
    if !web && !std::path::Path::new(target).exists() {
        return;
    }
    #[cfg(windows)]
    let spawned = std::process::Command::new("explorer").arg(target).spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(target).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(target).spawn();
    if let Err(e) = spawned {
        tracing::warn!("open {target}: {e}");
    }
}

fn dev_home() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("keplorayx-zgx-dev")
}

/// Keplorayx's window: an inset title bar with the system's caption buttons,
/// over the system backdrop (Mica on Windows 11), which the theme tints.
fn window(screen: Screen) -> WindowOptions {
    let (title, autosave) = match screen {
        Screen::Gallery => ("Keplorayx · Gallery", "keplorayx-gallery"),
        _ => ("Keplorayx", "keplorayx-main"),
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
    // zgx's popup windows have no system backdrop on Windows yet, so menus
    // and popovers draw solid there (zgx-asks/popup-glass-windows).
    host.shared("system.popups_blur", !cfg!(windows));
    // Development: open on a gallery page, in a theme, or on a fixture
    // (tools/shot.ps1 sets these for the design PNGs).
    for (key, var) in [("dev.page", "KX_PAGE"), ("dev.theme", "KX_THEME"), ("dev.fixture", "KX_FIXTURE")] {
        host.shared(key, std::env::var(var).unwrap_or_default().as_str());
    }
    host.shared("dev.solid", std::env::var_os("KX_SOLID").is_some_and(|v| v == "1"));
    transparency::install(cx, &host);
    let window = cx.window.clone();
    host.action("close", move |_| window.close());
    // A web address or a file, in the system's own app for it.
    host.action("open_url", |target| {
        if let Value::Str(t) = target {
            open_external(&t);
        }
    });
    // The window's own chrome (its caption buttons, its Mica) in the theme's
    // appearance: `true` dark, `false` light.
    let window = cx.window.clone();
    host.action("appearance", move |dark| {
        use zgx::theme::Appearance;
        let dark = matches!(dark, Value::Bool(true));
        window.set_theme(Some(if dark { Appearance::Dark } else { Appearance::Light }));
    });
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
