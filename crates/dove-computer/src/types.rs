//! The shapes every backend returns (Hermes `tools/computer_use/backend.py`).

use serde_json::{Map, Value};

/// One interactable element of a capture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiElement {
    /// The row index the model clicks by (`#N`; cua-driver counts from 0).
    pub index: u32,
    /// The platform's role spelling (`AXButton`, `Button`, `push button`).
    pub role: String,
    pub label: String,
    pub value: String,
    /// x, y, w, h in the screenshot's pixel space; all zero when unknown.
    pub bounds: (i64, i64, i64, i64),
    pub enabled: bool,
    /// The driver's per-snapshot handle (`s0000002a:7`); None for OCR rows.
    pub token: Option<String>,
    /// `tree` (the accessibility tree) or `ocr` (text read off the pixels).
    pub source: &'static str,
}

impl UiElement {
    pub fn bounds_known(&self) -> bool {
        let (x, y, w, h) = self.bounds;
        !(x == 0 && y == 0 && w == 0 && h == 0)
    }

    pub fn center(&self) -> Option<(i64, i64)> {
        let (x, y, w, h) = self.bounds;
        (self.bounds_known() && w > 0 && h > 0).then_some((x + w / 2, y + h / 2))
    }
}

/// One capture of a window (or of the whole screen).
#[derive(Clone, Debug, Default)]
pub struct Capture {
    /// `auto` | `som` | `vision` | `ax`.
    pub mode: String,
    pub width: u32,
    pub height: u32,
    /// Base64 image, when the capture carries one.
    pub image: Option<String>,
    pub image_mime: Option<String>,
    pub elements: Vec<UiElement>,
    pub app: String,
    pub window_title: String,
    pub pid: Option<u32>,
    pub window_id: Option<u64>,
    pub snapshot_id: Option<String>,
    /// Why there was nothing to capture, or guidance for the model.
    pub note: String,
    /// The driver's walk stopped at `max_elements`.
    pub truncated: bool,
    /// `since` read: the driver's diff text replaces the rows.
    pub diff: Option<String>,
    /// Why the image came along in `auto` mode.
    pub image_reason: Option<&'static str>,
}

/// What an action did (Hermes `ActionResult`). `ok` is transport success
/// only; `effect`, `escalation` and `code` carry the driver's verdict.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionResult {
    pub ok: bool,
    pub action: String,
    pub message: String,
    pub meta: Map<String, Value>,
    /// Accessibility read-back: confirmed, unconfirmed, or not applicable.
    pub verified: Option<bool>,
    /// `confirmed` | `unverifiable` | `suspected_noop`.
    pub effect: Option<String>,
    /// `{recommended: "px"|"foreground"|..., reason}`.
    pub escalation: Option<Map<String, Value>>,
    /// The delivery rung that ran.
    pub path: Option<String>,
    pub degraded: Option<bool>,
    pub delivery_mode: Option<String>,
    /// Refusal or limitation code (`stale`, `background_unavailable`, ...).
    pub code: Option<String>,
}

impl ActionResult {
    pub fn refused(action: &str, message: impl Into<String>, code: Option<&str>) -> Self {
        Self {
            ok: false,
            action: action.into(),
            message: message.into(),
            code: code.map(str::to_string),
            ..Self::default()
        }
    }
}

/// What the `computer` tool answers: MCP content parts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    pub text: String,
    /// Base64 data and its MIME type.
    pub images: Vec<(String, String)>,
    pub is_error: bool,
}

impl Output {
    pub fn text(text: impl Into<String>) -> Self {
        Self { text: text.into(), ..Self::default() }
    }

    pub fn json(v: &Value) -> Self {
        Self::text(serde_json::to_string(v).unwrap_or_default())
    }

    pub fn error(v: &Value) -> Self {
        Self { is_error: true, ..Self::json(v) }
    }

    /// The MCP `tools/call` result.
    pub fn to_mcp(&self) -> Value {
        let mut content = vec![serde_json::json!({"type": "text", "text": self.text})];
        for (data, mime) in &self.images {
            content.push(serde_json::json!({"type": "image", "data": data, "mimeType": mime}));
        }
        serde_json::json!({"content": content, "isError": self.is_error})
    }
}
