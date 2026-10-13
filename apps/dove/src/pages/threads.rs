//! All threads: search, filters (project, agent, status), sort, and bulk
//! actions (archive, delete, move to a Kanban column).

use dove_core::proto::{TaskId, TaskStatus};
use dove_core::store::Task;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, anchored, deferred, div, px,
};
use gpui_base::input::Input;

use super::{ThreadSort, page};
use crate::assets::{icon, icons};
use crate::kit;
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
            ThreadSort::Recent => v.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
            ThreadSort::Attention => attention_sort(&mut v),
            ThreadSort::Title => v.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase())),
        }
        v
    }

    fn bulk(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<TaskId> = self.pages.selected.drain().collect();
        for id in ids {
            match action {
                "archive" => self.set_archived(id, true, window, cx),
                "delete" => self.delete_task(id, window, cx),
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

    fn filter_chip(
        &self,
        id: &'static str,
        label: String,
        active: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.pages.threads_menu == Some(id);
        let chip = ui::chip(id, t)
            .border_1()
            .border_color(if active { t.accent.opacity(0.5) } else { t.border })
            .when(active, |d| d.text_color(t.text))
            .child(label)
            .child(icon(icons::ALT_ARROW_DOWN).size(px(11.0)).text_color(t.text_faint))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.pages.threads_menu = if this.pages.threads_menu == Some(id) { None } else { Some(id) };
                cx.notify();
            }));
        let menu = open.then(|| {
            let mut rows = div().w(px(220.0)).max_h(px(320.0)).flex().flex_col();
            let row =
                |key: String, label: String, selected: bool, f: Box<dyn Fn(&mut Shell)>, cx: &mut Context<Self>| {
                    ui::menu_row(
                        SharedString::from(key),
                        if selected { icons::CHECK } else { icons::LIST },
                        label,
                        false,
                        t,
                    )
                    .when(!selected, |d| d.opacity(0.8))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        f(this);
                        this.pages.threads_menu = None;
                        cx.notify();
                    }))
                };
            match id {
                "f-project" => {
                    rows = rows.child(row(
                        "fp-all".into(),
                        "All projects".into(),
                        self.pages.threads_project.is_none(),
                        Box::new(|s| s.pages.threads_project = None),
                        cx,
                    ));
                    for p in &self.projects {
                        let pid = p.id;
                        rows = rows.child(row(
                            format!("fp-{pid}"),
                            p.name.clone(),
                            self.pages.threads_project == Some(pid),
                            Box::new(move |s| s.pages.threads_project = Some(pid)),
                            cx,
                        ));
                    }
                }
                "f-agent" => {
                    rows = rows.child(row(
                        "fa-all".into(),
                        "All agents".into(),
                        self.pages.threads_agent.is_none(),
                        Box::new(|s| s.pages.threads_agent = None),
                        cx,
                    ));
                    let mut kinds: Vec<_> = self.tasks.iter().map(|t| t.harness).collect();
                    kinds.sort_by_key(|k| k.label());
                    kinds.dedup();
                    for k in kinds {
                        rows = rows.child(row(
                            format!("fa-{}", k.id()),
                            k.label().into(),
                            self.pages.threads_agent == Some(k),
                            Box::new(move |s| s.pages.threads_agent = Some(k)),
                            cx,
                        ));
                    }
                }
                "f-status" => {
                    rows = rows.child(row(
                        "fs-all".into(),
                        "Any status".into(),
                        self.pages.threads_status.is_none(),
                        Box::new(|s| s.pages.threads_status = None),
                        cx,
                    ));
                    for st in [
                        TaskStatus::NeedsInput,
                        TaskStatus::Running,
                        TaskStatus::Done,
                        TaskStatus::Failed,
                        TaskStatus::Idle,
                        TaskStatus::Draft,
                    ] {
                        rows = rows.child(row(
                            format!("fs-{}", st.id()),
                            status_name(st).into(),
                            self.pages.threads_status == Some(st),
                            Box::new(move |s| s.pages.threads_status = Some(st)),
                            cx,
                        ));
                    }
                }
                _ => {
                    for (key, label, sort) in [
                        ("so-att", "Attention", ThreadSort::Attention),
                        ("so-recent", "Most recent", ThreadSort::Recent),
                        ("so-title", "Title", ThreadSort::Title),
                    ] {
                        rows = rows.child(row(
                            key.into(),
                            label.into(),
                            self.pages.threads_sort == sort,
                            Box::new(move |s| s.pages.threads_sort = sort),
                            cx,
                        ));
                    }
                }
            }
            deferred(
                anchored().anchor(Anchor::TopLeft).snap_to_window_with_margin(px(8.0)).child(
                    div()
                        .mt(px(30.0))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.pages.threads_menu = None;
                            cx.notify();
                        }))
                        .child(ui::menu_card(t, rows)),
                ),
            )
            .with_priority(2)
        });
        div().relative().child(chip).children(menu).into_any_element()
    }

    pub fn render_threads_page(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let threads = self.filtered_threads();
        let p = &self.pages;
        let project_label = p
            .threads_project
            .and_then(|id| self.project(id))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Project".into());
        let agent_label = p.threads_agent.map(|k| k.label().to_string()).unwrap_or_else(|| "Agent".into());
        let status_label = p.threads_status.map(|s| status_name(s).to_string()).unwrap_or_else(|| "Status".into());
        let sort_label = match p.threads_sort {
            ThreadSort::Attention => "Sort: attention",
            ThreadSort::Recent => "Sort: recent",
            ThreadSort::Title => "Sort: title",
        };
        let (has_project, has_agent, has_status) =
            (p.threads_project.is_some(), p.threads_agent.is_some(), p.threads_status.is_some());
        let selected_n = p.selected.len();
        let all_selected = !threads.is_empty() && threads.iter().all(|x| p.selected.contains(&x.id));

        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .flex_wrap()
            .child(
                div()
                    .flex_1()
                    .min_w(px(220.0))
                    .h(px(32.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .rounded(px(8.0))
                    .bg(t.wash(0.05))
                    .text_size(px(13.0))
                    .child(icon(icons::MAGNIFER).size(px(14.0)).text_color(t.text_faint))
                    .child(div().flex_1().child(Input::new(&self.pages.threads_search))),
            )
            .child(self.filter_chip("f-project", project_label, has_project, &t, cx))
            .child(self.filter_chip("f-agent", agent_label, has_agent, &t, cx))
            .child(self.filter_chip("f-status", status_label, has_status, &t, cx))
            .child(self.filter_chip("f-sort", sort_label.into(), false, &t, cx));

        let bulk_bar = (selected_n > 0).then(|| {
            div()
                .h(px(40.0))
                .px(px(12.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .rounded(px(10.0))
                .bg(t.accent.opacity(0.08))
                .border_1()
                .border_color(t.accent.opacity(0.3))
                .text_size(px(13.0))
                .child(format!("{selected_n} selected"))
                .child(div().flex_1())
                .child(
                    ui::button_ghost("bulk-pin", "Pin", &t)
                        .on_click(cx.listener(|this, _, window, cx| this.bulk("pin", window, cx))),
                )
                .child(
                    ui::button_ghost("bulk-draft", "Move to Draft", &t)
                        .tooltip(ui::tip("Only threads that never started"))
                        .on_click(cx.listener(|this, _, window, cx| this.bulk("draft", window, cx))),
                )
                .child(
                    ui::button_ghost("bulk-archive", "Archive", &t)
                        .on_click(cx.listener(|this, _, window, cx| this.bulk("archive", window, cx))),
                )
                .child(
                    ui::button_ghost("bulk-delete", "Delete", &t)
                        .text_color(t.danger)
                        .on_click(cx.listener(|this, _, window, cx| this.bulk("delete", window, cx))),
                )
                .child(ui::button_ghost("bulk-clear", "Clear", &t).on_click(cx.listener(|this, _, _, cx| {
                    this.pages.selected.clear();
                    cx.notify();
                })))
        });

        let header_row = div()
            .h(px(30.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .text_size(px(11.5))
            .text_color(t.text_faint)
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
            .child(div().w(px(16.0)))
            .child(div().flex_1().child("Thread"))
            .child(div().w(px(150.0)).child("Project"))
            .child(div().w(px(110.0)).child("Status"))
            .child(div().w(px(70.0)).text_right().child("Updated"));

        let rows = div()
            .flex()
            .flex_col()
            .rounded(px(12.0))
            .border_1()
            .border_color(t.border)
            .overflow_hidden()
            .child(header_row)
            .when(threads.is_empty(), |d| {
                d.child(kit::empty_state(icons::LIST, "No threads", "Nothing matches these filters.", &t))
            })
            .children(threads.iter().map(|task| {
                let id = task.id;
                let sel = self.pages.selected.contains(&id);
                let project = self.project(task.project_id).map(|p| p.name.clone()).unwrap_or_default();
                div()
                    .id(SharedString::from(format!("tp-{id}")))
                    .h(px(40.0))
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .border_t_1()
                    .border_color(t.border)
                    .cursor_pointer()
                    .when(sel, |d| d.bg(t.accent.opacity(0.06)))
                    .hover(|s| s.bg(t.wash(0.04)))
                    .on_click(cx.listener(move |this, _, window, cx| this.open_task(id, window, cx)))
                    .child(kit::checkbox(SharedString::from(format!("tp-sel-{id}")), sel, "", &t).on_click(
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if !this.pages.selected.remove(&id) {
                                this.pages.selected.insert(id);
                            }
                            cx.notify();
                        }),
                    ))
                    .child(icon(ui::harness_icon(task.harness)).size(px(15.0)).text_color(ui::harness_tint(
                        task.harness,
                        &t,
                        t.text_muted,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.5))
                                    .when(task.unread, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                                    .child(task.title.clone()),
                            )
                            .when_some(task.branch.clone(), |d, b| {
                                d.child(div().truncate().text_size(px(11.5)).text_color(t.text_faint).child(b))
                            }),
                    )
                    .child(
                        div()
                            .w(px(150.0))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .text_size(px(12.5))
                            .text_color(t.text_muted)
                            .child(project_badge(&project, 14.0, &t))
                            .child(div().truncate().child(project)),
                    )
                    .child(
                        div()
                            .w(px(110.0))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .text_size(px(12.5))
                            .text_color(ui::status_color(task.status, &t))
                            .child(kit::status_glyph(
                                SharedString::from(format!("tp-st-{id}")),
                                task.status,
                                task.unread,
                                &t,
                            ))
                            .child(status_name(task.status)),
                    )
                    .child(
                        div()
                            .w(px(70.0))
                            .text_right()
                            .text_size(px(12.0))
                            .text_color(t.text_faint)
                            .child(ui::relative_time(task.updated_at)),
                    )
            }));

        let count = div()
            .text_size(px(14.0))
            .text_color(t.text_faint)
            .child(format!("{} threads", threads.len()))
            .into_any_element();
        page(
            "threads-page",
            "Threads",
            &t,
            Some(count),
            div().flex().flex_col().gap(px(12.0)).child(toolbar).children(bulk_bar).child(rows),
        )
    }
}
