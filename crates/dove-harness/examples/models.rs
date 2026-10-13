//! Print each installed agent's live model catalog next to the curated one.
//!
//! cargo run -p dove-harness --example models -- [harness-id ...]

use std::time::{Duration, Instant};

use dove_harness::{catalog, detect, models};
use dove_proto::HarnessKind;

fn main() -> anyhow::Result<()> {
    let wanted: Vec<HarnessKind> = std::env::args().skip(1).filter_map(|id| HarnessKind::from_id(&id)).collect();
    let rt = tokio::runtime::Runtime::new()?;
    for installed in detect::scan() {
        if !wanted.is_empty() && !wanted.contains(&installed.kind) {
            continue;
        }
        let kind = installed.kind;
        let started = Instant::now();
        let live = rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(90), catalog::fetch(kind, &installed.program)).await
        });
        let curated = models::catalog(kind).len();
        match live {
            Ok(Ok(list)) => {
                println!("{} — {} live (curated {curated}) in {:?}", kind.label(), list.len(), started.elapsed());
                for m in list.iter().take(40) {
                    let ladder: Vec<&str> = m.reasoning_levels.iter().map(|l| l.id()).collect();
                    let opts: Vec<&str> = m.options.iter().map(|o| o.id.as_str()).collect();
                    println!("    {:<44} {:<28} effort={:?} opts={:?}", m.id, m.label, ladder, opts);
                }
            }
            Ok(Err(e)) => println!("{} — failed: {e:#}", kind.label()),
            Err(_) => println!("{} — timed out", kind.label()),
        }
    }
    Ok(())
}
