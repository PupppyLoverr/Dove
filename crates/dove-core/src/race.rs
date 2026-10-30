//! Race and compare: one prompt, run by several contenders at once, each an
//! (agent, model, machine) pair in its own worktree and branch, then compared
//! side by side and the winner merged.
//!
//! This module holds no UI. It owns the race record (persisted as JSON in the
//! store's settings under `race.<id>`, so no schema migration is needed), the
//! rules for when a contender has finished, the figures shown in the compare
//! view, the optional check command run in each worktree, and the two git
//! actions: merging the winner into the base branch and discarding the rest.
//!
//! Contenders start through [`crate::Engine::start_task`] like any thread, so
//! transcripts, usage rows, approvals and remote hosts behave the same. A
//! contender on a remote machine runs in a synced copy of its worktree; the
//! engine brings its edits back after each turn, which is what the local diff
//! and the check see.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use dove_harness::host::Host;
use dove_proto::{HarnessKind, ProjectId, TaskId, TaskStatus, Usage};
use dove_store::Store;
use serde::{Deserialize, Serialize};

/// Fewest contenders a race makes sense with.
pub const MIN_CONTENDERS: usize = 2;
/// Most contenders one race starts (the same ceiling as an agent's fan-out).
pub const MAX_CONTENDERS: usize = 6;
/// Branches are `race/<name>`.
pub const BRANCH_PREFIX: &str = "race/";
/// How long a check command may run before it is stopped.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Lines of a check's output kept for the compare view.
pub const CHECK_TAIL: usize = 14;

const KEY_PREFIX: &str = "race.";

/// One race: the prompt, where it forked from, and its contenders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Race {
    pub id: String,
    pub project: ProjectId,
    pub title: String,
    pub prompt: String,
    pub created_ms: i64,
    /// The branch checked out when the race started: the winner merges here.
    /// `None` when the checkout was a detached HEAD.
    #[serde(default)]
    pub base_branch: Option<String>,
    /// The commit every contender's branch starts from.
    pub base: String,
    /// Run in each worktree after its contender finishes (`cargo test`, ...).
    #[serde(default)]
    pub check: Option<String>,
    pub contenders: Vec<Contender>,
    /// The contender merged into the base branch.
    #[serde(default)]
    pub winner: Option<TaskId>,
    #[serde(default)]
    pub merged_ms: Option<i64>,
}

/// One (agent, model, machine) entry in a race, and what it produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contender {
    /// Its thread.
    pub task: TaskId,
    pub harness: HarnessKind,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub host: Host,
    pub branch: String,
    pub worktree: PathBuf,
    pub started_ms: i64,
    /// When its first turn ended (later follow-ups don't move it).
    #[serde(default)]
    pub finished_ms: Option<i64>,
    #[serde(default)]
    pub outcome: Option<Outcome>,
    /// Last measured diff against the race's base.
    #[serde(default)]
    pub diff: Option<DiffStat>,
    /// Last read usage (tokens and reported cost) of its thread.
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub check: Option<CheckResult>,
    /// Its worktree, branch and thread were removed ("Discard others"); the
    /// figures above are kept so the race still reads.
    #[serde(default)]
    pub discarded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Done,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DiffStat {
    pub files: u32,
    pub adds: u32,
    pub dels: u32,
}

/// What a check command did in one worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub passed: bool,
    /// Exit code (`None` when it was killed or could not start).
    #[serde(default)]
    pub code: Option<i32>,
    #[serde(default)]
    pub timed_out: bool,
    /// The last lines of its output (stdout and stderr together).
    #[serde(default)]
    pub tail: Vec<String>,
    pub duration_ms: u64,
    pub ran_ms: i64,
}

/// Where a contender stands, from its record and its thread's status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    Running,
    NeedsInput,
    Done,
    Failed,
    Discarded,
}

/// A badge the compare view gives the best contender on one measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Fastest,
    Cheapest,
    Winner,
}

impl Mark {
    pub fn label(self) -> &'static str {
        match self {
            Mark::Fastest => "Fastest",
            Mark::Cheapest => "Cheapest",
            Mark::Winner => "Winner",
        }
    }
}

/// A race at a glance, for the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Some contenders still working (or waiting on the user).
    Running { left: usize },
    /// Every contender finished; nothing merged yet.
    Compare,
    /// A winner was merged.
    Decided,
}

/// Why a set of picks can't start, if it can't.
pub fn validate(count: usize) -> Result<(), String> {
    if count < MIN_CONTENDERS {
        return Err(format!("Pick at least {MIN_CONTENDERS} contenders."));
    }
    if count > MAX_CONTENDERS {
        return Err(format!("A race takes at most {MAX_CONTENDERS} contenders."));
    }
    Ok(())
}

/// A short, ref-safe worktree name for one contender: a few words of the
/// title, the agent and (off this machine) the host.
pub fn worktree_name(title: &str, kind: HarnessKind, host: &Host) -> String {
    let mut head = dove_git::slug(title);
    head.truncate(18);
    let head = head.trim_end_matches('-');
    let mut tail = kind.id().to_string();
    if !host.is_local() {
        tail.push(' ');
        tail.push_str(&host.label());
    }
    let tail = dove_git::slug(&tail);
    if head.is_empty() || head == "task" { tail } else { format!("{head}-{tail}") }
}

/// The last `n` non-blank lines of `text`, each cut to a readable width.
pub fn tail(text: &str, n: usize) -> Vec<String> {
    let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| clip(l, 240)).collect()
}

fn clip(line: &str, max: usize) -> String {
    if line.chars().count() <= max {
        return line.to_string();
    }
    let mut s: String = line.chars().take(max).collect();
    s.push('…');
    s
}

/// "45s", "4m 12s", "1h 03m".
pub fn duration_label(ms: i64) -> String {
    let s = (ms.max(0) / 1000) as u64;
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m", s / 3600, (s % 3600) / 60),
    }
}

impl Contender {
    /// Time from start to its first finished turn, or to `now` while it runs.
    pub fn elapsed_ms(&self, now: i64) -> i64 {
        self.finished_ms.unwrap_or(now) - self.started_ms
    }

    /// Where it stands, given its thread's live status (`None` when the
    /// thread is gone).
    pub fn standing(&self, status: Option<TaskStatus>) -> Standing {
        if self.discarded {
            return Standing::Discarded;
        }
        match status {
            Some(TaskStatus::NeedsInput) => Standing::NeedsInput,
            Some(TaskStatus::Running) => Standing::Running,
            _ => match self.outcome {
                Some(Outcome::Done) => Standing::Done,
                Some(Outcome::Failed) => Standing::Failed,
                // Not settled yet: an idle thread that never reported is
                // treated as still starting.
                None => match status {
                    Some(TaskStatus::Done) => Standing::Done,
                    Some(TaskStatus::Failed) | None => Standing::Failed,
                    _ => Standing::Running,
                },
            },
        }
    }

    /// "Claude Code · opus · ssh: vps"
    pub fn label(&self) -> String {
        let mut s = self.harness.label().to_string();
        if let Some(m) = self.model.as_deref().filter(|m| !m.is_empty()) {
            s.push_str(" · ");
            s.push_str(m);
        }
        if !self.host.is_local() {
            s.push_str(" · ");
            s.push_str(&self.host.label());
        }
        s
    }
}

impl Race {
    pub fn contender(&self, task: TaskId) -> Option<&Contender> {
        self.contenders.iter().find(|c| c.task == task)
    }

    pub fn contender_mut(&mut self, task: TaskId) -> Option<&mut Contender> {
        self.contenders.iter_mut().find(|c| c.task == task)
    }

    /// Record that `task`'s thread moved to `status` at `now`. The first
    /// finished turn settles the contender's time and outcome. True when the
    /// record changed (and should be saved).
    pub fn settle(&mut self, task: TaskId, status: TaskStatus, now: i64) -> bool {
        let Some(c) = self.contender_mut(task) else { return false };
        if c.finished_ms.is_some() {
            return false;
        }
        let outcome = match status {
            TaskStatus::Done => Outcome::Done,
            TaskStatus::Failed => Outcome::Failed,
            _ => return false,
        };
        c.finished_ms = Some(now.max(c.started_ms));
        c.outcome = Some(outcome);
        true
    }

    /// The race at a glance; `status` gives each live thread's status.
    pub fn phase(&self, status: impl Fn(TaskId) -> Option<TaskStatus>) -> Phase {
        if self.winner.is_some() {
            return Phase::Decided;
        }
        let left = self
            .contenders
            .iter()
            .filter(|c| matches!(c.standing(status(c.task)), Standing::Running | Standing::NeedsInput))
            .count();
        if left > 0 { Phase::Running { left } } else { Phase::Compare }
    }

    /// Badges per contender (same order as `contenders`): the winner, the
    /// fastest and the cheapest of those that finished well. A measure needs
    /// at least two finishers to mean anything; ties give no badge.
    pub fn marks(&self) -> Vec<Vec<Mark>> {
        let mut out: Vec<Vec<Mark>> = vec![Vec::new(); self.contenders.len()];
        let finished: Vec<usize> = self
            .contenders
            .iter()
            .enumerate()
            .filter(|(_, c)| c.outcome == Some(Outcome::Done) && c.check.as_ref().is_none_or(|k| k.passed))
            .map(|(i, _)| i)
            .collect();
        let best = |key: &dyn Fn(&Contender) -> Option<f64>| -> Option<usize> {
            let scored: Vec<(usize, f64)> =
                finished.iter().filter_map(|&i| key(&self.contenders[i]).map(|v| (i, v))).collect();
            if scored.len() < 2 {
                return None;
            }
            let min = scored.iter().map(|(_, v)| *v).fold(f64::INFINITY, f64::min);
            let at_min: Vec<usize> = scored.iter().filter(|(_, v)| *v == min).map(|(i, _)| *i).collect();
            (at_min.len() == 1).then(|| at_min[0])
        };
        if let Some(i) = best(&|c| c.finished_ms.map(|f| (f - c.started_ms) as f64)) {
            out[i].push(Mark::Fastest);
        }
        if let Some(i) = best(&|c| c.usage.and_then(|u| u.cost_usd)) {
            out[i].push(Mark::Cheapest);
        }
        if let Some(w) = self.winner
            && let Some(i) = self.contenders.iter().position(|c| c.task == w)
        {
            out[i].insert(0, Mark::Winner);
        }
        out
    }

    /// Contenders "Discard others" would remove once `winner` is picked.
    pub fn losers(&self) -> impl Iterator<Item = &Contender> {
        let winner = self.winner;
        self.contenders.iter().filter(move |c| Some(c.task) != winner && !c.discarded)
    }
}

// ---------------------------------------------------------------- storage

fn key(id: &str) -> String {
    format!("{KEY_PREFIX}{id}")
}

/// A fresh, time-ordered race id.
pub fn new_id() -> String {
    TaskId::new().to_string()
}

pub fn save(store: &Store, race: &Race) -> Result<()> {
    store.set_setting(&key(&race.id), &serde_json::to_string(race)?)
}

/// Every saved race, newest first. A record that no longer parses is skipped.
pub fn load_all(store: &Store) -> Vec<Race> {
    let mut races: Vec<Race> = store
        .settings_with_prefix(KEY_PREFIX)
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .filter_map(|(_, v)| serde_json::from_str(&v).ok())
        .collect();
    races.sort_by_key(|r: &Race| std::cmp::Reverse(r.created_ms));
    races
}

/// Forget a race (its threads and branches are left alone).
pub fn forget(store: &Store, id: &str) -> Result<()> {
    store.set_setting(&key(id), "")
}

// ---------------------------------------------------------------- git

/// Files and lines `worktree` changed against `base`, uncommitted and
/// untracked work included.
pub fn measure(worktree: &Path, base: &str) -> Result<DiffStat> {
    let (files, adds, dels) = dove_git::shortstat(worktree, base)?;
    Ok(DiffStat { files, adds, dels })
}

/// Merge contender `task` into the race's base branch, checked out at
/// `repo`. Work the agent left uncommitted is committed on its branch first.
/// Refuses (changing nothing) when `repo` has moved to another branch.
/// Returns a one-line summary.
pub fn merge_winner(repo: &Path, race: &Race, task: TaskId) -> Result<String> {
    let Some(c) = race.contender(task) else { bail!("that contender is not in this race") };
    if c.discarded {
        bail!("its worktree was discarded");
    }
    let Some(base) = race.base_branch.as_deref() else {
        bail!("the race started on a detached HEAD, so there is no branch to merge into; merge {} by hand", c.branch)
    };
    let current = dove_git::current_branch(repo)?;
    if current != base {
        bail!("the project is on `{current}`, not `{base}`. Check out `{base}` and try again");
    }
    if c.worktree.is_dir() {
        dove_git::commit_all(&c.worktree, &format!("{} (race: {})", race.title, c.harness.label()))?;
    }
    let message = format!("Merge {} (race winner: {})\n\n{}", c.branch, c.label(), race.title);
    dove_git::merge(repo, &c.branch, &message)?;
    Ok(format!("Merged {} into {base}", c.branch))
}

/// Remove a contender's worktree and branch. Uncommitted work is lost.
pub fn discard(repo: &Path, c: &Contender) -> Result<()> {
    dove_git::remove_worktree(repo, &c.worktree, Some(&c.branch))
}

// ---------------------------------------------------------------- checks

/// Run `command` through the system shell in `dir` and keep the tail of its
/// output. Blocking: call it off the UI thread. Never panics; a command that
/// can't start is a failed check that says why.
pub fn run_check(dir: &Path, command: &str, timeout: Duration, now_ms: i64) -> CheckResult {
    let started = Instant::now();
    let fail = |why: String| CheckResult {
        passed: false,
        code: None,
        timed_out: false,
        tail: vec![why],
        duration_ms: started.elapsed().as_millis() as u64,
        ran_ms: now_ms,
    };
    if !dir.is_dir() {
        return fail(format!("The worktree {} is gone.", dir.display()));
    }
    let mut cmd = shell(command);
    cmd.current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return fail(format!("Could not start the check: {e}")),
    };
    let lines: Arc<Mutex<VecDeque<String>>> = Arc::default();
    let readers: Vec<_> = [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
        child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(|pipe| {
        let lines = lines.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines() {
                let Ok(line) = line else { break };
                let mut l = lines.lock().unwrap_or_else(|p| p.into_inner());
                l.push_back(line);
                // Keep a bounded window; the tail is all that's shown.
                if l.len() > 400 {
                    l.pop_front();
                }
            }
        })
    })
    .collect();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                timed_out = true;
                break child.wait().ok();
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => break None,
        }
    };
    for r in readers {
        let _ = r.join();
    }
    let text = lines.lock().unwrap_or_else(|p| p.into_inner()).iter().cloned().collect::<Vec<_>>().join("\n");
    let mut tail = tail(&text, CHECK_TAIL);
    if timed_out {
        tail.push(format!("Stopped after {}.", duration_label(timeout.as_millis() as i64)));
    }
    let code = status.and_then(|s| s.code());
    CheckResult {
        passed: !timed_out && status.is_some_and(|s| s.success()),
        code,
        timed_out,
        tail,
        duration_ms: started.elapsed().as_millis() as u64,
        ran_ms: now_ms,
    }
}

fn shell(command: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c.creation_flags(0x0800_0000);
        c
    }
    #[cfg(not(windows))]
    {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contender(started: i64) -> Contender {
        Contender {
            task: TaskId::new(),
            harness: HarnessKind::ClaudeCode,
            model: None,
            host: Host::Local,
            branch: "race/x".into(),
            worktree: PathBuf::from("/nowhere"),
            started_ms: started,
            finished_ms: None,
            outcome: None,
            diff: None,
            usage: None,
            check: None,
            discarded: false,
        }
    }

    fn race(n: usize) -> Race {
        Race {
            id: new_id(),
            project: ProjectId::new(),
            title: "Fix the login".into(),
            prompt: "Fix the login".into(),
            created_ms: 1_000,
            base_branch: Some("main".into()),
            base: "abc".into(),
            check: None,
            contenders: (0..n).map(|_| contender(1_000)).collect(),
            winner: None,
            merged_ms: None,
        }
    }

    #[test]
    fn validate_bounds() {
        assert!(validate(1).is_err());
        assert!(validate(2).is_ok());
        assert!(validate(6).is_ok());
        assert!(validate(7).is_err());
    }

    #[test]
    fn worktree_names_are_ref_safe_and_distinct() {
        let a = worktree_name("Fix the flaky login test, please!", HarnessKind::ClaudeCode, &Host::Local);
        let b = worktree_name(
            "Fix the flaky login test, please!",
            HarnessKind::ClaudeCode,
            &Host::Ssh { alias: "vps".into() },
        );
        assert_eq!(a, "fix-the-flaky-logi-claude-code");
        assert_eq!(b, "fix-the-flaky-logi-claude-code-ssh-vps");
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        assert_eq!(worktree_name("!!!", HarnessKind::Codex, &Host::Local), "codex");
    }

    #[test]
    fn settle_records_only_the_first_finish() {
        let mut r = race(2);
        let t = r.contenders[0].task;
        assert!(!r.settle(t, TaskStatus::Running, 2_000));
        assert!(r.settle(t, TaskStatus::Done, 5_000));
        assert!(!r.settle(t, TaskStatus::Failed, 9_000));
        let c = r.contender(t).unwrap();
        assert_eq!((c.finished_ms, c.outcome), (Some(5_000), Some(Outcome::Done)));
        assert_eq!(c.elapsed_ms(99_000), 4_000);
        assert!(!r.settle(TaskId::new(), TaskStatus::Done, 1));
    }

    #[test]
    fn standing_prefers_live_status() {
        let mut c = contender(0);
        assert_eq!(c.standing(Some(TaskStatus::Running)), Standing::Running);
        assert_eq!(c.standing(Some(TaskStatus::Idle)), Standing::Running);
        assert_eq!(c.standing(None), Standing::Failed);
        c.outcome = Some(Outcome::Done);
        assert_eq!(c.standing(Some(TaskStatus::Idle)), Standing::Done);
        // A follow-up turn shows as working again.
        assert_eq!(c.standing(Some(TaskStatus::NeedsInput)), Standing::NeedsInput);
        c.discarded = true;
        assert_eq!(c.standing(Some(TaskStatus::Running)), Standing::Discarded);
    }

    #[test]
    fn phase_counts_what_is_left() {
        let mut r = race(3);
        let ids: Vec<TaskId> = r.contenders.iter().map(|c| c.task).collect();
        let running = |_: TaskId| Some(TaskStatus::Running);
        assert_eq!(r.phase(running), Phase::Running { left: 3 });
        r.settle(ids[0], TaskStatus::Done, 2_000);
        r.settle(ids[1], TaskStatus::Failed, 2_000);
        let live = |t: TaskId| Some(if t == ids[2] { TaskStatus::Running } else { TaskStatus::Idle });
        assert_eq!(r.phase(live), Phase::Running { left: 1 });
        r.settle(ids[2], TaskStatus::Done, 3_000);
        assert_eq!(r.phase(|_| Some(TaskStatus::Done)), Phase::Compare);
        r.winner = Some(ids[0]);
        assert_eq!(r.phase(running), Phase::Decided);
    }

    #[test]
    fn marks_need_two_finishers_and_no_tie() {
        let mut r = race(3);
        let ids: Vec<TaskId> = r.contenders.iter().map(|c| c.task).collect();
        r.settle(ids[0], TaskStatus::Done, 4_000);
        assert!(r.marks().iter().all(Vec::is_empty), "one finisher is not a comparison");
        r.settle(ids[1], TaskStatus::Done, 3_000);
        r.settle(ids[2], TaskStatus::Failed, 1_500);
        r.contenders[0].usage = Some(Usage { cost_usd: Some(0.10), ..Default::default() });
        r.contenders[1].usage = Some(Usage { cost_usd: Some(0.30), ..Default::default() });
        r.contenders[2].usage = Some(Usage { cost_usd: Some(0.01), ..Default::default() });
        let m = r.marks();
        assert_eq!(m[1], vec![Mark::Fastest]);
        assert_eq!(m[0], vec![Mark::Cheapest]);
        assert!(m[2].is_empty(), "a failed contender wins nothing");
        // A failed check disqualifies.
        r.contenders[1].check = Some(CheckResult {
            passed: false,
            code: Some(1),
            timed_out: false,
            tail: vec![],
            duration_ms: 1,
            ran_ms: 1,
        });
        assert!(r.marks().iter().all(|m| !m.contains(&Mark::Fastest)));
        r.winner = Some(ids[0]);
        assert_eq!(r.marks()[0], vec![Mark::Winner]);
        assert_eq!(r.losers().count(), 2);
    }

    #[test]
    fn tail_and_labels() {
        assert_eq!(tail("a\n\nb\nc\n  \n", 2), vec!["b", "c"]);
        assert_eq!(tail("", 3), Vec::<String>::new());
        assert_eq!(tail(&"x".repeat(300), 1)[0].chars().count(), 241);
        assert_eq!(duration_label(45_000), "45s");
        assert_eq!(duration_label(252_000), "4m 12s");
        assert_eq!(duration_label(3_780_000), "1h 03m");
        let mut c = contender(0);
        c.model = Some("opus".into());
        c.host = Host::Ssh { alias: "vps".into() };
        assert_eq!(c.label(), "Claude Code · opus · ssh: vps");
    }

    #[test]
    fn races_round_trip_through_settings() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut older = race(2);
        older.created_ms = 1;
        let newer = race(3);
        save(&store, &older).unwrap();
        save(&store, &newer).unwrap();
        let all = load_all(&store);
        assert_eq!(all.iter().map(|r| r.id.clone()).collect::<Vec<_>>(), vec![newer.id.clone(), older.id.clone()]);
        assert_eq!(all[0], newer);
        forget(&store, &older.id).unwrap();
        assert_eq!(load_all(&store).len(), 1);
        // Records from an older build without the optional fields still load.
        store
            .set_setting(
                "race.legacy",
                &format!(
                    r#"{{"id":"legacy","project":"{}","title":"t","prompt":"p","created_ms":0,"base":"b",
                    "contenders":[{{"task":"{}","harness":"codex","branch":"race/b","worktree":"/w","started_ms":0}}]}}"#,
                    ProjectId::new(),
                    TaskId::new()
                ),
            )
            .unwrap();
        let legacy = load_all(&store).into_iter().find(|r| r.id == "legacy").unwrap();
        assert_eq!(legacy.contenders[0].host, Host::Local);
        store.set_setting("race.broken", "{not json").unwrap();
        assert_eq!(load_all(&store).len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn checks_report_exit_and_tail() {
        let dir = tempfile::tempdir().unwrap();
        let ok = run_check(dir.path(), "echo one; echo two >&2; echo three", Duration::from_secs(10), 7);
        assert!(ok.passed);
        assert_eq!(ok.code, Some(0));
        assert_eq!(ok.ran_ms, 7);
        let mut lines = ok.tail.clone();
        lines.sort();
        assert_eq!(lines, vec!["one", "three", "two"]);
        let bad = run_check(dir.path(), "echo boom; exit 3", Duration::from_secs(10), 0);
        assert!(!bad.passed);
        assert_eq!((bad.code, bad.tail.clone()), (Some(3), vec!["boom".to_string()]));
        let slow = run_check(dir.path(), "sleep 5", Duration::from_millis(300), 0);
        assert!(slow.timed_out && !slow.passed);
        let gone = run_check(&dir.path().join("missing"), "true", Duration::from_secs(1), 0);
        assert!(!gone.passed && gone.tail[0].contains("is gone"));
    }

    #[test]
    fn merge_winner_merges_and_discard_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .current_dir(repo)
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "init"]);
        // Identity for the commits merge_winner makes itself.
        git(&["config", "user.name", "t"]);
        git(&["config", "user.email", "t@t"]);
        let base = dove_git::head_sha(repo).unwrap();
        let mut r = race(2);
        r.base = base.clone();
        for (i, c) in r.contenders.iter_mut().enumerate() {
            let wt = dove_git::create_worktree_from(repo, &format!("c{i}"), BRANCH_PREFIX, Some(&base)).unwrap();
            c.worktree = wt.path;
            c.branch = wt.branch;
        }
        // Contender 0 leaves its edit uncommitted, as agents often do.
        std::fs::write(r.contenders[0].worktree.join("b.txt"), "new\nfile\n").unwrap();
        assert_eq!(measure(&r.contenders[0].worktree, &base).unwrap(), DiffStat { files: 1, adds: 2, dels: 0 });
        let winner = r.contenders[0].task;
        let msg = merge_winner(repo, &r, winner).unwrap();
        assert_eq!(msg, "Merged race/c0 into main");
        assert!(repo.join("b.txt").is_file());
        // The loser goes away with its branch.
        discard(repo, &r.contenders[1]).unwrap();
        assert!(!r.contenders[1].worktree.exists());
        assert!(!dove_git::branches(repo).unwrap().contains(&"race/c1".to_string()));
        // On another branch, nothing is merged.
        git(&["checkout", "-q", "-b", "other"]);
        let err = merge_winner(repo, &r, winner).unwrap_err().to_string();
        assert!(err.contains("not `main`"), "{err}");
        r.base_branch = None;
        assert!(merge_winner(repo, &r, winner).unwrap_err().to_string().contains("detached"));
    }
}
