//! Pull requests through the GitHub CLI (`gh`), which owns authentication.

use std::path::{Path, PathBuf};
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
/// since (from Settings › Plugins, say) is only found there.
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
        dirs.extend(
            ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/home/linuxbrew/.linuxbrew/bin"].map(PathBuf::from),
        );
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

/// The signed-in account's token (`gh auth token`), for services that take
/// a GitHub token, such as GitHub's MCP server. `None` when signed out.
pub fn gh_token() -> Option<String> {
    let gh = which_gh()?;
    let mut cmd = Command::new(gh);
    cmd.env("GH_PROMPT_DISABLED", "1").args(["auth", "token"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !token.is_empty()).then_some(token)
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
    let last = text.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("the installer did not finish");
    bail!("{last}")
}

/// Open a terminal window running `gh auth login`, which signs in through
/// the browser. Returns once the window is open, not when sign-in ends.
pub fn open_gh_login() -> Result<()> {
    let gh = which_gh().context("the GitHub CLI (gh) is not installed")?;
    let gh = gh.to_string_lossy().into_owned();
    if cfg!(windows) {
        Command::new("cmd")
            .args([
                "/c",
                "start",
                "GitHub sign-in",
                "cmd",
                "/k",
                &gh,
                "auth",
                "login",
                "--web",
                "--hostname",
                "github.com",
                "--git-protocol",
                "https",
            ])
            .spawn()
            .context("opening a terminal")?;
    } else if cfg!(target_os = "macos") {
        let script = format!(
            "tell application \"Terminal\" to do script \"'{gh}' auth login --web --hostname github.com --git-protocol https\""
        );
        Command::new("osascript")
            .args(["-e", &script, "-e", "tell application \"Terminal\" to activate"])
            .spawn()
            .context("opening Terminal")?;
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

/// Where a pull request lives: a local clone (gh infers the repository from
/// its remote) or a GitHub `owner/repo` that may not be cloned at all, as
/// for a PR link an agent printed in chat.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PrRepo {
    Local(PathBuf),
    Remote(String),
}

impl PrRepo {
    /// Run `gh <args>` against this repository.
    fn gh(&self, args: &[&str]) -> Result<String> {
        match self {
            PrRepo::Local(dir) => gh(dir, args),
            PrRepo::Remote(slug) => {
                let mut all = args.to_vec();
                all.extend(["-R", slug.as_str()]);
                gh(&std::env::temp_dir(), &all)
            }
        }
    }
}

/// `https://github.com/<owner>/<repo>/pull/<n>[/...]` → (`owner/repo`, n).
pub fn parse_pr_url(url: &str) -> Option<(String, u64)> {
    let url = url.trim();
    let rest = url.strip_prefix("https://github.com/").or_else(|| url.strip_prefix("http://github.com/"))?;
    let mut parts = rest.split('/');
    let (owner, repo, kind, num) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if kind != "pull" || !valid(owner) || !valid(repo) {
        return None;
    }
    let digits: String = num.chars().take_while(char::is_ascii_digit).collect();
    let n = digits.parse().ok().filter(|n| *n > 0)?;
    Some((format!("{owner}/{repo}"), n))
}

/// Every distinct GitHub pull request URL in `text`, canonicalised to
/// `https://github.com/<owner>/<repo>/pull/<n>`, in order of appearance.
pub fn pr_urls(text: &str) -> Vec<String> {
    const SCHEMES: [&str; 2] = ["https://github.com/", "http://github.com/"];
    let mut out: Vec<String> = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let Some((start, _)) = SCHEMES.iter().filter_map(|s| text[at..].find(s).map(|i| (at + i, *s))).min() else {
            break;
        };
        let end = text[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | '>' | '"' | '\'' | ']' | ',' | '`'))
            .map_or(text.len(), |e| start + e);
        if let Some((slug, n)) = parse_pr_url(&text[start..end]) {
            let url = format!("https://github.com/{slug}/pull/{n}");
            if !out.contains(&url) {
                out.push(url);
            }
        }
        at = end.max(start + 1);
    }
    out
}

/// A pull request with its description, checks and (for [`pr_view`]) its
/// conversation, commits and files.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PrDetail {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    /// `OPEN`, `CLOSED` or `MERGED`.
    pub state: String,
    pub branch: String,
    pub base: String,
    pub mergeable: String,
    pub checks: Vec<Check>,
    pub author: String,
    pub draft: bool,
    pub additions: u64,
    pub deletions: u64,
    pub head_sha: String,
    pub created_at: String,
    /// `APPROVED`, `CHANGES_REQUESTED`, `REVIEW_REQUIRED` or empty.
    pub review_decision: String,
    /// `CLEAN`, `BLOCKED`, `BEHIND`, `DIRTY`, `UNSTABLE`, ... or empty.
    pub merge_state: String,
    pub changed_files: u64,
    pub reviews: Vec<Review>,
    pub comments: Vec<Comment>,
    pub commits: Vec<Commit>,
    pub files: Vec<PrFile>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Review {
    pub author: String,
    /// `APPROVED`, `CHANGES_REQUESTED`, `COMMENTED`, ...
    pub state: String,
    pub body: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Comment {
    pub author: String,
    pub body: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Commit {
    pub oid: String,
    pub headline: String,
    pub author: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrFile {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

/// How [`merge`] lands a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMethod {
    Squash,
    Merge,
    Rebase,
}

/// A review verdict for [`review`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewKind {
    Approve,
    RequestChanges,
    Comment,
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

/// What a branch lookup or a chat card needs: cheap for gh to answer.
const DETAIL_FIELDS: &str = concat!(
    "number,title,body,url,state,headRefName,baseRefName,mergeable,statusCheckRollup,",
    "author,isDraft,additions,deletions,headRefOid,changedFiles,reviewDecision"
);
/// Everything the pull request view shows.
const FULL_FIELDS: &str = concat!(
    "number,title,body,url,state,headRefName,baseRefName,mergeable,statusCheckRollup,",
    "author,isDraft,additions,deletions,headRefOid,changedFiles,reviewDecision,createdAt,",
    "mergeStateStatus,reviews,comments,commits,files"
);

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
    let list = |k: &str| p[k].as_array().cloned().unwrap_or_default();
    let text = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or_default().to_string();
    let login = |v: &serde_json::Value| v["author"]["login"].as_str().unwrap_or_default().to_string();
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
        author: login(&p),
        draft: p["isDraft"].as_bool().unwrap_or(false),
        additions: p["additions"].as_u64().unwrap_or(0),
        deletions: p["deletions"].as_u64().unwrap_or(0),
        head_sha: s("headRefOid"),
        created_at: s("createdAt"),
        review_decision: s("reviewDecision"),
        merge_state: s("mergeStateStatus"),
        changed_files: p["changedFiles"].as_u64().unwrap_or(0),
        reviews: list("reviews")
            .iter()
            .map(|r| Review {
                author: login(r),
                state: text(r, "state"),
                body: text(r, "body"),
                at: text(r, "submittedAt"),
            })
            .collect(),
        comments: list("comments")
            .iter()
            .map(|c| Comment { author: login(c), body: text(c, "body"), at: text(c, "createdAt") })
            .collect(),
        commits: list("commits")
            .iter()
            .map(|c| Commit {
                oid: text(c, "oid"),
                headline: text(c, "messageHeadline"),
                author: c["authors"][0]["login"]
                    .as_str()
                    .or_else(|| c["authors"][0]["name"].as_str())
                    .unwrap_or_default()
                    .to_string(),
                at: text(c, "committedDate"),
            })
            .collect(),
        files: list("files")
            .iter()
            .map(|f| PrFile {
                path: text(f, "path"),
                additions: f["additions"].as_u64().unwrap_or(0),
                deletions: f["deletions"].as_u64().unwrap_or(0),
            })
            .collect(),
    })
}

impl PrDetail {
    /// `merged`, `closed`, `draft` or `open`.
    pub fn status(&self) -> &'static str {
        match self.state.as_str() {
            "MERGED" => "merged",
            "CLOSED" => "closed",
            _ if self.draft => "draft",
            _ => "open",
        }
    }
}

/// The light details of PR `number` (title, state, checks, size): enough
/// for a chat card or a branch's PR chip.
pub fn pr_summary(repo: &PrRepo, number: u64) -> Result<PrDetail> {
    parse_detail(&repo.gh(&["pr", "view", &number.to_string(), "--json", DETAIL_FIELDS])?)
}

/// Everything about PR `number`: details, checks, reviews, comments,
/// commits and files.
pub fn pr_view(repo: &PrRepo, number: u64) -> Result<PrDetail> {
    parse_detail(&repo.gh(&["pr", "view", &number.to_string(), "--json", FULL_FIELDS])?)
}

/// The unified diff of PR `number`.
pub fn diff(repo: &PrRepo, number: u64) -> Result<String> {
    repo.gh(&["pr", "diff", &number.to_string(), "--color", "never"])
}

/// Merge PR `number` with `method`. Deletes the branch only for a local
/// clone, where gh can also tidy the local branch.
pub fn merge(repo: &PrRepo, number: u64, method: MergeMethod) -> Result<()> {
    let n = number.to_string();
    let flag = match method {
        MergeMethod::Squash => "--squash",
        MergeMethod::Merge => "--merge",
        MergeMethod::Rebase => "--rebase",
    };
    let mut args = vec!["pr", "merge", n.as_str(), flag];
    if matches!(repo, PrRepo::Local(_)) {
        args.push("--delete-branch");
    }
    repo.gh(&args)?;
    Ok(())
}

/// Leave a review on PR `number`. An approval may have an empty `body`.
pub fn review(repo: &PrRepo, number: u64, kind: ReviewKind, body: &str) -> Result<()> {
    let n = number.to_string();
    let flag = match kind {
        ReviewKind::Approve => "--approve",
        ReviewKind::RequestChanges => "--request-changes",
        ReviewKind::Comment => "--comment",
    };
    let mut args = vec!["pr", "review", n.as_str(), flag];
    if !body.trim().is_empty() || kind != ReviewKind::Approve {
        args.extend(["--body", body]);
    }
    repo.gh(&args)?;
    Ok(())
}

/// Comment on PR `number`.
pub fn comment(repo: &PrRepo, number: u64, body: &str) -> Result<()> {
    repo.gh(&["pr", "comment", &number.to_string(), "--body", body])?;
    Ok(())
}

/// Close PR `number` without merging.
pub fn close(repo: &PrRepo, number: u64) -> Result<()> {
    repo.gh(&["pr", "close", &number.to_string()])?;
    Ok(())
}

/// Details of PR `number`.
pub fn pr_detail(repo: &Path, number: u64) -> Result<PrDetail> {
    pr_summary(&PrRepo::Local(repo.to_path_buf()), number)
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
    diff(&PrRepo::Local(repo.to_path_buf()), number)
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
    merge(&PrRepo::Local(wt.to_path_buf()), number, MergeMethod::Squash)
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
    fn parses_full_pr_view() {
        let d = parse_detail(
            r#"{"number":9,"title":"T","body":"","url":"https://github.com/a/b/pull/9","state":"OPEN",
               "headRefName":"x","baseRefName":"main","mergeable":"MERGEABLE","statusCheckRollup":[],
               "author":{"login":"rav"},"isDraft":true,"additions":12,"deletions":3,"headRefOid":"abc",
               "changedFiles":2,"reviewDecision":"APPROVED","createdAt":"2026-10-19T00:00:00Z",
               "mergeStateStatus":"CLEAN",
               "reviews":[{"author":{"login":"kim"},"state":"APPROVED","body":"lgtm","submittedAt":"t1"}],
               "comments":[{"author":{"login":"lee"},"body":"nice","createdAt":"t2"}],
               "commits":[{"oid":"abc","messageHeadline":"Fix it","authors":[{"login":"rav","name":"Rav"}],"committedDate":"t0"}],
               "files":[{"path":"src/a.rs","additions":10,"deletions":3},{"path":"README.md","additions":2,"deletions":0}]}"#,
        )
        .unwrap();
        assert_eq!((d.author.as_str(), d.draft, d.head_sha.as_str()), ("rav", true, "abc"));
        assert_eq!(d.status(), "draft");
        assert_eq!((d.reviews[0].author.as_str(), d.reviews[0].state.as_str()), ("kim", "APPROVED"));
        assert_eq!(d.comments[0].body, "nice");
        assert_eq!((d.commits[0].headline.as_str(), d.commits[0].author.as_str()), ("Fix it", "rav"));
        assert_eq!((d.files.len(), d.files[0].additions), (2, 10));
        assert_eq!(d.checks_summary(), "none");
    }

    #[test]
    fn parses_pr_urls() {
        assert_eq!(parse_pr_url("https://github.com/PupppyLoverr/Dove/pull/9"), Some(("PupppyLoverr/Dove".into(), 9)));
        assert_eq!(parse_pr_url("https://github.com/a/b.c/pull/12/files#diff"), Some(("a/b.c".into(), 12)));
        assert_eq!(parse_pr_url("https://github.com/a/b/issues/3"), None);
        assert_eq!(parse_pr_url("https://github.com/a/b/pull/x"), None);
        let text = "Created https://github.com/a/b/pull/4\nsee (https://github.com/a/b/pull/4/checks) and \
                    [link](https://github.com/c/d/pull/77). Not https://github.com/e/f/issues/1";
        assert_eq!(
            pr_urls(text),
            vec!["https://github.com/a/b/pull/4".to_string(), "https://github.com/c/d/pull/77".to_string()]
        );
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
