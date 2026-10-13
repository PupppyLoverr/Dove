//! The sidebar: navigation, "Needs you", attention-sorted threads with
//! search and options, and the device footer. Collapsible to zero (Ctrl+B)
//! and resizable (208–400 px).

use std::time::Duration;

use dove_core::proto::{PermissionDecision, TaskStatus};
use dove_core::store::Task;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, Animation, AnimationExt as _, AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, anchored, deferred,
    div, hsla, px,
};
use gpui_base::input::Input;

use crate::assets::{icon, icons};
use crate::kit;
use crate::shell::{ChatMenu, Route, Shell};
use crate::theme::Theme;
use crate::ui;

/// A stable tint per project name for its badge.
pub fn project_hue(name: &str) -> f32 {
    let h = name.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    (h % 360) as f32 / 360.0
}

pub fn project_badge(name: &str, size: f32, theme: &Theme) -> impl IntoElement {
    let hue = project_hue(name);
    let (bg, fg): (Hsla, Hsla) = if theme.appearance.is_dark() {
        (hsla(hue, 0.45, 0.45, 0.28), hsla(hue, 0.70, 0.75, 1.0))
    } else {
        (hsla(hue, 0.55, 0.50, 0.18), hsla(hue, 0.65, 0.35, 1.0))
    };
    let initial: String =
        name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_ascii_uppercase()).unwrap_or('•').to_string();
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .bg(bg)
        .text_color(fg)
        .text_size(px(size * 0.66))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(initial)
}

/// Where a thread sorts: what needs you first, then work in flight, then
/// unread results, failures, and the rest by recency.
pub fn attention_rank(t: &Task) -> u8 {
    match (t.status, t.unread) {
        (TaskStatus::NeedsInput, _) => 0,
        (TaskStatus::Running, _) => 1,
        (TaskStatus::Done, true) => 2,
        (TaskStatus::Failed, _) => 3,
        _ => 4,
    }
}

/// Pinned first, then attention, then most recent.
pub fn attention_sort(tasks: &mut [Task]) {
    tasks.sort_by(|a, b| {
        b.pinned.cmp(&a.pinned).then(attention_rank(a).cmp(&attention_rank(b))).then(b.updated_at.cmp(&a.updated_at))
    });
}

impl Shell {
    pub fn render_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let compact = self.engine.store.setting("sidebar_density").as_deref() == Some("compact");
        let grouped = self.engine.store.setting("sidebar_group").as_deref() == Some("project");

        let nav_item = |id: &'static str,
                        glyph: &'static str,
                        label: &'static str,
                        route: Option<Route>,
                        badge: Option<AnyElement>,
                        this: &Self,
                        cx: &mut Context<Self>| {
            let selected = route.is_some_and(|r| r == this.route);
            div()
                .id(id)
                .h(px(30.0))
                .mx(px(8.0))
                .px(px(9.0))
                .flex()
                .items_center()
                .gap(px(9.0))
                .rounded(px(8.0))
                .cursor_pointer()
                .text_size(px(13.0))
                .when(selected, |d| d.bg(t.wash(0.10)).text_color(t.text))
                .when(!selected, |d| d.text_color(t.text_muted).hover(|s| s.bg(t.wash(0.05)).text_color(t.text)))
                .child(icon(glyph).size(px(16.0)))
                .child(div().flex_1().child(label))
                .children(badge)
                .on_click(cx.listener(move |this, _, window, cx| match route {
                    Some(r) => this.navigate(r, window, cx),
                    None => this.new_session(window, cx),
                }))
        };
        let drafts = self.tasks.iter().filter(|t| t.status == TaskStatus::Draft).count();
        // The one primary action, as a button rather than a list row.
        let new_thread = div()
            .id("nav-new")
            .mx(px(8.0))
            .mb(px(6.0))
            .h(px(34.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(9.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(t.wash(0.10))
            .bg(t.wash(0.05))
            .cursor_pointer()
            .text_size(px(13.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(t.text)
            .hover(|s| s.bg(t.wash(0.09)).border_color(t.wash(0.16)))
            .role(gpui::Role::Button)
            .aria_label("New thread")
            .child(icon(icons::PEN_NEW_SQUARE).size(px(16.0)).text_color(t.text))
            .child(div().flex_1().child("New thread"))
            .child(kit::kbd(&crate::keymap::pretty("secondary-n"), &t))
            .on_click(cx.listener(|this, _, window, cx| this.new_session(window, cx)));
        let nav = div()
            .flex()
            .flex_col()
            .gap(px(1.0))
            .pt(px(8.0))
            .child(new_thread)
            .child(nav_item("nav-threads", icons::MESSAGE_SQUARE_TEXT, "Threads", Some(Route::Threads), None, self, cx))
            .child(nav_item(
                "nav-kanban",
                icons::SQUARE_KANBAN,
                "Kanban",
                Some(Route::Kanban),
                (drafts > 0).then(|| {
                    div().text_size(px(11.5)).text_color(t.text_faint).child(drafts.to_string()).into_any_element()
                }),
                self,
                cx,
            ))
            .child(nav_item(
                "nav-automations",
                icons::CLOCK_CIRCLE,
                "Automations",
                Some(Route::Automations),
                (self.inbox_unread > 0).then(|| ui::dot(t.accent, 7.0).into_any_element()),
                self,
                cx,
            ))
            .child(nav_item("nav-prs", icons::PULL_REQUEST, "Pull requests", Some(Route::PullRequests), None, self, cx))
            .child(self.render_more_item(&t, cx));

        // Needs you: pending approvals and questions.
        let needs: Vec<Task> = self.needs_you().into_iter().cloned().collect();
        let needs_section = (!needs.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .pt(px(8.0))
                .child(
                    div()
                        .px(px(16.0))
                        .h(px(26.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(12.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(t.warning)
                        .child(icon(icons::BELL).size(px(13.0)).text_color(t.warning))
                        .child("Needs you")
                        .child(kit::badge(needs.len().to_string(), t.warning)),
                )
                .children(needs.iter().map(|task| self.render_needs_row(task, &t, cx)))
        });

        let q = self.search_q.clone();
        let mut visible: Vec<Task> = self
            .tasks
            .iter()
            .filter(|x| x.status != TaskStatus::Draft)
            .filter(|x| q.is_empty() || x.title.to_lowercase().contains(&q))
            .cloned()
            .collect();
        attention_sort(&mut visible);
        let show_archived = self.engine.store.setting("sidebar_archived").as_deref() == Some("true");

        let header = div()
            .relative()
            .px(px(10.0))
            .pt(px(10.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(
                div()
                    .flex_1()
                    .h(px(28.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .rounded(px(7.0))
                    .bg(t.wash(0.05))
                    .text_size(px(13.0))
                    .child(icon(icons::MAGNIFER).size(px(13.0)).text_color(t.text_faint))
                    .child(div().flex_1().child(Input::new(&self.search))),
            )
            .child(
                ui::icon_button(
                    "sb-options",
                    icons::MORE_HORIZONTAL,
                    "Sidebar options",
                    &t,
                    cx.listener(|this, _, _, cx| {
                        this.sidebar_options_open = !this.sidebar_options_open;
                        cx.notify();
                    }),
                )
                .when(self.sidebar_options_open, |d| d.bg(t.glass_hover())),
            )
            .when(self.sidebar_options_open, |d| {
                d.child(self.render_options_menu(grouped, compact, show_archived, &t, cx))
            });

        let mut list = div()
            .id("sb-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(1.0))
            .pb(px(8.0))
            .pt(px(4.0));
        if visible.is_empty() {
            list = list.child(div().px(px(16.0)).py(px(8.0)).text_size(px(13.0)).text_color(t.text_faint).child(
                if self.projects.is_empty() {
                    "Add a project to begin"
                } else if !q.is_empty() {
                    "No threads match"
                } else {
                    "No threads yet"
                },
            ));
        }
        if grouped {
            for p in &self.projects {
                let in_project: Vec<&Task> = visible.iter().filter(|x| x.project_id == p.id).collect();
                if in_project.is_empty() {
                    continue;
                }
                list = list.child(
                    div()
                        .px(px(16.0))
                        .pt(px(8.0))
                        .pb(px(2.0))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(11.5))
                        .text_color(t.text_faint)
                        .child(project_badge(&p.name, 12.0, &t))
                        .child(p.name.clone()),
                );
                for task in in_project {
                    list = list.child(self.render_row(task, compact, &t, cx));
                }
            }
        } else {
            for task in &visible {
                list = list.child(self.render_row(task, compact, &t, cx));
            }
        }
        if show_archived && !self.archived.is_empty() {
            list = list.child(
                div()
                    .px(px(16.0))
                    .pt(px(10.0))
                    .pb(px(2.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(11.5))
                    .text_color(t.text_faint)
                    .child(icon(icons::ARCHIVE_MINIMALISTIC).size(px(13.0)).text_color(t.text_faint))
                    .child("Archived"),
            );
            for task in self.archived.clone().iter().take(20) {
                list = list.child(self.render_row(task, compact, &t, cx));
            }
        }

        let footer = div()
            .h(px(52.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .border_t_1()
            .border_color(t.border)
            .child({
                let host = ui::host_name();
                let initial: String =
                    host.chars().find(|c| c.is_alphanumeric()).map_or('D', |c| c.to_ascii_uppercase()).to_string();
                div()
                    .size(px(26.0))
                    .flex_none()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui::linear_gradient(
                        135.0,
                        gpui::linear_color_stop(t.accent, 0.0),
                        gpui::linear_color_stop(t.accent_strong, 1.0),
                    ))
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(gpui::white())
                    .child(initial)
            })
            .child(
                div()
                    .id("sb-usage")
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| this.navigate(Route::Usage, window, cx)))
                    .child(div().text_size(px(13.0)).truncate().child(format!("Local · {}", ui::host_name())))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(t.text_faint)
                            .child(format!("Today {}", ui::usage_line(&self.today))),
                    ),
            )
            .child(ui::icon_button(
                "sb-settings",
                icons::SETTINGS,
                "Settings (Ctrl+,)",
                &t,
                cx.listener(|this, _, window, cx| this.navigate(Route::Settings, window, cx)),
            ));

        let handle = kit::resize_handle("sb-resize", &t, {
            let weak = cx.weak_entity();
            move |x, _, cx| {
                let _ = weak.update(cx, |s, _| s.begin_sidebar_resize(x));
            }
        });

        div()
            .w(px(self.sidebar_w))
            .h_full()
            .flex_none()
            .flex()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(nav)
                    .children(needs_section)
                    .child(header)
                    .child(list)
                    .child(footer),
            )
            .child(handle)
            .into_any_element()
    }

    fn render_more_item(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let selected =
            matches!(self.route, Route::Agents | Route::Usage | Route::Archived | Route::Settings | Route::Help);
        let open = self.more_open;
        let item = div()
            .id("nav-more")
            .h(px(32.0))
            .mx(px(6.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .text_size(px(13.5))
            .when(selected || open, |d| d.bg(t.wash(0.11)).text_color(t.text))
            .when(!selected && !open, |d| d.text_color(t.text_muted).hover(|s| s.bg(t.wash(0.06)).text_color(t.text)))
            .child(icon(icons::MORE_HORIZONTAL).size(px(16.0)))
            .child(div().flex_1().child("More"))
            .child(icon(icons::ALT_ARROW_RIGHT).size(px(12.0)).text_color(t.text_faint))
            .on_click(cx.listener(|this, _, _, cx| {
                this.more_open = !this.more_open;
                cx.notify();
            }));
        let menu = open.then(|| {
            let row =
                |id: &'static str, glyph: &'static str, label: &'static str, route: Route, cx: &mut Context<Self>| {
                    ui::menu_row(id, glyph, label, false, t).on_click(cx.listener(move |this, _, window, cx| {
                        this.more_open = false;
                        this.navigate(route, window, cx);
                    }))
                };
            let body = div()
                .w(px(220.0))
                .flex()
                .flex_col()
                .child(row("more-agents", icons::BOT, "Agents", Route::Agents, cx))
                .child(row("more-usage", icons::CLOCK_CIRCLE, "Usage", Route::Usage, cx))
                .child(row("more-archived", icons::ARCHIVE_MINIMALISTIC, "Archived", Route::Archived, cx))
                .child(ui::separator(t))
                .child(row("more-settings", icons::SETTINGS, "Settings", Route::Settings, cx))
                .child(row("more-help", icons::KEYBOARD, "Help & shortcuts", Route::Help, cx));
            deferred(
                anchored().anchor(Anchor::TopLeft).snap_to_window_with_margin(px(8.0)).child(
                    div()
                        .ml(px(self.sidebar_w - 20.0))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.more_open = false;
                            cx.notify();
                        }))
                        .child(ui::menu_card(t, body)),
                ),
            )
            .with_priority(2)
        });
        div().relative().child(item).children(menu).into_any_element()
    }

    fn render_options_menu(
        &self,
        grouped: bool,
        compact: bool,
        archived: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let toggle = |id: &'static str,
                      label: &'static str,
                      on: bool,
                      key: &'static str,
                      on_value: &'static str,
                      off_value: &'static str,
                      cx: &mut Context<Self>| {
            ui::menu_row(id, if on { icons::CHECK } else { icons::LIST }, label, false, t)
                .when(!on, |d| d.opacity(0.75))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let _ = this.engine.store.set_setting(key, if on { off_value } else { on_value });
                    cx.notify();
                }))
        };
        let body = div()
            .w(px(230.0))
            .flex()
            .flex_col()
            .child(toggle("opt-group", "Group by project", grouped, "sidebar_group", "project", "none", cx))
            .child(toggle("opt-compact", "Compact rows", compact, "sidebar_density", "compact", "comfortable", cx))
            .child(toggle("opt-archived", "Show archived", archived, "sidebar_archived", "true", "false", cx))
            .child(ui::separator(t))
            .child(ui::menu_row("opt-add", icons::DOCUMENT_ADD, "Add project…", false, t).on_click(cx.listener(
                |this, _, _, cx| {
                    this.sidebar_options_open = false;
                    this.add_project_dialog(cx);
                },
            )));
        deferred(
            anchored().anchor(Anchor::TopRight).snap_to_window_with_margin(px(8.0)).child(
                div()
                    .mt(px(32.0))
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.sidebar_options_open = false;
                        cx.notify();
                    }))
                    .child(ui::menu_card(t, body)),
            ),
        )
        .with_priority(2)
        .into_any_element()
    }

    /// A Needs-you row: the pending approval with inline Allow / Deny, or a
    /// question that opens the thread.
    fn render_needs_row(&self, task: &Task, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let id = task.id;
        let pending = self.threads.get(&id).and_then(|v| v.read(cx).chat.read(cx).pending());
        let summary = pending
            .as_ref()
            .map(|p| {
                if p.is_question() {
                    p.questions.first().map(|q| q.question.clone()).unwrap_or_default()
                } else {
                    format!("{}: {}", p.tool, p.summary)
                }
            })
            .unwrap_or_else(|| "Waiting for you".into());
        let decide = |label: &'static str, d: PermissionDecision, primary: bool, cx: &mut Context<Self>| {
            let rid = pending.as_ref().map(|p| p.request_id.clone()).unwrap_or_default();
            let el = if primary {
                ui::button_primary(SharedString::from(format!("need-{label}-{id}")), label, t).h(px(24.0))
            } else {
                ui::button_ghost(SharedString::from(format!("need-{label}-{id}")), label, t).h(px(24.0))
            };
            el.text_size(px(12.0)).on_click(cx.listener(move |this, _, _, cx| {
                if let Some(v) = this.threads.get(&id) {
                    let chat = v.read(cx).chat.clone();
                    let rid = rid.clone();
                    chat.update(cx, |c, cx| c.decide_from_outside(rid, d, cx));
                }
            }))
        };
        let approval = pending.as_ref().is_some_and(|p| !p.is_question());
        div()
            .id(SharedString::from(format!("need-{id}")))
            .mx(px(6.0))
            .px(px(10.0))
            .py(px(8.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .rounded(px(9.0))
            .border_1()
            .border_color(t.warning.opacity(0.35))
            .bg(t.warning.opacity(0.06))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| this.open_task(id, window, cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(icon(ui::harness_icon(task.harness)).size(px(13.0)).text_color(ui::harness_tint(
                        task.harness,
                        t,
                        t.text_muted,
                    )))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(13.0)).child(task.title.clone())),
            )
            .child(div().text_size(px(12.0)).text_color(t.text_muted).line_clamp(2).child(summary))
            .when(approval, |d| {
                d.child(
                    div()
                        .flex()
                        .gap(px(4.0))
                        .child(decide("Allow", PermissionDecision::AllowOnce, true, cx))
                        .child(decide("Deny", PermissionDecision::Deny, false, cx)),
                )
            })
            .into_any_element()
    }

    fn render_row(&self, task: &Task, compact: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let id = task.id;
        let selected = self.route == Route::Thread(id);
        let archived = task.status == TaskStatus::Archived;
        let renaming = self.rename.as_ref().filter(|(rid, _)| *rid == id).map(|(_, input)| input.clone());
        let group = SharedString::from(format!("row-{id}"));
        let rank = attention_rank(task);
        // What a working thread is doing right now, on hover.
        let step = (task.status == TaskStatus::Running)
            .then(|| self.threads.get(&id).and_then(|v| v.read(cx).chat.read(cx).current_step()))
            .flatten();
        let corner = div().text_size(px(11.5)).text_color(t.text_faint).child(ui::relative_time(task.updated_at));
        let archive_btn = div()
            .id(SharedString::from(format!("row-archive-{id}")))
            .size(px(22.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .hover(|s| s.bg(t.wash(0.12)))
            .tooltip(ui::tip(if archived { "Unarchive" } else { "Archive" }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.set_archived(id, !archived, window, cx);
            }))
            .child(
                icon(if archived { icons::ARCHIVE_UP_MINIMALISTIC } else { icons::ARCHIVE_MINIMALISTIC })
                    .size(px(14.0))
                    .text_color(t.text_muted),
            );

        let row = div()
            .id(SharedString::from(format!("row-{id}")))
            .group(group.clone())
            .h(px(if compact { 28.0 } else { 34.0 }))
            .mx(px(6.0))
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .when(selected, |d| d.bg(t.wash(0.11)))
            .when(!selected, |d| d.hover(|s| s.bg(t.wash(0.06))))
            .when(archived && !selected, |d| d.opacity(0.6))
            .when_some(step, |d, s| d.tooltip(ui::tip(s)))
            .child(div().w(px(12.0)).flex().justify_center().child(kit::status_glyph(
                SharedString::from(format!("sb-st-{id}")),
                task.status,
                task.unread,
                t,
            )))
            .child(icon(ui::harness_icon(task.harness)).size(px(15.0)).text_color(ui::harness_tint(
                task.harness,
                t,
                t.text_muted,
            )))
            .child(match renaming {
                Some(input) => div()
                    .flex_1()
                    .min_w_0()
                    .px(px(4.0))
                    .rounded(px(5.0))
                    .bg(t.wash(0.08))
                    .child(Input::new(&input))
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.0))
                    .when(task.unread, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                    .text_color(if selected || task.unread { t.text } else { t.text_muted })
                    .child(task.title.clone())
                    .into_any_element(),
            })
            .when(task.pinned, |d| d.child(icon(icons::PIN).size(px(12.0)).text_color(t.text_faint)))
            .child(
                div()
                    .flex_none()
                    .relative()
                    .child(div().group_hover(group.clone(), |s| s.invisible()).child(corner))
                    .child(
                        div()
                            .absolute()
                            .right(px(-4.0))
                            .top(px(-3.0))
                            .invisible()
                            .group_hover(group, |s| s.visible())
                            .child(archive_btn),
                    ),
            )
            .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                if e.click_count() == 2 {
                    this.begin_rename(id, window, cx);
                } else if this.rename.as_ref().is_none_or(|(rid, _)| *rid != id) {
                    this.open_task(id, window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    this.menu = Some(ChatMenu { task: id, at: e.position, copy_open: false, confirm_delete: false });
                    cx.notify();
                }),
            );
        // Rows glide in when they move to a new attention bucket.
        div()
            .child(row)
            .with_animation(
                SharedString::from(format!("glide-{id}-{rank}")),
                Animation::new(Duration::from_millis(260)).with_easing(gpui::ease_out_quint()),
                |el, delta| el.opacity(0.4 + 0.6 * delta).mt(px(6.0 * (1.0 - delta))),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dove_core::proto::{HarnessKind, ProjectId, TaskId};

    fn task(status: TaskStatus, unread: bool, pinned: bool, at: i64) -> Task {
        Task {
            id: TaskId::new(),
            project_id: ProjectId::new(),
            title: String::new(),
            harness: HarnessKind::ClaudeCode,
            model: None,
            status,
            worktree: None,
            branch: None,
            native_session_id: None,
            base: None,
            pinned,
            updated_at: at,
            prompt: None,
            unread,
            automation: None,
        }
    }

    #[test]
    fn attention_order() {
        let mut v = vec![
            task(TaskStatus::Idle, false, false, 9),
            task(TaskStatus::Failed, false, false, 1),
            task(TaskStatus::Done, true, false, 2),
            task(TaskStatus::Running, false, false, 3),
            task(TaskStatus::NeedsInput, false, false, 4),
            task(TaskStatus::Idle, false, true, 0),
        ];
        attention_sort(&mut v);
        let order: Vec<_> = v.iter().map(|t| (t.pinned, t.status)).collect();
        assert_eq!(
            order,
            vec![
                (true, TaskStatus::Idle),
                (false, TaskStatus::NeedsInput),
                (false, TaskStatus::Running),
                (false, TaskStatus::Done),
                (false, TaskStatus::Failed),
                (false, TaskStatus::Idle),
            ]
        );
    }
}
