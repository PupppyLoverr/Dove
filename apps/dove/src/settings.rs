//! Settings, with grouped navigation: App (General, Appearance,
//! Notifications, Keyboard) · Agents & Studio (Agents, Crew & Team, Composer, Canvas) ·
//! Workspace (Voice, Remote control, Integrations) · System (Data, About).
//! Includes live section filtering, status badges on the navigation rail,
//! contextual section hero headers, and interactive setting cards. Everything
//! persists in the store's settings table and applies live.

use std::sync::Arc;

use dove_core::Engine;
use dove_core::proto::HarnessKind;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FocusHandle,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::kit;
use crate::theme::{Accent, Surface, Theme};
use crate::ui;
use crate::voice::{InstallState, VoiceModel};
use crate::wallpaper::Effect;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Section {
    General,
    Appearance,
    Agents,
    Team,
    Canvas,
    Composer,
    Voice,
    Keyboard,
    Notifications,
    Remote,
    Integrations,
    Data,
    About,
}

/// What the settings page asks the shell to do.
pub enum SettingsEvent {
    /// Open the Team page (optionally its editor for a new teammate).
    OpenTeam { new: bool },
    /// Open the Agents page (sign-in terminals, binary paths).
    OpenAgents,
}

impl Section {
    const ALL: [Section; 13] = [
        Section::General,
        Section::Appearance,
        Section::Agents,
        Section::Team,
        Section::Canvas,
        Section::Composer,
        Section::Voice,
        Section::Keyboard,
        Section::Notifications,
        Section::Remote,
        Section::Integrations,
        Section::Data,
        Section::About,
    ];

    /// The navigation, in groups.
    const GROUPS: [(&'static str, &'static [Section]); 4] = [
        ("App", &[Section::General, Section::Appearance, Section::Notifications, Section::Keyboard]),
        ("Agents & Studio", &[Section::Agents, Section::Team, Section::Composer, Section::Canvas]),
        ("Workspace", &[Section::Voice, Section::Remote, Section::Integrations]),
        ("System", &[Section::Data, Section::About]),
    ];

    /// One line under the section title.
    fn blurb(self) -> &'static str {
        match self {
            Section::General => "Threads, git worktrees, branch naming, and where new work starts.",
            Section::Appearance => "Theme mode, accent color, surface materials, and the new-thread backdrop.",
            Section::Agents => "Installed agent CLIs Dove orchestrates and which harnesses appear in the composer.",
            Section::Team => "Specialists & Crew members with dedicated roles, standing briefs, and memory.",
            Section::Canvas => "Live interactive HTML, React/TSX, and Tailwind artboards built by agents.",
            Section::Composer => "Default execution permissions, slash skills, and per-project approval rules.",
            Section::Voice => "On-device Parakeet/Whisper dictation and spoken Jarvis voice responses.",
            Section::Keyboard => "Every global and workspace shortcut, customizable with live key recording.",
            Section::Notifications => "System alerts, completion sounds, and background daemon behavior.",
            Section::Remote => "Monitor and drive this computer's active sessions from your phone.",
            Section::Integrations => "MCP tools, Chrome bridge, Composio, Paper, and computer-use drivers.",
            Section::Data => "Local SQLite database, transcripts, voice weights, and storage location.",
            Section::About => "Build version, runtime architecture, and open-source licensing.",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Appearance => "Appearance",
            Section::Agents => "Agents",
            Section::Team => "Crew & Team",
            Section::Canvas => "Canvas",
            Section::Composer => "Composer",
            Section::Voice => "Voice & Jarvis",
            Section::Keyboard => "Keyboard",
            Section::Notifications => "Notifications",
            Section::Remote => "Remote Control",
            Section::Integrations => "Integrations",
            Section::Data => "Data & Storage",
            Section::About => "About",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Section::General => icons::SETTINGS,
            Section::Appearance => icons::PALETTE_SEARCH,
            Section::Agents => icons::BOT,
            Section::Team => icons::BOT,
            Section::Canvas => icons::GALLERY,
            Section::Composer => icons::PEN,
            Section::Voice => icons::MICROPHONE,
            Section::Keyboard => icons::KEYBOARD,
            Section::Notifications => icons::BELL,
            Section::Remote => icons::SMARTPHONE,
            Section::Integrations => icons::WIDGET,
            Section::Data => icons::HARD_DRIVE,
            Section::About => icons::INFO_CIRCLE,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Section::General => "general",
            Section::Appearance => "appearance",
            Section::Agents => "agents",
            Section::Team => "team",
            Section::Canvas => "canvas",
            Section::Composer => "composer",
            Section::Voice => "voice",
            Section::Keyboard => "keyboard",
            Section::Notifications => "notifications",
            Section::Remote => "remote",
            Section::Integrations => "integrations",
            Section::Data => "data",
            Section::About => "about",
        }
    }

    fn keywords(self) -> &'static str {
        match self {
            Section::General => "general threads tabs reopen restore prewarm git worktree branch prefix clone folder directory",
            Section::Appearance => "appearance theme dark light system accent sky indigo orange green pink surface glass acrylic normal frosted wallpaper background effect animate animation motion",
            Section::Agents => "agents harness claude codex cursor gemini opencode kimi droid qwen kilocode cli binary sign in",
            Section::Team => "team crew specialists personas teammates role brief notes memory",
            Section::Canvas => "canvas design artboard html react tsx tailwind svg preview mobile tablet desktop bridge",
            Section::Composer => "composer permissions default auto-edit plan ask bypass approvals",
            Section::Voice => "voice dictation microphone parakeet nexi whisper jarvis tts speech spoken jasper bella luna bruno fish audio",
            Section::Keyboard => "keyboard shortcuts keybindings hotkeys rebind keys",
            Section::Notifications => "notifications alerts sound tray background keep running",
            Section::Remote => "remote control phone mobile woozlit account device",
            Section::Integrations => "integrations mcp chrome browser composio paper computer use cua driver",
            Section::Data => "data storage folder disk size location dove_home telemetry privacy",
            Section::About => "about version license apache github repository source",
        }
    }

    fn matches(self, q: &str) -> bool {
        let q = q.trim().to_ascii_lowercase();
        if q.is_empty() {
            return true;
        }
        self.label().to_ascii_lowercase().contains(&q)
            || self.blurb().to_ascii_lowercase().contains(&q)
            || self.keywords().contains(&q)
    }
}

pub struct SettingsPage {
    engine: Arc<Engine>,
    section: Section,
    search: Entity<InputState>,
    query: String,
    prefix: Entity<InputState>,
    clone_dir: Entity<InputState>,
    /// The shortcut being recorded.
    recording: Option<&'static str>,
    devices: Vec<(String, String)>,
    data_size: Option<u64>,
    integrations: Entity<crate::pages::integrations::IntegrationsView>,
    fish_key: Entity<InputState>,
    voice_note: Option<String>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

fn text_setting(
    engine: &Arc<Engine>,
    key: &'static str,
    default: String,
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut Context<SettingsPage>,
) -> (Entity<InputState>, Subscription) {
    let style = ui::input_style(Theme::of(cx));
    let current = engine.store.setting(key).unwrap_or(default);
    let input = cx.new(|cx| {
        let mut s = InputState::new(window, cx).placeholder(placeholder);
        s.set_editor_style(style);
        s.set_value(current, window, cx);
        s
    });
    let engine = engine.clone();
    let sub = cx.subscribe(&input, move |_, s, ev: &InputEvent, cx| {
        if matches!(ev, InputEvent::Change | InputEvent::Blur) {
            let v = s.read(cx).value().trim().to_string();
            let _ = engine.store.set_setting(key, &v);
        }
    });
    (input, sub)
}

impl EventEmitter<SettingsEvent> for SettingsPage {}

impl SettingsPage {
    pub fn new(engine: Arc<Engine>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let style = ui::input_style(Theme::of(cx));
        let search = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Filter settings…");
            s.set_editor_style(style);
            s
        });
        let search_sub = cx.subscribe(&search, |this, s, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                this.query = s.read(cx).value().to_string();
                if !this.section.matches(&this.query)
                    && let Some(first) = Section::ALL.into_iter().find(|sec| sec.matches(&this.query))
                {
                    this.select(first, cx);
                } else {
                    cx.notify();
                }
            }
        });

        let (prefix, s1) = text_setting(&engine, "branch_prefix", "dove/".into(), "dove/", window, cx);
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let (clone_dir, s2) = text_setting(
            &engine,
            "clone_dir",
            home.join("Dove").display().to_string(),
            "Folder for cloned repositories",
            window,
            cx,
        );
        let mut subs = vec![search_sub, s1, s2];
        if let Some(model) = VoiceModel::global(cx) {
            subs.push(cx.observe(&model, |_, _, cx| cx.notify()));
        }
        if let Some(remote) = crate::remote::Remote::global(cx) {
            subs.push(cx.observe(&remote, |_, _, cx| cx.notify()));
        }
        let (fish_key, fish_sub) = text_setting(&engine, "fish_api_key", String::new(), "Fish Audio API key", window, cx);
        subs.push(fish_sub);
        Self {
            engine,
            section: Section::General,
            search,
            query: String::new(),
            prefix,
            clone_dir,
            recording: None,
            devices: Vec::new(),
            data_size: None,
            integrations: cx.new(|cx| crate::pages::integrations::IntegrationsView::new(window, cx)),
            fish_key,
            voice_note: None,
            focus: cx.focus_handle(),
            _subs: subs,
        }
    }

    pub fn show_section(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(s) = Section::ALL.into_iter().find(|s| s.id() == id) {
            self.select(s, cx);
        }
    }

    fn select(&mut self, s: Section, cx: &mut Context<Self>) {
        self.section = s;
        self.recording = None;
        match s {
            Section::Voice if self.devices.is_empty() => {
                let load = cx.background_spawn(async move { crate::voice::input_devices() });
                cx.spawn(async move |this, cx| {
                    let devices = load.await;
                    let _ = this.update(cx, |p, cx| {
                        p.devices = devices;
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
        match self.get(key).as_deref() {
            Some("true") => true,
            Some("false") => false,
            _ => default,
        }
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

    /// Renders a rich top hero card for the active section with icon badge, blurb, and contextual actions.
    fn section_hero(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let s = self.section;
        let action: Option<AnyElement> = match s {
            Section::General => {
                let wt = self.flag("use_worktree", true);
                Some(
                    kit::badge(
                        if wt { "Worktree isolation · On" } else { "Worktree isolation · Off" },
                        if wt { t.success } else { t.text_faint },
                    )
                    .into_any_element(),
                )
            }
            Section::Appearance => {
                let surface = Surface::from_settings(self.get("surface").as_deref(), self.get("frosted").as_deref());
                Some(kit::badge(format!("{} surface", surface.label()), t.accent).into_any_element())
            }
            Section::Agents => Some(
                ui::button_primary("hero-open-agents", "Configure Harnesses", t)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::OpenAgents)))
                    .into_any_element(),
            ),
            Section::Team => Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        kit::button_outline("hero-open-team", "Manage Crew", t)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::OpenTeam { new: false }))),
                    )
                    .child(
                        ui::button_primary("hero-new-team", "+ New Specialist", t)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::OpenTeam { new: true }))),
                    )
                    .into_any_element(),
            ),
            Section::Canvas => {
                let bridge = self.flag("canvas_bridge", true);
                Some(
                    kit::badge(
                        if bridge { "2-Way Bridge · Active" } else { "Preview Only" },
                        if bridge { t.accent } else { t.text_faint },
                    )
                    .into_any_element(),
                )
            }
            Section::Keyboard => {
                let store = self.engine.store.clone();
                let any_custom = crate::keymap::shortcuts()
                    .iter()
                    .any(|sc| crate::keymap::binding(&store, sc) != sc.default);
                if any_custom {
                    Some(
                        kit::button_outline("hero-reset-keys", "Reset All Shortcuts", t)
                            .on_click(cx.listener(|this, _, _, cx| {
                                for sc in crate::keymap::shortcuts() {
                                    crate::keymap::rebind(&this.engine.store, sc.id, sc.default, cx);
                                }
                                cx.notify();
                            }))
                            .into_any_element(),
                    )
                } else {
                    Some(kit::badge("Default bindings", t.text_faint).into_any_element())
                }
            }
            Section::Data => {
                let root = self.engine.store.root().to_path_buf();
                Some(
                    kit::button_outline("hero-open-data", "Open Data Folder", t)
                        .on_click(cx.listener(move |_, _, _, cx| cx.open_with_system(&root)))
                        .into_any_element(),
                )
            }
            Section::About => Some(kit::badge(format!("v{}", env!("CARGO_PKG_VERSION")), t.accent).into_any_element()),
            _ => None,
        };

        div()
            .w_full()
            .p(px(16.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(t.border)
            .bg(t.wash(0.026))
            .flex()
            .items_center()
            .gap(px(14.0))
            .child(
                div()
                    .size(px(38.0))
                    .flex_none()
                    .rounded(px(10.0))
                    .bg(t.accent.opacity(0.14))
                    .border_1()
                    .border_color(t.accent.opacity(0.28))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(s.glyph()).size(px(18.0)).text_color(t.accent)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(18.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child(s.label()),
                    )
                    .child(div().text_size(px(12.5)).text_color(t.text_muted).child(s.blurb())),
            )
            .when_some(action, |d, a| d.child(a))
            .into_any_element()
    }

    // ------------------------------------------------------------ sections

    fn general(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let restore = self.flag("restore_tabs", true);
        let prewarm = self.flag("prewarm", true);
        let worktree = self.flag("use_worktree", true);
        stack()
            .child(card(
                "Threads & Startup",
                t,
                div()
                    .child(row(
                        "Reopen thread tabs on launch",
                        "Restore the workspace tabs and active thread you had open last session",
                        kit::switch("set-restore", restore, t).on_click(cx.listener(move |this, _, _, cx| {
                            this.set("restore_tabs", if restore { "false" } else { "true" }, cx)
                        })),
                        t,
                    ))
                    .child(row(
                        "Prewarm agent harness while typing",
                        "Spawns the selected agent in the background as you compose so the first token arrives faster",
                        kit::switch("set-prewarm", prewarm, t).on_click(cx.listener(move |this, _, _, cx| {
                            this.set("prewarm", if prewarm { "false" } else { "true" }, cx)
                        })),
                        t,
                    )),
            ))
            .child(card(
                "Git & Worktrees",
                t,
                div()
                    .child(row(
                        "Isolate each new thread in a git worktree",
                        "Creates a dedicated branch and working directory so parallel agents never collide",
                        kit::switch("set-worktree", worktree, t).on_click(cx.listener(move |this, _, _, cx| {
                            this.set("use_worktree", if worktree { "false" } else { "true" }, cx)
                        })),
                        t,
                    ))
                    .child(row(
                        "Branch prefix",
                        "Prefix applied to automatically created worktree branches (e.g. dove/fix-auth)",
                        field(&self.prefix, 190.0, t),
                        t,
                    ))
                    .child(row(
                        "Default clone directory",
                        "Destination folder when cloning repositories from a GitHub URL",
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(field(&self.clone_dir, 240.0, t))
                            .child(
                                ui::button_ghost("browse-clone-dir", "Browse…", t)
                                    .on_click(cx.listener(|this, _, window, cx| this.pick_clone_dir(window, cx))),
                            ),
                        t,
                    )),
            ))
            .into_any_element()
    }

    fn agents(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let installed = self.engine.installed();
        let supported: Vec<HarnessKind> = HarnessKind::ALL
            .iter()
            .copied()
            .filter(|k| *k != HarnessKind::Shell && dove_core::harness_supported(*k))
            .collect();
        let installed_count = supported
            .iter()
            .filter(|k| installed.iter().any(|i| i.kind == **k))
            .count();

        let rows = supported.into_iter().map(|kind| {
                let found = installed.iter().find(|i| i.kind == kind);
                let enabled = crate::pages::agent_enabled(&self.engine.store, kind);
                let key = format!("agent_disabled.{}", kind.id());
                let control: AnyElement = if found.is_some() {
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(kit::badge(
                        if enabled { "Enabled" } else { "Hidden" },
                        if enabled { t.success } else { t.text_faint },
                    ))
                    .child(
                        kit::switch(SharedString::from(format!("set-agent-{}", kind.id())), enabled, t).on_click(
                            cx.listener(move |this, _, _, cx| {
                            this.set(&key, if enabled { "true" } else { "false" }, cx)
                            }),
                        ),
                    )
                        .into_any_element()
                } else {
                kit::badge("Not installed", t.text_faint).into_any_element()
                };
                div()
                    .px(px(16.0))
                    .py(px(12.0))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .border_t_1()
                    .border_color(t.border)
                    .when(found.is_none(), |d| d.opacity(0.6))
                    .child(
                        div()
                        .size(px(30.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.0))
                        .bg(t.wash(0.06))
                        .border_1()
                        .border_color(t.border)
                            .child(
                                icon(ui::harness_icon(kind))
                                    .size(px(15.0))
                                    .text_color(ui::harness_tint(kind, t, t.text_muted)),
                            ),
                    )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(px(13.5))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(t.text)
                                .child(kind.label()),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_family(t.font_mono.clone())
                                .text_color(t.text_faint)
                                .truncate()
                                .child(
                                    found
                                        .map(|f| f.program.display().to_string())
                                        .unwrap_or_else(|| "Binary not found on PATH".into()),
                                ),
                        ),
                )
                    .child(control)
            });
        stack()
            .child(card(
                "Installed Harnesses",
                t,
                div()
                    .child(
                        div()
                            .px(px(16.0))
                            .py(px(10.0))
                            .bg(t.wash(0.03))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(t.text_muted)
                                    .child(format!("{installed_count} agent CLI(s) detected on this machine")),
                            )
                            .child(
                                ui::button_ghost("set-open-agents-inline", "Sign-in & custom paths →", t)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::OpenAgents))),
                            ),
                    )
                    .children(rows),
            ))
            .into_any_element()
    }

    fn team(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let team = self.engine.store.personas().unwrap_or_default();
        let notes = self.flag("team_notes", true);
        let roster: AnyElement = if team.is_empty() {
            row(
                "No Crew specialists yet",
                "Give an agent a name, a role, and a standing brief—or seed starter specialists from the Crew page",
                ui::button_primary("set-team-new", "Create Specialist", t)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::OpenTeam { new: true }))),
                t,
            )
            .into_any_element()
        } else {
            div()
                .children(team.iter().map(|p| {
                    let role = if p.role.is_empty() { "Specialist".to_string() } else { p.role.clone() };
                    div()
                        .px(px(16.0))
                        .py(px(12.0))
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .border_t_1()
                        .border_color(t.border)
                        .child(crate::team::avatar(p, 30.0, t))
                        .child(kit::label_block(p.name.clone(), crate::team::subtitle(p), t))
                        .child(kit::badge(role, t.accent))
                }))
                .into_any_element()
        };
        stack()
            .child(card("Active Crew Roster", t, roster))
            .child(card(
                "Memory & Behavior",
                t,
                div()
                    .child(row(
                        "Persistent specialist notes",
                        "Each specialist reads its accumulated notes at the start of a thread and updates them as it learns",
                        kit::switch("set-team-notes", notes, t).on_click(cx.listener(move |this, _, _, cx| {
                            this.set("team_notes", if notes { "false" } else { "true" }, cx)
                        })),
                        t,
                    ))
                    .child(row(
                        "Crew Studio",
                        "Create, brief, and edit specialists or launch a specialist thread in 1 click",
                        kit::button_outline("set-open-team", "Open Crew Page", t)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::OpenTeam { new: false }))),
                        t,
                    )),
            ))
            .into_any_element()
    }

    fn canvas(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let auto = self.flag("canvas_auto_open", true);
        let bridge = self.flag("canvas_bridge", true);
        let dev = self.get("canvas_default_device").unwrap_or_else(|| "desktop".into());
        let dev_modes = ["desktop", "tablet", "mobile"];
        let dev_control = kit::segmented(
            "set-canvas-device",
            &["Desktop", "Tablet", "Mobile"],
            dev_modes.iter().position(|m| *m == dev).unwrap_or(0),
            t,
            {
                let weak = cx.weak_entity();
                move |i, _, cx| {
                    let _ = weak.update(cx, |s, cx| s.set("canvas_default_device", dev_modes[i], cx));
                }
            },
        );
        let pills = [
            "HTML5",
            "React 18 JSX / TSX",
            "Tailwind CSS",
            "SVG Artboards",
            "Responsive Viewports",
            "window.dove.send()",
        ];
        stack()
            .child(card(
                "Studio Behavior",
                t,
                div()
                    .child(row(
                        "Open designs automatically as they arrive",
                        "Immediately reveals the Live Design Canvas in the right dock when an agent calls show_canvas or emits an artboard",
                        kit::switch("set-canvas-auto", auto, t).on_click(cx.listener(move |this, _, _, cx| {
                            this.set("canvas_auto_open", if auto { "false" } else { "true" }, cx)
                        })),
                        t,
                    ))
                    .child(row(
                        "Interactive 2-way agent bridge",
                        "Allows buttons and forms inside a rendered design to call dove.send() and reply directly to the agent",
                        kit::switch("set-canvas-bridge", bridge, t).on_click(cx.listener(move |this, _, _, cx| {
                            this.set("canvas_bridge", if bridge { "false" } else { "true" }, cx)
                        })),
                        t,
                    ))
                    .child(row(
                        "Default artboard viewport",
                        "Preferred device frame when a design does not specify a target viewport",
                        dev_control,
                        t,
                    )),
            ))
            .child(card(
                "Supported Runtimes",
                t,
                    div()
                        .px(px(16.0))
                        .py(px(14.0))
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(6.0))
                            .children(pills.into_iter().map(|p| kit::badge(p, t.accent))),
                    )
                    .child(
                        div()
                        .text_size(px(12.5))
                        .text_color(t.text_muted)
                        .child(
                                "Agents invoke show_canvas with self-contained HTML, React (JSX/TSX), or SVG, or emit \
                                 fenced code blocks you can open in one click. Artboards are served locally on 127.0.0.1 \
                                 with live Desktop, Tablet, and Phone frames, zoom controls, and an editable source drawer.",
                        ),
                ),
            ))
            .into_any_element()
    }

    fn appearance(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.get("appearance").unwrap_or_else(|| "system".into());
        let accent = Accent::from_id(&self.get("accent").unwrap_or_default());
        let surface = Surface::from_settings(self.get("surface").as_deref(), self.get("frosted").as_deref());
        let wall = self.get("wallpaper_path").filter(|p| !p.is_empty());
        let effect = Effect::from_id(&self.get("wallpaper_effect").unwrap_or_default());
        let animate = self.flag("wallpaper_animate", false);
        let modes = ["system", "dark", "light"];
        let appearance = kit::segmented(
            "set-appearance",
            &["System", "Dark", "Light"],
            modes.iter().position(|m| *m == mode).unwrap_or(0),
            t,
            {
                let weak = cx.weak_entity();
                move |i, _, cx| {
                    let _ = weak.update(cx, |s, cx| s.set("appearance", modes[i], cx));
                }
            },
        );
        let swatches = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .children(Accent::ALL.into_iter().map(|a| {
            let color = Theme::new(t.appearance, a, Surface::Normal).accent;
                let selected = a == accent;
            div()
                .id(SharedString::from(format!("accent-{}", a.id())))
                    .size(px(24.0))
                .rounded_full()
                .bg(color)
                .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(selected, |d| {
                        d.border_2()
                            .border_color(t.text)
                            .child(icon(icons::CHECK).size(px(12.0)).text_color(gpui::white()))
                    })
                .on_click(cx.listener(move |this, _, _, cx| this.set("accent", a.id(), cx)))
        }));
        let surfaces: Vec<&str> = Surface::ALL.iter().map(|s| s.label()).collect();
        let surface_ix = Surface::ALL.iter().position(|s| *s == surface).unwrap_or(0);
        let surface_control = kit::segmented("set-surface", &surfaces, surface_ix, t, {
            let weak = cx.weak_entity();
            move |i, _, cx| {
                let _ = weak.update(cx, |s, cx| s.set("surface", Surface::ALL[i].id(), cx));
            }
        });
        let effects: Vec<&str> = Effect::ALL.iter().map(|e| e.label()).collect();
        let effect_ix = Effect::ALL.iter().position(|e| *e == effect).unwrap_or(0);
        let effect_control = kit::segmented("set-effect", &effects, effect_ix, t, {
            let weak = cx.weak_entity();
            move |i, _, cx| {
                let _ = weak.update(cx, |s, cx| s.set("wallpaper_effect", Effect::ALL[i].id(), cx));
            }
        });
        let wall_name = wall.as_ref().and_then(|w| {
            std::path::Path::new(w)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
        });
        let look = div()
            .child(row("Color theme", "Follow the OS appearance or lock to Dark or Light", appearance, t))
            .child(row("Accent color", "Primary highlight for buttons, active rings, and badges", swatches, t))
            .child(row(
                "Window surface material",
                match surface {
                    Surface::Glass => "Clear frosted glass over a blurred desktop backdrop",
                    Surface::Acrylic => "Denser frosted acrylic with richer tint and wider blur",
                    Surface::Normal => "Solid opaque surfaces with zero desktop blur",
                },
                surface_control,
                t,
            ));
        let canvas = div()
            .child(row(
                "New-thread background image",
                "Optional custom backdrop behind the composer on the home screen",
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .when_some(wall_name, |d, name| d.child(kit::badge(name, t.text_muted)))
                    .child(
                        ui::button_ghost("wall-pick", if wall.is_some() { "Change…" } else { "Choose image…" }, t)
                            .on_click(cx.listener(|this, _, _, cx| this.pick_wallpaper(cx))),
                    )
                    .when(wall.is_some(), |d| {
                        d.child(
                            ui::button_ghost("wall-clear", "Remove", t)
                                .on_click(cx.listener(|this, _, _, cx| this.set("wallpaper_path", "", cx))),
                        )
                    }),
                t,
            ))
            .child(row(
                "Backdrop shader effect",
                "Visual treatment applied over the new-thread canvas",
                effect_control,
                t,
            ))
            .child(row(
                "Animate backdrop",
                if wall.is_some() { effect.motion_hint() } else { "Choose a background image to animate it" },
                kit::switch("set-wall-animate", animate, t).when_else(
                    wall.is_some(),
                    |s| {
                        s.on_click(cx.listener(move |this, _, _, cx| {
                            this.set("wallpaper_animate", if animate { "false" } else { "true" }, cx)
                        }))
                    },
                    |s| s.opacity(0.45),
                ),
                t,
            ));
        stack()
            .child(card("Theme & Materials", t, look))
            .child(card("New-Thread Backdrop", t, canvas))
            .into_any_element()
    }

    fn composer(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.get("permission_mode").unwrap_or_else(|| "default".into());
        let modes = ["default", "accept_edits", "plan"];
        let default_mode = kit::segmented(
            "set-mode",
            &["Ask every time", "Auto-edit", "Plan"],
            modes.iter().position(|m| *m == mode).unwrap_or(0),
            t,
            {
                let weak = cx.weak_entity();
                move |i, _, cx| {
                    let _ = weak.update(cx, |s, cx| s.set("permission_mode", modes[i], cx));
                }
            },
        );
        let bypassed: Vec<String> = self
            .engine
            .store
            .projects()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| self.get(&format!("bypass_ok.{}", p.id)).as_deref() == Some("1"))
            .map(|p| p.name)
            .collect();
        let body = div()
            .child(row(
                "Default permission level",
                "New threads start in this mode; Bypass is selected per thread and confirmed per project",
                default_mode,
                t,
            ))
            .child(row(
                "Projects authorized for Bypass",
                if bypassed.is_empty() {
                    "No projects have Bypass enabled yet"
                } else {
                    "Agents may execute without confirmation prompts in these projects when Bypass is selected"
                },
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(if bypassed.is_empty() {
                        kit::badge("None", t.text_faint).into_any_element()
                    } else {
                        div()
                            .text_size(px(12.5))
                            .text_color(t.text_muted)
                            .max_w(px(260.0))
                            .truncate()
                            .child(bypassed.join(", "))
                            .into_any_element()
                    })
                    .when(!bypassed.is_empty(), |d| {
                        d.child(ui::button_ghost("bypass-reset", "Revoke all", t).on_click(cx.listener(
                            |this, _, _, cx| {
                            for p in this.engine.store.projects().unwrap_or_default() {
                                let _ = this.engine.store.set_setting(&format!("bypass_ok.{}", p.id), "0");
                            }
                            cx.notify();
                            },
                        )))
                    }),
                t,
            ));
        stack().child(card("Execution & Approvals", t, body)).into_any_element()
    }

    fn voice(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(model) = VoiceModel::global(cx) else { return div().into_any_element() };
        let state = model.read(cx).state.clone();
        let size_mb = VoiceModel::download_size() / 1_000_000;
        let control: AnyElement = match state {
            InstallState::Missing => ui::button_primary("voice-install", format!("Download ({size_mb} MB)"), t)
                .on_click(cx.listener(|_, _, _, cx| {
                    if let Some(m) = VoiceModel::global(cx) {
                        m.update(cx, |m, cx| m.install(cx));
                    }
                }))
                .into_any_element(),
            InstallState::Downloading { done, total } => {
                let frac = done as f32 / total.max(1) as f32;
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .w(px(160.0))
                            .h(px(6.0))
                            .rounded_full()
                            .bg(t.wash(0.10))
                            .child(div().h_full().w(px(160.0 * frac)).rounded_full().bg(t.accent)),
                    )
                    .child(div().text_size(px(12.0)).text_color(t.text_muted).child(format!("{:.0}%", frac * 100.0)))
                    .child(ui::button_ghost("voice-cancel", "Cancel", t).on_click(cx.listener(|_, _, _, cx| {
                        if let Some(m) = VoiceModel::global(cx) {
                            m.update(cx, |m, cx| m.cancel(cx));
                        }
                    })))
                    .into_any_element()
            }
            InstallState::Installed => div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(kit::badge("Installed", t.success))
                .child(ui::button_ghost("voice-remove", "Remove", t).on_click(cx.listener(|_, _, _, cx| {
                    if let Some(m) = VoiceModel::global(cx) {
                        m.update(cx, |m, cx| m.remove(cx));
                    }
                })))
                .into_any_element(),
            InstallState::Failed(e) => div()
                .flex()
                .flex_col()
                .items_end()
                .gap(px(4.0))
                .child(div().max_w(px(280.0)).text_size(px(12.0)).text_color(t.danger).child(e))
                .child(ui::button_ghost("voice-retry", "Try again", t).on_click(cx.listener(|_, _, _, cx| {
                    if let Some(m) = VoiceModel::global(cx) {
                        m.update(cx, |m, cx| {
                            m.state = InstallState::Missing;
                            m.install(cx);
                        });
                    }
                })))
                .into_any_element(),
        };
        let current = self.get("voice_device").unwrap_or_default();
        let devices = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                device_row("dev-default", "System default", current.is_empty(), t)
                    .on_click(cx.listener(|this, _, _, cx| this.set("voice_device", "", cx))),
            )
            .children(self.devices.iter().enumerate().map(|(i, (id, name))| {
                let id2 = id.clone();
                device_row(SharedString::from(format!("dev-{i}")), name, *id == current, t)
                    .on_click(cx.listener(move |this, _, _, cx| this.set("voice_device", &id2, cx)))
            }));
        let body = div()
            .child(row(
                "Local Parakeet dictation model",
                "NVIDIA Parakeet TDT 0.6B v3 runs 100% on-device. Verified file-by-file via SHA-256.",
                control,
                t,
            ))
            .child(row("Input microphone", "Select which audio input device captures voice dictation", devices, t))
            .child(row(
                "Dictation trigger",
                "Hold the mic icon in the composer or terminal to talk, or click to toggle recording.",
                kit::badge("Push-to-talk / Toggle", t.text_faint),
                t,
            ));
        let stt = self.get("stt_engine").unwrap_or_else(|| "parakeet".into());
        let tts = self.get("tts_backend").unwrap_or_else(|| "nexi".into());
        let spoken = self.get("tts_voice").unwrap_or_else(|| "Jasper".into());
        let jarvis = self.flag("jarvis", false);
        let report = crate::nexi::Report::probe();
        let nexi = report.clone();
        let body = body
            .child(row(
                "Speech-to-text engine",
                "Parakeet runs built-in. NEXI runs the local Whisper driver from NEXI-Models-Drivers.",
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(self.choice("stt-parakeet", "Parakeet", stt != "nexi", "stt_engine", "parakeet", t, cx))
                    .child(self.choice("stt-nexi", "NEXI", stt == "nexi", "stt_engine", "nexi", t, cx)),
                t,
            ))
            .child(row(
                "NEXI Whisper driver",
                "Whisper tiny.en downloaded once into your local NEXI checkout.",
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(6.0))
                    .child(div().max_w(px(280.0)).text_size(px(12.0)).text_color(t.text_muted).child(report.stt_line()))
                    .child(ui::button_ghost("stt-setup", "Copy setup", t).on_click(cx.listener(|this, _, _, cx| {
                        let script = crate::nexi::setup_script(crate::nexi::Report::probe().root.as_deref());
                        cx.write_to_clipboard(ClipboardItem::new_string(script));
                        this.voice_note = Some("Setup commands copied.".into());
                        cx.notify();
                    }))),
                t,
            ));
        let speak = div()
            .child(row(
                "Jarvis spoken mode",
                "Automatically speaks each finished agent reply aloud. Can also be toggled from the composer bar.",
                kit::switch("set-jarvis", jarvis, t).on_click(cx.listener(move |this, _, _, cx| {
                    this.set("jarvis", if jarvis { "false" } else { "true" }, cx);
                })),
                t,
            ))
            .child(row(
                "Spoken voice persona",
                "On-device English neural voice presets",
                div().flex().gap(px(6.0)).children(["Jasper", "Bella", "Luna", "Bruno"].into_iter().map(|name| {
                    self.choice(
                        SharedString::from(format!("tts-{name}")),
                        name,
                        spoken == name,
                        "tts_voice",
                        name,
                        t,
                        cx,
                    )
                })),
                t,
            ))
            .child(row(
                "Text-to-speech backend",
                "NEXI runs locally on your machine; Fish Audio streams cloud voices with your API key.",
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(self.choice("tts-nexi", "NEXI Local", tts != "fish", "tts_backend", "nexi", t, cx))
                    .child(self.choice("tts-fish", "Fish Audio", tts == "fish", "tts_backend", "fish", t, cx)),
                t,
            ))
            .child(row(
                "Fish Audio API key",
                "Stored locally in Dove settings. Leave blank to use on-device NEXI TTS.",
                field(&self.fish_key, 220.0, t),
                t,
            ))
            .child(row(
                "NEXI TTS runtime",
                "Kitten Nano (~25 MB weights), loaded on demand only while speaking.",
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(6.0))
                    .child(div().max_w(px(280.0)).text_size(px(12.0)).text_color(t.text_muted).child(nexi.tts_line()))
                    .child(ui::button_ghost("tts-test", "Test voice", t).on_click(cx.listener(|this, _, _, cx| {
                        let engine = this.engine.clone();
                        this.voice_note = Some("Speaking…".into());
                        cx.spawn(async move |this, cx| {
                            let result = cx
                                .background_spawn(async move {
                                    crate::jarvis::speak_now(&engine, "Dove is listening.".into())
                                })
                                .await;
                            let _ = this.update(cx, |s, cx| {
                                s.voice_note = Some(match result {
                                    Ok(()) => "Played a sample.".into(),
                                    Err(e) => e,
                                });
                                cx.notify();
                            });
                        })
                        .detach();
                        cx.notify();
                    }))),
                t,
            ))
            .when_some(self.voice_note.clone(), |d, note| {
                d.child(row(
                    "Voice check status",
                    "",
                    div().max_w(px(280.0)).text_size(px(12.0)).text_color(t.text_muted).child(note),
                    t,
                ))
            });
        stack()
            .child(card("On-Device Dictation (STT)", t, body))
            .child(card("Jarvis Spoken Mode (TTS)", t, speak))
            .into_any_element()
    }

    fn choice(
        &self,
        id: impl Into<gpui::ElementId>,
        label: &'static str,
        on: bool,
        key: &'static str,
        value: &'static str,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(id)
            .h(px(28.0))
            .px(px(11.0))
            .flex()
            .items_center()
            .rounded_full()
            .cursor_pointer()
            .text_size(px(12.5))
            .font_weight(if on { gpui::FontWeight::MEDIUM } else { gpui::FontWeight::NORMAL })
            .border_1()
            .border_color(if on { t.accent.opacity(0.45) } else { t.border })
            .bg(if on { t.accent.opacity(0.14) } else { t.wash(0.04) })
            .text_color(if on { t.text } else { t.text_muted })
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.set(key, value, cx)))
            .into_any_element()
    }

    fn keyboard(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let store = self.engine.store.clone();
        let list = div()
            .flex()
            .flex_col()
            .children(crate::keymap::shortcuts().into_iter().map(|s| {
                let key = crate::keymap::binding(&store, &s);
                let recording = self.recording == Some(s.id);
                let custom = key != s.default;
                let id = s.id;
                let default = s.default;
                row(
                    s.label,
                    if custom { "Customized shortcut" } else { "" },
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .when(custom && !recording, |d| d.child(kit::badge("Modified", t.accent)))
                        .child(if recording {
                            div()
                                .px(px(10.0))
                                .h(px(24.0))
                                .flex()
                                .items_center()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(t.accent)
                                .bg(t.accent.opacity(0.10))
                                .text_size(px(12.0))
                                .text_color(t.accent)
                                .child("Press keys… (Esc cancels)")
                                .into_any_element()
                        } else {
                            kit::kbd(&crate::keymap::pretty(&key), t).into_any_element()
                        })
                        .child(ui::button_ghost(SharedString::from(format!("rec-{id}")), "Rebind", t).on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.recording = Some(id);
                                window.focus(&this.focus, cx);
                                cx.notify();
                            }),
                        ))
                        .when(custom, |d| {
                            d.child(ui::button_ghost(SharedString::from(format!("reset-{id}")), "Reset", t).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    crate::keymap::rebind(&this.engine.store, id, default, cx);
                                    cx.notify();
                                }),
                            ))
                        }),
                    t,
                )
            }));
        stack().child(card("Keyboard Shortcuts", t, list)).into_any_element()
    }

    fn notifications(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let on = self.flag(crate::notify::NOTIFICATIONS, true);
        let sound = self.flag(crate::notify::SOUND, false);
        let keep = self.flag(crate::notify::KEEP_RUNNING, false);
        let body = div()
            .child(row(
                "System desktop notifications",
                "Alert when an agent awaits approval, finishes a turn, or encounters an error while Dove is unfocused",
                kit::switch("set-notify", on, t).on_click(cx.listener(move |this, _, _, cx| {
                    this.set(crate::notify::NOTIFICATIONS, if on { "false" } else { "true" }, cx)
                })),
                t,
            ))
            .child(row(
                "Completion sound",
                "Play a subtle chime when a background agent completes its task",
                kit::switch("set-sound", sound, t).on_click(cx.listener(move |this, _, _, cx| {
                    this.set(crate::notify::SOUND, if sound { "false" } else { "true" }, cx)
                })),
                t,
            ))
            .child(row(
                "Keep running in background",
                if crate::notify::tray_supported() {
                    "Closing the window keeps Dove and scheduled automations running in the system tray"
                } else {
                    "Closing the window minimizes Dove so scheduled automations continue running"
                },
                kit::switch("set-keep", keep, t).on_click(cx.listener(move |this, _, window, cx| {
                    crate::notify::set_keep_running(&this.engine, !keep, window, cx);
                    cx.notify();
                })),
                t,
            ));
        stack().child(card("Alerts & Background Daemon", t, body)).into_any_element()
    }

    fn data(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let root = self.engine.store.root().to_path_buf();
        let root_str = root.display().to_string();
        let root_copy = root_str.clone();
        let size = self.data_size.map(|b| format!("{:.1} MB", b as f64 / 1e6)).unwrap_or_else(|| "measuring…".into());
        let body = div()
            .child(row(
                "Workspace & transcript storage",
                "SQLite store containing projects, threads, transcripts, settings, and downloaded voice weights",
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(kit::badge(size, t.accent))
                    .child(
                        ui::button_ghost("open-data", "Open folder", t)
                            .child(icon(icons::ARROW_UP_RIGHT).size(px(12.0)).text_color(t.text_faint))
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_with_system(&root))),
                    ),
                t,
            ))
            .child(row(
                "Storage path (DOVE_HOME)",
                "Set the DOVE_HOME environment variable to relocate Dove's data directory",
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                div()
                    .text_size(px(12.0))
                    .font_family(t.font_mono.clone())
                            .text_color(t.text_muted)
                            .max_w(px(280.0))
                            .truncate()
                            .child(root_str),
                    )
                    .child(ui::button_ghost("copy-data-path", "Copy", t).on_click(cx.listener(
                        move |_, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(root_copy.clone()));
                        },
                    ))),
                t,
            ))
            .child(row(
                "Telemetry & privacy",
                "All transcripts, credentials, and settings stay on your machine",
                kit::badge("Zero telemetry", t.success),
                t,
            ));
        stack().child(card("Local Storage & Privacy", t, body)).into_any_element()
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
            ("Connected · Phone paired", t.success)
        } else if status.signed_in {
            ("Connecting…", t.busy)
        } else if status.running {
            ("Sign in to connect", t.warning)
        } else {
            ("Starting…", t.busy)
        };
        let account_button = if status.signed_in {
            ui::button_ghost("remote-sign-out", "Sign out", t).on_click({
                let remote = remote.clone();
                move |_, _, cx| remote.update(cx, |r, cx| r.sign_out(cx))
            })
        } else {
            ui::button_primary("remote-sign-in", "Sign in with Woozlit", t).on_click({
                let remote = remote.clone();
                move |_, _, cx| remote.update(cx, |r, cx| r.sign_in(cx))
            })
        };
        let body = div()
            .child(row(
                "Enable remote control",
                "Watch and steer this computer's active agent threads from the Dove mobile app. While off, no relay connection is opened.",
                kit::switch("set-remote", enabled, t).on_click({
                    let remote = remote.clone();
                    move |_, _, cx| remote.update(cx, |r, cx| r.set_enabled(!enabled, cx))
                }),
                t,
            ))
            .child(row(
                "Woozlit account",
                "Sign in on your phone with the same account to pair automatically",
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(div().text_size(px(12.5)).text_color(t.text_muted).child(account))
                    .child(account_button),
                t,
            ))
            .child(row("Connection status", "", kit::badge(state_label, state_col), t))
            .when_some(status.device_id.clone(), |d, id| {
                d.child(row(
                    "Device ID",
                    "",
                    div().font_family(t.font_mono.clone()).text_color(t.text_muted).text_size(px(12.0)).child(id),
                    t,
                ))
            })
            .when_some(status.error.clone(), |d, e| {
                d.child(row("Last error", "", div().text_color(t.danger).text_size(px(12.0)).child(e), t))
            });
        stack().child(card("Mobile Companion", t, body)).into_any_element()
    }

    fn about(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let body = div()
            .child(row(
                "Application version",
                "Native Rust + GPUI desktop workspace for multi-harness coding agents",
                kit::badge(format!("v{}", env!("CARGO_PKG_VERSION")), t.accent),
                t,
            ))
            .child(row(
                "License",
                "Apache-2.0 open-source license; third-party notices ship with the application",
                div().text_size(px(12.5)).text_color(t.text_muted).child("Apache-2.0"),
                t,
            ))
            .child(row(
                "Source repository",
                "Browse source code, file issues, or contribute on GitHub",
                ui::button_ghost("about-src", "Open GitHub", t)
                    .child(icon(icons::ARROW_UP_RIGHT).size(px(12.0)).text_color(t.text_faint))
                    .on_click(cx.listener(|_, _, _, cx| cx.open_url("https://github.com/dove-ade/dove"))),
                t,
            ));
        stack().child(card("Dove Desktop", t, body)).into_any_element()
    }

    /// Optional status badge shown on the right side of a navigation rail item.
    fn nav_badge(&self, s: Section, t: &Theme, cx: &App) -> Option<AnyElement> {
        match s {
            Section::Agents => {
                let count = self.engine.installed().len();
                if count > 0 {
                    Some(
                        div()
                            .px(px(5.0))
                            .h(px(16.0))
                            .rounded(px(4.0))
                            .bg(t.wash(0.08))
                            .flex()
                            .items_center()
                            .text_size(px(10.5))
                            .text_color(t.text_muted)
                            .child(format!("{count}"))
                            .into_any_element(),
                    )
                } else {
                    None
                }
            }
            Section::Team => {
                let count = self.engine.store.personas().map(|v| v.len()).unwrap_or(0);
                if count > 0 {
                    Some(
                        div()
                            .px(px(5.0))
                            .h(px(16.0))
                            .rounded(px(4.0))
                            .bg(t.accent.opacity(0.14))
                            .flex()
                            .items_center()
                            .text_size(px(10.5))
                            .text_color(t.accent)
                            .child(format!("{count}"))
                            .into_any_element(),
                    )
                } else {
                    None
                }
            }
            Section::Voice => {
                let installed = VoiceModel::global(cx)
                    .map(|m| matches!(m.read(cx).state, InstallState::Installed))
                    .unwrap_or(false);
                if installed || self.flag("jarvis", false) {
                    Some(ui::dot(t.success, 6.0).into_any_element())
                } else {
                    None
                }
            }
            Section::Remote => {
                let connected = crate::remote::Remote::global(cx)
                    .map(|r| {
                        let r = r.read(cx);
                        r.status.connected && r.enabled()
                    })
                    .unwrap_or(false);
                if connected {
                    Some(ui::dot(t.success, 6.0).into_any_element())
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

fn field(input: &Entity<InputState>, w: f32, t: &Theme) -> impl IntoElement {
    div()
        .w(px(w))
        .h(px(30.0))
        .px(px(9.0))
        .flex()
        .items_center()
        .rounded(px(8.0))
        .bg(t.wash(0.05))
        .border_1()
        .border_color(t.border)
        .text_size(px(13.0))
        .child(Input::new(input))
}

fn device_row(id: impl Into<gpui::ElementId>, name: &str, selected: bool, t: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(28.0))
        .px(px(8.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .text_size(px(13.0))
        .hover(|s| s.bg(t.wash(0.06)))
        .child(div().w(px(14.0)).when(selected, |d| d.child(icon(icons::CHECK).size(px(13.0)).text_color(t.accent))))
        .child(div().max_w(px(260.0)).truncate().child(name.to_string()))
}

/// One setting: label and hint on the left, its control on the right.
/// Rows draw a hairline above themselves; a [`card`] hides the first one.
fn row(label: &'static str, hint: &'static str, control: impl IntoElement, t: &Theme) -> gpui::Div {
    div()
        .px(px(16.0))
        .py(px(13.5))
        .flex()
        .items_center()
        .gap(px(16.0))
        .border_t_1()
        .border_color(t.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.5))
                .child(
                    div()
                        .text_size(px(13.5))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(t.text)
                        .child(label),
                )
                .when(!hint.is_empty(), |d| d.child(div().text_size(px(12.0)).text_color(t.text_faint).child(hint))),
        )
        .child(control)
}

/// A titled plate of rows.
fn card(title: &'static str, t: &Theme, rows: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .when(!title.is_empty(), |d| d.child(kit::section_label(title, t)))
        .child(
            div()
                .w_full()
                .rounded(px(12.0))
                .border_1()
                .border_color(t.border)
                .bg(t.wash(0.022))
                .overflow_hidden()
                .child(div().mt(px(-1.0)).child(rows)),
        )
}

/// Cards, stacked with room between them.
fn stack() -> gpui::Div {
    div().flex().flex_col().gap(px(20.0))
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

impl Render for SettingsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(cx).clone();
        let query = self.query.trim().to_string();
        let has_query = !query.is_empty();

        let search_box = div()
            .h(px(32.0))
            .px(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(t.border)
            .bg(t.wash(0.04))
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(icon(icons::MAGNIFER).size(px(13.5)).text_color(t.text_faint))
            .child(div().flex_1().min_w_0().text_size(px(12.5)).child(Input::new(&self.search)))
            .when(has_query, |d| {
                d.child(
                div()
                        .id("set-search-clear")
                        .size(px(18.0))
                        .rounded(px(4.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|s| s.bg(t.wash(0.08)))
                        .child(icon(icons::CLOSE).size(px(10.0)).text_color(t.text_faint))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.search.update(cx, |s, cx| s.set_value(String::new(), window, cx));
                            this.query.clear();
                            cx.notify();
                        })),
                )
            });

        let nav = div()
            .w(px(232.0))
            .flex_none()
                    .flex()
                    .flex_col()
            .gap(px(14.0))
            .pt(px(26.0))
            .child(
                div()
                    .px(px(6.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(t.text)
                            .child("Settings"),
                    )
                    .child(kit::badge("Live", t.text_faint)),
            )
            .child(search_box)
            .children(Section::GROUPS.into_iter().filter_map(|(group, sections)| {
                let visible: Vec<Section> = sections
                    .iter()
                    .copied()
                    .filter(|s| s.matches(&query))
                    .collect();
                if visible.is_empty() {
                    return None;
                }
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                    .child(
                        div()
                            .px(px(10.0))
                                .pb(px(3.0))
                                .text_size(px(11.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(t.text_faint)
                            .child(group),
                    )
                        .children(visible.into_iter().map(|s| {
                        let selected = s == self.section;
                            let badge = self.nav_badge(s, &t, cx);
                        div()
                            .id(SharedString::from(format!("set-nav-{}", s.id())))
                                .h(px(31.0))
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .rounded(px(8.0))
                            .cursor_pointer()
                            .text_size(px(13.0))
                                .border_1()
                                .when(selected, |d| {
                                    d.bg(t.accent.opacity(0.12))
                                        .border_color(t.accent.opacity(0.32))
                                        .text_color(t.text)
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                })
                            .when(!selected, |d| {
                                    d.border_color(gpui::transparent_black())
                                        .text_color(t.text_muted)
                                        .hover(|st| st.bg(t.wash(0.05)).text_color(t.text))
                                })
                                .child(
                                    icon(s.glyph())
                                        .size(px(14.5))
                                        .text_color(if selected { t.accent } else { t.text_muted }),
                                )
                                .child(div().flex_1().truncate().child(s.label()))
                                .when_some(badge, |d, b| d.child(b))
                            .on_click(cx.listener(move |this, _, _, cx| this.select(s, cx)))
                        })),
                )
            }));

        let hero = self.section_hero(&t, cx);
        let body = match self.section {
            Section::General => self.general(&t, cx),
            Section::Appearance => self.appearance(&t, cx),
            Section::Agents => self.agents(&t, cx),
            Section::Team => self.team(&t, cx),
            Section::Canvas => self.canvas(&t, cx),
            Section::Composer => self.composer(&t, cx),
            Section::Voice => self.voice(&t, cx),
            Section::Keyboard => self.keyboard(&t, cx),
            Section::Notifications => self.notifications(&t, cx),
            Section::Remote => self.remote(&t, cx),
            Section::Integrations => self.integrations.clone().into_any_element(),
            Section::Data => self.data(&t, cx),
            Section::About => self.about(&t, cx),
        };
        div()
            .id("settings")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .overflow_y_scroll()
            .flex()
            .justify_center()
            .child(
                    div()
                    .w_full()
                    .max_w(px(1040.0))
                    .px(px(32.0))
                    .pb(px(56.0))
                        .flex()
                    .gap(px(32.0))
                    .child(nav)
                        .child(
                            div()
                            .flex_1()
                            .min_w_0()
                            .pt(px(26.0))
                                .flex()
                                .flex_col()
                            .gap(px(20.0))
                            .child(hero)
                        .child(body),
                ),
            )
    }
}
