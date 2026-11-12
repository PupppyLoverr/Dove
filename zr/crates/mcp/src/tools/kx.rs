//! Keplorayx's own tools: the widgets on your desktop and Keplorayx's
//! calendar, as files in its data folder (`widgets.json`, `calendar.json`),
//! which the app watches: a widget set here shows on the desktop within a
//! couple of seconds, placed there if it has no place, drawn as glass over
//! the wallpaper. Nothing here asks the person: these are theirs to undo.

use serde_json::{Value, json};

use super::ToolDef;

pub(super) const LIST_WIDGETS: &str = "list_widgets";
pub(super) const SET_WIDGET: &str = "set_widget";
pub(super) const REMOVE_WIDGET: &str = "remove_widget";
pub(super) const LIST_CALENDAR: &str = "list_calendar";
pub(super) const CALENDAR_ADD: &str = "calendar_add";
pub(super) const CALENDAR_REMOVE: &str = "calendar_remove";
pub(super) const NAMES: [&str; 6] = [LIST_WIDGETS, SET_WIDGET, REMOVE_WIDGET, LIST_CALENDAR, CALENDAR_ADD, CALENDAR_REMOVE];

const KINDS: &[&str] = &["clock", "calendar", "agents", "usage", "ask", "threads", "system", "music", "note", "custom"];

fn variants(kind: &str) -> &'static [&'static str] {
    match kind {
        "clock" => &["digital", "analog", "world", "small"],
        "calendar" => &["agenda", "month", "small"],
        "system" => &["bars", "small", "graph"],
        "music" => &["medium", "small"],
        _ => &["medium", "small", "large"],
    }
}

pub(super) fn catalog() -> Vec<ToolDef> {
    let s = |what: &str| json!({ "type": "string", "description": what });
    vec![
        ToolDef {
            name: LIST_WIDGETS,
            description: "The widgets on the person's desktop (Keplorayx's): each one's id, kind, variant, title, settings, place (x, y in points from the desktop's top left) and, for a custom one, its script.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDef {
            name: SET_WIDGET,
            description: "Add a widget to the person's desktop, or change one (by its id). Widgets are glass cards over the wallpaper, under every window. kind: clock, calendar, agents, usage, ask, threads, system, music, note, or custom (with `script`: a Luau view, as show_visual's). variant: small, medium or large, or a kind's own (clock: digital, analog, world, small; calendar: agenda, month, small; system: bars, small, graph; music: medium, small). settings: the kind's own (clock: { zones = [\"Europe/London\"] }; note: { text }). Leave x and y out and it's placed for you. Returns its id.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": s("Omit to add one; give one to change it."),
                    "kind": s("clock, calendar, agents, usage, ask, threads, system, music, note or custom."),
                    "variant": s("Its look; see the description."),
                    "title": s("Its title, if it shows one."),
                    "settings": { "type": "object", "description": "The kind's own settings." },
                    "script": s("A custom widget's Luau view."),
                    "x": { "type": "number" }, "y": { "type": "number" }
                },
                "required": ["kind"]
            }),
        },
        ToolDef {
            name: REMOVE_WIDGET,
            description: "Take a widget off the person's desktop, by its id (list_widgets).",
            input_schema: json!({ "type": "object", "properties": { "id": s("The widget's id.") }, "required": ["id"] }),
        },
        ToolDef {
            name: LIST_CALENDAR,
            description: "The events in Keplorayx's calendar (the calendar widget's): each one's id, title, date (YYYY-MM-DD), time (HH:MM, 24-hour) if it has one, minutes and notes.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDef {
            name: CALENDAR_ADD,
            description: "Add an event to Keplorayx's calendar, or change one (by its id). date as YYYY-MM-DD; time as HH:MM, 24-hour, if it has one. Returns its id.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": s("Omit to add one."), "title": s("What it is."), "date": s("YYYY-MM-DD."),
                    "time": s("HH:MM, 24-hour."), "minutes": { "type": "number" }, "notes": s("Anything else.")
                },
                "required": ["title", "date"]
            }),
        },
        ToolDef {
            name: CALENDAR_REMOVE,
            description: "Remove an event from Keplorayx's calendar, by its id.",
            input_schema: json!({ "type": "object", "properties": { "id": s("The event's id.") }, "required": ["id"] }),
        },
    ]
}

/// Keplorayx's data folder: the engine's (`ZERON_DATA_DIR`, which the app
/// gives the engine it starts), else ~/.keplorayx.
pub(super) fn data_dir() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("ZERON_DATA_DIR").filter(|d| !d.is_empty()) {
        return dir.into();
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).unwrap_or_default();
    std::path::PathBuf::from(home).join(".keplorayx")
}

fn read(name: &str) -> Vec<Value> {
    std::fs::read_to_string(data_dir().join(name))
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
        .unwrap_or_default()
}

fn write(name: &str, list: &[Value]) -> Result<(), String> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(name), serde_json::to_string_pretty(list).unwrap_or_default()).map_err(|e| e.to_string())
}

fn new_id(prefix: &str) -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("{prefix}{:x}", n)
}

fn valid_date(d: &str) -> bool {
    let b = d.as_bytes();
    d.len() == 10 && b[4] == b'-' && b[7] == b'-' && d.chars().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

fn valid_time(t: &str) -> bool {
    let b = t.as_bytes();
    t.len() == 5 && b[2] == b':' && t.chars().enumerate().all(|(i, c)| i == 2 || c.is_ascii_digit())
}

pub(super) fn call(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        LIST_WIDGETS => Ok(json!({ "widgets": read("widgets.json") })),
        SET_WIDGET => {
            let kind = args["kind"].as_str().unwrap_or_default();
            if !KINDS.contains(&kind) {
                return Err(format!("kind: one of {}", KINDS.join(", ")));
            }
            if kind == "custom" && args["script"].as_str().unwrap_or_default().trim().is_empty() {
                return Err("a custom widget needs `script`".into());
            }
            let variant = args["variant"].as_str().unwrap_or(variants(kind)[0]);
            if !variants(kind).contains(&variant) {
                return Err(format!("variant of a {kind}: one of {}", variants(kind).join(", ")));
            }
            let mut list = read("widgets.json");
            let id = args["id"].as_str().map(str::to_string).unwrap_or_else(|| new_id("w"));
            let old = list.iter().find(|w| w["id"].as_str() == Some(&id)).cloned().unwrap_or(json!({}));
            let pick = |k: &str| if args[k].is_null() { old[k].clone() } else { args[k].clone() };
            let widget = json!({
                "id": id, "kind": kind, "variant": variant,
                "title": pick("title"),
                "settings": if args["settings"].is_object() { args["settings"].clone() } else if old["settings"].is_object() { old["settings"].clone() } else { json!({}) },
                "script": pick("script"),
                // (No place: the app finds it one.)
                "x": pick("x"), "y": pick("y"),
            });
            match list.iter().position(|w| w["id"].as_str() == Some(&id)) {
                Some(i) => list[i] = widget,
                None => list.push(widget),
            }
            write("widgets.json", &list)?;
            Ok(json!({ "id": id, "said": "It's on the desktop within a couple of seconds." }))
        }
        REMOVE_WIDGET => {
            let id = args["id"].as_str().unwrap_or_default();
            let mut list = read("widgets.json");
            let before = list.len();
            list.retain(|w| w["id"].as_str() != Some(id));
            if list.len() == before {
                return Err(format!("no widget `{id}`"));
            }
            write("widgets.json", &list)?;
            Ok(json!({ "removed": id }))
        }
        LIST_CALENDAR => Ok(json!({ "events": read("calendar.json") })),
        CALENDAR_ADD => {
            let title = args["title"].as_str().unwrap_or_default().trim().to_string();
            let date = args["date"].as_str().unwrap_or_default().to_string();
            if title.is_empty() || !valid_date(&date) {
                return Err("title, and date as YYYY-MM-DD".into());
            }
            let time = args["time"].as_str().filter(|t| !t.is_empty());
            if time.is_some_and(|t| !valid_time(t)) {
                return Err("time as HH:MM, 24-hour".into());
            }
            let mut list = read("calendar.json");
            let id = args["id"].as_str().map(str::to_string).unwrap_or_else(|| new_id("e"));
            let event = json!({
                "id": id, "title": title, "date": date, "time": time,
                "minutes": args["minutes"].as_u64(), "notes": args["notes"].as_str(),
            });
            match list.iter().position(|e| e["id"].as_str() == Some(&id)) {
                Some(i) => list[i] = event,
                None => list.push(event),
            }
            write("calendar.json", &list)?;
            Ok(json!({ "id": id }))
        }
        CALENDAR_REMOVE => {
            let id = args["id"].as_str().unwrap_or_default();
            let mut list = read("calendar.json");
            let before = list.len();
            list.retain(|e| e["id"].as_str() != Some(id));
            if list.len() == before {
                return Err(format!("no event `{id}`"));
            }
            write("calendar.json", &list)?;
            Ok(json!({ "removed": id }))
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_times() {
        assert!(valid_date("2026-11-06"));
        assert!(!valid_date("2026-1-6"));
        assert!(valid_time("09:30"));
        assert!(!valid_time("9:30"));
    }

    #[test]
    fn kinds_and_variants() {
        assert!(call(SET_WIDGET, &json!({ "kind": "toaster" })).is_err());
        assert!(call(SET_WIDGET, &json!({ "kind": "clock", "variant": "huge" })).is_err());
    }
}
