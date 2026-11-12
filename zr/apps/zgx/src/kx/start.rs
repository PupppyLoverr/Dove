//! What the main window starts beside itself, once a process: the island
//! over every window, the launcher (Alt+Space), the widgets on the desktop,
//! the tray and the keys from any app, and Start with Windows. Closing the
//! main window hides it to the tray (your agents carry on, the island
//! stays); the tray or a key brings it back.
//!
//! Keplorayx's switches for these (the island shown, the keys, staying in
//! the tray) are kept in `<data>/kx-settings.json`.

use std::{cell::Cell, rc::Rc};

use serde_json::json;
use zgx_desktop::{WindowContext, WindowOptions};
use zgx_script::{ScriptHost, Value};

use super::{launcher, notch, platform, widgets};

fn settings_path() -> std::path::PathBuf {
    crate::kx_account::data_dir().join("kx-settings.json")
}

/// A switch of Keplorayx's, or `default` while you never set it.
pub fn flag(key: &str, default: bool) -> bool {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v[key].as_bool())
        .unwrap_or(default)
}

pub fn set_flag(key: &str, on: bool) {
    let path = settings_path();
    let mut v = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or_else(|| json!({}));
    v[key] = json!(on);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, serde_json::to_string_pretty(&v).unwrap_or_default());
}

thread_local! {
    static STARTED: Cell<bool> = const { Cell::new(false) };
}

/// Starts them beside the main window in `cx` (its scripts' `host`).
pub fn start(cx: &mut WindowContext, host: &ScriptHost) {
    // What they open in Keplorayx brings the main window forward, on that
    // thread (Zeron's links: the shell's route follows `app.link`).
    let main = cx.window.clone();
    let to_main = move |thread: Option<String>| {
        main.set_visible(true);
        main.request_focus();
        if let Some(id) = thread {
            crate::links::open(&format!("zeron://notification/{id}"));
        }
    };
    host.action("launcher.summon", |_| launcher::summon());
    // Agents in the in-app browser (the engine's `browser_*` tools).
    crate::kx::browser::install(cx, host);
    // Buddies and their automations (the Buddies space).
    crate::kx::buddies::install(cx, host);
    // The main window frosted whichever window has the keyboard (Windows).
    crate::kx::frost::keep(cx);
    // What agents made in chats with no project (the Artifacts page).
    crate::kx::artifacts::install(host);
    // Your projects from before Keplorayx moved onto Zeron's engine.
    crate::kx::import::install(host);
    host.shared("kx.autostart", platform::autostart::on());
    host.action("kx.autostart", |on| platform::autostart::set(matches!(on, Value::Bool(true))));
    // Settings › General's Keplorayx switches: as they stand, and each set.
    let flags = host.shared(
        "kx.flags",
        Value::record([
            ("notch", Value::from(flag("notch", true))),
            ("keep_in_tray", Value::from(flag("keep_in_tray", true))),
            ("global_keys", Value::from(flag("global_keys", true))),
            ("notch_pin", Value::from(flag("notch_pin", true))),
        ]),
    );
    for key in ["keep_in_tray", "global_keys"] {
        let flags = flags.clone();
        host.action(&format!("kx.{key}"), move |on| {
            set_flag(key, matches!(on, Value::Bool(true)));
            flags.set(Value::record([
                ("notch", Value::from(flag("notch", true))),
                ("keep_in_tray", Value::from(flag("keep_in_tray", true))),
                ("global_keys", Value::from(flag("global_keys", true))),
                ("notch_pin", Value::from(flag("notch_pin", true))),
            ]));
        });
    }
    {
        let flags = flags.clone();
        host.action("kx.notch_pin", move |on| {
            let on = matches!(on, Value::Bool(true));
            set_flag("notch_pin", on);
            notch::set_pin(on);
            flags.set(Value::record([
                ("notch", Value::from(flag("notch", true))),
                ("keep_in_tray", Value::from(flag("keep_in_tray", true))),
                ("global_keys", Value::from(flag("global_keys", true))),
                ("notch_pin", Value::from(flag("notch_pin", true))),
            ]));
        });
    }

    if STARTED.with(|s| s.replace(true)) {
        return;
    }
    launcher::open(cx, to_main.clone());
    widgets::start(cx, to_main.clone());
    let notch_window = notch::enabled().then(|| {
        let to_main = to_main.clone();
        cx.windows.open(WindowOptions { visible: flag("notch", true), ..notch::window() }, move |cx| {
            notch::open(cx, to_main.clone(), false);
        })
    });
    {
        let notch_window = notch_window.clone();
        host.action("kx.notch_visible", move |on| {
            let on = matches!(on, Value::Bool(true));
            set_flag("notch", on);
            if let Some(w) = &notch_window {
                w.set_visible(on);
            }
        });
    }

    let mut tray_live = false;
    if std::env::var_os("KX_TRAY").is_none_or(|v| v != "0") {
        let (heard, tray) = platform::start(flag("global_keys", true), flag("notch", true));
        tray_live = tray.live();
        // (It lives as long as the process.)
        std::mem::forget(tray);
        let window = cx.window.clone();
        cx.tasks.spawn(async move {
            while let Ok(what) = heard.recv_async().await {
                match what {
                    platform::Shell::Quit => std::process::exit(0),
                    platform::Shell::Notch => {
                        let on = !flag("notch", true);
                        set_flag("notch", on);
                        if let Some(w) = &notch_window {
                            w.set_visible(on);
                        }
                    }
                    platform::Shell::Login => platform::autostart::set(!platform::autostart::on()),
                    platform::Shell::Launcher => launcher::summon(),
                    // (Answered from the keys: the window stays where it is.)
                    platform::Shell::Allow | platform::Shell::Deny => {}
                    _ => {
                        window.set_visible(true);
                        window.request_focus();
                    }
                }
            }
        });
    }
    let live = Rc::new(Cell::new(tray_live));
    let window = cx.window.clone();
    cx.on_close_requested(move || {
        if live.get() && flag("keep_in_tray", true) {
            window.set_visible(false);
            return false;
        }
        true
    });
}
