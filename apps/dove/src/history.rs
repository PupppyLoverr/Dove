//! Git history for the side panel: a lane graph, search, and a click that
//! opens the commit as a diff.

use std::path::PathBuf;
use std::sync::Arc;

use dove_core::git::{self, HistoryCommit};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    px, uniform_list,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::design::ty;
use crate::kit;
use crate::kit::controls::{self as ctl, Tone};
use crate::kit::page;
use crate::theme::Theme;
use crate::ui;

pub enum HistoryEvent {
    OpenCommit { sha: String, subject: String },
}

/// What the panel shows in place of the commit list.
#[derive(Clone)]
enum Note {
    /// A quiet empty-state line.
    Text(SharedString),
    /// The folder is not inside a git repository.
    NotRepo,
    /// A repository without a first commit.
    NoCommitsYet,
    /// git failed for another reason.
    Error(SharedString),
}

pub struct GitHistory {
    root: Option<PathBuf>,
    commits: Arc<Vec<HistoryCommit>>,
    search: Entity<InputState>,
    query: String,
    note: Option<Note>,
    /// `git init` is running from the not-a-repository state.
    initializing: bool,
    init_error: Option<SharedString>,
    _load: Option<gpui::Task<()>>,
    _sub: Subscription,
}

impl EventEmitter<HistoryEvent> for GitHistory {}

/// Why a log came back empty or failed.
enum Repo {
    Ready,
    Missing,
    Unborn,
}

impl GitHistory {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let style = ui::input_style(Theme::of(cx));
        let search = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Search commits");
            s.set_editor_style(style);
            s
        });
        let _sub = cx.subscribe(&search, |this, s, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                this.query = s.read(cx).value().to_string();
                this.refresh(cx);
            }
        });
        Self {
            root: None,
            commits: Arc::default(),
            search,
            query: String::new(),
            note: None,
            initializing: false,
            init_error: None,
            _load: None,
            _sub,
        }
    }

    pub fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        if root == self.root {
            return;
        }
        self.root = root;
        self.init_error = None;
        self.refresh(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            self.commits = Arc::default();
            self.note = Some(Note::Text("Open a thread to see its history.".into()));
            cx.notify();
            return;
        };
        let q = self.query.clone();
        let work = cx.background_spawn(async move {
            let result = git::history(&root, &q, 200);
            // Only an empty or failed log needs to know why.
            let repo = match &result {
                Ok(list) if !list.is_empty() => Repo::Ready,
                _ if root.is_dir() && git::repo_root(&root).is_none() => Repo::Missing,
                _ if git::is_unborn(&root) => Repo::Unborn,
                _ => Repo::Ready,
            };
            (result, repo)
        });
        self._load = Some(cx.spawn(async move |this, cx| {
            let (result, repo) = work.await;
            let _ = this.update(cx, |h, cx| {
                h.commits = Arc::default();
                h.note = match (result, repo) {
                    (_, Repo::Missing) => Some(Note::NotRepo),
                    (_, Repo::Unborn) => Some(Note::NoCommitsYet),
                    (Ok(list), _) if list.is_empty() => Some(Note::Text(if h.query.is_empty() {
                        "No commits, or this folder is not a git repository.".into()
                    } else {
                        "No commits match.".into()
                    })),
                    (Ok(list), _) => {
                        h.commits = Arc::new(list);
                        None
                    }
                    (Err(e), _) => Some(Note::Error(format!("{e:#}").into())),
                };
                cx.notify();
            });
        }));
    }

    /// Run `git init` in the folder, then load its (empty) history.
    fn init_repo(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        if self.initializing {
            return;
        }
        self.initializing = true;
        self.init_error = None;
        let work = cx.background_spawn(async move { git::init(&root) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |h, cx| {
                h.initializing = false;
                if let Err(e) = result {
                    h.init_error = Some(format!("{e:#}").into());
                }
                h.refresh(cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// The commit list's stand-in: an empty state, or an error that wraps
    /// inside the pane.
    fn render_note(&self, note: &Note, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        match note {
            Note::Text(text) => kit::empty_state(icons::GIT_BRANCH, "History", text, t).into_any_element(),
            Note::NoCommitsYet => page::empty_state(
                t,
                icons::GIT_BRANCH,
                "No commits yet",
                "Commit from the Diffs tab to start this folder's history.",
                None,
            )
            .into_any_element(),
            Note::Error(e) => div()
                .p(px(12.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(page::error_strip(t, e.clone()))
                .child(
                    div().flex().child(
                        ctl::button_sm(t, "hist-retry", "Try again", Tone::Filled)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    ),
                )
                .into_any_element(),
            Note::NotRepo => {
                let folder = self
                    .root
                    .as_ref()
                    .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| "This folder".into());
                let actions = div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(8.0))
                    .child(
                        ctl::button(
                            t,
                            "hist-git-init",
                            if self.initializing { "Initializing…" } else { "Initialize git" },
                            Tone::Solid,
                        )
                        .when(self.initializing, |b| b.opacity(0.6))
                        .on_click(cx.listener(|this, _, _, cx| this.init_repo(cx))),
                    )
                    .child(
                        ctl::button(t, "hist-open-folder", "Open folder", Tone::Outlined)
                            .child(icon(icons::ARROW_UP_RIGHT).size(px(12.0)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(root) = &this.root {
                                    cx.open_with_system(root);
                                }
                            })),
                    );
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(page::empty_state(
                        t,
                        icons::GIT_BRANCH,
                        "Not a git repository",
                        format!("{folder} has no git history. Initialize a repository to start one."),
                        Some(actions.into_any_element()),
                    ))
                    .when_some(self.init_error.clone(), |d, e| {
                        d.child(div().w_full().max_w(px(420.0)).px(px(16.0)).child(page::error_strip(t, e)))
                    })
                    .into_any_element()
            }
        }
    }
}

impl Render for GitHistory {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("GitHistory");
        let t = Theme::of(cx).clone();
        let search = div()
            .m(px(8.0))
            .h(px(30.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .rounded(px(8.0))
            .bg(t.wash(0.05))
            .border_1()
            .border_color(t.border)
            .text_size(ty::BODY)
            .child(icon(icons::MAGNIFER).size(px(13.0)).text_color(t.text_faint))
            .child(div().flex_1().child(Input::new(&self.search)));

        let commits = self.commits.clone();
        let lanes = commits.iter().map(|c| c.lane).max().unwrap_or(0) + 1;
        let rail = (lanes as f32 * 10.0 + 8.0).min(72.0);
        let weak = cx.weak_entity();
        let list: AnyElement = if let Some(note) = self.note.clone() {
            if commits.is_empty() { self.render_note(&note, &t, cx) } else { div().into_any_element() }
        } else {
            uniform_list("git-history", commits.len(), move |range, _, cx| {
                let t = Theme::of(cx);
                range
                    .map(|i| {
                        let c = &commits[i];
                        let sha = c.sha.clone();
                        let subject = c.subject.clone();
                        let weak = weak.clone();
                        let mut dots = div().w(px(rail)).flex_none().flex().items_center().gap(px(4.0));
                        for lane in 0..lanes {
                            let on = c.lane == lane;
                            dots = dots.child(div().size(px(if on { 8.0 } else { 5.0 })).rounded_full().bg(if on {
                                t.accent
                            } else {
                                t.wash(0.18)
                            }));
                        }
                        div()
                            .id(("hist-row", i))
                            .h(px(44.0))
                            .px(px(10.0))
                            .mx(px(4.0))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .rounded(px(8.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.wash(0.07)))
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |_, cx| {
                                    cx.emit(HistoryEvent::OpenCommit { sha: sha.clone(), subject: subject.clone() });
                                });
                            })
                            .child(dots)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.0))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.0))
                                            .child(
                                                div()
                                                    .font_family(t.font_mono.clone())
                                                    .text_size(ty::CAPTION)
                                                    .text_color(t.accent)
                                                    .child(c.short.clone()),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(ty::BODY)
                                                    .text_color(t.text)
                                                    .child(c.subject.clone()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.0))
                                            .text_size(ty::CAPTION)
                                            .text_color(t.text_faint)
                                            .child(c.author.clone())
                                            .child("·")
                                            .child(c.when.clone())
                                            .children(c.refs.iter().take(2).map(|r| {
                                                div()
                                                    .px(px(5.0))
                                                    .rounded(px(4.0))
                                                    .bg(t.wash(0.08))
                                                    .text_color(t.text_muted)
                                                    .child(r.clone())
                                            })),
                                    ),
                            )
                            .into_any_element()
                    })
                    .collect()
            })
            .flex_1()
            .into_any_element()
        };

        div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(search)
            .child(div().flex_1().min_h_0().overflow_hidden().child(list))
    }
}
