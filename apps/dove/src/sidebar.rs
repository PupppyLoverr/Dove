//! The sidebar: navigation, "Needs you", attention-sorted threads with
//! search and options, and the device footer. Collapsible to zero (Ctrl+B)
//! and resizable (208–400 px).

use dove_core::proto::{PermissionDecision, TaskStatus};
use dove_core::store::Task;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnimationExt as _, AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    MouseDownEvent, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, anchored, deferred,
    div, hsla, px,
};
use gpui_base::input::Input;

use crate::assets::{icon, icons};
use crate::design::{radius, ty};
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

/// A footer glyph. The hit target stays, and hover only darkens the mark —
/// a filled rounded plate here reads as a button stuck on the row.
fn quiet_glyph(
    id: &'static str,
    glyph: &'static str,
    label: &'static str,
    t: &Theme,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .group(id)
        .size(px(24.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .role(gpui::Role::Button)
        .aria_label(label)
        .tooltip(ui::tip(label))
        .on_click(on_click)
        .child(icon(glyph).size(px(16.0)).text_color(t.text_faint).group_hover(id, |s| s.text_color(t.text)))
}

/// A quiet label over a group of sidebar rows.
fn group_label(glyph: &'static str, label: &'static str, t: &Theme) -> gpui::Div {
    div()
        .px(px(16.0))
        .pt(px(6.0))
        .pb(px(2.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(9.0))
        .text_size(ty::CAPTION)
        .text_color(t.text_faint)
        .child(
            div()
                .w(px(16.0))
                .flex_none()
                .flex()
                .justify_center()
                .child(icon(glyph).size(px(12.0)).text_color(t.text_faint)),
        )
        .child(label)
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
                .group(id)
                .h(px(30.0))
                .mx(px(8.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .gap(px(9.0))
                .rounded(px(radius::RADIUS))
                .cursor_pointer()
                .text_size(ty::BODY)
                .when(selected, |d| d.text_color(t.text).font_weight(gpui::FontWeight::MEDIUM))
                .when(!selected, |d| {
                    crate::motion::hover_fade(
                        d,
                        SharedString::from(format!("nav-{label}")),
                        (t.wash(0.0), t.wash(0.0)),
                        Some((t.text_muted, t.text)),
                    )
                })
                .child(
                    icon(glyph)
                        .size(px(16.0))
                        .text_color(if selected { t.text } else { t.text_muted })
                        .group_hover(id, |s| s.text_color(t.text)),
                )
                .child(div().flex_1().child(label))
                .children(badge)
                .on_click(cx.listener(move |this, _, window, cx| match route {
                    Some(r) => this.navigate(r, window, cx),
                    None => this.new_session(window, cx),
                }))
        };
        let drafts = self.tasks.iter().filter(|t| t.status == TaskStatus::Draft).count();
        let active_threads = self.tasks.iter().filter(|t| t.status != TaskStatus::Draft).count();
        let running_threads = self.tasks.iter().filter(|t| t.status == TaskStatus::Running).count();
        // Leads the list as a row. Same hit area as the items under it —
        // a bordered plate here reads as a button sitting on the list.
        let new_thread = div()
            .id("nav-new")
            .h(px(30.0))
            .mx(px(8.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(9.0))
            .rounded(px(radius::RADIUS))
            .cursor_pointer()
            .text_size(ty::BODY)
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(t.text)
            .role(gpui::Role::Button)
            .aria_label("New thread")
            .child(icon(icons::PEN_NEW_SQUARE).size(px(15.0)).text_color(t.text))
            .child(div().flex_1().child("New thread"))
            .child(kit::kbd(&crate::keymap::pretty("secondary-n"), &t))
            .on_click(cx.listener(|this, _, window, cx| this.new_session(window, cx)));
        let nav = div()
            .flex()
            .flex_col()
            .gap(px(1.0))
            .pt(px(8.0))
            .child(new_thread)
            .child(nav_item(
                "nav-threads",
                icons::CHAT_ROUND_LINE,
                "Threads",
                Some(Route::Threads),
                if running_threads > 0 {
                    Some(kit::badge(format!("{running_threads} active"), t.busy).into_any_element())
                } else if active_threads > 0 {
                    Some(
                        div()
                            .text_size(ty::MICRO)
                            .text_color(t.text_faint)
                            .child(active_threads.to_string())
                            .into_any_element(),
                    )
                } else {
                    None
                },
                self,
                cx,
            ))
            .child(nav_item(
                "nav-kanban",
                icons::SPLIT_COLUMNS,
                "Kanban",
                Some(Route::Kanban),
                (drafts > 0).then(|| {
                    div().text_size(ty::MICRO).text_color(t.text_faint).child(drafts.to_string()).into_any_element()
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
                        .text_size(ty::META)
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

        // "Threads" reads like the Team header above it: a quiet label and
        // count, with search and options as trailing glyphs that end on the
        // same 16 px line as every other trailing mark.
        let searching = self.search_open || !q.is_empty();
        let header_button = |id: &'static str, glyph: &'static str, label: &'static str, on: bool| {
            div()
                .id(id)
                .group(id)
                .size(px(22.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .role(gpui::Role::Button)
                .aria_label(label)
                .tooltip(ui::tip(label))
                .child(
                    icon(glyph)
                        .size(px(14.0))
                        .text_color(if on { t.text } else { t.text_faint })
                        .group_hover(id, |s| s.text_color(t.text)),
                )
        };
        let options_exit = if self.sidebar_options_open { None } else { self.menus.options_exit() };
        let options_menu = (self.sidebar_options_open || options_exit.is_some())
            .then(|| self.render_options_menu(grouped, compact, show_archived, options_exit, &t, cx));
        let header = div()
            .relative()
            .pt(px(14.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .h(px(26.0))
                    .mx(px(8.0))
                    .pl(px(8.0))
                    .pr(px(4.0))
                    .flex()
                    .items_center()
                    .gap(px(2.0))
                    .text_size(ty::CAPTION)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(t.text_faint)
                    .child(div().flex_1().flex().items_center().gap(px(6.0)).child("Threads").when(
                        active_threads > 0,
                        |d| {
                            d.child(
                                div().text_size(ty::BADGE).text_color(t.text_faint).child(active_threads.to_string()),
                            )
                        },
                    ))
                    .child(header_button("sb-search", icons::MAGNIFER, "Search threads", searching).on_click(
                        cx.listener(|this, _, window, cx| {
                            this.search_open = true;
                            this.search.update(cx, |s, cx| s.focus(window, cx));
                            cx.notify();
                        }),
                    ))
                    .child(
                        header_button(
                            "sb-options",
                            icons::MORE_HORIZONTAL,
                            "Sidebar options",
                            self.sidebar_options_open,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.sidebar_options_open {
                                this.close_menus();
                            } else {
                                this.close_menus();
                                this.sidebar_options_open = true;
                                this.menu_opened(window, cx);
                            }
                            cx.notify();
                        })),
                    )
                    .children(options_menu),
            )
            .when(searching, |d| {
                d.child(
                    div()
                        .mx(px(8.0))
                        .h(px(30.0))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .gap(px(9.0))
                        .text_size(ty::BODY)
                        .child(icon(icons::MAGNIFER).size(px(14.0)).text_color(t.text_faint))
                        .child(div().flex_1().min_w_0().child(Input::new(&self.search))),
                )
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
            list = list.child(div().px(px(16.0)).py(px(8.0)).text_size(ty::BODY).text_color(t.text_faint).child(
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
                        .text_size(ty::CAPTION)
                        .text_color(t.text_faint)
                        .child(project_badge(&p.name, 12.0, &t))
                        .child(p.name.clone()),
                );
                for task in in_project {
                    list = list.child(self.render_row(task, compact, &t, cx));
                }
            }
        } else {
            // Pinned threads lead under their own label; the rest follow.
            let pinned = visible.iter().filter(|x| x.pinned).count();
            for (n, task) in visible.iter().enumerate() {
                if n == 0 && pinned > 0 {
                    list = list.child(group_label(icons::PIN, "Pinned", &t));
                }
                if n == pinned && pinned > 0 {
                    list = list.child(div().h(px(6.0)).flex_none());
                }
                list = list.child(self.render_row(task, compact, &t, cx));
            }
        }
        // Archived threads sit on a shelf that folds away (Zeron's archive
        // disclosure); the full list is one click further, on its page.
        if show_archived && !self.archived.is_empty() {
            let shelf_open = self.engine.store.flag("sidebar_archived_open", true);
            let turn = crate::motion::state_t("sb-shelf", shelf_open, crate::motion::CHEVRON, cx.reduce_motion());
            list = list.child(
                div()
                    .id("sb-shelf")
                    .mx(px(8.0))
                    .mt(px(8.0))
                    .h(px(26.0))
                    .px(px(8.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.0))
                    .rounded(px(radius::RADIUS))
                    .cursor_pointer()
                    .text_size(ty::CAPTION)
                    .text_color(t.text_faint)
                    .role(gpui::Role::Button)
                    .aria_expanded(shelf_open)
                    .aria_label("Archived threads")
                    .map(|d| {
                        crate::motion::hover_fade(
                            d,
                            "sb-shelf",
                            (t.wash(0.0), t.wash(0.0)),
                            Some((t.text_faint, t.text_muted)),
                        )
                    })
                    .child(icon(icons::ARCHIVE_MINIMALISTIC).size(px(13.0)))
                    .child(div().flex_1().child("Archived"))
                    .child(div().text_size(ty::BADGE).child(self.archived.len().to_string()))
                    .child(icon(icons::ALT_ARROW_RIGHT).size(px(11.0)).with_transformation(
                        gpui::Transformation::rotate(gpui::radians(turn * std::f32::consts::FRAC_PI_2)),
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let _ = this
                            .engine
                            .store
                            .set_setting("sidebar_archived_open", if shelf_open { "false" } else { "true" });
                        cx.notify();
                    })),
            );
            if shelf_open {
                let mut shelf = div().flex().flex_col().gap(px(1.0));
                for task in self.archived.clone().iter().take(20) {
                    shelf = shelf.child(self.render_row(task, compact, &t, cx));
                }
                if self.archived.len() > 20 {
                    shelf = shelf.child(
                        div()
                            .id("sb-shelf-all")
                            .mx(px(8.0))
                            .h(px(26.0))
                            .px(px(8.0))
                            .flex()
                            .items_center()
                            .rounded(px(radius::RADIUS))
                            .cursor_pointer()
                            .text_size(ty::CAPTION)
                            .text_color(t.text_faint)
                            .hover(|s| s.text_color(t.text_muted))
                            .child(format!("Show all {} archived", self.archived.len()))
                            .on_click(cx.listener(|this, _, window, cx| this.navigate(Route::Archived, window, cx))),
                    );
                }
                list = list.child(crate::motion::reveal("sb-shelf-body", shelf));
            }
        }

        let footer = div()
            .h(px(52.0))
            .pl(px(11.0))
            .pr(px(12.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .id("sb-usage")
                    .flex_1()
                    .min_w_0()
                    .h(px(38.0))
                    .ml(px(-5.0))
                    .px(px(5.0))
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .cursor_pointer()
                    .tooltip(ui::tip("Usage"))
                    .on_click(cx.listener(|this, _, window, cx| this.navigate(Route::Usage, window, cx)))
                    .child({
                        let host = ui::host_name();
                        let initial: String = host
                            .chars()
                            .find(|c| c.is_alphanumeric())
                            .map_or('D', |c| c.to_ascii_uppercase())
                            .to_string();
                        kit::avatar_tile(initial, 0.6, 26.0, &t)
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(ty::BODY)
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .truncate()
                                    .child(format!("Local · {}", ui::host_name())),
                            )
                            .child(
                                div()
                                    .text_size(ty::MICRO)
                                    .text_color(t.text_faint)
                                    .truncate()
                                    .child(format!("Today {}", ui::usage_line(&self.today))),
                            ),
                    ),
            )
            .child(quiet_glyph(
                "sb-add-proj",
                icons::FOLDER_WITH_FILES,
                "Add project (Ctrl+O)",
                &t,
                cx.listener(|this, _, _, cx| this.add_project_dialog(cx)),
            ))
            .child(quiet_glyph(
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
            .relative()
            .w(px(self.sidebar_w))
            .h_full()
            .flex_none()
            .flex()
            .child(
                // No panel of its own: the sidebar is the window's glass, one
                // surface with the titlebar above it, and the main card is the
                // only inset, rounded thing.
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(nav)
                    .child(div().pt(px(14.0)).child(self.render_team_section(&t, cx)))
                    .children(needs_section)
                    .child(header)
                    .child(list)
                    .child(footer),
            )
            // Laid over the edge rather than beside it, so the right
            // gutter matches the left one.
            .child(handle.absolute().top_0().right_0())
            .into_any_element()
    }

    fn render_more_item(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let selected =
            matches!(self.route, Route::Agents | Route::Usage | Route::Archived | Route::Settings | Route::Help);
        let open = self.more_open;
        let item = div()
            .id("nav-more")
            .h(px(30.0))
            .mx(px(8.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(9.0))
            .rounded(px(radius::RADIUS))
            .cursor_pointer()
            .text_size(ty::BODY)
            .group("nav-more")
            .when(selected || open, |d| d.text_color(t.text).font_weight(gpui::FontWeight::MEDIUM))
            .when(!selected && !open, |d| {
                crate::motion::hover_fade(d, "nav-more", (t.wash(0.0), t.wash(0.0)), Some((t.text_muted, t.text)))
            })
            .child(
                icon(icons::MORE_HORIZONTAL)
                    .size(px(16.0))
                    .text_color(if selected || open { t.text } else { t.text_muted })
                    .group_hover("nav-more", |s| s.text_color(t.text)),
            )
            .child(div().flex_1().child("More"))
            .child(icon(icons::ALT_ARROW_RIGHT).size(px(12.0)).text_color(t.text_faint))
            .on_click(cx.listener(|this, _, window, cx| {
                if this.more_open {
                    this.close_menus();
                } else {
                    this.close_menus();
                    this.more_open = true;
                    this.menu_opened(window, cx);
                }
                cx.notify();
            }));
        let exit = if open { None } else { self.menus.more_exit() };
        let menu = (open || exit.is_some()).then(|| {
            use crate::shell_menu::{Item, act};
            let row = |id: &'static str, glyph: &'static str, label: &'static str, route: Route| {
                Item::row(id, Some(glyph), label, act(move |this, window, cx| this.navigate(route, window, cx)))
            };
            let items = vec![
                row("more-agents", icons::BOT, "Agents", Route::Agents),
                row("more-usage", icons::CLOCK_CIRCLE, "Usage", Route::Usage),
                row("more-archived", icons::ARCHIVE_MINIMALISTIC, "Archived", Route::Archived),
                Item::Separator,
                row("more-settings", icons::SETTINGS, "Settings", Route::Settings),
                row("more-help", icons::KEYBOARD, "Help & shortcuts", Route::Help),
            ];
            let body = self.menu_list(items, 220.0, open, t, cx);
            let card = self.menu_surface(t, body, exit);
            deferred(
                anchored().anchor(Anchor::TopLeft).snap_to_window_with_margin(px(8.0)).child(
                    div()
                        .ml(px(self.sidebar_w - 20.0))
                        .when(open, |d| {
                            d.on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.close_menus();
                                cx.notify();
                            }))
                        })
                        .child(card),
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
        exit: Option<f32>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::shell_menu::{Item, act};
        // Toggles stay open so several can be flipped in one visit.
        let toggle = |id: &'static str,
                      label: &'static str,
                      on: bool,
                      key: &'static str,
                      on_value: &'static str,
                      off_value: &'static str| {
            Item::row(
                id,
                Some(if on { icons::CHECK } else { icons::LIST }),
                label,
                act(move |this, _, _| {
                    let _ = this.engine.store.set_setting(key, if on { off_value } else { on_value });
                }),
            )
            .muted(!on)
            .keep_open()
        };
        let items = vec![
            toggle("opt-group", "Group by project", grouped, "sidebar_group", "project", "none"),
            toggle("opt-compact", "Compact rows", compact, "sidebar_density", "compact", "comfortable"),
            toggle("opt-archived", "Show archived", archived, "sidebar_archived", "true", "false"),
            Item::Separator,
            Item::row(
                "opt-add",
                Some(icons::DOCUMENT_ADD),
                "Add project…",
                act(|this, _, cx| this.add_project_dialog(cx)),
            ),
        ];
        let live = exit.is_none();
        let body = self.menu_list(items, 230.0, live, t, cx);
        let card = self.menu_surface(t, body, exit);
        deferred(
            anchored().anchor(Anchor::TopRight).snap_to_window_with_margin(px(8.0)).child(
                div()
                    .mt(px(32.0))
                    .when(live, |d| {
                        d.on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.close_menus();
                            cx.notify();
                        }))
                    })
                    .child(card),
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
            el.text_size(ty::META).on_click(cx.listener(move |this, _, _, cx| {
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
            .mx(px(8.0))
            .px(px(10.0))
            .py(px(8.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .rounded(px(radius::RADIUS))
            .bg(t.warning.opacity(0.05))
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
                    .child(div().flex_1().min_w_0().truncate().text_size(ty::BODY).child(task.title.clone())),
            )
            .child(div().text_size(ty::META).text_color(t.text_muted).line_clamp(2).child(summary))
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
        let corner = div().text_size(ty::CAPTION).text_color(t.text_faint).child(ui::relative_time(task.updated_at));
        let archive_group = SharedString::from(format!("row-archive-{id}"));
        let archive_btn = div()
            .id(archive_group.clone())
            .group(archive_group.clone())
            .size(px(22.0))
            .flex()
            .items_center()
            .justify_center()
            .tooltip(ui::tip(if archived { "Unarchive" } else { "Archive" }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.set_archived(id, !archived, window, cx);
            }))
            .child(
                icon(if archived { icons::ARCHIVE_UP_MINIMALISTIC } else { icons::ARCHIVE_MINIMALISTIC })
                    .size(px(14.0))
                    .text_color(t.text_faint)
                    .group_hover(archive_group, |s| s.text_color(t.text)),
            );

        // The second line says where the thread stands: what it is doing,
        // whether it waits on you, or where it works.
        let (detail, detail_tint): (Option<String>, Hsla) = match task.status {
            TaskStatus::NeedsInput => (Some("Waiting for you".into()), t.warning),
            TaskStatus::Failed => (Some("Failed".into()), t.danger),
            TaskStatus::Running => {
                (Some(step.as_ref().map_or_else(|| "Working…".to_string(), |s| s.to_string())), t.text_faint)
            }
            _ => {
                let project =
                    (self.projects.len() > 1).then(|| self.project(task.project_id).map(|p| p.name.clone())).flatten();
                let parts: Vec<String> = project.into_iter().chain(task.branch.clone()).collect();
                let line = if parts.is_empty() { task.harness.label().to_string() } else { parts.join(" · ") };
                (Some(line), t.text_faint)
            }
        };
        let two_line = !compact && renaming.is_none();
        let row = div()
            .id(SharedString::from(format!("row-{id}")))
            .group(group.clone())
            .h(px(if compact { 32.0 } else { 44.0 }))
            .mx(px(8.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(radius::RADIUS))
            .cursor_pointer()
            .when(selected, |d| d.bg(ui::selected_bg(t)))
            .when(archived && !selected, |d| d.opacity(0.6))
            // Compact rows have no second line: the step a working thread is
            // on, or (with several projects) where it lives, is the tooltip.
            .when_some(
                step.clone()
                    .or_else(|| {
                        (self.projects.len() > 1)
                            .then(|| self.project(task.project_id).map(|p| SharedString::from(p.name.clone())))
                            .flatten()
                    })
                    .filter(|_| compact),
                |d, s| d.tooltip(ui::tip(s)),
            )
            // The status column keeps its width when empty, so titles align.
            .child(div().w(px(16.0)).flex_none().flex().justify_center().children(kit::status_mark(
                SharedString::from(format!("sb-st-{id}")),
                task.status,
                task.unread,
                t,
            )))
            // One mark per row: the teammate, or the agent.
            .child(
                div().relative().flex_none().size(px(16.0)).flex().items_center().justify_center().child(
                    match self.persona_for(id) {
                        Some(p) => crate::team::avatar(p, 16.0, t).into_any_element(),
                        None => icon(ui::harness_icon(task.harness))
                            .size(px(14.0))
                            .text_color(ui::harness_tint(task.harness, t, t.text_muted))
                            .into_any_element(),
                    },
                ),
            )
            .child(match renaming {
                Some(input) => div()
                    .flex_1()
                    .min_w_0()
                    .px(px(4.0))
                    .rounded(px(radius::RADIUS))
                    .bg(t.wash(0.08))
                    .child(Input::new(&input))
                    .into_any_element(),
                None => {
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .text_size(ty::BODY)
                                .when(task.unread, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                                .text_color(if selected || task.unread { t.text } else { t.text_muted })
                                .group_hover(group.clone(), |s| s.text_color(t.text))
                                .child(task.title.clone()),
                        )
                        .when(two_line, |d| {
                            d.children(detail.map(|line| {
                                div().truncate().text_size(ty::CAPTION).text_color(detail_tint).child(line)
                            }))
                        })
                        .into_any_element()
                }
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
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    this.close_menus();
                    this.menu = Some(ChatMenu { task: id, at: e.position, copy_open: false, confirm_delete: false });
                    this.menu_opened(window, cx);
                    cx.notify();
                }),
            );
        // Rows glide in when they move to a new attention bucket (offset and
        // opacity only: the list lays out once).
        div()
            .child(row)
            .with_animation(
                SharedString::from(format!("glide-{id}-{rank}")),
                crate::motion::LIST_REORDER.animation(),
                |el, delta| el.relative().opacity(0.4 + 0.6 * delta).top(px(6.0 * (1.0 - delta))),
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
