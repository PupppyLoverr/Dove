//! What the agent changed, and the way out: review, stage, commit, push,
//! open a pull request, watch its checks, merge.
//!
//! The diff is Zeron's Changes pane (MIT): every file header, notice, hunk
//! header and line is its own row in one variable-height `list()`, so a
//! 50k-line diff lays out only what is on screen and a collapsed file's rows
//! leave the list. Rows carry Zeron's look: a coloured accent bar, line
//! number gutters fitted to the file, a +/−/· marker column, syntax colours
//! computed off the UI thread, a sticky file header, and code that scrolls
//! sideways as one plane (or wraps). Unified and split share the rows.
//! Hovering a line offers a comment; comments collect into one review the
//! agent receives as a message.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use dove_core::git::{self, FileDiff, FileStatus, Hunk, LineKind, github};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, ElementId, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ListAlignment, ListState, ParentElement as _, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledText, Subscription, TextRun, Window, div, font, list, px,
};
use gpui_base::input::{Input, InputEvent, InputState};
use zeron_syntax::{HighlightRequest, HighlightSpan};

use crate::assets::{icon, icons};
use crate::design::{radius, ty};
use crate::kit;
use crate::kit::controls::{self as ctl, Tone};
use crate::kit::page;
use crate::surface_chrome;
use crate::theme::{Theme, ink};
use crate::ui;

// ------------------------------------------------------------------ metrics

const FILE_HEADER_HEIGHT: f32 = surface_chrome::HEADER_HEIGHT;
const HUNK_HEADER_HEIGHT: f32 = 28.0;
const NOTICE_HEIGHT: f32 = 24.0;
/// Diff text size and row height: Settings › Appearance › Code & diff font size.
fn code_px() -> f32 {
    crate::design::code()
}

fn line_h() -> f32 {
    crate::design::code_line()
}
const BODY_BOTTOM_PAD: f32 = 8.0;
/// Narrowest line-number column.
const GUTTER_WIDTH: f32 = 36.0;
/// The +/−/· column between the gutters and the code.
const MARKER_WIDTH: f32 = 28.0;
/// Each split half pays for its own marker, so it is narrower.
const SPLIT_MARKER_WIDTH: f32 = 18.0;
const ACCENT_BAR_WIDTH: f32 = 3.0;
const UNIFIED_CODE_PAD: f32 = 12.0;
const SPLIT_CODE_PAD: f32 = 6.0;
/// Room after the widest line when scrolled fully right.
const CODE_PAD_RIGHT: f32 = 24.0;
const ADDER_SIZE: f32 = 16.0;
const TAB: &str = "    ";
/// Files longer than this (in diff lines) are not syntax highlighted.
const MAX_HIGHLIGHT_LINES: usize = 20_000;

pub enum ChangesEvent {
    /// Send this text to the thread's agent (a review).
    SendToAgent(String),
}

/// One row of the flat list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
    File(usize),
    /// "New file", "Renamed from …", "Binary file".
    Notice(usize, usize),
    Hunk(usize, usize),
    /// Unified: (file, hunk, line).
    Line(usize, usize, usize),
    /// Split: the old and new line indices within the hunk.
    Pair(usize, usize, Option<usize>, Option<usize>),
    /// The comment box or a saved comment under a line.
    Comment(usize, usize, usize),
    /// Breathing room under an open file.
    End(usize),
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

/// Syntax spans by `[file][hunk][line]`; a file left empty is drawn plain.
type Highlights = Arc<Vec<Vec<Vec<Vec<HighlightSpan>>>>>;

pub struct Changes {
    target: Option<(PathBuf, String)>,
    /// The working tree target, to come back to from a commit.
    tree: Option<(PathBuf, String)>,
    /// When set, the panel shows that commit rather than the working tree.
    commit: Option<String>,
    title: String,
    files: Arc<Vec<FileDiff>>,
    highlights: Highlights,
    rows: Vec<Row>,
    /// Content fingerprint per row: a rebuild splices only what changed, so
    /// the scroll position survives refreshes, folds and comments.
    keys: Vec<u64>,
    /// Each file's rows, header first (drives the sticky header).
    ranges: Vec<Range<usize>>,
    list: ListState,
    /// The code plane every row scrolls sideways together.
    hscroll: ScrollHandle,
    collapsed: HashSet<String>,
    staged: HashSet<String>,
    layout: Layout,
    wrap: bool,
    /// The code size the rows were measured at; a change remeasures them.
    code_size: f32,
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
    /// (file, hunk, line) → comment.
    comments: HashMap<(usize, usize, usize), String>,
    commenting: Option<((usize, usize, usize), Entity<InputState>)>,
    confirm_discard: Option<usize>,
    message: Entity<InputState>,
    delivery: Delivery,
    _load: Option<gpui::Task<()>>,
    _highlight: Option<gpui::Task<()>>,
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
        let list = ListState::new(0, ListAlignment::Top, px(600.0));
        let weak = cx.weak_entity();
        // The sticky header follows the scroll position.
        list.set_scroll_handler(move |_, _, cx| {
            let _ = weak.update(cx, |_, cx| cx.notify());
        });
        Self {
            target: None,
            tree: None,
            commit: None,
            title: String::new(),
            files: Arc::default(),
            highlights: Arc::default(),
            rows: Vec::new(),
            keys: Vec::new(),
            ranges: Vec::new(),
            list,
            hscroll: ScrollHandle::new(),
            collapsed: HashSet::new(),
            staged: HashSet::new(),
            layout: Layout::Unified,
            code_size: crate::design::code(),
            wrap: false,
            loading: false,
            loaded: false,
            error: None,
            not_repo: false,
            initializing: false,
            init_error: None,
            comments: HashMap::new(),
            commenting: None,
            confirm_discard: None,
            message,
            delivery: Delivery { gh: github::gh_available(), ..Default::default() },
            _load: None,
            _highlight: None,
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
        if target == self.target && self.commit.is_none() {
            return;
        }
        self.tree = target.clone();
        self.target = target;
        self.commit = None;
        self.title = title.to_string();
        self.clear();
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
        self.clear();
        self.message.update(cx, |s, cx| s.set_value(subject.to_string(), window, cx));
        self.refresh(cx);
    }

    /// Leave a commit for the working tree again.
    fn show_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.target = self.tree.clone();
        self.commit = None;
        self.clear();
        let title = self.title.clone();
        self.message.update(cx, |s, cx| s.set_value(title, window, cx));
        self.refresh(cx);
    }

    fn clear(&mut self) {
        self.files = Arc::default();
        self.highlights = Arc::default();
        self.comments.clear();
        self.commenting = None;
        self.confirm_discard = None;
        self.collapsed.clear();
        self.loaded = false;
        self.not_repo = false;
        self.init_error = None;
        self.rows.clear();
        self.keys.clear();
        self.ranges.clear();
        self.list.reset(0);
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
                            // Comments are keyed by position; a changed diff
                            // keeps only the ones still on the same text.
                            v.comments.retain(|&(fi, hi, li), _| {
                                let line = |fs: &[FileDiff]| {
                                    fs.get(fi).and_then(|f| f.hunks.get(hi)).and_then(|h| h.lines.get(li)).cloned()
                                };
                                line(&v.files).is_some() && line(&v.files) == line(&files)
                            });
                            v.files = Arc::new(files);
                            v.highlights = Arc::default();
                            v.highlight(cx);
                        }
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

    /// Syntax-colour every file off the UI thread; colours only change paint,
    /// never layout, so the rows need no re-measure.
    fn highlight(&mut self, cx: &mut Context<Self>) {
        let files = self.files.clone();
        let work = cx.background_spawn(async move { highlight_files(&files) });
        self._highlight = Some(cx.spawn(async move |this, cx| {
            let spans = work.await;
            let _ = this.update(cx, |v, cx| {
                if spans.len() == v.files.len() {
                    v.highlights = Arc::new(spans);
                    cx.notify();
                }
            });
        }));
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        let mut keys = Vec::new();
        let mut ranges = Vec::with_capacity(self.files.len());
        let editing = self.commenting.as_ref().map(|(k, _)| *k);
        let has_comment = |k: (usize, usize, usize)| self.comments.contains_key(&k) || editing == Some(k);
        for (fi, f) in self.files.iter().enumerate() {
            let start = rows.len();
            let collapsed = self.collapsed.contains(&f.path);
            let mut push = |row: Row, extra: &dyn Fn(&mut std::collections::hash_map::DefaultHasher)| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                std::mem::discriminant(&row).hash(&mut h);
                f.path.hash(&mut h);
                extra(&mut h);
                rows.push(row);
                keys.push(h.finish());
            };
            push(Row::File(fi), &|h| (f.additions, f.deletions, collapsed).hash(h));
            if !collapsed {
                for (ni, n) in notices(f).iter().enumerate() {
                    push(Row::Notice(fi, ni), &|h| n.hash(h));
                }
                for (hi, hunk) in f.hunks.iter().enumerate() {
                    push(Row::Hunk(fi, hi), &|h| hunk.header.hash(h));
                    let comment_key = |li: usize, h: &mut std::collections::hash_map::DefaultHasher| {
                        (li, self.comments.get(&(fi, hi, li)), editing == Some((fi, hi, li))).hash(h)
                    };
                    let line_key = |li: usize, h: &mut std::collections::hash_map::DefaultHasher| {
                        let l = &hunk.lines[li];
                        (&hunk.header, li, &*l.text, l.old_no, l.new_no, self.wrap, self.code_size.to_bits()).hash(h)
                    };
                    match self.layout {
                        Layout::Unified => {
                            for li in 0..hunk.lines.len() {
                                push(Row::Line(fi, hi, li), &|h| line_key(li, h));
                                if has_comment((fi, hi, li)) {
                                    push(Row::Comment(fi, hi, li), &|h| comment_key(li, h));
                                }
                            }
                        }
                        Layout::Split => {
                            for (l, r) in split_pairs(&hunk.lines) {
                                push(Row::Pair(fi, hi, l, r), &|h| {
                                    1u8.hash(h);
                                    l.inspect(|&l| line_key(l, h));
                                    r.inspect(|&r| line_key(r, h));
                                });
                                let anchor = r.or(l).unwrap_or(0);
                                if has_comment((fi, hi, anchor)) {
                                    push(Row::Comment(fi, hi, anchor), &|h| comment_key(anchor, h));
                                }
                            }
                        }
                    }
                }
                push(Row::End(fi), &|_| {});
            }
            ranges.push(start..rows.len());
        }
        // Splice only the span between the unchanged head and tail.
        let head = self.keys.iter().zip(&keys).take_while(|(a, b)| a == b).count();
        let max_tail = self.keys.len().min(keys.len()) - head;
        let tail = self.keys.iter().rev().zip(keys.iter().rev()).take(max_tail).take_while(|(a, b)| a == b).count();
        if head != self.keys.len() || head != keys.len() {
            self.list.splice(head..self.keys.len() - tail, keys.len() - tail - head);
        }
        self.rows = rows;
        self.keys = keys;
        self.ranges = ranges;
    }

    fn toggle_file(&mut self, fi: usize, cx: &mut Context<Self>) {
        let Some(path) = self.files.get(fi).map(|f| f.path.clone()) else { return };
        if !self.collapsed.remove(&path) {
            self.collapsed.insert(path);
        }
        // Folding the stuck file brings its header back to the top.
        let top = self.list.logical_scroll_top().item_ix;
        let stuck = self.ranges.get(fi).is_some_and(|r| r.contains(&top) && top != r.start);
        self.rebuild();
        if stuck && let Some(r) = self.ranges.get(fi) {
            self.list.scroll_to(gpui::ListOffset { item_ix: r.start, offset_in_item: px(0.0) });
        }
        cx.notify();
    }

    /// Fold every file, or open them all when all are folded.
    fn toggle_all(&mut self, cx: &mut Context<Self>) {
        let all = self.files.iter().all(|f| self.collapsed.contains(&f.path));
        if all {
            self.collapsed.clear();
        } else {
            self.collapsed = self.files.iter().map(|f| f.path.clone()).collect();
        }
        self.rebuild();
        cx.notify();
    }

    fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        self.layout = layout;
        self.rebuild();
        cx.notify();
    }

    fn toggle_wrap(&mut self, cx: &mut Context<Self>) {
        self.wrap = !self.wrap;
        self.hscroll.set_offset(gpui::point(px(0.0), px(0.0)));
        self.rebuild();
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
                this._subs.clear();
                this.rebuild();
                cx.notify();
            }
        });
        self._subs = vec![sub];
        self.commenting = Some((key, input));
        self.rebuild();
        cx.notify();
    }

    fn remove_comment(&mut self, key: (usize, usize, usize), cx: &mut Context<Self>) {
        self.comments.remove(&key);
        if self.commenting.as_ref().is_some_and(|(k, _)| *k == key) {
            self.commenting = None;
            self._subs.clear();
        }
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

    // ------------------------------------------------------------ rows

    fn render_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let Some(row) = self.rows.get(ix).copied() else { return div().into_any_element() };
        let width = self.code_width(window, &t);
        match row {
            Row::File(fi) => self.render_file_header(fi, false, &t, cx),
            Row::Notice(fi, ni) => {
                notice_row(notices(&self.files[fi]).get(ni).cloned().unwrap_or_default(), &t).into_any_element()
            }
            Row::Hunk(fi, hi) => hunk_header_row(&self.files[fi].hunks[hi].header, &t).into_any_element(),
            Row::Line(fi, hi, li) => self.render_line(ix, fi, hi, li, width, &t, cx),
            Row::Pair(fi, hi, l, r) => self.render_pair(ix, fi, hi, l, r, width, &t, cx),
            Row::Comment(fi, hi, li) => self.render_comment(fi, hi, li, &t, cx),
            Row::End(_) => div().h(px(BODY_BOTTOM_PAD)).into_any_element(),
        }
    }

    /// The code plane's width: the widest line in the diff, so every row
    /// scrolls sideways as one.
    fn code_width(&self, window: &Window, t: &Theme) -> Option<f32> {
        if self.wrap {
            return None;
        }
        let cols = self
            .files
            .iter()
            .flat_map(|f| f.hunks.iter().flat_map(|h| h.lines.iter()))
            .map(|l| l.text.chars().count())
            .max()
            .unwrap_or(0);
        let ts = window.text_system();
        let id = ts.resolve_font(&font(t.font_mono.clone()));
        let advance = ts.advance(id, px(code_px()), '0').map(|s| f32::from(s.width)).unwrap_or(7.2);
        Some(cols as f32 * advance + CODE_PAD_RIGHT)
    }

    fn render_file_header(&self, fi: usize, sticky: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let f = &self.files[fi];
        let collapsed = self.collapsed.contains(&f.path);
        let staged = self.staged.contains(&f.path);
        let confirm = self.confirm_discard == Some(fi) && self.commit.is_none();
        let in_tree = self.commit.is_none();
        let path = f.path.clone();
        let group = SharedString::from(if sticky { "diff-file-sticky".to_string() } else { format!("diff-file-{fi}") });
        let id = |s: &str| ElementId::from(SharedString::from(format!("{s}-{}-{fi}", if sticky { "s" } else { "r" })));
        let (rest, hover) = (ink(0.025), ink(0.05));
        let head = div()
            .id(id("diff-file"))
            .group(group.clone())
            .w_full()
            .h(px(FILE_HEADER_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(10.0))
            .when(!sticky && fi > 0, |d| d.border_t_1().border_color(ink(0.04)))
            .when(sticky, |d| d.border_b_1().border_color(t.border))
            .bg(rest)
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_file(fi, cx)))
            .child(
                div().flex_none().size(px(14.0)).flex().items_center().justify_center().child(
                    icon(if collapsed { icons::ALT_ARROW_RIGHT } else { icons::ALT_ARROW_DOWN })
                        .size(px(13.0))
                        .text_color(t.text_muted.opacity(0.7)),
                ),
            )
            .child(crate::files_panel::file_type_icon(&f.path, false, t).size(px(14.0)).flex_none())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(t.font_mono.clone())
                    .text_size(ty::META)
                    .text_color(t.text_muted)
                    .child(f.path.clone()),
            )
            .when(staged && in_tree, |d| {
                d.child(
                    div()
                        .flex_none()
                        .px(px(5.0))
                        .rounded(px(4.0))
                        .bg(t.success.opacity(0.12))
                        .text_size(ty::BADGE)
                        .text_color(t.success)
                        .child("STAGED"),
                )
            })
            .when(f.binary, |d| d.child(div().flex_none().text_size(ty::BADGE).text_color(t.text_faint).child("BIN")))
            .when(f.additions > 0 || !f.binary, |d| {
                d.child(
                    div()
                        .flex_none()
                        .font_family(t.font_mono.clone())
                        .text_size(ty::MICRO)
                        .text_color(t.diff_add)
                        .child(format!("+{}", f.additions)),
                )
            })
            .when(f.deletions > 0 || !f.binary, |d| {
                d.child(
                    div()
                        .flex_none()
                        .font_family(t.font_mono.clone())
                        .text_size(ty::MICRO)
                        .text_color(t.diff_del)
                        .child(format!("−{}", f.deletions)),
                )
            });
        let action = |key: &str, glyph: &'static str, label: &'static str| {
            surface_chrome::toolbar_button(id(key), label)
                .child(icon(glyph).size(px(surface_chrome::ICON_SIZE)).text_color(t.text_muted))
        };
        let hidden = |el: gpui::Stateful<gpui::Div>| el.invisible().group_hover(group.clone(), |s| s.visible());
        head.when(in_tree, |d| {
            d.child(hidden(action("diff-stage", icons::CHECK, if staged { "Unstage" } else { "Stage" })).on_click(
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_stage(fi, cx);
                }),
            ))
            .child(if confirm {
                div()
                    .id(id("diff-discard-yes"))
                    .flex_none()
                    .h(px(surface_chrome::CONTROL_SIZE))
                    .px(px(7.0))
                    .flex()
                    .items_center()
                    .rounded(px(surface_chrome::CONTROL_RADIUS))
                    .bg(t.danger.opacity(0.14))
                    .text_size(ty::MICRO)
                    .text_color(t.danger)
                    .cursor_pointer()
                    .child("Discard?")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.discard(fi, cx);
                    }))
                    .into_any_element()
            } else {
                hidden(action("diff-discard", icons::RESTART, "Discard changes to this file"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.confirm_discard = Some(fi);
                        cx.notify();
                    }))
                    .into_any_element()
            })
        })
        .child(action("diff-open-file", icons::DOCUMENT, "Open in file browser").on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(Box::new(crate::actions::OpenPath { path: path.clone(), line: None }), cx);
        }))
        .into_any_element()
    }

    /// The `+` that opens a comment on a line. Each row has its own group,
    /// so the adder shows on the hovered row only. A shared id painted it
    /// on every virtual row; storing the hover and notifying rebuilt the list.
    fn adder(
        &self,
        ix: usize,
        left: f32,
        key: (usize, usize, usize),
        group: &SharedString,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .absolute()
            .left(px(left))
            .top_0()
            .h_full()
            .flex()
            .items_center()
            .child(
                div()
                    .id(("diff-adder", ix))
                    .size(px(ADDER_SIZE))
                    .invisible()
                    .group_hover(group.clone(), |s| s.visible())
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .bg(t.accent)
                    .cursor_pointer()
                    .tooltip(ui::tip("Comment on this line"))
                    .child(icon(icons::PLUS).size(px(11.0)).text_color(t.on_solid))
                    .on_click(cx.listener(move |this, _, window, cx| this.start_comment(key, window, cx))),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_line(
        &self,
        ix: usize,
        fi: usize,
        hi: usize,
        li: usize,
        width: Option<f32>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let f = &self.files[fi];
        let line = &f.hunks[hi].lines[li];
        let gutter = gutter_width(f);
        let paint = LinePaint::of(line.kind, t);
        let spans = self.spans(fi, hi, li);
        let commented = self.comments.contains_key(&(fi, hi, li));
        let key = (fi, hi, li);
        let group = SharedString::from(format!("diff-line-{ix}"));
        let number = |n: Option<u32>, color| {
            div()
                .w(px(gutter))
                .flex_none()
                .font_family(t.font_mono.clone())
                .text_size(ty::MICRO)
                .line_height(px(line_h()))
                .text_color(color)
                .flex()
                .justify_end()
                .pr(px(8.0))
                .child(n.map(|n| n.to_string()).unwrap_or_default())
        };
        let faint = t.text_faint.opacity(0.8);
        // A deletion is cited on the old side; everything else on the new.
        let adder_left =
            ACCENT_BAR_WIDTH + if line.kind == LineKind::Removed { 0.0 } else { gutter } + (gutter - ADDER_SIZE) / 2.0;
        div()
            .id(("diff-line", ix))
            .group(group.clone())
            .relative()
            .w_full()
            .overflow_hidden()
            .flex_none()
            .flex()
            .items_start()
            .map(|d| if self.wrap { d.min_h(px(line_h())) } else { d.h(px(line_h())) })
            .when_some(paint.bg, |d, bg| d.bg(bg))
            .child(div().w(px(ACCENT_BAR_WIDTH)).self_stretch().flex_none().when_some(paint.accent, |d, c| d.bg(c)))
            .child(number(line.old_no, if line.kind == LineKind::Removed { paint.number } else { faint }))
            .child(number(line.new_no, if line.kind == LineKind::Added { paint.number } else { faint }))
            .child(
                div()
                    .w(px(MARKER_WIDTH))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .font_family(t.font_mono.clone())
                    .text_size(px(code_px()))
                    .line_height(px(line_h()))
                    .text_color(if commented { t.accent } else { paint.marker_color })
                    .child(if commented { "●" } else { paint.marker }),
            )
            .child(self.code_plane(("diff-code", ix).into(), &line.text, spans, UNIFIED_CODE_PAD, width, t))
            .when(self.commit.is_none(), |d| d.child(self.adder(ix, adder_left, key, &group, t, cx)))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_pair(
        &self,
        ix: usize,
        fi: usize,
        hi: usize,
        left: Option<usize>,
        right: Option<usize>,
        width: Option<f32>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let f = &self.files[fi];
        let lines = &f.hunks[hi].lines;
        let gutter = gutter_width(f);
        let cell = |side: Option<usize>, old: bool| -> AnyElement {
            let Some(li) = side else {
                return div().flex_1().min_w_0().self_stretch().bg(ink(0.03)).into_any_element();
            };
            let line = &lines[li];
            let paint = LinePaint::of(line.kind, t);
            div()
                .flex_1()
                .min_w_0()
                .self_stretch()
                .overflow_hidden()
                .flex()
                .items_start()
                .when_some(paint.bg, |d, bg| d.bg(bg))
                .child(div().w(px(ACCENT_BAR_WIDTH)).self_stretch().flex_none().when_some(paint.accent, |d, c| d.bg(c)))
                .child(
                    div()
                        .w(px(gutter))
                        .flex_none()
                        .font_family(t.font_mono.clone())
                        .text_size(ty::MICRO)
                        .line_height(px(line_h()))
                        .text_color(paint.number)
                        .flex()
                        .justify_end()
                        .pr(px(8.0))
                        .child(if old { line.old_no } else { line.new_no }.map(|n| n.to_string()).unwrap_or_default()),
                )
                .child(
                    div()
                        .w(px(SPLIT_MARKER_WIDTH))
                        .flex_none()
                        .flex()
                        .justify_center()
                        .font_family(t.font_mono.clone())
                        .text_size(px(code_px()))
                        .line_height(px(line_h()))
                        .text_color(paint.marker_color)
                        .child(paint.marker),
                )
                .child(self.code_plane(
                    (SharedString::from(format!("diff-split-{}", if old { "l" } else { "r" })), ix).into(),
                    &line.text,
                    self.spans(fi, hi, li),
                    SPLIT_CODE_PAD,
                    width,
                    t,
                ))
                .into_any_element()
        };
        let anchor = right.or(left).unwrap_or(0);
        let key = (fi, hi, anchor);
        let group = SharedString::from(format!("diff-pair-{ix}"));
        let commentable = self.commit.is_none() && (right.is_some() || left.is_some());
        div()
            .id(("diff-pair", ix))
            .group(group.clone())
            .relative()
            .w_full()
            .overflow_hidden()
            .flex_none()
            .flex()
            .items_stretch()
            .map(|d| if self.wrap { d.min_h(px(line_h())) } else { d.h(px(line_h())) })
            .child(cell(left, true))
            .child(div().w(px(1.0)).self_stretch().flex_none().bg(ink(0.06)))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .self_stretch()
                    .flex()
                    .child(cell(right, false))
                    .when(commentable, |d| {
                        d.child(self.adder(ix, ACCENT_BAR_WIDTH + (gutter - ADDER_SIZE) / 2.0, key, &group, t, cx))
                    }),
            )
            .into_any_element()
    }

    fn spans(&self, fi: usize, hi: usize, li: usize) -> &[HighlightSpan] {
        self.highlights.get(fi).and_then(|f| f.get(hi)).and_then(|h| h.get(li)).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The only part of a row that moves sideways: gutters stay put while
    /// the code scrolls with every other row (or wraps).
    fn code_plane(
        &self,
        id: ElementId,
        text: &str,
        spans: &[HighlightSpan],
        pad: f32,
        width: Option<f32>,
        t: &Theme,
    ) -> AnyElement {
        let text: SharedString = if text.is_empty() { " ".into() } else { text.to_string().into() };
        let runs = runs(&text, spans, t);
        // The inner box is the scroll extent. It is clipped to that width so
        // a long line cannot stretch the row (and the virtual list) to the
        // text's full width — that left the gutters in place and the code
        // with nothing to scroll.
        let content = div()
            .pl(px(pad))
            .font_family(t.font_mono.clone())
            .text_size(px(code_px()))
            .line_height(px(line_h()))
            .map(|d| match width {
                Some(w) => d.w(px(w + pad)).flex_none().overflow_hidden().whitespace_nowrap(),
                None => d.w_full().min_w_0().whitespace_normal(),
            })
            .child(StyledText::new(text).with_runs(runs));
        let viewport = div().flex_1().min_w_0().min_h(px(line_h())).overflow_hidden().child(content);
        if width.is_none() {
            return viewport.into_any_element();
        }
        let mut viewport = viewport.id(id).overflow_x_scroll().track_scroll(&self.hscroll);
        viewport.style().restrict_scroll_to_axis = Some(true);
        viewport.into_any_element()
    }

    fn render_comment(&self, fi: usize, hi: usize, li: usize, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let key = (fi, hi, li);
        let editing = self.commenting.as_ref().filter(|(k, _)| *k == key).map(|(_, i)| i.clone());
        let text = self.comments.get(&key).cloned();
        let indent = ACCENT_BAR_WIDTH
            + match self.layout {
                Layout::Unified => 2.0 * gutter_width(&self.files[fi]) + MARKER_WIDTH,
                Layout::Split => 0.0,
            };
        let card = div()
            .w_full()
            .max_w(px(560.0))
            .px(px(10.0))
            .py(px(8.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .rounded(px(radius::RADIUS))
            .border_1()
            .border_color(t.border)
            .bg(t.surface_raised)
            .text_size(ty::SMALL)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(ty::CAPTION)
                    .text_color(t.text_faint)
                    .child(icon(icons::CHAT_ROUND_LINE).size(px(12.0)).text_color(t.accent))
                    .child(div().flex_1().child(if editing.is_some() { "New comment" } else { "Comment" }))
                    .child(
                        div()
                            .id(("comment-del", li + hi * 100_000 + fi * 10_000_000))
                            .cursor_pointer()
                            .hover(|s| s.text_color(t.text))
                            .child(if editing.is_some() && text.is_none() { "Cancel" } else { "Delete" })
                            .on_click(cx.listener(move |this, _, _, cx| this.remove_comment(key, cx))),
                    ),
            )
            .child(match editing {
                Some(input) => div().child(Input::new(&input)).into_any_element(),
                None => div()
                    .id(("comment", li + hi * 100_000 + fi * 10_000_000))
                    .cursor_pointer()
                    .text_color(t.text)
                    .tooltip(ui::tip("Edit"))
                    .child(text.unwrap_or_default())
                    .on_click(cx.listener(move |this, _, window, cx| this.start_comment(key, window, cx)))
                    .into_any_element(),
            });
        div().w_full().pl(px(indent + 8.0)).pr(px(12.0)).py(px(6.0)).child(card).into_any_element()
    }

    /// The header of the file whose rows fill the top of the viewport, pinned
    /// there and pushed up by the next file's header.
    fn render_sticky(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let top = self.list.logical_scroll_top();
        let fi = self.ranges.partition_point(|r| r.start <= top.item_ix).checked_sub(1)?;
        let range = self.ranges.get(fi)?;
        if !range.contains(&top.item_ix) || (top.item_ix == range.start && f32::from(top.offset_in_item) <= 0.0) {
            return None;
        }
        let next_y = self.ranges.get(fi + 1).and_then(|r| {
            let b = self.list.bounds_for_item(r.start)?;
            Some(f32::from(b.origin.y - self.list.viewport_bounds().origin.y))
        });
        let offset = next_y.map(|y| (y - FILE_HEADER_HEIGHT).min(0.0)).unwrap_or(0.0);
        Some(
            div()
                .absolute()
                .top(px(offset))
                .left_0()
                .w_full()
                .bg(t.bg)
                .occlude()
                .child(self.render_file_header(fi, true, t, cx))
                .into_any_element(),
        )
    }

    // ------------------------------------------------------------ chrome

    fn render_toolbar(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (n, adds, dels) = self.summary();
        let toggle = |id: &'static str, glyph: &'static str, label: &'static str, on: bool| {
            surface_chrome::toolbar_button(id, label).when(on, |d| d.bg(ink(0.10))).child(
                icon(glyph).size(px(surface_chrome::ICON_SIZE)).text_color(if on { t.text } else { t.text_muted }),
            )
        };
        let label = match (&self.commit, &self.target) {
            (Some(_), _) => format!("{n} Changed {} in this commit", if n == 1 { "file" } else { "files" }),
            (None, Some((_, base))) if base != "HEAD" => {
                format!("{n} Changed {} vs {base}", if n == 1 { "file" } else { "files" })
            }
            _ if n == 1 => "1 Uncommitted change".into(),
            _ => format!("{n} Uncommitted changes"),
        };
        let all_folded = !self.files.is_empty() && self.files.iter().all(|f| self.collapsed.contains(&f.path));
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
            .when(!self.comments.is_empty(), |d| {
                d.child(
                    ui::button_primary("chg-review", format!("Send review ({})", self.comments.len()), t)
                        .on_click(cx.listener(|this, _, _, cx| this.send_review(cx))),
                )
            })
            .when(self.commit.is_some() && self.tree.is_some(), |d| {
                d.child(
                    ui::button_ghost("chg-tree", "Working tree", t)
                        .on_click(cx.listener(|this, _, window, cx| this.show_tree(window, cx))),
                )
            })
            .child(
                toggle("diff-wrap", icons::WRAP_TEXT, "Wrap long lines", self.wrap)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_wrap(cx))),
            )
            .child(toggle("diff-split", icons::SPLIT_COLUMNS, "Side by side", self.layout == Layout::Split).on_click(
                cx.listener(|this, _, _, cx| {
                    let next = if this.layout == Layout::Split { Layout::Unified } else { Layout::Split };
                    this.set_layout(next, cx);
                }),
            ))
            .child(
                toggle(
                    "diff-fold",
                    icons::FOLD_VERTICAL,
                    if all_folded { "Expand all files" } else { "Collapse all files" },
                    all_folded,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_all(cx))),
            )
            .child(
                toggle("diff-refresh", icons::REFRESH, "Refresh", false)
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
        // The code font size changed in Settings: remeasure every row.
        let code_size = crate::design::code();
        if (code_size - self.code_size).abs() > f32::EPSILON {
            self.code_size = code_size;
            self.rebuild();
        }
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
            let sticky = self.render_sticky(&t, cx);
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(
                    list(self.list.clone(), cx.processor(Self::render_row))
                        .size_full()
                        .with_sizing_behavior(gpui::ListSizingBehavior::Auto),
                )
                .children(sticky)
                .into_any_element()
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

// ------------------------------------------------------------------ pieces

/// How a +, − or context line paints: ~5% washes over the pane, a solid
/// accent bar and tinted numbers on changed lines.
struct LinePaint {
    marker: &'static str,
    marker_color: gpui::Hsla,
    bg: Option<gpui::Hsla>,
    accent: Option<gpui::Hsla>,
    number: gpui::Hsla,
}

impl LinePaint {
    fn of(kind: LineKind, t: &Theme) -> Self {
        let tint = |c: gpui::Hsla| gpui::Hsla { a: 0.055, ..c };
        match kind {
            LineKind::Added => Self {
                marker: "+",
                marker_color: t.diff_add,
                bg: Some(tint(t.diff_add)),
                accent: Some(t.diff_add.opacity(0.55)),
                number: t.diff_add.opacity(0.9),
            },
            LineKind::Removed => Self {
                marker: "−",
                marker_color: t.diff_del,
                bg: Some(tint(t.diff_del)),
                accent: Some(t.diff_del.opacity(0.55)),
                number: t.diff_del.opacity(0.9),
            },
            LineKind::Context => Self {
                marker: "·",
                marker_color: t.text_faint.opacity(0.5),
                bg: None,
                accent: None,
                number: t.text_faint.opacity(0.8),
            },
        }
    }
}

/// A file diff the way a transcript tool shows it, as in Zeron: the pane's
/// own rows (hunk headers on the bluish wash, accent bars, fitted dual
/// gutters, the marker column, syntax colours) stacked as plain elements,
/// lines clipped rather than scrolled, cut after `take` lines.
pub(crate) fn inline_rows(f: &FileDiff, spans: &[Vec<Vec<HighlightSpan>>], take: usize, t: &Theme) -> Vec<AnyElement> {
    let gutter = gutter_width(f);
    let faint = t.text_faint.opacity(0.8);
    let number = |n: Option<u32>, color: gpui::Hsla| {
        div()
            .w(px(gutter))
            .flex_none()
            .flex()
            .justify_end()
            .pr(px(8.0))
            .text_size(ty::MICRO)
            .text_color(color)
            .child(n.map(|n| n.to_string()).unwrap_or_default())
    };
    let mut rows = Vec::new();
    let mut left = take;
    for (hi, h) in f.hunks.iter().enumerate() {
        if left == 0 {
            break;
        }
        rows.push(hunk_header_row(&h.header, t).into_any_element());
        for (li, line) in h.lines.iter().enumerate().take(left) {
            let paint = LinePaint::of(line.kind, t);
            let text: SharedString = if line.text.is_empty() { " ".into() } else { line.text.to_string().into() };
            let line_spans = spans.get(hi).and_then(|h| h.get(li)).map(Vec::as_slice).unwrap_or(&[]);
            let runs = runs(&text, line_spans, t);
            rows.push(
                div()
                    .w_full()
                    .h(px(line_h()))
                    .flex_none()
                    .flex()
                    .items_start()
                    .overflow_hidden()
                    .font_family(t.font_mono.clone())
                    .line_height(px(line_h()))
                    .when_some(paint.bg, |d, bg| d.bg(bg))
                    .child(div().w(px(ACCENT_BAR_WIDTH)).h_full().flex_none().when_some(paint.accent, |d, c| d.bg(c)))
                    .child(number(line.old_no, if line.kind == LineKind::Removed { paint.number } else { faint }))
                    .child(number(line.new_no, if line.kind == LineKind::Added { paint.number } else { faint }))
                    .child(
                        div()
                            .w(px(MARKER_WIDTH))
                            .flex_none()
                            .flex()
                            .justify_center()
                            .text_size(px(code_px()))
                            .text_color(paint.marker_color)
                            .child(paint.marker),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .pl(px(UNIFIED_CODE_PAD))
                            .text_size(px(code_px()))
                            .child(StyledText::new(text).with_runs(runs)),
                    )
                    .into_any_element(),
            );
        }
        left = left.saturating_sub(h.lines.len());
    }
    rows
}

/// Syntax spans for one file, per hunk per line (empty when the language is
/// unknown or the file is too long).
pub(crate) fn highlight_file(f: &FileDiff) -> Vec<Vec<Vec<HighlightSpan>>> {
    highlight_files(std::slice::from_ref(f)).pop().unwrap_or_default()
}

fn notice_row(notice: String, t: &Theme) -> impl IntoElement {
    div()
        .h(px(NOTICE_HEIGHT))
        .w_full()
        .flex_none()
        .flex()
        .items_center()
        .px(px(16.0))
        .text_size(ty::MICRO)
        .text_color(t.text_faint)
        .child(notice)
}

/// One `@@ … @@` row on the bluish-grey wash.
fn hunk_header_row(header: &str, t: &Theme) -> impl IntoElement {
    div()
        .h(px(HUNK_HEADER_HEIGHT))
        .w_full()
        .flex_none()
        .flex()
        .items_center()
        .px(px(16.0))
        .bg(t.diff_hunk_bg)
        .font_family(t.font_mono.clone())
        .text_size(ty::MICRO)
        .text_color(t.text_faint)
        .truncate()
        .child(header.to_string())
}

fn notices(f: &FileDiff) -> Vec<String> {
    let mut out = Vec::new();
    match f.status {
        FileStatus::Added => out.push("New file".to_string()),
        FileStatus::Deleted => out.push("Deleted file".to_string()),
        FileStatus::Renamed => out.push(format!("Renamed from {}", f.old_path.as_deref().unwrap_or("?"))),
        FileStatus::Modified => {}
    }
    if f.binary {
        out.push("Binary file — contents not shown".to_string());
    }
    out
}

/// One line-number column, fitted to the file's largest line number (11px
/// mono is about 6.6px a digit, plus padding), never under 36px.
fn gutter_width(f: &FileDiff) -> f32 {
    let max = f.hunks.iter().flat_map(|h| &h.lines).map(|l| l.old_no.unwrap_or(0).max(l.new_no.unwrap_or(0))).max();
    let digits = max.unwrap_or(1).max(1).ilog10() + 1;
    (digits as f32 * 6.6 + 14.0).max(GUTTER_WIDTH)
}

fn runs(text: &str, spans: &[HighlightSpan], t: &Theme) -> Vec<TextRun> {
    let mono = font(t.font_mono.clone());
    let plain = t.text.opacity(0.92);
    let run = |len: usize, color| TextRun {
        len,
        font: mono.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let mut out = Vec::with_capacity(spans.len() * 2 + 1);
    let mut at = 0usize;
    for s in spans {
        let (start, end) = (s.range.start.min(text.len()), s.range.end.min(text.len()));
        if start < at || start >= end {
            continue;
        }
        if start > at {
            out.push(run(start - at, plain));
        }
        out.push(run(end - start, t.syntax(s.kind)));
        at = end;
    }
    if at < text.len() {
        out.push(run(text.len() - at, plain));
    }
    out
}

/// Tabs become spaces once, before highlighting, so spans and widths agree.
fn expand_tabs(mut files: Vec<FileDiff>) -> Vec<FileDiff> {
    for line in files.iter_mut().flat_map(|f| f.hunks.iter_mut().flat_map(|h| h.lines.iter_mut())) {
        if line.text.contains('\t') {
            line.text = line.text.replace('\t', TAB).into();
        }
    }
    files
}

fn highlight_files(files: &[FileDiff]) -> Vec<Vec<Vec<Vec<HighlightSpan>>>> {
    files
        .iter()
        .map(|f| {
            let lines: usize = f.hunks.iter().map(|h| h.lines.len()).sum();
            if f.binary || lines > MAX_HIGHLIGHT_LINES || zeron_syntax::language_for_path(&f.path).is_none() {
                return Vec::new();
            }
            f.hunks.iter().map(|h| highlight_hunk(&f.path, h)).collect()
        })
        .collect()
}

/// Highlight a hunk's old and new sides as two excerpts, so a string or
/// comment opened on one line still colours the next.
fn highlight_hunk(path: &str, h: &Hunk) -> Vec<Vec<HighlightSpan>> {
    let mut out = vec![Vec::new(); h.lines.len()];
    // Old side first, so context lines end up with the new side's colours.
    for skip in [LineKind::Added, LineKind::Removed] {
        let idx: Vec<usize> = (0..h.lines.len()).filter(|&i| h.lines[i].kind != skip).collect();
        if idx.is_empty() {
            continue;
        }
        let source = idx.iter().map(|&i| &*h.lines[i].text).collect::<Vec<_>>().join("\n");
        let Ok(doc) = zeron_syntax::highlight(HighlightRequest { source: &source, path: Some(path), fence_tag: None })
        else {
            continue;
        };
        for (i, spans) in idx.into_iter().zip(doc.lines) {
            out[i] = spans;
        }
    }
    out
}

/// Pair a hunk's lines into split rows: context sits on both sides, and each
/// run of removals followed by additions lines up index for index, the
/// longer side's leftovers getting one-sided rows.
fn split_pairs(lines: &[git::DiffLine]) -> Vec<(Option<usize>, Option<usize>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind == LineKind::Context {
            out.push((Some(i), Some(i)));
            i += 1;
            continue;
        }
        let start = i;
        while i < lines.len() && lines[i].kind == LineKind::Removed {
            i += 1;
        }
        let removed: Vec<usize> = (start..i).collect();
        let add_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Added {
            i += 1;
        }
        let added: Vec<usize> = (add_start..i).collect();
        for k in 0..removed.len().max(added.len()) {
            out.push((removed.get(k).copied(), added.get(k).copied()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
