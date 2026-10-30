//! GitHub Codespaces through the logged-in GitHub CLI (`gh`); no key of its
//! own. Built from the gh source (github.com/cli/cli,
//! `pkg/cmd/codespace/ssh.go`, `create.go`, `internal/codespaces/codespaces.go`)
//! and https://cli.github.com/manual/gh_codespace.
//!
//! - list: `gh codespace list --json name,displayName,state,repository`.
//! - create: `gh codespace create -R <owner/repo>[ -b <branch>] -m <machine>
//!   --default-permissions`. A codespace needs a repository, so the box
//!   template must be `owner/repo` or `owner/repo@branch`; the smallest
//!   machine type is picked from `gh api repos/{repo}/codespaces/machines`
//!   so gh never prompts.
//! - wake: `gh api -X POST user/codespaces/{name}/start`, then poll its state
//!   (gh's own ssh would also start it, but only through an SSH round trip).
//! - sleep / delete: `gh codespace stop -c` / `gh codespace delete -c --force`.
//! - connect: `gh codespace ssh --config -c <name>` prints a `Host` block
//!   whose `ProxyCommand` is `gh cs ssh -c <name> --stdio -- -i <key>`. Its
//!   Host is renamed `dove-box` and both key paths point at a key Keplorayx
//!   keeps in the box's folder (gh uploads the `.pub` beside it), so nothing
//!   is written to `~/.ssh`. The devcontainer must run an SSH server (the
//!   default image does; others add the `sshd` feature).

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider, SSH_ALIAS};

pub struct Codespaces;
pub static PROVIDER: Codespaces = Codespaces;

fn gh(args: &[&str], timeout: Duration) -> Result<String> {
    let path =
        which::which("gh").context("the GitHub CLI (gh) is not installed: get it from https://cli.github.com")?;
    let mut cmd = Command::new(path);
    cmd.args(args);
    // gh must never stop to ask; with no TTY it errors out instead.
    cmd.env("GH_PROMPT_DISABLED", "1").env("NO_COLOR", "1");
    run_cli(&mut cmd, timeout)
        .with_context(|| format!("gh {}", args.iter().take(2).copied().collect::<Vec<_>>().join(" ")))
}

impl Codespaces {
    fn codespace(&self, name: &str) -> Result<Value> {
        let out = gh(&["api", &format!("user/codespaces/{name}")], Duration::from_secs(20))?;
        serde_json::from_str(&out).context("gh returned unreadable JSON")
    }

    fn wait_available(&self, name: &str, limit: Duration) -> Result<()> {
        let deadline = Instant::now() + limit;
        loop {
            let state = self.codespace(name)?["state"].as_str().unwrap_or("Unknown").to_string();
            match state.as_str() {
                "Available" => return Ok(()),
                "Failed" | "Deleted" | "Archived" | "Unavailable" => bail!("codespace {name} is {state}"),
                _ => {}
            }
            if Instant::now() >= deadline {
                bail!("codespace {name} was still {state} after {}s", limit.as_secs());
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    }
}

impl Provider for Codespaces {
    fn id(&self) -> &'static str {
        "codespaces"
    }

    fn label(&self) -> &'static str {
        "GitHub Codespaces"
    }

    fn key_url(&self) -> &'static str {
        "https://github.com/codespaces"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        false
    }

    fn probe(&self, _keys: &Keys) -> Probe {
        if which::which("gh").is_err() {
            return Probe::NeedsSetup(
                "Install the GitHub CLI (gh) from https://cli.github.com, then run `gh auth login`".into(),
            );
        }
        // `gh auth token` reads the local login only: fast and offline.
        match gh(&["auth", "token"], Duration::from_secs(3)) {
            Ok(t) if !t.trim().is_empty() => Probe::Ready,
            _ => Probe::NeedsSetup(
                "Sign in to GitHub: run `gh auth login` (add the codespace scope with `gh auth refresh -s codespace`)"
                    .into(),
            ),
        }
    }

    fn list(&self, _keys: &Keys) -> Result<Vec<BoxInfo>> {
        let out = gh(&["codespace", "list", "--json", "name,displayName,state,repository"], Duration::from_secs(30))?;
        parse_list(&out)
    }

    fn create(&self, _keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let template = spec
            .template
            .as_deref()
            .context("a codespace needs a repository: set the box's template to owner/repo (or owner/repo@branch)")?;
        let (repo, branch) = parse_repo(template)?;
        let machines = gh(&["api", &format!("repos/{repo}/codespaces/machines")], Duration::from_secs(20))?;
        let machine =
            smallest_machine(&machines).with_context(|| format!("GitHub offers no codespace machines for {repo}"))?;
        let mut args = vec!["codespace", "create", "-R", repo, "-m", &machine, "--default-permissions"];
        if let Some(b) = branch {
            args.extend(["-b", b]);
        }
        let display;
        if let Some(n) = &spec.name {
            display = n.chars().take(48).collect::<String>();
            args.extend(["--display-name", &display]);
        }
        let out = gh(&args, Duration::from_secs(600))?;
        let name =
            out.lines().map(str::trim).rfind(|l| !l.is_empty()).context("gh did not print the new codespace's name")?;
        let cs = self.codespace(name)?;
        let state = cs["state"].as_str().unwrap_or("Unknown").to_string();
        Ok(BoxInfo {
            id: name.to_string(),
            name: cs["display_name"].as_str().filter(|d| !d.is_empty()).unwrap_or(name).to_string(),
            running: state == "Available",
            state,
        })
    }

    fn wake(&self, _keys: &Keys, id: &str) -> Result<()> {
        let state = self.codespace(id)?["state"].as_str().unwrap_or("Unknown").to_string();
        match state.as_str() {
            "Available" => return Ok(()),
            "Shutdown" => {
                gh(&["api", "-X", "POST", &format!("user/codespaces/{id}/start")], Duration::from_secs(30))?;
            }
            _ => {}
        }
        self.wait_available(id, Duration::from_secs(300))
    }

    fn sleep(&self, _keys: &Keys, id: &str) -> Result<()> {
        gh(&["codespace", "stop", "-c", id], Duration::from_secs(120)).map(|_| ())
    }

    fn delete(&self, _keys: &Keys, id: &str) -> Result<()> {
        gh(&["codespace", "delete", "-c", id, "--force"], Duration::from_secs(120)).map(|_| ())
    }

    fn connect(&self, _keys: &Keys, id: &str, dir: &Path) -> Result<Endpoint> {
        let (identity, _) = super::boat::ensure_keypair(dir)?;
        let raw = gh(&["codespace", "ssh", "--config", "-c", id], Duration::from_secs(60))?;
        Ok(Endpoint::SshConfig { config: rewrite_config(&raw, &identity)? })
    }
}

/// `gh codespace list --json …` → boxes.
fn parse_list(out: &str) -> Result<Vec<BoxInfo>> {
    let items: Vec<Value> = serde_json::from_str(out).context("gh returned an unreadable codespace list")?;
    Ok(items
        .iter()
        .filter_map(|c| {
            let id = c["name"].as_str()?.to_string();
            let state = c["state"].as_str().unwrap_or("Unknown").to_string();
            let name = c["displayName"].as_str().filter(|d| !d.is_empty()).map(String::from).unwrap_or_else(|| {
                match c["repository"].as_str() {
                    Some(r) => format!("{r} ({id})"),
                    None => id.clone(),
                }
            });
            Some(BoxInfo { running: state == "Available", state, name, id })
        })
        .collect())
}

/// `owner/repo[@branch]`.
fn parse_repo(t: &str) -> Result<(&str, Option<&str>)> {
    let (repo, branch) = match t.trim().split_once('@') {
        Some((r, b)) => (r, Some(b).filter(|b| !b.is_empty())),
        None => (t.trim(), None),
    };
    let repo = repo.trim_start_matches("https://github.com/").trim_end_matches(".git").trim_end_matches('/');
    let mut parts = repo.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(o), Some(r), None) if !o.is_empty() && !r.is_empty() => Ok((repo, branch)),
        _ => bail!("\"{t}\" is not a repository: write it as owner/repo or owner/repo@branch"),
    }
}

/// The machine type with the fewest cores from `…/codespaces/machines`.
fn smallest_machine(out: &str) -> Option<String> {
    let v: Value = serde_json::from_str(out).ok()?;
    v["machines"]
        .as_array()?
        .iter()
        .filter_map(|m| Some((m["cpus"].as_u64().unwrap_or(u64::MAX), m["name"].as_str()?)))
        .min_by_key(|(cpus, _)| *cpus)
        .map(|(_, name)| name.to_string())
}

/// gh's per-codespace `Host` block, renamed `dove-box`, using `identity` for
/// both the SSH login and the key gh hands the codespace.
fn rewrite_config(raw: &str, identity: &Path) -> Result<String> {
    let key = format!("\"{}\"", identity.display());
    let mut out = String::new();
    let mut hosts = 0;
    for line in raw.lines() {
        let trimmed = line.trim_start();
        let word = trimmed.split(|c: char| c.is_whitespace() || c == '=').next().unwrap_or("").to_ascii_lowercase();
        match word.as_str() {
            "" if trimmed.is_empty() => continue,
            "host" => {
                hosts += 1;
                if hosts > 1 {
                    bail!("gh printed more than one codespace");
                }
                out.push_str(&format!("Host {SSH_ALIAS}\n"));
            }
            "identityfile" => out.push_str(&format!("  IdentityFile {key}\n  IdentitiesOnly yes\n")),
            // No ControlPath goes with it, and the launcher wants one plain session.
            "controlmaster" => {}
            "proxycommand" => {
                let mut words: Vec<String> = trimmed.split_whitespace().map(String::from).collect();
                if let Some(i) = words.iter().position(|w| w == "-i") {
                    if i + 1 < words.len() {
                        words[i + 1] = key.clone();
                    }
                } else {
                    words.extend(["-i".to_string(), key.clone()]);
                }
                out.push_str(&format!("  {}\n", words.join(" ")));
            }
            _ => out.push_str(&format!("  {trimmed}\n")),
        }
    }
    if hosts == 0 {
        bail!("gh printed no SSH config for the codespace (is it still being created?)");
    }
    out.push_str("  ServerAliveInterval 30\n  ServerAliveCountMax 4\n");
    Ok(out)
}

/// Run a CLI to completion with no stdin, killing it after `timeout`;
/// stdout on success, otherwise an error carrying its stderr.
pub(super) fn run_cli(cmd: &mut Command, timeout: Duration) -> Result<String> {
    use std::io::Read as _;
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().context("starting the command")?;
    let mut stdout = child.stdout.take().context("no stdout")?;
    let mut stderr = child.stderr.take().context("no stderr")?;
    let out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("timed out after {}s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    if !status.success() {
        let msg = stderr.trim();
        bail!("{}", if msg.is_empty() { format!("exited with {status}") } else { msg.chars().take(400).collect() });
    }
    Ok(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GH_CONFIG: &str = "Host cs.fluffy-space-abc123.main\n\tUser codespace\n\tProxyCommand /usr/bin/gh cs ssh -c fluffy-space-abc123 --stdio -- -i /home/u/.ssh/codespaces.auto\n\tUserKnownHostsFile=/dev/null\n\tStrictHostKeyChecking no\n\tLogLevel quiet\n\tControlMaster auto\n\tIdentityFile /home/u/.ssh/codespaces.auto\n\n";

    #[test]
    fn renames_the_host_and_swaps_in_our_key() {
        let c = rewrite_config(GH_CONFIG, Path::new("/d/id_ed25519")).unwrap();
        assert!(c.starts_with("Host dove-box\n  User codespace\n"));
        assert!(c.contains("ProxyCommand /usr/bin/gh cs ssh -c fluffy-space-abc123 --stdio -- -i \"/d/id_ed25519\"\n"));
        assert!(c.contains("  IdentityFile \"/d/id_ed25519\"\n  IdentitiesOnly yes\n"));
        assert!(!c.contains("codespaces.auto"));
        assert!(!c.contains("ControlMaster"));
        assert!(c.contains("  UserKnownHostsFile=/dev/null\n"));
    }

    #[test]
    fn refuses_empty_or_several_hosts() {
        assert!(rewrite_config("", Path::new("/k")).is_err());
        let two = format!("{GH_CONFIG}{GH_CONFIG}");
        assert!(rewrite_config(&two, Path::new("/k")).is_err());
    }

    #[test]
    fn codespaces_become_boxes() {
        let out = r#"[{"name":"fluffy-abc","displayName":"","state":"Available","repository":"o/r"},
                     {"name":"spooky-def","displayName":"agent","state":"Shutdown","repository":"o/r"}]"#;
        let boxes = parse_list(out).unwrap();
        assert_eq!(
            boxes[0],
            BoxInfo {
                id: "fluffy-abc".into(),
                name: "o/r (fluffy-abc)".into(),
                state: "Available".into(),
                running: true
            }
        );
        assert_eq!((boxes[1].name.as_str(), boxes[1].running), ("agent", false));
        assert!(parse_list("not json").is_err());
    }

    #[test]
    fn reads_repository_templates() {
        assert_eq!(parse_repo("octo/hello").unwrap(), ("octo/hello", None));
        assert_eq!(parse_repo("octo/hello@dev").unwrap(), ("octo/hello", Some("dev")));
        assert_eq!(parse_repo("https://github.com/octo/hello.git").unwrap(), ("octo/hello", None));
        assert!(parse_repo("hello").is_err());
        assert!(parse_repo("a/b/c").is_err());
    }

    #[test]
    fn picks_the_smallest_machine() {
        let out = r#"{"total_count":2,"machines":[{"name":"standardLinux32gb","cpus":4},{"name":"basicLinux32gb","cpus":2}]}"#;
        assert_eq!(smallest_machine(out).as_deref(), Some("basicLinux32gb"));
        assert_eq!(smallest_machine(r#"{"machines":[]}"#), None);
    }

    #[cfg(unix)]
    #[test]
    fn run_cli_returns_stdout_or_stderr() {
        let ok = run_cli(Command::new("sh").args(["-c", "echo hi"]), Duration::from_secs(5)).unwrap();
        assert_eq!(ok, "hi\n");
        let err =
            run_cli(Command::new("sh").args(["-c", "echo nope >&2; exit 3"]), Duration::from_secs(5)).unwrap_err();
        assert_eq!(err.to_string(), "nope");
        assert!(run_cli(Command::new("sh").args(["-c", "sleep 5"]), Duration::from_millis(200)).is_err());
    }
}
