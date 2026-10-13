//! What the agent changed, and the way out: review, stage, commit, push,
//! open a pull request, watch its checks, merge.
//!
//! Every file header, hunk header and line is flattened into one
//! `uniform_list`, so a 50k-line diff lays out only what is on screen.
//! Unified and split views share the flat list; paired removed/added lines
//! get intra-line highlights. Clicking a line number opens a comment box;
//! comments collect into one review the agent receives as a message.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use dove_core::git::{self, FileDiff, FileStatus, LineKind, github};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, HighlightStyle, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, StyledText, Subscription,
    UniformListScrollHandle, Window, div, px, transparent_black, uniform_list,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::kit;
use crate::theme::Theme;
use crate::ui;

pub enum ChangesEvent {
    /// Send this text to the thread's agent (a review).
    SendToAgent(String),
}

#[derive(Clone, Copy)]
enum Flat {
    File(usize),
    Hunk(usize, usize),
    /// Unified: one line.
    Line(usize, usize, usize),
    /// Split: (left line, right line) indices within the hunk.
    Pair(usize, usize, Option<usize>, Option<usize>),
    /// The comment box or a saved comment under a line.
    Comment(usize, usize, usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    Unified,
    Split,
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
    /// When set, the panel shows that commit rather than the working tree.
    commit: Option<String>,
    title: String,
    files: Arc<Vec<FileDiff>>,
    flat: Arc<Vec<Flat>>,
    collapsed: Vec<bool>,
    staged: HashSet<String>,
    layout: Layout,
    loading: bool,
    error: Option<SharedString>,
    scroll: UniformListScrollHandle,
    /// (file, hunk, line) → comment.
    comments: HashMap<(usize, usize, usize), String>,
    commenting: Option<((usize, usize, usize), Entity<InputState>)>,
    confirm_discard: Option<usize>,
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
        Self {
            target: None,
            commit: None,
            title: String::new(),
            files: Arc::default(),
            flat: Arc::default(),
            collapsed: Vec::new(),
            staged: HashSet::new(),
            layout: Layout::Unified,
            loading: false,
            error: None,
            scroll: UniformListScrollHandle::new(),
            comments: HashMap::new(),
            commenting: None,
            confirm_discard: None,
            message,
            delivery: Delivery { gh: github::gh_available(), ..Default::default() },
            _load: None,
            _subs: Vec::new(),
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
        if target == self.target {
            return;
        }
        self.target = target;
        self.commit = None;
        self.title = title.to_string();
        self.files = Arc::default();
        self.flat = Arc::default();
        self.comments.clear();
        self.commenting = None;
        let title = title.to_string();
        self.message.update(cx, |s, cx| s.set_value(title, window, cx));
        self.refresh(cx);
    }

    pub fn show_commit(&mut self, dir: PathBuf, sha: String, subject: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.target = Some((dir, sha.clone()));
        self.commit = Some(sha);
        self.title = subject.to_string();
        self.message.update(cx, |s, cx| s.set_value(subject.to_string(), window, cx));
        self.refresh(cx);
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
                None => git::changes(&dir, &base),
            };
            let staged = git::staged(&dir).unwrap_or_default();
            let branch = git::current_branch(&dir).ok();
            let has_remote = git::has_remote(&dir);
            let unpushed = git::unpushed(&dir);
            let pr = if gh && has_remote { github::pr_for_branch(&dir).ok().flatten() } else { None };
            (files, staged, branch, has_remote, unpushed, pr)
        });
        self._load = Some(cx.spawn(async move |this, cx| {
            let (files, staged, branch, has_remote, unpushed, pr) = work.await;
            let _ = this.update(cx, |v, cx| {
                v.loading = false;
                match files {
                    Ok(files) => {
                        v.error = None;
                        v.collapsed = vec![false; files.len()];
                        v.files = Arc::new(files);
                        v.rebuild();
                    }
                    Err(e) => v.error = Some(format!("{e:#}").into()),
                }
                v.staged = staged.into_iter().collect();
                v.delivery.branch = branch;
                v.delivery.has_remote = has_remote;
                v.delivery.unpushed = unpushed;
                v.delivery.pr = pr;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn rebuild(&mut self) {
        let mut flat = Vec::new();
        for (fi, f) in self.files.iter().enumerate() {
            flat.push(Flat::File(fi));
            if self.collapsed.get(fi).copied().unwrap_or(false) {
                continue;
            }
            for (hi, h) in f.hunks.iter().enumerate() {
                flat.push(Flat::Hunk(fi, hi));
                match self.layout {
                    Layout::Unified => {
                        for li in 0..h.lines.len() {
                            flat.push(Flat::Line(fi, hi, li));
                            if self.has_comment_row(fi, hi, li) {
                                flat.push(Flat::Comment(fi, hi, li));
                            }
                        }
                    }
                    Layout::Split => {
                        for (l, r) in split_pairs(&h.lines.iter().map(|l| l.kind).collect::<Vec<_>>()) {
                            flat.push(Flat::Pair(fi, hi, l, r));
                            let anchor = r.or(l).unwrap_or(0);
                            if self.has_comment_row(fi, hi, anchor) {
                                flat.push(Flat::Comment(fi, hi, anchor));
                            }
                        }
                    }
                }
            }
        }
        self.flat = Arc::new(flat);
    }

    fn has_comment_row(&self, fi: usize, hi: usize, li: usize) -> bool {
        self.comments.contains_key(&(fi, hi, li)) || self.commenting.as_ref().is_some_and(|(k, _)| *k == (fi, hi, li))
    }

    fn toggle_file(&mut self, fi: usize, cx: &mut Context<Self>) {
        if let Some(c) = self.collapsed.get_mut(fi) {
            *c = !*c;
            self.rebuild();
            cx.notify();
        }
    }

    fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        self.layout = layout;
        self.rebuild();
        cx.notify();
    }

    pub fn summary(&self) -> (usize, u32, u32) {
        (self.files.len(), self.files.iter().map(|f| f.additions).sum(), self.files.iter().map(|f| f.deletions).sum())
    }

    // ------------------------------------------------------------ git

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

    fn toggle_stage(&mut self, fi: usize, cx: &mut Context<Self>) {
        let Some(path) = self.files.get(fi).map(|f| f.path.clone()) else { return };
        let staged = self.staged.contains(&path);
        // Optimistic: flip now, the refresh confirms.
        if staged {
            self.staged.remove(&path);
        } else {
            self.staged.insert(path.clone());
        }
        self.run_git(if staged { "Unstage" } else { "Stage" }, cx, move |dir, _| {
            if staged {
                git::unstage(&dir, &path)?
            } else {
                git::stage(&dir, &path)?
            }
            Ok(None)
        });
    }

    fn discard(&mut self, fi: usize, cx: &mut Context<Self>) {
        self.confirm_discard = None;
        let Some(path) = self.files.get(fi).map(|f| f.path.clone()) else { return };
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
        format!("{}\n\n{n} files changed, +{adds} −{dels}\n\n{}\n\n_Opened from Dove._", self.title, files.join("\n"))
    }

    // ------------------------------------------------------------ comments

    fn start_comment(&mut self, key: (usize, usize, usize), window: &mut Window, cx: &mut Context<Self>) {
        let theme = Theme::of(cx).clone();
        let existing = self.comments.get(&key).cloned().unwrap_or_default();
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Comment for the agent… (Enter to save)");
            s.set_editor_style(ui::input_style(&theme));
            s.set_value(existing, window, cx);
            s
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        let sub = cx.subscribe_in(&input, window, move |this, input, ev: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                let text = input.read(cx).value().trim().to_string();
                if text.is_empty() {
                    this.comments.remove(&key);
                } else {
                    this.comments.insert(key, text);
                }
                this.commenting = None;
                this.rebuild();
                cx.notify();
            }
        });
        self._subs.push(sub);
        self.commenting = Some((key, input));
        self.rebuild();
        cx.notify();
    }

    fn send_review(&mut self, cx: &mut Context<Self>) {
        if self.comments.is_empty() {
            return;
        }
        let mut keys: Vec<_> = self.comments.keys().copied().collect();
        keys.sort();
        let mut out = String::from("Review comments on your changes:\n");
        for k in keys {
            let (fi, hi, li) = k;
            let Some(line) = self.files.get(fi).and_then(|f| f.hunks.get(hi)).and_then(|h| h.lines.get(li)) else {
                continue;
            };
            let path = &self.files[fi].path;
            let n = line.new_no.or(line.old_no).unwrap_or(0);
            out.push_str(&format!("\n- {path}:{n} `{}`\n  {}\n", line.text.trim(), self.comments[&k]));
        }
        out.push_str("\nPlease address each comment.");
        self.comments.clear();
        self.rebuild();
        cx.emit(ChangesEvent::SendToAgent(out));
        cx.notify();
    }

    // ------------------------------------------------------------ render

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
                            .text_size(px(12.0))
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
                                .text_size(px(12.5))
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
                steps.child(div().text_size(px(12.0)).text_color(t.text_faint).child("No remote: commits stay local"));
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
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.wash(0.03))
                    .text_size(px(13.0))
                    .child(div().flex_1().child(Input::new(&self.message))),
            )
            .child(steps)
            .when_some(d.note.clone(), |el, n| el.child(div().text_size(px(12.0)).text_color(t.text_muted).child(n)))
            .into_any_element()
    }
}

/// Pair removed and added runs side by side; context lines pair with
/// themselves.
fn split_pairs(kinds: &[LineKind]) -> Vec<(Option<usize>, Option<usize>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < kinds.len() {
        match kinds[i] {
            LineKind::Context => {
                out.push((Some(i), Some(i)));
                i += 1;
            }
            _ => {
                let start = i;
                while i < kinds.len() && kinds[i] == LineKind::Removed {
                    i += 1;
                }
                let removed: Vec<usize> = (start..i).collect();
                let add_start = i;
                while i < kinds.len() && kinds[i] == LineKind::Added {
                    i += 1;
                }
                let added: Vec<usize> = (add_start..i).collect();
                for k in 0..removed.len().max(added.len()) {
                    out.push((removed.get(k).copied(), added.get(k).copied()));
                }
            }
        }
    }
    out
}

/// Byte ranges that differ between a removed and an added line.
pub fn intraline(old: &str, new: &str) -> (Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>) {
    let diff = similar::TextDiff::from_chars(old, new);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    let (mut oa, mut ob) = (0usize, 0usize);
    for change in diff.iter_all_changes() {
        let len = change.value().len();
        match change.tag() {
            similar::ChangeTag::Delete => {
                push_range(&mut a, oa..oa + len);
                oa += len;
            }
            similar::ChangeTag::Insert => {
                push_range(&mut b, ob..ob + len);
                ob += len;
            }
            similar::ChangeTag::Equal => {
                oa += len;
                ob += len;
            }
        }
    }
    // Mostly-different lines read better without highlights.
    if a.iter().map(|r| r.len()).sum::<usize>() * 2 > old.len().max(1) * 3 / 2 {
        a.clear();
        b.clear();
    }
    (a, b)
}

fn push_range(v: &mut Vec<std::ops::Range<usize>>, r: std::ops::Range<usize>) {
    if let Some(last) = v.last_mut() {
        if last.end == r.start {
            last.end = r.end;
            return;
        }
    }
    v.push(r);
}

fn code(text: &str, highlights: Vec<std::ops::Range<usize>>, bg: gpui::Hsla) -> AnyElement {
    let text = text.replace('\t', "    ");
    if highlights.is_empty() {
        return div().child(text).into_any_element();
    }
    let style = HighlightStyle { background_color: Some(bg), ..Default::default() };
    let spans: Vec<_> = highlights.into_iter().filter(|r| r.end <= text.len()).map(|r| (r, style)).collect();
    StyledText::new(SharedString::from(text)).with_highlights(spans).into_any_element()
}

impl Render for Changes {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(cx).clone();
        let (n, adds, dels) = self.summary();
        let layout = self.layout;
        let header = div()
            .h(px(40.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .border_b_1()
            .border_color(t.border)
            .when(n > 0, |d| {
                d.child(div().text_size(px(12.0)).text_color(t.text_faint).child(format!("{n} files")))
                    .child(div().text_size(px(12.0)).text_color(t.diff_add).child(format!("+{adds}")))
                    .child(div().text_size(px(12.0)).text_color(t.diff_del).child(format!("−{dels}")))
            })
            .when_some(self.delivery.branch.clone(), |d, b| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .text_size(px(12.0))
                        .text_color(t.text_faint)
                        .child(icon(icons::GIT_BRANCH).size(px(12.0)))
                        .child(div().max_w(px(160.0)).truncate().child(b)),
                )
            })
            .child(div().flex_1())
            .when(!self.comments.is_empty(), |d| {
                d.child(
                    ui::button_primary("chg-review", format!("Send review ({})", self.comments.len()), &t)
                        .on_click(cx.listener(|this, _, _, cx| this.send_review(cx))),
                )
            })
            .child(kit::segmented(
                "chg-layout",
                &["Unified", "Split"],
                if layout == Layout::Unified { 0 } else { 1 },
                &t,
                {
                    let weak = cx.weak_entity();
                    move |i, _, cx| {
                        let _ = weak
                            .update(cx, |v, cx| v.set_layout(if i == 0 { Layout::Unified } else { Layout::Split }, cx));
                    }
                },
            ))
            .child(ui::icon_button("diff-refresh", icons::REFRESH, "Refresh", &t, {
                let weak = cx.weak_entity();
                move |_, _, cx| {
                    let _ = weak.update(cx, |v, cx| v.refresh(cx));
                }
            }));

        let body: AnyElement = if let Some(err) = &self.error {
            div().p(px(12.0)).text_size(px(13.0)).text_color(t.danger).child(err.clone()).into_any_element()
        } else if self.target.is_none() {
            kit::empty_state(icons::FILE_CODE, "No thread selected", "Open a thread to see what its agent changed.", &t)
                .flex_1()
                .into_any_element()
        } else if self.files.is_empty() {
            if self.loading {
                div()
                    .p(px(14.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .children((0..6).map(|i| {
                        kit::skeleton(("chg-skel", i as usize), px(220.0 + (i % 3) as f32 * 40.0), px(14.0), &t)
                    }))
                    .into_any_element()
            } else {
                kit::empty_state(
                    icons::CHECK,
                    "No changes yet",
                    "Edits the agent makes in this worktree show up here.",
                    &t,
                )
                .flex_1()
                .into_any_element()
            }
        } else {
            let files = self.files.clone();
            let flat = self.flat.clone();
            let staged = self.staged.clone();
            let comments = self.comments.clone();
            let commenting = self.commenting.clone();
            let confirm = self.confirm_discard;
            let weak = cx.weak_entity();
            uniform_list("diff-lines", flat.len(), move |range, _window, cx| {
                let t = Theme::of(cx);
                range
                    .map(|ix| match flat[ix] {
                        Flat::File(fi) => {
                            let f = &files[fi];
                            let (label, color) = match f.status {
                                FileStatus::Added => ("A", t.diff_add),
                                FileStatus::Deleted => ("D", t.diff_del),
                                FileStatus::Renamed => ("R", t.accent),
                                FileStatus::Modified => ("M", t.warning),
                            };
                            let is_staged = staged.contains(&f.path);
                            let (w1, w2, w3, w4) = (weak.clone(), weak.clone(), weak.clone(), weak.clone());
                            let path = f.path.clone();
                            div()
                                .id(("diff-file", ix))
                                .group("diff-file")
                                .h(px(30.0))
                                .px(px(8.0))
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .bg(t.wash(0.04))
                                .border_t_1()
                                .border_color(t.border)
                                .text_size(px(13.0))
                                .child(
                                    kit::checkbox(("stage", ix), is_staged, "", t)
                                        .tooltip(ui::tip(if is_staged { "Unstage" } else { "Stage" }))
                                        .on_click(move |_, _, cx| {
                                            cx.stop_propagation();
                                            let _ = w1.update(cx, |v, cx| v.toggle_stage(fi, cx));
                                        }),
                                )
                                .child(
                                    div()
                                        .w(px(12.0))
                                        .text_size(px(11.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(color)
                                        .child(label),
                                )
                                .child(
                                    div()
                                        .id(("diff-file-name", ix))
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .cursor_pointer()
                                        .child(f.path.clone())
                                        .on_click(move |_, _, cx| {
                                            let _ = w2.update(cx, |v, cx| v.toggle_file(fi, cx));
                                        }),
                                )
                                .when(f.binary, |d| {
                                    d.child(div().text_size(px(11.0)).text_color(t.text_faint).child("binary"))
                                })
                                .child(
                                    div().text_size(px(11.0)).text_color(t.diff_add).child(format!("+{}", f.additions)),
                                )
                                .child(
                                    div().text_size(px(11.0)).text_color(t.diff_del).child(format!("−{}", f.deletions)),
                                )
                                .child(
                                    div()
                                        .id(("diff-open", ix))
                                        .invisible()
                                        .group_hover("diff-file", |s| s.visible())
                                        .cursor_pointer()
                                        .tooltip(ui::tip("Open in preview"))
                                        .child(icon(icons::EYE).size(px(13.0)).text_color(t.text_faint))
                                        .on_click(move |_, window, cx| {
                                            window.dispatch_action(
                                                Box::new(crate::actions::OpenPath { path: path.clone(), line: None }),
                                                cx,
                                            );
                                        }),
                                )
                                .child(if confirm == Some(fi) {
                                    div()
                                        .id(("diff-discard-yes", ix))
                                        .px(px(6.0))
                                        .rounded(px(5.0))
                                        .bg(t.danger.opacity(0.14))
                                        .text_size(px(11.0))
                                        .text_color(t.danger)
                                        .cursor_pointer()
                                        .child("Discard?")
                                        .on_click(move |_, _, cx| {
                                            let _ = w3.update(cx, |v, cx| v.discard(fi, cx));
                                        })
                                        .into_any_element()
                                } else {
                                    div()
                                        .id(("diff-discard", ix))
                                        .invisible()
                                        .group_hover("diff-file", |s| s.visible())
                                        .cursor_pointer()
                                        .tooltip(ui::tip("Discard changes to this file"))
                                        .child(icon(icons::RESTART).size(px(13.0)).text_color(t.text_faint))
                                        .on_click(move |_, _, cx| {
                                            let _ = w4.update(cx, |v, cx| {
                                                v.confirm_discard = Some(fi);
                                                cx.notify();
                                            });
                                        })
                                        .into_any_element()
                                })
                                .into_any_element()
                        }
                        Flat::Hunk(fi, hi) => div()
                            .h(px(22.0))
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .text_size(px(11.0))
                            .text_color(t.text_faint)
                            .bg(t.diff_hunk_bg)
                            .font_family(t.font_mono.clone())
                            .truncate()
                            .child(files[fi].hunks[hi].header.to_string())
                            .into_any_element(),
                        Flat::Line(fi, hi, li) => {
                            let lines = &files[fi].hunks[hi].lines;
                            let l = &lines[li];
                            let (bg, sign, fg) = match l.kind {
                                LineKind::Added => (t.diff_add.opacity(0.12), "+", t.text),
                                LineKind::Removed => (t.diff_del.opacity(0.12), "−", t.text),
                                LineKind::Context => (transparent_black(), " ", t.text_muted),
                            };
                            // Pair a removed line with the added line at the same offset in its run.
                            let hl = paired(lines, li).map(|(old, new)| {
                                let (a, b) = intraline(&lines[old].text, &lines[new].text);
                                if li == old { (a, t.diff_del.opacity(0.35)) } else { (b, t.diff_add.opacity(0.35)) }
                            });
                            let num = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
                            let w = weak.clone();
                            let commented = comments.contains_key(&(fi, hi, li));
                            div()
                                .id(("diff-line", ix))
                                .group("diff-line")
                                .h(px(20.0))
                                .flex()
                                .bg(bg)
                                .font_family(t.font_mono.clone())
                                .text_size(px(12.0))
                                .child(
                                    div()
                                        .id(("diff-num", ix))
                                        .w(px(76.0))
                                        .flex()
                                        .flex_none()
                                        .cursor_pointer()
                                        .text_color(t.text_faint)
                                        .hover(|s| s.text_color(t.accent))
                                        .tooltip(ui::tip("Comment on this line"))
                                        .child(div().w(px(38.0)).pr(px(4.0)).text_right().child(num(l.old_no)))
                                        .child(div().w(px(38.0)).pr(px(4.0)).text_right().child(num(l.new_no)))
                                        .on_click(move |_, window, cx| {
                                            let _ = w.update(cx, |v, cx| v.start_comment((fi, hi, li), window, cx));
                                        }),
                                )
                                .child(
                                    div()
                                        .w(px(14.0))
                                        .text_color(if commented { t.accent } else { fg })
                                        .child(if commented { "●" } else { sign }),
                                )
                                .child(div().flex_1().whitespace_nowrap().overflow_hidden().text_color(fg).child(
                                    match hl {
                                        Some((ranges, color)) => code(&l.text, ranges, color),
                                        None => code(&l.text, Vec::new(), bg),
                                    },
                                ))
                                .into_any_element()
                        }
                        Flat::Pair(fi, hi, left, right) => {
                            let lines = &files[fi].hunks[hi].lines;
                            let (la, rb) = match (left, right) {
                                (Some(a), Some(b)) if a != b => intraline(&lines[a].text, &lines[b].text),
                                _ => (Vec::new(), Vec::new()),
                            };
                            let side = |idx: Option<usize>, ranges: Vec<std::ops::Range<usize>>, left: bool| {
                                let Some(i) = idx else {
                                    return div().flex_1().min_w_0().bg(t.wash(0.03)).into_any_element();
                                };
                                let l = &lines[i];
                                let (bg, strong) = match l.kind {
                                    LineKind::Added => (t.diff_add.opacity(0.12), t.diff_add.opacity(0.35)),
                                    LineKind::Removed => (t.diff_del.opacity(0.12), t.diff_del.opacity(0.35)),
                                    LineKind::Context => (transparent_black(), transparent_black()),
                                };
                                let n = if left { l.old_no } else { l.new_no };
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .bg(bg)
                                    .child(
                                        div()
                                            .w(px(38.0))
                                            .flex_none()
                                            .pr(px(4.0))
                                            .text_right()
                                            .text_color(t.text_faint)
                                            .child(n.map(|n| n.to_string()).unwrap_or_default()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_color(if l.kind == LineKind::Context { t.text_muted } else { t.text })
                                            .child(code(&l.text, ranges, strong)),
                                    )
                                    .into_any_element()
                            };
                            div()
                                .h(px(20.0))
                                .flex()
                                .font_family(t.font_mono.clone())
                                .text_size(px(12.0))
                                .child(side(left, la, true))
                                .child(div().w(px(1.0)).h_full().bg(t.border))
                                .child(side(right, rb, false))
                                .into_any_element()
                        }
                        Flat::Comment(fi, hi, li) => {
                            let editing =
                                commenting.as_ref().filter(|(k, _)| *k == (fi, hi, li)).map(|(_, i)| i.clone());
                            let text = comments.get(&(fi, hi, li)).cloned();
                            let w = weak.clone();
                            div()
                                .h(px(36.0))
                                .pl(px(90.0))
                                .pr(px(10.0))
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .bg(t.accent.opacity(0.06))
                                .text_size(px(12.5))
                                .child(icon(icons::CHAT_ROUND_LINE).size(px(13.0)).text_color(t.accent))
                                .child(match editing {
                                    Some(input) => div().flex_1().child(Input::new(&input)).into_any_element(),
                                    None => div()
                                        .id(("comment", ix))
                                        .flex_1()
                                        .truncate()
                                        .cursor_pointer()
                                        .text_color(t.text)
                                        .child(text.unwrap_or_default())
                                        .on_click(move |_, window, cx| {
                                            let _ = w.update(cx, |v, cx| v.start_comment((fi, hi, li), window, cx));
                                        })
                                        .into_any_element(),
                                })
                                .into_any_element()
                        }
                    })
                    .collect()
            })
            .track_scroll(&self.scroll)
            .flex_1()
            .into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(div().flex_1().min_h_0().flex().flex_col().child(body))
            .when(self.target.is_some(), |d| d.child(self.render_footer(&t, cx)))
    }
}

/// For a removed or added line inside a change run, the (removed, added)
/// pair at the same offset.
fn paired(lines: &[dove_core::git::DiffLine], li: usize) -> Option<(usize, usize)> {
    let kind = lines[li].kind;
    if kind == LineKind::Context {
        return None;
    }
    // Find the run of removed lines and the run of added lines around li.
    let mut start = li;
    while start > 0 && lines[start - 1].kind != LineKind::Context {
        start -= 1;
    }
    let mut end = li;
    while end + 1 < lines.len() && lines[end + 1].kind != LineKind::Context {
        end += 1;
    }
    let removed: Vec<usize> = (start..=end).filter(|i| lines[*i].kind == LineKind::Removed).collect();
    let added: Vec<usize> = (start..=end).filter(|i| lines[*i].kind == LineKind::Added).collect();
    let pos = if kind == LineKind::Removed {
        removed.iter().position(|i| *i == li)?
    } else {
        added.iter().position(|i| *i == li)?
    };
    Some((*removed.get(pos)?, *added.get(pos)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_pairs_runs() {
        use LineKind::*;
        let pairs = split_pairs(&[Context, Removed, Removed, Added, Context, Added]);
        assert_eq!(
            pairs,
            vec![(Some(0), Some(0)), (Some(1), Some(3)), (Some(2), None), (Some(4), Some(4)), (None, Some(5))]
        );
    }

    #[test]
    fn intraline_marks_only_the_change() {
        let (a, b) = intraline("let x = 1;", "let x = 2;");
        assert_eq!(a, vec![8..9]);
        assert_eq!(b, vec![8..9]);
        let (a, _) = intraline("abc", "xyz");
        assert!(a.is_empty(), "wholly different lines get no highlights");
    }
}
