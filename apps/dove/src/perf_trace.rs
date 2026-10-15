//! Opt-in measurement hooks for `scripts/perf` (all off unless the env var
//! is set, and then they only write one line to stderr per event).
//!
//! - `DOVE_INPUT_TRACE=1`: for every key press, prints how long it took for
//!   the next frame to start. Run with `ZED_MEASUREMENTS=1` as well and GPUI
//!   prints that frame's own `frame duration`, which completes the picture:
//!   latency = key → frame start + frame duration (present included).
//! - `DOVE_STARTUP_TRACE=1`: prints when `main` began and when the first
//!   frame was built, both as Unix milliseconds, so a harness can compare
//!   them with the process creation time.
//! - `DOVE_NO_ACTIVATE=1`: the window opens without taking the foreground,
//!   so benchmark runs never steal focus from whoever uses the machine.

use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use gpui::Window;

fn flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty() && v != "0")
}

fn unix_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

pub fn input_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| flag("DOVE_INPUT_TRACE"))
}

pub fn no_activate() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| flag("DOVE_NO_ACTIVATE"))
}

fn startup_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| flag("DOVE_STARTUP_TRACE"))
}

static MAIN_STARTED: OnceLock<(Instant, f64)> = OnceLock::new();

/// Call first thing in `main`.
pub fn mark_main() {
    let _ = MAIN_STARTED.set((Instant::now(), unix_ms()));
}

/// Call from the root view's render; reports only the first time.
pub fn mark_first_render(window: &Window) {
    static DONE: OnceLock<()> = OnceLock::new();
    if !startup_enabled() || DONE.set(()).is_err() {
        return;
    }
    let Some((started, main_unix)) = MAIN_STARTED.get().copied() else { return };
    let render_ms = started.elapsed().as_secs_f64() * 1000.0;
    window.on_next_frame(move |_, _| {
        eprintln!(
            "dove-startup: main_unix_ms={main_unix:.0} first_render_ms={render_ms:.1} next_frame_start_ms={:.1}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    });
}

fn frame_stats_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("DOVE_FRAME_STATS").is_some())
}

/// `DOVE_FRAME_STATS=1`: count renders per view and log a one-line summary
/// each second there were any ("views: Shell=2 TaskView=1"), so a timer or
/// animation that keeps the window busy shows who it is.
pub fn count_render(view: &'static str) {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    if !frame_stats_enabled() {
        return;
    }
    thread_local! {
        static COUNTS: RefCell<(Option<Instant>, BTreeMap<&'static str, u32>)> = RefCell::new((None, BTreeMap::new()));
    }
    COUNTS.with(|c| {
        let mut c = c.borrow_mut();
        let now = Instant::now();
        let start = *c.0.get_or_insert(now);
        if now.duration_since(start).as_millis() >= 1000 && !c.1.is_empty() {
            let line = c.1.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" ");
            tracing::info!(target: "dove::frames", "views: {line}");
            c.1.clear();
            c.0 = Some(now);
        }
        *c.1.entry(view).or_default() += 1;
    });
}

/// A key went down: time until the next frame starts drawing.
pub fn key_down(keystroke: &str, window: &Window) {
    let at = Instant::now();
    let key = keystroke.to_string();
    window.on_next_frame(move |_, _| {
        eprintln!("dove-input: key={key} to_frame_start_ms={:.3}", at.elapsed().as_secs_f64() * 1000.0);
    });
}
