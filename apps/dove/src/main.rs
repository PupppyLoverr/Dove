#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod browser;
mod canvas;
mod changes;
mod composer;
mod design;
mod file_icons;
mod files_panel;
mod history;
mod home;
mod intro;
mod keymap;
mod kit;
mod loaders;
mod markdown;
mod markdown_extra;
mod math;
mod mcp_browser;
mod motion;
mod notify;
mod office;
mod pages;
mod palette;
mod perf_trace;
mod picker_memory;
mod pull_requests;
mod remote;
mod roll_text;
mod settings;
mod shell;
mod side_chat;
mod side_panel;
mod sidebar;
mod surface_chrome;
mod team;
mod terminal;
mod theme;
mod thread;
mod tool_calls;
mod transcript;
mod ui;
mod updates;
mod voice;
mod wallpaper;
mod widget;

use std::sync::Arc;

use dove_core::Engine;
use gpui::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, div, prelude::FluentBuilder as _,
    px, size,
};

use crate::intro::{Intro, IntroFinished};
use crate::theme::{Accent, Theme, appearance_from_window};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod actions {
    gpui::actions!(
        dove,
        [
            Quit,
            NewSession,
            AddProject,
            ToggleSidebar,
            ToggleRightPanel,
            GoBack,
            GoForward,
            OpenSettings,
            ToggleTerminal,
            CommandPalette,
            CloseTab,
            NextTab,
            PrevTab,
            RenameThread,
            OpenThreads,
            OpenKanban,
            FocusComposer
        ]
    );

    /// Open a worktree file in the side panel's preview (chat `path:line`
    /// links, diff headers, tool rows).
    #[derive(Clone, PartialEq, Debug, gpui::Action)]
    #[action(namespace = dove, no_json)]
    pub struct OpenPath {
        pub path: String,
        pub line: Option<u32>,
    }

    /// Open a subagent's own chat in the side panel, by its spawning call.
    #[derive(Clone, PartialEq, Debug, gpui::Action)]
    #[action(namespace = dove, no_json)]
    pub struct OpenSubagent {
        pub call: String,
    }

    /// Ctrl+N: the Nth open thread tab.
    #[derive(Clone, PartialEq, Debug, gpui::Action)]
    #[action(namespace = dove, no_json)]
    pub struct GoToTab {
        pub index: usize,
    }
}

/// Rebuild the theme from settings and the OS appearance, and push the
/// matching window background (blur or opaque) to every window.
pub fn apply_theme(engine: &Arc<Engine>, cx: &mut App) {
    let store = &engine.store;
    let appearance = match store.setting("appearance").as_deref() {
        Some("dark") => theme::Appearance::Dark,
        Some("light") => theme::Appearance::Light,
        _ => appearance_from_window(cx.window_appearance()),
    };
    let accent = Accent::from_id(&store.setting("accent").unwrap_or_default());
    let surface =
        theme::Surface::from_settings(store.setting("surface").as_deref(), store.setting("frosted").as_deref());
    // Re-applying an unchanged theme is not free: on Windows, setting the
    // window backdrop can itself raise an appearance notification, which
    // would call back here and repaint forever.
    if let Some(current) = cx.try_global::<Theme>()
        && current.appearance == appearance
        && current.accent_choice == accent
        && current.surface_style == surface
    {
        return;
    }
    let theme = Theme::new(appearance, accent, surface);
    assets::set_icon_color(theme.text_muted);
    theme::set_current_appearance(appearance);
    let background = theme.window_background();
    cx.set_global(theme);
    let windows = cx.windows();
    cx.defer(move |cx| {
        for w in windows {
            let _ = w.update(cx, |_, window, _| window.set_background_appearance(background));
        }
    });
    cx.refresh_windows();
}

/// The window root: the shell, with the first-run intro drawn over it until
/// it finishes.
struct Root {
    shell: Entity<shell::Shell>,
    intro: Option<Entity<Intro>>,
    toasts: Entity<kit::Toasts>,
    _intro_done: Option<Subscription>,
}

impl Root {
    fn new(engine: Arc<Engine>, show_intro: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let toasts = kit::Toasts::init(cx);
        let shell = cx.new(|cx| shell::Shell::new(engine, window, cx));
        let weak = shell.downgrade();
        toasts.update(cx, |t, _| {
            t.on_open = Some(std::rc::Rc::new(move |task, window, cx| {
                let _ = weak.update(cx, |s, cx| s.open_task(task, window, cx));
            }));
        });
        let (intro, sub) = if show_intro {
            let intro = cx.new(|cx| Intro::new(window, cx));
            let sub = cx.subscribe_in(&intro, window, |this, _, _: &IntroFinished, window, cx| {
                this.intro = None;
                this._intro_done = None;
                // The intro held focus so keys could skip it; hand it back.
                let composer = this.shell.read(cx).composer.clone();
                composer::focus(&composer, window, cx);
                cx.notify();
            });
            (Some(intro), Some(sub))
        } else {
            (None, None)
        };
        Self { shell, intro, toasts, _intro_done: sub }
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("Root");
        perf_trace::mark_first_render(window);
        div()
            .relative()
            .size_full()
            .when(perf_trace::input_enabled(), |d| {
                d.capture_key_down(|e, window, _| perf_trace::key_down(&e.keystroke.unparse(), window))
            })
            .child(self.shell.clone())
            .child(self.toasts.clone())
            .children(self.intro.clone())
    }
}

/// 1360×860, shrunk (keeping its shape) to fit small or highly scaled displays.
fn default_window_size(cx: &App) -> gpui::Size<gpui::Pixels> {
    let (w, h) = (1360.0_f32, 860.0_f32);
    let fit = cx.primary_display().map_or(1.0, |d| {
        let area = d.visible_bounds().size;
        (f32::from(area.width) * 0.9 / w).min(f32::from(area.height) * 0.9 / h).min(1.0)
    });
    size(px(w * fit), px(h * fit))
}

/// Windows applies a background launcher's "minimized" show state to the
/// first window; activation alone does not undo it.
#[cfg(windows)]
fn restore_if_minimized(window: &Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, SW_RESTORE, ShowWindow};
    if let Ok(handle) = HasWindowHandle::window_handle(window)
        && let RawWindowHandle::Win32(h) = handle.as_raw()
    {
        let hwnd = h.hwnd.get() as windows_sys::Win32::Foundation::HWND;
        // SAFETY: a live top-level window owned by this process.
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
        }
    }
}

#[cfg(not(windows))]
fn restore_if_minimized(_: &Window) {}

fn main() {
    // `dove --mcp-browser`: the stdio MCP bridge agents spawn to drive the
    // in-app browser. No window, no store.
    if std::env::args_os().nth(1).is_some_and(|a| a == "--mcp-browser") {
        mcp_browser::run();
        return;
    }
    perf_trace::mark_main();
    // Dove can be launched from inside an agent session (a terminal in
    // Claude Code, say). Its child-session markers must not reach the agents
    // Dove starts: Claude would stop saving transcripts, breaking resume.
    for var in ["CLAUDE_CODE_CHILD_SESSION", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SSE_PORT"] {
        // SAFETY: first thing in main, before any thread exists.
        unsafe { std::env::remove_var(var) };
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("DOVE_LOG").unwrap_or_else(|_| "warn,dove=info".into()),
        )
        .init();

    let engine = match Engine::new(dove_core::store::default_root()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Dove could not start: {e:#}");
            std::process::exit(1);
        }
    };

    // Every agent Dove starts gets the in-app browser as MCP tools.
    // Office skills for every harness (and Dove's own terminal sessions).
    dove_core::office_skills::set_enabled(engine.store.setting("agent_office_skills").as_deref() != Some("false"));
    std::thread::spawn(dove_core::office_skills::ensure_installed);
    if engine.store.setting("agent_browser").as_deref() != Some("false")
        && let (Some(endpoint), Ok(exe)) = (browser::control::start(&engine.runtime()), std::env::current_exe())
    {
        // For tests and external tooling: where to reach this instance.
        if let Some(path) = std::env::var_os("DOVE_BROWSER_ENDPOINT_FILE") {
            let _ = std::fs::write(path, format!("{} {}", endpoint.port, endpoint.token));
        }
        dove_core::set_tool_server(Some(dove_core::ToolServer {
            name: "dove".into(),
            command: exe.display().to_string(),
            args: vec!["--mcp-browser".into()],
            env: vec![
                ("DOVE_BROWSER_PORT".into(), endpoint.port.to_string()),
                ("DOVE_BROWSER_TOKEN".into(), endpoint.token),
                ("DOVE_DATA".into(), engine.store.root().display().to_string()),
            ],
        }));
    }

    // `dove <folder>` opens (adding if needed) that project.
    if let Some(arg) = std::env::args_os().nth(1) {
        let path = std::path::PathBuf::from(arg);
        if path.is_dir() {
            let root = dove_core::git::repo_root(&path).unwrap_or(path);
            match engine.store.add_project(&root) {
                Ok(p) => {
                    let _ = engine.store.set_setting("active_project", &p.id.to_string());
                }
                Err(e) => eprintln!("could not open {}: {e:#}", root.display()),
            }
        }
    }

    let show_intro = intro::should_show(
        engine.store.setting(intro::SEEN_KEY).as_deref(),
        intro::env_flag("DOVE_SHOW_INTRO"),
        intro::env_flag("DOVE_SKIP_INTRO"),
    );
    if show_intro {
        // Marked up front so quitting mid-intro does not replay it.
        let _ = engine.store.set_setting(intro::SEEN_KEY, "1");
    }

    gpui_platform::application().with_assets(assets::Assets).run(move |cx: &mut App| {
        gpui_base::init(cx);
        assets::load_fonts(cx);
        motion::init(&engine.store, cx);
        design::load_preferences(&engine.store);
        apply_theme(&engine, cx);
        keymap::apply(&engine.store, cx);
        browser::bind_keys(cx);
        browser::set_runtime(engine.runtime());
        voice::VoiceModel::init(engine.clone(), cx);
        remote::Remote::init(engine.clone(), cx);
        cx.on_action(|_: &actions::Quit, cx| cx.quit());

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(default_window_size(cx), cx)),
            window_min_size: Some(size(px(900.0), px(600.0))),
            titlebar: Some(TitlebarOptions {
                // `DOVE_WINDOW_TITLE` marks test instances so they are never
                // mistaken for the real app.
                title: cfg!(target_os = "windows")
                    .then(|| std::env::var("DOVE_WINDOW_TITLE").unwrap_or_else(|_| "Dove".into()).into()),
                appears_transparent: true,
                traffic_light_position: Some(gpui::point(px(14.0), px(14.0))),
            }),
            // B1: always come up shown and focused, even when launched from
            // a background process.
            focus: true,
            show: true,
            app_owns_titlebar_drag: true,
            window_decorations: cfg!(target_os = "linux").then_some(gpui::WindowDecorations::Client),
            window_background: Theme::of(cx).window_background(),
            app_id: Some("dev.dove.Dove".into()),
            ..Default::default()
        };
        let engine2 = engine.clone();
        let window = cx
            .open_window(options, |window, cx| {
                // The interface font size is the window's rem size.
                design::apply_to_window(window);
                // Follow the OS light/dark switch when the theme is "system".
                let engine3 = engine2.clone();
                window
                    .observe_window_appearance(move |_, cx| {
                        if engine3.store.setting("appearance").is_none_or(|a| a == "system") {
                            apply_theme(&engine3, cx);
                        }
                    })
                    .detach();
                cx.new(|cx| Root::new(engine2.clone(), show_intro, window, cx))
            })
            .expect("failed to open the Dove window");
        // `DOVE_NO_ACTIVATE`: benchmark and QA runs open without taking the
        // foreground from whoever is using the machine.
        let activate = !perf_trace::no_activate();
        let _ = window.update(cx, |_, window, cx| {
            if activate {
                restore_if_minimized(window);
                window.activate_window();
            }
            notify::install(engine.clone(), window, cx);
        });
        if activate {
            cx.activate(true);
        }
        // B1: launched from a background process (a hidden console, another
        // app), Windows applies the launcher's "minimized" show state after
        // the first activation. Activating again restores the window.
        let handle = window;
        cx.spawn(async move |cx| {
            if !activate {
                return;
            }
            for delay in [250u64, 900] {
                cx.background_executor().timer(std::time::Duration::from_millis(delay)).await;
                let _ = handle.update(cx, |_, window, _| {
                    restore_if_minimized(window);
                    window.activate_window();
                });
            }
        })
        .detach();
    });
}
