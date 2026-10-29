//! Keeps a project folder and its copy on a remote host in step, with only
//! `git` and `sh` on the host. Replaces "clone `origin` and hope it's
//! pushed": unpushed commits travel as a git bundle and uncommitted edits
//! (untracked files included) as a binary patch, over the same hop the agent
//! uses (`ssh`, `cua sandbox exec`, `docker exec`).
//!
//! A folder's state is its fingerprint: `HEAD` plus the tree of the whole
//! working directory (written through a throwaway index, so the user's index
//! is never touched; ignored files don't count). After every sync both sides
//! have the same fingerprint, and it is saved locally. That makes each
//! direction safe:
//!
//! - [`push`] (before the agent starts) overwrites the remote copy only when
//!   it still has the saved fingerprint, i.e. nobody changed it since.
//! - [`pull`] (after a turn) overwrites the local folder only when *it* still
//!   has the saved fingerprint, so the agent's remote edits show up in the
//!   local diff without ever clobbering work done here.
//!
//! When both sides changed, nothing is overwritten and the note says so.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::host::Host;

/// What a sync did, for the launcher's stderr and the thread's log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The folder isn't a git repository; nothing to sync.
    NotGit,
    /// Both sides already match.
    InStep,
    /// Copied over (first time, or the target was untouched since last sync).
    Synced { first: bool },
    /// Both sides changed since the last sync; left alone.
    Diverged,
}

impl Outcome {
    pub fn note(&self, host: &Host, dir: &str, pushing: bool) -> Option<String> {
        let at = host.label();
        match self {
            Outcome::NotGit | Outcome::InStep => None,
            Outcome::Synced { first: true } => Some(format!("dove: copied this project to ~/{dir} on {at}")),
            Outcome::Synced { first: false } if pushing => Some(format!("dove: sent local changes to {at}")),
            Outcome::Synced { first: false } => Some(format!("dove: brought changes back from {at}")),
            Outcome::Diverged if pushing => Some(format!(
                "dove: ~/{dir} on {at} has changes this PC doesn't have yet, so local edits were not sent"
            )),
            Outcome::Diverged => {
                Some(format!("dove: this folder changed here and on {at}; remote changes were not brought back"))
            }
        }
    }
}

/// `HEAD` + working-tree tree id. Equal fingerprints mean equal files.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub head: String,
    pub tree: String,
}

impl Fingerprint {
    fn parse(line: &str) -> Option<Self> {
        let mut it = line.split_whitespace();
        let (head, tree) = (it.next()?, it.next()?);
        let hex = |s: &str| s.len() >= 40 && s.bytes().all(|b| b.is_ascii_hexdigit());
        (hex(head) && hex(tree)).then(|| Fingerprint { head: head.into(), tree: tree.into() })
    }
}

/// What the remote side reported.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Remote {
    Missing,
    NotGit,
    At(Fingerprint),
}

fn parse_remote(out: &str) -> Remote {
    let line = out.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("");
    match line {
        "missing" => Remote::Missing,
        "notgit" => Remote::NotGit,
        l => Fingerprint::parse(l).map(Remote::At).unwrap_or(Remote::NotGit),
    }
}

/// Prints `missing`, `notgit`, or `<HEAD> <tree>` for `$1`. The throwaway
/// index starts as a copy of the real one, so `add -A` is cheap.
const STATE: &str = r#"cd "$1" 2>/dev/null || { echo missing; exit 0; }
git rev-parse --verify -q HEAD >/dev/null 2>&1 || { echo notgit; exit 0; }
t=$(mktemp) || exit 1
cp "$(git rev-parse --git-path index)" "$t" 2>/dev/null
GIT_INDEX_FILE=$t git add -A >/dev/null 2>&1
echo "$(git rev-parse HEAD) $(GIT_INDEX_FILE=$t git write-tree)"
rm -f "$t""#;

/// Writes the patch of uncommitted changes (untracked included) to stdout.
const PATCH: &str = r#"cd "$1" || exit 1
t=$(mktemp) || exit 1
cp "$(git rev-parse --git-path index)" "$t" 2>/dev/null
GIT_INDEX_FILE=$t git add -A >/dev/null 2>&1
GIT_INDEX_FILE=$t git diff --cached --binary HEAD
rm -f "$t""#;

/// stdin: a bundle (`$4` = 1) or nothing, then a patch is fetched in a second
/// call. Makes `$1` match commit `$2` on branch `$3`: clones when missing,
/// otherwise fetches and resets. `$5` = origin URL to restore after a clone.
const APPLY_COMMITS: &str = r#"dir=$1 sha=$2 branch=$3 has_bundle=$4 origin=$5
b=""
if [ "$has_bundle" = 1 ]; then b=$(mktemp) && cat > "$b" || exit 1; fi
if [ ! -d "$dir/.git" ] && [ ! -f "$dir/.git" ]; then
  [ -n "$b" ] || { echo "dove: nothing to clone from" >&2; exit 1; }
  mkdir -p "$dir" && git clone -q "$b" "$dir" >&2 || exit 1
  cd "$dir" || exit 1
  if [ -n "$origin" ]; then git remote set-url origin "$origin"; else git remote remove origin; fi
else
  cd "$dir" || exit 1
  if [ -n "$b" ]; then git fetch -q "$b" HEAD >&2 || exit 1; fi
  git reset -q --hard || exit 1
  git clean -qfd || exit 1
fi
if [ "$branch" = HEAD ]; then git -c advice.detachedHead=false checkout -q --detach "$sha" >&2 || exit 1
else git checkout -q -B "$branch" "$sha" >&2 || exit 1; fi
rm -f "$b""#;

/// stdin: a patch. Applies it in `$1` (empty patch: nothing to do).
const APPLY_PATCH: &str = r#"cd "$1" || exit 1
p=$(mktemp) && cat > "$p" || exit 1
if [ -s "$p" ]; then git apply --whitespace=nowarn --binary "$p" || { rm -f "$p"; exit 1; }; fi
rm -f "$p""#;

/// stdout: a bundle of `HEAD` minus what `$2` already has.
const BUNDLE: &str = r#"cd "$1" || exit 1
git bundle create - HEAD --not "$2" 2>/dev/null"#;

/// Make `dir` on `host` match the local folder `local` (see the module docs).
pub fn push(host: &Host, local: &Path, dir: &str) -> Result<Outcome> {
    let Some(here) = local_fingerprint(local)? else { return Ok(Outcome::NotGit) };
    let saved = load(host, dir);
    let remote = parse_remote(&run(host, STATE, &[dir], None)?);
    let first = match &remote {
        Remote::NotGit => return Ok(Outcome::NotGit),
        Remote::At(fp) if *fp == here => {
            save(host, dir, &here);
            return Ok(Outcome::InStep);
        }
        Remote::At(fp) if saved.as_ref() != Some(fp) => return Ok(Outcome::Diverged),
        Remote::At(_) => false,
        Remote::Missing => true,
    };
    // Commits: everything on a first copy, otherwise what the remote lacks.
    let remote_head = match &remote {
        Remote::At(fp) => Some(fp.head.clone()),
        _ => None,
    };
    let bundle = match &remote_head {
        Some(h) if *h == here.head => None,
        Some(h) if is_ancestor(local, h) => Some(git_bytes(local, &["bundle", "create", "-", "HEAD", "--not", h])?),
        _ => Some(git_bytes(local, &["bundle", "create", "-", "HEAD"])?),
    };
    let branch = git_text(local, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|_| "HEAD".into());
    let origin = git_text(local, &["remote", "get-url", "origin"]).unwrap_or_default();
    let has = if bundle.is_some() { "1" } else { "0" };
    run(host, APPLY_COMMITS, &[dir, &here.head, &branch, has, &origin], bundle.as_deref())
        .with_context(|| format!("sending commits to {}", host.label()))?;
    let patch = local_patch(local)?;
    run(host, APPLY_PATCH, &[dir], Some(&patch)).with_context(|| format!("sending edits to {}", host.label()))?;
    match parse_remote(&run(host, STATE, &[dir], None)?) {
        Remote::At(fp) => save(host, dir, &fp),
        other => bail!("{} reported {other:?} after sync", host.label()),
    }
    Ok(Outcome::Synced { first })
}

/// Bring the agent's work on `host` back into `local` when `local` hasn't
/// changed since the last sync (see the module docs).
pub fn pull(host: &Host, local: &Path, dir: &str) -> Result<Outcome> {
    let Some(here) = local_fingerprint(local)? else { return Ok(Outcome::NotGit) };
    let Some(saved) = load(host, dir) else { return Ok(Outcome::Diverged) };
    let remote = match parse_remote(&run(host, STATE, &[dir], None)?) {
        Remote::At(fp) => fp,
        _ => return Ok(Outcome::NotGit),
    };
    if remote == here {
        save(host, dir, &here);
        return Ok(Outcome::InStep);
    }
    if here != saved {
        return Ok(Outcome::Diverged);
    }
    if remote.head != here.head {
        let bundle = run_bytes(host, BUNDLE, &[dir, &here.head], None)?;
        if bundle.is_empty() {
            bail!("{} sent no commits", host.label());
        }
        let file = temp_file(local, &bundle)?;
        let fetched = git_text(local, &["fetch", "-q", &file.to_string_lossy(), "HEAD"]);
        let _ = std::fs::remove_file(&file);
        fetched?;
    }
    let patch = run_bytes(host, PATCH, &[dir], None)?;
    git_text(local, &["reset", "-q", "--hard", &remote.head])?;
    git_text(local, &["clean", "-qfd"])?;
    if !patch.is_empty() {
        let file = temp_file(local, &patch)?;
        let applied = git_text(local, &["apply", "--whitespace=nowarn", "--binary", &file.to_string_lossy()]);
        let _ = std::fs::remove_file(&file);
        applied?;
    }
    let now = local_fingerprint(local)?.context("folder stopped being a git repository")?;
    save(host, dir, &now);
    Ok(Outcome::Synced { first: false })
}

/// The local folder's fingerprint, or `None` outside a git repository (or
/// before its first commit).
pub fn local_fingerprint(local: &Path) -> Result<Option<Fingerprint>> {
    let Ok(head) = git_text(local, &["rev-parse", "--verify", "-q", "HEAD"]) else { return Ok(None) };
    let index = scratch_index(local)?;
    let tree = (|| {
        git_env(local, &["add", "-A"], &index)?;
        git_env(local, &["write-tree"], &index)
    })();
    let _ = std::fs::remove_file(&index);
    Ok(Some(Fingerprint { head, tree: tree?.trim().to_string() }))
}

/// Uncommitted changes, untracked files included, as a binary patch.
fn local_patch(local: &Path) -> Result<Vec<u8>> {
    let index = scratch_index(local)?;
    let out = (|| {
        git_env(local, &["add", "-A"], &index)?;
        let mut cmd = git_cmd(local, &["diff", "--cached", "--binary", "HEAD"]);
        cmd.env("GIT_INDEX_FILE", &index);
        output(cmd, "git diff")
    })();
    let _ = std::fs::remove_file(&index);
    out
}

/// A copy of the repository's index to stage into without touching it.
fn scratch_index(local: &Path) -> Result<PathBuf> {
    let real = git_text(local, &["rev-parse", "--git-path", "index"])?;
    let real = if Path::new(&real).is_absolute() { PathBuf::from(real) } else { local.join(real) };
    let tmp = std::env::temp_dir().join(format!("dove-index-{}-{}", std::process::id(), uuid::Uuid::now_v7().simple()));
    if real.exists() {
        std::fs::copy(&real, &tmp).context("copying the git index")?;
    }
    Ok(tmp)
}

fn temp_file(local: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let _ = local;
    let path = std::env::temp_dir().join(format!("dove-sync-{}", uuid::Uuid::now_v7().simple()));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

fn is_ancestor(local: &Path, commit: &str) -> bool {
    git_cmd(local, &["merge-base", "--is-ancestor", commit, "HEAD"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn git_cmd(local: &Path, args: &[&str]) -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(local).args(args).stdin(Stdio::null());
    no_window(&mut cmd);
    cmd
}

fn git_bytes(local: &Path, args: &[&str]) -> Result<Vec<u8>> {
    output(git_cmd(local, args), &format!("git {}", args.join(" ")))
}

fn git_text(local: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&git_bytes(local, args)?).trim().to_string())
}

fn git_env(local: &Path, args: &[&str], index: &Path) -> Result<String> {
    let mut cmd = git_cmd(local, args);
    cmd.env("GIT_INDEX_FILE", index);
    Ok(String::from_utf8_lossy(&output(cmd, &format!("git {}", args.join(" ")))?).trim().to_string())
}

fn output(mut cmd: std::process::Command, what: &str) -> Result<Vec<u8>> {
    let out = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).output().with_context(|| format!("running {what}"))?;
    if !out.status.success() {
        bail!("{what} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

/// Run `sh -c <script> dove-sync <args…>` on `host`, feeding `stdin`.
fn run_bytes(host: &Host, script: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut a: Vec<String> = vec!["-c".into(), script.into(), "dove-sync".into()];
    a.extend(args.iter().map(|s| s.to_string()));
    let inv = host.wrap("sh", &a, ".", false);
    let mut cmd = std::process::Command::new(&inv.program);
    cmd.args(&inv.args).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    no_window(&mut cmd);
    let mut child = cmd.spawn().with_context(|| format!("reaching {} ({})", host.label(), inv.program.display()))?;
    // Feed stdin from a thread so a large bundle can't deadlock against a
    // full stdout pipe.
    let feeder = match (stdin, child.stdin.take()) {
        (Some(bytes), Some(mut pipe)) => {
            let bytes = bytes.to_vec();
            Some(std::thread::spawn(move || {
                let _ = std::io::Write::write_all(&mut pipe, &bytes);
            }))
        }
        _ => None,
    };
    let out = child.wait_with_output()?;
    if let Some(f) = feeder {
        let _ = f.join();
    }
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

fn run(host: &Host, script: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<String> {
    Ok(String::from_utf8_lossy(&run_bytes(host, script, args, stdin)?).into_owned())
}

fn no_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// `$DOVE_HOME/hosts/sync/<host>__<dir>.json`: the last fingerprint both
/// sides agreed on.
fn state_path(host: &Host, dir: &str) -> PathBuf {
    let safe =
        |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect::<String>();
    crate::hosts::dove_home().join("hosts").join("sync").join(format!("{}__{}.json", host.slug(), safe(dir)))
}

fn load(host: &Host, dir: &str) -> Option<Fingerprint> {
    serde_json::from_slice(&std::fs::read(state_path(host, dir)).ok()?).ok()
}

fn save(host: &Host, dir: &str, fp: &Fingerprint) {
    let path = state_path(host, dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec(fp) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remote_states() {
        assert_eq!(parse_remote("missing\n"), Remote::Missing);
        assert_eq!(parse_remote("notgit"), Remote::NotGit);
        let h = "a".repeat(40);
        let t = "b".repeat(40);
        assert_eq!(
            parse_remote(&format!("motd noise\n{h} {t}\n")),
            Remote::At(Fingerprint { head: h.clone(), tree: t.clone() })
        );
        assert_eq!(parse_remote("garbage"), Remote::NotGit);
    }

    #[test]
    fn scripts_take_values_as_arguments() {
        for s in [STATE, PATCH, APPLY_COMMITS, APPLY_PATCH, BUNDLE] {
            assert!(s.contains("$1"), "every script reads its folder from $1");
        }
    }
}
