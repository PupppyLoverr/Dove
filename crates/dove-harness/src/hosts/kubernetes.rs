//! Kubernetes. Not implemented yet: every call says so, and the provider shows as
//! unavailable in the app.

use std::path::Path;

use anyhow::{Result, bail};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

pub struct Kubernetes;
pub static PROVIDER: Kubernetes = Kubernetes;

impl Provider for Kubernetes {
    fn id(&self) -> &'static str {
        "kubernetes"
    }

    fn label(&self) -> &'static str {
        "Kubernetes"
    }

    fn key_url(&self) -> &'static str {
        "https://kubernetes.io/docs/concepts/configuration/organize-cluster-access-kubeconfig/"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        false
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        Probe::Unavailable("Kubernetes support is not built yet".into())
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        bail!("Kubernetes support is not built yet")
    }

    fn create(&self, _keys: &Keys, _spec: &BoxSpec) -> Result<BoxInfo> {
        bail!("Kubernetes support is not built yet")
    }

    fn wake(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Kubernetes support is not built yet")
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Kubernetes support is not built yet")
    }

    fn delete(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Kubernetes support is not built yet")
    }

    fn connect(&self, _keys: &Keys, _id: &str, _dir: &Path) -> Result<Endpoint> {
        bail!("Kubernetes support is not built yet")
    }
}
