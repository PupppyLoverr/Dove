//! Mentions as cards: files, folders, Dove threads, apps' windows and
//! browser tabs picked from the composer's `@` (or `#`) menu sit as cards
//! above the input, like Codex's. On send each card becomes one line of a
//! block the agent reads ([`MENTIONS_HEADER`]): a thread as a digest file,
//! a window as a picture (plus how to look closer with Computer use), a tab
//! as its address and which browser tools reach it. The transcript draws
//! the cards back from that block ([`split_mentions`]).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::assets::icons;

/// One mentioned thing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Card {
    File {
        path: PathBuf,
    },
    Folder {
        path: PathBuf,
    },
    Thread {
        id: String,
        title: String,
        agent: String,
    },
    Window {
        app: String,
        title: String,
        pid: u32,
        id: u64,
    },
    Tab {
        /// In the user's own browser (Dove for Chrome) rather than Dove's.
        chrome: bool,
        title: String,
        url: String,
    },
}

impl Card {
    /// The card's short name.
    pub fn label(&self) -> String {
        match self {
            Card::File { path } | Card::Folder { path } => {
                path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string())
            }
            Card::Thread { title, .. } => title.clone(),
            Card::Window { app, title, .. } if app.is_empty() => title.clone(),
            Card::Window { app, .. } => app.clone(),
            Card::Tab { title, url, .. } => {
                if title.is_empty() {
                    url.clone()
                } else {
                    title.clone()
                }
            }
        }
    }

    /// A second line for menus and tooltips.
    pub fn detail(&self) -> String {
        match self {
            Card::File { path } | Card::Folder { path } => path.display().to_string(),
            Card::Thread { agent, .. } => format!("Thread · {agent}"),
            Card::Window { title, .. } => title.clone(),
            Card::Tab { chrome, url, .. } => format!("{} · {url}", if *chrome { "Chrome" } else { "Dove browser" }),
        }
    }

    pub fn glyph(&self) -> &'static str {
        match self {
            Card::File { .. } => icons::DOCUMENT,
            Card::Folder { .. } => icons::FOLDER,
            Card::Thread { .. } => icons::CHAT_ROUND_LINE,
            Card::Window { .. } => icons::MONITOR,
            Card::Tab { .. } => icons::GLOBE,
        }
    }

    /// Whether two cards name the same thing (no duplicates in the row).
    pub fn same(&self, other: &Card) -> bool {
        match (self, other) {
            (Card::File { path: a }, Card::File { path: b }) | (Card::Folder { path: a }, Card::Folder { path: b }) => {
                a == b
            }
            (Card::Thread { id: a, .. }, Card::Thread { id: b, .. }) => a == b,
            (Card::Window { id: a, .. }, Card::Window { id: b, .. }) => a == b,
            (Card::Tab { url: a, .. }, Card::Tab { url: b, .. }) => a == b,
            _ => false,
        }
    }
}

/// The block a sent message carries for its cards.
pub const MENTIONS_HEADER: &str = "Mentioned (read or look at each as needed):";

/// What the agent gets for each card. Threads are written out as digests
/// under `<data>/mentions/`, windows are pictured there (where the
/// platform allows); a card whose source is gone is left out.
pub fn block(cards: &[Card], store: &dove_core::store::Store, computer_use: bool) -> Option<String> {
    let dir = store.root().join("mentions");
    let lines: Vec<String> = cards.iter().filter_map(|c| line(c, store, &dir, computer_use)).collect();
    (!lines.is_empty()).then(|| format!("{MENTIONS_HEADER}\n{}", lines.join("\n")))
}

fn line(card: &Card, store: &dove_core::store::Store, dir: &Path, computer_use: bool) -> Option<String> {
    Some(match card {
        Card::File { path } => format!("- File: {}", path.display()),
        Card::Folder { path } => format!("- Folder: {}", path.display()),
        Card::Thread { id, title, agent } => {
            let task = dove_core::proto::TaskId::parse(id).and_then(|t| store.task(t).ok().flatten())?;
            let text = dove_core::digest::transcript_digest(store, &task);
            std::fs::create_dir_all(dir).ok()?;
            let path = dir.join(format!("thread-{id}.md"));
            std::fs::write(&path, text).ok()?;
            format!(
                "- Thread \"{}\" ({}): its conversation is at {}. Read it for context.",
                one_line(title),
                agent,
                path.display()
            )
        }
        Card::Window { app, title, pid, id } => {
            let picture = crate::look::capture_window(*id as isize, title, dir)
                .map(|p| format!(" A picture of it is at {}.", p.display()))
                .unwrap_or_default();
            let closer = if computer_use {
                format!(
                    " To look closer, call the computer tool with action \"capture\", pid {pid} and window_id {id}; only read it unless the user asks you to act."
                )
            } else {
                String::new()
            };
            format!("- Window \"{}\" ({app}):{picture}{closer}", one_line(title))
        }
        Card::Tab { chrome, title, url } => {
            let how = if *chrome {
                "in the user's own browser; the chrome_* tools reach it"
            } else {
                "in Dove's browser; browser_navigate opens it"
            };
            format!("- Tab \"{}\": {url} ({how}).", one_line(title))
        }
    })
}

/// A title on one line, without the quotes the block uses.
fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ").replace('"', "'")
}

/// The cards a sent message mentioned, read back from its block, for the
/// transcript. Ids are not kept there, so threads and windows come back by
/// name only.
pub fn split_mentions(text: &str) -> Vec<Card> {
    let Some(at) = text.find(MENTIONS_HEADER) else { return Vec::new() };
    text[at + MENTIONS_HEADER.len()..]
        .lines()
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| l.trim_start().starts_with("- "))
        .filter_map(|l| parse_line(l.trim_start().strip_prefix("- ")?))
        .collect()
}

fn quoted(s: &str) -> Option<(String, &str)> {
    let s = s.strip_prefix('"')?;
    let end = s.find('"')?;
    Some((s[..end].to_string(), &s[end + 1..]))
}

fn parse_line(l: &str) -> Option<Card> {
    if let Some(p) = l.strip_prefix("File: ") {
        return Some(Card::File { path: PathBuf::from(p.trim()) });
    }
    if let Some(p) = l.strip_prefix("Folder: ") {
        return Some(Card::Folder { path: PathBuf::from(p.trim()) });
    }
    if let Some(rest) = l.strip_prefix("Thread ") {
        let (title, rest) = quoted(rest)?;
        let agent = rest.trim_start().strip_prefix('(')?.split_once(')')?.0.to_string();
        return Some(Card::Thread { id: String::new(), title, agent });
    }
    if let Some(rest) = l.strip_prefix("Window ") {
        let (title, rest) = quoted(rest)?;
        let app = rest.trim_start().strip_prefix('(')?.split_once(')')?.0.to_string();
        return Some(Card::Window { app, title, pid: 0, id: 0 });
    }
    if let Some(rest) = l.strip_prefix("Tab ") {
        let (title, rest) = quoted(rest)?;
        let rest = rest.trim_start().strip_prefix(':')?.trim_start();
        let (url, how) = rest.split_once(" (")?;
        return Some(Card::Tab { chrome: how.starts_with("in the user's"), title, url: url.trim().to_string() });
    }
    None
}

/// Cards persisted with a draft, as JSON.
pub fn to_json(cards: &[Card]) -> String {
    serde_json::to_string(cards).unwrap_or_default()
}

pub fn from_json(text: &str) -> Vec<Card> {
    serde_json::from_str(text).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_lines_read_back_as_cards() {
        let sent = format!(
            "compare these\n\n{MENTIONS_HEADER}\n- File: C:/p/src/main.rs\n- Folder: C:/p/docs\n\
             - Thread \"Fix login\" (Claude Code): its conversation is at C:/d/mentions/thread-1.md. Read it for context.\n\
             - Window \"notes.txt - Notepad\" (Notepad): A picture of it is at C:/d/x.png.\n\
             - Tab \"Docs\": https://example.com/a (in the user's own browser; the chrome_* tools reach it).\n\
             \nAttached files (read them as needed):\n- C:/a.png"
        );
        let cards = split_mentions(&sent);
        assert_eq!(
            cards,
            vec![
                Card::File { path: "C:/p/src/main.rs".into() },
                Card::Folder { path: "C:/p/docs".into() },
                Card::Thread { id: String::new(), title: "Fix login".into(), agent: "Claude Code".into() },
                Card::Window { app: "Notepad".into(), title: "notes.txt - Notepad".into(), pid: 0, id: 0 },
                Card::Tab { chrome: true, title: "Docs".into(), url: "https://example.com/a".into() },
            ]
        );
        assert_eq!(crate::composer::split_attachments(&sent).0, "compare these", "the bubble keeps only the words");
        assert!(split_mentions("plain text").is_empty());
    }

    #[test]
    fn cards_round_trip_through_drafts_and_dedupe() {
        let cards = vec![
            Card::Thread { id: "abc".into(), title: "T".into(), agent: "Codex".into() },
            Card::Tab { chrome: false, title: "".into(), url: "http://localhost:3000".into() },
        ];
        assert_eq!(from_json(&to_json(&cards)), cards);
        assert_eq!(cards[1].label(), "http://localhost:3000");
        assert!(cards[0].same(&Card::Thread { id: "abc".into(), title: "renamed".into(), agent: "x".into() }));
        assert!(!cards[0].same(&cards[1]));
        assert!(from_json("not json").is_empty());
    }

    #[test]
    fn titles_stay_on_one_line_inside_quotes() {
        assert_eq!(one_line("a \"b\"\nc"), "a 'b' c");
    }
}
