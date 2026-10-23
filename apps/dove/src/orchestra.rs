//! The team tools, performed in the app: agents delegate work to any agent,
//! model or teammate, talk to teammates, and read, steer, wait on and stop
//! threads. Every action goes through the same paths a click uses (a new
//! thread starts like one from the composer, a message is sent like a typed
//! one), so permissions, transcripts and the sidebar behave the same.
//!
//! Results flow back on their own: when a child (or a teammate asked for a
//! reply) finishes, its final answer is batched with any siblings finishing
//! at the same time and delivered to the parent as one team update, which
//! steers the parent if it's working or starts a turn if it's idle.
//!
//! The rules (which agent, permission ceiling, depth, wording) live in
//! [`dove_core::orchestra`]; this module applies them to live threads.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use dove_core::orchestra as orc;
use dove_core::proto::{HarnessKind, PermissionMode, ReasoningLevel, TaskId, TaskStatus};
use dove_core::store::{LinkKind, LinkStatus, Persona, Task};
use dove_core::{Joined, NewTask, RunSettings};
use gpui::{AppContext as _, AsyncWindowContext, Context, WeakEntity, Window};
use serde_json::{Value, json};

use crate::browser::agent::Outcome;
use crate::browser::control::Request;
use crate::orchestrate::{self as ask, Orchestrate, Proposal, ProposalCard, ProposalEvent};
use crate::shell::Shell;
use crate::team_tools::{Args, parse_thread};

/// Live orchestration state the shell keeps.
#[derive(Default)]
pub struct TeamState {
    /// Threads a tool call is waiting on right now; their result goes to
    /// that call, not into a team update.
    pub waiting: HashSet<TaskId>,
    /// Reports per parent, batched before waking it.
    pending: HashMap<TaskId, (Instant, Vec<orc::Report>)>,
    /// The status each linked thread was last seen in, so each change is
    /// handled once.
    seen: HashMap<TaskId, TaskStatus>,
    /// Threads just sent a message, until they start working on it: until
    /// then their finished status is the previous turn's, not an answer.
    arming: HashMap<TaskId, Instant>,
    /// Threads holding a queued message: the turn they're finishing isn't
    /// the answer to it.
    queued: HashSet<TaskId>,
    sweep: Option<gpui::Task<()>>,
    /// Cards waiting for the user (`/orchestrator` in Ask), by id.
    proposals: HashMap<u64, (gpui::Entity<ProposalCard>, gpui::Subscription)>,
    next_proposal: u64,
    /// What the user decided on each parent's cards, and whether it differs
    /// from what the parent asked (then the parent hears about it).
    decisions: HashMap<TaskId, (Vec<String>, bool)>,
}

/// What a team call answers: now, or after waiting on a thread.
pub enum TeamReply {
    Now(Outcome),
    Wait(Waiter),
}

pub struct Waiter {
    pub thread: TaskId,
    pub deadline: Instant,
    pub started: Instant,
    /// Seen working since the wait began (a just-sent turn may not have
    /// flipped the status yet).
    seen_running: bool,
    /// What the answer reports besides the thread's state.
    base: Value,
    /// The delegated child whose result this wait consumes, if any.
    consumes: Option<TaskId>,
}

/// How long a thread just messaged has to start working before its status
/// counts again.
const ARMING: Duration = Duration::from_secs(8);

fn json_out(v: Value) -> Outcome {
    Outcome::ok(serde_json::to_string_pretty(&v).unwrap_or_default())
}

fn status_id(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Idle => "idle",
        TaskStatus::Draft => "draft",
        TaskStatus::Running => "running",
        TaskStatus::NeedsInput => "needs_input",
        TaskStatus::Done => "done",
        TaskStatus::Failed => "failed",
        TaskStatus::Archived => "archived",
    }
}

/// Wait until `w` is answerable, polling the shell.
pub async fn wait(this: WeakEntity<Shell>, mut w: Waiter, cx: &mut AsyncWindowContext) -> Outcome {
    loop {
        cx.background_executor().timer(Duration::from_millis(400)).await;
        let step = this.update(cx, |s, cx| s.team_poll(&mut w, cx));
        match step {
            Ok(Some(out)) => return out,
            Ok(None) => {}
            Err(_) => return Outcome::err("Dove closed before the thread finished."),
        }
    }
}

impl Shell {
    /// Start the background sweep that catches status changes of threads
    /// whose view isn't open (and delivers batched updates on time).
    pub(crate) fn start_team_sweep(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.team.sweep = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(700)).await;
                if this.update_in(cx, |s, window, cx| s.team_tick(window, cx)).is_err() {
                    break;
                }
            }
        }));
    }

    /// Answer one team tool call.
    pub(crate) fn route_team_call(&mut self, req: &Request, window: &mut Window, cx: &mut Context<Self>) -> TeamReply {
        let caller = match self.team_caller(req) {
            Ok(c) => c,
            Err(e) => return TeamReply::Now(Outcome::err(e)),
        };
        let a = Args(&req.args);
        let result = match req.tool.as_str() {
            "team_capabilities" => Ok(TeamReply::Now(self.team_capabilities(caller, &a, cx))),
            "delegate_task" => self.team_delegate(caller, &a, window, cx),
            "delegate_tasks" => self.team_delegate_many(caller, &req.args, window, cx).map(TeamReply::Now),
            "task_status" => self.team_task_status(caller, &a).map(TeamReply::Now),
            "task_cancel" => self.team_task_cancel(caller, &a, cx).map(TeamReply::Now),
            "team_message" => self.team_message(caller, &a, window, cx),
            "thread_list" => Ok(TeamReply::Now(self.team_thread_list(caller, &a))),
            "thread_read" => self.team_thread_read(&a).map(TeamReply::Now),
            "thread_send" => self.team_thread_send(caller, &a, window, cx),
            "thread_wait" => self.team_thread_wait(&a),
            "thread_interrupt" => self.team_thread_interrupt(&a, cx).map(TeamReply::Now),
            other => Err(format!("Unknown team tool {other}.")),
        };
        result.unwrap_or_else(|e| TeamReply::Now(Outcome::err(e)))
    }

    /// The thread a call comes from: its session key, else the one thread
    /// working in its folder.
    pub(crate) fn team_caller(&self, req: &Request) -> Result<TaskId, String> {
        if let Some(id) = self.engine.task_for_session(&req.session)
            && self.task(id).is_some()
        {
            return Ok(id);
        }
        let norm =
            |p: &std::path::Path| p.display().to_string().replace('\\', "/").trim_end_matches('/').to_lowercase();
        if let Some(cwd) = &req.cwd {
            let want = norm(cwd);
            let here: Vec<&Task> = self
                .tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Running)
                .filter(|t| {
                    let dir = t
                        .worktree
                        .clone()
                        .or_else(|| self.project(t.project_id).map(|p| p.path.clone()))
                        .unwrap_or_default();
                    norm(&dir) == want
                })
                .collect();
            if let [only] = here.as_slice() {
                return Ok(only.id);
            }
        }
        Err("Dove couldn't tell which thread this call comes from (start a new thread so its session is \
             registered, then try again)."
            .into())
    }

    /// `/orchestrator` for the thread that started `id`'s tree of agents.
    pub(crate) fn orchestrate_of(&self, id: TaskId) -> Orchestrate {
        let parents = self.parent_map();
        let mut root = id;
        for _ in 0..16 {
            match parents.get(&root) {
                Some(p) => root = *p,
                None => break,
            }
        }
        ask::get(&self.engine.store, Some(root))
    }

    /// Put a spawn in front of the user as a card above `parent`'s input.
    #[allow(clippy::too_many_arguments)]
    fn propose(
        &mut self,
        parent: TaskId,
        kind: LinkKind,
        task: String,
        title: String,
        teammate: Option<Persona>,
        pick: orc::Pick,
        reasoning: Option<ReasoningLevel>,
        mode: PermissionMode,
        isolate: bool,
        request_key: Option<String>,
        wants_reply: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Outcome {
        let id = self.team.next_proposal;
        self.team.next_proposal += 1;
        let ceiling = self.mode_of(parent, cx);
        let proposal = Proposal { id, parent, kind, title: title.clone(), teammate, ceiling, request_key, wants_reply };
        let engine = self.engine.clone();
        let agents = self.installed_kinds();
        let card = cx.new(|cx| {
            ProposalCard::new(
                proposal,
                engine,
                agents,
                task,
                (pick.harness, pick.model.clone()),
                reasoning,
                mode,
                isolate,
                window,
                cx,
            )
        });
        let sub = cx.subscribe_in(&card, window, move |this, card, ev: &ProposalEvent, window, cx| match ev {
            ProposalEvent::Approve => this.approve_proposal(card.clone(), window, cx),
            ProposalEvent::Skip => this.skip_proposal(card.clone(), window, cx),
            ProposalEvent::ApproveAll => {
                let parent = card.read(cx).proposal.parent;
                let root = {
                    let parents = this.parent_map();
                    let mut r = parent;
                    while let Some(p) = parents.get(&r) {
                        r = *p;
                    }
                    r
                };
                ask::set(&this.engine.store, Some(root), Orchestrate::Auto);
                let all: Vec<gpui::Entity<ProposalCard>> = this
                    .team
                    .proposals
                    .values()
                    .map(|(c, _)| c.clone())
                    .filter(|c| c.read(cx).proposal.parent == parent)
                    .collect();
                for c in all {
                    this.approve_proposal(c, window, cx);
                }
            }
        });
        self.team.proposals.insert(id, (card, sub));
        self.sync_waiting(cx);
        // The user sees the card where the thread is; take them there if
        // they are looking at it already, and let the sidebar mark it.
        if let Some(v) = self.threads.get(&parent) {
            let chat = v.read(cx).chat.clone();
            chat.update(cx, |_, cx| cx.notify());
        }
        crate::kit::Toasts::push(
            cx,
            "An agent is waiting for your approval",
            Some(format!("\"{title}\" — review it above the input in its thread.")),
            crate::kit::Tone::Info,
            None,
        );
        json_out(json!({
            "status": "awaiting_approval",
            "proposal_id": id,
            "title": title,
            "note": "The user reviews it in Dove and may change the task, agent, model, effort or permission. End \
                     your turn when you have nothing else to do: Dove tells you what was started or skipped, and \
                     started agents report back when they finish.",
        }))
    }

    /// Publish the waiting cards for the chats to draw.
    fn sync_waiting(&mut self, cx: &mut Context<Self>) {
        let mut cards: Vec<(u64, gpui::Entity<ProposalCard>)> =
            self.team.proposals.iter().map(|(id, (c, _))| (*id, c.clone())).collect();
        cards.sort_by_key(|(id, _)| *id);
        let parents: HashSet<TaskId> = cards.iter().map(|(_, c)| c.read(cx).proposal.parent).collect();
        cx.set_global(ask::Waiting(cards.into_iter().map(|(_, c)| c).collect()));
        for p in parents.into_iter().chain(self.team.decisions.keys().copied().collect::<Vec<_>>()) {
            if let Some(v) = self.threads.get(&p) {
                let chat = v.read(cx).chat.clone();
                chat.update(cx, |_, cx| cx.notify());
            }
        }
        cx.notify();
    }

    fn approve_proposal(&mut self, card: gpui::Entity<ProposalCard>, window: &mut Window, cx: &mut Context<Self>) {
        let (id, parent, kind, teammate, key, wants_reply) = {
            let p = &card.read(cx).proposal;
            (p.id, p.parent, p.kind, p.teammate.clone(), p.request_key.clone(), p.wants_reply)
        };
        let choice = card.read(cx).choice(cx);
        let title = card.read(cx).title(cx);
        match self.start_proposal(parent, kind, &choice, title.clone(), teammate, key, wants_reply, window, cx) {
            Ok(child) => {
                self.team.proposals.remove(&id);
                let line = format!(
                    "\"{title}\" was approved{} and started on {} (task_id {}).",
                    if choice.edited { " with the user's changes" } else { "" },
                    choice.agent.label(),
                    child.0
                );
                self.decided(parent, line, choice.edited, window, cx);
            }
            Err(e) => {
                card.update(cx, |c, cx| {
                    c.error = Some(e.into());
                    cx.notify();
                });
            }
        }
    }

    fn skip_proposal(&mut self, card: gpui::Entity<ProposalCard>, window: &mut Window, cx: &mut Context<Self>) {
        let (id, parent) = {
            let p = &card.read(cx).proposal;
            (p.id, p.parent)
        };
        let title = card.read(cx).title(cx);
        self.team.proposals.remove(&id);
        self.decided(
            parent,
            format!("\"{title}\" was skipped by the user; don't start it again unless they ask."),
            true,
            window,
            cx,
        );
    }

    /// Record a decision; once a parent has none left waiting, tell it what
    /// differed from its request.
    fn decided(&mut self, parent: TaskId, line: String, differs: bool, window: &mut Window, cx: &mut Context<Self>) {
        let entry = self.team.decisions.entry(parent).or_default();
        entry.0.push(line);
        entry.1 |= differs;
        let left = self.team.proposals.values().any(|(c, _)| c.read(cx).proposal.parent == parent);
        if !left
            && let Some((lines, true)) = self.team.decisions.remove(&parent)
            && let Err(e) = self.deliver_text(parent, &ask::decisions_note(&lines), window, cx)
        {
            tracing::warn!("decision note for {parent:?} not delivered: {e}");
        }
        self.sync_waiting(cx);
    }

    /// Start an approved spawn, checking the limits again (time has passed).
    #[allow(clippy::too_many_arguments)]
    fn start_proposal(
        &mut self,
        parent: TaskId,
        kind: LinkKind,
        choice: &ask::Choice,
        title: String,
        teammate: Option<Persona>,
        key: Option<String>,
        wants_reply: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<TaskId, String> {
        if choice.task.is_empty() {
            return Err("Give the agent a task.".into());
        }
        if kind == LinkKind::Delegate {
            let active = self
                .engine
                .store
                .task_children(parent)
                .iter()
                .filter(|l| l.kind == LinkKind::Delegate && !l.status.terminal())
                .count();
            if active >= orc::MAX_ACTIVE_CHILDREN {
                return Err(format!(
                    "{active} agents from this thread are already working (the limit is {}).",
                    orc::MAX_ACTIVE_CHILDREN
                ));
            }
        }
        let caller_task = self.task(parent).cloned().ok_or("Its thread is gone.")?;
        let pick = orc::pick_agent(
            Some(choice.agent.id()),
            choice.model.as_deref(),
            teammate.as_ref(),
            caller_task.harness,
            &self.installed_kinds(),
        )?;
        let mode = orc::cap_mode(self.mode_of(parent, cx), Some(choice.mode));
        let from = self.sender(parent);
        let prompt = match kind {
            LinkKind::Message => orc::message_prompt(&from, &choice.task, wants_reply),
            _ => orc::delegation_prompt(&from, &choice.task, choice.isolate),
        };
        let run = RunSettings { mode, model: pick.model.clone(), reasoning: choice.reasoning, ..Default::default() };
        let child = self.start_child(parent, prompt, title, teammate, pick, run, choice.isolate, window, cx)?;
        let store = &self.engine.store;
        if kind == LinkKind::Delegate || wants_reply {
            store.link_task(child.id, parent, kind, true).map_err(|e| e.to_string())?;
        }
        if let Some(k) = &key {
            let _ = store.set_link_request_key(child.id, k);
        }
        Ok(child.id)
    }

    fn installed_kinds(&self) -> Vec<HarnessKind> {
        self.installed
            .iter()
            .map(|i| i.kind)
            .filter(|k| *k != HarnessKind::Shell && dove_core::harness_supported(*k))
            .collect()
    }

    /// The caller's permission mode, as its composer says.
    fn mode_of(&self, id: TaskId, cx: &gpui::App) -> PermissionMode {
        self.threads
            .get(&id)
            .map(|v| v.read(cx).chat.read(cx).composer.read(cx).run_settings(cx).mode)
            .unwrap_or_default()
    }

    fn teammate_of(&self, id: TaskId) -> Option<&Persona> {
        self.persona_of.get(&id).and_then(|p| self.persona(p))
    }

    fn who(&self, id: TaskId) -> String {
        let Some(t) = self.task(id) else { return "A thread".into() };
        match self.teammate_of(id) {
            Some(p) => format!("{} on {}", p.name.trim(), t.harness.label()),
            None => t.harness.label().to_string(),
        }
    }

    fn sender(&self, id: TaskId) -> String {
        let t = self.task(id);
        orc::sender_label(
            self.teammate_of(id),
            t.map(|t| t.title.as_str()).unwrap_or("a thread"),
            t.map(|t| t.harness).unwrap_or(HarnessKind::ClaudeCode),
        )
    }

    fn parent_map(&self) -> HashMap<TaskId, TaskId> {
        self.engine.store.task_links().into_iter().map(|l| (l.child, l.parent)).collect()
    }

    fn thread_brief(&self, t: &Task) -> Value {
        let link = self.engine.store.task_link(t.id);
        json!({
            "thread_id": t.id.0.to_string(),
            "title": t.title,
            "agent": t.harness.label(),
            "model": t.model,
            "teammate": self.teammate_of(t.id).map(|p| p.name.clone()),
            "status": status_id(t.status),
            "started_by": link.map(|l| l.parent.0.to_string()),
            "updated": dove_core::store::now_ms().saturating_sub(t.updated_at) / 1000,
        })
    }

    // ------------------------------------------------------------ tools

    fn team_capabilities(&self, caller: TaskId, a: &Args, cx: &mut Context<Self>) -> Outcome {
        let search = a.str("search").map(|s| s.to_lowercase());
        let me = self.task(caller).cloned();
        let agents: Vec<Value> = self
            .installed_kinds()
            .into_iter()
            .map(|k| {
                let models = self.engine.models(k);
                let matching: Vec<&dove_core::proto::Model> = models
                    .iter()
                    .filter(|m| {
                        search
                            .as_ref()
                            .is_none_or(|q| m.id.to_lowercase().contains(q) || m.label.to_lowercase().contains(q))
                    })
                    .collect();
                let shown: Vec<Value> = matching
                    .iter()
                    .take(12)
                    .map(|m| {
                        json!({
                            "id": m.id,
                            "name": m.label,
                            "reasoning": m.reasoning_levels.iter().map(|r| r.id()).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                json!({
                    "agent": k.id(),
                    "name": k.label(),
                    "models": shown,
                    "more_models": matching.len().saturating_sub(12),
                })
            })
            .collect();
        let team: Vec<Value> = self
            .personas
            .iter()
            .map(|p| {
                let threads = self.persona_tasks(&p.id);
                json!({
                    "id": p.id,
                    "name": p.name,
                    "role": p.role,
                    "prefers_agent": p.harness.map(|h| h.label()),
                    "prefers_model": p.model,
                    "threads": threads.iter().take(5).map(|t| self.thread_brief(t)).collect::<Vec<_>>(),
                })
            })
            .collect();
        let children: Vec<Value> = self
            .engine
            .store
            .task_children(caller)
            .iter()
            .map(|l| json!({"task_id": l.child.0.to_string(), "status": l.status.id(), "kind": l.kind.id()}))
            .collect();
        let depth = orc::depth(caller, &self.parent_map());
        json!({
            "you": me.as_ref().map(|t| json!({
                "thread_id": t.id.0.to_string(),
                "title": t.title,
                "agent": t.harness.label(),
                "model": t.model,
                "teammate": self.teammate_of(t.id).map(|p| p.name.clone()),
                "permission": orc::mode_label(self.mode_of(t.id, cx)),
                "project": self.project(t.project_id).map(|p| p.name.clone()),
                "depth": depth,
                "children": children,
            })),
            "agents": agents,
            "teammates": team,
            "limits": {
                "max_depth": orc::MAX_DEPTH,
                "max_active_children": orc::MAX_ACTIVE_CHILDREN,
                "max_wait_seconds": crate::team_tools::MAX_WAIT_SECS,
                "can_delegate": depth < orc::MAX_DEPTH,
            },
        })
        .pipe(json_out)
    }

    /// Start a child thread for a task (or a teammate's new thread).
    #[allow(clippy::too_many_arguments)]
    fn start_child(
        &mut self,
        caller: TaskId,
        prompt: String,
        title: String,
        teammate: Option<Persona>,
        pick: orc::Pick,
        run: RunSettings,
        isolate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Task, String> {
        let parent = self.task(caller).cloned().ok_or("Your thread is gone.")?;
        let project = self.project(parent.project_id).cloned().ok_or("Your project was removed.")?;
        let join = if isolate {
            let from_dir = parent.worktree.clone().unwrap_or_else(|| project.path.clone());
            let from = dove_core::git::head_sha(&from_dir).ok();
            let wt = dove_core::git::create_worktree_from(
                &project.path,
                &dove_core::git::slug(&title),
                "dove/",
                from.as_deref(),
            )
            .map_err(|e| format!("Couldn't create a worktree: {e:#}"))?;
            Joined { worktree: Some(wt.path), branch: Some(wt.branch), base: Some(wt.base) }
        } else {
            Joined::of(&parent)
        };
        let mode = run.mode;
        let new = NewTask { title, prompt, harness: pick.harness, run, join: Some(join), persona: teammate.clone() };
        let (task, live) = self.engine.start_task(&project, new).map_err(|e| format!("{e:#}"))?;
        // Its later turns keep the capped mode (a thread's composer reads it
        // back), not the user's default.
        let _ = self.engine.store.set_setting(&format!("mode.{}", task.id), crate::composer::mode_id(mode));
        // Its title comes from the task, not from automatic titles of its
        // first message (which opens with Dove's framing).
        let _ = self.engine.store.set_setting(&format!("renamed.{}", task.id), "1");
        if let Some(p) = &teammate {
            self.persona_of.insert(task.id, p.id.clone());
        }
        self.tasks.insert(0, task.clone());
        self.make_thread(task.clone(), project, Some(live), window, cx);
        self.team.seen.insert(task.id, TaskStatus::Running);
        cx.notify();
        Ok(task)
    }

    fn team_delegate(
        &mut self,
        caller: TaskId,
        a: &Args,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<TeamReply, String> {
        let task_text = a.need("task")?;
        // A retry with the same key finds the child it started.
        let key = a.str("request_key");
        if let Some(k) = &key
            && let Some(existing) = self.engine.store.link_by_request(caller, k)
        {
            return self.team_task_status_of(caller, existing).map(TeamReply::Now);
        }
        let parents = self.parent_map();
        let depth = orc::depth(caller, &parents);
        if depth >= orc::MAX_DEPTH {
            return Err(format!(
                "Delegation nests at most {} deep and this thread is already {depth} levels in. Do this part yourself.",
                orc::MAX_DEPTH
            ));
        }
        let active = self
            .engine
            .store
            .task_children(caller)
            .iter()
            .filter(|l| l.kind == LinkKind::Delegate && !l.status.terminal())
            .count();
        if active >= orc::MAX_ACTIVE_CHILDREN {
            return Err(format!(
                "You already have {active} delegated tasks working (the limit is {}). Wait for one to finish.",
                orc::MAX_ACTIVE_CHILDREN
            ));
        }
        let teammate = match a.str("teammate") {
            Some(name) => Some(
                orc::find_teammate(&self.personas, &name)
                    .cloned()
                    .ok_or_else(|| format!("No teammate called \"{name}\". team_capabilities lists them."))?,
            ),
            None => None,
        };
        let caller_task = self.task(caller).cloned().ok_or("Your thread is gone.")?;
        let pick = orc::pick_agent(
            a.str("agent").as_deref(),
            a.str("model").as_deref(),
            teammate.as_ref(),
            caller_task.harness,
            &self.installed_kinds(),
        )?;
        let parent_mode = self.mode_of(caller, cx);
        let mode = orc::cap_mode(parent_mode, a.str("permission").as_deref().and_then(orc::parse_mode));
        let reasoning = a.str("reasoning").as_deref().and_then(ReasoningLevel::from_id);
        let isolate = a.str("isolation").as_deref() == Some("worktree");
        let title = a.str("title").unwrap_or_else(|| crate::ui::title_from_prompt(&task_text));
        let from = self.sender(caller);
        let prompt = orc::delegation_prompt(&from, &task_text, isolate);
        let wait = a.str("mode").as_deref() == Some("wait");
        match self.orchestrate_of(caller) {
            Orchestrate::Off => return Err(ask::OFF.into()),
            Orchestrate::Ask => {
                if let Some(k) = &key
                    && let Some(id) = self.team.proposals.iter().find_map(|(id, (c, _))| {
                        (c.read(cx).proposal.request_key.as_deref() == Some(k.as_str())).then_some(*id)
                    })
                {
                    return Ok(TeamReply::Now(json_out(json!({"status": "awaiting_approval", "proposal_id": id}))));
                }
                let _ = prompt;
                return Ok(TeamReply::Now(self.propose(
                    caller,
                    LinkKind::Delegate,
                    task_text,
                    title,
                    teammate,
                    pick,
                    reasoning,
                    mode,
                    isolate,
                    key,
                    false,
                    window,
                    cx,
                )));
            }
            Orchestrate::Auto => {}
        }
        let run = RunSettings { mode, model: pick.model.clone(), reasoning, ..Default::default() };
        let child =
            self.start_child(caller, prompt, title, teammate.clone(), pick.clone(), run, isolate, window, cx)?;
        let store = &self.engine.store;
        store.link_task(child.id, caller, LinkKind::Delegate, !wait).map_err(|e| e.to_string())?;
        if let Some(k) = &key {
            let _ = store.set_link_request_key(child.id, k);
        }
        let base = json!({
            "task_id": child.id.0.to_string(),
            "thread_id": child.id.0.to_string(),
            "title": child.title,
            "agent": pick.harness.label(),
            "model": pick.model,
            "teammate": teammate.as_ref().map(|p| p.name.clone()),
            "permission": orc::mode_label(mode),
            "isolation": if isolate { "worktree" } else { "shared" },
            "worktree": child.worktree.as_ref().map(|p| p.display().to_string()),
            "branch": child.branch,
        });
        if wait {
            self.team.waiting.insert(child.id);
            return Ok(TeamReply::Wait(Waiter {
                thread: child.id,
                deadline: Instant::now() + a.timeout(),
                started: Instant::now(),
                seen_running: false,
                base,
                consumes: Some(child.id),
            }));
        }
        let mut out = base;
        out["status"] = json!("running");
        out["note"] = json!("Started. End your turn when you have nothing else to do: Dove wakes you with the result.");
        Ok(TeamReply::Now(json_out(out)))
    }

    /// Several delegations in one call: each task's own fields over the
    /// shared defaults, all async. One that can't start reports why; the
    /// others still start.
    fn team_delegate_many(
        &mut self,
        caller: TaskId,
        args: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Outcome, String> {
        let tasks = args.get("tasks").and_then(Value::as_array).cloned().unwrap_or_default();
        if tasks.is_empty() {
            return Err("Give at least one task in \"tasks\".".into());
        }
        if tasks.len() > orc::MAX_ACTIVE_CHILDREN {
            return Err(format!("At most {} tasks at once.", orc::MAX_ACTIVE_CHILDREN));
        }
        let mut results = Vec::new();
        for (i, t) in tasks.iter().enumerate() {
            let mut merged = serde_json::Map::new();
            for key in ["teammate", "agent", "model", "isolation", "permission", "reasoning"] {
                if let Some(v) = args.get(key) {
                    merged.insert(key.into(), v.clone());
                }
            }
            match t {
                Value::Object(own) => merged.extend(own.clone()),
                Value::String(s) => {
                    merged.insert("task".into(), json!(s));
                }
                _ => {}
            }
            merged.insert("mode".into(), json!("async"));
            let one = Value::Object(merged);
            let res = match self.team_delegate(caller, &Args(&one), window, cx) {
                Ok(TeamReply::Now(out)) if !out.is_error => {
                    serde_json::from_str::<Value>(&out.text).unwrap_or_else(|_| json!({"note": out.text}))
                }
                Ok(TeamReply::Now(out)) => json!({"error": out.text}),
                Ok(TeamReply::Wait(_)) => json!({"error": "unexpected wait"}),
                Err(e) => json!({"error": e}),
            };
            results.push(json!({"index": i, "result": res}));
        }
        let started = results.iter().filter(|r| r["result"].get("task_id").is_some()).count();
        let proposed = results.iter().filter(|r| r["result"].get("proposal_id").is_some()).count();
        Ok(json_out(json!({
            "started": started,
            "awaiting_approval": proposed,
            "tasks": results,
            "note": "Started tasks report back in team updates; end your turn when you have nothing else to do.",
        })))
    }

    fn team_task_status(&mut self, caller: TaskId, a: &Args) -> Result<Outcome, String> {
        let id = parse_thread(&a.need("task_id")?)?;
        self.team_task_status_of(caller, id)
    }

    fn team_task_status_of(&mut self, caller: TaskId, id: TaskId) -> Result<Outcome, String> {
        let link = self
            .engine
            .store
            .task_link(id)
            .filter(|l| l.parent == caller)
            .ok_or("That isn't a task this thread delegated (thread_read and thread_wait work on any thread).")?;
        let task = self.task(id).cloned();
        let status = task.as_ref().map(|t| orc::link_status(t.status)).unwrap_or(link.status);
        let summary = if status.terminal() || status == LinkStatus::NeedsInput {
            link.summary.clone().or_else(|| orc::final_answer(&self.engine.store.read_transcript(id)))
        } else {
            None
        };
        // Reading a finished result here takes it out of the next team update.
        if status.terminal() {
            let _ = self.engine.store.mark_link_delivered(id);
            for (_, reports) in self.team.pending.values_mut() {
                reports.retain(|r| r.thread != id);
            }
        }
        Ok(json_out(json!({
            "task_id": id.0.to_string(),
            "title": task.as_ref().map(|t| t.title.clone()),
            "agent": task.as_ref().map(|t| t.harness.label()),
            "teammate": self.teammate_of(id).map(|p| p.name.clone()),
            "status": status.id(),
            "result": summary,
            "needs_user": status == LinkStatus::NeedsInput,
        })))
    }

    fn team_task_cancel(&mut self, caller: TaskId, a: &Args, cx: &mut Context<Self>) -> Result<Outcome, String> {
        let id = parse_thread(&a.need("task_id")?)?;
        let link = self
            .engine
            .store
            .task_link(id)
            .filter(|l| l.parent == caller)
            .ok_or("That isn't a task this thread delegated (thread_interrupt stops any thread).")?;
        if link.status.terminal() {
            return Ok(json_out(
                json!({"task_id": id.0.to_string(), "status": link.status.id(), "note": "Already finished."}),
            ));
        }
        let mut stopped = Vec::new();
        self.cancel_tree(id, &mut stopped, cx);
        Ok(json_out(json!({
            "task_id": id.0.to_string(),
            "status": "cancelled",
            "stopped": stopped.iter().map(|t| t.0.to_string()).collect::<Vec<_>>(),
            "reason": a.str("reason"),
        })))
    }

    /// Stop `id` and everything it delegated, depth first.
    fn cancel_tree(&mut self, id: TaskId, stopped: &mut Vec<TaskId>, cx: &mut Context<Self>) {
        if stopped.contains(&id) {
            return;
        }
        stopped.push(id);
        for child in self.engine.store.task_children(id) {
            if child.kind == LinkKind::Delegate && !child.status.terminal() {
                self.cancel_tree(child.child, stopped, cx);
            }
        }
        if let Some(v) = self.threads.get(&id) {
            let chat = v.read(cx).chat.clone();
            chat.update(cx, |c, cx| c.interrupt(cx));
        }
        let store = &self.engine.store;
        let _ = store.mute_link(id);
        let _ = store.set_link_result(id, LinkStatus::Cancelled, Some("Cancelled by the thread that started it."));
        self.team.waiting.remove(&id);
    }

    fn team_message(
        &mut self,
        caller: TaskId,
        a: &Args,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<TeamReply, String> {
        let name = a.need("teammate")?;
        let message = a.need("message")?;
        let mate = orc::find_teammate(&self.personas, &name)
            .cloned()
            .ok_or_else(|| format!("No teammate called \"{name}\". team_capabilities lists them."))?;
        if self.teammate_of(caller).is_some_and(|p| p.id == mate.id) {
            return Err("That's you. Message another teammate, or delegate_task to start a parallel thread.".into());
        }
        let wants_reply = a.flag("wants_reply", true);
        let wait = a.str("mode").as_deref() == Some("wait");
        let caller_task = self.task(caller).cloned().ok_or("Your thread is gone.")?;
        let from = self.sender(caller);
        let text = orc::message_prompt(&from, &message, wants_reply || wait);
        // Their latest thread, unless it's busy on work another thread
        // handed them (its result belongs there).
        let store = self.engine.store.clone();
        let free =
            |t: &Task| store.task_link(t.id).is_none_or(|l| l.parent == caller || l.status.terminal() || !l.deliver);
        let existing = if a.flag("new_thread", false) {
            None
        } else {
            orc::reply_thread(
                self.tasks
                    .iter()
                    .filter(|t| free(t))
                    .filter_map(|t| self.persona_of.get(&t.id).map(|p| (t, p.as_str()))),
                &mate.id,
                caller_task.project_id,
                caller,
            )
        };
        let target = match existing {
            Some(id) => {
                self.deliver_text(id, &text, window, cx)?;
                id
            }
            None => {
                let pick = orc::pick_agent(
                    a.str("agent").as_deref(),
                    None,
                    Some(&mate),
                    caller_task.harness,
                    &self.installed_kinds(),
                )?;
                let mode = self.mode_of(caller, cx);
                match self.orchestrate_of(caller) {
                    Orchestrate::Off => return Err(ask::OFF.into()),
                    Orchestrate::Ask => {
                        let title = crate::ui::title_from_prompt(&message);
                        return Ok(TeamReply::Now(self.propose(
                            caller,
                            LinkKind::Message,
                            message,
                            title,
                            Some(mate),
                            pick,
                            None,
                            mode,
                            false,
                            None,
                            wants_reply || wait,
                            window,
                            cx,
                        )));
                    }
                    Orchestrate::Auto => {}
                }
                let run = RunSettings { mode, model: pick.model.clone(), ..Default::default() };
                let title = crate::ui::title_from_prompt(&message);
                self.start_child(caller, text, title, Some(mate.clone()), pick, run, false, window, cx)?.id
            }
        };
        let store = &self.engine.store;
        if wants_reply || wait {
            store.link_task(target, caller, LinkKind::Message, wants_reply && !wait).map_err(|e| e.to_string())?;
        }
        let base = json!({
            "teammate": mate.name,
            "thread_id": target.0.to_string(),
            "new_thread": existing.is_none(),
        });
        if wait {
            self.team.waiting.insert(target);
            return Ok(TeamReply::Wait(Waiter {
                thread: target,
                deadline: Instant::now() + a.timeout(),
                started: Instant::now(),
                seen_running: false,
                base,
                consumes: Some(target),
            }));
        }
        let mut out = base;
        out["status"] = json!("sent");
        if wants_reply {
            out["note"] = json!(
                "Sent. Their reply is delivered to you when they finish; end your turn if you have nothing else to do."
            );
        }
        Ok(TeamReply::Now(json_out(out)))
    }

    fn team_thread_list(&self, caller: TaskId, a: &Args) -> Outcome {
        let project = self.task(caller).map(|t| t.project_id);
        let all = a.flag("all_projects", false);
        let query = a.str("query").map(|q| q.to_lowercase());
        let status = a.str("status");
        let mate = a.str("teammate").and_then(|n| orc::find_teammate(&self.personas, &n).map(|p| p.id.clone()));
        let limit = a.int("limit").unwrap_or(30).clamp(1, 100) as usize;
        let mut rows: Vec<&Task> = self
            .tasks
            .iter()
            .filter(|t| t.status != TaskStatus::Draft)
            .filter(|t| all || Some(t.project_id) == project)
            .filter(|t| query.as_ref().is_none_or(|q| t.title.to_lowercase().contains(q)))
            .filter(|t| status.as_deref().is_none_or(|s| status_id(t.status) == s))
            .filter(|t| mate.as_ref().is_none_or(|m| self.persona_of.get(&t.id) == Some(m)))
            .collect();
        rows.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
        let list: Vec<Value> = rows.iter().take(limit).map(|t| self.thread_brief(t)).collect();
        json_out(json!({"threads": list, "total": rows.len()}))
    }

    fn team_thread_read(&self, a: &Args) -> Result<Outcome, String> {
        let id = parse_thread(&a.need("thread_id")?)?;
        let task = self.task(id).cloned().ok_or("No thread with that id.")?;
        let after = a.int("after").unwrap_or(0) as usize;
        let max = a.int("max_chars").unwrap_or(20_000).clamp(500, 60_000) as usize;
        let events = self.engine.store.read_transcript(id);
        let (text, turns) = orc::timeline(&events, after, max);
        let head = format!(
            "Thread \"{}\" ({}{}) · {} · {} turns\n\n",
            task.title,
            task.harness.label(),
            self.teammate_of(id).map(|p| format!(", as {}", p.name)).unwrap_or_default(),
            status_id(task.status),
            turns
        );
        Ok(Outcome::ok(format!("{head}{text}")))
    }

    fn team_thread_send(
        &mut self,
        caller: TaskId,
        a: &Args,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<TeamReply, String> {
        let id = parse_thread(&a.need("thread_id")?)?;
        if id == caller {
            return Err("That's your own thread.".into());
        }
        let message = a.need("message")?;
        // A message can't hand a thread more than the sender may do.
        let (mine, theirs) = (self.mode_of(caller, cx), self.mode_of(id, cx));
        if orc::cap_mode(mine, Some(theirs)) != theirs {
            return Err(format!(
                "That thread runs in {} mode, broader than yours ({}). Ask the user to message it.",
                orc::mode_label(theirs),
                orc::mode_label(mine)
            ));
        }
        let mut wants_reply = a.flag("wants_reply", false);
        let link = self.engine.store.task_link(id);
        // Work another thread handed over reports to that thread; a message
        // in between doesn't take its result away.
        let owed_elsewhere = link.as_ref().is_some_and(|l| l.parent != caller && !l.status.terminal() && l.deliver);
        let mut note = None;
        if wants_reply && owed_elsewhere {
            wants_reply = false;
            note = Some(
                "It is working on a task another thread handed it, so its result goes there. Use thread_wait and thread_read to follow it.",
            );
        }
        let from = self.sender(caller);
        let text = orc::message_prompt(&from, &message, wants_reply);
        self.deliver_text(id, &text, window, cx)?;
        if wants_reply {
            self.engine.store.link_task(id, caller, LinkKind::Message, true).map_err(|e| e.to_string())?;
        } else if let Some(l) = link.filter(|l| l.parent == caller) {
            // A follow-up to a delegated child: its next result comes back too.
            let _ = self.engine.store.reopen_link(id, l.deliver);
        }
        Ok(TeamReply::Now(json_out(json!({
            "thread_id": id.0.to_string(),
            "status": "sent",
            "wants_reply": wants_reply,
            "note": note,
        }))))
    }

    fn team_thread_wait(&mut self, a: &Args) -> Result<TeamReply, String> {
        let id = parse_thread(&a.need("thread_id")?)?;
        if self.task(id).is_none() {
            return Err("No thread with that id.".into());
        }
        Ok(TeamReply::Wait(Waiter {
            thread: id,
            deadline: Instant::now() + a.timeout(),
            started: Instant::now(),
            // Nothing was just sent: a thread that isn't working answers now.
            seen_running: true,
            base: json!({"thread_id": id.0.to_string()}),
            consumes: None,
        }))
    }

    fn team_thread_interrupt(&mut self, a: &Args, cx: &mut Context<Self>) -> Result<Outcome, String> {
        let id = parse_thread(&a.need("thread_id")?)?;
        let status = self.task(id).map(|t| t.status).ok_or("No thread with that id.")?;
        if status != TaskStatus::Running {
            return Ok(json_out(
                json!({"thread_id": id.0.to_string(), "status": status_id(status), "note": "It wasn't working."}),
            ));
        }
        let v = self.threads.get(&id).ok_or("That thread isn't open in Dove.")?;
        let chat = v.read(cx).chat.clone();
        chat.update(cx, |c, cx| c.interrupt(cx));
        Ok(json_out(json!({"thread_id": id.0.to_string(), "status": "interrupting"})))
    }

    // ------------------------------------------------------------ waiting

    /// One poll of a waiting call: `Some` answers it.
    fn team_poll(&mut self, w: &mut Waiter, cx: &mut Context<Self>) -> Option<Outcome> {
        let status = self.task(w.thread).map(|t| t.status);
        let Some(status) = status else {
            self.team.waiting.remove(&w.thread);
            return Some(Outcome::err("The thread was deleted while waiting."));
        };
        if status == TaskStatus::Running {
            w.seen_running = true;
        }
        // A message just sent may not have flipped the status yet (or waits
        // in the queue behind the current turn): until it starts, the
        // thread's finished status is the previous turn's.
        let pending = self.team.queued.contains(&w.thread)
            || self.team.arming.get(&w.thread).is_some_and(|at| at.elapsed() < ARMING);
        let settled = status != TaskStatus::Running && !pending && (w.seen_running || w.started.elapsed() > ARMING);
        let timed_out = Instant::now() >= w.deadline;
        if !settled && !timed_out {
            return None;
        }
        self.team.waiting.remove(&w.thread);
        let summary =
            (!timed_out || settled).then(|| orc::final_answer(&self.engine.store.read_transcript(w.thread))).flatten();
        if let Some(child) = w.consumes
            && settled
        {
            let ls = orc::link_status(status);
            let store = &self.engine.store;
            let _ = store.set_link_result(child, ls, summary.as_deref());
            let _ = store.mark_link_delivered(child);
            self.team.seen.insert(child, status);
        }
        let mut out = w.base.clone();
        out["status"] = json!(if settled { status_id(status) } else { "running" });
        out["timed_out"] = json!(timed_out && !settled);
        if settled {
            out["result"] = json!(summary);
        }
        if status == TaskStatus::NeedsInput {
            out["needs_user"] = json!(true);
            out["note"] = json!(
                "It is waiting for the user (an approval or a question). The user answers it in Dove; its result still comes back."
            );
        } else if timed_out && !settled {
            out["note"] =
                json!("Still working; waiting didn't stop it. Its result is delivered to you when it finishes.");
            // A timed-out delegate goes back to automatic delivery.
            if let Some(child) = w.consumes {
                let _ = self.engine.store.reopen_link(child, true);
            }
        }
        cx.notify();
        Some(json_out(out))
    }

    // ------------------------------------------------------------ delivery

    /// A thread's status changed (from its view): settle its link.
    pub(crate) fn team_on_status(&mut self, id: TaskId, s: TaskStatus) {
        self.team_settle(id, s);
    }

    fn team_settle(&mut self, id: TaskId, s: TaskStatus) {
        if matches!(s, TaskStatus::Done | TaskStatus::Failed) && self.team.queued.remove(&id) {
            // The turn before the queued message ended; that message starts now.
            self.team.arming.insert(id, Instant::now());
            return;
        }
        if let Some(at) = self.team.arming.get(&id).copied() {
            if s == TaskStatus::Running {
                self.team.arming.remove(&id);
            } else if at.elapsed() < ARMING {
                return;
            } else {
                // It never visibly started (a fast turn between sweeps):
                // take what it has now as the answer.
                self.team.arming.remove(&id);
                self.team.seen.insert(id, TaskStatus::Running);
            }
        }
        if self.team.seen.get(&id) == Some(&s) {
            return;
        }
        let prev = self.team.seen.insert(id, s);
        let store = self.engine.store.clone();
        let Some(link) = store.task_link(id) else { return };
        match s {
            TaskStatus::Running => {
                // Working again after needing the user: its final result is
                // still to come, so deliver again when it lands.
                if link.status == LinkStatus::NeedsInput {
                    let _ = store.reopen_link(id, link.deliver);
                }
            }
            TaskStatus::Done | TaskStatus::Failed | TaskStatus::NeedsInput | TaskStatus::Idle => {
                // Idle only counts after a turn actually ran.
                if s == TaskStatus::Idle && prev != Some(TaskStatus::Running) {
                    return;
                }
                if link.status == LinkStatus::Cancelled {
                    return;
                }
                let status = orc::link_status(s);
                let summary = if s == TaskStatus::NeedsInput {
                    Some("It needs the user before it can go on (an approval or a question in its thread).".to_string())
                } else {
                    orc::final_answer(&store.read_transcript(id))
                };
                let _ = store.set_link_result(id, status, summary.as_deref());
                if !link.deliver || self.team.waiting.contains(&id) {
                    return;
                }
                let report = orc::Report {
                    thread: id,
                    title: self.task(id).map(|t| t.title.clone()).unwrap_or_default(),
                    who: self.who(id),
                    status,
                    summary,
                    delegated: link.kind == LinkKind::Delegate,
                };
                let entry = self.team.pending.entry(link.parent).or_insert_with(|| (Instant::now(), Vec::new()));
                entry.1.retain(|r| r.thread != id);
                entry.1.push(report);
            }
            TaskStatus::Draft | TaskStatus::Archived => {}
        }
    }

    /// The sweep: settle linked threads whose views aren't open, and wake
    /// parents whose batch is due.
    fn team_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let links = self.engine.store.open_task_links();
        for l in &links {
            let status = match self.threads.get(&l.child) {
                Some(v) => Some(v.read(cx).chat.read(cx).status),
                None => self.engine.store.task(l.child).ok().flatten().map(|t| t.status),
            };
            if let Some(s) = status {
                self.team_settle(l.child, s);
            }
        }
        let due: Vec<TaskId> = self
            .team
            .pending
            .iter()
            .filter(|(_, (at, reports))| {
                !reports.is_empty() && at.elapsed() >= Duration::from_millis(orc::DELIVERY_BATCH_MS)
            })
            .map(|(p, _)| *p)
            .collect();
        for parent in due {
            let Some((_, reports)) = self.team.pending.remove(&parent) else { continue };
            // Results the parent already read with task_status are gone.
            let reports: Vec<orc::Report> = reports
                .into_iter()
                .filter(|r| self.engine.store.task_link(r.thread).is_some_and(|l| l.deliver && !l.delivered))
                .collect();
            if reports.is_empty() {
                continue;
            }
            let text = orc::team_update(&reports);
            match self.deliver_text(parent, &text, window, cx) {
                Ok(()) => {
                    for r in &reports {
                        // A child still needing the user reports again when it finishes.
                        if r.status.terminal() {
                            let _ = self.engine.store.mark_link_delivered(r.thread);
                        }
                    }
                }
                Err(e) => tracing::warn!("team update for {parent:?} not delivered: {e}"),
            }
        }
    }

    /// Send `text` to thread `id` as a message, opening its view if needed:
    /// it steers a working thread (or waits its turn) and starts an idle one.
    fn deliver_text(
        &mut self,
        id: TaskId,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let task = self.task(id).cloned().ok_or("No thread with that id.")?;
        if task.status == TaskStatus::Archived {
            return Err("That thread is archived.".into());
        }
        let harness = task.harness;
        if !self.threads.contains_key(&id) {
            let project = self.project(task.project_id).cloned().ok_or("Its project was removed.")?;
            self.make_thread(task, project, None, window, cx);
        }
        let chat = self.threads.get(&id).map(|v| v.read(cx).chat.clone()).ok_or("That thread couldn't be opened.")?;
        let text = text.to_string();
        let busy = chat.read(cx).status == TaskStatus::Running;
        if busy && !dove_core::steers(harness) {
            // Like typing mid-turn: it goes out when the turn ends.
            let composer = chat.read(cx).composer.clone();
            composer.update(cx, |c, cx| c.enqueue(text, cx));
            self.team.queued.insert(id);
        } else {
            chat.update(cx, |c, cx| c.submit(text, window, cx));
            self.team.arming.insert(id, Instant::now());
        }
        Ok(())
    }
}

/// `x.pipe(f)` = `f(x)`, for ending a builder chain in a function.
trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl<T> Pipe for T {}
