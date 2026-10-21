//! The call's brain: one utterance in, effects out.
//!
//! The conductor is harness-neutral. It reads a [`Snapshot`] and returns
//! [`Effect`]s; the app performs them through the same paths a click uses
//! (send, interrupt, approve), so permissions and transcripts behave exactly
//! as they do without voice. Everything it says about a thread comes from
//! the snapshot: it never reports progress Dove has not observed.

use std::collections::HashMap;

use crate::fastpath::{self, FastCommand};
use crate::intent::{self, Action, Intent, Scope};
use crate::rules;
use crate::safety::{self, Risk};
use crate::snapshot::{Snapshot, Thread, ThreadStatus};

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Speak(String),
    /// Stop Dove's speech now (the agent keeps working).
    StopSpeaking,
    MuteMic(bool),
    SetRate(f32),
    SetStyle(String),
    /// A checked action for the app to perform.
    Act(Action),
    /// A destructive request the user confirmed by voice: the app highlights
    /// its Approve button. Voice never sends this decision itself.
    AwaitClick {
        thread: String,
        request_id: String,
    },
    /// Ask the configured model router; its JSON comes back through
    /// [`Conductor::routed`].
    Route(String),
    EndCall,
}

#[derive(Debug, Clone, PartialEq)]
struct Armed {
    thread: String,
    request_id: String,
    phrase: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
struct Seen {
    status: ThreadStatus,
    pending: Option<String>,
}

pub const MIN_RATE: f32 = 0.6;
pub const MAX_RATE: f32 = 1.8;
const RATE_STEP: f32 = 0.15;

#[derive(Debug)]
pub struct Conductor {
    pub rate: f32,
    /// The thread before the last switch, for "take me back".
    previous: Option<String>,
    armed: Option<Armed>,
    last_spoken: Option<String>,
    seen: HashMap<String, Seen>,
    /// Speak when a thread finishes, fails or needs you.
    pub announce: bool,
}

impl Default for Conductor {
    fn default() -> Self {
        Self { rate: 1.0, previous: None, armed: None, last_spoken: None, seen: HashMap::new(), announce: true }
    }
}

fn label(t: &Thread) -> String {
    format!("{} on {}", t.agent, t.title)
}

/// A spoken status for one thread, from observed state only.
pub fn thread_status(t: &Thread) -> String {
    let mut s = format!("{} is {}.", label(t), t.status.spoken());
    if let Some(p) = t.pending.first() {
        if p.question {
            s.push_str(" It has a question for you in the chat.");
        } else {
            s.push_str(&format!(" It's waiting for approval to {}.", lower_first(&p.summary)));
        }
    }
    if t.status == ThreadStatus::Running
        && let Some(a) = t.activity.as_deref().filter(|a| !a.trim().is_empty())
    {
        s.push_str(&format!(" Right now: {}.", a.trim().trim_end_matches('.')));
    }
    if let Some((done, total, current)) = &t.todos
        && *total > 0
    {
        s.push_str(&format!(" {done} of {total} steps done"));
        match current {
            Some(c) if *done < *total => s.push_str(&format!(", now {}.", lower_first(c.trim_end_matches('.')))),
            _ => s.push('.'),
        }
    }
    if let Some(d) = t.diff.filter(|d| d.files > 0) {
        s.push_str(&format!(
            " {} {} changed, {} lines added and {} removed.",
            d.files,
            if d.files == 1 { "file" } else { "files" },
            d.added,
            d.removed
        ));
    }
    if t.status == ThreadStatus::Failed
        && let Some(e) = &t.error
    {
        s.push_str(&format!(" The error was: {}.", crate::speakable::speakable_line(e).trim_end_matches('.')));
    }
    s
}

/// Every thread at once: counts, then the ones that need attention.
pub fn overall_status(snap: &Snapshot) -> String {
    if snap.threads.is_empty() {
        return "No threads are open.".into();
    }
    let count = |st: ThreadStatus| snap.threads.iter().filter(|t| t.status == st).count();
    let mut parts = Vec::new();
    for (st, word) in [
        (ThreadStatus::Running, "working"),
        (ThreadStatus::NeedsInput, "waiting for you"),
        (ThreadStatus::Failed, "failed"),
        (ThreadStatus::Done, "done"),
    ] {
        let n = count(st);
        if n > 0 {
            parts.push(format!("{n} {word}"));
        }
    }
    let mut s = if parts.is_empty() { "Everything is idle.".to_string() } else { format!("{}.", parts.join(", ")) };
    let mut attention: Vec<&Thread> = snap
        .threads
        .iter()
        .filter(|t| matches!(t.status, ThreadStatus::NeedsInput | ThreadStatus::Failed | ThreadStatus::Running))
        .collect();
    attention.sort_by_key(|t| match t.status {
        ThreadStatus::NeedsInput => 0,
        ThreadStatus::Failed => 1,
        _ => 2,
    });
    for t in attention.iter().take(4) {
        s.push_str(&format!(" {} is {}.", label(t), t.status.spoken()));
    }
    s
}

fn lower_first(s: &str) -> String {
    let mut c = s.trim().chars();
    match c.next() {
        // Keep acronyms and names ("CI", "README") as they are.
        Some(f) if c.clone().next().is_some_and(|n| n.is_lowercase()) => f.to_lowercase().chain(c).collect(),
        Some(f) => std::iter::once(f).chain(c).collect(),
        None => String::new(),
    }
}

impl Conductor {
    pub fn new() -> Self {
        Self::default()
    }

    fn say(&mut self, out: &mut Vec<Effect>, text: impl Into<String>) {
        let text = text.into();
        self.last_spoken = Some(text.clone());
        out.push(Effect::Speak(text));
    }

    /// The user said `text` (a final transcript).
    pub fn utterance(&mut self, text: &str, snap: &Snapshot, router: bool) -> Vec<Effect> {
        let mut out = Vec::new();
        let n = fastpath::normalize(text);
        if n.is_empty() {
            return out;
        }
        // The confirmation phrase for an armed destructive request.
        if let Some(armed) = self.armed.clone()
            && n == armed.phrase
        {
            if self.still_waiting(snap, &armed) {
                self.say(&mut out, "Confirmed. Now click Approve to run it.");
                out.push(Effect::AwaitClick { thread: armed.thread, request_id: armed.request_id });
            } else {
                self.armed = None;
                self.say(&mut out, "That request is no longer waiting, so there's nothing to confirm.");
            }
            return out;
        }
        // A confirmation with nothing armed is never passed to an agent as
        // an instruction.
        if safety::CONFIRM_PHRASES.contains(&n.as_str()) {
            let text = match &self.armed {
                Some(a) => format!("To confirm this one, say \"{}\".", a.phrase),
                None => "There's nothing waiting for that confirmation.".into(),
            };
            self.say(&mut out, text);
            return out;
        }
        if let Some(cmd) = fastpath::parse(text) {
            self.fast(cmd, snap, &mut out);
            return out;
        }
        if let Some(i) = rules::route(text, snap) {
            self.intent(i, snap, &mut out);
            return out;
        }
        if router {
            out.push(Effect::Route(text.trim().to_string()));
            return out;
        }
        self.intent(Intent::Send { thread: None, text: text.trim().to_string() }, snap, &mut out);
        out
    }

    /// A model router answered for `utterance`.
    pub fn routed(&mut self, utterance: &str, json: &str, snap: &Snapshot) -> Vec<Effect> {
        let mut out = Vec::new();
        match intent::parse(json) {
            Ok(i) => self.intent(i, snap, &mut out),
            // A router that cannot answer cleanly is not obeyed; the words
            // go to the current thread as the user said them.
            Err(_) => self.intent(Intent::Send { thread: None, text: utterance.trim().to_string() }, snap, &mut out),
        }
        out
    }

    fn still_waiting(&self, snap: &Snapshot, armed: &Armed) -> bool {
        snap.thread(&armed.thread).is_some_and(|t| t.pending.iter().any(|p| p.request_id == armed.request_id))
    }

    fn fast(&mut self, cmd: FastCommand, snap: &Snapshot, out: &mut Vec<Effect>) {
        match cmd {
            FastCommand::Stop => {
                out.push(Effect::StopSpeaking);
                match intent::validate(Intent::Interrupt { thread: None }, snap) {
                    Ok(a) => {
                        out.push(Effect::Act(a));
                        self.say(out, "Stopping.");
                    }
                    Err(_) => self.say(out, "Okay."),
                }
            }
            FastCommand::Approve { session } => {
                let scope = if session { Scope::Session } else { Scope::Once };
                self.intent(Intent::Approve { thread: None, scope }, snap, out);
            }
            FastCommand::Deny => self.intent(Intent::Deny { thread: None }, snap, out),
            FastCommand::MuteMic => {
                out.push(Effect::MuteMic(true));
                self.say(out, "Muted. Hold space to talk.");
            }
            FastCommand::UnmuteMic => {
                out.push(Effect::MuteMic(false));
                self.say(out, "I'm listening.");
            }
            FastCommand::Silence => out.push(Effect::StopSpeaking),
            FastCommand::Faster | FastCommand::Slower => {
                let step = if cmd == FastCommand::Faster { RATE_STEP } else { -RATE_STEP };
                self.rate = (self.rate + step).clamp(MIN_RATE, MAX_RATE);
                out.push(Effect::SetRate(self.rate));
                self.say(out, if cmd == FastCommand::Faster { "Okay, faster." } else { "Okay, slower." });
            }
            FastCommand::Status { all } => self.intent(Intent::Status { thread: None, all }, snap, out),
            FastCommand::ShowDiff => self.intent(Intent::Show { panel: intent::Panel::Diff }, snap, out),
            FastCommand::SwitchBack => self.intent(Intent::SwitchBack, snap, out),
            FastCommand::EndCall => self.intent(Intent::EndCall, snap, out),
            FastCommand::Repeat => match self.last_spoken.clone() {
                Some(s) => out.push(Effect::Speak(s)),
                None => self.say(out, "I haven't said anything yet."),
            },
        }
    }

    fn intent(&mut self, i: Intent, snap: &Snapshot, out: &mut Vec<Effect>) {
        let action = match i {
            Intent::SwitchBack => match self.previous.clone() {
                Some(p) if snap.thread(&p).is_some() => Action::TalkTo { thread: p },
                _ => return self.say(out, "There's no earlier thread to go back to."),
            },
            i => match intent::validate(i, snap) {
                Ok(a) => a,
                Err(r) => return self.say(out, r.spoken()),
            },
        };
        match action {
            Action::Approve { ref thread, ref request_id, .. } => {
                let t = snap.thread(thread).expect("validated");
                let req = t.pending.iter().find(|p| &p.request_id == request_id).expect("validated");
                match safety::classify(req, t.cwd.as_deref()) {
                    Risk::Normal => {
                        self.armed = None;
                        out.push(Effect::Act(action.clone()));
                        self.say(out, "Approved.");
                    }
                    risk @ Risk::Destructive(_) => {
                        let phrase = safety::confirm_phrase(&risk);
                        let Risk::Destructive(reasons) = &risk else { unreachable!() };
                        self.armed = Some(Armed { thread: thread.clone(), request_id: request_id.clone(), phrase });
                        let text = format!(
                            "I won't approve that by voice: {}. If you want it, say \"{phrase}\", then click Approve.",
                            reasons.join(", and ")
                        );
                        self.say(out, text);
                    }
                }
            }
            Action::Deny { .. } => {
                self.armed = None;
                out.push(Effect::Act(action));
                self.say(out, "Denied.");
            }
            Action::Status { thread } => {
                let text = match thread.as_deref().and_then(|id| snap.thread(id)) {
                    Some(t) => thread_status(t),
                    None => overall_status(snap),
                };
                self.say(out, text);
            }
            Action::TalkTo { ref thread } => {
                if snap.current.as_deref() != Some(thread.as_str()) {
                    self.previous = snap.current.clone();
                }
                let t = snap.thread(thread).expect("validated");
                let text = format!("Talking to {}.", label(t));
                out.push(Effect::Act(action.clone()));
                self.say(out, text);
            }
            Action::Interrupt { .. } => {
                out.push(Effect::StopSpeaking);
                out.push(Effect::Act(action));
                self.say(out, "Stopping it.");
            }
            Action::Send { .. } => out.push(Effect::Act(action)),
            Action::StartTask { ref agent, ref prompt, .. } => {
                let who = snap.agents.iter().find(|a| &a.id == agent).map(|a| a.label.clone()).unwrap_or_default();
                let text = format!("Starting a {who} task: {}.", prompt.trim_end_matches('.'));
                if snap.current.is_some() {
                    self.previous = snap.current.clone();
                }
                out.push(Effect::Act(action.clone()));
                self.say(out, text);
            }
            Action::Show { panel } => {
                out.push(Effect::Act(action));
                self.say(out, format!("Opening the {}.", format!("{panel:?}").to_lowercase()));
            }
            Action::LookAt { .. } => {
                out.push(Effect::Act(action));
                self.say(out, "Okay, I'll attach that to your next message.");
            }
            Action::SetVoiceStyle { rate, style } => {
                if let Some(r) = rate {
                    self.rate = r.clamp(MIN_RATE, MAX_RATE);
                    out.push(Effect::SetRate(self.rate));
                }
                if let Some(s) = style {
                    out.push(Effect::SetStyle(s));
                }
                self.say(out, "Okay.");
            }
            Action::EndCall => {
                out.push(Effect::StopSpeaking);
                out.push(Effect::EndCall);
            }
            Action::SwitchBack => {}
        }
    }

    /// Compare with the last snapshot and announce what changed. The first
    /// call only records state.
    pub fn observe(&mut self, snap: &Snapshot) -> Vec<Effect> {
        let mut out = Vec::new();
        let first = self.seen.is_empty();
        // A confirmed-but-unclicked request that changed or went away.
        if let Some(armed) = self.armed.clone()
            && !self.still_waiting(snap, &armed)
        {
            self.armed = None;
            if self.announce {
                self.say(&mut out, "That request changed, so I dropped the confirmation.");
            }
        }
        for t in &snap.threads {
            let now = Seen { status: t.status, pending: t.pending.first().map(|p| p.request_id.clone()) };
            let before = self.seen.insert(t.id.clone(), now.clone());
            if first || !self.announce {
                continue;
            }
            let Some(before) = before else { continue };
            if now.pending.is_some() && now.pending != before.pending {
                let p = &t.pending[0];
                let text = if p.question {
                    format!("{} has a question for you.", label(t))
                } else {
                    match safety::classify(p, t.cwd.as_deref()) {
                        Risk::Normal => format!("{} wants to {}. Approve or deny?", label(t), lower_first(&p.summary)),
                        Risk::Destructive(r) => format!(
                            "{} wants to {}. Careful: {}. That one needs a click.",
                            label(t),
                            lower_first(&p.summary),
                            r.join(", and ")
                        ),
                    }
                };
                self.say(&mut out, text);
            } else if now.status != before.status {
                match now.status {
                    ThreadStatus::Done => self.say(&mut out, format!("{} finished.", label(t))),
                    ThreadStatus::Failed => self.say(&mut out, format!("{} stopped with an error.", label(t))),
                    _ => {}
                }
            }
        }
        self.seen.retain(|id, _| snap.threads.iter().any(|t| &t.id == id));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::fixtures::*;
    use crate::snapshot::{DiffStat, Pending};
    use serde_json::json;

    fn bash(id: &str, cmd: &str) -> Pending {
        Pending {
            request_id: id.into(),
            tool: "Bash".into(),
            summary: format!("Run {cmd}"),
            input: json!({"command": cmd}),
            question: false,
        }
    }

    fn acts(effects: &[Effect]) -> Vec<&Action> {
        effects.iter().filter_map(|e| if let Effect::Act(a) = e { Some(a) } else { None }).collect()
    }

    #[test]
    fn plain_words_go_to_the_current_thread() {
        let mut c = Conductor::new();
        let e = c.utterance("Add a retry to the refresh call", &snapshot(), false);
        assert_eq!(
            acts(&e),
            vec![&Action::Send { thread: "t1".into(), text: "Add a retry to the refresh call".into(), steer: true }]
        );
        // With a model router configured, unknown requests go to it instead.
        let e = c.utterance("Add a retry to the refresh call", &snapshot(), true);
        assert_eq!(e, vec![Effect::Route("Add a retry to the refresh call".into())]);
    }

    #[test]
    fn normal_approvals_go_through_destructive_ones_never_do() {
        let mut s = snapshot();
        s.threads[0].pending.push(bash("ok1", "cargo test"));
        let mut c = Conductor::new();
        let e = c.utterance("approve", &s, false);
        assert_eq!(
            acts(&e),
            vec![&Action::Approve { thread: "t1".into(), request_id: "ok1".into(), scope: Scope::Once }]
        );

        s.threads[0].pending = vec![bash("bad1", "rm -rf ~/projects")];
        for attempt in ["approve", "yes, go ahead", "approve for this session", "always allow"] {
            let e = c.utterance(attempt, &s, false);
            assert!(acts(&e).is_empty(), "{attempt} approved a destructive request: {e:?}");
        }
        // Even through a model router.
        let e = c.routed("just do it", r#"{"intent":"approve","scope":"once"}"#, &s);
        assert!(acts(&e).is_empty());
        // The confirmation phrase arms a click; it still sends nothing.
        let e = c.utterance("Confirm delete.", &s, false);
        assert!(acts(&e).is_empty());
        assert!(e.contains(&Effect::AwaitClick { thread: "t1".into(), request_id: "bad1".into() }));
        // A wrong phrase never arms.
        let mut c2 = Conductor::new();
        c2.utterance("approve", &s, false);
        let e = c2.utterance("confirm approval", &s, false);
        assert!(!e.iter().any(|e| matches!(e, Effect::AwaitClick { .. })));
    }

    #[test]
    fn a_changed_request_drops_the_confirmation() {
        let mut s = snapshot();
        s.threads[0].pending = vec![bash("bad1", "git push --force")];
        let mut c = Conductor::new();
        c.observe(&s);
        c.utterance("approve", &s, false);
        // The agent replaces the request before the user confirms.
        s.threads[0].pending = vec![bash("bad2", "git push --force origin main")];
        let e = c.observe(&s);
        assert!(e.iter().any(|e| matches!(e, Effect::Speak(t) if t.contains("dropped the confirmation"))));
        let e = c.utterance("confirm force push", &s, false);
        assert!(!e.iter().any(|e| matches!(e, Effect::AwaitClick { .. })));
        assert!(acts(&e).is_empty());
    }

    #[test]
    fn deny_and_stop_need_something_to_act_on() {
        let mut s = snapshot();
        let mut c = Conductor::new();
        let e = c.utterance("deny", &s, false);
        assert!(acts(&e).is_empty());
        s.threads[0].pending.push(bash("r1", "npm publish"));
        let e = c.utterance("deny", &s, false);
        assert_eq!(acts(&e), vec![&Action::Deny { thread: "t1".into(), request_id: "r1".into() }]);
        let e = c.utterance("stop", &s, false);
        assert_eq!(acts(&e), vec![&Action::Interrupt { thread: "t1".into() }]);
        assert_eq!(e[0], Effect::StopSpeaking);
        s.threads[0].status = ThreadStatus::Done;
        s.threads[0].pending.clear();
        let e = c.utterance("stop", &s, false);
        assert!(acts(&e).is_empty());
    }

    #[test]
    fn status_reports_only_observed_state() {
        let mut s = snapshot();
        let t = &mut s.threads[0];
        t.activity = Some("Writing src/auth.rs".into());
        t.todos = Some((2, 5, Some("Add refresh tests".into())));
        t.diff = Some(DiffStat { files: 3, added: 120, removed: 4 });
        let mut c = Conductor::new();
        let e = c.utterance("What's happening?", &s, false);
        assert_eq!(
            e,
            vec![Effect::Speak(
                "Claude Code on Fix the auth token refresh is working. Right now: Writing src/auth.rs. 2 of 5 steps done, now add refresh tests. 3 files changed, 120 lines added and 4 removed.".into()
            )]
        );
        // Nothing known, nothing invented.
        let bare = thread_status(&s.threads[2]);
        assert_eq!(bare, "Gemini on Landing page hero copy is done.");
        let all = overall_status(&s);
        assert!(all.starts_with("2 working, 1 done."), "{all}");
    }

    #[test]
    fn switching_threads_remembers_the_way_back() {
        let mut s = snapshot();
        let mut c = Conductor::new();
        let e = c.utterance("Talk to the tests thread", &s, false);
        assert_eq!(acts(&e), vec![&Action::TalkTo { thread: "t2".into() }]);
        s.current = Some("t2".into());
        let e = c.utterance("take me back", &s, false);
        assert_eq!(acts(&e), vec![&Action::TalkTo { thread: "t1".into() }]);
        let mut fresh = Conductor::new();
        let e = fresh.utterance("take me back", &s, false);
        assert!(acts(&e).is_empty());
    }

    #[test]
    fn rate_is_clamped() {
        let mut c = Conductor::new();
        for _ in 0..20 {
            c.utterance("faster", &snapshot(), false);
        }
        assert_eq!(c.rate, MAX_RATE);
        for _ in 0..20 {
            c.utterance("slower", &snapshot(), false);
        }
        assert_eq!(c.rate, MIN_RATE);
    }

    #[test]
    fn announcements_follow_observed_transitions_only() {
        let mut s = snapshot();
        let mut c = Conductor::new();
        assert!(c.observe(&s).is_empty(), "the first look only records");
        assert!(c.observe(&s).is_empty(), "nothing changed");
        s.threads[1].status = ThreadStatus::Done;
        assert_eq!(c.observe(&s), vec![Effect::Speak("Codex on Make the tests pass on Windows finished.".into())]);
        s.threads[0].pending.push(bash("p1", "cargo build"));
        s.threads[0].status = ThreadStatus::NeedsInput;
        let e = c.observe(&s);
        assert_eq!(
            e,
            vec![Effect::Speak(
                "Claude Code on Fix the auth token refresh wants to run cargo build. Approve or deny?".into()
            )]
        );
        s.threads[0].pending = vec![bash("p2", "rm -rf dist")];
        let e = c.observe(&s);
        assert!(matches!(&e[0], Effect::Speak(t) if t.contains("needs a click")));
        c.announce = false;
        s.threads[2].status = ThreadStatus::Failed;
        assert!(c.observe(&s).is_empty());
    }

    #[test]
    fn malformed_router_output_is_not_obeyed() {
        let mut c = Conductor::new();
        let e = c.routed("refactor the parser", r#"{"intent":"kill_everything"}"#, &snapshot());
        assert_eq!(
            acts(&e),
            vec![&Action::Send { thread: "t1".into(), text: "refactor the parser".into(), steer: true }]
        );
    }

    #[test]
    fn end_call_and_repeat() {
        let mut c = Conductor::new();
        let e = c.utterance("hang up", &snapshot(), false);
        assert_eq!(e, vec![Effect::StopSpeaking, Effect::EndCall]);
        let e = c.utterance("repeat that", &snapshot(), false);
        assert_eq!(e, vec![Effect::Speak("I haven't said anything yet.".into())]);
        c.utterance("status", &snapshot(), false);
        let e = c.utterance("say that again", &snapshot(), false);
        assert!(matches!(&e[0], Effect::Speak(t) if t.starts_with("Claude Code on")));
    }
}
