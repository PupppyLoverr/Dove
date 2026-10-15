//! Archived threads: search, unarchive, delete for good.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    Styled as _, div, px,
};
use gpui_base::input::Input;

use super::page_with;
use crate::assets::{icon, icons};
use crate::kit;
use crate::shell::Shell;
use crate::theme::Theme;
use crate::ui;

impl Shell {
    pub fn render_archived_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let q = self.pages.archived_q.clone();
        let list: Vec<_> =
            self.archived.iter().filter(|x| q.is_empty() || x.title.to_lowercase().contains(&q)).cloned().collect();
        let search = div()
            .h(px(32.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(8.0))
            .bg(t.wash(0.05))
            .text_size(px(13.0))
            .child(icon(icons::MAGNIFER).size(px(14.0)).text_color(t.text_faint))
            .child(div().flex_1().child(Input::new(&self.pages.archived_search)));
        let rows = div()
            .flex()
            .flex_col()
            .rounded(px(12.0))
            .border_1()
            .border_color(t.border)
            .overflow_hidden()
            .when(list.is_empty(), |d| {
                d.child(kit::empty_state(
                    icons::ARCHIVE_MINIMALISTIC,
                    "Nothing archived",
                    "Archived threads keep their transcripts and worktrees until you delete them.",
                    &t,
                ))
            })
            .children(list.iter().enumerate().map(|(i, task)| {
                let id = task.id;
                let project = self.project(task.project_id).map(|p| p.name.clone()).unwrap_or_default();
                div()
                    .id(SharedString::from(format!("arch-{id}")))
                    .h(px(44.0))
                    .px(px(14.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .when(i > 0, |d| d.border_t_1().border_color(t.border))
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
                            .child(div().truncate().text_size(px(13.5)).child(task.title.clone()))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(t.text_faint)
                                    .child(format!("{project} · {}", ui::relative_time(task.updated_at))),
                            ),
                    )
                    .child(ui::button_ghost(SharedString::from(format!("arch-open-{id}")), "Open", &t).on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.navigate(crate::shell::Route::Thread(id), window, cx)
                        }),
                    ))
                    .child(
                        ui::button_ghost(SharedString::from(format!("arch-un-{id}")), "Unarchive", &t)
                            .on_click(cx.listener(move |this, _, window, cx| this.set_archived(id, false, window, cx))),
                    )
                    .child(
                        ui::button_ghost(SharedString::from(format!("arch-del-{id}")), "Delete", &t)
                            .text_color(t.danger)
                            .on_click(cx.listener(move |this, _, window, cx| this.delete_task(id, window, cx))),
                    )
            }));
        page_with("archived-page", "Archived", "Threads you put away. They keep their transcripts and branches.", &t, None, div().flex().flex_col().gap(px(12.0)).child(search).child(rows))
    }
}
