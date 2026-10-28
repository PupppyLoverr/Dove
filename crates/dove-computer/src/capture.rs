//! Turning a capture into what the model reads (Hermes `_capture_response`):
//! a capped, compact row list, notes, and the image only when it is new.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde_json::json;
use sha2::{Digest as _, Sha256};

use crate::parse::image_dimensions;
use crate::types::{Capture, Output, UiElement};

/// Rows shown inline; the rest go to the spill file.
pub const MAX_ROWS: usize = 100;
/// A label identifies a control; chat clients put whole messages in labels.
pub const MAX_LABEL: usize = 120;
/// Captures and spills kept on disk.
const MAX_FILES: usize = 20;
/// Providers reject images smaller than this.
const MIN_IMAGE_SIDE: u32 = 8;
/// Identical frames sent without pixels in a row before a full resend.
const MAX_UNCHANGED_STREAK: u32 = 2;

/// Per-session state across captures.
#[derive(Default)]
pub struct Shaper {
    last: HashMap<(String, String), (String, u32)>,
}

fn clip(s: &str, n: usize) -> String {
    let flat: String = s.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
    if flat.chars().count() <= n { flat } else { format!("{}…", flat.chars().take(n).collect::<String>()) }
}

/// One row: `#7 Button "Save" @ (12, 80, 28, 28)`.
pub fn row(e: &UiElement) -> String {
    let mut out = format!("#{} {}", e.index, e.role);
    if !e.label.is_empty() {
        out.push_str(&format!(" {:?}", clip(&e.label, MAX_LABEL)));
    }
    if !e.value.is_empty() && e.value != e.label {
        out.push_str(&format!(" = {:?}", clip(&e.value, 60)));
    }
    if e.bounds_known() {
        let (x, y, w, h) = e.bounds;
        out.push_str(&format!(" @ ({x}, {y}, {w}, {h})"));
    }
    if !e.enabled {
        out.push_str(" (disabled)");
    }
    if e.source == "ocr" {
        out.push_str(" [ocr text: click acts by pixel]");
    }
    out
}

/// When element bounds live in another space than the screenshot (HiDPI
/// trees in native pixels, screenshots downscaled): the scale and a note.
pub fn bounds_hint(elements: &[UiElement], w: u32, h: u32) -> Option<(f64, String)> {
    if elements.is_empty() || w == 0 || h == 0 {
        return None;
    }
    let (mx, my) = elements
        .iter()
        .filter(|e| e.bounds_known())
        .fold((0i64, 0i64), |(mx, my), e| (mx.max(e.bounds.0 + e.bounds.2), my.max(e.bounds.1 + e.bounds.3)));
    // 5% slack: window chrome can hang a few pixels past the frame.
    if (mx as f64) <= w as f64 * 1.05 && (my as f64) <= h as f64 * 1.05 {
        return None;
    }
    let scale = ((mx as f64 / w as f64).max(my as f64 / h as f64) * 100.0).round() / 100.0;
    Some((
        scale,
        format!(
            "element bounds are in native coordinates (up to {mx}x{my}), not screenshot pixels ({w}x{h}); \
             click by element index, or multiply screenshot positions by ~{scale}"
        ),
    ))
}

fn bounded_write(dir: &Path, prefix: &str, ext: &str, bytes: &[u8]) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let mut old: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    old.sort();
    for (_, p) in old.iter().take(old.len().saturating_sub(MAX_FILES - 1)) {
        let _ = std::fs::remove_file(p);
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let path = dir.join(format!("{prefix}{stamp:x}{ext}"));
    write_private(&path, bytes).ok()?;
    Some(path)
}

/// Owner-only file (captures are as private as the screen they show).
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
        f.write_all(bytes)
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
    }
}

impl Shaper {
    /// Forget dedup state (the next capture sends pixels again).
    pub fn reset(&mut self) {
        self.last.clear();
    }

    /// Whether this frame repeats the last one for the same target.
    fn unchanged(&mut self, target: (String, String), digest: String) -> bool {
        match self.last.get_mut(&target) {
            Some((d, streak)) if *d == digest && *streak < MAX_UNCHANGED_STREAK => {
                *streak += 1;
                true
            }
            _ => {
                self.last.insert(target, (digest, 0));
                false
            }
        }
    }

    /// The capture as the model reads it.
    pub fn render(&mut self, cap: &Capture, cache: &Path) -> Output {
        let raw = cap.image.as_deref().and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok());
        let dims = raw.as_deref().and_then(image_dimensions);
        let (w, h) = dims.unwrap_or((cap.width, cap.height));
        let too_small = dims.is_some_and(|(w, h)| w.min(h) < MIN_IMAGE_SIDE);
        let has_image = raw.is_some() && cap.mode != "ax" && !too_small;
        let mime = cap
            .image_mime
            .clone()
            .unwrap_or_else(|| if cap.image.as_deref().is_some_and(|b| b.starts_with("/9j/")) { "image/jpeg" } else { "image/png" }.into());

        let mut head = format!("capture mode={}", cap.mode);
        if w > 0 && h > 0 {
            head.push_str(&format!(" {w}x{h}"));
        }
        if !cap.app.is_empty() {
            head.push_str(&format!(" app={}", cap.app));
        }
        if !cap.window_title.is_empty() {
            head.push_str(&format!(" window={:?}", clip(&cap.window_title, 120)));
        }
        if let (Some(pid), Some(win)) = (cap.pid, cap.window_id) {
            head.push_str(&format!(" pid={pid} window_id={win}"));
        }
        let mut lines = vec![head];
        if let Some(diff) = &cap.diff {
            lines.push("changes since your last capture of this window:".into());
            lines.push(diff.trim_end().to_string());
            lines.push("(element indices below are fresh)".into());
        }
        lines.push(format!("{} interactable element(s):", cap.elements.len()));
        let mut notes = Vec::new();
        if let Some((_, note)) = bounds_hint(&cap.elements, w, h) {
            notes.push(note);
        }
        if !cap.note.is_empty() {
            notes.push(cap.note.clone());
        }
        if let Some(why) = cap.image_reason {
            notes.push(why.to_string());
        }
        let lost = cap.elements.len() > MAX_ROWS || cap.elements.iter().any(|e| e.label.chars().count() > MAX_LABEL);
        if lost {
            let full = json!({
                "app": cap.app, "window_title": cap.window_title, "total_elements": cap.elements.len(),
                "elements": cap.elements.iter().map(|e| json!({"index": e.index, "role": e.role, "label": e.label,
                    "value": e.value, "bounds": [e.bounds.0, e.bounds.1, e.bounds.2, e.bounds.3], "source": e.source})).collect::<Vec<_>>(),
            });
            if let Some(p) = bounded_write(&cache.join("elements"), "elements_", ".json", full.to_string().as_bytes()) {
                notes.push(format!(
                    "{} element list with untruncated labels saved to {}; read that file for rows beyond the cap",
                    if cap.truncated { "the" } else { "the full" },
                    p.display()
                ));
            }
        }
        if cap.truncated {
            notes.push("the accessibility walk stopped at its element budget; pass query= or app= to narrow".into());
        }
        if too_small {
            notes.push(format!("screenshot omitted: {}x{} is below the provider minimum", w, h));
        }
        lines.extend(notes.into_iter().map(|n| format!("  ({n})")));
        lines.extend(cap.elements.iter().take(MAX_ROWS).map(|e| format!("  {}", row(e))));
        if cap.elements.len() > MAX_ROWS {
            lines.push(format!("  … +{} more (narrow with query= or read the saved list)", cap.elements.len() - MAX_ROWS));
        }

        let mut out = Output::default();
        if has_image {
            let raw = raw.unwrap_or_default();
            let digest = format!("{:x}", Sha256::digest(&raw));
            if self.unchanged((cap.app.clone(), cap.window_title.clone()), digest) {
                lines.push(
                    "  (screen unchanged since the previous capture: image omitted; the previous screenshot still \
                     shows the current state. Element indices above are fresh.)"
                        .into(),
                );
            } else {
                let ext = if mime == "image/jpeg" { ".jpg" } else { ".png" };
                if let Some(p) = bounded_write(&cache.join("captures"), "capture_", ext, &raw) {
                    lines.push(format!("  (screenshot saved to {}; share that path if the user asks to see it)", p.display()));
                }
                out.images.push((cap.image.clone().unwrap_or_default(), mime));
            }
        }
        out.text = lines.join("\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(i: u32, label: &str, bounds: (i64, i64, i64, i64)) -> UiElement {
        UiElement { index: i, role: "Button".into(), label: label.into(), bounds, enabled: true, source: "tree", ..Default::default() }
    }

    fn png(w: u32, h: u32) -> String {
        let mut img = image::RgbImage::new(w, h);
        img.put_pixel(0, 0, image::Rgb([w as u8, 1, 2]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
        base64::engine::general_purpose::STANDARD.encode(out.into_inner())
    }

    #[test]
    fn rows_are_compact_and_capped() {
        let dir = tempfile::tempdir().unwrap();
        let long = "x".repeat(300);
        let mut els: Vec<UiElement> = (1..=120).map(|i| el(i, "Save", (1, 2, 3, 4))).collect();
        els[0].label = long;
        let cap = Capture { mode: "auto".into(), elements: els, app: "Notepad".into(), ..Default::default() };
        let out = Shaper::default().render(&cap, dir.path());
        assert!(out.text.contains("#2 Button \"Save\" @ (1, 2, 3, 4)"));
        assert!(out.text.contains("… +20 more"));
        assert!(out.text.contains("element list with untruncated labels saved to"));
        assert!(!out.text.contains(&"x".repeat(200)));
        assert!(out.images.is_empty(), "auto mode without an image sends none");
    }

    #[test]
    fn identical_frames_are_sent_once_then_capped() {
        let dir = tempfile::tempdir().unwrap();
        let cap = Capture { mode: "som".into(), image: Some(png(40, 30)), app: "A".into(), ..Default::default() };
        let mut s = Shaper::default();
        assert_eq!(s.render(&cap, dir.path()).images.len(), 1);
        let again = s.render(&cap, dir.path());
        assert!(again.images.is_empty() && again.text.contains("screen unchanged"));
        assert!(s.render(&cap, dir.path()).images.is_empty());
        assert_eq!(s.render(&cap, dir.path()).images.len(), 1, "full pixels again after the streak cap");
        let other = Capture { image: Some(png(41, 30)), ..cap.clone() };
        assert_eq!(s.render(&other, dir.path()).images.len(), 1);
        // ax mode never carries pixels; tiny images are dropped.
        assert!(s.render(&Capture { mode: "ax".into(), ..other.clone() }, dir.path()).images.is_empty());
        let tiny = Capture { image: Some(png(4, 4)), ..cap };
        assert!(s.render(&tiny, dir.path()).text.contains("below the provider minimum"));
    }

    #[test]
    fn hidpi_bounds_are_flagged() {
        let els = vec![el(1, "a", (0, 0, 2880, 1800))];
        let (scale, note) = bounds_hint(&els, 1440, 900).unwrap();
        assert_eq!(scale, 2.0);
        assert!(note.contains("native coordinates"));
        assert!(bounds_hint(&[el(1, "a", (0, 0, 1440, 900))], 1440, 900).is_none());
    }
}
