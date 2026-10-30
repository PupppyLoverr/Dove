//! Ona (formerly Gitpod, ona.com): environments over Ona's Connect API
//! (https://ona.com/docs/api-reference.md; message shapes from
//! github.com/gitpod-io/gitpod-sdk-go), reached by SSH through the `ona`
//! CLI (https://ona.com/docs/ona/integrations/cli.md).
//!
//! Lifecycle is REST: `POST https://app.ona.com/api/gitpod.v1.EnvironmentService/<Method>`
//! with a bearer personal access token and protobuf-JSON bodies
//! (`ListEnvironments`, `GetEnvironment`, `CreateEnvironment`,
//! `StartEnvironment`, `StopEnvironment`, `DeleteEnvironment`).
//!
//! SSH: Ona documents no gateway address or user, only that
//! `ona environment ssh-config` writes `~/.ssh/ona/config` (older CLIs:
//! `gitpod` and `~/.ssh/gitpod/config`) with a `<envid>.ona.environment`
//! host whose `ProxyCommand` runs the CLI. Rather than guess its contents,
//! connect regenerates that file, asks the system `ssh -G` how it resolves
//! the environment's host, and copies the resolved route (HostName, User,
//! ProxyCommand, keys, host-key settings) into a `Host dove-box` block. The
//! CLI must be installed and signed in (`ona login`); the probe says so.
//! Unverified end to end: whether the CLI's proxy reads `ONA_TOKEN` from the
//! environment (it is set for the regenerate step only).
//!
//! Create needs a repository: the box template is a clone URL or
//! `owner/repo` (GitHub). The first enabled environment class is used.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider, SSH_ALIAS};

const API: &str = "https://app.ona.com/api/gitpod.v1.EnvironmentService";
const PHASE: &str = "ENVIRONMENT_PHASE_";

pub struct Ona;
pub static PROVIDER: Ona = Ona;

/// The installed CLI (`ona`, or `gitpod` from before the rename) and the
/// SSH host suffix and config folder it uses.
fn cli() -> Option<(PathBuf, &'static str, &'static str)> {
    which::which("ona")
        .map(|p| (p, "ona.environment", "ona"))
        .or_else(|_| which::which("gitpod").map(|p| (p, "gitpod.environment", "gitpod")))
        .ok()
}

impl Ona {
    fn key<'a>(&self, keys: &'a Keys) -> Result<&'a str> {
        keys.get(self.id()).context("no Ona token: add one in Settings › Machines")
    }

    fn call(&self, keys: &Keys, method: &str, body: Value) -> Result<Value> {
        let key = self.key(keys)?;
        let resp = super::http()
            .post(format!("{API}/{method}"))
            .bearer_auth(key)
            .header("Connect-Protocol-Version", "1")
            .json(&body)
            .send()
            .with_context(|| format!("calling Ona {method}"))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            bail!("Ona {method}: {}", api_error(status.as_u16(), &value));
        }
        Ok(value)
    }

    fn environment(&self, keys: &Keys, id: &str) -> Result<Value> {
        Ok(self.call(keys, "GetEnvironment", json!({ "environmentId": id }))?["environment"].clone())
    }

    /// Poll until the environment's phase is `want` (`running`, `stopped`…).
    fn wait_phase(&self, keys: &Keys, id: &str, want: &str, limit: Duration) -> Result<()> {
        let deadline = Instant::now() + limit;
        loop {
            let env = self.environment(keys, id)?;
            let p = phase(&env);
            if p == want {
                return Ok(());
            }
            if matches!(p.as_str(), "deleting" | "deleted") {
                bail!("Ona environment {id} is {p}");
            }
            if let Some(why) =
                env["status"]["failureMessage"].as_array().and_then(|f| f.first()).and_then(Value::as_str)
            {
                bail!("Ona environment {id} failed: {why}");
            }
            if Instant::now() >= deadline {
                bail!("Ona environment {id} was still {p} after {}s", limit.as_secs());
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    }
}

impl Provider for Ona {
    fn id(&self) -> &'static str {
        "ona"
    }

    fn label(&self) -> &'static str {
        "Ona (Gitpod)"
    }

    fn key_url(&self) -> &'static str {
        "https://app.ona.com/settings/personal-access-tokens"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, keys: &Keys) -> Probe {
        if !keys.has(self.id()) {
            return Probe::NeedsSetup("Add an Ona personal access token".into());
        }
        if cli().is_none() {
            return Probe::NeedsSetup(
                "Install the ona CLI and run `ona login` (https://ona.com/docs/ona/integrations/cli): SSH runs through it"
                    .into(),
            );
        }
        match self.call(keys, "ListEnvironments", json!({ "pagination": { "pageSize": 1 } })) {
            Ok(_) => Probe::Ready,
            Err(err) if err.to_string().contains("unauthenticated") || err.to_string().contains("HTTP 401") => {
                Probe::NeedsSetup("Ona rejected the token".into())
            }
            Err(err) => Probe::Unavailable(format!("{err:#}")),
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..5 {
            let mut pagination = json!({ "pageSize": 100 });
            if let Some(t) = &token {
                pagination["token"] = json!(t);
            }
            let page = self.call(keys, "ListEnvironments", json!({ "pagination": pagination }))?;
            out.extend(page["environments"].as_array().into_iter().flatten().filter_map(box_info));
            token = page["pagination"]["nextToken"].as_str().filter(|t| !t.is_empty()).map(String::from);
            if token.is_none() {
                break;
            }
        }
        Ok(out)
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let repo = spec
            .template
            .as_deref()
            .context("an Ona environment needs a repository: set the box's template to its URL or owner/repo")?;
        let classes = self.call(keys, "ListEnvironmentClasses", json!({}))?;
        let class = first_class(&classes).context("your Ona organization has no enabled environment class")?;
        let reply = self.call(keys, "CreateEnvironment", create_request(&context_url(repo), &class))?;
        let id =
            reply["environment"]["id"].as_str().context("Ona did not say which environment it created")?.to_string();
        self.wait_phase(keys, &id, "running", Duration::from_secs(600))?;
        box_info(&self.environment(keys, &id)?).context("Ona returned an unreadable environment")
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        let env = self.environment(keys, id)?;
        match phase(&env).as_str() {
            "running" => return Ok(()),
            "stopping" => {
                self.wait_phase(keys, id, "stopped", Duration::from_secs(300))?;
                self.call(keys, "StartEnvironment", json!({ "environmentId": id }))?;
            }
            "stopped" | "unspecified" => {
                self.call(keys, "StartEnvironment", json!({ "environmentId": id }))?;
            }
            p @ ("deleting" | "deleted") => bail!("Ona environment {id} is {p}"),
            _ => {}
        }
        self.wait_phase(keys, id, "running", Duration::from_secs(600))
    }

    fn sleep(&self, keys: &Keys, id: &str) -> Result<()> {
        self.call(keys, "StopEnvironment", json!({ "environmentId": id })).map(|_| ())
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        self.call(keys, "DeleteEnvironment", json!({ "environmentId": id, "force": false })).map(|_| ())
    }

    fn connect(&self, keys: &Keys, id: &str, _dir: &Path) -> Result<Endpoint> {
        let (exe, suffix, folder) =
            cli().context("the ona CLI is not installed: see https://ona.com/docs/ona/integrations/cli")?;
        let token = self.key(keys)?;
        let mut regen = Command::new(&exe);
        regen.args(["environment", "ssh-config"]).env("ONA_TOKEN", token).env("GITPOD_TOKEN", token);
        super::codespaces::run_cli(&mut regen, Duration::from_secs(30))
            .context("`ona environment ssh-config` failed (is the CLI signed in? run `ona login`)")?;
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).context("no home folder")?;
        let file = PathBuf::from(home).join(".ssh").join(folder).join("config");
        if !file.exists() {
            bail!("the ona CLI wrote no SSH config at {}", file.display());
        }
        let host = format!("{id}.{suffix}");
        let mut resolve = Command::new("ssh");
        resolve.arg("-G").arg("-F").arg(&file).arg(&host);
        let resolved = super::codespaces::run_cli(&mut resolve, Duration::from_secs(10)).context("running ssh -G")?;
        Ok(Endpoint::SshConfig { config: config_from_resolved(&resolved, &host)? })
    }
}

/// `ENVIRONMENT_PHASE_RUNNING` → `running`.
fn phase(env: &Value) -> String {
    env["status"]["phase"]
        .as_str()
        .unwrap_or("ENVIRONMENT_PHASE_UNSPECIFIED")
        .trim_start_matches(PHASE)
        .to_ascii_lowercase()
}

fn box_info(env: &Value) -> Option<BoxInfo> {
    let id = env["id"].as_str()?.to_string();
    let state = phase(env);
    Some(BoxInfo {
        name: env["metadata"]["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&id).to_string(),
        running: state == "running",
        state,
        id,
    })
}

/// `owner/repo` → its GitHub URL; anything with a scheme passes through.
fn context_url(repo: &str) -> String {
    let r = repo.trim();
    if r.contains("://") { r.to_string() } else { format!("https://github.com/{}", r.trim_end_matches(".git")) }
}

fn first_class(reply: &Value) -> Option<String> {
    reply["environmentClasses"]
        .as_array()?
        .iter()
        .find(|c| c["enabled"] != false)
        .and_then(|c| c["id"].as_str())
        .map(String::from)
}

fn create_request(url: &str, class: &str) -> Value {
    json!({
        "spec": {
            "desiredPhase": "ENVIRONMENT_PHASE_RUNNING",
            "machine": { "class": class },
            "content": { "initializer": { "specs": [ { "contextUrl": { "url": url } } ] } }
        }
    })
}

/// A `Host dove-box` block from `ssh -G <host>` output: the options that
/// make up the route, copied as resolved. Fails when nothing routes it.
fn config_from_resolved(resolved: &str, host: &str) -> Result<String> {
    const KEEP: &[(&str, &str)] = &[
        ("hostname", "HostName"),
        ("user", "User"),
        ("port", "Port"),
        ("proxycommand", "ProxyCommand"),
        ("proxyjump", "ProxyJump"),
        ("identityfile", "IdentityFile"),
        ("certificatefile", "CertificateFile"),
        ("identitiesonly", "IdentitiesOnly"),
        ("hostkeyalias", "HostKeyAlias"),
        ("userknownhostsfile", "UserKnownHostsFile"),
        ("stricthostkeychecking", "StrictHostKeyChecking"),
    ];
    let mut out = format!("Host {SSH_ALIAS}\n");
    let mut routed = false;
    for line in resolved.lines() {
        let Some((key, value)) = line.trim().split_once(' ') else { continue };
        let value = value.trim();
        let Some((_, name)) = KEEP.iter().find(|(k, _)| *k == key) else { continue };
        if value.is_empty() || value == "none" {
            continue;
        }
        if key == "proxycommand" || key == "proxyjump" {
            routed = true;
        }
        out.push_str(&format!("  {name} {value}\n"));
    }
    if !routed {
        bail!("the ona CLI's SSH config has no route to {host}");
    }
    out.push_str("  ServerAliveInterval 30\n  ServerAliveCountMax 4\n");
    Ok(out)
}

/// Connect errors: `{"code": "unauthenticated", "message": "…"}`.
fn api_error(status: u16, v: &Value) -> String {
    let code = v["code"].as_str().unwrap_or("error");
    let msg = v["message"].as_str().unwrap_or("request failed");
    format!("{code}: {msg} (HTTP {status})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environments_become_boxes() {
        let env =
            json!({"id": "019a-env", "metadata": {"name": "api"}, "status": {"phase": "ENVIRONMENT_PHASE_RUNNING"}});
        assert_eq!(
            box_info(&env).unwrap(),
            BoxInfo { id: "019a-env".into(), name: "api".into(), state: "running".into(), running: true }
        );
        let stopped = json!({"id": "e2", "status": {"phase": "ENVIRONMENT_PHASE_STOPPED"}});
        assert_eq!(box_info(&stopped).unwrap().state, "stopped");
        assert_eq!(phase(&json!({})), "unspecified");
    }

    #[test]
    fn builds_create_requests() {
        assert_eq!(context_url("octo/hello"), "https://github.com/octo/hello");
        assert_eq!(context_url("https://gitlab.com/a/b"), "https://gitlab.com/a/b");
        let classes = json!({"environmentClasses": [{"id": "c0", "enabled": false}, {"id": "c1", "enabled": true}]});
        assert_eq!(first_class(&classes).as_deref(), Some("c1"));
        let r = create_request("https://github.com/o/r", "c1");
        assert_eq!(r["spec"]["machine"]["class"], "c1");
        assert_eq!(r["spec"]["content"]["initializer"]["specs"][0]["contextUrl"]["url"], "https://github.com/o/r");
    }

    #[test]
    fn copies_the_resolved_route() {
        let g = "host 019a.ona.environment\nhostname 019a.ona.environment\nuser gitpod_devcontainer\nport 22\n\
                 proxycommand /usr/local/bin/ona environment ssh-proxy %h\nproxyjump none\n\
                 identityfile ~/.ssh/ona/key\nstricthostkeychecking false\nforwardagent no\n";
        let c = config_from_resolved(g, "019a.ona.environment").unwrap();
        assert!(
            c.starts_with("Host dove-box\n  HostName 019a.ona.environment\n  User gitpod_devcontainer\n  Port 22\n")
        );
        assert!(c.contains("  ProxyCommand /usr/local/bin/ona environment ssh-proxy %h\n"));
        assert!(c.contains("  IdentityFile ~/.ssh/ona/key\n"));
        assert!(!c.contains("ProxyJump") && !c.contains("forwardagent"));
        assert!(config_from_resolved("hostname x\nuser y\n", "x").is_err());
    }

    #[test]
    fn errors_read_like_sentences() {
        let v = json!({"code": "unauthenticated", "message": "invalid token"});
        assert_eq!(api_error(401, &v), "unauthenticated: invalid token (HTTP 401)");
    }

    #[test]
    fn no_key_needs_setup() {
        assert!(matches!(PROVIDER.probe(&Keys::default()), Probe::NeedsSetup(_)));
    }
}
