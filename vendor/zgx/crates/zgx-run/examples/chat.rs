//! The chat example (zgx-script's `examples/chat`) as an app, against a
//! model served in memory: replies stream in as server-sent events and draw
//! as markdown in a virtualized transcript; a composer, a sidebar of
//! conversations, and the commands' shortcuts in the menu bar.
//! Run: cargo run --release -p zgx-run --example chat
//! Edit `crates/zgx-script/examples/chat/*.luau` and save to reload it.
//! `CHAT_PACE=ms` sets the time between tokens (25), `CHAT_FAIL=n` fails the
//! first `n` replies.
#[path = "../../zgx-script/examples/support/chat_server.rs"]
mod chat_server;

use std::{path::Path, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../zgx-script/examples/chat");
    let pace = zgx::env::var("CHAT_PACE")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(25);
    let server = chat_server::ChatServer::new(Duration::from_millis(pace));
    if let Some(n) = zgx::env::var("CHAT_FAIL").ok().and_then(|n| n.parse().ok()) {
        server.fail(n);
    }
    zgx_run::run_hosting(&script, Vec::new(), move |host| server.serve(host))
}
