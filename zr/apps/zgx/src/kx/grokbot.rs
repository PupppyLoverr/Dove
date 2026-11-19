//! Bringing a Grok Bot backup across: the folder a backup of Grok Bot makes,
//! laid out as its guide has it (`bots/`, `conversations/`, `memories/`,
//! `skills/`, `routines/`, `files/`, `inventories/`). Each bot in `bots/`
//! becomes a buddy (buddies.rs does that); what the backup holds for it is
//! copied into the buddy's folder under `imported/`, which its agent reads as
//! history. What names no bot, and the inventories, go to
//! `grokbot-import/shared/` beside the buddies, for the person to look at.
//!
//! Routines are copied, never made into automations: an import starts
//! nothing on a clock. The backup folder is only read.
//!
//! A bot's history is joined to it by the bot's own id where the source has
//! one (the app's cache always does) and by its name only where it does not.
//! See `Bot::key`.

use std::path::{Path, PathBuf};

/// The folders of a backup that hold a bot's history.
pub const HISTORY: [&str; 5] = ["conversations", "memories", "skills", "routines", "files"];
/// A file over this size is not copied.
pub const MAX_FILE: u64 = 50 * 1024 * 1024;
/// What one import copies, in all.
pub const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
/// How much of a bot's text is read.
const MAX_TEXT: u64 = 256 * 1024;
/// A job longer than this is not pasted into every turn: the whole file stays
/// in the buddy's folder and the prompt points at it.
pub const MAX_JOB: usize = 8 * 1024;
/// How deep the backup's folders are followed.
const MAX_DEPTH: usize = 8;

/// Where a bot's job may be, best first: the keys Grok Bot writes, then what
/// other tools' exports use. A bot with only a `role` gets its role, never a
/// whole file of JSON read as its instructions.
const JOB_KEYS: [&str; 8] = ["instructions", "job", "prompt", "description", "role", "title", "system", "persona"];

/// A file to copy: where it is, and where it goes under the destination (relative).
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bot {
    pub name: String,
    /// Grok Bot's own id for it, when the source says it (the app's cache
    /// always does).
    pub id: Option<String>,
    /// What its history is joined to it by: `id:<id>` when the source gives an
    /// id, else `name:<slug of its name>`. A bot renamed in Grok Bot keeps its
    /// id, so its conversations and memories stay its own; a bot renamed here
    /// changes nothing either, because nothing of ours is joined by name.
    pub key: String,
    /// Its standing instructions, as its file says them: empty when they were
    /// too long to carry, and `brief` says where they are.
    pub job: String,
    /// A file too long to paste as its job: the path under its folder, and
    /// its size (which the prompt quotes back to it).
    pub brief: Option<(PathBuf, u64)>,
    /// Its profile files (from bots/) and its history.
    pub files: Vec<Item>,
    /// What its JSON file says besides: its look, pin, role (else null).
    pub extra: serde_json::Value,
}

impl Bot {
    /// How many of one part of its history it brings (`conversations`,
    /// `memories`, `skills`, `routines`, `files`, or `profile`).
    pub fn count(&self, kind: &str) -> usize {
        self.files.iter().filter(|i| top_of(&i.to).as_deref() == Some(kind)).count()
    }
}

#[derive(Debug, Default)]
pub struct Plan {
    pub bots: Vec<Bot>,
    /// Everything else in the backup, kept for the person.
    pub shared: Vec<Item>,
    /// Entries in bots/ that are neither text nor a folder.
    pub unread: usize,
    /// Where it came from, in words, when the source says (the app's cache
    /// names its schema).
    pub source: Option<String>,
    /// What the source could not carry at all, in words: the app's cache has
    /// no memories, skills, routines or files to give.
    pub absent: Vec<String>,
    /// Transcript entries the cache held that this import cannot read (a
    /// widget kind it doesn't know): counted, never silently dropped.
    pub dropped: usize,
}

/// Reads a backup's folder (only reads it). Refused without a `bots/` folder.
pub fn plan(root: &Path) -> Result<Plan, String> {
    let bots_dir = root.join("bots");
    if !bots_dir.is_dir() {
        return Err("There's no bots folder in it.".into());
    }
    let mut plan = Plan::default();
    read_source(root, &mut plan);
    let mut bots = Vec::new();
    for entry in read_dir_sorted(&bots_dir) {
        let Ok(meta) = std::fs::symlink_metadata(&entry) else { continue };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            let mut inner = Vec::new();
            walk(&entry, &mut inner, 0);
            let mut job = String::new();
            let mut files = Vec::new();
            for f in inner {
                if is_text(&f) && job.len() < MAX_TEXT as usize {
                    job.push_str(&read_capped(&f));
                    job.push('\n');
                }
                let to = Path::new("profile").join(f.strip_prefix(&entry).unwrap_or(&f));
                files.push(Item { from: f, to });
            }
            let name = tidy(&file_name(&entry));
            let job = job.trim().to_owned();
            let brief = oversized(&job, PathBuf::from("profile"));
            bots.push(Bot {
                key: key_of(None, &name),
                name,
                id: None,
                job: if brief.is_some() { String::new() } else { job },
                brief,
                files,
                extra: serde_json::Value::Null,
            });
        } else if is_text(&entry) {
            let text = bot_text(&entry);
            let to = Path::new("profile").join(file_name(&entry));
            let brief = oversized(&text.job, to.clone());
            bots.push(Bot {
                key: key_of(text.id.as_deref(), &text.name),
                name: text.name,
                id: text.id,
                job: if brief.is_some() { String::new() } else { text.job },
                brief,
                files: vec![Item { from: entry.clone(), to }],
                extra: text.extra,
            });
        } else {
            plan.unread += 1;
        }
    }

    // (Longest name first, so "Mina Pro" never gives its history to "Mina".)
    let mut index: Vec<(usize, String)> = bots
        .iter()
        .enumerate()
        .map(|(i, b)| (i, b.name.to_lowercase()))
        .filter(|(_, n)| n.chars().count() >= 3)
        .collect();
    index.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    let mut everything = Vec::new();
    walk(root, &mut everything, 0);
    for f in everything {
        let rel = f.strip_prefix(root).unwrap_or(&f).to_path_buf();
        let parts = components(&rel);
        let top = parts.first().map(String::as_str).unwrap_or_default();
        // (A bot's scratch folder while it exports: `_k22tmp`; and the note
        // the cache path leaves behind.)
        if top == "bots" || top.starts_with('_') {
            continue;
        }
        let owner = if HISTORY.contains(&top) { owner(&parts, &bots, &index) } else { None };
        let item = Item { from: f, to: rel };
        match owner {
            Some(i) => bots[i].files.push(item),
            None => plan.shared.push(item),
        }
    }
    plan.bots = bots;
    Ok(plan)
}

/// The bot a history file belongs to: the one whose id names the folder under
/// it or the file itself, and only then the one whose name it holds. The id
/// comes first because a bot renamed in Grok Bot keeps its id and loses
/// nothing; a name is what changes.
fn owner(parts: &[String], bots: &[Bot], by_name: &[(usize, String)]) -> Option<usize> {
    let segment = parts.get(1).map(String::as_str).unwrap_or_default().to_lowercase();
    let stem = parts.last().map(|p| p.rsplit_once('.').map_or(p.as_str(), |(s, _)| s)).unwrap_or_default().to_lowercase();
    // (By the id first: the folder under `conversations/`, or a file named for
    // it, in either form an id takes (`a1b2c3` or the folder's `id-a1b2c3`).)
    let carries = |b: &Bot| forms(b).iter().any(|f| holds(&segment, f) || holds(&stem, f));
    if let Some(i) = bots.iter().position(carries) {
        return Some(i);
    }
    let tail = parts[1..].join("/").to_lowercase();
    by_name.iter().find(|(_, name)| tail.contains(name.as_str())).map(|(i, _)| *i)
}

/// The forms a bot's own name on disk may take: its key as a folder, and its
/// id as the source wrote it.
fn forms(b: &Bot) -> Vec<String> {
    let mut out = vec![dir_key(&b.key)];
    if let Some(id) = &b.id {
        out.push(id.to_lowercase());
    }
    out
}

/// Whether a folder or file name is that form, or starts with it and then a
/// join (`a1b2c3-facts`, `a1b2c3.2026-10`): an id's own things, never another
/// bot's.
fn holds(name: &str, form: &str) -> bool {
    !form.is_empty()
        && (name == form || name.strip_prefix(form).is_some_and(|rest| rest.starts_with(['-', '_', '.'])))
}

/// A job too long to paste, and where the whole of it is: the file stays in
/// the buddy's folder and its instructions point at it.
fn oversized(job: &str, at: PathBuf) -> Option<(PathBuf, u64)> {
    (job.len() > MAX_JOB).then(|| (at, job.len() as u64))
}

/// What a bot's file says about it.
struct Text {
    name: String,
    id: Option<String>,
    job: String,
    extra: serde_json::Value,
}

/// A bot's name, id and job from its text file: a JSON one says them in its
/// own fields (and never has its whole text read as its job), a plain one is
/// its own job.
fn bot_text(path: &Path) -> Text {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let text = read_capped(path);
    if path.extension().map(|e| e.eq_ignore_ascii_case("json")).unwrap_or(false) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let name = v["name"].as_str().map(str::to_owned).unwrap_or(stem.clone());
            let id = ["grokbot_id", "id", "agent_id", "agentId"].iter().find_map(|k| v[*k].as_str().map(str::to_owned));
            let job = job_of(&v).unwrap_or_default();
            let extra = serde_json::json!({ "look": v["look"], "pinned": v["pinned"], "hidden": v["hidden"], "role": v["role"] });
            return Text { name: tidy(&name), id, job, extra };
        }
    }
    Text { name: tidy(&stem), id: None, job: text.trim().to_owned(), extra: serde_json::Value::Null }
}

/// A bot's job from its JSON, under the first of its keys that holds words: a
/// string, or the lines of an array.
fn job_of(v: &serde_json::Value) -> Option<String> {
    for k in JOB_KEYS {
        match &v[k] {
            serde_json::Value::String(s) if !s.trim().is_empty() => return Some(s.trim().to_owned()),
            serde_json::Value::Array(a) => {
                let lines: Vec<&str> = a.iter().filter_map(|x| x.as_str()).filter(|x| !x.trim().is_empty()).collect();
                if !lines.is_empty() {
                    return Some(lines.join("\n"));
                }
            }
            _ => {}
        }
    }
    None
}

/// The key a bot's history is joined to it by: its id when it has one, its
/// name otherwise.
pub fn key_of(id: Option<&str>, name: &str) -> String {
    let id: String = id
        .unwrap_or_default()
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(64)
        .collect();
    if id.is_empty() {
        format!("name:{}", slug(name))
    } else {
        format!("id:{}", id.to_ascii_lowercase())
    }
}

/// A name as a folder may hold it: lower-case, words joined by one dash.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_owned();
    if out.is_empty() { "bot".to_owned() } else { out }
}

/// A key as a folder's name (`:` is no part of a Windows file name).
pub fn dir_key(key: &str) -> String {
    key.replace(':', "-")
}

/// The note the cache path leaves so a plan can say what it could not carry.
fn read_source(root: &Path, plan: &mut Plan) {
    let Ok(text) = std::fs::read_to_string(root.join("_source.json")) else { return };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return };
    plan.source = v["source"].as_str().map(str::to_owned);
    plan.absent = v["absent"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    plan.dropped = v["dropped"].as_u64().unwrap_or(0) as usize;
}

/// Copies what fits: a file over MAX_FILE, or past what's left of `budget`,
/// is skipped and says why. A file already there at its own size is not
/// copied again and costs nothing: a second import carries what a first one
/// ran out of room for.
pub fn copy_into(items: &[Item], to: &Path, budget: &mut u64) -> Copied {
    let mut out = Copied::default();
    for item in items {
        let dest = to.join(&item.to);
        let size = std::fs::metadata(&item.from).map(|m| m.len()).unwrap_or(u64::MAX);
        if std::fs::metadata(&dest).is_ok_and(|m| m.len() == size) {
            out.kept += 1;
            continue;
        }
        if size > MAX_FILE {
            out.skipped.push((item.to.clone(), "over the 50 MB a file may be".to_owned()));
            continue;
        }
        if size > *budget {
            out.skipped.push((item.to.clone(), "past what one import carries".to_owned()));
            continue;
        }
        let made = dest.parent().is_some_and(|p| std::fs::create_dir_all(p).is_ok()) && std::fs::copy(&item.from, &dest).is_ok();
        if made {
            *budget -= size;
            out.files += 1;
        } else {
            out.skipped.push((item.to.clone(), "it couldn't be written".to_owned()));
        }
    }
    out
}

/// What one `copy_into` did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Copied {
    pub files: usize,
    /// Already there from a run that stopped: done, and free.
    pub kept: usize,
    /// What did not come, and why.
    pub skipped: Vec<(PathBuf, String)>,
}

fn tidy(name: &str) -> String {
    let name: String = name.trim().chars().take(60).collect();
    if name.is_empty() { "Bot".to_owned() } else { name }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn components(path: &Path) -> Vec<String> {
    path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect()
}

fn top_of(path: &Path) -> Option<String> {
    path.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned())
}

fn is_text(path: &Path) -> bool {
    path.extension().map(|e| matches!(e.to_string_lossy().to_ascii_lowercase().as_str(), "md" | "txt" | "json")).unwrap_or(false)
}

fn read_capped(path: &Path) -> String {
    use std::io::Read;
    let mut text = String::new();
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(MAX_TEXT).read_to_string(&mut text);
    }
    text
}

fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    out.sort();
    out
}

/// Every file under `dir`, links not followed.
fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for entry in read_dir_sorted(dir) {
        let Ok(meta) = std::fs::symlink_metadata(&entry) else { continue };
        if meta.file_type().is_symlink() {
            continue;
        } else if meta.is_dir() {
            // (What a tool rebuilds, never what a bot made.)
            if matches!(file_name(&entry).as_str(), "node_modules" | ".git" | "__pycache__" | ".venv" | "venv" | ".next" | ".cache") {
                continue;
            }
            walk(&entry, out, depth + 1);
        } else if meta.is_file() {
            out.push(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backup someone exported, read as the import will read it:
    /// `KX_BACKUP=<folder> cargo test -- --ignored plan_the_real_backup --nocapture`.
    #[test]
    #[ignore]
    fn plan_the_real_backup() {
        let Some(root) = std::env::var_os("KX_BACKUP") else { return };
        let p = plan(Path::new(&root)).unwrap();
        println!("source: {:?}, shared: {}", p.source, p.shared.len());
        for b in &p.bots {
            println!(
                "{} | key {} | job {} chars | conv {} mem {} skills {} routines {} made {}",
                b.name,
                b.key,
                b.job.len(),
                b.count("conversations"),
                b.count("memories"),
                b.count("skills"),
                b.count("routines"),
                b.count("files")
            );
        }
    }

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn a_backup_without_bots_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(plan(dir.path()).is_err());
    }

    #[test]
    fn bots_come_from_their_files_and_their_history_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        put(root, "bots/Mina.md", "Watch the inbox.");
        put(root, "bots/Otto.json", r#"{"name":"Otto","instructions":"Keep the notes."}"#);
        put(root, "conversations/Mina/2026-10-01.md", "hello");
        put(root, "memories/otto-facts.jsonl", "{}");
        put(root, "conversations/unknown.md", "who?");
        put(root, "inventories/files.md", "list");
        let p = plan(root).unwrap();
        assert_eq!(p.bots.len(), 2);
        let mina = p.bots.iter().find(|b| b.name == "Mina").unwrap();
        assert_eq!(mina.job, "Watch the inbox.");
        assert_eq!(mina.key, "name:mina");
        assert_eq!(mina.count("conversations"), 1);
        assert!(mina.files.iter().any(|i| i.to == Path::new("conversations/Mina/2026-10-01.md")));
        let otto = p.bots.iter().find(|b| b.name == "Otto").unwrap();
        assert_eq!(otto.job, "Keep the notes.");
        assert_eq!(otto.count("memories"), 1);
        let shared: Vec<&Path> = p.shared.iter().map(|i| i.to.as_path()).collect();
        assert!(shared.contains(&Path::new("conversations/unknown.md")));
        assert!(shared.contains(&Path::new("inventories/files.md")));
    }

    #[test]
    fn the_longer_name_takes_its_own_history() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        put(root, "bots/Mina.md", "a");
        put(root, "bots/Mina Pro.md", "b");
        put(root, "conversations/Mina Pro chat.md", "x");
        let p = plan(root).unwrap();
        let mina = p.bots.iter().find(|b| b.name == "Mina").unwrap();
        let pro = p.bots.iter().find(|b| b.name == "Mina Pro").unwrap();
        assert!(!mina.files.iter().any(|i| i.to.ends_with("Mina Pro chat.md")));
        assert!(pro.files.iter().any(|i| i.to.ends_with("Mina Pro chat.md")));
    }

    #[test]
    fn a_history_folder_named_for_the_id_goes_to_that_bot_whatever_it_is_called() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Renamed in Grok Bot since: the file says what it is called now, the
        // history folder still carries its id.
        put(root, "bots/a1b2c3.json", r#"{"id":"a1b2c3","name":"VCE Notes","description":"Tutor"}"#);
        put(root, "bots/orphan.json", r#"{"name":"Mina","instructions":"x"}"#);
        put(root, "conversations/a1b2c3/2026-10-01.md", "about the exam");
        put(root, "memories/a1b2c3-facts.md", "remembers");
        let p = plan(root).unwrap();
        let notes = p.bots.iter().find(|b| b.name == "VCE Notes").unwrap();
        assert_eq!(notes.key, "id:a1b2c3");
        assert_eq!(notes.count("conversations"), 1);
        assert_eq!(notes.count("memories"), 1);
        // ("Mina" is not given another bot's history for holding part of a word.)
        let mina = p.bots.iter().find(|b| b.name == "Mina").unwrap();
        assert_eq!(mina.files.len(), 1);
    }

    #[test]
    fn a_job_is_the_first_of_its_own_fields_and_never_the_whole_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        put(root, "bots/role-only.json", r#"{"name":"Otto","role":"Chief of staff","instructions":""}"#);
        put(root, "bots/lines.json", r#"{"name":"Tess","instructions":["First.","Second."]}"#);
        put(root, "bots/other.json", r#"{"name":"Kit","description":"Keep the books","job":"Not this one"}"#);
        let p = plan(root).unwrap();
        let job = |n: &str| p.bots.iter().find(|b| b.name == n).unwrap().job.clone();
        assert_eq!(job("Otto"), "Chief of staff");
        assert_eq!(job("Tess"), "First.\nSecond.");
        assert_eq!(job("Kit"), "Not this one");
    }

    #[test]
    fn a_job_too_long_to_paste_is_left_in_the_folder_with_a_pointer() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let long = "x".repeat(MAX_JOB + 10);
        put(root, "bots/Big.md", &long);
        let p = plan(root).unwrap();
        let big = &p.bots[0];
        assert!(big.job.is_empty());
        let (at, size) = big.brief.clone().unwrap();
        assert_eq!(at, Path::new("profile/Big.md"));
        assert_eq!(size as usize, long.len());
    }

    #[test]
    fn copies_stop_at_the_budget_and_keep_the_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        put(root, "a.md", "12345");
        put(root, "b.md", "1234567890");
        let items = vec![
            Item { from: root.join("a.md"), to: PathBuf::from("conversations/a.md") },
            Item { from: root.join("b.md"), to: PathBuf::from("conversations/b.md") },
        ];
        let dest = root.join("out");
        let mut budget = 6;
        let done = copy_into(&items, &dest, &mut budget);
        assert_eq!((done.files, done.skipped.len()), (1, 1));
        assert_eq!(done.skipped[0].1, "past what one import carries");
        assert!(dest.join("conversations/a.md").is_file());
        assert!(!dest.join("conversations/b.md").exists());
        assert_eq!(budget, 1);
        // (A second run with room to spare carries the rest, and the file that
        // is already there costs nothing.)
        let mut more = u64::MAX;
        let again = copy_into(&items, &dest, &mut more);
        assert_eq!((again.files, again.kept), (1, 1));
        assert!(dest.join("conversations/b.md").is_file());
    }
}

// ── Grok Bot's own cache ─────────────────────────────────────────────────
// The Windows app keeps a cache of the account beside it: the roster (each
// bot's name, look and pin) and each bot's transcript. Read, never written;
// turned into a backup's layout (`bots/`, `conversations/`) so `plan` takes it
// like any other. It carries no memory, skills, routines or files: what only a
// bot's own computer holds comes by a backup folder it exports itself.
//
// The layout is checked before it is trusted. A cache that has been updated
// past what this import knows is named as it is (with the schema it declares)
// and pointed at the backup folder, instead of "there are no bots in it".

/// Where the app keeps its cache, when it's on this PC.
pub fn app_cache() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("Grok Bot").join("sand-client-persistence");
    dir.is_dir().then_some(dir)
}

/// Where the docs say a bot writes its own backup (a backup's layout).
/// (Unverified: no code here proves the Grok Bot app writes this folder.)
pub fn export_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("USERPROFILE")?).join("Grok Bot Export"))
}

/// A cache file's name, decoded (lower-case base32, unpadded).
fn unbase32(s: &str) -> Option<String> {
    const A: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let (mut bits, mut n, mut out) = (0u32, 0u32, Vec::new());
    for c in s.bytes() {
        bits = (bits << 5) | A.iter().position(|&a| a == c)? as u32;
        n += 5;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
            bits &= (1 << n) - 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Every slice of the cache by its decoded name, and the file as it was read.
fn slices(cache: &Path) -> Vec<(String, serde_json::Value)> {
    let mut out = Vec::new();
    for f in read_dir_sorted(cache) {
        let name = file_name(&f);
        let Some(stem) = name.strip_suffix(".blob") else { continue };
        let Some(key) = unbase32(stem) else { continue };
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            out.push((key, v));
        }
    }
    out
}

/// What the cache says it is: the schema its slices declare, if any.
fn schema(all: &[(String, serde_json::Value)]) -> Option<i64> {
    all.iter().find_map(|(_, v)| v["schemaVersion"].as_i64())
}

/// A Grok Bot look in ours: its shape and colour by the nearest we have.
fn look(shape: &str, color: &str) -> serde_json::Value {
    let shape = match shape {
        "star" => "star4",
        "flower" => "puff",
        "hex" => "hexagon",
        "wedge" => "drop",
        "blob" => "cloud",
        s => s,
    };
    let color = match color {
        "cyan" | "teal" => "mint",
        "purple" => "violet",
        "pink" => "rose",
        "yellow" => "amber",
        "gray" | "grey" => "slate",
        "white" => "snow",
        c => c,
    };
    serde_json::json!({ "shape": shape, "color": color })
}

fn safe_name(name: &str) -> String {
    name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '-' } else { c }).collect()
}

/// One transcript entry as Markdown, or nothing for what holds no words.
fn entry_text(e: &serde_json::Value) -> Option<String> {
    let m = &e["message"];
    let (who, body) = match e["kind"].as_str()? {
        "message" => (if e["role"] == "user" { "You" } else { "Bot" }, e["content"].as_str()?.to_owned()),
        "send-message" => match m["type"].as_str()? {
            "text" => ("Bot", m["content"].as_str()?.to_owned()),
            "widget" => {
                let w = &m["widget"];
                let options: Vec<&str> = w["options"].as_array().map(|o| o.iter().filter_map(|x| x["label"].as_str()).collect()).unwrap_or_default();
                ("Bot", format!("{} [{}]", w["prompt"].as_str().unwrap_or(""), options.join(" / ")))
            }
            other => ("Bot", format!("({other})\n```json\n{}\n```", serde_json::to_string_pretty(&m[other.replace('-', "_")]).ok().filter(|s| s != "null").unwrap_or_else(|| serde_json::to_string_pretty(m).unwrap_or_default()))),
        },
        "user-attachment" => ("You", format!("(attached {})", e["name"].as_str().or(e["attachment"]["name"].as_str()).unwrap_or("a file"))),
        _ => return None,
    };
    Some(format!("**{who}:** {}\n", body.trim()))
}

/// The cache laid out as a backup in `out`: `bots/<id>.json` (its name,
/// description, look, pin and **its id**) and `conversations/<id>/transcript.md`.
/// Named by the id, not the name: two bots called the same thing don't collide,
/// and a bot renamed in Grok Bot keeps its history. Also writes `_source.json`,
/// which says what the cache could not carry and which schema it was read at.
/// Returns how many bots.
pub fn stage_cache(cache: &Path, out: &Path) -> Result<usize, String> {
    let all = slices(cache);
    let at = match schema(&all) {
        Some(n) => format!("schema {n}"),
        None => "an unreadable schema".to_owned(),
    };
    let find = |suffix: &str| all.iter().find(|(k, _)| k.ends_with(suffix)).map(|(_, v)| v["value"].clone());
    let rows = match find(".roster.last-roster").and_then(|v| v["rows"].as_array().cloned()) {
        Some(rows) => rows,
        None => {
            return Err(format!(
                "Grok Bot's cache is {at}, and this import doesn't find a roster where it expects one (it may have been updated past what it knows). Use a backup folder instead: in Grok Bot, Export, then choose that folder."
            ))
        }
    };
    let pinned: Vec<String> = find(".ui-agent-refs")
        .and_then(|v| v["pinnedAgentIds"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()))
        .unwrap_or_default();
    std::fs::create_dir_all(out.join("bots")).map_err(|e| e.to_string())?;
    let mut n = 0;
    let mut dropped = 0usize;
    for row in &rows {
        if row["isRemovedFromSidebar"] == true {
            continue;
    }
        let (Some(id), Some(name)) = (row["id"].as_str(), row["name"].as_str()) else { continue };
        let file = safe_name(&dir_key(&key_of(Some(id), name)));
        let bot = serde_json::json!({
            "name": tidy(name),
            "id": id,
            "description": row["description"].as_str().unwrap_or(""),
            "role": row["title"].as_str().unwrap_or(""),
            "look": look(row["avatarShape"].as_str().unwrap_or(""), row["avatarColor"].as_str().unwrap_or("")),
            "pinned": pinned.iter().any(|p| p == id),
            "hidden": row["isHiddenFromSidebar"] == true,
        });
        let text = serde_json::to_string_pretty(&bot).unwrap_or_default();
        std::fs::write(out.join("bots").join(format!("{file}.json")), text).map_err(|e| e.to_string())?;
        if let Some(t) = find(&format!(".transcript.replicas.{id}")) {
            let mut md = format!("# {} — conversation from Grok Bot\n\n", tidy(name));
            for e in t["entries"].as_array().into_iter().flatten() {
                match entry_text(e) {
                    Some(line) => md.push_str(&line),
                    None => dropped += 1,
                }
            }
            let dir = out.join("conversations").join(&file);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("transcript.md"), md).map_err(|e| e.to_string())?;
        }
        n += 1;
    }
    let source = serde_json::json!({
        "source": format!("Grok Bot's cache ({at})"),
        "absent": ["memories", "skills", "routines", "the files it made"],
        "dropped": dropped,
    });
    std::fs::write(out.join("_source.json"), serde_json::to_string_pretty(&source).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(n)
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn base32(s: &str) -> String {
        const A: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
        let (mut bits, mut n, mut out) = (0u32, 0u32, String::new());
        for b in s.bytes() {
            bits = (bits << 8) | b as u32;
            n += 8;
            while n >= 5 {
                n -= 5;
                out.push(A[((bits >> n) & 31) as usize] as char);
            }
        }
        if n > 0 {
            out.push(A[((bits << (5 - n)) & 31) as usize] as char);
        }
        out
    }

    #[test]
    fn its_cache_becomes_a_backup_with_looks_pins_and_transcripts() {
        let tmp = std::env::temp_dir().join(format!("kx-grokcache-{}", std::process::id()));
        let cache = tmp.join("cache");
        let out = tmp.join("out");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&cache).unwrap();
        let put = |key: &str, value: serde_json::Value| {
            let text = serde_json::json!({ "schemaVersion": 3, "value": value }).to_string();
            std::fs::write(cache.join(format!("{}.blob", base32(key))), text).unwrap();
        };
        put(
            "sand.client.slice.account.x.roster.last-roster",
            serde_json::json!({ "rows": [
                { "id": "a1", "name": "VCE Help", "description": "", "title": "Tutor", "avatarShape": "wedge", "avatarColor": "cyan" },
                { "id": "a2", "name": "Gone", "isRemovedFromSidebar": true },
                { "id": "a3", "name": "VCE Help", "title": "The other one" },
            ]}),
        );
        put("sand.client.slice.account.x.ui-agent-refs", serde_json::json!({ "pinnedAgentIds": ["a1"] }));
        put(
            "sand.client.slice.account.x.transcript.replicas.a1",
            serde_json::json!({ "entries": [
                { "kind": "message", "role": "user", "content": "hi" },
                { "kind": "send-message", "message": { "type": "text", "content": "Hey Rav" } },
                { "kind": "something-new", "message": {} },
            ]}),
        );
        assert_eq!(stage_cache(&cache, &out).unwrap(), 2);
        // (Two bots of one name are two files, by id.)
        let bot: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(out.join("bots").join("id-a1.json")).unwrap()).unwrap();
        assert_eq!(bot["look"]["shape"], "drop");
        assert_eq!(bot["look"]["color"], "mint");
        assert_eq!(bot["pinned"], true);
        assert!(out.join("bots").join("id-a3.json").is_file());
        let md = std::fs::read_to_string(out.join("conversations").join("id-a1").join("transcript.md")).unwrap();
        assert!(md.contains("**You:** hi") && md.contains("**Bot:** Hey Rav"));
        // (And the staged folder plans like any backup: the transcript is the
        // bot's, joined by its id, and what was dropped is counted.)
        let p = plan(&out).unwrap();
        assert_eq!(p.bots.len(), 2);
        let first = p.bots.iter().find(|b| b.id.as_deref() == Some("a1")).unwrap();
        assert_eq!(first.extra["role"], "Tutor");
        assert_eq!(first.key, "id:a1");
        assert!(first.files.iter().any(|i| i.to.ends_with("transcript.md")));
        assert_eq!(p.dropped, 1);
        assert_eq!(p.source.as_deref(), Some("Grok Bot's cache (schema 3)"));
        assert!(p.absent.contains(&"memories".to_owned()));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn a_cache_it_cannot_read_says_which_schema_and_where_to_go_instead() {
        let tmp = std::env::temp_dir().join(format!("kx-grokcache-old-{}", std::process::id()));
        let cache = tmp.join("cache");
        let out = tmp.join("out");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&cache).unwrap();
        let text = serde_json::json!({ "schemaVersion": 9, "value": { "bots": [] } }).to_string();
        std::fs::write(cache.join(format!("{}.blob", base32("sand.client.slice.account.x.roster.last-roster"))), text).unwrap();
        let err = stage_cache(&cache, &out).unwrap_err();
        assert!(err.contains("schema 9"), "{err}");
        assert!(err.contains("backup folder"), "{err}");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

#[cfg(test)]
#[test]
#[ignore]
fn stage_the_real_cache() {
    let (Some(cache), Some(out)) = (app_cache(), std::env::var_os("KX_STAGE_OUT")) else { return };
    let n = stage_cache(&cache, Path::new(&out)).unwrap();
    let p = plan(Path::new(&out)).unwrap();
    println!("staged {n}");
    for b in p.bots {
        println!("{} | {} files | {}", b.name, b.files.len(), b.extra);
    }
}

/// Where a bot's item goes in its buddy's folder: what it made (`files/`) to
/// its Library under `From Grok Bot/`; the rest (memories, skills, routines,
/// chats, its profile) under `.grokbot/<kind>/<its key>/`. The folder is named
/// for the key, not for what the bot is called today: a bot renamed in Grok Bot
/// adds to the same place on the next import instead of starting a new one.
pub fn place(to: &Path, bot: &str, key: &str) -> PathBuf {
    let parts = components(to);
    let Some(top) = parts.first() else { return Path::new(".grokbot").join(to) };
    if top == "files" {
        let skip = if parts.get(1).is_some_and(|p| p.eq_ignore_ascii_case(bot)) && parts.len() > 2 { 2 } else { 1 };
        let mut out = PathBuf::from("From Grok Bot");
        for p in &parts[skip..] {
            out.push(p);
        }
        return out;
    }
    if !HISTORY.contains(&top.as_str()) {
        return Path::new(".grokbot").join(to);
    }
    let key = dir_key(key);
    let mut out = Path::new(".grokbot").join(top).join(&key);
    let rest = if parts.get(1).is_some_and(|p| p.eq_ignore_ascii_case(bot) || p.eq_ignore_ascii_case(&key)) { &parts[2..] } else { &parts[1..] };
    for p in rest {
        out.push(p);
    }
    out
}

#[cfg(test)]
#[test]
fn what_it_made_goes_to_its_library_and_the_rest_under_its_key() {
    assert_eq!(place(Path::new("files/VCE Help/plan.md"), "VCE Help", "id:a1"), Path::new("From Grok Bot/plan.md"));
    assert_eq!(place(Path::new("files/report.pdf"), "VCE Help", "id:a1"), Path::new("From Grok Bot/report.pdf"));
    assert_eq!(place(Path::new("memories/VCE Help/about.md"), "VCE Help", "id:a1"), Path::new(".grokbot/memories/id-a1/about.md"));
    assert_eq!(place(Path::new("memories/VCE Help/about.md"), "VCE Help", "name:vce-help"), Path::new(".grokbot/memories/name-vce-help/about.md"));
    // (A file that names its bot in its own name, with no folder of the bot's.)
    assert_eq!(place(Path::new("memories/vce-facts.md"), "VCE Help", "id:a1"), Path::new(".grokbot/memories/id-a1/vce-facts.md"));
    assert_eq!(place(Path::new("profile/VCE Help.json"), "VCE Help", "id:a1"), Path::new(".grokbot/profile/VCE Help.json"));
    // (Already placed under its key by an earlier import: stays there.)
    assert_eq!(place(Path::new("conversations/id-a1/2026-10.md"), "VCE Help", "id:a1"), Path::new(".grokbot/conversations/id-a1/2026-10.md"));
}
