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

use std::path::{Path, PathBuf};

/// The folders of a backup that hold a bot's history (matched to the bot by name).
pub const HISTORY: [&str; 5] = ["conversations", "memories", "skills", "routines", "files"];
/// A file over this size is not copied.
pub const MAX_FILE: u64 = 50 * 1024 * 1024;
/// What one import copies, in all.
pub const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
/// How much of a bot's text is read for its job.
const MAX_TEXT: u64 = 256 * 1024;
/// How deep the backup's folders are followed.
const MAX_DEPTH: usize = 8;

/// A file to copy: where it is, and where it goes under the destination (relative).
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bot {
    pub name: String,
    /// Its standing instructions, as its file says them.
    pub job: String,
    /// Its profile files (from bots/) and its history (matched by name).
    pub files: Vec<Item>,
    /// What its JSON file says besides: its look, pin, role (else null).
    pub extra: serde_json::Value,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub bots: Vec<Bot>,
    /// Everything else in the backup, kept for the person.
    pub shared: Vec<Item>,
    /// Entries in bots/ that are neither text nor a folder.
    pub unread: usize,
}

/// Reads a backup's folder (only reads it). Refused without a `bots/` folder.
pub fn plan(root: &Path) -> Result<Plan, String> {
    let bots_dir = root.join("bots");
    if !bots_dir.is_dir() {
        return Err("There's no bots folder in it.".into());
    }
    let mut plan = Plan::default();
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
            plan.bots.push(Bot { name: tidy(&file_name(&entry)), job: job.trim().to_owned(), files, extra: serde_json::Value::Null });
        } else if is_text(&entry) {
            let (name, job, extra) = bot_text(&entry);
            let to = Path::new("profile").join(file_name(&entry));
            plan.bots.push(Bot { name, job, files: vec![Item { from: entry.clone(), to }], extra });
        } else {
            plan.unread += 1;
        }
    }

    // (Longest name first, so "Mina Pro" never gives its history to "Mina".)
    let mut names: Vec<(usize, String)> = plan
        .bots
        .iter()
        .enumerate()
        .map(|(i, b)| (i, b.name.to_lowercase()))
        .filter(|(_, n)| n.chars().count() >= 3)
        .collect();
    names.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    let mut everything = Vec::new();
    walk(root, &mut everything, 0);
    for f in everything {
        let rel = f.strip_prefix(root).unwrap_or(&f).to_path_buf();
        let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        let top = parts.first().map(String::as_str).unwrap_or_default();
        // (A bot's scratch folder while it exports: `_k22tmp`.)
        if top == "bots" || top.starts_with('_') {
            continue;
        }
        let owner = if HISTORY.contains(&top) {
            let tail = parts[1..].join("/").to_lowercase();
            names.iter().find(|(_, n)| tail.contains(n.as_str())).map(|(i, _)| *i)
        } else {
            None
        };
        let item = Item { from: f, to: rel };
        match owner {
            Some(i) => plan.bots[i].files.push(item),
            None => plan.shared.push(item),
        }
    }
    Ok(plan)
}

/// Copies what fits: a file over MAX_FILE, or past what's left of `budget`,
/// is skipped. Returns (copied, skipped).
pub fn copy_into(items: &[Item], to: &Path, budget: &mut u64) -> (usize, usize) {
    let (mut copied, mut skipped) = (0, 0);
    for item in items {
        let size = std::fs::metadata(&item.from).map(|m| m.len()).unwrap_or(u64::MAX);
        let dest = to.join(&item.to);
        let fits = size <= MAX_FILE && size <= *budget;
        let made = fits
            && dest.parent().map(|p| std::fs::create_dir_all(p).is_ok()).unwrap_or(false)
            && std::fs::copy(&item.from, &dest).is_ok();
        if made {
            *budget -= size;
            copied += 1;
        } else {
            skipped += 1;
        }
    }
    (copied, skipped)
}

/// A bot's name and job from its text file: a JSON one may say both.
fn bot_text(path: &Path) -> (String, String, serde_json::Value) {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let text = read_capped(path);
    if path.extension().map(|e| e.eq_ignore_ascii_case("json")).unwrap_or(false) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let name = v["name"].as_str().map(str::to_owned).unwrap_or(stem.clone());
            let job = ["instructions", "job", "prompt", "description"]
                .iter()
                .find_map(|k| v[*k].as_str().map(str::to_owned))
                .unwrap_or(text);
            let extra = serde_json::json!({ "look": v["look"], "pinned": v["pinned"], "hidden": v["hidden"], "role": v["role"], "tools": v["tools"] });
            return (tidy(&name), job, extra);
        }
    }
    (tidy(&stem), text, serde_json::Value::Null)
}

fn tidy(name: &str) -> String {
    let name: String = name.trim().chars().take(60).collect();
    if name.is_empty() { "Bot".to_owned() } else { name }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
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
        assert!(mina.files.iter().any(|i| i.to == Path::new("conversations/Mina/2026-10-01.md")));
        let otto = p.bots.iter().find(|b| b.name == "Otto").unwrap();
        assert_eq!(otto.job, "Keep the notes.");
        assert!(otto.files.iter().any(|i| i.to == Path::new("memories/otto-facts.jsonl")));
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
        assert_eq!(copy_into(&items, &dest, &mut budget), (1, 1));
        assert!(dest.join("conversations/a.md").is_file());
        assert!(!dest.join("conversations/b.md").exists());
        assert_eq!(budget, 1);
    }
}

// ── Grok Bot's own cache ─────────────────────────────────────────────────
// The Windows app keeps a cache of the account beside it: the roster (each
// bot's name, look and pin) and each bot's transcript. Read, never written;
// turned into a backup's layout (`bots/`, `conversations/`) so `plan` takes it
// like any other. What only a bot's computer holds (its memory, skills,
// files) comes by the export folder a bot writes itself (`export_dir`).

/// Where the app keeps its cache, when it's on this PC.
pub fn app_cache() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("Grok Bot").join("sand-client-persistence");
    dir.is_dir().then_some(dir)
}

/// Where the bots are asked to write their own export (a backup's layout).
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

/// Every slice of the cache by its decoded name, its value parsed.
fn slices(cache: &Path) -> Vec<(String, serde_json::Value)> {
    let mut out = Vec::new();
    for f in read_dir_sorted(cache) {
        let name = file_name(&f);
        let Some(stem) = name.strip_suffix(".blob") else { continue };
        let Some(key) = unbase32(stem) else { continue };
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            out.push((key, v["value"].clone()));
        }
    }
    out
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

/// The cache laid out as a backup in `out`: `bots/<name>.json` (its name,
/// description, look and pin) and `conversations/<name>/transcript.md`.
/// Returns how many bots.
pub fn stage_cache(cache: &Path, out: &Path) -> Result<usize, String> {
    let all = slices(cache);
    let find = |suffix: &str| all.iter().find(|(k, _)| k.ends_with(suffix)).map(|(_, v)| v.clone());
    let rows = find(".roster.last-roster").and_then(|v| v["rows"].as_array().cloned()).ok_or("Grok Bot's cache has no bots in it.")?;
    let pinned: Vec<String> = find(".ui-agent-refs")
        .and_then(|v| v["pinnedAgentIds"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()))
        .unwrap_or_default();
    std::fs::create_dir_all(out.join("bots")).map_err(|e| e.to_string())?;
    let mut n = 0;
    for row in &rows {
        if row["isRemovedFromSidebar"] == true {
            continue;
        }
        let (Some(id), Some(name)) = (row["id"].as_str(), row["name"].as_str()) else { continue };
        let file = safe_name(&tidy(name));
        let bot = serde_json::json!({
            "name": tidy(name),
            "description": row["description"].as_str().unwrap_or(""),
            "role": row["title"].as_str().unwrap_or(""),
            "look": look(row["avatarShape"].as_str().unwrap_or(""), row["avatarColor"].as_str().unwrap_or("")),
            "pinned": pinned.iter().any(|p| p == id),
            "hidden": row["isHiddenFromSidebar"] == true,
            "grokbot_id": id,
        });
        let text = serde_json::to_string_pretty(&bot).unwrap_or_default();
        std::fs::write(out.join("bots").join(format!("{file}.json")), text).map_err(|e| e.to_string())?;
        if let Some(t) = find(&format!(".transcript.replicas.{id}")) {
            let mut md = format!("# {} — conversation from Grok Bot\n\n", tidy(name));
            for e in t["entries"].as_array().into_iter().flatten() {
                if let Some(line) = entry_text(e) {
                    md.push_str(&line);
                    md.push('\n');
                }
            }
            let dir = out.join("conversations").join(&file);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("transcript.md"), md).map_err(|e| e.to_string())?;
        }
        n += 1;
    }
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
            let text = serde_json::json!({ "schemaVersion": 1, "value": value }).to_string();
            std::fs::write(cache.join(format!("{}.blob", base32(key))), text).unwrap();
        };
        put(
            "sand.client.slice.account.x.roster.last-roster",
            serde_json::json!({ "rows": [
                { "id": "a1", "name": "VCE Help", "description": "", "title": "Tutor", "avatarShape": "wedge", "avatarColor": "cyan" },
                { "id": "a2", "name": "Gone", "isRemovedFromSidebar": true },
            ]}),
        );
        put("sand.client.slice.account.x.ui-agent-refs", serde_json::json!({ "pinnedAgentIds": ["a1"] }));
        put(
            "sand.client.slice.account.x.transcript.replicas.a1",
            serde_json::json!({ "entries": [
                { "kind": "message", "role": "user", "content": "hi" },
                { "kind": "send-message", "message": { "type": "text", "content": "Hey Rav" } },
            ]}),
        );
        assert_eq!(stage_cache(&cache, &out).unwrap(), 1);
        let bot: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(out.join("bots").join("VCE Help.json")).unwrap()).unwrap();
        assert_eq!(bot["look"]["shape"], "drop");
        assert_eq!(bot["look"]["color"], "mint");
        assert_eq!(bot["pinned"], true);
        let md = std::fs::read_to_string(out.join("conversations").join("VCE Help").join("transcript.md")).unwrap();
        assert!(md.contains("**You:** hi") && md.contains("**Bot:** Hey Rav"));
        // (And the staged folder plans like any backup: the transcript is the bot's.)
        let p = plan(&out).unwrap();
        assert_eq!(p.bots.len(), 1);
        assert_eq!(p.bots[0].extra["role"], "Tutor");
        assert!(p.bots[0].files.iter().any(|i| i.to.ends_with("transcript.md")));
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
/// chats, its profile) under `.grokbot/`. A folder named for the bot is
/// dropped from what it made (`files/VCE Help/plan.md` -> `From Grok Bot/plan.md`).
pub fn place(to: &Path, bot: &str) -> PathBuf {
    let parts: Vec<String> = to.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    if parts.first().map(String::as_str) == Some("files") {
        let skip = if parts.get(1).is_some_and(|p| p.eq_ignore_ascii_case(bot)) && parts.len() > 2 { 2 } else { 1 };
        let mut out = PathBuf::from("From Grok Bot");
        for p in &parts[skip..] {
            out.push(p);
        }
        return out;
    }
    Path::new(".grokbot").join(to)
}

#[cfg(test)]
#[test]
fn what_it_made_goes_to_its_library_and_the_rest_out_of_sight() {
    assert_eq!(place(Path::new("files/VCE Help/plan.md"), "VCE Help"), Path::new("From Grok Bot/plan.md"));
    assert_eq!(place(Path::new("files/report.pdf"), "VCE Help"), Path::new("From Grok Bot/report.pdf"));
    assert_eq!(place(Path::new("memories/VCE Help/about.md"), "VCE Help"), Path::new(".grokbot/memories/VCE Help/about.md"));
    assert_eq!(place(Path::new("profile/VCE Help.json"), "VCE Help"), Path::new(".grokbot/profile/VCE Help.json"));
}
