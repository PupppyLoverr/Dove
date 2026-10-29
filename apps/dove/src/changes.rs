//! What the agent changed, and the way out: review, stage, commit, push,
//! open a pull request, watch its checks, merge.
//!
//! The diff itself is a [`DiffList`] in worktree mode (Zeron's Changes pane,
//! MIT): virtualized rows, folds, syntax colours and line comments. This pane
//! feeds it from git and owns what git does next: stage and discard from
//! the file headers, commit, push, PR. Comments collect into one review the
//! agent receives as a message.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use dove_core::git::{self, FileDiff, github};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    px,
};
use gpui_base::input::{Input, InputState};

use crate::assets::{icon, icons};
use crate::design::{radius, ty};
use crate::diff_list::{self, DiffList, DiffListEvent, DiffListMode, expand_tabs};
use crate::kit;
use crate::kit::controls::{self as ctl, Tone};
use crate::kit::page;
use crate::surface_chrome;
use crate::theme::{Theme, ink};
use crate::ui;

/// `transcript::MiniDiff` draws its inline diffs with the pane's own rows.
pub(crate) use crate::diff_list::{highlight_file, inline_rows};

pub enum ChangesEvent {
    /// Send this text to the thread's agent (a review).
    SendToAgent(String),
}

/// Where the branch stands on its way to a merged PR.
#[derive(Clone, Default)]
struct Delivery {
    branch: Option<String>,
    has_remote: bool,
    unpushed: Option<usize>,
    pr: Option<github::PrDetail>,
    gh: bool,
    busy: Option<&'static str>,
    note: Option<SharedString>,
}

pub struct Changes {
    target: Option<(PathBuf, String)>,
    /// The working tree target, to come back to from a commit.
    tree: Option<(PathBuf, String)>,
    /// When set, the panel shows that commit rather than the working tree.
    commit: Option<String>,
    title: String,
    files: Arc<Vec<FileDiff>>,
    /// The rows: folds, layout, highlighting and line comments.
    diff: Entity<DiffList>,
    staged: HashSet<String>,
    loading: bool,
    loaded: bool,
    error: Option<SharedString>,
    /// The folder is not inside a git repository: the panel offers to
    /// initialize one instead of showing git's stderr.
    not_repo: bool,
    /// `git init` is running from the not-a-repository state.
    initializing: bool,
    /// Why the last `git init` failed.
    init_error: Option<SharedString>,
    message: Entity<InputState>,
    delivery: Delivery,
    _load: Option<gpui::Task<()>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<ChangesEvent> for Changes {}

impl Changes {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let style = ui::input_style(Theme::of(cx));
        let message = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Commit message");
            s.set_editor_style(style);
            s
        });
        let diff = cx.new(|cx| DiffList::new(DiffListMode::Worktree, cx));
        let sub = cx.subscribe(&diff, |this, _, ev: &DiffListEvent, cx| match ev {
            // The toolbar mirrors the list's toggles and comment count.
            DiffListEvent::Changed => cx.notify(),
            DiffListEvent::ToggleStage(path) => this.toggle_stage(path.clone(), cx),
            DiffListEvent::Discard(path) => this.discard(path.clone(), cx),
            DiffListEvent::TopFile(_) => {}
        });
        Self {
            target: None,
            tree: None,
            commit: None,
            title: String::new(),
            files: Arc::default(),
            diff,
            staged: HashSet::new(),
            loading: false,
            loaded: false,
            error: None,
            not_repo: false,
            initializing: false,
            init_error: None,
            message,
            delivery: Delivery { gh: github::gh_available(), ..Default::default() },
            _load: None,
            _subs: vec![sub],
        }
    }

    /// Show changes in `dir` relative to `base`; `title` drafts the commit
    /// message and PR title.
    pub fn set_target(
        &mut self,
        dir: Option<PathBuf>,
        base: Option<String>,
        title: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = dir.map(|d| (d, base.unwrap_or_else(|| "HEAD".into())));
        if target == self.target && self.commit.is_none() {
            return;
        }
        self.tree = target.clone();
        self.target = target;
        self.commit = None;
        self.title = title.to_string();
        self.clear(cx);
        let title = title.to_string();
        self.message.update(cx, |s, cx| s.set_value(title, window, cx));
        self.refresh(cx);
    }

    pub fn show_commit(
        &mut self,
        dir: PathBuf,
        sha: String,
        subject: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.target = Some((dir, sha.clone()));
        self.commit = Some(sha);
        self.clear(cx);
        self.message.update(cx, |s, cx| s.set_value(subject.to_string(), window, cx));
        self.refresh(cx);
    }

    /// Leave a commit for the working tree again.
    fn show_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.target = self.tree.clone();
        self.commit = None;
        self.clear(cx);
        let title = self.title.clone();
        self.message.update(cx, |s, cx| s.set_value(title, window, cx));
        self.refresh(cx);
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.files = Arc::default();
        self.loaded = false;
        self.not_repo = false;
        self.init_error = None;
        // Stage, discard and comments belong to the working tree, not a commit.
        let tree = self.commit.is_none();
        self.diff.update(cx, |d, cx| {
            d.clear(cx);
            d.set_tree_actions(tree, cx);
        });
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some((dir, base)) = self.target.clone() else {
            cx.notify();
            return;
        };
        self.loading = true;
        let gh = self.delivery.gh;
        let commit = self.commit.clone();
        let work = cx.background_spawn(async move {
            let files = match commit.as_deref() {
                Some(sha) => git::commit_diff(&dir, sha),
                // Before the first commit HEAD names nothing: everything in
                // the folder is new, so diff against the empty tree.
                None => git::changes(&dir, &base).or_else(|e| {
                    if base == "HEAD" && git::is_unborn(&dir) { git::changes(&dir, git::EMPTY_TREE) } else { Err(e) }
                }),
            }
            .map(expand_tabs);
            let not_repo = files.is_err() && dir.is_dir() && git::repo_root(&dir).is_none();
            let staged = git::staged(&dir).unwrap_or_default();
            let branch = git::current_branch(&dir).ok();
            let has_remote = git::has_remote(&dir);
            let unpushed = git::unpushed(&dir);
            let pr = if gh && has_remote { github::pr_for_branch(&dir).ok().flatten() } else { None };
            (files, not_repo, staged, branch, has_remote, unpushed, pr)
        });
        self._load = Some(cx.spawn(async move |this, cx| {
            let (files, not_repo, staged, branch, has_remote, unpushed, pr) = work.await;
            let _ = this.update(cx, |v, cx| {
                v.loading = false;
                v.loaded = true;
                v.not_repo = not_repo;
                match files {
                    Ok(files) => {
                        v.error = None;
                        if *v.files != files {
                            v.files = Arc::new(files);
                            let files = v.files.clone();
                            v.diff.update(cx, |d, cx| d.set_files(files, cx));
                        }
                    }
                    Err(e) => v.error = Some(format!("{e:#}").into()),
                }
                v.staged = staged.into_iter().collect();
                let staged = v.staged.clone();
                v.diff.update(cx, |d, cx| d.set_staged(staged, cx));
                v.delivery.branch = branch;
                v.delivery.has_remote = has_remote;
                v.delivery.unpushed = unpushed;
                v.delivery.pr = pr;
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub fn summary(&self) -> (usize, u32, u32) {
        (self.files.len(), self.files.iter().map(|f| f.additions).sum(), self.files.iter().map(|f| f.deletions).sum())
    }

    // ------------------------------------------------------------ git

    /// Run `git init` in the target folder, then load its changes.
    fn init_repo(&mut self, cx: &mut Context<Self>) {
        let Some((dir, _)) = self.target.clone() else { return };
        if self.initializing {
            return;
        }
        self.initializing = true;
        self.init_error = None;
        let work = cx.background_spawn(async move { git::init(&dir) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |v, cx| {
                v.initializing = false;
                match result {
                    Ok(()) => {
                        v.not_repo = false;
                        v.error = None;
                        v.loaded = false;
                    }
                    Err(e) => v.init_error = Some(format!("{e:#}").into()),
                }
                v.refresh(cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// Show the target folder in the system file manager.
    fn open_folder(&self, cx: &mut Context<Self>) {
        if let Some((dir, _)) = &self.target {
            cx.open_with_system(dir);
        }
    }

    fn run_git(
        &mut self,
        label: &'static str,
        cx: &mut Context<Self>,
        op: impl FnOnce(PathBuf, String) -> anyhow::Result<Option<String>> + Send + 'static,
    ) {
        let Some((dir, base)) = self.target.clone() else { return };
        self.delivery.busy = Some(label);
        self.delivery.note = None;
        let work = cx.background_spawn(async move { op(dir, base) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |v, cx| {
                v.delivery.busy = None;
                match result {
                    Ok(note) => v.delivery.note = note.map(Into::into),
                    Err(e) => v.delivery.note = Some(format!("{label} failed: {e:#}").into()),
                }
                v.refresh(cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_stage(&mut self, path: String, cx: &mut Context<Self>) {
        let staged = self.staged.contains(&path);
        // Optimistic: flip now, the refresh confirms.
        if staged {
            self.staged.remove(&path);
        } else {
            self.staged.insert(path.clone());
        }
        let now = self.staged.clone();
        self.diff.update(cx, |d, cx| d.set_staged(now, cx));
        self.run_git(if staged { "Unstage" } else { "Stage" }, cx, move |dir, _| {
            if staged {
                git::unstage(&dir, &path)?
            } else {
                git::stage(&dir, &path)?
            }
            Ok(None)
        });
    }

    fn discard(&mut self, path: String, cx: &mut Context<Self>) {
        self.run_git("Discard", cx, move |dir, base| {
            git::discard_file(&dir, &base, &path)?;
            Ok(Some(format!("Discarded {path}")))
        });
    }

    fn commit(&mut self, cx: &mut Context<Self>) {
        let message = self.message.read(cx).value().trim().to_string();
        if message.is_empty() {
            self.delivery.note = Some("Write a commit message first".into());
            cx.notify();
            return;
        }
        let only_staged = !self.staged.is_empty();
        self.run_git("Commit", cx, move |dir, _| {
            let made = if only_staged { git::commit_staged(&dir, &message)? } else { git::commit_all(&dir, &message)? };
            Ok(Some(if made { "Committed".into() } else { "Nothing to commit".into() }))
        });
    }

    fn push(&mut self, cx: &mut Context<Self>) {
        self.run_git("Push", cx, |dir, _| {
            git::push(&dir)?;
            Ok(Some("Pushed".into()))
        });
    }

    fn open_pr(&mut self, cx: &mut Context<Self>) {
        let title = self.message.read(cx).value().lines().next().unwrap_or(&self.title).trim().to_string();
        let title = if title.is_empty() { self.title.clone() } else { title };
        let body = self.pr_body();
        self.run_git("Open PR", cx, move |dir, _| {
            if git::unpushed(&dir).is_none_or(|n| n > 0) {
                git::push(&dir)?;
            }
            let base = git::default_branch(&dir);
            let url = github::create_pr(&dir, &title, &body, base.as_deref(), false)?;
            Ok(Some(format!("Opened {url}")))
        });
    }

    fn merge(&mut self, cx: &mut Context<Self>) {
        let Some(n) = self.delivery.pr.as_ref().map(|p| p.number) else { return };
        self.run_git("Merge", cx, move |dir, _| {
            github::merge_pr(&dir, n)?;
            Ok(Some(format!("Merged #{n}")))
        });
    }

    /// A PR body drafted from the thread and the change list.
    fn pr_body(&self) -> String {
        let (n, adds, dels) = self.summary();
        let files: Vec<String> =
            self.files.iter().take(30).map(|f| format!("- `{}` (+{} −{})", f.path, f.additions, f.deletions)).collect();
        format!("{}\n\n{n} files changed, +{adds} −{dels}\n\n{}\n\n_Opened from Keplorayx._", self.title, files.join("\n"))
    }

    /// Send the line comments to the agent as one review.
    fn send_review(&mut self, cx: &mut Context<Self>) {
        if let Some(review) = self.diff.update(cx, |d, cx| d.take_review(cx)) {
            cx.emit(ChangesEvent::SendToAgent(review));
            cx.notify();
        }
    }

    // ------------------------------------------------------------ chrome

    fn render_toolbar(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (n, adds, dels) = self.summary();
        let label = match (&self.commit, &self.target) {
            (Some(_), _) => format!("{n} Changed {} in this commit", if n == 1 { "file" } else { "files" }),
            (None, Some((_, base))) if base != "HEAD" => {
                format!("{n} Changed {} vs {base}", if n == 1 { "file" } else { "files" })
            }
            _ if n == 1 => "1 Uncommitted change".into(),
            _ => format!("{n} Uncommitted changes"),
        };
        let comments = self.diff.read(cx).comment_count();
        surface_chrome::toolbar(t)
            .px(px(12.0))
            .gap(px(8.0))
            .when_some(self.commit.clone(), |d, sha| {
                d.child(
                    div()
                        .flex_none()
                        .h(px(surface_chrome::CONTROL_SIZE))
                        .px(px(6.0))
                        .flex()
                        .items_center()
                        .rounded(px(surface_chrome::CONTROL_RADIUS))
                        .bg(ink(0.05))
                        .font_family(t.font_mono.clone())
                        .text_size(ty::BADGE)
                        .text_color(t.text_muted)
                        .child(sha.chars().take(7).collect::<String>()),
                )
            })
            .when(self.loaded && self.error.is_none(), |d| {
                d.child(div().min_w_0().truncate().text_size(ty::META).text_color(t.text_muted).child(label))
                    .child(
                        div()
                            .flex_none()
                            .font_family(t.font_mono.clone())
                            .text_size(ty::MICRO)
                            .text_color(t.diff_add)
                            .child(format!("+{adds}")),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family(t.font_mono.clone())
                            .text_size(ty::MICRO)
                            .text_color(t.diff_del)
                            .child(format!("−{dels}")),
                    )
            })
            .child(div().flex_1())
            .when(comments > 0, |d| {
                d.child(
                    ui::button_primary("chg-review", format!("Send review ({comments})"), t)
                        .on_click(cx.listener(|this, _, _, cx| this.send_review(cx))),
                )
            })
            .when(self.commit.is_some() && self.tree.is_some(), |d| {
                d.child(
                    ui::button_ghost("chg-tree", "Working tree", t)
                        .on_click(cx.listener(|this, _, window, cx| this.show_tree(window, cx))),
                )
            })
            .children(diff_list::view_toggles(&self.diff, t, cx))
            .child(
                diff_list::toolbar_toggle("diff-refresh", icons::REFRESH, "Refresh", false, t)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            )
            .into_any_element()
    }

    fn render_footer(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let d = &self.delivery;
        let staged = self.staged.len();
        let has_changes = !self.files.is_empty();
        let busy = d.busy;
        let mut steps = div().flex().items_center().gap(px(6.0)).flex_wrap();
        steps = steps.child(
            ui::button_primary(
                "chg-commit",
                match (busy, staged) {
                    (Some("Commit"), _) => "Committing…".to_string(),
                    (_, 0) => "Commit all".to_string(),
                    (_, n) => format!("Commit {n} staged"),
                },
                t,
            )
            .when(!has_changes, |b| b.opacity(0.4))
            .on_click(cx.listener(|this, _, _, cx| this.commit(cx))),
        );
        if d.has_remote {
            steps = steps.child(
                ui::button_ghost("chg-push", if busy == Some("Push") { "Pushing…" } else { "Push" }, t)
                    .child(match d.unpushed {
                        Some(0) => kit::badge("up to date", t.text_faint).into_any_element(),
                        Some(n) => kit::badge(format!("{n} ahead"), t.accent).into_any_element(),
                        None => kit::badge("new branch", t.text_faint).into_any_element(),
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.push(cx))),
            );
            match (&d.pr, d.gh) {
                (_, false) => {
                    steps = steps.child(
                        div()
                            .text_size(ty::META)
                            .text_color(t.text_faint)
                            .child("Install the GitHub CLI (gh) to open PRs"),
                    );
                }
                (None, true) => {
                    steps = steps.child(
                        ui::button_ghost("chg-pr", if busy == Some("Open PR") { "Opening…" } else { "Open PR" }, t)
                            .child(icon(icons::PULL_REQUEST).size(px(13.0)))
                            .on_click(cx.listener(|this, _, _, cx| this.open_pr(cx))),
                    );
                }
                (Some(pr), true) => {
                    let summary = pr.checks_summary();
                    let (color, label) = match summary {
                        "passing" => (t.success, "Checks passing"),
                        "failing" => (t.danger, "Checks failing"),
                        "pending" => (t.warning, "Checks running"),
                        _ => (t.text_faint, "No checks"),
                    };
                    let url = pr.url.clone();
                    steps = steps
                        .child(
                            div()
                                .id("chg-pr-link")
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .text_size(ty::SMALL)
                                .cursor_pointer()
                                .hover(|s| s.text_color(t.text))
                                .text_color(t.text_muted)
                                .child(icon(icons::PULL_REQUEST).size(px(13.0)))
                                .child(format!("#{} {}", pr.number, pr.state.to_lowercase()))
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        )
                        .child(kit::badge(label, color))
                        .when(pr.state == "OPEN", |d| {
                            d.child(
                                ui::button_ghost(
                                    "chg-merge",
                                    if busy == Some("Merge") { "Merging…" } else { "Merge" },
                                    t,
                                )
                                .when(summary == "failing", |b| b.opacity(0.5))
                                .on_click(cx.listener(|this, _, _, cx| this.merge(cx))),
                            )
                        });
                }
            }
        } else {
            steps =
                steps.child(div().text_size(ty::META).text_color(t.text_faint).child("No remote: commits stay local"));
        }
        div()
            .p(px(10.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .border_t_1()
            .border_color(t.border)
            .child(
                div()
                    .h(px(32.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .rounded(px(radius::RADIUS))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.wash(0.03))
                    .text_size(ty::BODY)
                    .child(div().flex_1().child(Input::new(&self.message))),
            )
            .child(steps)
            .when_some(d.note.clone(), |el, n| el.child(div().text_size(ty::META).text_color(t.text_muted).child(n)))
            .into_any_element()
    }
}

impl Render for Changes {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("Changes");
        let t = Theme::of(cx).clone();
        let quiet = |text: SharedString| {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .px(px(16.0))
                .text_size(ty::META)
                .text_color(t.text_faint)
                .child(text)
                .into_any_element()
        };
        let body: AnyElement = if self.not_repo && self.target.is_some() {
            self.render_not_repo(&t, cx)
        } else if let Some(err) = self.error.clone() {
            // git's stderr can be long: it wraps inside the strip, and the
            // pane clips, so it never paints over the conversation.
            div()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .p(px(12.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(page::error_strip(&t, err))
                .child(
                    div().flex().child(
                        ctl::button_sm(&t, "chg-retry", "Try again", Tone::Filled)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    ),
                )
                .into_any_element()
        } else if self.target.is_none() {
            kit::empty_state(icons::FILE_CODE, "No thread selected", "Open a thread to see what its agent changed.", &t)
                .flex_1()
                .into_any_element()
        } else if !self.loaded {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .child(kit::spinner("changes-preparing", 16.0, t.text_muted))
                .child(div().text_size(ty::META).text_color(t.text_faint).child("Preparing diff…"))
                .into_any_element()
        } else if self.files.is_empty() {
            quiet(if self.commit.is_some() { "Empty commit".into() } else { "No uncommitted changes".into() })
        } else {
            self.diff.clone().into_any_element()
        };
        div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(self.render_toolbar(&t, cx))
            .child(div().flex_1().min_h_0().overflow_hidden().flex().flex_col().child(body))
            .when(self.target.is_some() && self.commit.is_none() && !self.not_repo, |d| {
                d.child(self.render_footer(&t, cx))
            })
    }
}

impl Changes {
    /// The folder is not a git repository: say so, and offer to make it one
    /// or to open it.
    fn render_not_repo(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let folder = self
            .target
            .as_ref()
            .and_then(|(dir, _)| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "This folder".into());
        let actions = div()
            .flex()
            .flex_wrap()
            .justify_center()
            .gap(px(8.0))
            .child(
                ctl::button(
                    t,
                    "chg-git-init",
                    if self.initializing { "Initializing…" } else { "Initialize git" },
                    Tone::Solid,
                )
                .when(self.initializing, |b| b.opacity(0.6))
                .on_click(cx.listener(|this, _, _, cx| this.init_repo(cx))),
            )
            .child(
                ctl::button(t, "chg-open-folder", "Open folder", Tone::Outlined)
                    .child(icon(icons::ARROW_UP_RIGHT).size(px(12.0)))
                    .on_click(cx.listener(|this, _, _, cx| this.open_folder(cx))),
            );
        div()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(page::empty_state(
                t,
                icons::GIT_BRANCH,
                "Not a git repository",
                format!(
                    "{folder} isn't tracked by git yet. Initialize a repository to review, stage and commit changes here."
                ),
                Some(actions.into_any_element()),
            ))
            .when_some(self.init_error.clone(), |d, e| {
                d.child(div().w_full().max_w(px(420.0)).px(px(16.0)).child(page::error_strip(t, e)))
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff_list::{GUTTER_WIDTH, gutter_width, split_pairs};
    use dove_core::git::{Hunk, LineKind};

    fn line(kind: LineKind) -> git::DiffLine {
        git::DiffLine { kind, old_no: None, new_no: None, text: "x".into() }
    }

    #[test]
    fn split_pairs_runs() {
        use LineKind::*;
        let lines: Vec<_> = [Context, Removed, Removed, Added, Context, Added].into_iter().map(line).collect();
        assert_eq!(
            split_pairs(&lines),
            vec![(Some(0), Some(0)), (Some(1), Some(3)), (Some(2), None), (Some(4), Some(4)), (None, Some(5))]
        );
    }

    #[test]
    fn gutters_fit_the_largest_line_number() {
        let mut f = git::diff::untracked_file(std::path::Path::new("."), "missing-file-for-test");
        f.hunks = vec![Hunk {
            header: "@@".into(),
            lines: vec![git::DiffLine { kind: LineKind::Added, old_no: None, new_no: Some(123_456), text: "".into() }],
        }];
        assert!(gutter_width(&f) > GUTTER_WIDTH);
        f.hunks[0].lines[0].new_no = Some(7);
        assert_eq!(gutter_width(&f), GUTTER_WIDTH);
    }

    #[test]
    fn tabs_expand_before_highlighting() {
        let mut f = git::diff::untracked_file(std::path::Path::new("."), "missing-file-for-test");
        f.hunks = vec![Hunk {
            header: "@@".into(),
            lines: vec![git::DiffLine {
                kind: LineKind::Context,
                old_no: Some(1),
                new_no: Some(1),
                text: "\tx".into(),
            }],
        }];
        let files = expand_tabs(vec![f]);
        assert_eq!(&*files[0].hunks[0].lines[0].text, "    x");
    }
}
