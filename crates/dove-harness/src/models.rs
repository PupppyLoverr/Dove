//! Curated model catalogs for the picker.
//!
//! Live discovery can replace these later. The lists mirror the catalogs
//! Zeron keeps for each agent: effort ladders, context-window choices, and
//! speed / fast-mode traits. Adapters clamp a chosen effort to what that
//! CLI actually accepts.

use dove_proto::{HarnessKind, Model, ModelOption, ModelOptionChoice, ReasoningLevel};

const FULL: &[ReasoningLevel] = &[
    ReasoningLevel::Low,
    ReasoningLevel::Medium,
    ReasoningLevel::High,
    ReasoningLevel::XHigh,
    ReasoningLevel::Max,
    ReasoningLevel::Ultracode,
    ReasoningLevel::Ultrathink,
];

const XHIGH: &[ReasoningLevel] = &[
    ReasoningLevel::Low,
    ReasoningLevel::Medium,
    ReasoningLevel::High,
    ReasoningLevel::XHigh,
    ReasoningLevel::Max,
    ReasoningLevel::Ultrathink,
];

const ULTRA: &[ReasoningLevel] = &[
    ReasoningLevel::Low,
    ReasoningLevel::Medium,
    ReasoningLevel::High,
    ReasoningLevel::XHigh,
    ReasoningLevel::Max,
    ReasoningLevel::Ultra,
];

const MAX: &[ReasoningLevel] =
    &[ReasoningLevel::Low, ReasoningLevel::Medium, ReasoningLevel::High, ReasoningLevel::XHigh, ReasoningLevel::Max];

const CODEX_XHIGH: &[ReasoningLevel] =
    &[ReasoningLevel::Low, ReasoningLevel::Medium, ReasoningLevel::High, ReasoningLevel::XHigh];

const GROK: &[ReasoningLevel] = &[ReasoningLevel::Low, ReasoningLevel::Medium, ReasoningLevel::High];

const GEMINI_PRO: &[ReasoningLevel] = &[ReasoningLevel::Low, ReasoningLevel::High];

pub fn catalog(kind: HarnessKind) -> Vec<Model> {
    match kind {
        HarnessKind::ClaudeCode => claude_models(),
        HarnessKind::Codex => codex_models(),
        HarnessKind::Grok => grok_models(),
        HarnessKind::Devin => devin_models(),
        HarnessKind::Hermes => hermes_models(),
        HarnessKind::Antigravity | HarnessKind::Gemini => gemini_models(),
        HarnessKind::OpenCode => prefixed_frontier(),
        HarnessKind::Pi => pi_models(),
        HarnessKind::Cursor => cursor_models(),
        HarnessKind::Qwen => qwen_models(),
        HarnessKind::Amp => amp_models(),
        HarnessKind::Kimi => kimi_models(),
        HarnessKind::Kilo => kilo_models(),
        HarnessKind::Shell => Vec::new(),
        HarnessKind::Copilot | HarnessKind::Goose | HarnessKind::Droid | HarnessKind::Cline => frontier_models(),
    }
}

/// Prefer High, otherwise the top of the ladder.
pub fn default_reasoning(levels: &[ReasoningLevel]) -> Option<ReasoningLevel> {
    if levels.is_empty() {
        return None;
    }
    Some(levels.iter().copied().find(|l| *l == ReasoningLevel::High).unwrap_or(*levels.last().unwrap()))
}

pub fn clamp_reasoning(current: Option<ReasoningLevel>, levels: &[ReasoningLevel]) -> Option<ReasoningLevel> {
    match current {
        Some(level) if levels.contains(&level) => Some(level),
        _ => default_reasoning(levels),
    }
}

/// Keep a previous choice when the new model still offers it.
pub fn align_options(model: &Model, prev: &[(String, String)]) -> Vec<(String, String)> {
    model
        .options
        .iter()
        .map(|opt| {
            let choice = prev
                .iter()
                .find(|(id, _)| id == &opt.id)
                .map(|(_, v)| v.clone())
                .filter(|v| opt.choices.iter().any(|c| &c.id == v))
                .unwrap_or_else(|| opt.default_choice.clone());
            (opt.id.clone(), choice)
        })
        .collect()
}

pub fn option_value<'a>(options: &'a [(String, String)], id: &str) -> Option<&'a str> {
    options.iter().find(|(k, _)| k == id).map(|(_, v)| v.as_str())
}

pub fn option_is_on(options: &[(String, String)], id: &str) -> bool {
    matches!(option_value(options, id), Some("on" | "true"))
}

/// "Context 200K · 1M  ·  Speed Standard · Fast  ·  Effort Low–Max"
pub fn trait_summary(model: &Model) -> String {
    let mut parts = Vec::new();
    if let Some(opt) = model.option("contextWindow") {
        let labels: Vec<&str> = opt.choices.iter().map(|c| c.label.as_str()).collect();
        parts.push(format!("Context {}", labels.join(" · ")));
    }
    if let Some(opt) = model.options.iter().find(|o| matches!(o.id.as_str(), "serviceTier" | "speed" | "fastMode")) {
        let labels: Vec<&str> = opt.choices.iter().map(|c| c.label.as_str()).collect();
        let name = if opt.id == "fastMode" { "Fast mode" } else { "Speed" };
        parts.push(format!("{name} {}", labels.join(" · ")));
    }
    if let (Some(first), Some(last)) = (model.reasoning_levels.first(), model.reasoning_levels.last()) {
        if first == last {
            parts.push(format!("Effort {}", first.label()));
        } else {
            parts.push(format!("Effort {}–{}", first.label(), last.label()));
        }
    }
    parts.join("   ")
}

pub fn claude_model_id(model: &str, options: &[(String, String)]) -> String {
    if option_value(options, "contextWindow").is_some_and(|w| w.eq_ignore_ascii_case("1m")) && !model.ends_with("[1m]")
    {
        format!("{model}[1m]")
    } else {
        model.to_string()
    }
}

pub fn claude_effort(reasoning: Option<ReasoningLevel>, model: Option<&str>) -> Option<&'static str> {
    let base = match reasoning? {
        ReasoningLevel::Ultrathink => return None,
        ReasoningLevel::Minimal | ReasoningLevel::Low => "low",
        ReasoningLevel::Medium => "medium",
        ReasoningLevel::High => "high",
        ReasoningLevel::XHigh | ReasoningLevel::Ultracode => "xhigh",
        ReasoningLevel::Max | ReasoningLevel::Ultra => "max",
    };
    if base == "xhigh" && !model.is_some_and(supports_xhigh) {
        return Some("max");
    }
    Some(base)
}

/// `--settings` JSON for fast mode, thinking, and ultracode. `None` when the
/// launch should leave the user's own settings alone.
pub fn claude_settings(reasoning: Option<ReasoningLevel>, options: &[(String, String)]) -> Option<String> {
    let mut settings = serde_json::Map::new();
    if option_is_on(options, "fastMode") {
        settings.insert("fastMode".into(), serde_json::Value::Bool(true));
    }
    if option_is_on(options, "thinking") {
        settings.insert("alwaysThinkingEnabled".into(), serde_json::Value::Bool(true));
    }
    if reasoning == Some(ReasoningLevel::Ultracode) {
        settings.insert("ultracode".into(), serde_json::Value::Bool(true));
    }
    if settings.is_empty() { None } else { Some(serde_json::Value::Object(settings).to_string()) }
}

const ULTRATHINK_PREFIX: &str = "Ultrathink:\n";

pub fn prepare_claude_prompt(reasoning: Option<ReasoningLevel>, text: &str) -> String {
    if reasoning == Some(ReasoningLevel::Ultrathink) && !is_slash_command(text) {
        format!("{ULTRATHINK_PREFIX}{text}")
    } else {
        text.to_string()
    }
}

pub fn codex_effort(reasoning: Option<ReasoningLevel>) -> Option<&'static str> {
    Some(match reasoning? {
        ReasoningLevel::Minimal | ReasoningLevel::Low => "low",
        ReasoningLevel::Medium => "medium",
        ReasoningLevel::High => "high",
        ReasoningLevel::XHigh | ReasoningLevel::Ultracode | ReasoningLevel::Ultrathink => "xhigh",
        ReasoningLevel::Max => "max",
        ReasoningLevel::Ultra => "ultra",
    })
}

/// Codex omits the standard tier; only an explicit fast tier goes on the wire.
pub fn codex_service_tier(options: &[(String, String)]) -> Option<&str> {
    option_value(options, "serviceTier").filter(|t| *t != "default" && *t != "standard")
}

fn supports_xhigh(model: &str) -> bool {
    [
        "fable-5", "opus-4-7", "opus-4-8", "opus-4-9", "opus-5", "opus-6", "opus-7", "opus-8", "opus-9", "sonnet-5",
        "sonnet-6", "sonnet-7", "sonnet-8", "sonnet-9",
    ]
    .iter()
    .any(|n| model.contains(n))
}

fn is_slash_command(text: &str) -> bool {
    let trimmed = text.trim_start_matches(['\n', '\r']);
    let rest = trimmed.trim_start_matches(' ');
    let spaces = trimmed.len() - rest.len();
    spaces < 4 && rest.starts_with('/') && rest.chars().nth(1).is_some_and(|c| c.is_ascii_alphabetic())
}

fn choice(id: &str, label: &str) -> ModelOptionChoice {
    ModelOptionChoice { id: id.into(), label: label.into() }
}

/// The 200K / 1M context-window trait.
pub(crate) fn context_window_option() -> ModelOption {
    context_window()
}

fn context_window() -> ModelOption {
    ModelOption {
        id: "contextWindow".into(),
        label: "Context".into(),
        choices: vec![choice("200k", "200K"), choice("1m", "1M")],
        default_choice: "200k".into(),
    }
}

fn toggle(id: &str, label: &str) -> ModelOption {
    ModelOption {
        id: id.into(),
        label: label.into(),
        choices: vec![choice("off", "Off"), choice("on", "On")],
        default_choice: "off".into(),
    }
}

fn service_tier() -> ModelOption {
    ModelOption {
        id: "serviceTier".into(),
        label: "Speed".into(),
        choices: vec![choice("default", "Standard"), choice("fast", "Fast")],
        default_choice: "default".into(),
    }
}

fn speed() -> ModelOption {
    ModelOption {
        id: "speed".into(),
        label: "Speed".into(),
        choices: vec![choice("standard", "Standard"), choice("fast", "Fast")],
        default_choice: "standard".into(),
    }
}

fn model(id: &str, label: &str, description: &str, ladder: &[ReasoningLevel], options: Vec<ModelOption>) -> Model {
    Model {
        id: id.into(),
        label: label.into(),
        description: (!description.is_empty()).then(|| description.into()),
        reasoning_levels: ladder.to_vec(),
        options,
    }
}

fn claude_models() -> Vec<Model> {
    vec![
        model("claude-fable-5-1", "Fable 5.1", "Most capable Claude for long agent work", FULL, vec![context_window()]),
        model("claude-fable-5", "Fable 5", "Previous Fable generation", FULL, vec![context_window()]),
        model(
            "claude-opus-5-5",
            "Opus 5.5",
            "Everyday complex coding",
            FULL,
            vec![context_window(), toggle("fastMode", "Fast mode")],
        ),
        model("claude-opus-4-8", "Opus 4.8", "Previous Opus generation", FULL, vec![toggle("fastMode", "Fast mode")]),
        model("claude-opus-4-7", "Opus 4.7", "Older Opus generation", XHIGH, vec![toggle("fastMode", "Fast mode")]),
        model("claude-sonnet-5", "Sonnet 5", "Balanced speed and intelligence", XHIGH, vec![context_window()]),
        model(
            "claude-haiku-4-5",
            "Haiku 4.5",
            "Fastest Claude for small tasks",
            &[],
            vec![toggle("thinking", "Thinking")],
        ),
    ]
}

fn codex_models() -> Vec<Model> {
    vec![
        model("gpt-6-astra", "GPT-6 Astra", "Most capable Codex model", ULTRA, vec![service_tier()]),
        model("gpt-5.6-sol", "GPT-5.6 Sol", "Frontier reasoning", ULTRA, vec![service_tier()]),
        model("gpt-5.6-terra", "GPT-5.6 Terra", "Deep multi-step work", ULTRA, vec![service_tier()]),
        model("gpt-5.6-luna", "GPT-5.6 Luna", "Fast frontier model", MAX, vec![service_tier()]),
        model("gpt-daybreak-blue-latest", "Daybreak Blue", "Defensive cybersecurity work", ULTRA, vec![]),
        model("gpt-5.5", "GPT-5.5", "Previous flagship", CODEX_XHIGH, vec![service_tier()]),
        model("gpt-5.4", "GPT-5.4", "Reliable general coding", CODEX_XHIGH, vec![service_tier()]),
        model("gpt-5.4-mini", "GPT-5.4 Mini", "Small and fast", CODEX_XHIGH, vec![service_tier()]),
        model(
            "gpt-5.3-codex-spark",
            "GPT-5.3 Spark",
            "Ultra-fast lightweight coding",
            CODEX_XHIGH,
            vec![service_tier()],
        ),
    ]
}

fn grok_models() -> Vec<Model> {
    vec![model("grok-4.5", "Grok 4.5", "xAI coding model · 500K context", GROK, vec![])]
}

fn devin_models() -> Vec<Model> {
    vec![
        model("swe-1-7-medium", "SWE-1.7 Medium", "Devin's default coding model", &[], vec![]),
        model(
            "claude-fable-5-1-high",
            "Claude Fable 5.1 High",
            "Anthropic's frontier model through Devin",
            &[],
            vec![],
        ),
        model("adaptive", "Adaptive", "Devin picks the model per request", &[], vec![]),
    ]
}

fn hermes_models() -> Vec<Model> {
    vec![
        model("hermes-4-405b", "Hermes 4 405B", "Nous hybrid-reasoning flagship", &[], vec![]),
        model("hermes-4-70b", "Hermes 4 70B", "Faster Hermes 4, same post-training", &[], vec![]),
    ]
}

fn gemini_models() -> Vec<Model> {
    vec![
        model("gemini-3.7-flash", "Gemini 3.7 Flash", "Fast Gemini · 1M context", GROK, vec![]),
        model("gemini-3.1-pro", "Gemini 3.1 Pro", "Deeper Gemini reasoning · 1M context", GEMINI_PRO, vec![]),
    ]
}

fn qwen_models() -> Vec<Model> {
    vec![
        model("qwen3-coder", "Qwen3 Coder", "Long-context coding model", GROK, vec![context_window()]),
        model(
            "qwen3-coder-flash",
            "Qwen3 Coder Flash",
            "Faster coding model",
            &[ReasoningLevel::Low, ReasoningLevel::Medium],
            vec![speed()],
        ),
        model("qwen3-max", "Qwen3 Max", "Qwen's strongest general model", GROK, vec![]),
    ]
}

fn cursor_models() -> Vec<Model> {
    let mut models = vec![model("composer", "Composer", "Cursor's own coding model", GROK, vec![speed()])];
    models.extend(frontier_models());
    models
}

fn frontier_models() -> Vec<Model> {
    vec![
        model("claude-fable-5-1", "Fable 5.1", "Most capable Claude", FULL, vec![context_window()]),
        model("claude-opus-5-5", "Opus 5.5", "Everyday complex coding", FULL, vec![context_window(), speed()]),
        model("claude-sonnet-5", "Sonnet 5", "Balanced speed and intelligence", XHIGH, vec![context_window()]),
        model("claude-haiku-4-5", "Haiku 4.5", "Fastest for small tasks", &[], vec![speed()]),
        model("gpt-5.6-sol", "GPT-5.6 Sol", "Frontier reasoning", ULTRA, vec![service_tier()]),
        model("gpt-5.6-luna", "GPT-5.6 Luna", "Fast frontier model", MAX, vec![service_tier()]),
        model("gpt-5.4-mini", "GPT-5.4 Mini", "Small and fast", CODEX_XHIGH, vec![service_tier()]),
    ]
}

fn prefixed_frontier() -> Vec<Model> {
    frontier_models()
        .into_iter()
        .map(|mut m| {
            let provider = if m.id.starts_with("gpt-") { "openai" } else { "anthropic" };
            m.description = Some(provider.to_string());
            m.id = format!("{provider}/{}", m.id);
            m
        })
        .collect()
}

fn amp_models() -> Vec<Model> {
    // amp-acp advertises an effort ladder of low / medium / high / ultra.
    // Model ids are applied only when the server lists them.
    let ladder = &[ReasoningLevel::Low, ReasoningLevel::Medium, ReasoningLevel::High, ReasoningLevel::Ultra];
    frontier_models()
        .into_iter()
        .map(|mut model| {
            model.reasoning_levels = ladder.to_vec();
            model
        })
        .collect()
}

fn kimi_models() -> Vec<Model> {
    let ladder = &[ReasoningLevel::Low, ReasoningLevel::Medium, ReasoningLevel::High];
    vec![
        model("kimi-k2.5", "Kimi K2.5", "Moonshot flagship · 256K context", ladder, vec![]),
        model("kimi-k2", "Kimi K2", "Strong coding model · 256K context", ladder, vec![]),
        model(
            "kimi-k2-turbo",
            "Kimi K2 Turbo",
            "Faster Kimi · 256K context",
            &[ReasoningLevel::Low, ReasoningLevel::Medium],
            vec![speed()],
        ),
    ]
}

/// Kilo Gateway rows as `kilo` lists them (`kilo/<provider>/<model>`).
/// Ladders are the per-model effort values Kilo 7.4 advertises; `none` is
/// Minimal.
fn kilo_models() -> Vec<Model> {
    use ReasoningLevel::{High, Low, Max, Medium, Minimal, XHigh};
    let none_to_max: &[ReasoningLevel] = &[Minimal, Low, Medium, High, XHigh, Max];
    let low_high_max: &[ReasoningLevel] = &[Low, High, Max];
    vec![
        model("kilo/anthropic/claude-fable-5.1", "Fable 5.1", "Anthropic", MAX, vec![]),
        model("kilo/anthropic/claude-opus-5.5", "Opus 5.5", "Anthropic", MAX, vec![]),
        model("kilo/anthropic/claude-sonnet-5.5", "Sonnet 5.5", "Anthropic", MAX, vec![]),
        model("kilo/anthropic/claude-haiku-5.5", "Haiku 5.5", "Anthropic", none_to_max, vec![]),
        model("kilo/openai/gpt-6-astra", "GPT-6 Astra", "OpenAI", MAX, vec![]),
        model("kilo/openai/gpt-5.6-sol", "GPT-5.6 Sol", "OpenAI", none_to_max, vec![]),
        model("kilo/openai/gpt-5.6-luna", "GPT-5.6 Luna", "OpenAI", none_to_max, vec![]),
        model("kilo/google/gemini-3.1-pro-preview", "Gemini 3.1 Pro", "Google", GROK, vec![]),
        model("kilo/google/gemini-3.7-flash", "Gemini 3.7 Flash", "Google", GROK, vec![]),
        model("kilo/x-ai/grok-4.7", "Grok 4.7", "xAI", CODEX_XHIGH, vec![]),
        model("kilo/moonshotai/kimi-k3", "Kimi K3", "Moonshot", low_high_max, vec![]),
        model("kilo/z-ai/glm-5.3", "GLM-5.3", "Z.ai", low_high_max, vec![]),
        model("kilo/deepseek/deepseek-v4-pro", "DeepSeek V4 Pro", "DeepSeek", &[Minimal, High, XHigh], vec![]),
        model("kilo/qwen/qwen3-coder-next", "Qwen3 Coder Next", "Qwen", &[], vec![]),
        model("kilo/kilo-auto/free", "Auto (free)", "Kilo picks a free model · no sign-in", &[], vec![]),
    ]
}

fn pi_models() -> Vec<Model> {
    let ladder = &[ReasoningLevel::Low, ReasoningLevel::Medium, ReasoningLevel::High, ReasoningLevel::XHigh];
    vec![
        model("anthropic/claude-opus-5-5", "Opus 5.5", "Anthropic", ladder, vec![context_window()]),
        model("anthropic/claude-sonnet-5", "Sonnet 5", "Anthropic", ladder, vec![context_window()]),
        model("openai/gpt-5.6-sol", "GPT-5.6 Sol", "OpenAI", ULTRA, vec![service_tier()]),
        model("openai/gpt-5.4", "GPT-5.4", "OpenAI", CODEX_XHIGH, vec![service_tier()]),
        model("google/gemini-3.1-pro", "Gemini 3.1 Pro", "Google · 1M context", GEMINI_PRO, vec![]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_clamps_xhigh_and_prefixes_ultrathink() {
        assert_eq!(claude_effort(Some(ReasoningLevel::XHigh), Some("claude-opus-4-5")), Some("max"));
        assert_eq!(claude_effort(Some(ReasoningLevel::XHigh), Some("claude-fable-5")), Some("xhigh"));
        assert_eq!(claude_effort(Some(ReasoningLevel::Ultrathink), Some("claude-fable-5")), None);
        assert_eq!(claude_model_id("claude-sonnet-5", &[("contextWindow".into(), "1m".into())]), "claude-sonnet-5[1m]");
        assert_eq!(prepare_claude_prompt(Some(ReasoningLevel::Ultrathink), "do it"), "Ultrathink:\ndo it");
        assert_eq!(prepare_claude_prompt(Some(ReasoningLevel::Ultrathink), "/compact"), "/compact");
    }

    #[test]
    fn codex_effort_and_tier() {
        assert_eq!(codex_effort(Some(ReasoningLevel::Ultra)), Some("ultra"));
        assert_eq!(codex_effort(Some(ReasoningLevel::Minimal)), Some("low"));
        assert_eq!(codex_service_tier(&[("serviceTier".into(), "default".into())]), None);
        assert_eq!(codex_service_tier(&[("serviceTier".into(), "fast".into())]), Some("fast"));
    }

    #[test]
    fn every_supported_catalog_has_labels() {
        for kind in HarnessKind::ALL {
            for model in catalog(*kind) {
                assert!(!model.id.is_empty(), "{}", kind.label());
                assert!(!model.label.is_empty());
            }
        }
    }
}
