//! Race and compare, app side: the sheet that picks contenders, starting
//! them (each an ordinary thread in its own worktree, through
//! `Engine::start_task`), keeping each race's record current, and the
//! compare view's actions (check, pick winner, discard others).
//!
//! The rules and the record live in [`dove_core::race`]; the pages are in
//! `pages/race.rs`. Git and checks always run off the UI thread.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use dove_core::host::Host;
use dove_core::proto::{HarnessKind, TaskId, TaskStatus};
use dove_core::race::{self, Race};
use dove_core::store::Project;
use dove_core::{Joined, NewTask, RunSettings};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_base::input::{InputState, Textarea, TextareaState};

use crate::assets::{icon, icons};
use crate::design::{radius, space, ty};
use crate::kit::controls::{self as ctl, Tone};
use crate::kit::{self, Toasts, page, popover};
use crate::shell::{Route, Shell};
use crate::theme::Theme;
use crate::ui;

/// How long an armed two-click action waits for its second click.
pub const ARM_FOR: Duration = Duration::from_secs(8);

/// Everything the shell keeps for races.
#[derive(Default)]
pub struct RaceState {
    races: Vec<Race>,
    loaded: bool,
    /// The race the compare view shows; `None` shows the list.
    pub open: Option<String>,
    pub sheet: Option<Sheet>,
    /// A destructive action waiting for its confirming second click.
    pub arm: Option<(Arm, Instant)>,
    /// What each race is doing right now ("Merging…").
    pub busy: HashMap<String, &'static str>,
    /// Contenders whose check is running.
    pub checking: HashSet<TaskId>,
    /// The last action's result per race: (text, is an error).
    pub note: HashMap<String, (String, bool)>,
    /// Contenders whose check output is unfolded.
    pub expanded: HashSet<TaskId>,
    /// Machines found for the sheet; `None` until a search finishes.
    machines: Option<Vec<Host>>,
    looking: bool,
    refreshing: HashSet<String>,
    tick: Option<gpui::Task<()>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Arm {
    Pick(String, TaskId),
    Discard(String),
    Forget(String),
}

/// The "Race this prompt" sheet.
pub struct Sheet {
    pub prompt: Entity<TextareaState>,
    pub check: Entity<InputState>,
    pub project: Option<Project>,
    /// The composer's settings: mode for everyone, its model for its agent.
    pub run: RunSettings,
    pub harness: HarnessKind,
    pub picks: Vec<(HarnessKind, Host)>,
    pub error: Option<String>,
    pub starting: bool,
    focus: FocusHandle,
}

impl RaceState {
    pub fn races(&self) -> &[Race] {
        &self.races
    }

    pub fn get(&self, id: &str) -> Option<&Race> {
        self.races.iter().find(|r| r.id == id)
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut Race> {
        self.races.iter_mut().find(|r| r.id == id)
    }

    /// Whether `arm` is armed and still fresh.
    pub fn armed(&self, arm: &Arm) -> bool {
        self.arm.as_ref().is_some_and(|(a, at)| a == arm && at.elapsed() < ARM_FOR)
    }

    fn race_of(&self, task: TaskId) -> Option<String> {
        self.races.iter().find(|r| r.contender(task).is_some()).map(|r| r.id.clone())
    }
}

impl Shell {
    pub(crate) fn ensure_races(&mut self) {
        if !self.race.loaded {
            self.race.races = race::load_all(&self.engine.store);
            self.race.loaded = true;
        }
    }

    fn save_race(&self, id: &str) {
        if let Some(r) = self.race.get(id)
            && let Err(e) = race::save(&self.engine.store, r)
        {
            tracing::warn!("saving race {id}: {e:#}");
        }
    }

    /// A thread's live status, as the sidebar knows it.
    pub(crate) fn race_status(&self, task: TaskId) -> Option<TaskStatus> {
        self.task(task).map(|t| t.status)
    }

    /// The Races page came up: load records and freshen the open race.
    pub(crate) fn races_shown(&mut self, cx: &mut Context<Self>) {
        self.ensure_races();
        match self.race.open.clone() {
            Some(id) => self.refresh_race(&id, cx),
            None => {
                let running: Vec<String> = self
                    .race
                    .races
                    .iter()
                    .filter(|r| r.winner.is_none() && r.contenders.iter().any(|c| c.finished_ms.is_none()))
                    .map(|r| r.id.clone())
                    .collect();
                for id in running {
                    self.refresh_race(&id, cx);
                }
            }
        }
    }

    pub(crate) fn open_race(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.race.open = id;
        self.race.arm = None;
        if self.route == Route::Races {
            self.races_shown(cx);
            cx.notify();
        } else {
            self.navigate(Route::Races, window, cx);
        }
    }

    // ------------------------------------------------------------ sheet

    /// Open the sheet with `prompt` (the composer's draft, possibly empty).
    pub(crate) fn open_race_sheet(&mut self, prompt: String, window: &mut Window, cx: &mut Context<Self>) {
        let (project, run, harness) = {
            let c = self.composer.read(cx);
            (c.project.clone(), c.run_settings(cx), c.harness(cx))
        };
        let project = project.or_else(|| self.home_project.and_then(|id| self.project(id)).cloned());
        let prompt_input = crate::pages::textarea("What should every contender do?", (3, 8), window, cx);
        prompt_input.update(cx, |s, cx| s.set_value(prompt, window, cx));
        let check = crate::pages::input("cargo test, npm test, … (optional)", window, cx);
        let picks = vec![(harness, run.host.clone())];
        let focus = cx.focus_handle();
        prompt_input.update(cx, |s, cx| s.focus(window, cx));
        self.race.sheet = Some(Sheet {
            prompt: prompt_input,
            check,
            project,
            run,
            harness,
            picks,
            error: None,
            starting: false,
            focus,
        });
        self.find_race_machines(cx);
        cx.notify();
    }

    pub(crate) fn close_race_sheet(&mut self, cx: &mut Context<Self>) {
        if self.race.sheet.as_ref().is_some_and(|s| s.starting) {
            return;
        }
        self.race.sheet = None;
        cx.notify();
    }

    /// Look for machines off the UI thread (WSL, SSH config, Spaces,
    /// containers, cloud boxes, paired devices all probe external tools).
    fn find_race_machines(&mut self, cx: &mut Context<Self>) {
        if self.race.looking {
            return;
        }
        self.race.looking = true;
        let find = cx.background_spawn(async move { dove_core::host::discover() });
        cx.spawn(async move |this, cx| {
            let found = find.await;
            let _ = this.update(cx, |s, cx| {
                s.race.looking = false;
                s.race.machines = Some(found);
                cx.notify();
            });
        })
        .detach();
    }

    /// Machines the sheet offers: what was found, plus any already picked.
    pub(crate) fn race_machines(&self) -> (Vec<Host>, bool) {
        let mut hosts = self.race.machines.clone().unwrap_or_else(|| vec![Host::Local]);
        if let Some(sheet) = &self.race.sheet {
            for (_, h) in &sheet.picks {
                if !hosts.contains(h) {
                    hosts.push(h.clone());
                }
            }
        }
        (hosts, self.race.looking)
    }

    /// Agents the sheet offers: installed, supported and switched on.
    pub(crate) fn race_agents(&self) -> Vec<HarnessKind> {
        let mut out: Vec<HarnessKind> = self
            .installed
            .iter()
            .map(|i| i.kind)
            .filter(|k| dove_core::harness_supported(*k) && crate::pages::agent_enabled(&self.engine.store, *k))
            .collect();
        out.dedup();
        out
    }

    pub(crate) fn toggle_race_pick(&mut self, kind: HarnessKind, host: Host, cx: &mut Context<Self>) {
        let Some(sheet) = self.race.sheet.as_mut() else { return };
        if let Some(i) = sheet.picks.iter().position(|(k, h)| *k == kind && *h == host) {
            sheet.picks.remove(i);
        } else if sheet.picks.len() < race::MAX_CONTENDERS {
            sheet.picks.push((kind, host));
        }
        sheet.error = None;
        cx.notify();
    }

    /// Start every picked contender: worktrees first (off the UI thread),
    /// then each thread the way the composer starts one.
    pub(crate) fn start_race(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.race.sheet.as_mut() else { return };
        if sheet.starting {
            return;
        }
        let prompt = sheet.prompt.read(cx).value().trim().to_string();
        let fail = |sheet: &mut Sheet, why: String, cx: &mut Context<Self>| {
            sheet.error = Some(why);
            cx.notify();
        };
        if prompt.is_empty() {
            return fail(sheet, "Write the prompt every contender will work on.".into(), cx);
        }
        if let Err(why) = race::validate(sheet.picks.len()) {
            return fail(sheet, why, cx);
        }
        let Some(project) = sheet.project.clone() else {
            return fail(sheet, "Add a project first: a race runs in a git repository.".into(), cx);
        };
        let check = sheet.check.read(cx).value().trim().to_string();
        let check = (!check.is_empty()).then_some(check);
        let picks = sheet.picks.clone();
        let title = ui::title_from_prompt(&prompt);
        sheet.starting = true;
        sheet.error = None;
        let repo = project.path.clone();
        let names: Vec<String> = picks.iter().map(|(k, h)| race::worktree_name(&title, *k, h)).collect();
        let prep = cx.background_spawn(async move {
            if dove_core::git::repo_root(&repo).is_none() {
                return Err(
                    "This project isn't a git repository. A race gives each contender its own branch, so it needs one."
                        .to_string(),
                );
            }
            let base = dove_core::git::head_sha(&repo)
                .map_err(|_| "This repository has no commits yet. Make a first commit, then race.".to_string())?;
            let branch = dove_core::git::current_branch(&repo).ok().filter(|b| b != "HEAD");
            let mut made = Vec::new();
            for name in &names {
                match dove_core::git::create_worktree_from(&repo, name, race::BRANCH_PREFIX, Some(&base)) {
                    Ok(w) => made.push(w),
                    Err(e) => {
                        for w in &made {
                            let _ = dove_core::git::remove_worktree(&repo, &w.path, Some(&w.branch));
                        }
                        return Err(format!("Couldn't create a worktree: {e:#}"));
                    }
                }
            }
            Ok((base, branch, made))
        });
        cx.spawn_in(window, async move |this, cx| {
            let ready = prep.await;
            let _ = this.update_in(cx, |s, window, cx| match ready {
                Ok((base, branch, worktrees)) => {
                    s.launch_race(project, prompt, title, check, picks, base, branch, worktrees, window, cx)
                }
                Err(why) => {
                    if let Some(sheet) = s.race.sheet.as_mut() {
                        sheet.starting = false;
                        sheet.error = Some(why);
                    }
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn launch_race(
        &mut self,
        project: Project,
        prompt: String,
        title: String,
        check: Option<String>,
        picks: Vec<(HarnessKind, Host)>,
        base: String,
        base_branch: Option<String>,
        worktrees: Vec<dove_core::git::Worktree>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_races();
        let Some(sheet) = self.race.sheet.as_ref() else { return };
        let (composer_run, composer_harness) = (sheet.run.clone(), sheet.harness);
        let mut contenders = Vec::new();
        let mut failures = Vec::new();
        for ((kind, host), wt) in picks.into_iter().zip(worktrees) {
            // The composer's model and effort only mean something to its own
            // agent; the others run their default model.
            let same = kind == composer_harness;
            let run = RunSettings {
                mode: composer_run.mode,
                model: if same { composer_run.model.clone() } else { None },
                reasoning: if same { composer_run.reasoning } else { None },
                model_options: if same { composer_run.model_options.clone() } else { Vec::new() },
                add_dirs: if dove_core::supports_add_dirs(kind) { composer_run.add_dirs.clone() } else { Vec::new() },
                host: host.clone(),
            };
            let model = run.model.clone();
            let mut name = format!("{title} · {}", kind.label());
            if !host.is_local() {
                name.push_str(&format!(" on {}", host.label()));
            }
            let new = NewTask {
                title: name,
                prompt: prompt.clone(),
                harness: kind,
                run,
                join: Some(Joined {
                    worktree: Some(wt.path.clone()),
                    branch: Some(wt.branch.clone()),
                    base: Some(base.clone()),
                }),
                persona: None,
            };
            match self.engine.start_task(&project, new) {
                Ok((task, live)) => {
                    // Keep the contender's name: an automatic title would make
                    // every contender read the same in the sidebar.
                    let _ = self.engine.store.set_setting(&format!("renamed.{}", task.id), "1");
                    contenders.push(race::Contender {
                        task: task.id,
                        harness: kind,
                        model,
                        host,
                        branch: wt.branch.clone(),
                        worktree: wt.path.clone(),
                        started_ms: dove_core::store::now_ms(),
                        finished_ms: None,
                        outcome: None,
                        diff: None,
                        usage: None,
                        check: None,
                        discarded: false,
                    });
                    self.tasks.insert(0, task.clone());
                    self.make_thread(task, project.clone(), Some(live), window, cx);
                }
                Err(e) => {
                    failures.push(format!("{} on {}: {e:#}", kind.label(), host.label()));
                    let repo = project.path.clone();
                    cx.background_spawn(async move {
                        let _ = dove_core::git::remove_worktree(&repo, &wt.path, Some(&wt.branch));
                    })
                    .detach();
                }
            }
        }
        if contenders.is_empty() {
            if let Some(sheet) = self.race.sheet.as_mut() {
                sheet.starting = false;
                sheet.error = Some(format!("No contender could start. {}", failures.join(" · ")));
            }
            cx.notify();
            return;
        }
        let record = Race {
            id: race::new_id(),
            project: project.id,
            title,
            prompt,
            created_ms: dove_core::store::now_ms(),
            base_branch,
            base,
            check,
            contenders,
            winner: None,
            merged_ms: None,
        };
        let id = record.id.clone();
        self.race.races.insert(0, record);
        self.save_race(&id);
        if !failures.is_empty() {
            self.race.note.insert(id.clone(), (format!("Didn't start: {}", failures.join(" · ")), true));
        }
        self.race.sheet = None;
        self.composer.update(cx, |c, cx| c.clear_draft(window, cx));
        self.open_race(Some(id), window, cx);
    }

    // ------------------------------------------------------------ progress

    /// A thread's status moved (from `Shell::on_status`).
    pub(crate) fn race_on_status(&mut self, task: TaskId, status: TaskStatus, cx: &mut Context<Self>) {
        self.ensure_races();
        let Some(id) = self.race.race_of(task) else { return };
        let now = dove_core::store::now_ms();
        let settled = self.race.get_mut(&id).is_some_and(|r| r.settle(task, status, now));
        if settled {
            self.save_race(&id);
            self.after_settle(&id, task, cx);
        }
        if matches!(status, TaskStatus::Done | TaskStatus::Failed) {
            self.refresh_race(&id, cx);
        }
        if self.route == Route::Races {
            cx.notify();
        }
    }

    /// A contender just finished: run the race's check on it.
    fn after_settle(&mut self, id: &str, task: TaskId, cx: &mut Context<Self>) {
        let Some(r) = self.race.get(id) else { return };
        let Some(c) = r.contender(task) else { return };
        if r.check.is_some() && c.check.is_none() && c.outcome == Some(race::Outcome::Done) {
            self.run_race_check(id.to_string(), task, cx);
        }
    }

    /// Re-read usage, diff and (for threads without a live view) status of
    /// every contender, off the UI thread.
    pub(crate) fn refresh_race(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(r) = self.race.get(id).cloned() else { return };
        if !self.race.refreshing.insert(id.to_string()) {
            return;
        }
        let engine = self.engine.clone();
        let read = cx.background_spawn(async move {
            r.contenders
                .iter()
                .filter(|c| !c.discarded)
                .map(|c| {
                    let usage = engine.store.task_usage(c.task).ok();
                    let diff = c.worktree.is_dir().then(|| race::measure(&c.worktree, &r.base).ok()).flatten();
                    let task = engine.store.task(c.task).ok().flatten().map(|t| (t.status, t.updated_at));
                    (c.task, usage, diff, task)
                })
                .collect::<Vec<_>>()
        });
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            let rows = read.await;
            let _ = this.update(cx, |s, cx| {
                s.race.refreshing.remove(&id);
                let mut settled = Vec::new();
                let Some(r) = s.race.get_mut(&id) else { return };
                let before = r.clone();
                for (task, usage, diff, stored) in rows {
                    if let Some((status, at)) = stored
                        && r.settle(task, status, at)
                    {
                        settled.push(task);
                    }
                    if let Some(c) = r.contender_mut(task) {
                        if let Some(u) = usage.filter(|u| u.total_tokens() > 0 || u.cost_usd.is_some()) {
                            c.usage = Some(u);
                        }
                        if diff.is_some() {
                            c.diff = diff;
                        }
                    }
                }
                let changed = *r != before;
                if changed {
                    s.save_race(&id);
                }
                for task in settled {
                    s.after_settle(&id, task, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Keep the compare view's clocks moving while anything runs: a repaint
    /// a second, a data refresh every five. Stops when the page is left or
    /// the race settles, so an idle window stays idle.
    pub(crate) fn ensure_race_tick(&mut self, cx: &mut Context<Self>) {
        if self.race.tick.is_some() {
            return;
        }
        self.race.tick = Some(cx.spawn(async move |this, cx| {
            let mut n = 0u32;
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                n += 1;
                let keep = this
                    .update(cx, |s, cx| {
                        let Some(id) = s.race.open.clone().filter(|_| s.route == Route::Races) else { return false };
                        let live = s.race_live(&id);
                        if n % 5 == 0 {
                            s.refresh_race(&id, cx);
                        }
                        cx.notify();
                        live
                    })
                    .unwrap_or(false);
                if !keep {
                    let _ = this.update(cx, |s, _| s.race.tick = None);
                    break;
                }
            }
        }));
    }

    /// Whether any contender is working, waiting on the user, or checking.
    pub(crate) fn race_live(&self, id: &str) -> bool {
        let Some(r) = self.race.get(id) else { return false };
        r.contenders.iter().any(|c| {
            self.race.checking.contains(&c.task)
                || matches!(c.standing(self.race_status(c.task)), race::Standing::Running | race::Standing::NeedsInput)
        })
    }

    // ------------------------------------------------------------ actions

    pub(crate) fn run_race_check(&mut self, id: String, task: TaskId, cx: &mut Context<Self>) {
        let Some(r) = self.race.get(&id) else { return };
        let (Some(cmd), Some(c)) = (r.check.clone(), r.contender(task)) else { return };
        if c.discarded {
            return;
        }
        let dir = c.worktree.clone();
        // A remote contender's edits come home just after its turn ends.
        let wait = if c.host.is_local() { Duration::ZERO } else { Duration::from_secs(4) };
        if !self.race.checking.insert(task) {
            return;
        }
        cx.spawn(async move |this, cx| {
            if !wait.is_zero() {
                cx.background_executor().timer(wait).await;
            }
            let result = cx
                .background_spawn(async move {
                    race::run_check(&dir, &cmd, race::CHECK_TIMEOUT, dove_core::store::now_ms())
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.race.checking.remove(&task);
                if !result.passed {
                    s.race.expanded.insert(task);
                }
                if let Some(c) = s.race.get_mut(&id).and_then(|r| r.contender_mut(task)) {
                    c.check = Some(result);
                }
                s.save_race(&id);
                s.refresh_race(&id, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// First click arms `arm`; a second click within [`ARM_FOR`] returns
    /// true (do it). The arm lapses by itself.
    pub(crate) fn race_confirm(&mut self, arm: Arm, cx: &mut Context<Self>) -> bool {
        if self.race.armed(&arm) {
            self.race.arm = None;
            cx.notify();
            return true;
        }
        self.race.arm = Some((arm, Instant::now()));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ARM_FOR).await;
            let _ = this.update(cx, |s, cx| {
                if s.race.arm.as_ref().is_some_and(|(_, at)| at.elapsed() >= ARM_FOR) {
                    s.race.arm = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
        false
    }

    /// Merge contender `task` into the race's base branch.
    pub(crate) fn pick_race_winner(&mut self, id: String, task: TaskId, cx: &mut Context<Self>) {
        let Some(r) = self.race.get(&id).cloned() else { return };
        let Some(project) = self.project(r.project).cloned() else {
            self.race.note.insert(id, ("Its project was removed from Keplorayx.".into(), true));
            cx.notify();
            return;
        };
        if self.race.busy.contains_key(&id) {
            return;
        }
        self.race.busy.insert(id.clone(), "Merging…");
        let merge = cx.background_spawn(async move { race::merge_winner(&project.path, &r, task) });
        cx.spawn(async move |this, cx| {
            let result = merge.await;
            let _ = this.update(cx, |s, cx| {
                s.race.busy.remove(&id);
                match result {
                    Ok(line) => {
                        if let Some(r) = s.race.get_mut(&id) {
                            r.winner = Some(task);
                            r.merged_ms = Some(dove_core::store::now_ms());
                        }
                        s.save_race(&id);
                        // The winner banner says it on the page; a toast says
                        // it anywhere else.
                        Toasts::push(cx, line, None, kit::Tone::Success, None);
                        s.race.note.remove(&id);
                    }
                    Err(e) => {
                        s.race.note.insert(id.clone(), (format!("Nothing was merged. {e:#}"), true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Remove every contender but the winner: thread, worktree and branch.
    /// Their figures stay on the race.
    pub(crate) fn discard_race_losers(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.race.get(&id).cloned() else { return };
        let Some(project) = self.project(r.project).cloned() else { return };
        let losers: Vec<race::Contender> = r.losers().cloned().collect();
        let mut leftovers = Vec::new();
        for c in &losers {
            if self.tasks.iter().chain(self.archived.iter()).any(|t| t.id == c.task) {
                // Stops the agent and removes its worktree and branch too.
                self.delete_task(c.task, window, cx);
            } else {
                leftovers.push(c.clone());
            }
        }
        if !leftovers.is_empty() {
            let repo = project.path.clone();
            cx.background_spawn(async move {
                for c in leftovers {
                    if let Err(e) = race::discard(&repo, &c) {
                        tracing::warn!("discarding {}: {e:#}", c.branch);
                    }
                }
            })
            .detach();
        }
        if let Some(r) = self.race.get_mut(&id) {
            for c in r.contenders.iter_mut().filter(|c| losers.iter().any(|l| l.task == c.task)) {
                c.discarded = true;
            }
        }
        self.save_race(&id);
        let n = losers.len();
        self.race.note.insert(
            id,
            (
                format!("Removed {n} contender{}: threads, worktrees and branches.", if n == 1 { "" } else { "s" }),
                false,
            ),
        );
        cx.notify();
    }

    /// Drop a race from the list. Its threads and branches stay.
    pub(crate) fn forget_race(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let _ = race::forget(&self.engine.store, &id);
        self.race.races.retain(|r| r.id != id);
        self.race.note.remove(&id);
        self.open_race(None, window, cx);
    }

    /// Open a contender's thread with its changes in the side panel.
    pub(crate) fn race_view_diff(&mut self, task: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Thread(task), window, cx);
        self.panel_open = true;
        let _ = self.engine.store.set_setting("panel_open", "true");
        self.panel.update(cx, |p, cx| p.show(crate::side_panel::Tab::Changes, window, cx));
        cx.notify();
    }

    // ------------------------------------------------------------ sheet view

    pub(crate) fn render_race_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sheet = self.race.sheet.as_ref()?;
        let t = Theme::of(cx).clone();
        let agents = self.race_agents();
        let (machines, looking) = self.race_machines();
        let picks = sheet.picks.clone();
        let count = picks.len();
        let full = count >= race::MAX_CONTENDERS;
        let starting = sheet.starting;
        let invalid = race::validate(count).err();
        let branch_note = sheet
            .project
            .as_ref()
            .map(|p| format!("Each contender works in its own worktree and branch of {}.", p.name))
            .unwrap_or_else(|| "Pick a project in the composer first.".into());

        let field = |child: AnyElement| {
            div()
                .w_full()
                .min_h(px(32.0))
                .px(px(10.0))
                .py(px(6.0))
                .flex()
                .items_center()
                .rounded(px(radius::SELECT))
                .bg(t.wash(0.06))
                .text_size(ty::BODY)
                .child(div().flex_1().min_w_0().child(child))
        };
        let label = |text: &'static str| {
            div().text_size(ty::META).line_height(ty::lh::META).text_color(t.text_muted).child(text)
        };

        let agent_rows = agents.iter().enumerate().map(|(ai, kind)| {
            let kind = *kind;
            let model = (kind == sheet.harness).then(|| sheet.run.model.clone()).flatten();
            let detail = model.map(|m| format!("uses {m}")).unwrap_or_else(|| "default model".into());
            div()
                .py(px(6.0))
                .flex()
                .items_start()
                .gap(px(space::MD))
                .when(ai > 0, |d| d.border_t_1().border_color(page::row_divider(&t)))
                .child(
                    div()
                        .w(px(150.0))
                        .flex_none()
                        .pt(px(4.0))
                        .flex()
                        .items_center()
                        .gap(px(space::SM))
                        .child(icon(ui::harness_icon(kind)).size(px(15.0)).text_color(ui::harness_tint(
                            kind,
                            &t,
                            t.text_muted,
                        )))
                        .child(
                            div()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(div().text_size(ty::BODY).text_color(t.text).truncate().child(kind.label()))
                                .child(div().text_size(ty::CAPTION).text_color(t.text_faint).truncate().child(detail)),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.0))
                        .children(machines.iter().enumerate().map(|(hi, host)| {
                            let on = picks.iter().any(|(k, h)| *k == kind && h == host);
                            let host = host.clone();
                            let chip = ui::chip(SharedString::from(format!("race-pick-{ai}-{hi}")), &t)
                                .h(px(26.0))
                                .aria_label(SharedString::from(format!("{} on {}", kind.label(), host.label())))
                                .when(on, |c| c.border_color(t.accent).bg(t.accent_wash).text_color(t.text))
                                .when(on, |c| c.child(icon(icons::CHECK).size(px(12.0)).text_color(t.accent)))
                                .child(host.label());
                            if !on && full {
                                ctl::disabled(chip, format!("A race takes at most {} contenders", race::MAX_CONTENDERS))
                                    .into_any_element()
                            } else {
                                chip.on_click(
                                    cx.listener(move |this, _, _, cx| this.toggle_race_pick(kind, host.clone(), cx)),
                                )
                                .into_any_element()
                            }
                        }))
                        .when(looking, |d| {
                            d.child(
                                div()
                                    .h(px(26.0))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .text_size(ty::SMALL)
                                    .text_color(t.text_faint)
                                    .child(kit::spinner("race-looking", 11.0, t.text_faint))
                                    .child("Looking for machines…"),
                            )
                        }),
                )
        });

        let start = ctl::button(&t, "race-start", if starting { "Starting…" } else { "Start race" }, Tone::Solid);
        let start = match (&invalid, starting) {
            (_, true) => ctl::disabled(start, "Creating worktrees and starting agents"),
            (Some(why), _) => ctl::disabled(start, why.clone()),
            (None, false) => start.on_click(cx.listener(|this, _, window, cx| this.start_race(window, cx))),
        };

        let body = div()
            .id("race-sheet-body")
            .max_h(px(520.0))
            .overflow_y_scroll()
            .px(px(20.0))
            .flex()
            .flex_col()
            .gap(px(space::MD))
            .child(div().flex().flex_col().gap(px(6.0)).child(label("Prompt")).child(field(
                Textarea::new(&sheet.prompt).into_any_element(),
            )))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .child(div().flex_1().child(label("Contenders: an agent on a machine")))
                            .child(
                                div()
                                    .text_size(ty::META)
                                    .text_color(if invalid.is_some() { t.text_faint } else { t.text_muted })
                                    .child(format!("{count} of {}", race::MAX_CONTENDERS)),
                            ),
                    )
                    .child(if agents.is_empty() {
                        page::info_strip(&t, "No agents are installed or switched on. Settings › Agents lists them.")
                            .into_any_element()
                    } else {
                        page::card(&t).px(px(space::MD)).py(px(2.0)).children(agent_rows).into_any_element()
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(label("Check (optional)"))
                    .child(field(gpui_base::input::Input::new(&sheet.check).into_any_element()))
                    .child(
                        div().text_size(ty::CAPTION).line_height(ty::lh::META).text_color(t.text_faint).child(
                            "Runs on this computer in each contender's worktree when it finishes. Pass or fail and the last lines show in the comparison.",
                        ),
                    ),
            )
            .child(page::info_strip(
                &t,
                "Contenders use your current permission mode; any that ask for approval wait for you. Agents on other machines work in a synced copy, and their edits come back after each turn.",
            ))
            .when_some(sheet.error.clone(), |d, e| d.child(page::error_strip(&t, e)));

        let card = popover::dialog_card(&t, 640.0)
            .id("race-sheet")
            .role(gpui::Role::Dialog)
            .aria_label("Race this prompt")
            .track_focus(&sheet.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    this.close_race_sheet(cx);
                    cx.stop_propagation();
                }
            }))
            .child(popover::dialog_body(&t, "Race this prompt", branch_note))
            .child(body)
            .child(
                popover::dialog_actions()
                    .child(
                        ctl::button(&t, "race-cancel", "Cancel", Tone::Outlined)
                            .when(starting, |b| b.opacity(0.45))
                            .on_click(cx.listener(|this, _, _, cx| this.close_race_sheet(cx))),
                    )
                    .child(start),
            );
        let weak = cx.weak_entity();
        Some(popover::modal(
            "race-sheet-layer",
            window.viewport_size(),
            t.scrim,
            None,
            card.into_any_element(),
            Some(Box::new(move |_, cx| {
                let _ = weak.update(cx, |this, cx| this.close_race_sheet(cx));
            })),
        ))
    }
}
