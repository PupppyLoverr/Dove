//! GitHub Codespaces. Not implemented yet: every call says so, and the provider shows as
//! unavailable in the app.

use std::path::Path;

use anyhow::{Result, bail};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

pub struct Codespaces;
pub static PROVIDER: Codespaces = Codespaces;

impl Provider for Codespaces {
    fn id(&self) -> &'static str {
        "codespaces"
    }

    fn label(&self) -> &'static str {
        "GitHub Codespaces"
    }

    fn key_url(&self) -> &'static str {
        "https://github.com/codespaces"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        false
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        Probe::Unavailable("GitHub Codespaces support is not built yet".into())
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        bail!("GitHub Codespaces support is not built yet")
    }

    fn create(&self, _keys: &Keys, _spec: &BoxSpec) -> Result<BoxInfo> {
        bail!("GitHub Codespaces support is not built yet")
    }

    fn wake(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("GitHub Codespaces support is not built yet")
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("GitHub Codespaces support is not built yet")
    }

    fn delete(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("GitHub Codespaces support is not built yet")
    }

    fn connect(&self, _keys: &Keys, _id: &str, _dir: &Path) -> Result<Endpoint> {
        bail!("GitHub Codespaces support is not built yet")
    }
}
