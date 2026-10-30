//! Cua Spaces through the `cua` CLI: the free way to use spare disk and RAM,
//! on this PC, on one of the user's other computers, or in their own cloud
//! account (docs/ENVIRONMENTS.md §3, decision 2).
//!
//! Lifecycle uses `cua spaces …`; running an agent uses `cua sandbox exec -i`,
//! the same streaming exec [`crate::host::Host::CuaSpace`] already uses, so
//! both routes reach the same Spaces by the same ids (`local:space-1a2b3c`).
//!
//! - `cua spaces ls --json --no-size` lists every Space, asleep or not.
//! - `cua spaces create [--name N] [--on WHERE] [IMAGE]`: `WHERE` is `local`
//!   (the default), `auto`, `aws`, `gcp`, `modal` or `host:<computer>`; the
//!   image is an alias (`linux`, `macos`, `macos:26`, `windows`) or a
//!   registry reference.
//! - `cua spaces start` resumes a suspended Space or boots a stopped one;
//!   `suspend` keeps memory, `stop` frees it; `delete --force` removes it.
//! - `cua sandbox exec -i <id> <argv…>` streams stdio and runs argv without a
//!   shell and without a time limit.
//!
//! Placement is encoded in [`BoxSpec::template`] as whitespace-separated
//! words: an `on=<where>` word picks the placement, any other word is the
//! image. So `"macos on=host:studio"`, `"on=aws"` and `"ghcr.io/o/i:tag"`
//! all work; an empty template is a local Linux Space.
//!
//! Cua's hosted cloud shut down on 2026-10-18 (its cloud guide); local Spaces,
//! paired computers and bring-your-own-cloud placements are unaffected.
//!
//! Sources: <https://cua.ai/docs/reference/cua-cli/cli-reference>,
//! <https://cua.ai/docs/cua-cli/reference/cli/spaces>,
//! <https://cua.ai/docs/cua-cli/reference/cli/sandboxes>,
//! <https://cua.ai/docs/cua-sdk/guides/cloud>. The JSON field names of
//! `spaces ls`/`create` are not documented; the parsers accept the shapes
//! `cua sandbox ls --json` is known to print (`id`, `name`, `status`).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

pub struct Cua;
pub static PROVIDER: Cua = Cua;

const INSTALL: &str = if cfg!(windows) { "pip install cua-cli" } else { "curl -fsSL https://cua.ai/install.sh | sh" };

impl Provider for Cua {
    fn id(&self) -> &'static str {
        "cua"
    }

    fn label(&self) -> &'static str {
        "Cua Spaces"
    }

    fn key_url(&self) -> &'static str {
        "https://cua.ai/docs"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        false
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        match find_program("cua") {
            Some(_) => Probe::Ready,
            None => Probe::NeedsSetup(format!("Install the Cua CLI with `{INSTALL}`, then try again")),
        }
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        let out = run_cli("cua", &["spaces", "ls", "--json", "--no-size"])?;
        parse_spaces(&out)
    }

    fn create(&self, _keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let args = create_args(spec);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = run_cli("cua", &refs)?;
        let id = created_id(&out, spec.name.as_deref())
            .or_else(|| {
                // Older builds print prose; find the new Space by its name.
                let name = spec.name.as_deref()?;
                self.list(&Keys::default()).ok()?.into_iter().find(|b| b.name == name || b.id == name).map(|b| b.id)
            })
            .context("Cua created a Space but did not say its id; it shows up in the list")?;
        let found = self.list(&Keys::default()).ok().and_then(|l| l.into_iter().find(|b| b.id == id));
        Ok(found.unwrap_or_else(|| BoxInfo {
            name: spec.name.clone().unwrap_or_else(|| id.clone()),
            id,
            state: "ready".into(),
            running: true,
        }))
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        let space = self.list(keys)?.into_iter().find(|b| b.id == id || b.name == id);
        let space = space.with_context(|| format!("there is no Cua Space named {id}"))?;
        if !space.running && is_asleep(&space.state) {
            run_cli("cua", &["spaces", "start", id])?;
        }
        Ok(())
    }

    fn sleep(&self, _keys: &Keys, id: &str) -> Result<()> {
        // Suspend keeps memory for a fast resume; Spaces that cannot suspend
        // (VMs on another computer, cloud placements) are stopped instead.
        match run_cli("cua", &["spaces", "suspend", id]) {
            Ok(_) => Ok(()),
            Err(suspend) => run_cli("cua", &["spaces", "stop", id])
                .map(|_| ())
                .map_err(|stop| anyhow::anyhow!("{suspend:#}; stopping it also failed: {stop:#}")),
        }
    }

    fn delete(&self, _keys: &Keys, id: &str) -> Result<()> {
        // `--force` is required without a terminal, or nothing is deleted.
        run_cli("cua", &["spaces", "delete", "--force", id]).map(|_| ())
    }

    fn connect(&self, _keys: &Keys, id: &str, _dir: &Path) -> Result<Endpoint> {
        Ok(Endpoint::Exec { argv: ["cua", "sandbox", "exec", "-i", id].map(String::from).to_vec() })
    }
}

/// `spaces create` argv from a spec (see the module docs for the encoding).
fn create_args(spec: &BoxSpec) -> Vec<String> {
    let mut a: Vec<String> = ["--json", "spaces", "create"].map(String::from).to_vec();
    if let Some(n) = spec.name.as_deref().filter(|n| !n.trim().is_empty()) {
        a.extend(["--name".into(), n.trim().into()]);
    }
    let (on, image) = placement(spec.template.as_deref().unwrap_or_default());
    a.extend(["--on".into(), on]);
    if let Some(i) = image {
        a.push(i);
    }
    a
}

/// `"macos on=host:studio"` → (`host:studio`, `Some("macos")`). Placement
/// defaults to `local`; with no image Cua picks its Linux default.
fn placement(template: &str) -> (String, Option<String>) {
    let mut on = "local".to_string();
    let mut image = None;
    for word in template.split_whitespace() {
        match word.strip_prefix("on=") {
            Some(w) if !w.is_empty() => on = w.to_string(),
            Some(_) => {}
            None => image = Some(word.to_string()),
        }
    }
    (on, image)
}

/// `spaces ls --json`: a bare array or `{"spaces": [...]}`.
fn parse_spaces(raw: &str) -> Result<Vec<BoxInfo>> {
    let v: Value = serde_json::from_str(raw.trim()).context("Cua printed a Space list Keplorayx cannot read")?;
    let items = match &v {
        Value::Array(a) => a.as_slice(),
        Value::Object(o) => {
            ["spaces", "sandboxes", "items", "data"].iter().find_map(|k| o.get(*k)?.as_array()).map_or(&[][..], |a| a)
        }
        _ => &[],
    };
    Ok(items.iter().filter_map(space_info).collect())
}

fn space_info(s: &Value) -> Option<BoxInfo> {
    let id = s["id"].as_str().or_else(|| s["name"].as_str())?.to_string();
    let state = ["status", "state"].iter().find_map(|k| s[*k].as_str()).unwrap_or("unknown").to_ascii_lowercase();
    let running = matches!(state.as_str(), "ready" | "running" | "started" | "online");
    let name = s["display_name"].as_str().or_else(|| s["name"].as_str()).unwrap_or(&id).to_string();
    Some(BoxInfo { id, name, state, running })
}

fn is_asleep(state: &str) -> bool {
    ["pause", "suspend", "stop", "off", "exited", "shutdown"].iter().any(|w| state.contains(w))
}

/// The new Space's id from `create`'s output: JSON (`id`, or the first of
/// `spaces`), else the first `<kind>:<name>` word in the text.
fn created_id(raw: &str, name: Option<&str>) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<Value>(raw.trim()) {
        let first = match &v {
            Value::Array(a) => a.first().cloned(),
            Value::Object(o) => match o.get("spaces").and_then(Value::as_array) {
                Some(a) => a.first().cloned(),
                None => Some(v.clone()),
            },
            _ => None,
        };
        if let Some(id) = first.as_ref().and_then(|s| s["id"].as_str().or_else(|| s["name"].as_str())) {
            return Some(id.to_string());
        }
    }
    let words = raw.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ',' | '(' | ')'));
    let word = |w: &str| {
        let w = w.trim_end_matches(['.', ':']);
        let (kind, rest) = w.split_once(':')?;
        (["local", "relay", "host", "aws", "gcp", "modal"].contains(&kind) && !rest.is_empty()).then(|| w.to_string())
    };
    let ids: Vec<String> = words.filter_map(word).collect();
    match name {
        Some(n) => ids.iter().find(|i| i.ends_with(&format!(":{n}"))).cloned().or_else(|| ids.first().cloned()),
        None => ids.first().cloned(),
    }
}

/// The program on `PATH` (with `PATHEXT` on Windows), without running it.
pub(super) fn find_program(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .map(|e| e.to_ascii_lowercase())
            .chain([String::new()])
            .collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&path)
        .find_map(|dir| exts.iter().map(|e| dir.join(format!("{name}{e}"))).find(|p| p.is_file()))
}

/// Run a management command: no stdin, no console window, output captured.
/// Fails with a sentence carrying the tool's own message.
pub(super) fn run_cli(program: &str, args: &[&str]) -> Result<String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let shown = format!("{program} {}", args.join(" "));
    let out = match cmd.output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!("{program} is not installed or not on PATH"),
        Err(e) => return Err(e).with_context(|| format!("could not run `{shown}`")),
    };
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        bail!("`{shown}` failed: {}", failure_text(&out.status, &out.stderr, &stdout));
    }
    Ok(stdout)
}

/// The last meaningful lines a failed tool printed, or its exit code.
pub(super) fn failure_text(status: &std::process::ExitStatus, stderr: &[u8], stdout: &str) -> String {
    let err = String::from_utf8_lossy(stderr);
    let text = if err.trim().is_empty() { stdout.trim() } else { err.trim() };
    if text.is_empty() {
        return match status.code() {
            Some(c) => format!("it exited with code {c}"),
            None => "it was killed by a signal".into(),
        };
    }
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(3)..].join(" / ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_words_split_from_the_image() {
        assert_eq!(placement(""), ("local".into(), None));
        assert_eq!(placement("macos on=host:studio"), ("host:studio".into(), Some("macos".into())));
        assert_eq!(placement("on=aws"), ("aws".into(), None));
        assert_eq!(placement("ghcr.io/o/i:tag"), ("local".into(), Some("ghcr.io/o/i:tag".into())));
        let spec = BoxSpec { name: Some("dev".into()), template: Some("linux on=gcp".into()) };
        assert_eq!(create_args(&spec), ["--json", "spaces", "create", "--name", "dev", "--on", "gcp", "linux"]);
        assert_eq!(create_args(&BoxSpec::default()), ["--json", "spaces", "create", "--on", "local"]);
    }

    #[test]
    fn lists_spaces_in_every_state() {
        let raw = r#"[{"id":"local:space-1","name":"space-1","status":"ready"},
                      {"id":"local:studio","status":"Suspended"},
                      {"name":"relay:space-9","state":"stopped"},
                      {"status":"ready"}]"#;
        let l = parse_spaces(raw).unwrap();
        assert_eq!(l.len(), 3);
        assert_eq!(
            l[0],
            BoxInfo { id: "local:space-1".into(), name: "space-1".into(), state: "ready".into(), running: true }
        );
        assert_eq!((l[1].state.as_str(), l[1].running), ("suspended", false));
        assert!(is_asleep(&l[1].state) && is_asleep(&l[2].state) && !is_asleep(&l[0].state));
        let wrapped = parse_spaces(r#"{"spaces":[{"id":"local:a","status":"running"}]}"#).unwrap();
        assert_eq!(wrapped[0].id, "local:a");
        assert!(parse_spaces("not json").is_err());
    }

    #[test]
    fn finds_the_created_id() {
        assert_eq!(created_id(r#"{"id":"local:space-1a2b"}"#, None).as_deref(), Some("local:space-1a2b"));
        assert_eq!(created_id(r#"{"spaces":[{"id":"local:x"}]}"#, None).as_deref(), Some("local:x"));
        let prose = "Creating Space on host:studio...\nCreated Space `host:dev` (macos).\n";
        assert_eq!(created_id(prose, Some("dev")).as_deref(), Some("host:dev"));
        assert_eq!(created_id("Created local:space-7.", None).as_deref(), Some("local:space-7"));
        assert_eq!(created_id("done", None), None);
    }

    #[test]
    fn connects_with_streaming_exec() {
        let e = PROVIDER.connect(&Keys::default(), "local:space-1", Path::new("/d")).unwrap();
        assert_eq!(
            e,
            Endpoint::Exec { argv: ["cua", "sandbox", "exec", "-i", "local:space-1"].map(String::from).to_vec() }
        );
    }

    #[test]
    fn missing_tools_fail_with_a_sentence() {
        let err = run_cli("dove-no-such-tool-xyz", &["--version"]).unwrap_err();
        assert_eq!(err.to_string(), "dove-no-such-tool-xyz is not installed or not on PATH");
        assert!(find_program("dove-no-such-tool-xyz").is_none());
    }
}
