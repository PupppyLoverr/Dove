//! Fly Machines (fly.io): one app with one machine per box, over the Machines
//! REST API (`api.machines.dev`, spec at
//! https://docs.fly.io/api/machines/openapi.json, guide at
//! https://docs.fly.io/machines/api), reached with plain SSH.
//!
//! Route chosen: Fly machines have no SSH gateway of their own, and
//! `flyctl ssh console -C` needs flyctl plus a WireGuard agent and takes the
//! command as one string, so Keplorayx runs its own sshd instead. A box is
//! an app `keplorayx-…` holding one machine of
//! `lscr.io/linuxserver/openssh-server` (Alpine; sshd on 2222, user `dove`,
//! home `/config`, see https://docs.linuxserver.io/images/docker-openssh-server/)
//! with Keplorayx's public key in `PUBLIC_KEY` and `git` added at boot via the
//! image's package mod. The app gets a free dedicated IPv6
//! (`POST /v1/apps/{app}/ip_assignments`) and a raw-TCP service mapping public
//! port 22 to 2222, so `ssh dove@<app>.fly.dev` works from any IPv6-capable
//! network. The service auto-suspends when idle and Fly's proxy wakes it on
//! the next connection.
//!
//! Caveats: a machine's root filesystem is reset on a full *stop* (suspend
//! keeps it), so sleep uses suspend and the code is re-synced if the disk was
//! lost. Agents installed there must run on musl. A template overrides the
//! image (it must run sshd on 2222 and honour `PUBLIC_KEY`). The key may be a
//! personal token or a `FlyV1 …` token; boxes are made in the `personal` org.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider, SshTarget};

const API: &str = "https://api.machines.dev";
const ORG: &str = "personal";
const PREFIX: &str = "keplorayx-";
const IMAGE: &str = "lscr.io/linuxserver/openssh-server:latest";
const SSH_USER: &str = "dove";

pub struct Fly;
pub static PROVIDER: Fly = Fly;

impl Fly {
    fn key<'a>(&self, keys: &'a Keys) -> Result<&'a str> {
        keys.get(self.id()).context("no Fly API token: add one in Settings › Machines")
    }

    fn request(&self, keys: &Keys, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value> {
        let key = self.key(keys)?;
        let mut req = super::http().request(method, format!("{API}{path}")).header("Authorization", auth_header(key));
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().with_context(|| format!("calling Fly {path}"))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            bail!("Fly {path}: {}", api_error(status.as_u16(), &value, &text));
        }
        Ok(value)
    }

    /// The box's one machine.
    fn machine(&self, keys: &Keys, app: &str) -> Result<Value> {
        let list = self.request(keys, reqwest::Method::GET, &format!("/v1/apps/{app}/machines"), None)?;
        list.as_array().and_then(|m| m.first()).cloned().with_context(|| format!("Fly app {app} has no machine"))
    }

    /// Poll until the machine's state is `want`.
    fn wait_state(&self, keys: &Keys, app: &str, want: &str, limit: Duration) -> Result<Value> {
        let deadline = Instant::now() + limit;
        loop {
            let m = self.machine(keys, app)?;
            let state = m["state"].as_str().unwrap_or("unknown");
            if state == want {
                return Ok(m);
            }
            if matches!(state, "failed" | "launch_failed" | "destroyed") {
                bail!("Fly machine in {app} is {state}");
            }
            if Instant::now() >= deadline {
                bail!("Fly machine in {app} was still {state} after {}s", limit.as_secs());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

impl Provider for Fly {
    fn id(&self) -> &'static str {
        "fly"
    }

    fn label(&self) -> &'static str {
        "Fly Machines"
    }

    fn key_url(&self) -> &'static str {
        "https://fly.io/user/personal_access_tokens"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, keys: &Keys) -> Probe {
        if !keys.has(self.id()) {
            return Probe::NeedsSetup("Add a Fly.io access token (fly tokens create org personal)".into());
        }
        match self.request(keys, reqwest::Method::GET, &format!("/v1/apps?org_slug={ORG}"), None) {
            Ok(_) => Probe::Ready,
            Err(err) if err.to_string().contains("HTTP 401") || err.to_string().contains("HTTP 403") => {
                Probe::NeedsSetup("Fly rejected the token".into())
            }
            Err(err) => Probe::Unavailable(format!("{err:#}")),
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        let apps = self.request(keys, reqwest::Method::GET, &format!("/v1/apps?org_slug={ORG}"), None)?;
        let mut out = Vec::new();
        for app in our_apps(&apps) {
            let state = match self.machine(keys, &app) {
                Ok(m) => m["state"].as_str().unwrap_or("unknown").to_string(),
                Err(_) => "empty".to_string(),
            };
            out.push(BoxInfo { name: app.clone(), running: state == "started", state, id: app });
        }
        Ok(out)
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let app = app_name(spec.name.as_deref(), &uuid::Uuid::now_v7().simple().to_string());
        self.request(keys, reqwest::Method::POST, "/v1/apps", Some(json!({ "name": app, "org_slug": ORG })))?;
        let made = (|| -> Result<Value> {
            self.request(
                keys,
                reqwest::Method::POST,
                &format!("/v1/apps/{app}/ip_assignments"),
                Some(json!({ "type": "v6" })),
            )?;
            let dir = super::box_dir(self.id(), &app);
            let (_, public) = super::boat::ensure_keypair(&dir)?;
            let image = spec.template.as_deref().unwrap_or(IMAGE);
            self.request(
                keys,
                reqwest::Method::POST,
                &format!("/v1/apps/{app}/machines"),
                Some(machine_request(image, &public)),
            )?;
            self.wait_state(keys, &app, "started", Duration::from_secs(300))
        })();
        if let Err(err) = made {
            // Leave nothing billable behind a failed create.
            let _ = self.delete(keys, &app);
            return Err(err);
        }
        Ok(BoxInfo { id: app.clone(), name: app, state: "started".into(), running: true })
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        let m = self.machine(keys, id)?;
        let state = m["state"].as_str().unwrap_or("unknown");
        match state {
            "started" => return Ok(()),
            "stopped" | "suspended" | "created" => {
                let mid = m["id"].as_str().context("Fly machine has no id")?;
                self.request(keys, reqwest::Method::POST, &format!("/v1/apps/{id}/machines/{mid}/start"), None)?;
            }
            "failed" | "destroyed" | "destroying" => bail!("Fly machine in {id} is {state}"),
            _ => {}
        }
        self.wait_state(keys, id, "started", Duration::from_secs(180)).map(|_| ())
    }

    fn sleep(&self, keys: &Keys, id: &str) -> Result<()> {
        let m = self.machine(keys, id)?;
        let mid = m["id"].as_str().context("Fly machine has no id")?;
        // Suspend keeps the disk; fall back to a stop when Fly refuses it.
        let path = format!("/v1/apps/{id}/machines/{mid}");
        if self.request(keys, reqwest::Method::POST, &format!("{path}/suspend"), None).is_err() {
            self.request(keys, reqwest::Method::POST, &format!("{path}/stop"), None)?;
        }
        Ok(())
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        if !id.starts_with(PREFIX) {
            bail!("{id} is not a Keplorayx box on Fly; delete it with flyctl");
        }
        if let Ok(list) = self.request(keys, reqwest::Method::GET, &format!("/v1/apps/{id}/machines"), None) {
            for mid in list.as_array().into_iter().flatten().filter_map(|m| m["id"].as_str()) {
                let _ = self.request(
                    keys,
                    reqwest::Method::DELETE,
                    &format!("/v1/apps/{id}/machines/{mid}?force=true"),
                    None,
                );
            }
        }
        self.request(keys, reqwest::Method::DELETE, &format!("/v1/apps/{id}"), None).map(|_| ())
    }

    fn connect(&self, _keys: &Keys, id: &str, dir: &Path) -> Result<Endpoint> {
        let (identity, _) = super::boat::ensure_keypair(dir)?;
        Ok(Endpoint::Ssh(SshTarget {
            host: format!("{id}.fly.dev"),
            port: None,
            user: SSH_USER.into(),
            identity: Some(identity),
            certificate: None,
            known_host: None,
        }))
    }
}

/// Personal tokens go as `Bearer`; macaroon tokens already carry `FlyV1`.
fn auth_header(key: &str) -> String {
    if key.starts_with("FlyV1 ") { key.to_string() } else { format!("Bearer {key}") }
}

/// `keplorayx-<slug>-<6 hex>`: lowercase, DNS-safe and globally unique.
fn app_name(name: Option<&str>, entropy: &str) -> String {
    let slug: String = name
        .unwrap_or("box")
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug: String = if slug.is_empty() { "box".into() } else { slug.chars().take(30).collect() };
    // The tail of a v7 UUID is random; its head is a timestamp.
    let tail = &entropy[entropy.len().saturating_sub(6)..];
    format!("{PREFIX}{}-{tail}", slug.trim_end_matches('-'))
}

/// The apps Keplorayx made, from `GET /v1/apps`.
fn our_apps(reply: &Value) -> Vec<String> {
    reply["apps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a["name"].as_str())
        .filter(|n| n.starts_with(PREFIX))
        .map(String::from)
        .collect()
}

/// A machine that runs sshd on 2222 behind a raw-TCP public port 22.
fn machine_request(image: &str, public_key: &str) -> Value {
    json!({
        "name": "box",
        "config": {
            "image": image,
            "guest": { "cpu_kind": "shared", "cpus": 1, "memory_mb": 2048 },
            "env": {
                "PUBLIC_KEY": public_key,
                "USER_NAME": SSH_USER,
                "PASSWORD_ACCESS": "false",
                "SUDO_ACCESS": "true",
                "DOCKER_MODS": "linuxserver/mods:universal-package-install",
                "INSTALL_PACKAGES": "git"
            },
            "services": [{
                "protocol": "tcp",
                "internal_port": 2222,
                "ports": [{ "port": 22 }],
                "autostop": "suspend",
                "autostart": true
            }],
            "restart": { "policy": "on-failure" },
            "metadata": { "created-by": "keplorayx" }
        }
    })
}

/// Fly errors are `{"error": "..."}`; otherwise the body.
fn api_error(status: u16, v: &Value, text: &str) -> String {
    let msg = v["error"].as_str().or(v["message"].as_str()).map(String::from).unwrap_or_else(|| {
        let t = text.trim();
        if t.is_empty() { "request failed".into() } else { t.chars().take(200).collect() }
    });
    format!("{msg} (HTTP {status})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_names_are_dns_safe_and_prefixed() {
        assert_eq!(app_name(Some("My Box!"), "0192abcdef123456"), "keplorayx-my-box-123456");
        assert_eq!(app_name(None, "abcdef"), "keplorayx-box-abcdef");
        assert_eq!(app_name(Some("***"), "abcdef"), "keplorayx-box-abcdef");
        assert!(app_name(Some(&"x".repeat(80)), "abcdef").len() <= PREFIX.len() + 30 + 7);
    }

    #[test]
    fn lists_only_our_apps() {
        let reply = json!({"apps": [{"name": "keplorayx-a-123456"}, {"name": "my-website"}], "total_apps": 2});
        assert_eq!(our_apps(&reply), vec!["keplorayx-a-123456".to_string()]);
        assert!(our_apps(&Value::Null).is_empty());
    }

    #[test]
    fn the_machine_runs_sshd_behind_raw_tcp() {
        let m = machine_request(IMAGE, "ssh-ed25519 AAAA keplorayx");
        assert_eq!(m["config"]["env"]["PUBLIC_KEY"], "ssh-ed25519 AAAA keplorayx");
        let svc = &m["config"]["services"][0];
        assert_eq!((svc["internal_port"].as_u64(), svc["ports"][0]["port"].as_u64()), (Some(2222), Some(22)));
        assert!(svc["ports"][0].get("handlers").is_none());
    }

    #[test]
    fn tokens_pick_their_scheme() {
        assert_eq!(auth_header("fo1_abc"), "Bearer fo1_abc");
        assert_eq!(auth_header("FlyV1 fm2_abc"), "FlyV1 fm2_abc");
    }

    #[test]
    fn errors_read_like_sentences() {
        assert_eq!(api_error(404, &json!({"error": "app not found"}), ""), "app not found (HTTP 404)");
        assert_eq!(api_error(500, &Value::Null, ""), "request failed (HTTP 500)");
    }

    #[test]
    fn no_key_needs_setup() {
        assert!(matches!(PROVIDER.probe(&Keys::default()), Probe::NeedsSetup(_)));
    }
}
