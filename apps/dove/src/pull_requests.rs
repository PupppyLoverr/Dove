//! Pull requests for a project's GitHub repository, through the `gh` CLI:
//! the list, and a detail page (description, checks, files) with "Check out
//! in new thread".

use std::path::PathBuf;

use dove_core::git::github::{self, PrDetail, PrFilter, PullRequest};
use dove_core::git::{FileDiff, LineKind};
use dove_core::store::Project;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    px,
};
use gpui_base::input::{InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::design::{space, ty};
use crate::kit::controls::{self as ctl, Segment, SelectState, Tone};
use crate::kit::{self, page};
use crate::theme::Theme;
use crate::ui;

pub enum PrEvent {
    /// Start a thread on this PR's branch.
    Checkout { project: Project, number: u64, title: String },
}

struct Detail {
    pr: PullRequest,
    detail: Option<PrDetail>,
    files: Vec<FileDiff>,
    error: Option<SharedString>,
}

enum Load {
    Idle,
    Loading,
    Ready(Vec<PullRequest>),
    Failed(SharedString),
}

pub struct PullRequestsPage {
    projects: Vec<Project>,
    repo: Option<Project>,
    filter: PrFilter,
    state: Load,
    search: Entity<InputState>,
    query: String,
    repo_select: SelectState,
    detail: Option<Detail>,
    _load: Option<gpui::Task<()>>,
    _detail: Option<gpui::Task<()>>,
    _sub: Subscription,
}

impl EventEmitter<PrEvent> for PullRequestsPage {}

impl PullRequestsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let style = ui::input_style(Theme::of(cx));
        let search = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Search pull requests");
            s.set_editor_style(style);
            s
        });
        let _sub = cx.subscribe(&search, |this, s, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                this.query = s.read(cx).value().to_lowercase();
                cx.notify();
            }
        });
        Self {
            projects: Vec::new(),
            repo: None,
            filter: PrFilter::Authored,
            state: Load::Idle,
            search,
            query: String::new(),
            repo_select: SelectState::default(),
            detail: None,
            _load: None,
            _detail: None,
            _sub,
        }
    }

    /// Point at `preferred` (the active project) unless the user chose one.
    pub fn set_projects(&mut self, projects: Vec<Project>, preferred: Option<&Project>, cx: &mut Context<Self>) {
        self.projects = projects;
        if self.repo.is_none()
            && let Some(p) = preferred.cloned().or_else(|| self.projects.first().cloned())
        {
            self.repo = Some(p);
            self.load(cx);
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        if !github::gh_available() {
            self.state =
                Load::Failed("Install the GitHub CLI (gh) and run `gh auth login` to see pull requests.".into());
            cx.notify();
            return;
        }
        self.state = Load::Loading;
        let filter = self.filter;
        let path: PathBuf = repo.path.clone();
        let work = cx.background_spawn(async move { github::pull_requests(&path, filter) });
        self._load = Some(cx.spawn(async move |this, cx| {
            let r = work.await;
            let _ = this.update(cx, |p, cx| {
                p.state = match r {
                    Ok(prs) => Load::Ready(prs),
                    Err(e) => Load::Failed(format!("{e:#}").into()),
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn open_detail(&mut self, pr: PullRequest, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        let number = pr.number;
        self.detail = Some(Detail { pr, detail: None, files: Vec::new(), error: None });
        let work = cx.background_spawn(async move {
            let detail = github::pr_detail(&repo.path, number)?;
            let diff = github::pr_diff(&repo.path, number).unwrap_or_default();
            anyhow::Ok((detail, dove_core::git::diff::parse(&diff)))
        });
        self._detail = Some(cx.spawn(async move |this, cx| {
            let r = work.await;
            let _ = this.update(cx, |p, cx| {
                if let Some(d) = p.detail.as_mut().filter(|d| d.pr.number == number) {
                    match r {
                        Ok((detail, files)) => {
                            d.detail = Some(detail);
                            d.files = files;
                        }
                        Err(e) => d.error = Some(format!("{e:#}").into()),
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn render_detail(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let d = self.detail.as_ref().expect("detail open");
        let pr = &d.pr;
        let checks = d.detail.as_ref().map(|x| x.checks.clone()).unwrap_or_default();
        let body = d.detail.as_ref().map(|x| x.body.clone()).unwrap_or_default();
        let url = pr.url.clone();
        let checkout = {
            let project = self.repo.clone();
            let (number, title) = (pr.number, pr.title.clone());
            ctl::button(t, "pr-checkout", "Check out in new thread", Tone::Solid).on_click(cx.listener(
                move |_, _, _, cx| {
                    if let Some(project) = project.clone() {
                        cx.emit(PrEvent::Checkout { project, number, title: title.clone() });
                    }
                },
            ))
        };
        let header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(space::SM))
            .child(ui::icon_button("pr-back", icons::ARROW_LEFT, "All pull requests", t, {
                let weak = cx.weak_entity();
                move |_, _, cx| {
                    let _ = weak.update(cx, |p, cx| {
                        p.detail = None;
                        cx.notify();
                    });
                }
            }))
            .child(
                div()
                    .flex_1()
                    .min_w(px(200.0))
                    .truncate()
                    .text_size(ty::PAGE)
                    .line_height(ty::lh::PAGE)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(pr.title.clone()),
            )
            .child(
                ctl::button(t, "pr-web", "Open on GitHub", Tone::Outlined).on_click(move |_, _, cx| cx.open_url(&url)),
            )
            .child(checkout);
        let meta = div()
            .mt(px(space::XS))
            .px(px(page::LABEL_INSET))
            .flex()
            .flex_wrap()
            .gap(px(10.0))
            .text_size(ty::BODY)
            .text_color(t.text_muted)
            .child(format!("#{}", pr.number))
            .child(pr.author.clone())
            .child(format!("{} → {}", pr.branch, d.detail.as_ref().map(|x| x.base.clone()).unwrap_or_default()))
            .child(div().text_color(t.diff_add).child(format!("+{}", pr.additions)))
            .child(div().text_color(t.diff_del).child(format!("−{}", pr.deletions)));
        let checks_label = match d.detail.as_ref() {
            Some(x) => format!("Checks · {}", x.checks_summary()),
            None => "Checks · loading…".into(),
        };
        let checks_card = page::card(t).children(checks.into_iter().enumerate().map(|(i, c)| {
            let color = match c.state.as_str() {
                "SUCCESS" | "NEUTRAL" | "SKIPPED" => t.success,
                "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT" => t.danger,
                _ => t.warning,
            };
            let link = c.url.clone();
            page::row_hover(t, page::table_row(t, i == 0).id(SharedString::from(format!("check-{i}"))), false)
                .text_size(ty::BODY)
                .on_click(move |_, _, cx| {
                    if !link.is_empty() {
                        cx.open_url(&link);
                    }
                })
                .child(ui::dot(color, 7.0))
                .child(div().flex_1().min_w_0().truncate().text_color(t.text).child(c.name))
                .child(div().flex_none().text_size(ty::META).text_color(t.text_faint).child(c.state.to_lowercase()))
        }));
        let checks_card = if d.detail.as_ref().is_some_and(|x| !x.checks.is_empty()) {
            checks_card
        } else if d.detail.is_some() {
            checks_card.child(
                page::table_row(t, true)
                    .text_size(ty::BODY)
                    .text_color(t.text_faint)
                    .child("No checks on this pull request."),
            )
        } else {
            checks_card.child(div().px(px(space::LG)).py(px(space::MD)).child(page::skeleton_lines(t, 2)))
        };
        let blocks = crate::markdown::parse(if body.trim().is_empty() { "_No description._" } else { &body });
        let files = page::card(t).children(d.files.iter().take(60).enumerate().map(|(i, f)| {
            let preview: Vec<_> =
                f.hunks.iter().flat_map(|h| h.lines.iter()).filter(|l| l.kind != LineKind::Context).take(6).collect();
            div()
                .when(i > 0, |d| d.border_t_1().border_color(page::row_divider(t)))
                .child(
                    div()
                        .h(px(page::TABLE_ROW))
                        .px(px(space::LG))
                        .flex()
                        .items_center()
                        .gap(px(space::SM))
                        .text_size(ty::BODY)
                        .child(icon(icons::DOCUMENT).size(px(14.0)).flex_none().text_color(t.text_faint))
                        .child(div().flex_1().min_w_0().truncate().text_color(t.text).child(f.path.clone()))
                        .child(div().text_size(ty::META).text_color(t.diff_add).child(format!("+{}", f.additions)))
                        .child(div().text_size(ty::META).text_color(t.diff_del).child(format!("−{}", f.deletions))),
                )
                .children(preview.into_iter().map(|l| {
                    let (bg, sign) = if l.kind == LineKind::Added {
                        (t.diff_add.opacity(0.10), "+")
                    } else {
                        (t.diff_del.opacity(0.10), "−")
                    };
                    div()
                        .h(px(19.0))
                        .px(px(space::LG))
                        .flex()
                        .gap(px(space::SM))
                        .bg(bg)
                        .font_family(t.font_mono.clone())
                        .text_size(ty::META)
                        .child(sign)
                        .child(div().flex_1().truncate().child(l.text.to_string()))
                }))
        }));
        div()
            .flex()
            .flex_col()
            .child(header)
            .child(meta)
            .when_some(d.error.clone(), |el, e| el.child(div().mt(px(space::LG)).child(page::error_strip(t, e))))
            .child(page::section(
                t,
                "Description",
                page::card(t).child(
                    div()
                        .px(px(space::LG))
                        .py(px(space::MD))
                        .child(crate::markdown::render("pr-body", &blocks, t, window)),
                ),
            ))
            .child(page::section(t, checks_label, checks_card))
            .child(page::section(t, format!("Files ({})", d.files.len()), files))
            .into_any_element()
    }

    fn count(&self) -> usize {
        match &self.state {
            Load::Ready(p) => p.len(),
            _ => 0,
        }
    }

    fn pr_row(&self, i: usize, p: &PullRequest, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let pr = p.clone();
        let (state_icon, state_color) =
            if p.draft { (icons::PULL_REQUEST_DRAFT, t.text_faint) } else { (icons::PULL_REQUEST, t.success) };
        page::row_hover(t, page::table_row(t, i == 0).id(SharedString::from(format!("pr-{}", p.number))), false)
            .h(px(52.0))
            .on_click(cx.listener(move |this, _, _, cx| this.open_detail(pr.clone(), cx)))
            .child(icon(state_icon).size(px(16.0)).flex_none().text_color(state_color))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .truncate()
                            .text_size(ty::BODY)
                            .line_height(ty::lh::BODY)
                            .text_color(t.text)
                            .child(p.title.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(space::SM))
                            .text_size(ty::META)
                            .text_color(t.text_faint)
                            .child(format!("#{}", p.number))
                            .child(p.author.clone())
                            .child(div().min_w_0().truncate().child(p.branch.clone()))
                            .when_some(p.review.clone(), |d, r| d.child(r.replace('_', " ").to_lowercase())),
                    ),
            )
            .child(div().flex_none().text_size(ty::META).text_color(t.diff_add).child(format!("+{}", p.additions)))
            .child(div().flex_none().text_size(ty::META).text_color(t.diff_del).child(format!("−{}", p.deletions)))
            .into_any_element()
    }
}

impl Render for PullRequestsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("PullRequestsPage");
        let t = Theme::of(cx).clone();
        if self.detail.is_some() {
            let detail = self.render_detail(&t, window, cx);
            return page::page_scroll("pr-page")
                .child(crate::motion::fade_in("pr-detail-in", page::page_column_max(860.0).child(detail)))
                .into_any_element();
        }

        let repo_ix =
            self.repo.as_ref().and_then(|r| self.projects.iter().position(|p| p.id == r.id)).unwrap_or(usize::MAX);
        let projects = self.projects.clone();
        let picker = ctl::select("pr-repo", "Repository", |p: &mut PullRequestsPage| &mut p.repo_select)
            .options(projects.iter().map(|p| ctl::SelectOption::new(p.name.clone()).glyph(icons::FOLDER)), repo_ix)
            .placeholder("Choose repository")
            .width(200.0)
            .menu_width(240.0)
            .on_select(move |this, i, _, cx| {
                if let Some(p) = projects.get(i) {
                    this.repo = Some(p.clone());
                    this.load(cx);
                }
            })
            .render(&t, &self.repo_select, cx);

        let filters = [PrFilter::Authored, PrFilter::Reviewing, PrFilter::All];
        let tabs = ctl::segmented(
            &t,
            "pr-filter",
            vec![Segment::new("Authored"), Segment::new("Reviewing"), Segment::new("All")],
            filters.iter().position(|f| *f == self.filter).unwrap_or(0),
            {
                let weak = cx.weak_entity();
                move |i, _, cx| {
                    let _ = weak.update(cx, |p, cx| {
                        p.filter = filters[i.min(2)];
                        p.load(cx);
                    });
                }
            },
        );
        let toolbar = div()
            .mt(px(24.0))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(space::SM))
            .child(tabs)
            .child(page::search_field(&t, &self.search).flex_1().min_w(px(220.0)));

        let body: AnyElement = match &self.state {
            _ if self.repo.is_none() => page::card(&t)
                .child(page::empty_state(
                    &t,
                    icons::PULL_REQUEST,
                    "Choose a repository",
                    "No GitHub repository was found for this project. Choose one to load its open pull requests.",
                    None,
                ))
                .into_any_element(),
            Load::Idle | Load::Loading => page::card(&t)
                .child(
                    page::table_head(&t).child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::SM))
                            .child(kit::spinner("pr-loading", 12.0, t.text_muted))
                            .child("Loading pull requests…"),
                    ),
                )
                .children((0..4).map(|i| {
                    page::table_row(&t, false).h(px(52.0)).child(page::skeleton(&t, px(16.0).into(), 16.0)).child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(page::skeleton(&t, gpui::relative([0.7, 0.55, 0.8, 0.6][i]), 11.0))
                            .child(page::skeleton(&t, gpui::relative(0.35), 9.0)),
                    )
                }))
                .into_any_element(),
            Load::Failed(e) => page::card(&t)
                .child(page::error_state(
                    &t,
                    "Couldn't load pull requests",
                    e.clone(),
                    Some(
                        ctl::button(&t, "pr-retry", "Try again", Tone::Outlined)
                            .on_click(cx.listener(|this, _, _, cx| this.load(cx)))
                            .into_any_element(),
                    ),
                ))
                .into_any_element(),
            Load::Ready(prs) => {
                let q = self.query.clone();
                let shown: Vec<PullRequest> = prs
                    .iter()
                    .filter(|p| {
                        q.is_empty() || p.title.to_lowercase().contains(&q) || p.branch.to_lowercase().contains(&q)
                    })
                    .cloned()
                    .collect();
                if shown.is_empty() {
                    page::card(&t)
                        .child(page::empty_state(
                            &t,
                            icons::PULL_REQUEST,
                            "No pull requests",
                            "Nothing matches this view yet.",
                            None,
                        ))
                        .into_any_element()
                } else {
                    page::card(&t)
                        .children(shown.iter().enumerate().map(|(i, p)| self.pr_row(i, p, &t, cx)))
                        .into_any_element()
                }
            }
        };

        let trailing = div()
            .flex()
            .items_center()
            .gap(px(space::SM))
            .child(page::badge(&t, self.count().to_string()))
            .child(ui::icon_button("pr-refresh", icons::REFRESH, "Refresh", &t, {
                let weak = cx.weak_entity();
                move |_, _, cx| {
                    let _ = weak.update(cx, |p, cx| p.load(cx));
                }
            }))
            .child(picker)
            .into_any_element();

        page::page_scroll("pr-page")
            .child(crate::motion::fade_in(
                "pr-list-in",
                page::page_column_max(page::PAGE_WIDE)
                    .child(page::page_head(
                        &t,
                        "Pull requests",
                        "Open pull requests on GitHub. Check one out into a thread to review or finish it.",
                        Some(trailing),
                    ))
                    .child(toolbar)
                    .child(div().mt(px(space::MD)).child(body)),
            ))
            .into_any_element()
    }
}
