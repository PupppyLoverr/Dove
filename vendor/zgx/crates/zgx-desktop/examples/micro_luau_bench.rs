//! The microbenchmarks (`micro_bench`) written in Luau and loaded through
//! `ScriptHost` (`support/micro_luau.rs`): a state operation runs the
//! script's handler, a scroll step moves its list as a host does. A binary
//! of its own, so `micro_bench` links no script engine.
//!
//!   SCENE=state|vscroll-fixed|vscroll-variable OPS=2000 cargo run --release -p zgx-desktop --example micro_luau_bench
//!
//! Prints `micro_bench`'s figures, as `"framework":"zgx-luau"`.
#[path = "support/micro_harness.rs"]
mod micro_harness;
#[path = "support/micro_luau.rs"]
mod micro_luau;

fn main() {
    micro_harness::main(
        "zgx-luau",
        Some(&|ui, scene| {
            let (host, step) =
                micro_luau::host(&ui.runtime, scene, micro_harness::vlist_view::ESTIMATE);
            ui.mount(host.view());
            (Box::new(host), step)
        }),
    );
}
