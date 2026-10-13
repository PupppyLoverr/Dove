//! Time to first reply: a cold session vs a pre-warmed one (Claude Code).
//!
//! cargo run -p dove-harness --example warm -- [model]

use std::time::{Duration, Instant};

use dove_harness::{Command, SessionSpec, detect, spawn};
use dove_proto::{AgentEvent, HarnessKind, PermissionMode};

const PROMPT: &str = "Reply with exactly the word OK and nothing else.";

fn first_text(handle: &dove_harness::SessionHandle, since: Instant) -> Option<Duration> {
    let deadline = since + Duration::from_secs(120);
    while Instant::now() < deadline {
        match handle.events.recv_timeout(Duration::from_secs(2)) {
            Ok(AgentEvent::TextDelta { .. }) => return Some(since.elapsed()),
            Ok(AgentEvent::Exited { .. }) | Err(flume::RecvTimeoutError::Disconnected) => return None,
            Ok(AgentEvent::Error { message }) => println!("  error: {message}"),
            _ => {}
        }
    }
    None
}

fn main() -> anyhow::Result<()> {
    let model = std::env::args().nth(1);
    let program = detect::find(HarnessKind::ClaudeCode).ok_or_else(|| anyhow::anyhow!("claude is not on PATH"))?;
    let rt = tokio::runtime::Runtime::new()?;
    let spec = |prompt: Option<String>| SessionSpec {
        kind: HarnessKind::ClaudeCode,
        program: program.clone(),
        cwd: std::env::temp_dir(),
        model: model.clone(),
        reasoning: None,
        model_options: Vec::new(),
        permission_mode: PermissionMode::Default,
        resume: None,
        prompt,
        fork: false,
    };

    let start = Instant::now();
    let cold = spawn(rt.handle(), spec(Some(PROMPT.into())))?;
    let cold_t = first_text(&cold, start);
    drop(cold);
    println!("cold: first text after {cold_t:?}");

    let warm = spawn(rt.handle(), spec(None))?;
    // The user is still typing.
    std::thread::sleep(Duration::from_secs(8));
    let start = Instant::now();
    warm.send(Command::Send(PROMPT.into()));
    let warm_t = first_text(&warm, start);
    println!("warm: first text after {warm_t:?} (from send)");
    Ok(())
}
