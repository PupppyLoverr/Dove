//! Cloudflare Sandbox (developers.cloudflare.com/sandbox): containers run by
//! a Worker the user deploys themselves. Not usable from Keplorayx yet, and
//! this file says so instead of pretending.
//!
//! What is documented today:
//! - SDK 0.x shipped a "bridge" Worker with an HTTP API
//!   (https://developers.cloudflare.com/sandbox/sdk/bridge/http-api/):
//!   `POST /v1/sandbox`, `DELETE /v1/sandbox/:id`, `POST /v1/sandbox/:id/exec`
//!   (SSE output, **no stdin**) and `GET /v1/sandbox/:id/pty` (a WebSocket
//!   PTY). An agent speaks JSON over stdin/stdout, so exec can't carry it,
//!   and a PTY would mangle its bytes.
//! - SDK 1.0 removed the bridge: "1.0 has no bridge. Add a Worker route for
//!   each call that your client makes"
//!   (https://developers.cloudflare.com/sandbox/sdk/migrate/other-features/).
//!   Streaming stdin exists only inside the Worker (`container.exec()`).
//!
//! So there is no stable HTTP or WebSocket surface for a non-PTY exec with
//! live stdin. Support needs a small Keplorayx Worker (routes for
//! create/list/delete and a WebSocket exec relaying stdin/stdout/stderr and
//! the exit code) published with its own protocol; the key format reserved
//! for it is `<worker URL>|<token>`. Until then `probe` explains this and
//! every other call fails with the same text.

use std::path::Path;

use anyhow::{Result, bail};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

const NOT_YET: &str = "Cloudflare Sandbox needs a Worker that streams a command's stdin and stdout. \
SDK 1.0 has no HTTP bridge and the old bridge's exec has no stdin, so Keplorayx can't run agents there yet";

pub struct Cloudflare;
pub static PROVIDER: Cloudflare = Cloudflare;

impl Provider for Cloudflare {
    fn id(&self) -> &'static str {
        "cloudflare"
    }

    fn label(&self) -> &'static str {
        "Cloudflare Sandbox"
    }

    fn key_url(&self) -> &'static str {
        "https://developers.cloudflare.com/sandbox/"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        Probe::NeedsSetup(NOT_YET.into())
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        bail!("{NOT_YET}")
    }

    fn create(&self, _keys: &Keys, _spec: &BoxSpec) -> Result<BoxInfo> {
        bail!("{NOT_YET}")
    }

    fn wake(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("{NOT_YET}")
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("{NOT_YET}")
    }

    fn delete(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("{NOT_YET}")
    }

    fn connect(&self, _keys: &Keys, _id: &str, _dir: &Path) -> Result<Endpoint> {
        bail!("{NOT_YET}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_explains_what_is_missing() {
        let mut k = Keys::default();
        k.set("cloudflare", "https://sbx.example.workers.dev|tok");
        match PROVIDER.probe(&k) {
            Probe::NeedsSetup(why) => assert!(why.contains("stdin")),
            other => panic!("unexpected probe {other:?}"),
        }
        assert!(PROVIDER.list(&k).is_err());
    }
}
