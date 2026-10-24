//! What the conductor knows about Dove right now, harness by harness.
//!
//! The app builds a [`Snapshot`] from state it already holds (task rows,
//! live sessions, todos, diff stats), so status answers never need a round
//! trip to an agent and never contain anything Dove has not observed.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStatus {
    Idle,
    Running,
    NeedsInput,
    Done,
    Failed,
}

impl ThreadStatus {
    pub fn spoken(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "working",
            Self::NeedsInput => "waiting for you",
            Self::Done => "done",
            Self::Failed => "stopped with an error",
        }
    }
}

/// A permission or question a thread is waiting on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub request_id: String,
    pub tool: String,
    pub summary: String,
    #[serde(default)]
    pub input: serde_json::Value,
    /// An agent question rather than a tool approval.
    #[serde(default)]
    pub question: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffStat {
    pub files: u32,
    pub added: u32,
    pub removed: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    pub id: String,
    pub title: String,
    /// The agent's display name ("Claude Code", "Codex").
    pub agent: String,
    /// Stable harness id ("claude", "codex").
    pub agent_id: String,
    pub project: String,
    pub status: ThreadStatus,
    /// A live session is attached: messages and interrupts reach the agent now.
    pub live: bool,
    /// A message sent mid-turn steers the running turn.
    pub steers: bool,
    pub interrupts: bool,
    /// The harness has a separate approve-for-session answer.
    pub session_approvals: bool,
    #[serde(default)]
    pub pending: Vec<Pending>,
    /// The harness's latest status line ("Writing main.rs").
    #[serde(default)]
    pub activity: Option<String>,
    /// Checklist progress: (completed, total, current item).
    #[serde(default)]
    pub todos: Option<(u32, u32, Option<String>)>,
    #[serde(default)]
    pub diff: Option<DiffStat>,
    /// The thread's worktree or folder, for "outside the worktree" checks.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Last error the thread reported.
    #[serde(default)]
    pub error: Option<String>,
    /// The teammate running it, by name.
    #[serde(default)]
    pub teammate: Option<String>,
    /// It shows the agent's own terminal UI (drafts go to its prompt line).
    #[serde(default)]
    pub terminal: bool,
}

/// One of the user's teammates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Teammate {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub threads: Vec<Thread>,
    /// The thread the call is talking to.
    pub current: Option<String>,
    /// Installed agents that can start a task.
    pub agents: Vec<Agent>,
    pub projects: Vec<Project>,
    /// The project new tasks go to when the user names none.
    pub default_project: Option<String>,
    /// The agent picked in the composer. With no thread open, a request
    /// starts a new thread on it (with the picker's model and mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,
    /// What Dove shows beside the thread (the side panel's open tab), which
    /// "take a look at this" points at. `None` means another app's window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub showing: Option<crate::intent::LookTarget>,
    #[serde(default)]
    pub teammates: Vec<Teammate>,
}

impl Snapshot {
    pub fn thread(&self, id: &str) -> Option<&Thread> {
        self.threads.iter().find(|t| t.id == id)
    }

    pub fn current_thread(&self) -> Option<&Thread> {
        self.current.as_deref().and_then(|id| self.thread(id))
    }

    /// The teammate a spoken name means (exact, then a unique prefix).
    pub fn teammate(&self, name: &str) -> Option<&Teammate> {
        let n = crate::fastpath::normalize(name);
        let n = n.trim_start_matches("the ").trim();
        if n.is_empty() {
            return None;
        }
        let exact = self.teammates.iter().find(|t| {
            crate::fastpath::normalize(&t.name) == n || t.id == name || crate::fastpath::normalize(&t.role) == n
        });
        exact.or_else(|| {
            let mut hits = self.teammates.iter().filter(|t| crate::fastpath::normalize(&t.name).starts_with(n));
            match (hits.next(), hits.next()) {
                (Some(t), None) if n.len() >= 3 => Some(t),
                _ => None,
            }
        })
    }

    /// A teammate's threads, newest first as the snapshot lists them.
    pub fn threads_of(&self, teammate: &Teammate) -> Vec<&Thread> {
        self.threads.iter().filter(|t| t.teammate.as_deref() == Some(teammate.name.as_str())).collect()
    }

    /// Threads waiting on the user, current one first.
    pub fn waiting(&self) -> Vec<&Thread> {
        let mut out: Vec<&Thread> = self.threads.iter().filter(|t| !t.pending.is_empty()).collect();
        out.sort_by_key(|t| Some(t.id.as_str()) != self.current.as_deref());
        out
    }
}

/// How well `query` names `thread`: agent name and title words.
fn score(thread: &Thread, query: &str) -> u32 {
    let q = crate::fastpath::normalize(query);
    let words: Vec<&str> = q
        .split(' ')
        .filter(|w| !matches!(*w, "the" | "a" | "an" | "thread" | "task" | "one" | "chat" | "on" | "about"))
        .collect();
    if words.is_empty() {
        return 0;
    }
    let title = crate::fastpath::normalize(&thread.title);
    let agent = crate::fastpath::normalize(&thread.agent);
    let mate = thread.teammate.as_deref().map(crate::fastpath::normalize).unwrap_or_default();
    let mut s = 0;
    for w in &words {
        if agent.split(' ').any(|a| a == *w) || thread.agent_id == *w {
            s += 3;
        }
        if !mate.is_empty() && mate.split(' ').any(|m| m == *w) {
            s += 3;
        }
        if title.split(' ').any(|t| t == *w || (w.len() >= 4 && t.starts_with(w))) {
            s += 2;
        }
    }
    s
}

/// Resolve a spoken thread name. `Err` lists the tied candidates (or none).
pub fn resolve<'a>(snap: &'a Snapshot, query: &str) -> Result<&'a Thread, Vec<&'a Thread>> {
    if let Some(t) = snap.thread(query) {
        return Ok(t);
    }
    let mut scored: Vec<(u32, &Thread)> =
        snap.threads.iter().map(|t| (score(t, query), t)).filter(|(s, _)| *s > 0).collect();
    scored.sort_by_key(|s| std::cmp::Reverse(s.0));
    match scored.as_slice() {
        [] => Err(Vec::new()),
        [(_, t)] => Ok(t),
        [(a, t), (b, _), ..] if a > b => Ok(t),
        _ => {
            let top = scored[0].0;
            // A tie that includes the current thread resolves to it.
            if let Some((_, t)) =
                scored.iter().find(|(s, t)| *s == top && Some(t.id.as_str()) == snap.current.as_deref())
            {
                return Ok(t);
            }
            Err(scored.into_iter().filter(|(s, _)| *s == top).map(|(_, t)| t).collect())
        }
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub fn thread(id: &str, title: &str, agent: &str, agent_id: &str, status: ThreadStatus) -> Thread {
        Thread {
            id: id.into(),
            title: title.into(),
            agent: agent.into(),
            agent_id: agent_id.into(),
            project: "dove".into(),
            status,
            live: true,
            steers: true,
            interrupts: true,
            session_approvals: true,
            pending: Vec::new(),
            activity: None,
            todos: None,
            diff: None,
            cwd: Some("/work/dove".into()),
            error: None,
            teammate: None,
            terminal: false,
        }
    }

    pub fn snapshot() -> Snapshot {
        Snapshot {
            threads: vec![
                thread("t1", "Fix the auth token refresh", "Claude Code", "claude", ThreadStatus::Running),
                thread("t2", "Make the tests pass on Windows", "Codex", "codex", ThreadStatus::Running),
                thread("t3", "Landing page hero copy", "Gemini", "gemini", ThreadStatus::Done),
            ],
            current: Some("t1".into()),
            agents: vec![
                Agent { id: "claude".into(), label: "Claude Code".into() },
                Agent { id: "codex".into(), label: "Codex".into() },
                Agent { id: "gemini".into(), label: "Gemini".into() },
            ],
            projects: vec![Project { id: "p1".into(), name: "dove".into() }],
            default_project: Some("p1".into()),
            default_agent: Some("claude".into()),
            showing: None,
            teammates: vec![
                Teammate { id: "m1".into(), name: "Reviewer".into(), role: "Code & security auditor".into() },
                Teammate { id: "m2".into(), name: "Builder".into(), role: "Full-stack engineer".into() },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn threads_resolve_by_agent_or_title_words() {
        let s = snapshot();
        assert_eq!(resolve(&s, "Codex").unwrap().id, "t2");
        assert_eq!(resolve(&s, "the tests thread").unwrap().id, "t2");
        assert_eq!(resolve(&s, "auth fix").unwrap().id, "t1");
        assert_eq!(resolve(&s, "landing page").unwrap().id, "t3");
        assert_eq!(resolve(&s, "Claude").unwrap().id, "t1");
        assert!(resolve(&s, "kubernetes").unwrap_err().is_empty());
    }

    #[test]
    fn ties_are_reported_unless_one_is_current() {
        let mut s = snapshot();
        s.threads.push(thread("t4", "Refactor the auth module", "Codex", "codex", ThreadStatus::Idle));
        // "codex" names t2 and t4 equally.
        let tied = resolve(&s, "codex").unwrap_err();
        assert_eq!(tied.len(), 2);
        s.current = Some("t4".into());
        assert_eq!(resolve(&s, "codex").unwrap().id, "t4");
    }
}
