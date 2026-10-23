//! A rule router for the conductor phrasings that need no model:
//! "Claude, how's the auth fix?", "tell Codex to stop", "start a Gemini task
//! on the landing page in a new worktree", "talk to the tests thread".
//!
//! Anything it does not recognise returns `None`, and the conductor sends
//! the words to the current thread (or asks a model router, when one is
//! configured).

use crate::intent::{Intent, LookTarget};
use crate::snapshot::{self, Snapshot};

/// "take a look at this", "look at the page", "check out this window": what
/// to attach to the next message. A bare "this" means what Dove shows beside
/// the thread, or else the last other app's window.
fn look(n: &str, snap: &Snapshot) -> Option<Intent> {
    const LEADS: &[&str] =
        &["take a look at", "have a look at", "look at", "check out", "can you see", "do you see", "see", "check"];
    let rest = LEADS.iter().find_map(|l| after(n, l))?;
    let rest = rest.trim_end_matches(" please").trim_end_matches(" here").trim_end_matches(" now");
    if matches!(rest, "this" | "that" | "it" | "this one" | "what i'm looking at" | "what i see") {
        return Some(Intent::LookAt { target: snap.showing.unwrap_or(LookTarget::Window) });
    }
    // Only a pointer at one thing ("the page", "my screen"): "check the
    // screen size handling" is a request for the agent, not a capture.
    let thing = ["this ", "that ", "the ", "my ", "our ", "the current ", "my current "]
        .iter()
        .rev()
        .find_map(|d| rest.strip_prefix(d))
        .unwrap_or(rest);
    let target = match thing {
        "diff" | "changes" | "change" => LookTarget::Diff,
        "page" | "site" | "website" | "browser" | "tab" | "browser tab" | "web page" => LookTarget::Browser,
        "window" | "app" | "screen" | "editor" => LookTarget::Window,
        _ => return None,
    };
    Some(Intent::LookAt { target })
}

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

fn number(word: &str) -> Option<u32> {
    const WORDS: [&str; 10] = ["one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];
    match word {
        "a" | "an" | "another" => Some(1),
        "a couple" | "couple" | "pair" => Some(2),
        w => w.parse().ok().or_else(|| WORDS.iter().position(|x| *x == w).map(|i| i as u32 + 1)),
    }
}

/// The user's own words after the first of `connectors` found at or after
/// byte `from` of the lowercase text (casing kept).
fn tail_after(raw: &str, from: usize, connectors: &[&str]) -> Option<String> {
    let lower = raw.to_lowercase();
    let at = connectors.iter().filter_map(|c| lower.get(from..)?.find(c).map(|i| (from + i, c.len()))).min()?;
    let text = raw.get(at.0 + at.1..)?.trim().trim_end_matches(['.', '!', '?']).trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// "launch three claude chats to review the auth code", "open two codex
/// terminals", "spin up a terminal as the builder".
fn launch(raw: &str, n: &str, snap: &Snapshot) -> Option<Intent> {
    const LEADS: &[&str] = &[
        "launch", "open up", "open", "start", "spin up", "kick off", "fire up", "create", "give me", "i need", "i want",
    ];
    const NOUNS: &[(&str, bool)] = &[
        ("terminals", true),
        ("terminal", true),
        ("terminal sessions", true),
        ("chats", false),
        ("chat", false),
        ("threads", false),
        ("thread", false),
        ("sessions", false),
        ("agents", false),
    ];
    let rest = LEADS.iter().find_map(|l| after(n, l))?;
    let mut words: Vec<&str> = rest.split(' ').collect();
    // "a couple of", "two new", "three more"
    let (count, used) = if words.len() >= 2 && number(&format!("{} {}", words[0], words[1])).is_some() {
        (number(&format!("{} {}", words[0], words[1]))?, 2)
    } else {
        (number(words.first()?)?, 1)
    };
    words.drain(..used);
    while matches!(words.first(), Some(&"of" | &"new" | &"more" | &"fresh" | &"separate" | &"parallel")) {
        words.remove(0);
    }
    let agent = words.first().and_then(|w| agent_named(snap, w));
    if agent.is_some() {
        words.remove(0);
    }
    let joined = words.join(" ");
    let (noun, terminal) = NOUNS.iter().find(|(noun, _)| joined == *noun || joined.starts_with(&format!("{noun} ")))?;
    // One chat is an ordinary request ("start a claude task"); one
    // terminal, or several of anything, is a launch.
    if count < 2 && !terminal {
        return None;
    }
    let after_noun = joined[noun.len()..].trim();
    let teammate = ["as the ", "as ", "for the ", "with the ", "with "]
        .iter()
        .find_map(|l| after_noun.strip_prefix(l))
        .and_then(|t| snap.teammate(t.split(' ').next().unwrap_or(t)).map(|m| m.name.clone()));
    // The prompt keeps the user's casing: what follows "to"/"for"/"on"
    // after the noun.
    let noun_at = raw.to_lowercase().find(noun.trim_end_matches('s')).unwrap_or(0);
    let prompt = tail_after(raw, noun_at, &[" to ", " that ", " on ", " about ", " for "])
        .filter(|p| teammate.as_deref().is_none_or(|m| !p.to_lowercase().starts_with(&m.to_lowercase())));
    let worktree = n.contains("worktree") || n.contains("new branch");
    let prompt = prompt.map(|p| {
        let mut p = p;
        for w in [
            " in a new worktree",
            " in their own worktrees",
            " in its own worktree",
            " in a worktree",
            " each in a worktree",
        ] {
            if let Some(s) = p.to_lowercase().strip_suffix(w).map(str::len) {
                p.truncate(s);
            }
        }
        p
    });
    Some(Intent::Launch {
        count: Some(count),
        agent,
        terminal: *terminal,
        prompt,
        prompts: Vec::new(),
        teammate,
        worktree,
    })
}

/// "what's the team doing", "how's the reviewer doing", "ask the builder to
/// add tests", "tell reviewer the API changed".
fn team(raw: &str, n: &str, snap: &Snapshot) -> Option<Intent> {
    const ALL: &[&str] = &[
        "what's the team doing",
        "what is the team doing",
        "how's the team",
        "how's the team doing",
        "how is the team doing",
        "team status",
        "team update",
        "what's everyone doing",
        "what is everyone doing",
        "what's everyone working on",
        "how's everyone doing",
        "check on the team",
        "check in with the team",
    ];
    if ALL.contains(&n) {
        return Some(Intent::TeamStatus { teammate: None });
    }
    if snap.teammates.is_empty() {
        return None;
    }
    // "how's the reviewer doing", "what is builder working on"
    for lead in ["how's", "how is", "what's", "what is", "check on", "check in with"] {
        if let Some(rest) = after(n, lead) {
            let who = rest
                .trim_end_matches(" doing")
                .trim_end_matches(" working on")
                .trim_end_matches(" up to")
                .trim_end_matches(" getting on");
            if let Some(m) = snap.teammate(who) {
                return Some(Intent::TeamStatus { teammate: Some(m.name.clone()) });
            }
        }
    }
    // "ask the reviewer to look at the diff"
    for lead in ["ask", "tell", "message", "ping", "have"] {
        let Some(rest) = after(n, lead) else { continue };
        let rest = rest.strip_prefix("the ").unwrap_or(rest);
        let name = rest.split(' ').next().unwrap_or_default();
        let Some(m) = snap.teammate(name) else { continue };
        // Keep the user's words after the name.
        let lower = raw.to_lowercase();
        let at = lower.find(&m.name.to_lowercase()).map(|i| i + m.name.len()).unwrap_or(0);
        let text = raw.get(at..).unwrap_or_default().trim_start_matches([',', ':', ' ']);
        let text = ["to ", "about ", "that "].iter().find_map(|l| text.strip_prefix(l)).unwrap_or(text);
        let text = text.trim().trim_end_matches(['.', '!']).trim();
        if text.is_empty() {
            return None;
        }
        return Some(Intent::AskTeammate { teammate: m.name.clone(), text: text.to_string() });
    }
    None
}

/// "draft for codex: run the tests", "type npm test into the codex terminal",
/// "draft: add a changelog entry".
fn draft(raw: &str, n: &str, snap: &Snapshot) -> Option<Intent> {
    const LEADS: &[&str] = &["draft", "write", "type", "prepare", "put"];
    let lead = LEADS.iter().find(|l| n == **l || n.starts_with(&format!("{l} ")))?;
    let lower = raw.to_lowercase();
    // "draft for codex: text" / "draft: text"
    if let Some(colon) = raw.find([':', ',']) {
        let head = crate::fastpath::normalize(&raw[..colon]);
        let text = raw[colon + 1..].trim().trim_end_matches(['.', '!']).trim().to_string();
        if text.is_empty() {
            return None;
        }
        let who = head.strip_prefix(lead).map(str::trim).unwrap_or_default();
        let who = ["for ", "to ", "in ", "into "].iter().find_map(|c| who.strip_prefix(c)).unwrap_or(who);
        if who.is_empty() {
            return Some(Intent::Draft { thread: None, text });
        }
        let id = names_thread(snap, who.trim_end_matches(" terminal").trim_end_matches(" thread"))?;
        return Some(Intent::Draft { thread: Some(id), text });
    }
    // "type npm test into the codex terminal", "write a fix in the claude
    // box". Without "into", only a named input counts: "write tests for
    // the parser" is work for the current thread.
    for c in [" into the ", " into ", " in the "] {
        let Some(at) = lower.rfind(c) else { continue };
        let tail = crate::fastpath::normalize(&raw[at + c.len()..]);
        let named_input = ["terminal", "box", "input", "prompt", "chat"].iter().any(|s| tail.ends_with(s));
        if !c.contains("into") && !named_input {
            continue;
        }
        let who = ["terminal", "thread", "chat", "box", "input", "prompt"]
            .iter()
            .fold(tail.as_str(), |w, s| w.trim_end_matches(s).trim())
            .to_string();
        let Some(id) = names_thread(snap, &who) else { continue };
        let start = lower.find(lead).map(|i| i + lead.len()).unwrap_or(0);
        let text = raw[start..at].trim().trim_end_matches(['.', '!']).trim().to_string();
        if text.is_empty() {
            return None;
        }
        return Some(Intent::Draft { thread: Some(id), text });
    }
    None
}

/// "open the tests thread beside this", "split with codex".
fn beside(n: &str, snap: &Snapshot) -> Option<Intent> {
    if let Some(rest) = after(n, "split with").or_else(|| after(n, "split view with"))
        && let Some(id) = names_thread(snap, rest)
    {
        return Some(Intent::OpenBeside { thread: id });
    }
    const SUFFIXES: &[&str] = &[
        " beside this",
        " beside it",
        " beside me",
        " beside",
        " side by side",
        " next to this",
        " next to it",
        " alongside this",
        " alongside",
        " in split view",
        " in a split",
        " on the side",
    ];
    for lead in ["open", "show", "put", "bring up", "pull up", "show me"] {
        let Some(rest) = after(n, lead) else { continue };
        let Some(who) = SUFFIXES.iter().find_map(|s| rest.strip_suffix(s)) else { continue };
        if let Some(id) = names_thread(snap, who) {
            return Some(Intent::OpenBeside { thread: id });
        }
    }
    None
}

pub fn route(text: &str, snap: &Snapshot) -> Option<Intent> {
    let raw = text.trim();
    let n = crate::fastpath::normalize(raw);
    if n.is_empty() {
        return None;
    }
    if let Some(i) = launch(raw, &n, snap)
        .or_else(|| team(raw, &n, snap))
        .or_else(|| beside(&n, snap))
        .or_else(|| draft(raw, &n, snap))
    {
        return Some(i);
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

    if let Some(intent) = look(&n, snap) {
        return Some(intent);
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
    fn pointing_phrases_attach_what_the_user_sees() {
        let look = |text: &str, showing| {
            let mut s = snapshot();
            s.showing = showing;
            route(text, &s)
        };
        let at = |target| Some(Intent::LookAt { target });
        assert_eq!(look("Take a look at this.", None), at(LookTarget::Window));
        assert_eq!(look("Take a look at this", Some(LookTarget::Browser)), at(LookTarget::Browser));
        assert_eq!(look("Look at the diff", Some(LookTarget::Browser)), at(LookTarget::Diff));
        assert_eq!(look("Check out this page.", None), at(LookTarget::Browser));
        assert_eq!(look("Can you see my screen?", None), at(LookTarget::Window));
        assert_eq!(look("Look at this window please", None), at(LookTarget::Window));
        // Requests about code are the agent's, not captures.
        assert_eq!(look("Check the screen size handling", None), None);
        assert_eq!(look("Look at the tests and fix them", None), None);
        assert_eq!(look("Check out the main branch", None), None);
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

    #[test]
    fn launches_open_several_threads_or_terminals() {
        let Some(Action::Launch { items, .. }) = act("Launch three Claude chats to review the auth code") else {
            panic!("not a launch")
        };
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|i| i.agent == "claude" && !i.terminal));
        assert_eq!(items[0].prompt.as_deref(), Some("review the auth code"), "the user's casing is kept");
        let Some(Action::Launch { items, .. }) = act("open two codex terminals") else { panic!() };
        assert!(items.len() == 2 && items.iter().all(|i| i.terminal && i.agent == "codex" && i.prompt.is_none()));
        let Some(Action::Launch { items, .. }) = act("spin up a terminal as the builder") else { panic!() };
        assert_eq!((items.len(), items[0].teammate.as_deref()), (1, Some("m2")));
        // One chat is an ordinary start, not a launch.
        assert!(!matches!(act("start a claude chat"), Some(Action::Launch { .. })));
        let s = snapshot();
        assert_eq!(
            route("launch 12 terminals", &s).map(|i| validate(i, &s)),
            Some(Err(crate::intent::Rejection::TooMany(12)))
        );
    }

    #[test]
    fn the_team_answers_and_gets_asked() {
        assert_eq!(act("What's the team doing?"), Some(Action::TeamStatus { teammate: None }));
        assert_eq!(act("How's the reviewer doing?"), Some(Action::TeamStatus { teammate: Some("m1".into()) }));
        assert_eq!(
            act("Ask the Reviewer to look at the login diff."),
            Some(Action::AskTeammate { teammate: "m1".into(), text: "look at the login diff".into() })
        );
        assert_eq!(
            act("tell builder the API changed"),
            Some(Action::AskTeammate { teammate: "m2".into(), text: "the API changed".into() })
        );
    }

    #[test]
    fn drafts_go_into_an_input_unsent() {
        assert_eq!(
            act("Draft for Codex: run the tests on Windows"),
            Some(Action::Draft { thread: "t2".into(), text: "run the tests on Windows".into() })
        );
        assert_eq!(
            act("type cargo test into the codex terminal"),
            Some(Action::Draft { thread: "t2".into(), text: "cargo test".into() })
        );
        assert_eq!(
            act("draft: add a changelog entry"),
            Some(Action::Draft { thread: "t1".into(), text: "add a changelog entry".into() })
        );
        // Work for the agent, not a draft.
        assert_eq!(route("write tests for the auth refresh", &snapshot()), None);
    }

    #[test]
    fn threads_open_beside_the_current_one() {
        assert_eq!(act("open the tests thread beside this"), Some(Action::OpenBeside { thread: "t2".into() }));
        assert_eq!(act("split with gemini"), Some(Action::OpenBeside { thread: "t3".into() }));
    }
}
