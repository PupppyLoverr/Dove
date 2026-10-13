//! Excel workbooks: sheets, cells (shared/inline strings, numbers, booleans,
//! formulas) and a small formula evaluator so formulas written without a
//! cached value (openpyxl, agents) still show results. Saving rewrites only
//! the edited `<c>` elements and asks Excel to recalculate on open.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use super::package::Package;
use super::xml::{self, Tok};

#[derive(Clone, Debug, Default)]
pub struct Cell {
    /// The stored value (cached result for formulas).
    pub value: String,
    pub formula: Option<String>,
    pub number: bool,
}

pub struct Sheet {
    pub name: String,
    pub part: String,
    /// (row, col), zero-based.
    pub cells: BTreeMap<(u32, u32), Cell>,
    pub rows: u32,
    pub cols: u32,
}

pub struct Book {
    pub sheets: Vec<Sheet>,
}

pub fn col_name(mut c: u32) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (c % 26) as u8);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

pub fn cell_name(row: u32, col: u32) -> String {
    format!("{}{}", col_name(col), row + 1)
}

/// `"B12"` (or `"$B$12"`) → (row 11, col 1).
pub fn parse_ref(r: &str) -> Option<(u32, u32)> {
    let r = r.replace('$', "");
    let split = r.find(|c: char| c.is_ascii_digit())?;
    let (letters, digits) = r.split_at(split);
    if letters.is_empty() || letters.len() > 3 || !letters.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut col = 0u32;
    for c in letters.to_ascii_uppercase().bytes() {
        col = col * 26 + (c - b'A' + 1) as u32;
    }
    let row: u32 = digits.parse().ok()?;
    (row >= 1).then(|| (row - 1, col - 1))
}

fn shared_strings(pkg: &Package) -> Vec<String> {
    let mut out = Vec::new();
    let Some(src) = pkg.text("xl/sharedStrings.xml") else { return out };
    let mut cur: Option<String> = None;
    let mut in_t = false;
    let mut phonetic = false;
    for t in xml::tokens(&src) {
        match t {
            Tok::Open { name, empty, .. } if xml::local(name) == "si" => {
                if empty {
                    out.push(String::new());
                } else {
                    cur = Some(String::new());
                }
            }
            Tok::Close { name, .. } if xml::local(name) == "si" => out.push(cur.take().unwrap_or_default()),
            Tok::Open { name, empty: false, .. } if xml::local(name) == "rPh" => phonetic = true,
            Tok::Close { name, .. } if xml::local(name) == "rPh" => phonetic = false,
            Tok::Open { name, empty: false, .. } if xml::local(name) == "t" => in_t = !phonetic,
            Tok::Close { name, .. } if xml::local(name) == "t" => in_t = false,
            Tok::Text { start, end } if in_t => {
                if let Some(c) = cur.as_mut() {
                    c.push_str(&xml::unescape(&src[start..end]));
                }
            }
            _ => {}
        }
    }
    out
}

fn parse_sheet(src: &str, shared: &[String]) -> (BTreeMap<(u32, u32), Cell>, u32, u32) {
    let mut cells = BTreeMap::new();
    let (mut rows, mut cols) = (0u32, 0u32);
    let mut cur: Option<((u32, u32), String, Cell)> = None;
    let mut field: Option<&str> = None;
    let mut row_at = 0u32;
    let mut col_at = 0u32;
    for t in xml::tokens(src) {
        match t {
            Tok::Open { name, attrs, .. } if xml::local(name) == "row" => {
                row_at = xml::attr(attrs, "r").and_then(|r| r.parse::<u32>().ok()).map_or(row_at + 1, |r| r);
                col_at = 0;
            }
            Tok::Open { name, attrs, empty, .. } if xml::local(name) == "c" => {
                let at = xml::attr(attrs, "r").and_then(|r| parse_ref(&r)).unwrap_or((row_at.saturating_sub(1), col_at));
                col_at = at.1 + 1;
                let ty = xml::attr(attrs, "t").unwrap_or_default();
                if empty {
                    continue;
                }
                cur = Some((at, ty, Cell::default()));
            }
            Tok::Close { name, .. } if xml::local(name) == "c" => {
                if let Some((at, ty, mut cell)) = cur.take() {
                    match ty.as_str() {
                        "s" => cell.value = cell.value.trim().parse::<usize>().ok().and_then(|i| shared.get(i)).cloned().unwrap_or_default(),
                        "b" => cell.value = if cell.value.trim() == "1" { "TRUE".into() } else { "FALSE".into() },
                        "str" | "inlineStr" | "e" => {}
                        _ => cell.number = !cell.value.is_empty(),
                    }
                    if !cell.value.is_empty() || cell.formula.is_some() {
                        rows = rows.max(at.0 + 1);
                        cols = cols.max(at.1 + 1);
                        cells.insert(at, cell);
                    }
                }
            }
            Tok::Open { name, empty: false, .. } if cur.is_some() && matches!(xml::local(name), "v" | "f" | "t") => {
                field = Some(match xml::local(name) {
                    "v" => "v",
                    "f" => "f",
                    _ => "t",
                })
            }
            Tok::Close { name, .. } if matches!(xml::local(name), "v" | "f" | "t") => field = None,
            Tok::Text { start, end } => {
                if let (Some(f), Some((_, _, cell))) = (field, cur.as_mut()) {
                    let text = xml::unescape(&src[start..end]);
                    match f {
                        "f" => cell.formula.get_or_insert_with(String::new).push_str(&text),
                        _ => cell.value.push_str(&text),
                    }
                }
            }
            _ => {}
        }
    }
    (cells, rows, cols)
}

pub fn load(path: &Path) -> Result<Book, String> {
    let pkg = Package::open(path)?;
    let wb = "xl/workbook.xml";
    let src = pkg.text(wb).ok_or("This .xlsx has no xl/workbook.xml")?;
    let rels = pkg.rels(wb);
    let shared = shared_strings(&pkg);
    let mut sheets = Vec::new();
    for t in xml::tokens(&src) {
        let Tok::Open { name, attrs, .. } = t else { continue };
        if xml::local(name) != "sheet" {
            continue;
        }
        let Some(rel) = xml::attr(attrs, "r:id").and_then(|id| rels.get(&id)) else { continue };
        let Some(xml_src) = pkg.text(&rel.target) else { continue };
        let (cells, rows, cols) = parse_sheet(&xml_src, &shared);
        sheets.push(Sheet { name: xml::attr(attrs, "name").unwrap_or_default(), part: rel.target.clone(), cells, rows, cols });
    }
    if sheets.is_empty() {
        return Err("This workbook has no worksheets".into());
    }
    Ok(Book { sheets })
}

/// What a typed entry becomes.
fn cell_xml(r: &str, style: Option<&str>, input: &str) -> String {
    let s = style.map(|s| format!(" s=\"{}\"", xml::escape(s))).unwrap_or_default();
    let input = input.trim_end_matches(['\r', '\n']);
    if let Some(f) = input.strip_prefix('=').filter(|f| !f.trim().is_empty()) {
        format!("<c r=\"{r}\"{s}><f>{}</f></c>", xml::escape(f.trim()))
    } else if input.trim().is_empty() {
        format!("<c r=\"{r}\"{s}/>")
    } else if let Some(n) = input.trim().parse::<f64>().ok().filter(|n| n.is_finite()) {
        format!("<c r=\"{r}\"{s}><v>{n}</v></c>")
    } else if matches!(input.trim().to_ascii_uppercase().as_str(), "TRUE" | "FALSE") {
        format!("<c r=\"{r}\"{s} t=\"b\"><v>{}</v></c>", u8::from(input.trim().eq_ignore_ascii_case("true")))
    } else {
        format!("<c r=\"{r}\"{s} t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>", xml::escape(input))
    }
}

/// Apply `(row, col) → input` to one worksheet's XML.
fn edit_sheet(src: &str, edits: &[((u32, u32), String)]) -> Result<String, String> {
    let toks = xml::tokens(src);
    // Existing cells and rows with their spans.
    let mut cells: HashMap<(u32, u32), (usize, usize, Option<String>)> = HashMap::new();
    let mut rows: BTreeMap<u32, (usize, usize, Option<usize>, Vec<(u32, usize)>)> = BTreeMap::new();
    let mut sheet_data: Option<(usize, usize, Option<usize>)> = None;
    let mut open_cell: Option<((u32, u32), usize, Option<String>)> = None;
    let mut open_row: Option<u32> = None;
    let mut row_at = 0u32;
    for t in &toks {
        match *t {
            Tok::Open { name, start, end, empty, .. } if xml::local(name) == "sheetData" => {
                sheet_data = Some((start, end, if empty { None } else { Some(end) }));
            }
            Tok::Close { name, start, .. } if xml::local(name) == "sheetData" => {
                if let Some(sd) = sheet_data.as_mut() {
                    sd.2 = Some(start);
                }
            }
            Tok::Open { name, attrs, start, end, empty } if xml::local(name) == "row" => {
                row_at = xml::attr(attrs, "r").and_then(|r| r.parse().ok()).unwrap_or(row_at + 1);
                let r = row_at - 1;
                rows.insert(r, (start, end, None, Vec::new()));
                if empty {
                    open_row = None;
                } else {
                    open_row = Some(r);
                }
            }
            Tok::Close { name, start, .. } if xml::local(name) == "row" => {
                if let Some(r) = open_row.take() {
                    if let Some(row) = rows.get_mut(&r) {
                        row.2 = Some(start);
                    }
                }
            }
            Tok::Open { name, attrs, start, end, empty } if xml::local(name) == "c" => {
                let Some(at) = xml::attr(attrs, "r").and_then(|r| parse_ref(&r)) else {
                    return Err("cells without references are not supported for editing".into());
                };
                let style = xml::attr(attrs, "s");
                if let Some(row) = open_row.and_then(|r| rows.get_mut(&r)) {
                    row.3.push((at.1, start));
                }
                if empty {
                    cells.insert(at, (start, end, style));
                } else {
                    open_cell = Some((at, start, style));
                }
            }
            Tok::Close { name, end, .. } if xml::local(name) == "c" => {
                if let Some((at, start, style)) = open_cell.take() {
                    cells.insert(at, (start, end, style));
                }
            }
            _ => {}
        }
    }
    let (sd_start, sd_end, sd_close) = sheet_data.ok_or("worksheet has no sheetData")?;
    let mut splices: Vec<(usize, usize, String)> = Vec::new();
    // New rows, gathered per insertion point.
    let mut new_rows: BTreeMap<u32, Vec<((u32, u32), String)>> = BTreeMap::new();
    // Cells to add to existing rows: (insert position) → xml.
    let mut row_inserts: BTreeMap<(usize, u32), String> = BTreeMap::new();
    let mut expand_rows: HashMap<u32, Vec<(u32, String)>> = HashMap::new();
    for ((r, c), input) in edits {
        let name = cell_name(*r, *c);
        if let Some((start, end, style)) = cells.get(&(*r, *c)) {
            splices.push((*start, *end, cell_xml(&name, style.as_deref(), input)));
        } else if let Some((rs, re, close, list)) = rows.get(r) {
            let x = cell_xml(&name, None, input);
            match close {
                Some(close) => {
                    let pos = list.iter().find(|(col, _)| col > c).map_or(*close, |(_, s)| *s);
                    row_inserts.insert((pos, *c), x);
                }
                None => expand_rows.entry(*r).or_default().push((*c, x)),
            }
            let _ = (rs, re);
        } else {
            new_rows.entry(*r).or_default().push(((*r, *c), input.clone()));
        }
    }
    // Group same-position inserts so their order is kept.
    let mut grouped: BTreeMap<usize, String> = BTreeMap::new();
    for ((pos, _), x) in row_inserts {
        grouped.entry(pos).or_default().push_str(&x);
    }
    for (pos, x) in grouped {
        splices.push((pos, pos, x));
    }
    for (r, mut list) in expand_rows {
        let (rs, re, _, _) = rows[&r];
        list.sort_by_key(|(c, _)| *c);
        let tag = src[rs..re].trim_end_matches('>').trim_end_matches('/');
        let body: String = list.into_iter().map(|(_, x)| x).collect();
        splices.push((rs, re, format!("{tag}>{body}</row>")));
    }
    let mut row_groups: BTreeMap<usize, String> = BTreeMap::new();
    for (r, mut list) in new_rows {
        list.sort_by_key(|(at, _)| at.1);
        let body: String = list.iter().map(|((r, c), input)| cell_xml(&cell_name(*r, *c), None, input)).collect();
        let x = format!("<row r=\"{}\">{body}</row>", r + 1);
        let pos = rows.range(r + 1..).next().map(|(_, row)| row.0).or(sd_close);
        match pos {
            Some(pos) => row_groups.entry(pos).or_default().push_str(&x),
            None => row_groups.entry(usize::MAX).or_default().push_str(&x),
        }
    }
    for (pos, x) in row_groups {
        if pos == usize::MAX {
            // `<sheetData/>`: open it up.
            splices.push((sd_start, sd_end, format!("<sheetData>{x}</sheetData>")));
        } else {
            splices.push((pos, pos, x));
        }
    }
    xml::splice(src, splices)
}

/// Make Excel recalculate formulas when the file opens.
fn full_calc(src: &str) -> String {
    for t in xml::tokens(src) {
        if let Tok::Open { name, start, end, .. } = t {
            if xml::local(name) == "calcPr" {
                let tag = xml::set_attr(&src[start..end], "fullCalcOnLoad", "1");
                return format!("{}{tag}{}", &src[..start], &src[end..]);
            }
        }
    }
    for anchor in ["</definedNames>", "</sheets>"] {
        if let Some(k) = src.find(anchor) {
            let at = k + anchor.len();
            return format!("{}<calcPr fullCalcOnLoad=\"1\"/>{}", &src[..at], &src[at..]);
        }
    }
    src.to_string()
}

/// Drop `name` from a relationships or content-types part.
fn without(src: &str, element: &str, attr: &str, needle: &str) -> String {
    let mut splices = Vec::new();
    for t in xml::tokens(src) {
        if let Tok::Open { name, attrs, start, end, .. } = t {
            if xml::local(name) == element && xml::attr(attrs, attr).is_some_and(|v| v.ends_with(needle)) {
                splices.push((start, end, String::new()));
            }
        }
    }
    xml::splice(src, splices).unwrap_or_else(|_| src.to_string())
}

/// Save cell edits: `(sheet part, (row, col), input)` where input is what
/// the user typed (`=…` for a formula).
pub fn save(path: &Path, edits: &[(String, (u32, u32), String)]) -> Result<(), String> {
    let pkg = Package::open(path)?;
    let mut by_part: HashMap<&str, Vec<((u32, u32), String)>> = HashMap::new();
    for (part, at, input) in edits {
        by_part.entry(part.as_str()).or_default().push((*at, input.clone()));
    }
    let mut replace = HashMap::new();
    for (part, list) in by_part {
        let src = pkg.text(part).ok_or_else(|| format!("missing {part}"))?;
        replace.insert(part.to_string(), edit_sheet(&src, &list)?.into_bytes());
    }
    let wb = "xl/workbook.xml";
    if let Some(src) = pkg.text(wb) {
        replace.insert(wb.to_string(), full_calc(&src).into_bytes());
    }
    // A stale calculation chain makes Excel offer to "repair" the file.
    let mut remove = HashSet::new();
    let chain = "xl/calcChain.xml";
    if pkg.has(chain) {
        remove.insert(chain.to_string());
        if let Some(src) = pkg.text("xl/_rels/workbook.xml.rels") {
            replace.insert("xl/_rels/workbook.xml.rels".into(), without(&src, "Relationship", "Target", "calcChain.xml").into_bytes());
        }
        if let Some(src) = pkg.text("[Content_Types].xml") {
            replace.insert("[Content_Types].xml".into(), without(&src, "Override", "PartName", "/calcChain.xml").into_bytes());
        }
    }
    pkg.save(path, &replace, &remove)
}

// ---- formulas -------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Num(f64),
    Text(String),
    Bool(bool),
    List(Vec<Val>),
}

impl Val {
    fn num(&self) -> Option<f64> {
        match self {
            Val::Num(n) => Some(*n),
            Val::Bool(b) => Some(f64::from(u8::from(*b))),
            Val::Text(t) if t.is_empty() => Some(0.0),
            Val::Text(t) => t.trim().parse().ok(),
            Val::List(l) if l.len() == 1 => l[0].num(),
            Val::List(_) => None,
        }
    }

    fn flat(self, out: &mut Vec<Val>) {
        match self {
            Val::List(l) => l.into_iter().for_each(|v| v.flat(out)),
            v => out.push(v),
        }
    }
}

/// Format a number the way a spreadsheet's General format roughly does.
pub fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        let s = format!("{:.10}", n);
        let s = s.trim_end_matches('0').trim_end_matches('.');
        s.to_string()
    }
}

struct Eval<'a> {
    book: &'a Book,
    sheet: usize,
    depth: u32,
    seen: HashSet<(usize, u32, u32)>,
}

impl Book {
    /// What a cell shows: formula results (evaluated when Dove understands
    /// the formula, else the cached value), numbers in General format.
    pub fn display(&self, sheet: usize, at: (u32, u32)) -> String {
        let Some(cell) = self.sheets.get(sheet).and_then(|s| s.cells.get(&at)) else { return String::new() };
        if let Some(f) = &cell.formula {
            let mut ev = Eval { book: self, sheet, depth: 0, seen: HashSet::new() };
            if let Some(v) = ev.formula(sheet, f) {
                return match v {
                    Val::Num(n) => fmt_num(n),
                    Val::Bool(b) => if b { "TRUE".into() } else { "FALSE".into() },
                    Val::Text(t) => t,
                    Val::List(l) => l.first().map(|v| format!("{v:?}")).unwrap_or_default(),
                };
            }
            if cell.value.is_empty() {
                return format!("={f}");
            }
        }
        if cell.number {
            if let Ok(n) = cell.value.parse::<f64>() {
                return fmt_num(n);
            }
        }
        cell.value.clone()
    }

    /// The text an editor starts from: the formula, or the stored value.
    pub fn input(&self, sheet: usize, at: (u32, u32)) -> String {
        match self.sheets.get(sheet).and_then(|s| s.cells.get(&at)) {
            Some(Cell { formula: Some(f), .. }) => format!("={f}"),
            Some(c) if c.number => c.value.parse::<f64>().map(fmt_num).unwrap_or_else(|_| c.value.clone()),
            Some(c) => c.value.clone(),
            None => String::new(),
        }
    }

    /// Apply a typed entry in memory (mirrors what [`save`] writes).
    pub fn set(&mut self, sheet: usize, at: (u32, u32), input: &str) {
        let Some(s) = self.sheets.get_mut(sheet) else { return };
        let input = input.trim_end_matches(['\r', '\n']);
        if input.trim().is_empty() {
            s.cells.remove(&at);
            return;
        }
        let cell = if let Some(f) = input.strip_prefix('=') {
            Cell { value: String::new(), formula: Some(f.trim().to_string()), number: false }
        } else if input.trim().parse::<f64>().is_ok_and(f64::is_finite) {
            Cell { value: input.trim().to_string(), formula: None, number: true }
        } else if matches!(input.trim().to_ascii_uppercase().as_str(), "TRUE" | "FALSE") {
            Cell { value: input.trim().to_ascii_uppercase(), formula: None, number: false }
        } else {
            Cell { value: input.to_string(), formula: None, number: false }
        };
        s.rows = s.rows.max(at.0 + 1);
        s.cols = s.cols.max(at.1 + 1);
        s.cells.insert(at, cell);
    }
}

impl Eval<'_> {
    fn cell(&mut self, sheet: usize, at: (u32, u32)) -> Option<Val> {
        let Some(cell) = self.book.sheets.get(sheet)?.cells.get(&at) else { return Some(Val::Text(String::new())) };
        if let Some(f) = &cell.formula {
            if self.depth > 64 || !self.seen.insert((sheet, at.0, at.1)) {
                return None;
            }
            let v = self.formula(sheet, f);
            self.seen.remove(&(sheet, at.0, at.1));
            return v.or_else(|| cell.value.parse().ok().map(Val::Num));
        }
        Some(if cell.number {
            Val::Num(cell.value.parse().ok()?)
        } else if cell.value == "TRUE" || cell.value == "FALSE" {
            Val::Bool(cell.value == "TRUE")
        } else {
            Val::Text(cell.value.clone())
        })
    }

    fn formula(&mut self, sheet: usize, f: &str) -> Option<Val> {
        self.depth += 1;
        let toks = lex(f)?;
        let prev = self.sheet;
        self.sheet = sheet;
        let mut p = Parser { toks: &toks, at: 0 };
        let v = p.expr(self);
        self.sheet = prev;
        self.depth -= 1;
        if p.at != toks.len() {
            return None;
        }
        v
    }

    fn sheet_index(&self, name: &str) -> Option<usize> {
        let name = name.trim_matches('\'');
        self.book.sheets.iter().position(|s| s.name.eq_ignore_ascii_case(name))
    }

    fn reference(&mut self, r: &str) -> Option<Val> {
        let (sheet, r) = match r.rsplit_once('!') {
            Some((s, r)) => (self.sheet_index(s)?, r),
            None => (self.sheet, r),
        };
        if let Some((a, b)) = r.split_once(':') {
            let (a, b) = (parse_ref(a)?, parse_ref(b)?);
            let (r0, r1) = (a.0.min(b.0), a.0.max(b.0));
            let (c0, c1) = (a.1.min(b.1), a.1.max(b.1));
            if (r1 - r0 + 1) as u64 * (c1 - c0 + 1) as u64 > 100_000 {
                return None;
            }
            let mut out = Vec::new();
            for r in r0..=r1 {
                for c in c0..=c1 {
                    out.push(self.cell(sheet, (r, c))?);
                }
            }
            return Some(Val::List(out));
        }
        self.cell(sheet, parse_ref(r)?)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum T {
    Num(f64),
    Str(String),
    Ref(String),
    Name(String),
    Op(char),
    Cmp(&'static str),
    LParen,
    RParen,
    Comma,
}

fn lex(f: &str) -> Option<Vec<T>> {
    let c: Vec<char> = f.trim().trim_start_matches('=').chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        match ch {
            ' ' => i += 1,
            '(' => {
                out.push(T::LParen);
                i += 1;
            }
            ')' => {
                out.push(T::RParen);
                i += 1;
            }
            ',' | ';' => {
                out.push(T::Comma);
                i += 1;
            }
            '+' | '-' | '*' | '/' | '^' | '&' | '%' => {
                out.push(T::Op(ch));
                i += 1;
            }
            '<' | '>' | '=' => {
                let two: String = c[i..(i + 2).min(c.len())].iter().collect();
                let op = match two.as_str() {
                    "<=" => "<=",
                    ">=" => ">=",
                    "<>" => "<>",
                    _ => match ch {
                        '<' => "<",
                        '>' => ">",
                        _ => "=",
                    },
                };
                i += op.len();
                out.push(T::Cmp(op));
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                loop {
                    let ch = *c.get(i)?;
                    i += 1;
                    if ch == '"' {
                        if c.get(i) == Some(&'"') {
                            s.push('"');
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    s.push(ch);
                }
                out.push(T::Str(s));
            }
            d if d.is_ascii_digit() || d == '.' => {
                let s = i;
                while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.' || ((c[i] == 'e' || c[i] == 'E') && i + 1 < c.len())) {
                    if c[i] == 'e' || c[i] == 'E' {
                        i += 1;
                        if c[i] == '+' || c[i] == '-' {
                            i += 1;
                        }
                        continue;
                    }
                    i += 1;
                }
                out.push(T::Num(c[s..i].iter().collect::<String>().parse().ok()?));
            }
            _ if ch.is_alphabetic() || ch == '$' || ch == '\'' || ch == '_' => {
                let s = i;
                if ch == '\'' {
                    i += 1;
                    while i < c.len() && c[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                }
                while i < c.len() && (c[i].is_alphanumeric() || matches!(c[i], '$' | '_' | '.' | '!' | ':')) {
                    i += 1;
                }
                let word: String = c[s..i].iter().collect();
                let bare = word.rsplit('!').next().unwrap_or("");
                let is_ref = bare.split(':').all(|p| parse_ref(p).is_some());
                if c.get(i) == Some(&'(') && !word.contains('!') {
                    out.push(T::Name(word.to_ascii_uppercase()));
                } else if is_ref {
                    out.push(T::Ref(word));
                } else if word.eq_ignore_ascii_case("TRUE") || word.eq_ignore_ascii_case("FALSE") {
                    out.push(T::Name(word.to_ascii_uppercase()));
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

struct Parser<'t> {
    toks: &'t [T],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&T> {
        self.toks.get(self.at)
    }

    fn expr(&mut self, ev: &mut Eval) -> Option<Val> {
        let a = self.concat(ev)?;
        if let Some(T::Cmp(op)) = self.peek().cloned() {
            self.at += 1;
            let b = self.concat(ev)?;
            let ord = match (a.num(), b.num(), &a, &b) {
                (Some(x), Some(y), _, _) => x.partial_cmp(&y)?,
                (_, _, Val::Text(x), Val::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
                _ => return None,
            };
            use std::cmp::Ordering::*;
            return Some(Val::Bool(match op {
                "=" => ord == Equal,
                "<>" => ord != Equal,
                "<" => ord == Less,
                ">" => ord == Greater,
                "<=" => ord != Greater,
                _ => ord != Less,
            }));
        }
        Some(a)
    }

    fn concat(&mut self, ev: &mut Eval) -> Option<Val> {
        let mut a = self.sum(ev)?;
        while self.peek() == Some(&T::Op('&')) {
            self.at += 1;
            let b = self.sum(ev)?;
            a = Val::Text(format!("{}{}", text_of(&a), text_of(&b)));
        }
        Some(a)
    }

    fn sum(&mut self, ev: &mut Eval) -> Option<Val> {
        let mut a = self.product(ev)?;
        while let Some(T::Op(op @ ('+' | '-'))) = self.peek().cloned() {
            self.at += 1;
            let b = self.product(ev)?;
            a = Val::Num(if op == '+' { a.num()? + b.num()? } else { a.num()? - b.num()? });
        }
        Some(a)
    }

    fn product(&mut self, ev: &mut Eval) -> Option<Val> {
        let mut a = self.power(ev)?;
        while let Some(T::Op(op @ ('*' | '/'))) = self.peek().cloned() {
            self.at += 1;
            let b = self.power(ev)?;
            a = Val::Num(if op == '*' {
                a.num()? * b.num()?
            } else {
                let d = b.num()?;
                if d == 0.0 {
                    return None;
                }
                a.num()? / d
            });
        }
        Some(a)
    }

    fn power(&mut self, ev: &mut Eval) -> Option<Val> {
        let a = self.unary(ev)?;
        if self.peek() == Some(&T::Op('^')) {
            self.at += 1;
            let b = self.power(ev)?;
            return Some(Val::Num(a.num()?.powf(b.num()?)));
        }
        Some(a)
    }

    fn unary(&mut self, ev: &mut Eval) -> Option<Val> {
        match self.peek() {
            Some(T::Op('-')) => {
                self.at += 1;
                Some(Val::Num(-self.unary(ev)?.num()?))
            }
            Some(T::Op('+')) => {
                self.at += 1;
                self.unary(ev)
            }
            _ => {
                let v = self.atom(ev)?;
                if self.peek() == Some(&T::Op('%')) {
                    self.at += 1;
                    return Some(Val::Num(v.num()? / 100.0));
                }
                Some(v)
            }
        }
    }

    fn atom(&mut self, ev: &mut Eval) -> Option<Val> {
        let t = self.peek()?.clone();
        self.at += 1;
        match t {
            T::Num(n) => Some(Val::Num(n)),
            T::Str(s) => Some(Val::Text(s)),
            T::Ref(r) => ev.reference(&r),
            T::LParen => {
                let v = self.expr(ev)?;
                (self.peek() == Some(&T::RParen)).then(|| self.at += 1)?;
                Some(v)
            }
            T::Name(n) if n == "TRUE" || n == "FALSE" => Some(Val::Bool(n == "TRUE")),
            T::Name(n) => {
                (self.peek() == Some(&T::LParen)).then(|| self.at += 1)?;
                let mut args = Vec::new();
                if self.peek() != Some(&T::RParen) {
                    loop {
                        args.push(self.expr(ev)?);
                        match self.peek() {
                            Some(T::Comma) => self.at += 1,
                            _ => break,
                        }
                    }
                }
                (self.peek() == Some(&T::RParen)).then(|| self.at += 1)?;
                call(&n, args)
            }
            _ => None,
        }
    }
}

fn text_of(v: &Val) -> String {
    match v {
        Val::Num(n) => fmt_num(*n),
        Val::Text(t) => t.clone(),
        Val::Bool(b) => if *b { "TRUE".into() } else { "FALSE".into() },
        Val::List(l) => l.first().map(text_of).unwrap_or_default(),
    }
}

fn call(name: &str, args: Vec<Val>) -> Option<Val> {
    let mut flat = Vec::new();
    for a in args.iter().cloned() {
        a.flat(&mut flat);
    }
    // Aggregates skip text and blanks, as spreadsheets do.
    let nums: Vec<f64> = flat.iter().filter_map(|v| if let Val::Num(n) = v { Some(*n) } else { None }).collect();
    let arg = |i: usize| args.get(i).and_then(Val::num);
    Some(Val::Num(match name {
        "SUM" => nums.iter().sum(),
        "AVERAGE" => {
            if nums.is_empty() {
                return None;
            }
            nums.iter().sum::<f64>() / nums.len() as f64
        }
        "MIN" => nums.iter().copied().fold(f64::INFINITY, f64::min).min(if nums.is_empty() { 0.0 } else { f64::INFINITY }),
        "MAX" => nums.iter().copied().fold(f64::NEG_INFINITY, f64::max).max(if nums.is_empty() { 0.0 } else { f64::NEG_INFINITY }),
        "COUNT" => nums.len() as f64,
        "COUNTA" => flat.iter().filter(|v| !matches!(v, Val::Text(t) if t.is_empty())).count() as f64,
        "PRODUCT" => nums.iter().product(),
        "ABS" => arg(0)?.abs(),
        "SQRT" => arg(0)?.sqrt(),
        "INT" => arg(0)?.floor(),
        "MOD" => {
            let (a, b) = (arg(0)?, arg(1)?);
            if b == 0.0 {
                return None;
            }
            a - b * (a / b).floor()
        }
        "ROUND" | "ROUNDUP" | "ROUNDDOWN" => {
            let p = 10f64.powi(arg(1).unwrap_or(0.0) as i32);
            let x = arg(0)? * p;
            (match name {
                "ROUND" => x.round(),
                "ROUNDUP" => x.abs().ceil() * x.signum(),
                _ => x.trunc(),
            }) / p
        }
        "IF" => {
            let c = args.first()?;
            let yes = match c {
                Val::Bool(b) => *b,
                v => v.num()? != 0.0,
            };
            return Some(args.get(if yes { 1 } else { 2 }).cloned().unwrap_or(Val::Bool(yes)));
        }
        "CONCAT" | "CONCATENATE" => return Some(Val::Text(flat.iter().map(text_of).collect())),
        _ => return None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Book {
        let mut cells = BTreeMap::new();
        let n = |v: &str| Cell { value: v.into(), formula: None, number: true };
        cells.insert((0, 0), Cell { value: "Item".into(), ..Default::default() });
        cells.insert((1, 1), n("1200"));
        cells.insert((1, 2), n("300.5"));
        cells.insert((1, 3), Cell { formula: Some("SUM(B2:C2)".into()), ..Default::default() });
        cells.insert((2, 3), Cell { formula: Some("D2*2+ROUND(C2,0)/Notes!A1".into()), ..Default::default() });
        cells.insert((3, 3), Cell { formula: Some("D4+1".into()), ..Default::default() });
        cells.insert((4, 3), Cell { formula: Some("IF(B2>1000,\"big\",\"small\")&\"!\"".into()), ..Default::default() });
        let mut notes = BTreeMap::new();
        notes.insert((0, 0), n("2"));
        Book {
            sheets: vec![
                Sheet { name: "Budget".into(), part: String::new(), cells, rows: 5, cols: 4 },
                Sheet { name: "Notes".into(), part: String::new(), cells: notes, rows: 1, cols: 1 },
            ],
        }
    }

    #[test]
    fn evaluates_formulas() {
        let mut b = book();
        assert_eq!(b.display(0, (1, 3)), "1500.5");
        assert_eq!(b.display(0, (2, 3)), "3151.5");
        // Circular: falls back to the formula text.
        assert_eq!(b.display(0, (3, 3)), "=D4+1");
        assert_eq!(b.display(0, (4, 3)), "big!");
        b.set(0, (1, 1), "4500");
        assert_eq!(b.display(0, (1, 3)), "4800.5");
        assert_eq!(b.input(0, (1, 3)), "=SUM(B2:C2)");
        assert_eq!(parse_ref("$AB$10"), Some((9, 27)));
        assert_eq!(cell_name(9, 27), "AB10");
    }

    #[test]
    fn edits_sheet_xml() {
        let src = r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="C1" s="2"><v>1</v></c></row><row r="3"><c r="A3"><v>5</v></c></row></sheetData></worksheet>"#;
        let out = edit_sheet(
            src,
            &[((0, 1), "mid".into()), ((0, 2), "=A3*2".into()), ((1, 0), "7".into()), ((4, 0), "end".into())],
        )
        .unwrap();
        assert_eq!(
            out,
            r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="inlineStr"><is><t xml:space="preserve">mid</t></is></c><c r="C1" s="2"><f>A3*2</f></c></row><row r="2"><c r="A2"><v>7</v></c></row><row r="3"><c r="A3"><v>5</v></c></row><row r="5"><c r="A5" t="inlineStr"><is><t xml:space="preserve">end</t></is></c></row></sheetData></worksheet>"#
        );
        let empty = edit_sheet("<worksheet><sheetData/></worksheet>", &[((0, 0), "1".into())]).unwrap();
        assert_eq!(empty, r#"<worksheet><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#);
        assert!(full_calc(r#"<workbook><sheets/><calcPr calcId="1"/></workbook>"#).contains(r#"fullCalcOnLoad="1""#));
    }
}
