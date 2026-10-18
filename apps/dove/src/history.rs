//! Git history for the side panel: the branch you are on, a search, and
//! the commits as a connected lane graph. A click opens the commit as a diff.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use dove_core::git::{self, GraphRow, HistoryCommit};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Bounds, ClipboardItem, Context, Entity, EventEmitter, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, PathBuilder, Pixels, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, canvas, div, hsla, point, px, relative, uniform_list,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::design::{radius, ty};
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
    /// The commit last opened as a diff, kept highlighted.
    selected: Option<String>,
    _load: Option<gpui::Task<()>>,
    /// Typing settles for a moment before the log is searched again.
    _debounce: Option<gpui::Task<()>>,
    _sub: Subscription,
}

/// Row height, lane spacing and the graph's side padding.
const ROW_H: f32 = 48.0;
const LANE_W: f32 = 10.0;
const RAIL_PAD: f32 = 8.0;
/// The rail's left inset: lane 0 is centred 23 px in, under the search
/// box's glyph and the header's branch glyph.
const RAIL_LEFT: f32 = 23.0 - LANE_W / 2.0;
/// Lanes beyond this are not drawn (the rail stays a rail).
const MAX_LANES: usize = 6;
/// How many commits a load asks git for.
const LIMIT: usize = 200;

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
                this._debounce = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(180)).await;
                    let _ = this.update(cx, |h, cx| h.refresh(cx));
                }));
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
            selected: None,
            _load: None,
            _debounce: None,
            _sub,
        }
    }

    pub fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        if root == self.root {
            return;
        }
        self.root = root;
        self.init_error = None;
        self.selected = None;
        self.commits = Arc::default();
        self.note = None;
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
            let result = git::history(&root, &q, LIMIT);
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

impl GitHistory {
    /// Open `c` as a diff and keep its row highlighted.
    fn open(&mut self, sha: String, subject: String, cx: &mut Context<Self>) {
        self.selected = Some(sha.clone());
        cx.emit(HistoryEvent::OpenCommit { sha, subject });
        cx.notify();
    }

    /// The branch HEAD is on (or "Detached HEAD"), and how many commits the
    /// list holds.
    fn render_header(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let branch: Option<SharedString> = self
            .commits
            .iter()
            .find(|c| c.head)
            .map(|c| head_branch(c).map(|b| b.to_string().into()).unwrap_or_else(|| "Detached HEAD".into()));
        let count = self.commits.len();
        let count_label = match count {
            0 => None,
            n if n >= LIMIT => Some(format!("Latest {n}")),
            1 => Some("1 commit".to_string()),
            n => Some(format!("{n} commits")),
        };
        div()
            .h(px(34.0))
            .pl(px(17.0))
            .pr(px(8.0))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .child(icon(icons::GIT_BRANCH).size(px(13.0)).text_color(t.text_muted))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(ty::BODY)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(t.text)
                    .child(branch.unwrap_or_else(|| "History".into())),
            )
            .when_some(count_label, |d, label| {
                d.child(div().flex_none().text_size(ty::CAPTION).text_color(t.text_faint).child(label))
            })
            .child(div().flex_1())
            .child(
                ui::icon_button(
                    "hist-refresh",
                    icons::REFRESH,
                    "Refresh",
                    t,
                    cx.listener(|this, _, _, cx| this.refresh(cx)),
                )
                .flex_none(),
            )
            .into_any_element()
    }

    /// Placeholder rows while the first log loads.
    fn render_loading(&self, t: &Theme) -> AnyElement {
        let widths = [0.72, 0.58, 0.81, 0.49, 0.66, 0.54];
        div()
            .pt(px(4.0))
            .flex()
            .flex_col()
            .children(widths.iter().map(|w| {
                div()
                    .h(px(ROW_H))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(div().w(px(LANE_W + RAIL_PAD)).ml(px(4.0)).flex().justify_center().child(page::skeleton(
                        t,
                        px(8.0).into(),
                        8.0,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(7.0))
                            .child(page::skeleton(t, relative(*w), 11.0))
                            .child(page::skeleton(t, relative(w * 0.5), 9.0)),
                    )
            }))
            .into_any_element()
    }
}

/// The local branch HEAD points at, when it is on one.
fn head_branch(c: &HistoryCommit) -> Option<&str> {
    c.refs.first().map(String::as_str).filter(|r| c.head && !r.starts_with("tag: ") && !r.contains('/'))
}

/// One lane's colour: the accent for the main line, then a fixed set of
/// distinct, calm hues so neighbouring branches stay apart.
fn lane_color(lane: usize, t: &Theme) -> Hsla {
    let dark = t.appearance.is_dark();
    let l = if dark { 0.66 } else { 0.46 };
    match lane % 6 {
        0 => t.accent,
        1 => hsla(0.42, 0.55, l, 1.0),
        2 => hsla(0.78, 0.55, l + 0.04, 1.0),
        3 => hsla(0.09, 0.70, l, 1.0),
        4 => hsla(0.53, 0.60, l, 1.0),
        _ => hsla(0.95, 0.55, l + 0.02, 1.0),
    }
}

/// Paint one row of the graph: the lines through it, the bends into and
/// out of its node, then the node.
fn paint_graph(
    bounds: Bounds<Pixels>,
    g: &GraphRow,
    lane: usize,
    merge: bool,
    head: bool,
    t: &Theme,
    window: &mut Window,
) {
    let x = |i: usize| bounds.origin.x + px(RAIL_LEFT + LANE_W * i as f32 + LANE_W / 2.0);
    let top = bounds.origin.y;
    let bottom = bounds.origin.y + bounds.size.height;
    let mid = bounds.origin.y + bounds.size.height / 2.0;
    let node = point(x(lane), mid);
    let stroke = |from: gpui::Point<Pixels>, to: gpui::Point<Pixels>, color: Hsla, window: &mut Window| {
        let mut b = PathBuilder::stroke(px(1.5));
        b.move_to(from);
        if from.x == to.x {
            b.line_to(to);
        } else {
            // A soft S-bend between lanes.
            let ym = (from.y + to.y) / 2.0;
            b.cubic_bezier_to(to, point(from.x, ym), point(to.x, ym));
        }
        if let Ok(path) = b.build() {
            window.paint_path(path, color);
        }
    };
    // Each segment is drawn once, in an opaque colour, so nothing doubles
    // up into a brighter stripe where segments meet.
    let visible = |i: usize| i < MAX_LANES;
    for &i in g.top.iter().filter(|&&i| visible(i)) {
        let color = lane_color(i, t);
        if i == lane || g.joins.contains(&i) {
            stroke(point(x(i), top), node, color, window);
        } else if g.bottom.contains(&i) {
            // A lane passing straight through: one line, edge to edge.
            stroke(point(x(i), top), point(x(i), bottom), color, window);
        } else {
            stroke(point(x(i), top), point(x(i), mid), color, window);
        }
    }
    for &i in g.bottom.iter().filter(|&&i| visible(i)) {
        let color = lane_color(i, t);
        let through = g.top.contains(&i) && i != lane && !g.joins.contains(&i);
        if i == lane || g.forks.contains(&i) {
            stroke(node, point(x(i), bottom), color, window);
        } else if !through {
            stroke(point(x(i), mid), point(x(i), bottom), color, window);
        }
    }
    if !visible(lane) {
        return;
    }
    // The node: a filled dot; a merge is a ring; HEAD gets a halo.
    let color = lane_color(lane, t);
    let disc = |r: f32, fill: Hsla, window: &mut Window| {
        let mut b = PathBuilder::fill();
        let k = 0.552_284_8 * r;
        let (cx, cy) = (node.x, node.y);
        let p = |dx: f32, dy: f32| point(cx + px(dx), cy + px(dy));
        b.move_to(p(r, 0.0));
        b.cubic_bezier_to(p(0.0, r), p(r, k), p(k, r));
        b.cubic_bezier_to(p(-r, 0.0), p(-k, r), p(-r, k));
        b.cubic_bezier_to(p(0.0, -r), p(-r, -k), p(-k, -r));
        b.cubic_bezier_to(p(r, 0.0), p(k, -r), p(r, -k));
        b.close();
        if let Ok(path) = b.build() {
            window.paint_path(path, fill);
        }
    };
    if head {
        disc(7.0, color.opacity(0.22), window);
    }
    disc(4.5, color, window);
    if merge {
        disc(2.5, t.bg, window);
    }
}

impl Render for GitHistory {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("GitHistory");
        let t = Theme::of(cx).clone();
        let search = div()
            .mx(px(8.0))
            .mb(px(6.0))
            .h(px(30.0))
            .px(px(8.0))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .rounded(px(radius::RADIUS))
            .bg(t.wash(0.05))
            .border_1()
            .border_color(t.border)
            .text_size(ty::BODY)
            .child(icon(icons::MAGNIFER).size(px(13.0)).text_color(t.text_faint))
            .child(div().flex_1().child(Input::new(&self.search)));

        let commits = self.commits.clone();
        let lanes = commits
            .iter()
            .map(|c| {
                let g = &c.graph;
                g.top.iter().chain(&g.bottom).copied().chain(std::iter::once(c.lane)).max().unwrap_or(0) + 1
            })
            .max()
            .unwrap_or(1)
            .min(MAX_LANES);
        let rail = RAIL_LEFT + RAIL_PAD + LANE_W * lanes as f32;
        let selected = self.selected.clone();
        let weak = cx.weak_entity();
        let list: AnyElement = if let Some(note) = self.note.clone() {
            if commits.is_empty() { self.render_note(&note, &t, cx) } else { div().into_any_element() }
        } else if commits.is_empty() {
            // The first `git log` is still running.
            self.render_loading(&t)
        } else {
            uniform_list("git-history", commits.len(), move |range, _, cx| {
                let t = Theme::of(cx).clone();
                range
                    .map(|i| {
                        let c = &commits[i];
                        let on = selected.as_deref() == Some(c.sha.as_str());
                        let group = SharedString::from(format!("hist-row-{i}"));
                        let (sha, subject) = (c.sha.clone(), c.subject.clone());
                        let open = weak.clone();
                        let copy_sha = c.sha.clone();
                        let graph = c.graph.clone();
                        let (lane, merge, head) = (c.lane, c.parents.len() > 1, c.head);
                        let paint_t = t.clone();
                        let rail_el = canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                paint_graph(bounds, &graph, lane, merge, head, &paint_t, window)
                            },
                        )
                        .w(px(rail))
                        .h_full()
                        .flex_none();
                        let current = head_branch(c);
                        let chips = c.refs.iter().take(3).map(|r| ref_chip(r, current == Some(r.as_str()), &t));
                        div()
                            .id(("hist-row", i))
                            .group(group.clone())
                            .h(px(ROW_H))
                            .mx(px(4.0))
                            .pr(px(8.0))
                            .flex()
                            .items_center()
                            .rounded(px(radius::RADIUS))
                            .cursor_pointer()
                            .when(on, |d| d.bg(ui::selected_bg(&t)))
                            .when(!on, |d| d.hover(|s| s.bg(t.wash(0.05))))
                            .on_click(move |_, _, cx| {
                                let _ = open.update(cx, |h, cx| h.open(sha.clone(), subject.clone(), cx));
                            })
                            .child(rail_el)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.0))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.0))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(ty::BODY)
                                                    .when(c.head, |d| d.font_weight(gpui::FontWeight::MEDIUM))
                                                    .text_color(t.text)
                                                    .child(c.subject.clone()),
                                            )
                                            .child(
                                                div()
                                                    .flex_none()
                                                    .text_size(ty::CAPTION)
                                                    .text_color(t.text_faint)
                                                    .child(c.when.clone()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(6.0))
                                            .min_w_0()
                                            .text_size(ty::CAPTION)
                                            .text_color(t.text_faint)
                                            .child(
                                                div()
                                                    .flex_none()
                                                    .font_family(t.font_mono.clone())
                                                    .text_color(t.text_muted)
                                                    .child(c.short.clone()),
                                            )
                                            .child(div().flex_none().child("·"))
                                            .child(div().min_w_0().truncate().child(c.author.clone()))
                                            .children(chips),
                                    ),
                            )
                            // Hover: copy the full hash; the chevron says the row opens.
                            .child(
                                div()
                                    .flex_none()
                                    .ml(px(4.0))
                                    .flex()
                                    .items_center()
                                    .gap(px(2.0))
                                    .invisible()
                                    .group_hover(group, |s| s.visible())
                                    .child(
                                        div()
                                            .id(("hist-copy", i))
                                            .size(px(22.0))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(radius::RADIUS))
                                            .hover(|s| s.bg(t.wash(0.10)))
                                            .tooltip(ui::tip("Copy commit hash"))
                                            .on_click(move |_, _, cx| {
                                                cx.stop_propagation();
                                                cx.write_to_clipboard(ClipboardItem::new_string(copy_sha.clone()));
                                            })
                                            .child(icon(icons::COPY).size(px(12.0)).text_color(t.text_muted)),
                                    )
                                    .child(icon(icons::ALT_ARROW_RIGHT).size(px(12.0)).text_color(t.text_faint)),
                            )
                            .into_any_element()
                    })
                    .collect()
            })
            .size_full()
            .into_any_element()
        };

        // The list area is a flex column, so the virtualized list gets the
        // remaining height (a uniform list with no definite height draws
        // nothing).
        div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(self.render_header(&t, cx))
            .child(search)
            .child(div().flex_1().min_h_0().overflow_hidden().flex().flex_col().child(list))
    }
}

/// A branch, remote or tag label. The branch HEAD is on reads strongest;
/// remotes are outlined; tags carry a tag glyph.
fn ref_chip(r: &str, current: bool, t: &Theme) -> AnyElement {
    let (label, tag) = match r.strip_prefix("tag: ") {
        Some(name) => (name.to_string(), true),
        None => (r.to_string(), false),
    };
    let remote = !tag && label.contains('/');
    div()
        .flex_none()
        .max_w(px(160.0))
        .h(px(16.0))
        .px(px(5.0))
        .flex()
        .items_center()
        .gap(px(3.0))
        .rounded(px(4.0))
        .text_size(ty::BADGE)
        .when(current, |d| d.bg(t.accent.opacity(0.16)).text_color(t.accent))
        .when(!current && remote, |d| d.border_1().border_color(t.wash(0.12)).text_color(t.text_faint))
        .when(!current && !remote, |d| d.bg(t.wash(0.08)).text_color(t.text_muted))
        .child(icon(if tag { icons::TAG } else { icons::GIT_BRANCH }).size(px(9.0)).text_color(if current {
            t.accent
        } else {
            t.text_faint
        }))
        .child(div().min_w_0().truncate().child(label))
        .into_any_element()
}
