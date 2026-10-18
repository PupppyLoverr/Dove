//! Rendering office documents in the Files panel, and the in-memory edits
//! the user makes before saving. Pages and slides are paper (white, as in
//! any office app); the chrome around them follows the theme.

use std::cell::Cell;
use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, ClickEvent, Context, FontStyle, FontWeight, HighlightStyle, InteractiveElement as _, IntoElement,
    ObjectFit, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _,
    StyledText, Window, canvas, div, hsla, px, relative, uniform_list,
};

use super::{Document, docx, pdf, pptx, xlsx};
use crate::design::{radius, ty};
use crate::files_panel::FilesPanel;
use crate::theme::Theme;

/// What the user selected to edit.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Para(usize),
    Slide { slide: usize, para: usize },
    Cell { sheet: usize, at: (u32, u32) },
    Field(usize),
}

/// The visible grid of the current sheet, precomputed so rows render
/// without re-evaluating formulas.
struct Grid {
    rows: usize,
    cols: usize,
    cells: Vec<Vec<(SharedString, bool)>>,
}

pub struct OfficeView {
    pub doc: Document,
    /// Unsaved edits in the order made (one per target).
    pub edits: Vec<(Target, String)>,
    pub selected: Option<Target>,
    pub sheet: usize,
    grid: Arc<Grid>,
    width: Rc<Cell<f32>>,
}

const GRID_MAX_ROWS: usize = 5000;
const GRID_MAX_COLS: usize = 80;
const COL_W: f32 = 96.0;
const ROWNUM_W: f32 = 44.0;
const ROW_H: f32 = 24.0;

fn paper() -> gpui::Hsla {
    hsla(0.0, 0.0, 1.0, 1.0)
}

fn ink(alpha: f32) -> gpui::Hsla {
    hsla(0.0, 0.0, 0.08, alpha)
}

impl OfficeView {
    /// `width` is the Files panel's measured width, shared across reloads.
    pub fn new(doc: Document, width: Rc<Cell<f32>>) -> Self {
        let mut v = Self {
            doc,
            edits: Vec::new(),
            selected: None,
            sheet: 0,
            grid: Arc::new(Grid { rows: 0, cols: 0, cells: Vec::new() }),
            width,
        };
        v.rebuild_grid();
        v
    }

    pub fn rebuild_grid(&mut self) {
        let Document::Xlsx(book) = &self.doc else { return };
        let Some(sheet) = book.sheets.get(self.sheet) else { return };
        let rows = (sheet.rows as usize + 20).clamp(50, GRID_MAX_ROWS);
        let cols = (sheet.cols as usize + 3).clamp(10, GRID_MAX_COLS);
        let filled = (sheet.rows as usize).min(rows);
        let cells = (0..filled)
            .map(|r| {
                (0..cols)
                    .map(|c| {
                        let s = book.display(self.sheet, (r as u32, c as u32));
                        let num = !s.is_empty() && s.parse::<f64>().is_ok();
                        (SharedString::from(s), num)
                    })
                    .collect()
            })
            .collect();
        self.grid = Arc::new(Grid { rows, cols, cells });
    }

    pub fn dirty(&self) -> usize {
        self.edits.len()
    }

    fn edited(&self, t: &Target) -> bool {
        self.edits.iter().any(|(x, _)| x == t)
    }

    fn slide_para(&self, slide: usize, para: usize) -> Option<&pptx::TextPara> {
        let Document::Pptx(deck) = &self.doc else { return None };
        deck.slides.get(slide)?.shapes.iter().find_map(|s| match &s.kind {
            pptx::ShapeKind::Text(ps) => ps.iter().find(|p| p.index == para),
            pptx::ShapeKind::Table(rows) => rows.iter().flatten().flatten().find(|p| p.index == para),
            pptx::ShapeKind::Picture(_) => None,
        })
    }

    /// The current text of `t` (what the edit field starts from).
    pub fn text_of(&self, t: &Target) -> String {
        match (t, &self.doc) {
            (Target::Para(i), Document::Docx(d)) => d.paras.get(*i).map(docx::Para::text).unwrap_or_default(),
            (Target::Slide { slide, para }, _) => {
                self.slide_para(*slide, *para).map(|p| p.text.clone()).unwrap_or_default()
            }
            (Target::Cell { sheet, at }, Document::Xlsx(b)) => b.input(*sheet, *at),
            (Target::Field(i), Document::Pdf(p)) => p.fields.get(*i).map(|f| f.value.clone()).unwrap_or_default(),
            _ => String::new(),
        }
    }

    pub fn label(&self, t: &Target) -> String {
        match (t, &self.doc) {
            (Target::Para(i), _) => format!("Paragraph {}", i + 1),
            (Target::Slide { slide, .. }, _) => format!("Slide {}", slide + 1),
            (Target::Cell { sheet, at }, Document::Xlsx(b)) => format!(
                "{}!{}",
                b.sheets.get(*sheet).map(|s| s.name.as_str()).unwrap_or(""),
                xlsx::cell_name(at.0, at.1)
            ),
            (Target::Field(i), Document::Pdf(p)) => p.fields.get(*i).map(|f| f.name.clone()).unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// Apply an edit in memory; [`save`](Self::save) writes it.
    pub fn apply(&mut self, t: Target, text: String) {
        if self.text_of(&t) == text {
            return;
        }
        match (&t, &mut self.doc) {
            (Target::Para(i), Document::Docx(d)) => {
                if let Some(p) = d.paras.get_mut(*i) {
                    let first = p.runs.first().cloned().unwrap_or_default();
                    p.runs = vec![docx::Run { text: text.replace(['\r', '\n'], " "), ..first }];
                }
            }
            (Target::Slide { slide, para }, Document::Pptx(deck)) => {
                let Some(s) = deck.slides.get_mut(*slide) else { return };
                for shape in &mut s.shapes {
                    let list: Vec<&mut pptx::TextPara> = match &mut shape.kind {
                        pptx::ShapeKind::Text(ps) => ps.iter_mut().collect(),
                        pptx::ShapeKind::Table(rows) => rows.iter_mut().flatten().flatten().collect(),
                        pptx::ShapeKind::Picture(_) => Vec::new(),
                    };
                    for p in list {
                        if p.index == *para {
                            p.text = text.replace(['\r', '\n'], " ");
                        }
                    }
                }
            }
            (Target::Cell { sheet, at }, Document::Xlsx(b)) => b.set(*sheet, *at, &text),
            (Target::Field(i), Document::Pdf(p)) => {
                if let Some(f) = p.fields.get_mut(*i) {
                    f.value = text.clone();
                }
            }
            _ => return,
        }
        self.edits.retain(|(x, _)| *x != t);
        self.edits.push((t, text));
        self.rebuild_grid();
    }

    pub fn toggle_field(&mut self, i: usize) {
        let on = self.text_of(&Target::Field(i)) == "true";
        self.apply(Target::Field(i), if on { "false" } else { "true" }.into());
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let e = &self.edits;
        match &self.doc {
            Document::Docx(_) => docx::save(
                path,
                &e.iter()
                    .filter_map(|(t, s)| if let Target::Para(i) = t { Some((*i, s.clone())) } else { None })
                    .collect::<Vec<_>>(),
            ),
            Document::Pptx(d) => pptx::save(
                path,
                &e.iter()
                    .filter_map(|(t, s)| match t {
                        Target::Slide { slide, para } => Some((d.slides.get(*slide)?.part.clone(), *para, s.clone())),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            ),
            Document::Xlsx(b) => xlsx::save(
                path,
                &e.iter()
                    .filter_map(|(t, s)| match t {
                        Target::Cell { sheet, at } => Some((b.sheets.get(*sheet)?.part.clone(), *at, s.clone())),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            ),
            Document::Pdf(p) => pdf::save(
                path,
                &e.iter()
                    .filter_map(|(t, s)| match t {
                        Target::Field(i) => Some((p.fields.get(*i)?.name.clone(), s.clone())),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            ),
        }
    }
}

fn select(target: Target, cx: &mut Context<FilesPanel>) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let weak = cx.weak_entity();
    move |_, window, cx| {
        let _ = weak.update(cx, |p, cx| p.office_select(target.clone(), window, cx));
    }
}

/// Tracks the panel width (slides and pages scale to it).
fn measure(width: Rc<Cell<f32>>, cx: &mut Context<FilesPanel>) -> impl IntoElement {
    let weak = cx.weak_entity();
    canvas(
        move |bounds, window, _| {
            let w: f32 = bounds.size.width.into();
            if (w - width.get()).abs() > 0.5 {
                width.set(w);
                // Notifying mid-frame is dropped; re-render on the next one.
                window.on_next_frame(move |_, cx| {
                    let _ = weak.update(cx, |_, cx| cx.notify());
                });
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub fn render(view: &OfficeView, t: &Theme, cx: &mut Context<FilesPanel>) -> AnyElement {
    let body = match &view.doc {
        Document::Docx(d) => docx_view(view, d, t, cx),
        Document::Pptx(d) => pptx_view(view, d, t, cx),
        Document::Xlsx(b) => xlsx_view(view, b, t, cx),
        Document::Pdf(p) => pdf_view(view, p, t, cx),
    };
    div()
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .child(measure(view.width.clone(), cx))
        .child(div().flex_1().min_h_0().flex().flex_col().child(body))
        .into_any_element()
}

fn content_width(view: &OfficeView) -> f32 {
    let w = view.width.get();
    if w > 64.0 { w - 32.0 } else { 400.0 }
}

// ---- Word -----------------------------------------------------------------

fn docx_view(view: &OfficeView, d: &docx::Docx, t: &Theme, cx: &mut Context<FilesPanel>) -> AnyElement {
    let mut items = Vec::new();
    let mut number = 0u32;
    let mut table_count = 0usize;
    let word_count: usize = d.paras.iter().map(|p| p.text().split_whitespace().count()).sum();
    for b in &d.blocks {
        match b {
            docx::Block::Para(i) => {
                let p = &d.paras[*i];
                let prefix = match p.kind {
                    docx::ParaKind::Number(_) => {
                        number += 1;
                        Some(format!("{number}."))
                    }
                    docx::ParaKind::Bullet(_) => Some("•".to_string()),
                    _ => None,
                };
                if !matches!(p.kind, docx::ParaKind::Number(_)) {
                    number = 0;
                }
                items.push(docx_para(view, *i, p, prefix, false, t, cx));
            }
            docx::Block::Table(rows) => {
                number = 0;
                table_count += 1;
                let line = t.border;
                let table = div()
                    .my(px(6.0))
                    .border_1()
                    .border_color(line)
                    .rounded(px(radius::RADIUS))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .children(rows.iter().enumerate().map(|(ri, row)| {
                        div()
                            .flex()
                            .when(ri > 0, |d| d.border_t_1().border_color(line))
                            .when(ri == 0, |d| d.bg(t.wash(0.04)).font_weight(FontWeight::MEDIUM))
                            .children(row.iter().enumerate().map(|(ci, cell)| {
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .p(px(4.0))
                                    .when(ci > 0, |d| d.border_l_1().border_color(line))
                                    .children(cell.iter().map(|&i| docx_para(view, i, &d.paras[i], None, true, t, cx)))
                            }))
                    }))
                    .into_any_element();
                items.push(table);
            }
        }
    }
    let strip = div()
        .h(px(28.0))
        .px(px(14.0))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.0))
        .border_b_1()
        .border_color(t.border)
        .bg(t.wash(0.02))
        .text_size(ty::CAPTION)
        .text_color(t.text_faint)
        .child(div().font_weight(FontWeight::MEDIUM).text_color(t.text_muted).child("Word Document"))
        .child("·")
        .child(format!("{word_count} words"))
        .child("·")
        .child(format!("{} paragraphs", d.paras.len()))
        .when(table_count > 0, |el| el.child("·").child(format!("{table_count} tables")))
        .child(div().flex_1())
        .when(view.dirty() > 0, |el| el.child(crate::kit::badge(format!("{} edited", view.dirty()), t.accent)));

    div()
        .size_full()
        .flex()
        .flex_col()
        .child(strip)
        .child(
            div().id("office-docx").flex_1().min_h_0().overflow_y_scroll().p(px(14.0)).child(
                div()
                    .rounded(px(radius::RADIUS))
                    .border_1()
                    .border_color(t.border)
                    .bg(t.wash(0.015))
                    .px(px(16.0))
                    .py(px(14.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .children(items),
            ),
        )
        .into_any_element()
}

fn docx_para(
    view: &OfficeView,
    i: usize,
    p: &docx::Para,
    prefix: Option<String>,
    in_table: bool,
    t: &Theme,
    cx: &mut Context<FilesPanel>,
) -> AnyElement {
    use docx::ParaKind::*;
    let target = Target::Para(i);
    let selected = view.selected.as_ref() == Some(&target);
    let edited = view.edited(&target);
    let (size, weight, color) = match p.kind {
        Title => (24.0, FontWeight::SEMIBOLD, t.text),
        Subtitle => (15.0, FontWeight::NORMAL, t.text_muted),
        Heading(1) => (19.0, FontWeight::SEMIBOLD, t.text),
        Heading(2) => (16.0, FontWeight::SEMIBOLD, t.text),
        Heading(_) => (14.0, FontWeight::SEMIBOLD, t.text),
        _ => (if in_table { 12.5 } else { 13.5 }, FontWeight::NORMAL, t.text),
    };
    let text = p.text();
    let mut highlights = Vec::new();
    let mut at = 0;
    for r in &p.runs {
        let end = at + r.text.len();
        if r.bold || r.italic {
            highlights.push((
                at..end,
                HighlightStyle {
                    font_weight: r.bold.then_some(FontWeight::BOLD),
                    font_style: r.italic.then_some(FontStyle::Italic),
                    ..Default::default()
                },
            ));
        }
        at = end;
    }
    let body = if text.is_empty() {
        div().h(px(size * 1.2)).into_any_element()
    } else {
        StyledText::new(SharedString::from(text)).with_highlights(highlights).into_any_element()
    };
    let indent = match p.kind {
        Bullet(l) | Number(l) => 4.0 + l as f32 * 16.0,
        _ => 0.0,
    };
    let quote = matches!(p.kind, Quote);
    let hover = t.wash(0.06);
    div()
        .id(("docx-p", i))
        .px(px(6.0))
        .py(px(match p.kind {
            Title | Heading(_) => 5.0,
            _ => 2.0,
        }))
        .rounded(px(radius::RADIUS))
        .cursor_text()
        .text_size(px(size))
        .font_weight(weight)
        .text_color(color)
        .line_height(relative(1.45))
        .when(p.center, |d| d.text_center())
        .when(selected, |d| d.bg(t.wash(0.11)))
        .when(edited && !selected, |d| d.bg(t.wash(0.05)))
        .hover(move |s| s.bg(hover))
        .child(
            div()
                .flex()
                .gap(px(8.0))
                .pl(px(indent))
                .when_some(prefix, |d, pre| d.child(div().flex_none().text_color(t.text_muted).child(pre)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .when(quote, |d| {
                            d.border_l_2().border_color(t.border_strong).pl(px(10.0)).italic().text_color(t.text_muted)
                        })
                        .child(body),
                ),
        )
        .children(
            p.images.iter().map(|img| {
                gpui::img(img.clone()).w_full().h(px(220.0)).object_fit(ObjectFit::Contain).into_any_element()
            }),
        )
        .on_click(select(target, cx))
        .into_any_element()
}

// ---- PowerPoint -------------------------------------------------------------

struct SlideCtx<'a> {
    deck: &'a pptx::Deck,
    slide: usize,
    scale: f32,
    pt: f32,
}

fn pptx_view(view: &OfficeView, d: &pptx::Deck, t: &Theme, cx: &mut Context<FilesPanel>) -> AnyElement {
    let w = content_width(view).max(160.0);
    let scale = w / d.width;
    let pt = w / (d.width / 12700.0);
    let h = d.height * scale;
    let total_slides = d.slides.len();
    let sel_slide = match &view.selected {
        Some(Target::Slide { slide, .. }) => Some(*slide),
        _ => None,
    };
    let mut slides = Vec::new();
    for (si, s) in d.slides.iter().enumerate() {
        let sc = SlideCtx { deck: d, slide: si, scale, pt };
        let shapes: Vec<AnyElement> = s.shapes.iter().map(|sh| pptx_shape(view, &sc, sh, cx)).collect();
        let slide_edited = view.edits.iter().any(|(tgt, _)| matches!(tgt, Target::Slide { slide, .. } if *slide == si));
        let is_sel = sel_slide == Some(si);
        slides.push(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(ty::CAPTION)
                        .text_color(if is_sel { t.text } else { t.text_faint })
                        .child(format!("Slide {} of {total_slides}", si + 1))
                        .when(slide_edited, |el| el.child(crate::kit::badge("Edited", t.accent))),
                )
                .child(
                    div()
                        .relative()
                        .w(px(w))
                        .h(px(h))
                        .bg(paper())
                        .rounded(px(radius::RADIUS))
                        .border_1()
                        .border_color(if is_sel { t.accent } else { t.border })
                        .shadow_sm()
                        .overflow_hidden()
                        .children(shapes),
                ),
        );
    }
    if slides.is_empty() {
        return crate::kit::empty_state(crate::assets::icons::DOCUMENT, "No slides", "This deck is empty.", t)
            .into_any_element();
    }
    let strip = div()
        .h(px(28.0))
        .px(px(14.0))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.0))
        .border_b_1()
        .border_color(t.border)
        .bg(t.wash(0.02))
        .text_size(ty::CAPTION)
        .text_color(t.text_faint)
        .child(div().font_weight(FontWeight::MEDIUM).text_color(t.text_muted).child("Presentation"))
        .child("·")
        .child(format!("{total_slides} slides"))
        .child(div().flex_1())
        .when(view.dirty() > 0, |el| el.child(crate::kit::badge(format!("{} edited", view.dirty()), t.accent)));

    div()
        .size_full()
        .flex()
        .flex_col()
        .child(strip)
        .child(
            div()
                .id("office-pptx")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p(px(16.0))
                .flex()
                .flex_col()
                .gap(px(18.0))
                .children(slides),
        )
        .into_any_element()
}

fn pptx_shape(view: &OfficeView, sc: &SlideCtx, sh: &pptx::Shape, cx: &mut Context<FilesPanel>) -> AnyElement {
    use pptx::Role;
    let d = sc.deck;
    let f = sh.frame.filter(|f| f.w > 0.0 && f.h > 0.0).unwrap_or(match sh.role {
        Role::Title | Role::CenterTitle => {
            pptx::Frame { x: d.width * 0.06, y: d.height * 0.05, w: d.width * 0.88, h: d.height * 0.16 }
        }
        _ => pptx::Frame { x: d.width * 0.06, y: d.height * 0.25, w: d.width * 0.88, h: d.height * 0.68 },
    });
    let s = sc.scale;
    let frame = div().absolute().left(px(f.x * s)).top(px(f.y * s)).w(px(f.w * s)).h(px(f.h * s)).overflow_hidden();
    match &sh.kind {
        pptx::ShapeKind::Picture(img) => {
            frame.child(gpui::img(img.clone()).size_full().object_fit(ObjectFit::Fill)).into_any_element()
        }
        pptx::ShapeKind::Text(paras) => {
            let pad = 91440.0 * s;
            frame
                .px(px(pad))
                .py(px(pad * 0.5))
                .flex()
                .flex_col()
                .when(matches!(sh.role, Role::Title), |d| d.justify_center())
                .when(matches!(sh.role, Role::CenterTitle), |d| d.justify_end())
                .text_color(ink(1.0))
                .children(paras.iter().map(|p| slide_para(view, sc, sh.role, p, None, cx)))
                .into_any_element()
        }
        pptx::ShapeKind::Table(rows) => {
            let line = ink(0.25);
            frame
                .flex()
                .flex_col()
                .border_1()
                .border_color(line)
                .text_color(ink(1.0))
                .children(rows.iter().enumerate().map(|(ri, row)| {
                    div()
                        .flex_1()
                        .flex()
                        .when(ri > 0, |d| d.border_t_1().border_color(line))
                        .when(ri == 0, |d| d.bg(ink(0.06)))
                        .children(row.iter().enumerate().map(|(ci, cell)| {
                            div()
                                .flex_1()
                                .min_w_0()
                                .px(px(4.0))
                                .flex()
                                .flex_col()
                                .justify_center()
                                .when(ci > 0, |d| d.border_l_1().border_color(line))
                                .children(cell.iter().map(|p| slide_para(view, sc, Role::Other, p, Some(14.0), cx)))
                        }))
                }))
                .into_any_element()
        }
    }
}

fn slide_para(
    view: &OfficeView,
    sc: &SlideCtx,
    role: pptx::Role,
    p: &pptx::TextPara,
    fallback: Option<f32>,
    cx: &mut Context<FilesPanel>,
) -> AnyElement {
    use pptx::Role;
    let d = sc.deck;
    let default = fallback.unwrap_or(match role {
        Role::Title | Role::CenterTitle => d.title_size,
        Role::Subtitle => d.body_sizes[0] * 0.75,
        Role::Body => d.body_sizes[(p.level as usize).min(4)],
        Role::Other => 18.0,
    });
    let size = (p.size.unwrap_or(default) * sc.pt).max(4.0);
    let target = Target::Slide { slide: sc.slide, para: p.index };
    let selected = view.selected.as_ref() == Some(&target);
    let edited = view.edited(&target);
    let bullet = role == Role::Body && !p.text.trim().is_empty();
    let center = p.center || matches!(role, Role::CenterTitle | Role::Subtitle);
    div()
        .id(("pptx-p", sc.slide * 100_000 + p.index))
        .flex()
        .gap(px(size * 0.45))
        .pl(px(p.level as f32 * size * 1.1))
        .text_size(px(size))
        .line_height(relative(1.15))
        .when(p.bold || matches!(role, Role::Title | Role::CenterTitle), |d| d.font_weight(FontWeight::SEMIBOLD))
        .when(center, |d| d.justify_center().text_center())
        .rounded(px(2.0))
        .cursor_text()
        .when(selected, |d| d.bg(ink(0.10)))
        .when(edited && !selected, |d| d.bg(ink(0.05)))
        .hover(|s| s.bg(ink(0.05)))
        .when(bullet, |d| d.child(div().flex_none().child("•")))
        .child(div().min_w_0().child(if p.text.is_empty() { " ".to_string() } else { p.text.clone() }))
        .on_click(select(target, cx))
        .into_any_element()
}

// ---- Excel ----------------------------------------------------------------

fn xlsx_view(view: &OfficeView, b: &xlsx::Book, t: &Theme, cx: &mut Context<FilesPanel>) -> AnyElement {
    let sheet = view.sheet;
    let selected = match &view.selected {
        Some(Target::Cell { sheet: s, at }) if *s == sheet => Some(*at),
        _ => None,
    };
    let (sh_rows, sh_cols) = b.sheets.get(sheet).map(|s| (s.rows, s.cols)).unwrap_or((0, 0));
    let tabs = div()
        .id("xlsx-tabs")
        .flex_none()
        .flex()
        .items_center()
        .gap(px(2.0))
        .px(px(8.0))
        .py(px(5.0))
        .overflow_x_scroll()
        .border_b_1()
        .border_color(t.border)
        .bg(t.wash(0.02))
        .children(b.sheets.iter().enumerate().map(|(i, s)| {
            let weak = cx.weak_entity();
            let hover = t.wash(0.06);
            div()
                .id(("xlsx-tab", i))
                .flex_none()
                .px(px(10.0))
                .h(px(24.0))
                .flex()
                .items_center()
                .rounded(px(radius::RADIUS))
                .text_size(ty::META)
                .font_weight(if i == sheet { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                .cursor_pointer()
                .when(i == sheet, |d| d.bg(t.surface_raised).border_1().border_color(t.border).text_color(t.text))
                .when(i != sheet, |d| d.text_color(t.text_muted).hover(move |s| s.bg(hover)))
                .child(s.name.clone())
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |p, cx| p.office_sheet(i, cx));
                })
        }))
        .child(div().flex_1())
        .child(div().px(px(6.0)).text_size(ty::MICRO).text_color(t.text_faint).child(format!("{sh_rows} × {sh_cols}")));

    let (cell_coord, cell_raw, cell_disp) = match selected {
        Some(at) => (xlsx::cell_name(at.0, at.1), b.input(sheet, at), b.display(sheet, at)),
        None => ("—".to_string(), String::new(), String::new()),
    };
    let is_formula = cell_raw.starts_with('=');
    let fx_bar = div()
        .h(px(28.0))
        .px(px(8.0))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.0))
        .border_b_1()
        .border_color(t.border)
        .bg(t.wash(0.015))
        .text_size(ty::META)
        .child(
            div()
                .min_w(px(46.0))
                .h(px(20.0))
                .px(px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(radius::RADIUS))
                .bg(t.wash(0.05))
                .font_family(t.font_mono.clone())
                .text_size(ty::MICRO)
                .text_color(t.text)
                .child(cell_coord),
        )
        .child(
            div().font_family(t.font_mono.clone()).italic().text_size(ty::CAPTION).text_color(t.text_faint).child("fx"),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(t.font_mono.clone())
                .text_size(ty::CAPTION)
                .text_color(if selected.is_some() && !cell_raw.is_empty() { t.text } else { t.text_faint })
                .child(if selected.is_none() {
                    "Click any cell to inspect formula or edit value".to_string()
                } else if cell_raw.is_empty() {
                    "(empty)".to_string()
                } else if is_formula && cell_disp != cell_raw {
                    format!("{cell_raw}  →  {cell_disp}")
                } else {
                    cell_raw
                }),
        );

    let grid = view.grid.clone();
    let edited: Arc<HashSet<(u32, u32)>> = Arc::new(
        view.edits
            .iter()
            .filter_map(|(t, _)| match t {
                Target::Cell { sheet: s, at } if *s == sheet => Some(*at),
                _ => None,
            })
            .collect(),
    );
    let total = ROWNUM_W + grid.cols as f32 * COL_W;
    let line = t.border;
    let header = div()
        .flex_none()
        .flex()
        .h(px(ROW_H))
        .bg(t.wash(0.03))
        .border_b_1()
        .border_color(line)
        .child(div().w(px(ROWNUM_W)).flex_none())
        .children((0..grid.cols).map(|c| {
            div()
                .w(px(COL_W))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .border_l_1()
                .border_color(line)
                .text_size(ty::CAPTION)
                .text_color(t.text_faint)
                .child(xlsx::col_name(c as u32))
        }));
    let weak = cx.weak_entity();
    let list = uniform_list("xlsx-grid", grid.rows, move |range, _, cx| {
        let t = Theme::of(cx);
        let line = t.border.opacity(0.7);
        range
            .map(|r| {
                let row = grid.cells.get(r);
                div()
                    .flex()
                    .h(px(ROW_H))
                    .child(
                        div()
                            .w(px(ROWNUM_W))
                            .flex_none()
                            .pr(px(6.0))
                            .flex()
                            .items_center()
                            .justify_end()
                            .border_b_1()
                            .border_color(line)
                            .text_size(ty::CAPTION)
                            .text_color(t.text_faint)
                            .child((r + 1).to_string()),
                    )
                    .children((0..grid.cols).map(|c| {
                        let (text, num) = row.and_then(|row| row.get(c)).cloned().unwrap_or_default();
                        let at = (r as u32, c as u32);
                        let is_sel = selected == Some(at);
                        let weak = weak.clone();
                        div()
                            .id(("xc", r * 1000 + c))
                            .w(px(COL_W))
                            .h_full()
                            .flex_none()
                            .px(px(6.0))
                            .flex()
                            .items_center()
                            .when(num, |d| d.justify_end())
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(ty::SMALL)
                            .border_l_1()
                            .border_b_1()
                            .border_color(line)
                            .when(edited.contains(&at), |d| d.bg(t.wash(0.05)))
                            .when(is_sel, |d| d.border_1().border_color(t.text).bg(t.wash(0.08)))
                            .cursor_pointer()
                            .child(text)
                            .on_click(move |_, window, cx| {
                                let _ =
                                    weak.update(cx, |p, cx| p.office_select(Target::Cell { sheet, at }, window, cx));
                            })
                            .into_any_element()
                    }))
                    .into_any_element()
            })
            .collect()
    })
    .flex_1();
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(tabs)
        .child(fx_bar)
        .child(
            div()
                .id("xlsx-h")
                .flex_1()
                .min_h_0()
                .overflow_x_scroll()
                .child(div().w(px(total)).h_full().flex().flex_col().child(header).child(list)),
        )
        .into_any_element()
}

// ---- PDF --------------------------------------------------------------------

fn pdf_view(view: &OfficeView, p: &pdf::Pdf, t: &Theme, cx: &mut Context<FilesPanel>) -> AnyElement {
    let w = content_width(view).max(160.0);
    let strip = div()
        .h(px(28.0))
        .px(px(14.0))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.0))
        .border_b_1()
        .border_color(t.border)
        .bg(t.wash(0.02))
        .text_size(ty::CAPTION)
        .text_color(t.text_faint)
        .child(div().font_weight(FontWeight::MEDIUM).text_color(t.text_muted).child("PDF Document"))
        .child("·")
        .child(format!("{} pages", p.page_count))
        .when(!p.fields.is_empty(), |el| el.child("·").child(format!("{} form fields", p.fields.len())))
        .child(div().flex_1())
        .when(view.dirty() > 0, |el| el.child(crate::kit::badge(format!("{} edited", view.dirty()), t.accent)));

    let fields = (!p.fields.is_empty()).then(|| {
        let hover = t.wash(0.06);
        div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .p(px(6.0))
            .rounded(px(radius::RADIUS))
            .border_1()
            .border_color(t.border)
            .bg(t.wash(0.02))
            .child(
                div()
                    .px(px(6.0))
                    .py(px(4.0))
                    .text_size(ty::CAPTION)
                    .text_color(t.text_faint)
                    .child("Interactive AcroForm Fields"),
            )
            .children(p.fields.iter().enumerate().map(|(i, f)| {
                let target = Target::Field(i);
                let selected = view.selected.as_ref() == Some(&target);
                let row = div()
                    .id(("pdf-field", i))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(6.0))
                    .py(px(5.0))
                    .rounded(px(radius::RADIUS))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .when(selected, |d| d.bg(t.wash(0.10)))
                    .when(view.edited(&target) && !selected, |d| d.bg(t.wash(0.05)))
                    .child(
                        div()
                            .w(relative(0.4))
                            .flex_none()
                            .truncate()
                            .text_size(ty::META)
                            .text_color(t.text_muted)
                            .child(f.name.clone()),
                    );
                if f.kind == pdf::FieldKind::Check {
                    let weak = cx.weak_entity();
                    row.child(
                        div()
                            .size(px(14.0))
                            .rounded(px(3.0))
                            .border_1()
                            .border_color(t.border_strong)
                            .when(f.value == "true", |d| d.bg(t.text)),
                    )
                    .on_click(move |_, _, cx| {
                        let _ = weak.update(cx, |p, cx| p.office_toggle(i, cx));
                    })
                    .into_any_element()
                } else {
                    row.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(ty::BODY)
                            .when(f.value.is_empty(), |d| d.text_color(t.text_faint))
                            .child(if f.value.is_empty() { "Empty".to_string() } else { f.value.clone() }),
                    )
                    .on_click(select(target, cx))
                    .into_any_element()
                }
            }))
    });
    let more = (p.page_count > p.pages.len()).then(|| {
        div().text_size(ty::META).text_color(t.text_faint).child(format!(
            "Showing the first {} of {} pages. Open the file in your PDF viewer for the rest.",
            p.pages.len(),
            p.page_count
        ))
    });
    let total_pages = p.page_count;
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(strip)
        .child(
            div()
                .id("office-pdf")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p(px(16.0))
                .flex()
                .flex_col()
                .gap(px(14.0))
                .children(fields)
                .children(p.pages.iter().enumerate().map(|(pi, page)| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.0))
                        .child(
                            div()
                                .text_size(ty::CAPTION)
                                .text_color(t.text_faint)
                                .child(format!("Page {} of {total_pages}", pi + 1)),
                        )
                        .child(
                            div()
                                .w(px(w))
                                .h(px(w * page.aspect))
                                .flex_none()
                                .rounded(px(radius::RADIUS))
                                .border_1()
                                .border_color(t.border)
                                .shadow_sm()
                                .overflow_hidden()
                                .bg(paper())
                                .child(gpui::img(page.image.clone()).size_full().object_fit(ObjectFit::Contain)),
                        )
                }))
                .children(more),
        )
        .into_any_element()
}
