//! Word documents: paragraphs (with style, list level, bold/italic runs and
//! inline images) and tables from `word/document.xml`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use super::package::Package;
use super::text;
use super::xml::{self, Tok};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParaKind {
    Title,
    Subtitle,
    Heading(u8),
    Bullet(u8),
    Number(u8),
    Quote,
    Body,
}

#[derive(Clone, Default, Debug)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone)]
pub struct Para {
    pub kind: ParaKind,
    pub center: bool,
    pub runs: Vec<Run>,
    pub images: Vec<Arc<gpui::Image>>,
}

impl Para {
    pub fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}

pub enum Block {
    Para(usize),
    /// Rows of cells; each cell lists its paragraphs.
    Table(Vec<Vec<Vec<usize>>>),
}

pub struct Docx {
    pub paras: Vec<Para>,
    pub blocks: Vec<Block>,
}

const DOC: &str = "word/document.xml";

fn on(attrs: &str) -> bool {
    !matches!(xml::attr(attrs, "w:val").as_deref(), Some("0" | "false" | "off" | "none"))
}

/// Style ids → display names from `word/styles.xml` (ids are localized in
/// documents written by non-English Word).
fn style_names(pkg: &Package) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(src) = pkg.text("word/styles.xml") else { return out };
    let mut id = None;
    for t in xml::tokens(&src) {
        match t {
            Tok::Open { name: "w:style", attrs, .. } => id = xml::attr(attrs, "w:styleId"),
            Tok::Open { name: "w:name", attrs, .. } => {
                if let (Some(id), Some(n)) = (id.take(), xml::attr(attrs, "w:val")) {
                    out.insert(id, n.to_ascii_lowercase());
                }
            }
            _ => {}
        }
    }
    out
}

fn kind_for(style: &str, level: Option<u8>) -> ParaKind {
    let s = style.to_ascii_lowercase().replace(' ', "");
    let lvl = level.unwrap_or(0);
    if s == "title" {
        ParaKind::Title
    } else if s == "subtitle" {
        ParaKind::Subtitle
    } else if let Some(n) = s.strip_prefix("heading").and_then(|n| n.parse::<u8>().ok()) {
        ParaKind::Heading(n.clamp(1, 9))
    } else if s.starts_with("listnumber") {
        ParaKind::Number(lvl.max(s.trim_start_matches("listnumber").parse::<u8>().unwrap_or(1).saturating_sub(1)))
    } else if s.starts_with("listbullet") || s == "listparagraph" || level.is_some() {
        ParaKind::Bullet(lvl.max(s.trim_start_matches("listbullet").parse::<u8>().unwrap_or(1).saturating_sub(1)))
    } else if s.contains("quote") {
        ParaKind::Quote
    } else {
        ParaKind::Body
    }
}

pub fn load(path: &Path) -> Result<Docx, String> {
    let pkg = Package::open(path)?;
    let src = pkg.text(DOC).ok_or("This .docx has no word/document.xml")?;
    let names = style_names(&pkg);
    let rels = pkg.rels(DOC);
    let mut paras: Vec<Para> = Vec::new();
    let mut blocks = Vec::new();
    // Open paragraphs (text boxes nest them) and tables.
    let mut stack: Vec<(usize, String, Option<u8>)> = Vec::new();
    let mut tables: Vec<Vec<Vec<Vec<usize>>>> = Vec::new();
    let mut run = Run::default();
    let mut in_t = false;
    let mut in_rpr = false;
    let mut in_run = false;
    for tok in xml::tokens(&src) {
        match tok {
            Tok::Open { name: "w:p", empty, .. } => {
                let i = paras.len();
                paras.push(Para { kind: ParaKind::Body, center: false, runs: Vec::new(), images: Vec::new() });
                match tables.last_mut().and_then(|t| t.last_mut()).and_then(|r| r.last_mut()) {
                    Some(cell) if stack.is_empty() => cell.push(i),
                    _ if stack.is_empty() => blocks.push(Block::Para(i)),
                    _ => {}
                }
                if !empty {
                    stack.push((i, String::new(), None));
                }
            }
            Tok::Close { name: "w:p", .. } => {
                if let Some((i, style, level)) = stack.pop() {
                    let style = names.get(&style).cloned().unwrap_or(style);
                    paras[i].kind = kind_for(&style, level);
                }
            }
            Tok::Open { name: "w:pStyle", attrs, .. } => {
                if let (Some(top), Some(v)) = (stack.last_mut(), xml::attr(attrs, "w:val")) {
                    top.1 = v;
                }
            }
            Tok::Open { name: "w:ilvl", attrs, .. } => {
                if let Some(top) = stack.last_mut() {
                    top.2 = Some(xml::attr(attrs, "w:val").and_then(|v| v.parse().ok()).unwrap_or(0));
                }
            }
            Tok::Open { name: "w:numId", attrs, .. } => {
                if let Some(top) = stack.last_mut() {
                    if xml::attr(attrs, "w:val").as_deref() != Some("0") {
                        top.2.get_or_insert(0);
                    }
                }
            }
            Tok::Open { name: "w:jc", attrs, .. } if !in_rpr => {
                if let Some(top) = stack.last() {
                    paras[top.0].center = xml::attr(attrs, "w:val").as_deref() == Some("center");
                }
            }
            Tok::Open { name: "w:r", empty: false, .. } => {
                run = Run::default();
                in_run = true;
            }
            Tok::Close { name: "w:r", .. } => {
                in_run = false;
                if let Some(top) = stack.last() {
                    if !run.text.is_empty() {
                        paras[top.0].runs.push(std::mem::take(&mut run));
                    }
                }
            }
            Tok::Open { name: "w:rPr", empty: false, .. } => in_rpr = true,
            Tok::Close { name: "w:rPr", .. } => in_rpr = false,
            Tok::Open { name: "w:b", attrs, .. } if in_rpr => run.bold = on(attrs),
            Tok::Open { name: "w:i", attrs, .. } if in_rpr => run.italic = on(attrs),
            Tok::Open { name: "w:t", empty: false, .. } => in_t = true,
            Tok::Close { name: "w:t", .. } => in_t = false,
            Tok::Text { start, end } if in_t => run.text.push_str(&xml::unescape(&src[start..end])),
            Tok::Open { name: "w:tab", .. } if in_run && !in_rpr => run.text.push('\t'),
            Tok::Open { name: "w:br" | "w:cr", .. } if in_run => run.text.push('\n'),
            Tok::Open { name: "a:blip", attrs, .. } => {
                let image = xml::attr(attrs, "r:embed")
                    .and_then(|id| rels.get(&id))
                    .and_then(|rel| super::image(&pkg, &rel.target));
                if let (Some(top), Some(image)) = (stack.last(), image) {
                    paras[top.0].images.push(image);
                }
            }
            Tok::Open { name: "w:tbl", empty: false, .. } if stack.is_empty() => tables.push(Vec::new()),
            Tok::Close { name: "w:tbl", .. } if stack.is_empty() => {
                if let Some(t) = tables.pop() {
                    match tables.last_mut().and_then(|t| t.last_mut()).and_then(|r| r.last_mut()) {
                        // A nested table flows into its parent cell.
                        Some(cell) => cell.extend(t.into_iter().flatten().flatten()),
                        None => blocks.push(Block::Table(t)),
                    }
                }
            }
            Tok::Open { name: "w:tr", empty: false, .. } if stack.is_empty() => {
                if let Some(t) = tables.last_mut() {
                    t.push(Vec::new());
                }
            }
            Tok::Open { name: "w:tc", empty: false, .. } if stack.is_empty() => {
                if let Some(r) = tables.last_mut().and_then(|t| t.last_mut()) {
                    r.push(Vec::new());
                }
            }
            _ => {}
        }
    }
    Ok(Docx { paras, blocks })
}

/// Save paragraph edits (`index → text`) into the document at `path`.
pub fn save(path: &Path, edits: &[(usize, String)]) -> Result<(), String> {
    let pkg = Package::open(path)?;
    let src = pkg.text(DOC).ok_or("This .docx has no word/document.xml")?;
    let out = text::set_paragraphs(&src, text::WORD, edits)?;
    pkg.save(path, &HashMap::from([(DOC.to_string(), out.into_bytes())]), &Default::default())
}
