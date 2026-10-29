//! Threads side by side in the chat area: the open thread and up to three
//! companions, each a whole thread (transcript, composer, Chat/Terminal
//! switch) in its own column.
//!
//! Any thread dragged onto the conversation (a sidebar row, a teammate, a
//! teammate's thread, a column's header) joins as a companion; voice ("open
//! the tests thread beside this") and `DOVE_BESIDE` do the same. Each open
//! thread keeps its own companions (`split.<thread>`), restored when it
//! opens again; a companion's view never hibernates while it shows.

use dove_core::proto::{TaskId, TaskStatus};
use gpui::{
    AnyElement, AnyView, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, StyleRefinement, Styled as _, Window, div, px,
};

use crate::assets::{icon, icons};
use crate::design::{radius, ty};
use crate::shell::{Route, Shell};
use crate::side_panel::ThreadDrag;
use crate::theme::Theme;
use crate::thread::{ThreadView, ViewMode};
use crate::ui;

/// Companions beside the open thread, at most.
pub const MAX_COMPANIONS: usize = 3;
/// A column's narrowest before the row scrolls.
const COLUMN_MIN: f32 = 340.0;
const HEAD: f32 = 38.0;

fn key(main: TaskId) -> String {
    format!("split.{main}")
}

impl Shell {
    /// The threads shown beside `main`, in order (saved ones loaded once).
    pub(crate) fn companions_of(&mut self, main: TaskId) -> Vec<TaskId> {
        if !self.companions.contains_key(&main) {
            let saved: Vec<TaskId> = self
                .engine
                .store
                .setting(&key(main))
                .map(|s| s.split(',').filter_map(TaskId::parse).collect())
                .unwrap_or_default();
            self.companions.insert(main, saved);
        }
        let ids = self.companions.get(&main).cloned().unwrap_or_default();
        ids.into_iter()
            .filter(|id| *id != main && self.task(*id).is_some_and(|t| t.status != TaskStatus::Draft))
            .take(MAX_COMPANIONS)
            .collect()
    }

    fn save_companions(&self, main: TaskId) {
        let list = self.companions.get(&main).cloned().unwrap_or_default();
        let s: Vec<String> = list.iter().map(|id| id.to_string()).collect();
        let _ = self.engine.store.set_setting(&key(main), &s.join(","));
    }

    /// Whether `id` shows as a companion of any thread.
    pub(crate) fn is_companion(&self, id: TaskId) -> bool {
        self.companions.values().any(|v| v.contains(&id))
    }

    /// Show thread `id` beside the open one in the chat area. With no
    /// thread open, it opens. Dropping the open thread on itself does
    /// nothing; one already beside moves to the end. The oldest companion
    /// makes room when there are already [`MAX_COMPANIONS`].
    pub(crate) fn split_with(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        let Route::Thread(main) = self.route else {
            self.open_task(id, window, cx);
            return;
        };
        if main == id {
            return;
        }
        let Some(view) = self.thread_view_for(id, window, cx) else { return };
        // A thread shows in one place: a dock pane showing it moves here.
        if self.panel.read(cx).holds_thread(id) {
            self.panel.update(cx, |p, cx| p.close(crate::side_panel::Surface::Thread(id), window, cx));
        }
        let mut list = self.companions_of(main);
        list.retain(|x| *x != id);
        list.push(id);
        while list.len() > MAX_COMPANIONS {
            list.remove(0);
        }
        let wanted = list.len() + 1;
        self.companions.insert(main, list);
        self.save_companions(main);
        view.update(cx, |v, cx| v.set_shown(true, cx));
        // No room with the dock open, room without it: the dock steps aside
        // so the thread just dropped is in view.
        if self.panel_open
            && self.columns_that_fit(true, window) < wanted
            && self.columns_that_fit(false, window) > self.columns_that_fit(true, window)
        {
            self.panel_open = false;
            let _ = self.engine.store.set_setting("panel_open", "false");
            crate::kit::Toasts::push(
                cx,
                "Closed the side panel to fit the threads",
                Some("Ctrl+\\ brings it back.".to_string()),
                crate::kit::Tone::Info,
                None,
            );
        }
        cx.notify();
    }

    /// How many thread columns fit the chat area, with or without the dock.
    fn columns_that_fit(&self, with_dock: bool, window: &Window) -> usize {
        let mut w = f32::from(window.viewport_size().width);
        if self.sidebar_shown() {
            w -= self.sidebar_w;
        }
        if with_dock {
            w -= self.panel_width(window);
        }
        ((w / COLUMN_MIN).floor() as usize).max(1)
    }

    /// Bring folded companion `id` into view (it becomes the newest).
    fn bring_in(&mut self, main: TaskId, id: TaskId, cx: &mut Context<Self>) {
        if let Some(list) = self.companions.get_mut(&main) {
            list.retain(|x| *x != id);
            list.push(id);
        }
        self.save_companions(main);
        cx.notify();
    }

    /// Take `id` out of the columns beside `main`.
    pub(crate) fn close_companion(&mut self, main: TaskId, id: TaskId, cx: &mut Context<Self>) {
        if let Some(list) = self.companions.get_mut(&main) {
            list.retain(|x| *x != id);
        }
        self.save_companions(main);
        if let Some(v) = self.threads.get(&id).cloned()
            && !self.keeps_thread(id, cx)
        {
            v.update(cx, |v, cx| v.set_shown(false, cx));
        }
        cx.notify();
    }

    /// Make companion `id` the open thread; `main` takes its column.
    fn swap_main(&mut self, main: TaskId, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        let mut list = self.companions_of(main);
        if let Some(at) = list.iter().position(|x| *x == id) {
            list[at] = main;
        }
        self.companions.insert(main, Vec::new());
        self.save_companions(main);
        self.companions.insert(id, list);
        self.save_companions(id);
        self.open_task(id, window, cx);
    }

    /// The open thread, with its companions beside it when it has any.
    /// A thread dragged onto the area joins them.
    pub(crate) fn render_split(
        &mut self,
        main: TaskId,
        view: Entity<ThreadView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = Theme::of(cx).clone();
        // A chat is drawn directly (its composer can't sit under a cached
        // view); a terminal is cached whole.
        let body = |v: &Entity<ThreadView>, cx: &mut Context<Self>| {
            if v.read(cx).view == ViewMode::Chat {
                v.clone().into_any_element()
            } else {
                AnyView::from(v.clone()).cached(StyleRefinement::default().size_full()).into_any_element()
            }
        };
        let all = self.companions_of(main);
        // As many columns as fit, the newest companions first in line; the
        // rest fold into the strip on the right.
        let room = self.columns_that_fit(self.panel_open, window).saturating_sub(1);
        let (folded, shown) = all.split_at(all.len().saturating_sub(room));
        let folded = folded.to_vec();
        let companions: Vec<(TaskId, Entity<ThreadView>)> =
            shown.iter().filter_map(|id| Some((*id, self.thread_view_for(*id, window, cx)?))).collect();
        let main_body = body(&view, cx);
        let content: AnyElement = if companions.is_empty() && folded.is_empty() {
            main_body
        } else {
            let mut row =
                div().id("split-row").size_full().flex().child(div().flex_1().min_w_0().h_full().child(main_body));
            for (id, v) in companions {
                let column_body = body(&v, cx);
                let head = self.companion_head(main, id, &v, &t, cx);
                row = row.child(
                    div()
                        .id(SharedString::from(format!("split-col-{id}")))
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .flex()
                        .flex_col()
                        .border_l_1()
                        .border_color(t.border)
                        .child(head)
                        .child(div().flex_1().min_h_0().child(column_body)),
                );
            }
            if !folded.is_empty() {
                row = row.child(self.folded_strip(main, &folded, &t, cx));
            }
            row.into_any_element()
        };
        // The drop layer exists only while something is dragged, so the
        // conversation keeps its own hover and clicks.
        let layer = cx.has_active_drag().then(|| {
            div()
                .id("split-drop")
                .group("split-drop")
                .absolute()
                .inset_0()
                .p(px(10.0))
                .on_drop(cx.listener(|this, d: &ThreadDrag, window, cx| this.split_with(d.id, window, cx)))
                .child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(radius::CARD))
                        .border_2()
                        .border_color(gpui::transparent_black())
                        .group_drag_over::<ThreadDrag>("split-drop", |s| {
                            s.border_color(t.accent.opacity(0.55)).bg(t.accent.opacity(0.07))
                        })
                        .child(
                            div()
                                .h(px(34.0))
                                .px(px(14.0))
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .rounded_full()
                                .bg(t.surface_raised)
                                .border_1()
                                .border_color(t.border)
                                .shadow_md()
                                .text_size(ty::SMALL)
                                .text_color(t.text)
                                .invisible()
                                .group_drag_over::<ThreadDrag>("split-drop", |s| s.visible())
                                .child(icon(icons::SPLIT_VERTICAL).size(px(14.0)).text_color(t.accent))
                                .child("Drop to open side by side"),
                        ),
                )
        });
        div().relative().size_full().child(content).children(layer).into_any_element()
    }

    /// Companions that don't fit: a slim strip of their marks on the right
    /// edge. A click brings one into view (the oldest shown one folds).
    fn folded_strip(&self, main: TaskId, folded: &[TaskId], t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let marks = folded.iter().filter_map(|id| {
            let task = self.task(*id)?.clone();
            let id = *id;
            let mark: AnyElement = match self.persona_for(id) {
                Some(p) => crate::team::avatar(p, 20.0, t).into_any_element(),
                None => icon(ui::harness_icon(task.harness))
                    .size(px(15.0))
                    .text_color(ui::harness_tint(task.harness, t, t.text_muted))
                    .into_any_element(),
            };
            Some(
                div()
                    .id(SharedString::from(format!("split-fold-{id}")))
                    .size(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(radius::RADIUS))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.wash(0.08)))
                    .tooltip(ui::tip(format!("{} · click to show (no room for more columns)", task.title)))
                    .child(mark)
                    .on_click(cx.listener(move |this, _, _, cx| this.bring_in(main, id, cx))),
            )
        });
        div()
            .w(px(44.0))
            .flex_none()
            .h_full()
            .pt(px(10.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.0))
            .border_l_1()
            .border_color(t.border)
            .bg(t.wash(0.02))
            .child(div().text_size(ty::MICRO).text_color(t.text_faint).child(format!("+{}", folded.len())))
            .children(marks)
            .into_any_element()
    }

    /// A companion column's header: the agent (or teammate) and status, the
    /// title, its Chat/Terminal switch, make-main and close. It drags the
    /// thread elsewhere (the dock, another column order).
    fn companion_head(
        &self,
        main: TaskId,
        id: TaskId,
        v: &Entity<ThreadView>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let task = v.read(cx).chat.read(cx).task.clone();
        let status = v.read(cx).chat.read(cx).status;
        let dot = match status {
            TaskStatus::Running => Some(t.busy),
            TaskStatus::NeedsInput => Some(t.warning),
            TaskStatus::Failed => Some(t.danger),
            _ => None,
        };
        let mark: AnyElement = match self.persona_for(id) {
            Some(p) => crate::team::avatar(p, 18.0, t).into_any_element(),
            None => icon(ui::harness_icon(task.harness))
                .size(px(15.0))
                .text_color(ui::harness_tint(task.harness, t, t.text_muted))
                .into_any_element(),
        };
        let switch = v.update(cx, |v, cx| v.view_switch(t, cx));
        let button = |key: &str, glyph: &'static str, tip: &'static str| {
            div()
                .id(SharedString::from(format!("split-{key}-{id}")))
                .size(px(26.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(radius::RADIUS))
                .cursor_pointer()
                .hover(|s| s.bg(t.wash(0.08)))
                .tooltip(ui::tip(tip))
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .child(icon(glyph).size(px(13.0)).text_color(t.text_muted))
        };
        let title: SharedString = task.title.clone().into();
        div()
            .id(SharedString::from(format!("split-head-{id}")))
            .h(px(HEAD))
            .flex_none()
            .pl(px(12.0))
            .pr(px(6.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .border_b_1()
            .border_color(t.border)
            .on_drag(ThreadDrag { id, title: title.clone() }, |d, _, _, cx| cx.new(|_| d.clone()))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .child(mark)
                    .children(dot.map(|c| div().absolute().right(px(-3.0)).bottom(px(-2.0)).child(ui::dot(c, 6.5)))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ty::SMALL)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(t.text)
                    .child(title),
            )
            .child(switch)
            .child(
                button("main", icons::PANE_SWAP, "Make this the main thread")
                    .on_click(cx.listener(move |this, _, window, cx| this.swap_main(main, id, window, cx))),
            )
            .child(
                button("close", icons::CLOSE, "Close this column")
                    .on_click(cx.listener(move |this, _, _, cx| this.close_companion(main, id, cx))),
            )
            .into_any_element()
    }
}
