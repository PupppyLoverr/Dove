//! Live model catalogs: what each installed agent offers right now.
//!
//! Shared agents ask Zeron's adapters (each has its own discovery and
//! caching). Agents on Dove's ACP driver advertise their models in a
//! throwaway `session/new`. Callers cache the result and fall back to the
//! curated lists in [`crate::models`] until a fetch lands or when it fails.

use std::path::Path;

use anyhow::Result;
use dove_proto::{HarnessKind, Model, ReasoningLevel};

use crate::{acp, external, process};

/// Fetch `kind`'s models through the binary at `program`. Slow (it may start
/// the agent), so run it off the UI thread and bound it with a timeout.
pub async fn fetch(kind: HarnessKind, program: &Path) -> Result<Vec<Model>> {
    let models = if kind == HarnessKind::Antigravity && is_agy_cli(program) {
        agy_models(program).await?
    } else if kind == HarnessKind::Cursor {
        // Zeron's probe needs the Cursor SDK's own login; `cursor-agent acp`
        // works with the CLI's.
        match external::live_models(kind, program).await {
            Ok(models) if !models.is_empty() => models,
            first => acp::discover_models(kind, program).await.map_err(|acp| match first {
                Err(sdk) => anyhow::anyhow!("{sdk:#}; ACP: {acp:#}"),
                Ok(_) => acp,
            })?,
        }
    } else if external::drives(kind) {
        external::live_models(kind, program).await?
    } else if kind == HarnessKind::Amp || acp::launch_args(kind).is_some() {
        acp::discover_models(kind, program).await?
    } else {
        anyhow::bail!("{} has no live model source", kind.label());
    };
    Ok(tidy(models))
}

/// The `agy` CLI itself, rather than Antigravity's separate ACP server.
fn is_agy_cli(program: &Path) -> bool {
    program.file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case("agy"))
}

/// `agy models` prints `id<TAB>Name` per line.
async fn agy_models(program: &Path) -> Result<Vec<Model>> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let mut cmd = process::command(program, &["models".to_string()], &home);
    cmd.stdin(std::process::Stdio::null());
    let out = cmd.output().await?;
    if !out.status.success() {
        anyhow::bail!("agy models exited with {}", out.status);
    }
    Ok(group_effort_variants(&String::from_utf8_lossy(&out.stdout)))
}

/// Fold `gemini-3.8-flash-high` / `Gemini 3.8 Flash (High)` rows into one
/// model whose ladder lists the variants, so the effort slider picks them.
fn group_effort_variants(listing: &str) -> Vec<Model> {
    let mut models: Vec<Model> = Vec::new();
    for line in listing.lines() {
        let Some((id, name)) = line.split_once('\t') else { continue };
        let (id, name) = (id.trim(), name.trim());
        if id.is_empty() {
            continue;
        }
        let variant = id.rsplit_once('-').and_then(|(base, level)| {
            let level = ReasoningLevel::from_id(level)?;
            let suffix = format!(" ({})", capitalize(&level_word(id)));
            let label = name.strip_suffix(&suffix).unwrap_or(name);
            Some((base, label, level))
        });
        match variant {
            Some((base, label, level)) => match models.iter_mut().find(|m| m.id == base) {
                Some(m) => {
                    if !m.reasoning_levels.contains(&level) {
                        m.reasoning_levels.push(level);
                        m.reasoning_levels.sort_by_key(|l| ladder_rank(*l));
                    }
                }
                None => models.push(Model {
                    id: base.into(),
                    label: label.into(),
                    description: None,
                    reasoning_levels: vec![level],
                    options: Vec::new(),
                }),
            },
            None => models.push(Model {
                id: id.into(),
                label: name.into(),
                description: None,
                reasoning_levels: Vec::new(),
                options: Vec::new(),
            }),
        }
    }
    models
}

fn level_word(id: &str) -> String {
    id.rsplit('-').next().unwrap_or_default().to_string()
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

fn ladder_rank(level: ReasoningLevel) -> u8 {
    match level {
        ReasoningLevel::Minimal => 0,
        ReasoningLevel::Low => 1,
        ReasoningLevel::Medium => 2,
        ReasoningLevel::High => 3,
        ReasoningLevel::XHigh => 4,
        ReasoningLevel::Max => 5,
        ReasoningLevel::Ultra => 6,
        ReasoningLevel::Ultracode => 7,
        ReasoningLevel::Ultrathink => 8,
    }
}

/// Drop blank and repeated ids, keeping the first of each.
fn tidy(models: Vec<Model>) -> Vec<Model> {
    let mut seen = std::collections::HashSet::new();
    models
        .into_iter()
        .filter(|m| !m.id.trim().is_empty() && seen.insert(m.id.clone()))
        .map(|mut m| {
            if m.label.trim().is_empty() {
                m.label = m.id.clone();
            }
            m
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agy_effort_variants_fold_into_one_model() {
        let listing = concat!(
            "Fetching available models...\n",
            "gemini-3.8-flash-high\tGemini 3.8 Flash (High)\n",
            "gemini-3.8-flash-medium\tGemini 3.8 Flash (Medium)\n",
            "gemini-3.8-flash-low\tGemini 3.8 Flash (Low)\n",
            "claude-sonnet-4-6\tClaude Sonnet 4.6 (Thinking)\n",
            "gpt-oss-120b-medium\tGPT-OSS 120B (Medium)\n",
        );
        let models = group_effort_variants(listing);
        let flash = &models[0];
        assert_eq!((flash.id.as_str(), flash.label.as_str()), ("gemini-3.8-flash", "Gemini 3.8 Flash"));
        assert_eq!(flash.reasoning_levels, vec![ReasoningLevel::Low, ReasoningLevel::Medium, ReasoningLevel::High]);
        assert_eq!(models[1].id, "claude-sonnet-4-6");
        assert!(models[1].reasoning_levels.is_empty());
        assert_eq!((models[2].id.as_str(), models[2].label.as_str()), ("gpt-oss-120b", "GPT-OSS 120B"));
    }

    #[test]
    fn tidy_drops_blank_and_repeated_ids() {
        let model = |id: &str, label: &str| Model {
            id: id.into(),
            label: label.into(),
            description: None,
            reasoning_levels: Vec::new(),
            options: Vec::new(),
        };
        let out = tidy(vec![model("a", ""), model("", "x"), model("a", "again"), model("b", "B")]);
        let ids: Vec<(&str, &str)> = out.iter().map(|m| (m.id.as_str(), m.label.as_str())).collect();
        assert_eq!(ids, vec![("a", "a"), ("b", "B")]);
    }
}
