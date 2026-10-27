//! What the notch reads from Dove and the calls it makes. Every action goes
//! through the same Shell paths a click in the main window uses (Voice Mode
//! uses them too), so steering, queueing, approvals and new threads behave
//! exactly as they do there.

use dove_core::attention::TaskState;
use dove_core::proto::{HarnessKind, ProjectId, TaskId, TaskStatus};
use dove_core::store::{Project, Task};
use gpui::{AnyWindowHandle, App, Context, Global, WeakEntity, Window};
use serde::{Deserialize, Serialize};

use crate::flock::AgentState;
use crate::shell::{Route, Shell};

/// How much of an agent's latest reply travels in a snapshot.
pub const REPLY_CHARS: usize = 1200;

/// The main window and its shell, registered once at startup.
#[derive(Clone)]
pub struct NotchHost {
    pub window: AnyWindowHandle,
    pub shell: WeakEntity<Shell>,
}

impl Global for NotchHost {}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub agents: Vec<Agent>,
    pub flock: Vec<FlockAgent>,
    pub installed: Vec<HarnessKind>,
    pub projects: Vec<(ProjectId, String)>,
    pub default_project: Option<ProjectId>,
    pub default_agent: Option<HarnessKind>,
    /// Plan usage per agent kind that has reported it (the rate-limit strip).
    #[serde(default)]
    pub limits: Vec<Meter>,
    /// The agent the island has open, with its recent turns.
    #[serde(default)]
    pub focus: Option<Focus>,
    /// Pull requests of agents' branches, with their checks.
    #[serde(default)]
    pub prs: Vec<PrInfo>,
    #[serde(default)]
    pub automations: Vec<AutomationInfo>,
}

/// The open agent's last few exchanges, oldest first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Focus {
    pub id: TaskId,
    pub turns: Vec<TurnView>,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct TurnView {
    pub user: String,
    pub reply: String,
    /// "Edited 3 files · ran 2 commands"
    pub steps: String,
    pub files: Vec<String>,
    pub failed: u32,
    pub done: bool,
}

impl Focus {
    pub fn of(id: TaskId, state: &TaskState) -> Self {
        let turns = state
            .turns
            .iter()
            .map(|t| TurnView {
                user: t.user.clone(),
                reply: t.reply.trim().to_string(),
                steps: t.steps.summary(),
                files: t.steps.files.clone(),
                failed: t.steps.failed,
                done: t.done,
            })
            .collect();
        Focus { id, turns }
    }
}

/// A pull request an agent's branch has, as the island shows it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrInfo {
    pub task: TaskId,
    pub kind: HarnessKind,
    pub number: u64,
    pub title: String,
    pub url: String,
    /// `open`, `draft`, `merged` or `closed`.
    pub state: String,
    /// `passing`, `failing`, `pending` or `none`.
    pub checks: String,
    pub mergeable: bool,
}

/// A scheduled automation, as the island shows it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutomationInfo {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// When it next runs (store ms), for schedules.
    pub next_ms: Option<i64>,
    pub last_ms: Option<i64>,
    /// The last run's thread and how it went.
    pub last_task: Option<TaskId>,
    pub last_status: Option<TaskStatus>,
}

/// Every automation with its next and last run.
pub fn automations(engine: &dove_core::Engine) -> Vec<AutomationInfo> {
    let runs = engine.store.runs(100).unwrap_or_default();
    let now = chrono::Local::now();
    engine
        .store
        .automations()
        .unwrap_or_default()
        .into_iter()
        .map(|a| {
            let next_ms = dove_core::automation::Trigger::parse(&a.trigger)
                .filter(|t| a.enabled && t.is_schedule())
                .and_then(|t| t.next_after(now))
                .map(|t| t.timestamp_millis());
            let last = runs.iter().filter(|r| r.automation_id == a.id).max_by_key(|r| r.at);
            let last_status = last.and_then(|r| {
                engine.attention().get(r.task_id).map(|s| s.status).or_else(|| engine.store.task(r.task_id).ok().flatten().map(|t| t.status))
            });
            AutomationInfo {
                id: a.id.clone(),
                name: a.name.clone(),
                enabled: a.enabled,
                next_ms,
                last_ms: last.map(|r| r.at).or(a.last_run),
                last_task: last.map(|r| r.task_id),
                last_status,
            }
        })
        .collect()
}

/// One plan usage window ("Claude · 5-hour limit · 72 %").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Meter {
    pub kind: HarnessKind,
    pub label: String,
    /// 0–100.
    pub used: f64,
    /// Unix seconds.
    pub resets_at: Option<i64>,
}

/// Every reported window of the installed agents.
pub fn meters(engine: &dove_core::Engine, kinds: &[HarnessKind]) -> Vec<Meter> {
    let mut out = Vec::new();
    for kind in kinds {
        for w in engine.rate_limits(*kind) {
            if let Some(used) = w.used_percent {
                out.push(Meter { kind: *kind, label: w.label(), used, resets_at: w.resets_at });
            }
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    pub id: TaskId,
    pub title: String,
    pub kind: HarnessKind,
    pub project: String,
    pub status: TaskStatus,
    pub live: bool,
    pub activity: Option<String>,
    pub pending: Vec<Pending>,
    pub todos: Option<(u32, u32)>,
    pub teammate: Option<String>,
    pub updated_at: i64,
    /// The latest reply, its last [`REPLY_CHARS`] characters.
    #[serde(default)]
    pub reply: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub worktree: bool,
    /// When the current or last turn started and ended (store ms).
    #[serde(default)]
    pub started_ms: Option<i64>,
    #[serde(default)]
    pub ended_ms: Option<i64>,
    /// "Edited 3 files · ran 2 commands"
    #[serde(default)]
    pub steps: String,
    /// The last finished turn's cost.
    #[serde(default)]
    pub cost: Option<f64>,
    /// A message sent while it works steers the running turn (else it waits
    /// for the turn to end).
    #[serde(default)]
    pub steers: bool,
}

impl Agent {
    pub fn active(&self) -> bool {
        matches!(self.status, TaskStatus::Running | TaskStatus::NeedsInput) || !self.pending.is_empty()
    }

    pub fn needs_you(&self) -> bool {
        !self.pending.is_empty() || self.status == TaskStatus::NeedsInput
    }

    /// How long the current turn has run (or the last one ran), in seconds.
    pub fn elapsed_secs(&self, now_ms: i64) -> Option<i64> {
        let start = self.started_ms?;
        let end = if matches!(self.status, TaskStatus::Running | TaskStatus::NeedsInput) {
            now_ms
        } else {
            self.ended_ms.unwrap_or(now_ms)
        };
        Some(((end - start) / 1000).max(0))
    }
}

/// What an approval would do, for the card: the diff it would make, the
/// command it would run, or the file it would touch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Preview {
    Diff { file: String, added: u32, removed: u32, lines: Vec<(char, String)> },
    Command(String),
    File(String),
}

/// Lines of a diff preview.
const PREVIEW_LINES: usize = 8;

/// Reads a tool's input the way the common agents shape it (Claude's
/// `Edit`/`Write`/`MultiEdit`/`Bash`, Codex's `exec`, ACP's `path`/`command`).
pub fn preview_of(input: &serde_json::Value) -> Option<Preview> {
    let s = |k: &str| input.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let file = s("file_path").or_else(|| s("path")).or_else(|| s("filePath")).or_else(|| s("notebook_path"));
    let mut pairs: Vec<(String, String)> = Vec::new();
    if let (Some(old), Some(new)) = (s("old_string"), s("new_string")) {
        pairs.push((old, new));
    } else if let Some(edits) = input.get("edits").and_then(|e| e.as_array()) {
        for e in edits {
            let g = |k: &str| e.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
            pairs.push((g("old_string"), g("new_string")));
        }
    } else if let Some(content) = s("content").or_else(|| s("new_source")) {
        pairs.push((String::new(), content));
    }
    if !pairs.is_empty() {
        let (mut added, mut removed, mut lines) = (0u32, 0u32, Vec::new());
        for (old, new) in &pairs {
            let diff = similar::TextDiff::from_lines(old.as_str(), new.as_str());
            for change in diff.iter_all_changes() {
                let tag = match change.tag() {
                    similar::ChangeTag::Insert => {
                        added += 1;
                        '+'
                    }
                    similar::ChangeTag::Delete => {
                        removed += 1;
                        '-'
                    }
                    similar::ChangeTag::Equal => ' ',
                };
                if tag != ' ' && lines.len() < PREVIEW_LINES {
                    let text: String = change.value().trim_end_matches(['\n', '\r']).chars().take(160).collect();
                    lines.push((tag, text));
                }
            }
        }
        return Some(Preview::Diff { file: file.unwrap_or_default(), added, removed, lines });
    }
    let command = s("command").or_else(|| s("cmd")).or_else(|| {
        input
            .get("command")
            .and_then(|c| c.as_array())
            .map(|parts| parts.iter().filter_map(|p| p.as_str()).collect::<Vec<_>>().join(" "))
    });
    if let Some(c) = command.filter(|c| !c.trim().is_empty()) {
        return Some(Preview::Command(c.chars().take(400).collect()));
    }
    file.map(Preview::File)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub request_id: String,
    pub tool: String,
    pub summary: String,
    pub question: bool,
    /// Safe to approve from the notch (not a deletion, force push or work
    /// outside the thread's folder).
    pub safe: bool,
    #[serde(default)]
    pub preview: Option<Preview>,
    /// A question request's questions, answered in place.
    #[serde(default)]
    pub questions: Vec<QuestionView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuestionView {
    pub header: String,
    pub question: String,
    pub options: Vec<String>,
    pub multi: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlockAgent {
    pub kind: HarnessKind,
    pub place: String,
    pub working: bool,
    pub needs_input: bool,
}

/// One task as the island shows it, from the engine's view of it (`state`)
/// when there is one. Shared by the window's snapshot and the headless one.
pub fn agent_from(task: &Task, project: Option<&Project>, state: Option<&TaskState>, teammate: Option<String>) -> Agent {
    let empty = TaskState::default();
    let s = state.unwrap_or(&empty);
    // A state the caller has (the engine's, or the view's) is newer than the
    // stored status.
    let status = state.map(|s| s.status).unwrap_or(task.status);
    let cwd = task.worktree.clone().or_else(|| project.map(|p| p.path.clone())).map(|p| p.display().to_string());
    let done = s.todos.iter().filter(|t| t.status == dove_core::proto::TodoStatus::Completed).count() as u32;
    Agent {
        id: task.id,
        title: task.title.clone(),
        kind: task.harness,
        project: project.map(|p| p.name.clone()).unwrap_or_default(),
        status,
        live: s.live,
        activity: s.activity.clone(),
        pending: s
            .pending
            .iter()
            .map(|r| {
                let p = dove_voice::snapshot::Pending {
                    request_id: r.request_id.clone(),
                    tool: r.tool.clone(),
                    summary: r.summary.to_string(),
                    input: r.input.clone(),
                    question: r.is_question(),
                };
                let safe = dove_voice::safety::classify(&p, cwd.as_deref()) == dove_voice::safety::Risk::Normal;
                let preview = preview_of(&p.input);
                let questions = r
                    .questions
                    .iter()
                    .map(|q| QuestionView {
                        header: q.header.clone(),
                        question: q.question.clone(),
                        options: q.options.clone(),
                        multi: q.multi_select,
                    })
                    .collect();
                Pending {
                    request_id: p.request_id,
                    tool: p.tool,
                    summary: p.summary,
                    question: p.question,
                    safe,
                    preview,
                    questions,
                }
            })
            .collect(),
        todos: (!s.todos.is_empty()).then_some((done, s.todos.len() as u32)),
        teammate,
        updated_at: task.updated_at,
        reply: s.reply_tail(REPLY_CHARS),
        branch: task.branch.clone(),
        worktree: task.worktree.is_some(),
        started_ms: s.turn_started_ms,
        ended_ms: s.turn_ended_ms,
        steps: s.steps.summary(),
        cost: s.turn_cost,
        steers: dove_core::steers(task.harness),
    }
}

/// Needs you, then working, then the rest by recency.
pub fn sort_agents(agents: &mut [Agent]) {
    agents.sort_by_key(|a| {
        let rank = if !a.pending.is_empty() || a.status == TaskStatus::NeedsInput {
            0
        } else if a.status == TaskStatus::Running {
            1
        } else {
            2
        };
        (rank, -a.updated_at)
    });
}

pub fn snapshot(cx: &App) -> Option<Snapshot> {
    let host = cx.try_global::<NotchHost>()?;
    let shell = host.shell.upgrade()?;
    Some(shell.read(cx).notch_snapshot(cx))
}

/// Runs `f` on the shell in the main window.
pub fn with_shell<R>(cx: &mut App, f: impl FnOnce(&mut Shell, &mut Window, &mut Context<Shell>) -> R) -> Option<R> {
    let host = cx.try_global::<NotchHost>()?.clone();
    let shell = host.shell.upgrade()?;
    host.window.update(cx, |_, window, cx| shell.update(cx, |s, cx| f(s, window, cx))).ok()
}

/// Brings the main window forward, restored if it was minimized or hidden in the tray.
fn raise(window: &mut Window) {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible, SW_RESTORE, SW_SHOW, ShowWindow};
        if let Ok(handle) = HasWindowHandle::window_handle(window)
            && let RawWindowHandle::Win32(h) = handle.as_raw()
        {
            let hwnd = h.hwnd.get() as windows_sys::Win32::Foundation::HWND;
            // SAFETY: a live top-level window owned by this process.
            unsafe {
                if IsWindowVisible(hwnd) == 0 {
                    ShowWindow(hwnd, SW_SHOW);
                }
                if IsIconic(hwnd) != 0 {
                    ShowWindow(hwnd, SW_RESTORE);
                }
            }
        }
    }
    window.activate_window();
}

impl Shell {
    pub fn notch_snapshot(&self, cx: &App) -> Snapshot {
        let mut agents = Vec::new();
        let mut recent = 0;
        for task in self.tasks.iter().filter(|t| !matches!(t.status, TaskStatus::Draft | TaskStatus::Archived)) {
            // The engine's attention model first (it knows threads with no
            // view open), then the view, then the stored status.
            let mut state = self.engine.attention().get(task.id).filter(|s| s.live).unwrap_or_else(|| {
                let v = self.task_state(task.id, cx);
                let status = self.threads.get(&task.id).map(|t| t.read(cx).chat.read(cx).status).unwrap_or(task.status);
                TaskState {
                    status,
                    live: v.live,
                    capabilities: v.capabilities,
                    pending: v.pending,
                    todos: v.todos,
                    activity: v.status_text,
                    last_error: v.last_error,
                    reply: String::new(),
                    ..TaskState::default()
                }
            });
            if state.reply.trim().is_empty() {
                state.reply = self.task_reply(task.id, REPLY_CHARS, cx);
            }
            let busy = matches!(state.status, TaskStatus::Running | TaskStatus::NeedsInput) || !state.pending.is_empty();
            if !busy {
                // Idle threads only as a short "recent" tail.
                if recent >= 6 {
                    continue;
                }
                recent += 1;
            }
            let teammate = self.persona_for(task.id).map(|p| p.name.clone());
            agents.push(agent_from(task, self.project(task.project_id), Some(&state), teammate));
        }
        sort_agents(&mut agents);
        let flock = self
            .workspace
            .read(cx)
            .agents()
            .iter()
            .filter(|r| r.state != AgentState::Exited)
            .map(|r| FlockAgent {
                kind: r.kind,
                place: r.place.clone(),
                working: r.state == AgentState::Working,
                needs_input: r.state == AgentState::NeedsInput,
            })
            .collect();
        let composer = self.composer.read(cx);
        Snapshot {
            agents,
            flock,
            installed: self.installed.iter().map(|i| i.kind).collect(),
            projects: self.projects.iter().map(|p| (p.id, p.name.clone())).collect(),
            default_project: composer.project.as_ref().map(|p| p.id).or(self.home_project),
            default_agent: Some(composer.harness(cx)),
            limits: meters(&self.engine, &self.installed.iter().map(|i| i.kind).collect::<Vec<_>>()),
            // The host adds these (they don't depend on the window).
            focus: None,
            prs: Vec::new(),
            automations: Vec::new(),
        }
    }

    /// The window is closing: hands over every thread's live session so the
    /// engine keeps it (the agents keep running, headless).
    pub fn surrender_sessions(&mut self, cx: &mut Context<Self>) -> Vec<dove_core::LiveSession> {
        let views: Vec<_> = self.threads.values().map(|v| v.read(cx).chat.clone()).collect();
        views.into_iter().filter_map(|chat| chat.update(cx, |c, cx| c.take_live(cx))).collect()
    }

    /// The window is back: a session kept while headless goes to its
    /// thread's view, as if the view had started it.
    pub fn adopt_live(&mut self, live: dove_core::LiveSession, window: &mut Window, cx: &mut Context<Self>) {
        let id = live.task;
        // The task list in memory may predate a thread started headless.
        if self.task(id).is_none()
            && let Ok(Some(task)) = self.engine.store.task(id)
        {
            self.tasks.insert(0, task);
        }
        let Some(task) = self.task(id).cloned() else { return };
        let Some(project) = self.project(task.project_id).cloned() else { return };
        self.threads.remove(&id);
        self.make_thread(task, project, Some(live), window, cx);
        cx.notify();
    }

    pub fn notch_open(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        self.open_task(id, window, cx);
        raise(window);
    }

    pub fn notch_open_flock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Flock, window, cx);
        raise(window);
    }

    pub fn notch_open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Settings, window, cx);
        raise(window);
    }

    /// Starts a thread as the home composer would, optionally in a new
    /// worktree. Returns its id, or the reason it didn't start.
    pub fn notch_start(
        &mut self,
        kind: HarnessKind,
        project: ProjectId,
        prompt: String,
        worktree: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<TaskId, String> {
        let project = self.projects.iter().find(|p| p.id == project).cloned().ok_or("That project is gone.")?;
        let join = if worktree {
            let w = dove_core::git::create_worktree(&project.path, &dove_core::git::slug(&prompt), "dove/")
                .map_err(|e| format!("Couldn't make a worktree: {e}"))?;
            Some(dove_core::Joined { worktree: Some(w.path), branch: Some(w.branch), base: Some(w.base) })
        } else {
            None
        };
        let (run, persona) = {
            let c = self.composer.read(cx);
            if c.harness(cx) == kind { (c.run_settings(cx), c.persona.clone()) } else { Default::default() }
        };
        let new = dove_core::NewTask {
            title: crate::ui::title_from_prompt(&prompt),
            prompt,
            harness: kind,
            run,
            join,
            persona,
        };
        let (task, live) = self.engine.start_task(&project, new).map_err(|e| format!("It didn't start: {e}"))?;
        let id = task.id;
        self.tasks.insert(0, task.clone());
        self.make_thread(task, project, Some(live), window, cx);
        cx.notify();
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_preview_as_a_diff() {
        let input = serde_json::json!({
            "file_path": "src/auth.rs",
            "old_string": "let a = 1;\nlet b = 2;\n",
            "new_string": "let a = 1;\nlet b = 3;\nlet c = 4;\n"
        });
        let Some(Preview::Diff { file, added, removed, lines }) = preview_of(&input) else { panic!("a diff") };
        assert_eq!(file, "src/auth.rs");
        assert_eq!((added, removed), (2, 1));
        assert_eq!(lines[0], ('-', "let b = 2;".to_string()));
    }

    #[test]
    fn commands_and_files_preview_too() {
        assert_eq!(preview_of(&serde_json::json!({"command": "cargo test"})), Some(Preview::Command("cargo test".into())));
        assert_eq!(
            preview_of(&serde_json::json!({"command": ["git", "push"]})),
            Some(Preview::Command("git push".into()))
        );
        assert_eq!(preview_of(&serde_json::json!({"path": "a.md"})), Some(Preview::File("a.md".into())));
        assert_eq!(preview_of(&serde_json::json!({})), None);
    }
}
