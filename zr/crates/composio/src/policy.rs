//! Who may use which connected app, and when an action waits for an OK.
//!
//! `<data>/connections.json`, written by the app (Settings → Connections):
//!
//! ```json
//! { "on": true,
//!   "scope": "everywhere",               // apps with no rule of their own
//!   "apps": { "gmail": { "scope": "some", "buddies": ["b123"], "rule": "ask_changes" } },
//!   "buddy_chats": { "<chat id>": "<buddy id>" } }
//! ```
//!
//! A chat is a buddy's when `buddy_chats` names it; otherwise it's one of
//! Zeron's own. Nothing here is a secret: the key lives in `keys.json`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const FILE: &str = "connections.json";

/// Where an app may be used.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Every chat and every buddy.
    #[default]
    Everywhere,
    /// Zeron's own chats, no buddy.
    Zeron,
    /// Every buddy, no ordinary chat.
    Buddies,
    /// Only the buddies listed.
    Some,
}

/// When an app's action waits for the person.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// Reading runs; anything that sends, posts, deletes or changes waits.
    #[default]
    AskChanges,
    /// Everything runs.
    Allow,
    /// Everything waits.
    AlwaysAsk,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct App {
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub buddies: Vec<String>,
    #[serde(default)]
    pub rule: Rule,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Policy {
    /// Off: no chat gets Composio at all.
    #[serde(default = "yes")]
    pub on: bool,
    /// For apps without a rule of their own.
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub buddies: Vec<String>,
    #[serde(default)]
    pub rule: Rule,
    /// By the app's slug, lowercase (`gmail`, `googlecalendar`).
    #[serde(default)]
    pub apps: BTreeMap<String, App>,
    /// Which chats are buddies' (chat id → buddy id).
    #[serde(default)]
    pub buddy_chats: BTreeMap<String, String>,
}

impl Default for Policy {
    fn default() -> Self {
        Self { on: true, scope: Scope::Everywhere, buddies: Vec::new(), rule: Rule::AskChanges, apps: BTreeMap::new(), buddy_chats: BTreeMap::new() }
    }
}

/// Where a call comes from: a buddy's chat, or one of Zeron's own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Place {
    pub buddy: Option<String>,
}

fn reaches(scope: Scope, buddies: &[String], place: &Place) -> bool {
    match scope {
        Scope::Everywhere => true,
        Scope::Zeron => place.buddy.is_none(),
        Scope::Buddies => place.buddy.is_some(),
        Scope::Some => place.buddy.as_ref().is_some_and(|b| buddies.iter().any(|x| x == b)),
    }
}

impl Policy {
    /// The policy kept in `data_dir`, or the default (on, everywhere,
    /// asking before changes) when there's none or it can't be read.
    pub fn load(data_dir: &Path) -> Self {
        std::fs::read(data_dir.join(FILE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let path = data_dir.join(FILE);
        let tmp = data_dir.join(format!("{FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default())?;
        std::fs::rename(tmp, path)
    }

    pub fn place(&self, chat: Option<&str>) -> Place {
        Place { buddy: chat.and_then(|c| self.buddy_chats.get(c).cloned()) }
    }

    fn app(&self, app: &str) -> Option<&App> {
        self.apps.get(&app.to_ascii_lowercase())
    }

    /// Whether `place` may use `app` at all.
    pub fn allows(&self, app: &str, place: &Place) -> bool {
        if !self.on {
            return false;
        }
        match self.app(app) {
            Some(a) => reaches(a.scope, &a.buddies, place),
            None => reaches(self.scope, &self.buddies, place),
        }
    }

    /// Whether `place` may use any app: if not, it gets no Composio at all.
    pub fn reaches(&self, place: &Place) -> bool {
        self.on && (reaches(self.scope, &self.buddies, place) || self.apps.values().any(|a| reaches(a.scope, &a.buddies, place)))
    }

    /// Whether `place` may use every app: only then are Composio's remote
    /// sandboxes (which can call any app) offered to it.
    pub fn unrestricted(&self, place: &Place) -> bool {
        self.on && reaches(self.scope, &self.buddies, place) && self.apps.values().all(|a| reaches(a.scope, &a.buddies, place))
    }

    pub fn rule(&self, app: &str) -> Rule {
        self.app(app).map(|a| a.rule).unwrap_or(self.rule)
    }

    /// The app a tool belongs to, by its slug (`GMAIL_SEND_EMAIL` → `gmail`):
    /// the longest app the policy names that prefixes it, else the slug's
    /// first part.
    pub fn app_of(&self, slug: &str) -> String {
        let upper = slug.to_ascii_uppercase();
        self.apps
            .keys()
            .filter(|k| upper.starts_with(&format!("{}_", k.to_ascii_uppercase())))
            .max_by_key(|k| k.len())
            .cloned()
            .unwrap_or_else(|| upper.split('_').next().unwrap_or_default().to_ascii_lowercase())
    }

    /// Whether a tool must wait for an OK under its app's rule.
    pub fn waits(&self, slug: &str) -> bool {
        match self.rule(&self.app_of(slug)) {
            Rule::Allow => false,
            Rule::AlwaysAsk => true,
            Rule::AskChanges => changes(slug),
        }
    }
}

/// Words in a tool's slug that only read. A tool with none of them is taken
/// to change something, so an unknown verb waits rather than runs.
const READS: &[&str] = &["GET", "LIST", "SEARCH", "FETCH", "FIND", "READ", "QUERY", "RETRIEVE", "LOOKUP", "DESCRIBE", "VIEW", "CHECK", "COUNT", "DOWNLOAD", "EXPORT"];

pub fn changes(slug: &str) -> bool {
    let upper = slug.to_ascii_uppercase();
    // (The app's own name comes first: `GITHUB_LIST_…`'s verb is LIST.)
    !upper.split('_').skip(1).any(|word| READS.contains(&word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buddy(id: &str) -> Place {
        Place { buddy: Some(id.into()) }
    }

    #[test]
    fn by_default_every_chat_and_buddy_may_use_every_app() {
        let p = Policy::default();
        assert!(p.allows("gmail", &Place::default()));
        assert!(p.allows("gmail", &buddy("b1")));
        assert!(p.reaches(&Place::default()) && p.unrestricted(&buddy("b1")));
    }

    #[test]
    fn an_app_kept_to_some_buddies_is_theirs_alone() {
        let mut p = Policy::default();
        p.apps.insert("gmail".into(), App { scope: Scope::Some, buddies: vec!["b1".into()], rule: Rule::AskChanges });
        assert!(p.allows("GMAIL", &buddy("b1")));
        assert!(!p.allows("gmail", &buddy("b2")));
        assert!(!p.allows("gmail", &Place::default()));
        // Slack, with no rule of its own, still goes everywhere; but nobody
        // but b1 may have the sandboxes that could reach Gmail.
        assert!(p.allows("slack", &buddy("b2")));
        assert!(!p.unrestricted(&buddy("b2")));
    }

    #[test]
    fn zeron_only_and_buddies_only_split_by_the_chat() {
        let mut p = Policy::default();
        p.buddy_chats.insert("chat-b".into(), "b1".into());
        p.scope = Scope::Zeron;
        assert!(p.reaches(&p.place(Some("chat-z"))));
        assert!(!p.reaches(&p.place(Some("chat-b"))));
        p.scope = Scope::Buddies;
        assert!(!p.reaches(&p.place(Some("chat-z"))));
        assert!(p.reaches(&p.place(Some("chat-b"))));
    }

    #[test]
    fn off_means_nothing_anywhere() {
        let p = Policy { on: false, ..Policy::default() };
        assert!(!p.reaches(&Place::default()) && !p.allows("gmail", &Place::default()));
    }

    #[test]
    fn reading_runs_and_changing_waits() {
        let mut p = Policy::default();
        assert!(!p.waits("GMAIL_FETCH_EMAILS"));
        assert!(!p.waits("GITHUB_LIST_REPOSITORIES"));
        assert!(p.waits("GMAIL_SEND_EMAIL"));
        assert!(p.waits("SLACK_SOMETHING_NEW"));
        p.apps.insert("gmail".into(), App { rule: Rule::Allow, ..App::default() });
        assert!(!p.waits("GMAIL_SEND_EMAIL"));
        p.apps.insert("notion".into(), App { rule: Rule::AlwaysAsk, ..App::default() });
        assert!(p.waits("NOTION_SEARCH_PAGES"));
    }

    #[test]
    fn a_tool_belongs_to_the_longest_app_named() {
        let mut p = Policy::default();
        assert_eq!(p.app_of("GOOGLECALENDAR_CREATE_EVENT"), "googlecalendar");
        p.apps.insert("google_drive".into(), App::default());
        assert_eq!(p.app_of("GOOGLE_DRIVE_UPLOAD"), "google_drive");
    }

    #[test]
    fn it_saves_and_loads_and_a_missing_file_is_the_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Policy::load(dir.path()), Policy::default());
        let mut p = Policy::default();
        p.scope = Scope::Buddies;
        p.save(dir.path()).unwrap();
        assert_eq!(Policy::load(dir.path()), p);
    }
}
