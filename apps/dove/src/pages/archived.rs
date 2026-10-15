//! Archived threads: search, open, unarchive, delete for good (after a
//! confirmation). The list is a `uniform_list`, so a long archive scrolls
//! at the cost of a screenful.

use std::rc::Rc;

use dove_core::store::Task;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px, uniform_list,
};

use super::PageDialog;
use crate::assets::{icon, icons};
use crate::design::{space, ty};
use crate::kit::controls::{self as ctl, Tone};
use crate::kit::page;
use crate::shell::Shell;
use crate::theme::Theme;
use crate::ui;

impl Shell {
    fn archived_row(&self, task: &Task, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let id = task.id;
        let project = self.project(task.project_id).map(|p| p.name.clone()).unwrap_or_default();
        page::table_row(t, false)
            .id(SharedString::from(format!("arch-{id}")))
            .hover(|s| s.bg(t.wash(0.04)))
            .child(icon(ui::harness_icon(task.harness)).size(px(15.0)).flex_none().text_color(ui::harness_tint(
                task.harness,
                t,
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
                            .text_size(ty::BODY)
                            .line_height(ty::lh::BODY)
                            .text_color(t.text)
                            .child(task.title.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(ty::CAPTION)
                            .text_color(t.text_faint)
                            .child(format!("{project} · {}", ui::relative_time(task.updated_at))),
                    ),
            )
            .child(ctl::button_sm(t, SharedString::from(format!("arch-open-{id}")), "Open", Tone::Quiet).on_click(
                cx.listener(move |this, _, window, cx| this.navigate(crate::shell::Route::Thread(id), window, cx)),
            ))
            .child(
                ctl::button_sm(t, SharedString::from(format!("arch-un-{id}")), "Unarchive", Tone::Filled)
                    .on_click(cx.listener(move |this, _, window, cx| this.set_archived(id, false, window, cx))),
            )
            .child(ctl::button_sm(t, SharedString::from(format!("arch-del-{id}")), "Delete", Tone::Danger).on_click(
                cx.listener(move |this, _, window, cx| {
                    this.open_page_dialog(PageDialog::DeleteArchived(id), window, cx)
                }),
            ))
            .into_any_element()
    }

    pub fn render_archived_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let q = self.pages.archived_q.clone();
        let list: Rc<Vec<Task>> = Rc::new(
            self.archived.iter().filter(|x| q.is_empty() || x.title.to_lowercase().contains(&q)).cloned().collect(),
        );
        let count = list.len();
        let body: AnyElement = if list.is_empty() {
            page::empty_state(
                &t,
                icons::ARCHIVE_MINIMALISTIC,
                "Nothing archived",
                "Archived threads keep their transcripts and worktrees until you delete them.",
                None,
            )
            .into_any_element()
        } else {
            let rows = list.clone();
            let theme = t.clone();
            uniform_list(
                "archived-list",
                count,
                cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                    range.filter_map(|ix| rows.get(ix)).map(|task| this.archived_row(task, &theme, cx)).collect()
                }),
            )
            .h(px(count as f32 * page::TABLE_ROW))
            .flex_shrink(1.0)
            .min_h_0()
            .into_any_element()
        };
        let head = page::table_head(&t)
            .child(div().flex_1().child("Thread"))
            .child(div().flex_none().child(format!("{} archived", self.archived.len())));
        let table = page::card(&t).mt(px(space::MD)).flex_shrink(1.0).min_h_0().child(head).child(body);
        let (root, column) = page::page_fill("archived-page", page::PAGE_WIDE);
        root.child(crate::motion::fade_in(
            "archived-in",
            column
                .child(page::page_head(
                    &t,
                    "Archived",
                    "Threads you put away. They keep their transcripts and branches.",
                    None,
                ))
                .child(div().mt(px(24.0)).flex_none().child(page::search_field(&t, &self.pages.archived_search)))
                .child(table),
        ))
        .into_any_element()
    }
}
