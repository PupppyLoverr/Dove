//! All threads: search, filters (project, agent, status), sort, and bulk
//! actions (pin, move to Draft, archive, delete). The table is a
//! `uniform_list`, so thousands of threads cost what a screenful does.

use std::rc::Rc;

use dove_core::proto::{HarnessKind, TaskId, TaskStatus};
use dove_core::store::Task;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, uniform_list,
};

use super::{PageDialog, ThreadSort, options};
use crate::assets::{icon, icons};
use crate::design::{space, ty};
use crate::kit::controls::{self as ctl, Tone};
use crate::kit::{self, page};
use crate::shell::Shell;
use crate::sidebar::{attention_sort, project_badge};
use crate::theme::Theme;
use crate::ui;

pub fn status_name(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Idle => "Idle",
        TaskStatus::Draft => "Draft",
        TaskStatus::Running => "Working",
        TaskStatus::NeedsInput => "Needs you",
        TaskStatus::Done => "Done",
        TaskStatus::Failed => "Failed",
        TaskStatus::Archived => "Archived",
    }
}

/// Statuses the Status filter offers, in its order.
const STATUSES: [TaskStatus; 6] = [
    TaskStatus::NeedsInput,
    TaskStatus::Running,
    TaskStatus::Done,
    TaskStatus::Failed,
    TaskStatus::Idle,
    TaskStatus::Draft,
];

/// Sorts the Sort filter offers, in its order.
const SORTS: [(ThreadSort, &str); 3] =
    [(ThreadSort::Attention, "Attention"), (ThreadSort::Recent, "Most recent"), (ThreadSort::Title, "Title")];

const COL_PROJECT: f32 = 150.0;
const COL_STATUS: f32 = 110.0;
const COL_UPDATED: f32 = 70.0;

impl Shell {
    fn filtered_threads(&self) -> Vec<Task> {
        let p = &self.pages;
        let mut v: Vec<Task> = self
            .tasks
            .iter()
            .filter(|t| {
                p.threads_q.is_empty()
                    || t.title.to_lowercase().contains(&p.threads_q)
                    || t.branch.as_deref().is_some_and(|b| b.to_lowercase().contains(&p.threads_q))
            })
            .filter(|t| p.threads_project.is_none_or(|id| t.project_id == id))
            .filter(|t| p.threads_agent.is_none_or(|k| t.harness == k))
            .filter(|t| p.threads_status.is_none_or(|s| t.status == s))
            .cloned()
            .collect();
        match p.threads_sort {
            ThreadSort::Recent => v.sort_by_key(|t| std::cmp::Reverse(t.updated_at)),
            ThreadSort::Attention => attention_sort(&mut v),
            ThreadSort::Title => v.sort_by_key(|a| a.title.to_lowercase()),
        }
        v
    }

    fn bulk(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) {
        if action == "delete" {
            let mut ids: Vec<TaskId> = self.pages.selected.iter().copied().collect();
            ids.sort_by_key(|id| self.tasks.iter().position(|t| t.id == *id));
            if !ids.is_empty() {
                self.open_page_dialog(PageDialog::DeleteThreads(ids), window, cx);
            }
            return;
        }
        let ids: Vec<TaskId> = self.pages.selected.drain().collect();
        for id in ids {
            match action {
                "archive" => self.set_archived(id, true, window, cx),
                "draft" => {
                    // Back to Draft only for threads that never started.
                    if let Some(t) = self.tasks.iter().find(|t| t.id == id).filter(|t| t.native_session_id.is_none()) {
                        let mut t = t.clone();
                        t.status = TaskStatus::Draft;
                        let _ = self.engine.store.update_task(&t);
                    }
                }
                "pin" => {
                    let _ = self.engine.store.set_pinned(id, true);
                }
                _ => {}
            }
        }
        self.reload(cx);
    }

    /// The four filters, each on the kit Select.
    fn thread_filters(&self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let p = &self.pages;
        let project_ids: Vec<_> = self.projects.iter().map(|p| p.id).collect();
        let project = select_index(p.threads_project.and_then(|id| project_ids.iter().position(|x| *x == id)));
        let projects = ctl::select("threads-f-project", "Project", |s: &mut Shell| &mut s.pages.sel_project)
            .options(
                options(
                    std::iter::once("All projects".to_string()).chain(self.projects.iter().map(|p| p.name.clone())),
                ),
                project,
            )
            .width(160.0)
            .menu_width(220.0)
            .on_select(move |this, i, _, cx| {
                this.pages.threads_project = i.checked_sub(1).and_then(|i| project_ids.get(i).copied());
                cx.notify();
            })
            .render(t, &p.sel_project, cx);

        let mut kinds: Vec<HarnessKind> = self.tasks.iter().map(|t| t.harness).collect();
        kinds.sort_by_key(|k| k.label());
        kinds.dedup();
        let agent = select_index(p.threads_agent.and_then(|k| kinds.iter().position(|x| *x == k)));
        let agent_options = std::iter::once(ctl::SelectOption::new("All agents"))
            .chain(kinds.iter().map(|k| ctl::SelectOption::new(k.label()).glyph(ui::harness_icon(*k))));
        let agents = ctl::select("threads-f-agent", "Agent", |s: &mut Shell| &mut s.pages.sel_agent)
            .options(agent_options, agent)
            .width(150.0)
            .menu_width(200.0)
            .on_select(move |this, i, _, cx| {
                this.pages.threads_agent = i.checked_sub(1).and_then(|i| kinds.get(i).copied());
                cx.notify();
            })
            .render(t, &p.sel_agent, cx);

        let status = select_index(p.threads_status.and_then(|s| STATUSES.iter().position(|x| *x == s)));
        let statuses = ctl::select("threads-f-status", "Status", |s: &mut Shell| &mut s.pages.sel_status)
            .options(options(std::iter::once("Any status").chain(STATUSES.iter().map(|s| status_name(*s)))), status)
            .width(140.0)
            .on_select(|this, i, _, cx| {
                this.pages.threads_status = i.checked_sub(1).and_then(|i| STATUSES.get(i).copied());
                cx.notify();
            })
            .render(t, &p.sel_status, cx);

        let sort = SORTS.iter().position(|(s, _)| *s == p.threads_sort).unwrap_or(0);
        let sorts = ctl::select("threads-f-sort", "Sort", |s: &mut Shell| &mut s.pages.sel_sort)
            .options(SORTS.iter().map(|(_, l)| ctl::SelectOption::new(*l).glyph(icons::SORT_VERTICAL)), sort)
            .width(150.0)
            .on_select(|this, i, _, cx| {
                if let Some((s, _)) = SORTS.get(i) {
                    this.pages.threads_sort = *s;
                }
                cx.notify();
            })
            .render(t, &p.sel_sort, cx);
        vec![projects, agents, statuses, sorts]
    }

    /// One 40 px row of the table.
    fn thread_row(&self, task: &Task, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let id = task.id;
        let sel = self.pages.selected.contains(&id);
        let project = self.project(task.project_id).map(|p| p.name.clone()).unwrap_or_default();
        page::row_hover(t, page::table_row(t, false).id(SharedString::from(format!("tp-{id}"))), sel)
            .on_click(cx.listener(move |this, _, window, cx| this.open_task(id, window, cx)))
            .child(kit::checkbox(SharedString::from(format!("tp-sel-{id}")), sel, "", t).on_click(cx.listener(
                move |this, _, _, cx| {
                    cx.stop_propagation();
                    if !this.pages.selected.remove(&id) {
                        this.pages.selected.insert(id);
                    }
                    cx.notify();
                },
            )))
            .child(
                div().w(px(16.0)).flex_none().flex().justify_center().child(match self.persona_for(id) {
                    Some(p) => crate::team::avatar(p, 16.0, t).into_any_element(),
                    None => icon(ui::harness_icon(task.harness))
                        .size(px(15.0))
                        .text_color(ui::harness_tint(task.harness, t, t.text_muted))
                        .into_any_element(),
                }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_size(ty::BODY)
                            .line_height(ty::lh::BODY)
                            .text_color(t.text)
                            .when(task.unread, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                            .child(task.title.clone()),
                    )
                    .when_some(task.branch.clone(), |d, b| {
                        d.child(div().truncate().text_size(ty::CAPTION).text_color(t.text_faint).child(b))
                    }),
            )
            .child(
                div()
                    .w(px(COL_PROJECT))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(ty::SMALL)
                    .text_color(t.text_muted)
                    .child(project_badge(&project, 14.0, t))
                    .child(div().min_w_0().truncate().child(project)),
            )
            .child(
                div()
                    .w(px(COL_STATUS))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(ty::SMALL)
                    .text_color(ui::status_color(task.status, t))
                    .child(kit::status_glyph(SharedString::from(format!("tp-st-{id}")), task.status, task.unread, t))
                    .child(status_name(task.status)),
            )
            .child(
                div()
                    .w(px(COL_UPDATED))
                    .flex_none()
                    .text_right()
                    .text_size(ty::META)
                    .text_color(t.text_faint)
                    .child(ui::relative_time(task.updated_at)),
            )
            .into_any_element()
    }

    pub fn render_threads_page(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let threads = Rc::new(self.filtered_threads());
        let selected_n = self.pages.selected.len();
        let all_selected = !threads.is_empty() && threads.iter().all(|x| self.pages.selected.contains(&x.id));

        let toolbar = div()
            .mt(px(24.0))
            .flex_none()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(space::SM))
            .child(page::search_field(&t, &self.pages.threads_search).flex_1().min_w(px(220.0)))
            .children(self.thread_filters(&t, cx));

        let bulk_bar = (selected_n > 0).then(|| {
            div()
                .mt(px(space::MD))
                .flex_none()
                .h(px(page::TABLE_ROW))
                .px(px(space::MD))
                .flex()
                .items_center()
                .gap(px(6.0))
                .rounded(px(crate::design::radius::CARD))
                .bg(t.accent.opacity(0.08))
                .text_size(ty::BODY)
                .child(div().pl(px(space::XS)).child(format!("{selected_n} selected")))
                .child(div().flex_1())
                .child(
                    ctl::button_sm(&t, "bulk-pin", "Pin", Tone::Quiet)
                        .on_click(cx.listener(|this, _, window, cx| this.bulk("pin", window, cx))),
                )
                .child(
                    ctl::button_sm(&t, "bulk-draft", "Move to Draft", Tone::Quiet)
                        .tooltip(ui::tip("Only threads that never started"))
                        .on_click(cx.listener(|this, _, window, cx| this.bulk("draft", window, cx))),
                )
                .child(
                    ctl::button_sm(&t, "bulk-archive", "Archive", Tone::Quiet)
                        .on_click(cx.listener(|this, _, window, cx| this.bulk("archive", window, cx))),
                )
                .child(
                    ctl::button_sm(&t, "bulk-delete", "Delete", Tone::Danger)
                        .on_click(cx.listener(|this, _, window, cx| this.bulk("delete", window, cx))),
                )
                .child(ctl::button_sm(&t, "bulk-clear", "Clear", Tone::Filled).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.pages.selected.clear();
                        cx.notify();
                    },
                )))
        });

        let head = page::table_head(&t)
            .child(kit::checkbox("sel-all", all_selected, "", &t).on_click({
                let ids: Vec<TaskId> = threads.iter().map(|x| x.id).collect();
                cx.listener(move |this, _, _, cx| {
                    if all_selected {
                        this.pages.selected.clear();
                    } else {
                        this.pages.selected.extend(ids.iter().copied());
                    }
                    cx.notify();
                })
            }))
            .child(div().w(px(16.0)).flex_none())
            .child(div().flex_1().child("Thread"))
            .child(div().w(px(COL_PROJECT)).flex_none().child("Project"))
            .child(div().w(px(COL_STATUS)).flex_none().child("Status"))
            .child(div().w(px(COL_UPDATED)).flex_none().text_right().child("Updated"));

        let count = threads.len();
        let body: AnyElement = if threads.is_empty() {
            page::empty_state(&t, icons::LIST, "No threads", "Nothing matches these filters.", None).into_any_element()
        } else {
            let rows = threads.clone();
            let theme = t.clone();
            uniform_list(
                "threads-list",
                count,
                cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                    range.filter_map(|ix| rows.get(ix)).map(|task| this.thread_row(task, &theme, cx)).collect()
                }),
            )
            .h(px(count as f32 * page::TABLE_ROW))
            .flex_shrink(1.0)
            .min_h_0()
            .into_any_element()
        };
        let table = page::card(&t).mt(px(space::MD)).flex_shrink(1.0).min_h_0().child(head).child(body);

        let right = div()
            .flex()
            .items_center()
            .gap(px(space::MD))
            .child(div().text_size(ty::BODY).text_color(t.text_muted).child(format!("{count} threads")))
            .child(
                ctl::button(&t, "threads-new", "New thread", Tone::Solid)
                    .on_click(cx.listener(|this, _, window, cx| this.new_session(window, cx))),
            )
            .into_any_element();
        let (root, column) = page::page_fill("threads-page", page::PAGE_WIDE);
        root.child(crate::motion::fade_in(
            "threads-in",
            column
                .child(page::page_head(
                    &t,
                    "Threads",
                    "Every conversation, filterable by project, agent and status.",
                    Some(right),
                ))
                .child(toolbar)
                .children(bulk_bar)
                .child(table),
        ))
        .into_any_element()
    }
}

/// The Select index of an optional filter whose option 0 is "All".
fn select_index(position: Option<usize>) -> usize {
    position.map_or(0, |i| i + 1)
}
