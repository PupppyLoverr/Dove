//! Artifacts: the files agents made in chats that have no project. Such a
//! chat works in a folder of its own under `<data>/scratch/` (the engine
//! makes one per chat), so what it writes there is nobody's repository's:
//! a document, a picture, a page, a script. The Artifacts page
//! (ui/kx/pages.luau) lists them, newest first, as the old Keplorayx did.
//!
//!   kx.artifacts.list   → [{ name, path, folder, rel, kind, size, modified }]
//!   kx.artifacts.open   (path) in what the system opens it with
//!
//! Only paths under the scratch folder are opened, whatever is asked.

use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};
use zgx_script::ScriptHost;

/// How deep a chat's folder is read, and how many files are listed at most.
const DEPTH: usize = 4;
const MOST: usize = 400;

fn root() -> PathBuf {
    crate::kx_account::data_dir().join("scratch")
}

/// What a file is, by its name: how the page groups and draws it.
fn kind(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "md" | "markdown" | "txt" | "pdf" | "doc" | "docx" | "rtf" | "odt" | "pptx" | "ppt" => "document",
        "html" | "htm" => "page",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif" | "ico" => "image",
        "csv" | "tsv" | "json" | "jsonl" | "xlsx" | "xls" | "yaml" | "yml" | "toml" | "xml" | "sql" | "db" | "sqlite" => "data",
        "rs" | "py" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "lua" | "luau" | "go" | "c" | "h" | "cpp" | "hpp" | "java" | "kt"
        | "swift" | "rb" | "php" | "sh" | "bash" | "ps1" | "bat" | "css" | "scss" | "vue" | "svelte" | "cs" | "zig" => "code",
        _ => "other",
    }
}

/// Folders that are never a chat's own work.
fn skipped(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "node_modules" | "target" | "__pycache__" | "dist" | "build" | "venv")
}

fn walk(dir: &Path, chat: &str, base: &Path, depth: usize, out: &mut Vec<Json>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            if depth < DEPTH && !skipped(&name) {
                walk(&path, chat, base, depth + 1, out);
            }
            continue;
        }
        if !meta.is_file() || name.starts_with('.') {
            continue;
        }
        let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
        out.push(json!({
            "name": name,
            "path": path.to_string_lossy(),
            "folder": chat,
            "rel": path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/"),
            "kind": kind(&name),
            "size": meta.len(),
            "modified": modified,
        }));
    }
}

/// Every chat's files, newest first.
fn scan(root: &Path) -> Vec<Json> {
    let mut out = Vec::new();
    let Ok(chats) = std::fs::read_dir(root) else { return out };
    for chat in chats.flatten() {
        let path = chat.path();
        if path.is_dir() {
            let name = chat.file_name().to_string_lossy().into_owned();
            walk(&path, &name, &path, 1, &mut out);
        }
    }
    out.sort_by_key(|f| std::cmp::Reverse(f["modified"].as_u64().unwrap_or(0)));
    out.truncate(MOST);
    out
}

/// `path`, if it is a file under the scratch folder.
fn inside(path: &str) -> Option<PathBuf> {
    let root = root().canonicalize().ok()?;
    let path = Path::new(path).canonicalize().ok()?;
    path.starts_with(&root).then_some(path)
}

pub fn install(host: &ScriptHost) {
    host.async_action("kx.artifacts.list", |_| async move {
        let found = crate::kx::off_thread(|| scan(&root())).await.unwrap_or_default();
        Ok(crate::kx::to_value(&Json::Array(found)))
    });
    host.action("kx.artifacts.open", |path| {
        if let Some(path) = inside(&crate::auth::text(&path)) {
            crate::kx::open_external(&path.to_string_lossy());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_name() {
        assert_eq!(kind("Report.MD"), "document");
        assert_eq!(kind("index.html"), "page");
        assert_eq!(kind("chart.png"), "image");
        assert_eq!(kind("rows.csv"), "data");
        assert_eq!(kind("main.rs"), "code");
        assert_eq!(kind("Makefile"), "other");
    }

    #[test]
    fn lists_a_chats_files_newest_first_and_skips_what_isnt_its_work() {
        let dir = tempfile::tempdir().unwrap();
        let chat = dir.path().join("2026-11-12-abc");
        std::fs::create_dir_all(chat.join("out")).unwrap();
        std::fs::create_dir_all(chat.join("node_modules/x")).unwrap();
        std::fs::write(chat.join("notes.md"), "a").unwrap();
        std::fs::write(chat.join("out/page.html"), "b").unwrap();
        std::fs::write(chat.join("node_modules/x/index.js"), "c").unwrap();
        std::fs::write(chat.join(".hidden"), "d").unwrap();
        let found = scan(dir.path());
        let mut names: Vec<&str> = found.iter().filter_map(|f| f["rel"].as_str()).collect();
        names.sort();
        assert_eq!(names, ["notes.md", "out/page.html"]);
        assert!(found.iter().all(|f| f["folder"] == "2026-11-12-abc"));
    }
}
