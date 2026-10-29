//! Notch V3 (docs/notch-v3-plan.md): talking to K22, agents' plans to
//! approve, and views — built-in lenses made from the snapshot, and views
//! agents compose with `notch_view` — all drawn from one block vocabulary
//! ([`crate::island::view`]). These pages are wide (`tk::WIDE_W`).

use super::*;
use dove_core::store::Store;
use std::collections::HashSet;
use crate::island::view::{Block, CallPhase, NotchView, PlanStep, PlanVerdict, Tone, ViewAction, ViewSource};

/// A built-in lens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lens {
    /// Every agent as a lane.
    Status,
    /// Approvals, questions and plans, in one queue.
    Needs,
    /// What finished, failed and is left, with next steps.
    Today,
    /// One agent, all of it.
    Agent(TaskId),
}

impl Lens {
    pub fn id(self) -> String {
        match self {
            Lens::Status => "status".into(),
            Lens::Needs => "needs".into(),
            Lens::Today => "today".into(),
            Lens::Agent(t) => format!("agent:{t}"),
        }
    }

    pub fn from_id(id: &str) -> Option<Lens> {
        match id {
            "status" | "running" => Some(Lens::Status),
            "needs" | "needs-you" => Some(Lens::Needs),
            "today" => Some(Lens::Today),
            _ => id.strip_prefix("agent:").and_then(TaskId::parse).map(Lens::Agent),
        }
    }

    /// A lens a question maps to with no model call ("what's running").
    pub fn for_question(q: &str) -> Option<Lens> {
        let q = q.trim().trim_end_matches(['?', '.', '!']).to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| q.contains(w));
        if has(&["needs me", "need me", "needs you", "waiting on me", "approve", "approvals", "pending"]) {
            Some(Lens::Needs)
        } else if has(&["today", "summary", "summarize my day", "what happened", "recap"]) {
            Some(Lens::Today)
        } else if has(&["what's running", "whats running", "what is running", "status", "everyone doing", "agents doing", "progress"])
        {
            Some(Lens::Status)
        } else {
            None
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Lens::Status => "Status",
            Lens::Needs => "Needs you",
            Lens::Today => "Today",
            Lens::Agent(_) => "Agent",
        }
    }
}

/// The plan being reviewed: its steps as edited so far.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanEdit {
    pub task: TaskId,
    pub request_id: Option<String>,
    /// The plan text it was made from (a new plan restarts the edit).
    pub source: String,
    pub steps: Vec<PlanStep>,
    pub original: Vec<PlanStep>,
    pub mode: PlanMode,
    pub selected: Option<usize>,
    pub open: HashSet<usize>,
    /// Answered: the plan turns into the agent's live checklist.
    pub answered: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanMode {
    View,
    /// Editing a step's text in the line field.
    Edit(usize),
    /// Adding a step at the end.
    Add,
    /// Writing why it's rejected.
    Reject,
}

impl PlanEdit {
    pub fn dirty(&self) -> bool {
        self.steps != self.original
    }
}

/// An ask waiting for K22's view.
#[derive(Clone, Debug)]
pub struct Asking {
    pub id: u64,
    pub text: String,
    pub task: Option<TaskId>,
    pub at: Instant,
}

fn tone_color(t: Tone) -> Hsla {
    match t {
        Tone::Good => tk::GREEN,
        Tone::Warn => tk::AMBER,
        Tone::Bad => tk::RED,
        Tone::Info => tk::BLUE,
        Tone::Neutral => tk::TEXT,
    }
}

fn eid(s: String) -> gpui::ElementId {
    gpui::ElementId::Name(s.into())
}

/// An icon by its file name (`check`, `bell`), for agents' views.
fn icon_named(name: &str) -> &'static str {
    let want = format!("/{}.svg", name.trim().to_lowercase().replace([' ', '_'], "-"));
    icons::ALL_ICONS.iter().copied().find(|p| p.ends_with(&want)).unwrap_or(icons::CHECK)
}

/// A rough height for a block, so the island sizes itself before drawing.
fn block_h(b: &Block) -> f32 {
    let lines = |t: &str, per: usize| t.lines().map(|l| l.chars().count().div_ceil(per).max(1)).sum::<usize>() as f32;
    match b {
        Block::Heading { .. } => 24.0,
        Block::Text { text } => lines(text, 84).min(30.0) * 18.0 + 4.0,
        Block::Stat { .. } | Block::Stats { .. } => 62.0,
        Block::Progress { .. } => 34.0,
        Block::List { items } => items.len() as f32 * 42.0,
        Block::Checklist { items } => items.len() as f32 * 26.0,
        Block::Timeline { items } => items.len() as f32 * 26.0,
        Block::Diff { lines, .. } => 44.0 + lines.len().min(16) as f32 * 16.0,
        Block::Agent { .. } => 64.0,
        Block::Plan { .. } => 150.0,
        Block::Actions { .. } => tk::CONTROL_H + 4.0,
        Block::Callout { text, .. } => 26.0 + lines(text, 80) * 17.0,
        Block::Split { left, right } => blocks_h(left).max(blocks_h(right)),
        Block::Section { blocks, .. } => 24.0 + blocks_h(blocks),
        Block::Unknown => 0.0,
    }
}

fn blocks_h(blocks: &[Block]) -> f32 {
    blocks.iter().map(block_h).sum::<f32>() + 10.0 * blocks.len().saturating_sub(1) as f32
}

impl Notch {
    // ----- size ------------------------------------------------------------------------

    /// The wide pages' width, if this page is one.
    pub(crate) fn page_width(&self) -> f32 {
        if self.page.wide() && self.query.trim().is_empty() { tk::WIDE_W } else { tk::EXPANDED_W }
    }

    /// The wide island's ceiling on this display.
    pub(crate) fn wide_max_h(&self) -> f32 {
        let display = self.display.map(|d| d.height as f32 / d.scale.max(0.5) * tk::WIDE_SHARE);
        display.map_or(tk::WIDE_MAX_H, |d| d.clamp(tk::EXPANDED_MAX_H, tk::WIDE_MAX_H))
    }

    /// The wide pages' body height (without the header).
    pub(crate) fn v3_body_height(&self) -> Option<f32> {
        let h = match &self.page {
            Page::Call => {
                if self.call.live {
                    // Each line wraps at about 80 characters.
                    let lines: f32 = self
                        .call
                        .lines
                        .iter()
                        .rev()
                        .take(4)
                        .map(|(_, t)| t.chars().count().div_ceil(80).max(1) as f32 * 19.0 + 6.0)
                        .sum();
                    let partial = if self.call.partial.as_ref().is_some_and(|p| !p.trim().is_empty()) { 24.0 } else { 0.0 };
                    let error = if self.call.error.is_some() { 46.0 } else { 0.0 };
                    let typing = if self.call_typing { 46.0 } else { 0.0 };
                    // Chips, the orb at its fullest (it swells with the voice), the
                    // state, the controls, and the gaps between.
                    344.0 + lines + partial + error + typing + if self.call.awaiting_click.is_some() { 56.0 } else { 0.0 }
                } else {
                    210.0 + self.views.len().min(3) as f32 * 42.0
                }
            }
            Page::Plan(task) => {
                let steps = self
                    .plan_edit
                    .as_ref()
                    .filter(|e| e.task == *task)
                    .map(|e| {
                        e.steps.iter().map(|s| s.text.chars().count().div_ceil(80).max(1) as f32 * 18.0 + 22.0).sum::<f32>()
                            + e.open.iter().filter_map(|i| e.steps.get(*i)).map(|s| s.detail.len() as f32 * 18.0).sum::<f32>()
                    })
                    .unwrap_or(120.0);
                let files = if self.plan_for(*task).is_some_and(|p| !p.files.is_empty()) { 30.0 } else { 0.0 };
                let editing = if self.plan_edit.as_ref().is_some_and(|e| e.mode != PlanMode::View) { 44.0 } else { 0.0 };
                // Title, steps, files, the answer row, and the gaps between.
                64.0 + steps + files + editing + 28.0 + 24.0 + tk::CONTROL_H + 24.0
            }
            Page::View(id) => match self.views.iter().find(|v| v.id == *id) {
                Some(v) => 52.0 + blocks_h(&v.blocks) + self.view_reply_h(v) + 58.0,
                None => 220.0,
            },
            Page::Lens(l) => 52.0 + blocks_h(&self.lens_view(*l).blocks) + 12.0,
            _ => return None,
        };
        Some(h.clamp(200.0, self.wide_max_h() - tk::HEADER_H - 14.0))
    }

    fn view_reply_h(&self, v: &NotchView) -> f32 {
        let reply = self.view_reply(v);
        if reply.is_empty() { 0.0 } else { (reply.chars().count().div_ceil(86).min(5) as f32) * 17.0 + 14.0 }
    }

    /// What the agent that made the view said with it (its latest reply).
    fn view_reply(&self, v: &NotchView) -> String {
        v.task
            .and_then(|t| self.snap.agents.iter().find(|a| a.id == t))
            .map(|a| a.reply.trim().to_string())
            .unwrap_or_default()
    }

    pub(crate) fn plan_for(&self, task: TaskId) -> Option<&crate::island::view::PendingPlan> {
        let plans: Vec<_> = self.snap.plans.iter().filter(|p| p.task == task).collect();
        plans.iter().find(|p| p.request_id.is_some()).or(plans.first()).copied()
    }

    // ----- opening ---------------------------------------------------------------------

    /// Opens a page and the island with it (a view arriving, voice's "show me").
    pub(crate) fn unfold(&mut self, page: Page, cx: &mut Context<Self>) {
        self.open_page(page, None, cx);
        if self.machine.state != State::Expanded && self.machine.open().is_some() {
            self.transition(cx);
        }
        cx.notify();
    }

    /// Talk to K22 (or about `task`), or end the call running.
    pub(crate) fn talk(&mut self, task: Option<TaskId>, cx: &mut Context<Self>) {
        if self.call.live && task.is_none() {
            self.command(ToEngine::Call { on: false, task: None });
        } else {
            self.command(ToEngine::Call { on: true, task });
            self.call.error = None;
            self.unfold(Page::Call, cx);
        }
    }

    /// Asks K22 (typed). A question a lens answers opens it at once.
    pub(crate) fn ask_k22(&mut self, text: String, cx: &mut Context<Self>) {
        if let Some(lens) = Lens::for_question(&text) {
            self.unfold(Page::Lens(lens), cx);
            return;
        }
        let id = self.next_request;
        self.next_request += 1;
        self.asking = Some(Asking { id, text: text.clone(), task: None, at: Instant::now() });
        self.command(ToEngine::Ask { id, text: text.clone() });
        let pending = NotchView {
            id,
            title: text.chars().take(80).collect(),
            subtitle: Some("K22 is on it…".into()),
            pending: true,
            ..Default::default()
        };
        self.remember_view(pending);
        self.unfold(Page::View(id), cx);
    }

    /// Keeps a view (newest first); a newer one with the same id replaces it.
    pub(crate) fn remember_view(&mut self, v: NotchView) {
        if let Some(w) = self.widgets.iter_mut().find(|w| w.id == v.id) {
            *w = v.clone();
            self.save_widgets();
        }
        self.views.retain(|x| x.id != v.id);
        self.views.insert(0, v);
        self.views.truncate(12);
    }

    pub(crate) fn save_widgets(&self) {
        if let Ok(json) = serde_json::to_string(&self.widgets) {
            let _ = self.store.set_setting("notch_widgets", &json);
        }
    }

    pub(crate) fn load_widgets(store: &Store) -> Vec<NotchView> {
        store.setting("notch_widgets").and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
    }

    fn toggle_widget(&mut self, v: &NotchView) {
        if self.widgets.iter().any(|w| w.id == v.id) {
            self.widgets.retain(|w| w.id != v.id);
            self.toast("Unpinned from Home");
        } else {
            let mut v = v.clone();
            v.pending = false;
            self.widgets.insert(0, v);
            self.widgets.truncate(3);
            self.toast("Pinned to Home");
        }
        self.save_widgets();
    }

    // ----- view actions ----------------------------------------------------------------

    /// A button or row in a view, clicked. Approvals the notch can't make
    /// safely go to the app instead.
    pub(crate) fn run_view_action(&mut self, a: ViewAction, window: &mut Window, cx: &mut Context<Self>) {
        // A double click never sends a command twice.
        if a.acts() {
            if self.last_action.as_ref().is_some_and(|(x, at)| *x == a && at.elapsed() < Duration::from_millis(1500)) {
                return;
            }
            self.last_action = Some((a.clone(), Instant::now()));
        }
        match a {
            ViewAction::Reply { task, text } => {
                self.command(ToEngine::Send { task, text });
                self.toast("Sent");
            }
            ViewAction::Approve { task, request } => {
                let safe = self
                    .snap
                    .agents
                    .iter()
                    .find(|x| x.id == task)
                    .and_then(|x| x.pending.iter().find(|p| p.request_id == request))
                    .is_some_and(|p| p.safe && !p.question);
                if safe {
                    self.command(ToEngine::Decide { task, request_id: request, decision: PermissionDecision::AllowOnce });
                    self.toast("Approved");
                } else {
                    self.command(ToEngine::Open { task: Some(task) });
                    self.toast("That one needs your click in Keplorayx");
                }
            }
            ViewAction::Deny { task, request } => {
                self.command(ToEngine::Decide { task, request_id: request, decision: PermissionDecision::Deny });
                self.toast("Denied");
            }
            ViewAction::Stop { task } => self.stop_agent(task, cx),
            ViewAction::Open { task } => self.open_page(Page::Agent(task), Some(window), cx),
            ViewAction::Start { prompt, agent, project } => {
                let find_agent = |s: &str| {
                    let s = s.to_lowercase();
                    self.snap
                        .installed
                        .iter()
                        .copied()
                        .find(|k| k.id() == s || k.label().to_lowercase().contains(&s))
                };
                let kind = agent.as_deref().and_then(find_agent).or(self.snap.voice.agent).or(self.snap.default_agent);
                let project = project
                    .as_deref()
                    .and_then(|p| {
                        let p = p.to_lowercase();
                        self.snap.projects.iter().find(|(id, name)| id.to_string() == p || name.to_lowercase() == p)
                    })
                    .map(|(id, _)| *id)
                    .or(self.snap.default_project)
                    .or(self.snap.projects.first().map(|p| p.0));
                match (kind, project) {
                    (Some(agent), Some(project)) => {
                        let id = self.next_request;
                        self.next_request += 1;
                        self.command(ToEngine::Start { id, agent, project, prompt, worktree: false });
                        self.toast(format!("Starting {}…", agent.label()));
                    }
                    _ => self.new_agent_with(prompt, Some(window), cx),
                }
            }
            ViewAction::Merge { task, pr } => {
                self.command(ToEngine::Merge { task, number: pr });
                self.toast("Merging…");
            }
            ViewAction::RunAutomation { id } => {
                self.command(ToEngine::RunAutomation { id });
                self.toast("Running");
            }
            ViewAction::Timer { mins } => {
                self.start_timer_mins(mins.clamp(1, 600), cx);
            }
            ViewAction::Ask { text } => self.ask_k22(text, cx),
            ViewAction::Plan { task } => self.open_page(Page::Plan(task), Some(window), cx),
            ViewAction::Call { task } => self.talk(task, cx),
            ViewAction::None => {}
        }
        cx.notify();
    }

    // ----- the line field (plan edits, typing into the call, follow-ups) ---------------

    /// Enter in the line field, by page.
    pub(crate) fn line_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.inputs.as_ref().map(|i| i.line.clone()) else { return };
        let text = input.read(cx).value().trim().to_string();
        let clear = |window: &mut Window, cx: &mut Context<Self>| input.update(cx, |s, cx| s.set_value("", window, cx));
        match self.page.clone() {
            Page::Call => {
                if !text.is_empty() {
                    self.command(ToEngine::CallSay { text });
                }
                self.call_typing = false;
                clear(window, cx);
            }
            Page::View(_) | Page::Lens(_) => {
                if !text.is_empty() {
                    clear(window, cx);
                    self.ask_k22(text, cx);
                }
            }
            Page::Plan(_) => {
                let Some(edit) = self.plan_edit.as_mut() else { return };
                match edit.mode {
                    PlanMode::Edit(i) => {
                        if text.is_empty() {
                            if i < edit.steps.len() {
                                edit.steps.remove(i);
                            }
                        } else if let Some(s) = edit.steps.get_mut(i) {
                            s.text = text;
                        }
                        edit.mode = PlanMode::View;
                    }
                    PlanMode::Add => {
                        if !text.is_empty() {
                            edit.steps.push(PlanStep { text, detail: Vec::new() });
                        }
                        edit.mode = PlanMode::View;
                    }
                    PlanMode::Reject => {
                        let (task, request_id) = (edit.task, edit.request_id.clone());
                        edit.mode = PlanMode::View;
                        edit.answered = true;
                        self.command(ToEngine::Plan { task, request_id, verdict: PlanVerdict::Reject { reason: text } });
                        self.toast("Sent back to keep planning");
                    }
                    PlanMode::View => {}
                }
                clear(window, cx);
            }
            _ => {}
        }
        cx.notify();
    }

    fn focus_line(&mut self, value: String, cx: &mut Context<Self>) {
        self.take_focus(cx);
        self.with_window(cx, move |n, window, cx| {
            if let Some(i) = n.inputs.clone() {
                i.line.update(cx, |s, cx| {
                    s.set_value(value, window, cx);
                    s.focus(window, cx);
                });
            }
        });
    }

    // ----- the call --------------------------------------------------------------------

    /// Five bars that move with the call's level (the slim capsule's mark).
    pub(crate) fn wave(&self, height: f32) -> AnyElement {
        let t = self.t();
        let level = if self.call.live { self.call.level.max(0.12) } else { 0.12 };
        let color = if self.call.muted { tk::DIM } else { tk::K22 };
        let mut row = div().h(px(height)).flex().items_center().gap(px(2.0));
        for (i, share) in [0.45f32, 0.8, 1.0, 0.7, 0.5].into_iter().enumerate() {
            let wobble = 0.65 + 0.35 * ((t * 7.0 + i as f32 * 1.3).sin() * 0.5 + 0.5);
            let h = (height * share * (0.3 + 0.7 * level) * wobble).max(3.0);
            row = row.child(div().w(px(2.5)).h(px(h)).rounded_full().bg(color));
        }
        row.into_any_element()
    }

    /// The orb: K22's mark in a ring that breathes with the voice.
    fn orb(&self, size: f32) -> AnyElement {
        let c = &self.call;
        let t = self.t();
        let level = if c.live { c.level } else { 0.0 };
        let (color, glow) = match c.phase {
            _ if !c.live => (tk::K22.opacity(0.55), 0.08),
            CallPhase::Muted => (tk::DIM, 0.05),
            CallPhase::Thinking => (tk::K22, 0.12 + 0.10 * breathe(t)),
            CallPhase::Speaking => (tk::K22, 0.16 + 0.35 * level),
            CallPhase::Hearing => (tk::K22, 0.18 + 0.4 * level),
            CallPhase::Listening => (tk::K22, 0.10 + 0.06 * breathe(t * 0.6)),
            CallPhase::Loading => (tk::K22.opacity(0.6), 0.06 + 0.06 * breathe(t)),
        };
        let swell = size * (0.16 + 0.22 * level);
        div()
            .size(px(size + 2.0 * swell.max(size * 0.16)))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size(px(size + 2.0 * swell))
                    .rounded_full()
                    .bg(color.opacity(glow))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .size(px(size))
                            .rounded_full()
                            .bg(linear_gradient(
                                160.0,
                                linear_color_stop(color.opacity(0.95), 0.0),
                                linear_color_stop(Hsla { l: 0.32, ..color }, 1.0),
                            ))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(icons::K22_MARK).size(px(size * 0.38)).text_color(tk::WHITE)),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn page_call(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let c = self.call.clone();
        if c.live && !crate::motion::reduced_now() {
            window.request_animation_frame();
        }
        let line = self.inputs.as_ref().map(|i| i.line.clone());
        let mut col = div().id("call").size_full().flex().flex_col().items_center().gap(px(10.0)).pt(px(6.0));
        // Who it talks to, and K22's agent.
        let target = c.target.clone().unwrap_or_else(|| "K22".into());
        let agent = c.agent.or(self.snap.voice.agent);
        let model = c.model.clone().or(self.snap.voice.model.clone());
        let model_label = model
            .as_ref()
            .and_then(|m| self.snap.voice.models.iter().find(|x| &x.0 == m).map(|x| x.1.clone()))
            .or(model.clone())
            .unwrap_or_else(|| "Default model".into());
        col = col.child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(6.0))
                .child(
                    div()
                        .h(px(26.0))
                        .px(px(10.0))
                        .rounded_full()
                        .bg(tk::WASH)
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(match c.target_kind {
                            Some(k) if target != "K22" => mark(k, 13.0),
                            _ => icon(icons::K22_MARK).size(px(13.0)).text_color(tk::K22).into_any_element(),
                        })
                        .child(div().text_size(px(tk::T12)).text_color(tk::TEXT).max_w(px(260.0)).truncate().child(target)),
                )
                .child(
                    div()
                        .id("call-model")
                        .h(px(26.0))
                        .px(px(10.0))
                        .rounded_full()
                        .bg(tk::WASH)
                        .hover(|s| s.bg(tk::WASH_HOVER))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .tooltip(tip("K22's agent and model"))
                        .children(agent.map(|k| mark(k, 13.0)))
                        .child(div().text_size(px(tk::T12)).text_color(tk::DIM).child(model_label))
                        .child(icon(icons::ALT_ARROW_DOWN).size(px(12.0)).text_color(tk::FAINT))
                        .on_click(cx.listener(|this, ev: &ClickEvent, _, cx| {
                            let items = this.voice_menu();
                            this.open_menu(ev.position(), items, cx);
                        })),
                ),
        );
        col = col.child(self.orb(96.0));
        let phase = if c.live { c.phase.word() } else { "Talk to K22" };
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .text_size(px(tk::T15))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if c.live { tk::TEXT } else { tk::DIM })
                        .child(phase),
                )
                .when(c.live, |d| {
                    d.child(div().font_family(tk::MONO).text_size(px(tk::T12)).text_color(tk::FAINT).child(mins_secs(
                        Duration::from_secs(c.elapsed_s as u64),
                    )))
                }),
        );
        if let Some(e) = &c.error {
            col = col.child(
                div()
                    .w_full()
                    .p(px(10.0))
                    .rounded(px(tk::R_ROW))
                    .bg(tk::RED.opacity(0.12))
                    .text_size(px(tk::T12))
                    .text_color(tk::RED)
                    .child(e.clone()),
            );
        }
        if c.live {
            // The conversation: you on the right, K22 on the left.
            let mut lines = div().w_full().flex().flex_col().gap(px(6.0));
            let shown: Vec<_> = c.lines.iter().rev().take(4).rev().cloned().collect();
            for (i, (you, text)) in shown.into_iter().enumerate() {
                lines = lines.child(
                    div().w_full().flex().when(you, |d| d.justify_end()).child(
                        div()
                            .id(("call-line", i))
                            .max_w(gpui::relative(0.86))
                            .text_size(px(if you { tk::T12 } else { tk::T13 }))
                            .text_color(if you { tk::DIM } else { tk::TEXT })
                            .when(you, |d| d.text_right())
                            .child(text),
                    ),
                );
            }
            if let Some(p) = c.partial.clone().filter(|p| !p.trim().is_empty()) {
                lines = lines.child(
                    div().w_full().flex().justify_end().child(
                        div().text_size(px(tk::T12)).text_color(tk::FAINT).italic().child(p),
                    ),
                );
            }
            col = col.child(lines);
            if let Some((task, _)) = c.awaiting_click {
                col = col.child(
                    div()
                        .w_full()
                        .p(px(10.0))
                        .rounded(px(tk::R_ROW))
                        .bg(tk::AMBER.opacity(0.12))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(icon(icons::DANGER_TRIANGLE).size(px(15.0)).text_color(tk::AMBER))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(tk::T12))
                                .text_color(tk::TEXT)
                                .child("That needs your click in Keplorayx (voice never confirms it)."),
                        )
                        .child(pill("call-confirm", "Open", false).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.command(ToEngine::Open { task: Some(task) });
                            cx.notify();
                        }))),
                );
            }
            if self.call_typing
                && let Some(line) = line
            {
                col = col.child(
                    div()
                        .w_full()
                        .h(px(36.0))
                        .px(px(12.0))
                        .rounded_full()
                        .bg(tk::WASH)
                        .flex()
                        .items_center()
                        .text_size(px(tk::T13))
                        .child(div().flex_1().child(Input::new(&line))),
                );
            }
            // Controls.
            let muted = c.muted;
            let last_view = self.views.iter().find(|v| !v.pending).map(|v| v.id);
            let target_task = c.target_task;
            col = col.child(
                div()
                    .mt(px(4.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        ibtn("call-mute", icons::MICROPHONE, 36.0, if muted { tk::RED } else { tk::TEXT }, "Mute")
                            .bg(if muted { tk::RED.opacity(0.16) } else { tk::WASH })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.command(ToEngine::CallMute { on: !muted });
                                this.call.muted = !muted;
                                cx.notify();
                            })),
                    )
                    .child(ibtn("call-type", icons::TEXT_CURSOR, 36.0, tk::TEXT, "Type instead").bg(tk::WASH).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.call_typing = !this.call_typing;
                            if this.call_typing {
                                this.focus_line(String::new(), cx);
                            }
                            cx.notify();
                        }),
                    ))
                    .child(ibtn("call-show", icons::ARTIFACTS, 36.0, tk::TEXT, "Show me").bg(tk::WASH).on_click(
                        cx.listener(move |this, _: &ClickEvent, window, cx| match last_view {
                            Some(id) => this.open_page(Page::View(id), Some(window), cx),
                            None => this.open_page(Page::Lens(Lens::Status), Some(window), cx),
                        }),
                    ))
                    .child(ibtn("call-open", icons::LINK, 36.0, tk::TEXT, "Open in Keplorayx").bg(tk::WASH).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.command(ToEngine::Open { task: target_task });
                            cx.notify();
                        }),
                    ))
                    .child(
                        div()
                            .id("call-end")
                            .h(px(36.0))
                            .px(px(18.0))
                            .rounded_full()
                            .bg(tk::RED)
                            .hover(|s| s.bg(tk::RED.opacity(0.85)))
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .text_size(px(tk::T13))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tk::WHITE)
                            .child("End")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.command(ToEngine::Call { on: false, task: None });
                                cx.notify();
                            })),
                    ),
            );
        } else {
            col = col
                .child(
                    div()
                        .max_w(px(360.0))
                        .text_center()
                        .text_size(px(tk::T12))
                        .text_color(tk::FAINT)
                        .child("Talk to K22 about your agents: what's running, start one, review a plan, approve safe steps."),
                )
                .child(primary("call-start", "Start talking").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.talk(None, cx);
                })));
            if !self.views.is_empty() {
                let mut recent = div().w_full().mt(px(6.0)).flex().flex_col().gap(px(2.0)).child(section("Recent views"));
                for (i, v) in self.views.iter().filter(|v| !v.pending).take(3).enumerate() {
                    let id = v.id;
                    recent = recent.child(
                        div()
                            .id(("recent-view", i))
                            .h(px(38.0))
                            .px(px(10.0))
                            .rounded(px(tk::R_ROW))
                            .hover(|s| s.bg(tk::WASH))
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(icon(icons::ARTIFACTS).size(px(tk::ICON_ROW)).text_color(tk::DIM))
                            .child(div().flex_1().truncate().text_size(px(tk::T13)).text_color(tk::TEXT).child(v.title.clone()))
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.open_page(Page::View(id), Some(window), cx)
                            })),
                    );
                }
                col = col.child(recent);
            }
        }
        col.into_any_element()
    }

    /// K22's agent and model, to pick from (the call page's chip).
    pub(crate) fn voice_menu(&self) -> Vec<MenuItem> {
        let mut items = vec![MenuItem::head("K22 runs on")];
        for k in &self.snap.installed {
            items.push(MenuItem::check(k.label(), self.snap.voice.agent == Some(*k), MenuAction::VoiceAgent(*k)));
        }
        if !self.snap.voice.models.is_empty() {
            items.push(MenuItem::head("Model"));
            items.push(MenuItem::check(
                "Agent's default",
                self.snap.voice.model.is_none(),
                MenuAction::VoiceModel(None),
            ));
            for (id, label) in self.snap.voice.models.iter().take(8) {
                items.push(MenuItem::check(
                    label.clone(),
                    self.snap.voice.model.as_ref() == Some(id),
                    MenuAction::VoiceModel(Some(id.clone())),
                ));
            }
        }
        items
    }

    // ----- plans -----------------------------------------------------------------------

    /// The edit state for `task`'s plan, made (or remade, for a new plan).
    fn plan_edit_for(&mut self, task: TaskId) -> Option<PlanEdit> {
        let plan = self.plan_for(task).cloned();
        match (plan, self.plan_edit.clone()) {
            (Some(p), Some(e)) if e.task == task && e.source == p.text => Some(e),
            (Some(p), _) => {
                let e = PlanEdit {
                    task,
                    request_id: p.request_id.clone(),
                    source: p.text.clone(),
                    steps: p.steps.clone(),
                    original: p.steps.clone(),
                    mode: PlanMode::View,
                    selected: None,
                    open: HashSet::new(),
                    answered: false,
                };
                self.plan_edit = Some(e.clone());
                Some(e)
            }
            (None, Some(e)) if e.task == task => Some(e),
            (None, _) => None,
        }
    }

    fn edit_plan(&mut self, f: impl FnOnce(&mut PlanEdit), cx: &mut Context<Self>) {
        if let Some(e) = self.plan_edit.as_mut() {
            f(e);
        }
        cx.notify();
    }

    /// Alt+↑/↓ move the selected step, Delete removes it, Enter edits it.
    pub(crate) fn plan_key(&mut self, key: &str, alt: bool, cx: &mut Context<Self>) -> bool {
        let Page::Plan(_) = self.page else { return false };
        let Some(e) = self.plan_edit.as_mut() else { return false };
        if e.mode != PlanMode::View {
            return false;
        }
        let Some(i) = e.selected else { return false };
        match (key, alt) {
            ("up", true) if i > 0 => {
                e.steps.swap(i, i - 1);
                e.selected = Some(i - 1);
            }
            ("down", true) if i + 1 < e.steps.len() => {
                e.steps.swap(i, i + 1);
                e.selected = Some(i + 1);
            }
            ("up", false) if i > 0 => e.selected = Some(i - 1),
            ("down", false) if i + 1 < e.steps.len() => e.selected = Some(i + 1),
            ("delete", _) | ("backspace", _) => {
                e.steps.remove(i);
                e.selected = if e.steps.is_empty() { None } else { Some(i.min(e.steps.len() - 1)) };
            }
            ("enter", _) => {
                let text = e.steps.get(i).map(|s| s.text.clone()).unwrap_or_default();
                e.mode = PlanMode::Edit(i);
                self.focus_line(text, cx);
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(crate) fn page_plan(&mut self, task: TaskId, cx: &mut Context<Self>) -> AnyElement {
        let Some(edit) = self.plan_edit_for(task) else {
            return empty(icons::LAYERS, "No plan waiting", "When an agent proposes a plan, it shows here to approve or change.")
                .into_any_element();
        };
        let plan = self.plan_for(task).cloned();
        let agent = self.snap.agents.iter().find(|a| a.id == task).cloned();
        let kind = plan.as_ref().map(|p| p.kind).or(agent.as_ref().map(|a| a.kind));
        let title = plan.as_ref().map(|p| p.title.clone()).or(agent.as_ref().map(|a| a.title.clone())).unwrap_or_default();
        let project = plan.as_ref().map(|p| p.project.clone()).unwrap_or_default();
        let line = self.inputs.as_ref().map(|i| i.line.clone());
        let mut col = div().id("plan").size_full().overflow_y_scroll().flex().flex_col().gap(px(8.0)).pt(px(4.0));
        // Who, and what it's for.
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .children(kind.map(|k| avatar(k, 28.0, None)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(tk::T15))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(tk::TEXT)
                                .truncate()
                                .child(title),
                        )
                        .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(format!(
                            "{}{} · {} steps{}",
                            kind.map(|k| k.label()).unwrap_or("Agent"),
                            if project.is_empty() { String::new() } else { format!(" · {project}") },
                            edit.steps.len(),
                            if edit.dirty() { " · edited" } else { "" }
                        ))),
                )
                .when(edit.request_id.is_none() && !edit.answered, |d| {
                    d.child(div().text_size(px(tk::T11)).text_color(tk::BLUE).child("Proposed"))
                }),
        );
        if edit.answered {
            // Approved: the agent's checklist as it works.
            let todos = agent.as_ref().map(|a| a.todo_list.clone()).unwrap_or_default();
            let mut list = div().flex().flex_col().gap(px(4.0));
            if todos.is_empty() {
                list = list.child(div().text_size(px(tk::T12)).text_color(tk::DIM).child("Sent. Its checklist shows here as it works."));
            }
            for (i, (text, status)) in todos.iter().enumerate() {
                let done = status == "completed";
                let active = status == "in_progress";
                list = list.child(
                    div()
                        .id(("plan-todo", i))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(if done {
                            icon(icons::CHECK).size(px(14.0)).text_color(tk::GREEN).into_any_element()
                        } else {
                            self.dot(if active { tk::BLUE } else { tk::GHOST }, 7.0, active).into_any_element()
                        })
                        .child(
                            div()
                                .text_size(px(tk::T13))
                                .text_color(if done { tk::FAINT } else { tk::TEXT })
                                .when(done, |d| d.line_through())
                                .child(text.clone()),
                        ),
                );
            }
            col = col.child(card().child(list));
            col = col.child(
                div().flex().gap(px(6.0)).child(pill("plan-open-agent", "Open agent", false).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| this.open_page(Page::Agent(task), Some(window), cx),
                ))),
            );
            return col.into_any_element();
        }
        // The steps.
        let mut steps = div().flex().flex_col().gap(px(2.0));
        for (i, s) in edit.steps.iter().enumerate() {
            let open = edit.open.contains(&i);
            let selected = edit.selected == Some(i);
            let editing = edit.mode == PlanMode::Edit(i);
            let group = SharedString::from(format!("plan-step-{i}"));
            let row = div()
                .id(("plan-step", i))
                .group(group.clone())
                .min_h(px(38.0))
                .px(px(8.0))
                .py(px(6.0))
                .rounded(px(tk::R_ROW))
                .flex()
                .items_start()
                .gap(px(10.0))
                .when(selected, |d| d.bg(tk::WASH))
                .hover(|s| s.bg(tk::WASH))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.edit_plan(
                        |e| {
                            e.selected = Some(i);
                            if !e.open.remove(&i) {
                                e.open.insert(i);
                            }
                        },
                        cx,
                    );
                }))
                .child(
                    div()
                        .mt(px(1.0))
                        .size(px(20.0))
                        .flex_none()
                        .rounded_full()
                        .bg(tk::WASH_HOVER)
                        .flex()
                        .items_center()
                        .justify_center()
                        .font_family(tk::MONO)
                        .text_size(px(tk::T11))
                        .text_color(tk::DIM)
                        .child(format!("{}", i + 1)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .child(match (editing, line.clone()) {
                            (true, Some(l)) => div()
                                .h(px(28.0))
                                .px(px(8.0))
                                .rounded(px(8.0))
                                .bg(tk::WASH_HOVER)
                                .flex()
                                .items_center()
                                .text_size(px(tk::T13))
                                .child(div().flex_1().child(Input::new(&l)))
                                .into_any_element(),
                            _ => div()
                                .text_size(px(tk::T13))
                                .text_color(tk::TEXT)
                                .child(markdown(&s.text, &format!("plan-{i}")))
                                .into_any_element(),
                        })
                        .when(open, |d| {
                            d.children(s.detail.iter().map(|x| {
                                div().text_size(px(tk::T12)).text_color(tk::DIM).child(format!("· {x}")).into_any_element()
                            }))
                        }),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(2.0))
                        .invisible()
                        .group_hover(group, |s| s.visible())
                        .when(selected, |d| d.visible())
                        .child(ibtn(("plan-edit", i), icons::HIGHLIGHTER, 24.0, tk::DIM, "Edit · Enter").on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                let text = this
                                    .plan_edit
                                    .as_ref()
                                    .and_then(|e| e.steps.get(i))
                                    .map(|s| s.text.clone())
                                    .unwrap_or_default();
                                this.edit_plan(
                                    |e| {
                                        e.mode = PlanMode::Edit(i);
                                        e.selected = Some(i);
                                    },
                                    cx,
                                );
                                this.focus_line(text, cx);
                            }),
                        ))
                        .child(ibtn(("plan-up", i), icons::ALT_ARROW_UP, 24.0, tk::DIM, "Move up · Alt+↑").on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.edit_plan(
                                    |e| {
                                        if i > 0 {
                                            e.steps.swap(i, i - 1);
                                            e.selected = Some(i - 1);
                                        }
                                    },
                                    cx,
                                );
                            }),
                        ))
                        .child(
                            ibtn(("plan-down", i), icons::ALT_ARROW_DOWN, 24.0, tk::DIM, "Move down · Alt+↓").on_click(
                                cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    cx.stop_propagation();
                                    this.edit_plan(
                                        |e| {
                                            if i + 1 < e.steps.len() {
                                                e.steps.swap(i, i + 1);
                                                e.selected = Some(i + 1);
                                            }
                                        },
                                        cx,
                                    );
                                }),
                            ),
                        )
                        .child(ibtn(("plan-remove", i), icons::CLOSE, 24.0, tk::DIM, "Remove · Delete").on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.edit_plan(
                                    |e| {
                                        if i < e.steps.len() {
                                            e.steps.remove(i);
                                        }
                                        e.selected = None;
                                    },
                                    cx,
                                );
                            }),
                        )),
                );
            steps = steps.child(row);
        }
        // Adding a step, or writing why it's rejected.
        match (edit.mode, line.clone()) {
            (PlanMode::Add | PlanMode::Reject, Some(l)) => {
                steps = steps.child(
                    div()
                        .mt(px(4.0))
                        .h(px(34.0))
                        .px(px(12.0))
                        .rounded_full()
                        .bg(if edit.mode == PlanMode::Reject { tk::RED.opacity(0.10) } else { tk::WASH })
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(px(tk::T13))
                        .child(
                            div()
                                .text_size(px(tk::T11))
                                .text_color(tk::FAINT)
                                .child(if edit.mode == PlanMode::Reject { "Why?" } else { "New step" }),
                        )
                        .child(div().flex_1().child(Input::new(&l))),
                );
            }
            _ => {
                steps = steps.child(
                    link("plan-add", "+ Add a step").mt(px(2.0)).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.edit_plan(|e| e.mode = PlanMode::Add, cx);
                        this.focus_line(String::new(), cx);
                    })),
                );
            }
        }
        col = col.child(steps);
        if let Some(p) = plan.as_ref().filter(|p| !p.files.is_empty()) {
            let mut files = div().flex().flex_wrap().items_center().gap(px(4.0)).child(
                div().text_size(px(tk::T11)).text_color(tk::FAINT).mr(px(2.0)).child("Touches"),
            );
            for f in p.files.iter().take(6) {
                files = files.child(
                    div()
                        .h(px(20.0))
                        .px(px(8.0))
                        .rounded_full()
                        .bg(tk::WASH)
                        .flex()
                        .items_center()
                        .font_family(tk::MONO)
                        .text_size(px(tk::T11))
                        .text_color(tk::DIM)
                        .child(f.clone()),
                );
            }
            col = col.child(files);
        }
        // The answer.
        let dirty = edit.dirty();
        let gated = edit.request_id.is_some();
        col = col.child(
            div()
                .mt(px(4.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    primary("plan-approve", if dirty { "Approve with edits" } else if gated { "Approve" } else { "Looks good" })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let Some(e) = this.plan_edit.as_mut() else { return };
                            let verdict = if e.dirty() {
                                PlanVerdict::Edited { text: crate::island::view::plan_markdown(&e.steps) }
                            } else {
                                PlanVerdict::Approve
                            };
                            let (task, request_id) = (e.task, e.request_id.clone());
                            e.answered = true;
                            this.command(ToEngine::Plan { task, request_id, verdict });
                            this.toast("Approved");
                            cx.notify();
                        })),
                )
                .when(dirty, |d| {
                    d.child(pill("plan-reset", "Undo edits", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.edit_plan(|e| e.steps = e.original.clone(), cx)
                    })))
                })
                .child(pill("plan-reject", "Keep planning…", false).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.edit_plan(|e| e.mode = PlanMode::Reject, cx);
                    this.focus_line(String::new(), cx);
                })))
                .child(div().flex_1())
                .child(
                    ibtn("plan-talk", icons::WAVEFORM, 30.0, tk::DIM, "Talk it through")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.talk(Some(task), cx))),
                )
                .child(
                    ibtn("plan-open", icons::LINK, 30.0, tk::DIM, "Open in Keplorayx").on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.command(ToEngine::Open { task: Some(task) });
                            cx.notify();
                        },
                    )),
                ),
        );
        col.into_any_element()
    }

    // ----- views and lenses ------------------------------------------------------------

    pub(crate) fn page_view(&mut self, id: u64, cx: &mut Context<Self>) -> AnyElement {
        let view = self.views.iter().find(|v| v.id == id).cloned();
        let Some(v) = view else {
            return empty(icons::ARTIFACTS, "That view is gone", "Views from K22 and your agents show here.")
                .into_any_element();
        };
        let reply = self.view_reply(&v);
        let pinned = self.widgets.iter().any(|w| w.id == v.id);
        let line = self.inputs.as_ref().map(|i| i.line.clone());
        let mut col = div().id(eid(format!("view-{id}"))).size_full().overflow_y_scroll().flex().flex_col().gap(px(10.0)).pt(px(4.0));
        col = col.child(self.view_title(&v, pinned, cx));
        if v.pending && v.blocks.is_empty() {
            // The skeleton while K22 works, with what it's saying.
            let t = self.t();
            if let Some(a) = self.asking.as_ref().filter(|a| a.id == id) {
                let slow = a.at.elapsed() > Duration::from_secs(90);
                col = col.child(
                    div()
                        .text_size(px(tk::T12))
                        .text_color(tk::FAINT)
                        .child(if slow { format!("Still working on “{}”…", a.text) } else { format!("“{}”", a.text) }),
                );
            }
            for (i, w) in [0.92f32, 0.7, 0.84].into_iter().enumerate() {
                col = col.child(
                    div()
                        .h(px(if i == 0 { 54.0 } else { 34.0 }))
                        .w(gpui::relative(w))
                        .rounded(px(tk::R_ROW))
                        .bg(tk::WASH.opacity(0.6 + 0.4 * breathe(t + i as f32 * 0.3))),
                );
            }
        }
        // The unfold: blocks rise in one after another.
        let key = self.key();
        for (i, b) in v.blocks.iter().enumerate() {
            let el = self.block(b, &format!("v{id}-{i}"), cx);
            col = col.child(rise(el, (key ^ id, i), i));
        }
        if !reply.is_empty() && !v.pending || (v.pending && !reply.is_empty()) {
            col = col.child(
                div()
                    .pt(px(4.0))
                    .text_size(px(tk::T12))
                    .text_color(tk::DIM)
                    .child(markdown(&reply.chars().rev().take(600).collect::<String>().chars().rev().collect::<String>(), &format!("vr{id}"))),
            );
        }
        if let Some(l) = line {
            col = col.child(
                div()
                    .mt(px(2.0))
                    .h(px(36.0))
                    .px(px(12.0))
                    .rounded_full()
                    .bg(tk::WASH)
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(icon(icons::K22_MARK).size(px(14.0)).text_color(tk::K22))
                    .child(div().flex_1().text_size(px(tk::T13)).child(Input::new(&l)))
                    .child(ibtn("view-talk", icons::WAVEFORM, 28.0, tk::DIM, "Talk to K22").on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| this.talk(None, cx),
                    ))),
            );
        }
        col.into_any_element()
    }

    fn view_title(&self, v: &NotchView, pinned: bool, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = v.clone();
        div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(match (v.kind, &v.source) {
                (_, ViewSource::Lens(_)) => icon(icons::LAYERS).size(px(18.0)).text_color(tk::DIM).into_any_element(),
                (Some(k), _) if !v.task.is_some_and(|t| self.is_k22_task(t)) => avatar(k, 26.0, None),
                _ => icon(icons::K22_MARK).size(px(18.0)).text_color(tk::K22).into_any_element(),
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(tk::T15))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tk::TEXT)
                            .truncate()
                            .child(v.title.clone()),
                    )
                    .children(v.subtitle.clone().map(|s| {
                        div().text_size(px(tk::T11)).text_color(if v.pending { tk::K22 } else { tk::FAINT }).truncate().child(s)
                    })),
            )
            .when(!v.pending, |d| {
                d.child(
                    ibtn("view-pin", icons::PIN, 28.0, if pinned { tk::TEXT } else { tk::DIM }, "Pin to Home")
                        .when(pinned, |d| d.bg(tk::WASH_ON))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.toggle_widget(&snapshot);
                            cx.notify();
                        })),
                )
            })
            .into_any_element()
    }

    fn is_k22_task(&self, t: TaskId) -> bool {
        self.snap.agents.iter().any(|a| a.id == t && a.teammate.as_deref() == Some("K22"))
    }

    pub(crate) fn page_lens(&mut self, lens: Lens, cx: &mut Context<Self>) -> AnyElement {
        let v = self.lens_view(lens);
        let mut col = div().id(eid(format!("lens-{}", lens.id()))).size_full().overflow_y_scroll().flex().flex_col().gap(px(10.0)).pt(px(4.0));
        col = col.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .children([Lens::Status, Lens::Needs, Lens::Today].into_iter().map(|l| {
                    pill(eid(format!("lens-tab-{}", l.id())), l.title(), l == lens)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.open_page(Page::Lens(l), Some(window), cx)
                        }))
                        .into_any_element()
                }))
                .child(div().flex_1())
                .children(v.subtitle.clone().map(|s| div().text_size(px(tk::T11)).text_color(tk::FAINT).child(s))),
        );
        for (i, b) in v.blocks.iter().enumerate() {
            col = col.child(self.block(b, &format!("l{}-{i}", lens.id()), cx));
        }
        col.into_any_element()
    }

    /// A lens as a view, from the snapshot (it updates with it).
    pub(crate) fn lens_view(&self, lens: Lens) -> NotchView {
        use crate::island::view::{Button, ListItem, StatItem, TimelineItem};
        let agents = &self.snap.agents;
        let mut blocks = Vec::new();
        let mut subtitle = None;
        match lens {
            Lens::Status => {
                let running = agents.iter().filter(|a| a.live && a.status == TaskStatus::Running).count();
                let needs = agents.iter().filter(|a| a.needs_you()).count();
                let failed = agents.iter().filter(|a| a.status == TaskStatus::Failed).count();
                blocks.push(Block::Stats {
                    items: vec![
                        StatItem { value: running.to_string(), label: "working".into(), trend: None, tone: Tone::Info },
                        StatItem {
                            value: needs.to_string(),
                            label: "need you".into(),
                            trend: None,
                            tone: if needs > 0 { Tone::Warn } else { Tone::Neutral },
                        },
                        StatItem {
                            value: failed.to_string(),
                            label: "failed".into(),
                            trend: None,
                            tone: if failed > 0 { Tone::Bad } else { Tone::Neutral },
                        },
                        StatItem { value: self.snap.plans.len().to_string(), label: "plans".into(), trend: None, tone: Tone::Neutral },
                    ],
                });
                let active: Vec<_> = agents.iter().filter(|a| a.active()).collect();
                if active.is_empty() {
                    blocks.push(Block::Callout { text: "Nothing is running right now.".into(), tone: Tone::Neutral });
                    blocks.push(Block::Actions {
                        items: vec![Button {
                            label: "Talk to K22".into(),
                            action: ViewAction::Call { task: None },
                            primary: true,
                        }],
                    });
                }
                for a in active.iter().take(8) {
                    blocks.push(Block::Agent { task: a.id });
                }
            }
            Lens::Needs => {
                for p in &self.snap.plans {
                    blocks.push(Block::List {
                        items: vec![ListItem {
                            title: format!("Plan ready · {}", p.title),
                            meta: Some(format!("{} · {} steps", p.kind.label(), p.steps.len())),
                            icon: Some("layers".into()),
                            tone: Tone::Info,
                            action: Some(ViewAction::Plan { task: p.task }),
                        }],
                    });
                }
                for a in agents.iter().filter(|a| a.needs_you()) {
                    let mut inner = Vec::new();
                    for p in a.pending.iter().filter(|p| !crate::island::view::is_plan_request(&dove_core::proto::PermissionRequest {
                        request_id: p.request_id.clone(),
                        tool: p.tool.clone(),
                        summary: p.summary.clone().into(),
                        input: serde_json::Value::Null,
                        questions: Vec::new(),
                    })) {
                        inner.push(Block::Text { text: p.summary.clone() });
                        if let Some(Preview::Diff { file, added, removed, lines }) = &p.preview {
                            inner.push(Block::Diff {
                                file: file.clone(),
                                added: *added,
                                removed: *removed,
                                lines: lines.iter().map(|(t, l)| format!("{t}{l}")).collect(),
                            });
                        } else if let Some(Preview::Command(c)) = &p.preview {
                            inner.push(Block::Diff { file: "command".into(), added: 0, removed: 0, lines: vec![format!(" {c}")] });
                        }
                        let mut buttons = Vec::new();
                        if p.question {
                            buttons.push(Button { label: "Answer".into(), action: ViewAction::Open { task: a.id }, primary: true });
                        } else {
                            buttons.push(Button {
                                label: if p.safe { "Approve".into() } else { "Review in app".into() },
                                action: ViewAction::Approve { task: a.id, request: p.request_id.clone() },
                                primary: true,
                            });
                            buttons.push(Button {
                                label: "Deny".into(),
                                action: ViewAction::Deny { task: a.id, request: p.request_id.clone() },
                                primary: false,
                            });
                        }
                        inner.push(Block::Actions { items: buttons });
                    }
                    if a.pending.is_empty() {
                        inner.push(Block::Text { text: "Waiting on your reply.".into() });
                        inner.push(Block::Actions {
                            items: vec![Button { label: "Reply".into(), action: ViewAction::Open { task: a.id }, primary: true }],
                        });
                    }
                    if !inner.is_empty() {
                        blocks.push(Block::Section { title: format!("{} · {}", a.kind.label(), a.title), blocks: inner });
                    }
                }
                if blocks.is_empty() {
                    blocks.push(Block::Callout { text: "You're all caught up: nothing needs you.".into(), tone: Tone::Good });
                }
            }
            Lens::Today => {
                let today = chrono::Local::now().date_naive();
                let is_today = |ms: Option<i64>| {
                    ms.and_then(chrono::DateTime::from_timestamp_millis)
                        .is_some_and(|t| t.with_timezone(&chrono::Local).date_naive() == today)
                };
                let done: Vec<_> = agents
                    .iter()
                    .filter(|a| a.status == TaskStatus::Done && is_today(a.ended_ms.or(Some(a.updated_at))))
                    .collect();
                let failed: Vec<_> = agents.iter().filter(|a| a.status == TaskStatus::Failed).collect();
                let running: Vec<_> = agents.iter().filter(|a| a.active()).collect();
                subtitle = Some(chrono::Local::now().format("%A %e %B").to_string());
                blocks.push(Block::Stats {
                    items: vec![
                        StatItem { value: done.len().to_string(), label: "finished".into(), trend: None, tone: Tone::Good },
                        StatItem { value: running.len().to_string(), label: "in progress".into(), trend: None, tone: Tone::Info },
                        StatItem {
                            value: failed.len().to_string(),
                            label: "failed".into(),
                            trend: None,
                            tone: if failed.is_empty() { Tone::Neutral } else { Tone::Bad },
                        },
                    ],
                });
                let mut timeline: Vec<(i64, TimelineItem)> = Vec::new();
                for a in done.iter().chain(failed.iter()) {
                    let at = a.ended_ms.unwrap_or(a.updated_at);
                    let time = chrono::DateTime::from_timestamp_millis(at)
                        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
                        .unwrap_or_default();
                    timeline.push((
                        at,
                        TimelineItem {
                            time,
                            text: format!("{} · {}", a.kind.label(), a.title),
                            tone: if a.status == TaskStatus::Failed { Tone::Bad } else { Tone::Good },
                        },
                    ));
                }
                timeline.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
                if !timeline.is_empty() {
                    blocks.push(Block::Section {
                        title: "What happened".into(),
                        blocks: vec![Block::Timeline { items: timeline.into_iter().map(|(_, t)| t).take(10).collect() }],
                    });
                }
                // Next steps.
                let mut next: Vec<ListItem> = Vec::new();
                for p in &self.snap.plans {
                    next.push(ListItem {
                        title: format!("Review {}'s plan", p.kind.label()),
                        meta: Some(p.title.clone()),
                        icon: Some("layers".into()),
                        tone: Tone::Info,
                        action: Some(ViewAction::Plan { task: p.task }),
                    });
                }
                for pr in self.snap.prs.iter().filter(|p| p.state == "open" && p.checks == "passing" && p.mergeable) {
                    next.push(ListItem {
                        title: format!("Merge #{} — checks passed", pr.number),
                        meta: Some(pr.title.clone()),
                        icon: Some("git-merge".into()),
                        tone: Tone::Good,
                        action: Some(ViewAction::Merge { task: pr.task, pr: pr.number }),
                    });
                }
                for a in failed.iter().take(3) {
                    next.push(ListItem {
                        title: format!("Look at what failed: {}", a.title),
                        meta: Some(a.kind.label().to_string()),
                        icon: Some("danger-triangle".into()),
                        tone: Tone::Bad,
                        action: Some(ViewAction::Open { task: a.id }),
                    });
                }
                for a in agents.iter().filter(|a| a.needs_you()).take(3) {
                    next.push(ListItem {
                        title: format!("Answer {}", a.kind.label()),
                        meta: Some(a.title.clone()),
                        icon: Some("bell".into()),
                        tone: Tone::Warn,
                        action: Some(ViewAction::Open { task: a.id }),
                    });
                }
                if next.is_empty() {
                    next.push(ListItem {
                        title: "Ask K22 what to do next".into(),
                        meta: None,
                        icon: Some("k22-mark".into()),
                        tone: Tone::Neutral,
                        action: Some(ViewAction::Ask { text: "What should I do next?".into() }),
                    });
                }
                blocks.push(Block::Section { title: "Next".into(), blocks: vec![Block::List { items: next }] });
            }
            Lens::Agent(t) => {
                blocks.push(Block::Agent { task: t });
                if self.plan_for(t).is_some() {
                    blocks.push(Block::Plan { task: t });
                }
                if let Some(a) = agents.iter().find(|a| a.id == t) {
                    if !a.todo_list.is_empty() {
                        blocks.push(Block::Checklist {
                            items: a
                                .todo_list
                                .iter()
                                .map(|(text, s)| crate::island::view::CheckItem {
                                    text: text.clone(),
                                    done: s == "completed",
                                    active: s == "in_progress",
                                })
                                .collect(),
                        });
                    }
                    if !a.reply.trim().is_empty() {
                        blocks.push(Block::Text { text: a.reply.clone() });
                    }
                }
            }
        }
        NotchView {
            id: 0,
            title: lens.title().into(),
            subtitle,
            blocks,
            source: ViewSource::Lens(lens.id()),
            ..Default::default()
        }
    }

    // ----- blocks ----------------------------------------------------------------------

    fn action_click(
        &self,
        a: &ViewAction,
        cx: &mut Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        let a = a.clone();
        cx.listener(move |this, _: &ClickEvent, window, cx| this.run_view_action(a.clone(), window, cx))
    }

    pub(crate) fn block(&self, b: &Block, key: &str, cx: &mut Context<Self>) -> AnyElement {
        match b {
            Block::Heading { text } => div()
                .pt(px(2.0))
                .text_size(px(tk::T15))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(tk::TEXT)
                .child(text.clone())
                .into_any_element(),
            Block::Text { text } => {
                div().text_size(px(tk::T13)).text_color(tk::DIM).child(markdown(text, key)).into_any_element()
            }
            Block::Stat { value, label, trend, tone } => self.stats(
                &[crate::island::view::StatItem { value: value.clone(), label: label.clone(), trend: trend.clone(), tone: *tone }],
                key,
            ),
            Block::Stats { items } => self.stats(items, key),
            Block::Progress { label, value, tone } => div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .text_size(px(tk::T12))
                        .child(div().text_color(tk::DIM).child(label.clone()))
                        .child(div().font_family(tk::MONO).text_color(tk::FAINT).child(format!("{:.0}%", value * 100.0))),
                )
                .child(bar(*value, if *tone == Tone::Neutral { tk::BLUE } else { tone_color(*tone) }, 5.0))
                .into_any_element(),
            Block::List { items } => {
                let mut col = div().flex().flex_col().gap(px(2.0));
                for (i, it) in items.iter().enumerate() {
                    let color = if it.tone == Tone::Neutral { tk::DIM } else { tone_color(it.tone) };
                    let mut row = div()
                        .id(eid(format!("{key}-li-{i}")))
                        .h(px(40.0))
                        .px(px(8.0))
                        .rounded(px(tk::R_ROW))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(icon(it.icon.as_deref().map(icon_named).unwrap_or(icons::CHECK)).size(px(tk::ICON_ROW)).text_color(color))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(div().text_size(px(tk::T13)).text_color(tk::TEXT).truncate().child(it.title.clone()))
                                .children(it.meta.clone().map(|m| {
                                    div().text_size(px(tk::T11)).text_color(tk::FAINT).truncate().child(m)
                                })),
                        );
                    if let Some(a) = &it.action {
                        row = row
                            .cursor_pointer()
                            .hover(|s| s.bg(tk::WASH))
                            .child(icon(icons::ALT_ARROW_RIGHT).size(px(13.0)).text_color(tk::GHOST))
                            .on_click(self.action_click(a, cx));
                    }
                    col = col.child(row);
                }
                col.into_any_element()
            }
            Block::Checklist { items } => {
                let mut col = div().flex().flex_col().gap(px(4.0));
                for it in items {
                    col = col.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(if it.done {
                                icon(icons::CHECK).size(px(14.0)).text_color(tk::GREEN).into_any_element()
                            } else {
                                self.dot(if it.active { tk::BLUE } else { tk::GHOST }, 7.0, it.active).into_any_element()
                            })
                            .child(
                                div()
                                    .text_size(px(tk::T13))
                                    .text_color(if it.done { tk::FAINT } else { tk::TEXT })
                                    .when(it.done, |d| d.line_through())
                                    .child(it.text.clone()),
                            ),
                    );
                }
                col.into_any_element()
            }
            Block::Timeline { items } => {
                let mut col = div().flex().flex_col().gap(px(4.0));
                for it in items {
                    col = col.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .w(px(40.0))
                                    .flex_none()
                                    .font_family(tk::MONO)
                                    .text_size(px(tk::T11))
                                    .text_color(tk::FAINT)
                                    .child(it.time.clone()),
                            )
                            .child(self.dot(if it.tone == Tone::Neutral { tk::DIM } else { tone_color(it.tone) }, 6.0, false))
                            .child(div().flex_1().min_w_0().truncate().text_size(px(tk::T13)).text_color(tk::TEXT).child(it.text.clone())),
                    );
                }
                col.into_any_element()
            }
            Block::Diff { file, added, removed, lines } => card()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(icon(icons::FILE_DIFF).size(px(14.0)).text_color(tk::DIM))
                        .child(div().flex_1().min_w_0().truncate().font_family(tk::MONO).text_size(px(tk::T12)).text_color(tk::TEXT).child(file.clone()))
                        .when(*added > 0, |d| d.child(div().font_family(tk::MONO).text_size(px(tk::T11)).text_color(tk::GREEN).child(format!("+{added}"))))
                        .when(*removed > 0, |d| d.child(div().font_family(tk::MONO).text_size(px(tk::T11)).text_color(tk::RED).child(format!("−{removed}")))),
                )
                .children(lines.iter().map(|l| {
                    let color = match l.chars().next() {
                        Some('+') => tk::GREEN,
                        Some('-') => tk::RED,
                        _ => tk::DIM,
                    };
                    div().font_family(tk::MONO).text_size(px(tk::T11)).text_color(color).truncate().child(l.clone()).into_any_element()
                }))
                .into_any_element(),
            Block::Agent { task } => match self.snap.agents.iter().position(|a| a.id == *task) {
                Some(i) => {
                    let a = self.snap.agents[i].clone();
                    card()
                        .p(px(4.0))
                        .flex()
                        .flex_col()
                        .child(self.agent_row(&a, 900 + i, cx))
                        .when_some(a.todos.filter(|(_, t)| *t > 0), |d, (done, total)| {
                            d.child(div().px(px(10.0)).pb(px(8.0)).child(bar(done as f32 / total as f32, tk::BLUE, 3.0)))
                        })
                        .into_any_element()
                }
                None => card().text_size(px(tk::T12)).text_color(tk::FAINT).child("That agent isn't around any more.").into_any_element(),
            },
            Block::Plan { task } => match self.plan_for(*task) {
                Some(p) => {
                    let task = *task;
                    card()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .child(icon(icons::LAYERS).size(px(14.0)).text_color(tk::BLUE))
                                .child(div().flex_1().text_size(px(tk::T13)).font_weight(FontWeight::MEDIUM).text_color(tk::TEXT).child(format!("{}'s plan", p.kind.label())))
                                .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).child(format!("{} steps", p.steps.len()))),
                        )
                        .children(p.steps.iter().take(4).enumerate().map(|(i, s)| {
                            div().text_size(px(tk::T12)).text_color(tk::DIM).truncate().child(format!("{}. {}", i + 1, s.text)).into_any_element()
                        }))
                        .child(
                            div().flex().gap(px(6.0)).child(
                                primary(eid(format!("{key}-review")), if p.request_id.is_some() { "Review plan" } else { "Open plan" })
                                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                        this.open_page(Page::Plan(task), Some(window), cx)
                                    })),
                            ),
                        )
                        .into_any_element()
                }
                None => match self.snap.agents.iter().find(|a| a.id == *task).filter(|a| !a.todo_list.is_empty()) {
                    Some(a) => self.block(
                        &Block::Checklist {
                            items: a
                                .todo_list
                                .iter()
                                .map(|(t, s)| crate::island::view::CheckItem {
                                    text: t.clone(),
                                    done: s == "completed",
                                    active: s == "in_progress",
                                })
                                .collect(),
                        },
                        key,
                        cx,
                    ),
                    None => div().into_any_element(),
                },
            },
            Block::Actions { items } => {
                let mut row = div().flex().flex_wrap().items_center().gap(px(6.0));
                for (i, b) in items.iter().enumerate() {
                    let id = eid(format!("{key}-a-{i}"));
                    let el = if b.primary { primary(id, b.label.clone()) } else { pill(id, b.label.clone(), false) };
                    row = row.child(el.on_click(self.action_click(&b.action, cx)));
                }
                row.into_any_element()
            }
            Block::Callout { text, tone } => {
                let color = if *tone == Tone::Neutral { tk::DIM } else { tone_color(*tone) };
                div()
                    .p(px(12.0))
                    .rounded(px(tk::R_ROW))
                    .bg(if *tone == Tone::Neutral { tk::CARD } else { color.opacity(0.12) })
                    .text_size(px(tk::T13))
                    .text_color(if *tone == Tone::Neutral { tk::DIM } else { color })
                    .child(markdown(text, key))
                    .into_any_element()
            }
            Block::Split { left, right } => div()
                .flex()
                .gap(px(12.0))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(10.0)).children(
                    left.iter().enumerate().map(|(i, b)| self.block(b, &format!("{key}-l{i}"), cx)).collect::<Vec<_>>(),
                ))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(10.0)).children(
                    right.iter().enumerate().map(|(i, b)| self.block(b, &format!("{key}-r{i}"), cx)).collect::<Vec<_>>(),
                ))
                .into_any_element(),
            Block::Section { title, blocks } => div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(section(title.clone()))
                .children(blocks.iter().enumerate().map(|(i, b)| self.block(b, &format!("{key}-s{i}"), cx)).collect::<Vec<_>>())
                .into_any_element(),
            Block::Unknown => div().into_any_element(),
        }
    }

    fn stats(&self, items: &[crate::island::view::StatItem], key: &str) -> AnyElement {
        let mut row = div().id(eid(format!("{key}-stats"))).flex().gap(px(8.0));
        for s in items {
            let color = tone_color(s.tone);
            row = row.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(60.0))
                    .px(px(12.0))
                    .rounded(px(tk::R_ROW))
                    .bg(tk::CARD)
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(px(2.0))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(6.0))
                            .child(div().text_size(px(tk::T20)).font_weight(FontWeight::SEMIBOLD).text_color(color).child(s.value.clone()))
                            .children(s.trend.clone().map(|t| div().text_size(px(tk::T11)).text_color(tk::FAINT).child(t))),
                    )
                    .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).truncate().child(s.label.clone())),
            );
        }
        row.into_any_element()
    }

    // ----- home ------------------------------------------------------------------------

    /// Home's V3 cards: the call, a plan, a pinned view, the usage meter.
    pub(crate) fn home_card_v3(&mut self, c: HomeCard, cx: &mut Context<Self>) -> AnyElement {
        match c {
            HomeCard::Call => {
                let caption = self.call.caption.clone().or(self.call.partial.clone()).unwrap_or_default();
                card()
                    .id("home-call")
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(tk::WASH))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_page(Page::Call, Some(window), cx)))
                    .child(self.orb(28.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(tk::T13)).font_weight(FontWeight::MEDIUM).text_color(tk::TEXT).child(format!(
                                "{} · {}",
                                self.call.target.clone().unwrap_or_else(|| "K22".into()),
                                self.call.phase.word()
                            )))
                            .child(div().text_size(px(tk::T11)).text_color(tk::FAINT).truncate().child(caption)),
                    )
                    .child(
                        div()
                            .id("home-call-end")
                            .h(px(28.0))
                            .px(px(12.0))
                            .rounded_full()
                            .bg(tk::RED)
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .text_size(px(tk::T12))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(tk::WHITE)
                            .child("End")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.command(ToEngine::Call { on: false, task: None });
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            }
            HomeCard::Plan => {
                let Some(p) = self.snap.plans.first().cloned() else { return div().into_any_element() };
                let task = p.task;
                card()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(avatar(p.kind, 26.0, None))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().text_size(px(tk::T13)).font_weight(FontWeight::MEDIUM).text_color(tk::TEXT).truncate().child(p.title.clone()))
                                    .child(div().text_size(px(tk::T11)).text_color(tk::BLUE).child(format!(
                                        "{} has a plan · {} steps",
                                        p.kind.label(),
                                        p.steps.len()
                                    ))),
                            ),
                    )
                    .children(p.steps.iter().take(2).enumerate().map(|(i, s)| {
                        div().text_size(px(tk::T12)).text_color(tk::DIM).truncate().child(format!("{}. {}", i + 1, s.text)).into_any_element()
                    }))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .child(primary("home-plan", "Review").on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.open_page(Page::Plan(task), Some(window), cx)
                            })))
                            .when(self.snap.plans.len() > 1, |d| {
                                let n = self.snap.plans.len() - 1;
                                d.child(pill("home-plans", format!("{n} more"), false).on_click(cx.listener(
                                    |this, _: &ClickEvent, window, cx| this.open_page(Page::Lens(Lens::Needs), Some(window), cx),
                                )))
                            }),
                    )
                    .into_any_element()
            }
            HomeCard::Widget => {
                let Some(w) = self.widgets.first().cloned() else { return div().into_any_element() };
                let id = w.id;
                let mut c = card().id("home-widget").flex().flex_col().gap(px(8.0)).child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(icon(icons::ARTIFACTS).size(px(14.0)).text_color(tk::DIM))
                        .child(div().flex_1().truncate().text_size(px(tk::T13)).font_weight(FontWeight::MEDIUM).text_color(tk::TEXT).child(w.title.clone()))
                        .child(ibtn("home-widget-open", icons::ALT_ARROW_RIGHT, 24.0, tk::DIM, "Open").on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| {
                                if !this.views.iter().any(|v| v.id == id)
                                    && let Some(w) = this.widgets.iter().find(|w| w.id == id).cloned()
                                {
                                    this.views.insert(0, w);
                                }
                                this.open_page(Page::View(id), Some(window), cx)
                            },
                        ))),
                );
                for (i, b) in w.blocks.iter().filter(|b| !matches!(b, Block::Heading { .. } | Block::Text { .. })).take(2).enumerate() {
                    c = c.child(self.block(b, &format!("w{id}-{i}"), cx));
                }
                c.into_any_element()
            }
            HomeCard::Usage => {
                let Some(m) = self.snap.limits.iter().filter(|m| m.used >= 80.0).max_by(|a, b| a.used.total_cmp(&b.used)).cloned()
                else {
                    return div().into_any_element();
                };
                let reset = m
                    .resets_at
                    .map(|s| format!(" · resets {}", until(s * 1000)))
                    .unwrap_or_default();
                card()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(mark(m.kind, 13.0))
                            .child(div().flex_1().text_size(px(tk::T12)).text_color(tk::DIM).child(format!("{} · {}{reset}", m.kind.label(), m.label)))
                            .child(div().font_family(tk::MONO).text_size(px(tk::T12)).text_color(if m.used >= 95.0 { tk::RED } else { tk::AMBER }).child(format!("{:.0}%", m.used))),
                    )
                    .child(bar((m.used / 100.0) as f32, if m.used >= 95.0 { tk::RED } else { tk::AMBER }, 4.0))
                    .into_any_element()
            }
            _ => div().into_any_element(),
        }
    }

    /// Height of a V3 home card.
    pub(crate) fn home_card_v3_height(&self, c: HomeCard) -> f32 {
        match c {
            HomeCard::Call => 56.0,
            HomeCard::Plan => 24.0 + 30.0 + self.snap.plans.first().map_or(0.0, |p| p.steps.len().min(2) as f32 * 18.0) + 8.0 + tk::CONTROL_H + 8.0,
            HomeCard::Widget => 24.0 + 32.0 + self.widgets.first().map_or(0.0, |w| {
                blocks_h(&w.blocks.iter().filter(|b| !matches!(b, Block::Heading { .. } | Block::Text { .. })).take(2).cloned().collect::<Vec<_>>())
            }),
            HomeCard::Usage => 52.0,
            _ => 0.0,
        }
    }

    /// The top automations as chips (Home's quick runs).
    pub(crate) fn automation_chips(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let autos: Vec<_> = self.snap.automations.iter().filter(|a| a.enabled).take(3).cloned().collect();
        if autos.is_empty() {
            return None;
        }
        let mut row = div().flex().flex_wrap().items_center().gap(px(6.0)).child(section("Run"));
        for (i, a) in autos.into_iter().enumerate() {
            let id = a.id.clone();
            row = row.child(
                pill(("auto-chip", i), a.name.clone(), false)
                    .h(px(24.0))
                    .px(px(10.0))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.command(ToEngine::RunAutomation { id: id.clone() });
                        this.toast("Running");
                        cx.notify();
                    })),
            );
        }
        Some(row.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_map_to_lenses_without_a_model() {
        assert_eq!(Lens::for_question("What's running?"), Some(Lens::Status));
        assert_eq!(Lens::for_question("what needs me"), Some(Lens::Needs));
        assert_eq!(Lens::for_question("Give me today's summary"), Some(Lens::Today));
        assert_eq!(Lens::for_question("refactor the parser"), None);
        for l in [Lens::Status, Lens::Needs, Lens::Today, Lens::Agent(TaskId::new())] {
            assert_eq!(Lens::from_id(&l.id()), Some(l));
        }
    }

    #[test]
    fn icons_resolve_by_name() {
        assert_eq!(icon_named("check"), icons::CHECK);
        assert_eq!(icon_named("git merge"), icons::GIT_MERGE);
        assert_eq!(icon_named("no-such-icon"), icons::CHECK);
    }

    #[test]
    fn a_plan_edit_knows_when_it_changed() {
        let steps = vec![PlanStep { text: "a".into(), detail: vec![] }, PlanStep { text: "b".into(), detail: vec![] }];
        let mut e = PlanEdit {
            task: TaskId::new(),
            request_id: Some("r".into()),
            source: "1. a\n2. b".into(),
            steps: steps.clone(),
            original: steps,
            mode: PlanMode::View,
            selected: None,
            open: HashSet::new(),
            answered: false,
        };
        assert!(!e.dirty());
        e.steps.swap(0, 1);
        assert!(e.dirty());
    }
}
