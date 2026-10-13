//! Keep a thread's chat live while its agent runs in the terminal.
//!
//! The TUI writes its own session transcript (Claude:
//! `~/.claude/projects/<slug>/<id>.jsonl`, Codex: its rollout file). The
//! mirror tails that file from where it stood when the TUI started, turns
//! each new line into [`AgentEvent`]s with a tolerant parser (unknown lines
//! are skipped), records them like a live session, and forwards them to the
//! chat view. Parsers are versioned by shape, not by CLI version: they read
//! only fields both agents have kept stable.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use dove_proto::{AgentEvent, HarnessKind, TodoItem, TodoStatus, ToolKind, Usage};
use serde_json::Value;

/// A running mirror. Dropping it stops the tail.
pub struct MirrorSession {
    pub events: flume::Receiver<AgentEvent>,
    stop: Arc<AtomicBool>,
}

impl Drop for MirrorSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Turns one transcript line into events. Keeps per-file state (tool names
/// by call id, whether a turn is open).
#[derive(Default)]
pub struct LineParser {
    kind: Option<HarnessKind>,
    tools: HashMap<String, String>,
    turn_open: bool,
    /// Lines stamped before this (ISO-8601 UTC) are history, not news.
    min_timestamp: Option<String>,
}

impl LineParser {
    pub fn new(kind: HarnessKind) -> Self {
        Self { kind: Some(kind), ..Default::default() }
    }

    /// Ignore lines stamped before `iso` (both agents stamp every line).
    pub fn since(mut self, iso: String) -> Self {
        self.min_timestamp = Some(iso);
        self
    }

    pub fn parse(&mut self, line: &str) -> Vec<AgentEvent> {
        let Ok(v) = serde_json::from_str::<Value>(line) else { return Vec::new() };
        if let (Some(min), Some(ts)) = (&self.min_timestamp, v["timestamp"].as_str()) {
            if ts < min.as_str() {
                return Vec::new();
            }
        }
        match self.kind {
            Some(HarnessKind::Codex) => self.codex(&v),
            _ => self.claude(&v),
        }
    }

    fn user_text(&mut self, text: &str, out: &mut Vec<AgentEvent>) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        if self.turn_open {
            out.push(AgentEvent::TurnEnded { usage: None, is_error: false });
        }
        self.turn_open = true;
        out.push(AgentEvent::UserMessage { text: text.into() });
    }

    // ------------------------------------------------------------- claude

    fn claude(&mut self, v: &Value) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        if v["isMeta"].as_bool() == Some(true) || v["isSidechain"].as_bool() == Some(true) {
            return out;
        }
        let msg = &v["message"];
        match v["type"].as_str() {
            Some("user") => match &msg["content"] {
                Value::String(text) if !is_claude_noise(text) => self.user_text(text, &mut out),
                Value::Array(blocks) => {
                    for b in blocks {
                        match b["type"].as_str() {
                            Some("text") => {
                                let text = b["text"].as_str().unwrap_or_default();
                                if !is_claude_noise(text) {
                                    self.user_text(text, &mut out);
                                }
                            }
                            Some("tool_result") => out.push(AgentEvent::ToolResult {
                                call_id: b["tool_use_id"].as_str().unwrap_or_default().to_string(),
                                output: tool_result_text(&b["content"]).into(),
                                is_error: b["is_error"].as_bool().unwrap_or(false),
                            }),
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
            Some("assistant") => {
                for b in msg["content"].as_array().into_iter().flatten() {
                    match b["type"].as_str() {
                        Some("text") => {
                            let text = b["text"].as_str().unwrap_or_default();
                            if !text.is_empty() {
                                out.push(AgentEvent::TextDelta { text: text.into() });
                                out.push(AgentEvent::BlockEnd);
                            }
                        }
                        Some("thinking") => {
                            let text = b["thinking"].as_str().unwrap_or_default();
                            if !text.is_empty() {
                                out.push(AgentEvent::ThinkingDelta { text: text.into() });
                            }
                        }
                        Some("tool_use") => {
                            let id = b["id"].as_str().unwrap_or_default().to_string();
                            let name = b["name"].as_str().unwrap_or("tool").to_string();
                            let input = b["input"].clone();
                            self.tools.insert(id.clone(), name.clone());
                            if name == "TodoWrite" {
                                out.push(AgentEvent::Todos { items: claude_todos(&input) });
                            }
                            out.push(AgentEvent::ToolCall {
                                call_id: id,
                                kind: ToolKind::classify(&name),
                                title: claude_tool_title(&name, &input).into(),
                                input: claude_edit_input(&name, &input),
                                name,
                            });
                        }
                        _ => {}
                    }
                }
                let u = &msg["usage"];
                if u.is_object() {
                    let tokens = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"]
                        .iter()
                        .filter_map(|k| u[*k].as_u64())
                        .sum::<u64>();
                    if tokens > 0 {
                        out.push(AgentEvent::ContextUsage { tokens: Some(tokens), window: None });
                    }
                }
                if matches!(msg["stop_reason"].as_str(), Some("end_turn" | "stop_sequence" | "max_tokens")) {
                    self.turn_open = false;
                    let usage = u.is_object().then(|| Usage {
                        input_tokens: u["input_tokens"].as_u64().unwrap_or(0),
                        output_tokens: u["output_tokens"].as_u64().unwrap_or(0),
                        cache_read_tokens: u["cache_read_input_tokens"].as_u64().unwrap_or(0),
                        cache_write_tokens: u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
                        cost_usd: None,
                        estimated: false,
                    });
                    out.push(AgentEvent::TurnEnded { usage, is_error: false });
                }
            }
            _ => {}
        }
        out
    }

    // -------------------------------------------------------------- codex

    fn codex(&mut self, v: &Value) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        let p = &v["payload"];
        match (v["type"].as_str(), p["type"].as_str()) {
            (Some("event_msg"), Some("user_message")) => {
                let text = p["message"].as_str().unwrap_or_default();
                self.user_text(text, &mut out);
            }
            (Some("event_msg"), Some("task_complete")) => {
                self.turn_open = false;
                out.push(AgentEvent::TurnEnded { usage: None, is_error: false });
            }
            (Some("event_msg"), Some("token_count")) => {
                let info = &p["info"];
                let tokens = info["last_token_usage"]["total_tokens"].as_u64();
                let window = info["model_context_window"].as_u64().filter(|w| *w > 0);
                if tokens.is_some() || window.is_some() {
                    out.push(AgentEvent::ContextUsage { tokens, window });
                }
            }
            (Some("response_item"), Some("message")) if p["role"] == "assistant" => {
                let text: String = p["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("");
                if !text.is_empty() {
                    out.push(AgentEvent::TextDelta { text: text.into() });
                    out.push(AgentEvent::BlockEnd);
                }
            }
            (Some("response_item"), Some("reasoning")) => {
                for s in p["summary"].as_array().into_iter().flatten() {
                    if let Some(t) = s["text"].as_str() {
                        out.push(AgentEvent::ThinkingDelta { text: format!("{t}\n").into() });
                    }
                }
            }
            (Some("response_item"), Some("function_call" | "custom_tool_call" | "local_shell_call")) => {
                let id = p["call_id"].as_str().unwrap_or_default().to_string();
                let name = p["name"].as_str().unwrap_or("shell").to_string();
                let args: Value = p["arguments"]
                    .as_str()
                    .and_then(|a| serde_json::from_str(a).ok())
                    .unwrap_or_else(|| p["input"].clone());
                let (kind, title) = codex_tool(&name, &args, p);
                self.tools.insert(id.clone(), name.clone());
                out.push(AgentEvent::ToolCall { call_id: id, name, kind, title: title.into(), input: args });
            }
            (Some("response_item"), Some("function_call_output" | "custom_tool_call_output")) => {
                let raw = p["output"].as_str().unwrap_or_default();
                // Shell outputs are JSON `{output, metadata:{exit_code}}`.
                let (text, failed) = match serde_json::from_str::<Value>(raw) {
                    Ok(o) if o["output"].is_string() => (
                        o["output"].as_str().unwrap_or_default().to_string(),
                        o["metadata"]["exit_code"].as_i64().is_some_and(|c| c != 0),
                    ),
                    _ => (raw.to_string(), false),
                };
                out.push(AgentEvent::ToolResult {
                    call_id: p["call_id"].as_str().unwrap_or_default().to_string(),
                    output: text.into(),
                    is_error: failed,
                });
            }
            _ => {}
        }
        out
    }
}

/// Command echoes and local-command wrappers Claude stores as user lines.
fn is_claude_noise(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("<command-")
        || t.starts_with("<local-command")
        || t.starts_with("Caveat:")
        || t.starts_with("[Request interrupted")
        || t.starts_with("<system-reminder>")
}

fn tool_result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

fn claude_tool_title(name: &str, input: &Value) -> String {
    let s = |k: &str| input[k].as_str().map(str::to_string);
    match name {
        "Bash" => s("command"),
        "Read" | "Edit" | "MultiEdit" | "Write" => s("file_path"),
        "NotebookEdit" => s("notebook_path"),
        "Grep" | "Glob" => s("pattern"),
        "WebFetch" => s("url"),
        "WebSearch" => s("query"),
        "Task" | "Agent" => s("description"),
        "TodoWrite" => Some("Plan".into()),
        _ => None,
    }
    .unwrap_or_else(|| name.to_string())
}

/// Edit inputs in the shape the chat's mini diff reads.
fn claude_edit_input(name: &str, input: &Value) -> Value {
    match name {
        "Edit" => serde_json::json!({
            "path": input["file_path"], "old_string": input["old_string"], "new_string": input["new_string"],
        }),
        "Write" => serde_json::json!({ "path": input["file_path"], "content": input["content"] }),
        _ => input.clone(),
    }
}

fn claude_todos(input: &Value) -> Vec<TodoItem> {
    input["todos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            Some(TodoItem {
                text: t["content"].as_str()?.to_string(),
                status: TodoStatus::parse(t["status"].as_str().unwrap_or("")),
            })
        })
        .collect()
}

fn codex_tool(name: &str, args: &Value, payload: &Value) -> (ToolKind, String) {
    match name {
        "shell" | "exec_command" | "local_shell" => {
            let cmd = match &args["command"] {
                Value::Array(parts) => {
                    let parts: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
                    // `bash -lc "<script>"`: show the script.
                    match parts.as_slice() {
                        [_, flag, script] if flag.starts_with('-') && flag.contains('c') => script.to_string(),
                        _ => parts.join(" "),
                    }
                }
                Value::String(s) => s.clone(),
                _ => payload["action"]["command"].to_string(),
            };
            (ToolKind::Execute, cmd)
        }
        "apply_patch" => {
            let patch = args.as_str().or_else(|| args["input"].as_str()).unwrap_or_default();
            let file = patch
                .lines()
                .find_map(|l| l.strip_prefix("*** Update File: ").or_else(|| l.strip_prefix("*** Add File: ")))
                .unwrap_or("patch");
            (ToolKind::Edit, file.to_string())
        }
        other => (ToolKind::classify(other), other.to_string()),
    }
}

/// Tail `path` from byte `from`, parsing with `parser`, until `stop`.
#[cfg(test)]
pub(crate) fn spawn_tail(
    rt: &tokio::runtime::Handle,
    path: PathBuf,
    from: u64,
    parser: LineParser,
    on_event: impl FnMut(&AgentEvent) + Send + 'static,
) -> MirrorSession {
    spawn_tail_resolving(rt, path, from, None::<fn() -> Option<PathBuf>>, parser, on_event)
}

/// Like [`spawn_tail`], re-resolving the file every couple of seconds:
/// an agent may start a new transcript file when it resumes (Codex).
/// A newly found file is read from its start; the parser's timestamp floor
/// keeps copied history out.
pub(crate) fn spawn_tail_resolving<R: Fn() -> Option<PathBuf> + Send + 'static>(
    rt: &tokio::runtime::Handle,
    mut path: PathBuf,
    from: u64,
    resolve: Option<R>,
    mut parser: LineParser,
    mut on_event: impl FnMut(&AgentEvent) + Send + 'static,
) -> MirrorSession {
    let (tx, rx) = flume::unbounded();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    rt.spawn_blocking(move || {
        let mut offset = from;
        let mut partial = String::new();
        let mut buf = Vec::new();
        let mut last_resolve = std::time::Instant::now();
        while !flag.load(Ordering::Relaxed) && !tx.is_disconnected() {
            if let Some(resolve) = &resolve {
                if last_resolve.elapsed() >= Duration::from_secs(2) {
                    last_resolve = std::time::Instant::now();
                    if let Some(p) = resolve().filter(|p| *p != path) {
                        path = p;
                        offset = 0;
                        partial.clear();
                    }
                }
            }
            let grew = std::fs::File::open(&path).ok().and_then(|mut f| {
                let len = f.metadata().ok()?.len();
                if len < offset {
                    // Rewritten from scratch: start over.
                    offset = 0;
                }
                if len == offset {
                    return None;
                }
                f.seek(SeekFrom::Start(offset)).ok()?;
                buf.clear();
                f.take(len - offset).read_to_end(&mut buf).ok()?;
                offset = len;
                Some(())
            });
            if grew.is_none() {
                std::thread::sleep(Duration::from_millis(150));
                continue;
            }
            partial.push_str(&String::from_utf8_lossy(&buf));
            while let Some(nl) = partial.find('\n') {
                let line: String = partial.drain(..=nl).collect();
                for ev in parser.parse(line.trim_end()) {
                    on_event(&ev);
                    let _ = tx.send(ev);
                }
            }
        }
    });
    MirrorSession { events: rx, stop }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude(lines: &[&str]) -> Vec<AgentEvent> {
        let mut p = LineParser::new(HarnessKind::ClaudeCode);
        lines.iter().flat_map(|l| p.parse(l)).collect()
    }

    #[test]
    fn claude_turn_round_trip() {
        let evs = claude(&[
            r#"{"type":"user","message":{"role":"user","content":"fix the bug"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hmm"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}],"stop_reason":"tool_use"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"ok"}]}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Fixed."}],"stop_reason":"end_turn","usage":{"input_tokens":10,"cache_read_input_tokens":90,"output_tokens":5}}}"#,
        ]);
        assert!(matches!(&evs[0], AgentEvent::UserMessage { text } if &**text == "fix the bug"));
        assert!(matches!(&evs[1], AgentEvent::ThinkingDelta { .. }));
        assert!(
            matches!(&evs[2], AgentEvent::ToolCall { kind: ToolKind::Execute, title, .. } if &**title == "cargo test")
        );
        assert!(matches!(&evs[3], AgentEvent::ToolResult { output, .. } if &**output == "ok"));
        assert!(matches!(&evs[4], AgentEvent::TextDelta { .. }));
        assert!(evs.iter().any(|e| matches!(e, AgentEvent::ContextUsage { tokens: Some(100), .. })));
        assert!(matches!(evs.last(), Some(AgentEvent::TurnEnded { usage: Some(u), .. }) if u.output_tokens == 5));
    }

    #[test]
    fn claude_skips_meta_and_command_noise() {
        let evs = claude(&[
            r#"{"type":"user","isMeta":true,"message":{"content":"Caveat: ..."}}"#,
            r#"{"type":"user","message":{"content":"<command-name>/model</command-name>"}}"#,
            r#"{"type":"summary","summary":"x"}"#,
            "not json",
        ]);
        assert!(evs.is_empty());
    }

    #[test]
    fn claude_edit_carries_diff_input() {
        let evs = claude(&[
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"e","name":"Edit","input":{"file_path":"a.rs","old_string":"a","new_string":"b"}}]}}"#,
        ]);
        let AgentEvent::ToolCall { input, kind, .. } = &evs[0] else { panic!("tool call") };
        assert_eq!(*kind, ToolKind::Edit);
        assert_eq!(input["old_string"], "a");
    }

    #[test]
    fn codex_rollout_lines() {
        let mut p = LineParser::new(HarnessKind::Codex);
        let lines = [
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"add tests"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"ls\"]}","call_id":"c1"}}"#,
            r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"{\"output\":\"a\\nb\",\"metadata\":{\"exit_code\":1}}"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done"}]}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":1200},"model_context_window":272000}}}"#,
            r#"{"type":"event_msg","payload":{"type":"task_complete"}}"#,
        ];
        let evs: Vec<AgentEvent> = lines.iter().flat_map(|l| p.parse(l)).collect();
        assert!(matches!(&evs[0], AgentEvent::UserMessage { text } if &**text == "add tests"));
        assert!(matches!(&evs[1], AgentEvent::ToolCall { title, .. } if &**title == "ls"));
        assert!(matches!(&evs[2], AgentEvent::ToolResult { is_error: true, output, .. } if &**output == "a\nb"));
        assert!(matches!(&evs[3], AgentEvent::TextDelta { .. }));
        assert!(evs.contains(&AgentEvent::ContextUsage { tokens: Some(1200), window: Some(272000) }));
        assert!(matches!(evs.last(), Some(AgentEvent::TurnEnded { .. })));
    }

    #[test]
    fn a_new_prompt_closes_an_open_turn() {
        let evs = claude(&[
            r#"{"type":"user","message":{"content":"one"}}"#,
            r#"{"type":"user","message":{"content":"two"}}"#,
        ]);
        assert!(matches!(evs[1], AgentEvent::TurnEnded { .. }));
    }

    #[test]
    fn tail_reads_appended_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        std::fs::write(&path, "{\"type\":\"user\",\"message\":{\"content\":\"old\"}}\n").unwrap();
        let from = std::fs::metadata(&path).unwrap().len();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let m = spawn_tail(rt.handle(), path.clone(), from, LineParser::new(HarnessKind::ClaudeCode), |_| {});
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "{{\"type\":\"user\",\"message\":{{\"content\":\"new\"}}}}").unwrap();
        let ev = m.events.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(matches!(ev, AgentEvent::UserMessage { text } if &*text == "new"));
    }
}
