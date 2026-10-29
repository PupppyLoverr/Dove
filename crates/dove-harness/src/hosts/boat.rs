//! Boat. Not implemented yet: every call says so, and the provider shows as
//! unavailable in the app.

use std::path::Path;

use anyhow::{Result, bail};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

pub struct Boat;
pub static PROVIDER: Boat = Boat;

impl Provider for Boat {
    fn id(&self) -> &'static str {
        "boat"
    }

    fn label(&self) -> &'static str {
        "Boat"
    }

    fn key_url(&self) -> &'static str {
        "https://boat.dev"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        Probe::Unavailable("Boat support is not built yet".into())
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        bail!("Boat support is not built yet")
    }

    fn create(&self, _keys: &Keys, _spec: &BoxSpec) -> Result<BoxInfo> {
        bail!("Boat support is not built yet")
    }

    fn wake(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Boat support is not built yet")
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Boat support is not built yet")
    }

    fn delete(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Boat support is not built yet")
    }

    fn connect(&self, _keys: &Keys, _id: &str, _dir: &Path) -> Result<Endpoint> {
        bail!("Boat support is not built yet")
    }
}
