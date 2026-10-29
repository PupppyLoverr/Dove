//! Voice Mode calls started from the notch (or the system-wide shortcut while
//! the notch runs). The call lives in the engine process, not in a window:
//!
//! - **Window open**: what you say goes through the Shell's own voice paths
//!   (`voice_heard`, `voice_observe`, `voice_apply`), exactly as a call
//!   started in the app, so the transcript, permission modes and the Approve
//!   highlight behave the same.
//! - **No window**: the same conductor decides, and its effects run straight
//!   on the engine (send, stop, approve, start), through the island host's
//!   commands. Replies are read aloud from the engine's attention model.
//!
//! Either way the notch sees the call ([`publish`]): the active call (this
//! one, or one the app started) is observed and sent as `ToIsland::Call`.
//! A call to K22 talks to today's K22 thread, opening it with a greeting.

use std::sync::Arc;

use dove_core::Engine;
use dove_core::proto::{HarnessKind, PermissionDecision, TaskId, TaskStatus};
use dove_voice::conductor::Effect;
use dove_voice::intent::{Action, Panel, Scope};
use dove_voice::snapshot::{self, Snapshot};
use gpui::{App, AppContext as _, Entity, Global, Subscription};

use super::host::Host;
use super::ipc::{ToEngine, ToIsland};
use super::k22;
use super::view::{CallInfo, ShowWhat};
use crate::voice_call::{ActiveCall, VoiceCall, VoiceCallEvent, strip};

/// The notch's call (when it started one), and the observation that
/// publishes whichever call is active.
#[derive(Default)]
pub struct Calls {
    own: Option<Own>,
    watching: Option<(gpui::EntityId, Subscription)>,
    last: Option<CallInfo>,
}

impl Global for Calls {}

struct Own {
    call: Entity<VoiceCall>,
    _sub: Subscription,
    /// A reply being read from the engine (no window holds the thread).
    read: Option<Reading>,
}

struct Reading {
    task: TaskId,
    /// The turn it waits for: started after this.
    after_ms: Option<i64>,
    /// How much of that turn's reply was spoken.
    consumed: usize,
}

fn engine(cx: &App) -> Arc<Engine> {
    Host::engine(cx)
}

/// Starts the notch's side: observe the active call as it comes and goes.
pub fn init(cx: &mut App) {
    cx.set_global(Calls::default());
    cx.observe_global::<ActiveCall>(active_changed).detach();
}

fn active_changed(cx: &mut App) {
    let active = strip(cx);
    let id = active.as_ref().map(|c| c.entity_id());
    if cx.global::<Calls>().watching.as_ref().map(|(i, _)| *i) == id {
        return;
    }
    match active {
        Some(call) => {
            let sub = cx.observe(&call, |call, cx| publish(&call, cx));
            cx.global_mut::<Calls>().watching = Some((call.entity_id(), sub));
            publish(&call, cx);
        }
        None => {
            let calls = cx.global_mut::<Calls>();
            calls.watching = None;
            calls.last = None;
            Host::broadcast(&ToIsland::Call { call: CallInfo::default() }, cx);
        }
    }
}

/// Sends the call's state to the notch when it changed.
fn publish(call: &Entity<VoiceCall>, cx: &mut App) {
    let mut info = call.read(cx).info();
    let engine = engine(cx);
    let voice = k22::voice_agent(&engine);
    info.agent = voice.as_ref().map(|v| v.kind);
    info.model = voice.and_then(|v| v.model);
    if let Some(id) = info.target_task {
        let task = engine.store.task(id).ok().flatten();
        info.target_kind = task.as_ref().map(|t| t.harness);
        if k22::is_k22(&engine.store, id) {
            info.target = Some("K22".into());
        } else if info.target.is_none() {
            info.target = task.map(|t| format!("{} · {}", t.title, t.harness.label()));
        }
    } else if info.target.as_deref().is_none_or(|t| t.starts_with("New thread")) {
        info.target = Some("K22".into());
    }
    if cx.global::<Calls>().last.as_ref() == Some(&info) {
        return;
    }
    cx.global_mut::<Calls>().last = Some(info.clone());
    Host::broadcast(&ToIsland::Call { call: info }, cx);
}

/// The notch's Call command: start (with K22, or about `task`), switch to
/// `task` while one runs, or end it.
pub fn toggle(on: bool, task: Option<TaskId>, cx: &mut App) {
    let active = strip(cx);
    match (on, active) {
        (false, Some(call)) => call.update(cx, |v, cx| v.end(cx)),
        (false, None) => {}
        (true, Some(call)) => {
            if let Some(t) = task {
                retarget(&call, Some(t), cx);
            }
            Host::broadcast(&ToIsland::Show { what: ShowWhat::Call }, cx);
        }
        (true, None) => start(task, cx),
    }
}

/// Starts the notch's call.
fn start(task: Option<TaskId>, cx: &mut App) {
    let engine = engine(cx);
    if !crate::voice_call::local_stt_ready(&engine) && !engine.store.flag("voice_cloud", false) {
        let info = CallInfo {
            error: Some(
                "Voice Mode needs speech recognition: install it in Settings › Voice (it runs on this device).".into(),
            ),
            ..Default::default()
        };
        Host::broadcast(&ToIsland::Call { call: info }, cx);
        return;
    }
    let call = cx.new(|cx| VoiceCall::start(engine.clone(), task, cx));
    let sub = cx.subscribe(&call, |call, ev: &VoiceCallEvent, cx| match ev {
        VoiceCallEvent::Heard(text) => heard(call, text.clone(), cx),
        VoiceCallEvent::Answer(word) => heard(call, word.to_string(), cx),
        VoiceCallEvent::Observe => observe(call, cx),
        VoiceCallEvent::Routed { utterance, json } => routed(call, utterance.clone(), json.clone(), cx),
        VoiceCallEvent::ShowView => Host::broadcast(&ToIsland::Show { what: ShowWhat::Call }, cx),
        VoiceCallEvent::Ended => ended(&call, cx),
    });
    cx.global_mut::<Calls>().own = Some(Own { call: call.clone(), _sub: sub, read: None });
    cx.set_global(ActiveCall(Some(call.clone())));
    // Who it talks to.
    match task {
        Some(t) => {
            let title = engine.store.task(t).ok().flatten().map(|t| format!("{} · {}", t.title, t.harness.label()));
            call.update(cx, |v, cx| {
                v.target = title.map(Into::into);
                cx.notify();
            });
        }
        None => open_k22(&call, cx),
    }
    Host::broadcast(&ToIsland::Show { what: ShowWhat::Call }, cx);
}

/// Points the call at today's K22 thread, opening it (K22 greets you) when
/// there isn't one yet.
fn open_k22(call: &Entity<VoiceCall>, cx: &mut App) {
    let engine = engine(cx);
    let existing = k22::voice_agent(&engine).and_then(|a| k22::thread(&engine, a.kind));
    let id = match existing {
        Some(id) => Some(id),
        None => match Host::k22_send(
            "(The user just opened a voice call with you from the notch. Greet them in a few words and ask what they need.)"
                .into(),
            cx,
        ) {
            Ok(id) => {
                read_reply(call, id, cx);
                Some(id)
            }
            Err(e) => {
                call.update(cx, |v, cx| v.say(&e, cx));
                None
            }
        },
    };
    call.update(cx, |v, cx| {
        v.current = id;
        v.target = Some("K22".into());
        v.new_thread = false;
        cx.notify();
    });
}

fn retarget(call: &Entity<VoiceCall>, task: Option<TaskId>, cx: &mut App) {
    match task {
        None => open_k22(call, cx),
        Some(id) => {
            let engine = engine(cx);
            let name = engine.store.task(id).ok().flatten().map(|t| format!("{} on {}", t.harness.label(), t.title));
            call.update(cx, |v, cx| {
                v.current = Some(id);
                v.target = name.clone().map(Into::into);
                if let Some(n) = &name {
                    v.say(&format!("Talking to {n}."), cx);
                }
                cx.notify();
            });
        }
    }
}

pub fn target(task: Option<TaskId>, cx: &mut App) {
    if let Some(call) = strip(cx) {
        retarget(&call, task, cx);
    }
}

pub fn mute(on: bool, cx: &mut App) {
    if let Some(call) = strip(cx) {
        call.update(cx, |v, cx| v.set_muted(on, cx));
    }
}

/// Typed into the call: handled as if said.
pub fn say(text: String, cx: &mut App) {
    let Some(call) = strip(cx) else { return };
    if call.read(cx).current.is_none() && cx.global::<Calls>().own.is_none() {
        return;
    }
    heard(call, text, cx);
}

fn ended(call: &Entity<VoiceCall>, cx: &mut App) {
    let ours = cx.global::<Calls>().own.as_ref().is_some_and(|o| o.call == *call);
    if ours {
        cx.global_mut::<Calls>().own = None;
        if strip(cx).as_ref() == Some(call) {
            cx.set_global(ActiveCall(None));
        }
    }
}

fn heard(call: Entity<VoiceCall>, text: String, cx: &mut App) {
    if Host::window_open(cx) {
        // The window's paths read replies from its views.
        stop_reading(cx);
        crate::notch::bridge::with_shell(cx, |s, w, cx| s.voice_heard(call, text, w, cx));
        return;
    }
    let snap = headless_snapshot(&engine(cx), call.read(cx));
    let effects = call.update(cx, |v, _| v.utterance_effects(&text, &snap));
    apply(&call, effects, Some(text), cx);
}

fn routed(call: Entity<VoiceCall>, utterance: String, json: String, cx: &mut App) {
    if Host::window_open(cx) {
        crate::notch::bridge::with_shell(cx, |s, w, cx| {
            let snap = s.voice_snapshot(call.read(cx), cx);
            let effects = call.update(cx, |v, _| v.conductor.routed(&utterance, &json, &snap));
            s.voice_apply(call, effects, None, w, cx);
        });
        return;
    }
    let snap = headless_snapshot(&engine(cx), call.read(cx));
    let effects = call.update(cx, |v, _| v.conductor.routed(&utterance, &json, &snap));
    apply(&call, effects, None, cx);
}

fn observe(call: Entity<VoiceCall>, cx: &mut App) {
    if Host::window_open(cx) {
        crate::notch::bridge::with_shell(cx, |s, _, cx| s.voice_observe(call, cx));
        return;
    }
    crate::look::note_foreground();
    let engine = engine(cx);
    let snap = headless_snapshot(&engine, call.read(cx));
    let effects = call.update(cx, |v, _| v.conductor.observe(&snap));
    call.update(cx, |v, cx| {
        for e in effects {
            if let Effect::Speak(t) = e {
                v.say(&t, cx);
            }
        }
        if let Some((id, req)) = v.awaiting_click.clone() {
            let still = snap.thread(&id.to_string()).is_some_and(|t| t.pending.iter().any(|p| p.request_id == req));
            if !still {
                v.awaiting_click = None;
                cx.notify();
            }
        }
    });
}

/// Feeds the reply being read from the engine (each host tick).
pub fn tick(cx: &mut App) {
    if Host::window_open(cx) {
        return;
    }
    let Some(own) = cx.global::<Calls>().own.as_ref() else { return };
    let Some(read) = own.read.as_ref() else { return };
    let (call, task, after, consumed) = (own.call.clone(), read.task, read.after_ms, read.consumed);
    // The user talked over the reply (or the call moved on): stop feeding it.
    if call.read(cx).reading_external() != Some(task) {
        stop_reading(cx);
        return;
    }
    let Some(state) = engine(cx).attention().get(task) else { return };
    // Wait for the turn our message started.
    let started = state.turn_started_ms.is_some_and(|s| after.is_none_or(|a| s > a));
    if !started {
        if !state.live && state.status != TaskStatus::Running && after.is_some() {
            stop_reading(cx);
        }
        return;
    }
    let reply = state.reply.as_str();
    // The front of a long reply is trimmed away: skip what's gone.
    let from = if consumed > reply.len() { reply.len() } else { consumed };
    let from = (from..=reply.len()).find(|i| reply.is_char_boundary(*i)).unwrap_or(reply.len());
    let fresh = reply[from..].to_string();
    let done = state.turn_ended_ms.is_some_and(|e| state.turn_started_ms.is_some_and(|s| e >= s))
        || matches!(state.status, TaskStatus::Done | TaskStatus::Failed | TaskStatus::Idle) && !state.live;
    if !fresh.is_empty() {
        call.update(cx, |v, cx| v.reply_delta(&fresh, cx));
    }
    if let Some(r) = cx.global_mut::<Calls>().own.as_mut().and_then(|o| o.read.as_mut()) {
        r.consumed = reply.len();
    }
    if done {
        let failed = state.status == TaskStatus::Failed || state.last_error.is_some();
        call.update(cx, |v, cx| v.reply_done(failed, cx));
        stop_reading(cx);
    }
}

fn read_reply(call: &Entity<VoiceCall>, task: TaskId, cx: &mut App) {
    let engine = engine(cx);
    let after = engine.attention().get(task).and_then(|s| s.turn_started_ms);
    // A thread that hasn't run yet in this engine: any turn counts.
    let after = after.or(Some(0)).filter(|a| *a > 0);
    call.update(cx, |v, cx| v.read_external(task, cx));
    engine.attention().watch_stream(Some(task));
    if let Some(own) = cx.global_mut::<Calls>().own.as_mut() {
        own.read = Some(Reading { task, after_ms: after, consumed: 0 });
    }
}

fn stop_reading(cx: &mut App) {
    let had = cx.global_mut::<Calls>().own.as_mut().and_then(|o| o.read.take()).is_some();
    if had {
        // Hand the stream back to whatever the notch watches.
        let watched = Host::watched(cx);
        engine(cx).attention().watch_stream(watched);
    }
}

/// What the conductor sees with no window: every recent thread, from the
/// store and the engine's attention model.
pub fn headless_snapshot(engine: &Engine, call: &VoiceCall) -> Snapshot {
    let projects = engine.store.projects().unwrap_or_default();
    let mut tasks = Vec::new();
    for p in &projects {
        for t in engine.store.tasks(p.id).unwrap_or_default() {
            if !matches!(t.status, TaskStatus::Draft | TaskStatus::Archived) {
                tasks.push((t, p.clone()));
            }
        }
    }
    tasks.sort_by_key(|(t, _)| -t.updated_at);
    tasks.truncate(40);
    let threads = tasks
        .iter()
        .map(|(task, project)| {
            let state = engine.attention().get(task.id).unwrap_or_default();
            let status = if state.live { state.status } else { task.status };
            let done =
                state.todos.iter().filter(|t| t.status == dove_core::proto::TodoStatus::Completed).count() as u32;
            let current_todo = state
                .todos
                .iter()
                .find(|t| t.status == dove_core::proto::TodoStatus::InProgress)
                .map(|t| t.text.clone());
            snapshot::Thread {
                id: task.id.to_string(),
                title: task.title.clone(),
                agent: task.harness.label().to_string(),
                agent_id: task.harness.id().to_string(),
                project: project.name.clone(),
                status: crate::voice_call::status_of(status),
                live: state.live,
                steers: dove_core::steers(task.harness),
                interrupts: state.live && state.capabilities.interrupt,
                session_approvals: state.live && state.capabilities.permissions,
                pending: state
                    .pending
                    .iter()
                    .map(|r| snapshot::Pending {
                        request_id: r.request_id.clone(),
                        tool: r.tool.clone(),
                        summary: r.summary.to_string(),
                        input: r.input.clone(),
                        question: r.is_question(),
                    })
                    .collect(),
                activity: state.activity.clone(),
                todos: (!state.todos.is_empty()).then_some((done, state.todos.len() as u32, current_todo)),
                diff: call.diffs.get(&task.id).copied(),
                cwd: task.worktree.clone().or_else(|| Some(project.path.clone())).map(|p| p.display().to_string()),
                error: state.last_error.clone(),
                teammate: k22::is_k22(&engine.store, task.id).then(|| "K22".to_string()),
                terminal: false,
            }
        })
        .collect();
    let voice = k22::voice_agent(engine);
    let installed: Vec<HarnessKind> = engine.installed().iter().map(|i| i.kind).collect();
    Snapshot {
        threads,
        current: call.current.map(|c| c.to_string()),
        agents: installed
            .iter()
            .map(|k| snapshot::Agent { id: k.id().to_string(), label: k.label().to_string() })
            .collect(),
        projects: projects.iter().map(|p| snapshot::Project { id: p.id.to_string(), name: p.name.clone() }).collect(),
        default_project: voice.as_ref().and_then(|v| v.project).map(|p| p.to_string()),
        default_agent: voice.as_ref().map(|v| v.kind.id().to_string()),
        showing: None,
        teammates: engine
            .store
            .personas()
            .unwrap_or_default()
            .into_iter()
            .map(|p| snapshot::Teammate { id: p.id, name: p.name, role: p.role })
            .collect(),
    }
}

/// Carries out the conductor's effects with no window.
fn apply(call: &Entity<VoiceCall>, effects: Vec<Effect>, utterance: Option<String>, cx: &mut App) {
    let mut acted_on = None;
    for effect in effects {
        match effect {
            Effect::Speak(t) => call.update(cx, |v, cx| v.say(&t, cx)),
            Effect::StopSpeaking => {
                stop_reading(cx);
                call.update(cx, |v, cx| v.silence(cx));
            }
            Effect::MuteMic(on) => call.update(cx, |v, cx| v.set_muted(on, cx)),
            Effect::SetRate(r) => call.update(cx, |v, _| v.set_rate(r)),
            Effect::SetStyle(style) => {
                let _ = engine(cx).store.set_setting("voice_persona", &style);
            }
            Effect::EndCall => call.update(cx, |v, cx| v.end(cx)),
            Effect::Route(text) => {
                let snap = headless_snapshot(&engine(cx), call.read(cx));
                call.update(cx, |v, cx| v.route(text, &snap, cx));
            }
            Effect::AwaitClick { thread, request_id } => {
                if let Some(id) = TaskId::parse(&thread) {
                    call.update(cx, |v, cx| {
                        v.awaiting_click = Some((id, request_id));
                        cx.notify();
                    });
                    Host::broadcast(&ToIsland::Show { what: ShowWhat::Agent { task: id } }, cx);
                }
            }
            Effect::Act(action) => {
                if let Some(id) = act(call, action, cx) {
                    acted_on = Some(id);
                }
            }
        }
    }
    if let Some(text) = utterance {
        call.update(cx, |v, _| v.note_user(&text, acted_on));
    }
}

/// One checked action, on the engine. Returns the thread it went to.
fn act(call: &Entity<VoiceCall>, action: Action, cx: &mut App) -> Option<TaskId> {
    let engine = engine(cx);
    match action {
        Action::Send { thread, text, .. } => {
            let id = TaskId::parse(&thread)?;
            let attach = call.update(cx, |v, _| v.attach.take());
            let text = match attach {
                Some(a) => format!("{text}\n\n{a}"),
                None => text,
            };
            call.update(cx, |v, _| v.current = Some(id));
            read_reply(call, id, cx);
            Host::run(ToEngine::Send { task: id, text }, cx);
            Some(id)
        }
        Action::Interrupt { thread } => {
            let id = TaskId::parse(&thread)?;
            Host::run(ToEngine::Stop { task: id }, cx);
            Some(id)
        }
        Action::Approve { thread, request_id, scope } => {
            let id = TaskId::parse(&thread)?;
            // Defense in depth: never approve a destructive request by voice.
            let state = engine.attention().get(id)?;
            let req = state.pending.iter().find(|r| r.request_id == request_id)?;
            let pending = snapshot::Pending {
                request_id: req.request_id.clone(),
                tool: req.tool.clone(),
                summary: req.summary.to_string(),
                input: req.input.clone(),
                question: req.is_question(),
            };
            let task = engine.store.task(id).ok().flatten();
            let project = task.as_ref().and_then(|t| {
                engine.store.projects().ok().and_then(|ps| ps.into_iter().find(|p| p.id == t.project_id))
            });
            let cwd = task.and_then(|t| t.worktree).or(project.map(|p| p.path)).map(|p| p.display().to_string());
            if dove_voice::safety::classify(&pending, cwd.as_deref()) != dove_voice::safety::Risk::Normal {
                call.update(cx, |v, cx| v.say("That one needs your click.", cx));
                return Some(id);
            }
            let decision = match scope {
                Scope::Once => PermissionDecision::AllowOnce,
                Scope::Session => PermissionDecision::AllowAlways,
            };
            Host::run(ToEngine::Decide { task: id, request_id, decision }, cx);
            Some(id)
        }
        Action::Deny { thread, request_id } => {
            let id = TaskId::parse(&thread)?;
            Host::run(ToEngine::Decide { task: id, request_id, decision: PermissionDecision::Deny }, cx);
            Some(id)
        }
        Action::TalkTo { thread } => {
            let id = TaskId::parse(&thread)?;
            retarget(call, Some(id), cx);
            Some(id)
        }
        Action::StartTask { agent, project, prompt, worktree } => {
            let kind = HarnessKind::from_id(&agent)?;
            let project = dove_core::proto::ProjectId::parse(&project)?;
            match Host::start_agent(kind, project, prompt, worktree, Default::default(), None, None, cx) {
                Ok(id) => {
                    call.update(cx, |v, _| v.current = Some(id));
                    read_reply(call, id, cx);
                    Some(id)
                }
                Err(e) => {
                    call.update(cx, |v, cx| v.say(&e, cx));
                    None
                }
            }
        }
        Action::Launch { project, items } => {
            let project = dove_core::proto::ProjectId::parse(&project)?;
            let mut first = None;
            let mut failed = Vec::new();
            for item in items {
                let Some(kind) = HarnessKind::from_id(&item.agent) else { continue };
                if item.terminal {
                    failed.push(format!("{}'s terminal needs the app", kind.label()));
                    continue;
                }
                let persona = item
                    .teammate
                    .as_deref()
                    .and_then(|id| engine.store.personas().ok()?.into_iter().find(|p| p.id == id));
                let prompt = item.prompt.clone().unwrap_or_default();
                match Host::start_agent(kind, project, prompt, item.worktree, Default::default(), persona, None, cx) {
                    Ok(id) => {
                        first.get_or_insert(id);
                    }
                    Err(e) => failed.push(e),
                }
            }
            if let Some(id) = first {
                call.update(cx, |v, _| v.current = Some(id));
                read_reply(call, id, cx);
            }
            if !failed.is_empty() {
                let n = failed.len();
                call.update(cx, |v, cx| v.say(&format!("{n} didn't start: {}.", failed.join("; ")), cx));
            }
            first
        }
        Action::AskTeammate { teammate, text } => {
            let mate = engine.store.personas().ok()?.into_iter().find(|p| p.id == teammate)?;
            let kind = mate.harness.or_else(|| k22::voice_agent(&engine).map(|a| a.kind))?;
            let project = k22::voice_agent(&engine).and_then(|a| a.project)?;
            let run = dove_core::RunSettings { model: mate.model.clone(), ..Default::default() };
            match Host::start_agent(kind, project, text, false, run, Some(mate), None, cx) {
                Ok(id) => {
                    read_reply(call, id, cx);
                    Some(id)
                }
                Err(e) => {
                    call.update(cx, |v, cx| v.say(&e, cx));
                    None
                }
            }
        }
        Action::Show { panel } => {
            let current = call.read(cx).current;
            match (panel, current) {
                (Panel::Plan, Some(id)) => Host::broadcast(&ToIsland::Show { what: ShowWhat::Plan { task: id } }, cx),
                (Panel::Chat, Some(id)) => Host::broadcast(&ToIsland::Show { what: ShowWhat::Agent { task: id } }, cx),
                (_, task) => {
                    Host::run(ToEngine::Open { task }, cx);
                    call.update(cx, |v, cx| v.say("I opened it in Keplorayx.", cx));
                }
            }
            None
        }
        Action::LookAt { target } => {
            let note = match target {
                dove_voice::intent::LookTarget::Diff => {
                    let current = call.read(cx).current;
                    current.and_then(|id| call.read(cx).diffs.get(&id).copied()).map(|d| {
                        format!(
                            "(From the voice call: please look at the current diff in this thread's folder: {} files changed, +{} −{}. Run `git diff` to see it.)",
                            d.files, d.added, d.removed
                        )
                    })
                }
                dove_voice::intent::LookTarget::Window => {
                    match crate::look::capture_last(&engine.store.root().join("voice").join("look")) {
                        Ok((path, title)) => Some(format!(
                            "(From the voice call: a screenshot of the window \"{title}\" the user is looking at is at {}. Open it to see what they mean.)",
                            path.display()
                        )),
                        Err(e) => {
                            call.update(cx, |v, cx| v.say(&e, cx));
                            return None;
                        }
                    }
                }
                dove_voice::intent::LookTarget::Browser => None,
            };
            call.update(cx, |v, cx| match note {
                Some(n) => v.attach = Some(n),
                None => v.say("I can't see that from here. Try the diff, or the window you're in.", cx),
            });
            None
        }
        Action::Draft { thread, .. } | Action::SendDraft { thread } | Action::ClearDraft { thread } => {
            let id = TaskId::parse(&thread)?;
            call.update(cx, |v, cx| v.say("Drafts need the app; I'll open the thread.", cx));
            Host::run(ToEngine::Open { task: Some(id) }, cx);
            Some(id)
        }
        Action::OpenBeside { thread } => {
            let id = TaskId::parse(&thread)?;
            Host::run(ToEngine::Open { task: Some(id) }, cx);
            Some(id)
        }
        Action::Status { .. }
        | Action::TeamStatus { .. }
        | Action::SetVoiceStyle { .. }
        | Action::SwitchBack
        | Action::EndCall => None,
    }
}
