//! A minimal XML tokenizer for OOXML parts that keeps byte offsets, so
//! edits splice new text into the original markup and everything Dove does
//! not understand (styles, themes, extensions) survives a save untouched.

#[derive(Clone, Copy, Debug)]
pub enum Tok<'a> {
    Open { name: &'a str, attrs: &'a str, empty: bool, start: usize, end: usize },
    Close { name: &'a str, start: usize, end: usize },
    Text { start: usize, end: usize },
}

pub fn tokens(s: &str) -> Vec<Tok<'_>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            let j = s[i..].find('<').map_or(b.len(), |k| i + k);
            out.push(Tok::Text { start: i, end: j });
            i = j;
            continue;
        }
        let rest = &s[i..];
        let skip_to = |pat: &str| rest.find(pat).map_or(b.len(), |k| i + k + pat.len());
        if rest.starts_with("<!--") {
            i = skip_to("-->");
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            i = skip_to("]]>");
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            i = skip_to(">");
            continue;
        }
        let mut j = i + 1;
        let mut quote = 0u8;
        while j < b.len() {
            let c = b[j];
            if quote != 0 {
                if c == quote {
                    quote = 0;
                }
            } else if c == b'"' || c == b'\'' {
                quote = c;
            } else if c == b'>' {
                break;
            }
            j += 1;
        }
        let end = (j + 1).min(b.len());
        let inner = &s[i + 1..j.min(b.len())];
        if let Some(name) = inner.strip_prefix('/') {
            out.push(Tok::Close { name: name.trim(), start: i, end });
        } else {
            let empty = inner.ends_with('/');
            let inner = if empty { &inner[..inner.len() - 1] } else { inner };
            let n = inner.find(char::is_whitespace).unwrap_or(inner.len());
            out.push(Tok::Open { name: &inner[..n], attrs: &inner[n..], empty, start: i, end });
        }
        i = end;
    }
    out
}

/// The name without its namespace prefix.
pub fn local(name: &str) -> &str {
    name.rfind(':').map_or(name, |k| &name[k + 1..])
}

/// The unescaped value of attribute `key` (matched with its prefix).
pub fn attr(attrs: &str, key: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        rest = rest.trim_start();
        let eq = rest.find('=')?;
        let name = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let q = after.chars().next()?;
        if q != '"' && q != '\'' {
            return None;
        }
        let close = after[1..].find(q)? + 1;
        if name == key {
            return Some(unescape(&after[1..close]));
        }
        rest = &after[close + 1..];
    }
}

pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(k) = rest.find('&') {
        out.push_str(&rest[..k]);
        let tail = &rest[k..];
        let Some(semi) = tail.find(';').filter(|&n| n <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Escape for text content and attribute values. Drops characters XML 1.0
/// cannot carry.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

/// Apply `(start, end, replacement)` splices. They must not overlap.
pub fn splice(src: &str, mut edits: Vec<(usize, usize, String)>) -> Result<String, String> {
    edits.sort_by_key(|e| (e.0, e.1));
    let mut out = String::with_capacity(src.len() + 256);
    let mut at = 0;
    for (start, end, text) in edits {
        if start < at || end < start || end > src.len() {
            return Err("overlapping edits".into());
        }
        out.push_str(&src[at..start]);
        out.push_str(&text);
        at = end;
    }
    out.push_str(&src[at..]);
    Ok(out)
}

/// Set (or add) attribute `key="value"` on the open tag `src[start..end]`.
pub fn set_attr(tag: &str, key: &str, value: &str) -> String {
    let attr = format!("{key}=\"{}\"", escape(value));
    if let Some(k) = find_attr(tag, key) {
        let (s, e) = k;
        return format!("{}{attr}{}", &tag[..s], &tag[e..]);
    }
    let close = if tag.ends_with("/>") { tag.len() - 2 } else { tag.len() - 1 };
    format!("{} {attr}{}", tag[..close].trim_end(), &tag[close..])
}

fn find_attr(tag: &str, key: &str) -> Option<(usize, usize)> {
    let mut from = 0;
    while let Some(k) = tag[from..].find(key) {
        let s = from + k;
        let before_ok = tag[..s].ends_with(char::is_whitespace);
        let after = &tag[s + key.len()..];
        let trimmed = after.trim_start();
        if before_ok && trimmed.starts_with('=') {
            let v = trimmed[1..].trim_start();
            let q = v.chars().next()?;
            let vstart = tag.len() - v.len();
            let vend = vstart + 1 + v[1..].find(q)? + 1;
            return Some((s, vend));
        }
        from = s + key.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_with_offsets() {
        let s = r#"<?xml version="1.0"?><a:p x="1"><a:t>Hi &amp; bye</a:t><a:br/></a:p>"#;
        let t = tokens(s);
        let names: Vec<String> = t
            .iter()
            .map(|t| match t {
                Tok::Open { name, empty, .. } => format!("<{name}{}", if *empty { "/" } else { "" }),
                Tok::Close { name, .. } => format!("</{name}"),
                Tok::Text { start, end } => unescape(&s[*start..*end]),
            })
            .collect();
        assert_eq!(names, ["<a:p", "<a:t", "Hi & bye", "</a:t", "<a:br/", "</a:p"]);
        assert_eq!(local("a:p"), "p");
        assert_eq!(attr(r#" x="1" r:id='rId2' id="7""#, "r:id").as_deref(), Some("rId2"));
        assert_eq!(attr(r#" x="1" r:id='rId2' id="7""#, "id").as_deref(), Some("7"));
    }

    #[test]
    fn sets_attributes() {
        assert_eq!(set_attr(r#"<calcPr calcId="1"/>"#, "fullCalcOnLoad", "1"), r#"<calcPr calcId="1" fullCalcOnLoad="1"/>"#);
        assert_eq!(set_attr(r#"<c r="A1" s="2">"#, "s", "3"), r#"<c r="A1" s="3">"#);
        assert_eq!(unescape("a&#x41;&#66;&lt;&bogus"), "aAB<&bogus");
    }
}
