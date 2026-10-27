//! A scripted cua-driver for tests: strict schemas shaped like stock
//! v0.38's, two fixture windows, and a record of every call.

use base64::Engine as _;
use serde_json::{Value, json};

use crate::driver::Driver;

pub struct FakeDriver {
    pub calls: Vec<(String, Value)>,
    windows: Value,
    rows: Value,
    image: bool,
    /// The next `since` read answers with this diff and snapshot id.
    pub next_diff: Option<(String, String)>,
    /// What actions report as their effect.
    pub effect: &'static str,
}

fn props(names: &[&str]) -> Value {
    let mut p = serde_json::Map::new();
    for n in names {
        p.insert((*n).into(), json!({}));
    }
    json!({"type": "object", "properties": p, "additionalProperties": false})
}

pub fn png(w: u32, h: u32) -> String {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 200, 200]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img).write_to(&mut out, image::ImageFormat::Png).expect("png");
    base64::engine::general_purpose::STANDARD.encode(out.into_inner())
}

fn row(i: u32, role: &str, label: &str, frame: (i32, i32, i32, i32)) -> Value {
    json!({"element_index": i, "role": role, "label": label, "element_token": format!("s0000000a:{i}"), "depth": 1,
        "enabled": true, "frame": {"x": frame.0, "y": frame.1, "w": frame.2, "h": frame.3}})
}

impl FakeDriver {
    fn new(app: &str, title: &str, rows: Value, image: bool) -> Self {
        Self {
            calls: Vec::new(),
            windows: json!([{"app_name": app, "pid": 100, "window_id": 200, "title": title, "is_on_screen": true, "z_index": 5},
                {"app_name": "Explorer", "pid": 7, "window_id": 8, "title": "Files", "is_on_screen": true, "z_index": 1}]),
            rows,
            image,
            next_diff: None,
            effect: "confirmed",
        }
    }

    /// A text editor with a rich tree.
    pub fn notepad() -> Self {
        Self::new(
            "Notepad",
            "Untitled - Notepad",
            json!([row(0, "MenuItem", "File", (0, 0, 40, 20)), row(1, "Edit", "Text editor", (0, 40, 600, 400)),
                row(2, "Button", "Save", (10, 5, 60, 24)), row(3, "Edit", "Search", (80, 5, 200, 24))]),
            false,
        )
    }

    /// A canvas app whose tree is nearly empty.
    pub fn canvas() -> Self {
        Self::new("Paint", "Untitled - Paint", json!([row(1, "Pane", "", (0, 0, 0, 0))]), true)
    }

    /// A login form with a password field and a Sign in button.
    pub fn login() -> Self {
        Self::new(
            "Bank",
            "Sign in",
            json!([row(1, "Edit", "User name", (10, 10, 200, 24)), row(2, "Edit", "Password", (10, 40, 200, 24)),
                row(3, "Button", "Sign in", (10, 70, 80, 24)), row(4, "Button", "Delete account", (100, 70, 120, 24))]),
            false,
        )
    }

    pub fn calls_to(&self, tool: &str) -> Vec<Value> {
        self.calls.iter().filter(|(t, _)| t == tool).map(|(_, a)| a.clone()).collect()
    }

    fn ok(structured: Value, text: &str) -> Value {
        json!({"content": [{"type": "text", "text": text}], "structuredContent": structured, "isError": false})
    }
}

impl Driver for FakeDriver {
    fn list_tools(&mut self) -> Result<Vec<Value>, String> {
        let pointer = ["session", "pid", "window_id", "element_token", "x", "y", "button", "count", "modifier", "delivery_mode", "scope"];
        Ok(vec![
            json!({"name": "start_session", "inputSchema": props(&["session"])}),
            json!({"name": "end_session", "inputSchema": props(&["session"])}),
            json!({"name": "list_windows", "inputSchema": props(&["pid", "on_screen_only"])}),
            json!({"name": "list_apps", "inputSchema": props(&["session"])}),
            json!({"name": "get_window_state", "inputSchema": props(&["session", "pid", "window_id", "query", "include_screenshot",
                "include_accessibility_tree", "max_elements", "max_image_dimension", "tree_format", "since"])}),
            json!({"name": "get_desktop_state", "inputSchema": props(&["session", "max_image_dimension"])}),
            json!({"name": "click", "inputSchema": props(&pointer)}),
            json!({"name": "drag", "inputSchema": props(&["session", "pid", "window_id", "from_x", "from_y", "to_x", "to_y", "button", "modifier", "delivery_mode"])}),
            json!({"name": "scroll", "inputSchema": props(&["session", "pid", "window_id", "element_token", "x", "y", "direction", "amount", "delivery_mode"])}),
            json!({"name": "type_text", "inputSchema": props(&["session", "pid", "window_id", "element_token", "text", "delivery_mode"])}),
            json!({"name": "press_key", "inputSchema": props(&["session", "pid", "window_id", "key", "modifiers", "delivery_mode"])}),
            json!({"name": "hotkey", "inputSchema": props(&["session", "pid", "window_id", "keys", "delivery_mode"])}),
            json!({"name": "set_value", "inputSchema": props(&["session", "pid", "window_id", "element_token", "value"])}),
            json!({"name": "bring_to_front", "inputSchema": props(&["pid", "window_id"])}),
            json!({"name": "run_steps", "inputSchema": props(&["steps", "pid", "window_id", "app", "delivery_mode", "observe", "delay_ms"])}),
            json!({"name": "verify_state", "inputSchema": props(&["session", "pid", "window_id", "expect"])}),
        ])
    }

    fn call_tool(&mut self, name: &str, args: Value) -> Result<Value, String> {
        self.calls.push((name.to_string(), args.clone()));
        Ok(match name {
            "list_windows" => Self::ok(json!({"windows": self.windows}), "2 windows"),
            "list_apps" => Self::ok(json!({"apps": [{"name": "Notepad", "pid": 100, "running": true}]}), ""),
            "get_window_state" => {
                let shot = args["include_screenshot"] == json!(true);
                let tree = args["include_accessibility_tree"] != json!(false);
                if tree && args.get("since").is_some() {
                    // As stock 0.38 answers a markdown `since` read: the diff
                    // text, no elements. A change also changes the rows.
                    let (diff, sid) = self.next_diff.take().unwrap_or_else(|| ("no change since s0000000a".into(), "s0000000b".into()));
                    let status = if diff.starts_with("no change") { "no_change" } else { "diff" };
                    if status == "diff" {
                        let rows = self.rows.as_array_mut().expect("rows");
                        rows.retain(|r| r["element_index"] != 3);
                        for r in rows.iter_mut().filter(|r| r["element_index"] == 2) {
                            r["label"] = json!("Saved");
                        }
                    }
                    return Ok(Self::ok(
                        json!({"snapshot_id": sid, "since_status": status, "tree_diff": diff,
                            "app_name": "Notepad", "window_title": "Untitled - Notepad"}),
                        "",
                    ));
                }
                let mut s = json!({"snapshot_id": "s0000000a", "app_name": self.windows[0]["app_name"], "window_title": self.windows[0]["title"]});
                if tree {
                    s["elements"] = self.rows.clone();
                }
                let mut r = Self::ok(s, "window state");
                if shot && self.image {
                    r["content"].as_array_mut().expect("content").push(json!({"type": "image", "data": png(64, 48), "mimeType": "image/png"}));
                }
                r
            }
            "get_desktop_state" => json!({"content": [{"type": "image", "data": png(64, 48), "mimeType": "image/png"}], "isError": false}),
            "run_steps" => {
                let mut s = json!({"ok": true, "total": args["steps"].as_array().map_or(0, Vec::len), "executed": 1, "failed_step": null});
                if args.get("observe").is_some() {
                    // As stock run_steps reports its end read: observation.state.
                    let mut rows = self.rows.clone();
                    for r in rows.as_array_mut().into_iter().flatten() {
                        let i = r["element_index"].as_u64().unwrap_or(0);
                        r["element_token"] = json!(format!("s0000000c:{i}"));
                    }
                    s["observation"] = json!({"ok": true, "tool": "get_window_state", "state": {"snapshot_id": "s0000000c", "elements": rows}});
                }
                Self::ok(s, "run_steps: ok")
            }
            "verify_state" => Self::ok(json!({"status": "satisfied"}), "satisfied"),
            "start_session" | "end_session" => Self::ok(json!({}), "ok"),
            _ => Self::ok(json!({"effect": self.effect, "path": "uia_invoke"}), &format!("{name} done")),
        })
    }
}
