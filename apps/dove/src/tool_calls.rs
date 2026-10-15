//! Tool-call chrome, ported from Zeron's transcript (MIT): the collapsible
//! activity group with its shimmering summary, the task-tree rail (trunk,
//! rounded elbow, tool glyph at each branch tip), expandable chip rows and
//! their fold tweens.
//!
//! What changed in the port:
//! - Heights are measured, not analytic: each folding body is probed in
//!   prepaint and the tween clips to the measured height, so invocations and
//!   output wrap naturally at any column width.
//! - Reasoning renders exactly as Zeron renders it: streamed through
//!   `zeron_markdown::IncrementalParser` (Zeron's "streamdown", mended while
//!   live) and flattened into styled detail lines by Zeron's own code below.
//! - The shimmer shapes its line once per frame and paints a handful of
//!   quantized bands, instead of one shaped line per 2px strip.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Bounds, ContentMask, EntityId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    PathBuilder, Pixels, Point, SharedString, StatefulInteractiveElement as _, Styled as _, TextAlign, TextRun, Window,
    canvas, div, point, px, size,
};

use zeron_markdown::{Block as MdBlock, BlockTree, InlineRun, InlineStyle};

use crate::assets::{icon, icons};
use crate::file_icons::{self, FileIconIdentity};
use crate::markdown;
use crate::theme::{self, Theme};

// ------------------------------------------------------------- geometry

/// Chip label / detail type size (Zeron's `TOOL_LABEL_SIZE`).
pub const LABEL_SIZE: f32 = 12.0;
pub const LINE_HEIGHT: f32 = 18.0;
/// The group's summary header.
pub const HEADER_HEIGHT: f32 = 26.0;
/// One collapsed row on the rail.
pub const ROW_HEIGHT: f32 = 32.0;
/// The header strip inside a row.
pub const CARD_HEIGHT: f32 = 30.0;
/// Rail gutter: trunk, a 6px elbow, a 4px break and the 16px glyph.
pub const GUTTER: f32 = 48.0;
pub const TEXT_GAP: f32 = 8.0;
pub const TRUNK_X: f32 = 12.5;
const BEND_RADIUS: f32 = 6.0;
const BRANCH_END_X: f32 = 28.0;
const ICON_LEFT: f32 = 32.0;
const ICON_SIZE: f32 = 16.0;

// ---------------------------------------------------------------- motion

pub const FOLD: Duration = Duration::from_millis(140);
pub const ROW_REVEAL: Duration = Duration::from_millis(360);
pub const CONNECTOR_REVEAL: Duration = Duration::from_millis(480);
const SHIMMER_PERIOD: Duration = Duration::from_millis(3_400);
const SHIMMER_HALF_WIDTH: f32 = 0.36;
/// Shimmer bands: wide enough to stay cheap, narrow enough to read smooth.
const SHIMMER_STRIP: f32 = 3.0;
/// Shimmer brightness steps; the strips at one step share one shaped line.
const SHIMMER_LEVELS: f32 = 10.0;

/// A CSS `cubic-bezier` timing curve.
#[derive(Clone, Copy)]
pub struct Bezier(f32, f32, f32, f32);

/// CSS `ease-out`: folds.
pub const EASE_OUT: Bezier = Bezier(0.0, 0.0, 0.58, 1.0);
/// `easeOutExpo`: a new row's height.
pub const EASE_OUT_EXPO: Bezier = Bezier(0.16, 1.0, 0.3, 1.0);
/// `easeOutQuint`: the connector drawing in.
pub const EASE_OUT_QUINT: Bezier = Bezier(0.22, 1.0, 0.36, 1.0);

impl Bezier {
    pub fn eval(self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        if x <= 0.0 || x >= 1.0 {
            return x;
        }
        let Bezier(x1, y1, x2, y2) = self;
        let curve = |a: f32, b: f32, t: f32| {
            let u = 1.0 - t;
            3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
        };
        // Solve x(t) = x by bisection: monotone in t, 24 steps ≈ 1e-7.
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = (lo + hi) * 0.5;
            if curve(x1, x2, mid) < x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        curve(y1, y2, (lo + hi) * 0.5)
    }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Progress of a timed transition that started at `start`.
pub fn progress(start: Option<Instant>, now: Instant, span: Duration, curve: Bezier) -> f32 {
    match start {
        Some(s) => curve.eval(now.saturating_duration_since(s).as_secs_f32() / span.as_secs_f32()),
        None => 1.0,
    }
}

/// Split one arrival into a continuous tree draw: a row with a predecessor
/// waits for the predecessor's continuation to reach its top edge, then
/// draws its incoming trunk, then the elbow (overlapping a little so the
/// bend has no dead frame). Returns `(incoming, branch)`.
pub fn connector_parts(progress: f32, has_predecessor: bool) -> (f32, f32) {
    let progress = progress.clamp(0.0, 1.0);
    let (start, end, branch_start) = if has_predecessor { (0.45, 0.72, 0.68) } else { (0.0, 0.62, 0.58) };
    let incoming = ((progress - start) / (end - start)).clamp(0.0, 1.0);
    let branch = ((progress - branch_start) / (1.0 - branch_start)).clamp(0.0, 1.0);
    (incoming, branch)
}

/// The trunk below a row belongs to that row but is timed by the next
/// row's arrival.
pub fn continuation(next_progress: Option<f32>) -> f32 {
    next_progress.map(|p| (p / 0.45).clamp(0.0, 1.0)).unwrap_or(1.0)
}

// ----------------------------------------------------------------- folds

/// Where a fold lives: `(row, part, index)`.
pub type FoldKey = (usize, u8, usize);
pub const PART_GROUP: u8 = 0;
pub const PART_TOOL: u8 = 1;
pub const PART_THOUGHT: u8 = 2;

#[derive(Default)]
struct Fold {
    shown: Option<bool>,
    at: Option<Instant>,
}

/// What a fold looks like this frame.
pub struct FoldFrame {
    /// Whether the body is built at all.
    pub mounted: bool,
    pub animating: bool,
    /// 0 = closed, 1 = open, for the chevron's rotation.
    pub disclosure: f32,
    /// The body's entrance (0 → 1): it fades in and settles 3 px. The height
    /// is final from the first frame, so nothing below reflows per frame.
    pub reveal: f32,
}

/// Fold tween state for one transcript view (render-side, so `&self`
/// renders can step it).
#[derive(Default)]
pub struct Folds(RefCell<HashMap<FoldKey, Fold>>);

impl Folds {
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    /// Step the fold at `key` toward `open`. Opening lays the body out at
    /// its full height at once and fades it in; closing removes it at once.
    /// Only the chevron turns over the transition, so a fold never animates
    /// layout. Reduced motion snaps both.
    pub fn step(&self, key: FoldKey, open: bool, now: Instant) -> FoldFrame {
        let mut map = self.0.borrow_mut();
        let f = map.entry(key).or_default();
        if f.shown.is_some_and(|was| was != open) {
            f.at = (!crate::motion::reduced_now()).then_some(now);
        }
        f.shown = Some(open);
        let mut frame =
            FoldFrame { mounted: open, animating: false, disclosure: if open { 1.0 } else { 0.0 }, reveal: 1.0 };
        if let Some(at) = f.at {
            let raw = now.saturating_duration_since(at).as_secs_f32() / FOLD.as_secs_f32();
            if raw < 1.0 {
                let e = EASE_OUT.eval(raw);
                frame.animating = true;
                frame.disclosure = if open { e } else { 1.0 - e };
                if open {
                    frame.reveal = e;
                }
            } else {
                f.at = None;
            }
        }
        frame
    }
}

/// The fold's body at its natural height, fading and settling in while it
/// opens. Callers build it only while the fold is mounted (open).
pub fn folding(child: impl IntoElement, frame: &FoldFrame) -> AnyElement {
    let body = div().w_full().flex_none().child(child);
    if frame.reveal < 1.0 {
        body.relative().opacity(frame.reveal).top(px(-3.0 * (1.0 - frame.reveal))).into_any_element()
    } else {
        body.into_any_element()
    }
}

/// A new arrival's entrance: it takes its full height at once, then fades
/// in and settles 4 px (opacity and offset only, so rows below it move
/// once, not every frame).
pub fn arriving(child: impl IntoElement, progress: f32) -> AnyElement {
    if progress >= 1.0 {
        return div().w_full().child(child).into_any_element();
    }
    div().relative().w_full().opacity(progress).top(px(-4.0 * (1.0 - progress))).child(child).into_any_element()
}

/// Keeps frames coming while something on screen is moving.
pub fn frame_pump(view: EntityId) -> AnyElement {
    canvas(|_, _, _| (), move |_, _, window, _| window.on_next_frame(move |_, cx| cx.notify(view)))
        .absolute()
        .inset_0()
        .into_any_element()
}

// ------------------------------------------------------------------ rail

/// What sits at a branch tip: the tool's glyph, in its tint.
pub enum Glyph {
    Icon(&'static str, Hsla),
}

/// The task-tree gutter for one row: incoming trunk, rounded elbow, glyph.
/// One paint owns every segment of the row (nonzero fill unions the
/// ribbons), so joints never double their alpha on glass.
pub fn rail(
    glyph: Glyph,
    has_predecessor: bool,
    continues: bool,
    reveal: f32,
    continuation_reveal: f32,
    t: &Theme,
) -> gpui::Div {
    let color = theme::ink(if t.appearance.is_dark() { 0.12 } else { 0.14 });
    let (incoming, branch) = connector_parts(reveal, has_predecessor);
    let head = ROW_HEIGHT;
    let tip = div()
        .absolute()
        .left(px(ICON_LEFT))
        .top(px(head / 2.0 - ICON_SIZE / 2.0))
        .size(px(ICON_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .opacity(branch);
    let Glyph::Icon(path, tint) = glyph;
    let tip = tip.child(icon(path).size(px(ICON_SIZE)).text_color(tint));
    div()
        .relative()
        .w(px(GUTTER))
        .flex_none()
        .self_stretch()
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let x = bounds.origin.x + px(TRUNK_X);
                    let branch_y = bounds.origin.y + px(head / 2.0);
                    let bend_y = branch_y - px(BEND_RADIUS);
                    let mut tree = PathBuilder::fill().with_style(gpui::PathStyle::Fill(
                        gpui::FillOptions::default().with_fill_rule(gpui::FillRule::NonZero),
                    ));
                    if incoming > 0.0 {
                        let mut bottom = point(x, bounds.origin.y + px((head / 2.0 - BEND_RADIUS) * incoming));
                        if incoming >= 1.0 && continues && continuation_reveal > 0.0 {
                            let rest = (f32::from(bounds.size.height) - (head / 2.0 - BEND_RADIUS)).max(0.0);
                            bottom = point(x, bend_y + px(rest * continuation_reveal));
                        }
                        ribbon(&mut tree, &[point(x, bounds.origin.y), bottom]);
                    }
                    if branch > 0.0 {
                        let pts: Vec<_> =
                            branch_points(branch).into_iter().map(|p| point(x + px(p.x), bend_y + px(p.y))).collect();
                        ribbon(&mut tree, &pts);
                    }
                    if let Ok(path) = tree.build() {
                        window.paint_path(path, color);
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
        .child(tip)
}

/// The elbow (a quadratic quarter-bend) then the straight leg, revealed by
/// distance along the path so the junction has no speed jump.
fn branch_points(progress: f32) -> Vec<Point<f32>> {
    let mut path: Vec<Point<f32>> = (0..=24)
        .map(|step| {
            let s = step as f32 / 24.0;
            point(BEND_RADIUS * s * s, BEND_RADIUS * (2.0 * s - s * s))
        })
        .collect();
    path.push(point(BRANCH_END_X - TRUNK_X, BEND_RADIUS));
    if progress >= 1.0 {
        return path;
    }
    let lengths: Vec<f32> = path.windows(2).map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y)).collect();
    let mut remaining = lengths.iter().sum::<f32>() * progress.clamp(0.0, 1.0);
    let mut visible = vec![path[0]];
    for (w, len) in path.windows(2).zip(lengths) {
        if remaining <= 0.0 {
            break;
        }
        let k = (remaining / len).min(1.0);
        visible.push(point(lerp(w[0].x, w[1].x, k), lerp(w[0].y, w[1].y, k)));
        remaining -= len;
    }
    visible
}

/// A 1px ribbon around `points`, wound clockwise.
fn ribbon(path: &mut PathBuilder, points: &[Point<Pixels>]) {
    if points.len() < 2 {
        return;
    }
    let mut left = Vec::with_capacity(points.len());
    let mut right = Vec::with_capacity(points.len());
    for (ix, p) in points.iter().enumerate() {
        let a = points[ix.saturating_sub(1)];
        let b = points[(ix + 1).min(points.len() - 1)];
        let dx = f32::from(b.x - a.x);
        let dy = f32::from(b.y - a.y);
        let len = dx.hypot(dy).max(0.0001);
        let normal = point(px(-dy / len * 0.5), px(dx / len * 0.5));
        left.push(*p + normal);
        right.push(*p - normal);
    }
    path.move_to(left[0]);
    for p in left.iter().skip(1).chain(right.iter().rev()) {
        path.line_to(*p);
    }
    path.close();
}

// ----------------------------------------------------------------- title

pub fn shimmer_phase(start: Instant, now: Instant) -> f32 {
    (now.saturating_duration_since(start).as_secs_f32() / SHIMMER_PERIOD.as_secs_f32()).fract()
}

/// BoardUI's sweep: a 300%-wide repeating gradient travelling 200% → -100%,
/// sampled at `x` (0..1 across the title).
fn shimmer_amount(x: f32, phase: f32) -> f32 {
    let center = -2.5 + phase.clamp(0.0, 1.0) * 6.0;
    (-2..=2)
        .map(|copy| center + copy as f32 * 3.0)
        .map(|c| (1.0 - (x - c).abs() / SHIMMER_HALF_WIDTH).clamp(0.0, 1.0))
        .fold(0.0, f32::max)
}

/// The group summary. While the group works, a light sweeps across it: the
/// intact line repaints through narrow moving clips (`background-clip:
/// text`, natively), so shaping and kerning never split per glyph.
pub fn title(text: SharedString, shimmer: Option<f32>, t: &Theme) -> AnyElement {
    let Some(phase) = shimmer else {
        return div().min_w_0().truncate().child(text).into_any_element();
    };
    let overlay_text = text.clone();
    let font = gpui::font(t.font_sans.clone());
    let (base, peak) = (t.text_muted, t.text);
    let overlay = canvas(
        move |bounds, window, _| {
            let shape = |color: Hsla, window: &mut Window| {
                window.text_system().shape_line(
                    overlay_text.clone(),
                    px(LABEL_SIZE),
                    &[TextRun {
                        len: overlay_text.len(),
                        font: font.clone(),
                        color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                )
            };
            let width = f32::from(shape(peak, window).width()).min(f32::from(bounds.size.width));
            let strips = (width / SHIMMER_STRIP).ceil() as usize;
            let mut levels: HashMap<u32, (gpui::ShapedLine, Vec<(f32, f32)>)> = HashMap::new();
            for ix in 0..strips {
                let left = ix as f32 * SHIMMER_STRIP;
                let right = (left + SHIMMER_STRIP).min(width);
                let amount = shimmer_amount((left + right) * 0.5 / width.max(1.0), phase);
                let level = (amount * SHIMMER_LEVELS).round() as u32;
                if level == 0 {
                    continue;
                }
                levels
                    .entry(level)
                    .or_insert_with(|| {
                        let k = level as f32 / SHIMMER_LEVELS;
                        (shape(mix(base, peak, k), window), Vec::new())
                    })
                    .1
                    .push((left, right));
            }
            levels.into_values().collect::<Vec<_>>()
        },
        move |bounds, levels, window, cx| {
            for (line, spans) in levels {
                for (left, right) in spans {
                    let mask = ContentMask {
                        bounds: Bounds {
                            origin: point(bounds.origin.x + px(left), bounds.origin.y),
                            size: size(px(right - left), bounds.size.height),
                        },
                    };
                    window.with_content_mask(Some(mask), |window| {
                        let _ = line.paint(bounds.origin, px(LINE_HEIGHT), TextAlign::Left, None, window, cx);
                    });
                }
            }
        },
    )
    .absolute()
    .inset_0();
    div()
        .relative()
        .h(px(LINE_HEIGHT))
        .min_w_0()
        .flex_1()
        .overflow_hidden()
        .child(div().truncate().child(text))
        .child(overlay)
        .into_any_element()
}

fn mix(a: Hsla, b: Hsla, k: f32) -> Hsla {
    let (a, b) = (gpui::Rgba::from(a), gpui::Rgba::from(b));
    gpui::Rgba { r: lerp(a.r, b.r, k), g: lerp(a.g, b.g, k), b: lerp(a.b, b.b, k), a: lerp(a.a, b.a, k) }.into()
}

/// The group's disclosure chevron, rotated by `disclosure` (0 = closed).
pub fn chevron(disclosure: f32, t: &Theme) -> gpui::Div {
    div().w(px(22.0)).h(px(18.0)).flex_none().relative().child(
        icon(icons::ALT_ARROW_DOWN)
            .absolute()
            .left(px(TRUNK_X - 7.0))
            .top(px(2.0))
            .size(px(14.0))
            .with_transformation(gpui::Transformation::rotate(gpui::radians(
                -std::f32::consts::FRAC_PI_2 * (1.0 - disclosure),
            )))
            .text_color(t.text_muted),
    )
}

// ------------------------------------------------------------- chip bits

/// Collapse model text onto one line for single-line surfaces.
pub fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The last path component; either separator (remote tools report Windows
/// paths anywhere).
pub fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).find(|c| !c.is_empty()).unwrap_or(path)
}

/// Zeron's file badge: a 22px pill with the file's glyph in a well and the
/// file name. It is its own click target (opens the file) so it never also
/// toggles the row underneath.
pub fn file_badge(id: SharedString, path: &str, failed: bool, t: &Theme) -> AnyElement {
    let name = file_name(path).to_string();
    let open = path.to_string();
    let badge = div()
        .id(id)
        .min_w_0()
        .h(px(22.0))
        .flex()
        .items_center()
        .overflow_hidden()
        .gap(px(6.0))
        .rounded(px(5.0))
        .bg(theme::ink(0.06))
        .pl(px(1.0))
        .pr(px(6.0))
        .text_color(if failed { t.danger } else { t.text.opacity(0.85) })
        .cursor_pointer()
        .hover(|s| s.bg(theme::ink(0.10)).text_color(t.text))
        .tooltip(crate::ui::tip(path.to_string()))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            let (path, line) = markdown::split_path_line(&open);
            window.dispatch_action(Box::new(crate::actions::OpenPath { path, line }), cx);
        })
        .child(
            div()
                .size(px(20.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.0))
                .bg(file_icons::well_bg(t))
                .child(file_icons::icon(FileIconIdentity::file(path), t.appearance).size(px(14.0))),
        )
        .child(div().min_w_0().truncate().child(name));
    crate::ui::frosted(5.0, 16.0, badge).into_any_element()
}

/// Mono lines (an invocation or an output), cut to `keep` lines.
///
/// `tail` keeps the end (a command's errors land last) and puts the
/// counted `more` row above; otherwise the head, with `more` below.
pub fn lines_block(
    text: &str,
    keep: usize,
    tail: bool,
    color: Hsla,
    more: impl FnOnce(usize) -> AnyElement,
    t: &Theme,
) -> AnyElement {
    let lines: Vec<&str> = text.trim_end().lines().collect();
    let cut = lines.len().saturating_sub(keep);
    let shown = if tail { &lines[cut..] } else { &lines[..lines.len() - cut] };
    let body = shown.iter().map(|l| l.replace('\t', "    ")).collect::<Vec<_>>().join("\n");
    let more = (cut > 0).then(|| more(cut));
    let (above, below) = if tail { (more, None) } else { (None, more) };
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .font_family(t.font_mono.clone())
        .text_size(px(LABEL_SIZE))
        .line_height(px(LINE_HEIGHT))
        .text_color(color)
        .children(above)
        .child(div().w_full().min_w_0().child(body))
        .children(below)
        .into_any_element()
}

/// How many lines `text` has past `keep`.
pub fn overflow(text: &str, keep: usize) -> usize {
    text.trim_end().lines().count().saturating_sub(keep)
}

/// Settled markdown (an agent's prompt or report) through the same
/// pipeline as a thought: Zeron's parser, flattened into detail lines.
pub fn prose_detail(src: &str) -> ThoughtDetail {
    thought_detail(&zeron_markdown::parse_full(src), false)
}

// ------------------------------------------------------------- streamdown
//
// Zeron's thought rendering, verbatim: the reasoning streams through
// `zeron_markdown::IncrementalParser` (its display tree is mended so half-typed
// markers never flash raw), then flattens into wrapped, STYLED one-line rows
// — inline markers become real bold/italic/code, blocks flatten structurally
// (bullets, quote bars, verbatim code lines).

/// Max detail lines per chip before the counted tail row.
pub const OUTPUT_DETAIL_MAX_LINES: usize = 24;

/// A thought's markdown, pre-flattened (Zeron's `ToolDetail::Thought`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThoughtDetail {
    pub lines: Vec<Vec<InlineRun>>,
    pub truncated_by: usize,
}

/// Zeron's `thought_item` detail: capped like tool outputs, keeping the TAIL
/// while streaming (the fresh thinking is the signal) and the head once
/// settled.
pub fn thought_detail(tree: &BlockTree, live: bool) -> ThoughtDetail {
    let mut lines = thought_lines(tree);
    let truncated_by = lines.len().saturating_sub(OUTPUT_DETAIL_MAX_LINES);
    if truncated_by > 0 {
        if live {
            lines.drain(..truncated_by);
            // The cut can land on a block separator — drop the orphan blank.
            while lines.first().is_some_and(|l| l.iter().all(|r| r.text.trim().is_empty())) {
                lines.remove(0);
            }
        } else {
            lines.truncate(OUTPUT_DETAIL_MAX_LINES);
        }
    }
    ThoughtDetail { lines, truncated_by }
}

/// Zeron's `detail_body` for a thought: one fixed 18px row per flattened
/// line, faint, with the counted tail.
pub fn thought_body(detail: &ThoughtDetail, theme: &Theme) -> AnyElement {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .overflow_hidden()
        .py(px(6.0))
        .text_size(px(LABEL_SIZE))
        .children(detail.lines.iter().map(|line| {
            let row = div().h(px(LINE_HEIGHT)).w_full().min_w_0().flex().items_center();
            let Some((text, runs)) = thought_line_text(line, theme) else {
                return row; // blank separator row
            };
            row.child(
                div()
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(gpui::StyledText::new(text).with_runs(runs)),
            )
        }))
        .when(detail.truncated_by > 0, |block| block.child(more_lines_row(detail.truncated_by, theme)))
        .into_any_element()
}

/// Column budget for soft-wrapping thought text into detail lines. The
/// detail body is preformatted (no element wrapping), so the wrap happens
/// here — conservative enough to fit the card at typical transcript widths.
const THOUGHT_WRAP_COLS: usize = 96;

/// Flatten a thought's parsed markdown into wrapped, STYLED detail lines —
/// inline markers render as real styling (bold/italic/code/links) instead of
/// literal `**` glyphs; blocks flatten structurally (headings bold, list
/// bullets, quote bars, verbatim code lines). Every line is one fixed-height
/// row, so the detail height stays analytic (lines × [`OUTPUT_LINE_HEIGHT`])
/// and the group's fold tween keeps working without measurement.
fn thought_lines(tree: &BlockTree) -> Vec<Vec<InlineRun>> {
    let mut out: Vec<Vec<InlineRun>> = Vec::new();
    for top in &tree.blocks {
        if !out.is_empty() {
            // One blank separator row between top-level blocks (the old
            // plain-text wrap kept paragraph gaps the same way).
            out.push(Vec::new());
        }
        thought_block_lines(&top.block, 0, &mut out);
    }
    while out.last().is_some_and(|l| l.iter().all(|r| r.text.trim().is_empty())) {
        out.pop();
    }
    out
}

/// The slot-0 indent run every emitted thought line opens with (possibly
/// empty). List/quote handlers rewrite it in place to plant markers/bars, so
/// it must exist even at zero indent.
fn indent_run(indent: usize) -> Vec<InlineRun> {
    vec![InlineRun { text: " ".repeat(indent), style: InlineStyle::default() }]
}

/// Append text to a line's run list, merging into the tail run when styles
/// match (keeps run counts small for the shaper).
fn push_styled(line: &mut Vec<InlineRun>, text: &str, style: &InlineStyle) {
    if text.is_empty() {
        return;
    }
    match line.last_mut() {
        Some(last) if last.style == *style => last.text.push_str(text),
        _ => line.push(InlineRun { text: text.to_owned(), style: style.clone() }),
    }
}

/// Close a wrapped line: the slot-0 indent run in front (see [`indent_run`]).
fn finish_line(indent: usize, mut line: Vec<InlineRun>) -> Vec<InlineRun> {
    let mut full = indent_run(indent);
    full.append(&mut line);
    full
}

/// Word-wrap styled runs at the thought column budget. Char-counted like
/// every detail wrap — block heights must stay analytic — with words glued
/// across style boundaries (`**bold**tail` wraps as one unit), separator
/// spaces riding the preceding run, and pathological overlong tokens
/// hard-split at the budget. Hard breaks (`\n` runs) split into
/// separately-wrapped segments.
fn wrap_styled_runs(runs: &[InlineRun], indent: usize, out: &mut Vec<Vec<InlineRun>>) {
    let budget = THOUGHT_WRAP_COLS.saturating_sub(indent).max(16);
    let mut segments: Vec<Vec<InlineRun>> = vec![Vec::new()];
    for run in runs {
        for (ix, piece) in run.text.split('\n').enumerate() {
            if ix > 0 {
                segments.push(Vec::new());
            }
            if !piece.is_empty() {
                segments.last_mut().unwrap().push(InlineRun { text: piece.to_owned(), style: run.style.clone() });
            }
        }
    }
    for segment in segments {
        // Tokens: maximal non-whitespace piece lists, glued across run
        // boundaries so a word split by styling never wraps mid-word.
        let mut tokens: Vec<Vec<InlineRun>> = Vec::new();
        let mut in_token = false;
        for run in &segment {
            let text = run.text.as_str();
            let mut pos = 0;
            while pos < text.len() {
                let rest = &text[pos..];
                let ws = rest.chars().next().is_some_and(char::is_whitespace);
                let end =
                    rest.char_indices().find(|(_, c)| c.is_whitespace() != ws).map_or(text.len(), |(i, _)| pos + i);
                if ws {
                    in_token = false;
                } else {
                    if !in_token {
                        tokens.push(Vec::new());
                        in_token = true;
                    }
                    push_styled(tokens.last_mut().unwrap(), &text[pos..end], &run.style);
                }
                pos = end;
            }
        }
        let mut line: Vec<InlineRun> = Vec::new();
        let mut len = 0usize;
        for token in tokens {
            let tok_len: usize = token.iter().map(|r| r.text.chars().count()).sum();
            if tok_len > budget {
                // Hard-split a pathological token at the budget.
                if len > 0 {
                    out.push(finish_line(indent, std::mem::take(&mut line)));
                    len = 0;
                }
                for piece in token {
                    let mut chars = piece.text.chars();
                    loop {
                        let chunk: String = chars.by_ref().take(budget - len).collect();
                        if chunk.is_empty() {
                            break;
                        }
                        len += chunk.chars().count();
                        push_styled(&mut line, &chunk, &piece.style);
                        if len == budget {
                            out.push(finish_line(indent, std::mem::take(&mut line)));
                            len = 0;
                        }
                    }
                }
                continue;
            }
            if len > 0 && len + 1 + tok_len > budget {
                out.push(finish_line(indent, std::mem::take(&mut line)));
                len = 0;
            }
            if len > 0 {
                if let Some(last) = line.last_mut() {
                    last.text.push(' ');
                }
                len += 1;
            }
            for piece in token {
                push_styled(&mut line, &piece.text, &piece.style);
            }
            len += tok_len;
        }
        if len > 0 {
            out.push(finish_line(indent, line));
        }
    }
}

/// One markdown block into thought detail lines, `indent` spaces deep.
fn thought_block_lines(block: &MdBlock, indent: usize, out: &mut Vec<Vec<InlineRun>>) {
    match block {
        MdBlock::Paragraph { runs } => wrap_styled_runs(runs, indent, out),
        MdBlock::Heading { runs, .. } => {
            // Headings keep the detail's single type size — bold is the cue
            // (an 18px line box can't host display sizes).
            let bold: Vec<InlineRun> = runs
                .iter()
                .map(|r| {
                    let mut r = r.clone();
                    r.style.bold = true;
                    r
                })
                .collect();
            wrap_styled_runs(&bold, indent, out);
        }
        MdBlock::CodeBlock { code, .. } => {
            let style = InlineStyle { code: true, ..InlineStyle::default() };
            for line in code.lines() {
                for chunk in wrap_cols(line, THOUGHT_WRAP_COLS.saturating_sub(indent).max(16)) {
                    let mut row = indent_run(indent);
                    if !chunk.is_empty() {
                        row.push(InlineRun { text: chunk.to_string(), style: style.clone() });
                    }
                    out.push(row);
                }
            }
        }
        MdBlock::List { ordered_start, items } => {
            // Tight rendering: no blank rows inside a list.
            for (ix, item) in items.iter().enumerate() {
                let marker = match ordered_start {
                    Some(start) => format!("{}. ", start + ix as u64),
                    None => "• ".to_string(),
                };
                let inner = indent + marker.chars().count();
                let mark = out.len();
                for child in item {
                    thought_block_lines(child, inner, out);
                }
                if out.len() == mark {
                    // An empty item still shows its marker.
                    out.push(indent_run(inner));
                }
                // The item's first line trades its indent spaces for the
                // marker (the slot-0 run is always the indent).
                if let Some(first) = out[mark].first_mut() {
                    first.text = format!("{}{marker}", " ".repeat(indent));
                }
            }
        }
        MdBlock::BlockQuote { children } => {
            let mark = out.len();
            for (ix, child) in children.iter().enumerate() {
                if ix > 0 {
                    out.push(Vec::new());
                }
                thought_block_lines(child, indent + 2, out);
            }
            // Trade the two quote-indent spaces for the bar on every quoted
            // line — replace, not overwrite: nested list handlers already
            // planted markers after their own deeper indent.
            for line in &mut out[mark..] {
                if let Some(first) = line.first_mut()
                    && first.text.len() >= indent + 2
                {
                    first.text.replace_range(indent..indent + 2, "│ ");
                }
            }
        }
        MdBlock::Table { header, rows, .. } => {
            // A thought is a record, not a layout surface: cells joined with
            // a dot separator, header bold — no column machinery.
            let join = |cells: &[Vec<InlineRun>], bold: bool| -> Vec<InlineRun> {
                let mut line: Vec<InlineRun> = Vec::new();
                for (ix, cell) in cells.iter().enumerate() {
                    if ix > 0 {
                        push_styled(&mut line, " · ", &InlineStyle::default());
                    }
                    for r in cell {
                        let mut r = r.clone();
                        r.style.bold |= bold;
                        line.push(r);
                    }
                }
                line
            };
            wrap_styled_runs(&join(header, true), indent, out);
            for row in rows {
                wrap_styled_runs(&join(row, false), indent, out);
            }
        }
        MdBlock::Rule => {
            let mut row = indent_run(indent);
            row.push(InlineRun { text: "———".into(), style: InlineStyle::default() });
            out.push(row);
        }
    }
}

/// Soft-wrap one raw line into [`CALL_WRAP_COLS`]-char chunks so a long
/// single-line command stays fully readable instead of ellipsizing.
fn wrap_cols(line: &str, cols: usize) -> Vec<SharedString> {
    if line.chars().count() <= cols {
        return vec![SharedString::from(line.to_owned())];
    }
    line.chars()
        .collect::<Vec<_>>()
        .chunks(cols)
        .map(|chunk| SharedString::from(chunk.iter().collect::<String>()))
        .collect()
}

/// The counted-tail row under a truncated Output/Thought detail.
fn more_lines_row(truncated_by: usize, theme: &Theme) -> gpui::Div {
    div()
        .h(px(LINE_HEIGHT))
        .flex()
        .items_center()
        .text_size(px(LABEL_SIZE))
        .text_color(theme.text_faint)
        .child(SharedString::from(format!("… {truncated_by} more lines")))
}

/// Shape one flattened thought line into gpui text runs — the detail-body
/// palette: faint foreground prose, semibold for bold, mono for code,
/// underlined links (NOT clickable — a thought is a record, not a surface).
fn thought_line_text(line: &[InlineRun], theme: &Theme) -> Option<(SharedString, Vec<TextRun>)> {
    let mut text = String::new();
    let mut runs: Vec<TextRun> = Vec::new();
    for run in line {
        if run.text.is_empty() {
            continue;
        }
        let mut f =
            if run.style.code { gpui::font(theme.font_mono.clone()) } else { gpui::font(theme.font_sans.clone()) };
        if run.style.bold {
            f.weight = gpui::FontWeight::SEMIBOLD;
        }
        if run.style.italic {
            f.style = gpui::FontStyle::Italic;
        }
        runs.push(TextRun {
            len: run.text.len(),
            font: f,
            color: theme.text_faint,
            background_color: None,
            underline: run.style.link.is_some().then_some(gpui::UnderlineStyle {
                color: Some(theme.text_faint),
                thickness: px(1.0),
                wavy: false,
            }),
            strikethrough: run
                .style
                .strikethrough
                .then_some(gpui::StrikethroughStyle { thickness: px(1.0), color: Some(theme.text_faint) }),
        });
        text.push_str(&run.text);
    }
    if text.trim().is_empty() {
        return None;
    }
    Some((text.into(), runs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_hit_their_ends_and_rise() {
        for c in [EASE_OUT, EASE_OUT_EXPO, EASE_OUT_QUINT] {
            assert_eq!(c.eval(0.0), 0.0);
            assert_eq!(c.eval(1.0), 1.0);
            assert!(c.eval(0.25) < c.eval(0.5) && c.eval(0.5) < c.eval(0.75));
        }
        // Ease-out front-loads the motion.
        assert!(EASE_OUT.eval(0.5) > 0.5);
    }

    #[test]
    fn connectors_draw_trunk_then_branch() {
        assert_eq!(connector_parts(0.0, false), (0.0, 0.0));
        assert_eq!(connector_parts(1.0, true), (1.0, 1.0));
        // A row with a predecessor waits for the continuation above it.
        assert_eq!(connector_parts(0.4, true).0, 0.0);
        let (trunk, branch) = connector_parts(0.6, false);
        assert!(trunk > 0.9 && branch > 0.0);
        assert_eq!(continuation(None), 1.0);
    }

    #[test]
    fn folds_snap_height_and_fade_content() {
        let folds = Folds::default();
        let key = (0, PART_GROUP, 0);
        let t0 = Instant::now();
        let first = folds.step(key, false, t0);
        assert!(!first.mounted && !first.animating, "a first frame never animates");
        // Opening mounts the body at once and fades it in.
        let opening = folds.step(key, true, t0);
        assert!(opening.animating && opening.mounted);
        assert_eq!(opening.reveal, 0.0);
        let mid = folds.step(key, true, t0 + FOLD / 2);
        assert!(mid.reveal > 0.0 && mid.reveal < 1.0 && mid.disclosure > 0.0);
        // Closing removes the body at once; only the chevron turns.
        let closing = folds.step(key, false, t0 + FOLD / 2);
        assert!(!closing.mounted && closing.animating && closing.disclosure > 0.0);
        let done = folds.step(key, false, t0 + FOLD * 3);
        assert!(!done.mounted && !done.animating && done.disclosure == 0.0);
    }

    #[test]
    fn shimmer_sweeps_across() {
        let peak = |phase: f32| (0..=20).map(|i| shimmer_amount(i as f32 / 20.0, phase)).fold(0.0, f32::max);
        assert!(peak(0.5) > 0.9);
        assert_eq!(single_line(" a \n  b\tc "), "a b c");
        assert_eq!(file_name(r"C:\src\main.rs"), "main.rs");
        assert_eq!(file_name("src/lib/"), "lib");
    }
}
