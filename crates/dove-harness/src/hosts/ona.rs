//! Ona (Gitpod). Not implemented yet: every call says so, and the provider shows as
//! unavailable in the app.

use std::path::Path;

use anyhow::{Result, bail};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

pub struct Ona;
pub static PROVIDER: Ona = Ona;

impl Provider for Ona {
    fn id(&self) -> &'static str {
        "ona"
    }

    fn label(&self) -> &'static str {
        "Ona (Gitpod)"
    }

    fn key_url(&self) -> &'static str {
        "https://app.ona.com/settings/personal-access-tokens"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        Probe::Unavailable("Ona (Gitpod) support is not built yet".into())
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        bail!("Ona (Gitpod) support is not built yet")
    }

    fn create(&self, _keys: &Keys, _spec: &BoxSpec) -> Result<BoxInfo> {
        bail!("Ona (Gitpod) support is not built yet")
    }

    fn wake(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Ona (Gitpod) support is not built yet")
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Ona (Gitpod) support is not built yet")
    }

    fn delete(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Ona (Gitpod) support is not built yet")
    }

    fn connect(&self, _keys: &Keys, _id: &str, _dir: &Path) -> Result<Endpoint> {
        bail!("Ona (Gitpod) support is not built yet")
    }
}
