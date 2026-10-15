//! Git history for the side panel: a lane graph, search, and a click that
//! opens the commit as a diff.

use std::path::PathBuf;
use std::sync::Arc;

use dove_core::git::{self, HistoryCommit};
use gpui::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    px, uniform_list,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::design::ty;
use crate::kit;
use crate::theme::Theme;
use crate::ui;

pub enum HistoryEvent {
    OpenCommit { sha: String, subject: String },
}

pub struct GitHistory {
    root: Option<PathBuf>,
    commits: Arc<Vec<HistoryCommit>>,
    search: Entity<InputState>,
    query: String,
    note: Option<SharedString>,
    _load: Option<gpui::Task<()>>,
    _sub: Subscription,
}

impl EventEmitter<HistoryEvent> for GitHistory {}

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
        Self { root: None, commits: Arc::default(), search, query: String::new(), note: None, _load: None, _sub }
    }

    pub fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        if root == self.root {
            return;
        }
        self.root = root;
        self.refresh(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            self.commits = Arc::default();
            self.note = Some("Open a thread to see its history.".into());
            cx.notify();
            return;
        };
        let q = self.query.clone();
        let work = cx.background_spawn(async move { git::history(&root, &q, 200) });
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |h, cx| {
                match result {
                    Ok(list) if list.is_empty() => {
                        h.commits = Arc::default();
                        h.note = Some(if h.query.is_empty() {
                            "No commits, or this folder is not a git repository.".into()
                        } else {
                            "No commits match.".into()
                        });
                    }
                    Ok(list) => {
                        h.commits = Arc::new(list);
                        h.note = None;
                    }
                    Err(e) => {
                        h.commits = Arc::default();
                        h.note = Some(format!("{e:#}").into());
                    }
                }
                cx.notify();
            });
        }));
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
        let list: AnyElement = if let Some(note) = &self.note {
            if commits.is_empty() {
                kit::empty_state(icons::GIT_BRANCH, "History", note, &t).into_any_element()
            } else {
                div().into_any_element()
            }
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

        div().size_full().flex().flex_col().child(search).child(div().flex_1().min_h_0().child(list))
    }
}
