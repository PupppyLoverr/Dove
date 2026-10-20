//! Teach: skills the user taught Dove (Tell + Practice).
//!
//! The work happens inside an ordinary agent thread: the bundled `teach`
//! skill (`skills/teach/`) has the agent interview the user, write a skill
//! into `$DOVE_HOME/taught/<name>/`, rehearse it on a fresh example and
//! publish it to every harness's skills folder with `teach_tool.py`. Dove
//! itself only reads the `TEACH.json` records to list taught skills and
//! writes the prompts that start a teaching or rehearsal thread, so any
//! harness can do the teaching.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Passing rehearsals before a skill counts as trusted (matches the script).
const TRUST_AFTER: usize = 2;

/// `$DOVE_HOME/taught`.
pub fn root() -> PathBuf {
    crate::office_skills::root().parent().map(Path::to_path_buf).unwrap_or_default().join("taught")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Written but never rehearsed.
    Draft,
    /// Rehearsed, not yet trusted (too few passes, or the last run failed).
    Rehearsing,
    Trusted,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Draft => "draft",
            Status::Rehearsing => "rehearsing",
            Status::Trusted => "trusted",
        }
    }
}

/// One taught skill, as `TEACH.json` records it.
#[derive(Clone, Debug)]
pub struct Taught {
    pub name: String,
    pub goal: String,
    pub status: Status,
    pub passes: usize,
    pub fails: usize,
    pub published: bool,
    pub dir: PathBuf,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Meta {
    goal: String,
    published: bool,
    trust_after: Option<usize>,
    rehearsals: Vec<Rehearsal>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Rehearsal {
    passed: bool,
}

fn read(dir: &Path) -> Option<Taught> {
    if !dir.join("SKILL.md").is_file() {
        return None;
    }
    let name = dir.file_name()?.to_str()?.to_string();
    let meta: Meta = std::fs::read_to_string(dir.join("TEACH.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let passes = meta.rehearsals.iter().filter(|r| r.passed).count();
    let fails = meta.rehearsals.len() - passes;
    let last_passed = meta.rehearsals.last().is_some_and(|r| r.passed);
    let status = if meta.rehearsals.is_empty() {
        Status::Draft
    } else if last_passed && passes >= meta.trust_after.unwrap_or(TRUST_AFTER) {
        Status::Trusted
    } else {
        Status::Rehearsing
    };
    Some(Taught { name, goal: meta.goal, status, passes, fails, published: meta.published, dir: dir.to_path_buf() })
}

/// Every taught skill, by name.
pub fn list() -> Vec<Taught> {
    list_in(&root())
}

fn list_in(root: &Path) -> Vec<Taught> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut out: Vec<Taught> = entries.flatten().filter_map(|e| read(&e.path())).collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

const LOAD: &str = "Use Dove's `teach` skill for this (if your agent has no skills folder, call the dove_skills tool \
                    with {\"name\": \"teach\"} to read it).";

/// The composer text that starts a teaching thread. The user finishes the
/// first line with what they want to teach.
pub fn teach_prompt() -> String {
    format!(
        "{LOAD} Interview me, then write, validate and rehearse the skill before publishing it.\n\n\
         Teach Dove how to: "
    )
}

/// The composer text that starts a rehearsal of `t` on a fresh example.
pub fn rehearse_prompt(t: &Taught) -> String {
    let next = match t.status {
        Status::Draft => "This is its first rehearsal.".to_string(),
        Status::Rehearsing => format!("So far: {} passed, {} failed.", t.passes, t.fails),
        Status::Trusted => "It is already trusted; this run checks it still works.".to_string(),
    };
    format!(
        "{LOAD} Rehearse the taught skill `{name}` (folder: {dir}) on a fresh example from its evals, following \
         the Practice steps exactly and in a safe place (scratch branch, test data). {next} Record the result \
         with teach_tool.py, fix the skill if anything failed, and publish it after a passing run.",
        name = t.name,
        dir = t.dir.display(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, meta: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: x\n---\n").unwrap();
        std::fs::write(dir.join("TEACH.json"), meta).unwrap();
    }

    #[test]
    fn reads_status_from_rehearsals() {
        let root = std::env::temp_dir().join(format!("dove-teach-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        write(&root.join("b-draft"), r#"{"goal":"g","rehearsals":[]}"#);
        write(&root.join("a-trusted"), r#"{"published":true,"rehearsals":[{"passed":false},{"passed":true},{"passed":true}]}"#);
        write(&root.join("c-failing"), r#"{"rehearsals":[{"passed":true},{"passed":true},{"passed":false}]}"#);
        std::fs::create_dir_all(root.join("not-a-skill")).unwrap();
        let all = list_in(&root);
        let names: Vec<_> = all.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["a-trusted", "b-draft", "c-failing"]);
        assert_eq!(all[0].status, Status::Trusted);
        assert!(all[0].published);
        assert_eq!((all[0].passes, all[0].fails), (2, 1));
        assert_eq!(all[1].status, Status::Draft);
        assert_eq!(all[1].goal, "g");
        assert_eq!(all[2].status, Status::Rehearsing);
        assert!(rehearse_prompt(&all[2]).contains("`c-failing`"));
        assert!(teach_prompt().contains("dove_skills"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
