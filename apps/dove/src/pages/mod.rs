//! Full pages routed from the sidebar: Artifacts, and the More pages
//! (Kanban, Automations, Agents, Usage, Archived, Help). They render as `Shell`
//! methods over the shell's state; `PageState` holds their own inputs,
//! filters and the confirmation dialog they share. Every page follows the
//! kit's page grammar (`kit::page`): a centred column, a 20/26 headline and
//! subtitle, labelled sections of filled cards, 40 px table rows, filters on
//! the kit Select, and one shape for empty and error states.

mod agents;
mod archived;
pub mod artifacts;
mod automations;
mod help;
pub mod integrations;
mod kanban;
pub mod machines;
pub mod memory_settings;
mod race;
pub mod skills;
mod usage;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use dove_core::Engine;
use dove_core::proto::{HarnessKind, ProjectId, TaskId};
use dove_core::store::{Automation, AutomationRun, UsageRow};
use gpui::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_base::input::{InputState, TextareaState};

use crate::kit::controls::{self as ctl, SelectState, Tone};
use crate::kit::{page, popover};
use crate::shell::Shell;
use crate::terminal::TerminalView;
use crate::theme::Theme;
use crate::ui;

pub use agents::agent_enabled;
pub use automations::TriggerKind;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AutoTab {
    List,
    Inbox,
}

/// The automation being created or edited.
pub struct AutoEditor {
    pub id: Option<String>,
    pub name: Entity<InputState>,
    pub prompt: Entity<TextareaState>,
    pub project: Option<ProjectId>,
    pub harness: HarnessKind,
    /// The teammate each run starts as.
    pub persona: Option<String>,
    pub kind: TriggerKind,
    pub minutes: Entity<InputState>,
    pub time: Entity<InputState>,
    pub weekdays: Vec<u32>,
    pub cron: Entity<InputState>,
    pub branch: Entity<InputState>,
    pub enabled: bool,
    pub error: Option<String>,
}

/// A confirmation a page is asking for, shown as a modal over the window.
#[derive(Clone, Debug, PartialEq)]
pub enum PageDialog {
    /// Archived: Delete.
    DeleteArchived(TaskId),
    /// Kanban: the draft editor's Delete.
    DeleteDraft(TaskId),
    /// Kanban: a card dropped on Done.
    ArchiveDone(TaskId),
    /// Automations: Delete.
    DeleteAutomation(String),
    /// Team: the editor's Delete.
    DeleteTeammate(String),
}

pub struct PageState {
    // Kanban
    pub draft_title: Entity<InputState>,
    pub draft_prompt: Entity<TextareaState>,
    pub adding_draft: bool,
    /// The teammate a draft is handed to.
    pub draft_persona: Option<String>,
    pub editing_draft: Option<TaskId>,
    /// The card being dragged, while a drag is live (its source dims).
    pub kanban_drag: Rc<Cell<Option<TaskId>>>,
    /// Open PR (number, url) per (project, branch).
    pub pr_by_branch: HashMap<String, (u64, String)>,
    pub diffstat: HashMap<TaskId, (u32, u32, u32)>,
    pub kanban_note: Option<String>,
    // Automations
    pub automations: Vec<Automation>,
    pub runs: Vec<AutomationRun>,
    pub auto_tab: AutoTab,
    pub editor: Option<AutoEditor>,
    /// The editor's dropdowns (held here, not in the editor, so a closing
    /// menu outlives a Save that drops the editor).
    pub auto_sel_project: SelectState,
    pub auto_sel_agent: SelectState,
    pub auto_sel_persona: SelectState,
    pub auto_sel_trigger: SelectState,
    // Usage
    pub usage_rows: Arc<Vec<UsageRow>>,
    pub usage_days: u32,
    pub usage_note: Option<String>,
    /// Claude and Codex plan limits, once read (`dove_core::quota`).
    pub quota: Option<Vec<dove_core::quota::Quota>>,
    pub quota_loading: bool,
    /// Bumped by each read, so only the newest schedules the next one.
    quota_round: u64,
    // Agents
    pub agent_terminal: Option<(HarnessKind, Entity<TerminalView>)>,
    // Artifacts
    pub artifact_kind: Option<artifacts::ArtKind>,
    pub artifact_open: Option<std::path::PathBuf>,
    // Archived
    pub archived_q: String,
    pub archived_search: Entity<InputState>,
    // Team
    pub team_focus: Option<String>,
    /// Teammates whose threads the sidebar shows, and those showing all of
    /// them rather than the latest few.
    pub team_expanded: HashSet<String>,
    pub team_all: HashSet<String>,
    pub team_edit: Option<crate::team::TeamEditor>,
    /// The focused teammate's notes, read when focus changes.
    pub team_notes: Option<String>,
    /// In-app editor for the focused teammate's NOTES.md.
    pub team_notes_edit: Option<Entity<TextareaState>>,
    /// Quick-dispatch prompt bar on the teammate detail page.
    pub team_prompt: Entity<InputState>,
    // Confirmation dialog
    pub dialog: Option<PageDialog>,
    /// Holds focus while a dialog is up, so Enter and Escape reach it.
    pub dialog_focus: FocusHandle,
    /// Where focus was when the dialog opened; it goes back there.
    pub dialog_return: Option<FocusHandle>,
}

pub(crate) fn input(placeholder: &'static str, window: &mut Window, cx: &mut Context<Shell>) -> Entity<InputState> {
    let style = ui::input_style(Theme::of(cx));
    cx.new(|cx| {
        let mut s = InputState::new(window, cx).placeholder(placeholder);
        s.set_editor_style(style);
        s
    })
}

pub(crate) fn textarea(
    placeholder: &'static str,
    rows: (usize, usize),
    window: &mut Window,
    cx: &mut Context<Shell>,
) -> Entity<TextareaState> {
    let style = ui::input_style(Theme::of(cx));
    cx.new(|cx| {
        let mut s = TextareaState::new(window, cx).auto_grow(rows.0, rows.1).placeholder(placeholder);
        s.set_editor_style(style);
        s
    })
}

impl PageState {
    pub fn new(_engine: &Arc<Engine>, window: &mut Window, cx: &mut Context<Shell>) -> Self {
        let archived_search = input("Search archived threads", window, cx);
        let subs_archived = cx.subscribe(&archived_search, |this, s, ev: &gpui_base::input::InputEvent, cx| {
            if matches!(ev, gpui_base::input::InputEvent::Change) {
                this.pages.archived_q = s.read(cx).value().to_lowercase();
                cx.notify();
            }
        });
        subs_archived.detach();
        Self {
            draft_title: input("Draft title", window, cx),
            draft_prompt: textarea("What should the agent do?", (3, 10), window, cx),
            adding_draft: false,
            draft_persona: None,
            editing_draft: None,
            kanban_drag: Rc::default(),
            pr_by_branch: HashMap::new(),
            diffstat: HashMap::new(),
            kanban_note: None,
            automations: Vec::new(),
            runs: Vec::new(),
            auto_tab: AutoTab::List,
            editor: None,
            auto_sel_project: SelectState::default(),
            auto_sel_agent: SelectState::default(),
            auto_sel_persona: SelectState::default(),
            auto_sel_trigger: SelectState::default(),
            usage_rows: Arc::default(),
            usage_days: 30,
            usage_note: None,
            quota: None,
            quota_loading: false,
            quota_round: 0,
            agent_terminal: None,
            artifact_kind: None,
            artifact_open: None,
            archived_q: String::new(),
            archived_search,
            team_focus: None,
            team_expanded: HashSet::new(),
            team_all: HashSet::new(),
            team_edit: None,
            team_notes: None,
            team_notes_edit: None,
            team_prompt: input("Dispatch a task to this teammate… (Enter to run)", window, cx),
            dialog: None,
            dialog_focus: cx.focus_handle(),
            dialog_return: None,
        }
    }

    pub fn refresh_automations(&mut self, engine: &Engine) {
        self.automations = engine.store.automations().unwrap_or_default();
        self.runs = engine.store.runs(1000).unwrap_or_default();
    }

    pub fn refresh_usage(&mut self, engine: &Arc<Engine>, cx: &mut Context<Shell>) {
        let since = dove_core::store::now_ms() - self.usage_days as i64 * 86_400_000;
        let engine = engine.clone();
        let load = cx.background_spawn(async move { engine.store.usage_rows(since).unwrap_or_default() });
        cx.spawn(async move |this, cx| {
            let rows = load.await;
            let _ = this.update(cx, |s, cx| {
                s.pages.usage_rows = Arc::new(rows);
                cx.notify();
            });
        })
        .detach();
        self.refresh_quota(cx);
    }

    /// Read the plan limits off the UI thread (a cached answer comes back at
    /// once), and again each time it goes stale while Usage stays open.
    pub fn refresh_quota(&mut self, cx: &mut Context<Shell>) {
        use dove_core::quota;
        if let Some(q) = quota::cached(quota::MAX_AGE) {
            self.quota = Some(q);
        }
        if self.quota_loading {
            return;
        }
        self.quota_loading = true;
        self.quota_round += 1;
        let round = self.quota_round;
        let load = cx.background_spawn(async { quota::get() });
        cx.spawn(async move |this, cx| {
            let q = load.await;
            let _ = this.update(cx, |s, cx| {
                s.pages.quota = Some(q);
                s.pages.quota_loading = false;
                cx.notify();
            });
            cx.background_executor().timer(quota::MAX_AGE).await;
            let _ = this.update(cx, |s, cx| {
                if s.route == crate::shell::Route::Usage && s.pages.quota_round == round {
                    s.pages.refresh_quota(cx);
                }
            });
        })
        .detach();
    }
}

/// A scrolling page on the kit grammar: the column at the wide measure, the
/// headline with an optional trailing control, the subtitle, then the body.
pub fn page_with(
    id: &'static str,
    title: &str,
    subtitle: &str,
    t: &Theme,
    header_right: Option<AnyElement>,
    body: impl IntoElement,
) -> AnyElement {
    page_at(id, page::PAGE_WIDE, title, subtitle, t, header_right, body)
}

/// [`page_with`] at an explicit measure (the 760 px reading column, or
/// wider where a table or board needs it). The column enters with the
/// standard fade and rise.
pub fn page_at(
    id: &'static str,
    max: f32,
    title: &str,
    subtitle: &str,
    t: &Theme,
    header_right: Option<AnyElement>,
    body: impl IntoElement,
) -> AnyElement {
    page::page_scroll(id)
        .child(crate::motion::fade_in(
            SharedString::from(format!("{id}-in")),
            page::page_column_max(max)
                .child(page::page_head(t, title.to_string(), subtitle.to_string(), header_right))
                .child(div().mt(px(24.0)).flex().flex_col().child(body)),
        ))
        .into_any_element()
}

/// A Select's options from labels.
pub(crate) fn options<I, S>(labels: I) -> Vec<ctl::SelectOption>
where
    I: IntoIterator<Item = S>,
    S: Into<SharedString>,
{
    labels.into_iter().map(ctl::SelectOption::new).collect()
}

impl Shell {
    /// Ask for a confirmation: the dialog takes focus (Enter confirms,
    /// Escape cancels) and hands it back when it closes.
    pub fn open_page_dialog(&mut self, dialog: PageDialog, window: &mut Window, cx: &mut Context<Self>) {
        self.pages.dialog_return = window.focused(cx);
        self.pages.dialog = Some(dialog);
        window.focus(&self.pages.dialog_focus, cx);
        cx.notify();
    }

    pub fn close_page_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pages.dialog = None;
        if let Some(back) = self.pages.dialog_return.take() {
            window.focus(&back, cx);
        }
        cx.notify();
    }

    fn confirm_page_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.pages.dialog.clone() else { return };
        self.close_page_dialog(window, cx);
        match dialog {
            PageDialog::DeleteArchived(id) => self.delete_task(id, window, cx),
            PageDialog::DeleteDraft(id) => {
                self.pages.adding_draft = false;
                self.pages.editing_draft = None;
                self.delete_task(id, window, cx);
            }
            PageDialog::ArchiveDone(id) => self.set_archived(id, true, window, cx),
            PageDialog::DeleteAutomation(id) => {
                let _ = self.engine.store.delete_automation(&id);
                self.pages.refresh_automations(&self.engine);
                cx.notify();
            }
            PageDialog::DeleteTeammate(id) => self.delete_teammate(&id, cx),
        }
    }

    /// What the open dialog says: title, message, confirm label, and
    /// whether confirming destroys something.
    fn page_dialog_text(&self, dialog: &PageDialog) -> (String, String, &'static str, bool) {
        let title_of = |id: TaskId| {
            self.task(id)
                .or_else(|| self.archived.iter().find(|t| t.id == id))
                .map(|t| t.title.clone())
                .unwrap_or_else(|| "this thread".into())
        };
        match dialog {
            PageDialog::DeleteArchived(id) => (
                format!("Delete \"{}\"?", title_of(*id)),
                "Its transcript and worktree are removed for good. This cannot be undone.".into(),
                "Delete",
                true,
            ),
            PageDialog::DeleteDraft(id) => (
                format!("Delete the draft \"{}\"?", title_of(*id)),
                "Its saved prompt is removed for good.".into(),
                "Delete",
                true,
            ),
            PageDialog::ArchiveDone(id) => (
                format!("Archive \"{}\"?", title_of(*id)),
                "Its worktree stays until you delete the thread.".into(),
                "Archive",
                false,
            ),
            PageDialog::DeleteAutomation(id) => (
                format!(
                    "Delete \"{}\"?",
                    self.pages
                        .automations
                        .iter()
                        .find(|a| &a.id == id)
                        .map(|a| a.name.clone())
                        .unwrap_or_else(|| "this automation".into())
                ),
                "It stops running. Threads it already started stay, and so do their Inbox entries.".into(),
                "Delete",
                true,
            ),
            PageDialog::DeleteTeammate(id) => (
                format!(
                    "Delete {}?",
                    self.persona(id).map(|p| p.name.clone()).unwrap_or_else(|| "this teammate".into())
                ),
                "Its brief and NOTES.md memory are deleted for good. Its threads stay.".into(),
                "Delete for good",
                true,
            ),
        }
    }

    /// The open confirmation, as a modal over the whole window.
    pub fn render_page_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.pages.dialog.clone()?;
        let t = Theme::of(cx).clone();
        let (title, message, confirm, danger) = self.page_dialog_text(&dialog);
        let card = popover::dialog_card(&t, 420.0)
            .id("page-dialog")
            .role(gpui::Role::Dialog)
            .aria_label(SharedString::from(title.clone()))
            .track_focus(&self.pages.dialog_focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| match e.keystroke.key.as_str() {
                "escape" => {
                    this.close_page_dialog(window, cx);
                    cx.stop_propagation();
                }
                "enter" => {
                    this.confirm_page_dialog(window, cx);
                    cx.stop_propagation();
                }
                _ => {}
            }))
            .child(popover::dialog_body(&t, title, message))
            .child(
                popover::dialog_actions()
                    .child(
                        ctl::button(&t, "page-dialog-cancel", "Cancel", Tone::Outlined)
                            .on_click(cx.listener(|this, _, window, cx| this.close_page_dialog(window, cx))),
                    )
                    .child(
                        ctl::button(
                            &t,
                            "page-dialog-confirm",
                            confirm,
                            if danger { Tone::Danger } else { Tone::Solid },
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.confirm_page_dialog(window, cx))),
                    ),
            );
        let weak = cx.weak_entity();
        Some(popover::modal(
            "page-dialog-layer",
            window.viewport_size(),
            t.scrim,
            None,
            card.into_any_element(),
            Some(Box::new(move |window, cx| {
                let _ = weak.update(cx, |this, cx| this.close_page_dialog(window, cx));
            })),
        ))
    }
}
