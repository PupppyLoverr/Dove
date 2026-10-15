//! Shared agents run on Zeron's harness drivers.
//!
//! Claude Code, Codex, Cursor, OpenCode, Pi, Devin, Grok, Hermes and
//! Antigravity are the agents both apps drive (Kilo Code rides the opencode
//! driver). Those sessions use Zeron's
//! adapters (its wire, launch flags and permission bridge). Agents Dove
//! drives that Zeron does not — Gemini, Copilot, Amp, goose, Qwen, Droid,
//! Cline, Kimi — stay on Dove's own ACP driver.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use dove_proto::{
    AgentEvent, Capabilities, HarnessKind, PermissionDecision, PermissionMode, PermissionRequest, Question, RateWindow,
    ReasoningLevel, SlashCommand, TodoItem, TodoStatus, ToolKind, Usage,
};
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use zeron_harness::acp::AcpHarness;
use zeron_harness::claude::ClaudeHarness;
use zeron_harness::codex::CodexHarness;
use zeron_harness::cursor::CursorHarness;
use zeron_harness::opencode::OpencodeHarness;
use zeron_harness::pi::PiHarness;
use zeron_harness::{Harness, RunControls, SteerMessage, TurnControl};
use zeron_proto::{
    AgentEvent as ZEvent, DoneStatus, HarnessId, RunRequest, SandboxLevel, ToolCall, UserInputAnswer, UserInputQuestion,
};

use crate::{Command, SessionSpec};

pub const CAPABILITIES: Capabilities =
    Capabilities { streaming: true, permissions: true, resume: true, interrupt: true, usage: true };

/// Agents whose driver is Zeron's, not Dove's.
pub fn drives(kind: HarnessKind) -> bool {
    matches!(
        kind,
        HarnessKind::ClaudeCode
            | HarnessKind::Codex
            | HarnessKind::Cursor
            | HarnessKind::OpenCode
            | HarnessKind::Pi
            | HarnessKind::Devin
            | HarnessKind::Grok
            | HarnessKind::Hermes
            | HarnessKind::Antigravity
            | HarnessKind::Kilo
    )
}

struct Pending {
    tx: oneshot::Sender<Vec<UserInputAnswer>>,
    questions: Vec<UserInputQuestion>,
}

pub async fn run(spec: SessionSpec, ev_tx: flume::Sender<AgentEvent>, cmd_rx: flume::Receiver<Command>) {
    let Some(harness) = driver(&spec) else {
        let _ = ev_tx.send(AgentEvent::Error { message: format!("{} is not supported yet", spec.kind.label()).into() });
        let _ = ev_tx.send(AgentEvent::Exited { code: None });
        return;
    };

    let given = spec.prompt.clone().filter(|p| !p.trim().is_empty());
    let pending: Arc<Mutex<HashMap<String, Pending>>> = Arc::new(Mutex::new(HashMap::new()));
    let (steer_tx, steer_rx) = mpsc::channel(32);
    let interrupt = zeron_harness::CancellationToken::new();
    let turn = TurnControl::default();
    let stops_in_place = harness.stops_turn_in_place();

    let controls = RunControls {
        realtime: None,
        execution_lease: None,
        request_input: permission_bridge(pending.clone(), ev_tx.clone(), spec.permission_mode),
        steering: steer_rx,
        interrupt: interrupt.clone(),
        turn: turn.clone(),
    };

    // A pre-warmed Claude run (no prompt yet): boot the CLI now, so its
    // process, settings and MCP servers are up by the time the user sends;
    // the first line is released on the first Send.
    let started;
    let prompt;
    if given.is_none() && spec.kind == HarnessKind::ClaudeCode {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let token = format!("{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let release = zeron_harness::claude::defer_prompt(&token);
        let mut request = run_request(&spec, String::new());
        request.model_options.insert("dove.deferPrompt".into(), Value::String(token));
        started = harness.run(request, controls).await;
        prompt = loop {
            match cmd_rx.recv_async().await {
                Ok(Command::Send(text)) if !text.trim().is_empty() => break text,
                Ok(Command::Kill) | Err(_) => {
                    interrupt.cancel();
                    drop(started);
                    let _ = ev_tx.send(AgentEvent::Exited { code: None });
                    return;
                }
                Ok(_) => {}
            }
        };
        let _ = release.send(prompt.clone());
    } else {
        prompt = match given {
            Some(text) => text,
            None => loop {
                match cmd_rx.recv_async().await {
                    Ok(Command::Send(text)) => break text,
                    Ok(Command::Kill) | Err(_) => {
                        let _ = ev_tx.send(AgentEvent::Exited { code: None });
                        return;
                    }
                    Ok(_) => {}
                }
            },
        };
        started = harness.run(run_request(&spec, prompt.clone()), controls).await;
    }

    {
        let steer_tx = steer_tx.clone();
        let interrupt = interrupt.clone();
        let turn = turn.clone();
        let pending = pending.clone();
        let ev_tx = ev_tx.clone();
        tokio::spawn(async move {
            while let Ok(cmd) = cmd_rx.recv_async().await {
                match cmd {
                    Command::Send(text) => {
                        let _ = ev_tx.send(AgentEvent::UserMessage { text: text.clone().into() });
                        if steer_tx.send(SteerMessage::text(text)).await.is_err() {
                            break;
                        }
                    }
                    Command::Interrupt => {
                        if stops_in_place {
                            turn.stop_turn();
                        } else {
                            interrupt.cancel();
                        }
                    }
                    Command::Permission { request_id, decision } => {
                        if let Some(slot) = pending.lock().unwrap().remove(&request_id) {
                            let answers = answers_for(decision, &slot.questions);
                            let _ = slot.tx.send(answers);
                            let _ = ev_tx.send(AgentEvent::PermissionResolved { request_id, decision });
                        }
                    }
                    Command::Answer { request_id, answers } => {
                        if let Some(slot) = pending.lock().unwrap().remove(&request_id) {
                            let reply = slot
                                .questions
                                .iter()
                                .zip(answers.into_iter().chain(std::iter::repeat(Vec::new())))
                                .map(|(q, labels)| UserInputAnswer { question_id: q.id.clone(), labels })
                                .collect();
                            let _ = slot.tx.send(reply);
                            let _ = ev_tx.send(AgentEvent::PermissionResolved {
                                request_id,
                                decision: PermissionDecision::AllowOnce,
                            });
                        }
                    }
                    Command::Kill => {
                        interrupt.cancel();
                        pending.lock().unwrap().clear();
                        break;
                    }
                }
            }
        });
    }

    let _ = ev_tx.send(AgentEvent::UserMessage { text: prompt.into() });
    match started {
        Ok(mut stream) => {
            let mut usage = Usage::default();
            while let Some(item) = stream.next().await {
                match item {
                    Ok(event) => {
                        for mapped in map_event(event, &mut usage) {
                            let _ = ev_tx.send(mapped);
                        }
                    }
                    Err(err) => {
                        let _ = ev_tx.send(AgentEvent::Error { message: err.to_string().into() });
                    }
                }
            }
        }
        Err(err) => {
            let _ = ev_tx.send(AgentEvent::Error { message: err.to_string().into() });
            let _ = ev_tx.send(AgentEvent::TurnEnded { usage: None, is_error: true });
        }
    }
    let _ = ev_tx.send(AgentEvent::Exited { code: None });
}

fn driver(spec: &SessionSpec) -> Option<Arc<dyn Harness>> {
    driver_for(spec.kind, &spec.program)
}

fn driver_for(kind: HarnessKind, program: &std::path::Path) -> Option<Arc<dyn Harness>> {
    Some(match kind {
        HarnessKind::ClaudeCode => Arc::new(ClaudeHarness::new().with_executable(program)),
        HarnessKind::Codex => Arc::new(CodexHarness::new().with_executable(program)),
        HarnessKind::Cursor => Arc::new(CursorHarness::new().with_executable(program)),
        HarnessKind::OpenCode => Arc::new(OpencodeHarness::new().with_executable(program)),
        HarnessKind::Pi => Arc::new(PiHarness::new().with_executable(program)),
        HarnessKind::Devin => Arc::new(AcpHarness::devin().with_executable(program)),
        HarnessKind::Grok => Arc::new(AcpHarness::grok().with_executable(program)),
        HarnessKind::Hermes => Arc::new(AcpHarness::hermes().with_executable(program)),
        HarnessKind::Antigravity => Arc::new(AcpHarness::antigravity().with_executable(program)),
        // Kilo Code is an opencode fork: `kilo serve` speaks the same HTTP +
        // SSE wire. Its `acp` mode hangs on prompts (7.4), so it isn't used.
        HarnessKind::Kilo => Arc::new(OpencodeHarness::new().with_executable(program)),
        _ => return None,
    })
}

fn permission_bridge(
    pending: Arc<Mutex<HashMap<String, Pending>>>,
    ev_tx: flume::Sender<AgentEvent>,
    mode: PermissionMode,
) -> Box<dyn Fn(Vec<UserInputQuestion>) -> oneshot::Receiver<Vec<UserInputAnswer>> + Send + Sync> {
    Box::new(move |questions| {
        let (tx, rx) = oneshot::channel();
        if questions.is_empty() {
            let _ = tx.send(Vec::new());
            return rx;
        }
        let approval = questions.len() == 1 && is_approval(&questions[0].header);
        if approval && auto_allows(mode, &questions[0].header) {
            let _ = tx.send(answers_for(PermissionDecision::AllowOnce, &questions));
            return rx;
        }
        let id = questions[0].id.clone();
        let header = questions[0].header.clone();
        let summary = questions
            .iter()
            .map(|q| {
                if q.header.is_empty() || approval {
                    q.question.clone()
                } else {
                    format!("{} — {}", q.header, q.question)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let asked = if approval {
            Vec::new()
        } else {
            questions
                .iter()
                .map(|q| Question {
                    id: q.id.clone(),
                    header: q.header.clone(),
                    question: q.question.clone(),
                    options: q.options.clone(),
                    multi_select: q.multi_select,
                })
                .collect()
        };
        pending.lock().unwrap().insert(id.clone(), Pending { tx, questions });
        let tool = if approval {
            header.trim_start_matches("Approve").trim().to_string()
        } else if header.is_empty() {
            "Question".into()
        } else {
            header
        };
        let _ = ev_tx.send(AgentEvent::PermissionRequested(PermissionRequest {
            request_id: id,
            tool: if tool.is_empty() { "tool".into() } else { tool },
            summary: summary.into(),
            input: Value::Null,
            questions: asked,
        }));
        rx
    })
}

/// Zeron's drivers title tool approvals "Approve <tool>"; anything else is a
/// question for the user.
fn is_approval(header: &str) -> bool {
    header.starts_with("Approve")
}

/// Approvals the composer's permission level answers without asking.
fn auto_allows(mode: PermissionMode, header: &str) -> bool {
    if crate::is_dove_tool(header) {
        return true;
    }
    let tool = header.trim_start_matches("Approve").trim().to_ascii_lowercase();
    match mode {
        PermissionMode::Bypass => true,
        PermissionMode::AcceptEdits => {
            matches!(ToolKind::classify(&tool), ToolKind::Edit) || tool.contains("file change") || tool == "edit"
        }
        PermissionMode::Default | PermissionMode::Plan => false,
    }
}

fn run_request(spec: &SessionSpec, prompt: String) -> RunRequest {
    let (sandbox, auto_approve) = match spec.permission_mode {
        PermissionMode::Plan => (SandboxLevel::ReadOnly, false),
        PermissionMode::Bypass => (SandboxLevel::DangerFullAccess, true),
        PermissionMode::Default | PermissionMode::AcceptEdits => (SandboxLevel::WorkspaceWrite, false),
    };
    let mut model_options = serde_json::Map::new();
    for (key, value) in &spec.model_options {
        model_options.insert(key.clone(), Value::String(value.clone()));
    }
    let cli_mode = match spec.permission_mode {
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Plan => "plan",
        _ => "default",
    };
    model_options.insert("dove.permissionMode".into(), Value::String(cli_mode.into()));
    RunRequest {
        prompt,
        harness: harness_id(spec.kind),
        model: spec.model.clone(),
        reasoning: spec.reasoning.map(reasoning),
        model_options,
        cwd: spec.cwd.display().to_string(),
        sandbox,
        auto_approve,
        resume: spec.resume.clone(),
        attachments: Vec::new(),
        worktree: None,
        mcp: crate::tool_server(&spec.cwd).map(|s| zeron_proto::McpServer {
            name: s.name,
            command: s.command,
            args: s.args,
            env: s.env.into_iter().collect(),
        }),
    }
}

fn harness_id(kind: HarnessKind) -> Option<HarnessId> {
    Some(match kind {
        HarnessKind::ClaudeCode => HarnessId::ClaudeCode,
        HarnessKind::Codex => HarnessId::Codex,
        HarnessKind::Cursor => HarnessId::Cursor,
        HarnessKind::Devin => HarnessId::Devin,
        HarnessKind::Grok => HarnessId::Grok,
        HarnessKind::Hermes => HarnessId::Hermes,
        HarnessKind::Pi => HarnessId::Pi,
        HarnessKind::OpenCode | HarnessKind::Kilo => HarnessId::Opencode,
        HarnessKind::Antigravity => HarnessId::Antigravity,
        _ => return None,
    })
}

/// The models the agent offers right now, from Zeron's adapter (its own
/// discovery, cache and timeouts). An adapter with no live source answers
/// with its static catalog.
pub(crate) async fn live_models(
    kind: HarnessKind,
    program: &std::path::Path,
) -> anyhow::Result<Vec<dove_proto::Model>> {
    let mut harness =
        driver_for(kind, program).ok_or_else(|| anyhow::anyhow!("{} has no shared driver", kind.label()))?;
    if kind == HarnessKind::Hermes {
        // Hermes checks its tool registry before answering; 10s is not enough.
        harness = Arc::new(
            AcpHarness::hermes()
                .with_executable(program)
                .with_model_discovery_timeout(std::time::Duration::from_secs(45)),
        );
    }
    let mut attempt = 0;
    let catalog = loop {
        match harness.model_catalog(true).await {
            Ok(catalog) => break catalog,
            // An agent that refreshes its login token on start (Grok) trips
            // Zeron's changed-credentials guard once; the retry sees a stable
            // token.
            Err(e) if attempt == 0 && e.to_string().contains("credentials changed") => attempt += 1,
            Err(e) => anyhow::bail!("{e}"),
        }
    };
    Ok(catalog.models.into_iter().map(from_zeron_model).collect())
}

fn from_zeron_model(m: zeron_proto::Model) -> dove_proto::Model {
    dove_proto::Model {
        id: m.id,
        label: m.label,
        description: m.description,
        reasoning_levels: m.reasoning_levels.into_iter().map(from_zeron_level).collect(),
        options: m
            .options
            .into_iter()
            .map(|o| dove_proto::ModelOption {
                id: o.id,
                label: o.label,
                choices: o
                    .choices
                    .into_iter()
                    .map(|c| dove_proto::ModelOptionChoice { id: c.id, label: c.label })
                    .collect(),
                default_choice: o.default_choice,
            })
            .collect(),
    }
}

fn from_zeron_level(level: zeron_proto::ReasoningLevel) -> ReasoningLevel {
    use zeron_proto::ReasoningLevel as Z;
    match level {
        Z::Minimal => ReasoningLevel::Minimal,
        Z::Low => ReasoningLevel::Low,
        Z::Medium => ReasoningLevel::Medium,
        Z::High => ReasoningLevel::High,
        Z::XHigh => ReasoningLevel::XHigh,
        Z::Max => ReasoningLevel::Max,
        Z::Ultra => ReasoningLevel::Ultra,
        Z::Ultracode => ReasoningLevel::Ultracode,
        Z::Ultrathink => ReasoningLevel::Ultrathink,
    }
}

fn reasoning(level: ReasoningLevel) -> zeron_proto::ReasoningLevel {
    match level {
        ReasoningLevel::Minimal => zeron_proto::ReasoningLevel::Minimal,
        ReasoningLevel::Low => zeron_proto::ReasoningLevel::Low,
        ReasoningLevel::Medium => zeron_proto::ReasoningLevel::Medium,
        ReasoningLevel::High => zeron_proto::ReasoningLevel::High,
        ReasoningLevel::XHigh => zeron_proto::ReasoningLevel::XHigh,
        ReasoningLevel::Max => zeron_proto::ReasoningLevel::Max,
        ReasoningLevel::Ultra => zeron_proto::ReasoningLevel::Ultra,
        ReasoningLevel::Ultracode => zeron_proto::ReasoningLevel::Ultracode,
        ReasoningLevel::Ultrathink => zeron_proto::ReasoningLevel::Ultrathink,
    }
}

/// Pick the option label Zeron's driver matches against the decision.
fn answers_for(decision: PermissionDecision, questions: &[UserInputQuestion]) -> Vec<UserInputAnswer> {
    questions
        .iter()
        .map(|q| UserInputAnswer { question_id: q.id.clone(), labels: vec![choice_label(decision, &q.options)] })
        .collect()
}

fn choice_label(decision: PermissionDecision, options: &[String]) -> String {
    let lower = |s: &String| s.to_ascii_lowercase();
    let allow = options
        .iter()
        .find(|o| lower(o) == "yes")
        .or_else(|| {
            options.iter().find(|o| {
                let l = lower(o);
                (l.contains("allow") || l.contains("accept") || l.contains("approve"))
                    && !l.contains("deny")
                    && !l.contains("reject")
            })
        })
        .or_else(|| options.first());
    let always = options.iter().find(|o| {
        let l = lower(o);
        l.contains("always") || l.contains("session")
    });
    let deny = options
        .iter()
        .find(|o| {
            let l = lower(o);
            l == "no" || l.contains("deny") || l.contains("reject") || l.contains("cancel")
        })
        .or_else(|| options.last());
    match decision {
        PermissionDecision::AllowAlways => always.or(allow).cloned().unwrap_or_else(|| "Yes".into()),
        PermissionDecision::AllowOnce => allow.cloned().unwrap_or_else(|| "Yes".into()),
        PermissionDecision::Deny => deny.cloned().unwrap_or_else(|| "No".into()),
    }
}

fn map_event(event: ZEvent, usage: &mut Usage) -> Vec<AgentEvent> {
    match event {
        ZEvent::SessionStarted { session_id, model, .. } => vec![AgentEvent::SessionStarted {
            native_session_id: (!session_id.is_empty()).then_some(session_id),
            model: (!model.is_empty()).then_some(model),
        }],
        ZEvent::TextDelta { text } => vec![AgentEvent::TextDelta { text: text.into() }],
        ZEvent::ReasoningDelta { text } => vec![AgentEvent::ThinkingDelta { text: text.into() }],
        ZEvent::AssistantMessageCompleted { .. } => vec![AgentEvent::BlockEnd],
        ZEvent::ToolCall { id, call } => tool_call(id, call),
        ZEvent::ToolResult { id, is_error, output, diff } => {
            let mut out = Vec::new();
            let text = match (output, &diff) {
                (Some(text), _) => text,
                (None, Some(diff)) => format!("updated {}", diff.path),
                (None, None) => String::new(),
            };
            if let Some(d) = diff {
                out.push(AgentEvent::ToolDiff {
                    call_id: id.clone(),
                    path: d.path,
                    old_text: d.old_text.map(|t| cap(t).into()),
                    new_text: cap(d.new_text).into(),
                });
            }
            out.push(AgentEvent::ToolResult { call_id: id, output: text.into(), is_error });
            out
        }
        ZEvent::Usage { input_tokens, output_tokens } => {
            usage.input_tokens = input_tokens;
            usage.output_tokens = output_tokens;
            Vec::new()
        }
        ZEvent::ContextUsage { tokens, window } => vec![AgentEvent::ContextUsage { tokens, window }],
        ZEvent::RateLimits { windows } => vec![AgentEvent::RateLimits {
            windows: windows
                .into_iter()
                .map(|w| RateWindow {
                    kind: w.kind,
                    used_percent: w.used_percent,
                    resets_at: w.resets_at,
                    window_minutes: w.window_minutes,
                    blocked: w.blocked,
                })
                .collect(),
        }],
        ZEvent::AvailableCommands { commands } => vec![AgentEvent::Commands {
            commands: commands.into_iter().map(|c| SlashCommand { name: c.name, description: c.description }).collect(),
        }],
        ZEvent::Error { message } => vec![AgentEvent::Error { message: message.into() }],
        ZEvent::Done { status, error, .. } => {
            let mut out = Vec::new();
            if let Some(message) = error.filter(|m| !m.is_empty()) {
                out.push(AgentEvent::Error { message: message.into() });
            }
            let turn = std::mem::take(usage);
            let reported = (turn.total_tokens() > 0).then_some(turn);
            out.push(AgentEvent::TurnEnded { usage: reported, is_error: status == DoneStatus::Errored });
            out
        }
        ZEvent::GeneratedImage { name, path, .. } => {
            vec![AgentEvent::Notice { text: format!("Image {name} · {path}").into() }]
        }
        // A subagent's own transcript, tagged with its spawning call. Its
        // usage is the parent's to report, so a scratch counter takes it.
        ZEvent::Subagent { parent_tool_use_id, event } => {
            let inner = match *event {
                ZEvent::UserMessage { text } => vec![AgentEvent::UserMessage { text: text.into() }],
                other => map_event(other, &mut Usage::default()),
            };
            inner
                .into_iter()
                .map(|e| AgentEvent::Subagent { parent: parent_tool_use_id.clone(), event: Box::new(e) })
                .collect()
        }
        // The host emits the parent's user lines.
        ZEvent::UserMessage { .. }
        | ZEvent::Steered { .. }
        | ZEvent::InputRequested { .. }
        | ZEvent::InputResolved { .. } => Vec::new(),
    }
}

/// Edit bodies ride along for the chat's mini diffs; keep them bounded.
fn cap(mut s: String) -> String {
    const MAX: usize = 16 * 1024;
    if s.len() > MAX {
        let mut end = MAX;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

fn tool_call(id: String, call: ToolCall) -> Vec<AgentEvent> {
    if let ToolCall::Todo { items } = &call {
        let mapped: Vec<TodoItem> = items
            .iter()
            .map(|item| TodoItem {
                text: item.text.clone(),
                status: match item.status {
                    Some(zeron_proto::TodoStatus::Completed) => TodoStatus::Completed,
                    Some(zeron_proto::TodoStatus::InProgress) => TodoStatus::InProgress,
                    _ if item.done => TodoStatus::Completed,
                    _ => TodoStatus::Pending,
                },
            })
            .collect();
        let text = mapped.iter().map(|i| i.text.as_str()).collect::<Vec<_>>().join("\n");
        return vec![
            AgentEvent::ToolCall {
                call_id: id,
                name: "todo".into(),
                kind: ToolKind::Other,
                title: "Plan".into(),
                input: json!({ "count": mapped.len() }),
            },
            AgentEvent::Todos { items: mapped },
            AgentEvent::Plan { text: text.into() },
        ];
    }
    let (name, kind, title, input) = match call {
        ToolCall::Exec { command } => {
            ("exec".into(), ToolKind::Execute, command.clone(), json!({ "command": command }))
        }
        ToolCall::ReadFile { path } => ("read".into(), ToolKind::Read, path.clone(), json!({ "path": path })),
        ToolCall::WriteFile { path, content } => {
            ("write".into(), ToolKind::Edit, path.clone(), json!({ "path": path, "content": content.map(cap) }))
        }
        ToolCall::EditFile { path, old_string, new_string } => (
            "edit".into(),
            ToolKind::Edit,
            path.clone(),
            json!({ "path": path, "old_string": old_string.map(cap), "new_string": new_string.map(cap) }),
        ),
        ToolCall::ApplyPatch { path } => {
            let title = path.clone().unwrap_or_else(|| "patch".into());
            ("apply_patch".into(), ToolKind::Edit, title, json!({ "path": path }))
        }
        ToolCall::Search { pattern, path } => {
            ("search".into(), ToolKind::Search, pattern.clone(), json!({ "pattern": pattern, "path": path }))
        }
        ToolCall::Glob { pattern } => ("glob".into(), ToolKind::Search, pattern.clone(), json!({ "pattern": pattern })),
        ToolCall::WebFetch { url, prompt } => {
            ("fetch".into(), ToolKind::Fetch, url.clone(), json!({ "url": url, "prompt": prompt }))
        }
        ToolCall::WebSearch { query } => {
            ("web_search".into(), ToolKind::Fetch, query.clone(), json!({ "query": query }))
        }
        ToolCall::Mcp { server, tool, input } => {
            (format!("{server}/{tool}"), ToolKind::classify(&tool), tool, input.unwrap_or(Value::Null))
        }
        ToolCall::Unknown { name, input } => {
            let kind = ToolKind::classify(&name);
            let title = name.clone();
            (name, kind, title, input.unwrap_or(Value::Null))
        }
        ToolCall::Todo { .. } => unreachable!("handled above"),
    };
    vec![AgentEvent::ToolCall { call_id: id, name, kind, title: title.into(), input }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subagent_traffic_is_tagged_not_dropped() {
        let tagged = |event| ZEvent::Subagent { parent_tool_use_id: "spawn".into(), event: Box::new(event) };
        let mut usage = Usage::default();
        let out = map_event(tagged(ZEvent::UserMessage { text: "brief".into() }), &mut usage);
        assert!(matches!(&out[..], [AgentEvent::Subagent { parent, event }]
            if parent == "spawn" && matches!(**event, AgentEvent::UserMessage { .. })));
        let out = map_event(tagged(ZEvent::TextDelta { text: "hi".into() }), &mut usage);
        assert!(matches!(&out[..], [AgentEvent::Subagent { event, .. }] if matches!(**event, AgentEvent::TextDelta { .. })));
        // The parent's own usage is untouched by a subagent's.
        map_event(tagged(ZEvent::Usage { input_tokens: 9, output_tokens: 9 }), &mut usage);
        assert_eq!(usage.total_tokens(), 0);
    }

    fn q(options: &[&str]) -> UserInputQuestion {
        UserInputQuestion {
            id: "q".into(),
            header: "Approve".into(),
            question: "Allow?".into(),
            options: options.iter().map(|s| (*s).to_string()).collect(),
            multi_select: false,
            prefill: None,
            multiline: false,
        }
    }

    #[test]
    fn codex_yes_no_and_acp_names_follow_the_decision() {
        let yes_no = q(&["Yes", "No"]);
        assert_eq!(choice_label(PermissionDecision::AllowOnce, &yes_no.options), "Yes");
        assert_eq!(choice_label(PermissionDecision::Deny, &yes_no.options), "No");
        let named = q(&["Allow once", "Allow always", "Reject"]);
        assert_eq!(choice_label(PermissionDecision::AllowAlways, &named.options), "Allow always");
        assert_eq!(choice_label(PermissionDecision::Deny, &named.options), "Reject");
    }

    #[test]
    fn permission_levels_auto_answer_the_right_approvals() {
        assert!(auto_allows(PermissionMode::Bypass, "Approve Bash"));
        assert!(auto_allows(PermissionMode::AcceptEdits, "Approve Edit"));
        assert!(auto_allows(PermissionMode::AcceptEdits, "Approve file change"));
        assert!(!auto_allows(PermissionMode::AcceptEdits, "Approve command"));
        assert!(!auto_allows(PermissionMode::Default, "Approve Edit"));
        assert!(is_approval("Approve command"));
        assert!(!is_approval("Which approach?"));
    }

    #[test]
    fn shared_agents_use_the_external_driver() {
        assert!(drives(HarnessKind::ClaudeCode));
        assert!(drives(HarnessKind::Hermes));
        assert!(drives(HarnessKind::Cursor));
        assert!(drives(HarnessKind::Kilo));
        assert_eq!(harness_id(HarnessKind::Kilo), Some(HarnessId::Opencode));
        assert!(!drives(HarnessKind::Gemini));
        assert!(!drives(HarnessKind::Kimi));
        assert!(!drives(HarnessKind::Shell));
    }
}
