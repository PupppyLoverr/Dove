//! The team tools every agent in Dove gets through Dove's tool server:
//! delegate work to any agent, model or teammate, talk to teammates, and
//! read, steer, wait on and stop threads. The hub process lists them and
//! forwards calls to the app (`Shell::route_team_call`), which performs them
//! with the same paths a click uses.

use serde_json::{Value, json};

/// The longest a single tool call waits on other agents.
pub const MAX_WAIT_SECS: u64 = 1800;

pub const NAMES: &[&str] = &[
    "team_capabilities",
    "delegate_task",
    "task_status",
    "task_cancel",
    "team_message",
    "thread_list",
    "thread_read",
    "thread_send",
    "thread_wait",
    "thread_interrupt",
];

pub fn is_team_tool(name: &str) -> bool {
    NAMES.contains(&name)
}

pub const INSTRUCTIONS: &str = "\n\nTeam: you can work with other agents and the user's teammates in Dove. \
`team_capabilities` lists the installed agents (Claude Code, Codex, Gemini, OpenCode, Cursor and others), their \
models, the user's teammates and your own thread. `delegate_task` hands one task to a new child thread on any agent \
and model, or to a teammate; the child gets only your task text, so make it self-contained. Use mode \"async\" for \
longer work and end your turn: when children finish, Dove wakes you with their results in one team update. Use \
mode \"wait\" only for short tasks you need right away. isolation \"worktree\" gives the child its own git worktree \
(for edits that would collide with yours). `task_status`/`task_cancel` track and stop what you delegated. \
`team_message` asks a teammate something; their reply comes back to you the same way. `thread_list`, `thread_read`, \
`thread_send`, `thread_wait` and `thread_interrupt` work on any thread. A child can never get broader permissions \
than you, delegation nests at most three deep, and each thread may have six children working at once. Don't \
delegate trivial steps; delegate parallel or specialised work (a review by another model, tests while you build, \
research).";

fn obj(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required})
}

/// The tool definitions, as MCP `tools/list` entries.
pub fn tools() -> Vec<Value> {
    let mode = json!({"type": "string", "enum": ["async", "wait"],
        "description": "async (default): return at once; the result is delivered to you when the child finishes. wait: block until it finishes, needs input, or timeout_seconds pass (the child keeps running after a timeout)."});
    let timeout = json!({"type": "integer", "minimum": 5, "maximum": MAX_WAIT_SECS,
        "description": "How long to wait, in seconds (default 600). A timeout never stops the other agent."});
    let thread = json!({"type": "string", "description": "A thread id from thread_list, team_capabilities or a delegate_task result."});
    vec![
        json!({
            "name": "team_capabilities",
            "description": "What you can orchestrate: your own thread (id, agent, model, teammate, permission mode, project), the installed agents with their models and reasoning levels, the user's teammates (name, role, preferred agent, what they are working on) and the limits on delegation. Call it before choosing an agent or teammate.",
            "inputSchema": obj(json!({"search": {"type": "string", "description": "Only models whose id or name contains this."}}), &[]),
            "annotations": {"title": "Team and agents", "readOnlyHint": true}
        }),
        json!({
            "name": "delegate_task",
            "description": "Hand one task to a new child thread: on any installed agent and model, or as one of the user's teammates. The child gets only `task` (not your conversation), so include the context it needs. It starts in your project and shares your working folder, or its own git worktree with isolation \"worktree\". Its permission mode is yours or narrower. Returns its task_id (the child thread's id). With mode async, end your turn: Dove wakes you with the result. Retrying with the same request_key returns the same child instead of starting another.",
            "inputSchema": obj(json!({
                "task": {"type": "string", "description": "The complete task: goal, context, constraints, and what to report back."},
                "title": {"type": "string", "description": "A short title for the child thread."},
                "teammate": {"type": "string", "description": "A teammate's name or id from team_capabilities: the child runs as them (their role, instructions and notes)."},
                "agent": {"type": "string", "description": "Which agent runs it: claude, codex, gemini, opencode, cursor, … (team_capabilities lists them). Default: the teammate's preferred agent, else yours."},
                "model": {"type": "string", "description": "A model id from team_capabilities for that agent. Default: the agent's default (or the teammate's preferred model)."},
                "reasoning": {"type": "string", "enum": ["minimal", "low", "medium", "high", "xhigh", "max"], "description": "Reasoning effort, where the agent supports it."},
                "isolation": {"type": "string", "enum": ["shared", "worktree"], "description": "shared (default): your working folder. worktree: a new git worktree from your latest commit (uncommitted edits are not copied)."},
                "permission": {"type": "string", "enum": ["plan", "default", "accept_edits", "bypass"], "description": "The child's permission mode; never more than yours (default: yours)."},
                "mode": mode,
                "timeout_seconds": timeout,
                "request_key": {"type": "string", "description": "A key that makes retries return the same child."}
            }), &["task"]),
            "annotations": {"title": "Delegate a task", "destructiveHint": false, "openWorldHint": true}
        }),
        json!({
            "name": "task_status",
            "description": "How a task you delegated stands: running, needs_input (waiting for the user's approval or answer), completed, failed or cancelled, with its result once finished. Reading a finished result here means Dove won't deliver it to you again.",
            "inputSchema": obj(json!({"task_id": {"type": "string"}}), &["task_id"]),
            "annotations": {"title": "Delegated task status", "readOnlyHint": true}
        }),
        json!({
            "name": "task_cancel",
            "description": "Stop a task you delegated, and everything it delegated in turn. Its thread stays for the user to read.",
            "inputSchema": obj(json!({"task_id": {"type": "string"}, "reason": {"type": "string"}}), &["task_id"]),
            "annotations": {"title": "Cancel a delegated task", "destructiveHint": true}
        }),
        json!({
            "name": "team_message",
            "description": "Talk to one of the user's teammates. The message goes to their most recent thread in this project (or a new thread as them, with new_thread). With wants_reply (default true) their answer is delivered back to you when they finish; with mode wait you get it in this call.",
            "inputSchema": obj(json!({
                "teammate": {"type": "string", "description": "A teammate's name or id."},
                "message": {"type": "string"},
                "new_thread": {"type": "boolean", "description": "Start a fresh thread as them instead of continuing their latest."},
                "wants_reply": {"type": "boolean"},
                "agent": {"type": "string", "description": "For a new thread: the agent to run them on (default: their preferred agent, else yours)."},
                "mode": mode,
                "timeout_seconds": timeout
            }), &["teammate", "message"]),
            "annotations": {"title": "Message a teammate", "openWorldHint": true}
        }),
        json!({
            "name": "thread_list",
            "description": "Threads in your project (or every project with all_projects), newest first: id, title, agent, teammate, status, and who started it.",
            "inputSchema": obj(json!({
                "query": {"type": "string", "description": "Only titles containing this."},
                "status": {"type": "string", "enum": ["running", "needs_input", "done", "failed", "idle"]},
                "teammate": {"type": "string"},
                "all_projects": {"type": "boolean"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 100}
            }), &[]),
            "annotations": {"title": "List threads", "readOnlyHint": true}
        }),
        json!({
            "name": "thread_read",
            "description": "Read a thread: each turn's request, the tools used and the answer. Long threads keep the newest turns; pass after (a turn number) to read from there.",
            "inputSchema": obj(json!({
                "thread_id": thread,
                "after": {"type": "integer", "minimum": 0, "description": "Skip this many turns."},
                "max_chars": {"type": "integer", "minimum": 500, "maximum": 60000}
            }), &["thread_id"]),
            "annotations": {"title": "Read a thread", "readOnlyHint": true}
        }),
        json!({
            "name": "thread_send",
            "description": "Send a message to any thread. A running thread is steered (or the message waits for its turn on agents that can't be steered); an idle one starts a turn. With wants_reply, its answer to this message is delivered back to you. The target can't be given broader permissions than yours.",
            "inputSchema": obj(json!({
                "thread_id": thread,
                "message": {"type": "string"},
                "wants_reply": {"type": "boolean"}
            }), &["thread_id", "message"]),
            "annotations": {"title": "Send to a thread", "openWorldHint": true}
        }),
        json!({
            "name": "thread_wait",
            "description": "Wait until a thread stops working (done, failed, or waiting for the user) or the timeout passes. Returns its status and latest answer. Waiting never stops it.",
            "inputSchema": obj(json!({"thread_id": thread, "timeout_seconds": timeout}), &["thread_id"]),
            "annotations": {"title": "Wait for a thread", "readOnlyHint": true}
        }),
        json!({
            "name": "thread_interrupt",
            "description": "Stop a thread's current turn, like the user pressing Stop. The thread stays and can be messaged again.",
            "inputSchema": obj(json!({"thread_id": thread}), &["thread_id"]),
            "annotations": {"title": "Interrupt a thread", "destructiveHint": true}
        }),
    ]
}

/// Arguments as strings, numbers and flags, with the helpers a tool needs.
pub struct Args<'a>(pub &'a Value);

impl Args<'_> {
    pub fn str(&self, key: &str) -> Option<String> {
        match self.0.get(key)? {
            Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
    pub fn need(&self, key: &str) -> Result<String, String> {
        self.str(key).ok_or_else(|| format!("Missing \"{key}\"."))
    }
    pub fn flag(&self, key: &str, default: bool) -> bool {
        match self.0.get(key) {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => matches!(s.as_str(), "true" | "yes" | "1"),
            _ => default,
        }
    }
    pub fn int(&self, key: &str) -> Option<u64> {
        match self.0.get(key)? {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }
    /// A wait budget: `timeout_seconds`, default 600, capped.
    pub fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.int("timeout_seconds").unwrap_or(600).clamp(5, MAX_WAIT_SECS))
    }
}

/// A thread id from a tool call.
pub fn parse_thread(s: &str) -> Result<dove_core::proto::TaskId, String> {
    s.trim()
        .parse::<uuid::Uuid>()
        .map(dove_core::proto::TaskId)
        .map_err(|_| format!("\"{s}\" isn't a thread id. thread_list shows them."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_is_listed_once_with_a_schema() {
        let tools = tools();
        assert_eq!(tools.len(), NAMES.len());
        for (t, name) in tools.iter().zip(NAMES) {
            assert_eq!(t["name"], *name);
            assert_eq!(t["inputSchema"]["type"], "object");
            assert!(t["description"].as_str().unwrap().len() > 40);
            // Tool names stay within the 64-character MCP limit with the hub prefix.
            assert!(format!("mcp__dove__{name}").len() <= 64);
        }
        assert!(is_team_tool("delegate_task") && !is_team_tool("browser_click"));
    }

    #[test]
    fn args_read_loosely_typed_values() {
        let v = json!({"a": " x ", "n": 30, "s": "45", "b": true, "e": "", "f": "yes"});
        let a = Args(&v);
        assert_eq!(a.str("a").as_deref(), Some("x"));
        assert_eq!(a.str("e"), None);
        assert_eq!(a.int("n"), Some(30));
        assert_eq!(a.int("s"), Some(45));
        assert!(a.flag("b", false) && a.flag("f", false) && a.flag("missing", true));
        assert!(a.need("missing").unwrap_err().contains("missing"));
        assert_eq!(Args(&json!({"timeout_seconds": 1})).timeout().as_secs(), 5);
        assert_eq!(Args(&json!({})).timeout().as_secs(), 600);
        assert_eq!(Args(&json!({"timeout_seconds": 99999})).timeout().as_secs(), MAX_WAIT_SECS);
    }

    #[test]
    fn thread_ids_parse_or_explain() {
        let id = uuid::Uuid::now_v7();
        assert_eq!(parse_thread(&id.to_string()).unwrap().0, id);
        assert!(parse_thread("abc").unwrap_err().contains("thread_list"));
    }
}
