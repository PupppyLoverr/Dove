//! What Keplorayx had before it moved onto Zeron's engine, brought across
//! on its first launch: your projects (the old app's database, read only,
//! never changed). ui/kx/import.luau registers each with the engine (a
//! space on this computer) unless one is already there, once.
//!
//! The old app's threads stay where they are, readable in it
//! (`keplorayx-classic`): Zeron's engine keeps conversations in documents of
//! its own that nothing outside it writes.

use std::path::PathBuf;

use zgx_script::{ScriptHost, Value};

/// The old app's database: `DOVE_HOME`'s, else Keplorayx's development
/// folder's, else Dove's.
fn old_database() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = std::env::var_os("KX_OLD_HOME").or_else(|| std::env::var_os("DOVE_HOME")) {
        candidates.push(PathBuf::from(home).join("dove.db"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("keplorayx-zgx-dev").join("dove.db"));
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        candidates.push(PathBuf::from(home).join(".dove").join("dove.db"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// The old app's projects: { name, path } each, the folders that still exist.
fn old_projects() -> Result<Vec<(String, String)>, String> {
    let Some(db) = old_database() else { return Ok(Vec::new()) };
    let conn = rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare("SELECT name, path FROM projects ORDER BY created_at").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().filter(|(_, path)| std::path::Path::new(path).is_dir()).collect())
}

fn marker() -> PathBuf {
    crate::kx_account::data_dir().join("imported-from-keplorayx")
}

pub fn install(host: &ScriptHost) {
    // `kx.import.projects`: those to bring across ({} once it's been done).
    host.async_action("kx.import.projects", |_| async move {
        if marker().is_file() {
            return Ok(Value::List(Vec::new().into()));
        }
        let list = super::off_thread(old_projects).await.unwrap_or_else(|| Err("failed".into()))?;
        let git = |path: &str| std::path::Path::new(path).join(".git").exists();
        Ok(Value::List(
            list.into_iter()
                .map(|(name, path)| {
                    Value::record([
                        ("name", Value::from(name.as_str())),
                        ("path", Value::from(path.as_str())),
                        ("git", Value::from(git(&path))),
                    ])
                })
                .collect::<Vec<_>>()
                .into(),
        ))
    });
    // `kx.import.done`: brought across; never asked again.
    host.action("kx.import.done", |_| {
        let path = marker();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, "");
    });
}
