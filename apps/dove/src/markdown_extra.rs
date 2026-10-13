//! Mermaid diagrams and a compact LaTeX → readable math line.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gpui::{Image, ImageFormat};

use crate::theme::Theme;

const MERMAID_MAX: usize = 16 * 1024;

#[derive(Clone)]
pub enum Diagram {
    Pending,
    /// The rendered SVG and its natural size.
    Ready(Arc<Image>, (f32, f32)),
    Failed(String),
}

static MERMAID: std::sync::LazyLock<Mutex<HashMap<u64, Diagram>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn key(source: &str, dark: bool) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    dark.hash(&mut h);
    h.finish()
}

pub fn mermaid(source: &str, theme: &Theme) -> Diagram {
    let dark = theme.appearance.is_dark();
    let k = key(source, dark);
    if let Some(hit) = MERMAID.lock().ok().and_then(|c| c.get(&k).cloned()) {
        return hit;
    }
    let rendered = render_mermaid(source, dark);
    let diagram = match rendered {
        Ok(svg) => {
            let size = svg_size(&svg).unwrap_or((640.0, 360.0));
            Diagram::Ready(Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes())), size)
        }
        Err(e) => Diagram::Failed(e),
    };
    if let Ok(mut cache) = MERMAID.lock() {
        cache.insert(k, diagram.clone());
    }
    diagram
}

fn render_mermaid(source: &str, dark: bool) -> Result<String, String> {
    if source.len() > MERMAID_MAX || source.lines().count() > 256 {
        return Err("Diagram is too large to preview".into());
    }
    std::panic::catch_unwind(|| {
        let mut options = mermaid_rs_renderer::RenderOptions::default();
        options.theme = if dark { mermaid_rs_renderer::Theme::dark() } else { mermaid_rs_renderer::Theme::modern() };
        // Sit on the chat's card, not a grey slab.
        options.theme.background = "transparent".to_string();
        options.layout.node_spacing = 36.0;
        options.layout.rank_spacing = 40.0;
        mermaid_rs_renderer::render_with_options(source, options).map_err(|e| e.to_string())
    })
    .unwrap_or_else(|_| Err("Diagram could not be rendered".into()))
}

/// Rasterize TIFF (and similar) so the chat and file viewer can show them.
pub fn preview_image(path: &str) -> Option<std::sync::Arc<Image>> {
    let lower = path.to_ascii_lowercase();
    if !(lower.ends_with(".tif") || lower.ends_with(".tiff")) {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(Arc::new(Image::from_bytes(ImageFormat::Png, png)))
}

/// A readable line of math from a small LaTeX subset (fractions, scripts, Greek).
pub fn math_line(src: &str) -> String {
    let mut s = src.trim().trim_matches('$').trim().to_string();
    for (pat, repl) in [
        ("\\cdot", "·"),
        ("\\times", "×"),
        ("\\div", "÷"),
        ("\\pm", "±"),
        ("\\infty", "∞"),
        ("\\rightarrow", "→"),
        ("\\leftarrow", "←"),
        ("\\Rightarrow", "⇒"),
        ("\\leq", "≤"),
        ("\\geq", "≥"),
        ("\\neq", "≠"),
        ("\\approx", "≈"),
        ("\\sum", "∑"),
        ("\\prod", "∏"),
        ("\\int", "∫"),
        ("\\partial", "∂"),
        ("\\sqrt", "√"),
        ("\\alpha", "α"),
        ("\\beta", "β"),
        ("\\gamma", "γ"),
        ("\\delta", "δ"),
        ("\\epsilon", "ε"),
        ("\\theta", "θ"),
        ("\\lambda", "λ"),
        ("\\mu", "μ"),
        ("\\pi", "π"),
        ("\\sigma", "σ"),
        ("\\phi", "φ"),
        ("\\omega", "ω"),
        ("\\Gamma", "Γ"),
        ("\\Delta", "Δ"),
        ("\\Theta", "Θ"),
        ("\\Lambda", "Λ"),
        ("\\Pi", "Π"),
        ("\\Sigma", "Σ"),
        ("\\Omega", "Ω"),
        ("\\mathbb{R}", "ℝ"),
        ("\\mathbb{N}", "ℕ"),
        ("\\mathbb{Z}", "ℤ"),
        ("\\ldots", "…"),
        ("\\dots", "…"),
    ] {
        s = s.replace(pat, repl);
    }
    s = fold_frac(&s);
    s = s.replace('{', "").replace('}', "");
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fold_frac(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if src[i..].starts_with("\\frac") {
            i += 5;
            let (a, n) = braced(&src[i..]);
            i += n;
            let (b, m) = braced(&src[i..]);
            i += m;
            out.push('(');
            out.push_str(a);
            out.push('/');
            out.push_str(b);
            out.push(')');
        } else {
            out.push(src[i..].chars().next().unwrap());
            i += src[i..].chars().next().unwrap().len_utf8();
        }
    }
    out
}

fn braced(src: &str) -> (&str, usize) {
    let src = src.trim_start();
    let skipped = src.len().saturating_sub(src.trim_start().len());
    if !src.starts_with('{') {
        let end = src.find(|c: char| c.is_whitespace() || c == '\\').unwrap_or(src.len().min(1));
        return (&src[..end], skipped + end);
    }
    let mut depth = 0i32;
    for (i, c) in src.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return (&src[1..i], skipped + i + 1);
                }
            }
            _ => {}
        }
    }
    (src, src.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_and_greek() {
        assert_eq!(math_line(r"\frac{a}{b} + \pi"), "(a/b) + π");
    }
}

/// An SVG's natural size from its `viewBox` (or width/height).
fn svg_size(svg: &str) -> Option<(f32, f32)> {
    let head = &svg[..svg.find('>')?];
    let attr = |name: &str| {
        let at = head.find(&format!(" {name}=\""))? + name.len() + 3;
        let end = head[at..].find('"')? + at;
        Some(head[at..end].to_string())
    };
    if let Some(vb) = attr("viewBox") {
        let n: Vec<f32> = vb.split([' ', ',']).filter_map(|x| x.parse().ok()).collect();
        if n.len() == 4 && n[2] > 0.0 && n[3] > 0.0 {
            return Some((n[2], n[3]));
        }
    }
    let num = |v: String| v.trim_end_matches("px").parse::<f32>().ok();
    Some((num(attr("width")?)?, num(attr("height")?)?))
}

#[cfg(test)]
mod svg_tests {
    #[test]
    fn reads_viewbox() {
        assert_eq!(super::svg_size(r#"<svg xmlns="x" viewBox="0 0 300 120">"#), Some((300.0, 120.0)));
        assert_eq!(super::svg_size(r#"<svg width="80px" height="40">"#), Some((80.0, 40.0)));
    }
}
