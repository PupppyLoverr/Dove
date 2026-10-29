//! K22: the assistant you reach from the notch, typed (Ask K22) or by voice.
//!
//! K22 is a thread like any other, on the agent and model picked in
//! Settings › Voice (`voice_harness`, `voice_agent_model`, `voice_effort`,
//! `voice_project`), running as a teammate named K22 whose instructions say
//! how to answer: briefly (it's often read aloud), with a `notch_view` when
//! structure helps. One K22 thread a day keeps its context useful without
//! growing forever; a new day (or a new agent) starts a fresh one.

use dove_core::Engine;
use dove_core::proto::{HarnessKind, ProjectId, ReasoningLevel, TaskId, TaskStatus};
use dove_core::store::{Persona, Store};

use super::view::VoiceInfo;

/// `<day>|<thread id>|<agent id>` of today's K22 thread.
const THREAD_KEY: &str = "k22_thread";

/// The agent, model and project K22 runs on.
#[derive(Clone, Debug, PartialEq)]
pub struct VoiceAgent {
    pub kind: HarnessKind,
    pub model: Option<String>,
    pub effort: Option<ReasoningLevel>,
    pub project: Option<ProjectId>,
    /// `concise`, `chatty` or `formal`.
    pub style: String,
}

/// K22's agent: Settings › Voice, else the first installed agent.
pub fn voice_agent(engine: &Engine) -> Option<VoiceAgent> {
    let s = &engine.store;
    let installed: Vec<HarnessKind> = engine.installed().iter().map(|i| i.kind).collect();
    let kind = s
        .setting("voice_harness")
        .and_then(|k| HarnessKind::from_id(&k))
        .filter(|k| installed.contains(k))
        .or_else(|| installed.first().copied())?;
    let model = s.setting("voice_agent_model").filter(|m| !m.trim().is_empty());
    // A model picked for another agent doesn't carry over.
    let model = model.filter(|m| engine.models(kind).iter().any(|x| &x.id == m));
    let effort = s.setting("voice_effort").and_then(|e| ReasoningLevel::from_id(&e)).or(Some(ReasoningLevel::Low));
    let projects = s.projects().unwrap_or_default();
    let project = s
        .setting("voice_project")
        .or_else(|| s.setting("active_project"))
        .and_then(|id| projects.iter().find(|p| p.id.to_string() == id).map(|p| p.id))
        .or_else(|| projects.first().map(|p| p.id));
    let style = s.setting("voice_persona").unwrap_or_else(|| "concise".into());
    Some(VoiceAgent { kind, model, effort, project, style })
}

/// What the notch shows (and offers) for K22's agent and model.
pub fn voice_info(engine: &Engine) -> VoiceInfo {
    let agent = voice_agent(engine);
    let models = agent
        .as_ref()
        .map(|a| engine.models(a.kind).into_iter().map(|m| (m.id, m.label)).collect())
        .unwrap_or_default();
    VoiceInfo { agent: agent.as_ref().map(|a| a.kind), model: agent.and_then(|a| a.model), models }
}

/// Picks K22's agent and model (`None` model: the agent's default).
pub fn set_voice_agent(store: &Store, agent: Option<HarnessKind>, model: Option<String>) {
    if let Some(kind) = agent {
        let _ = store.set_setting("voice_harness", kind.id());
    }
    let _ = store.set_setting("voice_agent_model", model.as_deref().unwrap_or(""));
}

/// How K22 works, as its teammate instructions.
pub fn instructions(style: &str) -> String {
    let style = match style {
        "chatty" => "Warm and conversational, still short.",
        "formal" => "Precise and formal.",
        _ => "Concise: the answer first, in plain words.",
    };
    format!(
        "You are K22, the user's assistant in Keplorayx, the app where they run coding agents. They reach you from \
the notch at the top of their screen, typing or talking. You help them run their agents: see what every thread is \
doing and what needs them (the team tools: thread_list, thread_read, thread_send, task_status), start or steer \
agents, review plans, and answer questions about their work.\n\
Your replies are often read aloud: lead with the answer in one or two plain sentences, with no tables, headings or \
code unless asked. When structure helps (status across agents, what needs the user, a plan, a comparison, next \
steps), call notch_view to show it in the notch, then keep your text to a sentence that doesn't restate it. Prefer \
live blocks (agent, plan) for threads, so the view keeps updating.\n\
Never approve or run anything destructive for the user; offer it as a button they click.\n\
Style: {style}"
    )
}

/// K22 as a teammate on `agent`.
pub fn persona(agent: &VoiceAgent) -> Persona {
    Persona {
        id: "k22".into(),
        name: "K22".into(),
        role: "Your assistant".into(),
        avatar: "K".into(),
        hue: 0.62,
        harness: Some(agent.kind),
        model: agent.model.clone(),
        instructions: instructions(&agent.style),
        created_at: 0,
    }
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Today's K22 thread on `kind`, if it's still there.
pub fn thread(engine: &Engine, kind: HarnessKind) -> Option<TaskId> {
    let saved = engine.store.setting(THREAD_KEY)?;
    let mut parts = saved.split('|');
    let (day, id, agent) = (parts.next()?, parts.next()?, parts.next()?);
    if day != today() || agent != kind.id() {
        return None;
    }
    let id = TaskId::parse(id)?;
    let task = engine.store.task(id).ok().flatten()?;
    (!matches!(task.status, TaskStatus::Archived | TaskStatus::Draft)).then_some(id)
}

pub fn remember(store: &Store, task: TaskId, kind: HarnessKind) {
    let _ = store.set_setting(THREAD_KEY, &format!("{}|{task}|{}", today(), kind.id()));
}

/// Whether `task` is a K22 thread (any day).
pub fn is_k22(store: &Store, task: TaskId) -> bool {
    store.setting(THREAD_KEY).is_some_and(|s| s.split('|').nth(1) == Some(task.to_string().as_str()))
}

/// A typed ask, framed so K22 knows it came from the notch.
pub fn ask_prompt(text: &str) -> String {
    format!("{text}\n\n(Asked in the notch. Answer in a sentence or two; if a view would help, show it with notch_view.)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instructions_follow_the_style() {
        assert!(instructions("formal").contains("formal"));
        assert!(instructions("anything").contains("Concise"));
        assert!(instructions("concise").contains("notch_view"));
    }

    #[test]
    fn the_thread_key_round_trips() {
        let dir = std::env::temp_dir().join(format!("dove-k22-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir).unwrap();
        let id = TaskId::new();
        remember(&store, id, HarnessKind::Codex);
        assert!(is_k22(&store, id));
        assert!(!is_k22(&store, TaskId::new()));
        let saved = store.setting(THREAD_KEY).unwrap();
        assert!(saved.starts_with(&today()));
        assert!(saved.ends_with("|codex"));
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
