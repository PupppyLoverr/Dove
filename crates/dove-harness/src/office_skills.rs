//! Dove's bundled office skills (`skills/` in the repo): pdf, pptx, xlsx and
//! docx, in the Agent Skills format (a folder per skill with `SKILL.md` and
//! `scripts/`).
//!
//! The files are compiled into the binary. Before the first agent starts,
//! [`ensure_installed`] writes them to `$DOVE_HOME/skills/<name>` and copies
//! them where harnesses look for user skills: `~/.claude/skills` (Claude
//! Code, OpenCode) and `~/.agents/skills` (Codex, OpenCode, Cursor, Devin,
//! Grok, Pi and other Agent Skills readers). A folder there that Dove did not
//! write is never touched. Agents that read no skill folders get the same
//! text from the `dove_skills` MCP tool.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// One bundled skill: its folder name, description and files.
pub struct Skill {
    pub name: &'static str,
    pub description: &'static str,
    pub files: &'static [(&'static str, &'static str)],
}

macro_rules! skill {
    ($name:literal, $script:literal, $description:literal) => {
        Skill {
            name: $name,
            description: $description,
            files: &[
                ("SKILL.md", include_str!(concat!("../../../skills/", $name, "/SKILL.md"))),
                (concat!("scripts/", $script), include_str!(concat!("../../../skills/", $name, "/scripts/", $script))),
            ],
        }
    };
}

pub const SKILLS: &[Skill] = &[
    skill!("pdf", "pdf_tool.py", "Create PDFs from Markdown/JSON, merge, split, rotate, extract text, fill forms."),
    skill!("pptx", "pptx_tool.py", "Create PowerPoint decks from outlines on real layouts; dump and edit slides."),
    skill!("xlsx", "xlsx_tool.py", "Create Excel workbooks with formulas, formats and charts; dump and edit cells."),
    skill!("docx", "docx_tool.py", "Create Word documents with styles, lists, tables and images; edit text."),
    skill!("teach", "teach_tool.py", "Teach Dove a reusable skill: interview the user, write it, rehearse it, publish it."),
];

/// Written into every folder Dove installs, so later runs know they may
/// update it.
const MARKER: &str = ".dove-skill";

static ENABLED: AtomicBool = AtomicBool::new(true);
static INSTALLED: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Turn installing the skills into harness folders on or off (the
/// `agent_office_skills` setting). Takes effect before the first agent starts.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// `$DOVE_HOME/skills`, the canonical copy agents can run scripts from.
pub fn root() -> PathBuf {
    std::env::var_os("DOVE_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".dove")))
        .unwrap_or_else(|| PathBuf::from(".dove"))
        .join("skills")
}

/// Where harnesses read user-level skills.
pub fn harness_dirs() -> Vec<PathBuf> {
    let Some(home) = home() else { return Vec::new() };
    let claude = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| home.join(".claude"));
    vec![claude.join("skills"), home.join(".agents").join("skills")]
}

pub fn find(name: &str) -> Option<&'static Skill> {
    let name = name.trim().to_ascii_lowercase();
    SKILLS.iter().find(|s| s.name == name)
}

/// A content stamp: a folder is rewritten only when the bundle changed.
fn stamp(skill: &Skill) -> String {
    // FNV-1a over every file, stable across builds and platforms.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (path, text) in skill.files {
        for b in path.bytes().chain(text.bytes()) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("dove {} {h:016x}\n", env!("CARGO_PKG_VERSION"))
}

/// Write `skill` into `dir/<name>` unless a folder Dove did not create is
/// already there. Returns whether the folder holds Dove's copy afterwards.
fn write_skill(skill: &Skill, dir: &Path) -> std::io::Result<bool> {
    let dest = dir.join(skill.name);
    let marker = dest.join(MARKER);
    let stamp = stamp(skill);
    if dest.exists() {
        match std::fs::read_to_string(&marker) {
            Ok(existing) if existing == stamp => return Ok(true),
            Ok(_) => {}
            // Someone else's skill of the same name: leave it alone.
            Err(_) => return Ok(false),
        }
    }
    for (rel, text) in skill.files {
        let path = dest.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, text)?;
        #[cfg(unix)]
        if rel.ends_with(".py") {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }
    }
    std::fs::write(marker, stamp)?;
    Ok(true)
}

/// Install the skills (once per process). Returns `$DOVE_HOME/skills` when
/// the canonical copy is in place.
pub fn ensure_installed() -> Option<PathBuf> {
    INSTALLED
        .get_or_init(|| {
            let root = root();
            let mut ok = true;
            for skill in SKILLS {
                if let Err(e) = write_skill(skill, &root) {
                    tracing::warn!("could not write the {} skill to {}: {e}", skill.name, root.display());
                    ok = false;
                }
            }
            if enabled() {
                for dir in harness_dirs() {
                    for skill in SKILLS {
                        match write_skill(skill, &dir) {
                            Ok(true) => {}
                            Ok(false) => tracing::info!(
                                "kept the existing {} skill in {} (not Dove's)",
                                skill.name,
                                dir.display()
                            ),
                            Err(e) => {
                                tracing::warn!("could not install the {} skill in {}: {e}", skill.name, dir.display())
                            }
                        }
                    }
                }
            }
            ok.then_some(root)
        })
        .clone()
}

/// The `dove_skills` tool's answer: the catalog, or one skill's full text.
pub fn describe(name: Option<&str>) -> Result<String, String> {
    let root = root();
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        None => {
            let mut out = String::from(
                "Dove's bundled skills. Call dove_skills with {\"name\": ...} for the full instructions.\n\n",
            );
            for s in SKILLS {
                out.push_str(&format!("- {}: {} (folder: {})\n", s.name, s.description, root.join(s.name).display()));
            }
            Ok(out)
        }
        Some(n) => {
            let skill = find(n).ok_or_else(|| {
                format!(
                    "No skill named {n:?}. Available: {}",
                    SKILLS.iter().map(|s| s.name).collect::<Vec<_>>().join(", ")
                )
            })?;
            let dir = root.join(skill.name);
            let body = skill.files.iter().find(|(p, _)| *p == "SKILL.md").map(|(_, t)| *t).unwrap_or_default();
            Ok(format!(
                "Skill folder: {dir}\nRun its scripts from there, for example `uv run {script}` (or python3 after \
                 installing the dependencies). Relative paths below are inside the skill folder.\n\n{body}",
                dir = dir.display(),
                script = dir.join(skill.files[1].0).display(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundles_every_skill_with_frontmatter() {
        for s in SKILLS {
            let md = s.files[0].1;
            assert!(md.starts_with("---\n"), "{} SKILL.md needs frontmatter", s.name);
            assert!(md.contains(&format!("\nname: {}\n", s.name)), "{} frontmatter name", s.name);
            assert!(md.contains("\ndescription: "), "{} frontmatter description", s.name);
            assert!(s.files[1].1.contains("# /// script"), "{} script needs PEP 723 metadata", s.name);
        }
    }

    #[test]
    fn installs_and_respects_foreign_folders() {
        let dir = std::env::temp_dir().join(format!("dove-skills-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docx")).unwrap();
        std::fs::write(dir.join("docx/SKILL.md"), "mine").unwrap();
        assert!(write_skill(&SKILLS[0], &dir).unwrap());
        assert!(dir.join("pdf/SKILL.md").exists());
        assert!(dir.join("pdf/scripts/pdf_tool.py").exists());
        // Rewriting an unchanged copy is a no-op that still reports ours.
        assert!(write_skill(&SKILLS[0], &dir).unwrap());
        let docx = SKILLS.iter().find(|s| s.name == "docx").unwrap();
        assert!(!write_skill(docx, &dir).unwrap());
        assert_eq!(std::fs::read_to_string(dir.join("docx/SKILL.md")).unwrap(), "mine");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn describes_catalog_and_skill() {
        assert!(describe(None).unwrap().contains("pptx"));
        assert!(describe(Some("XLSX")).unwrap().contains("name: xlsx"));
        assert!(describe(Some("nope")).is_err());
    }
}
