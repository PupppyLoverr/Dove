//! The authored token set, checked before a theme is worn: 42 tokens in nine
//! groups (`ui/ds/themes/keplorayx-{dark,light}.json`), and every derived
//! token a theme may not set. A theme that names one is **rejected**, not
//! quietly ignored — which is what stops a theme disappearing a selected
//! row, and what makes the small authored surface a guarantee rather than a
//! convention.
//!
//! ```text
//! authored::check(text, scheme)   stages 1–5 over one theme file, as a Report
//! authored::prepare(ui)           both schemes' files, checked, for the script
//! authored::smoke(...)            stage 6: the layout pass (src/layout_smoke)
//! ```
//!
//! The derivation itself lives in `ui/ds/authored.luau`, where the tokens are
//! written in terms of each other, and this file owns what may be *written*:
//! the registry, the clamps, the contrast lifts and the rejection messages.
//! `ui/ds/authored.luau` reads what this decides (`themes.authored`), so the
//! two agree by construction: the script cannot derive from a file this
//! refused, and it derives from the values this one checked.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value};

/// What a token's value is, and therefore what a theme may write for it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Colour,
    Length,
    Number,
    Font,
    Duration,
    Easing,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Colour => "a colour",
            Kind::Length => "a length",
            Kind::Number => "a number",
            Kind::Font => "a font's name",
            Kind::Duration => "a duration",
            Kind::Easing => "a curve",
        }
    }
}

pub struct Spec {
    pub name: &'static str,
    pub group: &'static str,
    pub kind: Kind,
    /// The least and most a theme may write (§KEPL-11 3.1). Outside it,
    /// the value is brought to the bound and said, not rejected: a person
    /// asking for a 20px body is asking for a bigger one, not a broken one.
    pub least: Option<f32>,
    pub most: Option<f32>,
}

const fn spec(name: &'static str, group: &'static str, kind: Kind) -> Spec {
    Spec { name, group, kind, least: None, most: None }
}

const fn ranged(name: &'static str, group: &'static str, kind: Kind, least: f32, most: f32) -> Spec {
    Spec { name, group, kind, least: Some(least), most: Some(most) }
}

/// The 42, in nine groups: surfaces, inks, lines, accent, tones, attention,
/// the agents' hues, type, and the metrics.
pub const SPEC: &[Spec] = &[
    spec("surface.window", "surface", Kind::Colour),
    spec("surface.base", "surface", Kind::Colour),
    spec("surface.raised", "surface", Kind::Colour),
    spec("surface.overlay", "surface", Kind::Colour),
    spec("ink.primary", "ink", Kind::Colour),
    spec("ink.secondary", "ink", Kind::Colour),
    spec("ink.tertiary", "ink", Kind::Colour),
    spec("line.hairline", "line", Kind::Colour),
    spec("line.strong", "line", Kind::Colour),
    spec("accent", "accent", Kind::Colour),
    spec("accent.ink", "accent", Kind::Colour),
    spec("tone.ok", "tone", Kind::Colour),
    spec("tone.warn", "tone", Kind::Colour),
    spec("tone.danger", "tone", Kind::Colour),
    spec("tone.info", "tone", Kind::Colour),
    spec("attention", "attention", Kind::Colour),
    spec("attention.ink", "attention", Kind::Colour),
    spec("agent.1", "agent", Kind::Colour),
    spec("agent.2", "agent", Kind::Colour),
    spec("agent.3", "agent", Kind::Colour),
    spec("agent.4", "agent", Kind::Colour),
    spec("agent.5", "agent", Kind::Colour),
    spec("agent.6", "agent", Kind::Colour),
    spec("agent.7", "agent", Kind::Colour),
    spec("agent.8", "agent", Kind::Colour),
    spec("font.sans", "font", Kind::Font),
    spec("font.mono", "font", Kind::Font),
    spec("font.display", "font", Kind::Font),
    ranged("font.size", "font", Kind::Length, 11.0, 18.0),
    ranged("font.line", "font", Kind::Length, 13.2, 27.0),
    ranged("font.weight", "font", Kind::Number, 300.0, 500.0),
    ranged("font.strong", "font", Kind::Number, 500.0, 700.0),
    ranged("metric.unit", "metric", Kind::Length, 2.0, 8.0),
    ranged("metric.radius", "metric", Kind::Length, 0.0, 16.0),
    ranged("metric.row", "metric", Kind::Length, 24.0, 48.0),
    ranged("metric.control", "metric", Kind::Length, 20.0, 40.0),
    ranged("metric.gutter", "metric", Kind::Length, 8.0, 32.0),
    ranged("metric.conversation", "metric", Kind::Length, 480.0, 1200.0),
    ranged("metric.page", "metric", Kind::Length, 480.0, 800.0),
    ranged("metric.duration", "metric", Kind::Duration, 0.0, 400.0),
    spec("metric.ease", "metric", Kind::Easing),
    ranged("metric.blur", "metric", Kind::Length, 0.0, 64.0),
];

/// The nine groups, in the order a theme file writes them.
pub const GROUPS: &[&str] = &["surface", "ink", "line", "accent", "tone", "attention", "agent", "font", "metric"];

/// The groups whose every member is derived: a theme naming one of these (or
/// anything under them) is rejected, with where its value comes from.
const DERIVED: &[(&str, &str)] = &[
    ("background", "surface.window"),
    ("glass", "surface.window"),
    ("popover", "surface.overlay"),
    ("surface.sunken", "ink.primary laid over surface.window"),
    ("text", "ink.*"),
    ("ink.disabled", "ink.primary at 0.28"),
    ("border", "ink.primary at 0.10, and line.*"),
    ("accent.hover", "accent toward the scheme's extreme"),
    ("accent.text", "accent.ink"),
    ("accent.wash", "accent at 0.14"),
    ("accent.active", "accent away from the extreme"),
    ("danger", "tone.danger"),
    ("warning", "tone.warn"),
    ("success", "tone.ok"),
    ("info", "tone.info"),
    ("tone.ok.", "tone.ok"),
    ("tone.warn.", "tone.warn"),
    ("tone.danger.", "tone.danger"),
    ("tone.info.", "tone.info"),
    ("attention.", "attention"),
    ("hover", "ink.primary at 0.05"),
    ("pressed", "ink.primary at 0.09"),
    ("selection", "accent"),
    ("focus", "accent at 0.6"),
    ("scrollbar", "ink.primary"),
    ("backdrop", "the window's own shade"),
    ("shadow", "the scheme's shadow family"),
    ("radius.", "metric.radius"),
    ("space.", "metric.unit"),
    ("font.size.", "font.size"),
    ("font.line.", "font.line"),
    ("font.weight.", "font.weight and font.strong"),
    ("control", "metric.control"),
    ("density", "metric.row, field and gutter"),
    ("band", "metric.control"),
    ("measure", "metric.conversation and metric.page"),
    ("motion", "metric.duration and metric.ease"),
    ("material", "metric.blur"),
    ("diff.", "tone.ok and tone.danger"),
    ("syntax", ""),
    ("zeron", ""),
    ("kx", ""),
    ("palette", ""),
    ("menu", ""),
    ("terminal", ""),
];

fn derived_at(name: &str) -> Option<&'static str> {
    for (key, why) in DERIVED {
        if name == *key || name.starts_with(key) {
            return Some(why);
        }
    }
    None
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Issue {
    pub stage: u8,
    pub token: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Adjustment {
    pub token: String,
    pub from: String,
    pub to: String,
    pub least: f32,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Report {
    pub ok: bool,
    pub scheme: String,
    pub tokens: BTreeMap<String, Value>,
    pub errors: Vec<Issue>,
    pub warnings: Vec<Issue>,
    pub adjusted: Vec<Adjustment>,
}

fn fail(stage: u8, token: Option<&str>, message: impl Into<String>) -> Issue {
    Issue { stage, token: token.map(str::to_owned), message: message.into() }
}

// -- colours ------------------------------------------------------------

/// `#rrggbb`, `#rrggbbaa`, `alpha(#…, n)` and `mix(#…, #…, n)`, resolved to a
/// colour. Amounts may be decimals with or without a leading zero.
fn colour(text: &str) -> Option<zeron_theme::Color> {
    let text = text.trim();
    if text.starts_with("alpha(") {
        let inner = text.strip_prefix("alpha(")?.strip_suffix(')')?;
        let (what, amount) = inner.rsplit_once(',')?;
        let c = colour(what)?;
        let was = c.a as f32 / 255.0;
        return Some(c.with_alpha(was * amount.trim().parse::<f32>().ok()?));
    }
    if text.starts_with("mix(") {
        let inner = text.strip_prefix("mix(")?.strip_suffix(')')?;
        let mut parts = inner.rsplitn(2, ',');
        let amount = parts.next()?.trim().parse::<f32>().ok()?;
        let rest = parts.next()?;
        let (a, b) = rest.split_once(',')?;
        return Some(colour(a)?.mix(colour(b)?, amount));
    }
    text.parse::<zeron_theme::Color>().ok()
}

fn hex(c: zeron_theme::Color) -> String {
    if c.a == 255 {
        format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a)
    }
}

// -- reference resolution ------------------------------------------------

/// `value` with every `{ref}` put to the authored token it names, and what
/// does not resolve named back. Cycles are caught by the caller's stack.
fn substitute(value: &str, at: &BTreeMap<String, Value>, stack: &mut Vec<String>, errors: &mut Vec<Issue>, name: &str) -> String {
    if !value.contains('{') {
        return value.to_owned();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let reference = &rest[open + 1..open + close];
        if stack.iter().any(|s| s == reference) {
            errors.push(fail(4, Some(name), format!("\"{name}\" refers to itself ({reference}).")));
            return out;
        }
        let Some(found) = at.get(reference) else {
            errors.push(fail(
                4,
                Some(name),
                format!("\"{name}\" refers to \"{reference}\", which is not one of the 42 authored tokens."),
            ));
            return out;
        };
        let text = match found {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            other => other.to_string(),
        };
        if SPEC.iter().any(|s| s.name == reference && s.kind == Kind::Colour) {
            stack.push(reference.to_owned());
            let resolved = substitute(&text, at, stack, errors, reference);
            stack.pop();
            out.push_str(&resolved);
        } else {
            out.push_str(&text);
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out
}

/// A number written as a number, or `a * 0.66`, `a - 1`, `a + 2`, `a / 2`.
fn number(value: &Value) -> Option<f32> {
    match value {
        Value::Number(n) => n.as_f64().map(|n| n as f32),
        Value::String(text) => {
            let text = text.trim();
            if let Ok(n) = text.parse::<f32>() {
                return Some(n);
            }
            let at = text.find(['*', '+', '/']).or_else(|| text[1..].find('-').map(|i| i + 1))?;
            let (a, rest) = text.split_at(at);
            let op = rest.chars().next()?;
            let b = rest[1..].trim();
            let (a, b) = (a.trim().parse::<f32>().ok()?, b.parse::<f32>().ok()?);
            Some(match op {
                '*' => a * b,
                '+' => a + b,
                '-' => a - b,
                '/' if b != 0.0 => a / b,
                _ => return None,
            })
        }
        _ => None,
    }
}

// -- the stages ----------------------------------------------------------

/// A theme file, checked: stages 1–5 (§KEPL-11 3.3). `defaults` fills what a
/// sparse theme leaves out (the built-in file for its scheme), so what comes
/// back is always the whole 42.
pub fn check(text: &str, scheme: &str, defaults: &BTreeMap<String, Value>) -> Report {
    let mut report = Report { scheme: scheme.to_owned(), ..Report::default() };
    // 1. It parses. (Comments are refused on purpose: the authored files
    // carry `description` fields instead, so a person's editor and this
    // reader agree about what the file says.)
    let root: Value = match serde_json::from_str(text) {
        Ok(root) => root,
        Err(error) => {
            report.errors.push(fail(1, None, format!("the file is not JSON: {error}")));
            return report;
        }
    };
    // 2. Its shape: a name, a scheme, optional extends, its tokens.
    let Some(object) = root.as_object() else {
        report.errors.push(fail(2, None, "a theme is a JSON object"));
        return report;
    };
    for key in object.keys() {
        if !matches!(key.as_str(), "name" | "type" | "extends" | "tokens" | "description" | "author") {
            report.errors.push(fail(
                2,
                Some(key),
                format!("\"{key}\" is not a theme's field: a theme has \"name\", \"type\", \"extends\" and \"tokens\"."),
            ));
        }
    }
    match object.get("type").and_then(Value::as_str) {
        Some("dark") | Some("light") | Some("high-contrast") | Some("high-contrast-light") => {}
        Some(other) => report
            .errors
            .push(fail(2, Some("type"), format!("\"type\" is \"dark\" or \"light\", not \"{other}\"."))),
        None => report.errors.push(fail(2, Some("type"), "a theme says its \"type\": \"dark\" or \"light\".")),
    }
    let Some(tokens) = object.get("tokens").and_then(Value::as_object) else {
        report.errors.push(fail(2, Some("tokens"), "\"tokens\" is an object of the 42 authored tokens."));
        return report;
    };

    // 3. Every key: authored, of its kind, in range.
    let mut at: BTreeMap<String, Value> = BTreeMap::new();
    for (name, value) in tokens {
        // (A group written as a nest of objects is the same token.)
        if let Value::Object(group) = value {
            let mut inner: Vec<(String, Value)> = Vec::new();
            flatten(name, group, &mut inner);
            for (name, value) in inner {
                at.insert(name, value);
            }
            continue;
        }
        at.insert(name.clone(), value.clone());
    }
    for (name, value) in std::mem::take(&mut at) {
        let Some(spec) = SPEC.iter().find(|s| s.name == name) else {
            let why = derived_at(&name);
            report.errors.push(match why {
                Some(why) if why.is_empty() => fail(
                    3,
                    Some(&name),
                    format!("\"{name}\" is not a theme's to set: the window computes it. Remove it."),
                ),
                Some(why) => fail(
                    3,
                    Some(&name),
                    format!("\"{name}\" is derived ({why}) and not a theme's to set. Remove it; the window computes it."),
                ),
                None => fail(
                    3,
                    Some(&name),
                    format!("\"{name}\" is not one of the 42 authored tokens (docs/themes.md, \"The authored set\")."),
                ),
            });
            continue;
        };
        match spec.kind {
            Kind::Colour => match value.as_str() {
                // (A `{ref}` is resolved in stage 4: what matters here is
                // that it is written as a colour expression at all.)
                Some(text) if colour(text).is_some() || (text.contains('{') && text.contains('(')) => {
                    at.insert(name, value);
                }
                _ => report.errors.push(fail(
                    3,
                    Some(&name),
                    format!("\"{name}\" is a colour (\"#rrggbb\", \"#rrggbbaa\", alpha(…), mix(…)), not {value}."),
                )),
            },
            Kind::Font | Kind::Easing => match value.as_str() {
                Some(_) => {
                    at.insert(name, value);
                }
                None => report
                    .errors
                    .push(fail(3, Some(&name), format!("\"{name}\" is {}, not {value}.", spec.kind.name()))),
            },
            Kind::Number | Kind::Length | Kind::Duration => match number(&value) {
                Some(n) => {
                    let mut n = n;
                    if let Some(least) = spec.least
                        && n < least
                    {
                        report.warnings.push(fail(
                            3,
                            Some(&name),
                            format!("\"{name}\" is {n}, under its least {least}: raised to {least}."),
                        ));
                        report.adjusted.push(Adjustment { token: name.clone(), from: n.to_string(), to: least.to_string(), least });
                        n = least;
                    }
                    if let Some(most) = spec.most
                        && n > most
                    {
                        report.warnings.push(fail(
                            3,
                            Some(&name),
                            format!("\"{name}\" is {n}, past its most {most}: brought to {most}."),
                        ));
                        report.adjusted.push(Adjustment { token: name.clone(), from: n.to_string(), to: most.to_string(), least: most });
                        n = most;
                    }
                    at.insert(name, serde_json::json!(n));
                }
                None => report.errors.push(fail(
                    3,
                    Some(&name),
                    format!("\"{name}\" is {}, not {value}.", spec.kind.name()),
                )),
            },
        }
    }
    // What the theme left out is the built-in's, so a sparse theme is whole.
    let mut whole: BTreeMap<String, Value> = BTreeMap::new();
    for spec in SPEC {
        match at.get(spec.name).or_else(|| defaults.get(spec.name)) {
            Some(value) => {
                whole.insert(spec.name.to_owned(), value.clone());
            }
            None => report.errors.push(fail(
                3,
                Some(spec.name),
                format!("\"{}\" has no value, and the built-in theme has none to fall back to.", spec.name),
            )),
        }
    }

    // 4. The references resolve, and none is a cycle.
    let mut errors = Vec::new();
    for spec in SPEC {
        if spec.kind != Kind::Colour {
            continue;
        }
        let Some(value) = whole.get(spec.name).and_then(Value::as_str).map(str::to_owned) else { continue };
        let mut stack = vec![spec.name.to_owned()];
        let resolved = substitute(&value, &whole, &mut stack, &mut errors, spec.name);
        whole.insert(spec.name.to_owned(), Value::String(resolved));
    }
    report.errors.extend(errors);
    if !report.errors.is_empty() {
        return report;
    }

    // 5. The inks read: each against the four surfaces, in the order they are
    // laid (a translucent ink reads as what it shows as). What falls short is
    // lifted toward the scheme's extreme and said; only a colour that cannot
    // be lifted is a rejection.
    let Some(window) = whole.get("surface.window").and_then(Value::as_str).and_then(colour) else {
        report.errors.push(fail(5, Some("surface.window"), "the window's fill is not a colour."));
        return report;
    };
    let mut surfaces: Vec<(&str, zeron_theme::Color)> = vec![("surface.window", window)];
    for name in ["surface.base", "surface.raised", "surface.overlay"] {
        if let Some(c) = whole.get(name).and_then(Value::as_str).and_then(colour) {
            surfaces.push((name, c.blend_over(window)));
        }
    }
    let dark = scheme != "light";
    let mut lift = |name: &str, least: f32, report: &mut Report| {
        let Some(value) = whole.get(name).and_then(Value::as_str).map(str::to_owned) else { return };
        let Some(base) = colour(&value) else { return };
        let worst = surfaces
            .iter()
            .map(|(_, surface)| base.blend_over(*surface).contrast(*surface))
            .fold(f32::INFINITY, f32::min);
        if worst >= least {
            return;
        }
        let extreme = if dark {
            zeron_theme::Color { r: 255, g: 255, b: 255, a: 255 }
        } else {
            zeron_theme::Color { r: 0, g: 0, b: 0, a: 255 }
        };
        let mut chosen = None;
        for step in 1..=100 {
            let candidate = base.mix(extreme, step as f32 / 100.0);
            if candidate.contrast(window) >= least {
                chosen = Some(candidate);
                break;
            }
        }
        match chosen {
            Some(candidate) => {
                let to = hex(candidate);
                report.warnings.push(fail(
                    5,
                    Some(name),
                    format!("\"{name}\" reads {worst:.2}:1 on the window; worn as {to} ({least}:1)."),
                ));
                report.adjusted.push(Adjustment { token: name.to_owned(), from: value, to: to.clone(), least });
                whole.insert(name.to_owned(), Value::String(to));
            }
            None => report.errors.push(fail(
                5,
                Some(name),
                format!("\"{name}\" cannot be read on the window ({worst:.2}:1 at best, {least}:1 wanted): choose a lighter or a darker colour."),
            )),
        }
    };
    lift("ink.primary", 4.5, &mut report);
    lift("ink.secondary", 4.5, &mut report);
    lift("ink.tertiary", 3.0, &mut report);
    lift("accent", 3.0, &mut report);
    lift("tone.ok", 3.0, &mut report);
    lift("tone.warn", 3.0, &mut report);
    lift("tone.danger", 3.0, &mut report);
    lift("tone.info", 3.0, &mut report);
    lift("attention", 3.0, &mut report);
    lift("attention.ink", 4.5, &mut report);

    report.ok = report.errors.is_empty();
    report.tokens = whole;
    report
}

fn flatten(prefix: &str, group: &Map<String, Value>, out: &mut Vec<(String, Value)>) {
    for (key, value) in group {
        let name = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        match value {
            Value::Object(inner) => flatten(&name, inner, out),
            value => out.push((name, value.clone())),
        }
    }
}

/// The built-in file's authored values for one scheme, as the fallback a
/// sparse theme is filled from. None where the file is missing or refused.
pub fn builtin(ui: &Path, scheme: &str) -> Option<BTreeMap<String, Value>> {
    let path = ui.join("ds/themes").join(format!("keplorayx-{scheme}.json"));
    let text = std::fs::read_to_string(&path).ok()?;
    let root: Value = serde_json::from_str(&text).ok()?;
    let mut out = BTreeMap::new();
    let mut found: Vec<(String, Value)> = Vec::new();
    flatten("", root.get("tokens")?.as_object()?, &mut found);
    for (name, value) in found {
        out.insert(name, value);
    }
    Some(out)
}

/// Both schemes' built-in themes, checked, as the script is handed them:
/// `{ dark = Report, light = Report }`. A refused scheme is a refused theme:
/// the script wears the framework's own rather than a theme it could not
/// check.
pub fn prepare(ui: &Path) -> Value {
    let mut out = Map::new();
    for scheme in ["dark", "light"] {
        let path = ui.join("ds/themes").join(format!("keplorayx-{scheme}.json"));
        let report = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let defaults = if scheme == "dark" {
                    builtin(ui, "dark").unwrap_or_default()
                } else {
                    builtin(ui, "light").unwrap_or_default()
                };
                check(&text, scheme, &defaults)
            }
            Err(error) => {
                let mut report = Report { scheme: scheme.to_owned(), ..Report::default() };
                report.errors.push(fail(1, None, format!("{path:?} could not be read: {error}")));
                report
            }
        };
        out.insert(scheme.to_owned(), serde_json::to_value(&report).unwrap_or(Value::Null));
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built_in(scheme: &str) -> (String, BTreeMap<String, Value>) {
        let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
        let text = std::fs::read_to_string(ui.join("ds/themes").join(format!("keplorayx-{scheme}.json"))).unwrap();
        (text, builtin(&ui, scheme).unwrap())
    }

    #[test]
    fn the_shipped_themes_are_42_tokens_in_nine_groups() {
        for scheme in ["dark", "light"] {
            let (text, defaults) = built_in(scheme);
            let report = check(&text, scheme, &defaults);
            assert!(report.errors.is_empty(), "{scheme}: {:?}", report.errors);
            assert_eq!(report.tokens.len(), 42, "{scheme}: {} tokens", report.tokens.len());
            let mut groups: Vec<&str> = report.tokens.keys().filter_map(|n| SPEC.iter().find(|s| s.name == n).map(|s| s.group)).collect();
            groups.sort_unstable();
            groups.dedup();
            assert_eq!(groups.len(), 9, "{scheme}: groups {groups:?}");
        }
    }

    #[test]
    fn a_theme_setting_a_derived_token_is_rejected_and_told_which() {
        let (_, defaults) = built_in("dark");
        let text = r##"{"name":"Harbor","type":"dark","tokens":{"hover":"#ffffff20","accent":"#7f97c9"}}"##;
        let report = check(text, "dark", &defaults);
        assert!(!report.ok);
        let issue = report.errors.iter().find(|e| e.token.as_deref() == Some("hover")).expect("hover named");
        assert_eq!(issue.stage, 3);
        assert!(issue.message.contains("derived"), "{}", issue.message);
        assert!(issue.message.contains("ink.primary at 0.05"), "{}", issue.message);
    }

    #[test]
    fn a_theme_setting_a_value_of_the_wrong_kind_is_rejected() {
        let (_, defaults) = built_in("dark");
        let text = r##"{"name":"Harbor","type":"dark","tokens":{"ink.primary":13}}"##;
        let report = check(text, "dark", &defaults);
        assert!(!report.ok);
        assert!(report.errors.iter().any(|e| e.token.as_deref() == Some("ink.primary")), "{:?}", report.errors);
    }

    #[test]
    fn a_key_nobody_authored_is_rejected() {
        let (_, defaults) = built_in("dark");
        let text = r##"{"name":"Harbor","type":"dark","tokens":{"ink.quaternary":"#ffffffff"}}"##;
        let report = check(text, "dark", &defaults);
        assert!(!report.ok);
        assert!(report.errors[0].message.contains("not one of the 42"), "{:?}", report.errors[0]);
    }

    #[test]
    fn a_row_under_the_hit_target_is_raised_and_said() {
        let (_, defaults) = built_in("dark");
        let text = r##"{"name":"Harbor","type":"dark","tokens":{"metric.row":16}}"##;
        let report = check(text, "dark", &defaults);
        assert!(report.ok, "{:?}", report.errors);
        assert_eq!(report.tokens["metric.row"], serde_json::json!(24.0));
        assert!(report.adjusted.iter().any(|a| a.token == "metric.row"), "{:?}", report.adjusted);
    }

    #[test]
    fn an_ink_that_cannot_be_read_is_lifted_toward_the_extreme() {
        let (_, defaults) = built_in("dark");
        let text = r##"{"name":"Harbor","type":"dark","tokens":{"ink.tertiary":"#1b1b1d"}}"##;
        let report = check(text, "dark", &defaults);
        assert!(report.ok, "{:?}", report.errors);
        let worn = report.tokens["ink.tertiary"].as_str().unwrap();
        assert_ne!(worn, "#1b1b1d", "the ink moved");
        assert!(report.adjusted.iter().any(|a| a.token == "ink.tertiary"), "{:?}", report.adjusted);
    }

    #[test]
    fn a_reference_to_a_derived_token_is_a_rejection_naming_the_reference() {
        let (_, defaults) = built_in("dark");
        let text = r##"{"name":"Harbor","type":"dark","tokens":{"ink.secondary":"alpha({text.primary}, 0.6)"}}"##;
        let report = check(text, "dark", &defaults);
        assert!(!report.ok);
        assert!(report.errors.iter().any(|e| e.message.contains("text.primary")), "{:?}", report.errors);
    }
}
