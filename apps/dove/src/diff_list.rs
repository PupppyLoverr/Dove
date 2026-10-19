//! A diff as one virtualized, collapsible list: Zeron's Changes pane (MIT),
//! lifted out of `changes.rs` so the working-tree pane and the pull request
//! view draw the same rows.
//!
//! Every file header, notice, hunk header and line is its own row in one
//! variable-height `list()`, so a 50k-line diff lays out only what is on
//! screen and a folded file's rows leave the list. Rows carry Zeron's look: a
//! coloured accent bar, line number gutters fitted to the file, a +/−/·
//! marker column, syntax colours computed off the UI thread, a sticky file
//! header, and code that scrolls sideways as one plane (or wraps). Unified
//! and split share the rows.
//!
//! Files, hunks and long runs of unchanged context fold. In
//! [`DiffListMode::Worktree`] each file offers stage, discard and open, and
//! hovering a line offers a comment; [`DiffListMode::ReadOnly`] shows the
//! diff alone.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use dove_core::git::{DiffLine, FileDiff, FileStatus, Hunk, LineKind};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, EventEmitter, InteractiveElement as _, IntoElement,
    ListAlignment, ListState, ParentElement as _, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledText, Subscription, TextRun, Window, div, font, list, px,
};
use gpui_base::input::{Input, InputEvent, InputState};
use zeron_syntax::{HighlightRequest, HighlightSpan};

use crate::assets::{icon, icons};
use crate::design::{radius, ty};
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
pub(crate) const GUTTER_WIDTH: f32 = 36.0;
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
/// A run of unchanged lines longer than this folds to one row.
const FOLD_CONTEXT_OVER: usize = 8;
/// Unchanged lines kept in view next to a change when its run folds.
const FOLD_CONTEXT_KEEP: usize = 3;
/// A diff with more files than this opens with every file folded.
const COLLAPSE_OVER_FILES: usize = 8;
/// A diff with more changed lines than this opens with every file folded.
const COLLAPSE_OVER_LINES: u32 = 400;

// ------------------------------------------------------------------ model

/// What the list offers besides the diff itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiffListMode {
    /// The working tree: stage, discard, open and line comments.
    Worktree,
    /// Someone else's diff (a pull request): read only.
    ReadOnly,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    Unified,
    Split,
}

pub enum DiffListEvent {
    /// A view setting changed (layout, wrap, folds, comments): toolbars that
    /// mirror it should re-render.
    Changed,
    /// Worktree: the file header's stage button.
    ToggleStage(String),
    /// Worktree: the file header's confirmed discard.
    Discard(String),
    /// The file whose rows fill the top of the viewport changed, by scrolling
    /// or by [`DiffList::scroll_to_file`]. The working-tree pane ignores it;
    /// the pull request view's file list follows it.
    #[allow(dead_code)]
    TopFile(Option<String>),
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
    /// Folded unchanged lines `start..end` of a hunk.
    Fold(usize, usize, usize, usize),
    /// The comment box or a saved comment under a line.
    Comment(usize, usize, usize),
    /// Breathing room under an open file.
    End(usize),
}

/// (file, hunk, line) → comment.
type CommentKey = (usize, usize, usize);

/// Syntax spans by `[file][hunk][line]`; a file left empty is drawn plain.
type Highlights = Arc<Vec<Vec<Vec<Vec<HighlightSpan>>>>>;

/// What is folded. Keys are paths and hunk headers rather than indices, so
/// folds survive a refresh that adds or drops files.
#[derive(Default, Clone)]
struct Folds {
    files: HashSet<String>,
    hunks: HashSet<(String, Arc<str>)>,
    /// Long context runs the reader opened, by their first line.
    open_context: HashSet<(String, Arc<str>, usize)>,
}

/// Everything a row build reads.
struct RowInputs<'a> {
    files: &'a [FileDiff],
    folds: &'a Folds,
    layout: Layout,
    wrap: bool,
    code_size: f32,
    comments: &'a HashMap<CommentKey, String>,
    editing: Option<CommentKey>,
}

struct BuiltRows {
    rows: Vec<Row>,
    /// Content fingerprint per row.
    keys: Vec<u64>,
    /// Each file's rows, header first.
    ranges: Vec<Range<usize>>,
}

type KeyHasher = std::collections::hash_map::DefaultHasher;

fn build_rows(i: &RowInputs) -> BuiltRows {
    let mut rows = Vec::new();
    let mut keys = Vec::new();
    let mut ranges = Vec::with_capacity(i.files.len());
    let has_comment = |k: CommentKey| i.comments.contains_key(&k) || i.editing == Some(k);
    for (fi, f) in i.files.iter().enumerate() {
        let start = rows.len();
        let collapsed = i.folds.files.contains(&f.path);
        let mut push = |row: Row, extra: &dyn Fn(&mut KeyHasher)| {
            let mut h = KeyHasher::new();
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
                let hunk_folded = i.folds.hunks.contains(&(f.path.clone(), hunk.header.clone()));
                push(Row::Hunk(fi, hi), &|h| (&hunk.header, hunk_folded).hash(h));
                if hunk_folded {
                    continue;
                }
                let folds = context_folds(&hunk.lines)
                    .into_iter()
                    .filter(|r| !i.folds.open_context.contains(&(f.path.clone(), hunk.header.clone(), r.start)))
                    .collect::<Vec<_>>();
                let folded_at = |li: usize| folds.iter().find(|r| r.contains(&li));
                let comment_key = |li: usize, h: &mut KeyHasher| {
                    (li, i.comments.get(&(fi, hi, li)), i.editing == Some((fi, hi, li))).hash(h)
                };
                let line_key = |li: usize, h: &mut KeyHasher| {
                    let l = &hunk.lines[li];
                    (&hunk.header, li, &*l.text, l.old_no, l.new_no, i.wrap, i.code_size.to_bits()).hash(h)
                };
                let fold_key = |r: &Range<usize>, h: &mut KeyHasher| (&hunk.header, r.start, r.end).hash(h);
                match i.layout {
                    Layout::Unified => {
                        let mut li = 0;
                        while li < hunk.lines.len() {
                            if let Some(r) = folded_at(li) {
                                push(Row::Fold(fi, hi, r.start, r.end), &|h| fold_key(r, h));
                                li = r.end;
                                continue;
                            }
                            push(Row::Line(fi, hi, li), &|h| line_key(li, h));
                            if has_comment((fi, hi, li)) {
                                push(Row::Comment(fi, hi, li), &|h| comment_key(li, h));
                            }
                            li += 1;
                        }
                    }
                    Layout::Split => {
                        for (l, r) in split_pairs(&hunk.lines) {
                            // Folds hold context only, which sits on both sides.
                            if let (Some(a), Some(b)) = (l, r)
                                && a == b
                                && let Some(range) = folded_at(a)
                            {
                                if a == range.start {
                                    push(Row::Fold(fi, hi, range.start, range.end), &|h| fold_key(range, h));
                                }
                                continue;
                            }
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
    BuiltRows { rows, keys, ranges }
}

/// The unchanged runs of a hunk long enough to fold, as line ranges. A run
/// keeps [`FOLD_CONTEXT_KEEP`] lines next to each change it touches.
fn context_folds(lines: &[DiffLine]) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != LineKind::Context {
            i += 1;
            continue;
        }
        let start = i;
        while i < lines.len() && lines[i].kind == LineKind::Context {
            i += 1;
        }
        if i - start <= FOLD_CONTEXT_OVER {
            continue;
        }
        let head = if start == 0 { 0 } else { FOLD_CONTEXT_KEEP };
        let tail = if i == lines.len() { 0 } else { FOLD_CONTEXT_KEEP };
        let fold = start + head..i - tail;
        if fold.len() > 1 {
            out.push(fold);
        }
    }
    out
}

/// Big diffs open folded, so the first paint is a file list rather than the
/// top of the first file.
fn starts_collapsed(files: &[FileDiff]) -> bool {
    let lines: u32 = files.iter().map(|f| f.additions + f.deletions).sum();
    files.len() > COLLAPSE_OVER_FILES || lines > COLLAPSE_OVER_LINES
}

// ------------------------------------------------------------------ entity

pub struct DiffList {
    mode: DiffListMode,
    /// Worktree mode: stage, discard and comments. Off while the working
    /// tree pane shows a commit.
    tree_actions: bool,
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
    folds: Folds,
    /// Paths a `set_files` has shown: the default fold applies to new ones
    /// only, so a refresh never refolds what the reader opened.
    seen: HashSet<String>,
    staged: HashSet<String>,
    layout: Layout,
    wrap: bool,
    /// The code size the rows were measured at; a change remeasures them.
    code_size: f32,
    comments: HashMap<CommentKey, String>,
    commenting: Option<(CommentKey, Entity<InputState>)>,
    confirm_discard: Option<usize>,
    top_file: Option<usize>,
    _highlight: Option<gpui::Task<()>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<DiffListEvent> for DiffList {}

impl DiffList {
    pub fn new(mode: DiffListMode, cx: &mut Context<Self>) -> Self {
        let list = ListState::new(0, ListAlignment::Top, px(600.0));
        let weak = cx.weak_entity();
        // The sticky header follows the scroll position.
        list.set_scroll_handler(move |_, _, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.track_top_file(cx);
                cx.notify();
            });
        });
        Self {
            mode,
            tree_actions: true,
            files: Arc::default(),
            highlights: Arc::default(),
            rows: Vec::new(),
            keys: Vec::new(),
            ranges: Vec::new(),
            list,
            hscroll: ScrollHandle::new(),
            folds: Folds::default(),
            seen: HashSet::new(),
            staged: HashSet::new(),
            layout: Layout::Unified,
            wrap: false,
            code_size: crate::design::code(),
            comments: HashMap::new(),
            commenting: None,
            confirm_discard: None,
            top_file: None,
            _highlight: None,
            _subs: Vec::new(),
        }
    }

    /// Show `files`. An identical diff changes nothing; a changed one keeps
    /// the scroll position, folds and the comments still on the same text,
    /// and is syntax coloured again off the UI thread. Paths seen for the
    /// first time start folded when the diff is large.
    pub fn set_files(&mut self, files: impl Into<Arc<Vec<FileDiff>>>, cx: &mut Context<Self>) {
        let mut files: Arc<Vec<FileDiff>> = files.into();
        if Arc::ptr_eq(&files, &self.files) || *files == *self.files {
            return;
        }
        if has_tabs(&files) {
            files = Arc::new(expand_tabs(files.as_ref().clone()));
        }
        // Comments are keyed by position; a changed diff keeps only the ones
        // still on the same text.
        let old = self.files.clone();
        self.comments.retain(|&(fi, hi, li), _| {
            let line =
                |fs: &[FileDiff]| fs.get(fi).and_then(|f| f.hunks.get(hi)).and_then(|h| h.lines.get(li)).cloned();
            line(&old).is_some() && line(&old) == line(&files)
        });
        if self.commenting.as_ref().is_some_and(|(k, _)| !self.comments.contains_key(k)) {
            self.commenting = None;
            self._subs.clear();
        }
        self.confirm_discard = None;
        let fold_new = starts_collapsed(&files);
        for f in files.iter() {
            if self.seen.insert(f.path.clone()) && fold_new {
                self.folds.files.insert(f.path.clone());
            }
        }
        self.files = files;
        self.highlights = Arc::default();
        self.highlight(cx);
        self.rebuild();
        cx.emit(DiffListEvent::Changed);
        cx.notify();
    }

    /// Drop the diff, its folds and comments: a different source follows.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.files = Arc::default();
        self.highlights = Arc::default();
        self._highlight = None;
        self.comments.clear();
        self.commenting = None;
        self._subs.clear();
        self.confirm_discard = None;
        self.folds = Folds::default();
        self.seen.clear();
        self.top_file = None;
        self.rows.clear();
        self.keys.clear();
        self.ranges.clear();
        self.list.reset(0);
        cx.emit(DiffListEvent::Changed);
        cx.notify();
    }

    /// Worktree mode: whether stage, discard and comments are offered (off
    /// while showing a commit).
    pub fn set_tree_actions(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.tree_actions != on {
            self.tree_actions = on;
            self.confirm_discard = None;
            if !on {
                self.commenting = None;
                self._subs.clear();
            }
            self.rebuild();
            cx.notify();
        }
    }

    /// Worktree mode: the paths that carry a STAGED badge.
    pub fn set_staged(&mut self, staged: HashSet<String>, cx: &mut Context<Self>) {
        if self.staged != staged {
            self.staged = staged;
            cx.notify();
        }
    }

    /// Scroll so `path`'s header sits at the top, opening the file if it is
    /// folded. False when the diff has no such file.
    pub fn scroll_to_file(&mut self, path: &str, cx: &mut Context<Self>) -> bool {
        let Some(fi) = self.files.iter().position(|f| f.path == path) else { return false };
        if self.folds.files.remove(path) {
            self.rebuild();
            cx.emit(DiffListEvent::Changed);
        }
        if let Some(r) = self.ranges.get(fi) {
            self.list.scroll_to(gpui::ListOffset { item_ix: r.start, offset_in_item: px(0.0) });
        }
        if self.top_file != Some(fi) {
            self.top_file = Some(fi);
            cx.emit(DiffListEvent::TopFile(Some(path.to_string())));
        }
        cx.notify();
        true
    }

    /// The file whose rows fill the top of the viewport.
    pub fn top_file(&self) -> Option<&str> {
        self.top_file.and_then(|fi| self.files.get(fi)).map(|f| f.path.as_str())
    }

    fn track_top_file(&mut self, cx: &mut Context<Self>) {
        let top = self.list.logical_scroll_top().item_ix;
        let fi = self.ranges.partition_point(|r| r.start <= top).checked_sub(1);
        if fi != self.top_file {
            self.top_file = fi;
            cx.emit(DiffListEvent::TopFile(self.top_file().map(str::to_string)));
        }
    }

    pub fn all_collapsed(&self) -> bool {
        !self.files.is_empty() && self.files.iter().all(|f| self.folds.files.contains(&f.path))
    }

    pub fn comment_count(&self) -> usize {
        self.comments.len()
    }

    /// Syntax-colour every file off the UI thread; colours only change paint,
    /// never layout, so the rows need no re-measure.
    fn highlight(&mut self, cx: &mut Context<Self>) {
        let files = self.files.clone();
        let work = cx.background_spawn({
            let files = files.clone();
            async move { highlight_files(&files) }
        });
        self._highlight = Some(cx.spawn(async move |this, cx| {
            let spans = work.await;
            let _ = this.update(cx, |v, cx| {
                if Arc::ptr_eq(&files, &v.files) {
                    v.highlights = Arc::new(spans);
                    cx.notify();
                }
            });
        }));
    }

    fn rebuild(&mut self) {
        let comments_on = self.comments_on();
        let empty = HashMap::new();
        let built = build_rows(&RowInputs {
            files: &self.files,
            folds: &self.folds,
            layout: self.layout,
            wrap: self.wrap,
            code_size: self.code_size,
            comments: if comments_on { &self.comments } else { &empty },
            editing: self.commenting.as_ref().map(|(k, _)| *k).filter(|_| comments_on),
        });
        // Splice only the span between the unchanged head and tail.
        let keys = built.keys;
        let head = self.keys.iter().zip(&keys).take_while(|(a, b)| a == b).count();
        let max_tail = self.keys.len().min(keys.len()) - head;
        let tail = self.keys.iter().rev().zip(keys.iter().rev()).take(max_tail).take_while(|(a, b)| a == b).count();
        if head != self.keys.len() || head != keys.len() {
            self.list.splice(head..self.keys.len() - tail, keys.len() - tail - head);
        }
        self.rows = built.rows;
        self.keys = keys;
        self.ranges = built.ranges;
    }

    /// Stage, discard and comments are on.
    fn tree_on(&self) -> bool {
        self.mode == DiffListMode::Worktree && self.tree_actions
    }

    fn comments_on(&self) -> bool {
        self.tree_on()
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.rebuild();
        cx.emit(DiffListEvent::Changed);
        cx.notify();
    }

    pub fn toggle_file(&mut self, fi: usize, cx: &mut Context<Self>) {
        let Some(path) = self.files.get(fi).map(|f| f.path.clone()) else { return };
        if !self.folds.files.remove(&path) {
            self.folds.files.insert(path);
        }
        // Folding the stuck file brings its header back to the top.
        let top = self.list.logical_scroll_top().item_ix;
        let stuck = self.ranges.get(fi).is_some_and(|r| r.contains(&top) && top != r.start);
        self.rebuild();
        if stuck && let Some(r) = self.ranges.get(fi) {
            self.list.scroll_to(gpui::ListOffset { item_ix: r.start, offset_in_item: px(0.0) });
        }
        cx.emit(DiffListEvent::Changed);
        cx.notify();
    }

    /// Fold every file, or open them all when all are folded.
    pub fn toggle_all(&mut self, cx: &mut Context<Self>) {
        if self.all_collapsed() {
            self.folds.files.clear();
        } else {
            self.folds.files = self.files.iter().map(|f| f.path.clone()).collect();
        }
        self.changed(cx);
    }

    /// Fold a hunk to its header, or open it again.
    pub fn toggle_hunk(&mut self, fi: usize, hi: usize, cx: &mut Context<Self>) {
        let Some((path, header)) =
            self.files.get(fi).and_then(|f| Some((f.path.clone(), f.hunks.get(hi)?.header.clone())))
        else {
            return;
        };
        let key = (path, header);
        if !self.folds.hunks.remove(&key) {
            self.folds.hunks.insert(key);
        }
        self.changed(cx);
    }

    /// Show the unchanged lines a fold row stands for.
    fn open_context(&mut self, fi: usize, hi: usize, start: usize, cx: &mut Context<Self>) {
        let Some((path, header)) =
            self.files.get(fi).and_then(|f| Some((f.path.clone(), f.hunks.get(hi)?.header.clone())))
        else {
            return;
        };
        self.folds.open_context.insert((path, header, start));
        self.changed(cx);
    }

    pub fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        self.layout = layout;
        self.changed(cx);
    }

    pub fn toggle_layout(&mut self, cx: &mut Context<Self>) {
        let next = if self.layout == Layout::Split { Layout::Unified } else { Layout::Split };
        self.set_layout(next, cx);
    }

    pub fn toggle_wrap(&mut self, cx: &mut Context<Self>) {
        self.wrap = !self.wrap;
        self.hscroll.set_offset(gpui::point(px(0.0), px(0.0)));
        self.changed(cx);
    }

    // ------------------------------------------------------------ comments

    fn start_comment(&mut self, key: CommentKey, window: &mut Window, cx: &mut Context<Self>) {
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
                this.changed(cx);
            }
        });
        self._subs = vec![sub];
        self.commenting = Some((key, input));
        self.changed(cx);
    }

    fn remove_comment(&mut self, key: CommentKey, cx: &mut Context<Self>) {
        self.comments.remove(&key);
        if self.commenting.as_ref().is_some_and(|(k, _)| *k == key) {
            self.commenting = None;
            self._subs.clear();
        }
        self.changed(cx);
    }

    /// The comments as one review message for the agent, clearing them.
    /// None when there are none.
    pub fn take_review(&mut self, cx: &mut Context<Self>) -> Option<String> {
        if self.comments.is_empty() {
            return None;
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
        self.changed(cx);
        Some(out)
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
            Row::Hunk(fi, hi) => self.render_hunk_header(fi, hi, &t, cx),
            Row::Line(fi, hi, li) => self.render_line(ix, fi, hi, li, width, &t, cx),
            Row::Pair(fi, hi, l, r) => self.render_pair(ix, fi, hi, l, r, width, &t, cx),
            Row::Fold(fi, hi, start, end) => self.render_fold(ix, fi, hi, start, end, &t, cx),
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
        let collapsed = self.folds.files.contains(&f.path);
        let staged = self.staged.contains(&f.path);
        let in_tree = self.tree_on();
        let confirm = self.confirm_discard == Some(fi) && in_tree;
        let can_open = self.mode == DiffListMode::Worktree;
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
            let stage_path = path.clone();
            d.child(hidden(action("diff-stage", icons::CHECK, if staged { "Unstage" } else { "Stage" })).on_click(
                cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.emit(DiffListEvent::ToggleStage(stage_path.clone()));
                }),
            ))
            .child(if confirm {
                let discard_path = path.clone();
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
                        this.confirm_discard = None;
                        cx.emit(DiffListEvent::Discard(discard_path.clone()));
                        cx.notify();
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
        .when(can_open, |d| {
            d.child(action("diff-open-file", icons::DOCUMENT, "Open in file browser").on_click(move |_, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(Box::new(crate::actions::OpenPath { path: path.clone(), line: None }), cx);
            }))
        })
        .into_any_element()
    }

    /// The `@@ … @@` row; a click folds the hunk to this one line.
    fn render_hunk_header(&self, fi: usize, hi: usize, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let f = &self.files[fi];
        let hunk = &f.hunks[hi];
        let folded = self.folds.hunks.contains(&(f.path.clone(), hunk.header.clone()));
        let muted = t.text_muted;
        div()
            .id(("diff-hunk", fi * 100_000 + hi))
            .h(px(HUNK_HEADER_HEIGHT))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.0))
            .pl(px(10.0))
            .pr(px(16.0))
            .bg(t.diff_hunk_bg)
            .font_family(t.font_mono.clone())
            .text_size(ty::MICRO)
            .text_color(t.text_faint)
            .cursor_pointer()
            .hover(move |s| s.text_color(muted))
            .tooltip(ui::tip(if folded { "Show this hunk" } else { "Fold this hunk" }))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_hunk(fi, hi, cx)))
            .child(
                icon(if folded { icons::ALT_ARROW_RIGHT } else { icons::ALT_ARROW_DOWN })
                    .size(px(11.0))
                    .flex_none()
                    .text_color(t.text_faint.opacity(0.8)),
            )
            .child(div().flex_1().min_w_0().truncate().child(hunk.header.to_string()))
            .when(folded, |d| {
                let n = hunk.lines.len();
                d.child(div().flex_none().child(format!("{n} {}", if n == 1 { "line" } else { "lines" })))
            })
            .into_any_element()
    }

    /// "⋯ N unchanged lines": a click shows them.
    #[allow(clippy::too_many_arguments)]
    fn render_fold(
        &self,
        ix: usize,
        fi: usize,
        hi: usize,
        start: usize,
        end: usize,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let indent = ACCENT_BAR_WIDTH
            + match self.layout {
                Layout::Unified => 2.0 * gutter_width(&self.files[fi]) + MARKER_WIDTH,
                Layout::Split => gutter_width(&self.files[fi]) + SPLIT_MARKER_WIDTH,
            };
        let n = end - start;
        let (rest, hover) = (ink(0.02), ink(0.05));
        let muted = t.text_muted;
        div()
            .id(("diff-fold", ix))
            .w_full()
            .h(px(line_h()))
            .flex_none()
            .flex()
            .items_center()
            .pl(px(indent))
            .bg(rest)
            .font_family(t.font_mono.clone())
            .text_size(ty::MICRO)
            .text_color(t.text_faint)
            .cursor_pointer()
            .hover(move |s| s.bg(hover).text_color(muted))
            .on_click(cx.listener(move |this, _, _, cx| this.open_context(fi, hi, start, cx)))
            .child(format!("⋯ {n} unchanged lines"))
            .into_any_element()
    }

    /// The `+` that opens a comment on a line. Each row has its own group,
    /// so the adder shows on the hovered row only. A shared id painted it
    /// on every virtual row; storing the hover and notifying rebuilt the list.
    fn adder(
        &self,
        ix: usize,
        left: f32,
        key: CommentKey,
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
            .when(self.comments_on(), |d| d.child(self.adder(ix, adder_left, key, &group, t, cx)))
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
        let commentable = self.comments_on() && (right.is_some() || left.is_some());
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
}

impl Render for DiffList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The code font size changed in Settings: remeasure every row.
        let code_size = crate::design::code();
        if (code_size - self.code_size).abs() > f32::EPSILON {
            self.code_size = code_size;
            self.rebuild();
        }
        let t = Theme::of(cx).clone();
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
    }
}

/// A toolbar toggle in the diff's style: lit while `on`.
pub fn toolbar_toggle(
    id: &'static str,
    glyph: &'static str,
    label: &'static str,
    on: bool,
    t: &Theme,
) -> gpui::Stateful<gpui::Div> {
    surface_chrome::toolbar_button(id, label)
        .when(on, |d| d.bg(ink(0.10)))
        .child(icon(glyph).size(px(surface_chrome::ICON_SIZE)).text_color(if on { t.text } else { t.text_muted }))
}

/// The wrap, side-by-side and fold-all toggles for a toolbar that hosts
/// `diff`. The host re-renders on [`DiffListEvent::Changed`] to keep them lit.
pub fn view_toggles(diff: &Entity<DiffList>, t: &Theme, cx: &App) -> Vec<AnyElement> {
    let d = diff.read(cx);
    let all_folded = d.all_collapsed();
    let (wrap, split) = (d.wrap, d.layout == Layout::Split);
    let on = |f: fn(&mut DiffList, &mut Context<DiffList>)| {
        let diff = diff.clone();
        move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut App| diff.update(cx, f)
    };
    vec![
        toolbar_toggle("diff-wrap", icons::WRAP_TEXT, "Wrap long lines", wrap, t)
            .on_click(on(DiffList::toggle_wrap))
            .into_any_element(),
        toolbar_toggle("diff-split", icons::SPLIT_COLUMNS, "Side by side", split, t)
            .on_click(on(DiffList::toggle_layout))
            .into_any_element(),
        toolbar_toggle(
            "diff-fold",
            icons::FOLD_VERTICAL,
            if all_folded { "Expand all files" } else { "Collapse all files" },
            all_folded,
            t,
        )
        .on_click(on(DiffList::toggle_all))
        .into_any_element(),
    ]
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
pub(crate) fn gutter_width(f: &FileDiff) -> f32 {
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

fn has_tabs(files: &[FileDiff]) -> bool {
    files.iter().flat_map(|f| f.hunks.iter().flat_map(|h| h.lines.iter())).any(|l| l.text.contains('\t'))
}

/// Tabs become spaces once, before highlighting, so spans and widths agree.
/// Callers loading a diff off the UI thread run it there; `set_files` runs
/// it for any diff that still has tabs.
pub(crate) fn expand_tabs(mut files: Vec<FileDiff>) -> Vec<FileDiff> {
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
pub(crate) fn split_pairs(lines: &[DiffLine]) -> Vec<(Option<usize>, Option<usize>)> {
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

    /// A hunk from `kinds`: C context, A added, R removed.
    fn hunk(header: &str, kinds: &str) -> Hunk {
        let (mut old, mut new) = (1u32, 1u32);
        let lines = kinds
            .chars()
            .map(|c| {
                let kind = match c {
                    'A' => LineKind::Added,
                    'R' => LineKind::Removed,
                    _ => LineKind::Context,
                };
                let (o, n) = match kind {
                    LineKind::Added => (None, Some(new)),
                    LineKind::Removed => (Some(old), None),
                    LineKind::Context => (Some(old), Some(new)),
                };
                old += o.is_some() as u32;
                new += n.is_some() as u32;
                DiffLine { kind, old_no: o, new_no: n, text: "x".into() }
            })
            .collect();
        Hunk { header: header.into(), lines }
    }

    fn file(path: &str, hunks: Vec<Hunk>) -> FileDiff {
        let count = |k| hunks.iter().flat_map(|h| &h.lines).filter(|l| l.kind == k).count() as u32;
        FileDiff {
            path: path.into(),
            old_path: None,
            status: FileStatus::Modified,
            binary: false,
            additions: count(LineKind::Added),
            deletions: count(LineKind::Removed),
            hunks,
        }
    }

    fn rows(files: &[FileDiff], folds: &Folds, layout: Layout) -> BuiltRows {
        build_rows(&RowInputs {
            files,
            folds,
            layout,
            wrap: false,
            code_size: 13.0,
            comments: &HashMap::new(),
            editing: None,
        })
    }

    #[test]
    fn open_file_lists_every_line() {
        let files = [file("a.rs", vec![hunk("@@ a", "CRAC")]), file("b.rs", vec![hunk("@@ b", "A")])];
        let b = rows(&files, &Folds::default(), Layout::Unified);
        assert_eq!(
            b.rows,
            vec![
                Row::File(0),
                Row::Hunk(0, 0),
                Row::Line(0, 0, 0),
                Row::Line(0, 0, 1),
                Row::Line(0, 0, 2),
                Row::Line(0, 0, 3),
                Row::End(0),
                Row::File(1),
                Row::Hunk(1, 0),
                Row::Line(1, 0, 0),
                Row::End(1),
            ]
        );
        assert_eq!(b.ranges, vec![0..7, 7..11]);
        assert_eq!(b.keys.len(), b.rows.len());
    }

    #[test]
    fn collapsed_file_is_its_header() {
        let files = [file("a.rs", vec![hunk("@@ a", "CRAC")]), file("b.rs", vec![hunk("@@ b", "A")])];
        let mut folds = Folds::default();
        folds.files.insert("a.rs".into());
        let b = rows(&files, &folds, Layout::Unified);
        assert_eq!(b.rows[..2], [Row::File(0), Row::File(1)]);
        assert_eq!(b.ranges, vec![0..1, 1..5]);
        // Folding changes the header's key, so the list remeasures it.
        let open = rows(&files, &Folds::default(), Layout::Unified);
        assert_ne!(b.keys[0], open.keys[0]);
        assert_eq!(b.keys[1], open.keys[7]);
    }

    #[test]
    fn collapsed_hunk_is_its_header() {
        let files = [file("a.rs", vec![hunk("@@ one", "CAC"), hunk("@@ two", "R")])];
        let mut folds = Folds::default();
        folds.hunks.insert(("a.rs".into(), "@@ one".into()));
        for layout in [Layout::Unified, Layout::Split] {
            let b = rows(&files, &folds, layout);
            assert_eq!(b.rows[..3], [Row::File(0), Row::Hunk(0, 0), Row::Hunk(0, 1)], "{layout:?}");
        }
    }

    #[test]
    fn long_context_folds_to_one_row() {
        // 2 changes around 12 unchanged lines: 3 stay on each side, 6 fold.
        let kinds = format!("A{}R", "C".repeat(12));
        let files = [file("a.rs", vec![hunk("@@", &kinds)])];
        assert_eq!(context_folds(&files[0].hunks[0].lines), vec![4..10]);
        let b = rows(&files, &Folds::default(), Layout::Unified);
        let lines = b.rows.iter().filter(|r| matches!(r, Row::Line(..))).count();
        assert_eq!(lines, 14 - 6);
        assert!(b.rows.contains(&Row::Fold(0, 0, 4, 10)));
        let split = rows(&files, &Folds::default(), Layout::Split);
        assert_eq!(split.rows.iter().filter(|r| matches!(r, Row::Fold(..))).count(), 1);
        assert_eq!(split.rows.iter().filter(|r| matches!(r, Row::Pair(..))).count(), 14 - 6);

        // Opening the fold brings every line back.
        let mut folds = Folds::default();
        folds.open_context.insert(("a.rs".into(), "@@".into(), 4));
        let b = rows(&files, &folds, Layout::Unified);
        assert_eq!(b.rows.iter().filter(|r| matches!(r, Row::Line(..))).count(), 14);
        assert!(!b.rows.iter().any(|r| matches!(r, Row::Fold(..))));
    }

    #[test]
    fn short_or_edge_context_folds_as_expected() {
        // Eight unchanged lines stay open.
        let eight = hunk("@@", &format!("A{}A", "C".repeat(8)));
        assert!(context_folds(&eight.lines).is_empty());
        // A run at the hunk's start keeps only the lines before the change.
        let leading = hunk("@@", &format!("{}A", "C".repeat(10)));
        assert_eq!(context_folds(&leading.lines), vec![0..7]);
        // A run at the end keeps only the lines after the change.
        let trailing = hunk("@@", &format!("R{}", "C".repeat(10)));
        assert_eq!(context_folds(&trailing.lines), vec![4..11]);
    }

    #[test]
    fn default_policy_folds_big_diffs() {
        let small: Vec<_> = (0..8).map(|i| file(&format!("{i}.rs"), vec![hunk("@@", "A")])).collect();
        assert!(!starts_collapsed(&small));
        let many: Vec<_> = (0..9).map(|i| file(&format!("{i}.rs"), vec![hunk("@@", "A")])).collect();
        assert!(starts_collapsed(&many));
        let long = [file("big.rs", vec![hunk("@@", &"A".repeat(401))])];
        assert!(starts_collapsed(&long));
        let edge = [file("big.rs", vec![hunk("@@", &"A".repeat(400))])];
        assert!(!starts_collapsed(&edge));
    }

    #[test]
    fn comment_rows_follow_their_line() {
        let files = [file("a.rs", vec![hunk("@@", "CAC")])];
        let comments = HashMap::from([((0, 0, 1), "why?".to_string())]);
        let b = build_rows(&RowInputs {
            files: &files,
            folds: &Folds::default(),
            layout: Layout::Unified,
            wrap: false,
            code_size: 13.0,
            comments: &comments,
            editing: None,
        });
        let at = b.rows.iter().position(|r| *r == Row::Line(0, 0, 1)).unwrap();
        assert_eq!(b.rows[at + 1], Row::Comment(0, 0, 1));
    }
}
