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
//!
//! Calls:
//!   kx.open(id)                     an action: open that thread
//!   kx.new()                        an action: no thread open (Home)
//!   kx.send({ text, project? })     run a message in the open thread, or a new one
//!   kx.interrupt()                  an action: stop the open thread's turn
//!
//! A live session's events fold into its thread's transcript whether it's
//! open or not; only the open one's reach the window, and then only what
//! changed: a streamed word writes `kx.draft` alone, and the list is
//! published again only when an entry comes, goes or changes, which
//! `Shared::set` reconciles by `id` so the rest keep their rows.

use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

use dove_core::{Command, Engine, LiveSession, NewTask, RunSettings};
use dove_proto::TaskId;
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Shared, Value};

use crate::transcript::{Change, Draft, Entry, Fold};

/// A thread with a running harness.
struct Live {
    session: Rc<LiveSession>,
    fold: Rc<RefCell<Fold>>,
}

struct State {
    engine: Arc<Engine>,
    open: Option<TaskId>,
    /// The open thread's transcript when it has no live session.
    replayed: Option<Rc<RefCell<Fold>>>,
    live: HashMap<TaskId, Live>,
    threads: Shared,
    projects: Shared,
    thread: Shared,
    entries: Shared,
    draft: Shared,
}

type Host = Rc<RefCell<State>>;

pub fn install(cx: &mut WindowContext, host: &ScriptHost, engine: Arc<Engine>) {
    let state: Host = Rc::new(RefCell::new(State {
        engine,
        open: None,
        replayed: None,
        live: HashMap::new(),
        threads: host.shared("kx.threads", Value::List(Rc::from([]))),
        projects: host.shared("kx.projects", Value::List(Rc::from([]))),
        thread: host.shared("kx.thread", Value::Nil),
        entries: host.shared("kx.entries", Value::List(Rc::from([]))),
        draft: host.shared("kx.draft", draft_value(None)),
    }));
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
    });

    let s = state.clone();
    host.action("kx.interrupt", move |_| {
        let st = s.borrow();
        if let Some(id) = st.open {
            st.engine.interrupt(id);
        }
    });

    let (s, tasks) = (state.clone(), cx.tasks.clone());
    host.async_action("kx.send", move |req| {
        let (s, tasks) = (s.clone(), tasks.clone());
        async move { send(&tasks, &s, req).await.map(|()| Value::Nil) }
    });

    start_scheduler(&cx.tasks, &state);
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

fn parse_task(value: &Value) -> Option<TaskId> {
    let Value::Str(id) = value else { return None };
    id.parse().ok().map(TaskId)
}

/// `f` on a thread of its own, its answer awaited.
async fn off_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
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
            (projects, threads)
        })
        .await;
        let Some((projects, threads)) = read else { return };
        let names: HashMap<_, _> = projects.iter().map(|p| (p.id, p.name.clone())).collect();
        let st = s.borrow();
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
                    let live = st.live.get(&t.id).is_some_and(|l| l.fold.borrow().running);
                    Value::record([
                        ("id", Value::from(t.id.0.to_string().as_str())),
                        ("title", Value::from(t.title.as_str())),
                        ("project", Value::from(names.get(&t.project_id).map(String::as_str).unwrap_or(""))),
                        ("status", Value::from(if live { "running" } else { t.status.id() })),
                        ("harness", Value::from(t.harness.label())),
                        ("updated", Value::from(t.updated_at as f64)),
                        ("unread", Value::from(t.unread)),
                    ])
                })
                .collect(),
        ));
    });
}

fn open(tasks: &zgx_desktop::TaskSpawner, state: &Host, id: TaskId) {
    {
        let mut st = state.borrow_mut();
        st.open = Some(id);
        st.replayed = None;
        st.draft.set(draft_value(None));
    }
    if state.borrow().live.contains_key(&id) {
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
        && let Some(live) = state.borrow().live.get(&id)
        && live.session.send(Command::Send(text.clone()))
    {
        return Ok(());
    }
    let project = req.field("project").and_then(|p| match p {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    });
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
                let harness = engine
                    .installed()
                    .first()
                    .map(|i| i.kind)
                    .ok_or_else(|| anyhow::anyhow!("no agent installed"))?;
                let title: String = text.lines().next().unwrap_or("").chars().take(60).collect();
                let new = NewTask { title, prompt: text, harness, run: RunSettings::default(), join: None, persona: None };
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
    state.borrow_mut().live.insert(id, Live { session: session.clone(), fold: fold.clone() });
    let s = state.clone();
    let tasks_for_refresh = tasks.clone();
    tasks.spawn(async move {
        while let Ok(event) = session.events.recv_async().await {
            // Everything ready now, folded at once: one publish a burst.
            let mut change = fold.borrow_mut().apply(&event);
            let mut ended = matches!(event, dove_proto::AgentEvent::TurnEnded { .. });
            while let Ok(event) = session.events.try_recv() {
                ended |= matches!(event, dove_proto::AgentEvent::TurnEnded { .. });
                change = change.and(fold.borrow_mut().apply(&event));
            }
            if s.borrow().open == Some(id) {
                publish(&s, change);
            }
            if ended {
                spawn_refresh(&tasks_for_refresh, &s);
            }
        }
        // The session ended: its transcript stays, as a replay.
        let mut st = s.borrow_mut();
        if let Some(live) = st.live.remove(&id)
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
    let fold = match st.live.get(&id) {
        Some(live) => live.fold.clone(),
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
