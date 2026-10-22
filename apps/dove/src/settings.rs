//! Settings, on Zeron's model. Opening Settings swaps the sidebar's thread
//! list for the settings sections (with "← Back"); the selected section
//! renders in the main card as a centred column of labelled cards. The
//! search at the top of the sections finds individual settings across every
//! section; choosing a result opens its section, scrolls to it and flashes
//! it. Every control saves and applies at once (text after a short pause),
//! and every value lives under the same storage key as before.
//!
//! The shell composes the page: [`SettingsPage::render_nav`] in the sidebar
//! slot and [`SettingsPage::render_content`] in the card (the Archived
//! section shows the shell's own Archived page there).

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dove_core::Engine;
use dove_core::proto::HarnessKind;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, canvas, div, point, px,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::design::{self, space, ty};
use crate::kit::controls::{self as ctl, SelectOption, SelectState, SliderState, Tone};
use crate::kit::page;
use crate::motion::{self, ReduceMotion};
use crate::theme::{Accent, Appearance, Surface, Theme};
use crate::ui;
use crate::voice::{InstallState, VoiceModel};
use crate::wallpaper::Effect;

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Section {
    General,
    Appearance,
    Notifications,
    Voice,
    Shortcuts,
    Agents,
    Team,
    /// Built-in tools, apps (Composio), MCP servers, skills and memory on
    /// one page (`pages/plugins.rs`).
    Plugins,
    Remote,
    Data,
    Archived,
    About,
}

/// What the settings page asks the shell to do.
pub enum SettingsEvent {
    /// Open the Team page (optionally its editor for a new teammate).
    OpenTeam { new: bool },
    /// Open the Agents page (sign-in terminals, binary paths).
    OpenAgents,
    /// Leave Settings (Back, or Escape).
    Close,
}

impl Section {
    #[cfg(test)]
    pub const ALL: [Section; 12] = [
        Section::General,
        Section::Appearance,
        Section::Notifications,
        Section::Voice,
        Section::Shortcuts,
        Section::Agents,
        Section::Team,
        Section::Plugins,
        Section::Remote,
        Section::Data,
        Section::Archived,
        Section::About,
    ];

    /// The navigation, grouped by spacing (no headers), as in Zeron.
    const GROUPS: [&'static [Section]; 3] = [
        &[Section::General, Section::Appearance, Section::Notifications, Section::Voice, Section::Shortcuts],
        &[Section::Agents, Section::Team, Section::Plugins, Section::Remote],
        &[Section::Data, Section::Archived, Section::About],
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Appearance => "Appearance",
            Section::Notifications => "Notifications",
            Section::Voice => "Voice",
            Section::Shortcuts => "Shortcuts",
            Section::Agents => "Agents",
            Section::Team => "Team",
            Section::Plugins => "Plugins",
            Section::Remote => "Remote",
            Section::Data => "Data & Storage",
            Section::Archived => "Archived",
            Section::About => "About",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Section::General => icons::SETTINGS,
            Section::Appearance => icons::TUNING,
            Section::Notifications => icons::BELL,
            Section::Voice => icons::MICROPHONE,
            Section::Shortcuts => icons::KEYBOARD,
            Section::Agents => icons::WIDGET,
            Section::Team => icons::USERS_GROUP,
            Section::Plugins => icons::WIDGET_ADD,
            Section::Remote => icons::SMARTPHONE,
            Section::Data => icons::HARD_DRIVE,
            Section::Archived => icons::ARCHIVE_MINIMALISTIC,
            Section::About => icons::INFO_CIRCLE,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Section::General => "general",
            Section::Appearance => "appearance",
            Section::Notifications => "notifications",
            Section::Voice => "voice",
            Section::Shortcuts => "keyboard",
            Section::Agents => "agents",
            Section::Team => "team",
            Section::Plugins => "plugins",
            Section::Remote => "remote",
            Section::Data => "data",
            Section::Archived => "archived",
            Section::About => "about",
        }
    }

    /// A section by its id, including earlier names and Zeron's: deep links
    /// (`DOVE_SETTINGS`, the composer's "Voice settings…") keep working.
    pub fn from_id(id: &str) -> Option<Section> {
        Some(match id {
            "general" | "composer" => Section::General,
            "appearance" => Section::Appearance,
            "notifications" => Section::Notifications,
            "voice" => Section::Voice,
            "keyboard" | "shortcuts" => Section::Shortcuts,
            "agents" | "providers" | "harnesses" => Section::Agents,
            "team" | "crew" => Section::Team,
            // The four pages Plugins replaced open their tab of it.
            "plugins" | "integrations" | "mcp" | "connectors" | "composio" | "apps" | "skills" | "skill" | "memory"
            | "supermemory" | "directory" | "computer" | "computer_use" | "ego" | "chrome" | "extension" | "github" => {
                Section::Plugins
            }
            "remote" | "devices" => Section::Remote,
            "data" | "storage" => Section::Data,
            "archived" => Section::Archived,
            "about" => Section::About,
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------------------
// The search index: one entry per setting row
// ---------------------------------------------------------------------------

struct Entry {
    section: Section,
    row: &'static str,
    title: &'static str,
    keywords: &'static str,
}

const fn e(section: Section, row: &'static str, title: &'static str, keywords: &'static str) -> Entry {
    Entry { section, row, title, keywords }
}

const INDEX: &[Entry] = &[
    e(Section::General, "restore_tabs", "Reopen thread tabs on launch", "restore startup tabs session"),
    e(Section::General, "topbar_tabs", "Show thread tabs in the title bar", "tabs title bar top bar strip slim"),
    e(Section::General, "prewarm", "Prewarm agent while typing", "prewarm faster first token harness start"),
    e(Section::General, "ai_titles", "Name threads with AI", "ai auto title name rename sidebar thread summary"),
    e(
        Section::General,
        "permission_mode",
        "Default permission level",
        "composer permissions ask auto-edit plan approvals mode",
    ),
    e(Section::General, "bypass", "Projects allowed to bypass permissions", "bypass permissions projects revoke trust"),
    e(Section::General, "clone_dir", "Default clone folder", "clone repository url folder directory github"),
    e(Section::General, "sidebar_group", "Group threads by project", "sidebar group project organize"),
    e(Section::General, "sidebar_density", "Compact thread rows", "sidebar density compact rows single line"),
    e(Section::General, "sidebar_archived", "Show archived threads in the sidebar", "sidebar archived show"),
    e(Section::General, "agent_browser", "Browser tools for agents", "agent browser mcp tools webview"),
    e(Section::Appearance, "appearance", "Color scheme", "theme dark light system mode appearance"),
    e(Section::Appearance, "accent", "Accent color", "accent color sky indigo orange green pink amber cyan blue"),
    e(
        Section::Appearance,
        "theme_light",
        "Light theme",
        "theme light palette colors zeron catppuccin github ayu gruvbox rose pine night owl winter vscode",
    ),
    e(
        Section::Appearance,
        "theme_dark",
        "Dark theme",
        "theme dark palette colors zeron dracula nord tokyo night one dark catppuccin synthwave cobalt vscode",
    ),
    e(Section::Appearance, "theme_import", "Import VS Code theme", "import vscode visual studio code theme json file"),
    e(Section::Appearance, "surface", "Window material", "glass acrylic normal frosted surface material blur"),
    e(Section::Appearance, "wallpaper_path", "New-thread background", "wallpaper background image backdrop home"),
    e(Section::Appearance, "wallpaper_effect", "Background effect", "dither halftone scanlines effect shader backdrop"),
    e(Section::Appearance, "wallpaper_animate", "Animate background", "animate animation backdrop wallpaper motion"),
    e(Section::Appearance, "reduce_motion", "Reduce motion", "motion animation accessibility reduce"),
    e(
        Section::Appearance,
        "pause_animations_in_background",
        "Pause animations in background",
        "pause animation background inactive battery",
    ),
    e(Section::Appearance, "ui_font_size", "Interface font size", "font size text scale zoom interface ui"),
    e(Section::Appearance, "code_font_size", "Code & diff font size", "font size code diff monospace"),
    e(Section::Appearance, "terminal_font_size", "Terminal font size", "font size terminal monospace"),
    e(
        Section::Appearance,
        "terminal_scrollback",
        "Scrollback lines",
        "terminal scrollback history lines buffer memory",
    ),
    e(Section::Appearance, "conversation_width", "Conversation width", "width column chat transcript conversation"),
    e(
        Section::Appearance,
        "home_input",
        "New-thread input",
        "composer input bar bottom center dock home layout permissions usage",
    ),
    e(Section::Notifications, "notifications", "Desktop notifications", "notifications alerts system os"),
    e(Section::Notifications, "notification_sound", "Completion sound", "sound chime audio notification"),
    e(
        Section::Notifications,
        "keep_running",
        "Keep running in background",
        "tray background daemon automations keep running",
    ),
    e(Section::Voice, "voice_model", "Dictation model", "voice dictation parakeet nvidia model download speech"),
    e(Section::Voice, "voice_device", "Microphone", "microphone input device audio voice"),
    e(
        Section::Voice,
        "voice_call_mode",
        "Voice Mode",
        "voice mode call hands free talk conversation listening push to talk",
    ),
    e(Section::Voice, "voice_name", "Voice", "voice speech tts speaker system voice"),
    e(Section::Voice, "voice_global_hotkey", "Shortcut outside Dove", "global hotkey shortcut voice call system wide"),
    e(Section::Voice, "voice_rate", "Speaking rate", "voice speed rate faster slower"),
    e(
        Section::Voice,
        "voice_echo_cancel",
        "Echo cancellation",
        "echo cancellation speakers headphones barge in interrupt",
    ),
    e(
        Section::Voice,
        "voice_announce",
        "Announce thread updates",
        "voice announce notifications finished failed approval",
    ),
    e(Section::Voice, "voice_earcons", "Sounds", "voice chime earcon sound cue ready heard beep"),
    e(Section::Voice, "voice_call_models", "Voice Mode models", "silero vad smart turn detection model download"),
    e(Section::Voice, "voice_cloud", "Cloud speech recognition", "cloud openai whisper groq speech api upload audio"),
    e(Section::Voice, "voice_router_url", "Command router", "router llama llm intent model voice commands"),
    e(Section::Shortcuts, "shortcuts", "Keyboard shortcuts", "keyboard shortcuts keybindings hotkeys rebind keys"),
    e(Section::Agents, "agents", "Agents", "agents harness claude codex cursor gemini opencode cli binary enable hide"),
    e(Section::Team, "team_roster", "Teammates", "team teammates personas roster specialists"),
    e(Section::Team, "team_notes", "Persistent teammate notes", "team notes memory learn"),
    e(
        Section::Plugins,
        "integrations",
        "Plugins: built in and MCP servers",
        "integrations mcp servers custom oauth sign in catalog figma linear notion chrome extension paper ego lite browser computer use dove-driver plugin github cli",
    ),
    e(
        Section::Plugins,
        "connectors",
        "Plugins: apps through Composio",
        "connectors composio apps gmail slack notion github linear calendar toolkits connect",
    ),
    e(
        Section::Plugins,
        "skills",
        "Plugins: skills",
        "skills agent skills library install github superpowers anthropic composio ego browser discover update",
    ),
    e(Section::Plugins, "agent_office_skills", "Office document skills", "office docx pptx xlsx pdf skills agents"),
    e(
        Section::Plugins,
        "memory",
        "Plugins: memory",
        "memory supermemory recall remember forget capture corrections learn profile never send",
    ),
    e(Section::Remote, "remote_enabled", "Remote control", "remote phone mobile companion woozlit account"),
    e(Section::Data, "data_size", "Storage", "data storage disk size folder"),
    e(Section::Data, "data_path", "Storage location", "dove_home path location folder"),
    e(Section::Data, "telemetry", "Privacy", "telemetry privacy local"),
    e(Section::Archived, "archived", "Archived threads", "archived threads unarchive delete"),
    e(Section::About, "version", "Version", "about version license source github"),
];

/// Every searchable setting, for the command palette:
/// `(section id, row id, title, section label)`.
pub fn search_index() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    INDEX.iter().map(|e| (e.section.id(), e.row, e.title, e.section.label())).collect()
}

fn find_settings(query: &str) -> Vec<&'static Entry> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut hits: Vec<(u8, &'static Entry)> = INDEX
        .iter()
        .filter_map(|e| {
            let title = e.title.to_ascii_lowercase();
            let section = e.section.label().to_ascii_lowercase();
            let all = format!("{title} {section} {}", e.keywords);
            if !words.iter().all(|w| all.contains(w)) {
                return None;
            }
            let rank = if title.starts_with(&q) {
                0
            } else if title.contains(&q) {
                1
            } else {
                2
            };
            Some((rank, e))
        })
        .collect();
    hits.sort_by_key(|(rank, _)| *rank);
    hits.into_iter().map(|(_, e)| e).collect()
}

// ---------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------

/// How long a revealed setting glows.
const FLASH: Duration = Duration::from_millis(900);
/// Text fields save this long after typing stops (and on blur).
const SAVE_AFTER: Duration = Duration::from_millis(400);

pub struct SettingsPage {
    engine: Arc<Engine>,
    pub section: Section,
    search: Entity<InputState>,
    query: String,
    /// Keyboard cursor in the search results.
    result: Option<usize>,
    clone_dir: Entity<InputState>,
    /// The shortcut being recorded.
    recording: Option<&'static str>,
    devices: Vec<(String, String)>,
    data_size: Option<u64>,
    integrations: Entity<crate::pages::integrations::IntegrationsView>,
    /// Held here; Plugins (the integrations view) draws them.
    _skills: Entity<crate::pages::skills::SkillsView>,
    _memory: Entity<crate::pages::memory_settings::MemoryView>,
    focus: FocusHandle,
    nav_focus: FocusHandle,
    scroll: ScrollHandle,
    /// The row to bring into view on the next paint.
    pending_scroll: Rc<Cell<Option<&'static str>>>,
    flash: Option<(&'static str, Instant)>,
    /// Live value of the conversation-width slider while it is dragged.
    width_drag: Option<f32>,
    sel_mode: SelectState,
    sel_surface: SelectState,
    sel_effect: SelectState,
    sel_reduce: SelectState,
    sel_theme_light: SelectState,
    sel_theme_dark: SelectState,
    /// The last VS Code import: the theme's name, or why it failed.
    theme_import: Option<Result<SharedString, SharedString>>,
    importing_theme: bool,
    sel_ui_size: SelectState,
    sel_code_size: SelectState,
    sel_term_size: SelectState,
    sel_scrollback: SelectState,
    sel_home_input: SelectState,
    sel_mic: SelectState,
    sel_voice: SelectState,
    /// System voices (loaded off-thread when Voice opens).
    voices: Vec<String>,
    /// Voice Mode model downloads: in flight (None) or failed (Some).
    voice_dl: HashMap<&'static str, Option<String>>,
    voice_cloud_url: Entity<InputState>,
    voice_router_url: Entity<InputState>,
    width_slider: SliderState,
    save_gen: HashMap<&'static str, u64>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsPage {}

impl SettingsPage {
    pub fn new(engine: Arc<Engine>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let style = ui::input_style(Theme::of(cx));
        let search = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Search settings");
            s.set_editor_style(style.clone());
            s
        });
        let search_sub = cx.subscribe(&search, |this, s, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                this.query = s.read(cx).value().to_string();
                this.result = (!this.query.trim().is_empty()).then_some(0);
                cx.notify();
            }
            if let InputEvent::PressEnter { .. } = ev
                && let Some(e) = find_settings(&this.query).get(this.result.unwrap_or(0)).copied()
            {
                this.reveal_entry(e, cx);
            }
        });
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let clone_value = engine.store.setting("clone_dir").unwrap_or_else(|| home.join("Dove").display().to_string());
        let clone_dir = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Folder for cloned repositories");
            s.set_editor_style(style.clone());
            s.set_value(clone_value, window, cx);
            s
        });
        let text_field = |placeholder: &'static str, key: &str, window: &mut Window, cx: &mut Context<Self>| {
            let value = engine.store.setting(key).unwrap_or_default();
            let style = style.clone();
            cx.new(|cx| {
                let mut s = InputState::new(window, cx).placeholder(placeholder);
                s.set_editor_style(style);
                s.set_value(value, window, cx);
                s
            })
        };
        let voice_cloud_url = text_field("https://api.openai.com/v1", "voice_cloud_url", window, cx);
        let voice_router_url = text_field("http://127.0.0.1:8080/v1 (off)", "voice_router_url", window, cx);
        let mut subs = vec![search_sub];
        for (input, key) in
            [(&clone_dir, "clone_dir"), (&voice_cloud_url, "voice_cloud_url"), (&voice_router_url, "voice_router_url")]
        {
            subs.push(cx.subscribe(input, move |this, s, ev: &InputEvent, cx| match ev {
                InputEvent::Change => {
                    let value = s.read(cx).value().trim().to_string();
                    this.save_later(key, value, cx);
                }
                InputEvent::Blur => {
                    let value = s.read(cx).value().trim().to_string();
                    this.save_gen.entry(key).and_modify(|g| *g += 1);
                    let _ = this.engine.store.set_setting(key, &value);
                }
                _ => {}
            }));
            subs.push(cx.observe(input, |_, _, cx| cx.notify()));
        }
        if let Some(model) = VoiceModel::global(cx) {
            subs.push(cx.observe(&model, |_, _, cx| cx.notify()));
        }
        if let Some(remote) = crate::remote::Remote::global(cx) {
            subs.push(cx.observe(&remote, |_, _, cx| cx.notify()));
        }
        let section =
            engine.store.setting("settings_section").and_then(|s| Section::from_id(&s)).unwrap_or(Section::General);
        let integrations = cx.new(|cx| crate::pages::integrations::IntegrationsView::new(window, cx));
        subs.push(cx.observe(&integrations, |_, _, cx| cx.notify()));
        let skills = cx.new(|cx| crate::pages::skills::SkillsView::new(window, cx));
        subs.push(cx.observe(&skills, |_, _, cx| cx.notify()));
        let memory = {
            let (engine, integrations) = (engine.clone(), integrations.clone());
            cx.new(|cx| crate::pages::memory_settings::MemoryView::new(engine, integrations, window, cx))
        };
        subs.push(cx.observe(&memory, |_, _, cx| cx.notify()));
        integrations.update(cx, |v, _| v.embed(&skills, &memory));
        Self {
            engine,
            section,
            search,
            query: String::new(),
            result: None,
            clone_dir,
            recording: None,
            devices: Vec::new(),
            data_size: None,
            integrations,
            _skills: skills,
            _memory: memory,
            focus: cx.focus_handle(),
            nav_focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            pending_scroll: Rc::default(),
            flash: None,
            width_drag: None,
            sel_mode: SelectState::default(),
            sel_surface: SelectState::default(),
            sel_effect: SelectState::default(),
            sel_reduce: SelectState::default(),
            sel_theme_light: SelectState::default(),
            sel_theme_dark: SelectState::default(),
            theme_import: None,
            importing_theme: false,
            sel_ui_size: SelectState::default(),
            sel_code_size: SelectState::default(),
            sel_term_size: SelectState::default(),
            sel_scrollback: SelectState::default(),
            sel_home_input: SelectState::default(),
            sel_mic: SelectState::default(),
            sel_voice: SelectState::default(),
            voices: Vec::new(),
            voice_dl: HashMap::new(),
            voice_cloud_url,
            voice_router_url,
            width_slider: SliderState::default(),
            save_gen: HashMap::new(),
            _subs: subs,
        }
    }

    /// Open a section by id (deep links); unknown ids keep the current one.
    /// Plugins' old page ids open their tab of it.
    pub fn show_section(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(s) = Section::from_id(id) {
            self.select(s, cx);
            if s == Section::Plugins {
                self.integrations.update(cx, |v, cx| v.open(id, cx));
            }
        }
    }

    /// Open the section holding `row`, bring it into view and flash it.
    pub fn reveal(&mut self, row: &str, cx: &mut Context<Self>) {
        if let Some(e) = INDEX.iter().find(|e| e.row == row) {
            self.reveal_entry(e, cx);
        }
    }

    fn reveal_entry(&mut self, e: &'static Entry, cx: &mut Context<Self>) {
        self.select(e.section, cx);
        if e.section == Section::Plugins {
            self.integrations.update(cx, |v, cx| v.open(e.row, cx));
        }
        self.pending_scroll.set(Some(e.row));
        self.flash = Some((e.row, Instant::now()));
        cx.notify();
    }

    /// Whether a shortcut is being recorded (Escape belongs to it then).
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    fn select(&mut self, s: Section, cx: &mut Context<Self>) {
        if self.section != s {
            self.scroll.set_offset(point(px(0.0), px(0.0)));
        }
        self.section = s;
        self.recording = None;
        let _ = self.engine.store.set_setting("settings_section", s.id());
        match s {
            Section::Voice if self.devices.is_empty() => {
                let load = cx.background_spawn(async move {
                    use dove_voice::engines::TextToSpeech as _;
                    (crate::voice::input_devices(), dove_voice::tts::System::new().voices())
                });
                cx.spawn(async move |this, cx| {
                    let (devices, voices) = load.await;
                    let _ = this.update(cx, |p, cx| {
                        p.devices = devices;
                        p.voices = voices;
                        cx.notify();
                    });
                })
                .detach();
            }
            Section::Data => {
                let root = self.engine.store.root().to_path_buf();
                let load = cx.background_spawn(async move { dir_size(&root) });
                cx.spawn(async move |this, cx| {
                    let size = load.await;
                    let _ = this.update(cx, |p, cx| {
                        p.data_size = Some(size);
                        cx.notify();
                    });
                })
                .detach();
            }
            _ => {}
        }
        cx.notify();
    }

    fn get(&self, key: &str) -> Option<String> {
        self.engine.store.setting(key)
    }

    fn set(&self, key: &str, value: &str, cx: &mut Context<Self>) {
        let _ = self.engine.store.set_setting(key, value);
        crate::apply_theme(&self.engine, cx);
        cx.notify();
    }

    fn flag(&self, key: &str, default: bool) -> bool {
        self.engine.store.flag(key, default)
    }

    fn set_flag(&self, key: &str, on: bool, cx: &mut Context<Self>) {
        self.set(key, if on { "true" } else { "false" }, cx);
    }

    /// Save `value` under `key` once typing pauses.
    fn save_later(&mut self, key: &'static str, value: String, cx: &mut Context<Self>) {
        let generation = {
            let g = self.save_gen.entry(key).or_insert(0);
            *g += 1;
            *g
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_AFTER).await;
            let _ = this.update(cx, |p, _| {
                if p.save_gen.get(key) == Some(&generation) {
                    let _ = p.engine.store.set_setting(key, &value);
                }
            });
        })
        .detach();
    }

    fn pick_wallpaper(&mut self, cx: &mut Context<Self>) {
        let pick = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose background image".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = pick.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let _ = this.update(cx, |s, cx| s.set("wallpaper_path", &path.to_string_lossy(), cx));
        })
        .detach();
    }

    /// Pick a VS Code theme JSON, import it (off the UI thread), save it
    /// under DOVE_HOME and switch the matching appearance to it.
    fn import_theme(&mut self, cx: &mut Context<Self>) {
        if self.importing_theme {
            return;
        }
        let pick = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import VS Code theme".into()),
        });
        let root = self.engine.store.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = pick.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            if this
                .update(cx, |s, cx| {
                    s.importing_theme = true;
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
            let result = cx.background_spawn(async move { crate::themes::import_vscode(&path, &root) }).await;
            let _ = this.update(cx, |s, cx| {
                s.importing_theme = false;
                match result {
                    Ok(palette) => {
                        s.theme_import = Some(Ok(palette.name.clone().into()));
                        s.set(crate::themes::key_for(palette.appearance()), &palette.id, cx);
                    }
                    Err(e) => {
                        s.theme_import = Some(Err(format!("{e:#}").into()));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn pick_clone_dir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pick = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Select clone folder".into()),
        });
        let input = self.clone_dir.clone();
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = pick.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let val = path.to_string_lossy().to_string();
            let _ = handle.update(cx, |_, window, cx| {
                input.update(cx, |s, cx| s.set_value(val.clone(), window, cx));
            });
            let _ = this.update(cx, |s, cx| s.set("clone_dir", &val, cx));
        })
        .detach();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.recording else { return };
        if ev.keystroke.key == "escape" {
            self.recording = None;
        } else if let Some(key) = crate::keymap::from_keystroke(&ev.keystroke) {
            crate::keymap::rebind(&self.engine.store, id, &key, cx);
            self.recording = None;
        } else {
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// 0…1 glow of a revealed row.
    fn flash_of(&self, row: &str) -> f32 {
        match self.flash {
            Some((r, at)) if r == row => {
                let raw = at.elapsed().as_secs_f32() / FLASH.as_secs_f32();
                if raw >= 1.0 { 0.0 } else { 1.0 - motion::EASE_OUT.eval(raw) }
            }
            _ => 0.0,
        }
    }

    /// A setting row with its search anchor: it scrolls into view and glows
    /// when revealed from search or the palette.
    fn srow(
        &self,
        t: &Theme,
        first: bool,
        row: &'static str,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        control: impl IntoElement,
    ) -> AnyElement {
        let glow = self.flash_of(row);
        page::setting_row(t, first, title, description, control)
            .relative()
            .when(glow > 0.0, |d| d.bg(t.accent_wash.opacity(glow)).rounded(px(design::radius::RADIUS)))
            .child(self.anchor(row))
            .into_any_element()
    }

    /// A zero-size probe that, when `row` is being revealed, scrolls the page
    /// so the element it sits in lands near the top.
    fn anchor(&self, row: &'static str) -> AnyElement {
        let pending = self.pending_scroll.clone();
        let scroll = self.scroll.clone();
        canvas(
            move |bounds, window, _| {
                if pending.get() == Some(row) {
                    pending.set(None);
                    let area = scroll.bounds();
                    let offset = scroll.offset();
                    let max = scroll.max_offset();
                    let delta = bounds.origin.y - area.origin.y - px(24.0);
                    let y = (offset.y - delta).clamp(-max.y, px(0.0));
                    scroll.set_offset(point(offset.x, y));
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }

    // ------------------------------------------------------------ navigation

    /// The sections, in the sidebar's slot: Back, search, then the groups
    /// (or, while searching, the matching settings).
    pub fn render_nav(&mut self, width: f32, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let query = self.query.trim().to_string();
        let back = div()
            .id("set-back")
            .mx(px(8.0))
            .h(px(32.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(9.0))
            .rounded(px(design::radius::RADIUS))
            .cursor_pointer()
            .text_size(ty::BODY)
            .text_color(motion::hover_blend("set-back", t.text_muted, t.text))
            .bg(motion::hover_blend("set-back", t.wash(0.0), t.wash(0.06)))
            .on_hover(motion::hover_listener("set-back"))
            .role(gpui::Role::Button)
            .aria_label("Back")
            .tooltip(ui::tip("Back to where you were (Esc)"))
            .child(icon(icons::ARROW_LEFT).size(px(16.0)))
            .child("Back")
            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close)));

        let search_box = div()
            .mx(px(8.0))
            .h(px(32.0))
            .px(px(9.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(design::radius::RADIUS))
            .bg(t.wash(0.05))
            .border_1()
            .border_color(t.wash(0.06))
            .text_size(ty::BODY)
            .child(icon(icons::MAGNIFER).size(px(14.0)).text_color(t.text_faint))
            .child(div().flex_1().min_w_0().child(Input::new(&self.search)))
            .when(!query.is_empty(), |d| {
                d.child(
                    div()
                        .id("set-search-clear")
                        .size(px(20.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(design::radius::RADIUS))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.wash(0.08)))
                        .tooltip(ui::tip("Clear search"))
                        .child(icon(icons::CLOSE).size(px(10.0)).text_color(t.text_faint))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.search.update(cx, |s, cx| s.set_value(String::new(), window, cx));
                            this.query.clear();
                            this.result = None;
                            cx.notify();
                        })),
                )
            })
            .capture_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if this.query.trim().is_empty() {
                    return;
                }
                let count = find_settings(&this.query).len();
                let step = match e.keystroke.key.as_str() {
                    "down" => 1,
                    "up" => -1,
                    _ => return,
                };
                this.result = crate::kit::popover::menu_step(this.result, count, step);
                cx.stop_propagation();
                cx.notify();
            }));

        let body: AnyElement = if query.is_empty() {
            let mut groups = div().flex().flex_col().gap(px(14.0));
            for group in Section::GROUPS {
                groups = groups.child(div().flex().flex_col().gap(px(1.0)).children(group.iter().map(|&s| {
                    let selected = s == self.section;
                    let key: SharedString = format!("set-nav-{}", s.id()).into();
                    let badge = self.nav_badge(s, &t, cx);
                    let rest_bg = if selected { ui::selected_bg(&t) } else { t.wash(0.0) };
                    let rest_fg = if selected { t.text } else { t.text_muted };
                    div()
                        .id(gpui::ElementId::Name(key.clone()))
                        .mx(px(8.0))
                        .h(px(32.0))
                        .px(px(8.0))
                        .flex()
                        .items_center()
                        .gap(px(9.0))
                        .rounded(px(design::radius::RADIUS))
                        .cursor_pointer()
                        .role(gpui::Role::Tab)
                        .aria_selected(selected)
                        .aria_label(s.label())
                        .text_size(ty::BODY)
                        .when(selected, |d| d.font_weight(gpui::FontWeight::MEDIUM).shadow(ui::selected_ring(&t)))
                        .text_color(motion::hover_blend(&key, rest_fg, t.text))
                        .bg(motion::hover_blend(&key, rest_bg, if selected { rest_bg } else { t.wash(0.05) }))
                        .on_hover(motion::hover_listener(key.clone()))
                        .child(icon(s.glyph()).size(px(16.0)))
                        .child(div().flex_1().min_w_0().truncate().child(s.label()))
                        .children(badge)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            window.focus(&this.nav_focus, cx);
                            this.select(s, cx);
                        }))
                })));
            }
            groups.into_any_element()
        } else {
            let hits = find_settings(&query);
            if hits.is_empty() {
                div()
                    .px(px(16.0))
                    .py(px(8.0))
                    .text_size(ty::BODY)
                    .text_color(t.text_faint)
                    .child("No settings match")
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .children(hits.into_iter().enumerate().map(|(i, e)| {
                        let active = self.result == Some(i);
                        div()
                            .id(SharedString::from(format!("set-hit-{}", e.row)))
                            .mx(px(8.0))
                            .px(px(8.0))
                            .py(px(6.0))
                            .flex()
                            .flex_col()
                            .rounded(px(design::radius::RADIUS))
                            .cursor_pointer()
                            .when(active, |d| d.bg(ui::selected_bg(&t)))
                            .hover(|s| s.bg(t.wash(0.06)))
                            .child(div().text_size(ty::BODY).text_color(t.text).truncate().child(e.title))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.0))
                                    .text_size(ty::CAPTION)
                                    .text_color(t.text_faint)
                                    .child(icon(e.section.glyph()).size(px(11.0)))
                                    .child(e.section.label()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| this.reveal_entry(e, cx)))
                    }))
                    .into_any_element()
            }
        };

        div()
            .id("settings-nav")
            .track_focus(&self.nav_focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                let order: Vec<Section> = Section::GROUPS.iter().flat_map(|g| g.iter().copied()).collect();
                let at = order.iter().position(|s| *s == this.section).unwrap_or(0);
                let next = match e.keystroke.key.as_str() {
                    "down" => Some((at + 1).min(order.len() - 1)),
                    "up" => Some(at.saturating_sub(1)),
                    "home" => Some(0),
                    "end" => Some(order.len() - 1),
                    _ => None,
                };
                if let Some(i) = next {
                    this.select(order[i], cx);
                    cx.stop_propagation();
                }
            }))
            .w(px(width))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .pt(px(8.0))
            .pb(px(12.0))
            .overflow_hidden()
            .child(back)
            .child(search_box)
            .child(div().id("settings-nav-list").flex_1().min_h_0().overflow_y_scroll().pt(px(4.0)).child(body))
            .into_any_element()
    }

    /// Optional status badge on a navigation row.
    fn nav_badge(&self, s: Section, t: &Theme, cx: &App) -> Option<AnyElement> {
        let count = |n: usize| {
            div()
                .px(px(5.0))
                .h(px(16.0))
                .flex()
                .items_center()
                .rounded(px(design::radius::fit(16.0)))
                .bg(t.wash(0.08))
                .text_size(ty::BADGE)
                .text_color(t.text_muted)
                .child(n.to_string())
                .into_any_element()
        };
        match s {
            Section::Agents => Some(self.engine.installed().len()).filter(|n| *n > 0).map(count),
            Section::Team => {
                Some(self.engine.store.personas().map(|v| v.len()).unwrap_or(0)).filter(|n| *n > 0).map(count)
            }
            Section::Voice => VoiceModel::global(cx)
                .is_some_and(|m| matches!(m.read(cx).state, InstallState::Installed))
                .then(|| ui::dot(t.success, 6.0).into_any_element()),
            Section::Remote => crate::remote::Remote::global(cx)
                .is_some_and(|r| {
                    let r = r.read(cx);
                    r.status.connected && r.enabled()
                })
                .then(|| ui::dot(t.success, 6.0).into_any_element()),
            _ => None,
        }
    }

    // ------------------------------------------------------------ content

    /// The selected section's page. `None` for Archived, which the shell
    /// draws with its own Archived page.
    pub fn render_content(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = Theme::of(cx).clone();
        if self.flash.is_some_and(|(_, at)| at.elapsed() < FLASH) {
            window.request_animation_frame();
        } else {
            self.flash = None;
        }
        let (header_action, body): (Option<AnyElement>, AnyElement) = match self.section {
            Section::General => (None, self.general(&t, window, cx)),
            Section::Appearance => (None, self.appearance(&t, window, cx)),
            Section::Notifications => (None, self.notifications(&t, cx)),
            Section::Voice => (None, self.voice(&t, cx)),
            Section::Shortcuts => (self.shortcuts_action(&t, cx), self.shortcuts(&t, cx)),
            Section::Agents => (
                Some(
                    ctl::button_sm(&t, "set-configure-agents", "Sign-in & paths", Tone::Outlined)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::OpenAgents)))
                        .into_any_element(),
                ),
                self.agents(&t, cx),
            ),
            Section::Team => (
                Some(
                    ctl::button_sm(&t, "set-team-new", "New teammate", Tone::Solid)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::OpenTeam { new: true })))
                        .into_any_element(),
                ),
                self.team(&t, cx),
            ),
            Section::Plugins => (Some(self.plugins_actions(&t)), self.plugins_section(&t)),
            Section::Remote => (None, self.remote(&t, cx)),
            Section::Data => (None, self.data(&t, cx)),
            Section::About => (None, self.about(&t, cx)),
            Section::Archived => return None,
        };
        Some(
            div()
                .id("settings-scroll")
                .track_focus(&self.focus)
                .on_key_down(cx.listener(Self::on_key))
                .track_scroll(&self.scroll)
                .size_full()
                .overflow_y_scroll()
                .child(
                    page::page_column().child(page::page_header(&t, self.section.label(), header_action)).child(body),
                )
                .into_any_element(),
        )
    }

    // ------------------------------------------------------------ sections

    fn general(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let restore = self.flag("restore_tabs", true);
        let tabs = self.flag("topbar_tabs", false);
        let prewarm = self.flag("prewarm", true);
        let ai_titles = self.flag("ai_titles", true);
        let grouped = self.get("sidebar_group").as_deref() == Some("project");
        let compact = self.get("sidebar_density").as_deref() == Some("compact");
        let archived = self.flag("sidebar_archived", false);
        let browser = self.flag("agent_browser", true);

        let threads = page::card(t)
            .child(self.srow(
                t,
                true,
                "restore_tabs",
                "Reopen thread tabs on launch",
                "Restore the tabs you had open last time.",
                ctl::on_toggle(ctl::toggle(t, "set-restore", restore), cx, move |this, _, cx| {
                    this.set_flag("restore_tabs", !restore, cx)
                }),
            ))
            .child(self.srow(
                t,
                false,
                "topbar_tabs",
                "Show thread tabs in the title bar",
                "Off: a slim bar with the open chat's title. The sidebar lists every thread; Ctrl+Tab and Ctrl+1–9 still switch between open ones.",
                ctl::on_toggle(ctl::toggle(t, "set-topbar-tabs", tabs), cx, move |this, _, cx| {
                    this.set_flag("topbar_tabs", !tabs, cx)
                }),
            ))
            .child(self.srow(
                t,
                false,
                "prewarm",
                "Prewarm agent while typing",
                "Start the agent in the background as you type, so the first reply arrives sooner.",
                ctl::on_toggle(ctl::toggle(t, "set-prewarm", prewarm), cx, move |this, _, cx| {
                    this.set_flag("prewarm", !prewarm, cx)
                }),
            ))
            .child(self.srow(
                t,
                false,
                "ai_titles",
                "Name threads with AI",
                "After the first reply, Claude Code or Codex names the thread in a few words, as their own apps do. Off: threads keep a title made from your first message. A rename always wins.",
                ctl::on_toggle(ctl::toggle(t, "set-ai-titles", ai_titles), cx, move |this, _, cx| {
                    this.set_flag("ai_titles", !ai_titles, cx)
                }),
            ));

        let mode = self.get("permission_mode").unwrap_or_else(|| "default".into());
        let modes = ["default", "accept_edits", "plan"];
        let mode_select = ctl::select::<Self>("set-mode", "Default permission level", |s| &mut s.sel_mode)
            .options(
                [
                    SelectOption::new("Ask every time").detail("Approve edits and commands"),
                    SelectOption::new("Auto-edit").detail("Edits apply; commands ask"),
                    SelectOption::new("Plan").detail("Read-only until you approve a plan"),
                ],
                modes.iter().position(|m| *m == mode).unwrap_or(0),
            )
            .width(170.0)
            .menu_width(320.0)
            .on_select(move |this, i, _, cx| this.set("permission_mode", modes[i], cx))
            .render(t, &self.sel_mode, cx);
        let bypassed: Vec<String> = self
            .engine
            .store
            .projects()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| self.get(&format!("bypass_ok.{}", p.id)).as_deref() == Some("1"))
            .map(|p| p.name)
            .collect();
        let bypass_control = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(if bypassed.is_empty() {
                page::badge(t, "None").into_any_element()
            } else {
                div()
                    .max_w(px(240.0))
                    .truncate()
                    .text_size(ty::META)
                    .text_color(t.text_muted)
                    .child(bypassed.join(", "))
                    .into_any_element()
            })
            .when(!bypassed.is_empty(), |d| {
                d.child(ctl::button_sm(t, "bypass-reset", "Revoke all", Tone::Danger).on_click(cx.listener(
                    |this, _, _, cx| {
                        for p in this.engine.store.projects().unwrap_or_default() {
                            let _ = this.engine.store.set_setting(&format!("bypass_ok.{}", p.id), "0");
                        }
                        cx.notify();
                    },
                )))
            });
        let composer = page::card(t)
            .child(self.srow(
                t,
                true,
                "permission_mode",
                "Default permission level",
                "New threads start in this mode. Bypass is chosen per thread and confirmed once per project.",
                mode_select,
            ))
            .child(self.srow(
                t,
                false,
                "bypass",
                "Projects allowed to bypass permissions",
                if bypassed.is_empty() {
                    "No project has bypass enabled."
                } else {
                    "Agents may run without asking in these projects when Bypass is chosen."
                },
                bypass_control,
            ));

        let clone_focused = self.clone_dir.read(cx).focus_handle(cx).is_focused(window);
        let git = page::card(t).child(
            self.srow(
                t,
                true,
                "clone_dir",
                "Default clone folder",
                "Where repositories cloned from a URL go.",
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(ctl::text_field(&self.clone_dir, Some(230.0), clone_focused, t))
                    .child(
                        ctl::button_sm(t, "browse-clone-dir", "Browse…", Tone::Filled)
                            .on_click(cx.listener(|this, _, window, cx| this.pick_clone_dir(window, cx))),
                    ),
            ),
        );

        let sidebar = page::card(t)
            .child(self.srow(
                t,
                true,
                "sidebar_group",
                "Group threads by project",
                "Also in the sidebar's options menu.",
                ctl::on_toggle(ctl::toggle(t, "set-sb-group", grouped), cx, move |this, _, cx| {
                    this.set("sidebar_group", if grouped { "none" } else { "project" }, cx)
                }),
            ))
            .child(self.srow(
                t,
                false,
                "sidebar_density",
                "Compact thread rows",
                "One line per thread; project and branch show on hover.",
                ctl::on_toggle(ctl::toggle(t, "set-sb-compact", compact), cx, move |this, _, cx| {
                    this.set("sidebar_density", if compact { "comfortable" } else { "compact" }, cx)
                }),
            ))
            .child(self.srow(
                t,
                false,
                "sidebar_archived",
                "Show archived threads in the sidebar",
                "Listed under the active threads.",
                ctl::on_toggle(ctl::toggle(t, "set-sb-archived", archived), cx, move |this, _, cx| {
                    this.set_flag("sidebar_archived", !archived, cx)
                }),
            ));

        let tools = page::card(t).child(self.srow(
            t,
            true,
            "agent_browser",
            "Browser tools for agents",
            "Agents can drive the in-app browser. Applies after Dove restarts.",
            ctl::on_toggle(ctl::toggle(t, "set-agent-browser", browser), cx, move |this, _, cx| {
                this.set_flag("agent_browser", !browser, cx)
            }),
        ));

        div()
            .flex()
            .flex_col()
            .child(page::section(t, "Threads", threads))
            .child(page::section(t, "Composer", composer))
            .child(page::section(t, "Git & worktrees", git))
            .child(page::section(t, "Sidebar", sidebar))
            .child(page::section(t, "Agent tools", tools))
            .into_any_element()
    }

    fn appearance(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.get("appearance").unwrap_or_else(|| "system".into());
        let accent = Accent::from_id(&self.get("accent").unwrap_or_default());
        let surface = Surface::from_settings(self.get("surface").as_deref(), self.get("frosted").as_deref());
        let wall = self.get("wallpaper_path").filter(|p| !p.is_empty());
        let effect = Effect::from_id(&self.get("wallpaper_effect").unwrap_or_default());
        let animate = self.flag("wallpaper_animate", false);

        // Color scheme: three preview cards.
        let schemes =
            [("system", "System", icons::MONITOR), ("light", "Light", icons::SUN), ("dark", "Dark", icons::MOON)];
        let cards = div().w_full().flex().gap(px(space::LG)).children(schemes.into_iter().map(|(id, label, glyph)| {
            let selected = mode == id;
            let ring = if selected { t.accent } else { t.border };
            div()
                .id(SharedString::from(format!("scheme-{id}")))
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.0))
                .cursor_pointer()
                .role(gpui::Role::RadioButton)
                .aria_selected(selected)
                .aria_label(label)
                .child(
                    div()
                        .w_full()
                        .h(px(104.0))
                        .rounded(px(design::radius::RADIUS))
                        .overflow_hidden()
                        .border_1()
                        .border_color(ring)
                        .when(selected, |d| {
                            d.shadow(vec![gpui::BoxShadow {
                                color: t.accent.opacity(0.35),
                                offset: point(px(0.0), px(0.0)),
                                blur_radius: px(0.0),
                                spread_radius: px(1.0),
                                inset: false,
                            }])
                        })
                        .child(scheme_preview(id)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(ty::BODY)
                        .when(selected, |d| d.font_weight(gpui::FontWeight::MEDIUM))
                        .text_color(if selected { t.accent } else { t.text_muted })
                        .child(icon(glyph).size(px(15.0)))
                        .child(label),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.set("appearance", id, cx)))
        }));

        // While a library theme is active its own accent is a choice too
        // (and the default when no accent was picked).
        let theme_accent = t.themed.as_ref().map(|applied| applied.palette.accent.hsla());
        let follows_theme = t.themed.as_ref().is_some_and(|applied| applied.theme_accent);
        let swatches = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .children(theme_accent.map(|color| {
                ctl::swatch(t, "accent-theme", color, follows_theme, "Theme default")
                    .tooltip(ui::tip("Theme default"))
                    .on_click(cx.listener(|this, _, _, cx| this.set("accent", crate::themes::THEME_ACCENT, cx)))
            }))
            .children(Accent::ALL.into_iter().map(|a| {
                let color = Theme::new(t.appearance, a, Surface::Normal).accent;
                ctl::swatch(t, format!("accent-{}", a.id()), color, !follows_theme && a == accent, a.label())
                    .tooltip(ui::tip(a.label()))
                    .on_click(cx.listener(move |this, _, _, cx| this.set("accent", a.id(), cx)))
            }));
        // Light and dark themes: Dove's look, Zeron's library, imports.
        let light_select = self.theme_select(Appearance::Light, |s| &mut s.sel_theme_light, &self.sel_theme_light, cx);
        let dark_select = self.theme_select(Appearance::Dark, |s| &mut s.sel_theme_dark, &self.sel_theme_dark, cx);
        let import_note: SharedString = if self.importing_theme {
            "Importing…".into()
        } else {
            match &self.theme_import {
                Some(Ok(name)) => format!("Imported “{name}” and switched to it.").into(),
                Some(Err(e)) => format!("Could not import: {e}").into(),
                None => "A color theme JSON from a VS Code extension, saved in Dove's data folder.".into(),
            }
        };
        let import_button = ctl::button_sm(t, "theme-import", "Import…", Tone::Filled)
            .on_click(cx.listener(|this, _, _, cx| this.import_theme(cx)));
        let scheme = page::card(t)
            .child(div().relative().p(px(space::LG)).child(cards).child(self.anchor("appearance")))
            .child(self.srow(
                t,
                false,
                "theme_light",
                "Light theme",
                "Used when the color scheme is light.",
                light_select,
            ))
            .child(self.srow(t, false, "theme_dark", "Dark theme", "Used when the color scheme is dark.", dark_select))
            .child(self.srow(t, false, "theme_import", "Import VS Code theme", import_note, import_button))
            .child(self.srow(
                t,
                false,
                "accent",
                "Accent color",
                "Buttons, focus rings, selections and badges.",
                swatches,
            ));

        let surface_select = ctl::select::<Self>("set-surface", "Window material", |s| &mut s.sel_surface)
            .options(
                Surface::ALL.iter().map(|s| {
                    SelectOption::new(s.label()).detail(match s {
                        Surface::Glass => "Frosted",
                        Surface::Acrylic => "Denser frost",
                        Surface::Normal => "Opaque",
                    })
                }),
                Surface::ALL.iter().position(|s| *s == surface).unwrap_or(0),
            )
            .width(150.0)
            .menu_width(220.0)
            .on_select(|this, i, _, cx| this.set("surface", Surface::ALL[i].id(), cx))
            .render(t, &self.sel_surface, cx);
        let effect_select = ctl::select::<Self>("set-effect", "Background effect", |s| &mut s.sel_effect)
            .options(
                Effect::ALL.iter().map(|e| SelectOption::new(e.label())),
                Effect::ALL.iter().position(|e| *e == effect).unwrap_or(0),
            )
            .width(150.0)
            .on_select(|this, i, _, cx| this.set("wallpaper_effect", Effect::ALL[i].id(), cx))
            .render(t, &self.sel_effect, cx);
        let wall_name =
            wall.as_ref().and_then(|w| std::path::Path::new(w).file_name().map(|n| n.to_string_lossy().to_string()));
        // A small preview of the chosen image with its effect applied.
        let (thumb_w, thumb_h) = crate::wallpaper::PREVIEW_SIZE;
        let thumb = wall.as_ref().map(|w| {
            let image = crate::wallpaper::preview(
                std::path::Path::new(w),
                effect,
                !t.appearance.is_dark(),
                window.scale_factor(),
                cx,
            );
            div()
                .id("wall-preview")
                .flex_none()
                .w(px(thumb_w))
                .h(px(thumb_h))
                .rounded(px(design::radius::RADIUS))
                .overflow_hidden()
                .border_1()
                .border_color(t.border)
                .bg(t.wash(0.06))
                .cursor_pointer()
                .tooltip(ui::tip("Replace image"))
                .on_click(cx.listener(|this, _, _, cx| this.pick_wallpaper(cx)))
                .child(match image {
                    Some(image) => gpui::StyledImage::object_fit(gpui::img(image), gpui::ObjectFit::Cover)
                        .size_full()
                        .into_any_element(),
                    None => page::skeleton(t, gpui::relative(1.0), thumb_h),
                })
        });
        let wall_control = div().flex().items_center().gap(px(10.0)).children(thumb).child(
            div()
                .flex()
                .flex_col()
                .items_end()
                .gap(px(6.0))
                .child(
                    ctl::button_sm(
                        t,
                        "wall-pick",
                        if wall.is_some() { "Replace…" } else { "Choose image…" },
                        Tone::Filled,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.pick_wallpaper(cx))),
                )
                .when(wall.is_some(), |d| {
                    d.child(
                        ctl::button_sm(t, "wall-clear", "Remove", Tone::Quiet)
                            .on_click(cx.listener(|this, _, _, cx| this.set("wallpaper_path", "", cx))),
                    )
                }),
        );
        let has_wall = wall.is_some();
        let material = page::card(t)
            .child(self.srow(
                t,
                true,
                "surface",
                "Window material",
                match surface {
                    Surface::Glass => "Frosted glass over the blurred desktop.",
                    Surface::Acrylic => "Denser frost: the desktop reads as colour, not shapes.",
                    Surface::Normal => "Solid surfaces, no blur.",
                },
                surface_select,
            ))
            .child(self.srow(
                t,
                false,
                "wallpaper_path",
                "New-thread background",
                wall_name.clone().unwrap_or_else(|| "No image selected".into()),
                wall_control,
            ))
            .child(self.srow(
                t,
                false,
                "wallpaper_effect",
                "Background effect",
                if has_wall {
                    "How the background is drawn behind the composer."
                } else {
                    "Choose a background image first."
                },
                if has_wall { effect_select } else { div().opacity(0.45).child(effect_select).into_any_element() },
            ))
            .child(self.srow(
                t,
                false,
                "wallpaper_animate",
                "Animate background",
                if has_wall { effect.motion_hint() } else { "Choose a background image to animate it." },
                if has_wall {
                    ctl::on_toggle(ctl::toggle(t, "set-wall-animate", animate), cx, move |this, _, cx| {
                        this.set_flag("wallpaper_animate", !animate, cx)
                    })
                    .into_any_element()
                } else {
                    ctl::disabled(ctl::toggle(t, "set-wall-animate", animate), "Choose a background image first")
                        .into_any_element()
                },
            ));

        // Motion.
        let preference = motion::preference(cx);
        let system = motion::system_reduces_motion(cx);
        let pause = motion::pause_in_background(cx);
        let reduce_select = ctl::select::<Self>("set-reduce-motion", "Reduce motion", |s| &mut s.sel_reduce)
            .options(
                ReduceMotion::ALL.iter().map(|m| {
                    let o = SelectOption::new(m.label());
                    if *m == ReduceMotion::System {
                        o.detail(if system { "Reduced" } else { "Full motion" })
                    } else {
                        o
                    }
                }),
                ReduceMotion::ALL.iter().position(|m| *m == preference).unwrap_or(0),
            )
            .width(130.0)
            .menu_width(200.0)
            .on_select(|this, i, _, cx| {
                motion::set_preference(ReduceMotion::ALL[i], &this.engine.store, cx);
                cx.notify();
            })
            .render(t, &self.sel_reduce, cx);
        let motion_card = page::card(t)
            .child(self.srow(
                t,
                true,
                "reduce_motion",
                "Reduce motion",
                match preference {
                    ReduceMotion::System if system => "Following your system: motion is reduced.",
                    ReduceMotion::System => "Following your system setting.",
                    ReduceMotion::On => "Transitions snap, indicators breathe slowly, background art holds still.",
                    ReduceMotion::Off => "Full motion, whatever the system says.",
                },
                reduce_select,
            ))
            .child(self.srow(
                t,
                false,
                "pause_animations_in_background",
                "Pause animations in background",
                if pause {
                    "Everything below holds still while another window is focused."
                } else {
                    "Off: everything below keeps playing behind other windows (minimized, nothing draws)."
                },
                ctl::on_toggle(ctl::toggle(t, "set-pause-bg", pause), cx, move |this, _, cx| {
                    motion::set_pause_in_background(!pause, &this.engine.store, cx);
                    cx.notify();
                }),
            ))
            .child(motion_inventory(t, cx.reduce_motion(), has_wall, animate, effect));

        // Fonts and layout.
        let size_select = |id: &'static str,
                           label: &'static str,
                           key: &'static str,
                           sizes: &'static [f32],
                           current: f32,
                           default: f32,
                           reach: fn(&mut Self) -> &mut SelectState,
                           state: &SelectState,
                           cx: &mut Context<Self>| {
            ctl::select::<Self>(id, label, reach)
                .options(
                    sizes.iter().map(|s| {
                        let o = SelectOption::new(format!("{} px", design::format_size(*s)));
                        if (*s - default).abs() < 0.01 { o.detail("Default") } else { o }
                    }),
                    sizes.iter().position(|s| (*s - current).abs() < 0.01).unwrap_or(0),
                )
                .width(110.0)
                .menu_width(170.0)
                .on_select(move |this, i, _, cx| {
                    design::set_preference(&this.engine.store, key, sizes[i], cx);
                    cx.notify();
                })
                .render(&Theme::of(cx).clone(), state, cx)
        };
        let ui_size = size_select(
            "set-ui-size",
            "Interface font size",
            "ui_font_size",
            &design::UI_SIZES,
            design::ui_size(),
            design::BASE,
            |s| &mut s.sel_ui_size,
            &self.sel_ui_size,
            cx,
        );
        let code_size = size_select(
            "set-code-size",
            "Code and diff font size",
            "code_font_size",
            &design::MONO_SIZES,
            design::code(),
            design::CODE_DEFAULT,
            |s| &mut s.sel_code_size,
            &self.sel_code_size,
            cx,
        );
        let term_size = size_select(
            "set-term-size",
            "Terminal font size",
            "terminal_font_size",
            &design::MONO_SIZES,
            design::terminal(),
            design::TERMINAL_DEFAULT,
            |s| &mut s.sel_term_size,
            &self.sel_term_size,
            cx,
        );
        let scrollback = {
            let current = design::scrollback();
            let choices = &design::SCROLLBACK_CHOICES;
            let selected = choices.iter().position(|n| *n >= current).unwrap_or(choices.len() - 1);
            ctl::select::<Self>("set-scrollback", "Scrollback lines", |s| &mut s.sel_scrollback)
                .options(
                    choices.iter().map(|n| {
                        let o = SelectOption::new(format!("{n} lines"));
                        if *n == design::SCROLLBACK_DEFAULT { o.detail("Default") } else { o }
                    }),
                    selected,
                )
                .width(130.0)
                .menu_width(190.0)
                .on_select(move |this, i, _, cx| {
                    design::set_scrollback(&this.engine.store, choices[i]);
                    cx.notify();
                })
                .render(&Theme::of(cx).clone(), &self.sel_scrollback, cx)
        };
        let width = self.width_drag.unwrap_or_else(design::conversation_width);
        let width_slider = ctl::slider::<Self>(
            t,
            "set-conversation-width",
            width,
            (design::CONVERSATION_MIN, design::CONVERSATION_MAX, design::CONVERSATION_STEP),
            180.0,
            &self.width_slider,
            |s| &mut s.width_slider,
            |this, v, commit, _, cx| {
                if commit {
                    this.width_drag = None;
                    design::set_preference(&this.engine.store, "conversation_width", v, cx);
                } else {
                    this.width_drag = Some(v);
                }
                cx.notify();
            },
            cx,
        );
        let width_control = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(width_slider)
            .child(div().w(px(56.0)).text_size(ty::META).text_color(t.text_muted).child(format!("{width:.0} px")))
            .child(ctl::button_sm(t, "set-width-reset", "Reset", Tone::Quiet).on_click(cx.listener(
                |this, _, _, cx| {
                    design::set_preference(&this.engine.store, "conversation_width", design::CONVERSATION_DEFAULT, cx);
                    cx.notify();
                },
            )));
        let fonts = page::card(t)
            .child(self.srow(
                t,
                true,
                "ui_font_size",
                "Interface font size",
                "Menus, labels and conversations.",
                ui_size,
            ))
            .child(self.srow(
                t,
                false,
                "code_font_size",
                "Code & diff font size",
                "Code blocks, diffs and file previews.",
                code_size,
            ))
            .child(self.srow(
                t,
                false,
                "terminal_font_size",
                "Terminal font size",
                "Terminals and agents' own terminal views.",
                term_size,
            ))
            .child(self.srow(
                t,
                false,
                "terminal_scrollback",
                "Scrollback lines",
                "History each terminal keeps. Applies to terminals opened, restarted or cleared from now on.",
                scrollback,
            ))
            .child(self.srow(
                t,
                false,
                "conversation_width",
                "Conversation width",
                "The widest the conversation and composer get.",
                width_control,
            ))
            .child(self.home_input_row(t, cx));

        div()
            .flex()
            .flex_col()
            .child(page::section(t, "Color scheme", scheme))
            .child(page::section(t, "Material and background", material))
            .child(page::section(t, "Motion", motion_card))
            .child(page::section(t, "Fonts and layout", fonts))
            .into_any_element()
    }

    fn home_input_row(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let dock = crate::composer::home_input_is_dock(self.get(crate::composer::HOME_INPUT).as_deref());
        let select = ctl::select::<Self>("set-home-input", "New-thread input", |s| &mut s.sel_home_input)
            .options(
                [
                    SelectOption::new("Center").detail("Current"),
                    SelectOption::new("Bottom").detail("Project, permissions, usage"),
                ],
                if dock { 1 } else { 0 },
            )
            .width(140.0)
            .menu_width(240.0)
            .on_select(|this, i, _, cx| {
                this.set(crate::composer::HOME_INPUT, if i == 1 { "dock" } else { "center" }, cx);
            })
            .render(t, &self.sel_home_input, cx);
        self.srow(
            t,
            false,
            "home_input",
            "New-thread input",
            if dock {
                "The composer sits at the bottom, with folder and device in one menu, permissions, and usage meters."
            } else {
                "The composer stays in the middle of a new thread."
            },
            select,
        )
    }

    /// The theme picker for one appearance: Dove's look first, then the
    /// built-ins, then imports, each with its accent as a swatch.
    fn theme_select(
        &self,
        appearance: Appearance,
        reach: fn(&mut Self) -> &mut SelectState,
        state: &SelectState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = crate::themes::key_for(appearance);
        let choices = crate::themes::choices(appearance);
        let current = self.get(key);
        let selected = current.and_then(|id| choices.iter().position(|c| c.id == id)).unwrap_or(0);
        let ids: Vec<String> = choices.iter().map(|c| c.id.clone()).collect();
        let dove_accent = Theme::new(appearance, Accent::default(), Surface::Normal).accent;
        let label = if appearance.is_dark() { "Dark theme" } else { "Light theme" };
        let t = Theme::of(cx).clone();
        ctl::select::<Self>(format!("set-{key}"), label, reach)
            .options(
                choices.iter().map(|c| {
                    let o = SelectOption::new(c.name.clone()).swatch(c.accent.map_or(dove_accent, |a| a.hsla()));
                    if c.imported {
                        o.detail("Imported")
                    } else if c.id == crate::themes::DOVE_ID {
                        o.detail("Default")
                    } else {
                        o
                    }
                }),
                selected,
            )
            .width(190.0)
            .menu_width(260.0)
            .on_select(move |this, i, _, cx| {
                if let Some(id) = ids.get(i) {
                    this.set(key, id, cx);
                }
            })
            .render(&t, state, cx)
    }

    fn notifications(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let on = self.flag(crate::notify::NOTIFICATIONS, true);
        let sound = self.flag(crate::notify::SOUND, false);
        let keep = self.flag(crate::notify::KEEP_RUNNING, false);
        let desktop = page::card(t).child(self.srow(
            t,
            true,
            "notifications",
            "Desktop notifications",
            "When an agent needs you, finishes or fails while Dove is in the background.",
            ctl::on_toggle(ctl::toggle(t, "set-notify", on), cx, move |this, _, cx| {
                this.set_flag(crate::notify::NOTIFICATIONS, !on, cx)
            }),
        ));
        let sounds = page::card(t).child(self.srow(
            t,
            true,
            "notification_sound",
            "Completion sound",
            "A short chime when a background agent finishes.",
            ctl::on_toggle(ctl::toggle(t, "set-sound", sound), cx, move |this, _, cx| {
                this.set_flag(crate::notify::SOUND, !sound, cx)
            }),
        ));
        let background = page::card(t).child(self.srow(
            t,
            true,
            "keep_running",
            "Keep running in background",
            if crate::notify::tray_supported() {
                "Closing the window keeps Dove and its automations running in the system tray."
            } else {
                "Closing the window minimizes Dove so automations keep running."
            },
            ctl::on_toggle(ctl::toggle(t, "set-keep", keep), cx, move |this, window, cx| {
                crate::notify::set_keep_running(&this.engine, !keep, window, cx);
                cx.notify();
            }),
        ));
        div()
            .flex()
            .flex_col()
            .child(page::section(t, "Desktop", desktop))
            .child(page::section(t, "Sounds", sounds))
            .child(page::section(t, "Background", background))
            .into_any_element()
    }

    fn voice(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(model) = VoiceModel::global(cx) else { return div().into_any_element() };
        let state = model.read(cx).state.clone();
        let size_mb = VoiceModel::download_size() / 1_000_000;
        let (status, control): (SharedString, AnyElement) = match state {
            InstallState::Missing => (
                "Not installed. Runs entirely on this device once downloaded.".into(),
                ctl::button_sm(t, "voice-install", format!("Download ({size_mb} MB)"), Tone::Solid)
                    .on_click(cx.listener(|_, _, _, cx| {
                        if let Some(m) = VoiceModel::global(cx) {
                            m.update(cx, |m, cx| m.install(cx));
                        }
                    }))
                    .into_any_element(),
            ),
            InstallState::Downloading { done, total } => {
                let frac = done as f32 / total.max(1) as f32;
                (
                    format!("Downloading · {:.0} of {:.0} MB", done as f64 / 1e6, total as f64 / 1e6).into(),
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .w(px(140.0))
                                .h(px(5.0))
                                .rounded_full()
                                .bg(t.wash(0.10))
                                .child(div().h_full().w(px(140.0 * frac)).rounded_full().bg(t.accent)),
                        )
                        .child(
                            div().text_size(ty::META).text_color(t.text_muted).child(format!("{:.0}%", frac * 100.0)),
                        )
                        .child(ctl::button_sm(t, "voice-cancel", "Cancel", Tone::Quiet).on_click(cx.listener(
                            |_, _, _, cx| {
                                if let Some(m) = VoiceModel::global(cx) {
                                    m.update(cx, |m, cx| m.cancel(cx));
                                }
                            },
                        )))
                        .into_any_element(),
                )
            }
            InstallState::Installed => (
                "NVIDIA Parakeet TDT 0.6B v3, verified file by file.".into(),
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(page::badge_tone("Installed", t.success))
                    .child(ctl::button_sm(t, "voice-remove", "Remove", Tone::Quiet).on_click(cx.listener(
                        |_, _, _, cx| {
                            if let Some(m) = VoiceModel::global(cx) {
                                m.update(cx, |m, cx| m.remove(cx));
                            }
                        },
                    )))
                    .into_any_element(),
            ),
            InstallState::Failed(e) => (
                e.into(),
                ctl::button_sm(t, "voice-retry", "Try again", Tone::Filled)
                    .on_click(cx.listener(|_, _, _, cx| {
                        if let Some(m) = VoiceModel::global(cx) {
                            m.update(cx, |m, cx| {
                                m.state = InstallState::Missing;
                                m.install(cx);
                            });
                        }
                    }))
                    .into_any_element(),
            ),
        };
        let current = self.get("voice_device").unwrap_or_default();
        let mut names: Vec<SelectOption> = vec![SelectOption::new("System default")];
        names.extend(self.devices.iter().map(|(_, name)| SelectOption::new(name.clone())));
        let selected = if current.is_empty() {
            0
        } else {
            self.devices.iter().position(|(id, _)| *id == current).map(|i| i + 1).unwrap_or(0)
        };
        let ids: Vec<String> =
            std::iter::once(String::new()).chain(self.devices.iter().map(|(id, _)| id.clone())).collect();
        let mic = ctl::select::<Self>("set-mic", "Microphone", |s| &mut s.sel_mic)
            .options(names, selected)
            .width(220.0)
            .menu_width(300.0)
            .on_select(move |this, i, _, cx| {
                if let Some(id) = ids.get(i) {
                    this.set("voice_device", id, cx);
                }
            })
            .render(t, &self.sel_mic, cx);
        let card = page::card(t)
            .child(self.srow(t, true, "voice_model", "Dictation model", status, control))
            .child(self.srow(t, false, "voice_device", "Microphone", "", mic))
            .child(
                page::row(t, false).child(page::row_text(
                    t,
                    "How to dictate",
                    "Hold the mic in the composer or a terminal to talk, or click it to toggle. Text is inserted, never sent.",
                )),
            );
        div()
            .flex()
            .flex_col()
            .child(page::section(t, "Dictation", card))
            .child(self.voice_mode(t, cx))
            .into_any_element()
    }

    /// Settings › Voice › Voice Mode: the hands-free call.
    fn voice_mode(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity().downgrade();
        let push = self.get("voice_mode").as_deref() == Some("push");
        let listening = ctl::segmented(
            t,
            "set-voice-mode",
            vec![ctl::Segment::new("Always listening"), ctl::Segment::new("Push to talk")],
            usize::from(push),
            {
                let this = this.clone();
                move |i, _, cx| {
                    let _ = this.update(cx, |p, cx| p.set("voice_mode", if i == 1 { "push" } else { "always" }, cx));
                }
            },
        );
        let rate: f32 = self.get("voice_rate").and_then(|r| r.parse().ok()).unwrap_or(1.0);
        let rates = ctl::segmented(
            t,
            "set-voice-rate",
            vec![ctl::Segment::new("Slower"), ctl::Segment::new("Normal"), ctl::Segment::new("Faster")],
            if rate < 0.95 {
                0
            } else if rate > 1.1 {
                2
            } else {
                1
            },
            move |i, _, cx| {
                let _ = this.update(cx, |p, cx| p.set("voice_rate", ["0.85", "1.0", "1.25"][i.min(2)], cx));
            },
        );
        let current = self.get("voice_name").unwrap_or_default();
        let mut names: Vec<SelectOption> = vec![SelectOption::new("System default")];
        names.extend(self.voices.iter().map(|v| SelectOption::new(v.clone())));
        let selected = self.voices.iter().position(|v| *v == current).map(|i| i + 1).unwrap_or(0);
        let voices = self.voices.clone();
        let voice = ctl::select::<Self>("set-voice-name", "Voice", |s| &mut s.sel_voice)
            .options(names, selected)
            .width(220.0)
            .menu_width(300.0)
            .on_select(move |this, i, _, cx| {
                let v = if i == 0 { String::new() } else { voices.get(i - 1).cloned().unwrap_or_default() };
                this.set("voice_name", &v, cx);
            })
            .render(t, &self.sel_voice, cx);
        let echo = self.flag("voice_echo_cancel", true);
        let announce = self.flag("voice_announce", true);
        let earcons = self.flag("voice_earcons", true);
        let cloud = self.flag("voice_cloud", false);
        let cloud_tts = self.flag("voice_cloud_tts", false);
        let shortcut = crate::keymap::shortcuts()
            .into_iter()
            .find(|s| s.id == "voice_call")
            .map(|s| crate::keymap::pretty(&crate::keymap::binding(&self.engine.store, &s)))
            .unwrap_or_default();
        let call = page::card(t)
            .child(self.srow(
                t,
                true,
                "voice_call_mode",
                "Listening",
                format!(
                    "Start a call with the phone button beside the mic, or {shortcut}. Push to talk: hold the orb (or Space) to speak."
                ),
                listening,
            ))
            .child({
                let global = self.flag("voice_global_hotkey", true);
                let detail: SharedString = match (global, crate::hotkey::last_error()) {
                    (true, Some(e)) => format!("The shortcut couldn't be registered: {e}. Pick another in Shortcuts.").into(),
                    (true, None) if cfg!(windows) => format!("{shortcut} starts or ends a call from any app.").into(),
                    (true, None) => "Available on Windows; elsewhere the shortcut works inside Dove.".into(),
                    (false, _) => "Off: the shortcut works only while Dove is in front.".into(),
                };
                self.srow(
                    t,
                    false,
                    "voice_global_hotkey",
                    "Shortcut outside Dove",
                    detail,
                    ctl::on_toggle(ctl::toggle(t, "set-voice-global", global), cx, move |this, _, cx| {
                        this.set_flag("voice_global_hotkey", !global, cx);
                        crate::hotkey::apply(&this.engine.store);
                        cx.notify();
                    }),
                )
            })
            .child(self.srow(t, false, "voice_name", "Voice", "Your operating system's voices. No download.", voice))
            .child(self.srow(t, false, "voice_rate", "Speaking rate", "Or say \"talk faster\" during a call.", rates))
            .child(self.srow(
                t,
                false,
                "voice_echo_cancel",
                "Echo cancellation",
                "Lets you use speakers instead of headphones, and talk over Dove to interrupt it.",
                ctl::on_toggle(ctl::toggle(t, "set-voice-echo", echo), cx, move |this, _, cx| {
                    this.set_flag("voice_echo_cancel", !echo, cx)
                }),
            ))
            .child(self.srow(
                t,
                false,
                "voice_announce",
                "Announce thread updates",
                "Say when a thread finishes, fails or needs your approval.",
                ctl::on_toggle(ctl::toggle(t, "set-voice-announce", announce), cx, move |this, _, cx| {
                    this.set_flag("voice_announce", !announce, cx)
                }),
            ))
            .child(self.srow(
                t,
                false,
                "voice_earcons",
                "Sounds",
                "A soft chime when the call starts listening and when Dove has heard you.",
                ctl::on_toggle(ctl::toggle(t, "set-voice-earcons", earcons), cx, move |this, _, cx| {
                    this.set_flag("voice_earcons", !earcons, cx)
                }),
            ))
            .child(
                page::row(t, false).child(page::row_text(
                    t,
                    "What you can say",
                    "\"What's happening?\", \"Stop\", \"Approve\" or \"Deny\", \"Talk to the tests thread\", \"Tell Codex to also run clippy\", \"Start a Claude task on the login page in a new worktree\", \"Take me back\", \"Show the diff\", \"End call\". Anything else goes to the thread you're talking to. Voice never approves deletions, force pushes or work outside the thread's folder: those need your click.",
                )),
            );
        let models = page::card(t)
            .child(self.voice_stt_row(t, cx))
            .child(self.voice_model_row(t, &dove_voice::models::SILERO, cx))
            .child(self.voice_model_row(t, &dove_voice::models::SMART_TURN, cx));
        let cloud_card = page::card(t)
            .child(self.srow(
                t,
                true,
                "voice_cloud",
                "Cloud speech recognition",
                if cloud && self.get("voice_cloud_stt_provider").as_deref() == Some("deepgram") {
                    "On: your spoken audio is uploaded to Deepgram for transcription."
                } else if cloud {
                    "On: your spoken audio is uploaded to the endpoint below for transcription."
                } else {
                    "Off: audio never leaves this computer."
                },
                ctl::on_toggle(ctl::toggle(t, "set-voice-cloud", cloud), cx, move |this, _, cx| {
                    this.set_flag("voice_cloud", !cloud, cx)
                }),
            ))
            .child(self.srow(
                t,
                false,
                "voice_cloud_tts",
                "Cloud voice",
                if cloud_tts && self.get("voice_cloud_tts_provider").as_deref() == Some("elevenlabs") {
                    "On: what Dove says (including agents' replies) is uploaded to ElevenLabs for speech."
                } else if cloud_tts {
                    "On: what Dove says (including agents' replies) is uploaded to the endpoint for speech."
                } else {
                    "Off: Dove speaks with your system's voice."
                },
                ctl::on_toggle(ctl::toggle(t, "set-voice-cloud-tts", cloud_tts), cx, move |this, _, cx| {
                    this.set_flag("voice_cloud_tts", !cloud_tts, cx)
                }),
            ))
            .child({
                let this = cx.entity().downgrade();
                let stt_provider = usize::from(self.get("voice_cloud_stt_provider").as_deref() == Some("deepgram"));
                self.srow(
                    t,
                    false,
                    "voice_cloud_stt_provider",
                    "Speech recognition service",
                    "OpenAI-compatible uses the endpoint below and DOVE_VOICE_API_KEY. Deepgram uses DEEPGRAM_API_KEY.",
                    ctl::segmented(
                        t,
                        "set-voice-stt-provider",
                        vec![ctl::Segment::new("OpenAI-compatible"), ctl::Segment::new("Deepgram")],
                        stt_provider,
                        move |i, _, cx| {
                            let _ = this.update(cx, |p, cx| {
                                p.set("voice_cloud_stt_provider", if i == 1 { "deepgram" } else { "openai" }, cx)
                            });
                        },
                    ),
                )
            })
            .child({
                let this = cx.entity().downgrade();
                let tts_provider = usize::from(self.get("voice_cloud_tts_provider").as_deref() == Some("elevenlabs"));
                self.srow(
                    t,
                    false,
                    "voice_cloud_tts_provider",
                    "Voice service",
                    "OpenAI-compatible uses the endpoint below. ElevenLabs uses ELEVENLABS_API_KEY.",
                    ctl::segmented(
                        t,
                        "set-voice-tts-provider",
                        vec![ctl::Segment::new("OpenAI-compatible"), ctl::Segment::new("ElevenLabs")],
                        tts_provider,
                        move |i, _, cx| {
                            let _ = this.update(cx, |p, cx| {
                                p.set("voice_cloud_tts_provider", if i == 1 { "elevenlabs" } else { "openai" }, cx)
                            });
                        },
                    ),
                )
            })
            .child(self.srow(
                t,
                false,
                "voice_cloud_url",
                "Speech endpoint",
                "An OpenAI-compatible /v1 URL (OpenAI, Groq, a local whisper server). The key is read from DOVE_VOICE_API_KEY.",
                div().w(px(260.0)).child(gpui_base::input::Input::new(&self.voice_cloud_url)),
            ))
            .child({
                let this = cx.entity().downgrade();
                let has_url = self.get("voice_router_url").is_some_and(|u| !u.trim().is_empty());
                let mode = self.get("voice_router").unwrap_or_else(|| if has_url { "endpoint".into() } else { "off".into() });
                let selected = match mode.as_str() {
                    "agent" => 1,
                    "endpoint" => 2,
                    _ => 0,
                };
                self.srow(
                    t,
                    false,
                    "voice_router",
                    "Command router",
                    match selected {
                        1 => "Requests Dove's rules don't recognise are routed by a tool-less run of the current thread's agent on its cheapest model (a few seconds). The request and thread titles go to that agent.",
                        2 => "Requests Dove's rules don't recognise are routed by the endpoint below. The request and thread titles are sent to it.",
                        _ => "Off: requests Dove's rules don't recognise go to the thread you're talking to, as you said them.",
                    },
                    ctl::segmented(
                        t,
                        "set-voice-router",
                        vec![ctl::Segment::new("Off"), ctl::Segment::new("An agent"), ctl::Segment::new("Endpoint")],
                        selected,
                        move |i, _, cx| {
                            let _ = this.update(cx, |p, cx| p.set("voice_router", ["off", "agent", "endpoint"][i.min(2)], cx));
                        },
                    ),
                )
            })
            .child(self.srow(
                t,
                false,
                "voice_router_url",
                "Router endpoint",
                "For the Endpoint router: an OpenAI-compatible /v1 URL (a local llama.cpp server works). The key is read from DOVE_VOICE_API_KEY.",
                div().w(px(260.0)).child(gpui_base::input::Input::new(&self.voice_router_url)),
            ));
        div()
            .flex()
            .flex_col()
            .child(page::section(t, "Voice Mode", call))
            .child(page::section(t, "Voice Mode models", models))
            .child(page::section(t, "Cloud (optional)", cloud_card))
            .into_any_element()
    }

    /// Which local recognizer a call uses, and the Moonshine download.
    fn voice_stt_row(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        use dove_voice::models::MoonshineSize;
        let root = self.engine.store.root().to_path_buf();
        let moon = dove_voice::stt::Moonshine::installed(&root, MoonshineSize::Base);
        let parakeet = VoiceModel::global(cx).is_some_and(|m| m.read(cx).state == InstallState::Installed);
        let choice = self.get("voice_stt").unwrap_or_default();
        let use_parakeet = choice == "parakeet" && parakeet;
        let state = self.voice_dl.get("moonshine").cloned();
        let size_mb: u64 = MoonshineSize::Base.files().iter().map(|m| m.size).sum::<u64>() / 1_000_000;
        let detail = format!(
            "Moonshine base: fast, about 160 MB in use, {size_mb} MB download, MIT. Parakeet (the dictation model above): more accurate, about 700 MB in use, CC-BY-4.0.{}",
            match &state {
                Some(Some(e)) => format!(" {e}"),
                _ => String::new(),
            }
        );
        let this = cx.entity().downgrade();
        let control: AnyElement = if matches!(state, Some(None)) {
            page::badge(t, "Downloading…").into_any_element()
        } else if !moon {
            ctl::button_sm(t, "voice-dl-moonshine", format!("Download Moonshine ({size_mb} MB)"), Tone::Solid)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.voice_dl.insert("moonshine", None);
                    let root = this.engine.store.root().to_path_buf();
                    let work = cx.background_spawn(async move {
                        for m in MoonshineSize::Base.files() {
                            if !dove_voice::models::installed(&root, &m) {
                                dove_voice::models::download(
                                    &root,
                                    &m,
                                    &std::sync::atomic::AtomicBool::new(false),
                                    |_| {},
                                )
                                .map_err(|e| format!("{e:#}"))?;
                            }
                        }
                        Ok::<(), String>(())
                    });
                    cx.spawn(async move |this, cx| {
                        let result = work.await;
                        let _ = this.update(cx, |p, cx| {
                            match result {
                                Ok(()) => {
                                    p.voice_dl.remove("moonshine");
                                }
                                Err(e) => {
                                    p.voice_dl.insert("moonshine", Some(e));
                                }
                            }
                            cx.notify();
                        });
                    })
                    .detach();
                    cx.notify();
                }))
                .into_any_element()
        } else {
            ctl::segmented(
                t,
                "set-voice-stt",
                vec![ctl::Segment::new("Moonshine (fast)"), ctl::Segment::new("Parakeet (accurate)")],
                usize::from(use_parakeet),
                move |i, _, cx| {
                    let _ = this
                        .update(cx, |p, cx| p.set("voice_stt", if i == 1 { "parakeet" } else { "moonshine-base" }, cx));
                },
            )
        };
        self.srow(t, true, "voice_call_stt", "Speech recognition", detail, control)
    }

    /// One downloadable Voice Mode model: what it does, size, licence.
    fn voice_model_row(
        &self,
        t: &Theme,
        m: &'static dove_voice::models::ModelFile,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = self.engine.store.root().to_path_buf();
        let installed = dove_voice::models::installed(&root, m);
        let state = self.voice_dl.get(m.id).cloned();
        let size = format!("{:.1} MB", m.size as f64 / 1e6);
        let detail = format!(
            "{}. {} · {}{}",
            m.purpose,
            size,
            m.licence,
            match &state {
                Some(Some(e)) => format!(" · {e}"),
                _ => String::new(),
            }
        );
        let control = match (installed, &state) {
            (_, Some(None)) => page::badge(t, "Downloading…").into_any_element(),
            (true, _) => div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(page::badge_tone("Installed", t.success))
                .child(
                    ctl::button_sm(t, SharedString::from(format!("voice-rm-{}", m.id)), "Remove", Tone::Quiet)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            dove_voice::pipeline::unload();
                            dove_voice::models::remove(this.engine.store.root(), m);
                            cx.notify();
                        })),
                )
                .into_any_element(),
            (false, _) => ctl::button_sm(
                t,
                SharedString::from(format!("voice-dl-{}", m.id)),
                format!("Download ({size})"),
                Tone::Solid,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.voice_dl.insert(m.id, None);
                let root = this.engine.store.root().to_path_buf();
                let work = cx.background_spawn(async move {
                    dove_voice::models::download(&root, m, &std::sync::atomic::AtomicBool::new(false), |_| {})
                        .map_err(|e| format!("{e:#}"))
                });
                cx.spawn(async move |this, cx| {
                    let result = work.await;
                    let _ = this.update(cx, |p, cx| {
                        match result {
                            Ok(()) => {
                                p.voice_dl.remove(m.id);
                            }
                            Err(e) => {
                                p.voice_dl.insert(m.id, Some(e));
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
            }))
            .into_any_element(),
        };
        let name = match m.id {
            "silero-vad" => "Voice activity (Silero VAD)",
            "smart-turn" => "Turn detection (Smart Turn v3.2)",
            other => other,
        };
        self.srow(t, false, "voice_call_models", name, detail, control)
    }

    fn shortcuts_action(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let store = self.engine.store.clone();
        let any_custom = crate::keymap::shortcuts().iter().any(|sc| crate::keymap::binding(&store, sc) != sc.default);
        Some(if any_custom {
            ctl::button_sm(t, "keys-restore", "Restore defaults", Tone::Quiet)
                .child(icon(icons::RESTART).size(px(13.0)))
                .on_click(cx.listener(|this, _, _, cx| {
                    for sc in crate::keymap::shortcuts() {
                        crate::keymap::rebind(&this.engine.store, sc.id, sc.default, cx);
                    }
                    cx.notify();
                }))
                .into_any_element()
        } else {
            page::badge(t, "Default bindings").into_any_element()
        })
    }

    fn shortcuts(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let store = self.engine.store.clone();
        let all = crate::keymap::shortcuts();
        let groups: [(&str, &[&str]); 4] = [
            ("Navigation", &["palette", "back", "forward", "threads", "kanban", "settings"]),
            (
                "Threads",
                &["new_thread", "close_tab", "next_tab", "prev_tab", "rename", "focus_composer", "toggle_terminal"],
            ),
            ("Panels", &["toggle_sidebar", "toggle_panel"]),
            ("App", &["add_project", "voice_call", "quit"]),
        ];
        let mut out = div()
            .flex()
            .flex_col()
            .child(page::page_subtitle(t, "Click a binding, then press the new keys. Escape cancels."));
        for (label, ids) in groups {
            let mut card = page::card(t);
            for (i, id) in ids.iter().enumerate() {
                let Some(s) = all.iter().find(|s| s.id == *id) else { continue };
                let key = crate::keymap::binding(&store, s);
                let recording = self.recording == Some(s.id);
                let custom = key != s.default;
                let (sid, default) = (s.id, s.default);
                let chip = div()
                    .id(SharedString::from(format!("rec-{sid}")))
                    .min_w(px(96.0))
                    .h(px(30.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(design::radius::RADIUS))
                    .bg(if recording { t.accent.opacity(0.12) } else { t.wash(0.06) })
                    .border_1()
                    .border_color(if recording { t.accent } else { gpui::transparent_black() })
                    .font_family(t.font_mono.clone())
                    .text_size(ty::META)
                    .text_color(if recording { t.accent } else { t.text })
                    .cursor_pointer()
                    .hover(|s| s.bg(t.wash(0.10)))
                    .role(gpui::Role::Button)
                    .aria_label(format!("{}: {}", s.label, crate::keymap::pretty(&key)))
                    .child(if recording { "Press keys…".to_string() } else { crate::keymap::pretty(&key) })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.recording = Some(sid);
                        window.focus(&this.focus, cx);
                        cx.notify();
                    }));
                let control = div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .when(custom && !recording, |d| {
                        d.child(
                            ctl::button_sm(t, format!("reset-{sid}"), "Reset", Tone::Quiet)
                                .tooltip(ui::tip(format!("Back to {}", crate::keymap::pretty(default))))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    crate::keymap::rebind(&this.engine.store, sid, default, cx);
                                    cx.notify();
                                })),
                        )
                    })
                    .child(chip);
                card =
                    card.child(page::setting_row(t, i == 0, s.label, if custom { "Customized" } else { "" }, control));
            }
            out = out.child(page::section(t, label, card));
        }
        // Keys that are part of a surface rather than rebindable commands.
        let fixed: [(&str, String); 14] = [
            ("Go to thread tab 1–9", format!("{} 1–9", crate::keymap::pretty("secondary"))),
            ("Close menu or overlay", "Esc".into()),
            ("Send, steer or queue a message", "Enter".into()),
            ("Stop the agent (empty composer)", "Esc".into()),
            ("Paste an image", crate::keymap::pretty("secondary-v")),
            ("Previous / next prompt", format!("{} ↑ / ↓", crate::keymap::pretty("alt"))),
            ("Jump to top / latest", format!("{} Home / End", crate::keymap::pretty("secondary"))),
            ("Allow once / always / deny", "A / S / D".into()),
            ("Answer a question", "1–9".into()),
            ("Browser: address bar", crate::keymap::pretty("secondary-l")),
            ("Browser: new / close tab", format!("{} T / W", crate::keymap::pretty("secondary"))),
            ("Browser: back / forward", format!("{} [ / ]", crate::keymap::pretty("secondary"))),
            ("Browser: reload", format!("{} R, F5", crate::keymap::pretty("secondary"))),
            ("Terminal: copy / paste", "Ctrl Shift C / V".into()),
        ];
        let mut fixed_card = page::card(t);
        for (i, (label, keys)) in fixed.into_iter().enumerate() {
            fixed_card = fixed_card.child(page::setting_row(
                t,
                i == 0,
                label,
                "",
                div()
                    .px(px(10.0))
                    .h(px(26.0))
                    .flex()
                    .items_center()
                    .rounded(px(design::radius::RADIUS))
                    .bg(t.wash(0.04))
                    .font_family(t.font_mono.clone())
                    .text_size(ty::META)
                    .text_color(t.text_muted)
                    .child(keys),
            ));
        }
        out.child(page::section(t, "Built in", fixed_card)).into_any_element()
    }

    fn agents(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let installed = self.engine.installed();
        let supported: Vec<HarnessKind> = HarnessKind::ALL
            .iter()
            .copied()
            .filter(|k| *k != HarnessKind::Shell && dove_core::harness_supported(*k))
            .collect();
        let found_count = supported.iter().filter(|k| installed.iter().any(|i| i.kind == **k)).count();
        let mut card = page::card(t);
        for (i, kind) in supported.into_iter().enumerate() {
            let found = installed.iter().find(|x| x.kind == kind);
            let enabled = crate::pages::agent_enabled(&self.engine.store, kind);
            let key = format!("agent_disabled.{}", kind.id());
            let version = self
                .get(&format!("agent_version.{}", kind.id()))
                .and_then(|v| v.rsplit_once('|').map(|(_, ver)| ver.to_string()))
                .filter(|v| !v.is_empty());
            let meta: SharedString = match (found, version) {
                (Some(_), Some(v)) => format!("v{v}").into(),
                (Some(f), None) => f.program.display().to_string().into(),
                (None, _) => "Not installed".into(),
            };
            let control: AnyElement = if found.is_some() {
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .id(SharedString::from(format!("agent-more-{}", kind.id())))
                            .size(px(26.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(design::radius::RADIUS))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.wash(0.08)))
                            .tooltip(ui::tip("Sign-in, binary path and updates"))
                            .child(icon(icons::ALT_ARROW_RIGHT).size(px(14.0)).text_color(t.text_faint))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::OpenAgents))),
                    )
                    .child(ctl::on_toggle(
                        ctl::toggle(t, format!("set-agent-{}", kind.id()), enabled).tooltip(ui::tip(if enabled {
                            "Shown in the composer"
                        } else {
                            "Hidden from the composer"
                        })),
                        cx,
                        move |this, _, cx| this.set(&key, if enabled { "true" } else { "false" }, cx),
                    ))
                    .into_any_element()
            } else {
                page::badge(t, "Not installed").into_any_element()
            };
            card = card.child(
                page::row(t, i == 0)
                    .when(found.is_none(), |d| d.opacity(0.6))
                    .child(page::row_tile(t, ui::harness_icon(kind), Some(ui::harness_tint(kind, t, t.text_muted))))
                    .child(page::row_text(t, kind.label(), meta))
                    .child(control),
            );
        }
        div()
            .flex()
            .flex_col()
            .child(page::page_subtitle(
                t,
                format!("{found_count} agent CLIs found on this machine. Hidden agents stay installed but leave the composer."),
            ))
            .child(page::section(t, "", card.relative().child(self.anchor("agents"))))
            .into_any_element()
    }

    fn team(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let team = self.engine.store.personas().unwrap_or_default();
        let notes = self.flag("team_notes", true);
        let roster: gpui::Div = if team.is_empty() {
            page::card(t).child(page::empty_state(
                t,
                icons::USERS_GROUP,
                "No teammates yet",
                "Give an agent a name, a role and a standing brief.",
                Some(
                    ctl::button_sm(t, "set-team-create", "New teammate", Tone::Solid)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::OpenTeam { new: true })))
                        .into_any_element(),
                ),
            ))
        } else {
            let mut card = page::card(t);
            for (i, p) in team.iter().enumerate() {
                let role = if p.role.is_empty() { "Teammate".to_string() } else { p.role.clone() };
                card = card.child(
                    page::row(t, i == 0)
                        .child(crate::team::avatar(p, 30.0, t))
                        .child(page::row_text(t, p.name.clone(), crate::team::subtitle(p)))
                        .child(page::badge(t, role)),
                );
            }
            card
        };
        let behavior = page::card(t)
            .child(self.srow(
                t,
                true,
                "team_notes",
                "Persistent teammate notes",
                "Each teammate reads its notes when a thread starts and updates them as it learns.",
                ctl::on_toggle(ctl::toggle(t, "set-team-notes", notes), cx, move |this, _, cx| {
                    this.set_flag("team_notes", !notes, cx)
                }),
            ))
            .child(
                self.srow(
                    t,
                    false,
                    "team_page",
                    "Team page",
                    "Create and brief teammates, or start a thread with one.",
                    ctl::button_sm(t, "set-open-team", "Open Team page", Tone::Filled)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::OpenTeam { new: false }))),
                ),
            );
        div()
            .flex()
            .flex_col()
            .child(page::section(t, "Teammates", roster.relative().child(self.anchor("team_roster"))))
            .child(page::section(t, "Memory and behavior", behavior))
            .into_any_element()
    }

    fn plugins_section(&self, t: &Theme) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .child(page::page_subtitle(t, "Built-in tools, apps, MCP servers and skills for every agent Dove starts."))
            .child(
                div()
                    .relative()
                    .child(self.anchor("integrations"))
                    .child(self.anchor("connectors"))
                    .child(self.anchor("skills"))
                    .child(self.anchor("agent_office_skills"))
                    .child(self.anchor("memory"))
                    .child(self.integrations.clone()),
            )
            .into_any_element()
    }

    /// Plugins' header: the directory and the Add menu (the view draws it).
    fn plugins_actions(&self, t: &Theme) -> AnyElement {
        let (browse, add) = (self.integrations.clone(), self.integrations.clone());
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                ctl::button_sm(t, "plugins-browse", "Browse directory", Tone::Outlined)
                    .on_click(move |_, _, cx| browse.update(cx, |v, cx| v.browse(cx))),
            )
            .child(
                ctl::button_sm(t, "plugins-add", "Add", Tone::Solid)
                    .on_click(move |_, _, cx| add.update(cx, |v, cx| v.toggle_add(cx))),
            )
            .into_any_element()
    }

    fn data(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let root = self.engine.store.root().to_path_buf();
        let root_str = root.display().to_string();
        let root_copy = root_str.clone();
        let size = self.data_size.map(|b| format!("{:.1} MB", b as f64 / 1e6)).unwrap_or_else(|| "Measuring…".into());
        let card = page::card(t)
            .child(
                self.srow(
                    t,
                    true,
                    "data_size",
                    "Storage",
                    "Projects, threads, transcripts, settings and the voice model.",
                    div().flex().items_center().gap(px(8.0)).child(page::badge(t, size)).child(
                        ctl::button_sm(t, "open-data", "Open folder", Tone::Filled)
                            .child(icon(icons::ARROW_UP_RIGHT).size(px(12.0)))
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_with_system(&root))),
                    ),
                ),
            )
            .child(
                self.srow(
                    t,
                    false,
                    "data_path",
                    "Storage location",
                    "Set DOVE_HOME to move Dove's data folder.",
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .max_w(px(260.0))
                                .truncate()
                                .font_family(t.font_mono.clone())
                                .text_size(ty::META)
                                .text_color(t.text_muted)
                                .child(root_str),
                        )
                        .child(ctl::button_sm(t, "copy-data-path", "Copy", Tone::Quiet).on_click(cx.listener(
                            move |_, _, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(root_copy.clone())),
                        ))),
                ),
            )
            .child(self.srow(
                t,
                false,
                "telemetry",
                "Privacy",
                "Transcripts, credentials and settings stay on this machine.",
                page::badge_tone("Zero telemetry", t.success),
            ));
        page::section(t, "Local data", card).into_any_element()
    }

    fn remote(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(remote) = crate::remote::Remote::global(cx) else {
            return div().into_any_element();
        };
        let (status, enabled) = {
            let r = remote.read(cx);
            (r.status.clone(), r.enabled())
        };
        let account: SharedString = if status.signed_in {
            status.email.clone().unwrap_or_else(|| "Signed in".into()).into()
        } else if status.signing_in {
            "Finish signing in in your browser".into()
        } else {
            "Not signed in".into()
        };
        let (state_label, state_col) = if !enabled {
            ("Off", t.text_faint)
        } else if status.connected {
            ("Connected · phone paired", t.success)
        } else if status.signed_in {
            ("Connecting…", t.busy)
        } else if status.running {
            ("Sign in to connect", t.warning)
        } else {
            ("Starting…", t.busy)
        };
        let account_button = if status.signed_in {
            ctl::button_sm(t, "remote-sign-out", "Sign out", Tone::Quiet).on_click({
                let remote = remote.clone();
                move |_, _, cx| remote.update(cx, |r, cx| r.sign_out(cx))
            })
        } else {
            ctl::button_sm(t, "remote-sign-in", "Sign in with Woozlit", Tone::Solid).on_click({
                let remote = remote.clone();
                move |_, _, cx| remote.update(cx, |r, cx| r.sign_in(cx))
            })
        };
        let toggle = ctl::toggle(t, "set-remote", enabled).on_click({
            let remote = remote.clone();
            move |_, _, cx| remote.update(cx, |r, cx| r.set_enabled(!enabled, cx))
        });
        let mut card = page::card(t)
            .child(self.srow(
                t,
                true,
                "remote_enabled",
                "Remote control",
                "Watch and steer this computer's threads from the Dove mobile app. While off, no connection is opened.",
                toggle,
            ))
            .child(self.srow(t, false, "remote_account", "Woozlit account", account, account_button))
            .child(page::setting_row(t, false, "Connection", "", page::badge_tone(state_label, state_col)));
        if let Some(id) = status.device_id.clone() {
            card = card.child(page::setting_row(
                t,
                false,
                "Device ID",
                "",
                div().font_family(t.font_mono.clone()).text_size(ty::META).text_color(t.text_muted).child(id),
            ));
        }
        if let Some(e) = status.error.clone() {
            card = card.child(page::row(t, false).child(page::error_strip(t, format!("Last error: {e}"))));
        }
        page::section(t, "This computer", card).into_any_element()
    }

    fn about(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let card = page::card(t)
            .child(self.srow(
                t,
                true,
                "version",
                "Version",
                "Native Rust and GPUI workspace for coding agents.",
                page::badge(t, format!("v{}", env!("CARGO_PKG_VERSION"))),
            ))
            .child(page::setting_row(
                t,
                false,
                "License",
                "Apache-2.0. Third-party notices ship with Dove.",
                page::badge(t, "Apache-2.0"),
            ))
            .child(page::setting_row(
                t,
                false,
                "Source",
                "Read the code, file issues or contribute.",
                ctl::button_sm(t, "about-src", "GitHub", Tone::Filled)
                    .child(icon(icons::ARROW_UP_RIGHT).size(px(12.0)))
                    .on_click(cx.listener(|_, _, _, cx| cx.open_url("https://github.com/dove-ade/dove"))),
            ));
        page::section(t, "Dove", card).into_any_element()
    }
}

/// A miniature window for the colour-scheme cards: a sidebar and a content
/// plane with a few lines, in light, dark, or split for System.
fn scheme_preview(id: &'static str) -> AnyElement {
    let pane = |dark: bool| {
        let (shell, plane, line, strong) = if dark {
            (gpui::rgb(0x0d0d0d), gpui::rgb(0x161616), gpui::rgb(0x2a2a2a), gpui::rgb(0x3a3a3a))
        } else {
            (gpui::rgb(0xf3f3f5), gpui::rgb(0xffffff), gpui::rgb(0xe2e2e6), gpui::rgb(0xcfcfd4))
        };
        div()
            .flex_1()
            .h_full()
            .bg(shell)
            .flex()
            .gap(px(6.0))
            .p(px(8.0))
            .child(div().w(px(22.0)).flex().flex_col().gap(px(5.0)).pt(px(4.0)).children(
                (0..4).map(move |i| div().h(px(3.0)).w(px(if i == 0 { 18.0 } else { 14.0 })).rounded_full().bg(strong)),
            ))
            .child(
                div()
                    .flex_1()
                    .rounded(px(design::radius::RADIUS))
                    .bg(plane)
                    .p(px(7.0))
                    .flex()
                    .flex_col()
                    .gap(px(5.0))
                    .child(div().h(px(4.0)).w(relative_w(0.55)).rounded_full().bg(strong))
                    .children(
                        [0.9, 0.75, 0.82]
                            .into_iter()
                            .map(move |w| div().h(px(3.0)).w(relative_w(w)).rounded_full().bg(line)),
                    ),
            )
    };
    match id {
        "light" => pane(false).into_any_element(),
        "dark" => pane(true).into_any_element(),
        _ => div().size_full().flex().child(pane(false)).child(pane(true)).into_any_element(),
    }
}

fn relative_w(f: f32) -> gpui::DefiniteLength {
    gpui::relative(f)
}

fn dir_size(p: &std::path::Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            match e.metadata() {
                Ok(m) if m.is_dir() => stack.push(e.path()),
                Ok(m) => total += m.len(),
                Err(_) => {}
            }
        }
    }
    total
}

/// What moves in Dove and what it does under the current motion settings,
/// so "Reduce motion" and "Pause animations in background" are not a guess.
fn motion_inventory(t: &Theme, reduced: bool, has_wall: bool, animate: bool, effect: Effect) -> AnyElement {
    let background = if !has_wall {
        ("No image chosen", "Off")
    } else if !animate {
        ("Animate background is off", "Off")
    } else {
        (effect.motion_hint(), if reduced { "Still" } else { "Playing" })
    };
    let items: [(&str, &str, &str, &str); 5] = [
        (
            "Working indicators",
            "Spinners on running threads in the sidebar, tabs and transcript",
            if reduced { "Slow breathe" } else { "Spinning" },
            icons::REFRESH,
        ),
        (
            "Live tool calls and thinking",
            "A shimmer over the step an agent is on right now",
            if reduced { "Still" } else { "Shimmer" },
            icons::MAGIC_STICK_3,
        ),
        (
            "Loading placeholders",
            "Skeleton bars while history, files or models load",
            if reduced { "Slow breathe" } else { "Pulsing" },
            icons::CLOCK_CIRCLE,
        ),
        (
            "Transitions",
            "Sidebar and panel slides, menus, toasts, fades and scroll-to-message",
            if reduced { "Snap" } else { "Animated" },
            icons::WIDGET,
        ),
        ("Background art", background.0, background.1, icons::GALLERY),
    ];
    let mut list = div().w_full().flex().flex_col().gap(px(8.0));
    for (title, detail, state, glyph) in items {
        let moving = matches!(state, "Spinning" | "Shimmer" | "Pulsing" | "Animated" | "Playing");
        list = list.child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(icon(glyph).size(px(14.0)).flex_none().text_color(t.text_muted))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().text_size(ty::BODY).text_color(t.text).child(title))
                        .child(div().text_size(ty::META).text_color(t.text_muted).child(detail)),
                )
                .child(page::badge_tone(state, if moving { t.accent } else { t.text_muted })),
        );
    }
    page::row(t, false)
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(page::row_text(t, "What animates", "Everything in Dove that moves, and what it does right now."))
                .child(list),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_round_trips_its_id() {
        for s in Section::ALL {
            assert_eq!(Section::from_id(s.id()), Some(s));
        }
        assert_eq!(Section::from_id("composer"), Some(Section::General));
        assert_eq!(Section::from_id("shortcuts"), Some(Section::Shortcuts));
        assert_eq!(Section::from_id("skills"), Some(Section::Plugins));
        assert_eq!(Section::from_id("composio"), Some(Section::Plugins));
        assert_eq!(Section::from_id("memory"), Some(Section::Plugins));
        assert_eq!(Section::from_id("nope"), None);
    }

    #[test]
    fn every_section_is_in_the_nav_once() {
        let mut seen: Vec<Section> = Section::GROUPS.iter().flat_map(|g| g.iter().copied()).collect();
        seen.sort_by_key(|s| s.id());
        seen.dedup();
        assert_eq!(seen.len(), Section::ALL.len());
    }

    #[test]
    fn search_finds_rows_by_title_and_keyword() {
        assert_eq!(find_settings("reduce motion").first().map(|e| e.row), Some("reduce_motion"));
        assert!(find_settings("clone").iter().any(|e| e.row == "clone_dir"));
        assert!(find_settings("parakeet").iter().any(|e| e.row == "voice_model"));
        assert!(find_settings("superpowers").iter().any(|e| e.section == Section::Plugins && e.row == "skills"));
        assert!(find_settings("").is_empty());
    }

    #[test]
    fn index_rows_are_unique() {
        let mut rows: Vec<&str> = INDEX.iter().map(|e| e.row).collect();
        rows.sort_unstable();
        let n = rows.len();
        rows.dedup();
        assert_eq!(rows.len(), n);
    }
}
