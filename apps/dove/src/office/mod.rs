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

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trips real documents: `DOVE_OFFICE_FIXTURES=<dir with
    /// brief.docx, budget.xlsx, launch.pptx, intake.pdf> cargo test -p dove
    /// office::tests -- --ignored`. Files are copied to a temp folder first.
    #[test]
    #[ignore]
    fn round_trips_fixtures() {
        let Some(src) = std::env::var_os("DOVE_OFFICE_FIXTURES") else { return };
        let src = std::path::PathBuf::from(src);
        let dir = std::env::temp_dir().join(format!("dove-office-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["brief.docx", "budget.xlsx", "launch.pptx", "intake.pdf"] {
            std::fs::copy(src.join(name), dir.join(name)).unwrap();
        }
        let out = |name: &str| std::env::var_os("DOVE_OFFICE_OUT").map(|o| std::path::PathBuf::from(o).join(name));

        // Word: edit the first body paragraph and a table cell.
        let p = dir.join("brief.docx");
        let Document::Docx(d) = load(&p, Kind::Docx).unwrap() else { panic!() };
        let body = d.paras.iter().position(|p| p.kind == docx::ParaKind::Body && !p.text().is_empty()).unwrap();
        let cell =
            d.blocks.iter().find_map(|b| if let docx::Block::Table(r) = b { Some(r[1][1][0]) } else { None }).unwrap();
        docx::save(&p, &[(body, "Edited in Dove & saved.".into()), (cell, "Cell edit".into())]).unwrap();
        let Document::Docx(d2) = load(&p, Kind::Docx).unwrap() else { panic!() };
        assert_eq!(d2.paras[body].text(), "Edited in Dove & saved.");
        assert_eq!(d2.paras[cell].text(), "Cell edit");
        assert_eq!(d2.paras.len(), d.paras.len());

        // Excel: change a number, add a formula and a text cell in a new row.
        let p = dir.join("budget.xlsx");
        let Document::Xlsx(b) = load(&p, Kind::Xlsx).unwrap() else { panic!() };
        let part = b.sheets[0].part.clone();
        let total_before = b.display(0, (1, 3));
        xlsx::save(
            &p,
            &[
                (part.clone(), (1, 1), "9999".into()),
                (part.clone(), (40, 0), "Added".into()),
                (part, (40, 1), "=B2*2".into()),
            ],
        )
        .unwrap();
        let Document::Xlsx(b2) = load(&p, Kind::Xlsx).unwrap() else { panic!() };
        assert_eq!(b2.display(0, (1, 1)), "9999");
        assert_eq!(b2.display(0, (40, 0)), "Added");
        assert_eq!(b2.display(0, (40, 1)), "19998");
        assert_ne!(b2.display(0, (1, 3)), total_before, "SUM follows the edit");

        // PowerPoint: retitle slide 1 and edit a bullet on slide 2.
        let p = dir.join("launch.pptx");
        let Document::Pptx(deck) = load(&p, Kind::Pptx).unwrap() else { panic!() };
        let first_text = |deck: &pptx::Deck, s: usize, nth: usize| {
            deck.slides[s]
                .shapes
                .iter()
                .filter_map(|sh| if let pptx::ShapeKind::Text(ps) = &sh.kind { Some(ps) } else { None })
                .flatten()
                .filter(|p| !p.text.is_empty())
                .nth(nth)
                .cloned()
                .unwrap()
        };
        let (t1, b2p) = (first_text(&deck, 0, 0), first_text(&deck, 1, 1));
        let parts = (deck.slides[0].part.clone(), deck.slides[1].part.clone());
        pptx::save(&p, &[(parts.0, t1.index, "Retitled in Dove".into()), (parts.1, b2p.index, "Edited bullet".into())])
            .unwrap();
        let Document::Pptx(deck2) = load(&p, Kind::Pptx).unwrap() else { panic!() };
        assert_eq!(first_text(&deck2, 0, 0).text, "Retitled in Dove");
        assert_eq!(first_text(&deck2, 1, 1).text, "Edited bullet");
        assert!(deck2.slides.iter().all(|s| !s.shapes.is_empty()));

        // PDF: render pages and fill the form.
        let p = dir.join("intake.pdf");
        let Document::Pdf(pdf) = load(&p, Kind::Pdf).unwrap() else { panic!() };
        assert!(pdf.page_count >= 1 && !pdf.pages.is_empty());
        let names: Vec<&str> = pdf.fields.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"full_name") && names.contains(&"agree"), "{names:?}");
        pdf::save(&p, &[("full_name".into(), "Ada Lovelace".into()), ("agree".into(), "true".into())]).unwrap();
        let Document::Pdf(pdf2) = load(&p, Kind::Pdf).unwrap() else { panic!() };
        // Page content and the filled values both draw into the page image.
        let ink = |pdf: &pdf::Pdf| {
            let img = ::image::load_from_memory(pdf.pages[0].image.bytes()).unwrap().to_luma8();
            img.pixels().filter(|p| p.0[0] < 128).count()
        };
        assert!(ink(&pdf) > 500, "page content renders");
        assert!(ink(&pdf2) > ink(&pdf), "filled values render");
        if let Some(o) = out("intake-page1.png") {
            std::fs::write(o, pdf2.pages[0].image.bytes()).unwrap();
        }
        let get = |n: &str| pdf2.fields.iter().find(|f| f.name == n).map(|f| f.value.clone()).unwrap();
        assert_eq!(get("full_name"), "Ada Lovelace");
        assert_eq!(get("agree"), "true");

        for name in ["brief.docx", "budget.xlsx", "launch.pptx", "intake.pdf"] {
            if let Some(o) = out(name) {
                std::fs::copy(dir.join(name), o).unwrap();
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
