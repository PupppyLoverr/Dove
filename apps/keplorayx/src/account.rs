//! Your Keplorayx account: Woozlit, on the Cosmos edge, kept exactly as the
//! shipping Dove has it (its `remote.rs`, ported without its GPUI). The
//! `dove-mesh-host` sidecar owns the session: it signs in through the
//! browser and a loopback callback, keeps its files in `$DOVE_HOME/mesh`, and
//! talks to the edge. This side starts it, asks it to sign in or out, and
//! publishes what it says.
//!
//! Optional and off by default. Nothing is published to the account here:
//! remote control (publishing threads to the phone) stays the shipping
//! Dove's until its offer ("bring" or "fresh") is ported too, so signing in
//! sends no thread anywhere.
//!
//! Scripts see `kx.account`: { running, signed_in, connected, signing_in,
//! email, error }. Actions: `kx.account.sign_in`, `kx.account.cancel`,
//! `kx.account.sign_out`.

use std::{
    cell::RefCell,
    io::{BufRead as _, BufReader, Write as _},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    rc::Rc,
    sync::Arc,
};

use serde_json::{Value as Json, json};
use zgx_script::{ScriptHost, Shared, Value};

#[derive(Default, Clone)]
struct Status {
    running: bool,
    signed_in: bool,
    connected: bool,
    signing_in: bool,
    email: Option<String>,
    error: Option<String>,
}

struct Account {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    status: Status,
    shared: Shared,
}

impl Account {
    fn publish(&self) {
        let s = &self.status;
        self.shared.set(Value::record([
            ("running", Value::from(s.running)),
            ("signed_in", Value::from(s.signed_in)),
            ("connected", Value::from(s.connected)),
            ("signing_in", Value::from(s.signing_in)),
            ("email", Value::from(s.email.as_deref().unwrap_or(""))),
            ("error", Value::from(s.error.as_deref().unwrap_or(""))),
        ]));
    }

    fn send(&mut self, value: Json) {
        if let Some(stdin) = &mut self.stdin {
            let _ = writeln!(stdin, "{value}");
            let _ = stdin.flush();
        }
    }
}

/// The sidecar: `DOVE_MESH_HOST`, else beside this binary, else a build of
/// this checkout's `mesh/` (the newest, as the shipping app picks).
fn host_binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("DOVE_MESH_HOST").map(PathBuf::from).filter(|p| p.exists()) {
        return Some(p);
    }
    let name = if cfg!(windows) { "dove-mesh-host.exe" } else { "dove-mesh-host" };
    let mut candidates = Vec::new();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(repo) = manifest.parent().and_then(|p| p.parent()) {
        for profile in ["debug", "release"] {
            candidates.push(repo.join("mesh").join("target").join(profile).join(name));
        }
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.to_path_buf())) {
        candidates.push(dir.join(name));
        candidates.push(dir.join("../Resources").join(name));
    }
    candidates.into_iter().filter(|p| p.is_file()).max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
}

/// The sidecar's raw errors name protocol commands; the screen shouldn't.
fn friendly(message: &str) -> String {
    if message.contains("unknown variant") {
        "Restart Keplorayx so the account helper can read your account.".into()
    } else {
        message.chars().take(180).collect()
    }
}

pub fn install(host: &ScriptHost, tasks: &zgx_desktop::TaskSpawner, engine: Arc<dove_core::Engine>) {
    let account = Rc::new(RefCell::new(Account {
        child: None,
        stdin: None,
        status: Status::default(),
        shared: host.shared("kx.account", Value::Nil),
    }));
    account.borrow().publish();
    let data = engine.store.root().join("mesh");

    // Start the sidecar and follow what it says, on the window's thread.
    let start = {
        let (account, tasks) = (account.clone(), tasks.clone());
        Rc::new(move || -> bool {
            if account.borrow().child.is_some() {
                return true;
            }
            let Some(bin) = host_binary() else {
                let mut a = account.borrow_mut();
                a.status.error =
                    Some("The account helper (dove-mesh-host) isn't in this build. Build mesh/ to sign in.".into());
                a.publish();
                return false;
            };
            let mut child = match Command::new(bin)
                .arg("--data-dir")
                .arg(&data)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    let mut a = account.borrow_mut();
                    a.status.error = Some(format!("The account helper couldn't start: {e}"));
                    a.publish();
                    return false;
                }
            };
            let stdout = child.stdout.take().expect("piped");
            {
                let mut a = account.borrow_mut();
                a.stdin = child.stdin.take();
                a.child = Some(child);
                a.status = Status { running: true, ..Default::default() };
                a.publish();
            }
            let (tell, heard) = flume::unbounded::<Json>();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if let Ok(v) = serde_json::from_str::<Json>(&line)
                        && tell.send(v).is_err()
                    {
                        break;
                    }
                }
            });
            let account = account.clone();
            tasks.spawn(async move {
                while let Ok(ev) = heard.recv_async().await {
                    on_event(&account, &ev);
                }
                let mut a = account.borrow_mut();
                a.child = None;
                a.stdin = None;
                a.status.running = false;
                a.status.connected = false;
                a.publish();
            });
            true
        })
    };

    // Signed in before: the sidecar resumes the session, quietly.
    if engine.store.setting("kx.account.on").as_deref() == Some("1") {
        start();
    }

    let (a, s, e) = (account.clone(), start.clone(), engine.clone());
    host.action("kx.account.sign_in", move |_| {
        let _ = e.store.set_setting("kx.account.on", "1");
        if !s() {
            return;
        }
        let mut a = a.borrow_mut();
        a.status.signing_in = true;
        a.status.error = None;
        a.send(json!({ "op": "sign_in" }));
        a.publish();
    });

    let a = account.clone();
    host.action("kx.account.cancel", move |_| {
        let mut a = a.borrow_mut();
        a.send(json!({ "op": "cancel_sign_in" }));
        a.status.signing_in = false;
        a.status.error = None;
        a.publish();
    });

    let (a, e) = (account, engine);
    host.action("kx.account.sign_out", move |_| {
        let _ = e.store.set_setting("kx.account.on", "0");
        let mut a = a.borrow_mut();
        a.send(json!({ "op": "sign_out" }));
        a.send(json!({ "op": "shutdown" }));
        a.stdin = None;
        if let Some(mut child) = a.child.take() {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let _ = child.kill();
                let _ = child.wait();
            });
        }
        a.status = Status::default();
        a.publish();
    });
}

fn on_event(account: &Rc<RefCell<Account>>, ev: &Json) {
    let s = |k: &str| ev.get(k).and_then(Json::as_str).map(str::to_owned);
    let mut a = account.borrow_mut();
    match ev.get("ev").and_then(Json::as_str).unwrap_or_default() {
        "status" => {
            a.status.signed_in = ev.get("signedIn").and_then(Json::as_bool).unwrap_or(false);
            a.status.connected = ev.get("connected").and_then(Json::as_bool).unwrap_or(false);
            a.status.email = s("email");
            if a.status.signed_in {
                a.status.signing_in = false;
            }
        }
        "connected" => {
            a.status.connected = true;
            a.status.error = None;
        }
        "sign_in_url" => {
            if let Some(url) = s("url") {
                crate::open_external(&url);
            }
        }
        "error" => {
            a.status.signing_in = false;
            a.status.error = s("message").map(|m| friendly(&m));
        }
        _ => return,
    }
    a.publish();
}
