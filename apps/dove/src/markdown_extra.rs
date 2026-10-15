//! Mermaid diagrams and a compact LaTeX → readable math line.

use std::sync::{Arc, Mutex};

use gpui::{Image, ImageFormat};

use crate::theme::Theme;

const MERMAID_MAX: usize = 16 * 1024;

#[derive(Clone)]
pub enum Diagram {
    /// The rendered SVG and its natural size.
    Ready(Arc<Image>, (f32, f32)),
    Failed(String),
}

/// Rendered diagrams kept (each holds an SVG and, once drawn, its raster).
pub const MERMAID_CACHE: usize = 64;
/// What the watchdog trims the diagram cache to.
const MERMAID_FLOOR: usize = 8;

struct Diagrams {
    cache: crate::memory::Lru<u64, Diagram>,
    /// Images that left the cache, whose decoded rasters are released on
    /// the next frame (that needs the app).
    evicted: Vec<Arc<Image>>,
}

static MERMAID: std::sync::LazyLock<Mutex<Diagrams>> = std::sync::LazyLock::new(|| {
    Mutex::new(Diagrams { cache: crate::memory::Lru::new(MERMAID_CACHE, usize::MAX), evicted: Vec::new() })
});

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
    if let Some(hit) = MERMAID.lock().ok().and_then(|mut c| c.cache.get(&k).cloned()) {
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
    if let Ok(mut d) = MERMAID.lock() {
        let out = d.cache.insert(k, diagram.clone(), 0);
        d.evicted.extend(out.into_iter().filter_map(|(_, v)| match v {
            Diagram::Ready(img, _) => Some(img),
            Diagram::Failed(_) => None,
        }));
    }
    diagram
}

/// Free the rasters of diagrams that left the cache. Call with a window
/// being drawn (the transcript does, each frame; it is free when empty).
pub fn release_evicted(window: &mut gpui::Window, cx: &mut gpui::App) {
    release(Some(window), cx);
}

fn release(mut window: Option<&mut gpui::Window>, cx: &mut gpui::App) {
    let gone = match MERMAID.lock() {
        Ok(mut d) if !d.evicted.is_empty() => std::mem::take(&mut d.evicted),
        _ => return,
    };
    for img in gone {
        gpui::ImageSource::Image(img).evict(window.as_deref_mut(), cx);
    }
}

/// The watchdog's trim: keep only the most recent few diagrams.
pub fn trim(window: Option<&mut gpui::Window>, cx: &mut gpui::App) {
    if let Ok(mut d) = MERMAID.lock() {
        let out = d.cache.shrink_to(MERMAID_FLOOR, usize::MAX, None);
        d.evicted.extend(out.into_iter().filter_map(|(_, v)| match v {
            Diagram::Ready(img, _) => Some(img),
            Diagram::Failed(_) => None,
        }));
    }
    release(window, cx);
}

fn render_mermaid(source: &str, dark: bool) -> Result<String, String> {
    if source.len() > MERMAID_MAX || source.lines().count() > 256 {
        return Err("Diagram is too large to preview".into());
    }
    std::panic::catch_unwind(|| {
        let mut theme = if dark { mermaid_rs_renderer::Theme::dark() } else { mermaid_rs_renderer::Theme::modern() };
        // Sit on the chat's card, not a grey slab.
        theme.background = "transparent".to_string();
        let mut options = mermaid_rs_renderer::RenderOptions { theme, ..Default::default() };
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
