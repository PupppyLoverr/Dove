//! A stand-in agent CLI for Dove's perf harness.
//!
//! Point an agent's binary path (`agent_path.<harness>`) at this program and
//! Dove drives it like the real thing: the same process spawn, pipes, wire
//! parsing and transcript recording, but every turn is synthetic, so a run of
//! ten concurrent sessions costs no tokens and is repeatable.
//!
//! Protocol by command line:
//! - `--version`: prints a version and exits.
//! - args containing `stream-json`: Claude Code's stream-json (stdin and
//!   stdout JSON lines, control requests answered with success).
//! - anything else: Agent Client Protocol (newline-delimited JSON-RPC).
//!
//! Tuning (environment):
//! - `FAKE_AGENT_SECS`: length of one turn in seconds (default 45).
//! - `FAKE_AGENT_CPS`: streamed characters per second (default 240, about 60
//!   tokens per second).
//! - `FAKE_AGENT_CHUNK`: characters per text delta (default 12).
//! - `FAKE_AGENT_TOOL_EVERY`: seconds between tool calls (default 6).
//! - `FAKE_AGENT_TURNS`: turns to run back to back for each prompt (default 1).

use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const CORPUS: &str = include_str!("corpus.md");

fn env_f64(name: &str, default: f64) -> f64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).filter(|v: &f64| *v > 0.0).unwrap_or(default)
}

#[derive(Clone, Copy)]
struct Tuning {
    turn: Duration,
    cps: f64,
    chunk: usize,
    tool_every: Duration,
    turns: u32,
}

impl Tuning {
    fn from_env() -> Self {
        Self {
            turn: Duration::from_secs_f64(env_f64("FAKE_AGENT_SECS", 45.0)),
            cps: env_f64("FAKE_AGENT_CPS", 240.0),
            chunk: env_f64("FAKE_AGENT_CHUNK", 12.0) as usize,
            tool_every: Duration::from_secs_f64(env_f64("FAKE_AGENT_TOOL_EVERY", 6.0)),
            turns: env_f64("FAKE_AGENT_TURNS", 1.0) as u32,
        }
    }
}

/// One line at a time to stdout, flushed, shared by every thread.
#[derive(Clone)]
struct Out(Arc<Mutex<std::io::Stdout>>);

impl Out {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(std::io::stdout())))
    }

    /// False once the reader is gone (Dove dropped the session).
    fn send(&self, value: &Value) -> bool {
        let mut out = self.0.lock().unwrap_or_else(|e| e.into_inner());
        writeln!(out, "{value}").and_then(|()| out.flush()).is_ok()
    }
}

/// What a turn streams, independent of the wire it is framed for.
enum Step<'a> {
    Thinking(&'a str),
    Text(&'a str),
    /// Close the current assistant message (a tool call follows, or the end).
    EndMessage,
    Tool { id: String, name: &'static str, title: String, output: String },
}

/// Drive one synthetic turn, calling `emit` for each step at the configured
/// pace. Stops early when `cancel` is set or `emit` reports a closed pipe.
fn run_turn(t: Tuning, seed: u64, cancel: &AtomicBool, mut emit: impl FnMut(Step) -> bool) {
    let started = Instant::now();
    let mut next_tool = t.tool_every;
    let gap = Duration::from_secs_f64(t.chunk as f64 / t.cps);
    let corpus: Vec<char> = CORPUS.chars().collect();
    let mut at = (seed as usize * 977) % corpus.len();
    let mut tools = 0u64;
    let mut buf = String::new();
    if !emit(Step::Thinking("Reading the relevant files before changing anything. ")) {
        return;
    }
    while started.elapsed() < t.turn && !cancel.load(Ordering::Relaxed) {
        if started.elapsed() >= next_tool {
            next_tool += t.tool_every;
            tools += 1;
            let (name, title) = match tools % 3 {
                0 => ("Bash", "cargo check -p dove".to_string()),
                1 => ("Read", format!("Read src/module_{}.rs", tools % 40)),
                _ => ("Grep", "Search for cx.notify".to_string()),
            };
            let output = (0..24).map(|i| format!("{name} output line {i}\n")).collect::<String>();
            if !emit(Step::EndMessage)
                || !emit(Step::Tool { id: format!("toolu_{seed}_{tools}"), name, title, output })
            {
                return;
            }
            continue;
        }
        buf.clear();
        for _ in 0..t.chunk {
            buf.push(corpus[at]);
            at = (at + 1) % corpus.len();
        }
        if !emit(Step::Text(&buf)) {
            return;
        }
        thread::sleep(gap);
    }
    emit(Step::EndMessage);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("2.1.300 (Claude Code) [dove-fake-agent]");
        return;
    }
    let tuning = Tuning::from_env();
    if args.iter().any(|a| a.contains("stream-json")) {
        claude(tuning);
    } else {
        acp(tuning);
    }
}

fn stdin_lines() -> mpsc::Receiver<Value> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
                if tx.send(v).is_err() {
                    break;
                }
            }
        }
    });
    rx
}

// ------------------------------------------------------------ Claude stream-json

fn claude(t: Tuning) {
    let out = Out::new();
    let session = format!("fake-{}", std::process::id());
    let cancel = Arc::new(AtomicBool::new(false));
    let (turn_tx, turn_rx) = mpsc::channel::<Option<String>>();
    let lines = stdin_lines();

    // Control requests are answered at once, even mid-turn.
    {
        let out = out.clone();
        let cancel = cancel.clone();
        thread::spawn(move || {
            for v in lines {
                match v["type"].as_str() {
                    Some("control_request") => {
                        let id = v["request_id"].as_str().unwrap_or_default();
                        let subtype = v["request"]["subtype"].as_str().unwrap_or_default();
                        if subtype == "interrupt" {
                            cancel.store(true, Ordering::Relaxed);
                        }
                        let body = if subtype == "initialize" { json!({"commands": [], "models": []}) } else { json!({}) };
                        out.send(&json!({"type": "control_response",
                            "response": {"subtype": "success", "request_id": id, "response": body}}));
                    }
                    Some("user") => {
                        let uuid = v["uuid"].as_str().map(str::to_string);
                        if let Some(id) = &uuid {
                            out.send(&json!({"type": "command_lifecycle", "command_uuid": id, "state": "queued"}));
                        }
                        if turn_tx.send(uuid).is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    let mut inited = false;
    let mut seed = 0u64;
    while let Ok(uuid) = turn_rx.recv() {
        if !inited {
            inited = true;
            out.send(&json!({"type": "system", "subtype": "init", "model": "fake-model", "tools": ["Bash", "Read", "Grep"],
                "cwd": std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default(), "session_id": session}));
        }
        if let Some(id) = &uuid {
            out.send(&json!({"type": "command_lifecycle", "command_uuid": id, "state": "started"}));
        }
        cancel.store(false, Ordering::Relaxed);
        let mut chars = 0u64;
        for _ in 0..t.turns.max(1) {
            seed += 1;
            run_turn(t, seed, &cancel, |step| match step {
                Step::Thinking(text) => out.send(&json!({"type": "stream_event",
                    "event": {"type": "content_block_delta", "delta": {"type": "thinking_delta", "thinking": text}}})),
                Step::Text(text) => {
                    chars += text.len() as u64;
                    out.send(&json!({"type": "stream_event",
                        "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": text}}}))
                }
                Step::EndMessage => out.send(&json!({"type": "assistant", "message": {"model": "fake-model",
                    "content": [], "usage": {"input_tokens": 1200, "cache_read_input_tokens": 30000}}})),
                Step::Tool { id, name, title, output } => {
                    out.send(&json!({"type": "assistant", "message": {"model": "fake-model",
                        "content": [{"type": "tool_use", "id": id, "name": name,
                            "input": {"command": title, "file_path": title, "pattern": title, "description": title}}]}}))
                        && {
                            thread::sleep(Duration::from_millis(250));
                            out.send(&json!({"type": "user", "message": {"role": "user",
                                "content": [{"type": "tool_result", "tool_use_id": id, "content": output, "is_error": false}]}}))
                        }
                }
            });
        }
        out.send(&json!({"type": "result", "subtype": "success", "result": "done", "session_id": session,
            "usage": {"input_tokens": 1200, "output_tokens": chars / 4},
            "modelUsage": {"fake-model": {"contextWindow": 200000}}}));
        if let Some(id) = &uuid {
            out.send(&json!({"type": "command_lifecycle", "command_uuid": id, "state": "completed"}));
        }
    }
}

// ------------------------------------------------------------ ACP

fn acp(t: Tuning) {
    let out = Out::new();
    let cancel = Arc::new(AtomicBool::new(false));
    let seeds = Arc::new(AtomicU64::new(0));
    let session = format!("fake-acp-{}", std::process::id());
    for v in stdin_lines() {
        let method = v["method"].as_str().unwrap_or_default();
        let id = v.get("id").cloned();
        let reply = |result: Value| {
            if let Some(id) = &id {
                out.send(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
            }
        };
        match method {
            "initialize" => reply(json!({"protocolVersion": 1,
                "agentCapabilities": {"loadSession": true, "promptCapabilities": {"image": false}}, "authMethods": []})),
            "session/new" => reply(json!({"sessionId": session,
                "models": {"availableModels": [{"modelId": "fake-model", "name": "Fake model"}], "currentModelId": "fake-model"}})),
            "session/cancel" => cancel.store(true, Ordering::Relaxed),
            "session/prompt" => {
                let (out, cancel, session) = (out.clone(), cancel.clone(), session.clone());
                let seed = seeds.fetch_add(1, Ordering::Relaxed) + 1;
                cancel.store(false, Ordering::Relaxed);
                thread::spawn(move || {
                    let update = |u: Value| {
                        out.send(&json!({"jsonrpc": "2.0", "method": "session/update",
                            "params": {"sessionId": session, "update": u}}))
                    };
                    for turn in 0..t.turns.max(1) {
                        run_turn(t, seed * 100 + u64::from(turn), &cancel, |step| match step {
                            Step::Thinking(text) => update(json!({"sessionUpdate": "agent_thought_chunk",
                                "content": {"type": "text", "text": text}})),
                            Step::Text(text) => update(json!({"sessionUpdate": "agent_message_chunk",
                                "content": {"type": "text", "text": text}})),
                            Step::EndMessage => true,
                            Step::Tool { id, title, output, .. } => {
                                update(json!({"sessionUpdate": "tool_call", "toolCallId": id, "title": title,
                                    "kind": "execute", "status": "in_progress", "rawInput": {"command": title}}))
                                    && {
                                        thread::sleep(Duration::from_millis(250));
                                        update(json!({"sessionUpdate": "tool_call_update", "toolCallId": id,
                                            "status": "completed",
                                            "content": [{"type": "content", "content": {"type": "text", "text": output}}]}))
                                    }
                            }
                        });
                    }
                    let stop = if cancel.load(Ordering::Relaxed) { "cancelled" } else { "end_turn" };
                    if let Some(id) = id {
                        out.send(&json!({"jsonrpc": "2.0", "id": id, "result": {"stopReason": stop}}));
                    }
                });
            }
            // session/load, set_model, set_mode, set_config_option, authenticate, …
            _ => reply(json!({})),
        }
    }
}
