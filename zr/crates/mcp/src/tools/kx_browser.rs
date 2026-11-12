//! Keplorayx's browser, driven by agents: the in-app browser beside a chat
//! (the panel's browser tab), opened, read, clicked, typed into and
//! scrolled by an agent, while the person watches. Each agent has a cursor
//! of its own on the page ("Agent 1", "Agent 2", …) and the page wears a
//! blue shimmer while any of them is at work; several agents can work in
//! one tab at once.
//!
//! A call is a file: this side writes `<data>/browser/requests/<id>.json`
//! and waits for `<data>/browser/replies/<id>.json`, which the app writes
//! once its page has done the step (apps/zgx/src/kx/browser.rs hands the
//! request to the main window's scripts, ui/kx/browser_agent.luau). So the
//! app must be open; with it closed, a call says so after a while.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use super::ToolDef;

pub(super) const OPEN: &str = "browser_open";
pub(super) const READ: &str = "browser_read";
pub(super) const CLICK: &str = "browser_click";
pub(super) const TYPE: &str = "browser_type";
pub(super) const KEY: &str = "browser_key";
pub(super) const SCROLL: &str = "browser_scroll";
pub(super) const GO: &str = "browser_go";
pub(super) const WAIT: &str = "browser_wait";
pub(super) const DONE: &str = "browser_done";
pub(super) const FILL: &str = "browser_fill";
pub(super) const FIND: &str = "browser_find";
pub(super) const TABS: &str = "browser_tabs";
pub(super) const NAMES: [&str; 12] = [OPEN, READ, FIND, CLICK, TYPE, FILL, KEY, SCROLL, GO, WAIT, TABS, DONE];

/// How long the app has to answer a step, the page's own loading included.
const ANSWER: Duration = Duration::from_secs(45);
const POLL: Duration = Duration::from_millis(80);

/// What every step takes: which cursor is the caller's, and whose tab.
fn with_common(mut properties: Value, required: &[&str]) -> Value {
    let p = properties.as_object_mut().expect("an object of properties");
    p.insert(
        "agent".into(),
        json!({ "type": "integer", "minimum": 1, "maximum": 8, "description": "Which cursor is yours on the page, when several agents share one tab: 1 (the default), 2, 3… Each number is a cursor of its own, labelled \"Agent N\". Keep the same number for all your steps." }),
    );
    p.insert(
        "chat".into(),
        json!({ "type": "string", "description": "Another chat's id, to work in that chat's browser tab beside its agent (parallel browsing in one tab). Omitted: your own chat's tab, or, for a subagent or a side chat, the tab of the chat you're under, so you and it share one page." }),
    );
    json!({ "type": "object", "properties": properties, "required": required })
}

pub(super) fn catalog() -> Vec<ToolDef> {
    let s = |what: &str| json!({ "type": "string", "description": what });
    let n = |what: &str| json!({ "type": "number", "description": what });
    let b = |what: &str| json!({ "type": "boolean", "description": what });
    vec![
        ToolDef {
            name: OPEN,
            description: "Open a web address in Keplorayx's in-app browser, in the tab beside your chat (made if there is none), and wait for the page. The person sees it: the page wears a blue shimmer while you work and your cursor moves on it. Words that aren't an address are searched. Returns the page's address and title; call browser_read for what's on it. The tab is the one beside your chat in Keplorayx's window, so that chat must be the one open there.",
            input_schema: with_common(json!({ "url": s("An address (https://…), or words to search."), "new_tab": b("Open another browser tab instead of using the one you have.") }), &["url"]),
        },
        ToolDef {
            name: READ,
            description: "Read the page in your browser tab: its address, title, how far it's scrolled, its headings, its visible text, and its links, buttons and fields, each with a `ref` number to click or type into (`focused` is the ref that has the keyboard). Refs are valid until the page changes; read again after a click that moved the page.",
            input_schema: with_common(json!({ "all": b("The whole page's text and controls, not just what's in view (longer).") }), &[]),
        },
        ToolDef {
            name: CLICK,
            description: "Click on the page: a control by its `ref` (from browser_read), or the first control whose words contain `text`, or a point (`x`, `y` in the page's view, CSS pixels). Your cursor moves there first, then clicks. Says what was clicked and where the page is after.",
            input_schema: with_common(json!({ "ref": n("A control's ref from browser_read."), "text": s("Words on the control to click."), "x": n("A point's x in the view."), "y": n("A point's y in the view."), "hover": b("Rest the pointer there without clicking (a menu or a tip that shows under it)."), "double": b("Click twice (what a double-click opens or edits).") }), &[]),
        },
        ToolDef {
            name: FIND,
            description: "Find words on the page, wherever they are (not only in what's in view): up to 12 places, each with the words around it, whether it's in view, and, where it sits in a link, button or field, that control's `ref` to click. The page scrolls to the first. Use it instead of reading the whole page when you know what you're looking for.",
            input_schema: with_common(json!({ "text": s("The words to find (any case).") }), &["text"]),
        },
        ToolDef {
            name: TYPE,
            description: "Type into a field: by `ref`, else the field that has the keyboard. Replaces what's there unless `append`. `submit` presses Enter after (a search box, a form).",
            input_schema: with_common(json!({ "ref": n("The field's ref from browser_read."), "text": s("What to type."), "append": b("Add to what's there instead of replacing it."), "submit": b("Press Enter after typing.") }), &["text"]),
        },
        ToolDef {
            name: FILL,
            description: "Fill several fields in one step, in the order given: each `{ ref, text }` types into a field or picks from a list as browser_type does, and `{ ref, check }` ticks (true) or clears (false) a tick box, or picks a radio choice. `submit` presses Enter in the last one after. Use it for a form instead of one browser_type a field. If one fails, the answer says which, and which were filled before it.",
            input_schema: with_common(json!({
                "fields": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 40,
                    "items": {
                        "type": "object",
                        "properties": {
                            "ref": { "type": "number", "description": "The field's ref from browser_read." },
                            "text": { "type": "string", "description": "What to type, or the choice to pick from a list." },
                            "check": { "type": "boolean", "description": "For a tick box or a radio choice: on (true) or off (false)." },
                            "append": { "type": "boolean", "description": "Add to what's there instead of replacing it." }
                        },
                        "required": ["ref"]
                    },
                    "description": "The fields, in order."
                },
                "submit": b("Press Enter in the last field after filling.")
            }), &["fields"]),
        },
        ToolDef {
            name: KEY,
            description: "Press a key on the page, on whatever has the keyboard: Enter, Escape, Tab, Backspace, ArrowDown, ArrowUp, ArrowLeft, ArrowRight, PageDown, PageUp, Home, End, or a single character.",
            input_schema: with_common(json!({ "key": s("The key's name.") }), &["key"]),
        },
        ToolDef {
            name: SCROLL,
            description: "Scroll the page: `to` top or bottom, or to a control by `ref`, or by `dy` pixels (positive is down; a view's height when nothing is given).",
            input_schema: with_common(json!({ "dy": n("Pixels to scroll by; positive is down."), "to": s("top or bottom."), "ref": n("A control's ref to bring into view.") }), &[]),
        },
        ToolDef {
            name: GO,
            description: "Move your browser tab through its history, or load the page again: `to` back, forward or reload.",
            input_schema: with_common(json!({ "to": s("back, forward or reload.") }), &["to"]),
        },
        ToolDef {
            name: WAIT,
            description: "Wait for the page: for `text` to appear on it, or for `ms` milliseconds (at most 10000), whichever is asked. Use it after a click that loads something slowly.",
            input_schema: with_common(json!({ "ms": n("How long to wait, in milliseconds."), "text": s("Words to wait for on the page.") }), &[]),
        },
        ToolDef {
            name: TABS,
            description: "The browser tabs beside your chat: `action` list (each tab's id, its address and whether it's the one in front, which is the one every other step works in), new (open another, at `url` if given), select (bring `tab` to the front) or close (`tab`). Every answer lists the tabs as they are after.",
            input_schema: with_common(json!({ "action": s("list (the default), new, select or close."), "tab": s("A tab's id from the list, for select and close."), "url": s("For new: an address to open it at.") }), &[]),
        },
        ToolDef {
            name: DONE,
            description: "Say you're finished with the browser: your cursor leaves the page, and its shimmer goes once no agent is on it. Call it when your browsing is done.",
            input_schema: with_common(json!({}), &[]),
        },
    ]
}

fn dir() -> PathBuf {
    super::kx::data_dir().join("browser")
}

/// One step, asked of the app and awaited. `who`: the calling chat, if it
/// is one (its cursor is its own). `home`: the chat whose tab it works in
/// unless it names another: its own, or the chat it's under.
pub(super) async fn call(name: &str, args: &Value, who: Option<&str>, home: Option<&str>) -> Result<Value, String> {
    let op = name.strip_prefix("browser_").unwrap_or(name);
    let (requests, replies) = (dir().join("requests"), dir().join("replies"));
    for d in [&requests, &replies] {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let request = json!({
        "id": id,
        "op": op,
        // (Whose tab: another chat's when asked, else the caller's.)
        "chat": args["chat"].as_str().filter(|c| !c.is_empty()).or(home),
        // (Whose cursor: the caller's chat and its number.)
        "who": who,
        "agent": args["agent"].as_u64().unwrap_or(1).clamp(1, 8),
        "args": args,
        "at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
    });
    // (Written whole, then named: the app never reads half a request.)
    let (half, whole) = (requests.join(format!("{id}.tmp")), requests.join(format!("{id}.json")));
    std::fs::write(&half, request.to_string()).map_err(|e| e.to_string())?;
    std::fs::rename(&half, &whole).map_err(|e| e.to_string())?;
    let reply = replies.join(format!("{id}.json"));
    let started = std::time::Instant::now();
    loop {
        // (Half written, it doesn't parse yet: read again next time round.)
        let answer = std::fs::read_to_string(&reply).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok());
        if let Some(answer) = answer {
            let _ = std::fs::remove_file(&reply);
            let _ = std::fs::remove_file(&whole);
            return answered(answer);
        }
        if started.elapsed() > ANSWER {
            let _ = std::fs::remove_file(&whole);
            return Err("Keplorayx's browser didn't answer. Its window must be open (the app running, signed in or not), and the page may still be loading: try browser_read in a moment.".into());
        }
        tokio::time::sleep(POLL).await;
    }
}

/// The app's answer: `{ ok, result }` or `{ ok: false, error }`.
fn answered(answer: Value) -> Result<Value, String> {
    if answer["ok"].as_bool() == Some(true) {
        return Ok(answer.get("result").cloned().unwrap_or(json!({})));
    }
    Err(answer["error"].as_str().unwrap_or("the browser couldn't do that").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_takes_an_agent_and_a_chat() {
        let tools = catalog();
        assert_eq!(tools.len(), NAMES.len());
        for tool in tools {
            assert!(NAMES.contains(&tool.name));
            assert!(tool.input_schema["properties"]["agent"].is_object(), "{}", tool.name);
            assert!(tool.input_schema["properties"]["chat"].is_object(), "{}", tool.name);
        }
    }

    #[test]
    fn answers_read_as_results_or_errors() {
        assert_eq!(answered(json!({ "ok": true, "result": { "url": "https://a.test/" } })).unwrap()["url"], "https://a.test/");
        assert_eq!(answered(json!({ "ok": false, "error": "no such ref" })).unwrap_err(), "no such ref");
        assert!(answered(json!({})).is_err());
    }
}
