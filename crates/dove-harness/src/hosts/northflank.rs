//! Northflank. Not implemented yet: every call says so, and the provider shows as
//! unavailable in the app.

use std::path::Path;

use anyhow::{Result, bail};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

pub struct Northflank;
pub static PROVIDER: Northflank = Northflank;

impl Provider for Northflank {
    fn id(&self) -> &'static str {
        "northflank"
    }

    fn label(&self) -> &'static str {
        "Northflank"
    }

    fn key_url(&self) -> &'static str {
        "https://app.northflank.com/s/account/settings/api/tokens"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        Probe::Unavailable("Northflank support is not built yet".into())
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        bail!("Northflank support is not built yet")
    }

    fn create(&self, _keys: &Keys, _spec: &BoxSpec) -> Result<BoxInfo> {
        bail!("Northflank support is not built yet")
    }

    fn wake(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Northflank support is not built yet")
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Northflank support is not built yet")
    }

    fn delete(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Northflank support is not built yet")
    }

    fn connect(&self, _keys: &Keys, _id: &str, _dir: &Path) -> Result<Endpoint> {
        bail!("Northflank support is not built yet")
    }
}
