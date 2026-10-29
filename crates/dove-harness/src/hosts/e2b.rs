//! E2B. Not implemented yet: every call says so, and the provider shows as
//! unavailable in the app.

use std::path::Path;

use anyhow::{Result, bail};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

pub struct E2b;
pub static PROVIDER: E2b = E2b;

impl Provider for E2b {
    fn id(&self) -> &'static str {
        "e2b"
    }

    fn label(&self) -> &'static str {
        "E2B"
    }

    fn key_url(&self) -> &'static str {
        "https://e2b.dev/dashboard?tab=keys"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        Probe::Unavailable("E2B support is not built yet".into())
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        bail!("E2B support is not built yet")
    }

    fn create(&self, _keys: &Keys, _spec: &BoxSpec) -> Result<BoxInfo> {
        bail!("E2B support is not built yet")
    }

    fn wake(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("E2B support is not built yet")
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("E2B support is not built yet")
    }

    fn delete(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("E2B support is not built yet")
    }

    fn connect(&self, _keys: &Keys, _id: &str, _dir: &Path) -> Result<Endpoint> {
        bail!("E2B support is not built yet")
    }
}
