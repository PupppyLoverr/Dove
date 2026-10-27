//! The island's Windows system layer, all event-driven so a resting island
//! does nothing:
//!
//! - [`events`]: one hidden window on a thread of its own receives raw mouse
//!   input (only while the mouse moves, coalesced to 60 Hz with a trailing
//!   update), clipboard changes, foreground and full-screen changes, the
//!   global hotkey, and display and setting changes.
//! - [`Audio`]: the default output's volume, its live peak level (sampled
//!   only while something shows it) and its name.
//! - File icons, listening dev-server ports, window capture, paste-back and
//!   the "transparency effects" setting.
//!
//! Other platforms get inert versions; the island is Windows-only for now.

/// Something the island should look at again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The pointer moved.
    Pointer,
    /// Something was copied.
    Clipboard,
    /// Another app came forward, or the front window moved or resized
    /// (entering or leaving full screen).
    Foreground,
    /// The island's global shortcut (Ctrl+Alt+Space).
    Hotkey,
    /// Ctrl+Alt+A (true) or Ctrl+Alt+D (false) while an approval shows
    /// ([`approval_keys`]).
    Approve(bool),
    /// Displays or system settings changed.
    Display,
}

/// A listening TCP port and what owns it.
#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub port: u16,
    pub pid: u32,
    pub process: String,
}

#[cfg(windows)]
pub use imp::*;

#[cfg(not(windows))]
pub use other::*;

#[cfg(windows)]
mod imp {
    use std::cell::Cell;
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use super::{Event, Port};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::DataExchange::AddClipboardFormatListener;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey, VK_A, VK_D, VK_SPACE,
    };
    use windows_sys::Win32::UI::Input::{RAWINPUTDEVICE, RIDEV_INPUTSINK, RegisterRawInputDevices};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_FOREGROUND,
        GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, KillTimer, MSG, OBJID_WINDOW, RegisterClassExW,
        PostMessageW, SetTimer, TranslateMessage, WINEVENT_OUTOFCONTEXT, WM_APP, WM_CLIPBOARDUPDATE, WM_DISPLAYCHANGE,
        WM_HOTKEY, WM_INPUT, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
    };

    static SINK: OnceLock<flume::Sender<Event>> = OnceLock::new();
    /// The event window, for messages from other threads.
    static WINDOW: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
    /// Register (wparam 1) or drop (0) the approval hotkeys.
    const WM_APPROVAL_KEYS: u32 = WM_APP + 1;
    const HOTKEY_TOGGLE: i32 = 1;
    const HOTKEY_ALLOW: i32 = 2;
    const HOTKEY_DENY: i32 = 3;

    /// Ctrl+Alt+A / Ctrl+Alt+D answer the showing approval from any app.
    /// Registered only while one shows, so they're never taken otherwise.
    pub fn approval_keys(on: bool) {
        let hwnd = WINDOW.load(std::sync::atomic::Ordering::Acquire);
        if hwnd != 0 {
            // SAFETY: posting to our own live window.
            unsafe { PostMessageW(hwnd as HWND, WM_APPROVAL_KEYS, on as usize, 0) };
        }
    }

    /// Pointer events at most this often; a trailing one follows the last move.
    const POINTER_EVERY: Duration = Duration::from_millis(16);
    const TRAILING_TIMER: usize = 1;

    thread_local! {
        static LAST_POINTER: Cell<Option<Instant>> = const { Cell::new(None) };
        static TRAILING: Cell<bool> = const { Cell::new(false) };
        static LOCATION_HOOK: Cell<HWINEVENTHOOK> = const { Cell::new(std::ptr::null_mut()) };
        static HOOK_PID: Cell<u32> = const { Cell::new(0) };
    }

    fn send(e: Event) {
        if let Some(tx) = SINK.get() {
            let _ = tx.send(e);
        }
    }

    /// Starts the event thread (once) and returns its events.
    pub fn events() -> flume::Receiver<Event> {
        let (tx, rx) = flume::unbounded();
        if SINK.set(tx).is_err() {
            tracing::warn!("island: events already started");
        }
        let _ = std::thread::Builder::new().name("island-events".into()).spawn(run);
        rx
    }

    fn run() {
        let class: Vec<u16> = "DoveIslandEvents".encode_utf16().chain(Some(0)).collect();
        // SAFETY: a window class and a hidden (never shown) top-level window
        // owned by this thread, whose message loop runs until the process
        // ends. Top-level, not message-only, so it gets display and setting
        // broadcasts.
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassExW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                tracing::warn!("island: event window failed");
                return;
            }
            // Mouse movement anywhere, without a hook that would slow every
            // app's input.
            let mouse = RAWINPUTDEVICE { usUsagePage: 0x01, usUsage: 0x02, dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd };
            RegisterRawInputDevices(&mouse, 1, size_of::<RAWINPUTDEVICE>() as u32);
            AddClipboardFormatListener(hwnd);
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                std::ptr::null_mut(),
                Some(on_win_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            hook_foreground_process();
            WINDOW.store(hwnd as isize, std::sync::atomic::Ordering::Release);
            if RegisterHotKey(hwnd, HOTKEY_TOGGLE, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_SPACE as u32) == 0 {
                tracing::info!("island: Ctrl+Alt+Space is taken by another app");
            }
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    /// Window moves and resizes, but only the foreground process's, so the
    /// island learns about F11 and full-screen video without waking for
    /// every window on the system.
    unsafe fn hook_foreground_process() {
        // SAFETY: plain hook bookkeeping on this thread.
        unsafe {
            let fg = GetForegroundWindow();
            let mut pid = 0u32;
            if !fg.is_null() {
                GetWindowThreadProcessId(fg, &mut pid);
            }
            if pid == HOOK_PID.get() {
                return;
            }
            let old = LOCATION_HOOK.replace(std::ptr::null_mut());
            if !old.is_null() {
                UnhookWinEvent(old);
            }
            HOOK_PID.set(pid);
            if pid == 0 || pid == std::process::id() {
                return;
            }
            let hook = SetWinEventHook(
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_LOCATIONCHANGE,
                std::ptr::null_mut(),
                Some(on_win_event),
                pid,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            LOCATION_HOOK.set(hook);
        }
    }

    unsafe extern "system" fn on_win_event(
        _: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        id_object: i32,
        id_child: i32,
        _: u32,
        _: u32,
    ) {
        if event == EVENT_SYSTEM_FOREGROUND {
            // SAFETY: called on the event thread.
            unsafe { hook_foreground_process() };
            send(Event::Foreground);
        } else if event == EVENT_OBJECT_LOCATIONCHANGE
            && id_object == OBJID_WINDOW
            && id_child == 0
            // SAFETY: a plain query.
            && hwnd == unsafe { GetForegroundWindow() }
        {
            send(Event::Foreground);
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_INPUT => {
                let now = Instant::now();
                let due = LAST_POINTER.get().is_none_or(|t| now.duration_since(t) >= POINTER_EVERY);
                if due {
                    LAST_POINTER.set(Some(now));
                    send(Event::Pointer);
                } else if !TRAILING.get() {
                    TRAILING.set(true);
                    // SAFETY: a timer on our own window.
                    unsafe { SetTimer(hwnd, TRAILING_TIMER, POINTER_EVERY.as_millis() as u32 + 4, None) };
                }
            }
            WM_TIMER if wparam == TRAILING_TIMER => {
                TRAILING.set(false);
                // SAFETY: as above.
                unsafe { KillTimer(hwnd, TRAILING_TIMER) };
                LAST_POINTER.set(Some(Instant::now()));
                send(Event::Pointer);
                return 0;
            }
            WM_CLIPBOARDUPDATE => {
                send(Event::Clipboard);
                return 0;
            }
            WM_HOTKEY => {
                send(match wparam as i32 {
                    HOTKEY_ALLOW => Event::Approve(true),
                    HOTKEY_DENY => Event::Approve(false),
                    _ => Event::Hotkey,
                });
                return 0;
            }
            WM_APPROVAL_KEYS => {
                // SAFETY: hotkeys on our own window.
                unsafe {
                    UnregisterHotKey(hwnd, HOTKEY_ALLOW);
                    UnregisterHotKey(hwnd, HOTKEY_DENY);
                    if wparam == 1 {
                        RegisterHotKey(hwnd, HOTKEY_ALLOW, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_A as u32);
                        RegisterHotKey(hwnd, HOTKEY_DENY, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_D as u32);
                    }
                }
                return 0;
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE => send(Event::Display),
            _ => {}
        }
        // SAFETY: default handling (raw input must reach it to be freed).
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    // -----------------------------------------------------------------------
    // Settings, memory
    // -----------------------------------------------------------------------

    /// Windows' "Transparency effects" (Settings › Personalization › Colors).
    pub fn transparency_on() -> bool {
        use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
        let key: Vec<u16> =
            r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize".encode_utf16().chain(Some(0)).collect();
        let name: Vec<u16> = "EnableTransparency".encode_utf16().chain(Some(0)).collect();
        let mut value = 1u32;
        let mut size = 4u32;
        // SAFETY: a DWORD out-parameter of the size given.
        let ok = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&raw mut value).cast(),
                &mut size,
            )
        } == 0;
        !ok || value != 0
    }

    /// Hands the process's idle pages back to Windows (after the island has
    /// been hidden a while). They fault back in on use.
    pub fn trim_working_set() {
        use windows_sys::Win32::System::ProcessStatus::K32EmptyWorkingSet;
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        // SAFETY: the current process's pseudo-handle.
        unsafe {
            K32EmptyWorkingSet(GetCurrentProcess());
        }
    }

    // -----------------------------------------------------------------------
    // Shell: file icons, ports, capture, paste
    // -----------------------------------------------------------------------

    /// A file's shell icon (32 px) as BGRA.
    pub fn file_icon(path: &std::path::Path) -> Option<image::RgbaImage> {
        use windows_sys::Win32::Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC,
            DeleteObject, SelectObject,
        };
        use windows_sys::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW};
        use windows_sys::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx};
        const SIZE: i32 = 32;
        let wide: Vec<u16> = path.as_os_str().encode_wide_nul();
        // SAFETY: SHGetFileInfoW fills `info`; the icon is drawn into a DIB
        // we own and then destroyed.
        unsafe {
            let mut info: SHFILEINFOW = std::mem::zeroed();
            let ok = SHGetFileInfoW(wide.as_ptr(), 0, &mut info, size_of::<SHFILEINFOW>() as u32, SHGFI_ICON | SHGFI_LARGEICON);
            if ok == 0 || info.hIcon.is_null() {
                return None;
            }
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: SIZE,
                biHeight: -SIZE,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            let mut out = None;
            if !dib.is_null() && !bits.is_null() {
                let old = SelectObject(dc, dib);
                DrawIconEx(dc, 0, 0, info.hIcon, SIZE, SIZE, 0, std::ptr::null_mut(), DI_NORMAL);
                let buf = std::slice::from_raw_parts(bits as *const u8, (SIZE * SIZE * 4) as usize).to_vec();
                SelectObject(dc, old);
                out = image::RgbaImage::from_raw(SIZE as u32, SIZE as u32, buf);
                DeleteObject(dib);
            }
            DeleteDC(dc);
            DestroyIcon(info.hIcon);
            out
        }
    }

    trait WideNul {
        fn encode_wide_nul(&self) -> Vec<u16>;
    }

    impl WideNul for std::ffi::OsStr {
        fn encode_wide_nul(&self) -> Vec<u16> {
            use std::os::windows::ffi::OsStrExt as _;
            self.encode_wide().chain(Some(0)).collect()
        }
    }

    /// TCP ports something on this machine listens on, with the process.
    /// System services and well-known ports are left out: what's left are
    /// the dev servers.
    pub fn listening_ports() -> Vec<Port> {
        use windows_sys::Win32::NetworkManagement::IpHelper::{
            GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
        };
        const AF_INET: u32 = 2;
        let mut size = 0u32;
        // SAFETY: sized by the first call, filled by the second.
        unsafe {
            GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_LISTENER, 0);
            if size == 0 {
                return Vec::new();
            }
            let mut buf = vec![0u8; size as usize];
            if GetExtendedTcpTable(buf.as_mut_ptr().cast(), &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_LISTENER, 0) != 0 {
                return Vec::new();
            }
            let count = u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
            let rows = buf.as_ptr().add(4) as *const MIB_TCPROW_OWNER_PID;
            let mut out: Vec<Port> = Vec::new();
            for i in 0..count {
                let row = std::ptr::read_unaligned(rows.add(i));
                let port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
                if port < 1024 || row.dwOwningPid <= 4 || out.iter().any(|p| p.port == port) {
                    continue;
                }
                let process = process_name(row.dwOwningPid).unwrap_or_default();
                if is_dev_server(&process) {
                    out.push(Port { port, pid: row.dwOwningPid, process });
                }
            }
            out.sort_by_key(|p| p.port);
            out
        }
    }

    /// The runtimes and tools that serve previews.
    fn is_dev_server(process: &str) -> bool {
        let p = process.to_ascii_lowercase();
        let name = p.trim_end_matches(".exe");
        matches!(
            name,
            "node" | "bun" | "deno" | "python" | "python3" | "pythonw" | "ruby" | "rails" | "php" | "java" | "dotnet"
                | "go" | "hugo" | "caddy" | "nginx" | "uvicorn" | "gunicorn" | "flask" | "vite" | "esbuild"
                | "next-server" | "wrangler" | "workerd" | "cargo" | "trunk"
        ) || name.ends_with("-server")
    }

    fn process_name(pid: u32) -> Option<String> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        };
        // SAFETY: a limited query handle, closed before returning.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0;
            CloseHandle(h);
            ok.then(|| {
                let full = String::from_utf16_lossy(&buf[..len as usize]);
                full.rsplit('\\').next().unwrap_or(&full).to_string()
            })
        }
    }

    /// Ends a dev server.
    /// The process still runs (a dev server that stopped listening may only
    /// be restarting).
    pub fn process_alive(pid: u32) -> bool {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        /// `STILL_ACTIVE`.
        const STILL_ACTIVE: u32 = 259;
        // SAFETY: a query-only handle, closed before returning.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false;
            }
            let mut code = 0u32;
            let alive = GetExitCodeProcess(h, &mut code) != 0 && code == STILL_ACTIVE;
            CloseHandle(h);
            alive
        }
    }

    pub fn stop_process(pid: u32) -> bool {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
        // SAFETY: a terminate-only handle, closed before returning.
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if h.is_null() {
                return false;
            }
            let ok = TerminateProcess(h, 1) != 0;
            CloseHandle(h);
            ok
        }
    }

    /// The front window (other than the island), as a PNG file in `dir`, and
    /// its title.
    pub fn capture_front(dir: &std::path::Path, skip: Option<isize>) -> Option<(std::path::PathBuf, String)> {
        use windows_sys::Win32::Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC,
            DeleteObject, GetDC, ReleaseDC, SelectObject,
        };
        use windows_sys::Win32::Storage::Xps::PrintWindow;
        // Captures GPU-composited content too.
        const PW_RENDERFULLCONTENT: u32 = 2;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowRect, GetWindowTextW};
        // SAFETY: GDI objects created, used and released here; the DIB is
        // read only while selected and alive.
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() || skip.is_some_and(|s| s == hwnd as isize) {
                return None;
            }
            let mut r = std::mem::zeroed();
            GetWindowRect(hwnd, &mut r);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            if w <= 0 || h <= 0 {
                return None;
            }
            let screen = GetDC(std::ptr::null_mut());
            let dc = CreateCompatibleDC(screen);
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            let mut out = None;
            if !dib.is_null() && !bits.is_null() {
                let old = SelectObject(dc, dib);
                if PrintWindow(hwnd, dc, PW_RENDERFULLCONTENT) != 0 {
                    let mut px = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
                    for p in px.chunks_exact_mut(4) {
                        p.swap(0, 2);
                        p[3] = 255;
                    }
                    if let Some(img) = image::RgbaImage::from_raw(w as u32, h as u32, px) {
                        let _ = std::fs::create_dir_all(dir);
                        let path = dir.join(format!("shot-{}.png", dove_core::store::now_ms()));
                        if img.save_with_format(&path, image::ImageFormat::Png).is_ok() {
                            let mut title = [0u16; 256];
                            let n = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32).max(0) as usize;
                            out = Some((path, String::from_utf16_lossy(&title[..n])));
                        }
                    }
                }
                SelectObject(dc, old);
                DeleteObject(dib);
            }
            DeleteDC(dc);
            ReleaseDC(std::ptr::null_mut(), screen);
            out
        }
    }

    /// Brings `window` back to the front and presses Ctrl+V in it (the
    /// clipboard already holds what to paste).
    pub fn paste_into(window: Option<isize>) {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_CONTROL, VK_V,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow};
        std::thread::spawn(move || {
            // SAFETY: the window is checked first; SendInput gets an array
            // of fully initialized INPUTs.
            unsafe {
                if let Some(w) = window
                    && IsWindow(w as HWND) != 0
                {
                    SetForegroundWindow(w as HWND);
                }
                std::thread::sleep(Duration::from_millis(120));
                let key = |vk: u16, up: bool| INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: vk,
                            wScan: 0,
                            dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };
                let inputs = [key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)];
                SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32);
            }
        });
    }

    // -----------------------------------------------------------------------
    // Audio
    // -----------------------------------------------------------------------

    enum AudioCmd {
        Nudge(f32),
        Sample(bool),
        Refresh,
    }

    /// What the island shows about sound.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct AudioState {
        /// 0–1.
        pub volume: f32,
        pub muted: bool,
        /// 0–1, the output's peak level right now (while sampling).
        pub peak: f32,
        pub device: String,
    }

    /// The default output, on a COM thread of its own.
    pub struct Audio {
        tx: flume::Sender<AudioCmd>,
        pub updates: flume::Receiver<AudioState>,
    }

    impl Audio {
        pub fn start() -> Audio {
            let (tx, rx) = flume::unbounded::<AudioCmd>();
            let (up_tx, updates) = flume::unbounded::<AudioState>();
            let _ = std::thread::Builder::new().name("island-audio".into()).spawn(move || audio_thread(rx, up_tx));
            let _ = tx.send(AudioCmd::Refresh);
            Audio { tx, updates }
        }

        /// Changes the volume by `delta` (−1…1).
        pub fn nudge(&self, delta: f32) {
            let _ = self.tx.send(AudioCmd::Nudge(delta));
        }

        /// Reports the peak level 30 times a second while on (the equalizer
        /// is showing), nothing while off.
        pub fn sample(&self, on: bool) {
            let _ = self.tx.send(AudioCmd::Sample(on));
        }

        pub fn refresh(&self) {
            let _ = self.tx.send(AudioCmd::Refresh);
        }
    }

    fn audio_thread(rx: flume::Receiver<AudioCmd>, tx: flume::Sender<AudioState>) {
        use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
        use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender};
        use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};

        struct Dev {
            volume: IAudioEndpointVolume,
            meter: IAudioMeterInformation,
            name: String,
        }

        fn open() -> Option<Dev> {
            // SAFETY: COM calls on this thread (initialized below).
            unsafe {
                let e: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
                let device = e.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
                let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).ok()?;
                let meter: IAudioMeterInformation = device.Activate(CLSCTX_ALL, None).ok()?;
                let name = device_name(&device).unwrap_or_default();
                Some(Dev { volume, meter, name })
            }
        }

        unsafe fn device_name(device: &windows::Win32::Media::Audio::IMMDevice) -> Option<String> {
            use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
            use windows::Win32::System::Com::STGM_READ;
            use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
            // SAFETY: the property store outlives the call; the string is
            // freed with CoTaskMemFree.
            unsafe {
                let store = device.OpenPropertyStore(STGM_READ).ok()?;
                let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
                let s = PropVariantToStringAlloc(&value).ok()?;
                let out = s.to_string().ok();
                windows::Win32::System::Com::CoTaskMemFree(Some(s.0 as *const _));
                out
            }
        }

        fn state(dev: &Option<Dev>, peak: f32) -> AudioState {
            match dev {
                // SAFETY: COM calls on this thread.
                Some(d) => unsafe {
                    AudioState {
                        volume: d.volume.GetMasterVolumeLevelScalar().unwrap_or(0.0),
                        muted: d.volume.GetMute().map(|m| m.as_bool()).unwrap_or(false),
                        peak,
                        device: d.name.clone(),
                    }
                },
                None => AudioState::default(),
            }
        }

        // SAFETY: once, first thing on this thread.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut dev = open();
        let mut sampling = false;
        let mut last = AudioState::default();
        loop {
            let cmd = if sampling {
                match rx.recv_timeout(Duration::from_millis(33)) {
                    Ok(c) => Some(c),
                    Err(flume::RecvTimeoutError::Timeout) => None,
                    Err(flume::RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match rx.recv() {
                    Ok(c) => Some(c),
                    Err(_) => return,
                }
            };
            match cmd {
                Some(AudioCmd::Nudge(d)) => {
                    if let Some(x) = &dev {
                        // SAFETY: COM calls on this thread.
                        unsafe {
                            let v = (x.volume.GetMasterVolumeLevelScalar().unwrap_or(0.0) + d).clamp(0.0, 1.0);
                            let _ = x.volume.SetMasterVolumeLevelScalar(v, std::ptr::null());
                        }
                    }
                }
                Some(AudioCmd::Sample(on)) => sampling = on,
                Some(AudioCmd::Refresh) => dev = open(),
                None => {}
            }
            let peak = if sampling {
                // SAFETY: COM call on this thread.
                dev.as_ref().and_then(|d| unsafe { d.meter.GetPeakValue().ok() }).unwrap_or(0.0)
            } else {
                0.0
            };
            let now = state(&dev, peak);
            if now != last {
                last = now.clone();
                if tx.send(now).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod other {
    use super::{Event, Port};

    pub fn events() -> flume::Receiver<Event> {
        let (tx, rx) = flume::unbounded();
        std::mem::forget(tx);
        rx
    }
    pub fn transparency_on() -> bool {
        true
    }
    pub fn trim_working_set() {}
    pub fn file_icon(_: &std::path::Path) -> Option<image::RgbaImage> {
        None
    }
    pub fn listening_ports() -> Vec<Port> {
        Vec::new()
    }
    pub fn stop_process(_: u32) -> bool {
        false
    }
    pub fn capture_front(_: &std::path::Path, _: Option<isize>) -> Option<(std::path::PathBuf, String)> {
        None
    }
    pub fn paste_into(_: Option<isize>) {}
    pub fn approval_keys(_: bool) {}
    pub fn process_alive(_: u32) -> bool {
        false
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct AudioState {
        pub volume: f32,
        pub muted: bool,
        pub peak: f32,
        pub device: String,
    }

    pub struct Audio {
        pub updates: flume::Receiver<AudioState>,
    }

    impl Audio {
        pub fn start() -> Audio {
            let (tx, updates) = flume::unbounded();
            std::mem::forget(tx);
            Audio { updates }
        }
        pub fn nudge(&self, _: f32) {}
        pub fn sample(&self, _: bool) {}
        pub fn refresh(&self) {}
    }
}
