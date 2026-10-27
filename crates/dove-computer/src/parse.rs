//! Pure parsing of cua-driver payloads (Hermes `cua_backend_parse.py`):
//! windows, elements, action verdicts, image headers, key combos.

use serde_json::{Map, Value};

use crate::driver::Envelope;
use crate::types::{ActionResult, UiElement};

/// A `list_windows` row Dove can target.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub app: String,
    pub pid: u32,
    pub window_id: u64,
    pub title: String,
    pub off_screen: bool,
    pub z: f64,
}

fn positive(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Number(n) => n.as_u64().filter(|n| *n > 0),
        Value::String(s) => s.trim().parse::<u64>().ok().filter(|n| *n > 0),
        _ => None,
    }
}

/// Usable windows, frontmost first. Rows without a pid or window id (X11
/// root and panel windows report `pid: null`) are skipped; Wayland's
/// unknown `z_index` sorts lowest.
pub fn windows(env: &Envelope) -> Vec<Window> {
    let raw = env
        .structured
        .get("windows")
        .and_then(Value::as_array)
        .or_else(|| env.data.get("windows").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<Window> = raw
        .iter()
        .filter_map(|w| {
            let pid = u32::try_from(positive(w.get("pid"))?).ok()?;
            let window_id = positive(w.get("window_id"))?;
            Some(Window {
                app: w.get("app_name").and_then(Value::as_str).unwrap_or("").to_string(),
                pid,
                window_id,
                title: w.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
                // Only an explicit false is off-screen; null is unknown.
                off_screen: w.get("is_on_screen") == Some(&Value::Bool(false)),
                z: w.get("z_index").and_then(Value::as_f64).unwrap_or(0.0),
            })
        })
        .collect();
    // Cursor overlays are the driver's own drawing surfaces, never a target.
    out.retain(|w| !w.title.starts_with("Cua.AgentCursorOverlay"));
    out.sort_by(|a, b| b.z.partial_cmp(&a.z).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Running apps from `list_apps` (structured first, then the text payload).
pub fn apps(env: &Envelope) -> Vec<Value> {
    for c in [env.structured.get("apps"), env.data.get("apps"), Some(&env.data)] {
        if let Some(list) = c.and_then(Value::as_array).filter(|l| !l.is_empty()) {
            return list.clone();
        }
    }
    Vec::new()
}

/// GNOME helpers and the desktop backdrop: targetable, but empty captures.
pub fn is_real_app_window(w: &Window) -> bool {
    let t = w.title.to_lowercase();
    !["@!", "desktop", "gnome-shell", "gnome shell"].iter().any(|p| t.starts_with(p))
}

/// The driver's own windows: its authorization daemon (which it refuses
/// to drive) and its agent-cursor overlays, from any cua-driver build.
pub fn is_driver_window(w: &Window) -> bool {
    let n: String = w.app.to_lowercase().chars().filter(|c| !matches!(c, ' ' | '_' | '-')).collect();
    n == "cuadriver" || w.title.starts_with("Cua.AgentCursorOverlay")
}

/// `structuredContent.elements` rows, renumbered 1.. in reading order of
/// the driver. Returns the elements and the raw rows (for the grounder).
pub fn elements(env: &Envelope) -> Vec<UiElement> {
    let Some(rows) = env.structured.get("elements").and_then(Value::as_array) else { return Vec::new() };
    rows.iter()
        .filter_map(|r| {
            let index = u32::try_from(r.get("element_index")?.as_u64()?).ok()?;
            let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let frame = r.get("frame");
            let n = |k: &str| frame.and_then(|f| f.get(k)).and_then(Value::as_f64).unwrap_or(0.0).round() as i64;
            Some(UiElement {
                index,
                role: s("role"),
                label: s("label"),
                value: s("value"),
                bounds: (n("x"), n("y"), n("w"), n("h")),
                enabled: r.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                token: r.get("element_token").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string),
                source: "tree",
            })
        })
        .collect()
}

/// The verdict fields of an action result (structured wins over text).
pub fn action_result(name: &str, env: &Envelope, requested_delivery: Option<&str>) -> ActionResult {
    let mut meta: Map<String, Value> = env.data.as_object().cloned().unwrap_or_default();
    for (k, v) in &env.structured {
        meta.insert(k.clone(), v.clone());
    }
    let raw = |k: &str| env.structured.get(k).or_else(|| meta.get(k)).cloned();
    let s = |k: &str| raw(k).and_then(|v| v.as_str().map(str::to_string));
    ActionResult {
        ok: !env.is_error,
        action: name.into(),
        message: env.message(),
        verified: raw("verified").and_then(|v| v.as_bool()),
        effect: s("effect"),
        escalation: raw("escalation").and_then(|v| v.as_object().cloned()),
        path: s("path"),
        degraded: raw("degraded").and_then(|v| v.as_bool()),
        delivery_mode: requested_delivery.map(str::to_string),
        code: s("code").or_else(|| s("reason_code")),
        meta,
    }
}

/// (width, height) from a PNG or JPEG header; None when unreadable.
pub fn image_dimensions(raw: &[u8]) -> Option<(u32, u32)> {
    if raw.len() >= 24 && raw.starts_with(b"\x89PNG\r\n\x1a\n") {
        let w = u32::from_be_bytes(raw[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(raw[20..24].try_into().ok()?);
        return (w > 0 && h > 0).then_some((w, h));
    }
    if raw.len() < 4 || raw[0] != 0xFF || raw[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 9 < raw.len() {
        if raw[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = raw[i + 1];
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([raw[i + 2], raw[i + 3]]) as usize;
        let sof = matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF);
        if sof {
            let h = u16::from_be_bytes([raw[i + 5], raw[i + 6]]) as u32;
            let w = u16::from_be_bytes([raw[i + 7], raw[i + 8]]) as u32;
            return (w > 0 && h > 0).then_some((w, h));
        }
        i += 2 + len;
    }
    None
}

const MODIFIERS: [&str; 10] = ["cmd", "command", "shift", "option", "alt", "ctrl", "control", "fn", "win", "super"];

/// `cmd+s` / `ctrl-alt-t` → (key, modifiers); the last non-modifier wins.
pub fn key_combo(keys: &str) -> (Option<String>, Vec<String>) {
    let mut key = None;
    let mut mods = Vec::new();
    // A lone "-" or "+" is the key itself.
    if matches!(keys.trim(), "-" | "+") {
        return (Some(keys.trim().into()), mods);
    }
    for part in keys.split(['+', '-']).map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()) {
        let norm = match part.as_str() {
            "command" | "⌘" => "cmd".to_string(),
            "alt" | "⌥" => "option".to_string(),
            "control" => "ctrl".to_string(),
            "windows" | "super" | "meta" => "win".to_string(),
            p => p.to_string(),
        };
        if MODIFIERS.contains(&norm.as_str()) || norm == "win" {
            mods.push(norm);
        } else {
            key = Some(part);
        }
    }
    (key, mods)
}

/// A zero or negative id is a schema filler (some models zero-fill every
/// optional integer), not a target.
pub fn placeholder_id(v: Option<&Value>) -> bool {
    v.and_then(Value::as_i64).is_some_and(|n| n <= 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn env(structured: Value) -> Envelope {
        Envelope::from_result(&json!({"content": [], "structuredContent": structured}))
    }

    #[test]
    fn windows_are_frontmost_first_and_skip_unusable_rows() {
        let e = env(json!({"windows": [
            {"app_name": "Back", "pid": 10, "window_id": 1, "title": "b", "is_on_screen": true, "z_index": 1},
            {"app_name": "Panel", "pid": null, "window_id": 2, "title": "p"},
            {"app_name": "Front", "pid": 11, "window_id": 3, "title": "f", "is_on_screen": null, "z_index": 9},
            {"app_name": "dove-driver-windows-x86_64.exe", "pid": 12, "window_id": 4, "title": "Cua.AgentCursorOverlay.default", "z_index": 99},
        ]}));
        let w = windows(&e);
        assert_eq!(w.iter().map(|w| w.app.as_str()).collect::<Vec<_>>(), ["Front", "Back"]);
        assert!(!w[0].off_screen, "null is unknown, not off-screen");
    }

    #[test]
    fn elements_keep_frames_and_tokens() {
        let e = env(json!({"elements": [
            {"element_index": 3, "role": "Button", "label": "Save", "element_token": "s0000002a:3",
             "frame": {"x": 10.4, "y": 20.0, "w": 80.0, "h": 24.0}, "enabled": false},
            {"element_index": "bad"},
        ]}));
        let el = elements(&e);
        assert_eq!(el.len(), 1);
        assert_eq!(el[0].bounds, (10, 20, 80, 24));
        assert_eq!(el[0].token.as_deref(), Some("s0000002a:3"));
        assert!(!el[0].enabled);
        assert_eq!(el[0].center(), Some((50, 32)));
    }

    #[test]
    fn action_verdict_prefers_structured() {
        let r = json!({"content": [{"type": "text", "text": "{\"effect\":\"unverifiable\"}"}],
            "structuredContent": {"effect": "confirmed", "escalation": {"recommended": "px"}, "reason_code": "stale"},
            "isError": true});
        let a = action_result("click", &Envelope::from_result(&r), Some("background"));
        assert!(!a.ok);
        assert_eq!(a.effect.as_deref(), Some("confirmed"));
        assert_eq!(a.code.as_deref(), Some("stale"));
        assert_eq!(a.escalation.unwrap()["recommended"], "px");
    }

    #[test]
    fn image_headers() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(800u32.to_be_bytes());
        png.extend(600u32.to_be_bytes());
        assert_eq!(image_dimensions(&png), Some((800, 600)));
        let jpeg = [0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8, 0x02, 0x58, 0x03, 0x20, 3, 0, 0, 0];
        assert_eq!(image_dimensions(&jpeg), Some((800, 600)));
        assert_eq!(image_dimensions(b"nope"), None);
    }

    #[test]
    fn key_combos() {
        assert_eq!(key_combo("cmd+s"), (Some("s".into()), vec!["cmd".into()]));
        assert_eq!(key_combo("Ctrl-Alt-T"), (Some("t".into()), vec!["ctrl".into(), "option".into()]));
        assert_eq!(key_combo("return"), (Some("return".into()), vec![]));
        assert_eq!(key_combo("-"), (Some("-".into()), vec![]));
        assert_eq!(key_combo("shift"), (None, vec!["shift".into()]));
    }
}
