//! Any agent that speaks the Agent Client Protocol (agentclientprotocol.com):
//! JSON-RPC 2.0 over stdio. One adapter covers Devin, Grok, Gemini, Copilot,
//! goose, Qwen, Droid, Hermes, Antigravity, Amp (via `amp-acp`), Kimi Code
//! and Cline. Cursor, OpenCode and Pi have their own drivers.
//!
//! Dove advertises no `fs` or `terminal` capability, so agents use their own
//! tools inside the task's working directory and Dove only observes.

use std::collections::{HashMap, VecDeque};

use dove_proto::{
    AgentEvent, Capabilities, HarnessKind, Model, ModelOption, ModelOptionChoice, PermissionDecision, PermissionMode,
    PermissionRequest, ReasoningLevel, TodoItem, TodoStatus, ToolKind, Usage,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::rpc::{self, Client, Incoming};
use crate::{Command, SessionSpec, process};

pub const CAPABILITIES: Capabilities =
    Capabilities { streaming: true, permissions: true, resume: true, interrupt: true, usage: true };

/// Arguments that start `kind` as an ACP server, for harnesses whose ACP
/// entry point is known. `None` means no ACP mode is known yet.
pub fn launch_args(kind: HarnessKind) -> Option<&'static [&'static str]> {
    Some(match kind {
        HarnessKind::Devin => &["acp"],
        // Used when Node is missing and the SDK shim cannot start. When
        // `node` is on PATH, `cursor` drives `@cursor/sdk` instead.
        HarnessKind::Cursor => &["acp"],
        HarnessKind::Cline => &["--acp"],
        HarnessKind::Kimi => &["acp"],
        // `--no-auto-update` skips the launch-time update check. `--no-leader`
        // starts a fresh agent instead of attaching to a wedged shared leader.
        HarnessKind::Grok => &["--no-auto-update", "agent", "--no-leader", "stdio"],
        HarnessKind::Gemini => &["--experimental-acp"],
        HarnessKind::Qwen => &["--experimental-acp"],
        HarnessKind::Copilot => &["--acp"],
        HarnessKind::Goose => &["acp"],
        HarnessKind::Droid => &["exec", "--output-format", "acp-daemon"],
        HarnessKind::Hermes => &["acp"],
        // Google's ACP server takes no args except an empty `--uid` on Linux,
        // which keeps a root-started process running as the invoking user.
        HarnessKind::Antigravity => {
            if cfg!(target_os = "linux") {
                &["--uid="]
            } else {
                &[]
            }
        }
        // Pi, OpenCode and (when Node is present) Cursor have native drivers.
        // Amp launches `amp-acp` rather than the `amp` binary.
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Purpose {
    Initialize,
    Session,
    Config,
    Model,
    Prompt,
}

#[derive(Default)]
pub struct Mapper {
    pub session_id: Option<String>,
    /// While `session/load` replays history, updates are not new events.
    pub replaying: bool,
    /// Cumulative cost from `usage_update`, and what was already reported.
    cost: Option<f64>,
    reported_cost: f64,
}

impl Mapper {
    pub fn update(&mut self, u: &Value, out: &mut Vec<AgentEvent>) {
        if self.replaying {
            return;
        }
        match u["sessionUpdate"].as_str().unwrap_or("") {
            "agent_message_chunk" => push_text(out, &u["content"], false),
            "agent_thought_chunk" => push_text(out, &u["content"], true),
            "tool_call" => {
                let id = u["toolCallId"].as_str().unwrap_or_default().to_string();
                let kind = tool_kind(u["kind"].as_str());
                let title = title_of(u).unwrap_or_else(|| u["kind"].as_str().unwrap_or("tool").to_string());
                out.push(AgentEvent::ToolCall {
                    call_id: id,
                    name: u["kind"].as_str().unwrap_or("tool").into(),
                    kind,
                    title: title.into(),
                    input: u["rawInput"].clone(),
                });
                // Some agents report a finished call in one message.
                self.finish_if_done(u, out);
            }
            "tool_call_update" => {
                let id = u["toolCallId"].as_str().unwrap_or_default().to_string();
                if let Some(title) = title_of(u) {
                    out.push(AgentEvent::ToolTitle { call_id: id, title: title.into() });
                }
                self.finish_if_done(u, out);
            }
            "plan" => {
                let items = u["entries"]
                    .as_array()
                    .map(|e| {
                        e.iter()
                            .filter_map(|e| {
                                Some(TodoItem {
                                    text: e["content"].as_str()?.to_string(),
                                    status: TodoStatus::parse(e["status"].as_str().unwrap_or("")),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                out.push(AgentEvent::Todos { items });
            }
            "usage_update" => {
                if let Some(c) = u["cost"]["amount"].as_f64() {
                    self.cost = Some(c);
                }
                let (used, size) = (u["used"].as_u64(), u["size"].as_u64());
                if used.is_some() || size.is_some() {
                    out.push(AgentEvent::ContextUsage { tokens: used, window: size.filter(|s| *s > 0) });
                }
            }
            "available_commands_update" => {
                let commands = u["availableCommands"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| {
                                Some(dove_proto::SlashCommand {
                                    name: c["name"].as_str()?.to_string(),
                                    description: c["description"].as_str().unwrap_or_default().to_string(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                out.push(AgentEvent::Commands { commands });
            }
            _ => {}
        }
    }

    fn finish_if_done(&self, u: &Value, out: &mut Vec<AgentEvent>) {
        let status = u["status"].as_str().unwrap_or("");
        if status != "completed" && status != "failed" {
            return;
        }
        for c in u["content"].as_array().into_iter().flatten().filter(|c| c["type"] == "diff") {
            if let (Some(path), Some(new)) = (c["path"].as_str(), c["newText"].as_str()) {
                out.push(AgentEvent::ToolDiff {
                    call_id: u["toolCallId"].as_str().unwrap_or_default().to_string(),
                    path: path.to_string(),
                    old_text: c["oldText"].as_str().map(Into::into),
                    new_text: new.into(),
                });
            }
        }
        let text: Vec<&str> = u["content"]
            .as_array()
            .map(|c| c.iter().filter_map(|c| c["content"]["text"].as_str()).collect())
            .unwrap_or_default();
        let output =
            if text.is_empty() { u["rawOutput"]["output"].as_str().unwrap_or("").to_string() } else { text.join("\n") };
        out.push(AgentEvent::ToolResult {
            call_id: u["toolCallId"].as_str().unwrap_or_default().to_string(),
            output: output.into(),
            is_error: status == "failed",
        });
    }

    /// The `session/prompt` result ends the turn.
    pub fn prompt_done(&mut self, r: &Value, out: &mut Vec<AgentEvent>) {
        let u = &r["usage"];
        let cost = self.cost.map(|c| {
            let d = (c - self.reported_cost).max(0.0);
            self.reported_cost = c;
            d
        });
        let usage = Usage {
            input_tokens: u["inputTokens"].as_u64().unwrap_or(0),
            output_tokens: u["outputTokens"].as_u64().unwrap_or(0),
            cache_read_tokens: u["cachedReadTokens"].as_u64().unwrap_or(0),
            cache_write_tokens: u["cachedWriteTokens"].as_u64().unwrap_or(0),
            cost_usd: cost.filter(|c| *c > 0.0),
            estimated: false,
        };
        match r["stopReason"].as_str() {
            Some("cancelled") => out.push(AgentEvent::Notice { text: "Interrupted".into() }),
            Some("refusal") => out.push(AgentEvent::Error { message: "The agent refused this request".into() }),
            Some("max_tokens") | Some("max_turn_requests") => {
                out.push(AgentEvent::Notice { text: "Stopped at the agent's limit".into() })
            }
            _ => {}
        }
        out.push(AgentEvent::BlockEnd);
        out.push(AgentEvent::TurnEnded { usage: Some(usage), is_error: false });
    }
}

fn title_of(u: &Value) -> Option<String> {
    let t = u["title"].as_str().filter(|t| !t.is_empty())?;
    // A bare tool name ("write") is less useful than its target path.
    let path = u["locations"].as_array().and_then(|l| l.first()).and_then(|l| l["path"].as_str());
    Some(match path {
        Some(p) if !t.contains(['/', '\\', ' ']) => {
            let name = p.rsplit(['/', '\\']).next().unwrap_or(p);
            format!("{t} {name}")
        }
        _ => t.to_string(),
    })
}

fn push_text(out: &mut Vec<AgentEvent>, content: &Value, thinking: bool) {
    let Some(t) = content["text"].as_str().filter(|t| !t.is_empty()) else { return };
    out.push(if thinking {
        AgentEvent::ThinkingDelta { text: t.into() }
    } else {
        AgentEvent::TextDelta { text: t.into() }
    });
}

fn tool_kind(k: Option<&str>) -> ToolKind {
    match k.unwrap_or("") {
        "read" => ToolKind::Read,
        "edit" | "delete" | "move" => ToolKind::Edit,
        "execute" => ToolKind::Execute,
        "search" => ToolKind::Search,
        "fetch" => ToolKind::Fetch,
        "think" => ToolKind::Think,
        _ => ToolKind::Other,
    }
}

/// One model or trait write to send after `session/new`, in order.
#[derive(Debug, Clone, PartialEq)]
enum WireSetup {
    SetModel(String),
    Config { config_id: String, payload: Value },
}

/// Model, effort, context window and speed, matched against what this session
/// actually advertises. Unknown choices are skipped so a curated catalog row
/// never forces an id the agent doesn't have.
fn plan_setup(spec: &SessionSpec, session: &Value) -> Vec<WireSetup> {
    let context_1m =
        crate::models::option_value(&spec.model_options, "contextWindow").is_some_and(|w| w.eq_ignore_ascii_case("1m"));
    let requested = spec.model.as_deref().map(|model| {
        if spec.kind == HarnessKind::Antigravity {
            antigravity_model(session, model, spec.reasoning)
        } else {
            model.to_string()
        }
    });
    let has_model_config = config_options(session).iter().any(|o| o["type"] == "select" && o["category"] == "model");
    let mut out = Vec::new();
    if !has_model_config
        && let Some(model) = requested.as_deref()
        && let Some(picked) = pick_model_value(model, &first_class_models(session), context_1m)
        && session["models"]["currentModelId"].as_str() != Some(picked.as_str())
    {
        out.push(WireSetup::SetModel(picked));
    }
    out.extend(config_sets(
        session,
        requested.as_deref(),
        &effort_values(spec.reasoning),
        &spec.model_options,
        context_1m,
    ));
    out.extend(plan_mode(spec, session));
    out
}

/// Plan mode for agents that expose it as a `mode` select (Kilo, OpenCode
/// forks) rather than through Dove's permission prompts.
fn plan_mode(spec: &SessionSpec, session: &Value) -> Option<WireSetup> {
    if spec.permission_mode != dove_proto::PermissionMode::Plan {
        return None;
    }
    let option = config_options(session).iter().find(|o| o["type"] == "select" && o["category"] == "mode")?;
    if option["currentValue"] == "plan" || !option_values(option).contains(&"plan") {
        return None;
    }
    Some(WireSetup::Config { config_id: option["id"].as_str()?.to_string(), payload: json!({"value": "plan"}) })
}

/// A model switch can change the effort ladder and other traits (Kilo, for
/// one, advertises effort per model). When the write answers with fresh
/// `configOptions`, plan the remaining traits against those instead of the
/// options the previous model had.
fn replan_after_model(spec: &SessionSpec, done: Option<&WireSetup>, response: &Value) -> Option<Vec<WireSetup>> {
    let Some(WireSetup::Config { config_id, .. }) = done else { return None };
    let switched_model =
        config_options(response).iter().any(|o| o["id"] == config_id.as_str() && o["category"] == "model");
    if !switched_model {
        return None;
    }
    let context_1m =
        crate::models::option_value(&spec.model_options, "contextWindow").is_some_and(|w| w.eq_ignore_ascii_case("1m"));
    let mut steps = config_sets(response, None, &effort_values(spec.reasoning), &spec.model_options, context_1m);
    steps.extend(plan_mode(spec, response));
    Some(steps)
}

fn config_options(session: &Value) -> &[Value] {
    session["configOptions"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn first_class_models(session: &Value) -> Vec<&str> {
    session["models"]["availableModels"]
        .as_array()
        .map(|m| m.iter().filter_map(|m| m["modelId"].as_str()).collect())
        .unwrap_or_default()
}

fn option_values(option: &Value) -> Vec<&str> {
    option["options"].as_array().map(|a| a.iter().filter_map(|o| o["value"].as_str()).collect()).unwrap_or_default()
}

fn antigravity_model(session: &Value, model: &str, reasoning: Option<ReasoningLevel>) -> String {
    let values: Vec<&str> =
        config_options(session).iter().find(|o| o["category"] == "model").map(option_values).unwrap_or_default();
    if values.is_empty() || values.iter().any(|v| *v == model) {
        return model.to_string();
    }
    if let Some(level) = reasoning {
        let composed = format!("{model}-{}", level.id());
        if values.iter().any(|v| *v == composed) {
            return composed;
        }
    }
    values.iter().find(|v| v.starts_with(model)).map(|v| (*v).to_string()).unwrap_or_else(|| model.to_string())
}

fn effort_values(level: Option<ReasoningLevel>) -> Vec<&'static str> {
    match level {
        None => vec![],
        Some(ReasoningLevel::Minimal) => vec!["none", "minimal", "low"],
        Some(ReasoningLevel::Low) => vec!["low", "minimal"],
        Some(ReasoningLevel::Medium) => vec!["medium"],
        Some(ReasoningLevel::High) => vec!["high"],
        Some(ReasoningLevel::XHigh) => vec!["xhigh", "x-high", "high"],
        Some(ReasoningLevel::Max) => vec!["max", "xhigh", "high"],
        Some(ReasoningLevel::Ultra | ReasoningLevel::Ultracode | ReasoningLevel::Ultrathink) => {
            vec!["ultra", "max", "high"]
        }
    }
}

fn pick_model_value(requested: &str, available: &[&str], context_1m: bool) -> Option<String> {
    if available.is_empty() {
        return None;
    }
    if context_1m {
        for composed in [format!("{requested}[1m]"), format!("{requested}-1m")] {
            if available.contains(&composed.as_str()) {
                return Some(composed);
            }
        }
    }
    if available.contains(&requested) {
        return Some(requested.to_string());
    }
    let family = ["fable", "opus", "sonnet", "haiku", "gpt"].into_iter().find(|f| norm_id(requested).contains(f))?;
    let candidates: Vec<&&str> = available.iter().filter(|v| norm_id(v).contains(family)).collect();
    candidates
        .iter()
        .find(|v| context_hint_1m(v) == context_1m)
        .or_else(|| candidates.first())
        .map(|v| (**v).to_string())
}

fn norm_id(id: &str) -> String {
    id.chars().filter(|c| c.is_ascii_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

fn context_hint_1m(id: &str) -> bool {
    id.contains("[1m]") || id.contains("-1m") || norm_id(id).ends_with("1m")
}

fn config_sets(
    session: &Value,
    model: Option<&str>,
    efforts: &[&str],
    model_options: &[(String, String)],
    context_1m: bool,
) -> Vec<WireSetup> {
    let mut sets = Vec::new();
    for option in config_options(session) {
        let Some(config_id) = option["id"].as_str() else { continue };
        let kind = option["type"].as_str().unwrap_or("");
        let category = option["category"].as_str();
        let available = option_values(option);
        let wanted = match (kind, category) {
            ("select", Some("model")) => model
                .and_then(|m| pick_model_value(m, &available, context_1m))
                // amp-acp's "model" select is its effort mode (low … ultra).
                .or_else(|| efforts.iter().find(|c| available.contains(c)).map(|c| (*c).to_string()))
                .map(Value::String),
            ("select", Some("thought_level")) => {
                efforts.iter().find(|c| available.contains(c)).map(|c| Value::String((*c).to_string()))
            }
            _ => model_options.iter().find_map(|(opt_id, choice)| {
                if norm_id(opt_id) != norm_id(config_id) || opt_id == "contextWindow" {
                    return None;
                }
                match kind {
                    "select" if available.iter().any(|v| v == choice) => Some(Value::String(choice.clone())),
                    "boolean" => Some(Value::Bool(choice == "on" || choice == "true")),
                    _ => None,
                }
            }),
        };
        let Some(value) = wanted else { continue };
        if option.get("currentValue") == Some(&value) {
            continue;
        }
        let payload = match value {
            Value::Bool(b) => json!({"type": "boolean", "value": b}),
            other => json!({"value": other}),
        };
        sets.push(WireSetup::Config { config_id: config_id.to_string(), payload });
    }
    sets
}

/// The permission request as an event, plus the options to pick from later.
fn permission(id: &Value, p: &Value) -> (PermissionRequest, Vec<AcpOption>) {
    let call = &p["toolCall"];
    let title = call["title"].as_str().unwrap_or("Run a tool").to_string();
    let options: Vec<AcpOption> = p["options"]
        .as_array()
        .map(|o| {
            o.iter()
                .filter_map(|o| {
                    Some(AcpOption {
                        id: o["optionId"].as_str()?.to_string(),
                        kind: o["kind"].as_str().unwrap_or("").to_string(),
                        name: o["name"].as_str().unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let request_id = match id {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    // Options without allow/reject kinds are the agent asking the user.
    let question = options
        .iter()
        .any(|o| !matches!(o.kind.as_str(), "allow_once" | "allow_always" | "reject_once" | "reject_always"));
    let questions = if question {
        vec![dove_proto::Question {
            id: request_id.clone(),
            header: "Question".into(),
            question: title.clone(),
            options: options.iter().map(|o| o.name.clone()).collect(),
            multi_select: false,
        }]
    } else {
        Vec::new()
    };
    let req = PermissionRequest {
        request_id,
        tool: call["kind"].as_str().unwrap_or("tool").to_string(),
        summary: title.into(),
        input: call["rawInput"].clone(),
        questions,
    };
    (req, options)
}

#[derive(Debug, Clone)]
pub struct AcpOption {
    pub id: String,
    pub kind: String,
    pub name: String,
}

/// Whether the permission level answers this approval without asking.
fn auto_allowed(mode: PermissionMode, req: &PermissionRequest) -> bool {
    !req.is_question()
        && (crate::is_dove_tool(&req.tool)
            || crate::is_dove_tool(&req.summary)
            || match mode {
            PermissionMode::Bypass => true,
            PermissionMode::AcceptEdits => req.tool == "edit",
            PermissionMode::Default | PermissionMode::Plan => false,
        })
}

/// Pick the option matching `decision`, falling back to the nearest kind.
pub fn choose(options: &[AcpOption], decision: PermissionDecision) -> Value {
    let prefs: &[&str] = match decision {
        PermissionDecision::AllowOnce => &["allow_once", "allow_always"],
        PermissionDecision::AllowAlways => &["allow_always", "allow_once"],
        PermissionDecision::Deny => &["reject_once", "reject_always"],
    };
    for kind in prefs {
        if let Some(o) = options.iter().find(|o| o.kind == *kind) {
            return json!({"outcome": {"outcome": "selected", "optionId": o.id}});
        }
    }
    json!({"outcome": {"outcome": "cancelled"}})
}

/// Amp's product CLI is a TUI. The ACP server is the `amp-acp` package, which
/// needs `AMP_CLI_PATH` pointing at the installed `amp` binary.
fn launch_command(spec: &SessionSpec) -> Result<(std::path::PathBuf, Vec<String>), String> {
    if spec.kind == HarnessKind::Amp {
        let npx =
            process::resolve("npx").ok_or_else(|| "Amp needs Node.js (`npx`) so Dove can start amp-acp".to_string())?;
        return Ok((npx, vec!["-y".into(), "amp-acp".into()]));
    }
    let args = launch_args(spec.kind).ok_or_else(|| format!("{} has no known ACP mode", spec.kind.label()))?;
    Ok((spec.program.clone(), args.iter().map(|s| (*s).to_string()).collect()))
}

pub(crate) async fn run(spec: SessionSpec, events: flume::Sender<AgentEvent>, commands: flume::Receiver<Command>) {
    let fail = |msg: String| {
        let _ = events.send(AgentEvent::Error { message: msg.into() });
        let _ = events.send(AgentEvent::Exited { code: None });
    };
    let (program, args) = match launch_command(&spec) {
        Ok(pair) => pair,
        Err(msg) => return fail(msg),
    };
    let mut cmd = process::command(&program, &args, &spec.cwd);
    if spec.kind == HarnessKind::Amp {
        cmd.env("AMP_CLI_PATH", &spec.program);
    }
    let mut child = match process::spawn(cmd, &program) {
        Ok(c) => c,
        Err(e) => return fail(format!("{e:#}")),
    };
    let mut client: Client<Purpose> = Client::new(child.stdin.take().expect("piped stdin"), true);
    let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
    let mut stderr = child.stderr.take().expect("piped stderr");
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let _ = stderr.read_to_string(&mut buf).await;
        buf
    });

    let mut mapper = Mapper::default();
    let mut batch = Vec::new();
    let mut queued: Vec<String> = spec.prompt.iter().cloned().collect();
    let mut setup: VecDeque<WireSetup> = VecDeque::new();
    let mut prompting = false;
    let mut can_load = false;
    let mut approvals: HashMap<String, (Value, Vec<AcpOption>)> = HashMap::new();
    let cwd = spec.cwd.to_string_lossy().to_string();

    let _ = client
        .request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
                "clientInfo": {"name": "dove", "title": "Dove", "version": env!("CARGO_PKG_VERSION")},
            }),
            Purpose::Initialize,
        )
        .await;

    loop {
        tokio::select! {
            line = stdout.next_line() => {
                let Ok(Some(line)) = line else { break };
                let Some(msg) = rpc::decode(&line) else { continue };
                match msg {
                    Incoming::Response { id, result } => match (client.take(id), result) {
                        (Some(Purpose::Initialize), Ok(r)) => {
                            can_load = r["agentCapabilities"]["loadSession"].as_bool().unwrap_or(false);
                            match (&spec.resume, can_load) {
                                (Some(sid), true) => {
                                    mapper.replaying = true;
                                    mapper.session_id = Some(sid.clone());
                                    let _ = client.request("session/load", json!({"sessionId": sid, "cwd": cwd, "mcpServers": crate::acp_mcp_servers(&spec.cwd)}), Purpose::Session).await;
                                }
                                _ => {
                                    let _ = client.request("session/new", json!({"cwd": cwd, "mcpServers": crate::acp_mcp_servers(&spec.cwd)}), Purpose::Session).await;
                                }
                            }
                        }
                        (Some(Purpose::Session), Ok(r)) => {
                            mapper.replaying = false;
                            if let Some(s) = r["sessionId"].as_str() {
                                mapper.session_id = Some(s.to_string());
                            }
                            let _ = events.send(AgentEvent::SessionStarted {
                                native_session_id: mapper.session_id.clone(),
                                model: spec.model.clone(),
                            });
                            setup = plan_setup(&spec, &r).into();
                            if let Some(sid) = mapper.session_id.clone() {
                                advance_setup(&mut client, &mut setup, &sid, &mut queued, &mut prompting, &mapper, &events).await;
                            }
                        }
                        (Some(Purpose::Config | Purpose::Model), result) => {
                            let done = setup.pop_front();
                            match result {
                                Err(e) => {
                                    let _ = events.send(AgentEvent::Notice {
                                        text: format!("Model setting was not applied: {e}").into(),
                                    });
                                }
                                Ok(r) => {
                                    if let Some(steps) = replan_after_model(&spec, done.as_ref(), &r) {
                                        setup = steps.into();
                                    }
                                }
                            }
                            if let Some(sid) = mapper.session_id.clone() {
                                advance_setup(&mut client, &mut setup, &sid, &mut queued, &mut prompting, &mapper, &events).await;
                            }
                        }
                        (Some(Purpose::Prompt), Ok(r)) => {
                            prompting = false;
                            mapper.prompt_done(&r, &mut batch);
                            for e in batch.drain(..) { let _ = events.send(e); }
                            if let Some(text) = (!queued.is_empty()).then(|| queued.remove(0)) {
                                prompting = prompt(&mut client, &mapper, &text, &events).await;
                            }
                        }
                        (Some(p), Err(e)) => {
                            let hint = if e.to_lowercase().contains("auth") {
                                format!(" — sign in with `{}` in a terminal first", spec.program.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default())
                            } else { String::new() };
                            let _ = events.send(AgentEvent::Error { message: format!("{e}{hint}").into() });
                            match p {
                                Purpose::Prompt => {
                                    prompting = false;
                                    let _ = events.send(AgentEvent::TurnEnded { usage: None, is_error: true });
                                }
                                Purpose::Session if spec.resume.is_some() && mapper.replaying => {
                                    // History could not be loaded: start fresh rather than fail.
                                    mapper.replaying = false;
                                    let _ = client.request("session/new", json!({"cwd": cwd, "mcpServers": crate::acp_mcp_servers(&spec.cwd)}), Purpose::Session).await;
                                }
                                _ => { let _ = child.start_kill(); }
                            }
                        }
                        _ => {}
                    },
                    Incoming::Request { id, method, params } => match method.as_str() {
                        "session/request_permission" => {
                            let (req, options) = permission(&id, &params);
                            if auto_allowed(spec.permission_mode, &req) {
                                let _ = client.respond(id, choose(&options, PermissionDecision::AllowOnce)).await;
                            } else {
                                approvals.insert(req.request_id.clone(), (id, options));
                                let _ = events.send(AgentEvent::PermissionRequested(req));
                            }
                        }
                        _ => { let _ = client.respond_error(id, -32601, "not supported by Dove").await; }
                    },
                    Incoming::Notification { method, params } => {
                        if method == "session/update" {
                            mapper.update(&params["update"], &mut batch);
                            for e in batch.drain(..) { let _ = events.send(e); }
                        }
                    }
                }
            }
            cmd = commands.recv_async() => match cmd {
                Ok(Command::Send(text)) => {
                    if mapper.session_id.is_some() && !prompting && !mapper.replaying && setup.is_empty() {
                        prompting = prompt(&mut client, &mapper, &text, &events).await;
                    } else {
                        queued.push(text);
                    }
                }
                Ok(Command::Interrupt) => {
                    if let Some(sid) = &mapper.session_id {
                        let _ = client.notify("session/cancel", Some(json!({"sessionId": sid}))).await;
                    }
                    // Pending permission prompts must be answered as cancelled.
                    for (rid, (id, _)) in approvals.drain() {
                        let _ = client.respond(id, json!({"outcome": {"outcome": "cancelled"}})).await;
                        let _ = events.send(AgentEvent::PermissionResolved { request_id: rid, decision: PermissionDecision::Deny });
                    }
                }
                Ok(Command::Permission { request_id, decision }) => {
                    if let Some((id, options)) = approvals.remove(&request_id) {
                        let _ = client.respond(id, choose(&options, decision)).await;
                        let _ = events.send(AgentEvent::PermissionResolved { request_id, decision });
                    }
                }
                Ok(Command::Answer { request_id, answers }) => {
                    if let Some((id, options)) = approvals.remove(&request_id) {
                        let label = answers.first().and_then(|a| a.first()).cloned().unwrap_or_default();
                        let reply = match options.iter().find(|o| o.name == label) {
                            Some(o) => json!({"outcome": {"outcome": "selected", "optionId": o.id}}),
                            None => json!({"outcome": {"outcome": "cancelled"}}),
                        };
                        let _ = client.respond(id, reply).await;
                        let _ = events.send(AgentEvent::PermissionResolved {
                            request_id,
                            decision: PermissionDecision::AllowOnce,
                        });
                    }
                }
                Ok(Command::Kill) | Err(_) => {
                    let _ = child.start_kill();
                    break;
                }
            },
        }
    }

    let _ = can_load;
    let code = child.wait().await.ok().and_then(|s| s.code());
    if code.is_some_and(|c| c != 0) {
        if let Ok(err) = stderr_task.await {
            let tail: Vec<&str> = err.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect();
            if !tail.is_empty() {
                let _ = events.send(AgentEvent::Error { message: tail.join("\n").into() });
            }
        }
    }
    let _ = events.send(AgentEvent::Exited { code });
}

async fn advance_setup(
    client: &mut Client<Purpose>,
    setup: &mut VecDeque<WireSetup>,
    session_id: &str,
    queued: &mut Vec<String>,
    prompting: &mut bool,
    mapper: &Mapper,
    events: &flume::Sender<AgentEvent>,
) {
    while let Some(step) = setup.front().cloned() {
        let sent = match &step {
            WireSetup::SetModel(model) => client
                .request("session/set_model", json!({"sessionId": session_id, "modelId": model}), Purpose::Model)
                .await
                .is_ok(),
            WireSetup::Config { config_id, payload } => {
                let mut params = json!({"sessionId": session_id, "configId": config_id});
                if let Some(obj) = payload.as_object() {
                    for (k, v) in obj {
                        params[k] = v.clone();
                    }
                }
                client.request("session/set_config_option", params, Purpose::Config).await.is_ok()
            }
        };
        if sent {
            return;
        }
        setup.pop_front();
    }
    if !*prompting && let Some(text) = (!queued.is_empty()).then(|| queued.remove(0)) {
        *prompting = prompt(client, mapper, &text, events).await;
    }
}

async fn prompt(client: &mut Client<Purpose>, mapper: &Mapper, text: &str, events: &flume::Sender<AgentEvent>) -> bool {
    let Some(sid) = &mapper.session_id else { return false };
    let _ = events.send(AgentEvent::UserMessage { text: text.into() });
    client
        .request(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type": "text", "text": text}]}),
            Purpose::Prompt,
        )
        .await
        .is_ok()
}

// ------------------------------------------------------------ model discovery

/// The models an ACP agent advertises, from one throwaway `session/new`.
/// Read-only: nothing is switched, so the agent's own default stays put.
/// Effort ladders come from the curated row when the id is known, else from
/// the session's `thought_level` select; other select options pass through
/// as traits. Same rules as Zeron's `models_from_session`.
pub(crate) async fn discover_models(kind: HarnessKind, program: &std::path::Path) -> anyhow::Result<Vec<Model>> {
    let spec = SessionSpec {
        kind,
        program: program.to_path_buf(),
        cwd: home_dir(),
        model: None,
        reasoning: None,
        model_options: Vec::new(),
        permission_mode: PermissionMode::Default,
        resume: None,
        prompt: None,
        fork: false,
    };
    let (exe, args) = launch_command(&spec).map_err(anyhow::Error::msg)?;
    let mut cmd = process::command(&exe, &args, &spec.cwd);
    if kind == HarnessKind::Amp {
        cmd.env("AMP_CLI_PATH", program);
    }
    let mut child = process::spawn(cmd, &exe)?;
    let mut client: Client<()> = Client::new(child.stdin.take().expect("piped stdin"), true);
    let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
    let cwd = spec.cwd.to_string_lossy().to_string();
    let session = async {
        client
            .request(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
                    "clientInfo": {"name": "dove", "title": "Dove", "version": env!("CARGO_PKG_VERSION")},
                }),
                (),
            )
            .await?;
        let mut asked = false;
        while let Some(line) = stdout.next_line().await? {
            let Some(Incoming::Response { result, .. }) = rpc::decode(&line) else { continue };
            let result = result.map_err(anyhow::Error::msg)?;
            if !asked {
                client.request("session/new", json!({"cwd": cwd, "mcpServers": []}), ()).await?;
                asked = true;
            } else {
                return Ok(result);
            }
        }
        anyhow::bail!("{} exited before advertising its models", kind.label())
    };
    // Amp's first run fetches amp-acp through npx, which can take a while.
    let result = tokio::time::timeout(std::time::Duration::from_secs(75), session).await;
    let _ = child.start_kill();
    let session = result.map_err(|_| anyhow::anyhow!("{} model discovery timed out", kind.label()))??;
    Ok(models_from_session(&session, &crate::models::catalog(kind)))
}

fn home_dir() -> std::path::PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

fn models_from_session(session: &Value, curated: &[Model]) -> Vec<Model> {
    let options = config_options(session);
    let ladder: Vec<ReasoningLevel> = options
        .iter()
        .find(|o| o["category"] == "thought_level")
        .map(|o| option_values(o).into_iter().filter_map(ReasoningLevel::from_id).collect())
        .unwrap_or_default();
    let traits: Vec<ModelOption> = options.iter().filter_map(trait_from_option).collect();
    let known = |id: &str| curated.iter().find(|m| norm_id(&m.id) == norm_id(id));
    let build = |id: &str, name: Option<&str>, description: Option<&str>, mut options: Vec<ModelOption>| {
        let known = known(id);
        if options.is_empty()
            && let Some(m) = known
        {
            options = m.options.clone();
        }
        Model {
            id: id.to_string(),
            label: name
                .map(str::to_string)
                .or_else(|| known.map(|m| m.label.clone()))
                .unwrap_or_else(|| id.to_string()),
            description: description.map(str::to_string).or_else(|| known.and_then(|m| m.description.clone())),
            reasoning_levels: known
                .filter(|m| !m.reasoning_levels.is_empty())
                .map(|m| m.reasoning_levels.clone())
                .unwrap_or_else(|| ladder.clone()),
            options,
        }
    };

    let select: Vec<&Value> = options
        .iter()
        .find(|o| o["type"] == "select" && o["category"] == "model")
        .and_then(|o| o["options"].as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    if !select.is_empty() {
        let ids: Vec<&str> = select.iter().filter_map(|o| o["value"].as_str()).collect();
        // Every "model" is an effort level (amp-acp): one model whose ladder
        // they are, so the slider sets the mode.
        let levels: Vec<ReasoningLevel> = ids.iter().filter_map(|id| ReasoningLevel::from_id(id)).collect();
        if levels.len() == ids.len() {
            return vec![Model {
                id: "default".into(),
                label: "Default".into(),
                description: Some("The agent picks the model; effort sets its mode".into()),
                reasoning_levels: levels,
                options: traits.clone(),
            }];
        }
        let has_real = ids.iter().any(|id| norm_id(id) != "default");
        return select
            .iter()
            .filter_map(|o| {
                let id = o["value"].as_str()?;
                if has_real && norm_id(id) == "default" {
                    return None;
                }
                let mut traits = traits.clone();
                if let Some(base) = strip_context_hint(id) {
                    // A 1M twin of an advertised base folds into its row.
                    if ids.contains(&base) {
                        return None;
                    }
                    let mut window = crate::models::context_window_option();
                    window.default_choice = "1m".into();
                    traits.push(window);
                    return Some(build(base, o["name"].as_str(), o["description"].as_str(), traits));
                }
                if ids.iter().any(|raw| strip_context_hint(raw) == Some(id)) {
                    traits.push(crate::models::context_window_option());
                }
                Some(build(id, o["name"].as_str(), o["description"].as_str(), traits))
            })
            .collect();
    }
    session["models"]["availableModels"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    let id = m["modelId"].as_str()?;
                    Some(build(id, m["name"].as_str(), m["description"].as_str(), Vec::new()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `claude-opus-5-5[1m]` / `…-1m` → `claude-opus-5-5`.
fn strip_context_hint(id: &str) -> Option<&str> {
    id.strip_suffix("[1m]").or_else(|| id.strip_suffix("-1m"))
}

/// A config option shown as a trait: anything but mode, model and effort.
fn trait_from_option(option: &Value) -> Option<ModelOption> {
    if matches!(option["category"].as_str(), Some("mode" | "model" | "thought_level")) {
        return None;
    }
    let id = option["id"].as_str()?;
    let label = option["name"].as_str().unwrap_or(id).to_string();
    let (choices, default_choice) = match option["type"].as_str()? {
        "select" => {
            let choices: Vec<ModelOptionChoice> = option["options"]
                .as_array()?
                .iter()
                .filter_map(|c| {
                    let id = c["value"].as_str()?;
                    Some(ModelOptionChoice { id: id.into(), label: c["name"].as_str().unwrap_or(id).into() })
                })
                .collect();
            let current = option["currentValue"].as_str().map(str::to_string);
            let default = current
                .filter(|v| choices.iter().any(|c| &c.id == v))
                .or_else(|| choices.first().map(|c| c.id.clone()))?;
            (choices, default)
        }
        "boolean" => {
            let on = option["currentValue"].as_bool().unwrap_or(false);
            (
                vec![
                    ModelOptionChoice { id: "off".into(), label: "Off".into() },
                    ModelOptionChoice { id: "on".into(), label: "On".into() },
                ],
                if on { "on".into() } else { "off".into() },
            )
        }
        _ => return None,
    };
    (choices.len() > 1).then(|| ModelOption { id: id.into(), label, choices, default_choice })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = include_str!("../fixtures/acp/opencode.log");

    #[test]
    fn opencode_transcript_maps_to_events() {
        let mut m = Mapper::default();
        let mut out = Vec::new();
        for line in LOG.lines().filter_map(|l| l.strip_prefix("<< ")) {
            match rpc::decode(line) {
                Some(Incoming::Notification { method, params }) if method == "session/update" => {
                    m.update(&params["update"], &mut out)
                }
                Some(Incoming::Response { result: Ok(r), .. }) if r.get("stopReason").is_some() => {
                    m.prompt_done(&r, &mut out)
                }
                _ => {}
            }
        }
        assert!(out.iter().any(|e| matches!(e, AgentEvent::TextDelta { .. })));
        assert!(out.iter().any(|e| matches!(e, AgentEvent::ThinkingDelta { .. })));
        let call = out
            .iter()
            .find_map(|e| match e {
                AgentEvent::ToolCall { call_id, kind: ToolKind::Edit, .. } => Some(call_id.clone()),
                _ => None,
            })
            .expect("edit tool call");
        assert!(
            out.iter()
                .any(|e| matches!(e, AgentEvent::ToolResult { call_id, is_error: false, .. } if *call_id == call))
        );
        assert!(out.iter().any(|e| matches!(e, AgentEvent::ToolTitle { title, .. } if title.contains("hello.txt"))));
        assert!(matches!(out.last(), Some(AgentEvent::TurnEnded { usage: Some(u), .. }) if u.output_tokens > 0));
    }

    #[test]
    fn setup_applies_advertised_model_effort_and_speed() {
        let session = json!({
            "configOptions": [
                {"id": "model", "type": "select", "category": "model", "currentValue": "claude-haiku-4-5",
                 "options": [{"value": "claude-opus-5-5"}, {"value": "claude-opus-5-5[1m]"}, {"value": "claude-haiku-4-5"}]},
                {"id": "thought", "type": "select", "category": "thought_level", "currentValue": "low",
                 "options": [{"value": "low"}, {"value": "high"}, {"value": "xhigh"}]},
                {"id": "speed", "type": "select", "category": "speed", "currentValue": "standard",
                 "options": [{"value": "standard"}, {"value": "fast"}]}
            ]
        });
        let spec = SessionSpec {
            kind: HarnessKind::Cursor,
            program: Default::default(),
            cwd: Default::default(),
            model: Some("claude-opus-5-5".into()),
            reasoning: Some(ReasoningLevel::High),
            model_options: vec![("contextWindow".into(), "1m".into()), ("speed".into(), "fast".into())],
            permission_mode: dove_proto::PermissionMode::Default,
            resume: None,
            prompt: None,
            fork: false,
        };
        let steps = plan_setup(&spec, &session);
        assert_eq!(
            steps,
            vec![
                WireSetup::Config { config_id: "model".into(), payload: json!({"value": "claude-opus-5-5[1m]"}) },
                WireSetup::Config { config_id: "thought".into(), payload: json!({"value": "high"}) },
                WireSetup::Config { config_id: "speed".into(), payload: json!({"value": "fast"}) },
            ]
        );
    }

    #[test]
    fn grok_uses_first_class_model_and_hermes_launches() {
        let session = json!({"models": {"currentModelId": "grok-3", "availableModels": [{"modelId": "grok-4.5"}, {"modelId": "grok-3"}]}});
        let spec = SessionSpec {
            kind: HarnessKind::Grok,
            program: Default::default(),
            cwd: Default::default(),
            model: Some("grok-4.5".into()),
            reasoning: Some(ReasoningLevel::High),
            model_options: vec![],
            permission_mode: dove_proto::PermissionMode::Default,
            resume: None,
            prompt: None,
            fork: false,
        };
        assert_eq!(plan_setup(&spec, &session), vec![WireSetup::SetModel("grok-4.5".into())]);
        assert_eq!(launch_args(HarnessKind::Hermes), Some(&["acp"][..]));
        assert_eq!(launch_args(HarnessKind::Grok), Some(&["--no-auto-update", "agent", "--no-leader", "stdio"][..]));
        assert!(launch_args(HarnessKind::Amp).is_none());
        assert!(launch_args(HarnessKind::Pi).is_none());
        assert!(launch_args(HarnessKind::OpenCode).is_none());
        assert_eq!(launch_args(HarnessKind::Cline), Some(&["--acp"][..]));
        assert_eq!(launch_args(HarnessKind::Kimi), Some(&["acp"][..]));
    }

    /// Shaped like `kilo acp` 7.4: model, a per-model effort select, and a
    /// mode select that carries plan mode.
    fn kilo_options(model: &str, efforts: &[&str], effort: &str) -> Value {
        let efforts: Vec<Value> = efforts.iter().map(|v| json!({"value": v})).collect();
        json!({"configOptions": [
            {"id": "model", "type": "select", "category": "model", "currentValue": model,
             "options": [{"value": "kilo/google/gemini-3-pro-image"}, {"value": "kilo/anthropic/claude-opus-5.5"}]},
            {"id": "effort", "type": "select", "category": "thought_level", "currentValue": effort, "options": efforts},
            {"id": "mode", "type": "select", "category": "mode", "currentValue": "code",
             "options": [{"value": "code"}, {"value": "ask"}, {"value": "plan"}]}
        ]})
    }

    fn kilo_spec(mode: dove_proto::PermissionMode) -> SessionSpec {
        SessionSpec {
            kind: HarnessKind::Kilo,
            program: Default::default(),
            cwd: Default::default(),
            model: Some("kilo/anthropic/claude-opus-5.5".into()),
            reasoning: Some(ReasoningLevel::High),
            model_options: vec![],
            permission_mode: mode,
            resume: None,
            prompt: None,
            fork: false,
        }
    }

    #[test]
    fn kilo_replans_effort_after_the_model_switch_and_enters_plan_mode() {
        let spec = kilo_spec(dove_proto::PermissionMode::Plan);
        // The starting model only offers "thinking", so effort can't be planned yet.
        let session = kilo_options("kilo/google/gemini-3-pro-image", &["thinking"], "thinking");
        let steps = plan_setup(&spec, &session);
        assert_eq!(
            steps,
            vec![
                WireSetup::Config {
                    config_id: "model".into(),
                    payload: json!({"value": "kilo/anthropic/claude-opus-5.5"})
                },
                WireSetup::Config { config_id: "mode".into(), payload: json!({"value": "plan"}) },
            ]
        );
        // The switch answers with Opus's ladder; effort is planned from that.
        let switched =
            kilo_options("kilo/anthropic/claude-opus-5.5", &["low", "medium", "high", "xhigh", "max"], "low");
        let replanned = replan_after_model(&spec, steps.first(), &switched).expect("model write replans");
        assert_eq!(
            replanned,
            vec![
                WireSetup::Config { config_id: "effort".into(), payload: json!({"value": "high"}) },
                WireSetup::Config { config_id: "mode".into(), payload: json!({"value": "plan"}) },
            ]
        );
        // Non-model writes leave the queue alone.
        assert!(replan_after_model(&spec, steps.get(1), &switched).is_none());
        // Kilo runs on the opencode driver; its ACP mode hangs on prompts.
        assert!(launch_args(HarnessKind::Kilo).is_none());
    }

    #[test]
    fn plan_mode_only_when_asked() {
        let session = kilo_options("kilo/anthropic/claude-opus-5.5", &["high"], "high");
        assert!(plan_setup(&kilo_spec(dove_proto::PermissionMode::Default), &session).is_empty());
    }

    #[test]
    fn chooses_permission_options() {
        let o = |id: &str, kind: &str| AcpOption { id: id.into(), kind: kind.into(), name: String::new() };
        let opts = vec![o("a1", "allow_once"), o("r1", "reject_once")];
        assert_eq!(choose(&opts, PermissionDecision::AllowAlways)["outcome"]["optionId"], "a1");
        assert_eq!(choose(&opts, PermissionDecision::Deny)["outcome"]["optionId"], "r1");
        assert_eq!(choose(&[], PermissionDecision::Deny)["outcome"]["outcome"], "cancelled");
    }

    #[test]
    fn replay_is_suppressed() {
        let mut m = Mapper { replaying: true, ..Default::default() };
        let mut out = Vec::new();
        m.update(
            &json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "old"}}),
            &mut out,
        );
        assert!(out.is_empty());
    }
}
