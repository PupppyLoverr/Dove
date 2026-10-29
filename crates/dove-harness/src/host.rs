//! Where an agent runs: this machine, a WSL distro, an SSH host, a Cua Space,
//! a Docker container, or a cloud sandbox the user brings a key for. A
//! [`Host`] rewrites a local invocation into one that runs on the target
//! through a system tool (`wsl.exe`, `ssh`, `cua`, `docker`), so the stdio/PTY
//! plumbing, the `AgentEvent` stream and the UI stay exactly as they are.
//! Nothing here bundles a client: every target costs zero binary size.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Host {
    #[default]
    Local,
    /// A WSL distro; `None` is the user's default distro.
    Wsl { distro: Option<String> },
    /// An alias from `~/.ssh/config` (or `user@host`), run with the system
    /// OpenSSH client so keys, agents and jump hosts just work.
    Ssh { alias: String },
    /// A Cua Space (local VM/container, or one in the user's own cloud),
    /// driven through `cua sandbox exec`.
    CuaSpace { name: String },
    /// A running Docker container, driven through `docker exec -i`.
    Docker { container: String },
    /// A box at a cloud provider ([`crate::hosts`]), reached over SSH with a
    /// config the launcher writes after waking the box.
    Sandbox { provider: String, id: String },
    /// Another computer on the user's account, paired with this one and
    /// reached through the account's device relay ([`crate::paired`]).
    Node { device: String },
}

/// A program, its arguments and the local directory to start it in, ready
/// for [`crate::process::command`] or `dove_term::program_command`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Where the local process starts. For remote hosts this is not the
    /// agent's working directory (that is passed to the remote side).
    pub local_cwd: Option<PathBuf>,
}

impl Host {
    pub fn is_local(&self) -> bool {
        matches!(self, Host::Local)
    }

    /// Short label for the composer's "where" chip.
    pub fn label(&self) -> String {
        match self {
            Host::Local => "This PC".into(),
            Host::Wsl { distro: Some(d) } => format!("WSL: {d}"),
            Host::Wsl { distro: None } => "WSL".into(),
            Host::Ssh { alias } => format!("ssh: {alias}"),
            Host::CuaSpace { name } => format!("Space: {name}"),
            Host::Docker { container } => format!("Docker: {container}"),
            Host::Node { device } => crate::paired::name(device),
            Host::Sandbox { provider, id } => {
                let label = crate::hosts::provider(provider).map(|p| p.label()).unwrap_or(provider.as_str());
                format!("{label}: {id}")
            }
        }
    }

    /// Whether the agent's files live elsewhere and must be synced
    /// ([`crate::sync`]). WSL sees the Windows drive itself.
    pub fn needs_sync(&self) -> bool {
        !matches!(self, Host::Local | Host::Wsl { .. })
    }

    /// Rewrite `program args` (run in `cwd` on the host) into a local
    /// invocation. `tty` asks for an interactive terminal (TUI panes).
    pub fn wrap(&self, program: &str, args: &[String], cwd: &str, tty: bool) -> Invocation {
        match self {
            Host::Local => {
                Invocation { program: PathBuf::from(program), args: args.to_vec(), local_cwd: Some(PathBuf::from(cwd)) }
            }
            Host::Wsl { distro } => {
                let mut a = Vec::new();
                if let Some(d) = distro {
                    a.extend(["-d".into(), d.clone()]);
                }
                a.extend(["--cd".into(), to_wsl_path(cwd), "--".into(), program.into()]);
                a.extend(args.iter().cloned());
                Invocation { program: "wsl.exe".into(), args: a, local_cwd: None }
            }
            Host::Ssh { alias } => ssh_invocation(None, alias, program, args, cwd, tty),
            Host::Sandbox { provider, id } => match crate::hosts::saved_endpoint(provider, id) {
                Some(crate::hosts::Endpoint::Exec { argv }) => exec_invocation(&argv, program, args, cwd),
                Some(crate::hosts::Endpoint::Native) => {
                    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("dove"));
                    let mut prefix = vec![exe.display().to_string(), "node".into(), "bridge".into()];
                    prefix.extend([provider.clone(), id.clone(), "--".into()]);
                    exec_invocation(&prefix, program, args, cwd)
                }
                // SSH routes, and a box not connected yet (the launcher
                // connects first; the config then exists).
                _ => {
                    let config = crate::hosts::ssh_config_path(provider, id);
                    ssh_invocation(Some(&config), crate::hosts::SSH_ALIAS, program, args, cwd, tty)
                }
            },
            Host::Node { device } => {
                let mut argv: Vec<String> =
                    ["sh", "-lc", r#"cd "$HOME" 2>/dev/null; cd "$0" && exec "$@""#].map(String::from).to_vec();
                argv.extend([cwd.to_string(), program.to_string()]);
                argv.extend(args.iter().cloned());
                crate::paired::exec_invocation(device, &argv)
            }
            Host::Docker { container } => {
                let mut a: Vec<String> = vec!["exec".into(), if tty { "-it" } else { "-i" }.into(), container.clone()];
                // Start in the container user's home so a relative `cwd`
                // means the same thing it does over SSH.
                a.extend(["sh".into(), "-lc".into(), r#"cd "$HOME" 2>/dev/null; cd "$0" && exec "$@""#.into()]);
                a.extend([cwd.into(), program.into()]);
                a.extend(args.iter().cloned());
                Invocation { program: "docker".into(), args: a, local_cwd: None }
            }
            Host::CuaSpace { name } => {
                let mut a: Vec<String> = ["sandbox", "exec"].map(String::from).to_vec();
                // `-i` streams stdio and runs argv without a shell, with no
                // time limit: what an agent's JSON-over-stdio needs. The
                // `sh` hop makes a relative `cwd` mean "under the Space's
                // home", and a login shell finds user-installed agents.
                a.extend(["-i".into(), name.clone(), "sh".into(), "-lc".into()]);
                a.extend([r#"cd "$0" && exec "$@""#.into(), cwd.into(), program.into()]);
                a.extend(args.iter().cloned());
                Invocation { program: "cua".into(), args: a, local_cwd: None }
            }
        }
    }

    /// The agent's directory on this host for a local project folder. WSL
    /// sees the Windows drive; other hosts use `<home>/<folder name>`, where
    /// the user keeps (or Dove will sync) a checkout.
    pub fn remote_dir(&self, local: &Path) -> String {
        match self {
            Host::Local => local.display().to_string(),
            Host::Wsl { .. } => to_wsl_path(&local.display().to_string()),
            Host::Ssh { .. } | Host::CuaSpace { .. } | Host::Docker { .. } | Host::Sandbox { .. } | Host::Node { .. } => {
                local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| ".".into())
            }
        }
    }

    /// A stable folder name for this host's launchers and sync state.
    pub(crate) fn slug(&self) -> String {
        let raw = match self {
            Host::Local => "local".to_string(),
            Host::Wsl { distro } => format!("wsl-{}", distro.as_deref().unwrap_or("default")),
            Host::Ssh { alias } => format!("ssh-{alias}"),
            Host::CuaSpace { name } => format!("cua-{name}"),
            Host::Docker { container } => format!("docker-{container}"),
            Host::Node { device } => format!("node-{device}"),
            Host::Sandbox { provider, id } => format!("{provider}-{id}"),
        };
        raw.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect()
    }
}

/// `<prefix…> sh -lc 'cd "$0" && exec "$@"' <cwd> <program> <args…>`: a
/// vendor exec command with live stdio. A relative `cwd` is under `$HOME`.
fn exec_invocation(prefix: &[String], program: &str, args: &[String], cwd: &str) -> Invocation {
    let (head, rest) =
        prefix.split_first().map(|(h, r)| (h.clone(), r.to_vec())).unwrap_or_else(|| ("sh".into(), vec![]));
    let mut a = rest;
    a.extend(["sh".into(), "-lc".into(), r#"cd "$HOME" 2>/dev/null; cd "$0" && exec "$@""#.into()]);
    a.extend([cwd.into(), program.into()]);
    a.extend(args.iter().cloned());
    Invocation { program: head.into(), args: a, local_cwd: None }
}

/// `ssh [-F config] -T|-tt … target -- "cd <cwd> && exec <program> <args>"`.
fn ssh_invocation(
    config: Option<&Path>,
    target: &str,
    program: &str,
    args: &[String],
    cwd: &str,
    tty: bool,
) -> Invocation {
    let mut line = format!("cd {} && exec {}", sh_quote(cwd), sh_quote(program));
    for arg in args {
        line.push(' ');
        line.push_str(&sh_quote(arg));
    }
    let mut a: Vec<String> = Vec::new();
    if let Some(c) = config {
        a.extend(["-F".into(), c.display().to_string()]);
    }
    a.push(if tty { "-tt" } else { "-T" }.into());
    a.extend(["-o".into(), "BatchMode=yes".into()]);
    if !cfg!(windows) {
        // Windows OpenSSH has no ControlMaster support.
        a.extend(
            ["-o", "ControlMaster=auto", "-o", "ControlPersist=10m", "-o", "ControlPath=~/.ssh/dove-%C"]
                .map(String::from),
        );
    }
    a.extend([target.to_string(), "--".into(), line]);
    Invocation { program: "ssh".into(), args: a, local_cwd: None }
}

/// What a launcher reads from the `host.json` beside it.
const LAUNCHER_CONFIG: &str = "host.json";

/// A local executable that runs `program` on `host`: a hard link to this
/// very binary, named like the agent, with the host in `host.json` beside
/// it. Every driver (Dove's own and Zeron's) launches it like the real
/// agent, so argv, stdio and kill-the-tree all keep working, and
/// [`run_launcher`] does the hop when it starts.
pub fn launcher(host: &Host, program: &Path) -> anyhow::Result<PathBuf> {
    use anyhow::Context as _;
    let stem = program.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "agent".into());
    let dir = std::env::temp_dir().join("dove-hosts").join(host.slug());
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    std::fs::write(dir.join(LAUNCHER_CONFIG), serde_json::to_vec(host)?)?;
    let exe = std::env::current_exe().context("locating Keplorayx's own binary")?;
    let link = dir.join(format!("{stem}{}", std::env::consts::EXE_SUFFIX));
    // Re-link so a launcher follows Dove updates. A launcher another thread
    // is running can't be removed on Windows; reusing it is fine, since any
    // Dove build runs the same hop.
    let _ = std::fs::remove_file(&link);
    if !link.exists() && std::fs::hard_link(&exe, &link).is_err() {
        std::fs::copy(&exe, &link).with_context(|| format!("creating launcher {}", link.display()))?;
    }
    Ok(link)
}

/// Call first thing in `main`. When this process is a [`launcher`], run the
/// agent on its host with this process's stdio and return the exit code;
/// otherwise `None` (a normal Dove start).
pub fn run_launcher() -> Option<i32> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    if dir.parent()?.file_name()? != "dove-hosts" {
        return None;
    }
    let host: Host = serde_json::from_slice(&std::fs::read(dir.join(LAUNCHER_CONFIG)).ok()?).ok()?;
    let program = exe.file_stem()?.to_string_lossy().into_owned();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_default();
    match &host {
        Host::CuaSpace { name } => wake_space(name),
        Host::Sandbox { provider, id } => {
            if let Err(err) = crate::hosts::connect_box(provider, id) {
                eprintln!("dove: could not reach {}: {err:#}", host.label());
                return Some(1);
            }
        }
        _ => {}
    }
    let dir = host.remote_dir(&cwd);
    if let Err(code) = prepare(&host, &program, &dir, &cwd) {
        return Some(code);
    }
    // The driver hands some agents settings through the environment, which
    // the hop does not carry, so they ride along as `env K=V program …`.
    let pass: Vec<(String, String)> =
        std::env::vars().filter(|(k, _)| FORWARD_ENV_PREFIXES.iter().any(|p| k.starts_with(p))).collect();
    let (program, args) = with_env(&program, &args, &pass);
    let mut inv = host.wrap(&program, &args, &dir, false);
    // Agents served over HTTP (`opencode serve --port P --hostname
    // 127.0.0.1`) listen on the host's loopback; bring that port here.
    let port = served_port(&args);
    let mut forward = None;
    match (&host, port) {
        (Host::Ssh { .. } | Host::Sandbox { .. }, Some(p)) if inv.program == Path::new("ssh") => {
            let spec = format!("{p}:127.0.0.1:{p}");
            inv.args.splice(0..0, ["-o", "ExitOnForwardFailure=yes", "-L", spec.as_str()].map(String::from));
        }
        (Host::CuaSpace { name }, Some(p)) => {
            let mut fwd = std::process::Command::new("cua");
            fwd.args(["sandbox", "port-forward", name, &format!("{p}:{p}")])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                fwd.creation_flags(0x0800_0000);
            }
            match fwd.spawn() {
                Ok(child) => forward = Some(child),
                Err(err) => eprintln!("dove: could not forward port {p} from {}: {err}", host.label()),
            }
        }
        // WSL2 forwards loopback ports to Windows by itself.
        _ => {}
    }
    // On a machine that can drop off (SSH, a box, a Space, a container), run
    // the agent under `dove node attach` there when possible: it keeps
    // running through a lost connection and this launcher reattaches.
    if forward.is_none() && port.is_none() && host.needs_sync() && crate::node::remote::ensure(&host) {
        return Some(crate::node::remote::run_attached(&host, &dir, &program, &args, RECONNECT_FOR));
    }
    let mut cmd = std::process::Command::new(&inv.program);
    cmd.args(&inv.args);
    // Unix replaces this process, so the driver's signals reach the hop
    // directly, unless a port forward must be stopped afterwards. Windows
    // waits; the driver's job object kills the whole tree.
    #[cfg(unix)]
    {
        if forward.is_none() {
            use std::os::unix::process::CommandExt;
            let err = cmd.exec();
            eprintln!("dove: could not start {} for {}: {err}", inv.program.display(), host.label());
            return Some(127);
        }
    }
    let code = match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("dove: could not start {} for {}: {err}", inv.program.display(), host.label());
            127
        }
    };
    if let Some(mut child) = forward {
        let _ = child.kill();
        let _ = child.wait();
    }
    Some(code)
}

/// How long a launcher keeps trying to reattach to a remote agent after the
/// connection drops (a laptop asleep in a bag, a train tunnel).
const RECONNECT_FOR: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Environment the launcher passes on to the agent on its host. OpenCode's
/// driver sets its server password and config this way.
const FORWARD_ENV_PREFIXES: &[&str] = &["OPENCODE_"];

/// `program args` run under `env K=V …` when there is anything to pass.
fn with_env(program: &str, args: &[String], vars: &[(String, String)]) -> (String, Vec<String>) {
    if vars.is_empty() {
        return (program.to_string(), args.to_vec());
    }
    let mut a: Vec<String> = vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
    a.push(program.to_string());
    a.extend(args.iter().cloned());
    ("env".into(), a)
}

/// The loopback port an agent serves on: `serve … --port P`.
fn served_port(args: &[String]) -> Option<u16> {
    if !args.iter().any(|a| a == "serve") {
        return None;
    }
    let i = args.iter().position(|a| a == "--port")?;
    args.get(i + 1)?.parse().ok()
}

/// Gets `dir` ready on the host before the agent starts: the project is
/// synced there ([`crate::sync::push`]: unpushed commits and uncommitted
/// edits included; a folder that isn't a git repository falls back to
/// cloning `origin`), and the agent must be on the host's `PATH`. Everything
/// runs as separate processes with captured output, so nothing lands on the
/// agent's stdout (the JSON channel). On failure prints why and returns the
/// exit code to use.
fn prepare(host: &Host, agent: &str, dir: &str, local: &Path) -> Result<(), i32> {
    if host.needs_sync() {
        match crate::sync::push(host, local, dir) {
            Ok(outcome) => {
                if let Some(note) = outcome.note(host, dir, true) {
                    eprintln!("{note}");
                }
            }
            // Sync is best-effort: the prepare script below still clones
            // `origin` when the folder is missing.
            Err(err) => eprintln!("dove: could not sync this project to {}: {err:#}", host.label()),
        }
    }
    let git = |args: &[&str]| {
        let mut a = vec!["-C", local.to_str().unwrap_or(".")];
        a.extend_from_slice(args);
        quiet_output("git", &a).map(|o| String::from_utf8_lossy(&o).trim().to_string()).filter(|s| !s.is_empty())
    };
    let (url, branch) = match host {
        // WSL sees the Windows folder itself; nothing to clone.
        Host::Wsl { .. } | Host::Local => (None, None),
        _ => (git(&["remote", "get-url", "origin"]), git(&["rev-parse", "--abbrev-ref", "HEAD"])),
    };
    let args = prepare_args(dir, url.as_deref(), branch.as_deref().filter(|b| *b != "HEAD"), agent);
    // Start in the host's home: `dir` may not exist yet.
    let inv = host.wrap("sh", &args, ".", false);
    let mut cmd = std::process::Command::new(&inv.program);
    cmd.args(&inv.args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = match cmd.output() {
        Ok(out) => out,
        Err(err) => {
            eprintln!("dove: could not reach {} ({}): {err}", host.label(), inv.program.display());
            return Err(127);
        }
    };
    let said = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        // Notes (a fresh clone, a branch the remote lacks) are not errors.
        if !said.trim().is_empty() {
            eprintln!("{}", said.trim());
        }
        return Ok(());
    }
    let code = out.status.code().unwrap_or(1);
    match code {
        PREPARE_NO_AGENT => eprintln!("{}", missing_agent_hint(agent, &host.label())),
        _ => eprintln!("dove: could not prepare {dir} on {}: {}", host.label(), said.trim()),
    }
    Err(code)
}

/// Exit code of the prepare script when the agent is not installed.
const PREPARE_NO_AGENT: i32 = 127;

/// `sh -c <script> dove-prepare <dir> <url> <branch> <agent>`.
fn prepare_args(dir: &str, url: Option<&str>, branch: Option<&str>, agent: &str) -> Vec<String> {
    const SCRIPT: &str = r#"dir=$1 url=$2 branch=$3 agent=$4
if [ ! -d "$dir" ]; then
  if [ -z "$url" ]; then mkdir -p "$dir" || exit 1
  else
    echo "dove: cloning $url into ~/$dir" >&2
    git clone -q "$url" "$dir" >&2 || exit 1
    if [ -n "$branch" ] && ! git -C "$dir" checkout -q "$branch" >&2 2>/dev/null; then
      echo "dove: branch $branch is not on origin; using the default branch" >&2
    fi
  fi
fi
command -v "$agent" >/dev/null 2>&1 || exit 127"#;
    ["-c", SCRIPT, "dove-prepare", dir, url.unwrap_or(""), branch.unwrap_or(""), agent].map(String::from).to_vec()
}

/// What to tell the user when `agent` is missing on `host`.
fn missing_agent_hint(agent: &str, host: &str) -> String {
    let install = match agent {
        "claude" => Some("npm install -g @anthropic-ai/claude-code"),
        "codex" => Some("npm install -g @openai/codex"),
        "gemini" => Some("npm install -g @google/gemini-cli"),
        "opencode" => Some("npm install -g opencode-ai"),
        _ => None,
    };
    match install {
        Some(cmd) => {
            format!("dove: {agent} is not installed on {host}. Install it there with `{cmd}`, then try again.")
        }
        None => format!("dove: {agent} is not installed on {host}. Install it there, then try again."),
    }
}

/// Resume a paused or suspended Space: `exec` refuses one. `resume` fails on
/// a running Space, so it only runs when `info` says the Space is asleep.
fn wake_space(name: &str) {
    let Some(raw) = quiet_output("cua", &["sandbox", "info", name, "--json"]) else { return };
    let Ok(info) = serde_json::from_slice::<serde_json::Value>(&raw) else { return };
    if space_asleep(&info) {
        let _ = quiet_output("cua", &["sandbox", "resume", name]);
    }
}

fn space_asleep(info: &serde_json::Value) -> bool {
    ["state", "status"].iter().any(|k| {
        info[*k].as_str().is_some_and(|v| {
            let v = v.to_ascii_lowercase();
            v.contains("pause") || v.contains("suspend")
        })
    })
}

/// `C:\Users\me\repo` -> `/mnt/c/Users/me/repo`; Linux paths pass through.
pub fn to_wsl_path(path: &str) -> String {
    let b = path.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        let rest = path[2..].replace('\\', "/");
        let rest = rest.trim_start_matches('/');
        let drive = (b[0] as char).to_ascii_lowercase();
        return if rest.is_empty() { format!("/mnt/{drive}") } else { format!("/mnt/{drive}/{rest}") };
    }
    path.to_string()
}

/// Quote for a POSIX shell: `it's` -> `'it'\''s'`.
pub fn sh_quote(s: &str) -> String {
    if !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || b"-_./=:@,+".contains(&c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Hosts this machine can reach right now, for the "Add a machine" list.
/// Each probe is best-effort and silent when the tool is missing.
pub fn discover() -> Vec<Host> {
    let mut hosts = vec![Host::Local];
    hosts.extend(wsl_distros().into_iter().map(|d| Host::Wsl { distro: Some(d) }));
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let config = std::fs::read_to_string(Path::new(&home).join(".ssh").join("config")).unwrap_or_default();
        hosts.extend(ssh_aliases(&config).into_iter().map(|alias| Host::Ssh { alias }));
    }
    hosts.extend(cua_spaces().into_iter().map(|name| Host::CuaSpace { name }));
    hosts.extend(docker_containers().into_iter().map(|container| Host::Docker { container }));
    hosts.extend(sandboxes());
    hosts.extend(crate::paired::devices().into_iter().map(|d| Host::Node { device: d.id }));
    hosts
}

/// Running Docker containers, by name.
pub fn docker_containers() -> Vec<String> {
    quiet_output("docker", &["ps", "--format", "{{.Names}}"])
        .map(|raw| {
            String::from_utf8_lossy(&raw).lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
        })
        .unwrap_or_default()
}

/// Boxes at every cloud provider the user has a key for. A sleeping box is
/// listed too: the launcher wakes it.
pub fn sandboxes() -> Vec<Host> {
    let keys = crate::hosts::Keys::load();
    crate::hosts::providers()
        .into_iter()
        .filter(|p| !p.needs_key() || keys.has(p.id()))
        .flat_map(|p| {
            p.list(&keys)
                .unwrap_or_default()
                .into_iter()
                .filter(|b| !matches!(b.state.as_str(), "deleting" | "deleted" | "error"))
                .map(|b| Host::Sandbox { provider: p.id().to_string(), id: b.id })
        })
        .collect()
}

/// Can `host` run a command right now? Runs `true` there (a box is not woken
/// for this; its provider's probe answers instead).
pub fn probe(host: &Host) -> crate::hosts::Probe {
    use crate::hosts::Probe;
    if let Host::Sandbox { provider, .. } = host {
        return match crate::hosts::provider(provider) {
            Some(p) => p.probe(&crate::hosts::Keys::load()),
            None => Probe::Unavailable(format!("unknown provider {provider}")),
        };
    }
    let inv = host.wrap("true", &[], ".", false);
    let mut cmd = std::process::Command::new(&inv.program);
    cmd.args(&inv.args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    match cmd.output() {
        Ok(out) if out.status.success() => Probe::Ready,
        Ok(out) => {
            Probe::NeedsSetup(String::from_utf8_lossy(&out.stderr).trim().lines().last().unwrap_or("").to_string())
        }
        Err(err) => Probe::Unavailable(format!("{} is not installed: {err}", inv.program.display())),
    }
}

fn quiet_output(program: &str, args: &[&str]) -> Option<Vec<u8>> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// Installed WSL distros, minus Docker Desktop's internal ones.
pub fn wsl_distros() -> Vec<String> {
    if !cfg!(windows) {
        return Vec::new();
    }
    quiet_output("wsl.exe", &["-l", "-q"]).map(|raw| parse_wsl_list(&raw)).unwrap_or_default()
}

/// `wsl -l -q` prints UTF-16LE.
fn parse_wsl_list(raw: &[u8]) -> Vec<String> {
    let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
        .lines()
        .map(|l| l.trim().trim_matches('\0').to_string())
        .filter(|l| !l.is_empty() && !l.starts_with("docker-desktop"))
        .collect()
}

/// Concrete `Host` aliases from an ssh config (wildcards skipped).
pub fn ssh_aliases(config: &str) -> Vec<String> {
    config
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let (key, rest) = l.split_once(char::is_whitespace)?;
            key.eq_ignore_ascii_case("host").then_some(rest)
        })
        .flat_map(|rest| rest.split_whitespace())
        .filter(|a| !a.contains(['*', '?', '!']))
        .map(String::from)
        .collect()
}

/// Ready Cua Spaces, from `cua sandbox ls --json`.
pub fn cua_spaces() -> Vec<String> {
    quiet_output("cua", &["sandbox", "ls", "--json"])
        .and_then(|raw| serde_json::from_slice::<Vec<serde_json::Value>>(&raw).ok())
        .map(|list| parse_cua_spaces(&list))
        .unwrap_or_default()
}

fn parse_cua_spaces(list: &[serde_json::Value]) -> Vec<String> {
    list.iter()
        .filter(|s| s["status"] == "ready")
        .filter_map(|s| s["id"].as_str().or_else(|| s["name"].as_str()).map(String::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn local_is_untouched() {
        let inv = Host::Local.wrap("claude", &s(&["-p", "hi"]), "C:\\repo", false);
        assert_eq!(inv.program, PathBuf::from("claude"));
        assert_eq!(inv.args, s(&["-p", "hi"]));
        assert_eq!(inv.local_cwd, Some(PathBuf::from("C:\\repo")));
    }

    #[test]
    fn wsl_maps_the_drive_path() {
        let inv =
            Host::Wsl { distro: Some("Ubuntu".into()) }.wrap("codex", &s(&["exec"]), "C:\\Users\\me\\repo", false);
        assert_eq!(inv.program, PathBuf::from("wsl.exe"));
        assert_eq!(inv.args, s(&["-d", "Ubuntu", "--cd", "/mnt/c/Users/me/repo", "--", "codex", "exec"]));
        assert_eq!(to_wsl_path("/home/me"), "/home/me");
        assert_eq!(to_wsl_path("D:\\"), "/mnt/d");
    }

    #[test]
    fn ssh_quotes_one_remote_line() {
        let inv = Host::Ssh { alias: "box".into() }.wrap("claude", &s(&["-p", "it's done"]), "/srv/my repo", false);
        assert_eq!(inv.program, PathBuf::from("ssh"));
        assert_eq!(inv.args.first().map(String::as_str), Some("-T"));
        let line = inv.args.last().unwrap();
        assert_eq!(line, r"cd '/srv/my repo' && exec claude -p 'it'\''s done'");
        assert_eq!(inv.args[inv.args.len() - 3], "box");
    }

    #[test]
    fn cua_streams_argv() {
        let inv = Host::CuaSpace { name: "local:space-1".into() }.wrap("gemini", &s(&["--acp"]), "/work", false);
        assert_eq!(inv.program, PathBuf::from("cua"));
        assert_eq!(
            inv.args,
            s(&[
                "sandbox",
                "exec",
                "-i",
                "local:space-1",
                "sh",
                "-lc",
                r#"cd "$0" && exec "$@""#,
                "/work",
                "gemini",
                "--acp"
            ])
        );
    }

    #[test]
    fn docker_execs_with_a_relative_home() {
        let inv = Host::Docker { container: "dev".into() }.wrap("claude", &s(&["-p"]), "repo", false);
        assert_eq!(inv.program, PathBuf::from("docker"));
        assert_eq!(&inv.args[..3], &s(&["exec", "-i", "dev"])[..]);
        assert_eq!(&inv.args[inv.args.len() - 3..], &s(&["repo", "claude", "-p"])[..]);
        let tty = Host::Docker { container: "dev".into() }.wrap("sh", &[], ".", true);
        assert_eq!(tty.args[1], "-it");
    }

    #[test]
    fn sandbox_uses_its_generated_ssh_config() {
        let h = Host::Sandbox { provider: "instabox".into(), id: "m_1".into() };
        let inv = h.wrap("claude", &s(&["-p"]), "repo", false);
        assert_eq!(inv.program, PathBuf::from("ssh"));
        assert_eq!(inv.args[0], "-F");
        assert!(inv.args[1].ends_with("ssh_config"));
        assert!(inv.args.iter().any(|a| a == crate::hosts::SSH_ALIAS));
        assert_eq!(h.label(), "InstaBox: m_1");
        assert!(h.needs_sync());
        assert!(!Host::Wsl { distro: None }.needs_sync());
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(json, r#"{"kind":"sandbox","provider":"instabox","id":"m_1"}"#);
    }

    #[test]
    fn remote_dirs() {
        let p = Path::new("C:\\code\\dove");
        assert_eq!(Host::Wsl { distro: None }.remote_dir(p), "/mnt/c/code/dove");
        // A local folder's own name: a Windows path only splits on Windows.
        let native = Path::new(if cfg!(windows) { "C:\\code\\dove" } else { "/code/dove" });
        assert_eq!(Host::Ssh { alias: "b".into() }.remote_dir(native), "dove");
        assert_eq!(Host::CuaSpace { name: "local:x".into() }.slug(), "cua-local_x");
    }

    #[test]
    fn parses_discovery_outputs() {
        let utf16: Vec<u8> = "Ubuntu\r\ndocker-desktop\r\n".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(parse_wsl_list(&utf16), s(&["Ubuntu"]));
        let cfg = "Host *\n  User x\nHost hetzner vps2\n  HostName 1.2.3.4\nhost github.com\n";
        assert_eq!(ssh_aliases(cfg), s(&["hetzner", "vps2", "github.com"]));
        let list: Vec<serde_json::Value> =
            serde_json::from_str(r#"[{"id":"local:a","status":"ready"},{"id":"local:b","status":"stopped"}]"#).unwrap();
        assert_eq!(parse_cua_spaces(&list), s(&["local:a"]));
        assert!(space_asleep(&serde_json::json!({"state": "Paused", "status": "ready"})));
        assert!(!space_asleep(&serde_json::json!({"state": "running", "status": "ready"})));
    }

    #[test]
    fn host_round_trips_through_settings_json() {
        let h = Host::Ssh { alias: "box".into() };
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(json, r#"{"kind":"ssh","alias":"box"}"#);
        assert_eq!(serde_json::from_str::<Host>(&json).unwrap(), h);
    }

    #[test]
    fn prepare_passes_values_as_arguments_not_script() {
        let a = prepare_args("repo", Some("https://x/y.git"), None, "claude");
        assert_eq!(a[0], "-c");
        assert_eq!(&a[2..], &s(&["dove-prepare", "repo", "https://x/y.git", "", "claude"])[..]);
        assert!(!a[1].contains("repo"), "values never land in the script text");
    }

    #[test]
    fn served_port_comes_from_serve_args_only() {
        assert_eq!(served_port(&s(&["serve", "--port", "4123", "--hostname", "127.0.0.1"])), Some(4123));
        assert_eq!(served_port(&s(&["run", "--port", "4123"])), None);
        assert_eq!(served_port(&s(&["serve"])), None);
    }

    #[test]
    fn env_rides_along_before_the_program() {
        let (p, a) = with_env("opencode", &s(&["serve"]), &[("OPENCODE_PASSWORD".into(), "x y".into())]);
        assert_eq!((p.as_str(), a), ("env", s(&["OPENCODE_PASSWORD=x y", "opencode", "serve"])));
        assert_eq!(with_env("claude", &s(&["-p"]), &[]), ("claude".to_string(), s(&["-p"])));
    }

    #[test]
    fn missing_agent_hint_names_the_install() {
        assert!(missing_agent_hint("claude", "ssh: box").contains("@anthropic-ai/claude-code"));
        assert!(missing_agent_hint("amp", "WSL").contains("Install it there"));
    }
}
