//! WSL containers through `wslc.exe`, the container CLI built into WSL 2.9.3
//! and later (Windows only).
//!
//! A box is a long-lived container: `wslc run -d --name <n> <image> sleep
//! infinity`, reached with `wslc exec -i <n> <argv…>`. Sleep stops it
//! (`wslc container stop`), wake starts it (`wslc container start`), delete
//! removes it (`wslc container rm`). The default image is `node:22-bookworm`
//! because the project is synced with `git` and agent CLIs install with
//! `npm`.
//!
//! Solid (Microsoft Learn): `wslc run -d --name`, `wslc exec <name> <cmd>`,
//! `wslc container list [--all]`, `wslc container stop`, `wslc container
//! inspect`, `wslc version`. Assumed from its Docker-compatible design and
//! not shown in the docs: `exec -i`, `container start`, `container rm -f`,
//! and `container list --format json` (the text table is parsed when JSON is
//! refused).
//!
//! Sources: <https://learn.microsoft.com/windows/wsl/wsl-container>,
//! <https://learn.microsoft.com/windows/wsl/tutorials/wsl-containers>.

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use super::cua::{find_program, run_cli};
use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

pub struct Wslc;
pub static PROVIDER: Wslc = Wslc;

const DEFAULT_IMAGE: &str = "node:22-bookworm";

fn windows_only() -> Result<()> {
    if !cfg!(windows) {
        bail!("WSL containers run on Windows only");
    }
    Ok(())
}

impl Provider for Wslc {
    fn id(&self) -> &'static str {
        "wslc"
    }

    fn label(&self) -> &'static str {
        "WSL containers"
    }

    fn key_url(&self) -> &'static str {
        "https://learn.microsoft.com/windows/wsl/wsl-container"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        false
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        if !cfg!(windows) {
            return Probe::Unavailable("Windows only".into());
        }
        match find_program("wslc") {
            Some(_) => Probe::Ready,
            None => Probe::NeedsSetup("Update WSL to 2.9.3 or later with `wsl --update`, which adds wslc.exe".into()),
        }
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        windows_only()?;
        match run_cli("wslc", &["container", "list", "--all", "--format", "json"]) {
            Ok(out) => parse_json_list(&out),
            Err(_) => Ok(parse_table(&run_cli("wslc", &["container", "list", "--all"])?)),
        }
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        windows_only()?;
        let name = spec
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(String::from)
            .unwrap_or_else(|| format!("dove-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
        let image = spec.template.as_deref().map(str::trim).filter(|t| !t.is_empty()).unwrap_or(DEFAULT_IMAGE);
        let args = run_args(&name, image);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        run_cli("wslc", &refs)?;
        let found = self.list(keys).ok().and_then(|l| l.into_iter().find(|b| b.id == name));
        Ok(found.unwrap_or(BoxInfo { id: name.clone(), name, state: "running".into(), running: true }))
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        windows_only()?;
        let b = self.list(keys)?.into_iter().find(|b| b.id == id);
        let b = b.with_context(|| format!("there is no WSL container named {id}"))?;
        if !b.running {
            run_cli("wslc", &["container", "start", id])?;
        }
        Ok(())
    }

    fn sleep(&self, _keys: &Keys, id: &str) -> Result<()> {
        windows_only()?;
        run_cli("wslc", &["container", "stop", id]).map(|_| ())
    }

    fn delete(&self, _keys: &Keys, id: &str) -> Result<()> {
        windows_only()?;
        run_cli("wslc", &["container", "rm", "-f", id]).map(|_| ())
    }

    fn connect(&self, _keys: &Keys, id: &str, _dir: &Path) -> Result<Endpoint> {
        windows_only()?;
        Ok(Endpoint::Exec { argv: ["wslc", "exec", "-i", id].map(String::from).to_vec() })
    }
}

/// `wslc run -d` for a container that stays up between runs.
fn run_args(name: &str, image: &str) -> Vec<String> {
    ["run", "-d", "--name", name, image, "sleep", "infinity"].map(String::from).to_vec()
}

fn is_running(state: &str) -> bool {
    let s = state.to_ascii_lowercase();
    s.starts_with("up") || s.starts_with("running")
}

/// JSON lines or an array, Docker-style (`Names`/`Name`, `State`/`Status`).
fn parse_json_list(raw: &str) -> Result<Vec<BoxInfo>> {
    let raw = raw.trim();
    let items: Vec<Value> = if raw.starts_with('[') {
        serde_json::from_str(raw).context("wslc printed a container list Keplorayx cannot read")?
    } else {
        raw.lines()
            .filter(|l| !l.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()
            .context("wslc printed a container list Keplorayx cannot read")?
    };
    Ok(items
        .iter()
        .filter_map(|c| {
            let field = |keys: &[&str]| keys.iter().find_map(|k| c[*k].as_str().or_else(|| c[*k][0].as_str()));
            let name = field(&["Names", "Name", "name", "names"])?.trim_start_matches('/').to_string();
            let state = field(&["State", "state", "Status", "status"]).unwrap_or("unknown").to_string();
            Some(BoxInfo { id: name.clone(), name, running: is_running(&state), state })
        })
        .collect())
}

/// The `container list` table: columns located by the header line.
fn parse_table(raw: &str) -> Vec<BoxInfo> {
    let mut lines = raw.lines().filter(|l| !l.trim().is_empty());
    let Some(header) = lines.next() else { return Vec::new() };
    let col = |names: &[&str]| names.iter().find_map(|n| header.find(n));
    let (Some(name_at), Some(status_at)) = (col(&["NAMES", "NAME"]), col(&["STATUS", "STATE"])) else {
        return Vec::new();
    };
    // Every column start, to know where the status column ends.
    let mut starts: Vec<usize> = header
        .char_indices()
        .filter(|(i, c)| !c.is_whitespace() && (*i == 0 || header[..*i].ends_with("  ")))
        .map(|(i, _)| i)
        .collect();
    starts.sort_unstable();
    let end_of = |at: usize| starts.iter().copied().find(|s| *s > at);
    let cell = |line: &str, at: usize| {
        let end = end_of(at).unwrap_or(line.len()).min(line.len());
        line.get(at.min(line.len())..end).unwrap_or("").trim().to_string()
    };
    lines
        .filter_map(|l| {
            let name = cell(l, name_at);
            let name = name.split_whitespace().next()?.to_string();
            let state = cell(l, status_at);
            Some(BoxInfo { id: name.clone(), name, running: is_running(&state), state })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_json_lists() {
        let lines = r#"{"ID":"1a","Image":"node:22","Names":"dove-1","State":"running","Status":"Up 2 minutes"}
{"ID":"2b","Image":"nginx","Names":"web","State":"exited","Status":"Exited (0)"}"#;
        let l = parse_json_list(lines).unwrap();
        assert_eq!(
            l[0],
            BoxInfo { id: "dove-1".into(), name: "dove-1".into(), state: "running".into(), running: true }
        );
        assert!(!l[1].running);
        let arr = parse_json_list(r#"[{"Names":["/dev"],"Status":"Up 1 hour"}]"#).unwrap();
        assert_eq!((arr[0].id.as_str(), arr[0].running), ("dev", true));
        assert!(parse_json_list("CONTAINER ID  IMAGE").is_err());
    }

    #[test]
    fn reads_the_text_table() {
        let t = "CONTAINER ID   IMAGE              COMMAND            STATUS          NAMES\n\
                 1a2b3c         node:22-bookworm   \"sleep infinity\"   Up 3 minutes    dove-1\n\
                 4d5e6f         nginx              \"nginx\"            Exited (0) 1h   web\n";
        let l = parse_table(t);
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].id.as_str(), l[0].state.as_str(), l[0].running), ("dove-1", "Up 3 minutes", true));
        assert_eq!((l[1].id.as_str(), l[1].running), ("web", false));
        assert!(parse_table("").is_empty());
    }

    #[test]
    fn creates_a_container_that_stays_up() {
        assert_eq!(
            run_args("dove-1", "node:22-bookworm"),
            ["run", "-d", "--name", "dove-1", "node:22-bookworm", "sleep", "infinity"]
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn elsewhere_it_is_unavailable() {
        assert_eq!(PROVIDER.probe(&Keys::default()), Probe::Unavailable("Windows only".into()));
        assert!(PROVIDER.list(&Keys::default()).is_err());
    }
}
