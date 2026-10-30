//! Bubblewrap (`bwrap`) sandboxes on this Linux machine: no daemon, no image,
//! just a private home over the system's own tools.
//!
//! A box is a profile in `<box dir>/profile.json` with a private home in
//! `<box dir>/home`. Connecting renders the full `bwrap` argv:
//!
//! - `/usr` and `/etc` read-only; `/bin`, `/sbin`, `/lib*` as the same
//!   symlinks the host has (merged-/usr systems) or read-only binds;
//!   `/opt` and `/run/systemd/resolve` (DNS) read-only when present;
//! - a fresh `/proc` and `/dev`, the host's `/tmp`, and the host network;
//! - the private home bound at the user's real `$HOME` path, so `PATH`
//!   entries and shebangs that name `~/.local/bin`, `~/.nvm/…` and the npm
//!   prefix stay valid; those tool folders are bound read-only on top so
//!   `git` and the user's agent CLIs (npm globals, the Claude Code native
//!   install in `~/.local/share/claude`) run inside;
//! - `--unshare-pid --die-with-parent`, so nothing outlives the run.
//!
//! The agent's own login (`~/.claude`, `~/.codex`, …) is deliberately not
//! shared: a box starts signed out, like any other fresh machine.
//!
//! Sources: <https://github.com/containers/bubblewrap> (README and
//! `bwrap.xml` man page), <https://wiki.archlinux.org/title/Bubblewrap>.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use super::cua::{find_program, run_cli};
use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

pub struct Bubblewrap;
pub static PROVIDER: Bubblewrap = Bubblewrap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Profile {
    id: String,
    name: String,
}

/// Tool folders under `$HOME` that hold agent CLIs and their runtimes.
const HOME_TOOLS: &[&str] = &[
    ".local/bin",
    ".local/share/claude",
    ".local/share/pnpm",
    ".npm-global",
    ".nvm",
    ".volta",
    ".bun",
    ".deno",
    ".cargo/bin",
    ".opencode",
];

fn profile_path(id: &str) -> PathBuf {
    super::box_dir("bubblewrap", id).join("profile.json")
}

fn real_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute())
}

/// `NPM_CONFIG_PREFIX`, else `prefix=` in `~/.npmrc` (without running npm).
fn npm_prefix(home: &Path) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("NPM_CONFIG_PREFIX").or_else(|| std::env::var_os("npm_config_prefix")) {
        return Some(PathBuf::from(p));
    }
    let rc = std::fs::read_to_string(home.join(".npmrc")).ok()?;
    parse_npmrc_prefix(&rc, home)
}

fn parse_npmrc_prefix(rc: &str, home: &Path) -> Option<PathBuf> {
    rc.lines().find_map(|l| {
        let (k, v) = l.trim().split_once('=')?;
        if k.trim() != "prefix" {
            return None;
        }
        let v = v.trim().trim_matches('"');
        Some(match v.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(v.replace("${HOME}", &home.display().to_string())),
        })
    })
}

/// How a top-level system path appears in the sandbox.
#[derive(Debug, PartialEq, Eq)]
enum Root {
    Symlink(String),
    Dir,
    Missing,
}

fn root_kind(p: &Path) -> Root {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.file_type().is_symlink() => {
            std::fs::read_link(p).map(|t| Root::Symlink(t.display().to_string())).unwrap_or(Root::Missing)
        }
        Ok(m) if m.is_dir() => Root::Dir,
        _ => Root::Missing,
    }
}

/// The full `bwrap` argv for a box whose private home is `home`, shown to
/// the guest at `guest_home`. `roots` says how `/bin`, `/lib`… look here;
/// `tools` are host folders bound read-only at the same path.
fn bwrap_argv(home: &Path, guest_home: &Path, roots: &[(&str, Root)], tools: &[PathBuf]) -> Vec<String> {
    let mut a: Vec<String> = vec!["bwrap".into()];
    let mut push = |xs: &[&str]| a.extend(xs.iter().map(|s| s.to_string()));
    push(&["--ro-bind", "/usr", "/usr"]);
    for (path, kind) in roots {
        match kind {
            Root::Symlink(target) => push(&["--symlink", target, path]),
            Root::Dir => push(&["--ro-bind", path, path]),
            Root::Missing => {}
        }
    }
    push(&["--ro-bind", "/etc", "/etc", "--proc", "/proc", "--dev", "/dev", "--bind", "/tmp", "/tmp"]);
    let home_s = home.display().to_string();
    let guest_s = guest_home.display().to_string();
    push(&["--bind", &home_s, &guest_s]);
    for t in tools {
        let t = t.display().to_string();
        push(&["--ro-bind", &t, &t]);
    }
    push(&["--setenv", "HOME", &guest_s, "--unshare-pid", "--die-with-parent", "--"]);
    a
}

/// The argv for this machine as it is right now.
fn current_argv(id: &str) -> Result<Vec<String>> {
    let guest = real_home().context("HOME is not set, so Bubblewrap has no home to stand in for")?;
    let home = super::box_dir("bubblewrap", id).join("home");
    super::create_private_dir(&home)?;
    let roots: Vec<(&str, Root)> =
        ["/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32", "/opt", "/run/systemd/resolve"]
            .into_iter()
            .map(|p| (p, root_kind(Path::new(p))))
            .collect();
    let mut tools: Vec<PathBuf> = HOME_TOOLS.iter().map(|t| guest.join(t)).collect();
    // An npm prefix outside /usr (e.g. ~/.npm-global or /opt/npm) holds the
    // agent CLIs installed with `npm install -g`.
    tools.extend(npm_prefix(&guest).filter(|p| !p.starts_with("/usr")));
    tools.retain(|t| t.is_dir());
    tools.dedup();
    Ok(bwrap_argv(&home, &guest, &roots, &tools))
}

/// A filesystem-safe id from a name, or a fresh one.
fn box_id(name: Option<&str>) -> String {
    let slug: String = name
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() { format!("dove-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]) } else { slug }
}

fn profiles_in(hosts: &Path) -> Vec<Profile> {
    let Ok(entries) = std::fs::read_dir(hosts) else { return Vec::new() };
    let mut out: Vec<Profile> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("bubblewrap-"))
        .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path().join("profile.json")).ok()?).ok())
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn info(p: &Profile) -> BoxInfo {
    // Nothing runs between agent runs, so a box is always ready.
    BoxInfo { id: p.id.clone(), name: p.name.clone(), state: "ready".into(), running: true }
}

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
        true
    }

    fn needs_key(&self) -> bool {
        false
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        if !cfg!(target_os = "linux") {
            return Probe::Unavailable("Bubblewrap runs on Linux only".into());
        }
        if find_program("bwrap").is_none() {
            return Probe::NeedsSetup(
                "Install Bubblewrap (`sudo apt install bubblewrap`, `sudo dnf install bubblewrap` or `sudo pacman -S bubblewrap`)"
                    .into(),
            );
        }
        // Unprivileged user namespaces can be switched off (sysctl, AppArmor).
        match run_cli("bwrap", &["--ro-bind", "/", "/", "--dev", "/dev", "--unshare-pid", "true"]) {
            Ok(_) => Probe::Ready,
            Err(e) => Probe::Unavailable(format!("Bubblewrap cannot create a sandbox here: {e:#}")),
        }
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        Ok(profiles_in(&super::dove_home().join("hosts")).iter().map(info).collect())
    }

    fn create(&self, _keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        if !cfg!(target_os = "linux") {
            bail!("Bubblewrap runs on Linux only");
        }
        let id = box_id(spec.name.as_deref());
        if profile_path(&id).exists() {
            bail!("a Bubblewrap box named {id} already exists");
        }
        let p = Profile { name: spec.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| id.clone()), id };
        let dir = super::box_dir("bubblewrap", &p.id);
        super::create_private_dir(&dir.join("home"))?;
        super::create_private_dir(&dir)?;
        super::write_private(&profile_path(&p.id), &serde_json::to_vec_pretty(&p)?)?;
        Ok(info(&p))
    }

    fn wake(&self, _keys: &Keys, id: &str) -> Result<()> {
        if !profile_path(id).exists() {
            bail!("there is no Bubblewrap box named {id}");
        }
        Ok(())
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        // Every process dies with its run (`--die-with-parent`).
        Ok(())
    }

    fn delete(&self, _keys: &Keys, id: &str) -> Result<()> {
        let dir = super::box_dir("bubblewrap", id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        }
        Ok(())
    }

    fn connect(&self, _keys: &Keys, id: &str, _dir: &Path) -> Result<Endpoint> {
        if !profile_path(id).exists() {
            bail!("there is no Bubblewrap box named {id}");
        }
        Ok(Endpoint::Exec { argv: current_argv(id)? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_has_a_private_home_and_readonly_system() {
        let roots = [("/bin", Root::Symlink("usr/bin".into())), ("/lib64", Root::Dir), ("/libx32", Root::Missing)];
        let tools = [PathBuf::from("/home/me/.local/bin")];
        let a = bwrap_argv(Path::new("/d/home"), Path::new("/home/me"), &roots, &tools);
        let line = a.join(" ");
        assert!(
            line.starts_with("bwrap --ro-bind /usr /usr --symlink usr/bin /bin --ro-bind /lib64 /lib64 --ro-bind /etc")
        );
        assert!(!line.contains("libx32"));
        assert!(
            line.contains("--bind /tmp /tmp --bind /d/home /home/me --ro-bind /home/me/.local/bin /home/me/.local/bin")
        );
        assert!(line.ends_with("--setenv HOME /home/me --unshare-pid --die-with-parent --"));
        assert!(!a.iter().any(|x| x == "--unshare-net"), "the network stays shared");
    }

    #[test]
    fn npmrc_prefix() {
        let h = Path::new("/home/me");
        assert_eq!(parse_npmrc_prefix("a=1\nprefix=~/.npm-global\n", h), Some(PathBuf::from("/home/me/.npm-global")));
        assert_eq!(parse_npmrc_prefix("prefix = \"${HOME}/n\"", h), Some(PathBuf::from("/home/me/n")));
        assert_eq!(parse_npmrc_prefix("registry=x", h), None);
    }

    #[test]
    fn ids_are_slugs() {
        assert_eq!(box_id(Some("My Box!")), "my-box");
        assert!(box_id(None).starts_with("dove-"));
        assert!(box_id(Some("  ")).starts_with("dove-"));
    }

    #[test]
    fn profiles_on_disk_are_boxes() {
        let root = std::env::temp_dir().join(format!("dove-bwrap-test-{}", std::process::id()));
        for (dir, json) in [
            ("bubblewrap-b", r#"{"id":"b","name":"B"}"#),
            ("bubblewrap-a", r#"{"id":"a","name":"a"}"#),
            ("apptainer-x", r#"{"id":"x","name":"x"}"#),
            ("bubblewrap-broken", "{"),
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join("profile.json"), json).unwrap();
        }
        let ids: Vec<String> = profiles_in(&root).into_iter().map(|p| p.id).collect();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn root_kinds() {
        assert_eq!(root_kind(Path::new("/definitely/not/here")), Root::Missing);
        #[cfg(unix)]
        assert_eq!(root_kind(Path::new("/usr")), Root::Dir);
        #[cfg(windows)]
        assert_eq!(root_kind(Path::new(r"C:\Windows")), Root::Dir);
    }
}
