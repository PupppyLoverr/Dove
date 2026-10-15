//! Pull requests for a project's GitHub repository, through the `gh` CLI:
//! the list, and a detail page (description, checks, files) with "Check out
//! in new thread".

use std::path::PathBuf;

use dove_core::git::github::{self, PrDetail, PrFilter, PullRequest};
use dove_core::git::{FileDiff, LineKind};
use dove_core::store::Project;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    anchored, deferred, div, px,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::assets::{icon, icons};
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
    picker_open: bool,
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
            picker_open: false,
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
            ui::button_primary("pr-checkout", "Check out in new thread", t).on_click(cx.listener(move |_, _, _, cx| {
                if let Some(project) = project.clone() {
                    cx.emit(PrEvent::Checkout { project, number, title: title.clone() });
                }
            }))
        };
        let header = div()
            .flex()
            .items_center()
            .gap(px(8.0))
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
                    .min_w_0()
                    .truncate()
                    .text_size(px(20.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(pr.title.clone()),
            )
            .child(ui::button_ghost("pr-web", "Open on GitHub", t).on_click(move |_, _, cx| cx.open_url(&url)))
            .child(checkout);
        let meta = div()
            .flex()
            .gap(px(10.0))
            .text_size(px(12.5))
            .text_color(t.text_faint)
            .child(format!("#{}", pr.number))
            .child(pr.author.clone())
            .child(format!("{} → {}", pr.branch, d.detail.as_ref().map(|x| x.base.clone()).unwrap_or_default()))
            .child(div().text_color(t.diff_add).child(format!("+{}", pr.additions)))
            .child(div().text_color(t.diff_del).child(format!("−{}", pr.deletions)));
        let checks_el = div()
            .flex()
            .flex_col()
            .rounded(px(10.0))
            .border_1()
            .border_color(t.border)
            .child(div().px(px(12.0)).py(px(8.0)).text_size(px(12.0)).text_color(t.text_faint).child(
                match d.detail.as_ref() {
                    Some(x) => format!("Checks · {}", x.checks_summary()),
                    None => "Checks · loading…".into(),
                },
            ))
            .children(checks.into_iter().enumerate().map(|(i, c)| {
                let color = match c.state.as_str() {
                    "SUCCESS" | "NEUTRAL" | "SKIPPED" => t.success,
                    "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT" => t.danger,
                    _ => t.warning,
                };
                let link = c.url.clone();
                div()
                    .id(SharedString::from(format!("check-{i}")))
                    .h(px(30.0))
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .border_t_1()
                    .border_color(t.border)
                    .text_size(px(13.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.wash(0.04)))
                    .on_click(move |_, _, cx| {
                        if !link.is_empty() {
                            cx.open_url(&link);
                        }
                    })
                    .child(ui::dot(color, 7.0))
                    .child(div().flex_1().truncate().child(c.name))
                    .child(div().text_size(px(11.0)).text_color(t.text_faint).child(c.state.to_lowercase()))
            }));
        let blocks = crate::markdown::parse(if body.trim().is_empty() { "_No description._" } else { &body });
        let files = div()
            .flex()
            .flex_col()
            .rounded(px(10.0))
            .border_1()
            .border_color(t.border)
            .overflow_hidden()
            .children(d.files.iter().take(60).enumerate().map(|(i, f)| {
                let preview: Vec<_> = f
                    .hunks
                    .iter()
                    .flat_map(|h| h.lines.iter())
                    .filter(|l| l.kind != LineKind::Context)
                    .take(6)
                    .collect();
                div()
                    .when(i > 0, |d| d.border_t_1().border_color(t.border))
                    .child(
                        div()
                            .h(px(30.0))
                            .px(px(12.0))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .bg(t.wash(0.03))
                            .text_size(px(13.0))
                            .child(div().flex_1().truncate().child(f.path.clone()))
                            .child(div().text_size(px(11.0)).text_color(t.diff_add).child(format!("+{}", f.additions)))
                            .child(div().text_size(px(11.0)).text_color(t.diff_del).child(format!("−{}", f.deletions))),
                    )
                    .children(preview.into_iter().map(|l| {
                        let (bg, sign) = if l.kind == LineKind::Added {
                            (t.diff_add.opacity(0.10), "+")
                        } else {
                            (t.diff_del.opacity(0.10), "−")
                        };
                        div()
                            .h(px(19.0))
                            .px(px(12.0))
                            .flex()
                            .gap(px(8.0))
                            .bg(bg)
                            .font_family(t.font_mono.clone())
                            .text_size(px(12.0))
                            .child(sign)
                            .child(div().flex_1().truncate().child(l.text.to_string()))
                    }))
            }));
        div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(header)
            .child(meta)
            .when_some(d.error.clone(), |el, e| el.child(div().text_size(px(13.0)).text_color(t.danger).child(e)))
            .child(
                div()
                    .p(px(14.0))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(t.border)
                    .child(crate::markdown::render("pr-body", &blocks, t, window)),
            )
            .child(checks_el)
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(format!("Files ({})", d.files.len())),
            )
            .child(files)
            .into_any_element()
    }

    fn count(&self) -> usize {
        match &self.state {
            Load::Ready(p) => p.len(),
            _ => 0,
        }
    }
}

impl Render for PullRequestsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(cx).clone();
        if self.detail.is_some() {
            let detail = self.render_detail(&t, window, cx);
            return div()
                .id("pr-page")
                .size_full()
                .overflow_y_scroll()
                .flex()
                .justify_center()
                .child(div().w_full().max_w(px(860.0)).px(px(24.0)).pt(px(24.0)).pb(px(40.0)).child(detail))
                .into_any_element();
        }

        let picker = div()
            .relative()
            .child(
                ui::chip("pr-repo", &t)
                    .child(icon(icons::FOLDER).size(px(15.0)).text_color(t.text_muted))
                    .child(self.repo.as_ref().map(|r| r.name.clone()).unwrap_or_else(|| "Choose repository".into()))
                    .child(icon(icons::ALT_ARROW_DOWN).size(px(12.0)).text_color(t.text_faint))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.picker_open = !this.picker_open;
                        cx.notify();
                    })),
            )
            .when(self.picker_open, |d| {
                let rows = div().w(px(240.0)).flex().flex_col().children(self.projects.iter().map(|p| {
                    let p2 = p.clone();
                    ui::menu_row(
                        SharedString::from(format!("pr-proj-{}", p.id)),
                        icons::FOLDER,
                        p.name.clone(),
                        false,
                        &t,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.repo = Some(p2.clone());
                        this.picker_open = false;
                        this.load(cx);
                    }))
                }));
                d.child(deferred(
                    anchored().anchor(Anchor::TopRight).snap_to_window_with_margin(px(8.0)).child(
                        div()
                            .mt(px(30.0))
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.picker_open = false;
                                cx.notify();
                            }))
                            .child(ui::menu_card(&t, rows)),
                    ),
                ))
            });

        let tab = |id: &'static str, label: &'static str, f: PrFilter, this: &Self, cx: &mut Context<Self>| {
            let selected = this.filter == f;
            div()
                .id(id)
                .h(px(28.0))
                .px(px(12.0))
                .flex()
                .items_center()
                .rounded(px(7.0))
                .text_size(px(13.0))
                .cursor_pointer()
                .when(selected, |d| d.bg(t.wash(0.11)).text_color(t.text))
                .when(!selected, |d| d.text_color(t.text_muted))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.filter = f;
                    this.load(cx);
                }))
        };

        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .p(px(3.0))
                    .flex()
                    .gap(px(2.0))
                    .rounded(px(9.0))
                    .bg(t.wash(0.04))
                    .child(tab("pr-authored", "Authored", PrFilter::Authored, self, cx))
                    .child(tab("pr-reviewing", "Reviewing", PrFilter::Reviewing, self, cx))
                    .child(tab("pr-all", "All", PrFilter::All, self, cx)),
            )
            .child(
                div()
                    .flex_1()
                    .h(px(34.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .rounded(px(9.0))
                    .bg(t.wash(0.04))
                    .text_size(px(13.0))
                    .child(icon(icons::MAGNIFER).size(px(14.0)).text_color(t.text_faint))
                    .child(div().flex_1().child(Input::new(&self.search))),
            );

        let body: AnyElement = match &self.state {
            _ if self.repo.is_none() => empty_state(
                &t,
                "Choose a repository",
                "No GitHub repository was found for this project. Choose one to load its open pull requests.",
            ),
            Load::Idle | Load::Loading => empty_state(&t, "Loading pull requests…", ""),
            Load::Failed(e) => empty_state(&t, "Couldn't load pull requests", e),
            Load::Ready(prs) => {
                let q = self.query.clone();
                let shown: Vec<&PullRequest> = prs
                    .iter()
                    .filter(|p| {
                        q.is_empty() || p.title.to_lowercase().contains(&q) || p.branch.to_lowercase().contains(&q)
                    })
                    .collect();
                if shown.is_empty() {
                    empty_state(&t, "No pull requests", "Nothing matches this view yet.")
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .rounded(px(12.0))
                        .border_1()
                        .border_color(t.border)
                        .overflow_hidden()
                        .children(shown.into_iter().enumerate().map(|(i, p)| {
                            let pr = p.clone();
                            let (state_icon, state_color) = if p.draft {
                                (icons::PULL_REQUEST_DRAFT, t.text_faint)
                            } else {
                                (icons::PULL_REQUEST, t.success)
                            };
                            div()
                                .id(SharedString::from(format!("pr-{}", p.number)))
                                .px(px(14.0))
                                .py(px(10.0))
                                .flex()
                                .items_center()
                                .gap(px(12.0))
                                .when(i > 0, |d| d.border_t_1().border_color(t.border))
                                .cursor_pointer()
                                .hover(|s| s.bg(t.wash(0.05)))
                                .on_click(cx.listener(move |this, _, _, cx| this.open_detail(pr.clone(), cx)))
                                .child(icon(state_icon).text_color(state_color))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap(px(2.0))
                                        .child(div().truncate().text_size(px(14.0)).child(p.title.clone()))
                                        .child(
                                            div()
                                                .flex()
                                                .gap(px(8.0))
                                                .text_size(px(12.0))
                                                .text_color(t.text_faint)
                                                .child(format!("#{}", p.number))
                                                .child(p.author.clone())
                                                .child(div().truncate().child(p.branch.clone()))
                                                .when_some(p.review.clone(), |d, r| {
                                                    d.child(r.replace('_', " ").to_lowercase())
                                                }),
                                        ),
                                )
                                .child(
                                    div().text_size(px(12.0)).text_color(t.diff_add).child(format!("+{}", p.additions)),
                                )
                                .child(
                                    div().text_size(px(12.0)).text_color(t.diff_del).child(format!("−{}", p.deletions)),
                                )
                        }))
                        .into_any_element()
                }
            }
        };

        div()
            .id("pr-page")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(980.0))
                    .px(px(32.0))
                    .pt(px(30.0))
                    .pb(px(56.0))
                    .flex()
                    .flex_col()
                    .gap(px(18.0))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.0))
                                    .child(
                                        div()
                                            .flex()
                                            .items_baseline()
                                            .gap(px(8.0))
                                            .child(
                                                div()
                                                    .text_size(px(22.0))
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .child("Pull requests"),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(14.0))
                                                    .text_color(t.text_faint)
                                                    .child(self.count().to_string()),
                                            ),
                                    )
                                    .child(div().text_size(px(13.0)).text_color(t.text_faint).child(
                                        "Open pull requests on GitHub. Check one out into a thread to review or finish it.",
                                    )),
                            )
                            .child(ui::icon_button("pr-refresh", icons::REFRESH, "Refresh", &t, {
                                let weak = cx.weak_entity();
                                move |_, _, cx| {
                                    let _ = weak.update(cx, |p, cx| p.load(cx));
                                }
                            }))
                            .child(picker),
                    )
                    .child(toolbar)
                    .child(body),
            )
            .into_any_element()
    }
}

fn empty_state(t: &Theme, title: &str, body: &str) -> AnyElement {
    div()
        .pt(px(120.0))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.0))
        .child(icon(icons::PULL_REQUEST).size(px(26.0)).text_color(t.text_muted))
        .child(div().text_size(px(16.0)).font_weight(gpui::FontWeight::SEMIBOLD).child(title.to_string()))
        .when(!body.is_empty(), |d| {
            d.child(
                div()
                    .max_w(px(420.0))
                    .text_center()
                    .text_size(px(13.0))
                    .text_color(t.text_muted)
                    .child(body.to_string()),
            )
        })
        .into_any_element()
}
