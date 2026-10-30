//! Full-text search over every transcript (Hermes's `hermes_state_fts`).
//!
//! What a person would look for is indexed: their own messages, the agent's
//! replies (one row per text block) and tool-call titles. Rows live in
//! `search_rows`; an external-content FTS5 table over them uses the `trigram`
//! tokenizer, so substrings match in any script (CJK included) without a
//! language-aware tokenizer. Queries of one or two characters, which trigrams
//! cannot answer, fall back to a `LIKE` scan.
//!
//! Live sessions are indexed as the engine records them ([`LiveIndexer`]).
//! Transcripts written before the index existed are read once, in the
//! background, the first time someone searches ([`Store::backfill_search`]);
//! `search_done` lists the threads already covered.

use std::io::{BufRead, BufReader};
use std::ops::Range;

use anyhow::Result;
use dove_proto::{AgentEvent, TaskId};
use rusqlite::{Connection, params};

use crate::{Store, now_ms, parse_uuid};

pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS search_rows (
        id INTEGER PRIMARY KEY,
        task TEXT NOT NULL,
        kind INTEGER NOT NULL,
        ts INTEGER NOT NULL,
        text TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS search_rows_by_task ON search_rows(task);
    CREATE VIRTUAL TABLE IF NOT EXISTS search_fts USING fts5(
        text, content='search_rows', content_rowid='id', tokenize='trigram'
    );
    CREATE TRIGGER IF NOT EXISTS search_rows_ai AFTER INSERT ON search_rows BEGIN
        INSERT INTO search_fts(rowid, text) VALUES (new.id, new.text);
    END;
    CREATE TRIGGER IF NOT EXISTS search_rows_ad AFTER DELETE ON search_rows BEGIN
        INSERT INTO search_fts(search_fts, rowid, text) VALUES ('delete', old.id, old.text);
    END;
    CREATE TABLE IF NOT EXISTS search_done (task TEXT PRIMARY KEY);";

/// One indexed row is at most this long; a longer reply is indexed by its
/// start (the transcript still has all of it).
const MAX_ROW_BYTES: usize = 32 * 1024;
/// Snippet context around a match, in characters.
const BEFORE: usize = 40;
const AFTER: usize = 110;

/// What kind of text matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitKind {
    User,
    Assistant,
    Tool,
}

impl HitKind {
    fn id(self) -> i64 {
        match self {
            Self::User => 0,
            Self::Assistant => 1,
            Self::Tool => 2,
        }
    }

    fn from_id(id: i64) -> Self {
        match id {
            0 => Self::User,
            2 => Self::Tool,
            _ => Self::Assistant,
        }
    }
}

/// One search result: the best match in one thread.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub task: TaskId,
    /// The thread's title.
    pub title: String,
    pub kind: HitKind,
    /// The matching text around the match, on one line.
    pub snippet: String,
    /// The match within `snippet` (bytes), for highlighting.
    pub matched: Option<Range<usize>>,
    /// When it was said (ms); for transcripts indexed after the fact, the
    /// thread's last activity.
    pub ts: i64,
}

/// Turns recorded events into indexable text: user messages, assistant text
/// blocks (deltas joined), and tool-call titles.
#[derive(Default)]
pub struct Extractor {
    text: String,
}

impl Extractor {
    pub fn push(&mut self, ev: &AgentEvent, out: &mut Vec<(HitKind, String)>) {
        match ev {
            AgentEvent::TextDelta { text } => {
                if self.text.len() < MAX_ROW_BYTES {
                    self.text.push_str(text);
                }
            }
            AgentEvent::UserMessage { text } => {
                self.finish(out);
                push_row(out, HitKind::User, text);
            }
            AgentEvent::ToolCall { title, .. } | AgentEvent::ToolTitle { title, .. } => {
                self.finish(out);
                push_row(out, HitKind::Tool, title);
            }
            // Thinking and subagent chatter interleave with text without
            // ending its block.
            AgentEvent::ThinkingDelta { .. } | AgentEvent::Subagent { .. } => {}
            _ => self.finish(out),
        }
    }

    /// The text block in progress, if any.
    pub fn finish(&mut self, out: &mut Vec<(HitKind, String)>) {
        if !self.text.is_empty() {
            let text = std::mem::take(&mut self.text);
            push_row(out, HitKind::Assistant, &text);
        }
    }
}

fn push_row(out: &mut Vec<(HitKind, String)>, kind: HitKind, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let mut end = text.len().min(MAX_ROW_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    out.push((kind, text[..end].to_string()));
}

/// Indexes one live session's events as they are recorded.
pub struct LiveIndexer {
    task: TaskId,
    extractor: Extractor,
    rows: Vec<(HitKind, String)>,
}

impl LiveIndexer {
    pub fn new(task: TaskId) -> Self {
        Self { task, extractor: Extractor::default(), rows: Vec::new() }
    }

    pub fn observe(&mut self, store: &Store, ev: &AgentEvent) {
        self.extractor.push(ev, &mut self.rows);
        if self.rows.is_empty() {
            return;
        }
        let rows = std::mem::take(&mut self.rows);
        if let Err(e) = store.index_rows(self.task, &rows, now_ms()) {
            tracing::debug!("search index: {e:#}");
        }
    }
}

impl Store {
    /// Add `rows` of `task` to the search index.
    pub fn index_rows(&self, task: TaskId, rows: &[(HitKind, String)], ts: i64) -> Result<()> {
        let conn = self.conn.lock();
        insert_rows(&conn, &task.to_string(), rows, ts)?;
        Ok(())
    }

    /// Whether every transcript has been read into the index this run.
    pub fn search_backfilled(&self) -> bool {
        *self.backfilled.lock()
    }

    /// Index every thread's transcript that the index does not cover yet
    /// (written before it existed). Slow on a first run: call it off the UI
    /// thread. Returns how many threads were read. Concurrent calls wait for
    /// the first and then return 0.
    pub fn backfill_search(&self) -> usize {
        let mut done = self.backfilled.lock();
        if *done {
            return 0;
        }
        let todo: Vec<(String, i64)> = {
            let conn = self.conn.lock();
            let mut st =
                match conn.prepare("SELECT id, updated_at FROM tasks WHERE id NOT IN (SELECT task FROM search_done)") {
                    Ok(st) => st,
                    Err(e) => {
                        tracing::warn!("search backfill: {e}");
                        return 0;
                    }
                };
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
        };
        let mut read = 0;
        for (id, updated) in todo {
            let task = TaskId(parse_uuid(id.clone()));
            // Read and parse outside the lock; replace the thread's rows in
            // one transaction (live rows recorded so far are in the file).
            let rows = transcript_rows(&self.transcript_path(task));
            let mut conn = self.conn.lock();
            let result = (|| -> rusqlite::Result<()> {
                let tx = conn.transaction()?;
                tx.execute("DELETE FROM search_rows WHERE task = ?1", [&id])?;
                insert_rows(&tx, &id, &rows, updated)?;
                tx.execute("INSERT OR IGNORE INTO search_done (task) VALUES (?1)", [&id])?;
                tx.commit()
            })();
            if let Err(e) = result {
                tracing::warn!("search backfill of {id}: {e}");
            }
            read += 1;
        }
        *done = true;
        read
    }

    /// Up to `limit` threads whose messages match `query`, best match first,
    /// one hit per thread. Archived threads are left out.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let terms: Vec<String> =
            query.split_whitespace().map(|t| t.trim_matches('"').to_string()).filter(|t| !t.is_empty()).collect();
        if terms.is_empty() || limit == 0 || terms.iter().map(|t| t.chars().count()).sum::<usize>() < 2 {
            return Vec::new();
        }
        let (long, short): (Vec<&String>, Vec<&String>) = terms.iter().partition(|t| t.chars().count() >= 3);
        let likes: Vec<String> = short.iter().map(|t| format!("%{}%", escape_like(t))).collect();
        let like_sql: String = (0..likes.len()).map(|i| format!(" AND r.text LIKE ?{} ESCAPE '\\'", i + 3)).collect();
        let sql = if long.is_empty() {
            format!(
                "SELECT r.task, r.kind, r.ts, r.text, t.title FROM search_rows r JOIN tasks t ON t.id = r.task
                 WHERE t.status != 'archived' AND ?1 = ?1{like_sql} ORDER BY r.ts DESC, r.id DESC LIMIT ?2"
            )
        } else {
            format!(
                "SELECT r.task, r.kind, r.ts, r.text, t.title FROM search_fts f
                 JOIN search_rows r ON r.id = f.rowid JOIN tasks t ON t.id = r.task
                 WHERE search_fts MATCH ?1 AND t.status != 'archived'{like_sql}
                 ORDER BY f.rank, r.ts DESC LIMIT ?2"
            )
        };
        let fts = long.iter().map(|t| format!("\"{}\"", t.replace('"', "\"\""))).collect::<Vec<_>>().join(" ");
        // Several rows per thread can match; fetch extra, keep one each.
        let fetch = (limit * 8).max(40) as i64;
        let mut values: Vec<rusqlite::types::Value> = vec![fts.into(), fetch.into()];
        values.extend(likes.into_iter().map(rusqlite::types::Value::from));
        let rows: Vec<(String, i64, i64, String, String)> = {
            let conn = self.conn.lock();
            let Ok(mut st) = conn.prepare_cached(&sql) else { return Vec::new() };
            st.query_map(rusqlite::params_from_iter(values), |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
        };
        let mut seen = std::collections::HashSet::new();
        let mut hits = Vec::new();
        for (task, kind, ts, text, title) in rows {
            if !seen.insert(task.clone()) {
                continue;
            }
            let (snippet, matched) = snippet(&text, &terms);
            hits.push(Hit {
                task: TaskId(parse_uuid(task)),
                title,
                kind: HitKind::from_id(kind),
                snippet,
                matched,
                ts,
            });
            if hits.len() == limit {
                break;
            }
        }
        hits
    }

    /// Drop `task` from the index (it was deleted).
    pub(crate) fn unindex(conn: &Connection, task: &str) -> rusqlite::Result<()> {
        conn.execute("DELETE FROM search_rows WHERE task = ?1", [task])?;
        conn.execute("DELETE FROM search_done WHERE task = ?1", [task])?;
        Ok(())
    }
}

fn insert_rows(conn: &Connection, task: &str, rows: &[(HitKind, String)], ts: i64) -> rusqlite::Result<()> {
    let mut st = conn.prepare_cached("INSERT INTO search_rows (task, kind, ts, text) VALUES (?1, ?2, ?3, ?4)")?;
    for (kind, text) in rows {
        st.execute(params![task, kind.id(), ts, text])?;
    }
    Ok(())
}

/// The indexable rows of a transcript file.
fn transcript_rows(path: &std::path::Path) -> Vec<(HitKind, String)> {
    let Ok(f) = std::fs::File::open(path) else { return Vec::new() };
    let mut ex = Extractor::default();
    let mut out = Vec::new();
    for line in BufReader::new(f).lines().map_while(Result::ok) {
        // Cheap skip for the bulk of a transcript (deltas of thinking, tool
        // output) before parsing JSON.
        if line.starts_with("{\"type\":\"tool_result\"") || line.starts_with("{\"type\":\"thinking_delta\"") {
            continue;
        }
        if let Ok(ev) = serde_json::from_str::<AgentEvent>(&line) {
            ex.push(&ev, &mut out);
        }
    }
    ex.finish(&mut out);
    out
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// The first case-insensitive match of any of `terms` in `text`.
fn find_any(text: &str, terms: &[String]) -> Option<Range<usize>> {
    terms.iter().filter_map(|t| find_ci(text, t)).min_by_key(|r| r.start)
}

/// Case-insensitive (Unicode lowercase) search, as byte offsets into `hay`.
fn find_ci(hay: &str, needle: &str) -> Option<Range<usize>> {
    let needle: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return None;
    }
    for (start, _) in hay.char_indices() {
        let mut want = needle.iter();
        let mut end = start;
        let mut ok = true;
        'chars: for (i, c) in hay[start..].char_indices() {
            for l in c.to_lowercase() {
                match want.next() {
                    Some(w) if *w == l => {}
                    // A mismatch, or the needle ends inside one character's
                    // lowercase expansion: no clean match here.
                    _ => {
                        ok = false;
                        break 'chars;
                    }
                }
            }
            end = start + i + c.len_utf8();
            if want.len() == 0 {
                break;
            }
        }
        if ok && want.len() == 0 {
            return Some(start..end);
        }
    }
    None
}

/// `text` around its first match, on one line, and the match within it.
fn snippet(text: &str, terms: &[String]) -> (String, Option<Range<usize>>) {
    let Some(m) = find_any(text, terms) else {
        let head: String = text.chars().take(BEFORE + AFTER).collect();
        let cut = head.len() < text.len();
        return (one_line(&head) + if cut { "…" } else { "" }, None);
    };
    let before_start = text[..m.start].char_indices().rev().nth(BEFORE - 1).map(|(i, _)| i).unwrap_or(0);
    let after_end = text[m.end..].char_indices().nth(AFTER).map(|(i, _)| m.end + i).unwrap_or(text.len());
    let mut out = String::new();
    if before_start > 0 {
        out.push('…');
    }
    out.push_str(one_line(&text[before_start..m.start]).trim_start());
    let start = out.len();
    out.push_str(&one_line(&text[m.clone()]));
    let end = out.len();
    out.push_str(one_line(&text[m.end..after_end]).trim_end());
    if after_end < text.len() {
        out.push('…');
    }
    (out, Some(start..end))
}

/// Whitespace runs (newlines included) as single spaces.
fn one_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Task;
    use dove_proto::{HarnessKind, TaskStatus, ToolKind};

    fn store_with_task(title: &str) -> (tempfile::TempDir, Store, TaskId) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let task = add_task(&store, dir.path(), title);
        (dir, store, task)
    }

    fn add_task(store: &Store, dir: &std::path::Path, title: &str) -> TaskId {
        let p = store.add_project(dir).unwrap();
        let task = Task {
            id: TaskId::new(),
            project_id: p.id,
            title: title.into(),
            harness: HarnessKind::ClaudeCode,
            model: None,
            status: TaskStatus::Idle,
            worktree: None,
            branch: None,
            native_session_id: None,
            base: None,
            pinned: false,
            updated_at: 0,
            prompt: None,
            unread: false,
            automation: None,
        };
        store.insert_task(&task).unwrap();
        task.id
    }

    fn live(store: &Store, task: TaskId, events: &[AgentEvent]) {
        let mut ix = LiveIndexer::new(task);
        for ev in events {
            ix.observe(store, ev);
        }
    }

    fn delta(s: &str) -> AgentEvent {
        AgentEvent::TextDelta { text: s.into() }
    }

    #[test]
    fn fts5_trigram_is_compiled_in() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE VIRTUAL TABLE t USING fts5(x, tokenize='trigram');").unwrap();
    }

    #[test]
    fn live_events_are_indexed_and_found() {
        let (_d, store, task) = store_with_task("Fix the login bug");
        live(
            &store,
            task,
            &[
                AgentEvent::UserMessage { text: "Why does the OAuth callback fail?".into() },
                delta("The callback "),
                delta("rejects the state parameter because it is regenerated."),
                AgentEvent::BlockEnd,
                AgentEvent::ToolCall {
                    call_id: "1".into(),
                    name: "Bash".into(),
                    kind: ToolKind::Execute,
                    title: "cargo test --package auth".into(),
                    input: serde_json::Value::Null,
                },
            ],
        );
        let hits = store.search("state parameter", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].task, task);
        assert_eq!(hits[0].title, "Fix the login bug");
        assert_eq!(hits[0].kind, HitKind::Assistant);
        let m = hits[0].matched.clone().unwrap();
        assert_eq!(&hits[0].snippet[m], "state");
        // Case-insensitive, user text and tool titles too.
        assert_eq!(store.search("oauth CALLBACK", 10)[0].kind, HitKind::User);
        assert_eq!(store.search("--package", 10)[0].kind, HitKind::Tool);
        assert!(store.search("nowhere to be found", 10).is_empty());
        // One hit per thread even when several rows match.
        assert_eq!(store.search("callback", 10).len(), 1);
    }

    #[test]
    fn substrings_and_cjk_match() {
        let (_d, store, task) = store_with_task("中文");
        live(&store, task, &[AgentEvent::UserMessage { text: "请帮我修复搜索功能的错误".into() }]);
        // Three characters and more go through FTS5, shorter through LIKE.
        assert_eq!(store.search("搜索功能", 5).len(), 1);
        assert_eq!(store.search("搜索", 5).len(), 1);
        assert_eq!(store.search("修复 错误", 5).len(), 1);
        assert!(store.search("搜查", 5).is_empty());
        let hit = &store.search("功能", 5)[0];
        assert_eq!(&hit.snippet[hit.matched.clone().unwrap()], "功能");
        // A single character is too vague to search.
        assert!(store.search("搜", 5).is_empty());
    }

    #[test]
    fn query_syntax_is_inert() {
        let (_d, store, task) = store_with_task("t");
        live(&store, task, &[AgentEvent::UserMessage { text: "100% done with a_b \"quoted\" AND NOT stuff".into() }]);
        assert_eq!(store.search("\"quoted\"", 5).len(), 1);
        assert_eq!(store.search("AND NOT", 5).len(), 1);
        assert_eq!(store.search("a_b", 5).len(), 1);
        assert!(store.search("x%", 5).is_empty());
        assert!(store.search("* OR (", 5).is_empty());
    }

    #[test]
    fn old_transcripts_are_backfilled_once() {
        let (dir, store, task) = store_with_task("Old thread");
        // Written before the index existed: a transcript and no index rows.
        {
            let mut w = store.transcript_writer(task).unwrap();
            for ev in [
                AgentEvent::UserMessage { text: "migrate the billing tables".into() },
                delta("Done: billing migrated."),
                AgentEvent::TurnEnded { usage: None, is_error: false },
            ] {
                w.append(&ev).unwrap();
            }
        }
        store.conn.lock().execute("DELETE FROM search_done", []).unwrap();
        assert!(store.search("billing", 5).is_empty());
        assert!(!store.search_backfilled());
        assert_eq!(store.backfill_search(), 1);
        assert!(store.search_backfilled());
        let hits = store.search("billing", 5);
        assert_eq!(hits.len(), 1);
        // A second backfill (next launch) skips threads already covered.
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.backfill_search(), 0);
        assert_eq!(store.search("migrated", 5).len(), 1);
        // Deleting the thread drops it from the index.
        store.delete_task(task).unwrap();
        assert!(store.search("billing", 5).is_empty());
        let n: i64 = store.conn.lock().query_row("SELECT count(*) FROM search_rows", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn archived_threads_are_left_out() {
        let (_d, store, task) = store_with_task("t");
        live(&store, task, &[AgentEvent::UserMessage { text: "kubernetes ingress".into() }]);
        store.set_status(task, TaskStatus::Archived).unwrap();
        assert!(store.search("ingress", 5).is_empty());
    }

    #[test]
    fn snippets_center_on_the_match() {
        let text = format!("{}\n\nthe NEEDLE is here\n{}", "a ".repeat(200), "b ".repeat(200));
        let (s, m) = snippet(&text, &["needle".into()]);
        assert!(s.starts_with('…') && s.ends_with('…'));
        assert_eq!(&s[m.unwrap()], "NEEDLE");
        assert!(!s.contains('\n'));
        // Case folding that changes byte lengths keeps offsets valid.
        assert_eq!(find_ci("xxİstanbul", "i̇stanbul"), Some(2..11));
        assert_eq!(find_ci("ÄBC", "äb"), Some(0..3));
    }
}
