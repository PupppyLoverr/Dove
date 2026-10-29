//! Cloud sandboxes the user brings a key for (InstaBox first; see
//! docs/ENVIRONMENTS.md §3). A [`Provider`] creates, lists, wakes, sleeps and
//! deletes boxes over the provider's own API, and [`Provider::connect`] says
//! how to reach a box. Most providers hand out SSH, so a box usually becomes
//! an `ssh -F <generated config>` hop and everything else (stdio, prepare,
//! sync, port forwards) is the SSH path [`crate::host`] already has.
//!
//! Keys live in `$DOVE_HOME/cloud-keys.json` ([`keys`]); they are read only in
//! Keplorayx's own process and the launcher, never handed to an agent.

pub mod apptainer;
pub mod blaxel;
pub mod boat;
pub mod bubblewrap;
pub mod cloudflare;
pub mod coder;
pub mod codespaces;
pub mod cua;
pub mod daytona;
pub mod deepinfra;
pub mod e2b;
pub mod fly;
pub mod instabox;
pub mod keys;
pub mod kubernetes;
pub mod modal;
pub mod morph;
pub mod northflank;
pub mod ona;
pub mod railway;
pub mod runloop;
pub mod smolmachines;
pub mod sprites;
pub mod vercel;
pub mod wslc;

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub use keys::Keys;

/// What a probe found. Probes are quick and change nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum Probe {
    Ready,
    /// Usable after the user does something (paste a key, wake a box).
    NeedsSetup(String),
    Unavailable(String),
}

impl Probe {
    pub fn is_ready(&self) -> bool {
        matches!(self, Probe::Ready)
    }
}

/// One box a provider knows about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoxInfo {
    pub id: String,
    pub name: String,
    /// The provider's own state word (`running`, `suspended`, …).
    pub state: String,
    pub running: bool,
}

/// What to create. Every field is optional; providers pick their smallest
/// size and sensible auto-sleep defaults.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BoxSpec {
    pub name: Option<String>,
    pub template: Option<String>,
}

/// How to reach a box's shell. Saved as `endpoint.json` in the box's folder
/// by [`connect_box`], so [`crate::host::Host::wrap`] stays a pure rewrite.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Endpoint {
    /// Rendered into `ssh_config` and reached with the system `ssh`.
    Ssh(SshTarget),
    /// A ready `ssh_config` text that defines `Host dove-box` (from a vendor
    /// CLI such as `gh codespace ssh --config` or `coder config-ssh`).
    SshConfig { config: String },
    /// A local command that runs an argv on the box with live stdio, e.g.
    /// `["kubectl", "exec", "-i", "pod", "--"]`. The remote argv
    /// (`sh -lc 'cd "$0" && exec "$@"' <cwd> <program> <args…>`) is appended.
    Exec { argv: Vec<String> },
    /// The provider streams stdio over its own API: the launcher runs
    /// `dove node bridge <provider> <id> -- <remote argv>`, which calls
    /// [`Provider::bridge`].
    Native,
}

/// Everything an `ssh_config` `Host` block needs. Rendered by
/// [`SshTarget::config`] and written beside the box's launchers, so the
/// system `ssh` does the connecting with no key ever on a command line.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshTarget {
    pub host: String,
    pub port: Option<u16>,
    pub user: String,
    /// A private key file Keplorayx wrote (0600) for this box.
    pub identity: Option<PathBuf>,
    /// A certificate for `identity`, when the provider signs keys.
    pub certificate: Option<PathBuf>,
    /// The gateway's host key line (`host keytype base64`), when known; the
    /// first connection otherwise pins whatever the gateway presents.
    pub known_host: Option<String>,
}

/// The `Host` alias the generated config defines.
pub const SSH_ALIAS: &str = "dove-box";

impl SshTarget {
    /// An `ssh_config` with one `Host dove-box` block. `dir` holds the
    /// box's `known_hosts`.
    pub fn config(&self, dir: &Path) -> String {
        let mut c = format!("Host {SSH_ALIAS}\n  HostName {}\n  User {}\n", self.host, self.user);
        if let Some(p) = self.port {
            c.push_str(&format!("  Port {p}\n"));
        }
        if let Some(id) = &self.identity {
            c.push_str(&format!("  IdentityFile \"{}\"\n  IdentitiesOnly yes\n", id.display()));
        }
        if let Some(cert) = &self.certificate {
            c.push_str(&format!("  CertificateFile \"{}\"\n", cert.display()));
        }
        let known = dir.join("known_hosts");
        c.push_str(&format!("  UserKnownHostsFile \"{}\"\n", known.display()));
        let strict = if self.known_host.is_some() { "yes" } else { "accept-new" };
        c.push_str(&format!("  StrictHostKeyChecking {strict}\n  ServerAliveInterval 30\n  ServerAliveCountMax 4\n"));
        c
    }
}

/// A cloud sandbox provider. Calls block (they run off the UI thread and in
/// the launcher) and use short timeouts.
pub trait Provider: Send + Sync {
    /// Stable id stored in `Host::Sandbox { provider }` and the keys file.
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    /// Where the user gets a key.
    fn key_url(&self) -> &'static str;
    /// Whether this provider was tested end to end with a real account.
    fn verified(&self) -> bool;
    /// Whether it needs an API key in `cloud-keys.json` (local runtimes and
    /// providers that reuse a logged-in CLI don't).
    fn needs_key(&self) -> bool {
        true
    }
    /// Is the key there and accepted? Under a few seconds, no side effects.
    fn probe(&self, keys: &Keys) -> Probe;
    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>>;
    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo>;
    /// Resume or start `id`; a no-op when it already runs.
    fn wake(&self, keys: &Keys, id: &str) -> Result<()>;
    /// Suspend or stop `id`.
    fn sleep(&self, keys: &Keys, id: &str) -> Result<()>;
    fn delete(&self, keys: &Keys, id: &str) -> Result<()>;
    /// How to reach `id` right now. `dir` is a private folder for this box
    /// where the provider may write key files.
    fn connect(&self, keys: &Keys, id: &str, dir: &Path) -> Result<Endpoint>;
    /// For [`Endpoint::Native`]: run `argv` on the box, pumping this
    /// process's stdin/stdout/stderr to it until it exits; returns its exit
    /// code. Raw bytes, never a PTY (agents speak JSON over stdio).
    fn bridge(&self, keys: &Keys, id: &str, argv: &[String]) -> Result<i32> {
        let _ = (keys, id, argv);
        anyhow::bail!("{} has no native stream", self.label())
    }
}

/// Every provider Keplorayx knows.
pub fn providers() -> Vec<&'static dyn Provider> {
    vec![
        &instabox::INSTABOX,
        &daytona::PROVIDER,
        &boat::PROVIDER,
        &runloop::PROVIDER,
        &morph::PROVIDER,
        &fly::PROVIDER,
        &codespaces::PROVIDER,
        &coder::PROVIDER,
        &ona::PROVIDER,
        &railway::PROVIDER,
        &e2b::PROVIDER,
        &sprites::PROVIDER,
        &modal::PROVIDER,
        &vercel::PROVIDER,
        &northflank::PROVIDER,
        &blaxel::PROVIDER,
        &kubernetes::PROVIDER,
        &deepinfra::PROVIDER,
        &cloudflare::PROVIDER,
        &cua::PROVIDER,
        &apptainer::PROVIDER,
        &bubblewrap::PROVIDER,
        &wslc::PROVIDER,
        &smolmachines::PROVIDER,
    ]
}

pub fn provider(id: &str) -> Option<&'static dyn Provider> {
    providers().into_iter().find(|p| p.id() == id)
}

/// `$DOVE_HOME` (default `~/.dove`).
pub fn dove_home() -> PathBuf {
    std::env::var_os("DOVE_HOME").map(PathBuf::from).unwrap_or_else(|| {
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(|h| PathBuf::from(h).join(".dove"))
            .unwrap_or_else(|| PathBuf::from(".dove"))
    })
}

/// The private folder for one box: `$DOVE_HOME/hosts/<provider>-<id>`.
pub fn box_dir(provider: &str, id: &str) -> PathBuf {
    let slug: String = format!("{provider}-{id}")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .collect();
    dove_home().join("hosts").join(slug)
}

/// The `ssh_config` [`connect_box`] writes for a box.
pub fn ssh_config_path(provider: &str, id: &str) -> PathBuf {
    box_dir(provider, id).join("ssh_config")
}

/// The `endpoint.json` [`connect_box`] writes for a box.
pub fn endpoint_path(provider: &str, id: &str) -> PathBuf {
    box_dir(provider, id).join("endpoint.json")
}

/// The endpoint last saved for a box, if any.
pub fn saved_endpoint(provider: &str, id: &str) -> Option<Endpoint> {
    serde_json::from_slice(&std::fs::read(endpoint_path(provider, id)).ok()?).ok()
}

/// Wake `id`, ask its provider how to reach it, and save that (plus an
/// `ssh_config` for SSH routes) in the box's folder. Called by the launcher
/// before every run, so credentials that expire are refreshed.
pub fn connect_box(provider_id: &str, id: &str) -> Result<Endpoint> {
    use anyhow::Context as _;
    let p = provider(provider_id).with_context(|| format!("unknown cloud provider {provider_id}"))?;
    let keys = Keys::load();
    p.wake(&keys, id).with_context(|| format!("waking {} box {id}", p.label()))?;
    let dir = box_dir(provider_id, id);
    create_private_dir(&dir)?;
    let endpoint = p.connect(&keys, id, &dir).with_context(|| format!("connecting to {} box {id}", p.label()))?;
    match &endpoint {
        Endpoint::Ssh(target) => {
            if let Some(line) = &target.known_host {
                write_private(&dir.join("known_hosts"), format!("{line}\n").as_bytes())?;
            }
            write_private(&dir.join("ssh_config"), target.config(&dir).as_bytes())?;
        }
        Endpoint::SshConfig { config } => write_private(&dir.join("ssh_config"), config.as_bytes())?,
        Endpoint::Exec { .. } | Endpoint::Native => {}
    }
    write_private(&dir.join("endpoint.json"), &serde_json::to_vec(&endpoint)?)?;
    Ok(endpoint)
}

/// `mkdir -p` that leaves the folder readable only by this user.
pub fn create_private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Write a file only this user can read (keys, tokens, configs).
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        #[cfg(unix)]
        let mut f = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?
        };
        #[cfg(not(unix))]
        let mut f = std::fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut f, bytes)?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// A blocking HTTP client with the timeouts every provider call uses.
pub(crate) fn http() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(6))
        .user_agent(concat!("Keplorayx/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_config_has_one_block_and_pins_known_hosts() {
        let t = SshTarget {
            host: "ssh.example.com".into(),
            port: Some(2222),
            user: "box-1".into(),
            identity: Some(PathBuf::from("/k/id")),
            certificate: None,
            known_host: None,
        };
        let c = t.config(Path::new("/k"));
        assert!(c.starts_with("Host dove-box\n  HostName ssh.example.com\n  User box-1\n  Port 2222\n"));
        assert!(c.contains("IdentityFile \"/k/id\"\n  IdentitiesOnly yes"));
        assert!(c.contains("StrictHostKeyChecking accept-new"));
        assert!(c.contains(&format!("UserKnownHostsFile \"{}\"", Path::new("/k").join("known_hosts").display())));
        let pinned = SshTarget { known_host: Some("ssh.example.com ssh-ed25519 AAAA".into()), ..t };
        assert!(pinned.config(Path::new("/k")).contains("StrictHostKeyChecking yes"));
    }

    #[test]
    fn box_dirs_are_filesystem_safe() {
        let d = box_dir("instabox", "a/b:c");
        assert_eq!(d.file_name().unwrap(), "instabox-a_b_c");
    }

    #[test]
    fn registry_finds_providers_by_id() {
        assert_eq!(provider("instabox").map(|p| p.label()), Some("InstaBox"));
        assert!(provider("nope").is_none());
    }

    #[test]
    fn probe_round_trips() {
        let p = Probe::NeedsSetup("paste a key".into());
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Probe>(&json).unwrap(), p);
    }
}
