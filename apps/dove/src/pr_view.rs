//! One pull request, shown the same way on the Pull Requests page and in a
//! thread's dock: a header with the PR's state and actions, then four tabs.
//!
//! - **Walkthrough**: where the PR stands (overview and a rule-based
//!   assessment with the next step), then a plain-language explanation.
//! - **Changes**: the diff, file by file, collapsible.
//! - **Checks**: CI grouped by outcome, with a hand-off to fix failures.
//! - **Activity**: the description, reviews, comments and commits, plus a
//!   reply box (comment, request changes, approve).
//!
//! Loading runs on the background executor; a small LRU keeps recently seen
//! PRs so reopening one is instant while it refreshes behind the scenes.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dove_core::Engine;
use dove_core::git::FileDiff;
use dove_core::git::github::{self, Check, MergeMethod, PrDetail, PrRepo, ReviewKind};
use dove_core::proto::HarnessKind;
use dove_core::store::Project;
use dove_core::walkthrough::{Category, Confidence, Level, Severity, Walkthrough, WalkthroughInput};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, Global, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};
use gpui_base::input::InputState;

use crate::assets::{icon, icons};
use crate::design::{space, ty};
use crate::diff_list::{DiffList, DiffListMode};
use crate::kit::controls::{self as ctl, Segment, SelectState, Tone};
use crate::kit::{self, page};
use crate::theme::Theme;
use crate::ui;

/// Which pull request a [`PrView`] shows.
#[derive(Debug, Clone, PartialEq)]
pub struct PrTarget {
    pub repo: PrRepo,
    pub number: u64,
    /// The PR's GitHub URL (for "Open on GitHub" before details load).
    pub url: String,
    /// The Dove project it belongs to, when there is one (enables checkout).
    pub project: Option<Project>,
}

pub enum PrViewEvent {
    /// The page's back arrow.
    Back,
    /// Start a thread on this PR's branch.
    Checkout { project: Project, number: u64, title: String },
    /// A prepared prompt for an agent (review, fix checks, address
    /// feedback, resolve conflicts). Staged for the user to send.
    HandOff(String),
    /// Fresh details arrived (the dock and chat card mirror them).
    Loaded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrTab {
    Walkthrough,
    Changes,
    Checks,
    Activity,
}

impl PrTab {
    const ALL: [PrTab; 4] = [PrTab::Walkthrough, PrTab::Changes, PrTab::Checks, PrTab::Activity];

    /// Walkthrough, or `DOVE_PR_TAB=<tab>` (QA and scripting).
    fn initial() -> Self {
        std::env::var("DOVE_PR_TAB")
            .ok()
            .and_then(|id| Self::ALL.into_iter().find(|t| t.label().eq_ignore_ascii_case(&id)))
            .unwrap_or(PrTab::Walkthrough)
    }

    fn label(self) -> &'static str {
        match self {
            PrTab::Walkthrough => "Walkthrough",
            PrTab::Changes => "Changes",
            PrTab::Checks => "Checks",
            PrTab::Activity => "Activity",
        }
    }
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

/// How many loaded PRs to keep (details plus parsed diff).
const CACHE_PRS: usize = 12;

#[derive(Default)]
struct PrCache {
    /// Most recently used last.
    entries: Vec<(PrRepo, u64, PrDetail, Arc<Vec<FileDiff>>)>,
}

impl Global for PrCache {}

fn cache_get(repo: &PrRepo, number: u64, cx: &App) -> Option<(PrDetail, Arc<Vec<FileDiff>>)> {
    cx.try_global::<PrCache>()?
        .entries
        .iter()
        .find(|(r, n, ..)| r == repo && *n == number)
        .map(|(.., d, f)| (d.clone(), f.clone()))
}

fn cache_put(repo: PrRepo, number: u64, detail: PrDetail, files: Arc<Vec<FileDiff>>, cx: &mut App) {
    let cache = cx.default_global::<PrCache>();
    cache.entries.retain(|(r, n, ..)| !(r == &repo && *n == number));
    cache.entries.push((repo, number, detail, files));
    if cache.entries.len() > CACHE_PRS {
        cache.entries.remove(0);
    }
}

/// Light details for chat cards, by canonical URL, fetched at most once a
/// [`SUMMARY_TTL`] however many cards show the same PR.
#[derive(Default)]
struct PrSummaries {
    map: std::collections::HashMap<String, (Instant, PrDetail)>,
    inflight: HashSet<String>,
}

impl Global for PrSummaries {}

const SUMMARY_TTL: Duration = Duration::from_secs(60);

/// The PR at `url`'s title, state, size and checks, when known. A missing
/// or stale entry starts one background fetch that re-renders the asking
/// view when it lands.
pub fn summary_for<V: 'static>(url: &str, cx: &mut Context<V>) -> Option<PrDetail> {
    let cached = cx.try_global::<PrSummaries>().and_then(|s| s.map.get(url).cloned());
    let stale = cached.as_ref().is_none_or(|(at, _)| at.elapsed() > SUMMARY_TTL);
    let inflight = cx.try_global::<PrSummaries>().is_some_and(|s| s.inflight.contains(url));
    if stale
        && !inflight
        && let Some((slug, number)) = github::parse_pr_url(url)
    {
        cx.default_global::<PrSummaries>().inflight.insert(url.to_string());
        let key = url.to_string();
        let work = cx.background_spawn(async move { github::pr_summary(&PrRepo::Remote(slug), number) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            cx.update(|cx| {
                let s = cx.default_global::<PrSummaries>();
                s.inflight.remove(&key);
                if let Ok(d) = result {
                    s.map.insert(key, (Instant::now(), d));
                }
            });
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }
    cached.map(|(_, d)| d)
}

/// Fresh full details also refresh the card summaries.
fn remember_summary(detail: &PrDetail, cx: &mut App) {
    if let Some((slug, n)) = github::parse_pr_url(&detail.url) {
        let url = format!("https://github.com/{slug}/pull/{n}");
        cx.default_global::<PrSummaries>().map.insert(url, (Instant::now(), detail.clone()));
    }
}

// ---------------------------------------------------------------------------
// Assessment
// ---------------------------------------------------------------------------

/// What stands between a PR and merging, worst first, with the hand-off
/// that would help.
struct Blocker {
    text: &'static str,
    tone: BlockTone,
    hand_off: Option<HandOff>,
}

#[derive(Clone, Copy, PartialEq)]
enum BlockTone {
    Bad,
    Wait,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandOff {
    Review,
    AddressFeedback,
    FixChecks,
    ResolveConflicts,
}

impl HandOff {
    fn label(self) -> &'static str {
        match self {
            HandOff::Review => "Ask an agent to review",
            HandOff::AddressFeedback => "Ask an agent to address feedback",
            HandOff::FixChecks => "Ask an agent to fix checks",
            HandOff::ResolveConflicts => "Ask an agent to resolve conflicts",
        }
    }

    fn prompt(self, d: &PrDetail) -> String {
        let id = format!("pull request #{} \"{}\" ({})", d.number, d.title, d.url);
        match self {
            HandOff::Review => format!(
                "Review {id}. Read the diff with `gh pr diff {n}`. Summarise what it does, then list bugs, \
                 regressions, risky changes and missing tests, each with the file and line.",
                n = d.number
            ),
            HandOff::AddressFeedback => format!(
                "Address the review feedback on {id}. Read it with `gh pr view {n} --comments`, make the \
                 requested changes on branch `{b}`, and push.",
                n = d.number,
                b = d.branch
            ),
            HandOff::FixChecks => {
                let failing: Vec<&str> =
                    d.checks.iter().filter(|c| check_bucket(c) == Bucket::Failed).map(|c| c.name.as_str()).collect();
                format!(
                    "These checks fail on {id}: {}. Find out why with `gh pr checks {n}` and the run logs, fix \
                     the cause on branch `{b}`, and push.",
                    failing.join(", "),
                    n = d.number,
                    b = d.branch
                )
            }
            HandOff::ResolveConflicts => format!(
                "{id} conflicts with `{base}`. Merge `{base}` into `{b}`, resolve the conflicts keeping both \
                 sides' intent, run the tests, and push.",
                base = d.base,
                b = d.branch
            ),
        }
    }
}

fn blockers(d: &PrDetail) -> Vec<Blocker> {
    let mut out = Vec::new();
    if d.state != "OPEN" {
        return out;
    }
    let failing = d.checks.iter().filter(|c| check_bucket(c) == Bucket::Failed).count();
    let pending = d.checks.iter().filter(|c| check_bucket(c) == Bucket::Pending).count();
    if d.mergeable == "CONFLICTING" || d.merge_state == "DIRTY" {
        out.push(Blocker {
            text: "It conflicts with the base branch",
            tone: BlockTone::Bad,
            hand_off: Some(HandOff::ResolveConflicts),
        });
    }
    if failing > 0 {
        out.push(Blocker {
            text: if failing == 1 { "A check is failing" } else { "Some checks are failing" },
            tone: BlockTone::Bad,
            hand_off: Some(HandOff::FixChecks),
        });
    }
    if d.review_decision == "CHANGES_REQUESTED" {
        out.push(Blocker {
            text: "A reviewer asked for changes",
            tone: BlockTone::Bad,
            hand_off: Some(HandOff::AddressFeedback),
        });
    }
    if d.draft {
        out.push(Blocker { text: "It is still a draft", tone: BlockTone::Wait, hand_off: None });
    }
    if pending > 0 {
        out.push(Blocker { text: "Checks are still running", tone: BlockTone::Wait, hand_off: None });
    }
    if d.review_decision == "REVIEW_REQUIRED" {
        out.push(Blocker {
            text: "It needs an approving review",
            tone: BlockTone::Wait,
            hand_off: Some(HandOff::Review),
        });
    }
    if d.merge_state == "BEHIND" {
        out.push(Blocker { text: "It is behind the base branch", tone: BlockTone::Wait, hand_off: None });
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bucket {
    Failed,
    Pending,
    Passed,
}

fn check_bucket(c: &Check) -> Bucket {
    match c.state.as_str() {
        "SUCCESS" | "NEUTRAL" | "SKIPPED" => Bucket::Passed,
        "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE" => Bucket::Failed,
        _ => Bucket::Pending,
    }
}

/// A PR's state as a pill: label and colour.
pub fn status_style(d: &PrDetail, t: &Theme) -> (&'static str, Hsla) {
    match d.status() {
        "merged" => ("Merged", t.accent),
        "closed" => ("Closed", t.danger),
        "draft" => ("Draft", t.text_faint),
        _ => ("Open", t.success),
    }
}

/// "2026-10-19T09:30:00Z" → milliseconds since the epoch, for
/// [`ui::relative_time`]. `None` for anything else.
fn iso_ms(iso: &str) -> Option<i64> {
    let b = iso.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[10] != b'T' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| iso.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    // Days from civil (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400) + hh * 3600 + mm * 60 + ss) * 1000)
}

fn ago(iso: &str) -> String {
    iso_ms(iso)
        .map(|ms| match ui::relative_time(ms).as_str() {
            "now" => "just now".to_string(),
            r => format!("{r} ago"),
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Refresh an open PR this often while it is on screen and checks run.
const POLL: Duration = Duration::from_secs(30);
/// Treat the view as off screen when it has not rendered for this long.
const SEEN_WITHIN: Duration = Duration::from_secs(5);

pub struct PrView {
    target: Option<PrTarget>,
    /// Dock layout: narrower, no back arrow, tighter header.
    compact: bool,
    tab: PrTab,
    detail: Option<PrDetail>,
    files: Option<Arc<Vec<FileDiff>>>,
    /// The raw unified diff (the walkthrough is written from it).
    patch: Option<Arc<str>>,
    body: Option<Arc<Vec<crate::markdown::Block>>>,
    error: Option<SharedString>,
    loading: bool,
    /// The Changes tab: Changes' own virtualized, collapsible diff.
    diff: Entity<DiffList>,
    engine: Arc<Engine>,
    /// The agent that writes walkthroughs (the thread's own, when known).
    harness: HarnessKind,
    walkthrough: Option<Walkthrough>,
    /// The walkthrough being written is for this head commit.
    walking: Option<String>,
    merge_method: MergeMethod,
    merge_select: SelectState,
    /// The action in flight ("Merging…") and the last action's outcome.
    busy: Option<&'static str>,
    note: Option<(SharedString, bool)>,
    reply: Entity<InputState>,
    last_render: Instant,
    _load: Option<gpui::Task<()>>,
    _act: Option<gpui::Task<()>>,
    _poll: Option<gpui::Task<()>>,
    _walk: Option<gpui::Task<()>>,
}

impl EventEmitter<PrViewEvent> for PrView {}

impl PrView {
    pub fn new(engine: Arc<Engine>, compact: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let style = ui::input_style(Theme::of(cx));
        let reply = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Leave a comment");
            s.set_editor_style(style);
            s
        });
        let diff = cx.new(|cx| DiffList::new(DiffListMode::ReadOnly, cx));
        cx.observe(&diff, |_, _, cx| cx.notify()).detach();
        Self {
            target: None,
            compact,
            tab: PrTab::initial(),
            detail: None,
            files: None,
            patch: None,
            body: None,
            error: None,
            loading: false,
            diff,
            engine,
            harness: HarnessKind::ClaudeCode,
            walkthrough: None,
            walking: None,
            merge_method: MergeMethod::Squash,
            merge_select: SelectState::default(),
            busy: None,
            note: None,
            reply,
            last_render: Instant::now(),
            _load: None,
            _act: None,
            _poll: None,
            _walk: None,
        }
    }

    /// Prefer `kind` for writing walkthroughs (the thread's own agent).
    pub fn set_harness(&mut self, kind: HarnessKind) {
        self.harness = kind;
    }

    pub fn target(&self) -> Option<&PrTarget> {
        self.target.as_ref()
    }

    pub fn detail(&self) -> Option<&PrDetail> {
        self.detail.as_ref()
    }

    /// Show `target` (no-op when it is already shown).
    pub fn set_target(&mut self, target: PrTarget, cx: &mut Context<Self>) {
        if self.target.as_ref().is_some_and(|t| t.repo == target.repo && t.number == target.number) {
            return;
        }
        self.tab = PrTab::initial();
        self.detail = None;
        self.files = None;
        self.patch = None;
        self.body = None;
        self.error = None;
        self.note = None;
        self.busy = None;
        self.walkthrough = None;
        self.walking = None;
        self._walk = None;
        self.diff.update(cx, |d, cx| d.clear(cx));
        if let Some((detail, files)) = cache_get(&target.repo, target.number, cx) {
            self.apply(detail, files, None, cx);
        }
        self.target = Some(target);
        self.refresh(cx);
        self.start_polling(cx);
    }

    /// Re-fetch details and diff.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.target.clone() else { return };
        self.loading = true;
        let (repo, number) = (target.repo.clone(), target.number);
        let work = cx.background_spawn(async move {
            let detail = github::pr_view(&repo, number)?;
            let patch = github::diff(&repo, number).unwrap_or_default();
            let files = Arc::new(dove_core::git::diff::parse(&patch));
            anyhow::Ok((detail, files, Arc::<str>::from(patch)))
        });
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |v, cx| {
                if v.target.as_ref().is_none_or(|t| t.repo != target.repo || t.number != number) {
                    return;
                }
                v.loading = false;
                match result {
                    Ok((detail, files, patch)) => {
                        cache_put(target.repo.clone(), number, detail.clone(), files.clone(), cx);
                        remember_summary(&detail, cx);
                        cx.emit(PrViewEvent::Loaded);
                        v.apply(detail, files, Some(patch), cx);
                        v.error = None;
                    }
                    Err(e) => v.error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn apply(&mut self, detail: PrDetail, files: Arc<Vec<FileDiff>>, patch: Option<Arc<str>>, cx: &mut Context<Self>) {
        if self.detail.as_ref().is_none_or(|d| d.body != detail.body) {
            self.body = Some(crate::markdown::parse(if detail.body.trim().is_empty() {
                "_No description._"
            } else {
                &detail.body
            }));
        }
        // A new head commit makes the walkthrough stale.
        if self.detail.as_ref().is_some_and(|d| d.head_sha != detail.head_sha) {
            self.walkthrough = None;
        }
        self.diff.update(cx, |d, cx| d.set_files(files.clone(), cx));
        self.detail = Some(detail);
        self.files = Some(files);
        if patch.is_some() {
            self.patch = patch;
        }
    }

    /// Ask for a walkthrough of the loaded PR (cached per head commit).
    /// `fresh` skips the cache (Regenerate).
    fn walk(&mut self, fresh: bool, cx: &mut Context<Self>) {
        let (Some(d), Some(patch), Some(target)) = (self.detail.clone(), self.patch.clone(), self.target.clone())
        else {
            return;
        };
        if self.walking.as_deref() == Some(d.head_sha.as_str()) {
            return;
        }
        let slug = github::parse_pr_url(&d.url).map(|(s, _)| s).unwrap_or_default();
        let key = dove_core::walkthrough_cache_key(&slug, target.number, &d.head_sha);
        let input = WalkthroughInput {
            title: d.title.clone(),
            body: d.body.clone(),
            commits: d.commits.iter().map(|c| c.headline.clone()).collect(),
            files: d
                .files
                .iter()
                .map(|f| dove_core::walkthrough::FileStat {
                    path: f.path.clone(),
                    additions: f.additions as u32,
                    deletions: f.deletions as u32,
                    status: self
                        .files
                        .as_ref()
                        .and_then(|fs| fs.iter().find(|x| x.path == f.path))
                        .map(|x| match x.status {
                            dove_core::git::FileStatus::Added => "added",
                            dove_core::git::FileStatus::Deleted => "removed",
                            dove_core::git::FileStatus::Renamed => "renamed",
                            dove_core::git::FileStatus::Modified => "modified",
                        })
                        .unwrap_or("modified")
                        .to_string(),
                })
                .collect(),
            patch: patch.to_string(),
        };
        let rx = if fresh {
            self.engine.generate_walkthrough(self.harness, input, None)
        } else {
            self.engine.generate_walkthrough(self.harness, input, Some(key.clone()))
        };
        self.walking = Some(d.head_sha.clone());
        let engine = self.engine.clone();
        let head = d.head_sha.clone();
        self._walk = Some(cx.spawn(async move |this, cx| {
            let Ok(w) = rx.await else { return };
            if fresh && w.ai {
                engine.store_walkthrough(&key, &w);
            }
            let _ = this.update(cx, |v, cx| {
                if v.detail.as_ref().is_some_and(|d| d.head_sha == head) {
                    v.walkthrough = Some(w);
                }
                v.walking = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Refresh every [`POLL`] while the view is on screen, the PR is open
    /// and checks are still running. Nothing runs while it is hidden.
    fn start_polling(&mut self, cx: &mut Context<Self>) {
        self._poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let alive = this.update(cx, |v, cx| {
                    let live = v.last_render.elapsed() < SEEN_WITHIN
                        && v.detail.as_ref().is_some_and(|d| {
                            d.state == "OPEN" && d.checks.iter().any(|c| check_bucket(c) == Bucket::Pending)
                        });
                    if live && !v.loading {
                        v.refresh(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    /// Run a gh action off the UI thread, then refresh.
    fn act(
        &mut self,
        label: &'static str,
        done: &'static str,
        work: impl FnOnce(&PrRepo, u64) -> anyhow::Result<()> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.target.clone() else { return };
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(label);
        self.note = None;
        let job = cx.background_spawn(async move { work(&target.repo, target.number) });
        self._act = Some(cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |v, cx| {
                v.busy = None;
                v.note = Some(match result {
                    Ok(()) => (done.into(), true),
                    Err(e) => (format!("{e:#}").into(), false),
                });
                v.refresh(cx);
            });
        }));
        cx.notify();
    }

    fn merge(&mut self, cx: &mut Context<Self>) {
        let method = self.merge_method;
        self.act("Merging…", "Merged.", move |repo, n| github::merge(repo, n, method), cx);
    }

    fn review(&mut self, kind: ReviewKind, window: &mut Window, cx: &mut Context<Self>) {
        let body = self.reply.read(cx).value().to_string();
        if kind != ReviewKind::Approve && body.trim().is_empty() {
            self.note = Some(("Write a comment first.".into(), false));
            cx.notify();
            return;
        }
        self.reply.update(cx, |s, cx| s.set_value("", window, cx));
        let (label, done) = match kind {
            ReviewKind::Approve => ("Approving…", "Approved."),
            ReviewKind::RequestChanges => ("Requesting changes…", "Changes requested."),
            ReviewKind::Comment => ("Commenting…", "Comment posted."),
        };
        self.act(
            label,
            done,
            move |repo, n| match kind {
                ReviewKind::Comment => github::comment(repo, n, &body),
                _ => github::review(repo, n, kind, &body),
            },
            cx,
        );
    }

    fn hand_off(&mut self, h: HandOff, cx: &mut Context<Self>) {
        if let Some(d) = &self.detail {
            cx.emit(PrViewEvent::HandOff(h.prompt(d)));
        }
    }

    pub fn show_tab(&mut self, tab: PrTab, cx: &mut Context<Self>) {
        self.tab = tab;
        cx.notify();
    }

    /// Open `path` in the Changes tab.
    pub fn reveal_file(&mut self, path: &str, cx: &mut Context<Self>) {
        self.tab = PrTab::Changes;
        let path = path.to_string();
        self.diff.update(cx, |d, cx| {
            d.scroll_to_file(&path, cx);
        });
        cx.notify();
    }

    // ---- header ----

    fn render_header(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let target = self.target.as_ref();
        let d = self.detail.as_ref();
        let url = d.map(|d| d.url.clone()).or_else(|| target.map(|t| t.url.clone())).unwrap_or_default();
        let number = target.map(|t| t.number).unwrap_or_default();
        let title: SharedString =
            d.map(|d| d.title.clone()).unwrap_or_else(|| format!("Pull request #{number}")).into();
        let pill = d.map(|d| {
            let (label, color) = status_style(d, t);
            page::badge_tone(label, color).flex_none()
        });
        let title_row = div()
            .flex()
            .items_start()
            .gap(px(space::SM))
            .when(!self.compact, |el| {
                el.child(ui::icon_button("pr-back", icons::ARROW_LEFT, "All pull requests", t, {
                    let weak = cx.weak_entity();
                    move |_, _, cx| {
                        let _ = weak.update(cx, |_, cx| cx.emit(PrViewEvent::Back));
                    }
                }))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(if self.compact { ty::BODY } else { ty::PAGE })
                            .line_height(if self.compact { ty::lh::BODY } else { ty::lh::PAGE })
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.text)
                            .child(title),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(8.0))
                            .text_size(ty::META)
                            .text_color(t.text_muted)
                            .children(pill)
                            .child(format!("#{number}"))
                            .when_some(d, |el, d| {
                                el.child(format!("{} → {}", d.branch, d.base))
                                    .when(!d.author.is_empty(), |el| el.child(format!("by {}", d.author)))
                                    .child(div().text_color(t.diff_add).child(format!("+{}", d.additions)))
                                    .child(div().text_color(t.diff_del).child(format!("−{}", d.deletions)))
                                    .child(format!(
                                        "{} file{}",
                                        d.changed_files,
                                        if d.changed_files == 1 { "" } else { "s" }
                                    ))
                            })
                            .when(self.loading, |el| el.child(kit::spinner("pr-view-loading", 11.0, t.text_faint))),
                    ),
            )
            .child(ui::icon_button("pr-view-refresh", icons::REFRESH, "Refresh", t, {
                let weak = cx.weak_entity();
                move |_, _, cx| {
                    let _ = weak.update(cx, |v, cx| v.refresh(cx));
                }
            }))
            .child(ui::icon_button("pr-view-web", icons::ARROW_UP_RIGHT, "Open on GitHub", t, move |_, _, cx| {
                if !url.is_empty() {
                    cx.open_url(&url)
                }
            }));

        let actions = d.filter(|d| d.state == "OPEN").map(|d| {
            let blockers = blockers(d);
            let hard = blockers.iter().any(|b| b.tone == BlockTone::Bad) || d.draft;
            let merge_label = self.busy.filter(|b| b.starts_with("Merg")).unwrap_or("Merge");
            let merge = ctl::button_sm(t, "pr-merge", merge_label, Tone::Solid);
            let merge = if hard || self.busy.is_some() {
                ctl::disabled(merge, blockers.first().map(|b| b.text).unwrap_or("Busy"))
            } else {
                merge.on_click(cx.listener(|v, _, _, cx| v.merge(cx)))
            };
            let methods = [MergeMethod::Squash, MergeMethod::Merge, MergeMethod::Rebase];
            let method = ctl::select("pr-merge-method", "Merge method", |v: &mut PrView| &mut v.merge_select)
                .options(
                    [
                        ctl::SelectOption::new("Squash").detail("One commit on the base branch"),
                        ctl::SelectOption::new("Merge commit").detail("Keep every commit"),
                        ctl::SelectOption::new("Rebase").detail("Replay commits on the base branch"),
                    ],
                    methods.iter().position(|m| *m == self.merge_method).unwrap_or(0),
                )
                .width(132.0)
                .menu_width(260.0)
                .on_select(move |v, i, _, cx| {
                    v.merge_method = methods[i.min(2)];
                    cx.notify();
                })
                .render(t, &self.merge_select, cx);
            let checkout = target.and_then(|t| t.project.clone()).map(|project| {
                let (number, title) = (d.number, d.title.clone());
                ctl::button_sm(t, "pr-checkout", "Check out in new thread", Tone::Filled).on_click(cx.listener(
                    move |_, _, _, cx| {
                        cx.emit(PrViewEvent::Checkout { project: project.clone(), number, title: title.clone() })
                    },
                ))
            });
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(space::SM))
                .child(merge)
                .child(method)
                .child(
                    ctl::button_sm(t, "pr-handoff-review", "Ask an agent to review", Tone::Filled)
                        .on_click(cx.listener(|v, _, _, cx| v.hand_off(HandOff::Review, cx))),
                )
                .children(checkout)
        });

        div()
            .flex()
            .flex_col()
            .gap(px(space::MD))
            .child(title_row)
            .children(actions)
            .when_some(self.note.clone(), |el, (text, ok)| {
                el.child(if ok { page::info_strip(t, text) } else { page::error_strip(t, text) })
            })
            .when_some(self.error.clone().filter(|_| self.detail.is_none()), |el, e| el.child(page::error_strip(t, e)))
            .into_any_element()
    }

    fn render_tabs(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let d = self.detail.as_ref();
        let segments = PrTab::ALL
            .iter()
            .map(|tab| {
                let n = match (tab, d) {
                    (PrTab::Changes, Some(d)) => Some(d.changed_files as usize),
                    (PrTab::Checks, Some(d)) => Some(d.checks.len()),
                    (PrTab::Activity, Some(d)) => Some(d.reviews.len() + d.comments.len()),
                    _ => None,
                };
                Segment::new(match n.filter(|n| *n > 0 && !self.compact) {
                    Some(n) => format!("{} {n}", tab.label()),
                    None => tab.label().to_string(),
                })
            })
            .collect();
        let weak = cx.weak_entity();
        ctl::segmented(
            t,
            "pr-view-tabs",
            segments,
            PrTab::ALL.iter().position(|x| *x == self.tab).unwrap_or(0),
            move |i, _, cx| {
                let _ = weak.update(cx, |v, cx| v.show_tab(PrTab::ALL[i.min(3)], cx));
            },
        )
    }

    // ---- Walkthrough ----

    fn render_walkthrough(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(d) = self.detail.as_ref() else {
            return page::card(t).child(div().p(px(space::LG)).child(page::skeleton_lines(t, 4))).into_any_element();
        };
        let (status, color) = status_style(d, t);
        let review = match d.review_decision.as_str() {
            "APPROVED" => "Approved",
            "CHANGES_REQUESTED" => "Changes requested",
            "REVIEW_REQUIRED" => "Review required",
            _ => "No review yet",
        };
        let checks = match d.checks_summary() {
            "passing" => "All passing",
            "failing" => "Failing",
            "pending" => "Running",
            _ => "None",
        };
        let fact = |label: &'static str, value: SharedString, tint: Option<Hsla>| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(space::MD))
                .py(px(6.0))
                .text_size(ty::BODY)
                .child(div().text_color(t.text_muted).child(label))
                .child(div().min_w_0().truncate().text_color(tint.unwrap_or(t.text)).child(value))
        };
        let overview = page::card(t).child(
            div()
                .px(px(space::LG))
                .py(px(space::SM))
                .child(fact("Status", status.into(), Some(color)))
                .child(fact("Review", review.into(), None))
                .child(fact("Checks", checks.into(), None))
                .child(fact("Branch", format!("{} → {}", d.branch, d.base).into(), None))
                .child(fact(
                    "Size",
                    format!("{} files · +{} −{}", d.changed_files, d.additions, d.deletions).into(),
                    None,
                ))
                .when(!d.created_at.is_empty(), |el| el.child(fact("Opened", ago(&d.created_at).into(), None))),
        );

        let blockers = blockers(d);
        let assessment = page::card(t).child(
            div().px(px(space::LG)).py(px(space::MD)).flex().flex_col().gap(px(space::SM)).map(|el| {
                if d.state == "MERGED" {
                    el.child(assess_line(t, icons::CHECK, t.accent, "This pull request has been merged."))
                } else if d.state == "CLOSED" {
                    el.child(assess_line(
                        t,
                        icons::CLOSE_CIRCLE,
                        t.text_muted,
                        "This pull request was closed without merging.",
                    ))
                } else if blockers.is_empty() {
                    el.child(assess_line(
                        t,
                        icons::CHECK,
                        t.success,
                        "Ready to merge: checks pass and nothing is blocking it.",
                    ))
                } else {
                    el.children(blockers.iter().enumerate().map(|(i, b)| {
                        let (glyph, tint) = match b.tone {
                            BlockTone::Bad => (icons::DANGER_TRIANGLE, t.danger),
                            BlockTone::Wait => (icons::CLOCK_CIRCLE, t.warning),
                        };
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::SM))
                            .child(assess_line(t, glyph, tint, b.text).flex_1())
                            .when_some(b.hand_off, |el, h| {
                                el.child(
                                    ctl::button_sm(
                                        t,
                                        SharedString::from(format!("pr-assess-{i}")),
                                        h.label(),
                                        Tone::Quiet,
                                    )
                                    .on_click(cx.listener(move |v, _, _, cx| v.hand_off(h, cx))),
                                )
                            })
                    }))
                }
            }),
        );

        div()
            .flex()
            .flex_col()
            .child(page::section(t, "Where it stands", assessment))
            .child(self.render_explained(t, cx))
            .child(page::section(t, "Overview", overview))
            .into_any_element()
    }

    /// The plain-language part of the walkthrough: one sentence, what
    /// changed by capability, how to check it, things to look at, and risk.
    fn render_explained(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(w) = self.walkthrough.as_ref() else {
            let label = if self.patch.is_none() { "Reading the changes…" } else { "Explaining this pull request…" };
            return page::section(
                t,
                "What this does",
                page::card(t).child(
                    div()
                        .px(px(space::LG))
                        .py(px(space::MD))
                        .flex()
                        .flex_col()
                        .gap(px(space::SM))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::SM))
                                .text_size(ty::META)
                                .text_color(t.text_muted)
                                .child(kit::spinner("pr-walk-spin", 11.0, t.text_muted))
                                .child(label),
                        )
                        .child(page::skeleton_lines(t, 3)),
                ),
            )
            .into_any_element();
        };
        let para = |text: String| div().text_size(ty::BODY).line_height(ty::lh::BODY).text_color(t.text).child(text);
        let summary = page::card(t).child(
            div()
                .px(px(space::LG))
                .py(px(space::MD))
                .flex()
                .flex_col()
                .gap(px(space::SM))
                .child(
                    div()
                        .text_size(if self.compact { ty::BODY } else { ty::PAGE })
                        .line_height(if self.compact { ty::lh::BODY } else { ty::lh::PAGE })
                        .text_color(t.text)
                        .child(w.one_liner.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::SM))
                        .text_size(ty::META)
                        .text_color(t.text_faint)
                        .child(icon(if w.ai { icons::MAGIC_STICK_3 } else { icons::LIST }).size(px(12.0)))
                        .child(if w.ai { "Written by an agent from the diff" } else { "Summarised from the file list" })
                        .child(div().flex_1())
                        .child(
                            ctl::button_sm(
                                t,
                                "pr-walk-again",
                                if self.walking.is_some() { "Writing…" } else { "Regenerate" },
                                Tone::Quiet,
                            )
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.walkthrough = None;
                                v.walk(true, cx);
                            })),
                        ),
                ),
        );

        let groups = (!w.groups.is_empty()).then(|| {
            page::card(t).children(w.groups.iter().enumerate().map(|(gi, g)| {
                div()
                    .when(gi > 0, |el| el.border_t_1().border_color(page::row_divider(t)))
                    .px(px(space::LG))
                    .py(px(space::MD))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_size(ty::BODY)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.text)
                            .child(g.title.clone()),
                    )
                    .when(!g.summary.is_empty(), |el| el.child(para(g.summary.clone()).text_color(t.text_muted)))
                    .when(!g.files.is_empty(), |el| {
                        el.child(div().flex().flex_wrap().gap(px(4.0)).children(
                            g.files.iter().take(12).enumerate().map(|(fi, path)| {
                                let p = path.clone();
                                let name = path.rsplit('/').next().unwrap_or(path).to_string();
                                div()
                                    .id(SharedString::from(format!("pr-walk-file-{gi}-{fi}")))
                                    .h(px(22.0))
                                    .px(px(8.0))
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .rounded(px(6.0))
                                    .bg(t.wash(0.05))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(t.wash(0.09)))
                                    .text_size(ty::META)
                                    .text_color(t.text_muted)
                                    .tooltip(ui::tip(path.clone()))
                                    .on_click(cx.listener(move |v, _, _, cx| v.reveal_file(&p, cx)))
                                    .child(icon(icons::DOCUMENT).size(px(11.0)))
                                    .child(name)
                            }),
                        ))
                    })
            }))
        });

        let checks = (!w.how_to_check.is_empty()).then(|| {
            page::card(t).child(div().px(px(space::LG)).py(px(space::MD)).flex().flex_col().gap(px(6.0)).children(
                w.how_to_check.iter().enumerate().map(|(i, step)| {
                    div()
                        .flex()
                        .gap(px(space::SM))
                        .child(
                            div()
                                .size(px(18.0))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(t.wash(0.07))
                                .text_size(ty::MICRO)
                                .text_color(t.text_muted)
                                .child((i + 1).to_string()),
                        )
                        .child(para(step.clone()).flex_1())
                }),
            ))
        });

        let findings = (!w.findings.is_empty()).then(|| {
            page::card(t).children(w.findings.iter().enumerate().map(|(i, f)| {
                let (sev, sev_color) = match f.severity {
                    Severity::High => ("High", t.danger),
                    Severity::Medium => ("Medium", t.warning),
                    Severity::Low => ("Low", t.text_muted),
                };
                let category = match f.category {
                    Category::Bug => "Bug",
                    Category::Risk => "Risk",
                    Category::Maintainability => "Maintainability",
                    Category::Refactor => "Refactor",
                };
                let confidence = match f.confidence {
                    Confidence::Confirmed => "Confirmed",
                    Confidence::Investigate => "Worth checking",
                };
                let at = f.file.clone().map(|file| {
                    let label = match f.line {
                        Some(l) => format!("{}:{l}", file.rsplit('/').next().unwrap_or(&file)),
                        None => file.rsplit('/').next().unwrap_or(&file).to_string(),
                    };
                    div()
                        .id(SharedString::from(format!("pr-finding-at-{i}")))
                        .text_size(ty::META)
                        .text_color(t.accent)
                        .cursor_pointer()
                        .tooltip(ui::tip(file.clone()))
                        .on_click(cx.listener(move |v, _, _, cx| v.reveal_file(&file, cx)))
                        .child(label)
                });
                div()
                    .when(i > 0, |el| el.border_t_1().border_color(page::row_divider(t)))
                    .px(px(space::LG))
                    .py(px(space::MD))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(6.0))
                            .child(page::badge_tone(sev, sev_color))
                            .child(page::badge(t, category))
                            .child(div().text_size(ty::META).text_color(t.text_faint).child(confidence))
                            .children(at),
                    )
                    .child(
                        div()
                            .text_size(ty::BODY)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.text)
                            .child(f.title.clone()),
                    )
                    .when(!f.detail.is_empty(), |el| el.child(para(f.detail.clone()).text_color(t.text_muted)))
            }))
        });

        let (risk, risk_color) = match w.risk.level {
            Level::Low => ("Low risk", t.success),
            Level::Medium => ("Medium risk", t.warning),
            Level::High => ("High risk", t.danger),
        };
        let risk = page::card(t).child(
            div()
                .px(px(space::LG))
                .py(px(space::MD))
                .flex()
                .items_start()
                .gap(px(space::SM))
                .child(page::badge_tone(risk, risk_color).flex_none())
                .when(!w.risk.reason.is_empty(), |el| {
                    el.child(para(w.risk.reason.clone()).flex_1().text_color(t.text_muted))
                }),
        );

        div()
            .flex()
            .flex_col()
            .child(page::section(t, "What this does", summary))
            .when_some(groups, |el, g| el.child(page::section(t, "What changed", g)))
            .when_some(checks, |el, c| el.child(page::section(t, "How to check it", c)))
            .when_some(findings, |el, f| el.child(page::section(t, "Things to look at", f)))
            .child(page::section(t, "Size and risk", risk))
            .into_any_element()
    }

    // ---- Changes ----

    fn render_changes(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(files) = self.files.clone() else {
            return page::card(t).child(div().p(px(space::LG)).child(page::skeleton_lines(t, 6))).into_any_element();
        };
        if files.is_empty() {
            return page::card(t)
                .child(page::empty_state(t, icons::DOCUMENT, "No changes", "This pull request changes no files.", None))
                .into_any_element();
        }
        // The list virtualizes, so it needs a definite height: most of the
        // window, leaving the header in view.
        let height = (f32::from(window.viewport_size().height) - if self.compact { 260.0 } else { 300.0 }).max(360.0);
        let toolbar = div()
            .flex()
            .items_center()
            .justify_between()
            .mb(px(space::SM))
            .text_size(ty::META)
            .text_color(t.text_muted)
            .child(format!("{} file{}", files.len(), if files.len() == 1 { "" } else { "s" }))
            .child(
                div().flex().items_center().gap(px(2.0)).children(crate::diff_list::view_toggles(&self.diff, t, cx)),
            );
        div()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(page::card(t).h(px(height)).flex().flex_col().overflow_hidden().child(self.diff.clone()))
            .into_any_element()
    }

    // ---- Checks ----

    fn render_checks(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(d) = self.detail.as_ref() else {
            return page::card(t).child(div().p(px(space::LG)).child(page::skeleton_lines(t, 3))).into_any_element();
        };
        if d.checks.is_empty() {
            return page::card(t)
                .child(page::empty_state(
                    t,
                    icons::QUEUE_CHECK,
                    "No checks",
                    "Nothing runs on this pull request.",
                    None,
                ))
                .into_any_element();
        }
        let group = |bucket: Bucket| -> Vec<&Check> { d.checks.iter().filter(|c| check_bucket(c) == bucket).collect() };
        let mut out = div().flex().flex_col();
        for (bucket, title, color) in [
            (Bucket::Failed, "Failing", t.danger),
            (Bucket::Pending, "Running", t.warning),
            (Bucket::Passed, "Passing", t.success),
        ] {
            let checks = group(bucket);
            if checks.is_empty() {
                continue;
            }
            let label = format!("{title} · {}", checks.len());
            let card = page::card(t).children(checks.into_iter().enumerate().map(|(i, c)| {
                let link = c.url.clone();
                page::row_hover(
                    t,
                    page::table_row(t, i == 0).id(SharedString::from(format!("pr-check-{title}-{i}"))),
                    false,
                )
                .text_size(ty::BODY)
                .on_click(move |_, _, cx| {
                    if link.starts_with("https://") {
                        cx.open_url(&link);
                    }
                })
                .child(ui::dot(color, 7.0))
                .child(div().flex_1().min_w_0().truncate().text_color(t.text).child(c.name.clone()))
                .child(
                    div()
                        .flex_none()
                        .text_size(ty::META)
                        .text_color(t.text_faint)
                        .child(c.state.to_lowercase().replace('_', " ")),
                )
            }));
            let card = if bucket == Bucket::Failed && d.state == "OPEN" {
                card.child(
                    div().px(px(space::LG)).py(px(space::SM)).border_t_1().border_color(page::row_divider(t)).child(
                        ctl::button_sm(t, "pr-fix-checks", HandOff::FixChecks.label(), Tone::Filled)
                            .on_click(cx.listener(|v, _, _, cx| v.hand_off(HandOff::FixChecks, cx))),
                    ),
                )
            } else {
                card
            };
            out = out.child(page::section(t, label, card));
        }
        out.into_any_element()
    }

    // ---- Activity ----

    fn render_activity(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(d) = self.detail.as_ref() else {
            return page::card(t).child(div().p(px(space::LG)).child(page::skeleton_lines(t, 4))).into_any_element();
        };
        let description = page::card(t).child(
            div()
                .px(px(space::LG))
                .py(px(space::MD))
                .children(self.body.as_ref().map(|b| crate::markdown::render("pr-view-body", b, t, window))),
        );

        enum Entry<'a> {
            Review(&'a github::Review),
            Comment(&'a github::Comment),
            Commit(&'a github::Commit),
        }
        let mut entries: Vec<(&str, Entry)> = Vec::new();
        entries.extend(d.reviews.iter().map(|r| (r.at.as_str(), Entry::Review(r))));
        entries.extend(d.comments.iter().map(|c| (c.at.as_str(), Entry::Comment(c))));
        entries.extend(d.commits.iter().map(|c| (c.at.as_str(), Entry::Commit(c))));
        entries.sort_by(|a, b| a.0.cmp(b.0));

        let timeline = page::card(t).children(entries.into_iter().enumerate().map(|(i, (at, e))| {
            let (glyph, tint, who, what, body): (&'static str, Hsla, String, String, Option<String>) = match e {
                Entry::Review(r) => {
                    let (what, tint) = match r.state.as_str() {
                        "APPROVED" => ("approved", t.success),
                        "CHANGES_REQUESTED" => ("asked for changes", t.danger),
                        "DISMISSED" => ("review dismissed", t.text_faint),
                        _ => ("reviewed", t.text_muted),
                    };
                    (
                        icons::EYE,
                        tint,
                        r.author.clone(),
                        what.into(),
                        Some(r.body.clone()).filter(|b| !b.trim().is_empty()),
                    )
                }
                Entry::Comment(c) => {
                    (icons::CHAT_ROUND_LINE, t.text_muted, c.author.clone(), "commented".into(), Some(c.body.clone()))
                }
                Entry::Commit(c) => (icons::GIT_BRANCH, t.text_faint, c.author.clone(), c.headline.clone(), None),
            };
            div()
                .when(i > 0, |el| el.border_t_1().border_color(page::row_divider(t)))
                .px(px(space::LG))
                .py(px(10.0))
                .flex()
                .gap(px(space::SM))
                .child(icon(glyph).size(px(14.0)).flex_none().mt(px(2.0)).text_color(tint))
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
                                .gap(px(6.0))
                                .text_size(ty::BODY)
                                .child(
                                    div()
                                        .flex_none()
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(t.text)
                                        .child(who),
                                )
                                .child(div().min_w_0().truncate().text_color(t.text_muted).child(what))
                                .child(div().flex_none().text_size(ty::META).text_color(t.text_faint).child(ago(at))),
                        )
                        .when_some(body, |el, b| {
                            el.child(div().text_size(ty::BODY).text_color(t.text).child(clip(&b, 1200)))
                        }),
                )
        }));

        let open = d.state == "OPEN";
        let reply = open.then(|| {
            div()
                .mt(px(space::MD))
                .flex()
                .flex_col()
                .gap(px(space::SM))
                .child(ctl::text_field(&self.reply, None, false, t))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(space::SM))
                        .child(
                            ctl::button_sm(t, "pr-comment", "Comment", Tone::Filled)
                                .on_click(cx.listener(|v, _, w, cx| v.review(ReviewKind::Comment, w, cx))),
                        )
                        .child(
                            ctl::button_sm(t, "pr-request-changes", "Request changes", Tone::Quiet)
                                .on_click(cx.listener(|v, _, w, cx| v.review(ReviewKind::RequestChanges, w, cx))),
                        )
                        .child(
                            ctl::button_sm(t, "pr-approve", "Approve", Tone::Filled)
                                .on_click(cx.listener(|v, _, w, cx| v.review(ReviewKind::Approve, w, cx))),
                        ),
                )
        });

        div()
            .flex()
            .flex_col()
            .child(page::section(t, "Description", description))
            .child(page::section(
                t,
                "Conversation",
                timeline.children(reply.map(|r| div().px(px(space::LG)).pb(px(space::MD)).child(r))),
            ))
            .into_any_element()
    }
}

/// `s` cut to `max` characters, with an ellipsis when cut.
fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", s[..i].trim_end()),
        None => s.to_string(),
    }
}

fn assess_line(t: &Theme, glyph: &'static str, tint: Hsla, text: &'static str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(space::SM))
        .text_size(ty::BODY)
        .text_color(t.text)
        .child(icon(glyph).size(px(14.0)).flex_none().text_color(tint))
        .child(text)
}

impl Render for PrView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("PrView");
        self.last_render = Instant::now();
        let t = Theme::of(cx).clone();
        if self.target.is_none() {
            return page::card(&t)
                .child(page::empty_state(&t, icons::PULL_REQUEST, "No pull request", "Nothing to show yet.", None))
                .into_any_element();
        }
        // The walkthrough is written when someone first looks at it.
        if self.tab == PrTab::Walkthrough
            && self.walkthrough.is_none()
            && self.walking.is_none()
            && self.patch.is_some()
        {
            self.walk(false, cx);
        }
        let header = self.render_header(&t, cx);
        let tabs = self.render_tabs(&t, cx);
        let body = match self.tab {
            PrTab::Walkthrough => self.render_walkthrough(&t, cx),
            PrTab::Changes => self.render_changes(&t, window, cx),
            PrTab::Checks => self.render_checks(&t, cx),
            PrTab::Activity => self.render_activity(&t, window, cx),
        };
        div()
            .flex()
            .flex_col()
            .child(header)
            .child(div().mt(px(space::LG)).child(tabs))
            .child(div().mt(px(space::SM)).child(body))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(state: &str) -> PrDetail {
        PrDetail {
            number: 4,
            title: "T".into(),
            state: state.into(),
            branch: "b".into(),
            base: "main".into(),
            ..Default::default()
        }
    }

    #[test]
    fn iso_times_parse() {
        assert_eq!(iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_ms("2000-03-01T00:00:01Z"), Some(951_868_801_000));
        assert_eq!(iso_ms("soon"), None);
    }

    #[test]
    fn blockers_rank_and_hand_off() {
        let mut d = pr("OPEN");
        assert!(blockers(&d).is_empty());
        d.mergeable = "CONFLICTING".into();
        d.review_decision = "CHANGES_REQUESTED".into();
        d.checks = vec![
            Check { name: "ci".into(), state: "FAILURE".into(), url: String::new() },
            Check { name: "lint".into(), state: "PENDING".into(), url: String::new() },
        ];
        let b = blockers(&d);
        assert_eq!(b[0].hand_off, Some(HandOff::ResolveConflicts));
        assert_eq!(b[1].hand_off, Some(HandOff::FixChecks));
        assert_eq!(b[2].hand_off, Some(HandOff::AddressFeedback));
        assert!(b.iter().any(|x| x.text == "Checks are still running"));
        assert!(HandOff::FixChecks.prompt(&d).contains("ci"));
        assert!(blockers(&pr("MERGED")).is_empty());
    }
}
