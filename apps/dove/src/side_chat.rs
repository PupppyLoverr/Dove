//! A side conversation that shares the parent thread's worktree.
//!
//! First send starts a joined task; after that this is a full Dove chat
//! (transcript + composer) docked in the panel.

use std::sync::Arc;

use dove_core::proto::TaskId;
use dove_core::store::{Project, Task};
use dove_core::{Engine, Joined, NewTask};
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    Window, div, px,
};

use crate::assets::{icon, icons};
use crate::composer::{self, Composer, ComposerEvent};
use crate::kit;
use crate::theme::Theme;
use crate::transcript::{TaskView, TaskViewEvent};
use crate::ui;

pub enum SideChatEvent {
    Started(#[allow(dead_code)] TaskId),
    Changed,
}

pub struct SideChat {
    engine: Arc<Engine>,
    parent: Task,
    project: Project,
    composer: Entity<Composer>,
    chat: Option<Entity<TaskView>>,
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
        Self { engine, parent, project, composer, chat: None, _subs: vec![sub] }
    }

    fn submit(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
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
            worktree: false,
            from_branch: None,
            join: Some(Joined::of(&self.parent)),
        };
        match self.engine.start_task(&self.project, new) {
            Ok((task, live)) => {
                let id = task.id;
                let chat =
                    cx.new(|cx| TaskView::new(self.engine.clone(), task, self.project.clone(), Some(live), window, cx));
                self._subs.push(cx.subscribe(&chat, |_, _, ev: &TaskViewEvent, cx| match ev {
                    TaskViewEvent::Changed | TaskViewEvent::Status(_) | TaskViewEvent::Title(_) => {
                        cx.emit(SideChatEvent::Changed);
                        cx.notify();
                    }
                    _ => {}
                }));
                self.chat = Some(chat);
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
        let t = Theme::of(cx).clone();
        if let Some(chat) = &self.chat {
            return div().size_full().child(chat.clone());
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().flex().items_center().justify_center().p(px(20.0)).child(kit::empty_state(
                icons::CHAT_ROUND_LINE,
                "Side chat",
                "A second conversation in this worktree. The parent thread stays put.",
                &t,
            )))
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
                            .text_size(px(12.0))
                            .text_color(t.text_faint)
                            .child(icon(icons::CHAT_ROUND_LINE).size(px(12.0)))
                            .child(format!("Beside {}", self.parent.title)),
                    )
                    .child(self.composer.clone()),
            )
    }
}
