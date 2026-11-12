//! The main window's frost, kept while another window has the keyboard.
//!
//! On Windows the window's glass is the system's Acrylic (src/main.rs
//! `zeron_window`), and DWM draws a system backdrop only for a window whose
//! frame is active: the moment another app is clicked, the blur is gone and
//! the window is a flat grey, which is how it is seen most of the time (beside
//! an editor, a browser). Told that its frame is active again
//! (`WM_NCACTIVATE` with true), DWM draws the backdrop again, whichever
//! window has the keyboard.
//!
//! So the main window is subclassed, and each time its frame is told it's
//! inactive, the default window procedure is told the opposite straight
//! after. Only the default procedure hears that: zgx (winit) hears what the
//! system said, so focus, the caret and shortcuts go on as they were.

use zgx_desktop::WindowContext;

/// Keep the frost of the main window beside `cx` (found once it's there).
pub fn keep(cx: &mut WindowContext) {
    #[cfg(windows)]
    cx.tasks.spawn(async move {
        // (The window is made after its scripts load: looked for, for a while.)
        for _ in 0..150 {
            if windows::subclass_main() {
                return;
            }
            zgx::timer::sleep(std::time::Duration::from_millis(200)).await;
        }
    });
    #[cfg(not(windows))]
    let _ = cx;
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Shell::{DefSubclassProc, SetWindowSubclass},
            WindowsAndMessaging::{
                DefWindowProcW, EnumWindows, GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
                WM_NCACTIVATE, WS_EX_TOPMOST,
            },
        },
    };

    /// The main window: this process's, titled "Keplorayx" alone, shown, and
    /// not one that floats over the others (the island's host is that).
    fn main_window() -> Option<HWND> {
        unsafe extern "system" fn each(h: HWND, out: LPARAM) -> i32 {
            // SAFETY: `out` is the Option this call was given.
            unsafe {
                let mut pid = 0u32;
                GetWindowThreadProcessId(h, &mut pid);
                if pid != std::process::id() || IsWindowVisible(h) == 0 {
                    return 1;
                }
                let mut buf = [0u16; 64];
                let n = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
                let named = n > 0 && String::from_utf16_lossy(&buf[..n as usize]) == "Keplorayx";
                if named && (GetWindowLongPtrW(h, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST) == 0 {
                    *(out as *mut Option<HWND>) = Some(h);
                    return 0;
                }
                1
            }
        }
        let mut found: Option<HWND> = None;
        // SAFETY: the callback writes only `found`.
        unsafe { EnumWindows(Some(each), &mut found as *mut _ as LPARAM) };
        found
    }

    /// The frame told it's inactive is told it's active, by the default
    /// procedure alone (-1: nothing of the frame is drawn again for it).
    unsafe extern "system" fn frame(h: HWND, msg: u32, w: WPARAM, l: LPARAM, _id: usize, _data: usize) -> LRESULT {
        // SAFETY: the subclass chain's own call, with what it was given.
        let answer = unsafe { DefSubclassProc(h, msg, w, l) };
        if msg == WM_NCACTIVATE && w == 0 {
            // SAFETY: our own window, the system's default procedure.
            unsafe { DefWindowProcW(h, WM_NCACTIVATE, 1, -1) };
        }
        answer
    }

    /// Whether the main window was there to subclass (on its own thread,
    /// which is this one: the window's tasks run where its messages do).
    pub fn subclass_main() -> bool {
        let Some(h) = main_window() else { return false };
        // SAFETY: our own window, on its thread; the procedure is a static fn.
        let ok = unsafe { SetWindowSubclass(h, Some(frame), 0x4b58_4652, 0) } != 0;
        // (Already behind another window as it starts: frosted from the start.)
        if ok && unsafe { GetForegroundWindow() } != h {
            // SAFETY: as above.
            unsafe { DefWindowProcW(h, WM_NCACTIVATE, 1, -1) };
        }
        ok
    }
}
