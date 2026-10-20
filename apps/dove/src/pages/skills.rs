//! Settings → Skills: the skills every agent Dove starts can use.
//!
//! Installed lists the bundled skills and the ones added from GitHub, each
//! with a switch that installs it into (or removes it from) every harness's
//! skills folder. Discover lists the skills of curated repositories and of
//! any repository the user adds. The work happens in
//! `dove_core::skills` on background threads; this view keeps a copy of
//! the registry and the listings it fetched.

use std::collections::{HashMap, HashSet};

use dove_core::skills::{self, Available, Installed, Listing, Registry, Source};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use gpui_base::input::{InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::design::{radius, space, ty};
use crate::kit::controls::{self as ctl, Segment, Tone};
use crate::kit::page;
use crate::theme::Theme;
use crate::ui;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Installed,
    Discover,
}

/// One repository's listing as Discover knows it.
enum Fetch {
    Loading,
    Ready(Listing),
    Failed(String),
}

pub struct SkillsView {
    tab: Tab,
    registry: Registry,
    search: Entity<InputState>,
    query: String,
    /// Discover's source filter; `None` shows every repository.
    source: Option<String>,
    listings: HashMap<String, Fetch>,
    /// Work in flight: `install:<repo>/<path>`, `remove:<name>`, `update`.
    busy: HashSet<String>,
    adding: bool,
    repo_input: Entity<InputState>,
    /// The last action's outcome.
    notice: Option<Result<String, String>>,
    _subs: Vec<Subscription>,
}

impl SkillsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let style = ui::input_style(Theme::of(cx));
        let search = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Search skills");
            s.set_editor_style(style.clone());
            s
        });
        let repo_input = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("owner/repo or a GitHub URL");
            s.set_editor_style(style);
            s
        });
        let subs = vec![
            cx.subscribe(&search, |this, s, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.query = s.read(cx).value().trim().to_ascii_lowercase();
                    cx.notify();
                }
            }),
            cx.subscribe(&repo_input, |this, _, ev: &InputEvent, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    this.add_repo(cx);
                }
            }),
        ];
        let mut view = Self {
            tab: Tab::Installed,
            registry: Registry::default(),
            search,
            query: String::new(),
            source: None,
            listings: HashMap::new(),
            busy: HashSet::new(),
            adding: false,
            repo_input,
            notice: None,
            _subs: subs,
        };
        view.reload(cx);
        view
    }

    /// Re-read the registry (writing the bundled skills first) off the UI
    /// thread.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move {
            skills::ensure_installed();
            skills::load()
        });
        cx.spawn(async move |this, cx| {
            let registry = task.await;
            let _ = this.update(cx, |v, cx| {
                v.registry = registry;
                cx.notify();
            });
        })
        .detach();
    }

    /// Every repository Discover shows: the curated ones, then the user's.
    fn repos(&self) -> Vec<String> {
        skills::CURATED.iter().map(|c| c.repo.to_string()).chain(self.registry.repos.iter().cloned()).collect()
    }

    fn show(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        if tab == Tab::Discover {
            for repo in self.repos() {
                if !self.listings.contains_key(&repo) {
                    self.fetch(repo, false, cx);
                }
            }
        }
        cx.notify();
    }

    fn fetch(&mut self, repo: String, refresh: bool, cx: &mut Context<Self>) {
        self.listings.insert(repo.clone(), Fetch::Loading);
        let key = repo.clone();
        let task = cx.background_spawn(async move { skills::listing(&key, refresh) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                v.listings.insert(
                    repo,
                    match result {
                        Ok(l) => Fetch::Ready(l),
                        Err(e) => Fetch::Failed(e),
                    },
                );
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Run a registry change in the background, then show its outcome and
    /// the new registry.
    fn act(
        &mut self,
        key: String,
        work: impl FnOnce() -> Result<String, String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if !self.busy.insert(key.clone()) {
            return;
        }
        let task = cx.background_spawn(async move {
            let result = work();
            (result, skills::load())
        });
        cx.spawn(async move |this, cx| {
            let (result, registry) = task.await;
            let _ = this.update(cx, |v, cx| {
                v.busy.remove(&key);
                v.registry = registry;
                v.notice = Some(result).filter(|r| r.as_ref().map_or(true, |m| !m.is_empty()));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn set_enabled(&mut self, name: String, on: bool, cx: &mut Context<Self>) {
        // Show the switch move at once; the registry write follows.
        if let Some(s) = self.registry.skills.iter_mut().find(|s| s.name == name) {
            s.enabled = on;
        }
        self.act(format!("toggle:{name}"), move || skills::set_skill_enabled(&name, on).map(|_| String::new()), cx);
    }

    fn install(&mut self, repo: String, skill: &Available, cx: &mut Context<Self>) {
        let path = skill.path.clone();
        self.act(
            format!("install:{repo}/{path}"),
            move || skills::install(&repo, &path).map(|s| format!("Installed {} for every agent", s.name)),
            cx,
        );
    }

    fn remove(&mut self, name: String, cx: &mut Context<Self>) {
        self.act(format!("remove:{name}"), move || skills::remove(&name).map(|_| format!("Removed {name}")), cx);
    }

    fn update_all(&mut self, cx: &mut Context<Self>) {
        self.act(
            "update".into(),
            || {
                skills::update_all().map(|names| match names.len() {
                    0 => "Every skill is up to date".into(),
                    _ => format!("Updated {}", names.join(", ")),
                })
            },
            cx,
        );
    }

    fn add_repo(&mut self, cx: &mut Context<Self>) {
        let input = self.repo_input.read(cx).value().trim().to_string();
        if input.is_empty() {
            return;
        }
        match skills::parse_repo(&input) {
            None => {
                self.notice = Some(Err(format!("{input:?} is not a GitHub repository (owner/repo or URL)")));
                cx.notify();
            }
            Some(repo) => {
                self.adding = false;
                self.source = Some(repo.clone());
                if !self.registry.repos.contains(&repo) && !skills::CURATED.iter().any(|c| c.repo == repo) {
                    self.registry.repos.push(repo.clone());
                }
                self.fetch(repo, true, cx);
                self.act("add-repo".into(), move || skills::add_repo(&input).map(|_| String::new()), cx);
            }
        }
    }

    fn remove_repo(&mut self, repo: String, cx: &mut Context<Self>) {
        self.registry.repos.retain(|r| *r != repo);
        self.listings.remove(&repo);
        if self.source.as_deref() == Some(repo.as_str()) {
            self.source = None;
        }
        self.act(format!("drop-repo:{repo}"), move || skills::remove_repo(&repo).map(|_| String::new()), cx);
    }

    fn matches(&self, name: &str, description: &str) -> bool {
        self.query.is_empty()
            || self.query.split_whitespace().all(|w| {
                name.to_ascii_lowercase().contains(w) || description.to_ascii_lowercase().contains(w)
            })
    }
}

/// The label Discover shows for a repository.
fn repo_label(repo: &str) -> String {
    skills::CURATED.iter().find(|c| c.repo == repo).map(|c| c.label.to_string()).unwrap_or_else(|| repo.to_string())
}

/// A filter chip: a pill, filled when chosen.
fn chip(id: SharedString, label: String, on: bool, t: &Theme) -> gpui::Stateful<gpui::Div> {
    let hover = t.wash(0.10);
    div()
        .id(gpui::ElementId::Name(id))
        .role(gpui::Role::RadioButton)
        .aria_selected(on)
        .aria_label(SharedString::from(label.clone()))
        .h(px(26.0))
        .px(px(10.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.0))
        .rounded_full()
        .text_size(ty::SMALL)
        .cursor_pointer()
        .map(|d| {
            if on {
                d.bg(t.accent).text_color(t.on_solid)
            } else {
                d.bg(t.wash(0.06)).text_color(t.text_muted).hover(move |s| s.bg(hover))
            }
        })
        .child(label)
}

fn icon_button(id: SharedString, glyph: &'static str, label: &'static str, t: &Theme) -> gpui::Stateful<gpui::Div> {
    let hover = t.wash(0.08);
    div()
        .id(gpui::ElementId::Name(id))
        .role(gpui::Role::Button)
        .aria_label(label)
        .size(px(26.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(radius::RADIUS))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(ui::tip(label))
        .child(icon(glyph).size(px(13.0)).text_color(t.text_muted))
}

fn right(children: impl IntoIterator<Item = AnyElement>) -> gpui::Div {
    div().flex_none().max_w_full().flex().items_center().justify_end().gap(px(space::SM)).children(children)
}

impl SkillsView {
    fn installed_row(&self, t: &Theme, s: &Installed, first: bool, cx: &mut Context<Self>) -> AnyElement {
        let name = s.name.clone();
        let on = s.enabled;
        let detail = match &s.source {
            Source::Bundled => format!("{} · Dove", s.description),
            Source::Git { repo, .. } => format!("{} · {repo}", s.description),
        };
        let mut controls: Vec<AnyElement> = Vec::new();
        if matches!(s.source, Source::Git { .. }) {
            let removing = self.busy.contains(&format!("remove:{name}"));
            let n = name.clone();
            controls.push(
                ctl::button_sm(t, format!("skill-rm-{name}"), if removing { "Removing…" } else { "Remove" }, Tone::Quiet)
                    .on_click(cx.listener(move |this, _, _, cx| this.remove(n.clone(), cx)))
                    .into_any_element(),
            );
        }
        controls.push(
            ctl::on_toggle(ctl::toggle(t, format!("skill-on-{name}"), on), cx, move |this, _, cx| {
                this.set_enabled(name.clone(), !on, cx)
            })
            .into_any_element(),
        );
        page::row(t, first)
            .child(page::row_tile(
                t,
                if s.source == Source::Bundled { icons::DOVE_LOGO } else { icons::MAGIC_STICK_3 },
                None,
            ))
            .child(page::row_text(t, s.name.clone(), detail))
            .child(right(controls))
            .into_any_element()
    }

    fn installed(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let shown: Vec<Installed> =
            self.registry.skills.iter().filter(|s| self.matches(&s.name, &s.description)).cloned().collect();
        let (bundled, git): (Vec<Installed>, Vec<Installed>) =
            shown.into_iter().partition(|s| s.source == Source::Bundled);
        let mut out = div().flex().flex_col();
        if !bundled.is_empty() {
            let rows: Vec<AnyElement> =
                bundled.iter().enumerate().map(|(i, s)| self.installed_row(t, s, i == 0, cx)).collect();
            out = out.child(page::section(t, "Included with Dove", page::card(t).children(rows)));
        }
        let git_rows: Vec<AnyElement> =
            git.iter().enumerate().map(|(i, s)| self.installed_row(t, s, i == 0, cx)).collect();
        let from_github = if git_rows.is_empty() {
            page::empty_state(
                t,
                icons::MAGIC_STICK_3,
                if self.query.is_empty() { "No skills from GitHub yet" } else { "No matching skills" },
                if self.query.is_empty() { "Find skills in Discover and install them for every agent." } else { "" },
                self.query.is_empty().then(|| {
                    ctl::button_sm(t, "skills-go-discover", "Discover skills", Tone::Filled)
                        .on_click(cx.listener(|this, _, _, cx| this.show(Tab::Discover, cx)))
                        .into_any_element()
                }),
            )
            .into_any_element()
        } else {
            page::card(t).children(git_rows).into_any_element()
        };
        out.child(page::section(t, "From GitHub", from_github)).into_any_element()
    }

    fn discover_row(&self, t: &Theme, repo: &str, a: &Available, first: bool, cx: &mut Context<Self>) -> AnyElement {
        let installed = self.registry.get(&a.name);
        let mine = installed.is_some_and(|s| matches!(&s.source, Source::Git { repo: r, path, .. } if r == repo && *path == a.path));
        let busy = self.busy.contains(&format!("install:{repo}/{}", a.path));
        let control: AnyElement = if mine {
            page::badge_tone("Installed", t.success).into_any_element()
        } else if let Some(other) = installed {
            div()
                .id(SharedString::from(format!("skill-taken-{repo}-{}", a.path)))
                .tooltip(ui::tip(format!("A skill named {} is installed from {}", a.name, other.source.label())))
                .child(page::badge(t, "Name in use"))
                .into_any_element()
        } else if let Some(problem) = &a.problem {
            div()
                .id(SharedString::from(format!("skill-bad-{repo}-{}", a.path)))
                .tooltip(ui::tip(problem.clone()))
                .child(page::badge_tone("Invalid", t.warning))
                .into_any_element()
        } else if busy {
            ctl::disabled(ctl::button_sm(t, format!("skill-get-{repo}-{}", a.path), "Installing…", Tone::Filled), "")
                .into_any_element()
        } else {
            let repo = repo.to_string();
            let skill = a.clone();
            ctl::button_sm(t, format!("skill-get-{repo}-{}", a.path), "Install", Tone::Filled)
                .on_click(cx.listener(move |this, _, _, cx| this.install(repo.clone(), &skill, cx)))
                .into_any_element()
        };
        let description = if a.description.is_empty() { a.problem.clone().unwrap_or_default() } else { a.description.clone() };
        page::row(t, first)
            .child(page::row_tile(t, icons::MAGIC_STICK_3, None))
            .child(page::row_text(t, a.name.clone(), description))
            .child(right([control]))
            .into_any_element()
    }

    fn discover(&self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let repos = self.repos();
        let mut chips: Vec<AnyElement> = vec![
            chip("skills-src-all".into(), "All".into(), self.source.is_none(), t)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.source = None;
                    cx.notify();
                }))
                .into_any_element(),
        ];
        for repo in &repos {
            let r = repo.clone();
            chips.push(
                chip(format!("skills-src-{repo}").into(), repo_label(repo), self.source.as_deref() == Some(repo), t)
                    .tooltip(ui::tip(repo.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.source = Some(r.clone());
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        let add_focused = self.repo_input.read(cx).focus_handle(cx).is_focused(window);
        let mut out = div().flex().flex_col().child(
            div().mt(px(space::LG)).px(px(page::LABEL_INSET)).flex().flex_wrap().gap(px(6.0)).children(chips),
        );
        if self.adding {
            out = out.child(
                page::card(t).mt(px(space::MD)).child(
                    page::row(t, true)
                        .child(page::row_text(
                            t,
                            "Add a repository",
                            "Any public GitHub repository with SKILL.md folders, or a private one your GitHub CLI can read.",
                        ))
                        .child(right([
                            ctl::text_field(&self.repo_input, Some(240.0), add_focused, t).into_any_element(),
                            ctl::button_sm(t, "skills-add-go", "Add", Tone::Solid)
                                .on_click(cx.listener(|this, _, _, cx| this.add_repo(cx)))
                                .into_any_element(),
                            ctl::button_sm(t, "skills-add-cancel", "Cancel", Tone::Quiet)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.adding = false;
                                    cx.notify();
                                }))
                                .into_any_element(),
                        ])),
                ),
            );
        }
        for repo in repos.iter().filter(|r| self.source.as_ref().is_none_or(|s| s == *r)) {
            let custom = !skills::CURATED.iter().any(|c| c.repo == repo.as_str());
            let mut actions: Vec<AnyElement> = Vec::new();
            let r = repo.clone();
            actions.push(
                icon_button(format!("skills-refresh-{repo}").into(), icons::REFRESH, "Refresh", t)
                    .on_click(cx.listener(move |this, _, _, cx| this.fetch(r.clone(), true, cx)))
                    .into_any_element(),
            );
            if custom {
                let r = repo.clone();
                actions.push(
                    icon_button(format!("skills-drop-{repo}").into(), icons::CLOSE, "Remove repository", t)
                        .on_click(cx.listener(move |this, _, _, cx| this.remove_repo(r.clone(), cx)))
                        .into_any_element(),
                );
            }
            let block: AnyElement = match self.listings.get(repo) {
                None | Some(Fetch::Loading) => div().px(px(space::LG)).py(px(space::MD)).child(page::skeleton_lines(t, 3)).into_any_element(),
                Some(Fetch::Failed(e)) => page::error_strip(t, e.clone()).into_any_element(),
                Some(Fetch::Ready(listing)) => {
                    let shown: Vec<&Available> =
                        listing.skills.iter().filter(|a| self.matches(&a.name, &a.description)).collect();
                    if shown.is_empty() {
                        page::info_strip(
                            t,
                            if listing.skills.is_empty() { "No skills found in this repository" } else { "No matching skills" },
                        )
                        .into_any_element()
                    } else {
                        page::card(t)
                            .children(shown.iter().enumerate().map(|(i, a)| self.discover_row(t, repo, a, i == 0, cx)))
                            .into_any_element()
                    }
                }
            };
            let head = div()
                .px(px(page::LABEL_INSET))
                .flex()
                .items_center()
                .gap(px(space::SM))
                .child(page::section_label(t, repo_label(repo)).px(px(0.0)))
                .child(div().flex_1().min_w_0().truncate().text_size(ty::META).text_color(t.text_faint).child(repo.clone()))
                .children(actions);
            out = out.child(div().mt(px(page::SECTION_GAP)).flex().flex_col().gap(px(space::SM)).child(head).child(block));
        }
        out.into_any_element()
    }
}

impl Render for SkillsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("SkillsView");
        let t = Theme::of(cx).clone();
        let tab = self.tab;
        let count = self.registry.skills.iter().filter(|s| s.enabled).count();
        let tabs = ctl::segmented(
            &t,
            "skills-tab",
            vec![Segment::new(format!("Installed  {count}")), Segment::new("Discover")],
            if tab == Tab::Installed { 0 } else { 1 },
            {
                let this = cx.entity().downgrade();
                move |i, _, cx| {
                    let _ = this.update(cx, |v, cx| v.show(if i == 0 { Tab::Installed } else { Tab::Discover }, cx));
                }
            },
        );
        let action: AnyElement = match tab {
            Tab::Installed => {
                let updating = self.busy.contains("update");
                let has_git = self.registry.skills.iter().any(|s| matches!(s.source, Source::Git { .. }));
                let b = ctl::button_sm(&t, "skills-update", if updating { "Updating…" } else { "Update all" }, Tone::Outlined);
                if has_git && !updating {
                    b.on_click(cx.listener(|this, _, _, cx| this.update_all(cx))).into_any_element()
                } else {
                    ctl::disabled(b, if updating { "Checking GitHub" } else { "No skills from GitHub to update" })
                        .into_any_element()
                }
            }
            Tab::Discover => ctl::button_sm(&t, "skills-add-repo", "Add repository", Tone::Outlined)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.adding = !this.adding;
                    if this.adding {
                        this.repo_input.update(cx, |s, cx| s.focus(window, cx));
                    }
                    cx.notify();
                }))
                .into_any_element(),
        };
        let toolbar = div()
            .mt(px(space::LG))
            .px(px(page::LABEL_INSET))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(space::MD))
            .child(tabs)
            .child(
                page::search_field(&t, &self.search)
                    .flex_1()
                    .min_w(px(160.0))
                    .rounded(px(radius::SELECT))
                    .bg(t.wash(0.06)),
            )
            .child(action);
        let notice = self.notice.clone().map(|n| {
            let strip = match n {
                Ok(m) => page::info_strip(&t, m),
                Err(e) => page::error_strip(&t, e),
            };
            div()
                .mt(px(space::MD))
                .flex()
                .items_start()
                .child(strip.flex_1())
                .child(
                    icon_button("skills-notice-close".into(), icons::CLOSE, "Dismiss", &t).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.notice = None;
                            cx.notify();
                        },
                    )),
                )
        });
        let body = match tab {
            Tab::Installed => self.installed(&t, cx),
            Tab::Discover => self.discover(&t, window, cx),
        };
        div()
            .flex()
            .flex_col()
            .child(toolbar)
            .children(notice)
            .child(body)
            .child(
                div()
                    .mt(px(space::LG))
                    .px(px(page::LABEL_INSET))
                    .text_size(ty::META)
                    .text_color(t.text_faint)
                    .child(
                        "Skills are copied into ~/.claude/skills, ~/.agents/skills and the skill folders of OpenCode, \
                         Gemini CLI and Pi. Folders you made yourself are never changed.",
                    ),
            )
    }
}
