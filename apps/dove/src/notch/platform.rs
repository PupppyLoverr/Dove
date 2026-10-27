//! What GPUI doesn't give the notch: where the screen's top edge is in
//! device pixels, where the pointer is while another app has focus, whether
//! a full-screen app is in front, a borderless topmost window with frosted
//! glass behind it and a shape of its own, keyboard focus on demand, the
//! clipboard's change counter, and dragging a file out to other apps.
//! Windows only for now; elsewhere [`SUPPORTED`] is false and the notch
//! stays off.

/// The display the notch lives on, in device pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Display {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    /// Device pixels per logical pixel.
    pub scale: f32,
}

impl Display {
    /// Horizontal center, in device pixels.
    pub fn center_x(&self) -> i32 {
        self.left + self.width / 2
    }
}

/// A rounded rectangle in the window, logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub r: f32,
}

#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
pub use other::*;

#[cfg(windows)]
mod windows {
    use super::{Display, Shape};
    use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
        DwmSetWindowAttribute,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST,
        MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, MonitorFromWindow, RGN_OR, SetWindowRgn,
    };
    use windows_sys::Win32::System::DataExchange::{
        IsClipboardFormatAvailable, RegisterClipboardFormatW,
    };
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId};
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
    use windows_sys::Win32::UI::Shell::{
        QUERY_USER_NOTIFICATION_STATE, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
        SHQueryUserNotificationState,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetCursorPos, GetForegroundWindow,
        GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, HWND_TOPMOST, IsWindow, IsZoomed, SW_HIDE,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SetForegroundWindow,
        SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos, ShowWindow, WDA_EXCLUDEFROMCAPTURE, WS_CAPTION,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP, WS_VISIBLE,
    };

    pub const SUPPORTED: bool = true;

    /// The notch's native window.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Handle(isize);

    impl Handle {
        pub fn of(window: &gpui::Window) -> Option<Self> {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let handle = HasWindowHandle::window_handle(window).ok()?;
            match handle.as_raw() {
                RawWindowHandle::Win32(h) => Some(Self(h.hwnd.get())),
                _ => None,
            }
        }

        fn hwnd(self) -> HWND {
            self.0 as HWND
        }

        pub fn raw(self) -> isize {
            self.0
        }

        /// A plain topmost popup that a click doesn't activate (focus stays in
        /// the user's app), with no taskbar button or Alt-Tab entry, and
        /// frosted glass (acrylic) behind it.
        ///
        /// GPUI gives a transparent window a caption style and a
        /// "transparent gradient" accent; together they make DWM fill the
        /// whole rectangle black with a shadow and small rounded corners. A
        /// popup with the acrylic accent instead, clipped to the island by
        /// [`Handle::shape`], shows the blurred desktop and nothing else.
        pub fn prepare(self) {
            let hwnd = self.hwnd();
            // SAFETY: a live top-level window owned by this process.
            unsafe {
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW) as isize);
                let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
                SetWindowLongPtrW(hwnd, GWL_STYLE, (WS_POPUP | (style & WS_VISIBLE)) as isize);
                // ACCENT_DISABLED. DWM's acrylic would blur the whole window
                // rectangle (it ignores the region), so the island paints its
                // own frosted glass from [`capture_behind`].
                set_accent(hwnd, 0, 0);
                // Keep the island out of screen captures, its own included, so
                // the glass never blurs itself. `DOVE_NOTCH_CAPTURABLE=1` (QA
                // screenshots) leaves it in.
                if std::env::var_os("DOVE_NOTCH_CAPTURABLE").is_none() {
                    SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
                }
                let corner = DWMWCP_DONOTROUND;
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                    (&raw const corner).cast(),
                    size_of_val(&corner) as u32,
                );
                let border = DWMWA_COLOR_NONE;
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_BORDER_COLOR as u32,
                    (&raw const border).cast(),
                    size_of_val(&border) as u32,
                );
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    0,
                    0,
                    SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }

        /// Shows the window at the top center of `display`, `width` × `height`
        /// logical pixels, above everything, without activating it.
        pub fn place(self, display: Display, width: f32, height: f32) {
            let w = (width * display.scale).round() as i32;
            let h = (height * display.scale).round() as i32;
            // SAFETY: as above.
            unsafe {
                SetWindowPos(
                    self.hwnd(),
                    HWND_TOPMOST,
                    display.center_x() - w / 2,
                    display.top,
                    w,
                    h,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
        }

        /// Clips the window (its glass, its paint and its hit testing) to
        /// `shapes`, so clicks around the island reach the app below.
        pub fn shape(self, shapes: &[Shape], scale: f32) {
            // SAFETY: regions are created here and handed to the window,
            // which owns the combined one; the parts are deleted.
            unsafe {
                let all = CreateRectRgn(0, 0, 0, 0);
                for s in shapes {
                    let (x, y) = ((s.x * scale).round() as i32, (s.y * scale).round() as i32);
                    let (w, h) = ((s.w * scale).round() as i32, (s.h * scale).round() as i32);
                    if w <= 0 || h <= 0 {
                        continue;
                    }
                    let d = ((s.r * 2.0 * scale).round() as i32).max(1);
                    let part = CreateRoundRectRgn(x, y, x + w + 1, y + h + 1, d, d);
                    CombineRgn(all, all, part, RGN_OR);
                    DeleteObject(part);
                }
                if SetWindowRgn(self.hwnd(), all, 1) == 0 {
                    DeleteObject(all);
                }
            }
        }

        pub fn hide(self) {
            // SAFETY: as above.
            unsafe {
                ShowWindow(self.hwnd(), SW_HIDE);
            }
        }

        /// Takes keyboard focus so the island's fields can be typed in.
        /// Returns the window that had it, to hand back later.
        pub fn activate(self) -> Option<isize> {
            let hwnd = self.hwnd();
            // SAFETY: as above; the foreground window is only compared and
            // stored.
            unsafe {
                let prev = GetForegroundWindow();
                if prev == hwnd {
                    return None;
                }
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_NOACTIVATE as isize));
                // The click that got here was ours, so Windows lets us take the
                // foreground; attaching to its input queue covers the rest.
                let theirs = GetWindowThreadProcessId(prev, std::ptr::null_mut());
                let ours = GetCurrentThreadId();
                let attached = theirs != 0 && theirs != ours && AttachThreadInput(ours, theirs, 1) != 0;
                BringWindowToTop(hwnd);
                SetForegroundWindow(hwnd);
                if attached {
                    AttachThreadInput(ours, theirs, 0);
                }
                (!prev.is_null()).then_some(prev as isize)
            }
        }

        /// Gives the keyboard back to `prev` (if it's still there) and makes
        /// clicks inert again.
        pub fn deactivate(self, prev: Option<isize>) {
            let hwnd = self.hwnd();
            // SAFETY: as above; `prev` is checked with IsWindow first.
            unsafe {
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE as isize);
                if GetForegroundWindow() == hwnd
                    && let Some(p) = prev
                    && IsWindow(p as HWND) != 0
                {
                    SetForegroundWindow(p as HWND);
                }
            }
        }

        pub fn is_foreground(self) -> bool {
            // SAFETY: a plain query.
            unsafe { GetForegroundWindow() == self.hwnd() }
        }

        /// Drags `path` out to other apps (Explorer, chat apps, upload
        /// fields) with the shell's own data object, as Explorer does. Blocks
        /// until the drop; call it from a fresh task, not inside an update.
        pub fn drag_file(self, path: &std::path::Path) -> bool {
            drag::file(path).is_ok()
        }
    }

    /// Sets the window's accent (`SetWindowCompositionAttribute`, which
    /// user32 exports without a header; GPUI calls it the same way).
    unsafe fn set_accent(hwnd: HWND, state: u32, color: u32) {
        #[repr(C)]
        struct AccentPolicy {
            state: u32,
            flags: u32,
            color: u32,
            animation: u32,
        }
        #[repr(C)]
        struct Data {
            attribute: u32,
            data: *mut std::ffi::c_void,
            size: usize,
        }
        /// `WCA_ACCENT_POLICY`.
        const ACCENT_POLICY: u32 = 19;
        type SetAttribute = unsafe extern "system" fn(HWND, *mut Data) -> i32;
        // SAFETY: user32 is loaded in every GUI process; the export has this
        // signature on every Windows version that has it, and a missing one
        // is skipped.
        unsafe {
            let user32 = GetModuleHandleW(windows_sys::w!("user32.dll"));
            let Some(f) = GetProcAddress(user32, windows_sys::s!("SetWindowCompositionAttribute")) else { return };
            let f: SetAttribute = std::mem::transmute(f);
            let mut policy = AccentPolicy { state, flags: 2, color, animation: 0 };
            let mut data =
                Data { attribute: ACCENT_POLICY, data: (&raw mut policy).cast(), size: size_of::<AccentPolicy>() };
            f(hwnd, &mut data);
        }
    }

    /// The screen behind the notch's window (left, top, width, height in
    /// device pixels), shrunk by `shrink` as GDI copies it (halftone
    /// filtering, so the full-size frame is never read), as BGRA, the notch
    /// itself excluded: the source of its frosted glass. Safe on any thread.
    pub fn capture_behind(x: i32, y: i32, w: i32, h: i32, shrink: i32) -> Option<image::RgbaImage> {
        use windows_sys::Win32::Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS,
            DeleteDC, GetDC, GetDIBits, HALFTONE, ReleaseDC, SRCCOPY, SelectObject, SetBrushOrgEx, SetStretchBltMode,
            StretchBlt,
        };
        let (sw, sh) = (w, h);
        let (w, h) = ((w / shrink.max(1)).max(1), (h / shrink.max(1)).max(1));
        if sw <= 0 || sh <= 0 {
            return None;
        }
        // SAFETY: GDI objects are created, used and released here; the
        // buffer is sized for `w * h` 32-bit pixels.
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            if screen.is_null() {
                return None;
            }
            let mem = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bitmap);
            SetStretchBltMode(mem, HALFTONE);
            SetBrushOrgEx(mem, 0, 0, std::ptr::null_mut());
            let copied = StretchBlt(mem, 0, 0, w, h, screen, x, y, sw, sh, SRCCOPY) != 0;
            SelectObject(mem, old);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                // Negative: rows top-down.
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut buf = vec![0u8; (w * h * 4) as usize];
            let rows = if copied {
                GetDIBits(mem, bitmap, 0, h as u32, buf.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS)
            } else {
                0
            };
            DeleteObject(bitmap);
            DeleteDC(mem);
            ReleaseDC(std::ptr::null_mut(), screen);
            if rows == 0 {
                return None;
            }
            for px in buf.chunks_exact_mut(4) {
                px[3] = 255;
            }
            image::RgbaImage::from_raw(w as u32, h as u32, buf)
        }
    }

    /// The display the island lives on: the one under the pointer, or the
    /// primary. Its top is below a taskbar docked at the top.
    pub fn display_for(pointer: bool) -> Option<Display> {
        // SAFETY: plain queries with out-parameters sized here.
        unsafe {
            let monitor = match (pointer, cursor()) {
                (true, Some((x, y))) => MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST),
                _ => MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY),
            };
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return None;
            }
            let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
            if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) != 0 {
                dpi_x = 96;
            }
            let (m, w) = (info.rcMonitor, info.rcWork);
            Some(Display { left: m.left, top: w.top.max(m.top), width: m.right - m.left, scale: dpi_x as f32 / 96.0 })
        }
    }

    /// The pointer, in device pixels.
    pub fn cursor() -> Option<(i32, i32)> {
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: one out-parameter.
        (unsafe { GetCursorPos(&mut p) } != 0).then_some((p.x, p.y))
    }

    /// A mouse button is held: a window or a file is being dragged, or text
    /// is being selected.
    pub fn button_down() -> bool {
        // SAFETY: no pointers.
        unsafe { GetAsyncKeyState(VK_LBUTTON as i32) < 0 || GetAsyncKeyState(VK_RBUTTON as i32) < 0 }
    }

    /// The clipboard holds something its owner asked history tools not to
    /// keep (password managers mark secrets this way).
    pub fn clipboard_concealed() -> bool {
        // SAFETY: registering a name returns the existing id; no pointers kept.
        unsafe {
            [
                windows_sys::w!("ExcludeClipboardContentFromMonitorProcessing"),
                windows_sys::w!("Clipboard Viewer Ignore"),
                windows_sys::w!("CanIncludeInClipboardHistory"),
            ]
            .into_iter()
            .any(|name| {
                let id = RegisterClipboardFormatW(name);
                id != 0 && IsClipboardFormatAvailable(id) != 0
            })
        }
    }

    /// A full-screen app (a game, a video, a presentation, a browser after
    /// F11) is in front on `display`. A maximized window with a title bar
    /// isn't full screen, even when an auto-hidden taskbar lets it cover the
    /// whole monitor.
    pub fn fullscreen(display: Display) -> bool {
        // SAFETY: plain queries on the foreground window, with out-parameters
        // sized here.
        unsafe {
            let mut state: QUERY_USER_NOTIFICATION_STATE = 0;
            if SHQueryUserNotificationState(&mut state) == 0
                && (state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE)
            {
                return true;
            }
            let fg = GetForegroundWindow();
            if fg.is_null() {
                return false;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(fg, &mut pid);
            if pid == GetCurrentProcessId() {
                return false;
            }
            let mut class = [0u16; 32];
            let len = GetClassNameW(fg, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
            let class = String::from_utf16_lossy(&class[..len]);
            // The desktop and the taskbar cover the monitor without being an app.
            if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
                return false;
            }
            let style = GetWindowLongPtrW(fg, GWL_STYLE) as u32;
            if IsZoomed(fg) != 0 && style & WS_CAPTION == WS_CAPTION {
                return false;
            }
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST), &mut info) == 0 {
                return false;
            }
            let m = info.rcMonitor;
            if m.left != display.left || m.right - m.left != display.width {
                return false;
            }
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(fg, &mut r) != 0
                && r.left <= m.left
                && r.top <= m.top
                && r.right >= m.right
                && r.bottom >= m.bottom
        }
    }

    /// OLE drag source over the shell's data object for a file.
    mod drag {
        use ::windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS};
        use ::windows::Win32::System::Com::{IBindCtx, IDataObject};
        use ::windows::Win32::System::Ole::{
            DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DoDragDrop, IDropSource, IDropSource_Impl,
        };
        use ::windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
        use ::windows::Win32::UI::Shell::{BHID_DataObject, IShellItem, SHCreateItemFromParsingName};
        use ::windows::core::{BOOL, HRESULT, HSTRING, implement};

        #[implement(IDropSource)]
        struct Source;

        impl IDropSource_Impl for Source_Impl {
            fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
                if escape.as_bool() {
                    DRAGDROP_S_CANCEL
                } else if keys.0 & MK_LBUTTON.0 == 0 {
                    DRAGDROP_S_DROP
                } else {
                    HRESULT(0)
                }
            }

            fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
                DRAGDROP_S_USEDEFAULTCURSORS
            }
        }

        pub fn file(path: &std::path::Path) -> ::windows::core::Result<()> {
            // SAFETY: COM calls on the UI thread, which GPUI initialized for
            // OLE (its own drop target needs it).
            unsafe {
                let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None::<&IBindCtx>)?;
                let data: IDataObject = item.BindToHandler(None::<&IBindCtx>, &BHID_DataObject)?;
                let source: IDropSource = Source.into();
                let mut effect = DROPEFFECT(0);
                DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_LINK, &mut effect).ok()
            }
        }
    }
}

#[cfg(not(windows))]
mod other {
    use super::{Display, Shape};

    pub const SUPPORTED: bool = false;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Handle;

    impl Handle {
        pub fn of(_: &gpui::Window) -> Option<Self> {
            None
        }
        pub fn prepare(self) {}
        pub fn place(self, _: Display, _: f32, _: f32) {}
        pub fn shape(self, _: &[Shape], _: f32) {}
        pub fn hide(self) {}
        pub fn raw(self) -> isize {
            0
        }
        pub fn activate(self) -> Option<isize> {
            None
        }
        pub fn deactivate(self, _: Option<isize>) {}
        pub fn is_foreground(self) -> bool {
            false
        }
        pub fn drag_file(self, _: &std::path::Path) -> bool {
            false
        }
    }

    pub fn display_for(_: bool) -> Option<Display> {
        None
    }
    pub fn capture_behind(_: i32, _: i32, _: i32, _: i32, _: i32) -> Option<image::RgbaImage> {
        None
    }
    pub fn cursor() -> Option<(i32, i32)> {
        None
    }
    pub fn button_down() -> bool {
        false
    }
    pub fn clipboard_concealed() -> bool {
        false
    }
    pub fn fullscreen(_: Display) -> bool {
        false
    }
}
