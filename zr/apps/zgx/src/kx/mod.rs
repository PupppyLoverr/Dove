//! Keplorayx's own, beside Zeron's interface: the island at the top of the
//! screen (notch), the launcher (Alt+Space), the widgets on the desktop,
//! what's playing (media), the tray and the keys from any app (platform),
//! and the clock the scripts read. Each was Keplorayx's before it moved onto
//! Zeron's app (apps/keplorayx/src); here they read Zeron's engine through
//! the same calls Zeron's own windows make (sessions, chats, agents).

pub mod artifacts;
pub mod browser;
pub mod buddies;
pub mod clock;
pub mod frost;
pub mod import;
pub mod launcher;
pub mod media;
pub mod notch;
pub mod platform;
pub mod start;
pub mod widgets;

use serde_json::Value as Json;
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Value};

/// What one of Keplorayx's windows reads of the engine, as the main
/// window does: the sessions (`sessions`), the chats (`chat.*`: one opened
/// in that window, its messages, what agents ask) and the agents.
pub fn engine_for(cx: &mut WindowContext, host: &ScriptHost) {
    crate::engine::install(cx);
    crate::auth::install(cx, host);
    crate::sessions::install(cx, host);
    crate::chat::install(cx, host);
    crate::agents::install(host);
    crate::projects::install(host);
}

/// JSON, as the scripts' values.
pub fn to_value(j: &Json) -> Value {
    match j {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::from(*b),
        Json::Number(n) => Value::from(n.as_f64().unwrap_or(0.)),
        Json::String(s) => Value::from(s.as_str()),
        Json::Array(a) => Value::List(a.iter().map(to_value).collect()),
        Json::Object(o) => Value::record(o.iter().map(|(k, v)| (k.as_str(), to_value(v)))),
    }
}

/// `work` on a thread of its own, its answer awaited here (None if it
/// panicked).
pub async fn off_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tell, mut heard) = zgx::task::latest::<T>();
    std::thread::spawn(move || {
        tell.send(work());
    });
    heard.recv().await
}

/// A link or file, in what the system opens it with.
pub fn open_external(target: &str) {
    let _ = open::that_detached(target);
}
