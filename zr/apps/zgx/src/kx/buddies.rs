//! Buddies: named bots with a job. A buddy is a record (`buddies.json`), a
//! folder of its own (`<data>/buddies/<id>/`, where its chat works and where
//! its standing instructions are written as `AGENTS.md` and `CLAUDE.md`, the
//! files coding agents read by themselves), and a chat on the engine. What a
//! buddy runs on is an ordered list of agents and models; which one a run
//! takes is the scripts' choice (ui/kx/buddies), made from who is signed in
//! and under their limit when it starts.
//!
//! Automations (`automations.json`) are prompts sent to a buddy by the
//! clock: every so many minutes, daily at a time on chosen weekdays, or once.
//! This file keeps the clock. What comes due is listed in
//! `kx.automations.due`; the scripts send it and say how it went
//! (`kx.automation.ran`), and each run is kept (`buddy-runs.json`, the last
//! few hundred). Runs happen while the app is open: one that came due while
//! it was closed is not replayed, the next is counted from now.
//!
//!   kx.buddies             [buddy]                      (shared)
//!   kx.automations         [automation, with `next`]    (shared)
//!   kx.automations.due     [{ run, automation, buddy, prompt, at }]
//!   kx.buddy.runs          [run], newest first          (shared)
//!   kx.buddy.save(buddy) → id      kx.buddy.remove(id)
//!   kx.automation.save(a) → id     kx.automation.remove(id)
//!   kx.automation.run_now(id)      kx.automation.ran({ run, status, note, chat })
//!   kx.buddy.send({ chat, prompt, harness, model, cwd, reasoning? })
//!                          a message to a chat by its id, as a turn of its own

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use chrono::{DateTime, Datelike, Local, NaiveTime, TimeZone};
use serde_json::{Value as Json, json};
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Value};

/// How often the clock is read.
const TICK: Duration = Duration::from_secs(15);
/// A run the scripts never answered for is written off after this.
const UNANSWERED_SECS: i64 = 300;
/// How many runs are kept.
const RUNS_KEPT: usize = 300;
/// The most an automation runs in a day unless it says otherwise: a typo in
/// its minutes never spends a night's worth of turns.
const DAILY_CAP: u64 = 24;

fn now() -> i64 {
    Local::now().timestamp()
}

fn read(path: &Path) -> Vec<Json> {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str::<Vec<Json>>(&s).ok()).unwrap_or_default()
}

/// Written whole, then named: nothing reads half a list.
fn write(path: &Path, list: &[Json]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let half = path.with_extension("tmp");
    if std::fs::write(&half, serde_json::to_string_pretty(list).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&half, path);
    }
}

fn new_id(prefix: &str) -> String {
    let id = crate::sessions::new_id();
    format!("{prefix}{}", id.chars().filter(|c| c.is_ascii_alphanumeric()).take(12).collect::<String>())
}

/// An id of ours (letters and digits), never a path.
fn plain(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// What a buddy is told, every turn, by the files its agent reads on its
/// own. Plain words: who it is, its job, where it works, what waits for a yes.
fn instructions(buddy: &Json, home: &Path) -> String {
    let name = buddy["name"].as_str().unwrap_or("Buddy");
    let role = buddy["role"].as_str().unwrap_or_default();
    let job = buddy["instructions"].as_str().unwrap_or_default().trim();
    let folder = buddy["folder"].as_str().filter(|f| !f.trim().is_empty());
    let may = match buddy["permissions"].as_str().unwrap_or("draft") {
        "folder" => "You may read, write and run anything inside your own folder and your working folder without asking. Ask before acting on anything outside them, and before sending, posting, paying for or deleting anything on an account.",
        "ask" => "Ask before every action that changes something: writing or deleting a file, running a command that changes state, sending, posting, paying. Reading and looking things up need no asking.",
        _ => "You may read, browse and look things up, and write drafts and notes in your own folder, without asking. Ask before sending, posting, paying for or deleting anything, and before changing files outside your own folder.",
    };
    let mut out = format!("# {name}\n\nYou are {name}, one of the person's buddies in Keplorayx: a standing assistant with a job of your own.");
    if !role.is_empty() {
        out.push_str(&format!(" Your role: {role}."));
    }
    out.push_str("\n\n## Your job\n\n");
    out.push_str(if job.is_empty() { "No job has been written yet. Ask the person what they want you to look after." } else { job });
    out.push_str(&format!("\n\n## Where you work\n\nYour own folder is `{}`. Keep your notes, drafts and what you want to remember between runs here (a `notes.md` is a good place: read it when you start).", home.display()));
    if let Some(folder) = folder {
        out.push_str(&format!(" The folder you look after is `{folder}`."));
    }
    match buddy["computer"]["kind"].as_str().unwrap_or_default() {
        "local" => {
            if let Some(computer) = buddy["computer"]["name"].as_str().filter(|n| !n.is_empty()) {
                out.push_str(&format!("\n\n## Your computer\n\nYou have a computer of your own: the cua sandbox `{computer}`, a real desktop the person can watch. Work that needs a desktop or a clean machine happens there, never on the person's own. Drive it with the `cua` command line: `cua sandbox exec {computer} -- <command>` runs a command in it, `cua sandbox screenshot {computer} -o shot.png` saves a picture of its screen, and `cua do --help` lists the mouse and keyboard actions. If it is stopped, `cua sandbox resume {computer}` starts it."));
            }
        }
        "boat" => {
            if let Some(id) = buddy["computer"]["id"].as_str().filter(|n| !n.is_empty()) {
                out.push_str(&format!("\n\n## Your computer\n\nYou have a computer of your own in the cloud: the Boat sandbox `{id}` (boat.dev), a Linux desktop the person can watch from Keplorayx. Work that needs a desktop or a clean machine belongs there, never on the person's own. If the `boat` command line is installed and signed in on this machine, `boat --help` shows how to run commands in it; if it isn't, say so rather than working on the person's computer instead. It costs the person money while it runs, so don't leave long idle work on it."));
            }
        }
        _ => {}
    }
    out.push_str(&format!("\n\n## What needs a yes\n\n{may}\n\n## How to work\n\n- You are run again and again, sometimes by a schedule with nobody watching. Say what you did and what is left, briefly, at the end of each run.\n- Keplorayx's `browser_*` tools drive the browser beside your chat; the person sees your cursor there.\n- If something is unclear or risky, stop and ask rather than guess.\n"));
    out
}

/// A buddy's folder made, with its instructions as its agent's files.
fn settle_home(dir: &Path, buddy: &Json) -> PathBuf {
    let home = dir.join("buddies").join(buddy["id"].as_str().unwrap_or_default());
    let _ = std::fs::create_dir_all(&home);
    let text = instructions(buddy, &home);
    for name in ["AGENTS.md", "CLAUDE.md"] {
        let path = home.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            let _ = std::fs::write(&path, &text);
        }
    }
    home
}

/// When an automation next runs, after `after` (unix seconds, local time
/// for clock times); None for one that never will (off, done, or unreadable).
pub fn next_run(a: &Json, after: i64) -> Option<i64> {
    if a["enabled"].as_bool() == Some(false) {
        return None;
    }
    let last = a["last_run"].as_i64();
    match a["kind"].as_str().unwrap_or("daily") {
        "interval" => {
            let every = a["minutes"].as_i64().unwrap_or(60).max(1) * 60;
            Some(match last {
                // (Counted from when it last ran, never from before now.)
                Some(last) => (last + every).max(after + 1),
                None => after + every,
            })
        }
        "once" => {
            let at = a["at"].as_i64()?;
            (last.is_none() && at > 0).then_some(at.max(after + 1))
        }
        _ => {
            let time = NaiveTime::parse_from_str(a["time"].as_str().unwrap_or("09:00"), "%H:%M").ok()?;
            // (Weekdays as numbers, Monday 0; none given, every day.)
            let days: Vec<u32> = a["days"].as_array().map(|d| d.iter().filter_map(|n| n.as_u64().map(|n| n as u32)).collect()).unwrap_or_default();
            let from: DateTime<Local> = Local.timestamp_opt(after, 0).single()?;
            (0..8).find_map(|ahead| {
                let day = from.date_naive() + chrono::Days::new(ahead);
                if !days.is_empty() && !days.contains(&day.weekday().num_days_from_monday()) {
                    return None;
                }
                let at = Local.from_local_datetime(&day.and_time(time)).earliest()?.timestamp();
                (at > after).then_some(at)
            })
        }
    }
}

struct State {
    dir: PathBuf,
    buddies: RefCell<Vec<Json>>,
    automations: RefCell<Vec<Json>>,
    runs: RefCell<Vec<Json>>,
    /// What came due and waits for the scripts' word.
    due: RefCell<Vec<Json>>,
    shared: RefCell<Option<Shared>>,
}

struct Shared {
    buddies: zgx_script::Shared,
    automations: zgx_script::Shared,
    runs: zgx_script::Shared,
    due: zgx_script::Shared,
}

impl State {
    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// The buddies as the scripts read them: each with its folder.
    fn buddies_value(&self) -> Value {
        let list: Vec<Json> = self
            .buddies
            .borrow()
            .iter()
            .map(|b| {
                let mut b = b.clone();
                b["home"] = json!(self.dir.join("buddies").join(b["id"].as_str().unwrap_or_default()).to_string_lossy());
                b
            })
            .collect();
        crate::kx::to_value(&Json::Array(list))
    }

    /// The automations as the scripts read them: each with when it's next.
    fn automations_value(&self) -> Value {
        let at = now();
        let list: Vec<Json> = self
            .automations
            .borrow()
            .iter()
            .map(|a| {
                let mut a = a.clone();
                a["next"] = json!(a["next_run"].as_i64().or_else(|| next_run(&a, at)));
                a
            })
            .collect();
        crate::kx::to_value(&Json::Array(list))
    }

    fn publish(&self) {
        if let Some(s) = self.shared.borrow().as_ref() {
            s.buddies.set(self.buddies_value());
            s.automations.set(self.automations_value());
            s.runs.set(crate::kx::to_value(&Json::Array(self.runs.borrow().clone())));
            s.due.set(crate::kx::to_value(&Json::Array(self.due.borrow().clone())));
        }
    }

    fn save_buddy(&self, mut buddy: Json) -> Option<String> {
        if !buddy.is_object() {
            return None;
        }
        let id = buddy["id"].as_str().filter(|id| plain(id)).map(str::to_owned).unwrap_or_else(|| new_id("b"));
        buddy["id"] = json!(id);
        // (Its folder is ours to say, never the caller's.)
        buddy.as_object_mut()?.remove("home");
        let name = buddy["name"].as_str().unwrap_or_default().trim().chars().take(60).collect::<String>();
        buddy["name"] = json!(if name.is_empty() { "Buddy".to_owned() } else { name });
        {
            let mut list = self.buddies.borrow_mut();
            match list.iter().position(|b| b["id"] == buddy["id"]) {
                Some(i) => {
                    // (What isn't said again stays: its chat, when it was made.)
                    let old = list[i].clone();
                    for (k, v) in old.as_object().into_iter().flatten() {
                        if buddy.get(k).is_none() {
                            buddy[k.as_str()] = v.clone();
                        }
                    }
                    list[i] = buddy.clone();
                }
                None => {
                    buddy["created"] = json!(now());
                    list.push(buddy.clone());
                }
            }
            write(&self.path("buddies.json"), &list);
        }
        settle_home(&self.dir, &buddy);
        self.publish();
        Some(id)
    }

    fn remove_buddy(&self, id: &str) {
        self.buddies.borrow_mut().retain(|b| b["id"].as_str() != Some(id));
        write(&self.path("buddies.json"), &self.buddies.borrow());
        // (Its automations go with it; its folder and its chat stay: what it
        // wrote is the person's to keep or delete.)
        self.automations.borrow_mut().retain(|a| a["buddy"].as_str() != Some(id));
        write(&self.path("automations.json"), &self.automations.borrow());
        self.publish();
    }

    fn save_automation(&self, mut a: Json) -> Option<String> {
        if !a.is_object() || !a["buddy"].as_str().is_some_and(plain) {
            return None;
        }
        let id = a["id"].as_str().filter(|id| plain(id)).map(str::to_owned).unwrap_or_else(|| new_id("a"));
        a["id"] = json!(id);
        a.as_object_mut()?.remove("next");
        {
            let mut list = self.automations.borrow_mut();
            match list.iter().position(|x| x["id"] == a["id"]) {
                Some(i) => {
                    for key in ["last_run", "created", "runs_today", "runs_day"] {
                        if a.get(key).is_none() && !list[i][key].is_null() {
                            a[key] = list[i][key].clone();
                        }
                    }
                    list[i] = a.clone();
                }
                None => {
                    a["created"] = json!(now());
                    list.push(a.clone());
                }
            }
            // (When it's next is counted again from what it says now.)
            let at = now();
            for x in list.iter_mut().filter(|x| x["id"] == a["id"]) {
                x["next_run"] = json!(next_run(x, at));
            }
            write(&self.path("automations.json"), &list);
        }
        self.publish();
        Some(id)
    }

    fn remove_automation(&self, id: &str) {
        self.automations.borrow_mut().retain(|a| a["id"].as_str() != Some(id));
        write(&self.path("automations.json"), &self.automations.borrow());
        self.publish();
    }

    /// One run begun: listed as due for the scripts to send.
    fn begin(&self, a: &Json, at: i64) {
        self.due.borrow_mut().push(json!({
            "run": new_id("r"),
            "automation": a["id"],
            "buddy": a["buddy"],
            "title": a["title"],
            "prompt": a["prompt"],
            "at": at,
        }));
    }

    /// The clock read: what's due begins, what was never answered is written off.
    fn tick(&self) {
        let at = now();
        let today = Local::now().ordinal() as u64;
        let mut changed = false;
        {
            let mut list = self.automations.borrow_mut();
            for a in list.iter_mut() {
                if a["next_run"].is_null() {
                    // (Not counted yet: read from an older file, or just enabled.)
                    if let Some(next) = next_run(a, at) {
                        a["next_run"] = json!(next);
                        changed = true;
                    }
                    continue;
                }
                let Some(next) = a["next_run"].as_i64() else { continue };
                if next > at {
                    continue;
                }
                if a["runs_day"].as_u64() != Some(today) {
                    a["runs_day"] = json!(today);
                    a["runs_today"] = json!(0);
                }
                let cap = a["cap"].as_u64().unwrap_or(DAILY_CAP);
                let ran = a["runs_today"].as_u64().unwrap_or(0);
                if a["enabled"].as_bool() != Some(false) && ran < cap {
                    a["runs_today"] = json!(ran + 1);
                    a["last_run"] = json!(at);
                    self.begin(a, at);
                } else if a["enabled"].as_bool() != Some(false) {
                    self.record(json!({ "run": new_id("r"), "automation": a["id"], "buddy": a["buddy"], "title": a["title"], "at": at, "status": "skipped", "note": format!("It had run {cap} times today, its most.") }));
                }
                if a["kind"].as_str() == Some("once") {
                    a["enabled"] = json!(false);
                }
                a["next_run"] = json!(next_run(a, at));
                changed = true;
            }
            if changed {
                write(&self.path("automations.json"), &list);
            }
        }
        let stale: Vec<Json> = {
            let mut due = self.due.borrow_mut();
            let (old, fresh): (Vec<Json>, Vec<Json>) = due.drain(..).partition(|d| at - d["at"].as_i64().unwrap_or(at) > UNANSWERED_SECS);
            *due = fresh;
            old
        };
        for mut d in stale {
            d["status"] = json!("missed");
            d["note"] = json!("The window never sent it.");
            d.as_object_mut().map(|o| o.remove("prompt"));
            self.record(d);
            changed = true;
        }
        if changed || !self.due.borrow().is_empty() {
            self.publish();
        }
    }

    /// A run kept, newest first.
    fn record(&self, run: Json) {
        let mut runs = self.runs.borrow_mut();
        runs.insert(0, run);
        runs.truncate(RUNS_KEPT);
        write(&self.path("buddy-runs.json"), &runs);
    }

    /// The scripts' word on a run that was due.
    fn ran(&self, word: &Json) {
        let Some(id) = word["run"].as_str() else { return };
        let was = {
            let mut due = self.due.borrow_mut();
            let at = due.iter().position(|d| d["run"].as_str() == Some(id));
            at.map(|i| due.remove(i))
        };
        let Some(mut run) = was else { return };
        run.as_object_mut().map(|o| o.remove("prompt"));
        run["status"] = json!(word["status"].as_str().unwrap_or("sent"));
        for key in ["note", "chat", "harness", "model"] {
            if !word[key].is_null() {
                run[key] = word[key].clone();
            }
        }
        self.record(run);
        self.publish();
    }
}

thread_local! {
    static STATE: RefCell<Option<Rc<State>>> = const { RefCell::new(None) };
}

/// A message to a chat by its id, as a turn of its own (the engine's `run`,
/// as the composer sends one): on the agent and model given, in `cwd`.
async fn send(req: Json) -> Result<Value, String> {
    let chat = req["chat"].as_str().filter(|c| !c.is_empty()).ok_or("no chat to send to")?.to_owned();
    let prompt = req["prompt"].as_str().filter(|p| !p.trim().is_empty()).ok_or("nothing to send")?.to_owned();
    let harness = req["harness"].as_str().filter(|h| !h.is_empty()).ok_or("no agent to run on")?.to_owned();
    let request = json!({
        "prompt": prompt,
        "harness": harness,
        "model": req["model"],
        "reasoning": req["reasoning"],
        "modelOptions": {},
        "cwd": req["cwd"].as_str().filter(|c| !c.is_empty()).unwrap_or("~"),
        "sandbox": "workspace-write",
        "autoApprove": false,
        "resume": null,
    });
    let run = json!({ "kind": "run", "messageId": crate::sessions::new_id(), "request": request });
    crate::engine::get().call("QueueCommand", json!({ "chatId": chat, "command": run })).await.map(|_| Value::Nil)
}

pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    let dir = crate::kx_account::data_dir();
    let state = Rc::new(State {
        buddies: RefCell::new(read(&dir.join("buddies.json"))),
        automations: RefCell::new(read(&dir.join("automations.json"))),
        runs: RefCell::new(read(&dir.join("buddy-runs.json"))),
        due: RefCell::new(Vec::new()),
        shared: RefCell::new(None),
        dir,
    });
    // (Away while the app was closed: each is counted again from now.)
    {
        let at = now();
        for a in state.automations.borrow_mut().iter_mut() {
            a["next_run"] = json!(next_run(a, at));
        }
        for b in state.buddies.borrow().iter() {
            settle_home(&state.dir, b);
        }
    }
    *state.shared.borrow_mut() = Some(Shared {
        buddies: host.shared("kx.buddies", state.buddies_value()),
        automations: host.shared("kx.automations", state.automations_value()),
        runs: host.shared("kx.buddy.runs", crate::kx::to_value(&Json::Array(state.runs.borrow().clone()))),
        due: host.shared("kx.automations.due", crate::kx::to_value(&Json::Array(Vec::new()))),
    });
    STATE.with(|s| *s.borrow_mut() = Some(state.clone()));

    let s = state.clone();
    host.async_action("kx.buddy.save", move |buddy| {
        let id = s.save_buddy(crate::kx::widgets::value_json(&buddy));
        async move { id.map(|id| Value::from(id.as_str())).ok_or_else(|| "That isn't a buddy.".to_owned()) }
    });
    let s = state.clone();
    host.action("kx.buddy.remove", move |id| s.remove_buddy(&crate::auth::text(&id)));
    let s = state.clone();
    host.async_action("kx.automation.save", move |a| {
        let id = s.save_automation(crate::kx::widgets::value_json(&a));
        async move { id.map(|id| Value::from(id.as_str())).ok_or_else(|| "That isn't an automation.".to_owned()) }
    });
    let s = state.clone();
    host.action("kx.automation.remove", move |id| s.remove_automation(&crate::auth::text(&id)));
    let s = state.clone();
    host.action("kx.automation.run_now", move |id| {
        let id = crate::auth::text(&id);
        let found = s.automations.borrow().iter().find(|a| a["id"].as_str() == Some(id.as_str())).cloned();
        if let Some(a) = found {
            s.begin(&a, now());
            s.publish();
        }
    });
    let s = state.clone();
    host.action("kx.automation.ran", move |word| s.ran(&crate::kx::widgets::value_json(&word)));
    host.async_action("kx.buddy.send", move |req| {
        let req = crate::kx::widgets::value_json(&req);
        async move { send(req).await }
    });

    cx.tasks.spawn(async move {
        loop {
            zgx::timer::sleep(TICK).await;
            state.tick();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).single().unwrap().timestamp()
    }

    #[test]
    fn an_interval_counts_from_its_last_run_and_never_into_the_past() {
        let now = at(2026, 11, 13, 10, 0);
        assert_eq!(next_run(&json!({ "kind": "interval", "minutes": 30 }), now), Some(now + 1800));
        assert_eq!(next_run(&json!({ "kind": "interval", "minutes": 30, "last_run": now - 600 }), now), Some(now + 1200));
        // (Last ran long ago, the app closed since: soon, not hours of catching up.)
        assert_eq!(next_run(&json!({ "kind": "interval", "minutes": 30, "last_run": now - 86_400 }), now), Some(now + 1));
    }

    #[test]
    fn a_daily_one_is_today_if_its_time_is_ahead_else_the_next_day_it_runs() {
        // 2026-11-13 is a Friday (weekday 4, Monday 0).
        let ten = at(2026, 11, 13, 10, 0);
        assert_eq!(next_run(&json!({ "kind": "daily", "time": "17:30" }), ten), Some(at(2026, 11, 13, 17, 30)));
        assert_eq!(next_run(&json!({ "kind": "daily", "time": "09:00" }), ten), Some(at(2026, 11, 14, 9, 0)));
        // Weekdays only: Friday's has passed, so Monday's.
        assert_eq!(next_run(&json!({ "kind": "daily", "time": "09:00", "days": [0, 1, 2, 3, 4] }), ten), Some(at(2026, 11, 16, 9, 0)));
        assert_eq!(next_run(&json!({ "kind": "daily", "time": "nine" }), ten), None);
    }

    #[test]
    fn once_runs_once_and_off_never() {
        let now = at(2026, 11, 13, 10, 0);
        assert_eq!(next_run(&json!({ "kind": "once", "at": now + 60 }), now), Some(now + 60));
        assert_eq!(next_run(&json!({ "kind": "once", "at": now + 60, "last_run": now }), now), None);
        assert_eq!(next_run(&json!({ "kind": "interval", "minutes": 5, "enabled": false }), now), None);
    }

    #[test]
    fn a_buddys_folder_holds_its_instructions_and_its_id_is_never_a_path() {
        let dir = tempfile::tempdir().unwrap();
        let buddy = json!({ "id": "b1", "name": "Mina", "role": "Researcher", "instructions": "Look things up.", "permissions": "ask" });
        let home = settle_home(dir.path(), &buddy);
        let text = std::fs::read_to_string(home.join("AGENTS.md")).unwrap();
        assert!(text.contains("You are Mina") && text.contains("Look things up.") && text.contains("Ask before every action"));
        assert_eq!(text, std::fs::read_to_string(home.join("CLAUDE.md")).unwrap());
        assert!(plain("b1a2") && !plain("../x") && !plain(""));
    }

    #[test]
    fn what_comes_due_begins_once_is_capped_and_is_recorded_when_answered() {
        let dir = tempfile::tempdir().unwrap();
        let state = State {
            dir: dir.path().to_path_buf(),
            buddies: RefCell::new(Vec::new()),
            automations: RefCell::new(Vec::new()),
            runs: RefCell::new(Vec::new()),
            due: RefCell::new(Vec::new()),
            shared: RefCell::new(None),
        };
        let id = state.save_automation(json!({ "buddy": "b1", "title": "Check", "prompt": "Check in.", "kind": "interval", "minutes": 5, "cap": 1 })).unwrap();
        assert!(state.save_automation(json!({ "buddy": "../x", "prompt": "no" })).is_none());
        // Not due yet.
        state.tick();
        assert!(state.due.borrow().is_empty());
        // Made due, it begins once; due again the same day, its cap skips it.
        state.automations.borrow_mut()[0]["next_run"] = json!(now() - 1);
        state.tick();
        assert_eq!(state.due.borrow().len(), 1);
        state.automations.borrow_mut()[0]["next_run"] = json!(now() - 1);
        state.tick();
        assert_eq!(state.due.borrow().len(), 1);
        assert_eq!(state.runs.borrow()[0]["status"], "skipped");
        // Answered: off the due list, kept as a run without its prompt.
        let run = state.due.borrow()[0]["run"].as_str().unwrap().to_owned();
        state.ran(&json!({ "run": run, "status": "sent", "chat": "c1" }));
        assert!(state.due.borrow().is_empty());
        assert_eq!(state.runs.borrow()[0]["status"], "sent");
        assert!(state.runs.borrow()[0]["prompt"].is_null());
        assert_eq!(state.runs.borrow()[0]["automation"], json!(id));
        // Removing its buddy takes its automations with it.
        state.remove_buddy("b1");
        assert!(state.automations.borrow().is_empty());
    }
}
