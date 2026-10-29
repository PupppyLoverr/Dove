//! Bubblewrap. Not implemented yet: every call says so, and the provider shows as
//! unavailable in the app.

use std::path::Path;

use anyhow::{Result, bail};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

pub struct Bubblewrap;
pub static PROVIDER: Bubblewrap = Bubblewrap;

impl Provider for Bubblewrap {
    fn id(&self) -> &'static str {
        "bubblewrap"
    }

    fn label(&self) -> &'static str {
        "Bubblewrap"
    }

    fn key_url(&self) -> &'static str {
        "https://github.com/containers/bubblewrap"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        false
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        Probe::Unavailable("Bubblewrap support is not built yet".into())
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        bail!("Bubblewrap support is not built yet")
    }

    fn create(&self, _keys: &Keys, _spec: &BoxSpec) -> Result<BoxInfo> {
        bail!("Bubblewrap support is not built yet")
    }

    fn wake(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Bubblewrap support is not built yet")
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Bubblewrap support is not built yet")
    }

    fn delete(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Bubblewrap support is not built yet")
    }

    fn connect(&self, _keys: &Keys, _id: &str, _dir: &Path) -> Result<Endpoint> {
        bail!("Bubblewrap support is not built yet")
    }
}
