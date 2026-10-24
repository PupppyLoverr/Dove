//! Antigravity's `agy` CLI, over its print-mode stream.
//!
//! The installed CLI is not the ACP server (`agy_acp_server`). It speaks
//! newline-delimited JSON: each stdin line is `{"event":"user","message":…}`
//! and stdout is `init`, `step_update`, and `result` frames.

use std::collections::VecDeque;
use std::path::Path;

use dove_proto::{AgentEvent, PermissionMode, ReasoningLevel, ToolKind, Usage};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::{Command, SessionSpec, process};

pub fn is_cli(program: &Path) -> bool {
    program.file_stem().is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case("agy"))
}

pub async fn run(spec: SessionSpec, events: flume::Sender<AgentEvent>, commands: flume::Receiver<Command>) {
    let mut queued: VecDeque<String> = VecDeque::new();
    if let Some(text) = spec.prompt.clone().filter(|text| !text.trim().is_empty()) {
        queued.push_back(text);
    }
    while queued.is_empty() {
        match commands.recv_async().await {
            Ok(Command::Send(text)) if !text.trim().is_empty() => queued.push_back(text),
            Ok(Command::Kill) | Err(_) => {
                let _ = events.send(AgentEvent::Exited { code: None });
                return;
            }
            Ok(_) => {}
        }
    }

    let mut args = vec![
        "--output-format".into(),
        "stream-json".into(),
        "--input-format".into(),
        "stream-json".into(),
        "--print".into(),
        String::new(),
    ];
    match spec.permission_mode {
        PermissionMode::Plan => args.extend(["--mode".into(), "plan".into()]),
        PermissionMode::AcceptEdits => args.extend(["--mode".into(), "accept-edits".into()]),
        PermissionMode::Bypass => args.push("--dangerously-skip-permissions".into()),
        PermissionMode::Default => {}
    }
    if let Some(model) = cli_model(spec.model.as_deref(), spec.reasoning) {
        args.extend(["--model".into(), model]);
    }
    if let Some(id) = spec.resume.as_deref().filter(|id| !id.is_empty()) {
        args.extend(["--conversation".into(), id.to_string()]);
    }

    let cmd = process::command(&spec.program, &args, &spec.cwd);
    let mut child = match process::spawn(cmd, &spec.program) {
        Ok(child) => child,
        Err(err) => {
            let _ = events.send(AgentEvent::Error { message: format!("{err:#}").into() });
            let _ = events.send(AgentEvent::Exited { code: None });
            return;
        }
    };
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
    let mut stderr = child.stderr.take().expect("piped stderr");
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let _ = tokio::io::AsyncReadExt::read_to_string(&mut stderr, &mut buf).await;
        buf
    });

    let mut turn_open = false;
    loop {
        if !turn_open && let Some(text) = queued.pop_front() {
            if !send_user(&mut stdin, &text).await {
                break;
            }
            let _ = events.send(AgentEvent::UserMessage { text: text.into() });
            turn_open = true;
        }
        tokio::select! {
            line = stdout.next_line() => {
                let Ok(Some(line)) = line else { break };
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                for event in map_line(line) {
                    let ended = matches!(event, AgentEvent::TurnEnded { .. });
                    let _ = events.send(event);
                    if ended {
                        turn_open = false;
                    }
                }
            }
            cmd = commands.recv_async() => match cmd {
                Ok(Command::Send(text)) if !text.trim().is_empty() => queued.push_back(text),
                Ok(Command::Interrupt) | Ok(Command::Kill) | Err(_) => {
                    let _ = child.start_kill();
                    if turn_open {
                        let _ = events.send(AgentEvent::TurnEnded { usage: None, is_error: false });
                    }
                    break;
                }
                Ok(_) => {}
            },
        }
    }

    let code = child.wait().await.ok().and_then(|status| status.code());
    if code.is_some_and(|code| code != 0)
        && let Ok(err) = stderr_task.await
    {
        let tail: Vec<&str> = err.lines().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
        let tail = tail.join("\n");
        if !tail.trim().is_empty() && !tail.contains("warning:") {
            let _ = events.send(AgentEvent::Error { message: tail.into() });
        }
    }
    let _ = events.send(AgentEvent::Exited { code });
}

async fn send_user(stdin: &mut tokio::process::ChildStdin, text: &str) -> bool {
    let line = json!({
        "event": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
    });
    let mut bytes = line.to_string();
    bytes.push('\n');
    stdin.write_all(bytes.as_bytes()).await.is_ok() && stdin.flush().await.is_ok()
}

/// `agy models` folds `gemini-3.8-flash-high` into `gemini-3.8-flash` plus an
/// effort. The CLI wants the variant id back.
fn cli_model(model: Option<&str>, reasoning: Option<ReasoningLevel>) -> Option<String> {
    let model = model.map(str::trim).filter(|model| !model.is_empty())?;
    let Some(level) = reasoning else { return Some(model.to_string()) };
    let suffix = format!("-{}", level.id());
    if model.ends_with(&suffix) { Some(model.to_string()) } else { Some(format!("{model}{suffix}")) }
}

pub(crate) fn map_line(line: &str) -> Vec<AgentEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line) else { return Vec::new() };
    match value.get("event").and_then(Value::as_str).unwrap_or("") {
        "init" => {
            let init = &value["init"];
            vec![AgentEvent::SessionStarted {
                native_session_id: value.get("conversation_id").and_then(Value::as_str).map(str::to_string),
                model: init.get("model").and_then(Value::as_str).map(str::to_string),
            }]
        }
        "step_update" => map_step(&value["step_update"]),
        "result" => map_result(&value["result"]),
        _ => Vec::new(),
    }
}

fn map_step(step: &Value) -> Vec<AgentEvent> {
    let step_type = step.get("step_type").and_then(Value::as_str).unwrap_or("");
    let state = step.get("state").and_then(Value::as_str).unwrap_or("");
    let text = step.get("text_delta").and_then(Value::as_str).unwrap_or("");
    if step_type == "user_input" {
        return Vec::new();
    }
    if step_type == "agent_response" {
        return (!text.is_empty()).then(|| AgentEvent::TextDelta { text: text.into() }).into_iter().collect();
    }
    if step_type == "error_message" {
        return (!text.is_empty()).then(|| AgentEvent::Error { message: text.into() }).into_iter().collect();
    }
    if step_type.contains("think") && !text.is_empty() {
        return vec![AgentEvent::ThinkingDelta { text: text.into() }];
    }
    let id = step
        .get("step_index")
        .and_then(Value::as_u64)
        .map(|index| format!("agy-{index}"))
        .unwrap_or_else(|| "agy-step".into());
    let brief = str_at(step, &["description", "prompt", "task", "title"]);
    let kind = ToolKind::classify(step_type);
    let (name, title) = if kind == ToolKind::Agent {
        let title = brief.clone().unwrap_or_else(|| step_type.to_string());
        (format!("Agent: {title}"), title)
    } else {
        (step_type.to_string(), brief.clone().unwrap_or_else(|| step_type.to_string()))
    };
    let mut input = step.clone();
    if let Some(obj) = input.as_object_mut() {
        if let Some(brief) = &brief {
            obj.entry("description".to_string()).or_insert_with(|| Value::String(brief.clone()));
            obj.entry("prompt".to_string()).or_insert_with(|| Value::String(brief.clone()));
        }
        if let Some(kind_name) = step.get("subagent_type").and_then(Value::as_str) {
            obj.entry("subagent_type".to_string()).or_insert_with(|| Value::String(kind_name.to_string()));
        }
    }
    match state {
        "ACTIVE" | "RUNNING" => {
            vec![AgentEvent::ToolCall { call_id: id, name, kind, title: title.into(), input }]
        }
        "DONE" | "FAILED" | "ERROR" => {
            vec![AgentEvent::ToolResult { call_id: id, output: text.into(), is_error: state != "DONE" }]
        }
        _ => Vec::new(),
    }
}

fn map_result(result: &Value) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    let status = result.get("status").and_then(Value::as_str).unwrap_or("");
    if let Some(message) = result.get("error").and_then(Value::as_str).filter(|text| !text.is_empty()) {
        out.push(AgentEvent::Error { message: message.into() });
    }
    let usage = result.get("usage");
    let usage = Usage {
        input_tokens: usage.and_then(|u| u.get("input_tokens")).and_then(Value::as_u64).unwrap_or(0),
        output_tokens: usage.and_then(|u| u.get("output_tokens")).and_then(Value::as_u64).unwrap_or(0),
        cache_read_tokens: usage.and_then(|u| u.get("cache_read_tokens")).and_then(Value::as_u64).unwrap_or(0),
        cache_write_tokens: 0,
        cost_usd: None,
        estimated: false,
    };
    let reported = (usage.total_tokens() > 0).then_some(usage);
    out.push(AgentEvent::TurnEnded { usage: reported, is_error: status != "SUCCESS" });
    out
}

fn str_at(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(Value::as_str).filter(|text| !text.trim().is_empty()).map(str::to_string).or_else(
            || {
                value
                    .get("input")
                    .and_then(|input| input.get(*key))
                    .and_then(Value::as_str)
                    .filter(|text| !text.trim().is_empty())
                    .map(str::to_string)
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_frames_become_text_and_a_finished_turn() {
        let init = map_line(r#"{"event":"init","conversation_id":"abc","init":{"model":"gemini-3.8-flash-low"}}"#);
        assert!(matches!(
            &init[..],
            [AgentEvent::SessionStarted { native_session_id, model }]
                if native_session_id.as_deref() == Some("abc") && model.as_deref() == Some("gemini-3.8-flash-low")
        ));
        let text = map_line(
            r#"{"event":"step_update","step_update":{"step_type":"agent_response","state":"ACTIVE","text_delta":"OK"}}"#,
        );
        assert!(matches!(&text[..], [AgentEvent::TextDelta { text }] if &**text == "OK"));
        let done = map_line(
            r#"{"event":"result","result":{"status":"SUCCESS","usage":{"input_tokens":3,"output_tokens":1}}}"#,
        );
        assert!(matches!(
            &done[..],
            [AgentEvent::TurnEnded { usage: Some(u), is_error: false }] if u.output_tokens == 1
        ));
    }

    #[test]
    fn invoke_subagent_is_an_agent_spawn() {
        let events = map_line(
            r#"{"event":"step_update","step_update":{"step_index":2,"state":"ACTIVE","step_type":"invoke_subagent","description":"Find the landing page","prompt":"Look in apps/"}}"#,
        );
        match &events[..] {
            [AgentEvent::ToolCall { name, kind, title, input, .. }] => {
                assert_eq!(*kind, ToolKind::Agent);
                assert_eq!(&**name, "Agent: Find the landing page");
                assert_eq!(&**title, "Find the landing page");
                assert_eq!(input["prompt"].as_str(), Some("Look in apps/"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn effort_is_appended_once() {
        assert_eq!(
            cli_model(Some("gemini-3.8-flash"), Some(ReasoningLevel::Low)).as_deref(),
            Some("gemini-3.8-flash-low")
        );
        assert_eq!(
            cli_model(Some("gemini-3.8-flash-low"), Some(ReasoningLevel::Low)).as_deref(),
            Some("gemini-3.8-flash-low")
        );
        assert_eq!(cli_model(Some("claude-sonnet-4-6"), None).as_deref(), Some("claude-sonnet-4-6"));
    }
}
