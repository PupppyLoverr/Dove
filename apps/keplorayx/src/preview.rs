//! Office documents' words, for the Files panel's preview: a Word file's
//! paragraphs, a PowerPoint deck's slides, an Excel book's shared strings.
//! Each is a zip of XML; the words are read from it and nothing is run. A
//! PDF, and anything this can't read, is opened in the system's own viewer
//! from the preview instead.

use std::{io::Read, path::Path};

/// The most of a document read for its words.
const CAP: u64 = 4 * 1024 * 1024;

pub fn document_text(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let file = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let parts: Vec<String> = match ext.as_str() {
        "docx" => vec!["word/document.xml".into()],
        "xlsx" => vec!["xl/sharedStrings.xml".into()],
        "pptx" => {
            let mut slides: Vec<String> =
                zip.file_names().filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml")).map(String::from).collect();
            // slide2 before slide10.
            slides.sort_by_key(|n| n.trim_start_matches("ppt/slides/slide").trim_end_matches(".xml").parse::<u32>().unwrap_or(0));
            slides
        }
        _ => return None,
    };
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        let Ok(entry) = zip.by_name(part) else { continue };
        let mut xml = String::new();
        entry.take(CAP).read_to_string(&mut xml).ok()?;
        if ext == "pptx" {
            out.push_str(&format!("— Slide {} —\n", i + 1));
        }
        out.push_str(&words(&xml, &ext));
        out.push('\n');
    }
    Some(out.trim().to_string())
}

/// The text runs of an Office XML part: `<w:t>`, `<a:t>` or `<t>`, with a
/// paragraph's or a row's end as a line.
fn words(xml: &str, ext: &str) -> String {
    let (run, line) = match ext {
        "docx" => ("w:t", "w:p"),
        "pptx" => ("a:t", "a:p"),
        _ => ("t", "si"),
    };
    let mut out = String::new();
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else { break };
        let tag = &after[..end];
        rest = &after[end + 1..];
        let name = tag.trim_start_matches('/').split([' ', '/']).next().unwrap_or("");
        if tag.starts_with('/') && name == line {
            out.push('\n');
        } else if !tag.starts_with('/') && name == run && !tag.ends_with('/') {
            let close = format!("</{run}>");
            if let Some(stop) = rest.find(&close) {
                out.push_str(&unescape(&rest[..stop]));
                rest = &rest[stop + close.len()..];
            }
        }
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_word_runs_by_paragraph() {
        let xml = r#"<w:document><w:body><w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:t xml:space="preserve"> world</w:t></w:r></w:p><w:p><w:r><w:t>A &amp; B</w:t></w:r></w:p></w:body></w:document>"#;
        assert_eq!(words(xml, "docx"), "Hello world\nA & B\n");
    }

    #[test]
    fn reads_shared_strings() {
        let xml = r#"<sst><si><t>Name</t></si><si><t>Total</t></si></sst>"#;
        assert_eq!(words(xml, "xlsx"), "Name\nTotal\n");
    }
}
