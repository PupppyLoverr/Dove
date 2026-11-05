//! The engine, for a window's scripts: its threads, the open one's
//! transcript as it streams, and what can be asked of it. The engine runs in
//! this process (`dove_core::Engine`); nothing here blocks the window's
//! thread: the store is read on a thread of its own, a live session's events
//! are awaited, never polled.
//!
//! What scripts see (`ui.shared`):
//!   `kx.threads`   every thread: { id, title, project, status, harness, updated, unread }
//!   `kx.projects`  { id, name, path }
//!   `kx.thread`    the open thread: { id, title, project, harness, running, status }, nil for none
//!   `kx.entries`   its transcript (src/transcript.rs), keyed by `id`
//!   `kx.draft`     the block being written: { id, kind, text }
//!   `kx.approval`  what the open thread's agent waits on you for:
//!                  { request_id, tool, summary, questions = { { text, options } } }, nil for nothing
//!   `kx.agents`    the agents installed: { { id, label } }
//!   `kx.attention` every thread waiting on you, across windows (the notch):
//!                  { { thread_id, title, harness, harness_id, request_id, tool, summary } }
//!
//! Calls:
//!   kx.open(id)                     an action: open that thread
//!   kx.new()                        an action: no thread open (Home)
//!   kx.send({ text, project?, harness? })
//!                                   run a message in the open thread, or a new
//!                                   one (in that project, with that agent)
//!   kx.interrupt()                  an action: stop the open thread's turn
//!   kx.decide({ request_id, decision = "allow_once" | "allow_always" | "deny", answers? })
//!                                   an action: answer what the open thread waits on
//!   kx.add_project(path)            add a folder as a project; its id
//!   kx.decide_for({ thread_id, request_id, decision })
//!                                   an action: answer one thread's request (the notch)
//!   kx.changes()                    the open thread's changes against where it
//!                                   started: { { path, status, added, removed,
//!                                   hunks = { { header, lines = { { kind, old, new, text } } } } } }
//!
//! Every window that installs this shares one hub: the live sessions, the
//! automation scheduler (one a process, however many windows are open), and
//! the thread list, published to each window's `kx.threads`. Which thread is
//! open is each window's own.
//!
//! A live session's events fold into its thread's transcript whether it's
//! open or not; only the open one's reach the window, and then only what
//! changed: a streamed word writes `kx.draft` alone, and the list is
//! published again only when an entry comes, goes or changes, which
//! `Shared::set` reconciles by `id` so the rest keep their rows.

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
    sync::Arc,
};

use dove_core::{Command, Engine, LiveSession, NewTask, RunSettings};
use dove_proto::TaskId;
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Shared, Value};

use crate::transcript::{Change, Draft, Entry, Fold, Pending};

/// A thread with a running harness.
struct Live {
    session: Rc<LiveSession>,
    fold: Rc<RefCell<Fold>>,
}

type LiveMap = Rc<RefCell<HashMap<TaskId, Live>>>;

/// What every window shares.
struct Hub {
    live: LiveMap,
    /// Each window's state, while it lasts.
    windows: Vec<Weak<RefCell<State>>>,
}

thread_local! {
    static HUB: RefCell<Option<Hub>> = const { RefCell::new(None) };
}

/// The process's live sessions, and whether this is the first window (which
/// starts the scheduler).
fn join_hub() -> (LiveMap, bool) {
    HUB.with(|hub| {
        let mut hub = hub.borrow_mut();
        match &*hub {
            Some(h) => (h.live.clone(), false),
            None => {
                let live: LiveMap = Rc::default();
                *hub = Some(Hub { live: live.clone(), windows: Vec::new() });
                (live, true)
            }
        }
    })
}

/// Every window's state that's still open.
fn windows() -> Vec<Host> {
    HUB.with(|hub| {
        let mut hub = hub.borrow_mut();
        let Some(h) = hub.as_mut() else { return Vec::new() };
        h.windows.retain(|w| w.strong_count() > 0);
        h.windows.iter().filter_map(Weak::upgrade).collect()
    })
}

struct State {
    engine: Arc<Engine>,
    open: Option<TaskId>,
    /// The open thread's transcript when it has no live session.
    replayed: Option<Rc<RefCell<Fold>>>,
    live: LiveMap,
    threads: Shared,
    projects: Shared,
    thread: Shared,
    entries: Shared,
    draft: Shared,
    approval: Shared,
    agents: Shared,
    attention: Shared,
}

type Host = Rc<RefCell<State>>;

pub fn install(cx: &mut WindowContext, host: &ScriptHost, engine: Arc<Engine>) {
    let (live, first) = join_hub();
    let state: Host = Rc::new(RefCell::new(State {
        engine,
        open: None,
        replayed: None,
        live,
        threads: host.shared("kx.threads", Value::List(Rc::from([]))),
        projects: host.shared("kx.projects", Value::List(Rc::from([]))),
        thread: host.shared("kx.thread", Value::Nil),
        entries: host.shared("kx.entries", Value::List(Rc::from([]))),
        draft: host.shared("kx.draft", draft_value(None)),
        approval: host.shared("kx.approval", Value::Nil),
        agents: host.shared("kx.agents", Value::List(Rc::from([]))),
        attention: host.shared("kx.attention", Value::List(Rc::from([]))),
    }));
    HUB.with(|hub| {
        if let Some(h) = hub.borrow_mut().as_mut() {
            h.windows.push(Rc::downgrade(&state));
        }
    });
    refresh_threads(cx, &state);

    let (s, tasks) = (state.clone(), cx.tasks.clone());
    host.action("kx.open", move |id| {
        let Some(id) = parse_task(&id) else { return };
        open(&tasks, &s, id);
    });

    let s = state.clone();
    host.action("kx.new", move |_| {
        let mut st = s.borrow_mut();
        st.open = None;
        st.replayed = None;
        st.thread.set(Value::Nil);
        st.entries.set(Value::List(Rc::from([])));
        st.draft.set(draft_value(None));
        st.approval.set(Value::Nil);
    });

    let s = state.clone();
    host.action("kx.interrupt", move |_| {
        let st = s.borrow();
        if let Some(id) = st.open {
            st.engine.interrupt(id);
        }
    });

    let s = state.clone();
    host.action("kx.decide", move |req| {
        let st = s.borrow();
        let Some(id) = st.open else { return };
        let live_map = st.live.borrow();
        let Some(live) = live_map.get(&id) else { return };
        let Some(request_id) = req.field("request_id").and_then(text_of) else { return };
        let command = match req.field("answers") {
            Some(Value::List(answers)) => Command::Answer {
                request_id,
                answers: answers
                    .iter()
                    .map(|a| match a {
                        Value::List(picked) => picked.iter().filter_map(text_of).collect(),
                        other => text_of(other).into_iter().collect(),
                    })
                    .collect(),
            },
            _ => {
                let decision = match req.field("decision").and_then(text_of).as_deref() {
                    Some("allow_always") => dove_proto::PermissionDecision::AllowAlways,
                    Some("deny") => dove_proto::PermissionDecision::Deny,
                    _ => dove_proto::PermissionDecision::AllowOnce,
                };
                Command::Permission { request_id, decision }
            }
        };
        live.session.send(command);
    });

    let s = state.clone();
    host.action("kx.decide_for", move |req| {
        let st = s.borrow();
        let Some(id) = req.field("thread_id").and_then(parse_task) else { return };
        let Some(request_id) = req.field("request_id").and_then(text_of) else { return };
        let decision = match req.field("decision").and_then(text_of).as_deref() {
            Some("allow_always") => dove_proto::PermissionDecision::AllowAlways,
            Some("deny") => dove_proto::PermissionDecision::Deny,
            _ => dove_proto::PermissionDecision::AllowOnce,
        };
        if let Some(live) = st.live.borrow().get(&id) {
            live.session.send(Command::Permission { request_id, decision });
        }
    });

    let (s, tasks) = (state.clone(), cx.tasks.clone());
    host.async_action("kx.add_project", move |path| {
        let (s, tasks) = (s.clone(), tasks.clone());
        async move {
            let Some(path) = text_of(&path).filter(|p| !p.is_empty()) else {
                return Err("no folder".to_string());
            };
            let engine = s.borrow().engine.clone();
            let added = off_thread(move || engine.store.add_project(std::path::Path::new(&path)))
                .await
                .ok_or("failed")?
                .map_err(|e| format!("{e:#}"))?;
            spawn_refresh(&tasks, &s);
            Ok(Value::from(added.id.0.to_string().as_str()))
        }
    });

    let s = state.clone();
    host.async_action("kx.changes", move |_| {
        let s = s.clone();
        async move {
            let (engine, open) = {
                let st = s.borrow();
                (st.engine.clone(), st.open)
            };
            let Some(id) = open else { return Ok(Value::List(Rc::from([]))) };
            let files = off_thread(move || -> anyhow::Result<Vec<dove_git::diff::FileDiff>> {
                let task = engine.store.task(id)?.ok_or_else(|| anyhow::anyhow!("missing"))?;
                let project = engine
                    .store
                    .projects()?
                    .into_iter()
                    .find(|p| p.id == task.project_id)
                    .ok_or_else(|| anyhow::anyhow!("no project"))?;
                // A worktree's diff is against the commit it started from;
                // a thread in the project itself, against HEAD.
                let dir = task.worktree.clone().unwrap_or(project.path.clone());
                let base = task.base.clone().unwrap_or_else(|| "HEAD".to_string());
                if dove_git::repo_root(&dir).is_none() {
                    return Ok(Vec::new());
                }
                dove_git::changes(&dir, &base)
            })
            .await
            .ok_or("failed")?
            .map_err(|e| format!("{e:#}"))?;
            Ok(Value::List(files.iter().map(file_value).collect()))
        }
    });

    let (s, tasks) = (state.clone(), cx.tasks.clone());
    host.async_action("kx.send", move |req| {
        let (s, tasks) = (s.clone(), tasks.clone());
        async move { send(&tasks, &s, req).await.map(|()| Value::Nil) }
    });

    let (engine, s) = (state.borrow().engine.clone(), state.clone());
    crate::panels::install(host, engine, move || s.borrow().open);

    if first {
        start_scheduler(&cx.tasks, &state);
    }
}

/// Runs automations as Dove's shell does: schedules every minute, event
/// triggers every fifth. The minute is a deadline, not a poll: nothing is
/// asked of the store between ticks.
fn start_scheduler(tasks: &zgx_desktop::TaskSpawner, state: &Host) {
    let (s, spawner) = (state.clone(), tasks.clone());
    tasks.spawn(async move {
        let mut tick = 0u64;
        loop {
            zgx::timer::sleep(std::time::Duration::from_secs(60)).await;
            tick += 1;
            let engine = s.borrow().engine.clone();
            let check_events = tick.is_multiple_of(5);
            let started = off_thread(move || {
                let mut due: Vec<_> = engine.due_schedules().into_iter().map(|a| (a, None)).collect();
                if check_events {
                    due.extend(engine.due_events().into_iter().map(|(a, c)| (a, Some(c))));
                }
                due.into_iter()
                    .filter_map(|(a, context)| match engine.run_automation(&a, context.as_deref()) {
                        Ok((_, live)) => Some(live),
                        Err(e) => {
                            tracing::warn!("automation {} failed to start: {e:#}", a.name);
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            if started.is_empty() {
                continue;
            }
            for live in started {
                follow(&spawner, &s, live);
            }
            spawn_refresh(&spawner, &s);
        }
    });
}

fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

fn parse_task(value: &Value) -> Option<TaskId> {
    let Value::Str(id) = value else { return None };
    id.parse().ok().map(TaskId)
}

/// `f` on a thread of its own, its answer awaited.
pub(crate) async fn off_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tell, heard) = flume::bounded(1);
    std::thread::spawn(move || {
        let _ = tell.send(f());
    });
    heard.recv_async().await.ok()
}

/// Reads the threads and projects again and publishes them.
fn refresh_threads(cx: &mut WindowContext, state: &Host) {
    spawn_refresh(&cx.tasks, state);
}

fn spawn_refresh(tasks: &zgx_desktop::TaskSpawner, state: &Host) {
    let engine = state.borrow().engine.clone();
    let s = state.clone();
    tasks.spawn(async move {
        let read = off_thread(move || {
            let projects = engine.store.projects().unwrap_or_default();
            let mut threads = engine.store.all_tasks().unwrap_or_default();
            threads.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            let agents: Vec<_> = engine.installed().into_iter().map(|i| i.kind).collect();
            (projects, threads, agents)
        })
        .await;
        let Some((projects, threads, agents)) = read else { return };
        let names: HashMap<_, _> = projects.iter().map(|p| (p.id, p.name.clone())).collect();
        let mut all = windows();
        if !all.iter().any(|w| Rc::ptr_eq(w, &s)) {
            all.push(s.clone());
        }
        for w in all {
            publish_lists(&w.borrow(), &names, &projects, &threads, &agents);
        }
    });
}

fn publish_lists(
    st: &State,
    names: &HashMap<dove_proto::ProjectId, String>,
    projects: &[dove_store::Project],
    threads: &[dove_store::Task],
    agents: &[dove_proto::HarnessKind],
) {
    {
        // What waits on you, across every live thread.
        let titles: HashMap<_, _> = threads.iter().map(|t| (t.id, t)).collect();
        let waiting: Vec<Value> = st
            .live
            .borrow()
            .iter()
            .filter_map(|(id, l)| {
                let f = l.fold.borrow();
                let p = f.pending.as_ref()?;
                let t = titles.get(id)?;
                Some(Value::record([
                    ("thread_id", Value::from(id.0.to_string().as_str())),
                    ("title", Value::from(t.title.as_str())),
                    ("harness", Value::from(t.harness.label())),
                    ("harness_id", Value::from(t.harness.id())),
                    ("request_id", Value::from(p.request_id.as_str())),
                    ("tool", Value::from(p.tool.as_str())),
                    ("summary", Value::from(p.summary.as_str())),
                ]))
            })
            .collect();
        st.attention.set(Value::List(waiting.into()));
        st.agents.set(Value::List(
            agents
                .iter()
                .map(|k| Value::record([("id", Value::from(k.id())), ("label", Value::from(k.label()))]))
                .collect(),
        ));
        st.projects.set(Value::List(
            projects
                .iter()
                .map(|p| {
                    Value::record([
                        ("id", Value::from(p.id.0.to_string().as_str())),
                        ("name", Value::from(p.name.as_str())),
                        ("path", Value::from(p.path.to_string_lossy().as_ref())),
                    ])
                })
                .collect(),
        ));
        st.threads.set(Value::List(
            threads
                .iter()
                .map(|t| {
                    let (live, needs_you) = st.live.borrow().get(&t.id).map_or((false, false), |l| {
                        let f = l.fold.borrow();
                        (f.running, f.pending.is_some())
                    });
                    let status = if needs_you {
                        "needs_you"
                    } else if live {
                        "running"
                    } else {
                        t.status.id()
                    };
                    Value::record([
                        ("id", Value::from(t.id.0.to_string().as_str())),
                        ("title", Value::from(t.title.as_str())),
                        ("project", Value::from(names.get(&t.project_id).map(String::as_str).unwrap_or(""))),
                        ("project_id", Value::from(t.project_id.0.to_string().as_str())),
                        ("status", Value::from(status)),
                        ("harness", Value::from(t.harness.label())),
                        ("harness_id", Value::from(t.harness.id())),
                        ("updated", Value::from(t.updated_at as f64)),
                        ("unread", Value::from(t.unread)),
                        ("branch", Value::from(t.branch.as_deref().unwrap_or(""))),
                        ("model", Value::from(t.model.as_deref().unwrap_or(""))),
                        ("pinned", Value::from(t.pinned)),
                        ("worktree", Value::from(t.worktree.is_some())),
                    ])
                })
                .collect(),
        ));
    }
}

fn open(tasks: &zgx_desktop::TaskSpawner, state: &Host, id: TaskId) {
    {
        let mut st = state.borrow_mut();
        st.open = Some(id);
        st.replayed = None;
        st.draft.set(draft_value(None));
    }
    if state.borrow().live.borrow().contains_key(&id) {
        publish(state, Change::Entries);
        return;
    }
    // Recorded: replayed from the store on a thread.
    let engine = state.borrow().engine.clone();
    let s = state.clone();
    tasks.spawn(async move {
        let read = off_thread(move || {
            let task = engine.store.task(id).ok().flatten();
            let (events, _) = engine.store.transcript_from(id, 0);
            (task, Fold::replay(events))
        })
        .await;
        let Some((task, fold)) = read else { return };
        {
            let mut st = s.borrow_mut();
            if st.open != Some(id) {
                return;
            }
            st.replayed = Some(Rc::new(RefCell::new(fold)));
            if let Some(task) = task {
                st.thread.set(thread_value(&task.id, &task.title, task.harness.label(), false, ""));
            }
        }
        publish(&s, Change::Entries);
    });
}

async fn send(tasks: &zgx_desktop::TaskSpawner, state: &Host, req: Value) -> Result<(), String> {
    let text = req.field("text").and_then(|t| match t {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    });
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
        return Err("empty".into());
    };
    let (engine, open) = {
        let st = state.borrow();
        (st.engine.clone(), st.open)
    };
    // The open thread's session takes it as it is.
    if let Some(id) = open
        && let Some(live) = state.borrow().live.borrow().get(&id)
        && live.session.send(Command::Send(text.clone()))
    {
        return Ok(());
    }
    let project = req.field("project").and_then(|p| match p {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    });
    let harness = req.field("harness").and_then(text_of);
    // How much a new thread's agent may do before it asks (the composer's
    // access chip).
    let mode = match req.field("access").and_then(text_of).as_deref() {
        Some("edits") => dove_proto::PermissionMode::AcceptEdits,
        Some("full") => dove_proto::PermissionMode::Bypass,
        Some("plan") => dove_proto::PermissionMode::Plan,
        _ => dove_proto::PermissionMode::Default,
    };
    // The model ("<agent>:<model>", the picker's) and how hard it thinks.
    let model = req
        .field("model")
        .and_then(text_of)
        .and_then(|m| m.split_once(':').map(|(_, id)| id.to_string()))
        .filter(|m| !m.is_empty());
    let reasoning = req
        .field("effort")
        .and_then(text_of)
        .and_then(|e| serde_json::from_value::<dove_proto::ReasoningLevel>(serde_json::Value::String(e)).ok());
    // The teammate a new thread runs as (Team › Start a thread), by id.
    let persona = req.field("persona").and_then(text_of).filter(|p| !p.is_empty());
    // Starting or resuming spawns the harness: on a thread.
    let started = off_thread(move || -> anyhow::Result<LiveSession> {
        match open {
            Some(id) => {
                let task = engine.store.task(id)?.ok_or_else(|| anyhow::anyhow!("missing"))?;
                let projects = engine.store.projects()?;
                let project = projects
                    .into_iter()
                    .find(|p| p.id == task.project_id)
                    .ok_or_else(|| anyhow::anyhow!("no project"))?;
                engine.resume_task(&task, &project, Some(text), RunSettings::default())
            }
            None => {
                let projects = engine.store.projects()?;
                let project = match &project {
                    Some(id) => projects.into_iter().find(|p| &p.id.0.to_string() == id),
                    None => projects.into_iter().next(),
                }
                .ok_or_else(|| anyhow::anyhow!("no project: add one first"))?;
                let persona = match persona {
                    Some(id) => engine.store.personas()?.into_iter().find(|p| p.id == id),
                    None => None,
                };
                // The agent asked for (or the teammate's own), if it's
                // installed; else the first that is.
                let installed = engine.installed();
                let harness = harness.or_else(|| persona.as_ref().and_then(|p| p.harness).map(|h| h.id().to_string()));
                let harness = harness
                    .and_then(|id| installed.iter().find(|i| i.kind.id() == id).map(|i| i.kind))
                    .or_else(|| installed.first().map(|i| i.kind))
                    .ok_or_else(|| anyhow::anyhow!("no agent installed"))?;
                let title: String = text.lines().next().unwrap_or("").chars().take(60).collect();
                let run = RunSettings { mode, ..RunSettings::default() };
                let run = RunSettings {
                    model: model.or_else(|| persona.as_ref().and_then(|p| p.model.clone())),
                    reasoning,
                    ..run
                };
                let new = NewTask { title, prompt: text, harness, run, join: None, persona };
                engine.start_task(&project, new).map(|(_, session)| session)
            }
        }
    })
    .await
    .ok_or("failed")?
    .map_err(|e| format!("{e:#}"))?;
    attach(tasks, state, started);
    spawn_refresh(tasks, state);
    Ok(())
}

/// Follows a live session and opens its thread.
fn attach(tasks: &zgx_desktop::TaskSpawner, state: &Host, session: LiveSession) {
    let id = session.task;
    follow(tasks, state, session);
    {
        let mut st = state.borrow_mut();
        st.replayed = None;
        st.open = Some(id);
    }
    publish(state, Change::Entries);
}

/// Follows a live session, open or not (an automation's runs in the
/// background): its events fold into its transcript as they come.
fn follow(tasks: &zgx_desktop::TaskSpawner, state: &Host, session: LiveSession) {
    let id = session.task;
    let session = Rc::new(session);
    // A resumed thread carries on from what it had.
    let fold = {
        let st = state.borrow();
        st.replayed
            .as_ref()
            .filter(|_| st.open == Some(id))
            .map(|f| Rc::new(RefCell::new(std::mem::take(&mut *f.borrow_mut()))))
            .unwrap_or_default()
    };
    state.borrow().live.borrow_mut().insert(id, Live { session: session.clone(), fold: fold.clone() });
    let s = state.clone();
    let tasks_for_refresh = tasks.clone();
    tasks.spawn(async move {
        while let Ok(event) = session.events.recv_async().await {
            // Everything ready now, folded at once: one publish a burst.
            let waited = fold.borrow().pending.is_some();
            let mut change = fold.borrow_mut().apply(&event);
            let mut ended = matches!(event, dove_proto::AgentEvent::TurnEnded { .. });
            while let Ok(event) = session.events.try_recv() {
                ended |= matches!(event, dove_proto::AgentEvent::TurnEnded { .. });
                change = change.and(fold.borrow_mut().apply(&event));
            }
            if s.borrow().open == Some(id) {
                publish(&s, change);
            }
            // A turn ending, or the agent starting or stopping waiting on
            // you, changes its row in the sidebar.
            if ended || fold.borrow().pending.is_some() != waited {
                spawn_refresh(&tasks_for_refresh, &s);
            }
        }
        // The session ended: its transcript stays, as a replay.
        let mut st = s.borrow_mut();
        let removed = st.live.borrow_mut().remove(&id);
        if let Some(live) = removed
            && st.open == Some(id)
        {
            live.fold.borrow_mut().running = false;
            st.replayed = Some(live.fold);
        }
        drop(st);
        spawn_refresh(&tasks_for_refresh, &s);
    });
}

/// The open thread's transcript, as far as `change` reaches.
fn publish(state: &Host, change: Change) {
    let st = state.borrow();
    let Some(id) = st.open else { return };
    let live_fold = st.live.borrow().get(&id).map(|l| l.fold.clone());
    let fold = match live_fold {
        Some(fold) => fold,
        None => match &st.replayed {
            Some(fold) => fold.clone(),
            None => return,
        },
    };
    let fold = fold.borrow();
    if change == Change::Entries {
        st.entries.set(Value::List(fold.entries.iter().map(entry_value).collect()));
        let was = st.thread.signal().with_untracked(Value::clone);
        if !matches!(was, Value::Nil) {
            let field = |k: &str| was.field(k).cloned().unwrap_or(Value::Nil);
            st.thread.set(Value::record([
                ("id", field("id")),
                ("title", field("title")),
                ("harness", field("harness")),
                ("running", Value::from(fold.running)),
                ("status", Value::from(fold.status.as_str())),
            ]));
        }
    }
    if change != Change::None {
        st.draft.set(draft_value(fold.draft.as_ref()));
        st.approval.set(approval_value(fold.pending.as_ref()));
    }
}

fn thread_value(id: &TaskId, title: &str, harness: &str, running: bool, status: &str) -> Value {
    Value::record([
        ("id", Value::from(id.0.to_string().as_str())),
        ("title", Value::from(title)),
        ("harness", Value::from(harness)),
        ("running", Value::from(running)),
        ("status", Value::from(status)),
    ])
}

fn entry_value(e: &Entry) -> Value {
    let mut fields = vec![
        ("id", Value::from(e.id.as_str())),
        ("kind", Value::from(e.kind.id())),
        ("text", Value::from(e.text.as_str())),
    ];
    if let Some(tool) = &e.tool {
        let changes = tool
            .changes
            .iter()
            .map(|(path, added, removed)| {
                Value::record([
                    ("path", Value::from(path.as_str())),
                    ("added", Value::from(*added as f64)),
                    ("removed", Value::from(*removed as f64)),
                ])
            })
            .collect();
        fields.push((
            "tool",
            Value::record([
                ("name", Value::from(tool.name.as_str())),
                ("state", Value::from(tool.state.id())),
                ("output", Value::from(tool.output.as_str())),
                ("changes", Value::List(changes)),
            ]),
        ));
    }
    Value::record(fields)
}

/// The most lines of one file's diff sent to the window: a review shows the
/// rest by opening the file.
const DIFF_LINES: usize = 600;

fn file_value(f: &dove_git::diff::FileDiff) -> Value {
    use dove_git::diff::{FileStatus, LineKind};
    let mut budget = DIFF_LINES;
    let hunks = f
        .hunks
        .iter()
        .filter_map(|h| {
            if budget == 0 {
                return None;
            }
            let take = h.lines.len().min(budget);
            budget -= take;
            let lines = h.lines[..take]
                .iter()
                .map(|l| {
                    Value::record([
                        (
                            "kind",
                            Value::from(match l.kind {
                                LineKind::Added => "add",
                                LineKind::Removed => "del",
                                LineKind::Context => "ctx",
                            }),
                        ),
                        ("old", l.old_no.map_or(Value::Nil, |n| Value::from(f64::from(n)))),
                        ("new", l.new_no.map_or(Value::Nil, |n| Value::from(f64::from(n)))),
                        ("text", Value::from(&*l.text)),
                    ])
                })
                .collect();
            Some(Value::record([("header", Value::from(&*h.header)), ("lines", Value::List(lines))]))
        })
        .collect();
    Value::record([
        ("path", Value::from(f.path.as_str())),
        (
            "status",
            Value::from(match f.status {
                FileStatus::Added => "added",
                FileStatus::Deleted => "deleted",
                FileStatus::Modified => "modified",
                FileStatus::Renamed => "renamed",
            }),
        ),
        ("binary", Value::from(f.binary)),
        ("added", Value::from(f64::from(f.additions))),
        ("removed", Value::from(f64::from(f.deletions))),
        ("hunks", Value::List(hunks)),
    ])
}

fn approval_value(pending: Option<&Pending>) -> Value {
    let Some(p) = pending else { return Value::Nil };
    Value::record([
        ("request_id", Value::from(p.request_id.as_str())),
        ("tool", Value::from(p.tool.as_str())),
        ("summary", Value::from(p.summary.as_str())),
        (
            "questions",
            Value::List(
                p.questions
                    .iter()
                    .map(|(text, options)| {
                        Value::record([
                            ("text", Value::from(text.as_str())),
                            ("options", Value::List(options.iter().map(|o| Value::from(o.as_str())).collect())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn draft_value(draft: Option<&Draft>) -> Value {
    match draft {
        Some(d) => Value::record([
            ("id", Value::from(d.id.as_str())),
            ("kind", Value::from(d.kind.id())),
            ("text", Value::from(d.text.as_str())),
        ]),
        None => Value::record([("id", Value::from("")), ("kind", Value::from("")), ("text", Value::from(""))]),
    }
}
