//! Replays: a verified `run`, saved with role/name selectors instead of
//! snapshot tokens, so it runs again on the window as it is later. The
//! driver re-finds each target by role and name and auto-waits for it; the
//! first step that no longer matches stops the replay for the model to
//! finish by hand.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Replay {
    pub version: u32,
    pub name: String,
    pub app: String,
    pub steps: Vec<Value>,
}

/// A file-safe name: lowercase letters, digits and dashes.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(60).collect()
}

fn dir(cache: &Path) -> PathBuf {
    cache.join("replays")
}

pub fn save(cache: &Path, name: &str, app: &str, steps: &[Value]) -> Result<PathBuf, String> {
    let slug = slug(name);
    if slug.is_empty() {
        return Err("name needs at least one letter or digit".into());
    }
    let d = dir(cache);
    std::fs::create_dir_all(&d).map_err(|e| format!("could not create {}: {e}", d.display()))?;
    let path = d.join(format!("{slug}.json"));
    let r = Replay { version: 1, name: slug, app: app.to_string(), steps: steps.to_vec() };
    let text = serde_json::to_vec_pretty(&r).map_err(|e| e.to_string())?;
    crate::capture::write_private(&path, &text).map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(path)
}

pub fn load(cache: &Path, name: &str) -> Result<Replay, String> {
    let path = dir(cache).join(format!("{}.json", slug(name)));
    let bytes = std::fs::read(&path).map_err(|_| format!("no replay named {:?}; list_replays shows the saved ones", slug(name)))?;
    let r: Replay = serde_json::from_slice(&bytes).map_err(|e| format!("{} is not a replay: {e}", path.display()))?;
    if r.version != 1 || r.steps.is_empty() || r.steps.len() > 25 {
        return Err(format!("{} is not a usable replay", path.display()));
    }
    // Only batchable input tools, never anything that reads files or runs scripts.
    if let Some(bad) = r.steps.iter().find(|s| !matches!(s["tool"].as_str(), Some("click" | "type_text" | "press_key" | "hotkey" | "set_value" | "scroll") | None)) {
        return Err(format!("replay {} has an unsupported step {}", r.name, bad["tool"]));
    }
    Ok(r)
}

/// Saved replays: name, app and step count.
pub fn list(cache: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = std::fs::read_dir(dir(cache))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let r: Replay = serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()?;
            Some(serde_json::json!({"name": r.name, "app": r.app, "steps": r.steps.len()}))
        })
        .collect();
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    out
}

/// One line for the review card.
pub fn describe(step: &Value) -> String {
    let a = &step["args"];
    let target = a.get("name").and_then(Value::as_str).map(|n| format!(" {n:?}")).unwrap_or_default();
    match step["tool"].as_str().unwrap_or("") {
        "type_text" => format!("type {:?}{target}", a.get("text").and_then(Value::as_str).unwrap_or("").chars().take(40).collect::<String>()),
        "set_value" => format!("set{target} to {:?}", a.get("value").and_then(Value::as_str).unwrap_or("")),
        "press_key" => format!("press {}", a.get("key").and_then(Value::as_str).unwrap_or("")),
        "hotkey" => format!("press {}", a.get("keys").and_then(Value::as_array).map(|k| k.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("+")).unwrap_or_default()),
        "scroll" => format!("scroll {}", a.get("direction").and_then(Value::as_str).unwrap_or("down")),
        t => format!("{t}{target}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trip_and_refuse_unknown_tools() {
        let dir = tempfile::tempdir().unwrap();
        let steps = vec![json!({"tool": "click", "args": {"role": "Button", "name": "Export"}})];
        save(dir.path(), "Export Report!", "Excel", &steps).unwrap();
        let r = load(dir.path(), "export report").unwrap();
        assert_eq!((r.name.as_str(), r.app.as_str(), r.steps.len()), ("export-report", "Excel", 1));
        assert_eq!(list(dir.path())[0]["name"], "export-report");
        assert_eq!(describe(&steps[0]), "click \"Export\"");
        save(dir.path(), "bad", "X", &[json!({"tool": "run_script", "args": {}})]).unwrap();
        assert!(load(dir.path(), "bad").is_err());
        assert!(load(dir.path(), "missing").is_err());
        assert!(save(dir.path(), "!!!", "X", &steps).is_err());
    }
}
