//! Paragraph text editing shared by Word (`w:p`/`w:t`) and PowerPoint
//! (`a:p`/`a:t`): find each paragraph's text runs by byte offset and
//! rewrite only the text, keeping run and paragraph formatting.

use super::xml::{self, Tok};

#[derive(Clone, Copy)]
pub struct Dialect {
    pub p: &'static str,
    pub t: &'static str,
    /// The run to insert into an empty paragraph; `{}` is the text.
    pub run: &'static str,
    /// Paragraph children that must stay after the runs (`a:endParaRPr`).
    pub tail: Option<&'static str>,
    /// Add `xml:space="preserve"` to the rewritten text element (Word).
    pub preserve: bool,
}

pub const WORD: Dialect =
    Dialect { p: "w:p", t: "w:t", run: "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>", tail: None, preserve: true };

pub const DRAWING: Dialect = Dialect {
    p: "a:p",
    t: "a:t",
    run: "<a:r><a:rPr lang=\"en-US\" dirty=\"0\"/><a:t>{}</a:t></a:r>",
    tail: Some("a:endParaRPr"),
    preserve: false,
};

struct TSpan {
    open: (usize, usize),
    /// `None` for `<w:t/>`.
    close: Option<(usize, usize)>,
}

struct PSpan {
    open: (usize, usize),
    close: Option<(usize, usize)>,
    texts: Vec<TSpan>,
    tail: Option<usize>,
}

/// Every paragraph in document order (counting nested ones, as the
/// renderers do).
fn scan(src: &str, d: Dialect) -> Vec<PSpan> {
    let mut out: Vec<PSpan> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut open_t: Option<(usize, usize)> = None;
    for tok in xml::tokens(src) {
        match tok {
            Tok::Open { name, empty, start, end, .. } if name == d.p => {
                out.push(PSpan { open: (start, end), close: None, texts: Vec::new(), tail: None });
                if !empty {
                    stack.push(out.len() - 1);
                }
            }
            Tok::Close { name, start, end } if name == d.p => {
                if let Some(i) = stack.pop() {
                    out[i].close = Some((start, end));
                }
            }
            Tok::Open { name, empty, start, end, .. } if name == d.t => {
                if let Some(&i) = stack.last() {
                    if empty {
                        out[i].texts.push(TSpan { open: (start, end), close: None });
                    } else {
                        open_t = Some((start, end));
                    }
                }
            }
            Tok::Close { name, start, end } if name == d.t => {
                if let (Some(open), Some(&i)) = (open_t.take(), stack.last()) {
                    out[i].texts.push(TSpan { open, close: Some((start, end)) });
                }
            }
            Tok::Open { name, start, .. } if Some(name) == d.tail => {
                if let Some(&i) = stack.last() {
                    out[i].tail.get_or_insert(start);
                }
            }
            _ => {}
        }
    }
    out
}

/// Replace the text of paragraphs `(index, text)` in `src`. The new text
/// goes into the first run (keeping its formatting); later runs empty out.
pub fn set_paragraphs(src: &str, d: Dialect, edits: &[(usize, String)]) -> Result<String, String> {
    let paras = scan(src, d);
    let mut splices = Vec::new();
    for (index, text) in edits {
        let p = paras.get(*index).ok_or_else(|| format!("paragraph {index} is gone"))?;
        let esc = xml::escape(&text.replace(['\r', '\n'], " "));
        let open_tag = |t: &TSpan| {
            let tag = &src[t.open.0..t.open.1];
            let tag = if t.close.is_none() {
                format!("{}>", tag.trim_end_matches('>').trim_end_matches('/'))
            } else {
                tag.to_string()
            };
            if d.preserve { xml::set_attr(&tag, "xml:space", "preserve") } else { tag }
        };
        if let Some(first) = p.texts.first() {
            match first.close {
                Some(close) => splices.push((first.open.0, close.0, format!("{}{esc}", open_tag(first)))),
                None => splices.push((first.open.0, first.open.1, format!("{}{esc}</{}>", open_tag(first), d.t))),
            }
            for t in &p.texts[1..] {
                if let Some(close) = t.close {
                    splices.push((t.open.1, close.0, String::new()));
                }
            }
        } else if !esc.is_empty() {
            let run = d.run.replace("{}", &esc);
            match p.close {
                Some(close) => splices.push((p.tail.unwrap_or(close.0), p.tail.unwrap_or(close.0), run)),
                None => {
                    let tag = src[p.open.0..p.open.1].trim_end_matches('>').trim_end_matches('/');
                    splices.push((p.open.0, p.open.1, format!("{tag}>{run}</{}>", d.p)));
                }
            }
        }
    }
    xml::splice(src, splices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_word_paragraphs() {
        let src = r#"<w:body><w:p><w:pPr/><w:r><w:rPr><w:b/></w:rPr><w:t>Hello</w:t></w:r><w:r><w:t xml:space="preserve"> world</w:t></w:r></w:p><w:p/><w:p><w:pPr/></w:p></w:body>"#;
        let out = set_paragraphs(src, WORD, &[(0, "Hi & bye".into()), (1, "New".into()), (2, "Third".into())]).unwrap();
        assert_eq!(
            out,
            r#"<w:body><w:p><w:pPr/><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">Hi &amp; bye</w:t></w:r><w:r><w:t xml:space="preserve"></w:t></w:r></w:p><w:p><w:r><w:t xml:space="preserve">New</w:t></w:r></w:p><w:p><w:pPr/><w:r><w:t xml:space="preserve">Third</w:t></w:r></w:p></w:body>"#
        );
    }

    #[test]
    fn rewrites_drawing_paragraphs() {
        let src = r#"<a:p><a:r><a:t>Old</a:t></a:r></a:p><a:p><a:endParaRPr lang="en-US"/></a:p>"#;
        let out = set_paragraphs(src, DRAWING, &[(0, "New".into()), (1, "Added".into())]).unwrap();
        assert_eq!(
            out,
            r#"<a:p><a:r><a:t>New</a:t></a:r></a:p><a:p><a:r><a:rPr lang="en-US" dirty="0"/><a:t>Added</a:t></a:r><a:endParaRPr lang="en-US"/></a:p>"#
        );
        assert!(set_paragraphs(src, DRAWING, &[(5, "x".into())]).is_err());
    }
}
