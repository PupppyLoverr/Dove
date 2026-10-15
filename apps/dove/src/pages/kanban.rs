//! Kanban: Draft · In progress · Needs you · Review · Done.
//!
//! Columns follow each thread's state (Review = an open PR exists for its
//! branch). Drafts are threads with a saved prompt that have not started;
//! dragging one to In progress starts it. Dragging to Done archives after a
//! confirmation. Agents never move cards.

use dove_core::proto::{TaskId, TaskStatus};
use dove_core::store::Task;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_base::input::{Input, Textarea};

use super::page_with;
use crate::assets::{icon, icons};
use crate::design::ty;
use crate::kit;
use crate::shell::Shell;
use crate::sidebar::project_badge;
use crate::theme::Theme;
use crate::ui;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Column {
    Draft,
    InProgress,
    NeedsYou,
    Review,
    Done,
}

impl Column {
    const ALL: [Column; 5] = [Column::Draft, Column::InProgress, Column::NeedsYou, Column::Review, Column::Done];

    fn label(self) -> &'static str {
        match self {
            Column::Draft => "Draft",
            Column::InProgress => "In progress",
            Column::NeedsYou => "Needs you",
            Column::Review => "Review",
            Column::Done => "Done",
        }
    }

    fn color(self, t: &Theme) -> gpui::Hsla {
        match self {
            Column::Draft => t.text_faint,
            Column::InProgress => t.busy,
            Column::NeedsYou => t.warning,
            Column::Review => t.accent,
            Column::Done => t.success,
        }
    }
}

#[derive(Clone)]
struct CardDrag {
    id: TaskId,
    title: SharedString,
    from: Column,
}

impl Render for CardDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("CardDrag");
        let t = Theme::of(cx);
        div()
            .w(px(220.0))
            .p(px(10.0))
            .rounded(px(10.0))
            .bg(t.surface_raised)
            .border_1()
            .border_color(t.accent.opacity(0.5))
            .shadow_lg()
            .text_size(ty::BODY)
            .child(self.title.clone())
    }
}

impl Shell {
    fn column_of(&self, task: &Task) -> Column {
        match task.status {
            TaskStatus::Draft => Column::Draft,
            TaskStatus::NeedsInput | TaskStatus::Failed => Column::NeedsYou,
            TaskStatus::Running | TaskStatus::Idle => {
                if self.has_pr(task) {
                    Column::Review
                } else {
                    Column::InProgress
                }
            }
            TaskStatus::Done | TaskStatus::Archived => {
                if self.has_pr(task) {
                    Column::Review
                } else {
                    Column::Done
                }
            }
        }
    }

    fn has_pr(&self, task: &Task) -> bool {
        task.branch.as_ref().is_some_and(|b| self.pages.pr_by_branch.contains_key(&format!("{}:{b}", task.project_id)))
    }

    /// Load open PRs per project and diffstats per worktree, off-thread.
    pub fn load_kanban_meta(&mut self, cx: &mut Context<Self>) {
        let projects = self.projects.clone();
        let tasks: Vec<(TaskId, std::path::PathBuf, String)> =
            self.tasks.iter().filter_map(|t| Some((t.id, t.worktree.clone()?, t.base.clone()?))).collect();
        let gh = dove_core::git::github::gh_available();
        let work = cx.background_spawn(async move {
            let mut prs = std::collections::HashMap::new();
            if gh {
                for p in &projects {
                    if let Ok(list) =
                        dove_core::git::github::pull_requests(&p.path, dove_core::git::github::PrFilter::All)
                    {
                        for pr in list {
                            prs.insert(format!("{}:{}", p.id, pr.branch), (pr.number, pr.url));
                        }
                    }
                }
            }
            let mut stats = std::collections::HashMap::new();
            for (id, wt, base) in tasks {
                if let Ok(s) = dove_core::git::shortstat(&wt, &base) {
                    stats.insert(id, s);
                }
            }
            (prs, stats)
        });
        cx.spawn(async move |this, cx| {
            let (prs, stats) = work.await;
            let _ = this.update(cx, |s, cx| {
                s.pages.pr_by_branch = prs;
                s.pages.diffstat = stats;
                cx.notify();
            });
        })
        .detach();
    }

    fn add_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.pages.draft_title.read(cx).value().trim().to_string();
        let prompt = self.pages.draft_prompt.read(cx).value().trim().to_string();
        if prompt.is_empty() {
            self.pages.kanban_note = Some("A draft needs a prompt".into());
            cx.notify();
            return;
        }
        let Some(project) =
            self.home_project.and_then(|id| self.project(id)).cloned().or_else(|| self.projects.first().cloned())
        else {
            self.pages.kanban_note = Some("Add a project first".into());
            cx.notify();
            return;
        };
        let persona = self.pages.draft_persona.clone().and_then(|id| self.persona(&id).cloned());
        let harness = persona.as_ref().map(|p| p.harness).unwrap_or_else(|| self.composer.read(cx).harness(cx));
        let title = if title.is_empty() { ui::title_from_prompt(&prompt) } else { title };
        match self.pages.editing_draft.take() {
            Some(id) => {
                if let Some(mut t) = self.task(id).cloned() {
                    t.title = title;
                    t.prompt = Some(prompt);
                    if let Some(p) = &persona {
                        t.harness = p.harness;
                        let _ = self.engine.store.assign_persona(id, &p.id);
                    }
                    let _ = self.engine.store.update_task(&t);
                }
            }
            None => match self.engine.create_draft(&project, &title, &prompt, harness) {
                Ok(task) => {
                    if let Some(p) = &persona {
                        let _ = self.engine.store.assign_persona(task.id, &p.id);
                    }
                }
                Err(e) => self.pages.kanban_note = Some(format!("{e:#}")),
            },
        }
        self.pages.draft_persona = None;
        self.pages.adding_draft = false;
        self.pages.draft_title.update(cx, |s, cx| s.set_value("", window, cx));
        self.pages.draft_prompt.update(cx, |s, cx| s.set_value("", window, cx));
        self.reload(cx);
    }

    fn edit_draft(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.task(id).cloned() else { return };
        self.pages.draft_persona = self.persona_of.get(&id).cloned();
        self.pages.editing_draft = Some(id);
        self.pages.adding_draft = true;
        self.pages.draft_title.update(cx, |s, cx| s.set_value(t.title.clone(), window, cx));
        self.pages.draft_prompt.update(cx, |s, cx| s.set_value(t.prompt.clone().unwrap_or_default(), window, cx));
        cx.notify();
    }

    fn drop_card(&mut self, card: &CardDrag, to: Column, window: &mut Window, cx: &mut Context<Self>) {
        match (card.from, to) {
            (a, b) if a == b => {}
            (Column::Draft, Column::InProgress) => self.start_draft(card.id, window, cx),
            (_, Column::Done) => {
                self.pages.confirm_done = Some(card.id);
                cx.notify();
            }
            _ => {
                self.pages.kanban_note =
                    Some(format!("{} follows its thread's state; agents and PRs move it", to.label()));
                cx.notify();
            }
        }
    }

    fn render_card(&self, task: &Task, col: Column, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let id = task.id;
        let project = self.project(task.project_id).map(|p| p.name.clone()).unwrap_or_default();
        let stat = self.pages.diffstat.get(&id).copied();
        let pr = task
            .branch
            .as_ref()
            .and_then(|b| self.pages.pr_by_branch.get(&format!("{}:{b}", task.project_id)))
            .cloned();
        let elapsed = ui::relative_time(task.updated_at);
        div()
            .id(SharedString::from(format!("card-{id}")))
            .p(px(10.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(t.border)
            .bg(t.surface_raised)
            .cursor_pointer()
            .hover(|s| s.border_color(t.border_strong))
            .on_drag(CardDrag { id, title: task.title.clone().into(), from: col }, |d, _, _, cx| cx.new(|_| d.clone()))
            .on_click(cx.listener(move |this, _, window, cx| {
                if col == Column::Draft {
                    this.edit_draft(id, window, cx);
                } else {
                    this.open_task(id, window, cx);
                }
            }))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.0))
                    .child(div().mt(px(2.0)).child(kit::status_glyph(
                        SharedString::from(format!("card-st-{id}")),
                        task.status,
                        task.unread,
                        t,
                    )))
                    .children(self.persona_for(id).map(|p| crate::team::avatar(p, 16.0, t).mt(px(1.0))))
                    .child(div().flex_1().min_w_0().text_size(ty::BODY).line_clamp(2).child(task.title.clone()))
                    .child(icon(ui::harness_icon(task.harness)).size(px(13.0)).text_color(ui::harness_tint(
                        task.harness,
                        t,
                        t.text_faint,
                    ))),
            )
            .when(col == Column::Draft, |d| {
                d.child(
                    div()
                        .text_size(ty::META)
                        .text_color(t.text_faint)
                        .line_clamp(2)
                        .child(task.prompt.clone().unwrap_or_default()),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap(px(6.0))
                    .text_size(ty::CAPTION)
                    .text_color(t.text_faint)
                    .child(
                        div().flex().items_center().gap(px(4.0)).child(project_badge(&project, 12.0, t)).child(project),
                    )
                    .when_some(task.branch.clone(), |d, b| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(3.0))
                                .child(icon(icons::GIT_BRANCH).size(px(11.0)))
                                .child(div().max_w(px(110.0)).truncate().child(b)),
                        )
                    })
                    .when_some(stat.filter(|s| s.0 > 0), |d, (_, a, r)| {
                        d.child(div().text_color(t.diff_add).child(format!("+{a}")))
                            .child(div().text_color(t.diff_del).child(format!("−{r}")))
                    })
                    .when_some(pr, |d, (n, url)| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("card-pr-{id}")))
                                .flex()
                                .items_center()
                                .gap(px(3.0))
                                .text_color(t.success)
                                .cursor_pointer()
                                .child(icon(icons::PULL_REQUEST).size(px(11.0)))
                                .child(format!("#{n}"))
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    cx.open_url(&url);
                                }),
                        )
                    })
                    .child(div().flex_1())
                    .child(elapsed),
            )
            .when(col == Column::Draft, |d| {
                d.child(
                    ui::button_ghost(SharedString::from(format!("card-start-{id}")), "Start", t)
                        .h(px(24.0))
                        .text_size(ty::META)
                        .child(icon(icons::ACTION_PLAY).size(px(12.0)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.start_draft(id, window, cx);
                        })),
                )
            })
            .into_any_element()
    }

    pub fn render_kanban(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let mut by_col: Vec<(Column, Vec<Task>)> = Column::ALL.iter().map(|c| (*c, Vec::new())).collect();
        for task in &self.tasks {
            let c = self.column_of(task);
            if let Some((_, v)) = by_col.iter_mut().find(|(col, _)| *col == c) {
                v.push(task.clone());
            }
        }
        for (_, v) in &mut by_col {
            v.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated_at.cmp(&a.updated_at)));
        }

        let columns = div().flex().gap(px(12.0)).items_start().children(by_col.into_iter().map(|(col, cards)| {
            let header = div()
                .h(px(32.0))
                .px(px(4.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(ui::dot(col.color(&t), 7.0))
                .child(div().text_size(ty::BODY).font_weight(gpui::FontWeight::MEDIUM).child(col.label()))
                .child(div().text_size(ty::META).text_color(t.text_faint).child(cards.len().to_string()))
                .child(div().flex_1())
                .child(ui::icon_button(
                    SharedString::from(format!("col-add-{col:?}")),
                    icons::PLUS,
                    "New draft",
                    &t,
                    cx.listener(|this, _, _, cx| {
                        this.pages.adding_draft = true;
                        this.pages.editing_draft = None;
                        cx.notify();
                    }),
                ));
            let form = (col == Column::Draft && self.pages.adding_draft).then(|| {
                div()
                    .p(px(10.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(t.accent.opacity(0.5))
                    .bg(t.surface_raised)
                    .child(
                        div()
                            .h(px(28.0))
                            .px(px(8.0))
                            .flex()
                            .items_center()
                            .rounded(px(6.0))
                            .bg(t.wash(0.05))
                            .text_size(ty::BODY)
                            .child(div().flex_1().child(Input::new(&self.pages.draft_title))),
                    )
                    .child(
                        div()
                            .px(px(8.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .bg(t.wash(0.05))
                            .text_size(ty::BODY)
                            .child(Textarea::new(&self.pages.draft_prompt)),
                    )
                    .when(!self.personas.is_empty(), |d| {
                        let current = self.pages.draft_persona.clone();
                        d.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(4.0))
                                .child(
                                    div().text_size(ty::CAPTION).text_color(t.text_faint).mr(px(2.0)).child("Run as"),
                                )
                                .child(
                                    ui::chip("draft-as-anyone", &t)
                                        .h(px(24.0))
                                        .when(current.is_none(), |c| c.border_color(t.accent).text_color(t.text))
                                        .child("Anyone")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.pages.draft_persona = None;
                                            cx.notify();
                                        })),
                                )
                                .children(self.personas.iter().map(|p| {
                                    let on = current.as_deref() == Some(p.id.as_str());
                                    let pid = p.id.clone();
                                    ui::chip(SharedString::from(format!("draft-as-{}", p.id)), &t)
                                        .h(px(24.0))
                                        .pl(px(4.0))
                                        .when(on, |c| c.border_color(t.accent).text_color(t.text))
                                        .child(crate::team::avatar(p, 16.0, &t))
                                        .child(p.name.clone())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.pages.draft_persona = Some(pid.clone());
                                            cx.notify();
                                        }))
                                })),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .child(
                                ui::button_primary(
                                    "draft-save",
                                    if self.pages.editing_draft.is_some() { "Save" } else { "Add draft" },
                                    &t,
                                )
                                .on_click(cx.listener(|this, _, window, cx| this.add_draft(window, cx))),
                            )
                            .child(ui::button_ghost("draft-cancel", "Cancel", &t).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.pages.adding_draft = false;
                                    this.pages.editing_draft = None;
                                    cx.notify();
                                },
                            )))
                            .when_some(self.pages.editing_draft, |d, id| {
                                d.child(div().flex_1()).child(
                                    ui::button_ghost("draft-delete", "Delete", &t).text_color(t.danger).on_click(
                                        cx.listener(move |this, _, window, cx| {
                                            this.pages.adding_draft = false;
                                            this.pages.editing_draft = None;
                                            this.delete_task(id, window, cx);
                                        }),
                                    ),
                                )
                            }),
                    )
            });
            div()
                .id(SharedString::from(format!("col-{col:?}")))
                .w(px(240.0))
                .flex_none()
                .min_h(px(240.0))
                .p(px(8.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .rounded(px(12.0))
                .bg(t.wash(0.03))
                .border_1()
                .border_color(t.border)
                .drag_over::<CardDrag>(move |s, _, _, cx| {
                    s.border_color(Theme::of(cx).accent).bg(Theme::of(cx).accent.opacity(0.05))
                })
                .on_drop(cx.listener(move |this, card: &CardDrag, window, cx| this.drop_card(card, col, window, cx)))
                .child(header)
                .children(form)
                .when(cards.is_empty() && !(col == Column::Draft && self.pages.adding_draft), |d| {
                    d.child(div().py(px(18.0)).text_center().text_size(ty::META).text_color(t.text_faint).child(
                        match col {
                            Column::Draft => "Drafts wait here until you start them",
                            Column::InProgress => "Running and idle threads",
                            Column::NeedsYou => "Approvals, questions and failures",
                            Column::Review => "Threads with an open PR",
                            Column::Done => "Finished threads",
                        },
                    ))
                })
                .children(cards.iter().map(|task| self.render_card(task, col, &t, cx)))
        }));

        let confirm = self.pages.confirm_done.and_then(|id| self.task(id).cloned()).map(|task| {
            let id = task.id;
            div()
                .p(px(12.0))
                .flex()
                .items_center()
                .gap(px(10.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(t.warning.opacity(0.4))
                .bg(t.warning.opacity(0.06))
                .text_size(ty::BODY)
                .child(
                    div()
                        .flex_1()
                        .child(format!("Archive \"{}\"? Its worktree stays until you delete the thread.", task.title)),
                )
                .child(ui::button_primary("done-yes", "Archive", &t).on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.pages.confirm_done = None;
                        this.set_archived(id, true, window, cx);
                    },
                )))
                .child(ui::button_ghost("done-no", "Cancel", &t).on_click(cx.listener(|this, _, _, cx| {
                    this.pages.confirm_done = None;
                    cx.notify();
                })))
        });
        let note = self.pages.kanban_note.clone().map(|n| {
            div().id("kanban-note").text_size(ty::SMALL).text_color(t.text_muted).cursor_pointer().child(n).on_click(
                cx.listener(|this, _, _, cx| {
                    this.pages.kanban_note = None;
                    cx.notify();
                }),
            )
        });
        let refresh = ui::icon_button(
            "kanban-refresh",
            icons::REFRESH,
            "Refresh PRs and diffstats",
            &t,
            cx.listener(|this, _, _, cx| this.load_kanban_meta(cx)),
        )
        .into_any_element();
        page_with(
            "kanban-page",
            "Kanban",
            "Drafts, work in flight and what is ready for review, across every project.",
            &t,
            Some(refresh),
            div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .children(confirm)
                .children(note)
                .child(div().id("kanban-scroll").overflow_x_scroll().child(columns)),
        )
    }
}
