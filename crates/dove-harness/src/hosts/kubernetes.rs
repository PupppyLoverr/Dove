//! Kubernetes: any pod in a cluster the user's kubeconfig reaches, through
//! the system `kubectl` (route: exec, `kubectl exec -i`, never `-t`).
//!
//! - Box id: `<context>/<namespace>/<pod>`, so a box keeps pointing at the
//!   same cluster after the user switches contexts.
//! - `list`: `kubectl get pods -o json` in the current context's namespace
//!   (https://kubernetes.io/docs/reference/kubectl/generated/kubectl_get/).
//! - `create`: a bare pod (`kubectl run --restart=Never … -- sleep infinity`,
//!   https://kubernetes.io/docs/reference/kubectl/generated/kubectl_run/)
//!   from `spec.template` or `buildpack-deps:noble-scm` (Ubuntu 24.04 plus
//!   git and curl; plain `ubuntu:24.04` has no git), then
//!   `kubectl wait --for=condition=Ready`.
//! - `connect`: `kubectl --context C -n N exec -i POD --`
//!   (https://kubernetes.io/docs/reference/kubectl/generated/kubectl_exec/).
//!   stdin, stdout and stderr stay separate byte streams and the exit code
//!   comes back.
//! - `wake` checks the pod is Running; `sleep` does nothing, because a pod
//!   can't be suspended (delete it, or scale its owner, to stop paying).
//!   A bare pod is not restarted if its node goes away.
//!
//! No key: kubectl's own config and credentials are used.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

const DEFAULT_IMAGE: &str = "buildpack-deps:noble-scm";

pub struct Kubernetes;
pub static PROVIDER: Kubernetes = Kubernetes;

/// `kubectl <args…>`'s stdout, or its stderr as the error.
fn kubectl(args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("kubectl");
    cmd.args(args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().context("running kubectl (is it installed and on PATH?)")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("kubectl {}: {}", args.first().unwrap_or(&""), err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `<context>/<namespace>/<pod>`, split from the right: context names may
/// contain `/` (EKS ARNs do), namespaces and pods never do.
fn split_id(id: &str) -> Result<(&str, &str, &str)> {
    let mut it = id.rsplitn(3, '/');
    match (it.next(), it.next(), it.next()) {
        (Some(p), Some(n), Some(c)) if !c.is_empty() && !n.is_empty() && !p.is_empty() => Ok((c, n, p)),
        _ => bail!("{id} is not a Kubernetes box id (context/namespace/pod)"),
    }
}

/// The current context and its namespace (default `default`).
fn current() -> Result<(String, String)> {
    let ctx = kubectl(&["config", "current-context"])?.trim().to_string();
    if ctx.is_empty() {
        bail!("kubectl has no current context");
    }
    let ns = kubectl(&["config", "view", "--minify", "-o", "jsonpath={..namespace}"]).unwrap_or_default();
    let ns = ns.trim();
    Ok((ctx, if ns.is_empty() { "default".into() } else { ns.to_string() }))
}

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
        if which::which("kubectl").is_err() {
            return Probe::NeedsSetup(
                "Install kubectl (https://kubernetes.io/docs/tasks/tools/) and point it at a cluster".into(),
            );
        }
        // Reads the kubeconfig only; no cluster call.
        match kubectl(&["config", "current-context"]) {
            Ok(c) if !c.trim().is_empty() => Probe::Ready,
            _ => Probe::NeedsSetup("kubectl has no current context: run `kubectl config use-context <name>`".into()),
        }
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        let (ctx, ns) = current()?;
        let json = kubectl(&["--context", &ctx, "-n", &ns, "--request-timeout=10s", "get", "pods", "-o", "json"])?;
        let v: Value = serde_json::from_str(&json).context("reading kubectl's pod list")?;
        Ok(parse_pods(&ctx, &v))
    }

    fn create(&self, _keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let (ctx, ns) = current()?;
        let name = pod_name(spec.name.as_deref());
        let image = format!("--image={}", spec.template.as_deref().unwrap_or(DEFAULT_IMAGE));
        kubectl(&[
            "--context",
            &ctx,
            "-n",
            &ns,
            "--request-timeout=20s",
            "run",
            &name,
            &image,
            "--restart=Never",
            "--labels=app.kubernetes.io/managed-by=keplorayx",
            "--command",
            "--",
            "sleep",
            "infinity",
        ])?;
        let target = format!("pod/{name}");
        kubectl(&["--context", &ctx, "-n", &ns, "wait", "--for=condition=Ready", &target, "--timeout=300s"])?;
        Ok(BoxInfo { id: format!("{ctx}/{ns}/{name}"), name, state: "Running".into(), running: true })
    }

    fn wake(&self, _keys: &Keys, id: &str) -> Result<()> {
        let (ctx, ns, pod) = split_id(id)?;
        let phase = kubectl(&[
            "--context",
            ctx,
            "-n",
            ns,
            "--request-timeout=10s",
            "get",
            "pod",
            pod,
            "-o",
            "jsonpath={.status.phase}",
        ])?;
        match phase.trim() {
            "Running" => Ok(()),
            "Pending" => {
                let target = format!("pod/{pod}");
                kubectl(&["--context", ctx, "-n", ns, "wait", "--for=condition=Ready", &target, "--timeout=300s"])
                    .map(|_| ())
            }
            other => bail!("pod {pod} is {other}; Kubernetes can't restart a finished pod, so create a new one"),
        }
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        // A pod can't be suspended; deleting it is the only way to stop it.
        Ok(())
    }

    fn delete(&self, _keys: &Keys, id: &str) -> Result<()> {
        let (ctx, ns, pod) = split_id(id)?;
        kubectl(&[
            "--context",
            ctx,
            "-n",
            ns,
            "--request-timeout=20s",
            "delete",
            "pod",
            pod,
            "--ignore-not-found",
            "--wait=false",
        ])
        .map(|_| ())
    }

    fn connect(&self, _keys: &Keys, id: &str, _dir: &Path) -> Result<Endpoint> {
        Ok(Endpoint::Exec { argv: exec_argv(id)? })
    }
}

fn exec_argv(id: &str) -> Result<Vec<String>> {
    let (ctx, ns, pod) = split_id(id)?;
    Ok(["kubectl", "--context", ctx, "-n", ns, "exec", "-i", pod, "--"].map(String::from).to_vec())
}

fn parse_pods(ctx: &str, v: &Value) -> Vec<BoxInfo> {
    v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let name = p["metadata"]["name"].as_str()?;
            let ns = p["metadata"]["namespace"].as_str().unwrap_or("default");
            let phase = p["status"]["phase"].as_str().unwrap_or("Unknown").to_string();
            // Being deleted shows as Running until it goes.
            let terminating = p["metadata"]["deletionTimestamp"].is_string();
            Some(BoxInfo {
                id: format!("{ctx}/{ns}/{name}"),
                name: name.to_string(),
                running: phase == "Running" && !terminating,
                state: if terminating { "Terminating".into() } else { phase },
            })
        })
        .collect()
}

/// DNS-1123: lowercase letters, digits and dashes, at most 63.
fn pod_name(wanted: Option<&str>) -> String {
    let cleaned: String = wanted
        .unwrap_or_default()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(63)
        .collect::<String>()
        .trim_end_matches('-')
        .to_string();
    if cleaned.is_empty() {
        let id = uuid::Uuid::now_v7().simple().to_string();
        format!("keplorayx-{}", &id[id.len() - 8..])
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ids_round_trip_into_exec() {
        assert_eq!(split_id("kind-dev/default/box-1").unwrap(), ("kind-dev", "default", "box-1"));
        assert_eq!(
            split_id("arn:aws:eks:us-east-1:1:cluster/x/ns/pod").unwrap(),
            ("arn:aws:eks:us-east-1:1:cluster/x", "ns", "pod")
        );
        assert!(split_id("default/box").is_err());
        assert_eq!(
            exec_argv("c/n/p").unwrap(),
            ["kubectl", "--context", "c", "-n", "n", "exec", "-i", "p", "--"].map(String::from).to_vec()
        );
    }

    #[test]
    fn pods_become_boxes() {
        let v = json!({"items": [
            {"metadata": {"name": "a", "namespace": "dev"}, "status": {"phase": "Running"}},
            {"metadata": {"name": "b", "namespace": "dev", "deletionTimestamp": "2026-01-01T00:00:00Z"}, "status": {"phase": "Running"}},
            {"metadata": {"name": "c"}, "status": {"phase": "Succeeded"}},
            {"status": {"phase": "Running"}}
        ]});
        let boxes = parse_pods("ctx", &v);
        assert_eq!(boxes.len(), 3);
        assert_eq!(
            boxes[0],
            BoxInfo { id: "ctx/dev/a".into(), name: "a".into(), state: "Running".into(), running: true }
        );
        assert_eq!((boxes[1].state.as_str(), boxes[1].running), ("Terminating", false));
        assert_eq!(boxes[2].id, "ctx/default/c");
    }

    #[test]
    fn pod_names_are_dns_safe() {
        assert_eq!(pod_name(Some("My Box_1")), "my-box-1");
        assert!(pod_name(Some("!!")).starts_with("keplorayx-"));
        assert_eq!(pod_name(Some(&"x".repeat(80))).len(), 63);
    }
}
