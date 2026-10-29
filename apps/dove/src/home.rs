//! The new-thread canvas: a full-bleed wallpaper, the composer, and the
//! detected-agents pill. With no threads open the window has no title bar;
//! the shell floats a small navigation cluster over this page instead.

use dove_core::proto::{HarnessKind, TaskStatus};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, px,
};

use crate::assets::{icon, icons};
use crate::design::{radius, ty};
use crate::shell::Shell;
use crate::theme::Theme;
use crate::ui;
use crate::wallpaper;

impl Shell {
    pub fn render_home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        if self.projects.is_empty() {
            return self.render_onboarding(&t, cx);
        }

        // The shell paints the artwork from the top of the window, through
        // the tab strip. This page only chooses the layout over it.
        let viewport_h = f32::from(window.viewport_size().height);
        let wall_path = self.engine.store.setting("wallpaper_path").filter(|p| std::path::Path::new(p).is_file());
        let animate = self.engine.store.setting("wallpaper_animate").as_deref() == Some("true");
        if !animate || wall_path.is_none() {
            wallpaper::release_animation(window);
        }
        let scenic = wall_path.is_some();

        let join_chip = self.join.as_ref().map(|(_, j)| {
            ui::chip("home-join", &t)
                .child(icon(icons::GIT_BRANCH).size(px(14.0)).text_color(t.accent))
                .child(format!("Side chat on {}", j.branch.clone().unwrap_or_else(|| "this branch".into())))
                .child(icon(icons::CLOSE).size(px(12.0)).text_color(t.text_faint))
                .tooltip(ui::tip("Shares the worktree of the thread you came from"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.join = None;
                    cx.notify();
                }))
        });

        let bounds_cell = self.composer_bounds.clone();
        let composer = div()
            .relative()
            .w_full()
            // Cached at its measured height: a sidebar spinner tick redraws
            // the shell, not the composer.
            .child(match self.composer_height.get() {
                h if h > 0.0 => {
                    self.composer.clone().cached(gpui::StyleRefinement::default().w_full().h(px(h))).into_any_element()
                }
                _ => self.composer.clone().into_any_element(),
            })
            .child(canvas(move |b, _, _| bounds_cell.set(Some(b)), |_, _, _, _| {}).absolute().inset_0());

        let dock =
            crate::composer::home_input_is_dock(self.engine.store.setting(crate::composer::HOME_INPUT).as_deref());
        self.composer.update(cx, |c, cx| c.apply_home_chrome(dock, window, cx));
        let spacer =
            if scenic { (viewport_h * 0.33).clamp(56.0, 380.0) } else { (viewport_h * 0.22).clamp(48.0, 240.0) };
        let error = self.error.clone().map(|e| {
            div()
                .id("home-error")
                .p(px(10.0))
                .rounded(px(radius::RADIUS))
                .bg(t.danger.opacity(0.12))
                .text_color(t.danger)
                .text_size(ty::BODY)
                .cursor_pointer()
                .child(e)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.error = None;
                    cx.notify();
                }))
        });
        let join = join_chip.map(|c| div().flex().justify_end().child(c));
        // A running voice call sits on the input it would start a thread from.
        let call = crate::voice_call::strip(cx);

        // Center: the bar sits a little above the middle. Bottom: the same
        // page, with the bar parked at the center of the bottom edge.
        // The bar centres in the main area beside the sidebar, at full width.
        let bar_max = 820.0;
        let mut page = div().relative().size_full().flex().flex_col().items_center().child(div().h(px(spacer)));
        let cards = self.render_home_cards(scenic, &t, window, cx);
        if dock {
            page = page
                .child(div().w_full().max_w(px(bar_max)).px(px(32.0)).flex().flex_col().gap(px(8.0)).children(join))
                .child(div().flex_1().w_full().flex().flex_col().items_center().justify_center().children(cards))
                .child(
                    div()
                        .w_full()
                        .max_w(px(bar_max))
                        .px(px(20.0))
                        .pb(px(20.0))
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .children(call)
                        .child(composer)
                        .children(error),
                );
        } else {
            page = page
                .child(
                    div()
                        .w_full()
                        .max_w(px(bar_max))
                        .px(px(32.0))
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .children(join)
                        .children(call)
                        .child(composer)
                        .children(error),
                )
                .child(div().w_full().flex().flex_col().items_center().pt(px(20.0)).children(cards))
                .child(div().flex_1());
        }
        // The agents-ready strip sits under the centered composer. In the
        // bottom layout it lands on the input, so it stays off that page.
        if !dock {
            page = page.child(self.render_agents_pill(&t, cx));
        }
        page.into_any_element()
    }

    /// The quiet cards under (or, docked, above) the composer: where the
    /// new thread will run, and the threads that want you. Dove's pixel
    /// mark perches on the first card.
    fn render_home_cards(
        &mut self,
        scenic: bool,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let current = self.home_project.or_else(|| self.projects.first().map(|p| p.id));
        // The current project first, then the rest as the sidebar has them.
        // Chats (no folder) has its own first row, not a project row.
        let chats_on = current.is_some_and(|id| self.is_chats_project(id));
        let mut projects: Vec<_> = self.projects.iter().filter(|p| !self.is_chats_project(p.id)).cloned().collect();
        projects.sort_by_key(|p| Some(p.id) != current);
        projects.truncate(HOME_PROJECTS);
        let card_bg =
            if scenic { t.glass_overlay() } else { t.surface_card.opacity(if t.is_frost() { 0.72 } else { 1.0 }) };
        let card = |id: &'static str| {
            div()
                .id(id)
                .w_full()
                .p(px(4.0))
                .flex()
                .flex_col()
                .rounded(px(radius::CARD))
                .border_1()
                .border_color(t.border)
                .bg(card_bg)
        };
        let hover = t.wash(0.07);
        let row = |id: SharedString| {
            div()
                .id(id)
                .h(px(34.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .gap(px(10.0))
                .rounded(px(radius::CARD - 4.0))
                .text_size(ty::BODY)
                .text_color(t.text)
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
        };
        let mut place = card("home-place").child(
            row("home-no-folder".into())
                .child(icon(icons::CHAT_ROUND_LINE).size(px(15.0)).text_color(if chats_on {
                    t.text
                } else {
                    t.text_muted
                }))
                .child(div().flex_1().min_w_0().truncate().child("No folder"))
                .child(
                    div()
                        .flex_none()
                        .text_size(ty::CAPTION)
                        .text_color(t.text_faint)
                        .child("Just chat · files go to Artifacts"),
                )
                .when(chats_on, |d| d.child(icon(icons::CHECK).size(px(14.0)).text_color(t.text)))
                .on_click(cx.listener(|this, _, _, cx| {
                    let dir = crate::pages::artifacts::chats_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    if let Ok(p) = this.engine.store.add_project(&dir) {
                        this.reload(cx);
                        this.set_home_project(p.id, cx);
                        let projects = this.projects.clone();
                        this.composer.update(cx, |c, cx| c.set_projects(projects, Some(p.id), cx));
                    }
                })),
        );
        for p in &projects {
            let id = p.id;
            let on = Some(id) == current;
            place = place.child(
                row(SharedString::from(format!("home-proj-{id}")))
                    .child(icon(icons::LAPTOP).size(px(15.0)).text_color(if on { t.text } else { t.text_muted }))
                    .child(div().flex_1().min_w_0().truncate().child(p.name.clone()))
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(160.0))
                            .truncate()
                            .text_size(ty::CAPTION)
                            .text_color(t.text_faint)
                            .child(short_path(&p.path)),
                    )
                    .when(on, |d| d.child(icon(icons::CHECK).size(px(14.0)).text_color(t.text)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_home_project(id, cx);
                        let projects = this.projects.clone();
                        this.composer.update(cx, |c, cx| c.set_projects(projects, Some(id), cx));
                    })),
            );
        }
        place = place.child(div().mx(px(8.0)).my(px(4.0)).h(px(1.0)).bg(t.border)).child(
            row("home-add-proj".into())
                .text_color(t.text_muted)
                .child(icon(icons::FOLDER_WITH_FILES).size(px(15.0)))
                .child(div().flex_1().child("Open another folder…"))
                .child(div().text_size(ty::MICRO).text_color(t.text_faint).child(crate::keymap::pretty("secondary-o")))
                .on_click(cx.listener(|this, _, _, cx| this.add_project_dialog(cx))),
        );

        // Threads that want you first, then what is running, then the latest.
        let mut recent: Vec<_> = self
            .tasks
            .iter()
            .filter(|x| !matches!(x.status, TaskStatus::Draft | TaskStatus::Archived))
            .cloned()
            .collect();
        crate::sidebar::attention_sort(&mut recent);
        recent.truncate(HOME_RECENTS);
        let mut lines = Vec::new();
        for task in &recent {
            let id = task.id;
            let step = (task.status == TaskStatus::Running)
                .then(|| self.threads.get(&id).and_then(|v| v.read(cx).chat.read(cx).current_step()))
                .flatten();
            let (word, tint, detail): (Option<&'static str>, _, String) = match task.status {
                TaskStatus::NeedsInput => (Some("Waiting"), t.warning, "Asked a question".into()),
                TaskStatus::Running => {
                    (Some("Running"), t.accent, step.map(|s| s.to_string()).unwrap_or_else(|| "Working…".into()))
                }
                TaskStatus::Failed => (Some("Failed"), t.danger, "Stopped with an error".into()),
                _ => (None, t.text_faint, ui::relative_time(task.updated_at)),
            };
            lines.push(
                row(SharedString::from(format!("home-task-{id}")))
                    .children(
                        word.map(|w| div().flex_none().text_color(tint).font_weight(gpui::FontWeight::MEDIUM).child(w)),
                    )
                    .child(div().flex_shrink(1.0).min_w_0().truncate().child(task.title.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(40.0))
                            .truncate()
                            .text_size(ty::CAPTION)
                            .text_color(t.text_faint)
                            .child(detail),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.navigate(crate::shell::Route::Thread(id), window, cx)
                    })),
            );
        }
        let attention = (!lines.is_empty()).then(|| card("home-recent").children(lines));

        let dove = window.use_keyed_state("home-dove", cx, |window, cx| crate::mascot::Dove::new(3.0, window, cx));
        let (dove_w, dove_h) = dove.read(cx).size();
        Some(
            div()
                .w_full()
                .max_w(px(HOME_CARD_W))
                .px(px(16.0))
                .pt(px(dove_h - 4.0))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .relative()
                        .child(place)
                        // Perched on the card's top edge, feet on the line.
                        .child(div().absolute().top(px(-(dove_h - 4.0))).right(px(24.0)).w(px(dove_w)).child(dove)),
                )
                .children(attention)
                .into_any_element(),
        )
    }

    fn render_agents_pill(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if !self.updates.is_empty() {
            return self.render_updates_pill(t, cx);
        }
        let ready: Vec<_> = self.installed.iter().filter(|i| dove_core::harness_supported(i.kind)).collect();
        let has_marks = !ready.is_empty();
        let marks = stacked_marks(ready.iter().take(4).map(|i| i.kind), t);
        let label = if self.installed.is_empty() {
            "Looking for agents…".to_string()
        } else {
            format!("{} agents ready · {}", ready.len(), ui::host_name())
        };
        let list = self.agents_open.then(|| {
            div().mb(px(8.0)).w(px(PILL_MENU_W)).child(ui::menu_card(
                t,
                div().flex().flex_col().children(self.installed.iter().map(|i| {
                    let ok = dove_core::harness_supported(i.kind);
                    div()
                        .h(px(34.0))
                        .px(px(14.0))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .text_size(ty::MD)
                        .when(!ok, |d| d.opacity(0.5))
                        .child(icon(ui::harness_icon(i.kind)).text_color(ui::harness_tint(i.kind, t, t.text)))
                        .child(div().flex_1().child(i.kind.label()))
                        .child(
                            div().text_size(ty::META).text_color(t.text_faint).child(
                                self.versions
                                    .get(&i.kind)
                                    .cloned()
                                    .unwrap_or_else(|| if ok { "ready".into() } else { "not supported yet".into() }),
                            ),
                        )
                })),
            ))
        });
        div()
            .pb(px(20.0))
            .flex()
            .flex_col()
            .items_center()
            .children(list)
            .child(ui::frosted(
                PILL_H / 2.0,
                16.0,
                pill_body("agents-pill", t)
                    .when(!has_marks, |d| d.pl(px(14.0)))
                    .when(has_marks, |d| d.child(marks))
                    .child(div().flex_none().whitespace_nowrap().child(label))
                    .child(pill_chevron(self.agents_open, t))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.agents_open = !this.agents_open;
                        cx.notify();
                    })),
            ))
            .into_any_element()
    }

    /// Zeron's agent-update island: overlapping marks and "3 agent updates
    /// · HOST"; open, a row per agent with its versions and an Update action.
    fn render_updates_pill(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let n = self.updates.len();
        let title = if n == 1 {
            let u = &self.updates[0];
            format!("{} {} available · {}", u.kind.label(), u.latest, ui::host_name())
        } else {
            format!("{n} agent updates · {}", ui::host_name())
        };
        let marks = stacked_marks(self.updates.iter().take(3).map(|u| u.kind), t);
        let list = self.agents_open.then(|| {
            div().mb(px(8.0)).w(px(PILL_MENU_W)).child(ui::menu_card(
                t,
                div().flex().flex_col().children(self.updates.iter().map(|u| {
                    let kind = u.kind;
                    div()
                        .h(px(56.0))
                        .px(px(14.0))
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .child(mark(kind, t))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(div().text_size(ty::BODY).text_color(t.text).child(kind.label()))
                                .child(
                                    div()
                                        .text_size(ty::META)
                                        .text_color(t.text_faint)
                                        .child(format!("{} → {}", u.installed, u.latest)),
                                ),
                        )
                        .child(
                            ui::button_primary(SharedString::from(format!("update-{}", kind.id())), "Update", t)
                                .h(px(26.0))
                                .text_size(ty::META)
                                .on_click(cx.listener(move |this, _, window, cx| this.run_update(kind, window, cx))),
                        )
                })),
            ))
        });
        div()
            .pb(px(20.0))
            .flex()
            .flex_col()
            .items_center()
            .children(list)
            .child(ui::frosted(
                PILL_H / 2.0,
                16.0,
                pill_body("updates-pill", t)
                    .child(marks)
                    .child(div().flex_none().whitespace_nowrap().child(title))
                    .child(pill_chevron(self.agents_open, t))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.agents_open = !this.agents_open;
                        cx.notify();
                    })),
            ))
            .into_any_element()
    }

    fn render_onboarding(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .child(icon(icons::FOLDER_WITH_FILES).size(px(36.0)).text_color(t.text.opacity(0.28)))
            .child(
                div()
                    .mt(px(18.0))
                    .text_size(ty::HEADING)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("Add a project to get started"),
            )
            .child(div().text_size(ty::BODY).text_color(t.text_muted).child("A project is any folder on this device."))
            .child(
                ui::button_primary("onboard-add", "Add a project", t)
                    .mt(px(16.0))
                    .on_click(cx.listener(|this, _, _, cx| this.add_project_dialog(cx))),
            )
            .into_any_element()
    }
}

/// The home cards' column width and how much each lists.
const HOME_CARD_W: f32 = 460.0;
const HOME_PROJECTS: usize = 4;
const HOME_RECENTS: usize = 3;

/// `~/code/dove` rather than the full path.
fn short_path(path: &std::path::Path) -> String {
    let full = path.display().to_string();
    let home = std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
    match full.strip_prefix(&home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => full,
    }
}

/// The island at the foot of the home page: one height, corner and inset
/// for both the agents and the updates variants.
const PILL_H: f32 = 36.0;
const PILL_MENU_W: f32 = 352.0;
/// Agent marks in the island: discs that overlap a little, so the stack
/// has an exact width and never runs into the label after it.
const MARK: f32 = 22.0;
const MARK_STEP: f32 = 17.0;

fn pill_body(id: &'static str, t: &Theme) -> gpui::Stateful<gpui::Div> {
    let hover = t.border_strong;
    div()
        .id(id)
        .h(px(PILL_H))
        // The first disc sits concentric with the pill's round end.
        .pl(px((PILL_H - MARK) / 2.0 - 1.0))
        .pr(px(14.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.0))
        .rounded(px(PILL_H / 2.0))
        .bg(t.glass_overlay())
        .border_1()
        .border_color(t.border)
        .text_size(ty::SMALL)
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(t.text)
        .cursor_pointer()
        .hover(move |s| s.border_color(hover))
}

fn pill_chevron(open: bool, t: &Theme) -> impl IntoElement {
    icon(if open { icons::ALT_ARROW_DOWN } else { icons::ALT_ARROW_UP })
        .size(px(12.0))
        .flex_none()
        .text_color(t.text_faint)
}

fn mark(kind: HarnessKind, t: &Theme) -> gpui::Div {
    div()
        .size(px(MARK))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(t.surface_raised)
        .border_1()
        .border_color(t.border_strong)
        .child(icon(ui::harness_icon(kind)).size(px(13.0)).text_color(ui::harness_tint(kind, t, t.text)))
}

/// Overlapping agent marks with a fixed footprint. Each disc is placed
/// rather than pulled left with a negative margin, so the label can't slide
/// underneath the stack.
fn stacked_marks(kinds: impl Iterator<Item = HarnessKind>, t: &Theme) -> gpui::Div {
    let kinds: Vec<HarnessKind> = kinds.collect();
    let width = if kinds.is_empty() { 0.0 } else { MARK + MARK_STEP * (kinds.len() - 1) as f32 };
    div().relative().flex_none().w(px(width)).h(px(MARK)).children(
        kinds
            .into_iter()
            .enumerate()
            .map(|(i, kind)| div().absolute().top_0().left(px(MARK_STEP * i as f32)).child(mark(kind, t))),
    )
}
