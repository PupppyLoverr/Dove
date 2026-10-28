//! Terminal paint and input encoding, adapted from Zeron (MIT):
//! crates/ui/src/terminal/view.rs.
//!
//! - theme ANSI16 plus the xterm 256-color cube / grayscale ramp;
//! - keystroke → PTY bytes (printables, control keys, CSI/SS3 navigation,
//!   Ctrl caret notation, Alt ESC-prefixing);
//! - [`TerminalElement`]: measures cells from the real mono font, reports the
//!   grid size back, and paints backgrounds, selection, column-pinned text
//!   segments and the cursor.

use dove_term::emulator::{CellColor, CellSnapshot, Side};
use gpui::{
    App, Bounds, Entity, GlobalElementId, Hsla, LayoutId, Modifiers, PaintQuad, Pixels, ShapedLine, SharedString,
    Style, TextRun, Window, fill, font, outline, point, px, relative, size,
};

use super::TerminalView;
use crate::theme::{Appearance, Theme};

const TERM_LINE_HEIGHT_RATIO: f32 = 18.0 / 13.0;
pub const TERM_PADDING: f32 = 12.0;

fn rgb8(r: u8, g: u8, b: u8) -> Hsla {
    gpui::rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32).into()
}

const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// 16-231 is the arithmetic 6×6×6 cube (left alone); 232-255 the grey ramp,
/// mirrored in light mode so "dim" stays dim on white.
fn extended_indexed_rgb(appearance: Appearance, index: u8) -> (u8, u8, u8) {
    match index {
        16..=231 => {
            let n = index as usize - 16;
            (CUBE_LEVELS[n / 36], CUBE_LEVELS[(n / 6) % 6], CUBE_LEVELS[n % 6])
        }
        _ => {
            let step = index.saturating_sub(232);
            let step = if appearance.is_dark() { step } else { 23 - step };
            let v = 8 + 10 * step;
            (v, v, v)
        }
    }
}

pub fn resolve_color(color: CellColor, theme: &Theme) -> Hsla {
    match color {
        CellColor::Foreground => theme.terminal_fg,
        CellColor::Background => theme.terminal_bg,
        CellColor::Indexed(ix @ 0..=15) => theme.ansi[ix as usize],
        CellColor::Indexed(ix) => {
            let (r, g, b) = extended_indexed_rgb(theme.appearance, ix);
            rgb8(r, g, b)
        }
        CellColor::Rgb(r, g, b) => rgb8(r, g, b),
    }
}

/// Which cell a pointer landed on and which edge a selection anchors to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellHit {
    pub row: usize,
    pub col: usize,
    pub side: Side,
}

/// Map a position relative to the grid's top-left onto a cell, clamping
/// overshoot so drags past the edge extend the selection to it.
pub fn cell_at(x: f32, y: f32, cell_w: f32, line_h: f32, cols: usize, rows: usize) -> CellHit {
    let usable = |v: f32| v.is_finite() && v > 0.0;
    if cols == 0 || rows == 0 || !usable(cell_w) || !usable(line_h) {
        return CellHit { row: 0, col: 0, side: Side::Left };
    }
    let x = if x.is_finite() { x } else { 0.0 };
    let y = if y.is_finite() { y } else { 0.0 };
    let raw_col = (x / cell_w).floor();
    let mut side = if x.max(0.0) % cell_w > cell_w / 2.0 { Side::Right } else { Side::Left };
    let col = if raw_col > (cols - 1) as f32 {
        side = Side::Right;
        cols - 1
    } else {
        raw_col.max(0.0) as usize
    };
    let raw_row = (y / line_h).floor();
    let row = if raw_row > (rows - 1) as f32 {
        side = Side::Right;
        rows - 1
    } else if raw_row < 0.0 {
        side = Side::Left;
        0
    } else {
        raw_row as usize
    };
    CellHit { row, col, side }
}

/// Encode a keystroke for the PTY. `None` means it is not the terminal's
/// (platform shortcuts fall through to the app keymap).
pub fn keystroke_bytes(key: &str, key_char: Option<&str>, mods: &Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    if mods.platform {
        return None;
    }
    if mods.alt {
        let inner = keystroke_bytes(key, key_char, &Modifiers { alt: false, ..*mods }, app_cursor)?;
        let mut out = vec![0x1b];
        out.extend(inner);
        return Some(out);
    }
    if mods.control {
        return control_bytes(key);
    }
    let seq = |csi: &[u8], ss3: &[u8]| Some(if app_cursor { ss3.to_vec() } else { csi.to_vec() });
    match key {
        "enter" => Some(b"\r".to_vec()),
        "backspace" => Some(vec![0x7f]),
        "tab" => Some(if mods.shift { b"\x1b[Z".to_vec() } else { b"\t".to_vec() }),
        "escape" => Some(vec![0x1b]),
        "space" => Some(b" ".to_vec()),
        "up" => seq(b"\x1b[A", b"\x1bOA"),
        "down" => seq(b"\x1b[B", b"\x1bOB"),
        "right" => seq(b"\x1b[C", b"\x1bOC"),
        "left" => seq(b"\x1b[D", b"\x1bOD"),
        "home" => seq(b"\x1b[H", b"\x1bOH"),
        "end" => seq(b"\x1b[F", b"\x1bOF"),
        "insert" => Some(b"\x1b[2~".to_vec()),
        "delete" => Some(b"\x1b[3~".to_vec()),
        "pageup" => Some(b"\x1b[5~".to_vec()),
        "pagedown" => Some(b"\x1b[6~".to_vec()),
        "f1" => Some(b"\x1bOP".to_vec()),
        "f2" => Some(b"\x1bOQ".to_vec()),
        "f3" => Some(b"\x1bOR".to_vec()),
        "f4" => Some(b"\x1bOS".to_vec()),
        "f5" => Some(b"\x1b[15~".to_vec()),
        "f6" => Some(b"\x1b[17~".to_vec()),
        "f7" => Some(b"\x1b[18~".to_vec()),
        "f8" => Some(b"\x1b[19~".to_vec()),
        "f9" => Some(b"\x1b[20~".to_vec()),
        "f10" => Some(b"\x1b[21~".to_vec()),
        "f11" => Some(b"\x1b[23~".to_vec()),
        "f12" => Some(b"\x1b[24~".to_vec()),
        _ => {
            let text =
                key_char.filter(|c| !c.is_empty()).or(if key.chars().count() == 1 { Some(key) } else { None })?;
            Some(text.as_bytes().to_vec())
        }
    }
}

fn control_bytes(key: &str) -> Option<Vec<u8>> {
    let mut chars = key.chars();
    let (c, rest) = (chars.next()?, chars.next());
    if rest.is_some() {
        return match key {
            "space" => Some(vec![0x00]),
            "backspace" => Some(vec![0x08]),
            "enter" => Some(b"\r".to_vec()),
            _ => None,
        };
    }
    match c {
        'a'..='z' => Some(vec![c as u8 - b'a' + 1]),
        '@' => Some(vec![0x00]),
        '[' => Some(vec![0x1b]),
        '\\' => Some(vec![0x1c]),
        ']' => Some(vec![0x1d]),
        '^' => Some(vec![0x1e]),
        '_' | '/' => Some(vec![0x1f]),
        '?' => Some(vec![0x7f]),
        _ => None,
    }
}

/// Bracketed-paste aware, stripping the one sequence a paste could inject.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let sanitized = text.replace("\x1b[201~", "");
    if bracketed {
        let mut out = b"\x1b[200~".to_vec();
        out.extend_from_slice(sanitized.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        sanitized.into_bytes()
    }
}

/// Grid geometry measured during prepaint, for pointer → cell mapping.
#[derive(Debug, Clone, Copy, Default)]
pub struct GridGeometry {
    pub origin: gpui::Point<Pixels>,
    pub cell_w: f32,
    pub line_h: f32,
    pub cols: u16,
    pub rows: u16,
}

pub struct TerminalElement {
    view: Entity<TerminalView>,
    focus: gpui::FocusHandle,
    focused: bool,
}

impl TerminalElement {
    pub fn new(view: Entity<TerminalView>, focus: gpui::FocusHandle, focused: bool) -> Self {
        Self { view, focus, focused }
    }
}

pub struct TerminalPrepaint {
    bg_quads: Vec<PaintQuad>,
    sel_quads: Vec<PaintQuad>,
    lines: Vec<Vec<(usize, ShapedLine)>>,
    cell_w: Pixels,
    line_h: Pixels,
    cursor: Option<PaintQuad>,
}

impl gpui::IntoElement for TerminalElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = TerminalPrepaint;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> TerminalPrepaint {
        let theme = Theme::of(cx).clone();
        // Ligatures off: a terminal is a fixed grid, and `->` ligating into
        // one glyph would shift the row against the cursor.
        let mut mono = font(theme.font_mono.clone());
        mono.features =
            gpui::FontFeatures(std::sync::Arc::new(vec![("liga".into(), 0), ("calt".into(), 0), ("dlig".into(), 0)]));
        let term_size = crate::design::terminal();
        let font_size = px(term_size);
        let font_id = window.text_system().resolve_font(&mono);
        let cell_w = window.text_system().em_advance(font_id, font_size).unwrap_or(px(term_size * 0.6));
        let line_h = px((term_size * TERM_LINE_HEIGHT_RATIO).round());

        let inner_w = f32::from(bounds.size.width) - 2.0 * TERM_PADDING;
        let inner_h = f32::from(bounds.size.height) - 2.0 * TERM_PADDING;
        let cols = ((inner_w / f32::from(cell_w)).floor() as i64).clamp(2, 500) as u16;
        let rows = ((inner_h / f32::from(line_h)).floor() as i64).clamp(1, 500) as u16;
        let origin = point(bounds.left() + px(TERM_PADDING), bounds.top() + px(TERM_PADDING));

        let (lines_snapshot, cursor_snapshot) = self.view.update(cx, |v, _| {
            v.on_grid_metrics(GridGeometry {
                origin,
                cell_w: f32::from(cell_w),
                line_h: f32::from(line_h),
                cols,
                rows,
            });
            (v.emu.lines(), v.emu.cursor())
        });

        let mut bg_quads = Vec::new();
        let mut sel_quads = Vec::new();
        let mut lines = Vec::with_capacity(lines_snapshot.len());
        for (row_ix, row) in lines_snapshot.iter().enumerate() {
            let y = origin.y + line_h * row_ix as f32;
            let mut sel_start: Option<usize> = None;
            for col in 0..=row.len() {
                let selected = row.get(col).is_some_and(|c| c.selected);
                match (sel_start, selected) {
                    (None, true) => sel_start = Some(col),
                    (Some(start), false) => {
                        sel_quads.push(fill(
                            Bounds::new(
                                point(origin.x + cell_w * start as f32, y),
                                size(cell_w * (col - start) as f32, line_h),
                            ),
                            theme.terminal_selection,
                        ));
                        sel_start = None;
                    }
                    _ => {}
                }
            }
            let mut run: Option<(usize, Hsla)> = None;
            for (col, color) in
                row.iter().map(|c| c.display_colors().1).chain(std::iter::once(CellColor::Background)).enumerate()
            {
                let paint = match color {
                    CellColor::Background => None,
                    other => Some(resolve_color(other, &theme)),
                };
                match (&run, paint) {
                    (None, Some(c)) => run = Some((col, c)),
                    (Some((start, current)), next) if next != Some(*current) => {
                        bg_quads.push(fill(
                            Bounds::new(
                                point(origin.x + cell_w * *start as f32, y),
                                size(cell_w * (col - *start) as f32, line_h),
                            ),
                            *current,
                        ));
                        run = next.map(|c| (col, c));
                    }
                    _ => {}
                }
            }
            lines.push(shape_row(row, &theme, &mono, font_size, window));
        }
        let cursor = cursor_snapshot.map(|c| {
            let b = Bounds::new(
                point(origin.x + cell_w * c.col as f32, origin.y + line_h * c.row as f32),
                size(cell_w, line_h),
            );
            if self.focused { fill(b, theme.cursor) } else { outline(b, theme.cursor, gpui::BorderStyle::Solid) }
        });
        TerminalPrepaint { bg_quads, sel_quads, lines, cell_w, line_h, cursor }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut TerminalPrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let origin = point(bounds.left() + px(TERM_PADDING), bounds.top() + px(TERM_PADDING));
        let (cell_w, line_h) = (prepaint.cell_w, prepaint.line_h);
        window.handle_input(&self.focus, gpui::ElementInputHandler::new(bounds, self.view.clone()), cx);
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for q in prepaint.bg_quads.drain(..) {
                window.paint_quad(q);
            }
            for q in prepaint.sel_quads.drain(..) {
                window.paint_quad(q);
            }
            for (ix, segments) in prepaint.lines.iter().enumerate() {
                let y = origin.y + line_h * ix as f32;
                for (col, line) in segments {
                    let _ = line.paint(
                        point(origin.x + cell_w * *col as f32, y),
                        line_h,
                        gpui::TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            }
            if let Some(c) = prepaint.cursor.take() {
                window.paint_quad(c);
            }
        });
    }
}

/// Fonts tried when JetBrains Mono has no glyph (emoji, symbols, box drawing).
/// JetBrains Mono is first so a later face does not steal box-drawing or Powerline
/// glyphs the mono face already has. Set only on pinned non-ASCII runs, so an
/// ASCII run stays on the mono face.
fn symbol_fallbacks() -> gpui::FontFallbacks {
    static FALLBACKS: std::sync::OnceLock<gpui::FontFallbacks> = std::sync::OnceLock::new();
    FALLBACKS
        .get_or_init(|| {
            gpui::FontFallbacks::from_fonts(vec![
                "JetBrains Mono".into(),
                "Cascadia Mono".into(),
                "Cascadia Code".into(),
                "Consolas".into(),
                "Segoe UI Symbol".into(),
                "Segoe UI Emoji".into(),
                "Segoe UI".into(),
                "Apple Color Emoji".into(),
                "Apple Symbols".into(),
                "Menlo".into(),
                "Noto Sans Mono".into(),
                "Noto Sans Symbols 2".into(),
                "Noto Color Emoji".into(),
                "DejaVu Sans Mono".into(),
                "Microsoft YaHei".into(),
                "Yu Gothic UI".into(),
                "Malgun Gothic".into(),
            ])
        })
        .clone()
}

/// Shape a row into column-pinned segments: ASCII runs shape together
/// (guaranteed cell width), anything that may fall back to another font is
/// pinned to its own column so box drawing and wide glyphs stay on the grid.
fn shape_row(
    row: &[CellSnapshot],
    theme: &Theme,
    mono: &gpui::Font,
    font_size: Pixels,
    window: &Window,
) -> Vec<(usize, ShapedLine)> {
    fn flush(
        segments: &mut Vec<(usize, ShapedLine)>,
        text: &mut String,
        runs: &mut Vec<TextRun>,
        col: usize,
        size: Pixels,
        window: &Window,
    ) {
        if text.is_empty() {
            return;
        }
        let shaped = window.text_system().shape_line(SharedString::from(std::mem::take(text)), size, runs, None);
        segments.push((col, shaped));
        runs.clear();
    }
    let mut segments = Vec::new();
    let mut text = String::with_capacity(row.len());
    let mut runs: Vec<TextRun> = Vec::new();
    let mut seg_col = 0usize;
    for (col, cell) in row.iter().enumerate() {
        if cell.wide_spacer {
            continue;
        }
        let ch = if cell.hidden { ' ' } else { cell.ch };
        let pinned = !ch.is_ascii() || cell.wide;
        if pinned {
            flush(&mut segments, &mut text, &mut runs, seg_col, font_size, window);
        }
        if text.is_empty() {
            seg_col = col;
        }
        let (fg, _) = cell.display_colors();
        let mut color = resolve_color(fg, theme);
        if cell.dim {
            color.a *= 0.6;
        }
        let mut f = mono.clone();
        f.weight = if cell.bold { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL };
        f.style = if cell.italic { gpui::FontStyle::Italic } else { gpui::FontStyle::Normal };
        if pinned {
            f.fallbacks = Some(symbol_fallbacks());
        }
        let underline =
            cell.underline.then_some(gpui::UnderlineStyle { color: Some(color), thickness: px(1.0), wavy: false });
        let len = ch.len_utf8();
        text.push(ch);
        match runs.last_mut() {
            Some(last) if last.color == color && last.font == f && last.underline == underline => last.len += len,
            _ => runs.push(TextRun { len, font: f, color, background_color: None, underline, strikethrough: None }),
        }
        if pinned {
            flush(&mut segments, &mut text, &mut runs, seg_col, font_size, window);
        }
    }
    flush(&mut segments, &mut text, &mut runs, seg_col, font_size, window);
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> Modifiers {
        Modifiers::default()
    }

    #[test]
    fn keys_encode() {
        assert_eq!(keystroke_bytes("enter", None, &m(), false), Some(b"\r".to_vec()));
        assert_eq!(keystroke_bytes("up", None, &m(), false), Some(b"\x1b[A".to_vec()));
        assert_eq!(keystroke_bytes("up", None, &m(), true), Some(b"\x1bOA".to_vec()));
        assert_eq!(keystroke_bytes("a", Some("A"), &m(), false), Some(b"A".to_vec()));
        let ctrl = Modifiers { control: true, ..m() };
        assert_eq!(keystroke_bytes("c", None, &ctrl, false), Some(vec![3]));
        let alt = Modifiers { alt: true, ..m() };
        assert_eq!(keystroke_bytes("b", Some("b"), &alt, false), Some(vec![0x1b, b'b']));
        let platform = Modifiers { platform: true, ..m() };
        assert_eq!(keystroke_bytes("c", None, &platform, false), None);
    }

    #[test]
    fn paste_is_bracketed_and_sanitized() {
        assert_eq!(paste_bytes("hi", false), b"hi".to_vec());
        let p = paste_bytes("x\x1b[201~y", true);
        assert_eq!(p, b"\x1b[200~xy\x1b[201~".to_vec());
    }

    #[test]
    fn cell_hits_clamp() {
        let h = cell_at(25.0, 5.0, 10.0, 18.0, 80, 24);
        assert_eq!((h.row, h.col), (0, 2));
        let far = cell_at(10_000.0, 10_000.0, 10.0, 18.0, 80, 24);
        assert_eq!((far.row, far.col, far.side), (23, 79, Side::Right));
    }
}
