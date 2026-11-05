//! What the panels and pages ask of the host, beside the threads
//! (`host.rs`): a project's files and their text, its history and pull
//! requests, the app's settings, the team, and Windows' voice typing. Every
//! call reads on a thread of its own; nothing blocks the window.
//!
//! Calls (async unless said):
//!   kx.list_dir({ project?, path })   one folder: { { name, path, dir, size, status } },
//!                                     folders first; `status` is git's mark ("M", "?", â€¦)
//!   kx.read_file({ project?, path })  { text, lang, size, binary, truncated, image, full }
//!   kx.history({ project?, query? })  { { sha, short, subject, author, when, refs, head,
//!                                     lane, top, bottom, joins, forks } }
//!   kx.pulls({ project? })            { { number, title, author, branch, state, draft,
//!                                     url, updated, review, added, removed } }; `gh` missing:
//!                                     an error that says so
//!   kx.settings()                     every setting: { [key] = value }
//!   kx.set_setting({ key, value })    an action: write one
//!   kx.team()                         { agents = { { id, label, installed } },
//!                                     members = { { id, name, role, avatar, hue, harness,
//!                                     instructions } } }
//!   kx.save_member(member)            add or change a teammate; its id
//!   kx.delete_member(id)              an action
//!   kx.dictate()                      an action: Windows' voice typing (Win+H)
//!
//! `project` is a project's id; without one, the open thread's folder (its
//! worktree, if it has one), else the first project.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use dove_core::Engine;
use dove_proto::TaskId;
use serde_json::{Value as Json, json};
use zgx_script::{ScriptHost, Value};

use crate::host::off_thread;

/// A file's text is shown up to this much.
const READ_CAP: u64 = 512 * 1024;

pub fn install(host: &ScriptHost, engine: Arc<Engine>, open: impl Fn() -> Option<TaskId> + Clone + 'static) {
    let (e, o) = (engine.clone(), open.clone());
    host.async_action("kx.list_dir", move |req| {
        let (engine, open) = (e.clone(), o());
        async move {
            let rel = field(&req, "path").unwrap_or_default();
            let project = field(&req, "project");
            off_thread(move || -> Result<Json, String> {
                let root = project_dir(&engine, project.as_deref(), open)?;
                let dir = inside(&root, &rel)?;
                Ok(Json::Array(list_dir(&root, &dir)))
            })
            .await
            .ok_or("failed")?
            .map(|j| to_value(&j))
        }
    });

    // The branch a project's folder is on (the composer's where-chip: a new
    // thread starts in the project's own checkout, on this branch).
    let (e, o) = (engine.clone(), open.clone());
    host.async_action("kx.branch", move |req| {
        let (engine, open) = (e.clone(), o());
        async move {
            let project = field(&req, "project");
            off_thread(move || -> Result<Json, String> {
                let root = project_dir(&engine, project.as_deref(), open)?;
                if dove_git::repo_root(&root).is_none() {
                    return Ok(Json::Null);
                }
                Ok(dove_git::current_branch(&root).map(Json::from).unwrap_or(Json::Null))
            })
            .await
            .ok_or("failed")?
            .map(|j| to_value(&j))
        }
    });

    // Every file at once (git's list, or a walk outside a repository), with
    // git's word for each that changed: the file pane builds its tree from it.
    let (e, o) = (engine.clone(), open.clone());
    host.async_action("kx.list_files", move |req| {
        let (engine, open) = (e.clone(), o());
        async move {
            let project = field(&req, "project");
            off_thread(move || -> Result<Json, String> {
                let root = project_dir(&engine, project.as_deref(), open)?;
                let files = if dove_git::repo_root(&root).is_some() {
                    dove_git::list_files(&root).map_err(|e| format!("{e:#}"))?
                } else {
                    walk(&root, &root, 20_000)
                };
                let words = |c: char| match c {
                    'A' => "added",
                    'D' => "deleted",
                    'R' => "renamed",
                    '?' => "untracked",
                    _ => "modified",
                };
                let statuses: serde_json::Map<String, Json> = dove_git::status(&root)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(p, c)| (p, Json::from(words(c))))
                    .collect();
                let name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                Ok(json!({
                    "name": name,
                    "root": root.to_string_lossy(),
                    "files": files.join("\n"),
                    "statuses": statuses,
                }))
            })
            .await
            .ok_or("failed")?
            .map(|j| to_value(&j))
        }
    });

    let (e, o) = (engine.clone(), open.clone());
    host.async_action("kx.read_file", move |req| {
        let (engine, open) = (e.clone(), o());
        async move {
            let rel = field(&req, "path").unwrap_or_default();
            let project = field(&req, "project");
            off_thread(move || -> Result<Json, String> {
                let root = project_dir(&engine, project.as_deref(), open)?;
                read_file(&inside(&root, &rel)?)
            })
            .await
            .ok_or("failed")?
            .map(|j| to_value(&j))
        }
    });

    let (e, o) = (engine.clone(), open.clone());
    host.async_action("kx.history", move |req| {
        let (engine, open) = (e.clone(), o());
        async move {
            let project = field(&req, "project");
            let query = field(&req, "query").unwrap_or_default();
            off_thread(move || -> Result<Json, String> {
                let root = project_dir(&engine, project.as_deref(), open)?;
                if dove_git::repo_root(&root).is_none() {
                    return Ok(json!([]));
                }
                let commits = dove_git::history(&root, &query, 200).map_err(|e| format!("{e:#}"))?;
                Ok(Json::Array(commits.iter().map(commit_value).collect()))
            })
            .await
            .ok_or("failed")?
            .map(|j| to_value(&j))
        }
    });

    let (e, o) = (engine.clone(), open.clone());
    host.async_action("kx.pulls", move |req| {
        let (engine, open) = (e.clone(), o());
        async move {
            let project = field(&req, "project");
            off_thread(move || -> Result<Json, String> {
                if !dove_git::github::gh_available() {
                    return Err("GitHub's command line (gh) isn't installed".into());
                }
                let root = project_dir(&engine, project.as_deref(), open)?;
                let pulls = dove_git::github::pull_requests(&root, dove_git::github::PrFilter::All)
                    .map_err(|e| format!("{e:#}"))?;
                Ok(Json::Array(pulls.iter().map(pull_value).collect()))
            })
            .await
            .ok_or("failed")?
            .map(|j| to_value(&j))
        }
    });

    // The model picker's catalog: every installed agent's models, each with
    // its reasoning levels (as their ids) and the one it starts on.
    let e = engine.clone();
    host.async_action("kx.models", move |_| {
        let engine = e.clone();
        async move {
            off_thread(move || -> Result<Json, String> {
                let mut models = Vec::new();
                for kind in engine.installed().into_iter().map(|i| i.kind) {
                    for m in engine.models(kind) {
                        let efforts: Vec<Json> = m
                            .reasoning_levels
                            .iter()
                            .map(|l| json!({ "value": serde_json::to_value(l).unwrap_or(Json::Null), "label": l.label() }))
                            .collect();
                        let effort = efforts.iter().find(|l| l["value"] == "medium").or(efforts.first()).map(|l| l["value"].clone());
                        models.push(json!({
                            "value": format!("{}:{}", kind.id(), m.id),
                            "label": m.label,
                            "harness": kind.id(),
                            "efforts": efforts,
                            "effort": effort,
                        }));
                    }
                }
                Ok(Json::Array(models))
            })
            .await
            .ok_or("failed")?
            .map(|j| to_value(&j))
        }
    });

    let e = engine.clone();
    host.async_action("kx.settings", move |_| {
        let engine = e.clone();
        async move {
            let all = off_thread(move || engine.store.settings_with_prefix("")).await.ok_or("failed")?;
            Ok(Value::record(all.iter().map(|(k, v)| (k.as_str(), Value::from(v.as_str())))))
        }
    });

    let e = engine.clone();
    host.action("kx.set_setting", move |req| {
        let (Some(key), Some(value)) = (field(&req, "key"), scalar(req.field("value"))) else { return };
        if let Err(err) = e.store.set_setting(&key, &value) {
            tracing::warn!("setting {key}: {err:#}");
        }
    });

    let e = engine.clone();
    host.async_action("kx.team", move |_| {
        let engine = e.clone();
        async move {
            off_thread(move || -> Result<Json, String> {
                let installed: Vec<_> = engine.installed().into_iter().map(|i| i.kind).collect();
                let agents: Vec<Json> = dove_proto::HarnessKind::ALL
                    .iter()
                    .map(|k| json!({ "id": k.id(), "label": k.label(), "installed": installed.contains(k) }))
                    .collect();
                let members = engine.store.personas().map_err(|e| format!("{e:#}"))?;
                let members: Vec<Json> = members.iter().map(member_value).collect();
                Ok(json!({ "agents": agents, "members": members }))
            })
            .await
            .ok_or("failed")?
            .map(|j| to_value(&j))
        }
    });

    let e = engine.clone();
    host.async_action("kx.save_member", move |req| {
        let engine = e.clone();
        // What the script sent, read here: a script's values stay on its
        // thread.
        let id = field(&req, "id").filter(|s| !s.is_empty()).unwrap_or_else(|| format!("p{}", now_ms()));
        let persona = dove_store::Persona {
            id: id.clone(),
            name: field(&req, "name").unwrap_or_default(),
            role: field(&req, "role").unwrap_or_default(),
            avatar: field(&req, "avatar").unwrap_or_default(),
            hue: number(req.field("hue")).unwrap_or(0.6) as f32,
            harness: field(&req, "harness").and_then(|h| dove_proto::HarnessKind::from_id(&h)),
            model: field(&req, "model").filter(|m| !m.is_empty()),
            instructions: field(&req, "instructions").unwrap_or_default(),
            created_at: now_ms() / 1000,
        };
        async move {
            off_thread(move || -> Result<Json, String> {
                engine.store.save_persona(&persona).map_err(|e| format!("{e:#}"))?;
                Ok(Json::from(id))
            })
            .await
            .ok_or("failed")?
            .map(|j| to_value(&j))
        }
    });

    let e = engine;
    host.action("kx.delete_member", move |id| {
        if let Some(id) = scalar(Some(&id))
            && let Err(err) = e.store.delete_persona(&id)
        {
            tracing::warn!("teammate {id}: {err:#}");
        }
    });

    host.action("kx.dictate", |_| crate::platform::dictate());
}

fn field(v: &Value, key: &str) -> Option<String> {
    match v.field(key) {
        Some(Value::Str(s)) => Some(s.to_string()),
        _ => None,
    }
}

fn number(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Num(n)) => Some(*n),
        _ => None,
    }
}

/// A setting's value as Dove's store keeps it: text.
fn scalar(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Str(s) => Some(s.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Num(n) => Some(if n.fract() == 0.0 { format!("{}", *n as i64) } else { n.to_string() }),
        _ => None,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// The folder a call means: the project asked for, else the open thread's
/// (its worktree first), else the first project.
fn project_dir(engine: &Engine, project: Option<&str>, open: Option<TaskId>) -> Result<PathBuf, String> {
    let projects = engine.store.projects().map_err(|e| format!("{e:#}"))?;
    if let Some(id) = project.filter(|p| !p.is_empty()) {
        return projects
            .into_iter()
            .find(|p| p.id.0.to_string() == id)
            .map(|p| p.path)
            .ok_or_else(|| "no such project".into());
    }
    if let Some(id) = open
        && let Ok(Some(task)) = engine.store.task(id)
    {
        if let Some(wt) = task.worktree.filter(|w| w.exists()) {
            return Ok(wt);
        }
        if let Some(p) = projects.iter().find(|p| p.id == task.project_id) {
            return Ok(p.path.clone());
        }
    }
    projects.into_iter().next().map(|p| p.path).ok_or_else(|| "no project yet".into())
}

/// `rel` under `root`, refusing a path that climbs out of it.
fn inside(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = Path::new(rel);
    if rel.is_absolute() || rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err("outside the project".into());
    }
    Ok(root.join(rel))
}

/// Folders a project never needs to show.
const HIDDEN: &[&str] = &[".git", "node_modules", "target", ".next", "dist", "__pycache__", ".venv"];

fn list_dir(root: &Path, dir: &Path) -> Vec<Json> {
    let marks: std::collections::HashMap<String, char> = dove_git::status(root).unwrap_or_default().into_iter().collect();
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut items: Vec<(bool, String, Json)> = read
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let meta = entry.metadata().ok()?;
            let is_dir = meta.is_dir();
            if is_dir && HIDDEN.contains(&name.as_str()) {
                return None;
            }
            let rel = entry.path().strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
            // A folder shows a change below it as its own.
            let status = if is_dir {
                let prefix = format!("{rel}/");
                marks.keys().any(|k| k.starts_with(&prefix)).then_some('M')
            } else {
                marks.get(&rel).copied()
            };
            let status = status.map(String::from).unwrap_or_default();
            let value = json!({ "name": name, "path": rel, "dir": is_dir, "size": meta.len(), "status": status });
            Some((is_dir, name.to_lowercase(), value))
        })
        .collect();
    items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    items.into_iter().map(|(_, _, v)| v).collect()
}

/// Every file under `dir` (relative to `root`, `/`-separated), the folders
/// a project never shows left out, at most `cap`.
fn walk(root: &Path, dir: &Path, cap: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else { continue };
        for entry in read.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !HIDDEN.contains(&name.as_str()) {
                    stack.push(path);
                }
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
                if out.len() >= cap {
                    return out;
                }
            }
        }
    }
    out.sort();
    out
}

/// A language for a file's extension (the viewer's highlighter's names).
fn lang_of(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "rs" => "rust",
        "luau" | "lua" => "luau",
        "ts" | "tsx" | "mts" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "py" => "python",
        "go" => "go",
        "json" | "jsonc" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "css" | "scss" => "css",
        "html" | "htm" => "html",
        "sh" | "bash" | "zsh" => "bash",
        "ps1" => "powershell",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        "swift" => "swift",
        "kt" => "kotlin",
        "java" => "java",
        _ => "",
    }
}

const IMAGES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg"];
/// Documents a preview opens in the system's own viewer.
const DOCUMENTS: &[&str] = &["pdf", "docx", "doc", "xlsx", "xls", "pptx", "ppt", "odt", "ods", "odp"];

fn read_file(path: &Path) -> Result<Json, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let full = path.to_string_lossy().to_string();
    let base = |text: &str, binary: bool, truncated: bool, kind: &str| {
        json!({
            "text": text, "lang": lang_of(path), "size": meta.len(), "binary": binary,
            "truncated": truncated, "kind": kind, "full": full,
        })
    };
    if IMAGES.contains(&ext.as_str()) {
        return Ok(base("", true, false, "image"));
    }
    if DOCUMENTS.contains(&ext.as_str()) {
        let text = crate::preview::document_text(path).unwrap_or_default();
        return Ok(base(&text, true, false, "document"));
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(READ_CAP).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.iter().take(8000).any(|&b| b == 0) {
        return Ok(base("", true, false, "binary"));
    }
    let text = String::from_utf8_lossy(&bytes);
    Ok(base(&text, false, meta.len() > READ_CAP, "text"))
}

fn commit_value(c: &dove_git::HistoryCommit) -> Json {
    json!({
        "sha": c.sha, "short": c.short, "subject": c.subject, "author": c.author, "when": c.when,
        "refs": c.refs, "head": c.head, "lane": c.lane,
        "top": c.graph.top, "bottom": c.graph.bottom, "joins": c.graph.joins, "forks": c.graph.forks,
    })
}

fn pull_value(p: &dove_git::github::PullRequest) -> Json {
    json!({
        "number": p.number, "title": p.title, "author": p.author, "branch": p.branch, "state": p.state,
        "draft": p.draft, "url": p.url, "updated": p.updated_at, "review": p.review.as_deref().unwrap_or(""),
        "added": p.additions, "removed": p.deletions,
    })
}

fn member_value(p: &dove_store::Persona) -> Json {
    json!({
        "id": p.id, "name": p.name, "role": p.role, "avatar": p.avatar, "hue": p.hue,
        "harness": p.harness.map(|h| h.id()).unwrap_or(""), "instructions": p.instructions,
    })
}

/// JSON, read off the window's thread, as the scripts' values.
pub(crate) fn to_value(j: &Json) -> Value {
    match j {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::from(*b),
        Json::Number(n) => Value::from(n.as_f64().unwrap_or(0.)),
        Json::String(s) => Value::from(s.as_str()),
        Json::Array(a) => Value::List(a.iter().map(to_value).collect()),
        Json::Object(o) => Value::record(o.iter().map(|(k, v)| (k.as_str(), to_value(v)))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_refuses_climbing_out() {
        let root = Path::new("C:/p");
        assert!(inside(root, "src/main.rs").is_ok());
        assert!(inside(root, "../secret").is_err());
        assert!(inside(root, "src/../../x").is_err());
    }

    #[test]
    fn languages_by_extension() {
        assert_eq!(lang_of(Path::new("a/b.rs")), "rust");
        assert_eq!(lang_of(Path::new("ui/app.luau")), "luau");
        assert_eq!(lang_of(Path::new("README")), "");
    }
}
