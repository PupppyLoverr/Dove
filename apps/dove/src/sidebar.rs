//! The sidebar: navigation, "Needs you", attention-sorted threads with
//! search and options, and the device footer. Collapsible to zero (Ctrl+B)
//! and resizable (208–400 px).
//!
//! Threads another thread started (delegated tasks, side chats) and the
//! subagents a thread runs sit folded under it, one click from open.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use dove_core::proto::{ProjectId, TaskId, TaskStatus};
use dove_core::store::Task;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnimationExt as _, AnyElement, App, AppContext as _, Context, Div, Entity, Hsla, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, anchored, deferred, div, hsla, px,
};
use gpui_base::input::Input;

use crate::assets::{icon, icons};
use crate::design::{radius, ty};
use crate::kit;
use crate::shell::{ChatMenu, Route, Shell};
use crate::theme::Theme;
use crate::ui;

/// Each nested thread's parent: a delegated task under the thread that
/// delegated it (`links`, child to parent), and a side chat (a thread
/// sharing a worktree) under the first thread in that worktree. Ids are
/// time-ordered, so the smallest is the first. A chain that loops back is
/// cut, so every thread still shows once.
pub fn nest(tasks: &[(TaskId, Option<PathBuf>)], links: &[(TaskId, TaskId)]) -> HashMap<TaskId, TaskId> {
    let known: HashSet<TaskId> = tasks.iter().map(|(id, _)| *id).collect();
    let mut parent: HashMap<TaskId, TaskId> = HashMap::new();
    for (child, p) in links {
        if child != p && known.contains(child) && known.contains(p) {
            parent.entry(*child).or_insert(*p);
        }
    }
    let mut first: HashMap<&PathBuf, TaskId> = HashMap::new();
    for (id, wt) in tasks {
        if let Some(wt) = wt {
            first.entry(wt).and_modify(|f| *f = (*f).min(*id)).or_insert(*id);
        }
    }
    for (id, wt) in tasks {
        if let Some(owner) = wt.as_ref().and_then(|w| first.get(w)).copied().filter(|o| o != id) {
            parent.entry(*id).or_insert(owner);
        }
    }
    // Cut loops: walk up from each thread; one that comes back to itself
    // keeps no parent.
    let ids: Vec<TaskId> = parent.keys().copied().collect();
    for id in ids {
        let mut seen = HashSet::from([id]);
        let mut at = id;
        while let Some(p) = parent.get(&at).copied() {
            if !seen.insert(p) {
                parent.remove(&id);
                break;
            }
            at = p;
        }
    }
    parent
}

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

/// The sidebar as its own cached view. The shell re-renders on every frame
/// of anything below it (a streaming thread, a keystroke in the composer);
/// the sidebar re-renders only when the shell itself changed, when one of
/// its own spinners ticks, or when a working thread's step line moves.
/// What the sidebar list shows (Projects row and its menu). Persisted as
/// `sidebar_show` (all / projects / chats / p:<id>), `sidebar_status`
/// (any / needs / running / failed / done), `sidebar_group` and
/// `sidebar_archived`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Show {
    All,
    Projects,
    Chats,
    Project(ProjectId),
}

#[derive(Clone, Debug)]
pub struct SidebarFilter {
    pub show: Show,
    pub status: String,
    pub group_by_project: bool,
    pub archived: bool,
}

impl SidebarFilter {
    pub fn read(shell: &Shell) -> Self {
        let get = |k: &str| shell.engine.store.setting(k);
        let show = match get("sidebar_show").as_deref() {
            Some("projects") => Show::Projects,
            Some("chats") => Show::Chats,
            Some(v) if v.starts_with("p:") => {
                shell.projects.iter().find(|p| p.id.to_string() == v[2..]).map_or(Show::All, |p| Show::Project(p.id))
            }
            _ => Show::All,
        };
        let status = get("sidebar_status").filter(|s| ["needs", "running", "failed", "done"].contains(&s.as_str()));
        SidebarFilter {
            show,
            status: status.unwrap_or_else(|| "any".into()),
            group_by_project: get("sidebar_group").as_deref() != Some("none"),
            archived: get("sidebar_archived").as_deref() == Some("true"),
        }
    }

    /// Anything narrowing the list (grouping is a layout, not a filter).
    pub fn is_active(&self) -> bool {
        self.show != Show::All || self.status != "any" || self.archived
    }

    pub fn admits(&self, t: &Task, shell: &Shell) -> bool {
        let place = match self.show {
            Show::All => true,
            Show::Projects => !shell.is_chats_project(t.project_id),
            Show::Chats => shell.is_chats_project(t.project_id),
            Show::Project(id) => t.project_id == id,
        };
        place
            && match self.status.as_str() {
                "needs" => t.status == TaskStatus::NeedsInput,
                "running" => t.status == TaskStatus::Running,
                "failed" => t.status == TaskStatus::Failed,
                "done" => t.status == TaskStatus::Done,
                _ => true,
            }
    }
}

pub struct Sidebar {
    shell: WeakEntity<Shell>,
    /// The step lines the last render showed, per working thread.
    pub(crate) steps: Vec<(TaskId, Option<SharedString>)>,
    _shell_changed: Subscription,
}

impl Sidebar {
    pub fn new(shell: &Entity<Shell>, cx: &mut Context<Self>) -> Self {
        Self { shell: shell.downgrade(), steps: Vec::new(), _shell_changed: cx.observe(shell, |_, _, cx| cx.notify()) }
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("Sidebar");
        let Some(shell) = self.shell.upgrade() else { return div().into_any_element() };
        let id = cx.entity_id();
        crate::motion::owned_render(id, || {
            shell.update(cx, |shell, cx| {
                self.steps = shell.sidebar_steps(cx);
                shell.render_sidebar(cx)
            })
        })
    }
}

impl Shell {
    /// What each working thread is doing now, as its sidebar row shows it.
    pub(crate) fn sidebar_steps(&self, cx: &App) -> Vec<(TaskId, Option<SharedString>)> {
        self.tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Running)
            .map(|t| (t.id, self.threads.get(&t.id).and_then(|v| v.read(cx).chat.read(cx).current_step())))
            .collect()
    }

    pub fn render_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let compact = self.engine.store.setting("sidebar_density").as_deref() == Some("compact");
        let filter = SidebarFilter::read(self);

        // The six fixed rows: New, Search, Projects, Artifacts, Customize,
        // More. 36 px, an 18 px outline glyph, the label; only New rests
        // filled, so it reads as the primary action.
        let row = |id: &'static str,
                   glyph: &'static str,
                   label: &'static str,
                   selected: bool,
                   trailing: Option<AnyElement>| {
            div()
                .id(id)
                .group(id)
                .h(px(36.0))
                .mx(px(8.0))
                .px(px(10.0))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(10.0))
                .rounded(px(radius::ROW))
                .cursor_pointer()
                .text_size(ty::BODY)
                .role(gpui::Role::Button)
                .aria_label(label)
                .when(selected, |d| d.bg(ui::selected_bg(&t)).text_color(t.text).font_weight(gpui::FontWeight::MEDIUM))
                .when(!selected, |d| d.text_color(t.text_muted).hover(|s| s.bg(t.wash(0.05)).text_color(t.text)))
                .child(
                    icon(glyph)
                        .size(px(18.0))
                        .flex_none()
                        .text_color(if selected { t.text } else { t.text_muted })
                        .group_hover(id, |s| s.text_color(t.text)),
                )
                .child(div().flex_1().min_w_0().truncate().child(label))
                .children(trailing)
        };
        let new_row = div()
            .id("nav-new")
            .h(px(36.0))
            .mx(px(8.0))
            .px(px(10.0))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.0))
            .rounded(px(radius::ROW))
            .cursor_pointer()
            .text_size(ty::BODY)
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(t.text)
            .bg(t.wash(0.07))
            .hover(|s| s.bg(t.wash(0.11)))
            .role(gpui::Role::Button)
            .aria_label("New thread")
            .child(icon(icons::PLUS).size(px(18.0)).flex_none().text_color(t.text))
            .child(div().flex_1().child("New"))
            .when(self.sidebar_w >= 280.0, |d| d.child(kit::kbd(&crate::keymap::pretty("secondary-n"), &t)))
            .on_click(cx.listener(|this, _, window, cx| this.new_session(window, cx)));
        let artifacts = crate::pages::artifacts::scan(&crate::pages::artifacts::chats_dir(), 999).len();
        let filter_open = self.sidebar_options_open;
        let filter_exit = if filter_open { None } else { self.menus.options_exit() };
        let filter_menu =
            (filter_open || filter_exit.is_some()).then(|| self.render_filter_menu(&filter, filter_exit, &t, cx));
        let nav =
            div()
                .flex()
                .flex_none()
                .flex_col()
                .gap(px(2.0))
                .pt(px(8.0))
                .child(new_row)
                .child(row("nav-search", icons::MAGNIFER, "Search", false, None).on_click(
                    cx.listener(|this, _, window, cx| this.open_search(crate::palette::Scope::All, window, cx)),
                ))
                .child(
                    div()
                        .relative()
                        .child(
                            row(
                                "nav-projects",
                                icons::PROJECTS,
                                "Projects",
                                filter_open,
                                filter.is_active().then(|| ui::dot(t.accent, 6.0).into_any_element()),
                            )
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_filter_menu(window, cx))),
                        )
                        .children(filter_menu),
                )
                .child(
                    row(
                        "nav-artifacts",
                        icons::ARTIFACTS,
                        "Artifacts",
                        self.route == Route::Artifacts,
                        (artifacts > 0).then(|| {
                            div()
                                .text_size(ty::MICRO)
                                .text_color(t.text_faint)
                                .child(artifacts.to_string())
                                .into_any_element()
                        }),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.pages.artifact_open = None;
                        this.navigate(Route::Artifacts, window, cx);
                    })),
                )
                .child(
                    row(
                        "nav-customize",
                        icons::CUSTOMIZE,
                        "Customize",
                        self.route == Route::Settings
                            && self.settings.read(cx).section == crate::settings::Section::Appearance,
                        None,
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.open_settings_section("appearance", window, cx))),
                )
                .child(self.render_more_item(&t, cx));

        // The list: Pinned, one section per project, then Chats (threads
        // with no folder). Status reads as a word in each row, so there is
        // no separate Needs-you block.
        let mut visible: Vec<Task> = self
            .tasks
            .iter()
            .filter(|x| x.status != TaskStatus::Draft)
            .filter(|x| filter.admits(x, self))
            .cloned()
            .collect();
        if filter.archived {
            visible.extend(self.archived.iter().filter(|x| filter.admits(x, self)).cloned());
        }
        attention_sort(&mut visible);
        // Nested threads follow their parent; a parent not in the list
        // (archived, or filtered out) leaves them on top.
        let parent_of = self.thread_family();
        let shown: HashSet<TaskId> = visible.iter().map(|x| x.id).collect();
        let mut kids: HashMap<TaskId, Vec<Task>> = HashMap::new();
        for x in &visible {
            if let Some(p) = parent_of.get(&x.id).filter(|p| shown.contains(p)) {
                kids.entry(*p).or_default().push(x.clone());
            }
        }
        visible.retain(|x| parent_of.get(&x.id).is_none_or(|p| !shown.contains(p)));

        let mut list = div()
            .id("sb-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(1.0))
            .pb(px(8.0))
            .pt(px(10.0));
        if filter.is_active() {
            list = list.child(self.render_filter_chip(&filter, &t, cx));
        }
        if visible.is_empty() {
            list = list.child(div().px(px(18.0)).py(px(8.0)).text_size(ty::BODY).text_color(t.text_faint).child(
                if self.projects.is_empty() && self.tasks.is_empty() {
                    "Start a thread with New"
                } else if filter.is_active() {
                    "No threads match the filter"
                } else {
                    "No threads yet"
                },
            ));
        }
        let section = |list: Stateful<Div>,
                       id: String,
                       lead: Option<AnyElement>,
                       label: String,
                       tasks: Vec<&Task>,
                       this: &Self,
                       cx: &mut Context<Self>| {
            if tasks.is_empty() {
                return list;
            }
            let key = format!("sidebar_closed.{id}");
            let open = this.engine.store.setting(&key).as_deref() != Some("1");
            let mut list = list.child(this.section_header(
                SharedString::from(format!("sb-{id}")),
                lead,
                label.into(),
                tasks.len(),
                open,
                key,
                &t,
                cx,
            ));
            if open {
                for task in tasks {
                    list = list.children(this.render_branch(task, 0, &kids, compact, &t, cx));
                }
            }
            list
        };
        let pinned: Vec<&Task> = visible.iter().filter(|x| x.pinned).collect();
        list = section(list, "pinned".into(), None, "Pinned".into(), pinned, self, cx);
        let rest: Vec<&Task> = visible.iter().filter(|x| !x.pinned).collect();
        if filter.group_by_project {
            let projects: Vec<_> = self.projects.iter().filter(|p| !self.is_chats_project(p.id)).cloned().collect();
            for p in projects.iter() {
                let in_project: Vec<&Task> = rest.iter().copied().filter(|x| x.project_id == p.id).collect();
                list = section(list, format!("p{}", p.id), None, p.name.clone(), in_project, self, cx);
            }
            let chats: Vec<&Task> = rest.iter().copied().filter(|x| self.is_chats_project(x.project_id)).collect();
            list = section(list, "chats".into(), None, "Chats".into(), chats, self, cx);
        } else {
            list = section(list, "recent".into(), None, "Recent".into(), rest, self, cx);
        }

        // Footer: avatar, name · Local, a chevron for the account menu; a
        // feedback glyph on the right.
        let account_open = self.account_open;
        let account_exit = if account_open { None } else { self.menus.account_exit() };
        let account_menu =
            (account_open || account_exit.is_some()).then(|| self.render_account_menu(account_exit, &t, cx));
        let host = ui::host_name();
        let initial: String =
            host.chars().find(|c| c.is_alphanumeric()).map_or('D', |c| c.to_ascii_uppercase()).to_string();
        let footer = div()
            .relative()
            .flex_none()
            .h(px(52.0))
            .pl(px(10.0))
            .pr(px(10.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .id("sb-account")
                    .flex_1()
                    .min_w_0()
                    .h(px(38.0))
                    .px(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .rounded(px(radius::ROW))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.wash(0.05)))
                    .when(account_open, |d| d.bg(t.wash(0.07)))
                    .role(gpui::Role::Button)
                    .aria_label("Account")
                    .child(kit::avatar_tile(initial, 0.6, 24.0, &t))
                    .child(
                        div().flex_1().min_w_0().truncate().text_size(ty::BODY).child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.0))
                                .child(div().font_weight(gpui::FontWeight::MEDIUM).text_color(t.text).child(host))
                                .child(div().text_color(t.text_faint).child("· Local"))
                                .child(icon(icons::ALT_ARROW_DOWN).size(px(12.0)).text_color(t.text_faint)),
                        ),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        let was = this.account_open;
                        this.close_menus();
                        if !was {
                            this.account_open = true;
                            this.menu_opened(window, cx);
                        }
                        cx.notify();
                    })),
            )
            .child(quiet_glyph(
                "sb-feedback",
                icons::ACTION_DEBUG,
                "Send feedback",
                &t,
                cx.listener(|_, _, _, cx| cx.open_url("https://github.com/PupppyLoverr/Dove/issues/new")),
            ))
            .children(account_menu);

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
                // surface with the titlebar above it.
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(nav)
                    .children(self.render_agents_section(&t, cx))
                    .child(list)
                    .child(footer),
            )
            // Laid over the edge rather than beside it, so the right
            // gutter matches the left one.
            .child(handle.absolute().top_0().right_0())
            .into_any_element()
    }

    fn toggle_filter_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was = self.sidebar_options_open;
        self.close_menus();
        if !was {
            self.sidebar_options_open = true;
            self.menu_opened(window, cx);
        }
        cx.notify();
    }

    /// The sidebar's one filter: which threads (all, projects, chats, one
    /// project), which status, how they group, and archived ones.
    fn render_filter_menu(
        &self,
        filter: &SidebarFilter,
        exit: Option<f32>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::shell_menu::{Item, act};
        let pick = |id: &'static str, label: String, on: bool, key: &'static str, value: String| {
            Item::row(
                id,
                Some(if on { icons::CHECK } else { icons::LIST }),
                label,
                act(move |this, _, _| {
                    let _ = this.engine.store.set_setting(key, &value);
                }),
            )
            .muted(!on)
            .keep_open()
        };
        let mut items = vec![
            pick("flt-all", "All threads".into(), filter.show == Show::All, "sidebar_show", "all".into()),
            pick("flt-proj", "Projects only".into(), filter.show == Show::Projects, "sidebar_show", "projects".into()),
            pick("flt-chats", "Chats only".into(), filter.show == Show::Chats, "sidebar_show", "chats".into()),
        ];
        for p in self.projects.iter().filter(|p| !self.is_chats_project(p.id)) {
            let on = filter.show == Show::Project(p.id);
            items.push(pick("flt-p", format!("Only {}", p.name), on, "sidebar_show", format!("p:{}", p.id)));
        }
        items.push(Item::Separator);
        for (id, label, value) in [
            ("flt-any", "Any status", "any"),
            ("flt-needs", "Needs you", "needs"),
            ("flt-run", "Running", "running"),
            ("flt-failed", "Failed", "failed"),
            ("flt-done", "Done", "done"),
        ] {
            items.push(pick(id, label.into(), filter.status == value, "sidebar_status", value.into()));
        }
        items.push(Item::Separator);
        items.push(pick(
            "flt-group",
            "Group by project".into(),
            filter.group_by_project,
            "sidebar_group",
            if filter.group_by_project { "none".into() } else { "project".into() },
        ));
        items.push(pick(
            "flt-arch",
            "Include archived".into(),
            filter.archived,
            "sidebar_archived",
            if filter.archived { "false".into() } else { "true".into() },
        ));
        items.push(Item::Separator);
        items.push(Item::row(
            "flt-add",
            Some(icons::DOCUMENT_ADD),
            "Add project…",
            act(|this, _, cx| this.add_project_dialog(cx)),
        ));
        let live = exit.is_none();
        let body = self.menu_list(items, 240.0, live, t, cx);
        let card = self.menu_surface(t, body, exit);
        deferred(
            anchored().anchor(Anchor::TopLeft).snap_to_window_with_margin(px(8.0)).child(
                div()
                    .ml(px(self.sidebar_w - 20.0))
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

    /// The quiet line that says a filter is on, with one click to clear it.
    fn render_filter_chip(&self, filter: &SidebarFilter, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mut parts: Vec<String> = Vec::new();
        match filter.show {
            Show::All => {}
            Show::Projects => parts.push("Projects".into()),
            Show::Chats => parts.push("Chats".into()),
            Show::Project(id) => {
                parts.push(self.project(id).map(|p| p.name.clone()).unwrap_or_else(|| "Project".into()))
            }
        }
        if filter.status != "any" {
            parts.push(
                match filter.status.as_str() {
                    "needs" => "Needs you",
                    "running" => "Running",
                    "failed" => "Failed",
                    _ => "Done",
                }
                .into(),
            );
        }
        if filter.archived {
            parts.push("Archived".into());
        }
        div()
            .id("sb-filter-chip")
            .mx(px(16.0))
            .mb(px(4.0))
            .h(px(24.0))
            .px(px(8.0))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .rounded(px(radius::ROW))
            .bg(t.wash(0.06))
            .cursor_pointer()
            .text_size(ty::CAPTION)
            .text_color(t.text_muted)
            .hover(|s| s.bg(t.wash(0.09)).text_color(t.text))
            .tooltip(ui::tip("Clear the filter"))
            .child(icon(icons::TUNING).size(px(12.0)))
            .child(div().flex_1().min_w_0().truncate().child(parts.join(" · ")))
            .child(icon(icons::CLOSE).size(px(11.0)))
            .on_click(cx.listener(|this, _, _, cx| {
                for (k, v) in [("sidebar_show", "all"), ("sidebar_status", "any"), ("sidebar_archived", "false")] {
                    let _ = this.engine.store.set_setting(k, v);
                }
                cx.notify();
            }))
            .into_any_element()
    }

    /// The footer's account menu: settings, usage, theme, projects.
    fn render_account_menu(&self, exit: Option<f32>, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use crate::shell_menu::{Item, act};
        let usage = format!("Today {}", ui::usage_line(&self.today));
        let items = vec![
            Item::row(
                "acc-usage",
                Some(icons::CLOCK_CIRCLE),
                usage,
                act(|this, window, cx| this.navigate(Route::Usage, window, cx)),
            ),
            Item::Separator,
            Item::row(
                "acc-settings",
                Some(icons::SETTINGS),
                "Settings",
                act(|this, window, cx| this.navigate(Route::Settings, window, cx)),
            ),
            Item::row(
                "acc-appearance",
                Some(icons::CUSTOMIZE),
                "Appearance",
                act(|this, window, cx| this.open_settings_section("appearance", window, cx)),
            ),
            Item::row(
                "acc-project",
                Some(icons::FOLDER_WITH_FILES),
                "Add project…",
                act(|this, _, cx| this.add_project_dialog(cx)),
            ),
            Item::Separator,
            Item::row(
                "acc-help",
                Some(icons::KEYBOARD),
                "Help & shortcuts",
                act(|this, window, cx| this.navigate(Route::Help, window, cx)),
            ),
        ];
        let live = exit.is_none();
        let body = self.menu_list(items, 230.0, live, t, cx);
        let card = self.menu_surface(t, body, exit);
        deferred(
            anchored().anchor(Anchor::BottomLeft).snap_to_window_with_margin(px(8.0)).child(
                div()
                    .mb(px(48.0))
                    .ml(px(8.0))
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

    /// A sidebar section's label: a glyph, the name and a count, with a
    /// caret that folds the section. The fold is remembered under `key`.
    #[allow(clippy::too_many_arguments)]
    fn section_header(
        &self,
        id: SharedString,
        lead: Option<AnyElement>,
        label: SharedString,
        count: usize,
        open: bool,
        key: String,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let turn = crate::motion::state_t(&id, open, crate::motion::CHEVRON, cx.reduce_motion());
        let group = id.clone();
        div()
            .id(id)
            .group(group.clone())
            .mx(px(8.0))
            .mt(px(8.0))
            .h(px(26.0))
            .px(px(8.0))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(9.0))
            .rounded(px(radius::ROW))
            .cursor_pointer()
            .text_size(ty::CAPTION)
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(t.text_faint)
            .hover(|s| s.text_color(t.text_muted))
            .role(gpui::Role::Button)
            .aria_expanded(open)
            .aria_label(label.clone())
            .children(lead.map(|l| div().w(px(16.0)).flex_none().flex().justify_center().child(l)))
            .child(div().min_w_0().truncate().child(label))
            .child(
                icon(icons::ALT_ARROW_RIGHT).size(px(10.0)).text_color(t.text_faint).with_transformation(
                    gpui::Transformation::rotate(gpui::radians(turn * std::f32::consts::FRAC_PI_2)),
                ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_size(ty::BADGE)
                    .text_color(t.text_faint)
                    .invisible()
                    .group_hover(group, |s| s.visible())
                    .child(count.to_string()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                let _ = this.engine.store.set_setting(&key, if open { "1" } else { "0" });
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_more_item(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let selected = matches!(
            self.route,
            Route::Kanban
                | Route::Flock
                | Route::Automations
                | Route::PullRequests
                | Route::Team
                | Route::Agents
                | Route::Usage
                | Route::Archived
                | Route::Help
        ) || (self.route == Route::Settings
            && self.settings.read(cx).section != crate::settings::Section::Appearance);
        let open = self.more_open;
        let lit = selected || open;
        let item = div()
            .id("nav-more")
            .group("nav-more")
            .h(px(36.0))
            .mx(px(8.0))
            .px(px(10.0))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.0))
            .rounded(px(radius::ROW))
            .cursor_pointer()
            .text_size(ty::BODY)
            .role(gpui::Role::Button)
            .aria_label("More")
            .aria_expanded(open)
            .when(lit, |d| d.bg(ui::selected_bg(t)).text_color(t.text).font_weight(gpui::FontWeight::MEDIUM))
            .when(!lit, |d| d.text_color(t.text_muted).hover(|s| s.bg(t.wash(0.05)).text_color(t.text)))
            .child(
                icon(icons::MORE_HORIZONTAL)
                    .size(px(18.0))
                    .flex_none()
                    .text_color(if lit { t.text } else { t.text_muted })
                    .group_hover("nav-more", |s| s.text_color(t.text)),
            )
            .child(div().flex_1().child("More"))
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
                row("more-kanban", icons::SPLIT_THREE_COLUMNS, "Kanban", Route::Kanban),
                row("more-flock", icons::TERMINAL, "Flock", Route::Flock),
                row("more-automations", icons::CLOCK_CIRCLE, "Automations", Route::Automations),
                row("more-prs", icons::PULL_REQUEST, "Pull requests", Route::PullRequests),
                Item::Separator,
                row("more-team", icons::USERS_GROUP, "Team", Route::Team),
                row("more-agents", icons::BOT, "Agents", Route::Agents),
                Item::Separator,
                row("more-archived", icons::ARCHIVE_MINIMALISTIC, "Archived", Route::Archived),
                row("more-usage", icons::CHART, "Usage", Route::Usage),
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

    pub(crate) fn render_row(
        &self,
        task: &Task,
        compact: bool,
        fold: Option<(bool, usize)>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
        // Where it stands, as a coloured word in place of the time: status
        // reads as words, not badges, and quiet rows keep their age.
        let corner = match task.status {
            TaskStatus::NeedsInput => Some(("Waiting", t.warning)),
            TaskStatus::Failed => Some(("Failed", t.danger)),
            TaskStatus::Running => Some(("Running", t.busy)),
            _ => None,
        }
        .map_or_else(
            || div().text_size(ty::CAPTION).text_color(t.text_faint).child(ui::relative_time(task.updated_at)),
            |(word, tint)| {
                div().text_size(ty::CAPTION).font_weight(gpui::FontWeight::MEDIUM).text_color(tint).child(word)
            },
        );
        // Hover reveals "···": the same menu as a right-click (rename, pin,
        // archive, move, delete), so the row itself stays clean.
        let more_group = SharedString::from(format!("row-more-{id}"));
        let archive_btn = div()
            .id(more_group.clone())
            .group(more_group.clone())
            .size(px(22.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(radius::ROW - 2.0))
            .hover(|s| s.bg(t.wash(0.08)))
            .tooltip(ui::tip(if archived { "More (archived)" } else { "More" }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.close_menus();
                    this.menu = Some(ChatMenu { task: id, at: e.position, copy_open: false, confirm_delete: false });
                    this.menu_opened(window, cx);
                    cx.notify();
                }),
            )
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                icon(icons::MORE_HORIZONTAL)
                    .size(px(14.0))
                    .text_color(t.text_faint)
                    .group_hover(more_group, |s| s.text_color(t.text)),
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
        let renaming_now = renaming.is_some();
        let row = div()
            .id(SharedString::from(format!("row-{id}")))
            .group(group.clone())
            .h(px(if compact { 32.0 } else { 44.0 }))
            .mx(px(8.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(radius::ROW))
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
            // Rows carry no icon unless a teammate owns them; an unread
            // thread gets a small accent dot in the margin.
            .child(
                div()
                    .w(px(6.0))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .when(task.unread && !selected, |d| d.child(div().size(px(6.0)).rounded_full().bg(t.accent))),
            )
            .when_some(self.persona_for(id), |d, p| d.child(crate::team::avatar(p, 16.0, t)))
            .child(match renaming {
                Some(input) => div()
                    .flex_1()
                    .min_w_0()
                    .px(px(4.0))
                    .rounded(px(radius::ROW))
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
            .when_some(fold, |d, (open, n)| {
                let turn = if open { std::f32::consts::FRAC_PI_2 } else { 0.0 };
                d.child(
                    div()
                        .id(SharedString::from(format!("row-fold-{id}")))
                        .h(px(20.0))
                        .px(px(5.0))
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(3.0))
                        .rounded(px(radius::ROW))
                        .text_size(ty::BADGE)
                        .text_color(t.text_faint)
                        .hover(|s| s.bg(t.wash(0.08)).text_color(t.text))
                        .tooltip(ui::tip(if open {
                            "Fold the threads it started"
                        } else {
                            "Show the threads it started"
                        }))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            let _ = this
                                .engine
                                .store
                                .set_setting(&format!("sidebar_open.{id}"), if open { "0" } else { "1" });
                            cx.notify();
                        }))
                        .child(n.to_string())
                        .child(
                            icon(icons::ALT_ARROW_RIGHT)
                                .size(px(10.0))
                                .with_transformation(gpui::Transformation::rotate(gpui::radians(turn))),
                        ),
                )
            })
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
            // Drag a thread onto the conversation or a pane to open it beside.
            .when(!archived && !renaming_now, |d| {
                // The sidebar is its own cached view: wake the shell so the
                // conversation shows its drop target at once.
                let shell = cx.entity().downgrade();
                d.on_drag(crate::side_panel::ThreadDrag { id, title: task.title.clone().into() }, move |d, _, _, cx| {
                    let _ = shell.update(cx, |_, cx| cx.notify());
                    cx.new(|_| d.clone())
                })
            })
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

impl Shell {
    /// Every nested thread's parent (see [`nest`]).
    fn thread_family(&self) -> HashMap<TaskId, TaskId> {
        let tasks: Vec<(TaskId, Option<PathBuf>)> = self.tasks.iter().map(|t| (t.id, t.worktree.clone())).collect();
        let links: Vec<(TaskId, TaskId)> = self
            .engine
            .store
            .task_links()
            .into_iter()
            .filter(|l| l.kind == dove_core::store::LinkKind::Delegate)
            .map(|l| (l.child, l.parent))
            .collect();
        nest(&tasks, &links)
    }

    /// A thread's row and, when unfolded, the threads it started and its
    /// subagents beneath it. Unfolded by default while one of them works or
    /// waits on you, or holds the open thread.
    fn render_branch(
        &self,
        task: &Task,
        depth: usize,
        kids: &HashMap<TaskId, Vec<Task>>,
        compact: bool,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let children = kids.get(&task.id).cloned().unwrap_or_default();
        let subs = self.threads.get(&task.id).map(|v| v.read(cx).chat.read(cx).subagents()).unwrap_or_default();
        let count = children.len() + subs.len();
        let open = count > 0
            && match self.engine.store.setting(&format!("sidebar_open.{}", task.id)).as_deref() {
                Some("1") => true,
                Some("0") => false,
                _ => {
                    children.iter().any(|c| {
                        matches!(c.status, TaskStatus::Running | TaskStatus::NeedsInput)
                            || self.route == Route::Thread(c.id)
                    }) || subs.iter().any(|s| s.running)
                }
            };
        let row = self.render_row(task, compact, (count > 0).then_some((open, count)), t, cx);
        let mut out = vec![div().pl(px(14.0 * depth as f32)).child(row).into_any_element()];
        if open {
            for c in &children {
                out.extend(self.render_branch(c, depth + 1, kids, compact, t, cx));
            }
            for s in &subs {
                out.push(self.render_subagent_row(task.id, s, depth + 1, t, cx));
            }
        }
        out
    }

    /// A subagent under its thread: what it is, and whether it is working,
    /// done or failed. A click opens it beside its thread.
    fn render_subagent_row(
        &self,
        thread: TaskId,
        s: &crate::transcript::Subagent,
        depth: usize,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let call = s.id.clone();
        let mark: AnyElement = if s.running {
            kit::spinner(SharedString::from(format!("sb-sub-spin-{call}")), 11.0, t.busy).into_any_element()
        } else if s.error {
            icon(icons::CLOSE_CIRCLE).size(px(12.0)).text_color(t.danger).into_any_element()
        } else {
            icon(icons::CHECK).size(px(12.0)).text_color(t.success).into_any_element()
        };
        let detail = if s.running { s.doing.clone().unwrap_or_else(|| "Working".into()) } else { s.kind.clone() };
        div()
            .pl(px(14.0 * depth as f32))
            .child(
                div()
                    .id(SharedString::from(format!("sb-sub-{call}")))
                    .h(px(30.0))
                    .mx(px(8.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .rounded(px(radius::ROW))
                    .cursor_pointer()
                    .hover(|st| st.bg(t.wash(0.05)))
                    .tooltip(ui::tip(if s.prompt.is_empty() { s.title.clone() } else { s.prompt.clone() }))
                    .child(div().w(px(16.0)).flex_none().flex().justify_center().child(mark))
                    .child(icon(icons::FORK).size(px(13.0)).flex_none().text_color(t.text_faint))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(ty::SMALL)
                            .text_color(t.text_muted)
                            .child(s.title.clone()),
                    )
                    .when(!detail.is_empty(), |d| {
                        d.child(
                            div()
                                .flex_none()
                                .max_w(px(90.0))
                                .truncate()
                                .text_size(ty::CAPTION)
                                .text_color(t.text_faint)
                                .child(detail),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_task(thread, window, cx);
                        window.dispatch_action(Box::new(crate::actions::OpenSubagent { call: call.clone() }), cx);
                    })),
            )
            .into_any_element()
    }

    /// Agents running as TUIs in Flock: harness, working or idle, where,
    /// and when it last printed. A row shows its pane.
    fn render_agents_section(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        use crate::flock::AgentState;
        let agents = self.workspace.read(cx).agents().to_vec();
        if agents.is_empty() {
            return None;
        }
        let on_page = self.route == Route::Flock;
        let header = div()
            .h(px(26.0))
            .mx(px(8.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(ty::CAPTION)
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(t.text_faint)
            .child("Agents")
            .child(div().text_size(ty::BADGE).child(agents.len().to_string()));
        let mut list = div().flex().flex_col().gap(px(1.0)).pt(px(14.0)).child(header);
        for a in agents {
            let selected = on_page && a.selected;
            let (dot, label) = match a.state {
                AgentState::Working => (t.busy, a.state.label()),
                AgentState::NeedsInput => (t.warning, a.state.label()),
                AgentState::Idle => (t.text_faint, a.state.label()),
                AgentState::Exited => (t.danger, a.state.label()),
            };
            let pane = a.pane;
            list = list.child(
                div()
                    .id(gpui::ElementId::Name(format!("sb-agent-{pane}").into()))
                    .mx(px(8.0))
                    .px(px(8.0))
                    .py(px(5.0))
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .rounded(px(radius::ROW))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(t.wash(0.08)))
                    .when(!selected, |d| d.hover(|s| s.bg(t.wash(0.04))))
                    .child(icon(ui::harness_icon(a.kind)).size(px(16.0)).text_color(ui::harness_tint(
                        a.kind,
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
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(ty::BODY)
                                            .text_color(t.text)
                                            .child(a.kind.label()),
                                    )
                                    .child(
                                        div()
                                            .text_size(ty::MICRO)
                                            .text_color(t.text_faint)
                                            .child(ui::relative_time(a.last_ms)),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.0))
                                    .text_size(ty::MICRO)
                                    .text_color(t.text_faint)
                                    .child(ui::dot(dot, 6.0))
                                    .child(div().truncate().child(format!("{label} · {}", a.place))),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.navigate(Route::Flock, window, cx);
                        this.workspace.update(cx, |w, cx| w.focus_pane(pane, window, cx));
                    })),
            );
        }
        Some(list.into_any_element())
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
    fn nested_threads_find_their_parent() {
        let id = |n: u128| TaskId(uuid::Uuid::from_u128(n));
        let wt = |s: &str| Some(PathBuf::from(s));
        let tasks = vec![
            (id(1), wt("/w/a")),
            (id(2), wt("/w/a")), // a side chat of 1
            (id(3), None),       // delegated by 1
            (id(4), None),       // delegated by 3
            (id(5), wt("/w/b")),
            (id(6), None),
            (id(7), None),
        ];
        let links = vec![(id(3), id(1)), (id(4), id(3)), (id(6), id(7)), (id(7), id(6)), (id(9), id(1))];
        let p = nest(&tasks, &links);
        assert_eq!(p.get(&id(2)), Some(&id(1)));
        assert_eq!(p.get(&id(3)), Some(&id(1)));
        assert_eq!(p.get(&id(4)), Some(&id(3)));
        assert!(!p.contains_key(&id(1)) && !p.contains_key(&id(5)));
        // A loop is cut once: one of the two leads, the other sits under it.
        assert_eq!(
            [p.contains_key(&id(6)), p.contains_key(&id(7))].iter().filter(|x| **x).count(),
            1,
            "a loop keeps exactly one link"
        );
        assert!(!p.contains_key(&id(9)), "unknown threads are ignored");
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
