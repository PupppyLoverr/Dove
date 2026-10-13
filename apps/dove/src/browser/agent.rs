//! The in-app browser as a tool any agent can drive like a person.
//!
//! Every harness Dove starts gets a `dove` MCP server (see `mcp_browser.rs`)
//! whose calls land here: snapshot the page into an outline with stable
//! refs, then move a visible cursor and click, type, scroll, or press keys.
//! On Windows input goes through the DevTools protocol, so pages receive
//! trusted events exactly as from a mouse and keyboard; elsewhere it falls
//! back to page scripts. Screenshots come back as images the model can see.

use std::time::Duration;

use gpui::{AsyncWindowContext, WeakEntity};
use serde_json::{Value, json};

use super::{BrowserSurface, EvalDone};

const PAGE_API: &str = include_str!("page_api.js");

/// What a tool call hands back to the agent.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub text: String,
    /// Base64 PNG.
    pub image: Option<String>,
    pub is_error: bool,
}

impl Outcome {
    pub fn ok(text: impl Into<String>) -> Self {
        Self { text: text.into(), image: None, is_error: false }
    }

    pub fn err(text: impl Into<String>) -> Self {
        Self { text: text.into(), image: None, is_error: true }
    }
}

/// MCP tool definitions (`tools/list`).
pub fn tools() -> Value {
    let target = json!({
        "type": "string",
        "description": "Element to act on: a ref from browser_snapshot (e.g. \"e12\"), a CSS selector, or its visible text."
    });
    json!([
        {
            "name": "browser_navigate",
            "description": "Open a URL in Dove's in-app browser (the user watches it in the side panel). Returns a snapshot of the loaded page.",
            "inputSchema": {"type": "object", "properties": {"url": {"type": "string", "description": "URL or host, e.g. localhost:3000"}}, "required": ["url"]}
        },
        {
            "name": "browser_snapshot",
            "description": "Outline of the visible page: headings, text, links, buttons and inputs, each actionable element tagged [ref=eN]. Use refs with the other browser tools.",
            "inputSchema": {"type": "object", "properties": {
                "ref": {"type": "string", "description": "Only snapshot inside this element."},
                "full": {"type": "boolean", "description": "Include longer text runs."}
            }}
        },
        {
            "name": "browser_click",
            "description": "Move the cursor to an element and click it like a person (trusted mouse events). Returns the page snapshot afterwards.",
            "inputSchema": {"type": "object", "properties": {
                "target": target,
                "button": {"type": "string", "enum": ["left", "right", "middle"]},
                "double": {"type": "boolean"}
            }, "required": ["target"]}
        },
        {
            "name": "browser_hover",
            "description": "Move the cursor over an element (menus, tooltips).",
            "inputSchema": {"type": "object", "properties": {"target": target}, "required": ["target"]}
        },
        {
            "name": "browser_type",
            "description": "Click into a field and type text with the keyboard. `clear` replaces what is there; `submit` presses Enter afterwards.",
            "inputSchema": {"type": "object", "properties": {
                "target": target,
                "text": {"type": "string"},
                "clear": {"type": "boolean"},
                "submit": {"type": "boolean"}
            }, "required": ["text"]}
        },
        {
            "name": "browser_press",
            "description": "Press a key or chord on the focused element, e.g. Enter, Escape, Tab, ArrowDown, Control+A, Meta+K.",
            "inputSchema": {"type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]}
        },
        {
            "name": "browser_scroll",
            "description": "Scroll the page (or the element under `target`) with the mouse wheel. Positive dy scrolls down.",
            "inputSchema": {"type": "object", "properties": {
                "dy": {"type": "number"}, "dx": {"type": "number"}, "target": target
            }}
        },
        {
            "name": "browser_select",
            "description": "Choose option(s) in a <select> by value or label.",
            "inputSchema": {"type": "object", "properties": {
                "target": target, "values": {"type": "array", "items": {"type": "string"}}
            }, "required": ["target", "values"]}
        },
        {
            "name": "browser_drag",
            "description": "Press on one element, drag to another, release.",
            "inputSchema": {"type": "object", "properties": {"from": target, "to": target}, "required": ["from", "to"]}
        },
        {
            "name": "browser_screenshot",
            "description": "PNG screenshot of the visible page (or the whole page with full_page).",
            "inputSchema": {"type": "object", "properties": {"full_page": {"type": "boolean"}}}
        },
        {
            "name": "browser_read",
            "description": "The page's readable text (innerText), for reading articles, docs and results.",
            "inputSchema": {"type": "object", "properties": {"max_chars": {"type": "number"}}}
        },
        {
            "name": "browser_eval",
            "description": "Run JavaScript in the page (an async function body; `return` a JSON-serialisable value).",
            "inputSchema": {"type": "object", "properties": {"script": {"type": "string"}}, "required": ["script"]}
        },
        {
            "name": "browser_wait",
            "description": "Wait until text (or an element) appears, or disappears with gone=true. Times out after `seconds` (default 10).",
            "inputSchema": {"type": "object", "properties": {
                "text": {"type": "string"}, "target": target, "gone": {"type": "boolean"}, "seconds": {"type": "number"}
            }}
        },
        {
            "name": "browser_console",
            "description": "Console messages and page errors logged since the last call.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "browser_back",
            "description": "Go back in history.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "browser_forward",
            "description": "Go forward in history.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "browser_reload",
            "description": "Reload the page.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "browser_tabs",
            "description": "List, open, switch to, or close the thread's browser tabs.",
            "inputSchema": {"type": "object", "properties": {
                "action": {"type": "string", "enum": ["list", "new", "select", "close"]},
                "index": {"type": "number", "description": "0-based tab index for select/close"},
                "url": {"type": "string", "description": "For new"}
            }, "required": ["action"]}
        }
    ])
}

/// A short present-tense label for the browser's "agent is driving" pill.
pub fn activity(tool: &str, args: &Value) -> String {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().chars().take(40).collect::<String>();
    match tool {
        "browser_navigate" => format!("Opening {}", s("url")),
        "browser_click" => format!("Clicking {}", s("target")),
        "browser_hover" => format!("Hovering {}", s("target")),
        "browser_type" => "Typing".into(),
        "browser_press" => format!("Pressing {}", s("key")),
        "browser_scroll" => "Scrolling".into(),
        "browser_select" => "Choosing an option".into(),
        "browser_drag" => "Dragging".into(),
        "browser_screenshot" => "Taking a screenshot".into(),
        "browser_snapshot" | "browser_read" => "Reading the page".into(),
        "browser_eval" => "Running a script".into(),
        "browser_wait" => "Waiting".into(),
        "browser_back" => "Going back".into(),
        "browser_forward" => "Going forward".into(),
        "browser_reload" => "Reloading".into(),
        _ => "Working".into(),
    }
}

type Browser = WeakEntity<BrowserSurface>;

async fn answer(
    b: &Browser,
    cx: &mut AsyncWindowContext,
    secs: u64,
    start: impl FnOnce(&BrowserSurface, EvalDone),
) -> Result<Value, String> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Value, String>>(2);
    let timeout_tx = tx.clone();
    b.update(cx, |b, _| {
        start(
            b,
            Box::new(move |r| {
                let _ = tx.try_send(r);
            }),
        )
    })
    .map_err(|_| "The browser tab was closed".to_string())?;
    let timer = cx.background_executor().timer(Duration::from_secs(secs));
    cx.background_executor()
        .spawn(async move {
            timer.await;
            let _ = timeout_tx.try_send(Err(format!("timed out after {secs}s")));
        })
        .detach();
    rx.recv().await.unwrap_or_else(|| Err("The page did not answer".into()))
}

/// Evaluate `expr` (with the page helpers loaded) and return its value.
async fn js(b: &Browser, cx: &mut AsyncWindowContext, expr: &str) -> Result<Value, String> {
    let script = format!(
        "{PAGE_API}\n;(() => {{ try {{ return JSON.stringify({{ok: ({expr})}}); }} catch (e) {{ return JSON.stringify({{err: String(e && e.message || e)}}); }} }})()"
    );
    let raw = answer(b, cx, 20, move |b, done| b.eval_value(&script, done)).await?;
    let Value::String(text) = raw else {
        return Err("The page is still loading; try again".into());
    };
    let parsed: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if let Some(err) = parsed.get("err").and_then(Value::as_str) {
        return Err(err.to_string());
    }
    Ok(parsed.get("ok").cloned().unwrap_or(Value::Null))
}

/// One DevTools call. `Err("unsupported")` off Windows.
async fn cdp(b: &Browser, cx: &mut AsyncWindowContext, method: &'static str, params: Value) -> Result<Value, String> {
    answer(b, cx, 20, move |b, done| b.cdp(method, &params, done)).await
}

fn unsupported(e: &str) -> bool {
    e == "unsupported"
}

fn lit(s: &str) -> String {
    Value::String(s.to_string()).to_string()
}

async fn sleep(cx: &mut AsyncWindowContext, ms: u64) {
    cx.background_executor().timer(Duration::from_millis(ms)).await;
}

/// Wait for the document to finish loading (or give up quietly).
async fn settle(b: &Browser, cx: &mut AsyncWindowContext, max_ms: u64) {
    sleep(cx, 250).await;
    let mut waited = 250;
    while waited < max_ms {
        let loading = b.update(cx, |b, _| b.page.loading).unwrap_or(false);
        if !loading {
            if let Ok(Value::String(state)) = js(b, cx, "document.readyState").await {
                if state == "complete" || (state == "interactive" && waited > 3000) {
                    return;
                }
            }
        }
        sleep(cx, 200).await;
        waited += 200;
    }
}

async fn snapshot(b: &Browser, cx: &mut AsyncWindowContext, args: &Value) -> Result<String, String> {
    let opts = json!({
        "ref": args.get("ref").and_then(Value::as_str),
        "full": args.get("full").and_then(Value::as_bool).unwrap_or(false),
    });
    match js(b, cx, &format!("__dove.snapshot({opts})")).await? {
        Value::String(s) => Ok(s),
        other => Ok(other.to_string()),
    }
}

async fn with_snapshot(b: &Browser, cx: &mut AsyncWindowContext, head: String) -> Outcome {
    match snapshot(b, cx, &Value::Null).await {
        Ok(s) => Outcome::ok(format!("{head}\n\n{s}")),
        Err(e) => Outcome::ok(format!("{head}\n\n(snapshot unavailable: {e})")),
    }
}

struct Point {
    x: f64,
    y: f64,
    label: String,
    covered: Option<String>,
}

async fn point(b: &Browser, cx: &mut AsyncWindowContext, target: &str) -> Result<Point, String> {
    let v = js(b, cx, &format!("__dove.point({})", lit(target))).await?;
    Ok(Point {
        x: v.get("x").and_then(Value::as_f64).unwrap_or(0.0),
        y: v.get("y").and_then(Value::as_f64).unwrap_or(0.0),
        label: v.get("label").and_then(Value::as_str).unwrap_or(target).to_string(),
        covered: v.get("covered").and_then(Value::as_str).map(str::to_string),
    })
}

async fn show_cursor(b: &Browser, cx: &mut AsyncWindowContext, p: &Point, kind: &str) {
    let _ = js(b, cx, &format!("__dove.cursor({}, {}, {})", p.x, p.y, lit(kind))).await;
    // Let the cursor glide there before the click lands.
    sleep(cx, 260).await;
}

async fn mouse(
    b: &Browser,
    cx: &mut AsyncWindowContext,
    kind: &str,
    x: f64,
    y: f64,
    button: &str,
    clicks: u32,
) -> Result<(), String> {
    cdp(
        b,
        cx,
        "Input.dispatchMouseEvent",
        json!({"type": kind, "x": x, "y": y, "button": button, "clickCount": clicks, "buttons": if kind == "mousePressed" { 1 } else { 0 }}),
    )
    .await
    .map(|_| ())
}

/// `(key, code, windows virtual key, text)` for a key name.
fn key_info(name: &str) -> Option<(String, String, u32, Option<String>)> {
    let named = |key: &str, vk: u32, text: Option<&str>| Some((key.to_string(), key.to_string(), vk, text.map(str::to_string)));
    match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => named("Enter", 13, Some("\r")),
        "tab" => named("Tab", 9, None),
        "escape" | "esc" => named("Escape", 27, None),
        "backspace" => named("Backspace", 8, None),
        "delete" | "del" => named("Delete", 46, None),
        "arrowup" | "up" => named("ArrowUp", 38, None),
        "arrowdown" | "down" => named("ArrowDown", 40, None),
        "arrowleft" | "left" => named("ArrowLeft", 37, None),
        "arrowright" | "right" => named("ArrowRight", 39, None),
        "home" => named("Home", 36, None),
        "end" => named("End", 35, None),
        "pageup" => named("PageUp", 33, None),
        "pagedown" => named("PageDown", 34, None),
        "space" | " " => Some((" ".into(), "Space".into(), 32, Some(" ".into()))),
        n if n.len() >= 2 && n.starts_with('f') && n[1..].parse::<u32>().is_ok_and(|f| (1..=12).contains(&f)) => {
            let f: u32 = n[1..].parse().ok()?;
            Some((format!("F{f}"), format!("F{f}"), 111 + f, None))
        }
        _ => {
            let mut chars = name.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            let upper = c.to_ascii_uppercase();
            let code = if c.is_ascii_alphabetic() {
                format!("Key{upper}")
            } else if c.is_ascii_digit() {
                format!("Digit{c}")
            } else {
                String::new()
            };
            let vk = if c.is_ascii_alphanumeric() { upper as u32 } else { 0 };
            Some((c.to_string(), code, vk, Some(c.to_string())))
        }
    }
}

async fn press(b: &Browser, cx: &mut AsyncWindowContext, chord: &str) -> Result<(), String> {
    let parts: Vec<&str> = chord.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let Some((key, mods)) = parts.split_last() else { return Err("Which key?".into()) };
    let mut modifiers = 0;
    for m in mods {
        modifiers |= match m.to_ascii_lowercase().as_str() {
            "alt" | "option" => 1,
            "ctrl" | "control" => 2,
            "meta" | "cmd" | "command" | "super" | "win" => 4,
            "shift" => 8,
            other => return Err(format!("Unknown modifier {other}")),
        };
    }
    let (k, code, vk, text) = key_info(key).ok_or_else(|| format!("Unknown key {key}"))?;
    let shortcut = modifiers & (1 | 2 | 4) != 0;
    let command = (modifiers & (2 | 4) != 0)
        .then(|| match k.to_ascii_lowercase().as_str() {
            "a" => Some("selectAll"),
            "c" => Some("copy"),
            "v" => Some("paste"),
            "x" => Some("cut"),
            "z" => Some("undo"),
            "y" => Some("redo"),
            _ => None,
        })
        .flatten();
    let mut down = json!({
        "type": if text.is_some() && !shortcut { "keyDown" } else { "rawKeyDown" },
        "key": k, "code": code, "windowsVirtualKeyCode": vk, "modifiers": modifiers,
    });
    if let Some(t) = text.as_ref().filter(|_| !shortcut) {
        down["text"] = json!(t);
        down["unmodifiedText"] = json!(t);
    }
    if let Some(c) = command {
        down["commands"] = json!([c]);
    }
    match cdp(b, cx, "Input.dispatchKeyEvent", down).await {
        Ok(_) => {
            cdp(
                b,
                cx,
                "Input.dispatchKeyEvent",
                json!({"type": "keyUp", "key": k, "code": code, "windowsVirtualKeyCode": vk, "modifiers": modifiers}),
            )
            .await?;
            Ok(())
        }
        Err(e) if unsupported(&e) => js(b, cx, &format!("__dove.press({})", lit(&k))).await.map(|_| ()),
        Err(e) => Err(e),
    }
}

/// Run one browser tool (everything but `browser_tabs`, which the shell
/// owns) against `b`.
pub async fn run(b: Browser, tool: &str, args: Value, cx: &mut AsyncWindowContext) -> Outcome {
    match run_inner(&b, tool, &args, cx).await {
        Ok(o) => o,
        Err(e) => Outcome::err(e),
    }
}

async fn run_inner(b: &Browser, tool: &str, args: &Value, cx: &mut AsyncWindowContext) -> Result<Outcome, String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let flag = |k: &str| args.get(k).and_then(Value::as_bool).unwrap_or(false);
    let has_page = b.update(cx, |b, _| b.has_page()).map_err(|_| "The browser tab was closed".to_string())?;
    if !has_page && tool != "browser_navigate" {
        return Err("Nothing is open yet. Call browser_navigate first.".into());
    }
    Ok(match tool {
        "browser_navigate" => {
            let url = s("url");
            if url.trim().is_empty() {
                return Err("Give a url".into());
            }
            b.update_in(cx, |b, window, cx| {
                b.navigate_quiet(&url, window, cx);
                b.set_headless(true);
            })
            .map_err(|_| "The browser tab was closed")?;
            let problem = b
                .update(cx, |b, _| b.validation.clone().or_else(|| b.page.error.clone()))
                .map_err(|_| "The browser tab was closed")?;
            if let Some(p) = problem {
                return Err(p);
            }
            settle(b, cx, 20_000).await;
            let error = b.update(cx, |b, _| b.page.error.clone()).ok().flatten();
            if let Some(e) = error {
                return Err(e);
            }
            with_snapshot(b, cx, format!("Opened {url}")).await
        }
        "browser_snapshot" => Outcome::ok(snapshot(b, cx, args).await?),
        "browser_click" => {
            let target = s("target");
            let p = point(b, cx, &target).await?;
            show_cursor(b, cx, &p, "click").await;
            let button = match s("button").as_str() {
                "right" => "right",
                "middle" => "middle",
                _ => "left",
            };
            let clicks = if flag("double") { 2 } else { 1 };
            let trusted = async {
                mouse(b, cx, "mouseMoved", p.x, p.y, "none", 0).await?;
                for n in 1..=clicks {
                    mouse(b, cx, "mousePressed", p.x, p.y, button, n).await?;
                    mouse(b, cx, "mouseReleased", p.x, p.y, button, n).await?;
                }
                Ok::<(), String>(())
            }
            .await;
            match trusted {
                Ok(()) => {}
                Err(e) if unsupported(&e) => {
                    js(b, cx, &format!("__dove.synthClick({}, {{double: {}}})", lit(&target), clicks == 2)).await?;
                }
                Err(e) => return Err(e),
            }
            settle(b, cx, 4000).await;
            let mut head = format!("Clicked {}", p.label);
            if let Some(c) = p.covered {
                head.push_str(&format!("\nNote: another element was on top at that point: {c}"));
            }
            with_snapshot(b, cx, head).await
        }
        "browser_hover" => {
            let p = point(b, cx, &s("target")).await?;
            show_cursor(b, cx, &p, "move").await;
            match mouse(b, cx, "mouseMoved", p.x, p.y, "none", 0).await {
                Err(e) if unsupported(&e) => {
                    js(
                        b,
                        cx,
                        &format!(
                            "(() => {{ const el = __dove.resolve({}); for (const t of ['pointerover','mouseover','pointerenter','mouseenter','mousemove']) el.dispatchEvent(new MouseEvent(t, {{bubbles: true}})); return true; }})()",
                            lit(&s("target"))
                        ),
                    )
                    .await?;
                }
                Err(e) => return Err(e),
                Ok(()) => {}
            }
            sleep(cx, 300).await;
            with_snapshot(b, cx, format!("Hovering {}", p.label)).await
        }
        "browser_type" => {
            let target = s("target");
            let text = s("text");
            let clear = flag("clear");
            let mut label = "the focused element".to_string();
            if !target.is_empty() {
                let p = point(b, cx, &target).await?;
                show_cursor(b, cx, &p, "click").await;
                label = p.label.clone();
                let _ = async {
                    mouse(b, cx, "mousePressed", p.x, p.y, "left", 1).await?;
                    mouse(b, cx, "mouseReleased", p.x, p.y, "left", 1).await
                }
                .await;
                js(b, cx, &format!("__dove.focusFor({}, {clear})", lit(&target))).await?;
            }
            match cdp(b, cx, "Input.insertText", json!({"text": text})).await {
                Ok(_) => {}
                Err(e) if unsupported(&e) => {
                    let t = if target.is_empty() { "null".to_string() } else { lit(&target) };
                    js(b, cx, &format!("__dove.setValue({t}, {}, {clear})", lit(&text))).await?;
                }
                Err(e) => return Err(e),
            }
            if flag("submit") {
                press(b, cx, "Enter").await?;
                settle(b, cx, 6000).await;
                with_snapshot(b, cx, format!("Typed into {label} and pressed Enter")).await
            } else {
                Outcome::ok(format!("Typed into {label}"))
            }
        }
        "browser_press" => {
            let key = s("key");
            press(b, cx, &key).await?;
            settle(b, cx, 2500).await;
            with_snapshot(b, cx, format!("Pressed {key}")).await
        }
        "browser_scroll" => {
            let dy = args.get("dy").and_then(Value::as_f64).unwrap_or(600.0);
            let dx = args.get("dx").and_then(Value::as_f64).unwrap_or(0.0);
            let target = s("target");
            let (x, y) = if target.is_empty() {
                let v = js(b, cx, "[innerWidth / 2, innerHeight / 2]").await?;
                (v[0].as_f64().unwrap_or(200.0), v[1].as_f64().unwrap_or(200.0))
            } else {
                let p = point(b, cx, &target).await?;
                (p.x, p.y)
            };
            match cdp(b, cx, "Input.dispatchMouseEvent", json!({"type": "mouseWheel", "x": x, "y": y, "deltaX": dx, "deltaY": dy}))
                .await
            {
                Ok(_) => {}
                Err(e) if unsupported(&e) => {
                    let t = if target.is_empty() { "null".to_string() } else { lit(&target) };
                    js(
                        b,
                        cx,
                        &format!("(() => {{ const t = {t}; const n = t ? __dove.resolve(t) : null; (n || window).scrollBy({dx}, {dy}); return true; }})()"),
                    )
                    .await?;
                }
                Err(e) => return Err(e),
            }
            sleep(cx, 350).await;
            with_snapshot(b, cx, format!("Scrolled by {dy}px")).await
        }
        "browser_select" => {
            let values = args.get("values").cloned().unwrap_or(json!([]));
            let v = js(b, cx, &format!("__dove.select({}, {values})", lit(&s("target")))).await?;
            Outcome::ok(format!("Selected in {}", v.as_str().unwrap_or("the list")))
        }
        "browser_drag" => {
            let from = point(b, cx, &s("from")).await?;
            show_cursor(b, cx, &from, "move").await;
            let to = js(
                b,
                cx,
                &format!(
                    "(() => {{ const r = __dove.resolve({}).getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; }})()",
                    lit(&s("to"))
                ),
            )
            .await?;
            let (tx, ty) = (to[0].as_f64().unwrap_or(from.x), to[1].as_f64().unwrap_or(from.y));
            mouse(b, cx, "mouseMoved", from.x, from.y, "none", 0).await?;
            mouse(b, cx, "mousePressed", from.x, from.y, "left", 1).await?;
            for step in 1..=10 {
                let f = step as f64 / 10.0;
                let (x, y) = (from.x + (tx - from.x) * f, from.y + (ty - from.y) * f);
                let _ = js(b, cx, &format!("__dove.cursor({x}, {y}, 'move')")).await;
                cdp(
                    b,
                    cx,
                    "Input.dispatchMouseEvent",
                    json!({"type": "mouseMoved", "x": x, "y": y, "button": "left", "buttons": 1}),
                )
                .await?;
            }
            mouse(b, cx, "mouseReleased", tx, ty, "left", 1).await?;
            sleep(cx, 300).await;
            with_snapshot(b, cx, format!("Dragged {} onto {}", from.label, s("to"))).await
        }
        "browser_screenshot" => {
            let mut params = json!({"format": "png"});
            if flag("full_page") {
                let metrics = cdp(b, cx, "Page.getLayoutMetrics", json!({})).await?;
                let size = metrics.get("cssContentSize").or_else(|| metrics.get("contentSize")).cloned().unwrap_or_default();
                let w = size.get("width").and_then(Value::as_f64).unwrap_or(1280.0);
                let h = size.get("height").and_then(Value::as_f64).unwrap_or(800.0).min(16_000.0);
                params = json!({"format": "png", "captureBeyondViewport": true, "clip": {"x": 0, "y": 0, "width": w, "height": h, "scale": 1}});
            }
            match cdp(b, cx, "Page.captureScreenshot", params).await {
                Ok(v) => {
                    let data = v.get("data").and_then(Value::as_str).unwrap_or_default().to_string();
                    if data.is_empty() {
                        return Err("The page returned no image (is the browser tab visible?)".into());
                    }
                    let state = js(b, cx, "__dove.state()").await.unwrap_or_default();
                    Outcome {
                        text: format!(
                            "Screenshot of {} — {}",
                            state.get("title").and_then(Value::as_str).unwrap_or(""),
                            state.get("url").and_then(Value::as_str).unwrap_or("")
                        ),
                        image: Some(data),
                        is_error: false,
                    }
                }
                Err(e) if unsupported(&e) => {
                    return Err("Screenshots need the Windows build for now; use browser_snapshot instead.".into());
                }
                Err(e) => return Err(e),
            }
        }
        "browser_read" => {
            let max = args.get("max_chars").and_then(Value::as_u64).unwrap_or(20_000).clamp(500, 100_000);
            match js(b, cx, &format!("__dove.text({max})")).await? {
                Value::String(t) => Outcome::ok(t),
                other => Outcome::ok(other.to_string()),
            }
        }
        "browser_eval" => {
            let id = format!("r{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
            let body = s("script");
            let start = format!(
                "(() => {{ window.__doveOut = window.__doveOut || {{}}; const id = {id}; (async () => {{ {body}\n }})().then(v => window.__doveOut[id] = {{ok: v === undefined ? null : v}}, e => window.__doveOut[id] = {{err: String(e && e.stack || e)}}); return true; }})()",
                id = lit(&id)
            );
            js(b, cx, &start).await?;
            let poll = format!(
                "(() => {{ const o = window.__doveOut && window.__doveOut[{id}]; if (!o) return null; delete window.__doveOut[{id}]; return o; }})()",
                id = lit(&id)
            );
            let mut waited = 0;
            loop {
                let v = js(b, cx, &poll).await?;
                if let Some(err) = v.get("err").and_then(Value::as_str) {
                    return Err(err.to_string());
                }
                if let Some(ok) = v.get("ok") {
                    let text = match ok {
                        Value::String(s) => s.clone(),
                        other => serde_json::to_string_pretty(other).unwrap_or_default(),
                    };
                    break Outcome::ok(if text.len() > 60_000 { format!("{}… (truncated)", text.chars().take(60_000).collect::<String>()) } else { text });
                }
                if waited > 30_000 {
                    return Err("The script did not finish within 30s".into());
                }
                sleep(cx, 60).await;
                waited += 60;
            }
        }
        "browser_wait" => {
            let secs = args.get("seconds").and_then(Value::as_f64).unwrap_or(10.0).clamp(0.2, 60.0);
            let gone = flag("gone");
            let text = s("text");
            let target = s("target");
            let probe = if !text.is_empty() {
                format!("__dove.find({})", lit(&text))
            } else if !target.is_empty() {
                format!("(() => {{ try {{ return !!__dove.resolve({}); }} catch (e) {{ return false; }} }})()", lit(&target))
            } else {
                sleep(cx, (secs * 1000.0) as u64).await;
                return Ok(Outcome::ok(format!("Waited {secs}s")));
            };
            let deadline = (secs * 1000.0) as u64;
            let mut waited = 0;
            loop {
                let present = js(b, cx, &probe).await.ok().and_then(|v| v.as_bool()).unwrap_or(false);
                if present != gone {
                    break with_snapshot(b, cx, format!("Done waiting after {:.1}s", waited as f64 / 1000.0)).await;
                }
                if waited >= deadline {
                    return Err(format!("Still {} after {secs}s", if gone { "there" } else { "missing" }));
                }
                sleep(cx, 200).await;
                waited += 200;
            }
        }
        "browser_console" => {
            let v = js(b, cx, "__dove.logs()").await?;
            let lines: Vec<String> = v
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|e| {
                            format!(
                                "[{}] {}",
                                e.get("level").and_then(Value::as_str).unwrap_or("log"),
                                e.get("text").and_then(Value::as_str).unwrap_or("")
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            Outcome::ok(if lines.is_empty() { "No console messages since the last check.".into() } else { lines.join("\n") })
        }
        "browser_back" | "browser_forward" => {
            let forward = tool == "browser_forward";
            b.update(cx, |b, _| b.go_history(forward)).map_err(|_| "The browser tab was closed")?;
            settle(b, cx, 10_000).await;
            with_snapshot(b, cx, if forward { "Went forward".into() } else { "Went back".into() }).await
        }
        "browser_reload" => {
            b.update(cx, |b, cx| b.reload_page(cx)).map_err(|_| "The browser tab was closed")?;
            settle(b, cx, 15_000).await;
            with_snapshot(b, cx, "Reloaded".into()).await
        }
        other => return Err(format!("Unknown browser tool {other}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_list_is_well_formed() {
        let tools = tools();
        let names: Vec<&str> = tools.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"browser_click"));
        assert!(names.iter().all(|n| n.starts_with("browser_")));
        for t in tools.as_array().unwrap() {
            assert_eq!(t["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn keys_map() {
        assert_eq!(key_info("Enter").unwrap().2, 13);
        assert_eq!(key_info("a").unwrap().1, "KeyA");
        assert_eq!(key_info("F5").unwrap().2, 116);
        assert!(key_info("Nope").is_none());
    }
}
