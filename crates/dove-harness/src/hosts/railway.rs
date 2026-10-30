//! Railway (railway.com): attach to services you already run on Railway,
//! over Railway's native SSH. Honest scope, as of October 2026:
//!
//! - `ssh railway.new` is an onboarding flow (sign-up, the Railway Agent and
//!   short-lived sandboxes that idle out after 5 minutes on Trial/Free;
//!   https://railway.com/changelog/2026-06-26-railway-over-ssh), not a VM
//!   with a lifecycle API, so Keplorayx does not drive it.
//! - What does work: every service instance answers plain OpenSSH at
//!   `<service-instance-id>@ssh.railway.com` for any SSH key registered with
//!   Railway (`railway ssh keys add`, or the dashboard), and piped stdin runs
//!   without a PTY (https://docs.railway.com/cli/ssh). So a box is a service
//!   instance (one service in one environment), reached with the user's own
//!   key via their ssh-agent/default keys. Nothing is stored for SSH.
//! - Listing and start/stop use the public GraphQL API
//!   (https://docs.railway.com/integrations/api, `backboard.railway.com/graphql/v2`)
//!   with an account or workspace token as `Bearer`, which is why this
//!   provider needs a key.
//! - Not supported: create (deploy a service in Railway; it then shows up
//!   here) and delete (left to Railway's dashboard, since these are the
//!   user's own services). Containers are ephemeral (only volumes persist)
//!   and the image needs `sh`, `git` and the agent CLI.
//!
//! Unverified: whether a sleeping (serverless) service wakes on SSH, and the
//! exact GraphQL field names below (`serviceInstances`, `latestDeployment`),
//! which come from the docs' examples rather than a schema dump.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider, SshTarget};

const API: &str = "https://backboard.railway.com/graphql/v2";
const GATEWAY: &str = "ssh.railway.com";

const LIST: &str = "query { projects { edges { node { id name \
    environments { edges { node { id name } } } \
    services { edges { node { id name serviceInstances { edges { node { id environmentId \
    latestDeployment { id status } } } } } } } } } } }";

pub struct Railway;
pub static PROVIDER: Railway = Railway;

/// One service instance: what list shows and what lifecycle calls need.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Instance {
    id: String,
    name: String,
    service_id: String,
    environment_id: String,
    deployment_id: Option<String>,
    /// Railway's deployment status (`SUCCESS`, `SLEEPING`, `CRASHED`…), or `NONE`.
    status: String,
}

impl Railway {
    fn key<'a>(&self, keys: &'a Keys) -> Result<&'a str> {
        keys.get(self.id()).context("no Railway token: add one in Settings › Machines")
    }

    fn graphql(&self, keys: &Keys, query: &str, variables: Value) -> Result<Value> {
        let key = self.key(keys)?;
        let resp = super::http()
            .post(API)
            .bearer_auth(key)
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .context("calling Railway's API")?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if let Some(msg) = graphql_error(&value) {
            bail!("Railway: {msg} (HTTP {})", status.as_u16());
        }
        if !status.is_success() {
            bail!("Railway: request failed (HTTP {})", status.as_u16());
        }
        Ok(value["data"].clone())
    }

    fn instances(&self, keys: &Keys) -> Result<Vec<Instance>> {
        Ok(parse_instances(&self.graphql(keys, LIST, json!({}))?))
    }

    fn instance(&self, keys: &Keys, id: &str) -> Result<Instance> {
        self.instances(keys)?
            .into_iter()
            .find(|i| i.id == id)
            .with_context(|| format!("Railway has no service instance {id} for this token"))
    }
}

impl Provider for Railway {
    fn id(&self) -> &'static str {
        "railway"
    }

    fn label(&self) -> &'static str {
        "Railway"
    }

    fn key_url(&self) -> &'static str {
        "https://railway.com/account/tokens"
    }

    fn verified(&self) -> bool {
        false
    }

    /// The GraphQL API needs an account or workspace token to list services.
    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, keys: &Keys) -> Probe {
        if !keys.has(self.id()) {
            return Probe::NeedsSetup(
                "Add a Railway account or workspace token, and register your SSH key with `railway ssh keys add`"
                    .into(),
            );
        }
        match self.graphql(keys, "query { projects { edges { node { id } } } }", json!({})) {
            Ok(_) => Probe::Ready,
            Err(err) if err.to_string().contains("Not Authorized") || err.to_string().contains("HTTP 401") => {
                Probe::NeedsSetup("Railway rejected the token (project tokens can't list; use an account token)".into())
            }
            Err(err) => Probe::Unavailable(format!("{err:#}")),
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        Ok(self
            .instances(keys)?
            .into_iter()
            .map(|i| BoxInfo {
                running: i.status == "SUCCESS",
                state: i.status.to_ascii_lowercase(),
                name: i.name,
                id: i.id,
            })
            .collect())
    }

    fn create(&self, _keys: &Keys, _spec: &BoxSpec) -> Result<BoxInfo> {
        bail!(
            "Keplorayx can't create Railway services: deploy one at https://railway.com/new (an image with sh, git and \
             your agent), then pick it here"
        )
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        let i = self.instance(keys, id)?;
        match i.status.as_str() {
            // Asleep is assumed to wake on the connection (unverified).
            "SUCCESS" | "SLEEPING" => return Ok(()),
            "BUILDING" | "DEPLOYING" | "INITIALIZING" | "QUEUED" | "WAITING" => {}
            // Stopped, crashed, removed or never deployed: deploy again.
            _ => {
                self.graphql(
                    keys,
                    "mutation($s: String!, $e: String!) { serviceInstanceRedeploy(serviceId: $s, environmentId: $e) }",
                    json!({ "s": i.service_id, "e": i.environment_id }),
                )?;
            }
        }
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            std::thread::sleep(Duration::from_secs(5));
            let now = self.instance(keys, id)?;
            match now.status.as_str() {
                "SUCCESS" => return Ok(()),
                "FAILED" | "CRASHED" => {
                    bail!("Railway deployment of {} is {}", now.name, now.status.to_ascii_lowercase())
                }
                _ if Instant::now() >= deadline => {
                    bail!("Railway deployment of {} was still {} after 600s", now.name, now.status.to_ascii_lowercase())
                }
                _ => {}
            }
        }
    }

    fn sleep(&self, keys: &Keys, id: &str) -> Result<()> {
        let i = self.instance(keys, id)?;
        let dep = i.deployment_id.with_context(|| format!("{} has no deployment to stop", i.name))?;
        self.graphql(keys, "mutation($id: String!) { deploymentStop(id: $id) }", json!({ "id": dep })).map(|_| ())
    }

    fn delete(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Keplorayx leaves your Railway services alone: delete it in Railway's dashboard")
    }

    fn connect(&self, _keys: &Keys, id: &str, _dir: &Path) -> Result<Endpoint> {
        // The user's own key (ssh-agent or ~/.ssh defaults) authenticates.
        Ok(Endpoint::Ssh(SshTarget {
            host: GATEWAY.into(),
            port: None,
            user: id.to_string(),
            identity: None,
            certificate: None,
            known_host: None,
        }))
    }
}

/// Flatten projects → services → instances into boxes named
/// `project / service (environment)`.
fn parse_instances(data: &Value) -> Vec<Instance> {
    let edges = |v: &Value| v["edges"].as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for p in edges(&data["projects"]) {
        let p = &p["node"];
        let project = p["name"].as_str().unwrap_or("project");
        let env_name = |id: &str| {
            edges(&p["environments"])
                .iter()
                .find(|e| e["node"]["id"] == id)
                .and_then(|e| e["node"]["name"].as_str().map(String::from))
        };
        for s in edges(&p["services"]) {
            let s = &s["node"];
            let Some(service_id) = s["id"].as_str() else { continue };
            let service = s["name"].as_str().unwrap_or(service_id);
            for inst in edges(&s["serviceInstances"]) {
                let inst = &inst["node"];
                let (Some(id), Some(env)) = (inst["id"].as_str(), inst["environmentId"].as_str()) else { continue };
                let env_label = env_name(env).unwrap_or_else(|| env.to_string());
                out.push(Instance {
                    id: id.to_string(),
                    name: format!("{project} / {service} ({env_label})"),
                    service_id: service_id.to_string(),
                    environment_id: env.to_string(),
                    deployment_id: inst["latestDeployment"]["id"].as_str().map(String::from),
                    status: inst["latestDeployment"]["status"].as_str().unwrap_or("NONE").to_string(),
                });
            }
        }
    }
    out
}

/// GraphQL errors arrive with HTTP 200: `{"errors": [{"message"}]}`.
fn graphql_error(v: &Value) -> Option<String> {
    let errors = v["errors"].as_array().filter(|e| !e.is_empty())?;
    Some(errors.iter().filter_map(|e| e["message"].as_str()).collect::<Vec<_>>().join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        json!({"projects": {"edges": [{"node": {
            "id": "p1", "name": "shop",
            "environments": {"edges": [{"node": {"id": "e1", "name": "production"}}]},
            "services": {"edges": [{"node": {"id": "s1", "name": "api", "serviceInstances": {"edges": [
                {"node": {"id": "si1", "environmentId": "e1", "latestDeployment": {"id": "d1", "status": "SUCCESS"}}},
                {"node": {"id": "si2", "environmentId": "e9", "latestDeployment": null}}
            ]}}}]}
        }}]}})
    }

    #[test]
    fn service_instances_become_boxes() {
        let list = parse_instances(&fixture());
        assert_eq!(
            list[0],
            Instance {
                id: "si1".into(),
                name: "shop / api (production)".into(),
                service_id: "s1".into(),
                environment_id: "e1".into(),
                deployment_id: Some("d1".into()),
                status: "SUCCESS".into(),
            }
        );
        assert_eq!((list[1].name.as_str(), list[1].status.as_str()), ("shop / api (e9)", "NONE"));
        assert!(parse_instances(&Value::Null).is_empty());
    }

    #[test]
    fn graphql_errors_read_like_sentences() {
        let v = json!({"data": null, "errors": [{"message": "Not Authorized"}]});
        assert_eq!(graphql_error(&v).as_deref(), Some("Not Authorized"));
        assert_eq!(graphql_error(&json!({"data": {}})), None);
    }

    #[test]
    fn connects_to_the_ssh_gateway_as_the_instance() {
        let Endpoint::Ssh(t) = PROVIDER.connect(&Keys::default(), "si1", Path::new("/d")).unwrap() else { panic!() };
        assert_eq!((t.host.as_str(), t.user.as_str(), t.identity), (GATEWAY, "si1", None));
    }

    #[test]
    fn no_key_needs_setup() {
        assert!(matches!(PROVIDER.probe(&Keys::default()), Probe::NeedsSetup(_)));
    }
}
