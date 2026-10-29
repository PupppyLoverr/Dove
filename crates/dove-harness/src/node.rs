//! `dove node …`: Keplorayx without a window, for other machines. The same
//! binary as the desktop app; `main` dispatches here before any GPUI code
//! runs, so it works on a headless server.
//!
//! - `attach <session> --from <n> -- <argv…>`: run `argv` under a small
//!   daemon that outlives the connection, or reattach to it, replaying its
//!   stdout from byte `n`. This is how a remote agent survives a dropped SSH
//!   connection or a sleeping laptop (Unix only).
//! - `bridge <provider> <id> -- <argv…>`: run `argv` on a cloud box over its
//!   provider's own streaming API ([`crate::hosts::Provider::bridge`]).
//! - `pipe <host:port>`: join stdio to a TCP connection, so any port can be
//!   forwarded over an exec stream.
//! - `version`: print the version (the launcher checks it on remote hosts).

use std::io::{Read, Write};
use std::path::PathBuf;

/// Run a `dove node` subcommand; returns the process exit code.
pub fn main(args: &[String]) -> i32 {
    let (cmd, rest) = match args.split_first() {
        Some((c, r)) => (c.as_str(), r),
        None => ("help", &[][..]),
    };
    let result = match cmd {
        "version" => {
            println!("{}", VERSION);
            Ok(0)
        }
        "bridge" => bridge(rest),
        "pipe" => pipe(rest),
        #[cfg(unix)]
        "attach" => session::attach(rest),
        #[cfg(unix)]
        "daemon" => session::daemon(rest),
        _ => {
            eprintln!("{USAGE}");
            Ok(2)
        }
    };
    result.unwrap_or_else(|err| {
        eprintln!("dove node {cmd}: {err:#}");
        1
    })
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "usage: dove node <command>
  attach <session> [--from <bytes>] -- <program> [args…]   run or reattach a long-lived process
  bridge <provider> <box id> -- <program> [args…]          run on a cloud box over its API
  pipe <host:port>                                        join stdin/stdout to a TCP port
  version                                                 print the version";

/// Split `… -- argv…` into the part before `--` and the argv after it.
fn split_argv(args: &[String]) -> (&[String], &[String]) {
    match args.iter().position(|a| a == "--") {
        Some(i) => (&args[..i], &args[i + 1..]),
        None => (args, &[]),
    }
}

fn bridge(args: &[String]) -> anyhow::Result<i32> {
    let (head, argv) = split_argv(args);
    let [provider, id] = head else { anyhow::bail!("expected <provider> <box id> -- <program>") };
    anyhow::ensure!(!argv.is_empty(), "nothing to run");
    let p = crate::hosts::provider(provider).ok_or_else(|| anyhow::anyhow!("unknown provider {provider}"))?;
    p.bridge(&crate::hosts::Keys::load(), id, argv)
}

fn pipe(args: &[String]) -> anyhow::Result<i32> {
    let [addr] = args else { anyhow::bail!("expected <host:port>") };
    let stream = std::net::TcpStream::connect(addr.as_str())?;
    let mut up = stream.try_clone()?;
    let mut down = stream;
    let t = std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut up);
        let _ = up.shutdown(std::net::Shutdown::Write);
    });
    let mut out = std::io::stdout().lock();
    let _ = std::io::copy(&mut down, &mut out);
    let _ = out.flush();
    drop(t);
    Ok(0)
}

/// `$DOVE_HOME/node/sessions/<session>`.
pub fn session_dir(session: &str) -> PathBuf {
    let safe: String = session.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    crate::hosts::dove_home().join("node").join("sessions").join(safe)
}

/// Read `--from <n>` out of the arguments before `--`.
fn parse_from(head: &[String]) -> (Option<&String>, u64) {
    let mut session = None;
    let mut from = 0;
    let mut it = head.iter();
    while let Some(a) = it.next() {
        if a == "--from" {
            from = it.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        } else if session.is_none() {
            session = Some(a);
        }
    }
    (session, from)
}

#[cfg(unix)]
mod session {
    //! The attach daemon. Files in the session folder:
    //! `sock` (Unix socket), `out` (every byte of the program's stdout),
    //! `err` (its stderr), `exit` (its exit code, once it has one).
    //!
    //! A client sends one line, the byte offset it already has, then its
    //! stdin as frames (`u32` big-endian length + bytes; a zero length means
    //! the input really ended). It receives stdout from that offset on, live,
    //! until the program has exited and everything was sent. Only the newest
    //! client's input reaches the program. A client that just disappears (a
    //! dropped connection) leaves the program's stdin open for the next one.

    use super::*;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    pub(super) fn attach(args: &[String]) -> anyhow::Result<i32> {
        let (head, argv) = split_argv(args);
        let (Some(session), from) = parse_from(head) else { anyhow::bail!("expected <session>") };
        let dir = session_dir(session);
        std::fs::create_dir_all(&dir)?;
        let sock = dir.join("sock");
        let mut stream = match UnixStream::connect(&sock) {
            Ok(s) => s,
            Err(_) if exit_code(&dir).is_some() => return replay_finished(&dir, from),
            Err(_) => {
                anyhow::ensure!(!argv.is_empty(), "session {session} is not running and nothing was given to start");
                start_daemon(session, argv)?;
                connect_retry(&sock, Duration::from_secs(10))?
            }
        };
        stream.write_all(format!("{from}\n").as_bytes())?;
        let mut up = stream.try_clone()?;
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let mut buf = [0u8; 16 * 1024];
            loop {
                let n = match stdin.read(&mut buf) {
                    Ok(n) => n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => return,
                };
                if up.write_all(&frame(&buf[..n])).is_err() || n == 0 {
                    return;
                }
            }
        });
        let mut out = std::io::stdout().lock();
        let mut buf = [0u8; 64 * 1024];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    out.write_all(&buf[..n])?;
                    out.flush()?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        // The daemon closes the connection after the last byte once the
        // program exited; a close before that is a lost daemon.
        Ok(exit_code(&dir).unwrap_or(1))
    }

    fn replay_finished(dir: &std::path::Path, from: u64) -> anyhow::Result<i32> {
        let mut f = std::fs::File::open(dir.join("out"))?;
        std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(from))?;
        std::io::copy(&mut f, &mut std::io::stdout().lock())?;
        Ok(exit_code(dir).unwrap_or(1))
    }

    fn exit_code(dir: &std::path::Path) -> Option<i32> {
        std::fs::read_to_string(dir.join("exit")).ok()?.trim().parse().ok()
    }

    fn connect_retry(sock: &std::path::Path, limit: Duration) -> anyhow::Result<UnixStream> {
        let deadline = Instant::now() + limit;
        loop {
            match UnixStream::connect(sock) {
                Ok(s) => return Ok(s),
                Err(e) if Instant::now() >= deadline => anyhow::bail!("the session daemon did not start: {e}"),
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    fn start_daemon(session: &str, argv: &[String]) -> anyhow::Result<()> {
        use std::os::unix::process::CommandExt;
        let exe = std::env::current_exe()?;
        let mut cmd = Command::new(exe);
        cmd.args(["node", "daemon", session, "--"]).args(argv);
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        // Its own process group: a hangup on the SSH session doesn't reach it.
        cmd.process_group(0);
        cmd.spawn()?;
        Ok(())
    }

    struct Shared {
        /// Bytes of stdout written to `out` so far.
        len: AtomicU64,
        done: Mutex<bool>,
        changed: Condvar,
        /// The newest client; only it feeds the program's stdin.
        generation: AtomicU64,
        clients: AtomicU64,
    }

    pub(super) fn daemon(args: &[String]) -> anyhow::Result<i32> {
        let (head, argv) = split_argv(args);
        let [session] = head else { anyhow::bail!("expected <session> -- <program>") };
        anyhow::ensure!(!argv.is_empty(), "nothing to run");
        let dir = session_dir(session);
        std::fs::create_dir_all(&dir)?;
        let sock = dir.join("sock");
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock)?;
        let err = std::fs::File::create(dir.join("err"))?;
        let mut child = Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(err))
            .spawn()?;
        let stdin = Arc::new(Mutex::new(child.stdin.take()));
        let mut stdout = child.stdout.take().expect("piped stdout");
        let shared = Arc::new(Shared {
            len: AtomicU64::new(0),
            done: Mutex::new(false),
            changed: Condvar::new(),
            generation: AtomicU64::new(0),
            clients: AtomicU64::new(0),
        });
        // stdout → `out`, waking every client.
        let out_path = dir.join("out");
        let mut out = std::fs::File::create(&out_path)?;
        let s = shared.clone();
        let pump = std::thread::spawn(move || {
            let mut buf = [0u8; 64 * 1024];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 || out.write_all(&buf[..n]).is_err() {
                    break;
                }
                let _ = out.flush();
                s.len.fetch_add(n as u64, Ordering::SeqCst);
                s.changed.notify_all();
            }
        });
        // Accept clients until the program has exited and the last client
        // is served (or nobody came back for ten minutes).
        listener.set_nonblocking(true)?;
        let s = shared.clone();
        let (stdin_c, out_c) = (stdin.clone(), out_path.clone());
        let acceptor = std::thread::spawn(move || {
            let mut idle_since: Option<Instant> = None;
            loop {
                match listener.accept() {
                    Ok((conn, _)) => {
                        idle_since = None;
                        let _ = conn.set_nonblocking(false);
                        let (s, stdin, out) = (s.clone(), stdin_c.clone(), out_c.clone());
                        std::thread::spawn(move || serve(conn, s, stdin, out));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        let finished = *s.done.lock().unwrap();
                        if finished && s.clients.load(Ordering::SeqCst) == 0 {
                            let since = *idle_since.get_or_insert_with(Instant::now);
                            if since.elapsed() > Duration::from_secs(600) {
                                return;
                            }
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(_) => return,
                }
            }
        });
        let status = child.wait()?;
        let _ = pump.join();
        let code = status.code().unwrap_or(1);
        std::fs::write(dir.join("exit"), code.to_string())?;
        *shared.done.lock().unwrap() = true;
        shared.changed.notify_all();
        let _ = acceptor.join();
        let _ = std::fs::remove_file(&sock);
        Ok(code)
    }

    fn serve(conn: UnixStream, s: Arc<Shared>, stdin: Arc<Mutex<Option<std::process::ChildStdin>>>, out: PathBuf) {
        s.clients.fetch_add(1, Ordering::SeqCst);
        let me = s.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let mut reader = std::io::BufReader::new(match conn.try_clone() {
            Ok(c) => c,
            Err(_) => return,
        });
        let mut line = String::new();
        let _ = std::io::BufRead::read_line(&mut reader, &mut line);
        let mut pos: u64 = line.trim().parse().unwrap_or(0);
        // Input: this client → the program, while it is the newest.
        let s_in = s.clone();
        std::thread::spawn(move || {
            while let Ok(Some(bytes)) = read_frame(&mut reader) {
                if s_in.generation.load(Ordering::SeqCst) != me {
                    return;
                }
                let mut guard = stdin.lock().unwrap();
                if bytes.is_empty() {
                    // The client's input really ended: so does the program's.
                    guard.take();
                    return;
                }
                let Some(w) = guard.as_mut() else { return };
                if w.write_all(&bytes).and_then(|_| w.flush()).is_err() {
                    return;
                }
            }
        });
        // Output: `out` from `pos`, live.
        let mut conn = conn;
        let result = (|| -> std::io::Result<()> {
            let mut f = std::fs::File::open(&out)?;
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let len = s.len.load(Ordering::SeqCst);
                if pos < len {
                    std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(pos))?;
                    let want = ((len - pos) as usize).min(buf.len());
                    let n = f.read(&mut buf[..want])?;
                    if n == 0 {
                        continue;
                    }
                    conn.write_all(&buf[..n])?;
                    pos += n as u64;
                    continue;
                }
                let done = s.done.lock().unwrap();
                if *done && pos >= s.len.load(Ordering::SeqCst) {
                    return Ok(());
                }
                let _ = s.changed.wait_timeout(done, Duration::from_millis(250)).unwrap();
            }
        })();
        let _ = result;
        let _ = conn.shutdown(std::net::Shutdown::Both);
        s.clients.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The desktop side: put `dove` on a remote host and run an agent under
/// `dove node attach` there, reconnecting when the connection drops.
pub mod remote {
    use std::io::{Read, Write};
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use crate::host::Host;

    /// Runs the remote `dove` with the arguments after it.
    const RUN_DOVE: &str = r#"exec "$HOME/.dove/bin/dove" "$@""#;

    /// Prints `ok` when the remote `dove` is this version, else `need <os> <arch>`.
    const CHECK: &str = r#"v=$("$HOME/.dove/bin/dove" node version 2>/dev/null)
if [ "$v" = "$1" ]; then echo ok; else echo "need $(uname -s) $(uname -m)"; fi"#;

    /// stdin: the binary.
    const INSTALL: &str = r#"mkdir -p "$HOME/.dove/bin" && t="$HOME/.dove/bin/dove.tmp" && cat > "$t" && chmod 755 "$t" && mv -f "$t" "$HOME/.dove/bin/dove""#;

    /// A binary bigger than this (an unoptimized dev build) isn't uploaded.
    const MAX_UPLOAD: u64 = 300 * 1024 * 1024;

    /// Make sure `dove` (this version) is on `host`, uploading this binary
    /// when the host has the same OS and CPU. `false` when it can't be: the
    /// agent then runs directly and won't survive a dropped connection.
    pub fn ensure(host: &Host) -> bool {
        // `DOVE_NODE=0` turns it off (a host where nothing may be installed).
        if std::env::var_os("DOVE_NODE").is_some_and(|v| v == "0") {
            return false;
        }
        let said = match run(host, CHECK, &[super::VERSION], None) {
            Ok(out) => out,
            Err(_) => return false,
        };
        let said = said.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("").to_string();
        if said == "ok" {
            return true;
        }
        let mut words = said.split_whitespace().skip(1);
        let (os, arch) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
        if !same_platform(os, arch) {
            eprintln!(
                "dove: {} is {os} {arch}; install Keplorayx there (or run it as `dove node`) for sessions that survive disconnects",
                host.label()
            );
            return false;
        }
        let Ok(exe) = std::env::current_exe() else { return false };
        if std::fs::metadata(&exe).map(|m| m.len()).unwrap_or(u64::MAX) > MAX_UPLOAD {
            eprintln!("dove: this Keplorayx build is too large to copy to {}; running without reconnect", host.label());
            return false;
        }
        let Ok(bytes) = std::fs::read(&exe) else { return false };
        eprintln!(
            "dove: installing Keplorayx {} on {} for sessions that survive disconnects",
            super::VERSION,
            host.label()
        );
        if let Err(err) = run(host, INSTALL, &[], Some(&bytes)) {
            eprintln!("dove: could not install Keplorayx on {}: {err:#}", host.label());
            return false;
        }
        run(host, CHECK, &[super::VERSION], None).is_ok_and(|o| o.lines().any(|l| l.trim() == "ok"))
    }

    /// `uname -s` / `uname -m` against the platform this binary was built for.
    pub fn same_platform(os: &str, arch: &str) -> bool {
        let os_ok = match std::env::consts::OS {
            "linux" => os == "Linux",
            "macos" => os == "Darwin",
            _ => false,
        };
        let arch_ok = match std::env::consts::ARCH {
            "x86_64" => arch == "x86_64" || arch == "amd64",
            "aarch64" => arch == "aarch64" || arch == "arm64",
            _ => false,
        };
        os_ok && arch_ok
    }

    /// The argv that runs `program args` under the remote session daemon.
    pub fn attach_args(session: &str, from: u64, program: &str, args: &[String]) -> Vec<String> {
        let mut a: Vec<String> = vec!["-c".into(), RUN_DOVE.into(), "dove".into(), "node".into(), "attach".into()];
        a.extend([session.to_string(), "--from".into(), from.to_string(), "--".into(), program.to_string()]);
        a.extend(args.iter().cloned());
        a
    }

    /// Stdin bytes waiting for the current connection, and that connection.
    #[derive(Default)]
    struct Input {
        pipe: Option<std::process::ChildStdin>,
        pending: Vec<u8>,
        eof: bool,
    }

    impl Input {
        /// Hand everything buffered to the current connection; keep it when
        /// the write fails (the next connection gets it).
        fn flush(&mut self) {
            if let Some(p) = self.pipe.as_mut() {
                if !self.pending.is_empty() && p.write_all(&self.pending).and_then(|_| p.flush()).is_ok() {
                    self.pending.clear();
                } else if !self.pending.is_empty() {
                    self.pipe = None;
                }
            }
            if self.eof && self.pending.is_empty() {
                self.pipe = None;
            }
        }
    }

    /// Run `program args` in `dir` on `host` under `dove node attach`, with
    /// this process's stdio. When the connection drops (ssh exits 255), wait
    /// and reattach from the last byte received, for up to `give_up` without
    /// any progress. Returns the agent's exit code.
    pub fn run_attached(host: &Host, dir: &str, program: &str, args: &[String], give_up: Duration) -> i32 {
        let session = uuid::Uuid::now_v7().simple().to_string();
        let input = Arc::new(Mutex::new(Input::default()));
        let feeder = input.clone();
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let mut buf = [0u8; 16 * 1024];
            loop {
                let n = match stdin.read(&mut buf) {
                    Ok(n) => n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => 0,
                };
                let mut i = feeder.lock().unwrap();
                if n == 0 {
                    i.eof = true;
                } else {
                    i.pending.extend_from_slice(&buf[..n]);
                }
                i.flush();
                if n == 0 {
                    return;
                }
            }
        });
        let mut received: u64 = 0;
        let mut backoff = Duration::from_secs(1);
        let mut last_progress = Instant::now();
        loop {
            let inv = host.wrap("sh", &attach_args(&session, received, program, args), dir, false);
            let mut cmd = std::process::Command::new(&inv.program);
            cmd.args(&inv.args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000);
            }
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(err) => {
                    eprintln!("dove: could not start {}: {err}", inv.program.display());
                    return 127;
                }
            };
            {
                let mut i = input.lock().unwrap();
                i.pipe = child.stdin.take();
                i.flush();
            }
            let before = received;
            if let Some(mut out) = child.stdout.take() {
                let mut stdout = std::io::stdout().lock();
                let mut buf = [0u8; 64 * 1024];
                loop {
                    match out.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if stdout.write_all(&buf[..n]).and_then(|_| stdout.flush()).is_err() {
                                // The driver is gone: stop here, the agent keeps
                                // running remotely and can be resumed.
                                let _ = child.kill();
                                return 0;
                            }
                            received += n as u64;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            }
            let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(255);
            input.lock().unwrap().pipe = None;
            // 255 is ssh's own "connection failed"; anything else is the agent.
            if code != 255 || !matches!(host, Host::Ssh { .. } | Host::Sandbox { .. }) {
                return code;
            }
            if received > before {
                last_progress = Instant::now();
                backoff = Duration::from_secs(1);
            }
            if last_progress.elapsed() > give_up {
                eprintln!("dove: lost the connection to {} and could not get it back", host.label());
                return 255;
            }
            eprintln!("dove: connection to {} lost; reconnecting in {}s", host.label(), backoff.as_secs());
            std::thread::sleep(backoff);
            backoff = (backoff * 2).min(Duration::from_secs(30));
            if let Host::Sandbox { provider, id } = host {
                // Credentials (InstaBox certificates) may need refreshing.
                let _ = crate::hosts::connect_box(provider, id);
            }
        }
    }

    /// `sh -c <script> dove-node <args…>` on `host`, run from its home.
    fn run(host: &Host, script: &str, args: &[&str], stdin: Option<&[u8]>) -> anyhow::Result<String> {
        let mut a: Vec<String> = vec!["-c".into(), script.into(), "dove-node".into()];
        a.extend(args.iter().map(|s| s.to_string()));
        let inv = host.wrap("sh", &a, ".", false);
        let mut cmd = std::process::Command::new(&inv.program);
        cmd.args(&inv.args)
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd.spawn()?;
        let feeder = match (stdin, child.stdin.take()) {
            (Some(bytes), Some(mut pipe)) => {
                let bytes = bytes.to_vec();
                Some(std::thread::spawn(move || {
                    let _ = pipe.write_all(&bytes);
                }))
            }
            _ => None,
        };
        let out = child.wait_with_output()?;
        if let Some(f) = feeder {
            let _ = f.join();
        }
        anyhow::ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn attach_args_run_the_home_binary() {
            let a = attach_args("s1", 42, "claude", &["-p".into()]);
            assert_eq!(a[1], RUN_DOVE);
            assert_eq!(&a[3..], &["node", "attach", "s1", "--from", "42", "--", "claude", "-p"].map(String::from)[..]);
        }

        #[test]
        fn platform_names_match_uname() {
            if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
                assert!(same_platform("Linux", "x86_64"));
                assert!(!same_platform("Darwin", "arm64"));
            }
        }

        #[test]
        fn input_keeps_bytes_until_a_connection_takes_them() {
            let mut i = Input { pending: b"hi".to_vec(), ..Default::default() };
            i.flush();
            assert_eq!(i.pending, b"hi", "no connection yet: nothing lost");
        }
    }
}

/// One stdin frame: `u32` big-endian length, then the bytes.
fn frame(bytes: &[u8]) -> Vec<u8> {
    let mut f = (bytes.len() as u32).to_be_bytes().to_vec();
    f.extend_from_slice(bytes);
    f
}

/// The next frame; `None` when the connection closed between frames.
fn read_frame(r: &mut impl Read) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let mut bytes = vec![0u8; u32::from_be_bytes(len) as usize];
    r.read_exact(&mut bytes)?;
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_mark_eof() {
        let mut wire = frame(b"hello");
        wire.extend(frame(b""));
        let mut r = &wire[..];
        assert_eq!(read_frame(&mut r).unwrap(), Some(b"hello".to_vec()));
        assert_eq!(read_frame(&mut r).unwrap(), Some(Vec::new()));
        assert_eq!(read_frame(&mut r).unwrap(), None);
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn splits_at_double_dash() {
        let a = s(&["sess", "--from", "42", "--", "claude", "-p", "--", "x"]);
        let (head, argv) = split_argv(&a);
        assert_eq!(head, &s(&["sess", "--from", "42"])[..]);
        assert_eq!(argv, &s(&["claude", "-p", "--", "x"])[..]);
        let (session, from) = parse_from(head);
        assert_eq!((session.map(String::as_str), from), (Some("sess"), 42));
    }

    #[test]
    fn session_dirs_are_safe() {
        assert!(session_dir("../x").ends_with("___x"));
    }
}
