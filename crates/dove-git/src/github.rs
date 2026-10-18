//! Pull requests through the GitHub CLI (`gh`), which owns authentication.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrFilter {
    /// Opened by the signed-in user.
    Authored,
    /// Awaiting the signed-in user's review.
    Reviewing,
    All,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub branch: String,
    pub state: String,
    pub draft: bool,
    pub url: String,
    pub updated_at: String,
    pub review: Option<String>,
    pub additions: u64,
    pub deletions: u64,
}

/// Whether `gh` is installed. Cheap: a PATH lookup, no process.
pub fn gh_available() -> bool {
    which_gh().is_some()
}

/// Where `gh` lives, if anywhere: PATH first, then the folders its
/// installers use. Dove's PATH is fixed at launch, so a `gh` installed
/// since (from Settings › Integrations, say) is only found there.
pub fn gh_path() -> Option<std::path::PathBuf> {
    which_gh()
}

fn which_gh() -> Option<std::path::PathBuf> {
    let exe = if cfg!(windows) { "gh.exe" } else { "gh" };
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|d| d.join(exe));
    on_path.chain(install_dirs().into_iter().map(|d| d.join(exe))).find(|p| p.is_file())
}

/// Where the GitHub CLI's installers put it, beyond PATH.
fn install_dirs() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut dirs = Vec::new();
    if cfg!(windows) {
        // winget / MSI (machine and per-user), Scoop and Chocolatey.
        dirs.extend(env("ProgramFiles").map(|p| p.join("GitHub CLI")));
        dirs.extend(env("LOCALAPPDATA").map(|p| p.join("Programs").join("GitHub CLI")));
        dirs.extend(env("USERPROFILE").map(|p| p.join("scoop").join("shims")));
        dirs.extend(env("ProgramData").map(|p| p.join("chocolatey").join("bin")));
    } else {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/home/linuxbrew/.linuxbrew/bin"].map(PathBuf::from));
    }
    dirs
}

/// The signed-in GitHub account, `None` when `gh` is missing or signed out.
pub fn gh_user() -> Option<String> {
    let gh = which_gh()?;
    let mut cmd = Command::new(gh);
    cmd.env("GH_PROMPT_DISABLED", "1").args(["api", "user", "--jq", ".login"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    let login = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !login.is_empty()).then_some(login)
}

/// Install the GitHub CLI with the platform's package manager: winget on
/// Windows (it may ask for admin rights), Homebrew on macOS. Blocks until
/// the installer exits.
pub fn install_gh() -> Result<()> {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("winget");
        c.args([
            "install",
            "--id",
            "GitHub.cli",
            "--exact",
            "--source",
            "winget",
            "--silent",
            "--accept-package-agreements",
            "--accept-source-agreements",
        ]);
        c
    } else if cfg!(target_os = "macos") {
        let brew = ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
            .into_iter()
            .find(|p| Path::new(p).is_file())
            .context("Homebrew is not installed (brew.sh), or install gh from cli.github.com")?;
        let mut c = Command::new(brew);
        c.args(["install", "gh"]);
        c
    } else {
        bail!("install gh with your package manager (see cli.github.com)");
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().context(if cfg!(windows) { "winget is not available" } else { "running brew" })?;
    if which_gh().is_some() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr }).into_owned();
    let last = text.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("the installer did not finish");
    bail!("{last}")
}

/// Open a terminal window running `gh auth login`, which signs in through
/// the browser. Returns once the window is open, not when sign-in ends.
pub fn open_gh_login() -> Result<()> {
    let gh = which_gh().context("the GitHub CLI (gh) is not installed")?;
    let gh = gh.to_string_lossy().into_owned();
    if cfg!(windows) {
        Command::new("cmd")
            .args(["/c", "start", "GitHub sign-in", "cmd", "/k", &gh, "auth", "login", "--web", "--hostname", "github.com", "--git-protocol", "https"])
            .spawn()
            .context("opening a terminal")?;
    } else if cfg!(target_os = "macos") {
        let script = format!("tell application \"Terminal\" to do script \"'{gh}' auth login --web --hostname github.com --git-protocol https\"");
        Command::new("osascript").args(["-e", &script, "-e", "tell application \"Terminal\" to activate"]).spawn().context("opening Terminal")?;
    } else {
        let line = format!("'{gh}' auth login --web --hostname github.com --git-protocol https; exec $SHELL");
        Command::new("x-terminal-emulator").args(["-e", "sh", "-c", &line]).spawn().context("opening a terminal")?;
    }
    Ok(())
}

fn gh(repo: &Path, args: &[&str]) -> Result<String> {
    let gh = which_gh().context("the GitHub CLI (gh) is not installed")?;
    let mut cmd = Command::new(gh);
    cmd.current_dir(repo).env("GH_PROMPT_DISABLED", "1").args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().context("running gh")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A pull request with its description and checks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PrDetail {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    pub state: String,
    pub branch: String,
    pub base: String,
    pub mergeable: String,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub name: String,
    /// `SUCCESS`, `FAILURE`, `PENDING`, ...
    pub state: String,
    pub url: String,
}

impl PrDetail {
    /// `passing`, `failing`, `pending` or `none`.
    pub fn checks_summary(&self) -> &'static str {
        let failed = |s: &str| matches!(s, "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED");
        let ok = |s: &str| matches!(s, "SUCCESS" | "NEUTRAL" | "SKIPPED");
        if self.checks.is_empty() {
            "none"
        } else if self.checks.iter().any(|c| failed(&c.state)) {
            "failing"
        } else if self.checks.iter().all(|c| ok(&c.state)) {
            "passing"
        } else {
            "pending"
        }
    }
}

const DETAIL_FIELDS: &str = "number,title,body,url,state,headRefName,baseRefName,mergeable,statusCheckRollup";

fn parse_detail(json: &str) -> Result<PrDetail> {
    let p: serde_json::Value = serde_json::from_str(json).context("unexpected gh output")?;
    let s = |k: &str| p[k].as_str().unwrap_or_default().to_string();
    let checks = p["statusCheckRollup"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| Check {
                    name: c["name"].as_str().or_else(|| c["context"].as_str()).unwrap_or("check").to_string(),
                    state: c["conclusion"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .or_else(|| c["state"].as_str())
                        .or_else(|| c["status"].as_str())
                        .unwrap_or("PENDING")
                        .to_ascii_uppercase(),
                    url: c["detailsUrl"].as_str().or_else(|| c["targetUrl"].as_str()).unwrap_or_default().to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(PrDetail {
        number: p["number"].as_u64().unwrap_or(0),
        title: s("title"),
        body: s("body"),
        url: s("url"),
        state: s("state"),
        branch: s("headRefName"),
        base: s("baseRefName"),
        mergeable: s("mergeable"),
        checks,
    })
}

/// Details of PR `number`.
pub fn pr_detail(repo: &Path, number: u64) -> Result<PrDetail> {
    parse_detail(&gh(repo, &["pr", "view", &number.to_string(), "--json", DETAIL_FIELDS])?)
}

/// The PR for the checked-out branch of `wt`, if any.
pub fn pr_for_branch(wt: &Path) -> Result<Option<PrDetail>> {
    match gh(wt, &["pr", "view", "--json", DETAIL_FIELDS]) {
        Ok(json) => Ok(Some(parse_detail(&json)?)),
        Err(e) if e.to_string().to_ascii_lowercase().contains("no pull requests found") => Ok(None),
        Err(e) => Err(e),
    }
}

/// The unified diff of PR `number`.
pub fn pr_diff(repo: &Path, number: u64) -> Result<String> {
    gh(repo, &["pr", "diff", &number.to_string(), "--color", "never"])
}

/// Open a PR from the current branch of `wt`. Returns its URL.
pub fn create_pr(wt: &Path, title: &str, body: &str, base: Option<&str>, draft: bool) -> Result<String> {
    let mut args = vec!["pr", "create", "--title", title, "--body", body];
    if let Some(b) = base {
        args.extend(["--base", b]);
    }
    if draft {
        args.push("--draft");
    }
    Ok(gh(wt, &args)?.lines().last().unwrap_or_default().trim().to_string())
}

/// Squash-merge PR `number` and delete its branch.
pub fn merge_pr(wt: &Path, number: u64) -> Result<()> {
    gh(wt, &["pr", "merge", &number.to_string(), "--squash", "--delete-branch"])?;
    Ok(())
}

/// Fetch PR `number`'s head into the local branch `pr-<number>` (for a new
/// worktree). Returns the branch name.
pub fn fetch_pr(repo: &Path, number: u64) -> Result<String> {
    let local = format!("pr-{number}");
    let refspec = format!("pull/{number}/head:{local}");
    let mut cmd = Command::new("git");
    cmd.current_dir(repo).env("GIT_TERMINAL_PROMPT", "0").args(["fetch", "-q", "-f", "origin", &refspec]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().context("git is not installed")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(local)
}

pub fn pull_requests(repo: &Path, filter: PrFilter) -> Result<Vec<PullRequest>> {
    let gh = which_gh().context("the GitHub CLI (gh) is not installed")?;
    let mut cmd = Command::new(gh);
    cmd.current_dir(repo).env("GH_PROMPT_DISABLED", "1").args([
        "pr",
        "list",
        "--limit",
        "50",
        "--json",
        "number,title,author,headRefName,state,isDraft,url,updatedAt,reviewDecision,additions,deletions",
    ]);
    match filter {
        PrFilter::Authored => {
            cmd.args(["--author", "@me"]);
        }
        PrFilter::Reviewing => {
            cmd.args(["--search", "review-requested:@me"]);
        }
        PrFilter::All => {}
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().context("running gh")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    parse(&String::from_utf8_lossy(&out.stdout))
}

fn parse(json: &str) -> Result<Vec<PullRequest>> {
    let v: serde_json::Value = serde_json::from_str(json).context("unexpected gh output")?;
    let s = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or_default().to_string();
    Ok(v.as_array()
        .map(|a| {
            a.iter()
                .map(|p| PullRequest {
                    number: p["number"].as_u64().unwrap_or(0),
                    title: s(p, "title"),
                    author: s(&p["author"], "login"),
                    branch: s(p, "headRefName"),
                    state: s(p, "state"),
                    draft: p["isDraft"].as_bool().unwrap_or(false),
                    url: s(p, "url"),
                    updated_at: s(p, "updatedAt"),
                    review: p["reviewDecision"].as_str().filter(|r| !r.is_empty()).map(Into::into),
                    additions: p["additions"].as_u64().unwrap_or(0),
                    deletions: p["deletions"].as_u64().unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pr_detail_and_checks() {
        let d = parse_detail(
            r#"{"number":3,"title":"T","body":"B","url":"u","state":"OPEN","headRefName":"dove/x","baseRefName":"main",
               "mergeable":"MERGEABLE","statusCheckRollup":[
                 {"name":"ci","conclusion":"SUCCESS","detailsUrl":"d"},
                 {"context":"lint","state":"PENDING","targetUrl":"t"}]}"#,
        )
        .unwrap();
        assert_eq!((d.number, d.base.as_str(), d.checks.len()), (3, "main", 2));
        assert_eq!(d.checks_summary(), "pending");
        assert_eq!(d.checks[1].name, "lint");
    }

    #[test]
    fn parses_gh_json() {
        let prs = parse(
            r#"[{"number":7,"title":"Add dove","author":{"login":"rav"},"headRefName":"dove/x","state":"OPEN",
               "isDraft":true,"url":"https://github.com/a/b/pull/7","updatedAt":"2026-10-01T00:00:00Z",
               "reviewDecision":"","additions":10,"deletions":2}]"#,
        )
        .unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!((prs[0].number, prs[0].draft, prs[0].author.as_str()), (7, true, "rav"));
        assert_eq!(prs[0].review, None);
    }
}
