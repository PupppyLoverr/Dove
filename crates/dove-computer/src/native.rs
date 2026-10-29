//! What only the OS can answer: text read off pixels (Windows.Media.Ocr,
//! Apple Vision, tesseract on Linux) and, on Windows, whether a window's
//! field is a password field (UI Automation `IsPassword`, which stock
//! cua-driver rows do not carry).

use crate::host::OcrLine;

/// Text lines in an encoded PNG or JPEG, in its pixel space.
pub fn ocr(image: &[u8]) -> Result<Vec<OcrLine>, String> {
    imp::ocr(image)
}

/// Whether text would land in a password field of window `window_id`:
/// the field labelled `label` when given, else the window's focused field.
/// When the window has password fields and the target cannot be told,
/// that counts as yes. None off Windows (the tree's roles cover it there).
pub fn secure_target(window_id: u64, label: Option<&str>) -> Option<bool> {
    imp::secure_target(window_id, label)
}

#[cfg(windows)]
mod imp {
    use super::OcrLine;
    use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, TreeScope_Descendants, UIA_IsPasswordPropertyId,
    };

    fn com() {
        // S_FALSE or RPC_E_CHANGED_MODE: this thread already has COM.
        // SAFETY: plain COM initialization for the calling thread.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
    }

    pub fn ocr(image: &[u8]) -> Result<Vec<OcrLine>, String> {
        com();
        let run = || -> windows::core::Result<Vec<OcrLine>> {
            let stream = InMemoryRandomAccessStream::new()?;
            let writer = DataWriter::CreateDataWriter(&stream)?;
            writer.WriteBytes(image)?;
            writer.StoreAsync()?.get()?;
            writer.FlushAsync()?.get()?;
            writer.DetachStream()?;
            stream.Seek(0)?;
            let decoder = BitmapDecoder::CreateAsync(&stream)?.get()?;
            let bitmap = decoder.GetSoftwareBitmapAsync()?.get()?;
            let bitmap =
                SoftwareBitmap::ConvertWithAlpha(&bitmap, BitmapPixelFormat::Bgra8, BitmapAlphaMode::Premultiplied)?;
            let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;
            let result = engine.RecognizeAsync(&bitmap)?.get()?;
            let mut out = Vec::new();
            for line in result.Lines()? {
                let text = line.Text()?.to_string();
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for word in line.Words()? {
                    let r = word.BoundingRect()?;
                    x0 = x0.min(r.X);
                    y0 = y0.min(r.Y);
                    x1 = x1.max(r.X + r.Width);
                    y1 = y1.max(r.Y + r.Height);
                }
                if x1 > x0 && y1 > y0 {
                    out.push(OcrLine { text, x: x0 as i64, y: y0 as i64, w: (x1 - x0) as i64, h: (y1 - y0) as i64 });
                }
            }
            Ok(out)
        };
        run().map_err(|e| format!("Windows OCR failed: {e}"))
    }

    pub fn secure_target(window_id: u64, label: Option<&str>) -> Option<bool> {
        com();
        // SAFETY: COM calls on interfaces this function creates and owns;
        // the window handle is only read by UI Automation.
        unsafe {
            let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
            let root = uia.ElementFromHandle(HWND(window_id as usize as *mut core::ffi::c_void)).ok()?;
            let cond = uia.CreatePropertyCondition(UIA_IsPasswordPropertyId, &VARIANT::from(true)).ok()?;
            let found = root.FindAll(TreeScope_Descendants, &cond).ok()?;
            let n = found.Length().ok()?;
            if n == 0 {
                return Some(false);
            }
            let mut any_focused = false;
            for i in 0..n {
                let Ok(e) = found.GetElement(i) else { continue };
                if let Some(want) = label.map(str::trim).filter(|l| !l.is_empty())
                    && e.CurrentName().is_ok_and(|name| name.to_string().trim().eq_ignore_ascii_case(want))
                {
                    return Some(true);
                }
                if e.CurrentHasKeyboardFocus().is_ok_and(|f| f.as_bool()) {
                    any_focused = true;
                }
            }
            // A named target that is none of the password fields is fine;
            // an unnamed one in a window with password fields is not known.
            Some(any_focused || label.is_none_or(|l| l.trim().is_empty()))
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::OcrLine;
    use objc2::AnyThread as _;
    use objc2::rc::Retained;
    use objc2_foundation::{NSArray, NSData, NSDictionary};
    use objc2_vision::{VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel};

    pub fn ocr(image: &[u8]) -> Result<Vec<OcrLine>, String> {
        let (w, h) = crate::parse::image_dimensions(image).ok_or("unreadable image")?;
        let data = NSData::with_bytes(image);
        let options = NSDictionary::new();
        // SAFETY: Vision request objects created and used on this thread only.
        unsafe {
            let handler = VNImageRequestHandler::initWithData_options(VNImageRequestHandler::alloc(), &data, &options);
            let request = VNRecognizeTextRequest::new();
            request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
            request.setUsesLanguageCorrection(true);
            let as_request: Retained<VNRequest> = Retained::into_super(Retained::into_super(request.clone()));
            let requests = NSArray::from_retained_slice(&[as_request]);
            handler.performRequests_error(&requests).map_err(|e| format!("Vision OCR failed: {e:?}"))?;
            let mut out = Vec::new();
            for obs in request.results().into_iter().flatten() {
                let Some(top) = obs.topCandidates(1).firstObject() else { continue };
                let text = top.string().to_string();
                // Normalized, origin bottom-left: flip into image pixels.
                let b = obs.boundingBox();
                let (fw, fh) = (w as f64, h as f64);
                let x = b.origin.x * fw;
                let y = (1.0 - b.origin.y - b.size.height) * fh;
                out.push(OcrLine {
                    text,
                    x: x as i64,
                    y: y as i64,
                    w: (b.size.width * fw) as i64,
                    h: (b.size.height * fh) as i64,
                });
            }
            Ok(out)
        }
    }

    pub fn secure_target(_window_id: u64, _label: Option<&str>) -> Option<bool> {
        None
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::OcrLine;

    /// `tesseract <png> stdout tsv`, word rows grouped into lines.
    pub fn ocr(image: &[u8]) -> Result<Vec<OcrLine>, String> {
        let dir = std::env::temp_dir().join(format!("dove-ocr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("frame.png");
        crate::capture::write_private(&path, image).map_err(|e| e.to_string())?;
        let out = std::process::Command::new("tesseract")
            .arg(&path)
            .args(["stdout", "--psm", "11", "tsv"])
            .stdin(std::process::Stdio::null())
            .output();
        let _ = std::fs::remove_dir_all(&dir);
        let out = out.map_err(|_| "tesseract is not installed (sudo apt install tesseract-ocr)".to_string())?;
        if !out.status.success() {
            return Err(format!("tesseract failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(parse_tsv(&String::from_utf8_lossy(&out.stdout)))
    }

    pub fn parse_tsv(tsv: &str) -> Vec<OcrLine> {
        use std::collections::BTreeMap;
        let mut lines: BTreeMap<(i64, i64, i64), (Vec<String>, i64, i64, i64, i64)> = BTreeMap::new();
        for row in tsv.lines().skip(1) {
            let c: Vec<&str> = row.split('\t').collect();
            if c.len() < 12 || c[0] != "5" || c[11].trim().is_empty() {
                continue;
            }
            let n = |i: usize| c[i].trim().parse::<i64>().unwrap_or(0);
            let conf = c[10].trim().parse::<f64>().unwrap_or(0.0);
            if conf < 40.0 {
                continue;
            }
            let (x, y, w, h) = (n(6), n(7), n(8), n(9));
            let e = lines.entry((n(2), n(3), n(4))).or_insert((Vec::new(), i64::MAX, i64::MAX, i64::MIN, i64::MIN));
            e.0.push(c[11].trim().to_string());
            e.1 = e.1.min(x);
            e.2 = e.2.min(y);
            e.3 = e.3.max(x + w);
            e.4 = e.4.max(y + h);
        }
        lines
            .into_values()
            .map(|(words, x0, y0, x1, y1)| OcrLine { text: words.join(" "), x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
            .collect()
    }

    pub fn secure_target(_window_id: u64, _label: Option<&str>) -> Option<bool> {
        None
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn tsv_words_become_lines() {
            let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                5\t1\t1\t1\t1\t1\t10\t20\t30\t12\t95\tSave\n\
                5\t1\t1\t1\t1\t2\t45\t20\t40\t12\t90\tchanges\n\
                5\t1\t1\t1\t2\t1\t10\t50\t20\t12\t10\tnoise\n";
            let l = super::parse_tsv(tsv);
            assert_eq!(l.len(), 1);
            assert_eq!((l[0].text.as_str(), l[0].x, l[0].w), ("Save changes", 10, 75));
        }
    }
}
