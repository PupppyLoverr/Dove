//! A debug build's access violations (Windows), each's stack written as it
//! happens to `<engine data>\logs\crash-<pid>.txt`: where no debugger is at
//! hand, what the system's crash report (admin-only) won't say. Handled ones
//! are written too (the first few), then passed on, unchanged.

use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};

use windows_sys::Win32::Foundation::EXCEPTION_ACCESS_VIOLATION;
use windows_sys::Win32::System::Diagnostics::Debug::{AddVectoredExceptionHandler, EXCEPTION_POINTERS};

/// How many are written: a loop of handled ones fills no disk.
const MOST: u32 = 5;
static WRITTEN: AtomicU32 = AtomicU32::new(0);

pub(crate) fn install() {
    // SAFETY: a handler for the process's lifetime, first in line.
    unsafe { AddVectoredExceptionHandler(1, Some(caught)) };
}

unsafe extern "system" fn caught(info: *mut EXCEPTION_POINTERS) -> i32 {
    // (Continue the search: whoever handles it still does.)
    const SEARCH: i32 = 0;
    // SAFETY: the system's own record of the exception, for this call.
    let record = unsafe { &*(*info).ExceptionRecord };
    if record.ExceptionCode != EXCEPTION_ACCESS_VIOLATION || WRITTEN.fetch_add(1, Ordering::Relaxed) >= MOST {
        return SEARCH;
    }
    let data = std::env::var_os("ZERON_ZGX_ENGINE_DATA")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|home| std::path::Path::new(&home).join(".zeron-zgx")));
    let Some(data) = data else { return SEARCH };
    let logs = data.join("logs");
    let _ = std::fs::create_dir_all(&logs);
    let thread = std::thread::current();
    let text = format!(
        "{:?}: access violation at {:?} ({} {:#x} of {:#x}), thread {:?}\n{}\n\n",
        std::time::SystemTime::now(),
        record.ExceptionAddress,
        if record.ExceptionInformation[0] == 0 { "reading" } else { "writing" },
        record.ExceptionInformation[1],
        record.ExceptionInformation[0],
        thread.name().unwrap_or("unnamed"),
        std::backtrace::Backtrace::force_capture(),
    );
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(logs.join(format!("crash-{}.txt", std::process::id()))) {
        let _ = file.write_all(text.as_bytes());
    }
    SEARCH
}
