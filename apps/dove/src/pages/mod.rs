//! Full pages routed from the sidebar: Threads, Kanban, Automations, and the
//! More pages (Agents, Usage, Archived, Help). They render as `Shell`
//! methods over the shell's state; `PageState` holds their own inputs and
//! filters.

mod agents;
mod archived;
mod automations;
mod help;
mod kanban;
mod threads;
mod usage;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use dove_core::Engine;
use dove_core::proto::{HarnessKind, ProjectId, TaskId, TaskStatus};
use dove_core::store::{Automation, AutomationRun, UsageRow};
use gpui::{AppContext as _, Context, Entity, Window};
use gpui_base::input::{InputState, TextareaState};

use crate::shell::Shell;
use crate::terminal::TerminalView;
use crate::theme::Theme;
use crate::ui;

pub use agents::agent_enabled;
pub use automations::TriggerKind;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThreadSort {
    Recent,
    Attention,
    Title,
}

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
    pub kind: TriggerKind,
    pub minutes: Entity<InputState>,
    pub time: Entity<InputState>,
    pub weekdays: Vec<u32>,
    pub cron: Entity<InputState>,
    pub branch: Entity<InputState>,
    pub worktree: bool,
    pub enabled: bool,
    pub error: Option<String>,
}

pub struct PageState {
    // Threads
    pub threads_search: Entity<InputState>,
    pub threads_q: String,
    pub threads_project: Option<ProjectId>,
    pub threads_agent: Option<HarnessKind>,
    pub threads_status: Option<TaskStatus>,
    pub threads_sort: ThreadSort,
    pub selected: HashSet<TaskId>,
    pub threads_menu: Option<&'static str>,
    // Kanban
    pub draft_title: Entity<InputState>,
    pub draft_prompt: Entity<TextareaState>,
    pub adding_draft: bool,
    pub editing_draft: Option<TaskId>,
    pub confirm_done: Option<TaskId>,
    /// Open PR (number, url) per (project, branch).
    pub pr_by_branch: HashMap<String, (u64, String)>,
    pub diffstat: HashMap<TaskId, (u32, u32, u32)>,
    pub kanban_note: Option<String>,
    // Automations
    pub automations: Vec<Automation>,
    pub runs: Vec<AutomationRun>,
    pub auto_tab: AutoTab,
    pub editor: Option<AutoEditor>,
    pub confirm_delete_auto: Option<String>,
    // Usage
    pub usage_rows: Arc<Vec<UsageRow>>,
    pub usage_days: u32,
    pub usage_note: Option<String>,
    // Agents
    pub agent_terminal: Option<(HarnessKind, Entity<TerminalView>)>,
    pub agent_path_edit: Option<(HarnessKind, Entity<InputState>)>,
    // Archived
    pub archived_q: String,
    pub archived_search: Entity<InputState>,
}

fn input(placeholder: &'static str, window: &mut Window, cx: &mut Context<Shell>) -> Entity<InputState> {
    let style = ui::input_style(Theme::of(cx));
    cx.new(|cx| {
        let mut s = InputState::new(window, cx).placeholder(placeholder);
        s.set_editor_style(style);
        s
    })
}

fn textarea(
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
        let threads_search = input("Search all threads", window, cx);
        let archived_search = input("Search archived threads", window, cx);
        let subs_threads = cx.subscribe(&threads_search, |this, s, ev: &gpui_base::input::InputEvent, cx| {
            if matches!(ev, gpui_base::input::InputEvent::Change) {
                this.pages.threads_q = s.read(cx).value().to_lowercase();
                cx.notify();
            }
        });
        let subs_archived = cx.subscribe(&archived_search, |this, s, ev: &gpui_base::input::InputEvent, cx| {
            if matches!(ev, gpui_base::input::InputEvent::Change) {
                this.pages.archived_q = s.read(cx).value().to_lowercase();
                cx.notify();
            }
        });
        subs_threads.detach();
        subs_archived.detach();
        Self {
            threads_search,
            threads_q: String::new(),
            threads_project: None,
            threads_agent: None,
            threads_status: None,
            threads_sort: ThreadSort::Attention,
            selected: HashSet::new(),
            threads_menu: None,
            draft_title: input("Draft title", window, cx),
            draft_prompt: textarea("What should the agent do?", (3, 10), window, cx),
            adding_draft: false,
            editing_draft: None,
            confirm_done: None,
            pr_by_branch: HashMap::new(),
            diffstat: HashMap::new(),
            kanban_note: None,
            automations: Vec::new(),
            runs: Vec::new(),
            auto_tab: AutoTab::List,
            editor: None,
            confirm_delete_auto: None,
            usage_rows: Arc::default(),
            usage_days: 30,
            usage_note: None,
            agent_terminal: None,
            agent_path_edit: None,
            archived_q: String::new(),
            archived_search,
        }
    }

    pub fn refresh_automations(&mut self, engine: &Engine) {
        self.automations = engine.store.automations().unwrap_or_default();
        self.runs = engine.store.runs(200).unwrap_or_default();
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
    }
}

/// A page's centered column with a title row.
pub fn page(
    id: &'static str,
    title: &str,
    t: &Theme,
    header_right: Option<gpui::AnyElement>,
    body: impl gpui::IntoElement,
) -> gpui::AnyElement {
    use gpui::prelude::FluentBuilder as _;
    use gpui::{
        InteractiveElement as _, IntoElement as _, ParentElement as _, StatefulInteractiveElement as _, Styled as _,
        div, px,
    };
    div()
        .id(id)
        .size_full()
        .overflow_y_scroll()
        .flex()
        .justify_center()
        .child(
            div()
                .w_full()
                .max_w(px(980.0))
                .px(px(28.0))
                .pt(px(26.0))
                .pb(px(48.0))
                .flex()
                .flex_col()
                .gap(px(18.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .text_size(px(24.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(t.text)
                                .child(title.to_string()),
                        )
                        .child(div().flex_1())
                        .when_some(header_right, |d, r| d.child(r)),
                )
                .child(body),
        )
        .into_any_element()
}
