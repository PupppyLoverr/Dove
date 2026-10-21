//! A rule router for the conductor phrasings that need no model:
//! "Claude, how's the auth fix?", "tell Codex to stop", "start a Gemini task
//! on the landing page in a new worktree", "talk to the tests thread".
//!
//! Anything it does not recognise returns `None`, and the conductor sends
//! the words to the current thread (or asks a model router, when one is
//! configured).

use crate::intent::Intent;
use crate::snapshot::{self, Snapshot};

const STOP_WORDS: &[&str] = &["stop", "halt", "cancel", "abort", "stop it", "stop working", "quit", "pause", "hold on"];

fn status_question(rest: &str) -> bool {
    const LEADS: &[&str] = &[
        "how's",
        "how is",
        "how are",
        "what's the status",
        "what is the status",
        "status",
        "are you done",
        "are you finished",
        "is it done",
        "how far",
        "where are you",
        "what are you doing",
        "what's happening",
        "what is happening",
        "any progress",
        "progress",
        "how's it going",
        "how is it going",
        "update",
    ];
    LEADS.iter().any(|l| rest == *l || rest.starts_with(&format!("{l} ")))
}

/// Strip a leading phrase, returning the rest.
fn after<'a>(s: &'a str, lead: &str) -> Option<&'a str> {
    s.strip_prefix(lead).and_then(|r| r.strip_prefix(' ')).map(str::trim).filter(|r| !r.is_empty())
}

/// Does `name` (a vocative or "tell X") name an agent or thread in `snap`?
fn names_thread(snap: &Snapshot, name: &str) -> Option<String> {
    let n = crate::fastpath::normalize(name);
    if n.is_empty() || n.split(' ').count() > 5 {
        return None;
    }
    snapshot::resolve(snap, &n).ok().map(|t| t.id.clone())
}

fn agent_named(snap: &Snapshot, word: &str) -> Option<String> {
    let w = crate::fastpath::normalize(word);
    snap.agents
        .iter()
        .find(|a| {
            a.id == w
                || crate::fastpath::normalize(&a.label) == w
                || crate::fastpath::normalize(&a.label).split(' ').next() == Some(w.as_str())
        })
        .map(|a| a.label.clone())
}

pub fn route(text: &str, snap: &Snapshot) -> Option<Intent> {
    let raw = text.trim();
    let n = crate::fastpath::normalize(raw);
    if n.is_empty() {
        return None;
    }

    // "start a gemini task on the landing page in a new worktree"
    for lead in ["start a", "start an", "kick off a", "kick off an", "spin up a", "spin up an", "start a new", "new"] {
        if let Some(rest) = after(&n, lead) {
            let mut words = rest.splitn(2, ' ');
            let agent = words.next().unwrap_or_default();
            let tail = words.next().unwrap_or_default();
            let Some(agent) = agent_named(snap, agent) else { continue };
            let Some(mut prompt) = [
                "task to",
                "task on",
                "task for",
                "task",
                "thread to",
                "thread on",
                "thread",
                "session to",
                "session on",
            ]
            .iter()
            .find_map(|k| after(tail, k).map(str::to_string)) else {
                continue;
            };
            let mut worktree = false;
            for w in [" in a new worktree", " in its own worktree", " in a worktree", " on a new branch"] {
                if let Some(p) = prompt.strip_suffix(w) {
                    prompt = p.to_string();
                    worktree = true;
                }
            }
            return Some(Intent::StartTask { agent, project: None, prompt, worktree });
        }
    }

    // "talk to the tests thread", "switch to codex"
    for lead in ["talk to", "switch to", "go to", "open", "let me talk to", "i want to talk to", "connect me to"] {
        if let Some(rest) = after(&n, lead)
            && names_thread(snap, rest).is_some()
        {
            return Some(Intent::TalkTo { thread: rest.to_string() });
        }
    }

    // "tell codex to stop", "tell the tests thread to also run clippy"
    if let Some(rest) = after(&n, "tell").or_else(|| after(&n, "ask"))
        && let Some((who, what)) = rest.split_once(" to ")
        && let Some(id) = names_thread(snap, who)
    {
        let what = what.trim();
        if STOP_WORDS.contains(&what) {
            return Some(Intent::Interrupt { thread: Some(id) });
        }
        // Keep the user's own casing for the message text.
        let at = raw.to_lowercase().find(" to ").map(|i| i + 4).unwrap_or(0);
        let text = raw[at..].trim().trim_end_matches(['.', '!']).to_string();
        return Some(Intent::Steer { thread: Some(id), text: if text.is_empty() { what.to_string() } else { text } });
    }

    // "stop codex", "interrupt the tests thread"
    for lead in ["stop", "interrupt", "cancel", "halt"] {
        if let Some(rest) = after(&n, lead)
            && let Some(id) = names_thread(snap, rest)
        {
            return Some(Intent::Interrupt { thread: Some(id) });
        }
    }

    // "Claude, how's the auth fix?" / "Codex, also run clippy"
    if let Some((who, rest)) = raw.split_once(',') {
        let rest_n = crate::fastpath::normalize(rest);
        if let Some(id) = names_thread(snap, who)
            && !rest_n.is_empty()
        {
            // The vocative names the agent; the rest may name the thread too.
            let id = if status_question(&rest_n) {
                let topic = LEADS_STRIP.iter().find_map(|l| after(&rest_n, l)).unwrap_or_default();
                let combined = format!("{} {topic}", crate::fastpath::normalize(who));
                names_thread(snap, &combined).unwrap_or(id)
            } else {
                id
            };
            if status_question(&rest_n) {
                return Some(Intent::Status { thread: Some(id), all: false });
            }
            if STOP_WORDS.contains(&rest_n.as_str()) {
                return Some(Intent::Interrupt { thread: Some(id) });
            }
            return Some(Intent::Steer { thread: Some(id), text: rest.trim().to_string() });
        }
    }

    // "how's the tests thread doing?"
    if status_question(&n) {
        let topic = LEADS_STRIP.iter().find_map(|l| after(&n, l)).unwrap_or_default();
        let topic = topic.trim_end_matches(" doing").trim_end_matches(" going");
        if let Some(id) = names_thread(snap, topic) {
            return Some(Intent::Status { thread: Some(id), all: false });
        }
    }
    None
}

const LEADS_STRIP: &[&str] =
    &["how's", "how is", "how are", "what's the status of", "what is the status of", "status of", "progress on"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::{Action, validate};
    use crate::snapshot::fixtures::*;

    fn act(text: &str) -> Option<Action> {
        let s = snapshot();
        route(text, &s).map(|i| validate(i, &s).unwrap_or_else(|e| panic!("{text}: {e:?}")))
    }

    #[test]
    fn conductor_phrasings_route_to_checked_actions() {
        assert_eq!(act("Claude, how's the auth fix?"), Some(Action::Status { thread: Some("t1".into()) }));
        assert_eq!(act("Tell Codex to stop."), Some(Action::Interrupt { thread: "t2".into() }));
        assert_eq!(
            act("Tell Codex to also run clippy on Windows."),
            Some(Action::Send { thread: "t2".into(), text: "also run clippy on Windows".into(), steer: true })
        );
        assert_eq!(
            act("Start a Gemini task on the landing page in a new worktree."),
            Some(Action::StartTask {
                agent: "gemini".into(),
                project: "p1".into(),
                prompt: "the landing page".into(),
                worktree: true
            })
        );
        assert_eq!(act("Talk to the tests thread."), Some(Action::TalkTo { thread: "t2".into() }));
        assert_eq!(act("Switch to codex"), Some(Action::TalkTo { thread: "t2".into() }));
        assert_eq!(act("Stop Codex"), Some(Action::Interrupt { thread: "t2".into() }));
        assert_eq!(act("How's the landing page going?"), Some(Action::Status { thread: Some("t3".into()) }));
        assert_eq!(act("Codex, stop"), Some(Action::Interrupt { thread: "t2".into() }));
    }

    #[test]
    fn ordinary_requests_are_left_for_the_current_thread() {
        for text in [
            "Add a retry with exponential backoff to the token refresh",
            "What does this function return when the cache is empty?",
            "Start the dev server and open localhost three thousand",
            "Tell me why the build failed",
            "Open the settings file and change the port",
            "Hi, can you hear me?",
        ] {
            assert_eq!(route(text, &snapshot()), None, "{text}");
        }
    }
}
