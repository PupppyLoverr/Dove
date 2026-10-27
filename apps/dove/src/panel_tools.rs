//! Tools that let an agent use Dove's side panel the way the user does:
//! open files, diffs, a browser page or a terminal beside the chat (several
//! at once, split into panes), see what is open, and close what it opened.
//! The usual moment is the end of a turn: "here is what I made", shown.
//!
//! Also the guide every agent in Dove gets with its tools (the server's
//! instructions): what each group is for and when to reach for it.

use serde_json::{Value, json};

use crate::side_panel::{PanelAsk, PanelLayout, Tab};

const NAMES: [&str; 3] = ["panel_open", "panel_state", "panel_close"];

pub fn is_panel_tool(name: &str) -> bool {
    NAMES.contains(&name)
}

/// How to work in Dove: the tools by purpose, and when each fits. Sent to
/// every agent (main chat, side chats, teammates, delegated agents, and
/// agents running in Dove's terminal).
pub const GUIDE: &str = "\n\nWorking in Dove: you are one agent inside Dove, a desktop app where the user runs coding \
agents in threads. Read your first message to know where you are: a message wrapped in <dove-task> is work another \
agent handed you (do exactly that, then end with a clear report, since your final answer is sent back); \
<dove-message> is a teammate writing to you (reply to what they ask); <dove-team-update> is results from agents you \
started; a teammate preamble means you work as that teammate (their role and instructions apply); anything else is \
the user talking to you directly, in the main chat, a side chat or a terminal.\n\
Pick tools by purpose. Code and files: your own file and shell tools first. Web pages, local dev servers and \
docs: the browser tools (browser_*), or chrome_* when the page needs the user's own signed-in browser. Native \
desktop apps: the computer tool (only when the user asks for something outside the browser and the code). The user's \
other apps and services: their MCP tools. Skills: dove_skills before an unfamiliar document task. Charts and \
diagrams in the chat: show_visual. Other threads and agents: the team tools.\n\
Show your work in the side panel when it helps the user see it: when you finish making or changing something \
visible, open it with panel_open (the page in the browser, the changed files or the diff, a terminal running the \
app), several at once with layout \"right\" or \"down\". Don't open panels for every small step, and never more than \
the user needs to review the result.";

/// The panel tools, as MCP `tools/list` entries.
pub fn tools() -> Vec<Value> {
    let item = json!({
        "type": "object",
        "properties": {
            "kind": {"type": "string", "enum": ["browser", "file", "diff", "terminal", "history", "plan", "pull_request", "files"],
                "description": "browser: a web page (url). file: one file in the file viewer (path, optional line). diff: the uncommitted changes. terminal: a shell in the project folder. history: commits. plan: your todo list. pull_request: the thread's PR. files: the file tree."},
            "url": {"type": "string", "description": "For browser: the page, often a local dev server like http://localhost:5173."},
            "path": {"type": "string", "description": "For file: the path, relative to the project or absolute."},
            "line": {"type": "integer", "minimum": 1, "description": "For file: the line to show."}
        },
        "required": ["kind"]
    });
    vec![
        json!({
            "name": "panel_open",
            "description": "Show things in Dove's side panel beside the chat, for the user to look at: a page in the browser, a file, the diff, a terminal, history or your plan. Pass several to open them together; layout \"right\" or \"down\" puts each in its own pane, \"tabs\" (default) as tabs of one pane. If the user is looking at another thread, they open when the user comes back to yours. Use it to present finished work, not for every step.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "items": {"type": "array", "items": item, "minItems": 1, "maxItems": 6},
                    "layout": {"type": "string", "enum": ["tabs", "right", "down"], "description": "How several items are arranged (default tabs)."}
                },
                "required": ["items"]
            },
            "annotations": {"title": "Open in the side panel", "readOnlyHint": false, "destructiveHint": false}
        }),
        json!({
            "name": "panel_state",
            "description": "What is open in your thread's side panel: each tab (index, kind, title, and whether a pane shows it), and whether the panel is visible.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"title": "Side panel state", "readOnlyHint": true}
        }),
        json!({
            "name": "panel_close",
            "description": "Close a tab you opened (its index from panel_state), or hide the panel with all: true. Closing a terminal stops its shell; closing a browser tab closes the page.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "index": {"type": "integer", "minimum": 0},
                    "all": {"type": "boolean", "description": "Hide the side panel (tabs stay)."}
                }
            },
            "annotations": {"title": "Close in the side panel", "destructiveHint": true}
        }),
    ]
}

/// Read `panel_open`'s items.
pub fn parse_open(args: &Value) -> Result<(Vec<PanelAsk>, PanelLayout), String> {
    let items = args.get("items").and_then(Value::as_array).ok_or("Give the panels to open in \"items\".")?;
    if items.is_empty() {
        return Err("Give at least one item.".into());
    }
    let mut out = Vec::new();
    for it in items.iter().take(6) {
        let kind = it.get("kind").and_then(Value::as_str).unwrap_or_default();
        let s = |k: &str| it.get(k).and_then(Value::as_str).map(str::to_string).filter(|v| !v.trim().is_empty());
        let ask = match kind {
            "browser" => PanelAsk { kind: Tab::Browser, path: None, line: None, url: s("url") },
            "file" => PanelAsk {
                kind: Tab::Files,
                path: Some(s("path").ok_or("A file item needs \"path\".")?),
                line: it.get("line").and_then(Value::as_u64).map(|l| l as u32),
                url: None,
            },
            "files" => PanelAsk::of(Tab::Files),
            "diff" => PanelAsk::of(Tab::Changes),
            "terminal" => PanelAsk::of(Tab::Terminal),
            "history" => PanelAsk::of(Tab::History),
            "plan" => PanelAsk::of(Tab::Plan),
            "pull_request" => PanelAsk::of(Tab::PullRequest),
            other => return Err(format!("Unknown kind \"{other}\".")),
        };
        out.push(ask);
    }
    let layout = match args.get("layout").and_then(Value::as_str) {
        Some("right") => PanelLayout::Right,
        Some("down") => PanelLayout::Down,
        _ => PanelLayout::Tabs,
    };
    Ok((out, layout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_items_parse() {
        let (asks, layout) = parse_open(&json!({
            "items": [{"kind": "browser", "url": "http://localhost:3000"}, {"kind": "file", "path": "src/main.rs", "line": 4}, {"kind": "diff"}],
            "layout": "right"
        }))
        .unwrap();
        assert_eq!(layout, PanelLayout::Right);
        assert_eq!(asks[0].kind, Tab::Browser);
        assert_eq!(asks[0].url.as_deref(), Some("http://localhost:3000"));
        assert_eq!((asks[1].path.as_deref(), asks[1].line), (Some("src/main.rs"), Some(4)));
        assert_eq!(asks[2].kind, Tab::Changes);
        assert!(parse_open(&json!({"items": [{"kind": "file"}]})).is_err());
        assert!(parse_open(&json!({"items": []})).is_err());
        assert!(parse_open(&json!({"items": [{"kind": "spaceship"}]})).is_err());
    }

    #[test]
    fn the_tools_are_named_as_routed() {
        for t in tools() {
            assert!(is_panel_tool(t["name"].as_str().unwrap()));
        }
    }
}
