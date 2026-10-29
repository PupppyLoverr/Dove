//! Runloop. Not implemented yet: every call says so, and the provider shows as
//! unavailable in the app.

use std::path::Path;

use anyhow::{Result, bail};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

pub struct Runloop;
pub static PROVIDER: Runloop = Runloop;

impl Provider for Runloop {
    fn id(&self) -> &'static str {
        "runloop"
    }

    fn label(&self) -> &'static str {
        "Runloop"
    }

    fn key_url(&self) -> &'static str {
        "https://platform.runloop.ai/settings"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        Probe::Unavailable("Runloop support is not built yet".into())
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        bail!("Runloop support is not built yet")
    }

    fn create(&self, _keys: &Keys, _spec: &BoxSpec) -> Result<BoxInfo> {
        bail!("Runloop support is not built yet")
    }

    fn wake(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Runloop support is not built yet")
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Runloop support is not built yet")
    }

    fn delete(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Runloop support is not built yet")
    }

    fn connect(&self, _keys: &Keys, _id: &str, _dir: &Path) -> Result<Endpoint> {
        bail!("Runloop support is not built yet")
    }
}
