//! The engine's side of the island, in the `dove` process with or without
//! its window.
//!
//! - Serves the island ([`super::ipc`]): pushes a snapshot of every agent
//!   whenever it changes, runs the island's commands.
//! - **Headless**: with no window, commands go straight to the engine, and
//!   the sessions the closed window held are kept (and drained) here, then
//!   handed back when a window opens again.
//! - **Lifecycle**: closing the window keeps the engine only while something
//!   runs (an agent working or waiting on you, or automations kept running);
//!   otherwise Dove quits. A headless engine quits once the last of that work
//!   is done. The island stays either way.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dove_core::proto::{HarnessKind, ProjectId, TaskId, TaskStatus};
use dove_core::{Engine, LiveSession};
use gpui::{AnyWindowHandle, App, AppContext as _, Entity, Global, Subscription};

use super::ipc::{self, Hub, ToEngine, ToIsland};
use crate::notch::{NotchHost, Snapshot, bridge};
use crate::voice::{Dictation, DictationEvent};

/// Opens Dove's main window (returning it), from `main.rs`.
pub type OpenWindow = Rc<dyn Fn(&mut App) -> Option<AnyWindowHandle>>;

pub struct Host {
    engine: Arc<Engine>,
    hub: Hub,
    open_window: OpenWindow,
    /// Sessions kept while no window holds them.
    headless: HashMap<TaskId, LiveSession>,
    last: Option<Snapshot>,
    /// The attention version the last snapshot was built at.
    version: u64,
    /// When the window last closed (a grace period before quitting).
    closed_at: Option<Instant>,
    /// The agent the island has open (its text streams).
    watched: Option<TaskId>,
    /// Agents' pull requests, refreshed in the background about once a
    /// minute while an island is connected.
    prs: Vec<bridge::PrInfo>,
    prs_at: Option<Instant>,
    prs_busy: bool,
    automations: Vec<bridge::AutomationInfo>,
    /// Push-to-talk for the island's composer.
    dictation: Option<(Entity<Dictation>, Subscription)>,
}

impl Global for Host {}

/// An island is connected: it shows finishes, failures and approvals itself,
/// so Dove's own OS notifications stand down (`notify::os_notify`).
static SERVING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn serving() -> bool {
    SERVING.load(std::sync::atomic::Ordering::Relaxed)
}

/// After the window closes with nothing running, Dove waits this long for
/// work to appear (an island command, an automation) before quitting.
const IDLE_GRACE: Duration = Duration::from_secs(20);

impl Host {
    /// Starts serving. Returns false when another engine owns this data
    /// folder (the caller asks it to show its window and exits).
    pub fn start(engine: Arc<Engine>, open_window: OpenWindow, cx: &mut App) -> bool {
        let root = engine.store.root().to_path_buf();
        let token = ipc::new_token(&root);
        let hub = Hub::default();
        let (in_tx, in_rx) = flume::unbounded::<ToEngine>();
        let (conn_tx, conn_rx) = flume::unbounded::<()>();
        if !ipc::serve(&engine.runtime(), &root, token, hub.clone(), in_tx, conn_tx) {
            return false;
        }
        cx.set_global(Host {
            engine: engine.clone(),
            hub,
            open_window,
            headless: HashMap::new(),
            last: None,
            version: u64::MAX,
            // A headless start (the island launching Dove to run a command)
            // gets the same grace as a closed window, so it's still there when
            // the island connects.
            closed_at: Some(Instant::now()),
            watched: None,
            prs: Vec::new(),
            prs_at: None,
            prs_busy: false,
            automations: Vec::new(),
            dictation: None,
        });
        // Commands from islands.
        cx.spawn(async move |cx| {
            while let Ok(msg) = in_rx.recv_async().await {
                cx.update(|cx| Host::run(msg, cx));
            }
        })
        .detach();
        // A new island gets the current snapshot at once.
        cx.spawn(async move |cx| {
            while conn_rx.recv_async().await.is_ok() {
                cx.update(|cx| {
                    cx.global_mut::<Host>().last = None;
                    Host::publish(cx);
                });
            }
        })
        .detach();
        // Snapshots on attention changes, a sweep every 2 s for what the
        // model doesn't cover (titles, Flock), headless draining and the
        // idle quit.
        let pings = engine.attention().subscribe();
        cx.spawn(async move |cx| {
            let mut sweep = Instant::now();
            loop {
                let ping = pings.recv_async();
                let tick = cx.background_executor().timer(Duration::from_millis(500));
                futures_lite_race(ping, tick).await;
                let periodic = sweep.elapsed() >= Duration::from_secs(2);
                if periodic {
                    sweep = Instant::now();
                }
                cx.update(|cx| Host::tick(periodic, cx));
            }
        })
        .detach();
        true
    }

    fn tick(periodic: bool, cx: &mut App) {
        let headless = !Self::window_open(cx);
        if headless {
            // Nobody renders these sessions' events; the transcript has them.
            for live in cx.global::<Host>().headless.values() {
                while live.events.try_recv().is_ok() {}
            }
        }
        SERVING.store(!cx.global::<Host>().hub.is_empty(), std::sync::atomic::Ordering::Relaxed);
        if !cx.global::<Host>().hub.is_empty() {
            let version = cx.global::<Host>().engine.attention().version();
            let changed = cx.global::<Host>().version != version;
            if periodic {
                let engine = cx.global::<Host>().engine.clone();
                cx.global_mut::<Host>().automations = bridge::automations(&engine);
                Self::refresh_prs(cx);
            }
            if changed || periodic {
                cx.global_mut::<Host>().version = version;
                Self::publish(cx);
            }
        }
        if headless && periodic {
            let host = cx.global::<Host>();
            let idle = host.closed_at.is_none_or(|t| t.elapsed() >= IDLE_GRACE);
            if idle && !Self::busy(&host.engine) {
                tracing::info!("headless: nothing left to run; quitting");
                cx.global::<Host>().hub.broadcast(&ToIsland::Bye);
                cx.quit();
            }
        }
    }

    /// Whether the engine has work worth staying for: an agent working or
    /// waiting on the user, or automations the user keeps running.
    pub fn busy(engine: &Engine) -> bool {
        let working = engine
            .attention()
            .all()
            .iter()
            .any(|(_, s)| s.live && (matches!(s.status, TaskStatus::Running | TaskStatus::NeedsInput) || !s.pending.is_empty()));
        working || engine.store.setting(crate::notify::KEEP_RUNNING).as_deref() == Some("true")
    }

    fn window_open(cx: &App) -> bool {
        cx.try_global::<NotchHost>().is_some_and(|h| h.shell.upgrade().is_some())
    }

    /// Sends the snapshot to every island if it changed.
    fn publish(cx: &mut App) {
        let mut snap = match bridge::snapshot(cx) {
            Some(s) => s,
            None => engine_snapshot(&cx.global::<Host>().engine),
        };
        let host = cx.global::<Host>();
        snap.focus = host.watched.map(|id| {
            let state = host.engine.attention().get(id).unwrap_or_default();
            let mut focus = bridge::Focus::of(id, &state);
            // A thread the engine hasn't seen run since it started: its
            // last reply, from the view or the transcript.
            if focus.turns.is_empty()
                && let Some(a) = snap.agents.iter().find(|a| a.id == id)
                && !a.reply.trim().is_empty()
            {
                focus.turns.push(bridge::TurnView { reply: a.reply.clone(), done: true, ..Default::default() });
            }
            focus
        });
        snap.prs = host.prs.clone();
        snap.automations = host.automations.clone();
        let host = cx.global_mut::<Host>();
        if host.last.as_ref() == Some(&snap) {
            return;
        }
        host.hub.broadcast(&ToIsland::Snapshot { snap: snap.clone() });
        host.last = Some(snap);
    }

    fn run(msg: ToEngine, cx: &mut App) {
        let ui = Self::window_open(cx);
        let engine = cx.global::<Host>().engine.clone();
        match msg {
            ToEngine::Hello { .. } => {}
            ToEngine::Send { task, text } => {
                if ui {
                    bridge::with_shell(cx, |s, w, cx| s.task_send(task, text, w, cx));
                } else if !engine.send(task, text.clone()) {
                    Self::resume_headless(&engine, task, text, cx);
                }
            }
            ToEngine::Decide { task, request_id, decision } => {
                if ui {
                    bridge::with_shell(cx, |s, w, cx| s.task_decide(task, request_id, decision, w, cx));
                } else {
                    engine.decide(task, request_id, decision);
                }
            }
            ToEngine::Stop { task } => {
                if ui {
                    bridge::with_shell(cx, |s, w, cx| s.task_interrupt(task, w, cx));
                } else {
                    engine.interrupt(task);
                }
            }
            ToEngine::Start { id, agent, project, prompt, worktree } => {
                let result = if ui {
                    bridge::with_shell(cx, |s, w, cx| s.notch_start(agent, project, prompt, worktree, w, cx))
                        .unwrap_or_else(|| Err("Dove's window isn't ready.".into()))
                } else {
                    Self::start_headless(&engine, agent, project, prompt, worktree, cx)
                };
                cx.global::<Host>().hub.broadcast(&ToIsland::Started { id, result });
            }
            ToEngine::Open { task } => {
                Self::show(cx);
                if let Some(task) = task {
                    bridge::with_shell(cx, |s, w, cx| s.notch_open(task, w, cx));
                } else {
                    bridge::with_shell(cx, |_, w, _| {
                        w.activate_window();
                    });
                }
            }
            ToEngine::OpenSettings => {
                Self::show(cx);
                bridge::with_shell(cx, |s, w, cx| s.notch_open_settings(w, cx));
            }
            ToEngine::OpenFlock => {
                Self::show(cx);
                bridge::with_shell(cx, |s, w, cx| s.notch_open_flock(w, cx));
            }
            ToEngine::Answer { task, request_id, answers } => {
                if ui {
                    bridge::with_shell(cx, |s, w, cx| s.task_answer(task, request_id, answers, w, cx));
                } else {
                    engine.answer(task, request_id, answers);
                }
            }
            ToEngine::Watch { task } => {
                engine.attention().watch_stream(task);
                cx.global_mut::<Host>().watched = task;
            }
            ToEngine::Merge { task, number } => Self::merge(task, number, cx),
            ToEngine::RunAutomation { id } => {
                let Some(a) = engine.store.automations().unwrap_or_default().into_iter().find(|a| a.id == id) else {
                    return;
                };
                if ui {
                    bridge::with_shell(cx, |s, w, cx| s.run_automation(&a, None, w, cx));
                } else {
                    match engine.run_automation(&a, None) {
                        Ok((task, live)) => {
                            cx.global_mut::<Host>().headless.insert(task.id, live);
                        }
                        Err(e) => tracing::warn!("automation {} didn't start: {e:#}", a.name),
                    }
                }
                cx.global_mut::<Host>().automations = bridge::automations(&engine);
            }
            ToEngine::Dictate { on } => Self::dictate(on, cx),
            ToEngine::QuitAll => {
                tracing::info!("island asked Dove to quit");
                cx.global::<Host>().hub.broadcast(&ToIsland::Bye);
                cx.quit();
            }
        }
        cx.global_mut::<Host>().last = None;
        Self::publish(cx);
    }

    /// Push-to-talk: the mic is pressed or released. A short tap keeps it
    /// listening until the next press (as in the composer).
    fn dictate(on: bool, cx: &mut App) {
        if cx.global::<Host>().dictation.is_none() {
            let engine = cx.global::<Host>().engine.clone();
            let d = cx.new(|cx| Dictation::new(engine, cx));
            let sub = cx.subscribe(&d, |d, ev: &DictationEvent, cx| {
                let msg = match ev {
                    DictationEvent::Text(t) => Some(ToIsland::Dictation { text: t.clone(), done: true, error: None }),
                    DictationEvent::Live(t) => Some(ToIsland::Dictation { text: t.clone(), done: false, error: None }),
                    DictationEvent::LiveDone(t) => Some(ToIsland::Dictation { text: t.clone(), done: true, error: None }),
                    _ => d.read(cx).error.clone().map(|e| ToIsland::Dictation { text: String::new(), done: true, error: Some(e) }),
                };
                if let Some(m) = msg {
                    cx.global::<Host>().hub.broadcast(&m);
                }
            });
            cx.global_mut::<Host>().dictation = Some((d, sub));
        }
        let Some(d) = cx.global::<Host>().dictation.as_ref().map(|(d, _)| d.clone()) else { return };
        d.update(cx, |d, cx| if on { d.press(cx) } else { d.release(cx) });
    }

    /// Looks up the pull requests of recently active agents' branches, off
    /// the UI thread, at most once a minute.
    fn refresh_prs(cx: &mut App) {
        let host = cx.global::<Host>();
        if host.prs_busy || host.prs_at.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
            return;
        }
        let engine = host.engine.clone();
        let week = dove_core::store::now_ms() - 3 * 86_400_000;
        let mut work: Vec<(TaskId, HarnessKind, std::path::PathBuf)> = Vec::new();
        for p in engine.store.projects().unwrap_or_default() {
            for t in engine.store.tasks(p.id).unwrap_or_default() {
                if let Some(wt) = t.worktree.clone()
                    && t.updated_at >= week
                    && !matches!(t.status, TaskStatus::Draft | TaskStatus::Archived)
                    && wt.exists()
                {
                    work.push((t.id, t.harness, wt));
                }
            }
        }
        work.sort_by_key(|(id, _, _)| std::cmp::Reverse(engine.store.task(*id).ok().flatten().map(|t| t.updated_at)));
        work.truncate(8);
        let host = cx.global_mut::<Host>();
        host.prs_at = Some(Instant::now());
        if work.is_empty() {
            host.prs.clear();
            return;
        }
        host.prs_busy = true;
        let job = cx.background_executor().spawn(async move {
            if !dove_core::git::github::gh_available() {
                return Vec::new();
            }
            work.into_iter()
                .filter_map(|(task, kind, wt)| {
                    let pr = dove_core::git::github::pr_for_branch(&wt).ok().flatten()?;
                    Some(bridge::PrInfo {
                        task,
                        kind,
                        number: pr.number,
                        title: pr.title.clone(),
                        url: pr.url.clone(),
                        state: pr.status().to_string(),
                        checks: pr.checks_summary().to_string(),
                        mergeable: pr.mergeable == "MERGEABLE",
                    })
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |cx| {
            let prs = job.await;
            cx.update(|cx| {
                let host = cx.global_mut::<Host>();
                host.prs = prs;
                host.prs_busy = false;
                host.last = None;
                Host::publish(cx);
            });
        })
        .detach();
    }

    fn merge(task: TaskId, number: u64, cx: &mut App) {
        let engine = cx.global::<Host>().engine.clone();
        let Some(wt) = engine.store.task(task).ok().flatten().and_then(|t| t.worktree) else {
            cx.global::<Host>().hub.broadcast(&ToIsland::Merged { task, result: Err("That thread has no worktree.".into()) });
            return;
        };
        let job = cx
            .background_executor()
            .spawn(async move { dove_core::git::github::merge_pr(&wt, number).map_err(|e| format!("{e:#}")) });
        cx.spawn(async move |cx| {
            let result = job.await;
            cx.update(|cx| {
                cx.global::<Host>().hub.broadcast(&ToIsland::Merged { task, result });
                let host = cx.global_mut::<Host>();
                host.prs_at = None;
                Host::refresh_prs(cx);
            });
        })
        .detach();
    }

    /// Opens the window if it's closed, handing it the sessions kept while
    /// headless.
    pub fn show(cx: &mut App) {
        if Self::window_open(cx) {
            if let Some(h) = cx.try_global::<NotchHost>().cloned() {
                let _ = h.window.update(cx, |_, window, _| window.activate_window());
            }
            return;
        }
        let open = cx.global::<Host>().open_window.clone();
        if open(cx).is_none() {
            return;
        }
        let sessions: Vec<LiveSession> = cx.global_mut::<Host>().headless.drain().map(|(_, l)| l).collect();
        cx.global_mut::<Host>().closed_at = None;
        if !sessions.is_empty() {
            bridge::with_shell(cx, move |s, w, cx| {
                for live in sessions {
                    s.adopt_live(live, w, cx);
                }
            });
        }
    }

    /// The window is closing: keep its sessions and stay headless if
    /// there's work, else quit. Returns whether the window may close.
    pub fn on_window_close(cx: &mut App) -> bool {
        let engine = cx.global::<Host>().engine.clone();
        let sessions = bridge::with_shell(cx, |s, _, cx| s.surrender_sessions(cx)).unwrap_or_default();
        let host = cx.global_mut::<Host>();
        for live in sessions {
            host.headless.insert(live.task, live);
        }
        host.closed_at = Some(Instant::now());
        if !Self::busy(&engine) && host.hub.is_empty() {
            // Nothing running and no island to serve: done.
            cx.quit();
        }
        true
    }

    fn resume_headless(engine: &Engine, task: TaskId, text: String, cx: &mut App) {
        let Ok(Some(t)) = engine.store.task(task) else { return };
        let Some(project) = engine.store.projects().ok().and_then(|ps| ps.into_iter().find(|p| p.id == t.project_id))
        else {
            return;
        };
        match engine.resume_task(&t, &project, Some(text), Default::default()) {
            Ok(live) => {
                cx.global_mut::<Host>().headless.insert(task, live);
            }
            Err(e) => tracing::warn!("headless resume of {task} failed: {e:#}"),
        }
    }

    fn start_headless(
        engine: &Engine,
        agent: HarnessKind,
        project: ProjectId,
        prompt: String,
        worktree: bool,
        cx: &mut App,
    ) -> Result<TaskId, String> {
        let project = engine
            .store
            .projects()
            .ok()
            .and_then(|ps| ps.into_iter().find(|p| p.id == project))
            .ok_or("That project is gone.")?;
        let join = if worktree {
            let w = dove_core::git::create_worktree(&project.path, &dove_core::git::slug(&prompt), "dove/")
                .map_err(|e| format!("Couldn't make a worktree: {e}"))?;
            Some(dove_core::Joined { worktree: Some(w.path), branch: Some(w.branch), base: Some(w.base) })
        } else {
            None
        };
        let new = dove_core::NewTask {
            title: crate::ui::title_from_prompt(&prompt),
            prompt,
            harness: agent,
            run: Default::default(),
            join,
            persona: None,
        };
        let (task, live) = engine.start_task(&project, new).map_err(|e| format!("It didn't start: {e}"))?;
        cx.global_mut::<Host>().headless.insert(task.id, live);
        Ok(task.id)
    }
}

/// Waits for whichever finishes first (both are cancel-safe).
async fn futures_lite_race<A: std::future::Future, B: std::future::Future>(a: A, b: B) {
    use std::pin::pin;
    use std::task::Poll;
    let mut a = pin!(a);
    let mut b = pin!(b);
    std::future::poll_fn(|cx| {
        if a.as_mut().poll(cx).is_ready() || b.as_mut().poll(cx).is_ready() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await
}

/// Waits for whichever finishes first; `Some` with `a`'s output when it was
/// `a`, `None` when `b` (a timer, say) came first.
pub async fn race_value<A: std::future::Future, B: std::future::Future>(a: A, b: B) -> Option<A::Output> {
    use std::pin::pin;
    use std::task::Poll;
    let mut a = pin!(a);
    let mut b = pin!(b);
    std::future::poll_fn(|cx| {
        if let Poll::Ready(v) = a.as_mut().poll(cx) {
            return Poll::Ready(Some(v));
        }
        if b.as_mut().poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        Poll::Pending
    })
    .await
}

/// The island's snapshot from the engine and the store alone (no window).
pub fn engine_snapshot(engine: &Engine) -> Snapshot {
    let projects = engine.store.projects().unwrap_or_default();
    let mut tasks = Vec::new();
    for p in &projects {
        for t in engine.store.tasks(p.id).unwrap_or_default() {
            tasks.push((t, p.clone()));
        }
    }
    tasks.sort_by_key(|(t, _)| -t.updated_at);
    let mut agents = Vec::new();
    let mut recent = 0;
    for (task, project) in tasks.iter().filter(|(t, _)| !matches!(t.status, TaskStatus::Draft | TaskStatus::Archived)) {
        let state = engine.attention().get(task.id);
        let busy = state.as_ref().is_some_and(|s| s.live && (s.needs_you() || s.status == TaskStatus::Running));
        if !busy {
            if recent >= 6 {
                continue;
            }
            recent += 1;
        }
        agents.push(bridge::agent_from(task, Some(project), state.as_ref(), None));
    }
    bridge::sort_agents(&mut agents);
    let installed: Vec<_> = engine.installed().iter().map(|i| i.kind).collect();
    Snapshot {
        agents,
        flock: Vec::new(),
        limits: bridge::meters(engine, &installed),
        installed,
        projects: projects.iter().map(|p| (p.id, p.name.clone())).collect(),
        default_project: engine
            .store
            .setting("active_project")
            .and_then(|s| projects.iter().find(|p| p.id.to_string() == s).map(|p| p.id)),
        default_agent: engine.installed().first().map(|i| i.kind),
        focus: None,
        prs: Vec::new(),
        automations: Vec::new(),
    }
}
