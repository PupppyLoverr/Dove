//! A side conversation that shares the parent thread's worktree.
//!
//! First send starts a joined task; after that this is a full Dove chat
//! (transcript + composer) docked in the panel.

use std::sync::Arc;

use dove_core::proto::TaskId;
use dove_core::store::{Project, Task};
use dove_core::{Engine, Joined, NewTask};
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
    px,
};

use crate::assets::{icon, icons};
use crate::composer::{self, Composer, ComposerEvent};
use crate::design::{radius, ty};
use crate::kit;
use crate::surface_chrome;
use crate::theme::Theme;
use crate::transcript::{TaskView, TaskViewEvent};
use crate::ui;

pub enum SideChatEvent {
    Started(#[allow(dead_code)] TaskId),
    OpenThread(TaskId),
    Changed,
}

pub struct SideChat {
    engine: Arc<Engine>,
    parent: Task,
    project: Project,
    composer: Entity<Composer>,
    chat: Option<Entity<TaskView>>,
    task_id: Option<TaskId>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<SideChatEvent> for SideChat {}

impl SideChat {
    pub fn new(
        engine: Arc<Engine>,
        parent: Task,
        project: Project,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let composer = cx.new(|cx| {
            let mut c = Composer::home(engine.clone(), window, cx);
            c.as_pill(cx);
            c.use_harness(parent.harness, parent.model.clone(), cx);
            c
        });
        let projects = vec![project.clone()];
        let pid = project.id;
        composer.update(cx, |c, cx| c.set_projects(projects, Some(pid), cx));
        let sub = cx.subscribe_in(&composer, window, |this, _, ev: &ComposerEvent, window, cx| match ev {
            ComposerEvent::Submit(text) => this.submit(text.clone(), window, cx),
            ComposerEvent::Stop => {}
            _ => {}
        });
        Self { engine, parent, project, composer, chat: None, task_id: None, _subs: vec![sub] }
    }

    pub fn submit(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(chat) = &self.chat {
            chat.update(cx, |v, cx| v.submit(text, window, cx));
            return;
        }
        let run = self.composer.read(cx).run_settings(cx);
        let harness = self.composer.read(cx).harness(cx);
        let new = NewTask {
            title: ui::title_from_prompt(&text),
            prompt: text.clone(),
            harness,
            run,
            join: Some(Joined::of(&self.parent)),
            persona: None,
        };
        match self.engine.start_task(&self.project, new) {
            Ok((task, live)) => {
                let id = task.id;
                let chat = cx.new(|cx| {
                    let mut v = TaskView::new(self.engine.clone(), task, self.project.clone(), Some(live), window, cx);
                    v.without_ai_titles();
                    v
                });
                self._subs.push(cx.subscribe(&chat, |_, _, ev: &TaskViewEvent, cx| match ev {
                    TaskViewEvent::Changed | TaskViewEvent::Status(_) | TaskViewEvent::Title(_) => {
                        cx.emit(SideChatEvent::Changed);
                        cx.notify();
                    }
                    _ => {}
                }));
                self.chat = Some(chat);
                self.task_id = Some(id);
                cx.emit(SideChatEvent::Started(id));
                cx.notify();
            }
            Err(e) => {
                composer::set_text(&self.composer, &text, window, cx);
                tracing::warn!("side chat did not start: {e:#}");
            }
        }
    }
}

impl Render for SideChat {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("SideChat");
        let t = Theme::of(cx).clone();
        if let Some(chat) = &self.chat {
            let task = chat.read(cx).task.clone();
            let tid = self.task_id;
            let header = surface_chrome::toolbar(&t)
                .px(px(12.0))
                .gap(px(8.0))
                .child(kit::status_glyph("side-chat-st", task.status, task.unread, &t))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(ty::SMALL)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(t.text)
                        .child(task.title.clone()),
                )
                .when_some(self.parent.branch.clone(), |d, b| {
                    d.child(
                        div()
                            .h(px(20.0))
                            .px(px(6.0))
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .rounded(px(radius::fit(20.0)))
                            .bg(t.wash(0.05))
                            .font_family(t.font_mono.clone())
                            .text_size(ty::BADGE)
                            .text_color(t.text_muted)
                            .child(icon(icons::GIT_BRANCH).size(px(10.0)).text_color(t.text_faint))
                            .child(b),
                    )
                })
                .when_some(tid, |d, id| {
                    d.child(
                        surface_chrome::toolbar_button("side-chat-open-tab", "Open as full tab")
                            .child(icon(icons::ARROW_UP_RIGHT).size(px(12.5)).text_color(t.text_muted))
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(SideChatEvent::OpenThread(id)))),
                    )
                });
            return div()
                .size_full()
                .flex()
                .flex_col()
                .child(header)
                .child(div().flex_1().min_h_0().child(chat.clone()));
        }
        let starters: &[(&str, &str)] = &[
            (
                "Review current diff",
                "Review the uncommitted changes in this worktree and point out bugs or edge cases.",
            ),
            ("Write tests", "Write focused unit/integration tests for the recent changes in this worktree."),
            ("Explain changes", "Summarize what changed in this worktree and how the pieces fit together."),
            ("Simplify code", "Find opportunities to simplify and clean up the recently modified files."),
        ];
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .p(px(20.0))
                    .gap(px(16.0))
                    .child(kit::empty_state(
                        icons::CHAT_ROUND_LINE,
                        "Side chat",
                        "A parallel conversation sharing this worktree and branch. Ask a question or run a side task without interrupting the main thread.",
                        &t,
                    ))
                    .child(
                        div()
                            .max_w(px(320.0))
                            .flex()
                            .flex_wrap()
                            .justify_center()
                            .gap(px(6.0))
                            .children(starters.iter().enumerate().map(|(i, (label, prompt))| {
                                let prompt = (*prompt).to_string();
                                div()
                                    .id(SharedString::from(format!("sc-starter-{i}")))
                                    .h(px(26.0))
                                    .px(px(10.0))
                                    .flex()
                                    .items_center()
                                    .rounded(px(radius::RADIUS))
                                    .border_1()
                                    .border_color(t.border)
                                    .bg(t.wash(0.03))
                                    .cursor_pointer()
                                    .text_size(ty::CAPTION)
                                    .text_color(t.text_muted)
                                    .hover(|s| s.bg(t.wash(0.07)).text_color(t.text))
                                    .child(*label)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.submit(prompt.clone(), window, cx);
                                    }))
                            })),
                    ),
            )
            .child(
                div()
                    .px(px(12.0))
                    .pb(px(12.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .pb(px(8.0))
                            .text_size(ty::META)
                            .text_color(t.text_faint)
                            .child(icon(icons::CHAT_ROUND_LINE).size(px(12.0)))
                            .child(format!("Beside {}", self.parent.title)),
                    )
                    .child(self.composer.clone()),
            )
    }
}
