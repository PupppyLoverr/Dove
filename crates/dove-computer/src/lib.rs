//! Dove Computer Use: the one `computer` tool agents see.
//!
//! A Rust port of Hermes Agent's `computer_use` layer (MIT, Nous Research:
//! `tools/computer_use/`) driving stock cua-driver (MIT, Cua AI) over MCP:
//! background-first input with the verify → escalate verdict ladder,
//! `#N` row indices backed by per-snapshot element tokens (a stale index is
//! refused, never re-resolved), hard blocks, scoped approvals and the
//! human-takeover fence. Dove adds tree-first captures with `since` diffs,
//! batched `run` steps, on-device grounding (`act`, NEXI-CUA-1), OCR rows
//! for sparse trees, signed-identity app rules and replays.

pub mod capture;
pub mod computer;
pub mod cua;
pub mod driver;
pub mod host;
pub mod native;
pub mod parse;
pub mod policy;
pub mod replay;
pub mod schema;
pub mod types;
pub mod verdict;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use computer::{Approvals, Computer, Config};
pub use driver::Driver;
pub use host::{Decision, Host, Lease, OcrLine, Review, Scope};
pub use policy::{AppIdentity, Rule};
pub use types::Output;
