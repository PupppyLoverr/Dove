//! The time, as the scripts see it: `kx.now`, the local date and time to
//! the minute, published at each minute's turn (a window that shows a clock
//! wakes once a minute, no more). For the notch at rest and the widgets.
//!
//!   kx.now = { hour, minute, weekday ("Thursday"), day, month ("November"),
//!              month_n, year, date ("2026-11-05"), tomorrow, unix }

use chrono::{Datelike as _, Local, Timelike as _};
use serde_json::json;
use zgx_desktop::WindowContext;
use zgx_script::ScriptHost;

fn now() -> serde_json::Value {
    let t = Local::now();
    json!({
        "hour": t.hour(),
        "minute": t.minute(),
        "weekday": t.format("%A").to_string(),
        "day": t.day(),
        "month": t.format("%B").to_string(),
        "month_n": t.month(),
        "year": t.year(),
        "date": t.format("%Y-%m-%d").to_string(),
        "tomorrow": (t.date_naive() + chrono::Days::new(1)).format("%Y-%m-%d").to_string(),
        "unix": t.timestamp(),
    })
}

pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    let shared = host.shared("kx.now", crate::kx::to_value(&now()));
    cx.tasks.spawn(async move {
        loop {
            let past = Local::now().second() as u64;
            zgx::timer::sleep(std::time::Duration::from_secs(60 - past.min(59))).await;
            shared.set(crate::kx::to_value(&now()));
        }
    });
}
