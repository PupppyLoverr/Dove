//! PDF: pages drawn by macOS's own PDF engine (PDFKit into a CoreGraphics
//! bitmap, nothing bundled), and AcroForm text/checkbox fields read and
//! filled through PDFKit. Other platforms get an honest message instead.

use std::path::Path;
use std::sync::Arc;

#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // read by the macOS PDFKit backend
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Check,
    Choice,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub kind: FieldKind,
    /// Text value, or `"true"`/`"false"` for checkboxes.
    pub value: String,
}

pub struct Page {
    pub image: Arc<gpui::Image>,
    /// Height / width.
    pub aspect: f32,
}

pub struct Pdf {
    pub pages: Vec<Page>,
    pub page_count: usize,
    pub fields: Vec<Field>,
}

/// Pages rendered up front; the rest are counted and left to "Open in editor".
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // read by the macOS PDFKit backend
pub const MAX_PAGES: usize = 40;
/// Rendered page width in pixels (2x the panel width, for Retina).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // read by the macOS PDFKit backend
const RENDER_WIDTH: f64 = 1200.0;

pub fn load(path: &Path) -> Result<Pdf, String> {
    imp::load(path)
}

/// Fill fields `(name, value)` and write the file in place.
pub fn save(path: &Path, edits: &[(String, String)]) -> Result<(), String> {
    imp::save(path, edits)
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;
    use std::path::Path;
    use std::sync::Arc;

    use objc2::encode::{Encode, Encoding};
    use objc2::msg_send;
    use objc2::rc::{Retained, autoreleasepool};
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::{NSRect, NSString, NSURL};

    use super::{Field, FieldKind, MAX_PAGES, Page, Pdf, RENDER_WIDTH};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        w: f64,
        h: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGAffineTransform {
        a: f64,
        b: f64,
        c: f64,
        d: f64,
        tx: f64,
        ty: f64,
    }

    /// kPDFDisplayBoxCropBox.
    const CROP_BOX: isize = 1;
    // kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big: RGBA bytes.
    const RGBA: u32 = 1 | (4 << 12);

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
        fn CGColorSpaceRelease(cs: *mut c_void);
        fn CGBitmapContextCreate(
            data: *mut c_void,
            w: usize,
            h: usize,
            bpc: usize,
            bpr: usize,
            cs: *mut c_void,
            info: u32,
        ) -> *mut c_void;
        fn CGContextRelease(ctx: *mut c_void);
        fn CGContextSetRGBFillColor(ctx: *mut c_void, r: f64, g: f64, b: f64, a: f64);
        fn CGContextFillRect(ctx: *mut c_void, rect: CGRect);
        fn CGContextConcatCTM(ctx: *mut c_void, t: CGAffineTransform);
    }

    #[link(name = "PDFKit", kind = "framework")]
    unsafe extern "C" {}

    /// A `CGContextRef` argument for PDFKit.
    #[repr(transparent)]
    #[derive(Clone, Copy)]
    struct CGContextRef(*mut c_void);

    unsafe impl Encode for CGContextRef {
        const ENCODING: Encoding = Encoding::Pointer(&Encoding::Struct("CGContext", &[]));
    }

    /// Draw one `PDFPage` into an RGBA bitmap. PDFKit draws annotations and
    /// form widgets too, so filled-in values show on the page.
    fn render_page(page: &AnyObject) -> Option<Page> {
        let rect: NSRect = unsafe { msg_send![page, boundsForBox: CROP_BOX] };
        let rot: isize = unsafe { msg_send![page, rotation] };
        let (pw, ph) = if rot.rem_euclid(180) == 90 {
            (rect.size.height, rect.size.width)
        } else {
            (rect.size.width, rect.size.height)
        };
        if pw <= 0.0 || ph <= 0.0 {
            return None;
        }
        let w = RENDER_WIDTH as usize;
        let h = ((RENDER_WIDTH * ph / pw).round() as usize).clamp(1, 8000);
        let mut buf = vec![0u8; w * h * 4];
        unsafe {
            let cs = CGColorSpaceCreateDeviceRGB();
            let ctx = CGBitmapContextCreate(buf.as_mut_ptr().cast(), w, h, 8, w * 4, cs, RGBA);
            CGColorSpaceRelease(cs);
            if ctx.is_null() {
                return None;
            }
            let full = CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { w: w as f64, h: h as f64 } };
            CGContextSetRGBFillColor(ctx, 1.0, 1.0, 1.0, 1.0);
            CGContextFillRect(ctx, full);
            let s = w as f64 / pw;
            CGContextConcatCTM(ctx, CGAffineTransform { a: s, b: 0.0, c: 0.0, d: s, tx: 0.0, ty: 0.0 });
            // PDFKit applies the crop box origin and the page rotation.
            let _: () = msg_send![page, drawWithBox: CROP_BOX, toContext: CGContextRef(ctx)];
            CGContextRelease(ctx);
        }
        let img = image::RgbaImage::from_raw(w as u32, h as u32, buf)?;
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .ok()?;
        Some(Page { image: Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, png)), aspect: (ph / pw) as f32 })
    }

    fn url(path: &Path) -> Retained<NSURL> {
        NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
    }

    fn pdf_document(path: &Path) -> Option<Retained<AnyObject>> {
        let cls = AnyClass::get(c"PDFDocument")?;
        let url = url(path);
        unsafe {
            let obj: *mut AnyObject = msg_send![cls, alloc];
            let obj: *mut AnyObject = msg_send![obj, initWithURL: &*url];
            Retained::from_raw(obj)
        }
    }

    fn string(obj: &AnyObject, sel: &str) -> Option<String> {
        let s: Option<Retained<NSString>> = unsafe {
            match sel {
                "fieldName" => msg_send![obj, fieldName],
                "widgetFieldType" => msg_send![obj, widgetFieldType],
                "widgetStringValue" => msg_send![obj, widgetStringValue],
                _ => msg_send![obj, type],
            }
        };
        s.map(|s| s.to_string())
    }

    /// Every form widget: `(annotation, field name, kind)`.
    fn widgets(doc: &AnyObject) -> Vec<(Retained<AnyObject>, String, FieldKind)> {
        let mut out = Vec::new();
        let pages: usize = unsafe { msg_send![doc, pageCount] };
        for i in 0..pages {
            let page: Option<Retained<AnyObject>> = unsafe { msg_send![doc, pageAtIndex: i] };
            let Some(page) = page else { continue };
            let anns: Option<Retained<AnyObject>> = unsafe { msg_send![&*page, annotations] };
            let Some(anns) = anns else { continue };
            let n: usize = unsafe { msg_send![&*anns, count] };
            for k in 0..n {
                let ann: Retained<AnyObject> = unsafe { msg_send![&*anns, objectAtIndex: k] };
                if string(&ann, "type").as_deref() != Some("Widget") {
                    continue;
                }
                let Some(name) = string(&ann, "fieldName").filter(|n| !n.is_empty()) else { continue };
                let kind = match string(&ann, "widgetFieldType").as_deref() {
                    Some("/Tx") => FieldKind::Text,
                    Some("/Btn") => {
                        // kPDFWidgetCheckBoxControl; push and radio buttons are skipped.
                        let control: isize = unsafe { msg_send![&*ann, widgetControlType] };
                        if control != 2 {
                            continue;
                        }
                        FieldKind::Check
                    }
                    Some("/Ch") => FieldKind::Choice,
                    _ => continue,
                };
                out.push((ann, name, kind));
            }
        }
        out
    }

    pub fn load(path: &Path) -> Result<Pdf, String> {
        let (count, pages) = autoreleasepool(|_| {
            let doc = pdf_document(path).ok_or("Not a readable PDF (damaged or encrypted)")?;
            let locked: bool = unsafe { msg_send![&*doc, isLocked] };
            if locked {
                return Err("This PDF is password protected".to_string());
            }
            let count: usize = unsafe { msg_send![&*doc, pageCount] };
            let mut pages = Vec::new();
            for i in 0..count.min(MAX_PAGES) {
                let page: Option<Retained<AnyObject>> = unsafe { msg_send![&*doc, pageAtIndex: i] };
                if let Some(p) = page.as_deref().and_then(render_page) {
                    pages.push(p);
                }
            }
            Ok::<_, String>((count, pages))
        })?;
        let fields = autoreleasepool(|_| {
            let Some(doc) = pdf_document(path) else { return Vec::new() };
            let mut fields: Vec<Field> = Vec::new();
            for (ann, name, kind) in widgets(&doc) {
                if fields.iter().any(|f| f.name == name) {
                    continue;
                }
                let value = match kind {
                    FieldKind::Check => {
                        let state: isize = unsafe { msg_send![&*ann, buttonWidgetState] };
                        if state != 0 { "true".into() } else { "false".into() }
                    }
                    _ => string(&ann, "widgetStringValue").unwrap_or_default(),
                };
                fields.push(Field { name, kind, value });
            }
            fields
        });
        Ok(Pdf { page_count: count, pages, fields })
    }

    pub fn save(path: &Path, edits: &[(String, String)]) -> Result<(), String> {
        autoreleasepool(|_| {
            let doc = pdf_document(path).ok_or("PDFKit could not open this PDF")?;
            for (ann, name, kind) in widgets(&doc) {
                let Some((_, value)) = edits.iter().rev().find(|(n, _)| *n == name) else { continue };
                unsafe {
                    match kind {
                        FieldKind::Check => {
                            let on: isize = if value == "true" { 1 } else { 0 };
                            let _: () = msg_send![&*ann, setButtonWidgetState: on];
                        }
                        _ => {
                            let v = NSString::from_str(value);
                            let _: () = msg_send![&*ann, setWidgetStringValue: &*v];
                        }
                    }
                }
            }
            let tmp = path.with_extension("pdf.dove-save");
            let ok: bool = unsafe { msg_send![&*doc, writeToURL: &*url(&tmp)] };
            if !ok {
                let _ = std::fs::remove_file(&tmp);
                return Err("PDFKit could not write the PDF".to_string());
            }
            std::fs::rename(&tmp, path).map_err(|e| e.to_string())
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::path::Path;

    use super::Pdf;

    pub fn load(_: &Path) -> Result<Pdf, String> {
        Err("PDF pages render with macOS's built-in PDF engine. On this platform, open the file in your PDF viewer."
            .into())
    }

    pub fn save(_: &Path, _: &[(String, String)]) -> Result<(), String> {
        Err("Filling PDF forms in Dove needs macOS's PDFKit.".into())
    }
}
