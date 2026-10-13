//! Drive one real session end to end and print its events.
//!
//! cargo run -p dove-harness --example smoke -- <harness-id> [model] [effort] [prompt]
//! e.g. cargo run -p dove-harness --example smoke -- kilo kilo/google/gemini-3.7-flash low

use std::time::Duration;

use dove_harness::{SessionSpec, detect, spawn};
use dove_proto::{AgentEvent, HarnessKind, PermissionMode, ReasoningLevel};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let kind =
        args.next().and_then(|id| HarnessKind::from_id(&id)).ok_or_else(|| anyhow::anyhow!("unknown harness id"))?;
    let model = args.next().filter(|m| !m.is_empty() && m != "-");
    let reasoning = args.next().and_then(|r| ReasoningLevel::from_id(&r));
    let prompt = args.next().unwrap_or_else(|| "Reply with exactly the word OK and nothing else.".into());
    let program = detect::find(kind).ok_or_else(|| anyhow::anyhow!("{} is not on PATH", kind.label()))?;
    println!("{} at {}", kind.label(), program.display());

    let rt = tokio::runtime::Runtime::new()?;
    let spec = SessionSpec {
        kind,
        program,
        cwd: std::env::temp_dir(),
        model,
        reasoning,
        model_options: Vec::new(),
        permission_mode: PermissionMode::Default,
        resume: None,
        prompt: Some(prompt),
        fork: false,
    };
    let handle = spawn(rt.handle(), spec)?;
    let start = std::time::Instant::now();
    let deadline = start + Duration::from_secs(180);
    while std::time::Instant::now() < deadline {
        match handle.events.recv_timeout(Duration::from_secs(5)) {
            Ok(AgentEvent::TextDelta { text }) => print!("{text}"),
            Ok(AgentEvent::ThinkingDelta { .. }) => {}
            Ok(ev @ AgentEvent::TurnEnded { .. }) => {
                println!("\n{ev:?}");
                break;
            }
            Ok(ev @ AgentEvent::Exited { .. }) => {
                println!("\n{ev:?}");
                break;
            }
            Ok(ev) => println!("\n{ev:?}"),
            Err(flume::RecvTimeoutError::Timeout) => {}
            Err(flume::RecvTimeoutError::Disconnected) => {
                println!("\n[event stream closed after {:?}]", start.elapsed());
                break;
            }
        }
    }
    if std::time::Instant::now() >= deadline {
        println!("\n[timed out]");
    }
    Ok(())
}
