//! The skills Dove bundles (`skills/` in the repo), in the Agent Skills
//! format (a folder per skill with `SKILL.md` and optional `scripts/`):
//! the office skills pdf, pptx, xlsx and docx, plus dove-browser (ego lite
//! playbook), dove-memory (Supermemory), dove-computer (Dove Computer Use)
//! and teach (skills the user teaches Dove, see [`crate::teach`]).
//!
//! The files are compiled into the binary; [`crate::skills`] writes them to
//! `$DOVE_HOME/skills/<name>` and mirrors them, with every skill installed
//! from GitHub, into the folders harnesses read. To bundle another skill,
//! add its folder under `skills/` and one [`skill!`] line to [`SKILLS`].
//!
//! The engine's entry points are re-exported here so older callers
//! (`office_skills::ensure_installed`, `office_skills::describe`) keep
//! working.

pub use crate::skills::{describe, enabled, ensure_installed, harness_dirs, root, set_enabled};

/// One bundled skill: its folder name, description and files.
pub struct Skill {
    pub name: &'static str,
    pub description: &'static str,
    pub files: &'static [(&'static str, &'static str)],
}

/// A bundled skill from `skills/<name>`: `SKILL.md` plus the listed files,
/// given relative to the skill folder.
macro_rules! skill {
    ($name:literal, $description:literal, [$($file:literal),* $(,)?]) => {
        Skill {
            name: $name,
            description: $description,
            files: &[
                ("SKILL.md", include_str!(concat!("../../../skills/", $name, "/SKILL.md"))),
                $(($file, include_str!(concat!("../../../skills/", $name, "/", $file))),)*
            ],
        }
    };
}

pub const SKILLS: &[Skill] = &[
    skill!(
        "pdf",
        "Create PDFs from Markdown/JSON, merge, split, rotate, extract text, fill forms.",
        ["scripts/pdf_tool.py"]
    ),
    skill!(
        "pptx",
        "Create PowerPoint decks from outlines on real layouts; dump and edit slides.",
        ["scripts/pptx_tool.py"]
    ),
    skill!(
        "xlsx",
        "Create Excel workbooks with formulas, formats and charts; dump and edit cells.",
        ["scripts/xlsx_tool.py"]
    ),
    skill!(
        "docx",
        "Create Word documents with styles, lists, tables and images; edit text.",
        ["scripts/docx_tool.py"]
    ),
    skill!(
        "dove-browser",
        "Browser playbook: ego lite (the user's own Chromium) or Dove's browser, with Dove's safety and hand-off rules.",
        ["reference/recipes.md", "reference/troubleshooting.md", "evals/tasks.md"]
    ),
    skill!(
        "dove-memory",
        "Long-term memory across sessions and harnesses (Supermemory): what to search, save and never save.",
        []
    ),
    skill!(
        "dove-computer",
        "Desktop app playbook for Dove Computer Use (computer__* tools): tree-first loop, intent steps, safety.",
        []
    ),
    skill!(
        "teach",
        "Teach Dove a reusable skill: interview the user, write it, rehearse it, publish it to every agent.",
        ["scripts/teach_tool.py"]
    ),
];

pub fn find(name: &str) -> Option<&'static Skill> {
    let name = name.trim().to_ascii_lowercase();
    SKILLS.iter().find(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundles_every_skill_with_frontmatter() {
        for s in SKILLS {
            let md = s.files[0].1;
            let meta = crate::skills::validate(s.name, md).unwrap_or_else(|e| panic!("{}: {e}", s.name));
            assert_eq!(meta.name, s.name);
            for (path, text) in s.files {
                if path.ends_with(".py") {
                    assert!(text.contains("# /// script"), "{} {path} needs PEP 723 metadata", s.name);
                }
            }
        }
    }
}
