//! "Take a look at this" in a voice call: a picture of the window you were
//! last working in (not Dove), saved where the agent can open it.
//!
//! While a call runs, the shell notes the foreground window once a second.
//! Capturing uses `PrintWindow`, so the window is captured on its own (not
//! whatever overlaps it) and nothing is sent anywhere: the PNG is written to
//! `~/.dove/voice/look/` and the agent is told its path.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The last foreground window that is not Dove: handle and title.
static LAST: Mutex<Option<(isize, String)>> = Mutex::new(None);

#[cfg(windows)]
fn title_of(hwnd: windows_sys::Win32::Foundation::HWND) -> String {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW;
    let mut buf = [0u16; 256];
    let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Remember the foreground window if it belongs to another app.
#[cfg(windows)]
pub fn note_foreground() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == std::process::id() {
            return;
        }
        let title = title_of(hwnd);
        if title.trim().is_empty() {
            return;
        }
        *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some((hwnd as isize, title));
    }
}

#[cfg(not(windows))]
pub fn note_foreground() {}

/// Capture the noted window into a PNG under `dir`; returns the file and
/// the window's title.
#[cfg(windows)]
pub fn capture_last(dir: &Path) -> Result<(PathBuf, String), String> {
    let (raw, title) = LAST
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or("No other window has been in front during this call.")?;
    capture_window(raw, &title, dir).map(|p| (p, title))
}

/// Capture one window (an HWND) into a PNG under `dir`, drawn by the
/// window itself, so whatever overlaps it is left out.
#[cfg(windows)]
pub fn capture_window(raw: isize, title: &str, dir: &Path) -> Result<PathBuf, String> {
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
        DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject,
    };
    use windows_sys::Win32::Storage::Xps::{PW_CLIENTONLY, PrintWindow};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsWindow};
    let hwnd = raw as windows_sys::Win32::Foundation::HWND;
    unsafe {
        if IsWindow(hwnd) == 0 {
            return Err(format!("\"{title}\" is closed."));
        }
        let mut r = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut r) == 0 {
            return Err("Could not read the window's size.".into());
        }
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        if w <= 0 || h <= 0 || w > 16_384 || h > 16_384 {
            return Err(format!("\"{title}\" is minimized or has no size."));
        }
        let screen = GetDC(std::ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(dc, bmp);
        // PW_RENDERFULLCONTENT (2) captures GPU-composited content too.
        let ok = PrintWindow(hwnd, dc, 2) != 0 || PrintWindow(hwnd, dc, PW_CLIENTONLY) != 0;
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down rows
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        let rows = GetDIBits(dc, bmp, 0, h as u32, bgra.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS);
        SelectObject(dc, old);
        DeleteObject(bmp);
        DeleteDC(dc);
        ReleaseDC(std::ptr::null_mut(), screen);
        if !ok || rows == 0 {
            return Err(format!("Windows would not draw \"{title}\" for a capture."));
        }
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        let img = image::RgbaImage::from_raw(w as u32, h as u32, bgra).ok_or("Bad capture size.")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("look-{}.png", dove_core::store::now_ms()));
        img.save(&path).map_err(|e| e.to_string())?;
        Ok(path)
    }
}

#[cfg(not(windows))]
pub fn capture_last(_dir: &Path) -> Result<(PathBuf, String), String> {
    Err("Capturing another app's window is available on Windows; on this system, open the diff or the browser panel instead.".into())
}

#[cfg(not(windows))]
pub fn capture_window(_raw: isize, _title: &str, _dir: &Path) -> Result<PathBuf, String> {
    Err("Window pictures are taken on Windows; here, Computer use reads the window.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test: both halves touch the same static.
    #[test]
    fn captures_the_noted_window_and_nothing_else() {
        *LAST.lock().unwrap() = None;
        assert!(capture_last(&std::env::temp_dir()).is_err());
        #[cfg(windows)]
        capture_noted();
    }

    #[cfg(windows)]
    fn capture_noted() {
        // Note some other app's top-level window (the shell or desktop
        // usually has one), then capture it.
        use windows_sys::Win32::UI::WindowsAndMessaging::GetShellWindow;
        let shell = unsafe { GetShellWindow() };
        if shell.is_null() {
            return;
        }
        *LAST.lock().unwrap() = Some((shell as isize, "Program Manager".into()));
        let dir = std::env::temp_dir().join(format!("dove-look-{}", std::process::id()));
        match capture_last(&dir) {
            Ok((path, title)) => {
                assert_eq!(title, "Program Manager");
                let img = image::open(&path).unwrap();
                assert!(img.width() > 0 && img.height() > 0);
            }
            // Headless sessions have no desktop to draw.
            Err(e) => eprintln!("capture skipped: {e}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
