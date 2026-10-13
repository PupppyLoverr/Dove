//! What the agent picker remembers between launches: starred models, the
//! last model / effort / traits used with each agent, and which agent new
//! sessions start on. Stored as one JSON value in the settings table.

use std::collections::HashMap;
use std::sync::Arc;

use dove_core::proto::{HarnessKind, ReasoningLevel};
use dove_core::store::Store;
use serde_json::{Map, Value, json};

const KEY: &str = "picker.v1";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pick {
    pub model: String,
    pub reasoning: Option<ReasoningLevel>,
    pub options: Vec<(String, String)>,
}

#[derive(Default)]
pub struct PickerMemory {
    favorites: Vec<(HarnessKind, String)>,
    last: HashMap<HarnessKind, Pick>,
    default: Option<HarnessKind>,
    store: Option<Arc<Store>>,
}

impl PickerMemory {
    pub fn load(store: Arc<Store>) -> Self {
        let mut this = store
            .setting(KEY)
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .map(|v| Self::from_json(&v))
            .unwrap_or_default();
        this.store = Some(store);
        this
    }

    fn from_json(v: &Value) -> Self {
        let favorites = v["favorites"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|f| Some((HarnessKind::from_id(f[0].as_str()?)?, f[1].as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let last = v["last"]
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(kind, p)| {
                        let options = p["options"]
                            .as_object()
                            .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
                            .unwrap_or_default();
                        Some((
                            HarnessKind::from_id(kind)?,
                            Pick {
                                model: p["model"].as_str()?.to_string(),
                                reasoning: p["reasoning"].as_str().and_then(ReasoningLevel::from_id),
                                options,
                            },
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let default = v["default"].as_str().and_then(HarnessKind::from_id);
        Self { favorites, last, default, store: None }
    }

    fn to_json(&self) -> Value {
        let favorites: Vec<Value> = self.favorites.iter().map(|(k, m)| json!([k.id(), m])).collect();
        let mut last = Map::new();
        for (kind, pick) in &self.last {
            let options: Map<String, Value> =
                pick.options.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
            last.insert(
                kind.id().to_string(),
                json!({"model": pick.model, "reasoning": pick.reasoning.map(|r| r.id()), "options": options}),
            );
        }
        json!({"favorites": favorites, "last": last, "default": self.default.map(|k| k.id())})
    }

    fn save(&self) {
        if let Some(store) = &self.store
            && let Err(err) = store.set_setting(KEY, &self.to_json().to_string())
        {
            tracing::warn!("saving picker memory: {err}");
        }
    }

    pub fn is_favorite(&self, kind: HarnessKind, model: &str) -> bool {
        self.favorites.iter().any(|(k, m)| *k == kind && m == model)
    }

    pub fn favorites(&self) -> &[(HarnessKind, String)] {
        &self.favorites
    }

    pub fn toggle_favorite(&mut self, kind: HarnessKind, model: &str) {
        if let Some(at) = self.favorites.iter().position(|(k, m)| *k == kind && m == model) {
            self.favorites.remove(at);
        } else {
            self.favorites.push((kind, model.to_string()));
        }
        self.save();
    }

    pub fn last(&self, kind: HarnessKind) -> Option<&Pick> {
        self.last.get(&kind)
    }

    pub fn default_harness(&self) -> Option<HarnessKind> {
        self.default
    }

    /// Record a pick. `sticky` also makes `kind` the agent new sessions
    /// start on (the new-session composer, not a task's).
    pub fn remember(&mut self, kind: HarnessKind, pick: Pick, sticky: bool) {
        let unchanged = self.last.get(&kind) == Some(&pick) && (!sticky || self.default == Some(kind));
        if unchanged {
            return;
        }
        self.last.insert(kind, pick);
        if sticky {
            self.default = Some(kind);
        }
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let mut m = PickerMemory::default();
        m.toggle_favorite(HarnessKind::Codex, "gpt-5.5");
        m.toggle_favorite(HarnessKind::ClaudeCode, "claude-opus-5-5");
        m.toggle_favorite(HarnessKind::Codex, "gpt-5.5");
        m.remember(
            HarnessKind::Kilo,
            Pick {
                model: "kilo/anthropic/claude-opus-5.5".into(),
                reasoning: Some(ReasoningLevel::XHigh),
                options: vec![("speed".into(), "fast".into())],
            },
            true,
        );
        let back = PickerMemory::from_json(&m.to_json());
        assert_eq!(back.favorites(), &[(HarnessKind::ClaudeCode, "claude-opus-5-5".to_string())]);
        assert_eq!(back.default_harness(), Some(HarnessKind::Kilo));
        let pick = back.last(HarnessKind::Kilo).unwrap();
        assert_eq!(pick.reasoning, Some(ReasoningLevel::XHigh));
        assert_eq!(pick.options, vec![("speed".to_string(), "fast".to_string())]);
    }

    #[test]
    fn garbage_loads_empty() {
        let m = PickerMemory::from_json(&json!({"favorites": [["nope", 3]], "last": {"codex": {}}}));
        assert!(m.favorites().is_empty());
        assert!(m.last(HarnessKind::Codex).is_none());
    }
}
