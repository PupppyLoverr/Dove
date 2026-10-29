//! DeepInfra Sandbox. Not implemented yet: every call says so, and the provider shows as
//! unavailable in the app.

use std::path::Path;

use anyhow::{Result, bail};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

pub struct Deepinfra;
pub static PROVIDER: Deepinfra = Deepinfra;

impl Provider for Deepinfra {
    fn id(&self) -> &'static str {
        "deepinfra"
    }

    fn label(&self) -> &'static str {
        "DeepInfra Sandbox"
    }

    fn key_url(&self) -> &'static str {
        "https://deepinfra.com/dash/api_keys"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        Probe::Unavailable("DeepInfra Sandbox support is not built yet".into())
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        bail!("DeepInfra Sandbox support is not built yet")
    }

    fn create(&self, _keys: &Keys, _spec: &BoxSpec) -> Result<BoxInfo> {
        bail!("DeepInfra Sandbox support is not built yet")
    }

    fn wake(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("DeepInfra Sandbox support is not built yet")
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("DeepInfra Sandbox support is not built yet")
    }

    fn delete(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("DeepInfra Sandbox support is not built yet")
    }

    fn connect(&self, _keys: &Keys, _id: &str, _dir: &Path) -> Result<Endpoint> {
        bail!("DeepInfra Sandbox support is not built yet")
    }
}
