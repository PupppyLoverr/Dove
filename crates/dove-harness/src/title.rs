//! AI thread titles, after Zeron's `engine/src/titles.rs`.
//!
//! The agents that support it (Claude Code and Codex, through Zeron's
//! restricted title drivers) get a throwaway, tool-less run in an empty
//! scratch folder: no repository, no MCP servers, no session to resume. The
//! run is told to treat the prompt as quoted data and answer with a short
//! title only. Anything that looks like coding (a tool call) or fails is
//! dropped, and the caller keeps its own fallback title.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use dove_proto::HarnessKind;
use futures::StreamExt;
use zeron_harness::{Harness, RunControls, SteerMessage};
use zeron_proto::{AgentEvent as ZEvent, DoneStatus, ReasoningLevel, RunRequest, SandboxLevel, UserInputAnswer};

/// Retries after a failed attempt (Zeron's ladder).
const RETRY_DELAYS_MS: &[u64] = &[250, 1_000];
/// One attempt's budget; a title that takes longer is not worth waiting for.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);

/// Whether `kind` has a restricted title path.
pub fn supports(kind: HarnessKind) -> bool {
    crate::external::harness_id(kind).is_some_and(zeron_harness::supports_titles)
}

/// The cheapest model in `models` (Zeron's heuristic): a small tier by name
/// (haiku, mini, nano, flash, small, lite), else the last listed; `None`
/// when the list is empty, which lets the agent use its default.
pub fn cheapest_model(models: &[dove_proto::Model]) -> Option<String> {
    let small = models.iter().find(|m| {
        let haystack = format!("{} {}", m.id, m.label).to_lowercase();
        ["haiku", "mini", "nano", "flash", "small", "lite"].iter().any(|tier| haystack.contains(tier))
    });
    small.or(models.last()).map(|m| m.id.clone())
}

/// A 3–5 word title for a thread whose first prompt is `prompt`, from the
/// agent at `program`. `None` when the agent has no title path or every
/// attempt failed.
pub async fn generate(kind: HarnessKind, program: &Path, prompt: &str, model: Option<String>) -> Option<String> {
    let id = crate::external::harness_id(kind).filter(|id| zeron_harness::supports_titles(*id))?;
    let harness = crate::external::driver_for(kind, program)?;
    let scratch = Scratch::new()?;
    let title_prompt = format!(
        "{}\n\nSession request (JSON string):\n{}",
        zeron_harness::TITLE_INSTRUCTIONS,
        serde_json::to_string(prompt).ok()?
    );
    for attempt in 0..=RETRY_DELAYS_MS.len() {
        let request = RunRequest {
            mcp: None,
            prompt: title_prompt.clone(),
            harness: Some(id),
            // The last try leaves the model to the agent, in case the picked
            // one is not available on this account.
            model: if attempt == RETRY_DELAYS_MS.len() { None } else { model.clone() },
            reasoning: Some(ReasoningLevel::Minimal),
            model_options: serde_json::Map::new(),
            cwd: scratch.0.display().to_string(),
            sandbox: SandboxLevel::ReadOnly,
            auto_approve: false,
            attachments: Vec::new(),
            resume: None,
            worktree: None,
        };
        match tokio::time::timeout(ATTEMPT_TIMEOUT, collect_text(harness.as_ref(), request)).await {
            Ok(Ok(raw)) => {
                let title = clean_title(&raw);
                if !title.is_empty() {
                    return Some(title);
                }
            }
            Ok(Err(e)) => tracing::info!("{} title attempt {} failed: {e}", kind.label(), attempt + 1),
            Err(_) => tracing::info!("{} title attempt {} timed out", kind.label(), attempt + 1),
        }
        if let Some(delay) = RETRY_DELAYS_MS.get(attempt) {
            tokio::time::sleep(Duration::from_millis(*delay)).await;
        }
    }
    None
}

/// First line, stripped of quote and heading dressing, capped at 60 chars.
pub fn clean_title(raw: &str) -> String {
    let first = raw.trim().lines().next().unwrap_or("");
    let t: String = first
        .trim_start_matches(['"', '\'', '#', '*', '`', ' ', '\t'])
        .trim_end_matches(['"', '\'', '*', '`', '.', ' ', '\t'])
        .chars()
        .take(60)
        .collect();
    t.trim().to_string()
}

/// Drive one title run: no steering, questions answered empty at once (a
/// title run must never wait on input), and a tool call fails the run.
async fn collect_text(harness: &dyn Harness, request: RunRequest) -> anyhow::Result<String> {
    let (steer_tx, steer_rx) = tokio::sync::mpsc::channel::<SteerMessage>(1);
    let interrupt = zeron_harness::CancellationToken::new();
    let _cancel_on_drop = interrupt.clone().drop_guard();
    let controls = RunControls {
        realtime: None,
        execution_lease: None,
        request_input: Box::new(|_| {
            let (tx, rx) = tokio::sync::oneshot::channel::<Vec<UserInputAnswer>>();
            let _ = tx.send(Vec::new());
            rx
        }),
        steering: steer_rx,
        interrupt: interrupt.clone(),
        turn: Default::default(),
    };
    let mut stream = harness.run_title(request, controls).await?;
    let mut text = String::new();
    let mut completed = false;
    while let Some(event) = stream.next().await {
        match event? {
            ZEvent::TextDelta { text: delta } => text.push_str(&delta),
            ZEvent::ToolCall { .. } => anyhow::bail!("the title run tried to use a tool"),
            ZEvent::Error { message } => anyhow::bail!("title run error: {message}"),
            ZEvent::Done { status, error, .. } => {
                if status == DoneStatus::Completed {
                    completed = true;
                    break;
                }
                anyhow::bail!("title run ended {status:?}: {}", error.unwrap_or_default());
            }
            _ => {}
        }
    }
    // The mailbox stays open for the run's whole lifetime.
    drop(steer_tx);
    anyhow::ensure!(completed, "the title stream ended without completing");
    Ok(text)
}

/// An empty folder the title run works in, removed afterwards.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Option<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "dove-title-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).ok()?;
        Some(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, label: &str) -> dove_proto::Model {
        dove_proto::Model {
            id: id.into(),
            label: label.into(),
            description: None,
            reasoning_levels: Vec::new(),
            options: Vec::new(),
        }
    }

    #[test]
    fn cheapest_prefers_a_small_tier_then_the_last() {
        let models = vec![model("opus", "Opus"), model("haiku", "Haiku"), model("sonnet", "Sonnet")];
        assert_eq!(cheapest_model(&models).as_deref(), Some("haiku"));
        let big = vec![model("opus", "Opus"), model("sonnet", "Sonnet")];
        assert_eq!(cheapest_model(&big).as_deref(), Some("sonnet"));
        assert_eq!(cheapest_model(&[]), None);
    }

    #[test]
    fn titles_lose_their_dressing() {
        assert_eq!(clean_title("\"Fix Sidebar Titles\"\n\nextra"), "Fix Sidebar Titles");
        assert_eq!(clean_title("# Refactor Auth Flow."), "Refactor Auth Flow");
        assert_eq!(clean_title("**Bold Title**"), "Bold Title");
        assert_eq!(clean_title("   "), "");
        assert_eq!(clean_title(&"x".repeat(80)).chars().count(), 60);
    }

    #[test]
    fn only_restricted_drivers_title() {
        assert!(supports(HarnessKind::ClaudeCode));
        assert!(supports(HarnessKind::Codex));
        assert!(!supports(HarnessKind::Cursor));
        assert!(!supports(HarnessKind::Shell));
    }
}
