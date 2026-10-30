//! Git operations for tasks.
//!
//! Every call shells out to `git` with a fixed argument list (never a shell
//! string). The API is deliberately narrow so the hot paths (status, diff) can
//! move to in-process `gix` later without touching callers.

pub mod diff;
pub mod github;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};

pub use diff::{DiffLine, FileDiff, FileStatus, Hunk, LineKind};

fn git(cwd: &Path) -> Command {
    let mut c = Command::new("git");
    c.current_dir(cwd).env("GIT_TERMINAL_PROMPT", "0").env("LC_ALL", "C");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

fn run(cwd: &Path, args: &[&str]) -> Result<String> {
    let out: Output = git(cwd).args(args).output().context("git is not installed or not on PATH")?;
    if !out.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The top-level directory of the repository containing `path`, if any.
pub fn repo_root(path: &Path) -> Option<PathBuf> {
    run(path, &["rev-parse", "--show-toplevel"]).ok().map(|s| PathBuf::from(s.trim()))
}

/// Make `dir` a git repository (`git init`).
pub fn init(dir: &Path) -> Result<()> {
    run(dir, &["init"]).map(|_| ())
}

/// Whether `repo` has no commits yet (its HEAD names an unborn branch).
pub fn is_unborn(repo: &Path) -> bool {
    repo_root(repo).is_some() && run(repo, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_err()
}

/// The empty tree, the base to diff against before the first commit.
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

pub fn current_branch(repo: &Path) -> Result<String> {
    Ok(run(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string())
}

/// The commit `reference` (a branch, tag or sha) points at.
pub fn rev(repo: &Path, reference: &str) -> Result<String> {
    Ok(run(repo, &["rev-parse", "--verify", &format!("{reference}^{{commit}}")])?.trim().to_string())
}

pub fn head_sha(repo: &Path) -> Result<String> {
    Ok(run(repo, &["rev-parse", "HEAD"])?.trim().to_string())
}

/// The commit `wt`'s HEAD forked from `other` (a branch name or sha).
pub fn merge_base(wt: &Path, other: &str) -> Result<String> {
    Ok(run(wt, &["merge-base", "HEAD", other])?.trim().to_string())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
    /// Commit the branch started from; diffs are taken against it.
    pub base: String,
}

/// A filesystem- and ref-safe slug of a task title.
pub fn slug(title: &str) -> String {
    let mut s = String::with_capacity(title.len());
    let mut dash = false;
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            s.push(c);
            dash = false;
        } else if !dash && !s.is_empty() {
            s.push('-');
            dash = true;
        }
        if s.len() >= 40 {
            break;
        }
    }
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "task".into() } else { s }
}

/// Create `<repo>/.dove/worktrees/<name>` on a new branch `<prefix><name>`
/// from the current HEAD. Picks a free name if one is taken.
pub fn create_worktree(repo: &Path, name: &str, branch_prefix: &str) -> Result<Worktree> {
    create_worktree_from(repo, name, branch_prefix, None)
}

/// Like [`create_worktree`], starting from `from` (a branch or sha) instead
/// of HEAD.
pub fn create_worktree_from(repo: &Path, name: &str, branch_prefix: &str, from: Option<&str>) -> Result<Worktree> {
    if run(repo, &["rev-parse", "--verify", "HEAD"]).is_err() {
        bail!("the repository has no commits yet; make an initial commit first");
    }
    ensure_excluded(repo, ".dove/")?;
    let base = match from {
        Some(r) => run(repo, &["rev-parse", "--verify", &format!("{r}^{{commit}}")])?.trim().to_string(),
        None => head_sha(repo)?,
    };
    let root = repo.join(".dove").join("worktrees");
    std::fs::create_dir_all(&root)?;
    let mut n = 1;
    let (path, branch) = loop {
        let candidate = if n == 1 { name.to_string() } else { format!("{name}-{n}") };
        let path = root.join(&candidate);
        let branch = format!("{branch_prefix}{candidate}");
        let exists = run(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_ok();
        if !path.exists() && !exists {
            break (path, branch);
        }
        n += 1;
    };
    let p = path.to_string_lossy();
    run(repo, &["worktree", "add", "-b", &branch, &p, &base])?;
    copy_included(repo, &path)?;
    Ok(Worktree { path, branch, base })
}

/// Remove a worktree and, optionally, its branch. Uncommitted work is lost.
pub fn remove_worktree(repo: &Path, wt: &Path, delete_branch: Option<&str>) -> Result<()> {
    let p = wt.to_string_lossy();
    if run(repo, &["worktree", "remove", "--force", &p]).is_err() {
        // A stale entry (directory deleted by hand) still blocks the branch.
        let _ = std::fs::remove_dir_all(wt);
        run(repo, &["worktree", "prune"])?;
    }
    if let Some(b) = delete_branch {
        let _ = run(repo, &["branch", "-D", b]);
    }
    Ok(())
}

/// Add a pattern to `.git/info/exclude` so Dove's own files never show up as
/// changes, without touching the user's tracked `.gitignore`.
fn ensure_excluded(repo: &Path, pattern: &str) -> Result<()> {
    let git_dir = PathBuf::from(run(repo, &["rev-parse", "--git-common-dir"])?.trim());
    let git_dir = if git_dir.is_absolute() { git_dir } else { repo.join(git_dir) };
    let info = git_dir.join("info");
    std::fs::create_dir_all(&info)?;
    let file = info.join("exclude");
    let current = std::fs::read_to_string(&file).unwrap_or_default();
    if !current.lines().any(|l| l.trim() == pattern) {
        let sep = if current.is_empty() || current.ends_with('\n') { "" } else { "\n" };
        std::fs::write(&file, format!("{current}{sep}{pattern}\n"))?;
    }
    Ok(())
}

/// Copy gitignored files listed in `.worktreeinclude` (e.g. `.env`) into a new
/// worktree. Lines are paths relative to the repo root; `#` starts a comment.
fn copy_included(repo: &Path, wt: &Path) -> Result<()> {
    let Ok(list) = std::fs::read_to_string(repo.join(".worktreeinclude")) else { return Ok(()) };
    for line in list.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if line.contains("..") {
            continue;
        }
        let from = repo.join(line);
        let to = wt.join(line);
        if from.is_file() {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&from, &to)?;
            // A copied secret must never show up as a change to commit, even
            // when the repo's own ignore rules don't cover it in the worktree.
            if run(wt, &["check-ignore", "-q", line]).is_err() {
                ensure_excluded(repo, &format!("/{}", line.replace('\\', "/")))?;
            }
        }
    }
    Ok(())
}

/// The files a commit introduced, against its first parent (or the empty
/// tree for a root commit).
pub fn commit_diff(repo: &Path, sha: &str) -> Result<Vec<FileDiff>> {
    let patch = run(
        repo,
        &["show", "--pretty=format:", "--no-color", "--no-ext-diff", "-M", "--src-prefix=a/", "--dst-prefix=b/", sha],
    )?;
    let mut files = diff::parse(&patch);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Recent commits for the history panel: newest first, with parents and
/// the commit graph laid out row by row.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryCommit {
    pub sha: String,
    pub short: String,
    pub subject: String,
    pub author: String,
    pub when: String,
    pub parents: Vec<String>,
    pub refs: Vec<String>,
    /// HEAD points here (the checked-out commit).
    pub head: bool,
    /// Graph column, 0 = the leftmost (usually HEAD's lane).
    pub lane: usize,
    /// This row's slice of the commit graph.
    pub graph: GraphRow,
}

/// One row of the commit graph. Lanes are columns; a row is split at the
/// commit's node: lines in `top` run from the row's top edge down to the
/// middle, lines in `bottom` from the middle to the bottom edge. A lane in
/// `joins` bends from its top into the node (a branch merging here); a lane
/// in `forks` bends from the node down into that lane (a second parent).
/// Every other lane listed passes straight through.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraphRow {
    pub top: Vec<usize>,
    pub bottom: Vec<usize>,
    pub joins: Vec<usize>,
    pub forks: Vec<usize>,
}

pub fn history(repo: &Path, query: &str, limit: usize) -> Result<Vec<HistoryCommit>> {
    let n = limit.clamp(20, 400).to_string();
    let pretty = "%H%x01%h%x01%s%x01%an%x01%ar%x01%P%x01%D";
    let out = if query.trim().is_empty() {
        run(repo, &["log", &format!("-n{n}"), &format!("--pretty=format:{pretty}")])?
    } else {
        run(
            repo,
            &[
                "log",
                &format!("-n{n}"),
                "--regexp-ignore-case",
                &format!("--grep={query}"),
                &format!("--pretty=format:{pretty}"),
            ],
        )?
    };
    let mut commits: Vec<HistoryCommit> = out
        .lines()
        .filter_map(|line| {
            let mut p = line.split('\u{1}');
            let sha = p.next()?.to_string();
            let short = p.next()?.to_string();
            let subject = p.next()?.to_string();
            let author = p.next()?.to_string();
            let when = p.next()?.to_string();
            let parents = p.next().unwrap_or("").split_whitespace().map(str::to_string).collect();
            let decorations = p.next().unwrap_or("");
            let head = decorations.split(", ").any(|s| s == "HEAD" || s.starts_with("HEAD -> "));
            let refs = decorations
                .split(", ")
                .filter(|s| !s.is_empty() && *s != "HEAD")
                .map(|s| s.trim_start_matches("HEAD -> ").to_string())
                .collect();
            Some(HistoryCommit {
                sha,
                short,
                subject,
                author,
                when,
                parents,
                refs,
                head,
                lane: 0,
                graph: GraphRow::default(),
            })
        })
        .collect();
    assign_lanes(&mut commits);
    Ok(commits)
}

/// Lay the commit graph out row by row. Each column remembers the commit it
/// is waiting for; a commit takes the first column waiting for it (other
/// columns waiting for it merge into it here), hands its column to its first
/// parent and opens (or joins) a column for each further parent. Freed
/// columns are reused, so the graph stays as narrow as the branches that
/// are actually open.
fn assign_lanes(commits: &mut [HistoryCommit]) {
    let mut cols: Vec<Option<String>> = Vec::new();
    for c in commits.iter_mut() {
        let waiting: Vec<usize> =
            cols.iter().enumerate().filter(|(_, w)| w.as_deref() == Some(c.sha.as_str())).map(|(i, _)| i).collect();
        let lane = match waiting.first() {
            Some(&i) => i,
            None => match cols.iter().position(Option::is_none) {
                Some(i) => i,
                None => {
                    cols.push(None);
                    cols.len() - 1
                }
            },
        };
        let top: Vec<usize> = cols.iter().enumerate().filter(|(_, w)| w.is_some()).map(|(i, _)| i).collect();
        let joins: Vec<usize> = waiting.iter().copied().filter(|&i| i != lane).collect();
        for &i in &joins {
            cols[i] = None;
        }
        cols[lane] = c.parents.first().cloned();
        let mut forks = Vec::new();
        for p in c.parents.iter().skip(1) {
            let j = match cols.iter().position(|w| w.as_deref() == Some(p.as_str())) {
                Some(j) => j,
                None => {
                    let j = match cols.iter().position(Option::is_none) {
                        Some(j) => j,
                        None => {
                            cols.push(None);
                            cols.len() - 1
                        }
                    };
                    cols[j] = Some(p.clone());
                    j
                }
            };
            forks.push(j);
        }
        let bottom: Vec<usize> = cols.iter().enumerate().filter(|(_, w)| w.is_some()).map(|(i, _)| i).collect();
        while cols.last().is_some_and(Option::is_none) {
            cols.pop();
        }
        c.lane = lane;
        c.graph = GraphRow { top, bottom, joins, forks };
    }
}

/// Every change in `wt` relative to `base`: committed, staged, unstaged and
/// untracked, as one list sorted by path.
pub fn changes(wt: &Path, base: &str) -> Result<Vec<FileDiff>> {
    let patch = run(wt, &["diff", "--no-color", "--no-ext-diff", "-M", "--src-prefix=a/", "--dst-prefix=b/", base])?;
    let mut files = diff::parse(&patch);
    let untracked = run(wt, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    for path in untracked.split('\0').filter(|p| !p.is_empty()) {
        files.push(diff::untracked_file(wt, path));
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Every tracked and untracked (not ignored) file under `dir`, relative and
/// `/`-separated, sorted. For the files panel and @-mentions.
pub fn list_files(dir: &Path) -> Result<Vec<String>> {
    let out = run(dir, &["ls-files", "-co", "--exclude-standard", "-z"])?;
    let mut files: Vec<String> = out.split('\0').filter(|p| !p.is_empty()).map(str::to_string).collect();
    files.sort();
    files.dedup();
    Ok(files)
}

/// Working-tree status per path: `M` modified, `A` added, `D` deleted, `R`
/// renamed, `?` untracked (from `git status --porcelain`).
pub fn status(dir: &Path) -> Result<Vec<(String, char)>> {
    let out = run(dir, &["status", "--porcelain", "-z", "--untracked-files=all"])?;
    let mut items = Vec::new();
    let mut parts = out.split('\0').filter(|p| p.len() > 3);
    while let Some(entry) = parts.next() {
        let (code, path) = entry.split_at(3);
        let xy: Vec<char> = code.chars().take(2).collect();
        let mark = match (xy.first().copied().unwrap_or(' '), xy.get(1).copied().unwrap_or(' ')) {
            ('?', _) => '?',
            ('R', _) | (_, 'R') => {
                // A rename is followed by its old path; skip it.
                parts.next();
                'R'
            }
            ('A', _) => 'A',
            ('D', _) | (_, 'D') => 'D',
            _ => 'M',
        };
        items.push((path.to_string(), mark));
    }
    Ok(items)
}

/// (files, additions, deletions) of `wt` against `base`, untracked files
/// included as additions.
pub fn shortstat(wt: &Path, base: &str) -> Result<(u32, u32, u32)> {
    let out = run(wt, &["diff", "--numstat", base])?;
    let (mut files, mut adds, mut dels) = (0u32, 0u32, 0u32);
    for line in out.lines() {
        let mut parts = line.split('\t');
        let a = parts.next().and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
        let d = parts.next().and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
        files += 1;
        adds += a;
        dels += d;
    }
    let untracked = run(wt, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    for path in untracked.split('\0').filter(|p| !p.is_empty()) {
        files += 1;
        adds += std::fs::read_to_string(wt.join(path)).map(|s| s.lines().count() as u32).unwrap_or(0);
    }
    Ok((files, adds, dels))
}

/// Local branches, most recently committed first.
pub fn branches(repo: &Path) -> Result<Vec<String>> {
    let out = run(repo, &["for-each-ref", "--sort=-committerdate", "--format=%(refname:short)", "refs/heads"])?;
    Ok(out.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
}

/// Switch the checkout to `branch`. Fails (and changes nothing) when local
/// edits would be overwritten.
pub fn checkout(repo: &Path, branch: &str) -> Result<()> {
    run(repo, &["checkout", "-q", branch])?;
    Ok(())
}

/// Merge `branch` into the branch checked out at `repo` with a merge commit
/// (`--no-ff`, so the merge is one commit to revert). A conflict or a dirty
/// checkout fails and is aborted, leaving `repo` as it was; the error carries
/// git's own explanation.
pub fn merge(repo: &Path, branch: &str, message: &str) -> Result<()> {
    let out: Output = git(repo)
        .args(["merge", "--no-ff", "--no-edit", "-m", message, branch])
        .output()
        .context("git is not installed or not on PATH")?;
    if out.status.success() {
        return Ok(());
    }
    // A conflicted merge leaves MERGE_HEAD; put everything back.
    let _ = run(repo, &["merge", "--abort"]);
    let mut why = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let conflicts: Vec<&str> = stdout.lines().filter(|l| l.starts_with("CONFLICT")).collect();
    if !conflicts.is_empty() {
        why = format!("{}{}", conflicts.join("\n"), if why.is_empty() { String::new() } else { format!("\n{why}") });
    }
    if why.is_empty() {
        why = stdout.trim().to_string();
    }
    bail!("git merge {branch}: {why}")
}

/// The default branch of `origin` (`main`, `master`, ...), if known.
pub fn default_branch(repo: &Path) -> Option<String> {
    run(repo, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .ok()
        .map(|s| s.trim().trim_start_matches("origin/").to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            ["main", "master"]
                .iter()
                .find(|b| run(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{b}")]).is_ok())
                .map(|b| b.to_string())
        })
}

/// Clone `url` into a new folder under `parent`; returns the folder.
pub fn clone(url: &str, parent: &Path) -> Result<PathBuf> {
    let name =
        url.trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or("repo").trim_end_matches(".git").to_string();
    if name.is_empty() || name.contains("..") {
        bail!("cannot tell the repository name from {url}");
    }
    std::fs::create_dir_all(parent)?;
    let mut dest = parent.join(&name);
    let mut n = 2;
    while dest.exists() {
        dest = parent.join(format!("{name}-{n}"));
        n += 1;
    }
    let d = dest.to_string_lossy();
    run(parent, &["clone", "--quiet", url, &d])?;
    Ok(dest)
}

/// Stage one path (`git add -A -- path`).
pub fn stage(wt: &Path, path: &str) -> Result<()> {
    run(wt, &["add", "-A", "--", path])?;
    Ok(())
}

/// Unstage one path, keeping its working-tree changes.
pub fn unstage(wt: &Path, path: &str) -> Result<()> {
    if run(wt, &["reset", "-q", "HEAD", "--", path]).is_err() {
        // No HEAD yet: drop it from the index.
        run(wt, &["rm", "-q", "--cached", "--", path])?;
    }
    Ok(())
}

/// Paths with staged changes.
pub fn staged(wt: &Path) -> Result<Vec<String>> {
    let out = run(wt, &["diff", "--cached", "--name-only", "-z"])?;
    Ok(out.split('\0').filter(|p| !p.is_empty()).map(str::to_string).collect())
}

/// Commit what is staged. Returns false when nothing is staged.
pub fn commit_staged(wt: &Path, message: &str) -> Result<bool> {
    if run(wt, &["diff", "--cached", "--quiet"]).is_ok() {
        return Ok(false);
    }
    run(wt, &["commit", "-q", "-m", message])?;
    Ok(true)
}

/// Push the current branch to `origin`, setting its upstream.
pub fn push(wt: &Path) -> Result<()> {
    let branch = current_branch(wt)?;
    run(wt, &["push", "-u", "origin", &branch])?;
    Ok(())
}

/// Commits on HEAD not yet on its upstream (None without an upstream).
pub fn unpushed(wt: &Path) -> Option<usize> {
    run(wt, &["rev-list", "--count", "@{u}..HEAD"]).ok().and_then(|s| s.trim().parse().ok())
}

/// Whether `origin` exists.
pub fn has_remote(wt: &Path) -> bool {
    run(wt, &["remote", "get-url", "origin"]).is_ok()
}

/// One-line log of `wt` since `base`, newest first.
pub fn log_since(wt: &Path, base: &str, max: usize) -> Result<Vec<String>> {
    let n = format!("-{max}");
    let range = format!("{base}..HEAD");
    let out = run(wt, &["log", &n, "--format=%h %s", &range])?;
    Ok(out.lines().map(str::to_string).collect())
}

/// A shadow ref for one thread: `refs/dove/checkpoints/<label>`. The commit is
/// the dirty worktree when there is one (`git stash create`, which does not
/// move HEAD or the index), otherwise HEAD. Rewind restores tracked files
/// from that commit and leaves the branch where it is.
pub fn checkpoint(wt: &Path, label: &str) -> Result<Option<String>> {
    let root = repo_root(wt).unwrap_or_else(|| wt.to_path_buf());
    let sha = match run(&root, &["stash", "create"]) {
        Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => {
            let head = run(&root, &["rev-parse", "HEAD"])?;
            let head = head.trim();
            if head.is_empty() {
                return Ok(None);
            }
            head.to_string()
        }
    };
    let r = format!("refs/dove/checkpoints/{}", checkpoint_label(label));
    run(&root, &["update-ref", &r, &sha])?;
    Ok(Some(sha))
}

/// Put tracked files back to checkpoint `sha`. Does not move HEAD, and does
/// not delete untracked files the agent created.
pub fn rewind_to(wt: &Path, sha: &str) -> Result<()> {
    let root = repo_root(wt).unwrap_or_else(|| wt.to_path_buf());
    if !sha.bytes().all(|b| b.is_ascii_hexdigit()) || sha.len() < 7 {
        bail!("not a commit");
    }
    run(&root, &["restore", "--source", sha, "--worktree", "--staged", "."])?;
    Ok(())
}

fn checkpoint_label(label: &str) -> String {
    let mut s: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .take(80)
        .collect();
    if s.is_empty() {
        s.push_str("turn");
    }
    s
}

/// Stage everything and commit. Returns false when there was nothing to commit.
pub fn commit_all(wt: &Path, message: &str) -> Result<bool> {
    run(wt, &["add", "-A"])?;
    if run(wt, &["diff", "--cached", "--quiet"]).is_ok() {
        return Ok(false);
    }
    run(wt, &["commit", "-q", "-m", message])?;
    Ok(true)
}

/// Reverse every change to one file, restoring it to `base` (or deleting it
/// if it did not exist there).
pub fn discard_file(wt: &Path, base: &str, path: &str) -> Result<()> {
    if run(wt, &["cat-file", "-e", &format!("{base}:{path}")]).is_ok() {
        run(wt, &["checkout", base, "--", path])?;
    } else {
        let _ = run(wt, &["rm", "-q", "--cached", "--", path]);
        let _ = std::fs::remove_file(wt.join(path));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(sha: &str, parents: &[&str]) -> HistoryCommit {
        HistoryCommit {
            sha: sha.into(),
            short: sha.into(),
            subject: String::new(),
            author: String::new(),
            when: String::new(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            refs: Vec::new(),
            head: false,
            lane: 0,
            graph: GraphRow::default(),
        }
    }

    #[test]
    fn a_straight_history_is_one_lane() {
        let mut c = vec![commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])];
        assign_lanes(&mut c);
        assert!(c.iter().all(|c| c.lane == 0));
        assert_eq!(c[0].graph.top, Vec::<usize>::new());
        assert_eq!(c[0].graph.bottom, vec![0]);
        assert_eq!(c[1].graph.top, vec![0]);
        assert_eq!(c[2].graph.bottom, Vec::<usize>::new());
    }

    #[test]
    fn a_merge_forks_and_rejoins_and_the_lane_is_reused() {
        // m merges f (side branch) into b; both come from a; then a newer
        // side branch g also off a must reuse the freed lane 1.
        let mut c = vec![
            commit("m", &["b", "f"]),
            commit("f", &["a"]),
            commit("b", &["a"]),
            commit("a", &["z"]),
            commit("g", &["z"]),
            commit("z", &[]),
        ];
        assign_lanes(&mut c);
        assert_eq!(c[0].lane, 0);
        assert_eq!(c[0].graph.forks, vec![1]);
        assert_eq!(c[0].graph.bottom, vec![0, 1]);
        assert_eq!(c[1].lane, 1);
        assert_eq!(c[2].lane, 0);
        // `a` is waited for by both lanes: lane 1 joins into lane 0.
        assert_eq!(c[3].lane, 0);
        assert_eq!(c[3].graph.joins, vec![1]);
        assert_eq!(c[3].graph.bottom, vec![0]);
        // `g` opens a lane again, reusing column 1 rather than a new one.
        assert_eq!(c[4].lane, 1);
        assert!(c.iter().all(|c| c.lane <= 1));
    }

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        run(p, &["init", "-q", "-b", "main"]).unwrap();
        run(p, &["config", "user.email", "t@example.com"]).unwrap();
        run(p, &["config", "user.name", "T"]).unwrap();
        std::fs::write(p.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        run(p, &["add", "."]).unwrap();
        run(p, &["commit", "-q", "-m", "init"]).unwrap();
        d
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Fix the Login bug!"), "fix-the-login-bug");
        assert_eq!(slug("  ***  "), "task");
        assert!(slug(&"x".repeat(100)).len() <= 40);
    }

    #[test]
    fn worktree_lifecycle_and_changes() {
        let d = repo();
        let r = d.path();
        std::fs::write(r.join(".env"), "SECRET=1").unwrap();
        std::fs::write(r.join(".gitignore"), ".env\n").unwrap();
        std::fs::write(r.join(".worktreeinclude"), "# secrets\n.env\n").unwrap();

        let wt = create_worktree(r, "fix-bug", "dove/").unwrap();
        assert_eq!(wt.branch, "dove/fix-bug");
        assert!(wt.path.join(".env").exists(), ".worktreeinclude copied");
        let second = create_worktree(r, "fix-bug", "dove/").unwrap();
        assert_eq!(second.branch, "dove/fix-bug-2");

        std::fs::write(wt.path.join("a.txt"), "one\nTWO\nthree\n").unwrap();
        std::fs::write(wt.path.join("new.rs"), "fn main() {}\n").unwrap();
        let ch = changes(&wt.path, &wt.base).unwrap();
        let paths: Vec<_> = ch.iter().map(|f| f.path.as_str()).collect();
        // The ignore rules are uncommitted, so only the explicit exclusion
        // keeps the copied .env out of the change list.
        assert_eq!(paths, ["a.txt", "new.rs"]);
        let a = ch.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!((a.additions, a.deletions), (1, 1));
        assert_eq!(ch.iter().find(|f| f.path == "new.rs").unwrap().status, FileStatus::Added);

        discard_file(&wt.path, &wt.base, "a.txt").unwrap();
        discard_file(&wt.path, &wt.base, "new.rs").unwrap();
        assert!(changes(&wt.path, &wt.base).unwrap().iter().all(|f| f.path != "a.txt" && f.path != "new.rs"));

        assert!(!commit_all(&wt.path, "nothing yet").unwrap(), "discarding left a clean tree");
        std::fs::write(wt.path.join("b.txt"), "b\n").unwrap();
        assert!(commit_all(&wt.path, "work").unwrap());
        assert!(!commit_all(&wt.path, "nothing").unwrap());

        let files = list_files(&wt.path).unwrap();
        assert!(files.contains(&"a.txt".to_string()) && !files.contains(&".env".to_string()));

        // Stage / unstage / commit only what is staged.
        std::fs::write(wt.path.join("c.txt"), "c\n").unwrap();
        std::fs::write(wt.path.join("d.txt"), "d\n").unwrap();
        stage(&wt.path, "c.txt").unwrap();
        assert_eq!(staged(&wt.path).unwrap(), ["c.txt"]);
        stage(&wt.path, "d.txt").unwrap();
        unstage(&wt.path, "d.txt").unwrap();
        assert!(commit_staged(&wt.path, "c only").unwrap());
        assert!(!commit_staged(&wt.path, "none").unwrap());
        assert!(changes(&wt.path, "HEAD").unwrap().iter().any(|f| f.path == "d.txt"));
        assert_eq!(log_since(&wt.path, &wt.base, 10).unwrap().len(), 2);

        assert!(branches(r).unwrap().contains(&"main".to_string()));
        assert_eq!(default_branch(r).as_deref(), Some("main"));
        let from = create_worktree_from(r, "from-branch", "dove/", Some(&wt.branch)).unwrap();
        assert!(from.path.join("c.txt").exists(), "started from the other branch");
        assert!(!has_remote(r));

        remove_worktree(r, &wt.path, Some(&wt.branch)).unwrap();
        assert!(!wt.path.exists());
        let exclude = std::fs::read_to_string(r.join(".git/info/exclude")).unwrap();
        assert!(exclude.contains(".dove/"));
    }

    #[test]
    fn init_then_diff_an_unborn_branch() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        std::fs::write(
            p.join("notes.md"),
            "hello
",
        )
        .unwrap();
        if repo_root(p).is_some() {
            // The temp dir sits inside another repository; nothing to test.
            return;
        }
        init(p).unwrap();
        assert!(is_unborn(p));
        assert!(changes(p, "HEAD").is_err(), "HEAD does not resolve before the first commit");
        let files = changes(p, EMPTY_TREE).unwrap();
        assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["notes.md"]);
    }
}
