//! Run one restricted, tool-less title run and print the result.
//!
//! cargo run -p dove-harness --example title -- <harness-id> [model] [prompt]
//! e.g. cargo run -p dove-harness --example title -- codex - "Fix the login redirect loop"

use dove_harness::{detect, title};
use dove_proto::HarnessKind;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let kind =
        args.next().and_then(|id| HarnessKind::from_id(&id)).ok_or_else(|| anyhow::anyhow!("unknown harness id"))?;
    let model = args.next().filter(|m| !m.is_empty() && m != "-");
    let prompt = args.next().unwrap_or_else(|| "Fix the login page so it stops redirecting in a loop.".into());
    let program = detect::find(kind).ok_or_else(|| anyhow::anyhow!("{} is not on PATH", kind.label()))?;
    println!("{} at {}", kind.label(), program.display());

    let rt = tokio::runtime::Runtime::new()?;
    let start = std::time::Instant::now();
    match rt.block_on(title::try_generate(kind, &program, &prompt, model)) {
        Ok(t) => println!("title: {t:?} ({:.1}s)", start.elapsed().as_secs_f32()),
        Err(e) => println!("failed: {e:#} ({:.1}s)", start.elapsed().as_secs_f32()),
    }
    Ok(())
}
