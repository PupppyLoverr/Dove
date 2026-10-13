//! ConPTY launch, matching Zeron's Windows terminal.
//!
//! portable-pty starts the child with its standard handles set to the ConPTY
//! pipes. Those handles are not a console, so `isatty` is false and TUIs drop
//! VT color and Unicode icons. Here the handles are `INVALID_HANDLE_VALUE` and
//! the pseudoconsole is attached with `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`,
//! which is what makes the child a real console. A job owns the tree so close
//! kills descendants the shell started.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use portable_pty::{Child, ChildKiller, CommandBuilder, ExitStatus, MasterPty, PtySize};
use windows_sys::Win32::Foundation::{INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole, SetConsoleCP, SetConsoleOutputCP,
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
    PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute,
    WaitForSingleObject,
};

const CP_UTF8: u32 = 65001;

pub fn open(
    cmd: &CommandBuilder,
    size: PtySize,
) -> anyhow::Result<(Box<dyn MasterPty + Send>, Box<dyn Child + Send + Sync>)> {
    let argv = cmd.get_argv();
    if argv.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "missing program").into());
    }
    let executable = resolve_program(&argv[0], cmd.get_env("PATH"))?;
    let mut command = Vec::new();
    append_quoted(executable.as_os_str(), &mut command);
    for arg in argv.iter().skip(1) {
        if arg.encode_wide().any(|c| c == 0) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "argument contains NUL").into());
        }
        command.push(u16::from(b' '));
        append_quoted(arg, &mut command);
    }
    command.push(0);
    let application = wide(executable.as_os_str())?;
    let cwd = cmd.get_cwd().map(|dir| current_dir_wide(dir)).transpose()?;
    let environment = environment_block(cmd);

    // A GUI process often has no console; the calls no-op then. When one
    // exists, ConPTY inherits UTF-8 from it.
    unsafe {
        SetConsoleOutputCP(CP_UTF8);
        SetConsoleCP(CP_UTF8);
    }

    let job = Arc::new(Job::new()?);
    let (input, writer) = pipe()?;
    let (reader, output) = pipe()?;
    let mut console_handle: HPCON = 0;
    hresult(unsafe {
        CreatePseudoConsole(coord(size), input.as_raw_handle(), output.as_raw_handle(), 0, &mut console_handle)
    })?;
    let console = Console(console_handle);
    // ConPTY duplicated the ends it needs. Drop ours so the pipes close with
    // the pseudoconsole.
    drop(input);
    drop(output);

    let jobs = [job.as_raw_handle()];
    let mut attrs = Attributes::for_console(&console.0, &jobs)?;
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    startup.lpAttributeList = attrs.ptr();
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let cwd_ptr = cwd.as_ref().map(|w| w.as_ptr()).unwrap_or(std::ptr::null());
    // Job membership is part of creation, so shell startup cannot race out of
    // the tree. Suspension only orders the resume below the pipe setup.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED,
            environment.as_ptr().cast(),
            cwd_ptr,
            &startup.StartupInfo,
            &mut info,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error().into());
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
    let thread = unsafe { OwnedHandle::from_raw_handle(info.hThread) };
    drop(attrs);
    // The output pipe absorbs startup bytes until the reader thread starts.
    // A full pipe blocks the child instead of dropping output.
    if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
        return Err(io::Error::last_os_error().into());
    }
    drop(thread);

    let child = Process { handle, pid: info.dwProcessId, job };
    Ok((
        Box::new(Master { console, reader, writer: Mutex::new(Some(writer)), size: Mutex::new(size) }),
        Box::new(child),
    ))
}

struct Console(HPCON);
impl Drop for Console {
    fn drop(&mut self) {
        unsafe { ClosePseudoConsole(self.0) };
    }
}

struct Master {
    console: Console,
    reader: File,
    writer: Mutex<Option<File>>,
    size: Mutex<PtySize>,
}
impl MasterPty for Master {
    fn resize(&self, size: PtySize) -> anyhow::Result<()> {
        hresult(unsafe { ResizePseudoConsole(self.console.0, coord(size)) })?;
        *lock(&self.size) = size;
        Ok(())
    }
    fn get_size(&self) -> anyhow::Result<PtySize> {
        Ok(*lock(&self.size))
    }
    fn try_clone_reader(&self) -> anyhow::Result<Box<dyn Read + Send>> {
        Ok(Box::new(self.reader.try_clone()?))
    }
    fn take_writer(&self) -> anyhow::Result<Box<dyn Write + Send>> {
        Ok(Box::new(lock(&self.writer).take().ok_or_else(|| io::Error::other("PTY writer already taken"))?))
    }
}

#[derive(Debug)]
struct Process {
    handle: OwnedHandle,
    pid: u32,
    job: Arc<Job>,
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.job.terminate();
    }
}
#[derive(Debug)]
struct Killer(Arc<Job>);
impl ChildKiller for Killer {
    fn kill(&mut self) -> io::Result<()> {
        self.0.terminate()
    }
    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(Self(self.0.clone()))
    }
}
impl ChildKiller for Process {
    fn kill(&mut self) -> io::Result<()> {
        self.job.terminate()
    }
    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(Killer(self.job.clone()))
    }
}
impl Process {
    fn wait_for(&self, timeout: u32) -> io::Result<Option<ExitStatus>> {
        match unsafe { WaitForSingleObject(self.handle.as_raw_handle(), timeout) } {
            WAIT_OBJECT_0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(self.handle.as_raw_handle(), &mut code) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                let _ = self.job.terminate();
                Ok(Some(ExitStatus::with_exit_code(code)))
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }
}
impl Child for Process {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.wait_for(0)
    }
    fn wait(&mut self) -> io::Result<ExitStatus> {
        self.wait_for(INFINITE)?.ok_or_else(|| io::Error::other("PTY wait returned without exit"))
    }
    fn process_id(&self) -> Option<u32> {
        Some(self.pid)
    }
    fn as_raw_handle(&self) -> Option<*mut std::ffi::c_void> {
        Some(self.handle.as_raw_handle())
    }
}

fn coord(size: PtySize) -> COORD {
    COORD { X: size.cols as i16, Y: size.rows as i16 }
}
fn hresult(result: i32) -> io::Result<()> {
    if result < 0 { Err(io::Error::other(format!("ConPTY failed: HRESULT {result:#x}"))) } else { Ok(()) }
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}
fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut result: Vec<u16> = value.encode_wide().collect();
    if result.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "embedded NUL"));
    }
    result.push(0);
    Ok(result)
}
fn pipe() -> io::Result<(File, File)> {
    let (mut read, mut write) = (std::ptr::null_mut(), std::ptr::null_mut());
    // Null attributes: the handles are not inheritable. 64KiB gives the child
    // room to write before the reader thread is running.
    if unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), 64 * 1024) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { (File::from_raw_handle(read), File::from_raw_handle(write)) })
}

fn environment_block(cmd: &CommandBuilder) -> Vec<u16> {
    let mut block = Vec::new();
    for (key, value) in cmd.iter_full_env_as_str() {
        block.extend(OsStr::new(key).encode_wide());
        block.push(u16::from(b'='));
        block.extend(OsStr::new(value).encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

/// Resolve before CreateProcessW. A partial `lpApplicationName` is completed
/// against the current directory, so a missing PATH entry must not select a
/// workspace-planted executable.
fn resolve_program(program: &OsStr, path: Option<&OsStr>) -> io::Result<PathBuf> {
    let named = Path::new(program);
    let candidate = if named.is_absolute() || named.components().count() > 1 {
        existing_program(named)
    } else {
        path.and_then(|path| {
            std::env::split_paths(path)
                .filter(|dir| !dir.as_os_str().is_empty())
                .find_map(|dir| existing_program(&dir.join(named)))
        })
    };
    let candidate = candidate
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("executable not found: {}", named.display())))?;
    std::path::absolute(candidate).map_err(Into::into)
}

fn existing_program(candidate: &Path) -> Option<PathBuf> {
    if candidate.is_file() {
        return Some(candidate.to_path_buf());
    }
    if candidate.extension().is_none() {
        let exe = candidate.with_extension("exe");
        if exe.is_file() {
            return Some(exe);
        }
    }
    let exts = std::env::var_os("PATHEXT")?;
    for ext in std::env::split_paths(&exts) {
        let ext = ext.to_string_lossy();
        let ext = ext.trim_start_matches('.');
        if ext.is_empty() {
            continue;
        }
        let with = candidate.with_extension(ext);
        if with.is_file() {
            return Some(with);
        }
    }
    None
}

/// CreateProcessW rejects `\\?\` verbatim paths: cmd.exe treats them as UNC
/// and starts in the Windows directory. Strip the drive and UNC forms.
fn plain_current_dir(cwd: &str) -> io::Result<String> {
    if cwd.starts_with(r"\\.\") || cwd.starts_with(r"\??\") {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "device namespace is not a valid current directory"));
    }
    let Some(rest) = cwd.strip_prefix(r"\\?\") else {
        return Ok(cwd.to_string());
    };
    if rest.get(..4).is_some_and(|prefix| prefix.eq_ignore_ascii_case("UNC\\")) {
        let share = &rest[4..];
        let mut components = share.split('\\');
        let valid = components.next().is_some_and(|server| !server.is_empty())
            && components.next().is_some_and(|name| !name.is_empty());
        if valid {
            return Ok(format!(r"\\{share}"));
        }
    } else {
        let bytes = rest.as_bytes();
        if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' {
            return Ok(rest.to_string());
        }
    }
    Err(io::Error::new(io::ErrorKind::InvalidInput, "unsupported verbatim current directory"))
}

fn current_dir_wide(cwd: &OsStr) -> io::Result<Vec<u16>> {
    wide(OsStr::new(&plain_current_dir(&cwd.to_string_lossy())?))
}

/// Windows command-line quoting (CommandLineToArgvW rules).
fn append_quoted(arg: &OsStr, cmdline: &mut Vec<u16>) {
    let raw: Vec<u16> = arg.encode_wide().collect();
    let needs_quotes = raw.is_empty() || raw.iter().any(|c| matches!(*c, 0x20 | 0x09 | 0x0a | 0x0b | 0x22));
    if !needs_quotes {
        cmdline.extend_from_slice(&raw);
        return;
    }
    cmdline.push(u16::from(b'"'));
    let mut i = 0;
    while i < raw.len() {
        let mut slashes = 0;
        while i < raw.len() && raw[i] == u16::from(b'\\') {
            i += 1;
            slashes += 1;
        }
        if i == raw.len() {
            for _ in 0..slashes * 2 {
                cmdline.push(u16::from(b'\\'));
            }
            break;
        } else if raw[i] == u16::from(b'"') {
            for _ in 0..slashes * 2 + 1 {
                cmdline.push(u16::from(b'\\'));
            }
            cmdline.push(raw[i]);
        } else {
            for _ in 0..slashes {
                cmdline.push(u16::from(b'\\'));
            }
            cmdline.push(raw[i]);
        }
        i += 1;
    }
    cmdline.push(u16::from(b'"'));
}

/// Owns the session process tree. Closing the last handle kills it.
#[derive(Debug)]
struct Job(OwnedHandle);
impl Job {
    fn new() -> io::Result<Self> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                job.0.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    fn as_raw_handle(&self) -> RawHandle {
        self.0.as_raw_handle()
    }

    fn terminate(&self) -> io::Result<()> {
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Creation-time pseudoconsole and job list. The pointed-to values are
/// borrowed until `CreateProcessW` returns.
struct Attributes {
    buffer: Vec<usize>,
    initialized: bool,
}
impl Attributes {
    fn for_console(console: &HPCON, jobs: &[RawHandle]) -> io::Result<Self> {
        let mut attrs = Self::new(2)?;
        // PSEUDOCONSOLE takes the handle value, unlike the pointer-to-list attributes.
        attrs.add(
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            (*console) as *const std::ffi::c_void,
            std::mem::size_of::<HPCON>(),
        )?;
        attrs.add(PROC_THREAD_ATTRIBUTE_JOB_LIST as usize, jobs.as_ptr().cast(), std::mem::size_of_val(jobs))?;
        Ok(attrs)
    }

    fn new(count: u32) -> io::Result<Self> {
        let mut bytes = 0usize;
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), count, 0, &mut bytes) };
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut attrs = Self { buffer: vec![0; bytes.div_ceil(std::mem::size_of::<usize>())], initialized: false };
        if unsafe { InitializeProcThreadAttributeList(attrs.ptr(), count, 0, &mut bytes) } == 0 {
            return Err(io::Error::last_os_error());
        }
        attrs.initialized = true;
        Ok(attrs)
    }

    fn add(&mut self, attribute: usize, value: *const std::ffi::c_void, bytes: usize) -> io::Result<()> {
        if unsafe {
            UpdateProcThreadAttribute(self.ptr(), 0, attribute, value, bytes, std::ptr::null_mut(), std::ptr::null())
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.buffer.as_mut_ptr().cast()
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { DeleteProcThreadAttributeList(self.ptr()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_wraps_spaces_and_keeps_plain_tokens() {
        let mut line = Vec::new();
        append_quoted(OsStr::new("powershell.exe"), &mut line);
        line.push(u16::from(b' '));
        append_quoted(OsStr::new("say hi"), &mut line);
        let text = String::from_utf16(&line).unwrap();
        assert_eq!(text, "powershell.exe \"say hi\"");
    }

    #[test]
    fn plain_current_dir_strips_verbatim_prefixes() {
        assert_eq!(plain_current_dir(r"\\?\C:\work\repo").unwrap(), r"C:\work\repo");
        assert_eq!(plain_current_dir(r"\\?\UNC\server\share\repo").unwrap(), r"\\server\share\repo");
        assert_eq!(plain_current_dir(r"C:\work\repo").unwrap(), r"C:\work\repo");
        assert!(plain_current_dir(r"\\.\C:\repo").is_err());
    }

    #[test]
    fn missing_path_does_not_search_the_current_directory() {
        assert!(resolve_program(OsStr::new("definitely-not-a-shell.exe"), None).is_err());
        assert!(resolve_program(OsStr::new("definitely-not-a-shell.exe"), Some(OsStr::new(""))).is_err());
    }
}
