//! The MCP transport to cua-driver, and the flattened tool result every
//! caller reads (Hermes `cua_backend_parse._extract_tool_result`).

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

/// One MCP connection to `cua-driver mcp`. The app implements it over its
/// stdio client; tests use a scripted fake.
pub trait Driver {
    /// `tools/call`: the raw `CallToolResult` JSON.
    fn call_tool(&mut self, name: &str, args: Value) -> Result<Value, String>;
    /// `tools/list`: every tool with its `inputSchema`.
    fn list_tools(&mut self) -> Result<Vec<Value>, String>;
}

/// A flattened tool result: joined text (parsed when it is JSON), image
/// parts with their MIME types, `structuredContent` and `isError`.
#[derive(Clone, Debug, Default)]
pub struct Envelope {
    pub data: Value,
    pub images: Vec<(String, String)>,
    pub structured: Map<String, Value>,
    pub is_error: bool,
}

impl Envelope {
    pub fn from_result(result: &Value) -> Self {
        let mut texts = Vec::new();
        let mut images = Vec::new();
        for part in result.get("content").and_then(Value::as_array).into_iter().flatten() {
            match part.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(t) = part.get("text").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                        texts.push(t.to_string());
                    }
                }
                Some("image") => {
                    if let Some(d) = part.get("data").and_then(Value::as_str).filter(|d| !d.is_empty()) {
                        let mime = part.get("mimeType").and_then(Value::as_str).unwrap_or("").to_string();
                        images.push((d.to_string(), mime));
                    }
                }
                _ => {}
            }
        }
        let joined = texts.join("\n");
        let trimmed = joined.trim_start();
        let data = if trimmed.starts_with('{') || trimmed.starts_with('[') {
            serde_json::from_str(&joined).unwrap_or(Value::String(joined))
        } else {
            Value::String(joined)
        };
        let structured = result.get("structuredContent").and_then(Value::as_object).cloned().unwrap_or_default();
        // Identity, not truthiness: only an explicit `true` is an error.
        let is_error = result.get("isError") == Some(&Value::Bool(true));
        Self { data, images, structured, is_error }
    }

    /// The text payload, when it is text.
    pub fn text(&self) -> &str {
        self.data.as_str().unwrap_or("")
    }

    /// Text for an error message: the text, else `structured.message`.
    pub fn message(&self) -> String {
        let t = match &self.data {
            Value::String(s) => s.clone(),
            Value::Object(o) => o.get("message").and_then(Value::as_str).unwrap_or_default().to_string(),
            _ => String::new(),
        };
        if !t.is_empty() {
            return t;
        }
        self.structured.get("message").and_then(Value::as_str).unwrap_or_default().to_string()
    }

    /// The first image (base64, MIME): an image part, else a base64 field
    /// in `structuredContent` (newer builds put it there).
    pub fn image(&self) -> Option<(String, Option<String>)> {
        if let Some((d, m)) = self.images.first() {
            return Some((d.clone(), (!m.is_empty()).then(|| m.clone())));
        }
        let b64 = ["screenshot_png_b64", "png_b64"]
            .iter()
            .find_map(|k| self.structured.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()))?;
        let mime = ["screenshot_mime_type", "mime_type"]
            .iter()
            .find_map(|k| self.structured.get(*k).and_then(Value::as_str))
            .map(str::to_string);
        Some((b64.to_string(), mime))
    }

    /// `structured[key]`, falling back to the object text payload.
    pub fn field(&self, key: &str) -> Option<&Value> {
        self.structured.get(key).or_else(|| self.data.as_object().and_then(|o| o.get(key)))
    }
}

/// What the live driver accepts, from `tools/list`: per tool, its declared
/// properties and whether it refuses others. Every call is filtered through
/// it, so a property one platform's build lacks never fails the call.
#[derive(Clone, Debug, Default)]
pub struct Schemas {
    tools: HashMap<String, (HashSet<String>, bool)>,
    discovered: bool,
}

impl Schemas {
    pub fn from_tools(tools: &[Value]) -> Self {
        let mut map = HashMap::new();
        for t in tools {
            let Some(name) = t.get("name").and_then(Value::as_str) else { continue };
            let schema = t.get("inputSchema").cloned().unwrap_or(Value::Null);
            let props: HashSet<String> = schema
                .get("properties")
                .and_then(Value::as_object)
                .map(|p| p.keys().cloned().collect())
                .unwrap_or_default();
            let strict = schema.get("additionalProperties") == Some(&Value::Bool(false));
            map.insert(name.to_string(), (props, strict));
        }
        Self { tools: map, discovered: true }
    }

    pub fn discovered(&self) -> bool {
        self.discovered
    }

    pub fn has_tool(&self, tool: &str) -> bool {
        !self.discovered || self.tools.contains_key(tool)
    }

    /// Whether `tool` accepts `prop`. Unknown before discovery: yes.
    pub fn accepts(&self, tool: &str, prop: &str) -> bool {
        match self.tools.get(tool) {
            Some((props, strict)) => props.contains(prop) || !strict,
            None => !self.discovered,
        }
    }

    /// `args` without the properties a strict schema would refuse.
    pub fn filter(&self, tool: &str, mut args: Map<String, Value>) -> Map<String, Value> {
        if let Some((props, true)) = self.tools.get(tool) {
            args.retain(|k, _| props.contains(k));
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flattens_text_images_and_structured() {
        let r = json!({"content": [{"type": "text", "text": "{\"ok\":true}"},
            {"type": "image", "data": "iVBORw0KGgo", "mimeType": "image/png"}],
            "structuredContent": {"snapshot_id": "s00000001"}, "isError": false});
        let e = Envelope::from_result(&r);
        assert_eq!(e.data, json!({"ok": true}));
        assert_eq!(e.image(), Some(("iVBORw0KGgo".into(), Some("image/png".into()))));
        assert_eq!(e.field("snapshot_id"), Some(&json!("s00000001")));
        assert!(!e.is_error);
        // A truthy non-boolean is not an error.
        assert!(!Envelope::from_result(&json!({"isError": "yes"})).is_error);
        let s = Envelope::from_result(&json!({"structuredContent": {"screenshot_png_b64": "AAAA"}}));
        assert_eq!(s.image(), Some(("AAAA".into(), None)));
    }

    #[test]
    fn strict_schemas_drop_unknown_properties() {
        let tools = vec![
            json!({"name": "click", "inputSchema": {"properties": {"pid": {}, "element_token": {}}, "additionalProperties": false}}),
            json!({"name": "loose", "inputSchema": {"properties": {"a": {}}}}),
        ];
        let s = Schemas::from_tools(&tools);
        let args = json!({"pid": 1, "element_index": 3, "element_token": "s00000001:3"}).as_object().cloned().unwrap();
        let out = s.filter("click", args.clone());
        assert!(out.contains_key("pid") && out.contains_key("element_token") && !out.contains_key("element_index"));
        assert_eq!(s.filter("loose", args.clone()).len(), 3);
        assert!(s.accepts("click", "pid") && !s.accepts("click", "nope") && s.accepts("loose", "nope"));
        assert!(!s.has_tool("missing") && Schemas::default().has_tool("missing"));
    }
}
