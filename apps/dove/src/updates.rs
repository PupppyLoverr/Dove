//! Agent updates: which installed agent CLIs have a newer release, and the
//! command that updates each one. The latest version comes from the agent's
//! own release channel (npm or GitHub releases), as Zeron checks it; agents
//! with no public channel are skipped. Checked at launch and every six hours,
//! with results cached in the store so a restart does not re-ask.

use std::time::Duration;

use dove_core::proto::HarnessKind;

/// An installed agent with a newer release.
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub kind: HarnessKind,
    pub installed: String,
    pub latest: String,
}

enum Source {
    Npm(&'static str),
    Github { repo: &'static str, tag_prefix: &'static str },
}

fn source(kind: HarnessKind) -> Option<Source> {
    Some(match kind {
        HarnessKind::ClaudeCode => Source::Npm("@anthropic-ai/claude-code"),
        HarnessKind::Codex => Source::Github { repo: "openai/codex", tag_prefix: "rust-v" },
        HarnessKind::OpenCode => Source::Npm("opencode-ai"),
        HarnessKind::Pi => Source::Npm("@earendil-works/pi-coding-agent"),
        HarnessKind::Gemini => Source::Npm("@google/gemini-cli"),
        HarnessKind::Copilot => Source::Npm("@github/copilot"),
        HarnessKind::Qwen => Source::Npm("@qwen-code/qwen-code"),
        HarnessKind::Amp => Source::Npm("@sourcegraph/amp"),
        HarnessKind::Cline => Source::Npm("cline"),
        _ => return None,
    })
}

/// How to update `kind`: the agent's own updater where it has one, its
/// package manager otherwise. `program` is the agent's resolved binary.
pub fn command(kind: HarnessKind, program: &std::path::Path) -> (std::path::PathBuf, Vec<String>) {
    let own = |args: &[&str]| (program.to_path_buf(), args.iter().map(|a| a.to_string()).collect());
    let npm = |pkg: &str| {
        let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
        (std::path::PathBuf::from(npm), vec!["install".into(), "-g".into(), format!("{pkg}@latest")])
    };
    match kind {
        HarnessKind::ClaudeCode => own(&["update"]),
        HarnessKind::OpenCode => own(&["upgrade"]),
        HarnessKind::Pi => own(&["update", "--self"]),
        HarnessKind::Codex => npm("@openai/codex"),
        _ => match source(kind) {
            Some(Source::Npm(pkg)) => npm(pkg),
            _ => own(&["update"]),
        },
    }
}

/// The numeric parts of a version ("rust-v0.47.0", "2.1.3 (Claude Code)").
fn parts(v: &str) -> Vec<u64> {
    let v = v.trim().trim_start_matches(|c: char| !c.is_ascii_digit());
    let core = v.split(|c: char| c.is_whitespace() || c == '-' || c == '+').next().unwrap_or("");
    core.split('.').map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>()).filter_map(|p| p.parse().ok()).collect()
}

/// Whether `latest` is newer than `installed`.
pub fn newer(installed: &str, latest: &str) -> bool {
    let (a, b) = (parts(installed), parts(latest));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return y > x;
        }
    }
    false
}

async fn latest(client: &reqwest::Client, kind: HarnessKind) -> Option<String> {
    match source(kind)? {
        Source::Npm(pkg) => {
            let url = format!("https://registry.npmjs.org/{}/latest", pkg.replace('/', "%2F"));
            let v: serde_json::Value = client.get(url).send().await.ok()?.json().await.ok()?;
            v.get("version").and_then(|v| v.as_str()).map(str::to_string)
        }
        Source::Github { repo, tag_prefix } => {
            let url = format!("https://api.github.com/repos/{repo}/releases/latest");
            let v: serde_json::Value = client
                .get(url)
                .header("User-Agent", "dove")
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
                .ok()?
                .json()
                .await
                .ok()?;
            let tag = v.get("tag_name")?.as_str()?;
            Some(tag.strip_prefix(tag_prefix).unwrap_or(tag).to_string())
        }
    }
}

/// Look up the latest release of each `(kind, installed version)` and keep
/// the ones that are newer. Runs on the engine's tokio runtime.
pub async fn check(installed: Vec<(HarnessKind, String)>) -> Vec<Update> {
    let Ok(client) = reqwest::Client::builder().timeout(Duration::from_secs(12)).build() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (kind, version) in installed {
        if let Some(latest) = latest(&client, kind).await
            && newer(&version, &latest)
        {
            out.push(Update { kind, installed: version, latest });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_the_way_people_read_them() {
        assert!(newer("2.1.3", "2.1.10"));
        assert!(newer("0.46.0", "rust-v0.47.0"));
        assert!(newer("1.2.3 (Claude Code)", "1.3.0"));
        assert!(!newer("2.0.0", "2.0.0"));
        assert!(!newer("2.1.0", "2.0.9"));
        assert!(!newer("", "1.0.0"));
    }

    #[test]
    fn npm_agents_update_through_npm() {
        let (program, args) = command(HarnessKind::Gemini, std::path::Path::new("gemini"));
        assert!(program.to_string_lossy().starts_with("npm"));
        assert_eq!(args.last().map(String::as_str), Some("@google/gemini-cli@latest"));
        let (program, args) = command(HarnessKind::ClaudeCode, std::path::Path::new("claude"));
        assert_eq!(program, std::path::PathBuf::from("claude"));
        assert_eq!(args, vec!["update".to_string()]);
    }
}
