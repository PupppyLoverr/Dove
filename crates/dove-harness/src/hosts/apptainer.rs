//! Apptainer (or its older name, Singularity) instances on this machine: an
//! unprivileged container runtime common on HPC clusters and shared Linux
//! boxes.
//!
//! - `apptainer instance start --home <box>/home <image> <name>` starts a
//!   box. The image comes from [`BoxSpec::template`] (any Apptainer URI or
//!   `.sif` path). The home is a private folder in the box's own directory,
//!   so a synced checkout never lands in the user's real `$HOME`.
//! - `apptainer instance list --json` prints
//!   `{"instances":[{"instance","pid","img","ip","logErrPath","logOutPath"}]}`
//!   (the `instanceInfo` struct in `internal/app/apptainer/instance_linux.go`).
//! - `apptainer exec instance://<name> <argv…>` runs with plain inherited
//!   stdio, which is what an agent's JSON-over-stdio needs.
//! - `apptainer instance stop <name>` both sleeps and deletes; a stopped
//!   instance is forgotten by Apptainer, so Keplorayx keeps the image in
//!   `<box>/profile.json` and restarts it on wake.
//!
//! The default image is `docker://node:22-bookworm` rather than a bare
//! Ubuntu: the project is synced with `git`, and agent CLIs install with
//! `npm`, and that image has both.
//!
//! Sources: <https://apptainer.org/docs/user/latest/running_services.html>,
//! <https://www.mankier.com/1/apptainer-instance-list>,
//! <https://apptainer.org/docs/user/latest/cli/apptainer_instance_start.html>.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::cua::{find_program, run_cli};
use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

pub struct Apptainer;
pub static PROVIDER: Apptainer = Apptainer;

const DEFAULT_IMAGE: &str = "docker://node:22-bookworm";

/// What Keplorayx remembers about a box so a stopped one can start again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Profile {
    name: String,
    image: String,
}

/// `apptainer`, else the legacy `singularity` binary.
fn binary() -> Option<&'static str> {
    ["apptainer", "singularity"].into_iter().find(|b| find_program(b).is_some())
}

fn need_binary() -> Result<&'static str> {
    binary().context("Apptainer is not installed: see https://apptainer.org/docs/admin/latest/installation.html")
}

fn profile_path(name: &str) -> PathBuf {
    super::box_dir("apptainer", name).join("profile.json")
}

fn home_dir(name: &str) -> PathBuf {
    super::box_dir("apptainer", name).join("home")
}

fn load_profile(name: &str) -> Option<Profile> {
    serde_json::from_slice(&std::fs::read(profile_path(name)).ok()?).ok()
}

/// Every saved profile (stopped boxes included).
fn saved_profiles() -> Vec<Profile> {
    let Ok(entries) = std::fs::read_dir(super::dove_home().join("hosts")) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("apptainer-"))
        .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path().join("profile.json")).ok()?).ok())
        .collect()
}

/// Instance names Apptainer accepts: letters, digits, `-`, `_`, `.`.
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn start_args(home: &Path, image: &str, name: &str) -> Vec<String> {
    vec!["instance".into(), "start".into(), "--home".into(), home.display().to_string(), image.into(), name.into()]
}

impl Apptainer {
    fn running(&self) -> Result<Vec<String>> {
        let out = run_cli(need_binary()?, &["instance", "list", "--json"])?;
        parse_instances(&out)
    }

    fn start(&self, p: &Profile) -> Result<()> {
        let home = home_dir(&p.name);
        super::create_private_dir(&home)?;
        let args = start_args(&home, &p.image, &p.name);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        run_cli(need_binary()?, &refs).map(|_| ())
    }
}

impl Provider for Apptainer {
    fn id(&self) -> &'static str {
        "apptainer"
    }

    fn label(&self) -> &'static str {
        "Apptainer"
    }

    fn key_url(&self) -> &'static str {
        "https://apptainer.org/docs/user/latest/"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        false
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        if !cfg!(target_os = "linux") {
            return Probe::Unavailable("Apptainer runs on Linux only".into());
        }
        match binary() {
            Some(_) => Probe::Ready,
            None => Probe::NeedsSetup(
                "Install Apptainer (https://apptainer.org/docs/admin/latest/installation.html), then try again".into(),
            ),
        }
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        let running = self.running()?;
        Ok(merge(&running, &saved_profiles()))
    }

    fn create(&self, _keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let name = match spec.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) if valid_name(n) => n.to_string(),
            Some(n) => bail!("{n:?} cannot be an Apptainer instance name: use letters, digits, '-', '_' or '.'"),
            None => format!("dove-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
        };
        if load_profile(&name).is_some() {
            bail!("an Apptainer box named {name} already exists");
        }
        let image =
            spec.template.as_deref().map(str::trim).filter(|t| !t.is_empty()).unwrap_or(DEFAULT_IMAGE).to_string();
        let p = Profile { name: name.clone(), image };
        let dir = super::box_dir("apptainer", &name);
        super::create_private_dir(&dir)?;
        super::write_private(&profile_path(&name), &serde_json::to_vec_pretty(&p)?)?;
        if let Err(e) = self.start(&p) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
        Ok(BoxInfo { id: name.clone(), name, state: "running".into(), running: true })
    }

    fn wake(&self, _keys: &Keys, id: &str) -> Result<()> {
        if self.running()?.iter().any(|n| n == id) {
            return Ok(());
        }
        let p = load_profile(id)
            .with_context(|| format!("Apptainer instance {id} is not running and Keplorayx did not create it"))?;
        self.start(&p)
    }

    fn sleep(&self, _keys: &Keys, id: &str) -> Result<()> {
        if !self.running()?.iter().any(|n| n == id) {
            return Ok(());
        }
        run_cli(need_binary()?, &["instance", "stop", id]).map(|_| ())
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        self.sleep(keys, id)?;
        let dir = super::box_dir("apptainer", id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        }
        Ok(())
    }

    fn connect(&self, _keys: &Keys, id: &str, _dir: &Path) -> Result<Endpoint> {
        let bin = need_binary()?;
        Ok(Endpoint::Exec { argv: vec![bin.into(), "exec".into(), format!("instance://{id}")] })
    }
}

/// Instance names from `instance list --json`.
fn parse_instances(raw: &str) -> Result<Vec<String>> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let v: Value =
        serde_json::from_str(raw.trim()).context("Apptainer printed an instance list Keplorayx cannot read")?;
    Ok(v["instances"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| i["instance"].as_str().map(String::from))
        .collect())
}

/// Running instances plus the stopped boxes Keplorayx created.
fn merge(running: &[String], saved: &[Profile]) -> Vec<BoxInfo> {
    let mut out: Vec<BoxInfo> = running
        .iter()
        .map(|n| BoxInfo { id: n.clone(), name: n.clone(), state: "running".into(), running: true })
        .collect();
    for p in saved {
        if !running.contains(&p.name) {
            out.push(BoxInfo { id: p.name.clone(), name: p.name.clone(), state: "stopped".into(), running: false });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_instance_list_json() {
        let raw = r#"{
          "instances": [
            {"instance": "dove-1", "pid": 11963, "img": "/home/me/u.sif", "ip": "", "logErrPath": "/x.err", "logOutPath": "/x.out"},
            {"instance": "web", "pid": 2, "img": "/w.sif", "ip": "10.0.0.2", "logErrPath": "", "logOutPath": ""}
          ]
        }"#;
        assert_eq!(parse_instances(raw).unwrap(), ["dove-1", "web"]);
        assert_eq!(parse_instances(r#"{"instances": []}"#).unwrap(), Vec::<String>::new());
        assert_eq!(parse_instances("").unwrap(), Vec::<String>::new());
        assert!(parse_instances("INSTANCE NAME PID").is_err());
    }

    #[test]
    fn stopped_boxes_stay_listed() {
        let saved = [
            Profile { name: "dove-1".into(), image: DEFAULT_IMAGE.into() },
            Profile { name: "old".into(), image: "x.sif".into() },
        ];
        let l = merge(&["dove-1".into(), "web".into()], &saved);
        assert_eq!(
            l.iter().map(|b| (b.id.as_str(), b.running)).collect::<Vec<_>>(),
            [("dove-1", true), ("web", true), ("old", false)]
        );
        assert_eq!(l[2].state, "stopped");
    }

    #[test]
    fn starts_with_a_private_home() {
        let a = start_args(Path::new("/b/home"), "docker://node:22-bookworm", "dove-1");
        assert_eq!(a, ["instance", "start", "--home", "/b/home", "docker://node:22-bookworm", "dove-1"]);
        assert!(valid_name("dove-1.a_b") && !valid_name("a b") && !valid_name("x/y") && !valid_name(""));
    }
}
