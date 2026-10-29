//! Coder (coder.com, self-hosted): workspaces over the deployment's REST API
//! (https://coder.com/docs/reference/api/workspaces and
//! https://coder.com/docs/reference/api/builds), reached by SSH tunnelled
//! through the `coder` CLI, which must be installed.
//!
//! The key holds both halves a deployment needs, as
//! `https://coder.example.com|<session token>` (make a token with
//! `coder tokens create`). REST calls send it in `Coder-Session-Token`.
//!
//! Connecting mirrors what `coder config-ssh` writes (`cli/configssh.go`):
//! `ProxyCommand coder --global-config <dir> ssh --stdio <owner>/<workspace>`.
//! The token reaches the CLI through `<dir>/coder/{url,session}` (0600, the
//! CLI's own config layout), never a command line or the agent's
//! environment. Like `config-ssh`, host keys are not pinned: the tunnel is
//! already authenticated by the token. The SSH user is the workspace agent's
//! OS user (often `coder`), set by the template.
//!
//! Lifecycle is `POST /api/v2/workspaces/{id}/builds` with transition
//! `start`, `stop` or `delete`. Create needs a template: the box template
//! (name or id), or the deployment's only template.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider, SSH_ALIAS};

const KEY_FORMAT: &str = "https://coder.example.com|<session token>";

pub struct Coder;
pub static PROVIDER: Coder = Coder;

impl Coder {
    /// `(deployment URL, token)` from the stored key.
    fn creds<'a>(&self, keys: &'a Keys) -> Result<(&'a str, &'a str)> {
        let key = keys.get(self.id()).context("no Coder key: add one in Settings › Machines")?;
        split_key(key).with_context(|| format!("the Coder key must look like {KEY_FORMAT}"))
    }

    fn request(&self, keys: &Keys, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let (url, token) = self.creds(keys)?;
        let mut req = super::http().request(method, format!("{url}{path}")).header("Coder-Session-Token", token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().with_context(|| format!("calling Coder {path}"))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            bail!("Coder {path}: {}", api_error(status.as_u16(), &value));
        }
        Ok(value)
    }

    fn workspace(&self, keys: &Keys, id: &str) -> Result<Value> {
        self.request(keys, reqwest::Method::GET, &format!("/api/v2/workspaces/{id}"), None)
    }

    fn build(&self, keys: &Keys, id: &str, transition: &str) -> Result<()> {
        self.request(
            keys,
            reqwest::Method::POST,
            &format!("/api/v2/workspaces/{id}/builds"),
            Some(json!({ "transition": transition })),
        )
        .map(|_| ())
    }

    /// Poll until the latest build reaches `want`.
    fn wait_status(&self, keys: &Keys, id: &str, want: &str, limit: Duration) -> Result<Value> {
        let deadline = Instant::now() + limit;
        loop {
            let w = self.workspace(keys, id)?;
            let status = status(&w);
            if status == want {
                return Ok(w);
            }
            if matches!(status, "failed" | "canceled" | "deleted") {
                bail!("Coder workspace {} is {status}", w["name"].as_str().unwrap_or(id));
            }
            if Instant::now() >= deadline {
                bail!(
                    "Coder workspace {} was still {status} after {}s",
                    w["name"].as_str().unwrap_or(id),
                    limit.as_secs()
                );
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    }

    /// The template id for a create: `wanted` by name or id, else the only one.
    fn template(&self, keys: &Keys, wanted: Option<&str>) -> Result<String> {
        let list = self.request(keys, reqwest::Method::GET, "/api/v2/templates", None)?;
        pick_template(&list, wanted)
    }
}

impl Provider for Coder {
    fn id(&self) -> &'static str {
        "coder"
    }

    fn label(&self) -> &'static str {
        "Coder"
    }

    fn key_url(&self) -> &'static str {
        "https://coder.com/docs/admin/users/sessions-tokens"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, keys: &Keys) -> Probe {
        let Some(key) = keys.get(self.id()) else {
            return Probe::NeedsSetup(format!("Add your Coder URL and session token as {KEY_FORMAT}"));
        };
        if split_key(key).is_none() {
            return Probe::NeedsSetup(format!("Write the Coder key as {KEY_FORMAT}"));
        }
        if which::which("coder").is_err() {
            return Probe::NeedsSetup(
                "Install the coder CLI (https://coder.com/docs/install/cli): SSH runs through it".into(),
            );
        }
        match self.request(keys, reqwest::Method::GET, "/api/v2/users/me", None) {
            Ok(_) => Probe::Ready,
            Err(err) if err.to_string().contains("HTTP 401") => {
                Probe::NeedsSetup("Coder rejected the session token".into())
            }
            Err(err) => Probe::Unavailable(format!("{err:#}")),
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        let reply = self.request(keys, reqwest::Method::GET, "/api/v2/workspaces?q=owner:me&limit=100", None)?;
        Ok(reply["workspaces"].as_array().into_iter().flatten().filter_map(box_info).collect())
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let template_id = self.template(keys, spec.template.as_deref())?;
        let name = workspace_name(spec.name.as_deref(), &uuid::Uuid::now_v7().simple().to_string());
        let w = self.request(
            keys,
            reqwest::Method::POST,
            "/api/v2/users/me/workspaces",
            Some(json!({ "name": name, "template_id": template_id })),
        )?;
        let id = w["id"].as_str().context("Coder did not say which workspace it created")?.to_string();
        let w = self.wait_status(keys, &id, "running", Duration::from_secs(600))?;
        box_info(&w).context("Coder returned an unreadable workspace")
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        let w = self.workspace(keys, id)?;
        match status(&w) {
            "running" => return Ok(()),
            "stopped" | "failed" | "canceled" => self.build(keys, id, "start")?,
            "stopping" | "canceling" => {
                self.wait_status(keys, id, "stopped", Duration::from_secs(300))?;
                self.build(keys, id, "start")?;
            }
            "deleting" | "deleted" => bail!("Coder workspace {id} is being deleted"),
            _ => {}
        }
        self.wait_status(keys, id, "running", Duration::from_secs(600)).map(|_| ())
    }

    fn sleep(&self, keys: &Keys, id: &str) -> Result<()> {
        self.build(keys, id, "stop")
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        self.build(keys, id, "delete")
    }

    fn connect(&self, keys: &Keys, id: &str, dir: &Path) -> Result<Endpoint> {
        let coder = which::which("coder")
            .context("the coder CLI is not installed: get it from https://coder.com/docs/install/cli")?;
        let (url, token) = self.creds(keys)?;
        let w = self.workspace(keys, id)?;
        let name = w["name"].as_str().context("Coder gave a workspace without a name")?;
        let owner = w["owner_name"].as_str().context("Coder gave a workspace without an owner")?;
        let global = dir.join("coder");
        super::create_private_dir(&global)?;
        super::write_private(&global.join("url"), url.as_bytes())?;
        super::write_private(&global.join("session"), token.as_bytes())?;
        Ok(Endpoint::SshConfig { config: ssh_config(&coder, &global, owner, name) })
    }
}

/// `https://host|token` → `("https://host", "token")`.
fn split_key(key: &str) -> Option<(&str, &str)> {
    let (url, token) = key.split_once('|')?;
    let url = url.trim().trim_end_matches('/');
    let token = token.trim();
    (url.starts_with("https://") || url.starts_with("http://")).then_some(())?;
    (!token.is_empty()).then_some((url, token))
}

fn status(w: &Value) -> &str {
    w["latest_build"]["status"].as_str().unwrap_or("unknown")
}

fn box_info(w: &Value) -> Option<BoxInfo> {
    let id = w["id"].as_str()?.to_string();
    let state = status(w).to_string();
    Some(BoxInfo { name: w["name"].as_str().unwrap_or(&id).to_string(), running: state == "running", state, id })
}

fn pick_template(list: &Value, wanted: Option<&str>) -> Result<String> {
    let all = list.as_array().map(Vec::as_slice).unwrap_or_default();
    let found = match wanted {
        Some(w) => all.iter().find(|t| t["id"] == w || t["name"] == w),
        None if all.len() == 1 => all.first(),
        None => {
            let names: Vec<&str> = all.iter().filter_map(|t| t["name"].as_str()).collect();
            bail!(
                "this Coder deployment has {} templates; set the box's template to one of: {}",
                all.len(),
                names.join(", ")
            )
        }
    };
    let t = found.with_context(|| match wanted {
        Some(w) => format!("Coder has no template called {w}"),
        None => "this Coder deployment has no templates".to_string(),
    })?;
    t["id"].as_str().map(String::from).context("Coder gave a template without an id")
}

/// Coder names are lowercase letters, digits and single dashes, at most 32.
fn workspace_name(name: Option<&str>, entropy: &str) -> String {
    let base: String = name
        .unwrap_or("keplorayx")
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = if base.is_empty() { "keplorayx".to_string() } else { base };
    let tail = &entropy[entropy.len().saturating_sub(4)..];
    let base: String = base.chars().take(32 - tail.len() - 1).collect();
    format!("{}-{tail}", base.trim_end_matches('-'))
}

/// A `Host dove-box` block whose transport is `coder ssh --stdio`.
fn ssh_config(coder: &Path, global: &Path, owner: &str, workspace: &str) -> String {
    format!(
        "Host {SSH_ALIAS}\n  ProxyCommand \"{}\" --global-config \"{}\" ssh --stdio {owner}/{workspace}\n  \
         ConnectTimeout 0\n  StrictHostKeyChecking no\n  UserKnownHostsFile /dev/null\n  LogLevel ERROR\n  \
         ServerAliveInterval 30\n  ServerAliveCountMax 4\n",
        coder.display(),
        global.display()
    )
}

/// `{message, detail}` → `message: detail (HTTP n)`.
fn api_error(status: u16, v: &Value) -> String {
    let msg = v["message"].as_str().unwrap_or("request failed");
    match v["detail"].as_str().filter(|d| !d.is_empty()) {
        Some(d) => format!("{msg}: {d} (HTTP {status})"),
        None => format!("{msg} (HTTP {status})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_carry_url_and_token() {
        assert_eq!(split_key("https://coder.example.com/|abc "), Some(("https://coder.example.com", "abc")));
        assert_eq!(split_key("coder.example.com|abc"), None);
        assert_eq!(split_key("https://coder.example.com|"), None);
        assert_eq!(split_key("abc"), None);
    }

    #[test]
    fn workspaces_become_boxes() {
        let w = json!({"id": "w-1", "name": "dev", "owner_name": "me",
            "latest_build": {"status": "running", "transition": "start"}});
        assert_eq!(
            box_info(&w).unwrap(),
            BoxInfo { id: "w-1".into(), name: "dev".into(), state: "running".into(), running: true }
        );
        let stopped = json!({"id": "w-2", "name": "x", "latest_build": {"status": "stopped"}});
        assert!(!box_info(&stopped).unwrap().running);
    }

    #[test]
    fn picks_templates() {
        let list = json!([{"id": "t1", "name": "docker"}, {"id": "t2", "name": "k8s"}]);
        assert_eq!(pick_template(&list, Some("k8s")).unwrap(), "t2");
        assert_eq!(pick_template(&list, Some("t1")).unwrap(), "t1");
        assert!(pick_template(&list, None).unwrap_err().to_string().contains("docker, k8s"));
        assert!(pick_template(&list, Some("nope")).is_err());
        assert_eq!(pick_template(&json!([{"id": "only", "name": "x"}]), None).unwrap(), "only");
    }

    #[test]
    fn workspace_names_fit_coder_rules() {
        assert_eq!(workspace_name(Some("My Agent Box"), "0192abcd"), "my-agent-box-abcd");
        assert_eq!(workspace_name(None, "ffff"), "keplorayx-ffff");
        assert!(workspace_name(Some(&"a".repeat(60)), "1234").len() <= 32);
    }

    #[test]
    fn the_config_tunnels_through_the_cli() {
        let c = ssh_config(Path::new("/usr/bin/coder"), Path::new("/d/coder"), "me", "dev");
        assert!(c.starts_with(
            "Host dove-box\n  ProxyCommand \"/usr/bin/coder\" --global-config \"/d/coder\" ssh --stdio me/dev\n"
        ));
        assert_eq!(c.matches("Host ").count(), 1);
    }

    #[test]
    fn errors_read_like_sentences() {
        let v = json!({"message": "Workspace not found.", "detail": "no rows"});
        assert_eq!(api_error(404, &v), "Workspace not found.: no rows (HTTP 404)");
        assert_eq!(api_error(401, &json!({"message": "Unauthorized"})), "Unauthorized (HTTP 401)");
    }

    #[test]
    fn no_key_needs_setup() {
        assert!(matches!(PROVIDER.probe(&Keys::default()), Probe::NeedsSetup(_)));
    }
}
