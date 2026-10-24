//! `/orchestrator`: whether a thread may start other agents, and the cards
//! the user approves them with.
//!
//! Spawning is off until the user types `/orchestrator` in a thread (most
//! people have one or two agents; most threads don't need a crew). Then each
//! agent the thread wants to start appears as a card above its input, where
//! the user can change the task, the agent, the model, the effort, the
//! permission mode and whether it gets its own worktree, and Approve or Skip
//! it. "Approve all" switches the thread to Auto: later spawns start at once.
//! The mode belongs to the thread that started a tree of agents, so its
//! children follow it.

use std::sync::Arc;

use dove_core::Engine;
use dove_core::proto::{HarnessKind, Model, PermissionMode, ReasoningLevel, TaskId};
use dove_core::store::{LinkKind, Persona, Store};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_base::input::{Input, InputState, Textarea, TextareaState};

use crate::assets::{icon, icons};
use crate::design::ty;
use crate::kit::controls::{self as ctl, Segment, SelectOption, SelectState, Tone};
use crate::theme::Theme;

/// Whether a thread may start other agents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orchestrate {
    /// It works alone (the default).
    #[default]
    Off,
    /// It proposes agents; the user approves each one.
    Ask,
    /// Its agents start at once.
    Auto,
}

impl Orchestrate {
    pub fn id(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Ask => "ask",
            Self::Auto => "auto",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "ask" | "on" => Self::Ask,
            "auto" => Self::Auto,
            _ => Self::Off,
        }
    }

    /// The next setting when the composer's chip is clicked.
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Ask,
            Self::Ask => Self::Auto,
            Self::Auto => Self::Off,
        }
    }
}

fn key(task: Option<TaskId>) -> String {
    match task {
        Some(id) => format!("orchestrator.{id}"),
        None => "orchestrator.home".into(),
    }
}

/// A thread's setting (`None`: the new-thread composer's).
pub fn get(store: &Store, task: Option<TaskId>) -> Orchestrate {
    store.setting(&key(task)).map(|s| Orchestrate::from_id(&s)).unwrap_or_default()
}

pub fn set(store: &Store, task: Option<TaskId>, o: Orchestrate) {
    let _ = store.set_setting(&key(task), o.id());
}

/// What `/orchestrator` typed as a message means: `Some` when the text is
/// the command (with `off`, `auto` or `ask` after it, or nothing: on).
pub fn command(text: &str) -> Option<Orchestrate> {
    let t = text.trim();
    let rest = t.strip_prefix("/orchestrator")?;
    if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
        return None;
    }
    Some(match rest.trim().to_ascii_lowercase().as_str() {
        "off" | "stop" => Orchestrate::Off,
        "auto" => Orchestrate::Auto,
        _ => Orchestrate::Ask,
    })
}

/// What the agent hears when spawning is off.
pub const OFF: &str = "Starting other agents is off in this thread. Do the work yourself. If several agents in \
parallel (or another model's review) would really help, say so in one sentence and suggest the user types \
/orchestrator: they then approve each agent and can change its task, agent, model and effort.";

/// One spawn waiting for the user.
pub struct Proposal {
    pub id: u64,
    /// The thread that asked.
    pub parent: TaskId,
    pub kind: LinkKind,
    pub title: String,
    pub teammate: Option<Persona>,
    /// What the user may grant at most: the parent's permission mode.
    pub ceiling: PermissionMode,
    pub request_key: Option<String>,
    /// A message's sender wants the reply back.
    pub wants_reply: bool,
}

/// The user's decision on a card.
pub enum ProposalEvent {
    Approve,
    Skip,
    /// Approve this one and switch the thread to Auto.
    ApproveAll,
}

/// A spawn as the user edited it.
pub struct Choice {
    pub task: String,
    pub agent: HarnessKind,
    pub model: Option<String>,
    pub reasoning: Option<ReasoningLevel>,
    pub mode: PermissionMode,
    pub isolate: bool,
    /// The user changed something the agent asked for.
    pub edited: bool,
}

const MODES: [PermissionMode; 4] =
    [PermissionMode::Plan, PermissionMode::Default, PermissionMode::AcceptEdits, PermissionMode::Bypass];

fn mode_name(m: PermissionMode) -> &'static str {
    match m {
        PermissionMode::Plan => "Plan",
        PermissionMode::Default => "Ask",
        PermissionMode::AcceptEdits => "Edits",
        PermissionMode::Bypass => "Bypass",
    }
}

pub struct ProposalCard {
    pub proposal: Proposal,
    engine: Arc<Engine>,
    agents: Vec<HarnessKind>,
    agent: HarnessKind,
    models: Vec<Model>,
    /// The chosen model's index in `models` (`None`: the agent's default).
    model: Option<usize>,
    reasoning: Option<ReasoningLevel>,
    mode: PermissionMode,
    isolate: bool,
    task: Entity<TextareaState>,
    title: Entity<InputState>,
    /// What the agent asked for, to tell whether the user changed it.
    asked: (String, HarnessKind, Option<String>, Option<ReasoningLevel>, PermissionMode, bool),
    editing: bool,
    pub error: Option<SharedString>,
    sel_agent: SelectState,
    sel_model: SelectState,
}

impl EventEmitter<ProposalEvent> for ProposalCard {}

impl ProposalCard {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        proposal: Proposal,
        engine: Arc<Engine>,
        agents: Vec<HarnessKind>,
        task: String,
        pick: (HarnessKind, Option<String>),
        reasoning: Option<ReasoningLevel>,
        mode: PermissionMode,
        isolate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let style = crate::ui::input_style(Theme::of(cx));
        let task_text = task.clone();
        let task_input = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx).auto_grow(3, 12).placeholder("What the agent should do");
            s.set_editor_style(style.clone());
            s.set_value(task_text, window, cx);
            s
        });
        let title_text = proposal.title.clone();
        let title = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Title");
            s.set_value(title_text, window, cx);
            s
        });
        let models = engine.models(pick.0);
        let model = pick.1.as_ref().and_then(|m| models.iter().position(|x| &x.id == m));
        let asked = (task, pick.0, pick.1.clone(), reasoning, mode, isolate);
        Self {
            proposal,
            engine,
            agents,
            agent: pick.0,
            models,
            model,
            reasoning,
            mode,
            isolate,
            task: task_input,
            title,
            asked,
            editing: false,
            error: None,
            sel_agent: SelectState::default(),
            sel_model: SelectState::default(),
        }
    }

    /// The spawn as it stands now.
    pub fn choice(&self, cx: &App) -> Choice {
        let task = self.task.read(cx).value().trim().to_string();
        let model = self.model.and_then(|i| self.models.get(i)).map(|m| m.id.clone());
        let edited = task != self.asked.0.trim()
            || self.agent != self.asked.1
            || model != self.asked.2
            || self.reasoning != self.asked.3
            || self.mode != self.asked.4
            || self.isolate != self.asked.5;
        Choice {
            task,
            agent: self.agent,
            model,
            reasoning: self.reasoning,
            mode: self.mode,
            isolate: self.isolate,
            edited,
        }
    }

    pub fn title(&self, cx: &App) -> String {
        let t = self.title.read(cx).value().trim().to_string();
        if t.is_empty() { self.proposal.title.clone() } else { t }
    }

    fn set_agent(&mut self, agent: HarnessKind, cx: &mut Context<Self>) {
        if agent == self.agent {
            return;
        }
        self.agent = agent;
        self.models = self.engine.models(agent);
        self.model = None;
        // Keep the effort only where the new agent offers it.
        if !self.efforts().contains(&self.reasoning.unwrap_or(ReasoningLevel::Medium)) {
            self.reasoning = None;
        }
        cx.notify();
    }

    /// The efforts the chosen model (or agent) offers.
    fn efforts(&self) -> Vec<ReasoningLevel> {
        let of_model = self.model.and_then(|i| self.models.get(i)).map(|m| m.reasoning_levels.clone());
        let levels = of_model.unwrap_or_else(|| {
            let mut all: Vec<ReasoningLevel> = Vec::new();
            for m in &self.models {
                for r in &m.reasoning_levels {
                    if !all.contains(r) {
                        all.push(*r);
                    }
                }
            }
            all
        });
        levels.into_iter().filter(|r| !matches!(r, ReasoningLevel::Ultracode | ReasoningLevel::Ultrathink)).collect()
    }

    fn summary(&self) -> String {
        let model = self
            .model
            .and_then(|i| self.models.get(i))
            .map(|m| m.label.clone())
            .unwrap_or_else(|| "default model".into());
        let mut parts = vec![self.agent.label().to_string(), model];
        if let Some(r) = self.reasoning {
            parts.push(format!("{} effort", r.label().to_lowercase()));
        }
        parts.push(format!("{} mode", mode_name(self.mode).to_lowercase()));
        if self.isolate {
            parts.push("own worktree".into());
        }
        parts.join(" · ")
    }

    fn render_form(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let label = |text: &'static str| {
            div().w(px(86.0)).flex_none().text_size(ty::CAPTION).text_color(t.text_muted).child(text)
        };
        let row = |l: &'static str, control: AnyElement| {
            div().flex().items_center().gap(px(10.0)).child(label(l)).child(div().flex_1().min_w_0().child(control))
        };
        let agents = self.agents.clone();
        let agent_ix = agents.iter().position(|k| *k == self.agent).unwrap_or(0);
        let agent = ctl::select("prop-agent", "Agent", |c: &mut ProposalCard| &mut c.sel_agent)
            .options(agents.iter().map(|k| SelectOption::new(k.label())), agent_ix)
            .width(220.0)
            .on_select(move |this, i, _, cx| {
                if let Some(k) = agents.get(i) {
                    this.set_agent(*k, cx);
                }
            })
            .render(t, &self.sel_agent, cx);
        let names: Vec<SelectOption> = std::iter::once(SelectOption::new("Default"))
            .chain(self.models.iter().map(|m| SelectOption::new(m.label.clone())))
            .collect();
        let model = ctl::select("prop-model", "Model", |c: &mut ProposalCard| &mut c.sel_model)
            .options(names, self.model.map(|i| i + 1).unwrap_or(0))
            .width(220.0)
            .menu_width(280.0)
            .on_select(|this, i, _, cx| {
                this.model = i.checked_sub(1);
                if this.reasoning.is_some_and(|r| !this.efforts().contains(&r)) {
                    this.reasoning = None;
                }
                cx.notify();
            })
            .render(t, &self.sel_model, cx);
        let efforts = self.efforts();
        let effort: AnyElement = if efforts.is_empty() {
            div()
                .text_size(ty::CAPTION)
                .text_color(t.text_faint)
                .child("This model has no effort setting")
                .into_any_element()
        } else {
            let selected =
                self.reasoning.and_then(|r| efforts.iter().position(|x| *x == r)).map(|i| i + 1).unwrap_or(0);
            let weak = cx.weak_entity();
            let list = efforts.clone();
            ctl::segmented(
                t,
                "prop-effort",
                std::iter::once(Segment::new("Default"))
                    .chain(efforts.iter().map(|r| Segment::new(r.label())))
                    .collect(),
                selected,
                move |i, _, cx| {
                    let pick = i.checked_sub(1).and_then(|i| list.get(i).copied());
                    let _ = weak.update(cx, |c, cx| {
                        c.reasoning = pick;
                        cx.notify();
                    });
                },
            )
        };
        let allowed: Vec<PermissionMode> = MODES
            .into_iter()
            .filter(|m| dove_core::orchestra::cap_mode(self.proposal.ceiling, Some(*m)) == *m)
            .collect();
        let mode = {
            let weak = cx.weak_entity();
            let list = allowed.clone();
            ctl::segmented(
                t,
                "prop-mode",
                allowed.iter().map(|m| Segment::new(mode_name(*m))).collect(),
                allowed.iter().position(|m| *m == self.mode).unwrap_or(0),
                move |i, _, cx| {
                    if let Some(m) = list.get(i).copied() {
                        let _ = weak.update(cx, |c, cx| {
                            c.mode = m;
                            cx.notify();
                        });
                    }
                },
            )
        };
        let isolate = self.isolate;
        let worktree = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(ctl::on_toggle(ctl::toggle(t, "prop-worktree", isolate), cx, move |this, _, cx| {
                this.isolate = !isolate;
                cx.notify();
            }))
            .child(div().text_size(ty::CAPTION).text_color(t.text_muted).child(if isolate {
                "Its own git worktree (its edits can't collide with others')"
            } else {
                "Works in the same folder as this thread"
            }));
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(row("Title", Input::new(&self.title).into_any_element()))
            .child(div().flex().flex_col().gap(px(4.0)).child(label("Task")).child(Textarea::new(&self.task)))
            .child(row("Agent", agent))
            .child(row("Model", model))
            .child(row("Effort", effort))
            .child(row("Permission", mode))
            .child(row("Worktree", worktree.into_any_element()))
            .into_any_element()
    }
}

impl Render for ProposalCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("ProposalCard");
        let t = Theme::of(cx).clone();
        let id = self.proposal.id as usize;
        let who = match &self.proposal.teammate {
            Some(p) => format!("{} wants to ask {}", "This thread", p.name.trim()),
            None if self.proposal.kind == LinkKind::Message => "This thread wants to message a teammate".into(),
            None => "This thread wants to start an agent".into(),
        };
        let editing = self.editing;
        let form = editing.then(|| self.render_form(&t, cx));
        let preview: String = self.task.read(cx).value().lines().take(3).collect::<Vec<_>>().join("\n");
        div()
            .id(("proposal", id))
            .occlude()
            .w_full()
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .rounded(px(14.0))
            .bg(t.composer_bg())
            .border_1()
            .border_color(t.accent.opacity(0.35))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(icon(icons::FORK).size(px(14.0)).text_color(t.accent))
                    .child(div().text_size(ty::CAPTION).text_color(t.text_muted).child(who))
                    .child(div().flex_1())
                    .child(
                        ctl::button_sm(
                            &t,
                            format!("prop-edit-{id}"),
                            if editing { "Done editing" } else { "Edit" },
                            Tone::Quiet,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.editing = !this.editing;
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .text_size(ty::BODY)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(t.text)
                    .truncate()
                    .child(self.title(cx)),
            )
            .when(!editing, |d| {
                d.child(div().text_size(ty::SMALL).text_color(t.text_muted).line_clamp(3).child(preview))
                    .child(div().text_size(ty::CAPTION).text_color(t.text_faint).child(self.summary()))
            })
            .children(form)
            .when_some(self.error.clone(), |d, e| d.child(div().text_size(ty::CAPTION).text_color(t.danger).child(e)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().flex_1())
                    .child(
                        ctl::button_sm(&t, format!("prop-skip-{id}"), "Skip", Tone::Quiet)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ProposalEvent::Skip))),
                    )
                    .child(
                        ctl::button_sm(&t, format!("prop-all-{id}"), "Approve all in this thread", Tone::Filled)
                            .tooltip(crate::ui::tip("Start this one, and start later ones without asking"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ProposalEvent::ApproveAll))),
                    )
                    .child(
                        ctl::button_sm(&t, format!("prop-go-{id}"), "Approve", Tone::Solid)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ProposalEvent::Approve))),
                    ),
            )
    }
}

/// The cards waiting in each thread, for its chat to draw above the input.
#[derive(Default)]
pub struct Waiting(pub Vec<Entity<ProposalCard>>);

impl gpui::Global for Waiting {}

/// The cards thread `id` shows.
pub fn cards_for(id: TaskId, cx: &App) -> Vec<Entity<ProposalCard>> {
    cx.try_global::<Waiting>()
        .map(|w| w.0.iter().filter(|c| c.read(cx).proposal.parent == id).cloned().collect())
        .unwrap_or_default()
}

/// The note a parent gets once the user has decided on all its cards, when
/// something differs from what it asked (an edit or a skip).
pub fn decisions_note(lines: &[String]) -> String {
    let mut out = String::from("<dove-team-update>\n\n## The user reviewed the agents you proposed\n");
    for l in lines {
        out.push_str(&format!("- {l}\n"));
    }
    out.push_str(
        "\n</dove-team-update>\n\nThis is an automatic update from Dove, not a message from the user. Started \
         agents report back when they finish; carry on (or end your turn) with that in mind.",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_reads_its_argument() {
        assert_eq!(command("/orchestrator"), Some(Orchestrate::Ask));
        assert_eq!(command("  /orchestrator auto "), Some(Orchestrate::Auto));
        assert_eq!(command("/orchestrator off"), Some(Orchestrate::Off));
        assert_eq!(command("/orchestrators"), None);
        assert_eq!(command("please /orchestrator"), None);
    }

    #[test]
    fn settings_round_trip_and_cycle() {
        for o in [Orchestrate::Off, Orchestrate::Ask, Orchestrate::Auto] {
            assert_eq!(Orchestrate::from_id(o.id()), o);
        }
        assert_eq!(Orchestrate::Off.next().next().next(), Orchestrate::Off);
        assert_eq!(Orchestrate::from_id("garbage"), Orchestrate::Off);
    }

    #[test]
    fn the_note_is_a_team_update() {
        let n = decisions_note(&["\"Tests\" was skipped".into()]);
        assert!(dove_core::orchestra::is_team_update(&n));
        assert!(matches!(
            dove_core::orchestra::parse_team_text(&n),
            Some(dove_core::orchestra::TeamText::Update { .. })
        ));
    }
}
