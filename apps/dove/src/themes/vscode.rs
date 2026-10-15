//! VS Code colour theme import, after Zeron's `crates/theme/src/vscode.rs`
//! (MIT, Copyright (c) 2026 Wing): read a theme JSON (JSONC: comments and
//! trailing commas allowed; `include` chains and `tokenColors` files
//! resolved next to it), map workbench colours onto a [`Palette`], then
//! harden contrast the way Zeron does so an imported theme stays legible.
//!
//! Unmapped roles keep the Zeron base palette of the same appearance.
//! TextMate `.tmTheme` (plist) token files are not read: such themes import
//! with their workbench colours and the base syntax colours.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};
use serde_json::Value;

use super::{Color, Palette};

/// Theme files larger than this are refused.
const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;
/// `include` chains deeper than this are refused.
const MAX_INCLUDE_DEPTH: usize = 8;
/// Prefix of imported themes' ids, so they never collide with a built-in.
pub const ID_PREFIX: &str = "imported-";

/// A theme file with its includes merged.
#[derive(Debug, Default)]
pub struct Source {
    pub name: Option<String>,
    pub kind: Option<String>,
    pub colors: BTreeMap<String, String>,
    /// TextMate rules in order: (scopes, foreground).
    pub token_colors: Vec<(Vec<String>, String)>,
    /// Semantic selectors and their foregrounds.
    pub semantic: Vec<(String, String)>,
}

/// Read and map a VS Code theme file.
pub fn import_file(path: &Path) -> anyhow::Result<Palette> {
    let path = path.canonicalize().with_context(|| format!("could not open {}", path.display()))?;
    let root = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut stack = Vec::new();
    let source = load(&path, &root, &mut stack)?;
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported theme");
    Ok(convert(&source, stem))
}

/// Remove `//` and `/* */` comments and trailing commas, leaving strings
/// untouched, so the result parses as strict JSON.
pub fn strip_jsonc(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    let mut in_string = false;
    let mut start = 0;
    let flush = |out: &mut String, from: usize, to: usize| out.push_str(&source[from..to]);
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => {
                in_string = true;
                i += 1;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                flush(&mut out, start, i);
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                start = i;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                flush(&mut out, start, i);
                i += 2;
                while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
                out.push(' ');
                start = i;
            }
            b',' => {
                // A comma whose next significant character closes a
                // container is a trailing comma: drop it.
                let mut j = i + 1;
                loop {
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if bytes.get(j) == Some(&b'/') && bytes.get(j + 1) == Some(&b'/') {
                        while j < bytes.len() && bytes[j] != b'\n' {
                            j += 1;
                        }
                    } else if bytes.get(j) == Some(&b'/') && bytes.get(j + 1) == Some(&b'*') {
                        j += 2;
                        while j < bytes.len() && !(bytes[j] == b'*' && bytes.get(j + 1) == Some(&b'/')) {
                            j += 1;
                        }
                        j = (j + 2).min(bytes.len());
                    } else {
                        break;
                    }
                }
                if matches!(bytes.get(j), Some(b'}') | Some(b']')) {
                    flush(&mut out, start, i);
                    start = i + 1;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    flush(&mut out, start, bytes.len());
    out
}

/// Parse JSONC text.
pub fn parse_jsonc(source: &str) -> anyhow::Result<Value> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    Ok(serde_json::from_str(&strip_jsonc(source))?)
}

fn read_bounded(path: &Path) -> anyhow::Result<String> {
    let len = std::fs::metadata(path).with_context(|| format!("could not read {}", path.display()))?.len();
    if len > MAX_SOURCE_BYTES {
        bail!("{} is larger than {} MB", path.display(), MAX_SOURCE_BYTES / 1024 / 1024);
    }
    std::fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))
}

/// Resolve `relative` next to `owner`, refusing anything outside `root`.
fn resolve_inside(owner: &Path, relative: &str, root: &Path) -> anyhow::Result<PathBuf> {
    let candidate = owner.parent().unwrap_or(Path::new(".")).join(relative);
    let resolved = candidate.canonicalize().with_context(|| format!("could not find {}", candidate.display()))?;
    if !resolved.starts_with(root) {
        bail!("{} is outside the theme's folder", resolved.display());
    }
    Ok(resolved)
}

fn load(path: &Path, root: &Path, stack: &mut Vec<PathBuf>) -> anyhow::Result<Source> {
    if stack.iter().any(|p| p == path) {
        bail!("theme include cycle at {}", path.display());
    }
    if stack.len() >= MAX_INCLUDE_DEPTH {
        bail!("theme includes nest deeper than {MAX_INCLUDE_DEPTH}");
    }
    stack.push(path.to_path_buf());
    let value =
        parse_jsonc(&read_bounded(path)?).with_context(|| format!("{} is not a VS Code theme", path.display()))?;
    let object = value.as_object().ok_or_else(|| anyhow!("{} is not a JSON object", path.display()))?;

    let mut source = match object.get("include").and_then(Value::as_str) {
        Some(include) => load(&resolve_inside(path, include, root)?, root, stack)?,
        None => Source::default(),
    };
    if let Some(name) = object.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()) {
        source.name = Some(name.trim().to_string());
    }
    if let Some(kind) = object.get("type").and_then(Value::as_str) {
        source.kind = Some(kind.to_string());
    }
    if let Some(colors) = object.get("colors").and_then(Value::as_object) {
        for (key, value) in colors {
            if let Some(value) = value.as_str() {
                source.colors.insert(key.clone(), value.to_string());
            }
        }
    }
    match object.get("tokenColors") {
        Some(Value::Array(rules)) => source.token_colors.extend(token_rules(rules)),
        Some(Value::String(relative)) if relative.to_ascii_lowercase().ends_with(".json") => {
            let file = resolve_inside(path, relative, root)?;
            let value = parse_jsonc(&read_bounded(&file)?)?;
            let rules = value
                .get("tokenColors")
                .and_then(Value::as_array)
                .or_else(|| value.as_array())
                .cloned()
                .unwrap_or_default();
            source.token_colors.extend(token_rules(&rules));
        }
        _ => {}
    }
    if let Some(semantic) = object.get("semanticTokenColors").and_then(Value::as_object) {
        for (selector, style) in semantic {
            let foreground = match style {
                Value::String(color) => Some(color.clone()),
                Value::Object(o) => o.get("foreground").and_then(Value::as_str).map(str::to_string),
                _ => None,
            };
            if let Some(foreground) = foreground {
                source.semantic.push((selector.clone(), foreground));
            }
        }
    }
    stack.pop();
    Ok(source)
}

fn token_rules(rules: &[Value]) -> Vec<(Vec<String>, String)> {
    rules
        .iter()
        .filter_map(|rule| {
            let foreground = rule.get("settings")?.get("foreground")?.as_str()?.to_string();
            let scopes = match rule.get("scope") {
                Some(Value::String(s)) => s.split(',').map(|s| s.trim().to_string()).collect(),
                Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(|s| s.trim().to_string()).collect(),
                _ => Vec::new(),
            };
            (!scopes.is_empty()).then_some((scopes, foreground))
        })
        .collect()
}

/// Whether the source is a dark theme: its `type`, else its editor
/// background, else dark.
pub fn is_dark(source: &Source) -> bool {
    match source.kind.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("light" | "vs" | "hc-light") => return false,
        Some("dark" | "vs-dark" | "hc-black" | "hc") => return true,
        _ => {}
    }
    source.colors.get("editor.background").and_then(|v| v.parse::<Color>().ok()).is_none_or(|bg| !bg.is_light())
}

/// A stable, file-name-safe id from the theme's name.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() { "theme".into() } else { out.chars().take(64).collect() }
}

fn first(source: &Source, keys: &[&str]) -> Option<Color> {
    keys.iter().find_map(|k| source.colors.get(*k).and_then(|v| v.parse().ok()))
}

/// Map a merged VS Code theme onto a palette (Zeron's `convert` and
/// `harden_variant`). `fallback_name` names themes without a `name`.
pub fn convert(source: &Source, fallback_name: &str) -> Palette {
    let dark = is_dark(source);
    let base_id = if dark { "zeron-dark" } else { "zeron-light" };
    let mut p = (*super::find(base_id).expect("the Zeron base themes are built in")).clone();
    let name = source.name.clone().unwrap_or_else(|| fallback_name.to_string());
    p.id = format!("{ID_PREFIX}{}", slug(&name));
    p.name = name;
    p.dark = dark;

    let set = |target: &mut Color, keys: &[&str]| {
        if let Some(c) = first(source, keys) {
            *target = c;
        }
    };
    set(&mut p.background, &["editor.background"]);
    set(&mut p.shell, &["sideBar.background", "activityBar.background", "panel.background"]);
    set(&mut p.raised, &["list.hoverBackground", "input.background"]);
    set(&mut p.card, &["panel.background", "sideBar.background"]);
    set(&mut p.dialog, &["editorWidget.background", "quickInput.background"]);
    set(&mut p.overlay, &["dropdown.background", "menu.background", "editorWidget.background"]);
    set(&mut p.hover, &["list.hoverBackground", "toolbar.hoverBackground"]);
    set(&mut p.active, &["list.activeSelectionBackground", "list.inactiveSelectionBackground"]);
    set(&mut p.border, &["panel.border", "widget.border", "contrastBorder"]);
    set(&mut p.border_strong, &["focusBorder", "contrastActiveBorder"]);
    set(&mut p.text, &["foreground", "editor.foreground"]);
    set(&mut p.text_muted, &["descriptionForeground", "tab.inactiveForeground"]);
    set(&mut p.text_faint, &["disabledForeground", "input.placeholderForeground"]);
    set(&mut p.solid, &["button.background"]);
    set(&mut p.on_solid, &["button.foreground"]);
    set(&mut p.danger, &["editorError.foreground", "errorForeground", "gitDecoration.deletedResourceForeground"]);
    set(&mut p.warning, &["editorWarning.foreground", "gitDecoration.modifiedResourceForeground"]);
    set(&mut p.success, &["gitDecoration.addedResourceForeground", "testing.iconPassed"]);
    set(&mut p.cursor, &["editorCursor.foreground"]);
    // Dove draws diff colours as text and gutters, so a foreground wins over
    // the (usually translucent) inserted/removed backgrounds.
    if let Some(c) = first(source, &["gitDecoration.addedResourceForeground", "diffEditor.insertedTextBackground"]) {
        p.diff_add = c.opaque();
    }
    if let Some(c) = first(source, &["gitDecoration.deletedResourceForeground", "diffEditor.removedTextBackground"]) {
        p.diff_delete = c.opaque();
    }
    set(&mut p.diff_hunk, &["diffEditor.diagonalFill", "editor.findMatchHighlightBackground"]);
    let accent_keys = [
        "focusBorder",
        "textLink.foreground",
        "button.background",
        "activityBarBadge.background",
        "progressBar.background",
        "editorCursor.foreground",
    ];
    let accent_candidates: Vec<Color> = accent_keys.iter().filter_map(|k| first(source, &[k])).collect();
    if let Some(accent) = accent_candidates.first() {
        p.accent = *accent;
    }
    set(&mut p.terminal_background, &["terminal.background"]);
    set(&mut p.terminal_foreground, &["terminal.foreground"]);
    set(&mut p.terminal_selection, &["terminal.selectionBackground"]);
    const ANSI: [&str; 16] = [
        "terminal.ansiBlack",
        "terminal.ansiRed",
        "terminal.ansiGreen",
        "terminal.ansiYellow",
        "terminal.ansiBlue",
        "terminal.ansiMagenta",
        "terminal.ansiCyan",
        "terminal.ansiWhite",
        "terminal.ansiBrightBlack",
        "terminal.ansiBrightRed",
        "terminal.ansiBrightGreen",
        "terminal.ansiBrightYellow",
        "terminal.ansiBrightBlue",
        "terminal.ansiBrightMagenta",
        "terminal.ansiBrightCyan",
        "terminal.ansiBrightWhite",
    ];
    for (slot, key) in p.ansi.iter_mut().zip(ANSI) {
        set(slot, &[key]);
    }
    map_syntax(source, &mut p);
    harden(source, &mut p, &accent_candidates);
    p
}

fn map_syntax(source: &Source, p: &mut Palette) {
    let mut found: BTreeMap<String, Color> = BTreeMap::new();
    for (scopes, foreground) in &source.token_colors {
        let Ok(color) = foreground.parse::<Color>() else { continue };
        for scope in scopes {
            if let Some(role) = role_for_scope(scope) {
                found.insert(role.into(), color.opaque());
            }
        }
    }
    for (selector, foreground) in &source.semantic {
        let Ok(color) = foreground.parse::<Color>() else { continue };
        if let Some(role) = role_for_semantic(selector) {
            found.insert(role.into(), color.opaque());
        }
    }
    if found.is_empty() {
        return;
    }
    // Roles a theme rarely names borrow their closest relative, so the code
    // reads in one palette rather than mixing the theme with the base.
    let related: [(&str, &[&str]); 17] = [
        ("stringSpecial", &["attribute", "string"]),
        ("escape", &["stringSpecial", "attribute", "string"]),
        ("boolean", &["constant", "number"]),
        ("constant", &["number"]),
        ("typeBuiltin", &["type"]),
        ("constructor", &["type"]),
        ("functionBuiltin", &["function"]),
        ("label", &["function"]),
        ("macro", &["keyword"]),
        ("variableSpecial", &["keyword"]),
        ("operator", &["keyword"]),
        ("parameter", &["variable"]),
        ("property", &["variable"]),
        ("attribute", &["property", "type"]),
        ("tag", &["keyword"]),
        ("punctuation", &["variable"]),
        ("embedded", &["punctuation"]),
    ];
    for (role, from) in related {
        if !found.contains_key(role)
            && let Some(color) = from.iter().find_map(|r| found.get(*r).copied())
        {
            found.insert(role.into(), color);
        }
    }
    for (role, color) in found {
        p.syntax.insert(role, color);
    }
}

/// The syntax role a TextMate scope colours (most specific first).
pub fn role_for_scope(scope: &str) -> Option<&'static str> {
    // A descendant selector (`meta.tag string.quoted`) colours its last part.
    let scope = scope.trim().rsplit(' ').next().unwrap_or_default().to_ascii_lowercase();
    const PAIRS: [(&str, &str); 22] = [
        ("comment", "comment"),
        ("invalid", "invalid"),
        ("keyword.operator", "operator"),
        ("keyword", "keyword"),
        ("storage", "keyword"),
        ("constant.character.escape", "escape"),
        ("string.regexp", "stringSpecial"),
        ("string", "string"),
        ("constant.numeric", "number"),
        ("constant.language", "boolean"),
        ("entity.name.type", "type"),
        ("entity.name.class", "type"),
        ("support.type", "typeBuiltin"),
        ("support.class", "typeBuiltin"),
        ("entity.name.function", "function"),
        ("support.function", "functionBuiltin"),
        ("entity.other.attribute", "attribute"),
        ("entity.name.tag", "tag"),
        ("variable.parameter", "parameter"),
        ("variable.other.property", "property"),
        ("variable", "variable"),
        ("punctuation", "punctuation"),
    ];
    PAIRS.into_iter().find_map(|(needle, role)| scope.starts_with(needle).then_some(role)).or_else(|| {
        // `meta.x constant.other` and the like: fall back to containment.
        if scope.contains("constant") { Some("constant") } else { None }
    })
}

/// The syntax role a semantic token selector colours.
pub fn role_for_semantic(selector: &str) -> Option<&'static str> {
    let token = selector.split(['.', ':']).next()?.to_ascii_lowercase();
    Some(match token.as_str() {
        "comment" => "comment",
        "keyword" | "modifier" => "keyword",
        "string" | "regexp" => "string",
        "number" => "number",
        "type" | "class" | "interface" | "enum" | "struct" | "typeparameter" => "type",
        "function" | "method" => "function",
        "property" | "enummember" => "property",
        "parameter" => "parameter",
        "variable" => "variable",
        "macro" => "macro",
        "decorator" => "attribute",
        "operator" => "operator",
        "label" => "label",
        _ => return None,
    })
}

fn min_contrast(color: Color, backgrounds: &[Color]) -> f32 {
    backgrounds.iter().map(|b| color.contrast(*b)).fold(f32::INFINITY, f32::min)
}

/// Zeron's `ensure_contrast_across`: mix toward `preferred`, then black,
/// then white, until every background reaches `minimum`.
fn contrast_across(color: Color, backgrounds: &[Color], minimum: f32, preferred: Option<Color>) -> Color {
    if min_contrast(color, backgrounds) >= minimum {
        return color;
    }
    let mut best = color;
    let mut best_contrast = min_contrast(color, backgrounds);
    for target in preferred.into_iter().chain([Color::BLACK, Color::WHITE]) {
        for step in 1..=100 {
            let candidate = color.mix(target, step as f32 / 100.0);
            let contrast = min_contrast(candidate, backgrounds);
            if contrast >= minimum {
                return candidate;
            }
            if contrast > best_contrast {
                best = candidate;
                best_contrast = contrast;
            }
        }
    }
    best
}

/// A foreground that fails contrast: first try the theme's other
/// foregrounds, then move the colour itself.
fn harden_foreground(
    source: &Source,
    current: Color,
    keys: &[&str],
    backgrounds: &[Color],
    minimum: f32,
    preferred: Option<Color>,
) -> Color {
    if min_contrast(current, backgrounds) >= minimum {
        return current;
    }
    for key in keys {
        if let Some(candidate) = first(source, &[key])
            && min_contrast(candidate, backgrounds) >= minimum
        {
            return candidate;
        }
    }
    contrast_across(current, backgrounds, minimum, preferred)
}

/// Zeron's `harden_variant`: flatten translucent surfaces onto the
/// background, then lift text, status colours and the accent to their
/// minimum contrast.
fn harden(source: &Source, p: &mut Palette, accent_candidates: &[Color]) {
    let fallback = if p.dark { Color::rgb(6, 6, 6) } else { Color::WHITE };
    p.background = p.background.blend_over(fallback);
    let bg = p.background;
    for surface in [&mut p.shell, &mut p.raised, &mut p.card, &mut p.dialog, &mut p.overlay, &mut p.solid] {
        *surface = surface.blend_over(bg);
    }
    p.terminal_background = p.terminal_background.blend_over(bg);
    let surfaces = [bg, p.shell, p.raised, p.card, p.dialog, p.overlay];
    p.text = harden_foreground(
        source,
        p.text,
        &[
            "foreground",
            "sideBar.foreground",
            "editorWidget.foreground",
            "quickInput.foreground",
            "input.foreground",
            "menu.foreground",
            "editor.foreground",
        ],
        &surfaces,
        4.5,
        None,
    );
    p.text_muted = harden_foreground(
        source,
        p.text_muted,
        &["descriptionForeground", "tab.inactiveForeground", "sideBarSectionHeader.foreground"],
        &surfaces,
        4.5,
        Some(p.text),
    );
    p.text_faint = harden_foreground(
        source,
        p.text_faint,
        &["disabledForeground", "input.placeholderForeground", "descriptionForeground", "foreground"],
        &surfaces,
        3.0,
        Some(p.text_muted),
    );
    p.on_solid = harden_foreground(
        source,
        p.on_solid,
        &["button.foreground", "foreground", "editor.foreground"],
        &[p.solid],
        4.5,
        Some(p.text),
    );
    p.terminal_foreground = harden_foreground(
        source,
        p.terminal_foreground,
        &["terminal.foreground", "editor.foreground", "foreground"],
        &[p.terminal_background],
        4.5,
        Some(p.text),
    );
    for color in [&mut p.danger, &mut p.warning, &mut p.success, &mut p.border_strong] {
        *color = contrast_across(*color, &surfaces[..2], 3.0, None);
    }
    for color in [&mut p.diff_add, &mut p.diff_delete] {
        *color = contrast_across(*color, &surfaces[..1], 3.0, None);
    }
    let backgrounds = &surfaces[..4];
    if min_contrast(p.accent, backgrounds) < 3.0 {
        p.accent = accent_candidates
            .iter()
            .copied()
            .find(|c| c.a == 255 && min_contrast(*c, backgrounds) >= 3.0)
            .unwrap_or_else(|| contrast_across(p.accent.opaque(), backgrounds, 3.0, None));
    }
    p.accent = p.accent.blend_over(bg);
    p.active = if first(source, &["list.activeSelectionBackground", "list.inactiveSelectionBackground"]).is_some() {
        p.active
    } else {
        p.accent.with_alpha(if p.dark { 0.18 } else { 0.10 })
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(json: &str) -> Source {
        let value = parse_jsonc(json).unwrap();
        let object = value.as_object().unwrap();
        let mut s = Source {
            name: object.get("name").and_then(Value::as_str).map(str::to_string),
            kind: object.get("type").and_then(Value::as_str).map(str::to_string),
            ..Source::default()
        };
        for (k, v) in object.get("colors").and_then(Value::as_object).into_iter().flatten() {
            s.colors.insert(k.clone(), v.as_str().unwrap().to_string());
        }
        if let Some(rules) = object.get("tokenColors").and_then(Value::as_array) {
            s.token_colors = token_rules(rules);
        }
        s
    }

    #[test]
    fn jsonc_comments_and_trailing_commas_are_tolerated() {
        let text = r#"{
            // a line comment
            "a": "http://x.y/z", /* block, with a comma, */
            "b": [1, 2, 3,],
            "c": { "d": "/* not a comment */", },
        }"#;
        let value = parse_jsonc(text).unwrap();
        assert_eq!(value["a"], "http://x.y/z");
        assert_eq!(value["b"].as_array().unwrap().len(), 3);
        assert_eq!(value["c"]["d"], "/* not a comment */");
        assert!(parse_jsonc("\u{feff}{\"x\": \"a,]\" ,}").is_ok());
        assert!(parse_jsonc("{ not json").is_err());
    }

    #[test]
    fn workbench_colors_map_to_dove_roles() {
        let s = source(
            r##"{
                "name": "Night Test",
                "type": "dark",
                "colors": {
                    "editor.background": "#101820",
                    "sideBar.background": "#0c1218",
                    "editor.foreground": "#e6e6e6",
                    "focusBorder": "#ff79c6",
                    "editorError.foreground": "#ff5555",
                    "terminal.ansiRed": "#ff0000",
                    "terminal.background": "#0a0f14",
                },
                "tokenColors": [
                    { "scope": "keyword.control", "settings": { "foreground": "#bd93f9" } },
                    { "scope": ["string", "string.quoted"], "settings": { "foreground": "#f1fa8c" } },
                    { "scope": "comment", "settings": { "foreground": "#6272a4", "fontStyle": "italic" } },
                ],
            }"##,
        );
        let p = convert(&s, "fallback");
        assert!(p.dark);
        assert_eq!(p.name, "Night Test");
        assert_eq!(p.id, "imported-night-test");
        assert_eq!(p.background, "#101820".parse().unwrap());
        assert_eq!(p.shell, "#0c1218".parse().unwrap());
        assert_eq!(p.text, "#e6e6e6".parse().unwrap());
        assert_eq!(p.accent, "#ff79c6".parse().unwrap());
        assert_eq!(p.danger, "#ff5555".parse().unwrap());
        assert_eq!(p.ansi[1], "#ff0000".parse().unwrap());
        assert_eq!(p.terminal_background, "#0a0f14".parse().unwrap());
        assert_eq!(p.syntax["keyword"], "#bd93f9".parse().unwrap());
        assert_eq!(p.syntax["string"], "#f1fa8c".parse().unwrap());
        assert_eq!(p.syntax["comment"], "#6272a4".parse().unwrap());
        // Related roles follow the theme rather than the base palette.
        assert_eq!(p.syntax["macro"], "#bd93f9".parse().unwrap());
        // Muted text came from the base but is legible on this background.
        assert!(p.text_muted.contrast(p.background) >= 4.5);
    }

    #[test]
    fn appearance_comes_from_type_then_background() {
        assert!(!is_dark(&source(r##"{ "type": "light", "colors": { "editor.background": "#000000" } }"##)));
        assert!(is_dark(&source(r##"{ "type": "hc-black" }"##)));
        assert!(!is_dark(&source(r##"{ "colors": { "editor.background": "#fdf6e3" } }"##)));
        assert!(is_dark(&source(r##"{ "colors": { "editor.background": "#002b36" } }"##)));
        assert!(is_dark(&source("{}")));
        let light = convert(&source(r##"{ "name": "Paper", "colors": { "editor.background": "#fafafa" } }"##), "x");
        assert!(!light.dark);
        assert_eq!(light.text, super::super::find("zeron-light").unwrap().text);
    }

    #[test]
    fn translucent_surfaces_flatten_and_text_is_hardened() {
        let p = convert(
            &source(
                r##"{
                    "type": "dark",
                    "colors": {
                        "editor.background": "#101820cc",
                        "sideBar.background": "#ffffff10",
                        "foreground": "#202020",
                        "focusBorder": "#111111",
                    }
                }"##,
            ),
            "Murky",
        );
        assert_eq!(p.background.a, 255);
        assert_eq!(p.shell.a, 255);
        assert!(p.text.contrast(p.background) >= 4.5);
        assert!(p.accent.contrast(p.background) >= 3.0);
        assert_eq!(p.name, "Murky");
    }

    #[test]
    fn scopes_and_semantic_selectors_map_to_roles() {
        assert_eq!(role_for_scope("keyword.operator.assignment"), Some("operator"));
        assert_eq!(role_for_scope("keyword.control"), Some("keyword"));
        assert_eq!(role_for_scope("storage.type"), Some("keyword"));
        assert_eq!(role_for_scope("constant.numeric.integer"), Some("number"));
        assert_eq!(role_for_scope("entity.name.function.rust"), Some("function"));
        assert_eq!(role_for_scope("variable.parameter"), Some("parameter"));
        assert_eq!(role_for_scope("source.rust"), None);
        assert_eq!(role_for_scope("meta.tag string.quoted"), Some("string"));
        assert_eq!(role_for_semantic("function.declaration"), Some("function"));
        assert_eq!(role_for_semantic("enumMember"), Some("property"));
        assert_eq!(role_for_semantic("unknownThing"), None);
    }

    #[test]
    fn slugs_are_file_safe() {
        assert_eq!(slug("Rosé Pine (Moon)"), "ros-pine-moon");
        assert_eq!(slug("  !!  "), "theme");
        assert_eq!(slug("One Dark Pro"), "one-dark-pro");
    }

    #[test]
    fn includes_and_token_files_resolve_inside_the_folder() {
        let dir = std::env::temp_dir().join(format!("dove-vscode-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("base.json"),
            r##"{ "colors": { "editor.background": "#202020", "foreground": "#eeeeee", }, }"##,
        )
        .unwrap();
        std::fs::write(
            dir.join("tokens.json"),
            r##"{ "tokenColors": [ { "scope": "comment", "settings": { "foreground": "#777777" } } ] }"##,
        )
        .unwrap();
        std::fs::write(
            dir.join("theme.json"),
            r##"{
                "name": "Child",
                "include": "./base.json",
                "tokenColors": "./tokens.json",
                "colors": { "focusBorder": "#00aaff" },
            }"##,
        )
        .unwrap();
        let p = import_file(&dir.join("theme.json")).unwrap();
        assert_eq!(p.background, "#202020".parse().unwrap());
        assert_eq!(p.accent, "#00aaff".parse().unwrap());
        assert_eq!(p.syntax["comment"], "#777777".parse().unwrap());

        std::fs::write(dir.join("cycle.json"), r##"{ "include": "./cycle.json" }"##).unwrap();
        assert!(import_file(&dir.join("cycle.json")).is_err());
        std::fs::write(dir.join("escape.json"), r##"{ "include": "../outside.json" }"##).unwrap();
        assert!(import_file(&dir.join("escape.json")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
