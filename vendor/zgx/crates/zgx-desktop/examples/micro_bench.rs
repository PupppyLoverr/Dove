//! The microbenchmarks (zgx_workload::micro), headless: CPU time per
//! operation, for the operation alone ("write") and through to a frame ready
//! to present ("frame": layout, damage and the renderer's encode and submit;
//! with `RENDER=0`, text measured as a renderer would and nothing drawn, as
//! CI's instruction gate counts it). "frame" is the headline: "write" is
//! reported so a fast write cannot hide a slow frame, and is never made
//! faster at the frame's expense.
//!
//!   SCENE=state|vscroll-fixed|vscroll-variable OPS=2000 cargo run --release -p zgx-desktop --example micro_bench
//!
//! `micro_luau_bench` runs the same scenes written in Luau.
//!
//! - `state`: one counter bound to one text, over a static grid; an
//!   operation sets the counter.
//! - `vscroll-fixed`, `vscroll-variable`: 10,000 rows, fixed or measured
//!   variable heights (the vlist rows); an operation scrolls 12 points.
//!
//! Prints one JSON line per figure: nanoseconds of the process's CPU time per
//! operation (median, p10, p90, min, max over batches).
#[path = "support/micro_harness.rs"]
mod micro_harness;

fn main() {
    micro_harness::main("zgx", None);
}
