//! Local persistence.
//!
//! Metadata (projects, tasks, usage) lives in one SQLite file in WAL mode.
//! Transcripts are append-only JSONL, one file per task, so writing an event
//! is a single buffered append and a crash loses at most the last line.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use dove_proto::{AgentEvent, HarnessKind, ProjectId, TaskId, TaskStatus, Usage};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

const SCHEMA: &str = "CREATE TABLE projects (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        path TEXT NOT NULL UNIQUE,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE tasks (
        id TEXT PRIMARY KEY,
        project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        title TEXT NOT NULL,
        harness TEXT NOT NULL,
        model TEXT,
        status TEXT NOT NULL,
        worktree TEXT,
        branch TEXT,
        native_session_id TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX tasks_by_project ON tasks(project_id, updated_at DESC);
    CREATE TABLE usage (
        id INTEGER PRIMARY KEY,
        task_id TEXT NOT NULL,
        project_id TEXT NOT NULL,
        harness TEXT NOT NULL,
        model TEXT,
        input_tokens INTEGER NOT NULL,
        output_tokens INTEGER NOT NULL,
        cache_read_tokens INTEGER NOT NULL,
        cache_write_tokens INTEGER NOT NULL,
        cost_usd REAL,
        estimated INTEGER NOT NULL,
        at INTEGER NOT NULL
    );
    CREATE INDEX usage_by_task ON usage(task_id);
    CREATE INDEX usage_by_time ON usage(at);
    CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);";

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub harness: HarnessKind,
    pub model: Option<String>,
    pub status: TaskStatus,
    pub worktree: Option<PathBuf>,
    pub branch: Option<String>,
    pub native_session_id: Option<String>,
    /// Commit a worktree task started from; its diff is taken against this.
    pub base: Option<String>,
    pub pinned: bool,
    pub updated_at: i64,
    /// A draft's prompt (Kanban), sent when the draft starts.
    pub prompt: Option<String>,
    /// Finished or failed since the user last looked.
    pub unread: bool,
    /// The automation that created this thread, if any.
    pub automation: Option<String>,
}

/// A scheduled or event-triggered prompt that starts threads by itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Automation {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub project_id: ProjectId,
    pub harness: HarnessKind,
    pub model: Option<String>,
    /// JSON trigger, interpreted by dove-core's scheduler.
    pub trigger: String,
    pub worktree: bool,
    pub enabled: bool,
    pub last_run: Option<i64>,
    /// Trigger state (last seen commit, PR numbers), as JSON.
    pub state: String,
}

/// One automation run: the thread it started.
#[derive(Debug, Clone, PartialEq)]
pub struct AutomationRun {
    pub automation_id: String,
    pub task_id: TaskId,
    pub at: i64,
    pub unread: bool,
}

/// A teammate: a named persona one agent harness runs under for every
/// thread it is given. Its instructions lead each new thread; its notes file
/// carries what it learned from one thread to the next.
#[derive(Debug, Clone, PartialEq)]
pub struct Persona {
    pub id: String,
    pub name: String,
    /// What it does on the team ("Frontend lead", "Reviewer").
    pub role: String,
    /// One emoji or a single letter, shown on a tinted tile.
    pub avatar: String,
    /// Tile hue, 0..1.
    pub hue: f32,
    pub harness: HarnessKind,
    pub model: Option<String>,
    /// How it works: tone, standards, what to always or never do.
    pub instructions: String,
    pub created_at: i64,
}

pub struct Store {
    conn: Mutex<Connection>,
    root: PathBuf,
    /// Every setting, loaded once at open and written through. The UI reads
    /// settings while it renders; this keeps those reads off SQLite and off
    /// the connection lock the engine's writers hold.
    settings: parking_lot::RwLock<std::collections::HashMap<String, String>>,
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for name in names {
        if name? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Creates the schema, then adds columns introduced after the first release.
/// Idempotent, including databases written by the old migration crate.
fn migrate(conn: &Connection) -> Result<()> {
    let projects: bool = conn
        .query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'projects'", [], |_| Ok(true))
        .optional()?
        .unwrap_or(false);
    if !projects {
        conn.execute_batch(SCHEMA)?;
    }
    if !has_column(conn, "tasks", "pinned")? {
        conn.execute_batch(
            "ALTER TABLE tasks ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE tasks ADD COLUMN base TEXT;",
        )?;
    }
    if !has_column(conn, "tasks", "unread")? {
        conn.execute_batch(
            "ALTER TABLE tasks ADD COLUMN prompt TEXT;
             ALTER TABLE tasks ADD COLUMN unread INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE tasks ADD COLUMN automation TEXT;
             CREATE TABLE IF NOT EXISTS automations (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 prompt TEXT NOT NULL,
                 project_id TEXT NOT NULL,
                 harness TEXT NOT NULL,
                 model TEXT,
                 trigger TEXT NOT NULL,
                 worktree INTEGER NOT NULL DEFAULT 1,
                 enabled INTEGER NOT NULL DEFAULT 1,
                 last_run INTEGER,
                 state TEXT NOT NULL DEFAULT '{}',
                 created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS automation_runs (
                 automation_id TEXT NOT NULL,
                 task_id TEXT NOT NULL,
                 at INTEGER NOT NULL,
                 unread INTEGER NOT NULL DEFAULT 1
             );
             CREATE INDEX IF NOT EXISTS runs_by_time ON automation_runs(at DESC);",
        )?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS personas (
             id TEXT PRIMARY KEY,
             name TEXT NOT NULL,
             role TEXT NOT NULL DEFAULT '',
             avatar TEXT NOT NULL DEFAULT '',
             hue REAL NOT NULL DEFAULT 0.6,
             harness TEXT NOT NULL,
             model TEXT,
             instructions TEXT NOT NULL DEFAULT '',
             created_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS persona_tasks (
             task_id TEXT PRIMARY KEY,
             persona_id TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS persona_tasks_by_persona ON persona_tasks(persona_id);",
    )?;
    Ok(())
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// `~/.dove`, or `DOVE_HOME` when set (tests, portable installs).
pub fn default_root() -> PathBuf {
    if let Some(p) = std::env::var_os("DOVE_HOME") {
        return p.into();
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| ".".into());
    home.join(".dove")
}

impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root.join("sessions")).with_context(|| format!("creating {}", root.display()))?;
        let conn = Connection::open(root.join("dove.db"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&conn)?;
        let settings = {
            let mut st = conn.prepare("SELECT key, value FROM settings")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            rows.filter_map(|r| r.ok()).collect()
        };
        Ok(Self { conn: Mutex::new(conn), root: root.to_path_buf(), settings: parking_lot::RwLock::new(settings) })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    // ---- projects ----

    /// Add a project, or return the existing one for the same path.
    pub fn add_project(&self, path: &Path) -> Result<Project> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let path = strip_verbatim(&path);
        let key = path.to_string_lossy().to_string();
        let conn = self.conn.lock();
        if let Some(p) =
            conn.query_row("SELECT id, name, path FROM projects WHERE path = ?1", [&key], row_project).optional()?
        {
            return Ok(p);
        }
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| key.clone());
        let id = ProjectId::new();
        conn.execute(
            "INSERT INTO projects (id, name, path, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id.0.to_string(), name, key, now_ms()],
        )?;
        Ok(Project { id, name, path })
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        let conn = self.conn.lock();
        let mut st = conn.prepare_cached("SELECT id, name, path FROM projects ORDER BY name COLLATE NOCASE")?;
        let rows = st.query_map([], row_project)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn remove_project(&self, id: ProjectId) -> Result<()> {
        self.conn.lock().execute("DELETE FROM projects WHERE id = ?1", [id.0.to_string()])?;
        Ok(())
    }

    // ---- tasks ----

    pub fn insert_task(&self, t: &Task) -> Result<()> {
        let now = now_ms();
        self.conn.lock().execute(
            "INSERT INTO tasks (id, project_id, title, harness, model, status, worktree, branch,
                                native_session_id, base, pinned, created_at, updated_at, prompt, unread, automation)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?11, ?12, ?10, ?10, ?13, ?14, ?15)",
            params![
                t.id.0.to_string(),
                t.project_id.0.to_string(),
                t.title,
                t.harness.id(),
                t.model,
                t.status.id(),
                t.worktree.as_ref().map(|p| p.to_string_lossy().to_string()),
                t.branch,
                t.native_session_id,
                now,
                t.base,
                t.pinned,
                t.prompt,
                t.unread,
                t.automation
            ],
        )?;
        Ok(())
    }

    /// Rewrite a task row (a draft that started: its worktree, branch, status).
    pub fn update_task(&self, t: &Task) -> Result<()> {
        self.conn.lock().execute(
            "UPDATE tasks SET title = ?2, harness = ?3, model = ?4, status = ?5, worktree = ?6, branch = ?7,
                              native_session_id = ?8, base = ?9, pinned = ?10, prompt = ?11, updated_at = ?12
             WHERE id = ?1",
            params![
                t.id.0.to_string(),
                t.title,
                t.harness.id(),
                t.model,
                t.status.id(),
                t.worktree.as_ref().map(|p| p.to_string_lossy().to_string()),
                t.branch,
                t.native_session_id,
                t.base,
                t.pinned,
                t.prompt,
                now_ms()
            ],
        )?;
        Ok(())
    }

    pub fn set_unread(&self, id: TaskId, unread: bool) -> Result<()> {
        self.conn.lock().execute("UPDATE tasks SET unread = ?2 WHERE id = ?1", params![id.0.to_string(), unread])?;
        Ok(())
    }

    pub fn set_prompt(&self, id: TaskId, prompt: &str) -> Result<()> {
        self.conn.lock().execute("UPDATE tasks SET prompt = ?2 WHERE id = ?1", params![id.0.to_string(), prompt])?;
        Ok(())
    }

    pub fn tasks(&self, project: ProjectId) -> Result<Vec<Task>> {
        self.query_tasks(
            "WHERE project_id = ?1 AND status != 'archived' ORDER BY pinned DESC, updated_at DESC",
            [project.0.to_string()],
        )
    }

    /// Every live task across projects, pinned first, most recent next.
    pub fn all_tasks(&self) -> Result<Vec<Task>> {
        self.query_tasks("WHERE status != 'archived' ORDER BY pinned DESC, updated_at DESC", params![])
    }

    pub fn archived_tasks(&self, limit: usize) -> Result<Vec<Task>> {
        self.query_tasks("WHERE status = 'archived' ORDER BY updated_at DESC LIMIT ?1", [limit as i64])
    }

    fn query_tasks<P: rusqlite::Params>(&self, filter: &str, p: P) -> Result<Vec<Task>> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT id, project_id, title, harness, model, status, worktree, branch, native_session_id, updated_at, base, pinned,
                    prompt, unread, automation
             FROM tasks {filter}"
        );
        let mut st = conn.prepare_cached(&sql)?;
        let rows = st.query_map(p, row_task)?;
        Ok(rows.filter_map(|r| r.ok()).flatten().collect())
    }

    pub fn task(&self, id: TaskId) -> Result<Option<Task>> {
        Ok(self.query_tasks("WHERE id = ?1", [id.0.to_string()])?.into_iter().next())
    }

    pub fn set_pinned(&self, id: TaskId, pinned: bool) -> Result<()> {
        self.conn.lock().execute("UPDATE tasks SET pinned = ?2 WHERE id = ?1", params![id.0.to_string(), pinned])?;
        Ok(())
    }

    /// Remove a task, its usage rows and its transcript. Worktree cleanup is
    /// the caller's job (it needs git).
    pub fn delete_task(&self, id: TaskId) -> Result<()> {
        {
            let conn = self.conn.lock();
            conn.execute("DELETE FROM usage WHERE task_id = ?1", [id.0.to_string()])?;
            conn.execute("DELETE FROM tasks WHERE id = ?1", [id.0.to_string()])?;
        }
        let _ = std::fs::remove_file(self.transcript_path(id));
        Ok(())
    }

    pub fn set_status(&self, id: TaskId, status: TaskStatus) -> Result<()> {
        self.conn.lock().execute(
            "UPDATE tasks SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![id.0.to_string(), status.id(), now_ms()],
        )?;
        Ok(())
    }

    pub fn set_native_session(&self, id: TaskId, native: &str) -> Result<()> {
        self.conn
            .lock()
            .execute("UPDATE tasks SET native_session_id = ?2 WHERE id = ?1", params![id.0.to_string(), native])?;
        Ok(())
    }

    pub fn rename_task(&self, id: TaskId, title: &str) -> Result<()> {
        self.conn.lock().execute(
            "UPDATE tasks SET title = ?2, updated_at = ?3 WHERE id = ?1",
            params![id.0.to_string(), title, now_ms()],
        )?;
        Ok(())
    }

    // ---- usage ledger ----

    pub fn record_usage(
        &self,
        task: TaskId,
        project: ProjectId,
        harness: HarnessKind,
        model: Option<&str>,
        u: &Usage,
    ) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO usage (task_id, project_id, harness, model, input_tokens, output_tokens,
                                cache_read_tokens, cache_write_tokens, cost_usd, estimated, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                task.0.to_string(),
                project.0.to_string(),
                harness.id(),
                model,
                u.input_tokens as i64,
                u.output_tokens as i64,
                u.cache_read_tokens as i64,
                u.cache_write_tokens as i64,
                u.cost_usd,
                u.estimated,
                now_ms()
            ],
        )?;
        Ok(())
    }

    pub fn task_usage(&self, task: TaskId) -> Result<Usage> {
        self.sum_usage("WHERE task_id = ?1", [task.0.to_string()])
    }

    /// Totals since `since_ms`, grouped by harness id.
    pub fn usage_by_harness(&self, since_ms: i64) -> Result<Vec<(String, Usage)>> {
        let conn = self.conn.lock();
        let mut st = conn.prepare_cached(
            "SELECT harness, SUM(input_tokens), SUM(output_tokens), SUM(cache_read_tokens),
                    SUM(cache_write_tokens), SUM(cost_usd), MAX(estimated)
             FROM usage WHERE at >= ?1 GROUP BY harness ORDER BY SUM(cost_usd) DESC",
        )?;
        let rows = st.query_map([since_ms], |r| Ok((r.get::<_, String>(0)?, row_usage(r, 1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn sum_usage<P: rusqlite::Params>(&self, filter: &str, p: P) -> Result<Usage> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT SUM(input_tokens), SUM(output_tokens), SUM(cache_read_tokens),
                    SUM(cache_write_tokens), SUM(cost_usd), MAX(estimated) FROM usage {filter}"
        );
        Ok(conn.query_row(&sql, p, |r| row_usage(r, 0))?)
    }

    /// Usage per UTC day, harness and project, newest first.
    pub fn usage_rows(&self, since_ms: i64) -> Result<Vec<UsageRow>> {
        let conn = self.conn.lock();
        let mut st = conn.prepare_cached(
            "SELECT at / 86400000, harness, project_id, SUM(input_tokens), SUM(output_tokens), SUM(cache_read_tokens),
                    SUM(cache_write_tokens), SUM(cost_usd), MAX(estimated)
             FROM usage WHERE at >= ?1 GROUP BY at / 86400000, harness, project_id ORDER BY 1 DESC",
        )?;
        let rows = st.query_map([since_ms], |r| {
            Ok(UsageRow {
                day: r.get::<_, i64>(0)? * 86_400_000,
                harness: r.get(1)?,
                project_id: ProjectId(parse_uuid(r.get(2)?)),
                usage: row_usage(r, 3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---- automations ----

    pub fn save_automation(&self, a: &Automation) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO automations (id, name, prompt, project_id, harness, model, trigger, worktree, enabled,
                                      last_run, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, prompt = excluded.prompt,
                project_id = excluded.project_id, harness = excluded.harness, model = excluded.model,
                trigger = excluded.trigger, worktree = excluded.worktree, enabled = excluded.enabled,
                last_run = excluded.last_run, state = excluded.state",
            params![
                a.id,
                a.name,
                a.prompt,
                a.project_id.0.to_string(),
                a.harness.id(),
                a.model,
                a.trigger,
                a.worktree,
                a.enabled,
                a.last_run,
                a.state,
                now_ms()
            ],
        )?;
        Ok(())
    }

    pub fn automations(&self) -> Result<Vec<Automation>> {
        let conn = self.conn.lock();
        let mut st = conn.prepare_cached(
            "SELECT id, name, prompt, project_id, harness, model, trigger, worktree, enabled, last_run, state
             FROM automations ORDER BY created_at",
        )?;
        let rows = st.query_map([], row_automation)?;
        Ok(rows.filter_map(|r| r.ok()).flatten().collect())
    }

    pub fn delete_automation(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM automations WHERE id = ?1", [id])?;
        conn.execute("DELETE FROM automation_runs WHERE automation_id = ?1", [id])?;
        Ok(())
    }

    pub fn record_run(&self, automation: &str, task: TaskId) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO automation_runs (automation_id, task_id, at, unread) VALUES (?1, ?2, ?3, 1)",
            params![automation, task.0.to_string(), now_ms()],
        )?;
        Ok(())
    }

    pub fn runs(&self, limit: usize) -> Result<Vec<AutomationRun>> {
        let conn = self.conn.lock();
        let mut st = conn.prepare_cached(
            "SELECT automation_id, task_id, at, unread FROM automation_runs ORDER BY at DESC LIMIT ?1",
        )?;
        let rows = st.query_map([limit as i64], |r| {
            Ok(AutomationRun {
                automation_id: r.get(0)?,
                task_id: TaskId(parse_uuid(r.get(1)?)),
                at: r.get(2)?,
                unread: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn mark_run_read(&self, task: TaskId) -> Result<()> {
        self.conn.lock().execute("UPDATE automation_runs SET unread = 0 WHERE task_id = ?1", [task.0.to_string()])?;
        Ok(())
    }

    // ---- team ----

    pub fn save_persona(&self, p: &Persona) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO personas (id, name, role, avatar, hue, harness, model, instructions, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, role = excluded.role, avatar = excluded.avatar,
                hue = excluded.hue, harness = excluded.harness, model = excluded.model,
                instructions = excluded.instructions",
            params![
                p.id,
                p.name,
                p.role,
                p.avatar,
                p.hue as f64,
                p.harness.id(),
                p.model,
                p.instructions,
                p.created_at
            ],
        )?;
        Ok(())
    }

    pub fn personas(&self) -> Result<Vec<Persona>> {
        let conn = self.conn.lock();
        let mut st = conn.prepare_cached(
            "SELECT id, name, role, avatar, hue, harness, model, instructions, created_at
             FROM personas ORDER BY created_at",
        )?;
        let rows = st.query_map([], |r| {
            let Some(harness) = HarnessKind::from_id(&r.get::<_, String>(5)?) else { return Ok(None) };
            Ok(Some(Persona {
                id: r.get(0)?,
                name: r.get(1)?,
                role: r.get(2)?,
                avatar: r.get(3)?,
                hue: r.get::<_, f64>(4)? as f32,
                harness,
                model: r.get(6)?,
                instructions: r.get(7)?,
                created_at: r.get(8)?,
            }))
        })?;
        Ok(rows.filter_map(|r| r.ok()).flatten().collect())
    }

    /// Remove a teammate. Its threads stay, unassigned.
    pub fn delete_persona(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM personas WHERE id = ?1", [id])?;
        conn.execute("DELETE FROM persona_tasks WHERE persona_id = ?1", [id])?;
        Ok(())
    }

    pub fn assign_persona(&self, task: TaskId, persona: &str) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO persona_tasks (task_id, persona_id) VALUES (?1, ?2)
             ON CONFLICT(task_id) DO UPDATE SET persona_id = excluded.persona_id",
            [task.0.to_string(), persona.to_string()],
        )?;
        Ok(())
    }

    /// Which teammate runs each thread, as (task, persona).
    pub fn persona_assignments(&self) -> Result<Vec<(TaskId, String)>> {
        let conn = self.conn.lock();
        let mut st = conn.prepare_cached("SELECT task_id, persona_id FROM persona_tasks")?;
        let rows = st.query_map([], |r| Ok((TaskId(parse_uuid(r.get(0)?)), r.get::<_, String>(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A teammate's working notes, carried between its threads.
    pub fn persona_notes_path(&self, id: &str) -> PathBuf {
        self.root.join("team").join(id).join("NOTES.md")
    }

    // ---- settings ----

    pub fn setting(&self, key: &str) -> Option<String> {
        self.settings.read().get(key).cloned()
    }

    /// Whether `key` is set to `"true"` (`default` when unset).
    pub fn flag(&self, key: &str, default: bool) -> bool {
        match self.settings.read().get(key).map(String::as_str) {
            Some("true") => true,
            Some("false") => false,
            _ => default,
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        if self.settings.read().get(key).is_some_and(|v| v == value) {
            return Ok(());
        }
        self.conn.lock().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        self.settings.write().insert(key.to_string(), value.to_string());
        Ok(())
    }

    /// Every setting whose key starts with `prefix`, sorted by key.
    pub fn settings_with_prefix(&self, prefix: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .settings
            .read()
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        out.sort();
        out
    }

    // ---- transcripts ----

    pub fn transcript_path(&self, task: TaskId) -> PathBuf {
        self.root.join("sessions").join(format!("{task}.jsonl"))
    }

    pub fn transcript_writer(&self, task: TaskId) -> Result<TranscriptWriter> {
        let f = OpenOptions::new().create(true).append(true).open(self.transcript_path(task))?;
        Ok(TranscriptWriter { out: BufWriter::new(f) })
    }

    /// Every event recorded for `task`. Lines that fail to parse (a torn final
    /// write, a format from a newer build) are skipped, not fatal.
    pub fn read_transcript(&self, task: TaskId) -> Vec<AgentEvent> {
        let Ok(f) = File::open(self.transcript_path(task)) else { return Vec::new() };
        BufReader::new(f).lines().map_while(Result::ok).filter_map(|l| serde_json::from_str(&l).ok()).collect()
    }
}

/// One bucket of the usage history.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageRow {
    /// UTC midnight, in ms.
    pub day: i64,
    pub harness: String,
    pub project_id: ProjectId,
    pub usage: Usage,
}

pub struct TranscriptWriter {
    out: BufWriter<File>,
}

impl TranscriptWriter {
    pub fn append(&mut self, e: &AgentEvent) -> Result<()> {
        serde_json::to_writer(&mut self.out, e)?;
        self.out.write_all(b"\n")?;
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        self.out.flush()?;
        Ok(())
    }
}

impl Drop for TranscriptWriter {
    fn drop(&mut self) {
        let _ = self.out.flush();
    }
}

fn parse_uuid(s: String) -> Uuid {
    Uuid::parse_str(&s).unwrap_or_default()
}

fn row_project(r: &rusqlite::Row) -> rusqlite::Result<Project> {
    Ok(Project { id: ProjectId(parse_uuid(r.get(0)?)), name: r.get(1)?, path: PathBuf::from(r.get::<_, String>(2)?) })
}

fn row_task(r: &rusqlite::Row) -> rusqlite::Result<Option<Task>> {
    let Some(harness) = HarnessKind::from_id(&r.get::<_, String>(3)?) else { return Ok(None) };
    Ok(Some(Task {
        id: TaskId(parse_uuid(r.get(0)?)),
        project_id: ProjectId(parse_uuid(r.get(1)?)),
        title: r.get(2)?,
        harness,
        model: r.get(4)?,
        status: TaskStatus::from_id(&r.get::<_, String>(5)?),
        worktree: r.get::<_, Option<String>>(6)?.map(PathBuf::from),
        branch: r.get(7)?,
        native_session_id: r.get(8)?,
        updated_at: r.get(9)?,
        base: r.get(10)?,
        pinned: r.get::<_, Option<bool>>(11)?.unwrap_or(false),
        prompt: r.get(12)?,
        unread: r.get::<_, Option<bool>>(13)?.unwrap_or(false),
        automation: r.get(14)?,
    }))
}

fn row_automation(r: &rusqlite::Row) -> rusqlite::Result<Option<Automation>> {
    let Some(harness) = HarnessKind::from_id(&r.get::<_, String>(4)?) else { return Ok(None) };
    Ok(Some(Automation {
        id: r.get(0)?,
        name: r.get(1)?,
        prompt: r.get(2)?,
        project_id: ProjectId(parse_uuid(r.get(3)?)),
        harness,
        model: r.get(5)?,
        trigger: r.get(6)?,
        worktree: r.get(7)?,
        enabled: r.get(8)?,
        last_run: r.get(9)?,
        state: r.get(10)?,
    }))
}

fn row_usage(r: &rusqlite::Row, o: usize) -> rusqlite::Result<Usage> {
    let n = |i: usize| -> rusqlite::Result<u64> { Ok(r.get::<_, Option<i64>>(o + i)?.unwrap_or(0) as u64) };
    Ok(Usage {
        input_tokens: n(0)?,
        output_tokens: n(1)?,
        cache_read_tokens: n(2)?,
        cache_write_tokens: n(3)?,
        cost_usd: r.get(o + 4)?,
        estimated: r.get::<_, Option<bool>>(o + 5)?.unwrap_or(false),
    })
}

/// `\\?\C:\x` → `C:\x`, so stored paths are what users and git expect.
fn strip_verbatim(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => p.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(project: ProjectId) -> Task {
        Task {
            id: TaskId::new(),
            project_id: project,
            title: "Fix the thing".into(),
            harness: HarnessKind::ClaudeCode,
            model: None,
            status: TaskStatus::Idle,
            worktree: None,
            branch: Some("dove/fix".into()),
            native_session_id: None,
            base: Some("abc123".into()),
            pinned: false,
            updated_at: 0,
            prompt: None,
            unread: false,
            automation: None,
        }
    }

    #[test]
    fn team_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path()).unwrap();
        let mut ava = Persona {
            id: "ava".into(),
            name: "Ava".into(),
            role: "Frontend lead".into(),
            avatar: "🦊".into(),
            hue: 0.08,
            harness: HarnessKind::ClaudeCode,
            model: Some("opus".into()),
            instructions: "Ship small, tested changes.".into(),
            created_at: 1,
        };
        s.save_persona(&ava).unwrap();
        ava.role = "Design engineer".into();
        s.save_persona(&ava).unwrap();
        assert_eq!(s.personas().unwrap(), vec![ava.clone()]);
        let t = TaskId::new();
        s.assign_persona(t, "ava").unwrap();
        assert_eq!(s.persona_assignments().unwrap(), vec![(t, "ava".to_string())]);
        s.delete_persona("ava").unwrap();
        assert!(s.personas().unwrap().is_empty());
        assert!(s.persona_assignments().unwrap().is_empty());
    }

    #[test]
    fn drafts_unread_and_automations() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path()).unwrap();
        let p = s.add_project(dir.path()).unwrap();
        let mut t = task(p.id);
        t.status = TaskStatus::Draft;
        t.prompt = Some("do it".into());
        s.insert_task(&t).unwrap();
        let got = s.task(t.id).unwrap().unwrap();
        assert_eq!((got.status, got.prompt.as_deref()), (TaskStatus::Draft, Some("do it")));
        s.set_unread(t.id, true).unwrap();
        assert!(s.task(t.id).unwrap().unwrap().unread);
        t.status = TaskStatus::Running;
        t.worktree = Some("/w".into());
        s.update_task(&t).unwrap();
        assert_eq!(s.task(t.id).unwrap().unwrap().worktree.as_deref(), Some(Path::new("/w")));

        let a = Automation {
            id: "a1".into(),
            name: "Nightly".into(),
            prompt: "check".into(),
            project_id: p.id,
            harness: HarnessKind::Codex,
            model: None,
            trigger: "{}".into(),
            worktree: true,
            enabled: true,
            last_run: None,
            state: "{}".into(),
        };
        s.save_automation(&a).unwrap();
        s.save_automation(&Automation { enabled: false, ..a.clone() }).unwrap();
        assert!(!s.automations().unwrap()[0].enabled);
        s.record_run("a1", t.id).unwrap();
        assert!(s.runs(10).unwrap()[0].unread);
        s.mark_run_read(t.id).unwrap();
        assert!(!s.runs(10).unwrap()[0].unread);
        s.delete_automation("a1").unwrap();
        assert!(s.automations().unwrap().is_empty() && s.runs(10).unwrap().is_empty());

        s.record_usage(t.id, p.id, t.harness, None, &Usage { output_tokens: 3, ..Default::default() }).unwrap();
        let rows = s.usage_rows(0).unwrap();
        assert_eq!((rows.len(), rows[0].usage.output_tokens), (1, 3));
    }

    #[test]
    fn projects_are_deduplicated_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path()).unwrap();
        let a = s.add_project(dir.path()).unwrap();
        let b = s.add_project(dir.path()).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(s.projects().unwrap().len(), 1);
    }

    #[test]
    fn task_lifecycle_and_usage() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path()).unwrap();
        let p = s.add_project(dir.path()).unwrap();
        let t = task(p.id);
        s.insert_task(&t).unwrap();
        s.set_status(t.id, TaskStatus::Running).unwrap();
        s.set_native_session(t.id, "abc").unwrap();
        let got = &s.tasks(p.id).unwrap()[0];
        assert_eq!(got.status, TaskStatus::Running);
        assert_eq!(got.native_session_id.as_deref(), Some("abc"));

        let u = Usage { input_tokens: 5, output_tokens: 7, cost_usd: Some(0.5), ..Default::default() };
        s.record_usage(t.id, p.id, t.harness, None, &u).unwrap();
        s.record_usage(t.id, p.id, t.harness, None, &u).unwrap();
        let total = s.task_usage(t.id).unwrap();
        assert_eq!(total.output_tokens, 14);
        assert_eq!(total.cost_usd, Some(1.0));
        assert_eq!(s.usage_by_harness(0).unwrap()[0].0, "claude_code");

        s.set_pinned(t.id, true).unwrap();
        assert!(s.task(t.id).unwrap().unwrap().pinned);
        assert_eq!(s.task(t.id).unwrap().unwrap().base.as_deref(), Some("abc123"));

        s.set_status(t.id, TaskStatus::Archived).unwrap();
        assert!(s.tasks(p.id).unwrap().is_empty());
        assert_eq!(s.archived_tasks(10).unwrap().len(), 1);

        s.delete_task(t.id).unwrap();
        assert!(s.task(t.id).unwrap().is_none());
        assert_eq!(s.task_usage(t.id).unwrap().output_tokens, 0);
    }

    #[test]
    fn transcript_round_trip_tolerates_torn_line() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path()).unwrap();
        let id = TaskId::new();
        {
            let mut w = s.transcript_writer(id).unwrap();
            w.append(&AgentEvent::UserMessage { text: "hi".into() }).unwrap();
            w.append(&AgentEvent::TextDelta { text: "yo".into() }).unwrap();
        }
        std::fs::OpenOptions::new()
            .append(true)
            .open(s.transcript_path(id))
            .unwrap()
            .write_all(b"{\"type\":\"text_de")
            .unwrap();
        assert_eq!(s.read_transcript(id).len(), 2);
    }

    #[test]
    fn settings() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path()).unwrap();
        assert_eq!(s.setting("theme"), None);
        s.set_setting("theme", "dark").unwrap();
        s.set_setting("theme", "light").unwrap();
        assert_eq!(s.setting("theme").as_deref(), Some("light"));
        assert!(s.flag("missing", true));
        s.set_setting("on", "true").unwrap();
        assert!(s.flag("on", false));
        drop(s);
        // The cache is loaded from disk on open.
        let s = Store::open(dir.path()).unwrap();
        assert_eq!(s.setting("theme").as_deref(), Some("light"));
        assert_eq!(s.settings_with_prefix("t"), vec![("theme".to_string(), "light".to_string())]);
    }
}
