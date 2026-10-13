//! PowerPoint decks: slide size, and per slide its text boxes (with
//! positions inherited from the layout and master), pictures and tables.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use super::package::Package;
use super::text;
use super::xml::{self, Tok};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Debug)]
pub struct TextPara {
    /// Index among the slide's `a:p` elements (the edit address).
    pub index: usize,
    pub text: String,
    pub level: u8,
    /// Points, when the run says.
    pub size: Option<f32>,
    pub bold: bool,
    pub center: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Title,
    CenterTitle,
    Subtitle,
    Body,
    Other,
}

pub enum ShapeKind {
    Text(Vec<TextPara>),
    Picture(Arc<gpui::Image>),
    Table(Vec<Vec<Vec<TextPara>>>),
}

pub struct Shape {
    pub frame: Option<Frame>,
    pub role: Role,
    pub kind: ShapeKind,
}

pub struct Slide {
    pub part: String,
    pub shapes: Vec<Shape>,
}

pub struct Deck {
    pub width: f32,
    pub height: f32,
    pub slides: Vec<Slide>,
    /// Default sizes in points from the master's text styles.
    pub title_size: f32,
    pub body_sizes: [f32; 5],
}

/// The master's `p:titleStyle`/`p:bodyStyle` default run sizes.
fn master_sizes(src: &str) -> (f32, [f32; 5]) {
    let (mut title, mut body) = (44.0, [32.0, 28.0, 24.0, 20.0, 20.0]);
    let mut section = "";
    let mut level = 0usize;
    for t in xml::tokens(src) {
        match t {
            Tok::Open { name: "p:titleStyle", .. } => section = "title",
            Tok::Open { name: "p:bodyStyle", .. } => section = "body",
            Tok::Close { name: "p:titleStyle" | "p:bodyStyle", .. } => section = "",
            Tok::Open { name, .. } if !section.is_empty() && name.starts_with("a:lvl") && name.ends_with("pPr") => {
                level = name[5..name.len() - 3].parse::<usize>().unwrap_or(1).saturating_sub(1);
            }
            Tok::Open { name: "a:defRPr", attrs, .. } if !section.is_empty() => {
                if let Some(sz) = xml::attr(attrs, "sz").and_then(|v| v.parse::<f32>().ok()) {
                    match section {
                        "title" if level == 0 => title = sz / 100.0,
                        "body" if level < 5 => body[level] = sz / 100.0,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    (title, body)
}

fn role(ty: Option<&str>) -> Role {
    match ty {
        Some("title") => Role::Title,
        Some("ctrTitle") => Role::CenterTitle,
        Some("subTitle") => Role::Subtitle,
        Some("body" | "obj") | None => Role::Body,
        _ => Role::Other,
    }
}

fn emu(attrs: &str, k: &str) -> f32 {
    xml::attr(attrs, k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) as f32
}

/// Placeholder frames in a layout or master: by `idx` and by type.
#[derive(Default)]
struct Placeholders {
    by_idx: HashMap<String, Frame>,
    by_type: HashMap<String, Frame>,
}

fn placeholders(src: &str) -> Placeholders {
    let mut out = Placeholders::default();
    let mut ph: Option<(Option<String>, String)> = None;
    let mut frame: Option<Frame> = None;
    let mut depth = 0usize;
    for t in xml::tokens(src) {
        match t {
            Tok::Open { name: "p:sp", empty: false, .. } => {
                depth += 1;
                ph = None;
                frame = None;
            }
            Tok::Close { name: "p:sp", .. } => {
                depth = depth.saturating_sub(1);
                if let (Some((idx, ty)), Some(f)) = (ph.take(), frame.take()) {
                    if let Some(idx) = idx {
                        out.by_idx.insert(idx, f);
                    }
                    out.by_type.entry(ty).or_insert(f);
                }
            }
            Tok::Open { name: "p:ph", attrs, .. } if depth > 0 => {
                ph = Some((xml::attr(attrs, "idx"), xml::attr(attrs, "type").unwrap_or_else(|| "body".into())));
            }
            Tok::Open { name: "a:off", attrs, .. } if depth > 0 && frame.is_none() => {
                frame = Some(Frame { x: emu(attrs, "x"), y: emu(attrs, "y"), w: 0.0, h: 0.0 });
            }
            Tok::Open { name: "a:ext", attrs, .. } if depth > 0 => {
                if let Some(f) = frame.as_mut().filter(|f| f.w == 0.0) {
                    f.w = emu(attrs, "cx");
                    f.h = emu(attrs, "cy");
                }
            }
            _ => {}
        }
    }
    out
}

fn base_type(ty: &str) -> &str {
    match ty {
        "ctrTitle" => "title",
        "subTitle" | "obj" => "body",
        t => t,
    }
}

fn rel_of_kind<'a>(rels: &'a HashMap<String, super::package::Rel>, suffix: &str) -> Option<&'a str> {
    rels.values().find(|r| r.kind.ends_with(suffix)).map(|r| r.target.as_str())
}

fn parse_slide(pkg: &Package, part: &str, cache: &mut HashMap<String, Placeholders>) -> Result<Slide, String> {
    let src = pkg.text(part).ok_or_else(|| format!("missing {part}"))?;
    let rels = pkg.rels(part);
    let layout = rel_of_kind(&rels, "/slideLayout").map(str::to_string);
    let master = layout.as_ref().and_then(|l| rel_of_kind(&pkg.rels(l), "/slideMaster").map(str::to_string));
    for p in layout.iter().chain(master.iter()) {
        if !cache.contains_key(p) {
            cache.insert(p.clone(), placeholders(&pkg.text(p).unwrap_or_default()));
        }
    }
    let inherited = |idx: &Option<String>, ty: &str| -> Option<Frame> {
        let l = layout.as_ref().and_then(|l| cache.get(l));
        let m = master.as_ref().and_then(|m| cache.get(m));
        idx.as_ref()
            .and_then(|i| l.and_then(|l| l.by_idx.get(i)))
            .or_else(|| l.and_then(|l| l.by_type.get(ty).or_else(|| l.by_type.get(base_type(ty)))))
            .or_else(|| m.and_then(|m| m.by_type.get(base_type(ty))))
            .copied()
    };

    struct Cur {
        pic: bool,
        frame: Option<Frame>,
        ph: Option<(Option<String>, String)>,
        paras: Vec<TextPara>,
        image: Option<Arc<gpui::Image>>,
        table: Vec<Vec<Vec<TextPara>>>,
    }
    let mut shapes = Vec::new();
    let mut cur: Option<Cur> = None;
    let mut para_count = 0usize;
    let mut para: Option<TextPara> = None;
    let mut in_t = false;
    for t in xml::tokens(&src) {
        match t {
            Tok::Open { name: name @ ("p:sp" | "p:pic" | "p:graphicFrame" | "p:cxnSp"), empty: false, .. } => {
                cur = Some(Cur {
                    pic: name == "p:pic",
                    frame: None,
                    ph: None,
                    paras: Vec::new(),
                    image: None,
                    table: Vec::new(),
                });
            }
            Tok::Close { name: "p:sp" | "p:pic" | "p:graphicFrame" | "p:cxnSp", .. } => {
                let Some(c) = cur.take() else { continue };
                let ty = c.ph.as_ref().map(|p| p.1.clone());
                let frame = c.frame.or_else(|| c.ph.as_ref().and_then(|(idx, ty)| inherited(idx, ty)));
                let kind = match (c.pic, c.image) {
                    (true, Some(img)) => ShapeKind::Picture(img),
                    _ if !c.table.is_empty() => ShapeKind::Table(c.table),
                    _ if c.paras.iter().any(|p| !p.text.trim().is_empty()) || c.ph.is_some() => {
                        ShapeKind::Text(c.paras)
                    }
                    _ => continue,
                };
                shapes.push(Shape {
                    frame,
                    role: if c.ph.is_some() { role(ty.as_deref()) } else { Role::Other },
                    kind,
                });
            }
            Tok::Open { name: "p:ph", attrs, .. } => {
                if let Some(c) = cur.as_mut() {
                    c.ph = Some((xml::attr(attrs, "idx"), xml::attr(attrs, "type").unwrap_or_else(|| "body".into())));
                }
            }
            Tok::Open { name: "a:off", attrs, .. } => {
                if let Some(c) = cur.as_mut().filter(|c| c.frame.is_none()) {
                    c.frame = Some(Frame { x: emu(attrs, "x"), y: emu(attrs, "y"), w: 0.0, h: 0.0 });
                }
            }
            Tok::Open { name: "a:ext", attrs, .. } => {
                if let Some(f) = cur.as_mut().and_then(|c| c.frame.as_mut()).filter(|f| f.w == 0.0) {
                    f.w = emu(attrs, "cx");
                    f.h = emu(attrs, "cy");
                }
            }
            Tok::Open { name: "a:blip", attrs, .. } => {
                let img =
                    xml::attr(attrs, "r:embed").and_then(|id| rels.get(&id)).and_then(|r| super::image(pkg, &r.target));
                if let Some(c) = cur.as_mut() {
                    c.image = c.image.take().or(img);
                }
            }
            Tok::Open { name: "a:tr", empty: false, .. } => {
                if let Some(c) = cur.as_mut() {
                    c.table.push(Vec::new());
                }
            }
            Tok::Open { name: "a:tc", empty: false, .. } => {
                if let Some(r) = cur.as_mut().and_then(|c| c.table.last_mut()) {
                    r.push(Vec::new());
                }
            }
            Tok::Open { name: "a:p", empty, .. } => {
                let p = TextPara {
                    index: para_count,
                    text: String::new(),
                    level: 0,
                    size: None,
                    bold: false,
                    center: false,
                };
                para_count += 1;
                if empty {
                    if let Some(c) = cur.as_mut() {
                        push_para(c.table.last_mut().and_then(|r| r.last_mut()).unwrap_or(&mut c.paras), p);
                    }
                } else {
                    para = Some(p);
                }
            }
            Tok::Close { name: "a:p", .. } => {
                if let (Some(p), Some(c)) = (para.take(), cur.as_mut()) {
                    let in_cell = !c.table.is_empty();
                    let target = if in_cell { c.table.last_mut().and_then(|r| r.last_mut()) } else { None };
                    match target {
                        Some(cell) => push_para(cell, p),
                        None => push_para(&mut c.paras, p),
                    }
                }
            }
            Tok::Open { name: "a:pPr", attrs, .. } => {
                if let Some(p) = para.as_mut() {
                    p.level = xml::attr(attrs, "lvl").and_then(|v| v.parse().ok()).unwrap_or(0);
                    p.center = xml::attr(attrs, "algn").as_deref() == Some("ctr");
                }
            }
            Tok::Open { name: "a:rPr" | "a:endParaRPr", attrs, .. } => {
                if let Some(p) = para.as_mut().filter(|p| p.size.is_none()) {
                    p.size = xml::attr(attrs, "sz").and_then(|v| v.parse::<f32>().ok()).map(|v| v / 100.0);
                    p.bold = matches!(xml::attr(attrs, "b").as_deref(), Some("1" | "true"));
                }
            }
            Tok::Open { name: "a:t", empty: false, .. } => in_t = true,
            Tok::Close { name: "a:t", .. } => in_t = false,
            Tok::Text { start, end } if in_t => {
                if let Some(p) = para.as_mut() {
                    p.text.push_str(&xml::unescape(&src[start..end]));
                }
            }
            Tok::Open { name: "a:br", .. } => {
                if let Some(p) = para.as_mut() {
                    p.text.push('\n');
                }
            }
            _ => {}
        }
    }
    Ok(Slide { part: part.to_string(), shapes })
}

fn push_para(list: &mut Vec<TextPara>, p: TextPara) {
    list.push(p);
}

pub fn load(path: &Path) -> Result<Deck, String> {
    let pkg = Package::open(path)?;
    let pres = "ppt/presentation.xml";
    let src = pkg.text(pres).ok_or("This .pptx has no ppt/presentation.xml")?;
    let rels = pkg.rels(pres);
    let (mut width, mut height) = (12_192_000.0, 6_858_000.0);
    let mut order = Vec::new();
    for t in xml::tokens(&src) {
        match t {
            Tok::Open { name: "p:sldSz", attrs, .. } => {
                width = emu(attrs, "cx").max(1.0);
                height = emu(attrs, "cy").max(1.0);
            }
            Tok::Open { name: "p:sldId", attrs, .. } => {
                if let Some(rel) = xml::attr(attrs, "r:id").and_then(|id| rels.get(&id)) {
                    order.push(rel.target.clone());
                }
            }
            _ => {}
        }
    }
    let mut cache = HashMap::new();
    let slides: Vec<Slide> = order.iter().map(|part| parse_slide(&pkg, part, &mut cache)).collect::<Result<_, _>>()?;
    let master = rels.values().find(|r| r.kind.ends_with("/slideMaster")).and_then(|r| pkg.text(&r.target));
    let (title_size, body_sizes) =
        master.as_deref().map(master_sizes).unwrap_or((44.0, [32.0, 28.0, 24.0, 20.0, 20.0]));
    Ok(Deck { width, height, slides, title_size, body_sizes })
}

/// Save text edits: `(slide part, paragraph index, text)`.
pub fn save(path: &Path, edits: &[(String, usize, String)]) -> Result<(), String> {
    let pkg = Package::open(path)?;
    let mut by_part: HashMap<&str, Vec<(usize, String)>> = HashMap::new();
    for (part, i, text) in edits {
        by_part.entry(part.as_str()).or_default().push((*i, text.clone()));
    }
    let mut replace = HashMap::new();
    for (part, list) in by_part {
        let src = pkg.text(part).ok_or_else(|| format!("missing {part}"))?;
        replace.insert(part.to_string(), text::set_paragraphs(&src, text::DRAWING, &list)?.into_bytes());
    }
    pkg.save(path, &replace, &Default::default())
}
