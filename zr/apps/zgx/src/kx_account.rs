//! Keplorayx's account in Zeron's interface: Woozlit, on the Cosmos edge,
//! signed into by the `dove-mesh-host` sidecar exactly as Keplorayx always
//! has (apps/keplorayx/src/account.rs), behind the calls Zeron's sign-in and
//! setup make (src/auth.rs lists them). Zeron's own account (WorkOS, its
//! edge) is never asked: the engine stays a local one, signed out of Zeron.
//!
//! What Zeron kept on its server stays on this computer instead, in
//! `<data>/account.json`: your username, your member card (made here from
//! your id with Zeron's own seed, ds/card_seed, so it is the same card
//! wherever it is drawn), the setup pages you finished, and your
//! organization's name. Invites need a server to send their emails, so none
//! are sent: each comes back failed, saying so.
//!
//! `account` as Zeron's: { state = "restoring" | "signed_out" | "waiting" |
//! "signed_in", user = { id, email, first_name }?, failure?, env }.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io::{BufRead as _, BufReader, Write as _},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    rc::Rc,
    time::Duration,
};

use serde_json::{Value as Json, json};
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Shared, Value};

use crate::auth::text;

/// Where Keplorayx keeps what it knows of you: the engine's data folder
/// (~/.keplorayx).
pub fn data_dir() -> PathBuf {
    let account = crate::auth::endpoint();
    crate::engine::manager::Config::from_env(&account.edge, &account.client, account.port)
        .map(|c| c.data_dir)
        .unwrap_or_else(|_| home().join(".keplorayx"))
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default()
}

fn kept_path() -> PathBuf {
    data_dir().join("account.json")
}

fn kept() -> Json {
    std::fs::read_to_string(kept_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| json!({}))
}

fn keep(value: &Json) {
    let path = kept_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, serde_json::to_string_pretty(value).unwrap_or_default());
}

/// Whether you signed in before (a launch signed in skips the intro).
pub fn session_exists() -> bool {
    kept()["signed_in"].as_bool().unwrap_or(false)
}

/// Who is signed in (a conversation link's workspace is hashed from it).
pub fn signed_in() -> Option<(String, Option<String>)> {
    ACCOUNT.with(|a| {
        let s = a.status.borrow();
        s.signed_in.then(|| (s.email.clone().unwrap_or_default(), None))
    })
}

#[derive(Default, Clone)]
struct Status {
    started: bool,
    signed_in: bool,
    signing_in: bool,
    email: Option<String>,
    failure: Option<String>,
}

struct Account {
    child: RefCell<Option<Child>>,
    stdin: RefCell<Option<ChildStdin>>,
    status: RefCell<Status>,
    shares: RefCell<Vec<Shared>>,
    /// Sign-ins waiting on the browser: each told how it ended.
    waiting: RefCell<Vec<zgx::task::LatestSender<Result<(), String>>>>,
    restoring: Cell<bool>,
    /// The account's sessions (every device's, from the Cosmos registry),
    /// as the helper last listed them, and who shows them.
    synced: RefCell<Json>,
    synced_shares: RefCell<Vec<Shared>>,
    /// Transcripts asked for, waiting on their chat room.
    pulls: RefCell<HashMap<String, Vec<zgx::task::LatestSender<Json>>>>,
}

thread_local! {
    static ACCOUNT: Rc<Account> = Rc::new(Account {
        child: RefCell::default(),
        stdin: RefCell::default(),
        status: RefCell::default(),
        shares: RefCell::default(),
        waiting: RefCell::default(),
        restoring: Cell::new(false),
        synced: RefCell::new(read_json(&synced_path()).unwrap_or_else(|| json!([]))),
        synced_shares: RefCell::default(),
        pulls: RefCell::default(),
    });
}

impl Account {
    fn user(&self) -> Value {
        let s = self.status.borrow();
        let email = s.email.clone().unwrap_or_default();
        let kept = kept();
        let first = kept["first_name"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| kept["username"].as_str().map(str::to_owned))
            .unwrap_or_else(|| email.split('@').next().unwrap_or("").to_owned());
        Value::record([
            ("id", Value::from(email.as_str())),
            ("email", Value::from(email.as_str())),
            ("first_name", Value::from(first.as_str())),
        ])
    }

    fn value(&self) -> Value {
        let s = self.status.borrow().clone();
        let state = if s.signed_in {
            "signed_in"
        } else if s.signing_in {
            "waiting"
        } else if self.restoring.get() {
            "restoring"
        } else {
            "signed_out"
        };
        Value::record([
            ("state", Value::from(state)),
            ("user", if s.signed_in { self.user() } else { Value::Nil }),
            ("failure", s.failure.as_deref().map_or(Value::Nil, Value::from)),
            ("env", Value::from("production")),
        ])
    }

    fn publish(&self) {
        let v = self.value();
        for share in self.shares.borrow().iter() {
            share.set(v.clone());
        }
    }

    fn publish_synced(&self) {
        let v = crate::kx::to_value(&self.synced.borrow());
        for share in self.synced_shares.borrow().iter() {
            share.set(v.clone());
        }
    }

    fn send(&self, value: Json) {
        if let Some(stdin) = self.stdin.borrow_mut().as_mut() {
            let _ = writeln!(stdin, "{value}");
            let _ = stdin.flush();
        }
    }

    fn settle(&self, outcome: Result<(), String>) {
        for tell in self.waiting.borrow_mut().drain(..) {
            tell.send(outcome.clone());
        }
    }

    fn on_event(&self, ev: &Json) {
        let s = |k: &str| ev.get(k).and_then(Json::as_str).map(str::to_owned);
        match ev.get("ev").and_then(Json::as_str).unwrap_or_default() {
            "status" => {
                let signed_in = ev.get("signedIn").and_then(Json::as_bool).unwrap_or(false);
                {
                    let mut st = self.status.borrow_mut();
                    st.signed_in = signed_in;
                    st.email = s("email").or(st.email.take());
                    if signed_in {
                        st.signing_in = false;
                        st.failure = None;
                    }
                }
                self.restoring.set(false);
                let mut k = kept();
                k["signed_in"] = json!(signed_in);
                keep(&k);
                if signed_in {
                    self.settle(Ok(()));
                    // (The helper lists them once its registry connects; asked
                    // again, in case that was before we listened.)
                    self.send(json!({ "op": "list_sessions" }));
                }
            }
            "account_sessions" => {
                let Some(chats) = ev.get("chats").and_then(Json::as_array) else { return };
                let device = s("deviceId").unwrap_or_default();
                let rows: Vec<Json> = chats
                    .iter()
                    .filter(|c| c["id"].as_str().is_some_and(|id| !id.is_empty()))
                    .map(|c| {
                        let mut c = c.clone();
                        c["here"] = json!(c["deviceId"].as_str() == Some(device.as_str()));
                        c
                    })
                    .collect();
                // (An empty list while rows were known is a pull not yet in.)
                if rows.is_empty() && self.synced.borrow().as_array().is_some_and(|a| !a.is_empty()) {
                    return;
                }
                let rows = Json::Array(rows);
                write_json(&synced_path(), &rows);
                *self.synced.borrow_mut() = rows;
                self.publish_synced();
                return;
            }
            "account_transcript" => {
                let Some(chat) = s("chat") else { return };
                let messages = ev.get("messages").cloned().unwrap_or_else(|| json!([]));
                if messages.as_array().is_none_or(|m| m.is_empty()) {
                    return;
                }
                let flat = flatten(&messages);
                write_json(&transcript_path(&chat), &flat);
                for tell in self.pulls.borrow_mut().remove(&chat).unwrap_or_default() {
                    tell.send(flat.clone());
                }
                return;
            }
            "sign_in_url" => {
                if let Some(url) = s("url") {
                    let _ = crate::auth::open_sign_in(&url);
                }
            }
            "error" => {
                {
                    let mut st = self.status.borrow_mut();
                    st.signing_in = false;
                    st.failure = s("message");
                }
                self.restoring.set(false);
                self.settle(Err("failed".into()));
            }
            _ => return,
        }
        self.publish();
    }
}

/// The sidecar: `DOVE_MESH_HOST`, else beside this app, else the build
/// Keplorayx keeps of it.
fn host_binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("DOVE_MESH_HOST").map(PathBuf::from).filter(|p| p.exists()) {
        return Some(p);
    }
    let name = if cfg!(windows) { "dove-mesh-host.exe" } else { "dove-mesh-host" };
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.to_path_buf())) {
        candidates.push(dir.join(name));
    }
    candidates.push(PathBuf::from("C:/dvw/target-kx-zgx/mesh/debug").join(name));
    candidates.into_iter().find(|p| p.is_file())
}

fn start(tasks: &zgx_desktop::TaskSpawner) -> bool {
    let account = ACCOUNT.with(Rc::clone);
    if account.child.borrow().is_some() {
        return true;
    }
    let Some(bin) = host_binary() else {
        account.status.borrow_mut().failure =
            Some("The account helper (dove-mesh-host) isn't beside Keplorayx.".into());
        account.publish();
        return false;
    };
    let mut command = Command::new(bin);
    command.arg("--data-dir").arg(data_dir().join("mesh")).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // (No console window of its own.)
        command.creation_flags(0x0800_0000);
    }
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            account.status.borrow_mut().failure = Some(format!("The account helper couldn't start: {e}"));
            account.publish();
            return false;
        }
    };
    let stdout = child.stdout.take().expect("piped");
    *account.stdin.borrow_mut() = child.stdin.take();
    *account.child.borrow_mut() = Some(child);
    account.status.borrow_mut().started = true;
    let (tell, mut heard) = zgx::task::latest::<Vec<Json>>();
    let queue = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Json>::new()));
    {
        let queue = queue.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Json>(&line) {
                    let batch = {
                        let mut q = queue.lock().unwrap();
                        q.push(v);
                        q.clone()
                    };
                    if !tell.send(batch) {
                        break;
                    }
                }
            }
        });
    }
    let a = account.clone();
    tasks.spawn(async move {
        // (`latest` keeps the newest batch; the queue keeps every line, so
        // none is lost between reads.)
        while heard.recv().await.is_some() {
            let lines: Vec<Json> = std::mem::take(&mut *queue.lock().unwrap());
            for ev in &lines {
                a.on_event(ev);
            }
        }
        *a.child.borrow_mut() = None;
        *a.stdin.borrow_mut() = None;
        a.status.borrow_mut().started = false;
        a.publish();
    });
    true
}

/// `account` and the `account.*` calls, Woozlit's.
pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    let account = ACCOUNT.with(Rc::clone);
    account.shares.borrow_mut().push(host.shared("account", account.value()));
    account
        .synced_shares
        .borrow_mut()
        .push(host.shared("kx.account.sessions", crate::kx::to_value(&account.synced.borrow())));
    let tasks = cx.tasks.clone();
    if !account.status.borrow().started && session_exists() {
        // Signed in before: the sidecar resumes the session, quietly.
        account.restoring.set(true);
        start(&tasks);
        account.publish();
        let a = account.clone();
        tasks.spawn(async move {
            zgx::timer::sleep(Duration::from_secs(8)).await;
            if a.restoring.get() {
                a.restoring.set(false);
                a.publish();
            }
        });
    }

    let t = tasks.clone();
    host.async_action("account.sign_in", move |_kind| {
        let account = ACCOUNT.with(Rc::clone);
        let started = start(&t);
        let (tell, mut heard) = zgx::task::latest::<Result<(), String>>();
        if started {
            account.waiting.borrow_mut().push(tell.clone());
            {
                let mut st = account.status.borrow_mut();
                st.signing_in = true;
                st.failure = None;
            }
            account.send(json!({ "op": "sign_in" }));
            account.publish();
            // (Given up on after ten minutes, as Zeron's is.)
            let a = account.clone();
            t.spawn(async move {
                zgx::timer::sleep(Duration::from_secs(600)).await;
                if a.status.borrow().signing_in {
                    a.status.borrow_mut().signing_in = false;
                    a.settle(Err("timed_out".into()));
                    a.publish();
                }
            });
        } else {
            tell.send(Err("failed".into()));
        }
        async move {
            heard.recv().await.unwrap_or_else(|| Err("cancelled".into()))?;
            Ok(ACCOUNT.with(|a| a.user()))
        }
    });
    host.action("account.cancel", |_| {
        let a = ACCOUNT.with(Rc::clone);
        a.send(json!({ "op": "cancel_sign_in" }));
        a.status.borrow_mut().signing_in = false;
        a.settle(Err("cancelled".into()));
        a.publish();
    });
    host.async_action("account.sign_out", |_| {
        let a = ACCOUNT.with(Rc::clone);
        a.send(json!({ "op": "sign_out" }));
        a.send(json!({ "op": "shutdown" }));
        *a.stdin.borrow_mut() = None;
        if let Some(mut child) = a.child.borrow_mut().take() {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(2));
                let _ = child.kill();
                let _ = child.wait();
            });
        }
        *a.status.borrow_mut() = Status::default();
        let mut k = kept();
        k["signed_in"] = json!(false);
        keep(&k);
        a.publish();
        async move { Ok(Value::Nil) }
    });
    // The account's sessions: listed again, and one's transcript (kept here
    // once read; asked of its chat room otherwise, for up to 20 seconds).
    host.action("kx.account.refresh", |_| {
        ACCOUNT.with(|a| a.send(json!({ "op": "list_sessions" })));
    });
    let t = tasks.clone();
    host.async_action("kx.account.transcript", move |id| {
        let id = text(&id);
        let kept = read_json(&transcript_path(&id));
        let (tell, mut heard) = zgx::task::latest::<Json>();
        let a = ACCOUNT.with(Rc::clone);
        if kept.is_none() && !id.is_empty() && a.stdin.borrow().is_some() {
            a.pulls.borrow_mut().entry(id.clone()).or_default().push(tell.clone());
            a.send(json!({ "op": "pull_chat", "id": id }));
            t.spawn(async move {
                zgx::timer::sleep(Duration::from_secs(20)).await;
                tell.send(json!([]));
            });
        } else {
            tell.send(kept.unwrap_or_else(|| json!([])));
        }
        async move {
            let got = heard.recv().await.unwrap_or_else(|| json!([]));
            Ok(crate::kx::to_value(&got))
        }
    });
    host.async_action("account.username", |_| async move {
        Ok(kept()["username"].as_str().map_or(Value::Nil, Value::from))
    });
    host.async_action("account.check_username", |name| {
        let name = text(&name).to_lowercase();
        async move {
            let reason = username_problem(&name);
            Ok(Value::record([
                ("available", Value::from(reason.is_none())),
                ("reason", reason.map_or(Value::Nil, Value::from)),
            ]))
        }
    });
    host.async_action("account.claim_username", |name| {
        let name = text(&name).to_lowercase();
        async move {
            if let Some(reason) = username_problem(&name) {
                return Err(reason.into());
            }
            let mut k = kept();
            if k["username"].as_str().is_some_and(|u| u != name) && k["card"].is_object() {
                // (A card once dealt stays: its number and day.)
            }
            k["username"] = json!(name);
            let card = make_card(&name, &k);
            k["card"] = card.clone();
            keep(&k);
            Ok(card_value(&card))
        }
    });
    host.async_action("account.card", |name| {
        let name = text(&name).to_lowercase();
        async move {
            let k = kept();
            Ok(match k["card"].as_object() {
                Some(_) if name.is_empty() || k["username"].as_str() == Some(name.as_str()) => card_value(&k["card"]),
                _ => Value::Nil,
            })
        }
    });
    host.async_action("account.orgs", |_| async move {
        let k = kept();
        Ok(Value::List(
            k["org"]["name"]
                .as_str()
                .map(|name| {
                    vec![Value::record([
                        ("id", Value::from("local")),
                        ("organization_id", Value::from("local")),
                        ("name", Value::from(name)),
                        ("role", Value::from("owner")),
                    ])]
                })
                .unwrap_or_default()
                .into(),
        ))
    });
    host.async_action("account.create_org", |name| {
        let name = text(&name);
        async move {
            let mut k = kept();
            k["org"] = json!({ "name": name });
            keep(&k);
            Ok(Value::record([("organization_id", Value::from("local"))]))
        }
    });
    host.async_action("account.choose_org", |id| {
        let id = text(&id);
        async move { Ok(Value::record([("organization_id", Value::from(id.as_str()))])) }
    });
    host.async_action("account.onboarding", |_| async move { Ok(done(&kept())) });
    host.async_action("account.onboarded", |ids| {
        let ids: Vec<String> = match &ids {
            Value::List(list) => list.iter().map(text).collect(),
            _ => Vec::new(),
        };
        async move {
            let mut k = kept();
            if !k["done"].is_object() {
                k["done"] = json!({});
            }
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            for id in ids {
                k["done"][id] = json!(now);
            }
            keep(&k);
            Ok(done(&k))
        }
    });
    host.async_action("account.invite", |args| {
        let emails: Vec<String> = match args.field("emails") {
            Some(Value::List(list)) => list.iter().map(text).collect(),
            _ => Vec::new(),
        };
        async move {
            let failed: Vec<Value> = emails
                .iter()
                .map(|e| {
                    Value::record([
                        ("email", Value::from(e.as_str())),
                        ("error", Value::from("Invites aren't sent yet: share Keplorayx with them instead.")),
                    ])
                })
                .collect();
            Ok(Value::record([("sent", Value::from(0.)), ("failed", Value::List(failed.into()))]))
        }
    });
}

fn synced_path() -> PathBuf {
    data_dir().join("account").join("sessions.json")
}

fn transcript_path(id: &str) -> PathBuf {
    let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    data_dir().join("account").join("transcripts").join(format!("{safe}.json"))
}

fn read_json(path: &std::path::Path) -> Option<Json> {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok())
}

fn write_json(path: &std::path::Path, value: &Json) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, serde_json::to_string(value).unwrap_or_default());
}

/// A chat room's messages as the page reads them: each `{ role, blocks }`,
/// a block `{ kind = "text" | "reasoning" | "tool" | "error", text, name? }`
/// (a tool's `text` is what it acted on: a command, a path, a query).
fn flatten(messages: &Json) -> Json {
    let mut out = Vec::new();
    for m in messages.as_array().into_iter().flatten() {
        let role = m["role"].as_str().unwrap_or("assistant");
        let mut blocks = Vec::new();
        for part in m["parts"].as_array().into_iter().flatten() {
            match part["kind"].as_str().unwrap_or_default() {
                "text" => {
                    let t = part["text"].as_str().unwrap_or_default();
                    if !t.trim().is_empty() {
                        blocks.push(json!({ "kind": "text", "text": t }));
                    }
                }
                "reasoning" => {
                    let t = part["reasoning"].as_str().or(part["text"].as_str()).unwrap_or_default();
                    if !t.trim().is_empty() {
                        blocks.push(json!({ "kind": "reasoning", "text": t }));
                    }
                }
                "tool" => {
                    let call = &part["call"];
                    let kind = call["kind"].as_str().unwrap_or("tool");
                    let (name, what) = match kind {
                        "exec" => ("Ran", &call["command"]),
                        "readFile" => ("Read", &call["path"]),
                        "writeFile" | "editFile" | "applyPatch" => ("Edited", &call["path"]),
                        "search" | "glob" => ("Searched", &call["pattern"]),
                        "webFetch" => ("Fetched", &call["url"]),
                        "webSearch" => ("Searched the web", &call["query"]),
                        "mcp" => ("Used", &call["tool"]),
                        _ => ("Used", &call["name"]),
                    };
                    let what = what.as_str().map(str::to_owned).unwrap_or_else(|| kind.to_owned());
                    blocks.push(json!({
                        "kind": "tool", "name": name, "text": what,
                        "error": part["isError"].as_bool() == Some(true),
                    }));
                }
                "error" => blocks.push(json!({ "kind": "error", "text": part["message"].as_str().unwrap_or_default() })),
                _ => {}
            }
        }
        if !blocks.is_empty() {
            out.push(json!({ "role": role, "blocks": blocks }));
        }
    }
    Json::Array(out)
}

/// The finished setup pages' ids.
fn done(k: &Json) -> Value {
    Value::List(
        k["done"]
            .as_object()
            .map(|d| d.keys().map(|id| Value::from(id.as_str())).collect::<Vec<_>>())
            .unwrap_or_default()
            .into(),
    )
}

/// Why `name` can't be a username, if it can't.
fn username_problem(name: &str) -> Option<&'static str> {
    let ok_len = (3..=24).contains(&name.len());
    let ok_chars = name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if !ok_len || !ok_chars || name.starts_with(['-', '_']) {
        return Some("invalid");
    }
    if matches!(name, "admin" | "keplorayx" | "support" | "help" | "root" | "zeron" | "woozlit" | "cosmos") {
        return Some("reserved");
    }
    None
}

/// A member card made here, as Zeron's server makes one: the scene, foil and
/// sky from the user's id by Zeron's own seed (ds/card_seed), a number, and
/// the day you joined.
fn make_card(username: &str, k: &Json) -> Json {
    let seed = ACCOUNT.with(|a| a.status.borrow().email.clone()).filter(|e| !e.is_empty()).unwrap_or_else(|| username.to_owned());
    const SCENES: u32 = 21;
    const HOLOS: [&str; 5] = ["aurora", "reverse", "secret", "starlight", "glyph"];
    let number = k["card"]["userNumber"].as_u64().unwrap_or_else(|| (hash(&format!("{seed}:number")) % 9000 + 1000) as u64);
    let joined = k["card"]["joinedOn"].as_str().map(str::to_owned).unwrap_or_else(today);
    let first = k["first_name"].as_str().map(str::to_owned).unwrap_or_else(|| {
        let mut s = username.to_owned();
        if let Some(c) = s.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        s
    });
    json!({
        "username": username,
        "firstName": first,
        "userNumber": number,
        "joinedOn": joined,
        "badges": [{ "id": "early", "name": "Early", "icon": "star" }],
        "scene": hash(&format!("{seed}:scene")) % SCENES,
        "holo": HOLOS[(hash(&format!("{seed}:holo")) % HOLOS.len() as u32) as usize],
        "sky": hash(&format!("{seed}:sky")),
    })
}

fn today() -> String {
    // (UTC's date, as the server's.)
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0) as i64;
    let (y, m, d) = civil(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 to a date (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// ds/card_seed's hash: FNV-1a 32 over the bytes, then murmur3's fmix32.
fn hash(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h
}

/// A card as Zeron's scripts take it.
fn card_value(card: &Json) -> Value {
    let str = |key: &str| card[key].as_str().map_or(Value::Nil, Value::from);
    let num = |key: &str| card[key].as_f64().map_or(Value::Nil, Value::from);
    let badges: Vec<Value> = card["badges"]
        .as_array()
        .map(|badges| {
            badges
                .iter()
                .map(|b| {
                    Value::record([
                        ("id", b["id"].as_str().map_or(Value::Nil, Value::from)),
                        ("name", b["name"].as_str().map_or(Value::Nil, Value::from)),
                        ("icon", b["icon"].as_str().map_or(Value::Nil, Value::from)),
                    ])
                })
                .collect()
        })
        .unwrap_or_default();
    Value::record([
        ("username", str("username")),
        ("first_name", str("firstName")),
        ("number", num("userNumber")),
        ("joined_on", str("joinedOn")),
        ("badges", Value::List(badges.into())),
        ("scene", num("scene")),
        ("holo", str("holo")),
        ("sky", num("sky")),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same hash as ds/card_seed's, so the card is the one Zeron's
    /// seed makes.
    #[test]
    fn the_hash_is_card_seeds() {
        // FNV-1a of "" is its offset; fmix32 of it, by hand.
        let mut h: u32 = 0x811c_9dc5;
        h ^= h >> 16;
        h = h.wrapping_mul(0x85eb_ca6b);
        h ^= h >> 13;
        h = h.wrapping_mul(0xc2b2_ae35);
        h ^= h >> 16;
        assert_eq!(hash(""), h);
        assert_ne!(hash("a:scene"), hash("b:scene"));
    }

    #[test]
    fn usernames() {
        assert_eq!(username_problem("mara"), None);
        assert_eq!(username_problem("ma"), Some("invalid"));
        assert_eq!(username_problem("Mara!"), Some("invalid"));
        assert_eq!(username_problem("keplorayx"), Some("reserved"));
    }

    #[test]
    fn transcripts_flatten() {
        let m = json!([
            { "role": "user", "parts": [{ "kind": "text", "text": "fix it" }] },
            { "role": "assistant", "parts": [
                { "kind": "tool", "call": { "kind": "exec", "command": "cargo test" }, "isError": false },
                { "kind": "text", "text": "Done." },
                { "kind": "text", "text": "  " }
            ] },
            { "role": "assistant", "parts": [] }
        ]);
        let f = flatten(&m);
        assert_eq!(f.as_array().unwrap().len(), 2);
        assert_eq!(f[1]["blocks"][0]["name"], "Ran");
        assert_eq!(f[1]["blocks"][0]["text"], "cargo test");
        assert_eq!(f[1]["blocks"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_398), (2025, 11, 6));
    }
}
