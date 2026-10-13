//! Office documents in the Files panel: native preview and editing of
//! .docx, .xlsx, .pptx and .pdf, with no bundled runtimes. OOXML is read
//! and edited in place (only the touched XML changes; everything else in
//! the package is copied byte for byte); PDF uses the OS's PDF engine.

pub mod docx;
pub mod pdf;
pub mod pptx;
pub mod view;
pub mod xlsx;

mod package;
mod text;
mod xml;

use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Docx,
    Xlsx,
    Pptx,
    Pdf,
}

impl Kind {
    pub fn of(path: &str) -> Option<Kind> {
        match path.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
            "docx" | "docm" => Some(Kind::Docx),
            "xlsx" | "xlsm" => Some(Kind::Xlsx),
            "pptx" | "pptm" => Some(Kind::Pptx),
            "pdf" => Some(Kind::Pdf),
            _ => None,
        }
    }

    /// The matching skill in `skills/`.
    pub fn skill(self) -> &'static str {
        match self {
            Kind::Docx => "docx",
            Kind::Xlsx => "xlsx",
            Kind::Pptx => "pptx",
            Kind::Pdf => "pdf",
        }
    }
}

pub enum Document {
    Docx(docx::Docx),
    Xlsx(xlsx::Book),
    Pptx(pptx::Deck),
    Pdf(pdf::Pdf),
}

pub fn load(path: &Path, kind: Kind) -> Result<Document, String> {
    Ok(match kind {
        Kind::Docx => Document::Docx(docx::load(path)?),
        Kind::Xlsx => Document::Xlsx(xlsx::load(path)?),
        Kind::Pptx => Document::Pptx(pptx::load(path)?),
        Kind::Pdf => Document::Pdf(pdf::load(path)?),
    })
}

/// An embedded picture as a GPUI image (formats GPUI cannot draw, like
/// EMF/WMF, are skipped).
fn image(pkg: &package::Package, part: &str) -> Option<Arc<gpui::Image>> {
    let ext = part.rsplit('.').next()?.to_ascii_lowercase();
    let bytes = pkg.read(part)?;
    let format = match ext.as_str() {
        "png" => gpui::ImageFormat::Png,
        "jpg" | "jpeg" => gpui::ImageFormat::Jpeg,
        "gif" => gpui::ImageFormat::Gif,
        "webp" => gpui::ImageFormat::Webp,
        "svg" => gpui::ImageFormat::Svg,
        "bmp" | "tif" | "tiff" => {
            let img = ::image::load_from_memory(&bytes).ok()?;
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), ::image::ImageFormat::Png).ok()?;
            return Some(Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png)));
        }
        _ => return None,
    };
    Some(Arc::new(gpui::Image::from_bytes(format, bytes)))
}
