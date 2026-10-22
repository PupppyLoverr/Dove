//! Keyboard shortcuts: defaults, user overrides (Settings › Keyboard) and
//! the table shown in Help.

use dove_core::store::Store;
use gpui::{Action, App, KeyBinding};

use crate::actions::*;

/// (setting id, label, default keystroke, action).
pub struct Shortcut {
    pub id: &'static str,
    pub label: &'static str,
    pub default: &'static str,
    build: fn() -> Box<dyn Action>,
}

macro_rules! sc {
    ($id:literal, $label:literal, $key:literal, $action:expr) => {
        Shortcut { id: $id, label: $label, default: $key, build: || Box::new($action) }
    };
}

pub fn shortcuts() -> Vec<Shortcut> {
    vec![
        sc!("palette", "Command palette", "secondary-k", CommandPalette),
        sc!("new_thread", "New thread", "secondary-n", NewSession),
        sc!("toggle_sidebar", "Toggle sidebar", "secondary-b", ToggleSidebar),
        sc!("toggle_panel", "Toggle side panel", "secondary-\\", ToggleRightPanel),
        sc!("toggle_terminal", "Chat ⇄ Terminal", "ctrl-`", ToggleTerminal),
        sc!("split_right", "Split pane right", "secondary-alt-\\", SplitPaneRight),
        sc!("split_down", "Split pane down", "secondary-alt--", SplitPaneDown),
        sc!("close_pane", "Close pane", "secondary-alt-w", ClosePane),
        sc!("maximize_pane", "Maximize or restore pane", "secondary-alt-enter", MaximizePane),
        sc!("pane_left", "Focus pane on the left", "secondary-alt-left", FocusPaneLeft),
        sc!("pane_right", "Focus pane on the right", "secondary-alt-right", FocusPaneRight),
        sc!("pane_up", "Focus pane above", "secondary-alt-up", FocusPaneUp),
        sc!("pane_down", "Focus pane below", "secondary-alt-down", FocusPaneDown),
        sc!("close_tab", "Close thread tab", "secondary-w", CloseTab),
        sc!("next_tab", "Next thread tab", "ctrl-tab", NextTab),
        sc!("prev_tab", "Previous thread tab", "ctrl-shift-tab", PrevTab),
        sc!("rename", "Rename thread", "f2", RenameThread),
        sc!("threads", "All threads", "secondary-shift-t", OpenThreads),
        sc!("kanban", "Kanban", "secondary-shift-k", OpenKanban),
        sc!("back", "Back", "alt-left", GoBack),
        sc!("forward", "Forward", "alt-right", GoForward),
        sc!("settings", "Settings", "secondary-,", OpenSettings),
        sc!("add_project", "Add project", "secondary-o", AddProject),
        sc!("focus_composer", "Focus the composer", "secondary-l", FocusComposer),
        sc!("voice_call", "Start or end a voice call", "secondary-shift-space", ToggleVoiceCall),
        sc!("quit", "Quit", "secondary-q", Quit),
    ]
}

/// The keystroke for `id`, with the user's override.
pub fn binding(store: &Store, s: &Shortcut) -> String {
    store.setting(&format!("key.{}", s.id)).filter(|k| !k.is_empty()).unwrap_or_else(|| s.default.to_string())
}

fn load(key: &str, action: Box<dyn Action>) -> Option<KeyBinding> {
    KeyBinding::load(key, action, None, false, None, &gpui::DummyKeyboardMapper)
        .map_err(|e| tracing::warn!("bad shortcut {key}: {e:#}"))
        .ok()
}

/// Bind every shortcut at startup, honouring overrides. Later bindings win
/// in GPUI, so an overridden default is first disabled with `NoAction`.
pub fn apply(store: &Store, cx: &mut App) {
    let mut bindings: Vec<KeyBinding> = Vec::new();
    for s in shortcuts() {
        let key = binding(store, &s);
        if key != s.default {
            bindings.extend(load(s.default, Box::new(gpui::NoAction)));
        }
        bindings.extend(load(&key, (s.build)()));
    }
    // Ctrl+N jumps to the Nth thread tab.
    for n in 1..=9usize {
        bindings.push(KeyBinding::new(&format!("secondary-{n}"), GoToTab { index: n - 1 }, None));
    }
    cx.bind_keys(bindings);
}

/// Move one shortcut to `key` (from Settings › Keyboard).
pub fn rebind(store: &Store, id: &str, key: &str, cx: &mut App) {
    let Some(s) = shortcuts().into_iter().find(|s| s.id == id) else { return };
    let old = binding(store, &s);
    let _ = store.set_setting(&format!("key.{id}"), key);
    let mut bindings = Vec::new();
    if old != key {
        bindings.extend(load(&old, Box::new(gpui::NoAction)));
    }
    bindings.extend(load(key, (s.build)()));
    cx.bind_keys(bindings);
    if id == "voice_call" {
        crate::hotkey::apply(store);
    }
}

/// "secondary-k" → "Ctrl K" (or "⌘ K" on macOS).
pub fn pretty(key: &str) -> String {
    key.split('-')
        .map(|part| match part {
            "secondary" => if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" }.to_string(),
            "ctrl" => "Ctrl".into(),
            "shift" => "Shift".into(),
            "alt" => if cfg!(target_os = "macos") { "⌥" } else { "Alt" }.to_string(),
            "cmd" | "platform" => "⌘".into(),
            "left" => "←".into(),
            "right" => "→".into(),
            "tab" => "Tab".into(),
            "space" => "Space".into(),
            "\\" => "\\".into(),
            other => other.to_uppercase(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A keystroke recorded in Settings, in binding syntax.
pub fn from_keystroke(k: &gpui::Keystroke) -> Option<String> {
    if matches!(k.key.as_str(), "shift" | "control" | "alt" | "platform" | "function" | "") {
        return None;
    }
    let mut parts = Vec::new();
    if k.modifiers.control {
        parts.push("ctrl");
    }
    if k.modifiers.alt {
        parts.push("alt");
    }
    if k.modifiers.shift {
        parts.push("shift");
    }
    if k.modifiers.platform {
        parts.push("cmd");
    }
    let key = k.key.clone();
    let mut out = parts.join("-");
    if !out.is_empty() {
        out.push('-');
    }
    out.push_str(&key);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_ids_are_unique_and_pretty() {
        let all = shortcuts();
        let mut ids: Vec<_> = all.iter().map(|s| s.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        assert!(pretty("secondary-shift-k").ends_with("Shift K"));
        assert_eq!(pretty("alt-left").split(' ').next_back(), Some("←"));
        assert!(pretty("secondary-shift-space").ends_with("Shift Space"));
    }
}
