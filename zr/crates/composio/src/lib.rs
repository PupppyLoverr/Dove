//! Composio Connect for Keplorayx: the apps a person connected on their own
//! Composio account (Gmail, Slack, Notion, …), offered to chats and buddies
//! as far as they said.
//!
//! - [`policy`]: who may use which app (everywhere, Zeron's chats, every
//!   buddy, or some buddies) and when an app action waits for the person's
//!   OK. Kept in `<data>/connections.json`, written by the app.
//! - [`key`]: the person's Composio key (`ck_…`), sealed in
//!   `<data>/keys.json` beside the app's other keys, opened only here.
//! - [`client`]: Composio's hosted MCP server, over streamable HTTP.
//! - [`bridge`]: a stdio MCP server the engine gives a chat: it relays to
//!   Composio with the key added, and holds the chat to the policy. Every
//!   harness takes a stdio server, and none ever sees the key.

pub mod bridge;
pub mod client;
pub mod key;
pub mod policy;

/// Composio's hosted MCP endpoint (Composio Connect).
pub const ENDPOINT: &str = "https://connect.composio.dev/mcp";
