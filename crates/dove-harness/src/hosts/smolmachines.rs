//! smol machines: persistent microVMs on this computer through the `smolvm`
//! CLI (libkrun; macOS on Apple Silicon, or Linux with `/dev/kvm`).
//!
//! - `smolvm machine create --name N --net --image IMAGE` records a machine
//!   (default image `node:22-bookworm`: the project is synced with `git`
//!   and agent CLIs install with `npm`); `machine start --name N` boots it.
//! - `smolvm machine ls --json` lists machines; `machine stop --name N`
//!   stops one and keeps its disk; `machine delete --name N -f` removes it.
//! - `smolvm machine exec -i --stream --name N -- <argv…>` runs the agent
//!   with stdin kept open and output streamed, never a PTY (the docs warn a
//!   PTY "is not a reliable byte pipe").
//!
//! Caveat: Hermes Agent's `smolmachines` catalog entry says "The engine stops
//! a command once it has printed 11 MiB" (buffered exec). `--stream` is
//! used to avoid that buffer; if the cap also applies to streamed exec, a
//! long, chatty agent session ends after 11 MiB of output. Not verified.
//!
//! The JSON fields of `machine ls --json` are not documented; the parser
//! accepts an array or `{"machines": [...]}` with `name` and
//! `state`/`status`.
//!
//! Sources: <https://github.com/smol-machines/smolvm> (README, AGENTS.md CLI
//! synopsis), <https://smolmachines.com/docs/local>,
//! `plugin-catalog/smolmachines.yaml` in NousResearch/hermes-agent.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use super::cua::{find_program, run_cli};
use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

pub struct SmolMachines;
pub static PROVIDER: SmolMachines = SmolMachines;

const DEFAULT_IMAGE: &str = "node:22-bookworm";
const INSTALL: &str = "curl -sSL https://smolmachines.com/install.sh | bash";

fn supported() -> Result<()> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) || cfg!(target_os = "linux") {
        return Ok(());
    }
    bail!("smol machines run on Apple Silicon Macs and on Linux with KVM")
}

impl Provider for SmolMachines {
    fn id(&self) -> &'static str {
        "smolmachines"
    }

    fn label(&self) -> &'static str {
        "smol machines"
    }

    fn key_url(&self) -> &'static str {
        "https://smolmachines.com/docs/local"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        false
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        if let Err(e) = supported() {
            return Probe::Unavailable(e.to_string());
        }
        if find_program("smolvm").is_none() {
            return Probe::NeedsSetup(format!("Install smolvm with `{INSTALL}`, then try again"));
        }
        if cfg!(target_os = "linux") && !Path::new("/dev/kvm").exists() {
            return Probe::Unavailable("smol machines need KVM, and /dev/kvm is missing on this machine".into());
        }
        Probe::Ready
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        supported()?;
        parse_machines(&run_cli("smolvm", &["machine", "ls", "--json"])?)
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        supported()?;
        let name = spec
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(String::from)
            .unwrap_or_else(|| format!("dove-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
        let image = spec.template.as_deref().map(str::trim).filter(|t| !t.is_empty()).unwrap_or(DEFAULT_IMAGE);
        run_cli("smolvm", &["machine", "create", "--name", &name, "--net", "--image", image])?;
        if let Err(e) = run_cli("smolvm", &["machine", "start", "--name", &name]) {
            let _ = run_cli("smolvm", &["machine", "delete", "--name", &name, "-f"]);
            return Err(e);
        }
        let found = self.list(keys).ok().and_then(|l| l.into_iter().find(|b| b.id == name));
        Ok(found.unwrap_or(BoxInfo { id: name.clone(), name, state: "running".into(), running: true }))
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        let m = self.list(keys)?.into_iter().find(|b| b.id == id);
        let m = m.with_context(|| format!("there is no smol machine named {id}"))?;
        if !m.running {
            run_cli("smolvm", &["machine", "start", "--name", id])?;
        }
        Ok(())
    }

    fn sleep(&self, _keys: &Keys, id: &str) -> Result<()> {
        supported()?;
        run_cli("smolvm", &["machine", "stop", "--name", id]).map(|_| ())
    }

    fn delete(&self, _keys: &Keys, id: &str) -> Result<()> {
        supported()?;
        run_cli("smolvm", &["machine", "delete", "--name", id, "-f"]).map(|_| ())
    }

    fn connect(&self, _keys: &Keys, id: &str, _dir: &Path) -> Result<Endpoint> {
        Ok(Endpoint::Exec { argv: exec_argv(id) })
    }
}

fn exec_argv(id: &str) -> Vec<String> {
    ["smolvm", "machine", "exec", "-i", "--stream", "--name", id, "--"].map(String::from).to_vec()
}

fn parse_machines(raw: &str) -> Result<Vec<BoxInfo>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let v: Value = serde_json::from_str(raw).context("smolvm printed a machine list Keplorayx cannot read")?;
    let items = match &v {
        Value::Array(a) => a.as_slice(),
        Value::Object(o) => {
            ["machines", "items", "data"].iter().find_map(|k| o.get(*k)?.as_array()).map_or(&[][..], |a| a)
        }
        _ => &[],
    };
    Ok(items
        .iter()
        .filter_map(|m| {
            let name = m["name"].as_str().or_else(|| m["id"].as_str())?.to_string();
            let state =
                ["state", "status"].iter().find_map(|k| m[*k].as_str()).unwrap_or("unknown").to_ascii_lowercase();
            let running = matches!(state.as_str(), "running" | "started" | "ready" | "up");
            Some(BoxInfo { id: name.clone(), name, state, running })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_machine_lists() {
        let l = parse_machines(
            r#"[{"name":"dev","state":"Running","image":"alpine"},{"name":"old","status":"stopped"},{}]"#,
        )
        .unwrap();
        assert_eq!(l[0], BoxInfo { id: "dev".into(), name: "dev".into(), state: "running".into(), running: true });
        assert_eq!((l[1].state.as_str(), l[1].running), ("stopped", false));
        assert_eq!(l.len(), 2);
        assert_eq!(parse_machines(r#"{"machines":[{"name":"a","state":"running"}]}"#).unwrap()[0].id, "a");
        assert!(parse_machines("").unwrap().is_empty());
        assert!(parse_machines("NAME STATE").is_err());
    }

    #[test]
    fn execs_with_open_stdin_and_streamed_output() {
        assert_eq!(exec_argv("dev"), ["smolvm", "machine", "exec", "-i", "--stream", "--name", "dev", "--"]);
        assert!(!exec_argv("dev").iter().any(|a| a == "-t" || a == "-it"), "never a PTY");
    }
}
