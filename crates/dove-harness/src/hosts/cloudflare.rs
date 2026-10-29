//! Cloudflare Sandbox. Not implemented yet: every call says so, and the provider shows as
//! unavailable in the app.

use std::path::Path;

use anyhow::{Result, bail};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

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
        Probe::Unavailable("Cloudflare Sandbox support is not built yet".into())
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        bail!("Cloudflare Sandbox support is not built yet")
    }

    fn create(&self, _keys: &Keys, _spec: &BoxSpec) -> Result<BoxInfo> {
        bail!("Cloudflare Sandbox support is not built yet")
    }

    fn wake(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Cloudflare Sandbox support is not built yet")
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Cloudflare Sandbox support is not built yet")
    }

    fn delete(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Cloudflare Sandbox support is not built yet")
    }

    fn connect(&self, _keys: &Keys, _id: &str, _dir: &Path) -> Result<Endpoint> {
        bail!("Cloudflare Sandbox support is not built yet")
    }
}
