//! Plugins, on Zeron's model (its docs/plugins.md, MIT): a folder in
//! `$DOVE_HOME/plugins/<id>/` holding `kx-plugin.json` (its manifest) and
//! `init.luau` (its code), run by zgx in a sandbox of its own (a VM apart,
//! reaching only what it's granted), in the main window. Nothing is bundled
//! as running: a plugin loads only once you allow it in Settings › Plugins,
//! with exactly the permissions it asked for then; an update asking for more
//! asks again.
//!
//! Agents make plugins too, through Keplorayx's own tools (src/mcp.rs:
//! `create_plugin`, `write_plugin_file`, `read_plugin_file`, `list_plugins`,
//! `reload_plugins`); what they make waits for your allowing like any other.
//!
//! A plugin may also bring agents tools of its own: a program that speaks MCP
//! (the manifest's `mcp`), which the bridge starts and offers every agent
//! Keplorayx starts, under the plugin's id (`computer__click`). That's how
//! the Computer use plugin works: installed from the catalog, never bundled.
//!
//! Manifest:
//!   { "id": "focus-timer", "name": "Focus timer", "version": "1.0.0",
//!     "description": "…", "entry": "init.luau",
//!     "permissions": ["ui.add", "sessions.read"],
//!     "mention": "computer",            (an @-mention the composer offers)
//!     "mcp": { "command": "cua-driver", "args": ["mcp"] } }
//!
//! Settings, as Zeron's (the manifest's `settings`: switch, choice, text,
//! number): Settings › Plugins draws them under the plugin, you set them,
//! the plugin reads them (`kx.plugin_settings`, by plugin id; its SDK's
//! `kx.setting(id)`). A value never set reads its default.
//!
//! Scripts see `kx.plugins` (the list, for Settings), and call
//! `kx.plugins.allow({ id, on })`, `kx.plugins.install(id)` (from the
//! catalog), `kx.plugins.remove(id)`, `kx.plugins.folder()`,
//! `kx.plugins.set({ plugin, id, value })`.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use zgx_script::{Capability, Plugin, ScriptHost, Shared, Value};

/// Where they live.
pub fn dir() -> PathBuf {
    dove_store::default_root().join("plugins")
}

/// The catalog: plugins Keplorayx offers to install (copied into `dir()`
/// when you choose one; never running until then), from the app's own
/// `plugins/` folder.
pub fn catalog_dir() -> PathBuf {
    crate::ui_dir().parent().map(|p| p.join("plugins")).unwrap_or_else(|| PathBuf::from("plugins"))
}

const MANIFEST: &str = "kx-plugin.json";
/// The store's key for what you allowed: `{ id: [permissions] }`.
const ALLOWED: &str = "kx.plugins.allowed";
/// The store's key for what you set: `{ plugin: { setting: value } }`.
const VALUES: &str = "kx.plugins.values";
/// Zeron's helpers for a plugin, ours: a copy goes in each plugin made.
const SDK: &str = include_str!("../plugins-sdk/kx.luau");

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Mcp {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// What it tells agents about its tools (added to the bridge's
    /// instructions).
    #[serde(default)]
    pub instructions: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default = "version")]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "entry")]
    pub entry: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub mention: Option<String>,
    #[serde(default)]
    pub mcp: Option<Mcp>,
    /// What to install first, said in Settings (the Computer use plugin's
    /// `cua-driver`): a page to open.
    #[serde(default)]
    pub requires: Option<Json>,
    /// What you can set for it (Zeron's `settings`).
    #[serde(default)]
    pub settings: Vec<Json>,
}

/// The kinds of setting Settings draws.
const SETTING_KINDS: &[&str] = &["switch", "choice", "text", "number"];

/// Why a manifest's settings don't read, if they don't.
fn check_settings(settings: &[Json]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for s in settings {
        let id = s["id"].as_str().unwrap_or_default();
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
            return Err(format!("setting `{id}`: an id of letters, digits and - _ ."));
        }
        if !seen.insert(id.to_string()) {
            return Err(format!("setting `{id}` twice"));
        }
        let kind = s["kind"].as_str().unwrap_or_default();
        if !SETTING_KINDS.contains(&kind) {
            return Err(format!("setting `{id}`: kind is one of {}", SETTING_KINDS.join(", ")));
        }
        if s["label"].as_str().unwrap_or_default().is_empty() {
            return Err(format!("setting `{id}`: a label"));
        }
        if kind == "choice" && s["options"].as_array().is_none_or(|o| o.is_empty()) {
            return Err(format!("setting `{id}`: a choice needs options"));
        }
    }
    Ok(())
}

/// A setting's default: its own, else the kind's.
fn default_of(s: &Json) -> Json {
    if !s["default"].is_null() {
        return s["default"].clone();
    }
    match s["kind"].as_str() {
        Some("switch") => Json::Bool(false),
        Some("number") => s["min"].as_f64().map(Json::from).unwrap_or(Json::from(0)),
        Some("choice") => s["options"][0]["value"].clone(),
        _ => Json::from(""),
    }
}

/// `value` as setting `s` keeps it, or None when it doesn't fit.
fn fit(s: &Json, value: &Json) -> Option<Json> {
    match s["kind"].as_str()? {
        "switch" => value.as_bool().map(Json::Bool),
        "text" => value.as_str().map(|t| Json::from(t.chars().take(2000).collect::<String>())),
        "number" => {
            let mut n = value.as_f64()?;
            if let Some(min) = s["min"].as_f64() {
                n = n.max(min);
            }
            if let Some(max) = s["max"].as_f64() {
                n = n.min(max);
            }
            Some(Json::from(n))
        }
        "choice" => s["options"].as_array()?.iter().any(|o| &o["value"] == value).then(|| value.clone()),
        _ => None,
    }
}

fn values(engine: &dove_core::Engine) -> BTreeMap<String, BTreeMap<String, Json>> {
    engine.store.setting(VALUES).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Each plugin's settings as it reads them: what you set where it still
/// fits, else the default.
fn settings_of(m: &Manifest, set: Option<&BTreeMap<String, Json>>) -> serde_json::Map<String, Json> {
    m.settings
        .iter()
        .filter_map(|s| {
            let id = s["id"].as_str()?;
            let value = set.and_then(|v| v.get(id)).and_then(|v| fit(s, v)).unwrap_or_else(|| default_of(s));
            Some((id.to_string(), value))
        })
        .collect()
}

/// Its settings as Settings draws them: each with its value now.
fn with_values(m: &Manifest, set: Option<&BTreeMap<String, Json>>) -> Vec<Json> {
    let now = settings_of(m, set);
    m.settings
        .iter()
        .map(|s| {
            let mut s = s.clone();
            s["value"] = now.get(s["id"].as_str().unwrap_or_default()).cloned().unwrap_or(Json::Null);
            s
        })
        .collect()
}

/// `kx.plugin_settings`: `{ plugin: { setting: value } }` for those installed.
pub fn settings_json(engine: &dove_core::Engine) -> Json {
    let all = values(engine);
    Json::Object(
        installed()
            .into_iter()
            .filter_map(|(_, m)| m.ok())
            .map(|m| (m.id.clone(), Json::Object(settings_of(&m, all.get(&m.id)))))
            .collect(),
    )
}

fn version() -> String {
    "0.1.0".into()
}
fn entry() -> String {
    "init.luau".into()
}

/// The permissions a plugin may ask for, and what each says in Settings.
pub const PERMISSIONS: &[(&str, &str)] = &[
    ("ui.add", "Add to Keplorayx's window"),
    ("ui.change", "Replace parts of Keplorayx's window"),
    ("sessions.read", "See your threads' titles and status"),
    ("messages.read", "Read the open thread's messages"),
    ("navigate", "Open threads and pages"),
    ("compose", "Put text in the composer (you still send it)"),
    ("network", "Reach the internet"),
    ("clipboard", "Read and write the clipboard"),
    ("files", "Open files and folders you choose"),
    ("web", "Show web pages"),
    ("windows", "Open windows of its own"),
    ("terminals", "Use terminals"),
    ("tools", "Give agents tools (a program of its own)"),
];

/// A valid id: 2 to 40 lowercase letters, digits and dashes, not one of ours.
pub fn valid_id(id: &str) -> bool {
    (2..=40).contains(&id.len())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
        && !matches!(id, "kx" | "keplorayx" | "app" | "templates")
}

pub fn read_manifest(folder: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(folder.join(MANIFEST)).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let m: Manifest = serde_json::from_str(&text).map_err(|e| format!("{MANIFEST}: {e}"))?;
    if !valid_id(&m.id) {
        return Err(format!("`{}` isn't a plugin id: 2 to 40 lowercase letters, digits and dashes", m.id));
    }
    if let Some(bad) = m.permissions.iter().find(|p| !PERMISSIONS.iter().any(|(k, _)| k == p)) {
        return Err(format!("`{bad}` isn't a permission"));
    }
    if m.mcp.is_some() && !m.permissions.iter().any(|p| p == "tools") {
        return Err("`mcp` needs the `tools` permission".into());
    }
    check_settings(&m.settings)?;
    Ok(m)
}

/// Every plugin in `dir()`, read: its folder and its manifest (or why not).
pub fn installed() -> Vec<(PathBuf, Result<Manifest, String>)> {
    let Ok(read) = std::fs::read_dir(dir()) else { return Vec::new() };
    let mut out: Vec<_> = read
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let path = e.path();
            let m = read_manifest(&path);
            (path, m)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn catalog() -> Vec<Manifest> {
    let Ok(read) = std::fs::read_dir(catalog_dir()) else { return Vec::new() };
    read.flatten().filter_map(|e| read_manifest(&e.path()).ok()).collect()
}

pub fn allowed(engine: &dove_core::Engine) -> BTreeMap<String, Vec<String>> {
    engine.store.setting(ALLOWED).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn set_allowed(engine: &dove_core::Engine, map: &BTreeMap<String, Vec<String>>) {
    let _ = engine.store.set_setting(ALLOWED, &serde_json::to_string(map).unwrap_or_default());
}

/// Whether `m` runs: allowed, with every permission it asks for now.
pub fn runs(m: &Manifest, allowed: &BTreeMap<String, Vec<String>>) -> bool {
    allowed.get(&m.id).is_some_and(|granted| m.permissions.iter().all(|p| granted.contains(p)))
}

/// What each permission lets a plugin reach in zgx.
fn grants(m: &Manifest) -> Vec<Capability> {
    let mut out = Vec::new();
    for p in &m.permissions {
        match p.as_str() {
            "network" => out.push(Capability::Fetch),
            "clipboard" => out.push(Capability::Clipboard),
            "files" => out.extend([Capability::Files, Capability::Folders]),
            "web" => out.push(Capability::Web),
            "windows" => out.push(Capability::Windows),
            "terminals" => out.push(Capability::Terminals),
            "ui.change" => out.push(Capability::OverrideElement("*".into())),
            "sessions.read" | "messages.read" | "navigate" | "compose" => out.push(Capability::Named(p.clone())),
            _ => {}
        }
    }
    out
}

/// The plugins that run, as zgx loads them.
pub fn loaded(engine: &dove_core::Engine) -> Vec<Plugin> {
    let allowed = allowed(engine);
    installed()
        .into_iter()
        .filter_map(|(folder, m)| {
            let m = m.ok()?;
            if !runs(&m, &allowed) {
                return None;
            }
            let entry = folder.join(&m.entry);
            entry.is_file().then(|| Plugin { name: m.id.clone(), entry, grants: grants(&m) })
        })
        .collect()
}

/// The MCP servers of the plugins that run (the bridge starts them).
pub fn servers(engine: &dove_core::Engine) -> Vec<(String, Mcp, PathBuf)> {
    let allowed = allowed(engine);
    installed()
        .into_iter()
        .filter_map(|(folder, m)| {
            let m = m.ok()?;
            (runs(&m, &allowed)).then_some(())?;
            Some((m.id.clone(), m.mcp.clone()?, folder))
        })
        .collect()
}

/// Settings' list: installed and the catalog's, each as the page shows it.
pub fn list_json(engine: &dove_core::Engine) -> Json {
    let allowed = allowed(engine);
    let all = values(engine);
    let mut out = Vec::new();
    let mut ids = Vec::new();
    for (folder, m) in installed() {
        match m {
            Ok(m) => {
                ids.push(m.id.clone());
                out.push(json!({
                    "id": m.id, "name": m.name, "version": m.version, "description": m.description,
                    "permissions": m.permissions.iter().map(|p| json!({
                        "id": p,
                        "says": PERMISSIONS.iter().find(|(k, _)| k == p).map(|(_, s)| *s).unwrap_or(""),
                    })).collect::<Vec<_>>(),
                    "installed": true,
                    "on": runs(&m, &allowed),
                    "asks_more": allowed.contains_key(&m.id) && !runs(&m, &allowed),
                    "mention": m.mention,
                    "tools": m.mcp.is_some(),
                    "requires": m.requires,
                    "folder": folder.display().to_string(),
                    "settings": with_values(&m, all.get(&m.id)),
                }));
            }
            Err(error) => out.push(json!({
                "id": folder.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                "name": folder.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                "installed": true, "on": false, "error": error, "permissions": [],
                "folder": folder.display().to_string(),
            })),
        }
    }
    for m in catalog() {
        if ids.contains(&m.id) {
            continue;
        }
        out.push(json!({
            "id": m.id, "name": m.name, "version": m.version, "description": m.description,
            "permissions": m.permissions.iter().map(|p| json!({
                "id": p,
                "says": PERMISSIONS.iter().find(|(k, _)| k == p).map(|(_, s)| *s).unwrap_or(""),
            })).collect::<Vec<_>>(),
            "installed": false, "on": false, "mention": m.mention, "tools": m.mcp.is_some(),
            "requires": m.requires,
        }));
    }
    Json::Array(out)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)?.flatten() {
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

// ── The window's side ──────────────────────────────────────────────────

/// A host that loads plugins, its list and its plugins' settings.
type Loader = (ScriptHost, Shared, Shared);

thread_local! {
    /// The hosts that load plugins (the main window's), and the list each
    /// publishes: told again whenever a plugin changes.
    static HOSTS: RefCell<Vec<Loader>> = const { RefCell::new(Vec::new()) };
}

/// Loads the plugins again everywhere, and says so: after an allow, an
/// install, a removal, or an agent's change.
pub fn reload_all(engine: &dove_core::Engine) {
    let list = crate::panels::to_value(&list_json(engine));
    let settings = crate::panels::to_value(&settings_json(engine));
    let plugins = loaded(engine);
    HOSTS.with(|hosts| {
        for (host, shared, set) in hosts.borrow().iter() {
            host.set_plugins(plugins.clone());
            set.set(settings.clone());
            if let Err(e) = host.reload() {
                tracing::warn!("plugins: reload: {e}");
            }
            shared.set(list.clone());
        }
    });
}

pub fn install(host: &ScriptHost, engine: Arc<dove_core::Engine>) {
    let _ = std::fs::create_dir_all(dir());
    // What a plugin may reach of ours, by the permission that grants it.
    host.expose_shared_read("kx.threads", Some(Capability::Named("sessions.read".into())));
    host.expose_shared_read("kx.thread", Some(Capability::Named("sessions.read".into())));
    host.expose_shared_read("kx.entries", Some(Capability::Named("messages.read".into())));
    host.expose_action("kx.open", Some(Capability::Named("navigate".into())));
    host.expose_shared("ray.compose", Some(Capability::Named("compose".into())));
    host.expose_shared_read("kx.now", None);
    // Each plugin's settings (no secrets: every plugin reads this).
    host.expose_shared_read("kx.plugin_settings", None);
    // A short note in the window (the SDK's kx.toast).
    host.expose_shared("kx.plugin_toast", None);
    host.reserve_prefix("kx");
    host.set_plugins(loaded(&engine));
    let shared = host.shared("kx.plugins", crate::panels::to_value(&list_json(&engine)));
    let set = host.shared("kx.plugin_settings", crate::panels::to_value(&settings_json(&engine)));
    HOSTS.with(|h| h.borrow_mut().push((host.clone(), shared, set)));

    // A setting you changed: kept, and every plugin told (no reload).
    let e = engine.clone();
    host.action("kx.plugins.set", move |req| {
        let (Some(Value::Str(plugin)), Some(Value::Str(id))) = (req.field("plugin"), req.field("id")) else { return };
        let Some(m) = installed().into_iter().filter_map(|(_, m)| m.ok()).find(|m| m.id == plugin.as_ref()) else { return };
        let Some(s) = m.settings.iter().find(|s| s["id"].as_str() == Some(id.as_ref())) else { return };
        let value = req.field("value").map(crate::widgets::value_json).unwrap_or(Json::Null);
        let Some(value) = fit(s, &value) else { return };
        let mut all = values(&e);
        all.entry(m.id.clone()).or_default().insert(id.to_string(), value);
        let _ = e.store.set_setting(VALUES, &serde_json::to_string(&all).unwrap_or_default());
        let (list, settings) = (crate::panels::to_value(&list_json(&e)), crate::panels::to_value(&settings_json(&e)));
        HOSTS.with(|hosts| {
            for (_, shared, set) in hosts.borrow().iter() {
                set.set(settings.clone());
                shared.set(list.clone());
            }
        });
    });

    let e = engine.clone();
    host.action("kx.plugins.allow", move |req| {
        let Some(Value::Str(id)) = req.field("id") else { return };
        let on = matches!(req.field("on"), Some(Value::Bool(true)));
        let mut map = allowed(&e);
        if on {
            let Some(m) = installed().into_iter().filter_map(|(_, m)| m.ok()).find(|m| m.id == id.as_ref()) else { return };
            map.insert(m.id.clone(), m.permissions.clone());
        } else {
            map.remove(id.as_ref());
        }
        set_allowed(&e, &map);
        reload_all(&e);
    });
    let e = engine.clone();
    host.action("kx.plugins.install", move |id| {
        let Value::Str(id) = id else { return };
        if !valid_id(&id) {
            return;
        }
        let from = catalog_dir().join(id.as_ref());
        if let Err(err) = copy_dir(&from, &dir().join(id.as_ref())) {
            tracing::warn!("plugins: install {id}: {err}");
        }
        reload_all(&e);
    });
    let e = engine.clone();
    host.action("kx.plugins.remove", move |id| {
        let Value::Str(id) = id else { return };
        if !valid_id(&id) {
            return;
        }
        let mut map = allowed(&e);
        map.remove(id.as_ref());
        set_allowed(&e, &map);
        let _ = std::fs::remove_dir_all(dir().join(id.as_ref()));
        // What you set for it goes with it.
        let mut all = values(&e);
        if all.remove(id.as_ref()).is_some() {
            let _ = e.store.set_setting(VALUES, &serde_json::to_string(&all).unwrap_or_default());
        }
        reload_all(&e);
    });
    host.action("kx.plugins.folder", |_| crate::open_external(&dir().to_string_lossy()));
}

// ── Agents' side (the bridge's calls, on this thread) ──────────────────

/// `create_plugin`: a new plugin's folder, its manifest and code. It runs
/// once you allow it.
pub fn create(args: &Json) -> Result<String, String> {
    let id = args["id"].as_str().unwrap_or_default();
    if !valid_id(id) {
        return Err("id: 2 to 40 lowercase letters, digits and dashes".into());
    }
    let folder = dir().join(id);
    if folder.exists() && !args["replace"].as_bool().unwrap_or(false) {
        return Err(format!("`{id}` exists: write its files with write_plugin_file, or pass replace: true"));
    }
    let manifest = json!({
        "id": id,
        "name": args["name"].as_str().unwrap_or(id),
        "version": args["version"].as_str().unwrap_or("0.1.0"),
        "description": args["description"].as_str().unwrap_or(""),
        "entry": "init.luau",
        "permissions": args["permissions"].as_array().cloned().unwrap_or_default(),
        "settings": args["settings"].as_array().cloned().unwrap_or_default(),
    });
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    std::fs::write(folder.join(MANIFEST), serde_json::to_string_pretty(&manifest).unwrap_or_default())
        .map_err(|e| e.to_string())?;
    std::fs::write(folder.join("init.luau"), args["code"].as_str().unwrap_or("-- Your plugin.\n"))
        .map_err(|e| e.to_string())?;
    // The helpers: `local kx = require("./kx")("<id>")`.
    std::fs::write(folder.join("kx.luau"), SDK).map_err(|e| e.to_string())?;
    read_manifest(&folder)?;
    Ok(format!(
        "Created `{id}` in {}. The person allows it in Settings › Plugins before it runs; then it reloads on each change.",
        folder.display()
    ))
}

fn inside(id: &str, path: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("no such plugin".into());
    }
    let rel = Path::new(path);
    if rel.is_absolute() || rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err("a path inside the plugin's folder".into());
    }
    Ok(dir().join(id).join(rel))
}

pub fn write_file(args: &Json) -> Result<String, String> {
    let id = args["id"].as_str().unwrap_or_default();
    let path = inside(id, args["path"].as_str().unwrap_or_default())?;
    if !dir().join(id).is_dir() {
        return Err(format!("no plugin `{id}`: create_plugin first"));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, args["content"].as_str().unwrap_or_default()).map_err(|e| e.to_string())?;
    // A manifest that no longer reads says so now.
    read_manifest(&dir().join(id))?;
    Ok(format!("Wrote {}", path.display()))
}

pub fn read_file(args: &Json) -> Result<String, String> {
    let id = args["id"].as_str().unwrap_or_default();
    let path = inside(id, args["path"].as_str().unwrap_or("init.luau"))?;
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

pub fn list_text(engine: &dove_core::Engine) -> String {
    let mut text = serde_json::to_string_pretty(&list_json(engine)).unwrap_or_default();
    // What failed as the window loaded them (a plugin's own error names it).
    let errors: Vec<String> = HOSTS.with(|hosts| {
        hosts.borrow().iter().filter_map(|(host, _, _)| host.error().with_untracked(Clone::clone)).collect()
    });
    if !errors.is_empty() {
        text.push_str("

Errors:
");
        text.push_str(&errors.join("
"));
    }
    text
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn values_fit_their_setting_or_fall_back() {
        let number = json!({ "id": "n", "kind": "number", "label": "N", "min": 5, "max": 90 });
        assert_eq!(fit(&number, &json!(200)), Some(json!(90.0)));
        assert_eq!(fit(&number, &json!("x")), None);
        assert_eq!(default_of(&number), json!(5.0));
        let choice = json!({ "id": "c", "kind": "choice", "label": "C", "options": [{ "value": "a" }, { "value": "b" }] });
        assert_eq!(fit(&choice, &json!("b")), Some(json!("b")));
        assert_eq!(fit(&choice, &json!("z")), None);
        assert_eq!(default_of(&choice), json!("a"));
        assert!(check_settings(&[json!({ "id": "x", "kind": "colour", "label": "X" })]).is_err());
        assert!(check_settings(&[choice.clone(), choice]).is_err());
    }
}
