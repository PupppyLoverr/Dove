//! Settings, with left navigation: General · Appearance · Composer · Voice ·
//! Keyboard · Notifications · Data · About. Everything persists in the
//! store's settings table and applies live.

use std::sync::Arc;

use dove_core::Engine;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    px,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::kit;
use crate::theme::{Accent, Theme};
use crate::ui;
use crate::voice::{InstallState, VoiceModel};
use crate::wallpaper::Effect;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Section {
    General,
    Appearance,
    Composer,
    Voice,
    Keyboard,
    Notifications,
    Remote,
    Data,
    About,
}

impl Section {
    const ALL: [Section; 9] = [
        Section::General,
        Section::Appearance,
        Section::Composer,
        Section::Voice,
        Section::Keyboard,
        Section::Notifications,
        Section::Remote,
        Section::Data,
        Section::About,
    ];

    fn label(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Appearance => "Appearance",
            Section::Composer => "Composer",
            Section::Voice => "Voice",
            Section::Keyboard => "Keyboard",
            Section::Notifications => "Notifications",
            Section::Remote => "Remote control",
            Section::Data => "Data",
            Section::About => "About",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Section::General => icons::SETTINGS,
            Section::Appearance => icons::PALETTE_SEARCH,
            Section::Composer => icons::PEN,
            Section::Voice => icons::MICROPHONE,
            Section::Keyboard => icons::KEYBOARD,
            Section::Notifications => icons::BELL,
            Section::Remote => icons::SMARTPHONE,
            Section::Data => icons::HARD_DRIVE,
            Section::About => icons::INFO_CIRCLE,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Section::General => "general",
            Section::Appearance => "appearance",
            Section::Composer => "composer",
            Section::Voice => "voice",
            Section::Keyboard => "keyboard",
            Section::Notifications => "notifications",
            Section::Remote => "remote",
            Section::Data => "data",
            Section::About => "about",
        }
    }
}

pub struct SettingsPage {
    engine: Arc<Engine>,
    section: Section,
    prefix: Entity<InputState>,
    clone_dir: Entity<InputState>,
    /// The shortcut being recorded.
    recording: Option<&'static str>,
    devices: Vec<(String, String)>,
    data_size: Option<u64>,
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

impl SettingsPage {
    pub fn new(engine: Arc<Engine>, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        let mut subs = vec![s1, s2];
        if let Some(model) = VoiceModel::global(cx) {
            subs.push(cx.observe(&model, |_, _, cx| cx.notify()));
        }
        if let Some(remote) = crate::remote::Remote::global(cx) {
            subs.push(cx.observe(&remote, |_, _, cx| cx.notify()));
        }
        Self {
            engine,
            section: Section::General,
            prefix,
            clone_dir,
            recording: None,
            devices: Vec::new(),
            data_size: None,
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

    // ------------------------------------------------------------ sections

    fn general(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let restore = self.flag("restore_tabs", true);
        div()
            .child(row(
                "Reopen thread tabs",
                "Open the threads you had open last time",
                kit::switch("set-restore", restore, t).on_click(cx.listener(move |this, _, _, cx| {
                    this.set("restore_tabs", if restore { "false" } else { "true" }, cx)
                })),
                t,
            ))
            .child(row(
                "Run each new thread in a worktree",
                "Its own branch and folder, so threads never collide (on by default)",
                kit::switch("set-worktree", self.flag("use_worktree", true), t).on_click(cx.listener({
                    let on = self.flag("use_worktree", true);
                    move |this, _, _, cx| this.set("use_worktree", if on { "false" } else { "true" }, cx)
                })),
                t,
            ))
            .child(row("Branch prefix", "New thread branches start with this", field(&self.prefix, 180.0, t), t))
            .child(row(
                "Clone folder",
                "Where \"Clone from URL\" puts repositories",
                field(&self.clone_dir, 280.0, t),
                t,
            ))
            .into_any_element()
    }

    fn appearance(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.get("appearance").unwrap_or_else(|| "system".into());
        let accent = Accent::from_id(&self.get("accent").unwrap_or_default());
        let frosted = self.flag("frosted", true);
        let wall = self.get("wallpaper_path").filter(|p| !p.is_empty());
        let effect = Effect::from_id(&self.get("wallpaper_effect").unwrap_or_default());
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
        let swatches = div().flex().gap(px(8.0)).children(Accent::ALL.into_iter().map(|a| {
            let color = Theme::new(t.appearance, a, false).accent;
            div()
                .id(SharedString::from(format!("accent-{}", a.id())))
                .size(px(22.0))
                .rounded_full()
                .bg(color)
                .cursor_pointer()
                .when(a == accent, |d| d.border_2().border_color(t.text))
                .on_click(cx.listener(move |this, _, _, cx| this.set("accent", a.id(), cx)))
        }));
        let effects: Vec<&str> = Effect::ALL.iter().map(|e| e.label()).collect();
        let effect_ix = Effect::ALL.iter().position(|e| *e == effect).unwrap_or(0);
        let effect_control = kit::segmented("set-effect", &effects, effect_ix, t, {
            let weak = cx.weak_entity();
            move |i, _, cx| {
                let _ = weak.update(cx, |s, cx| s.set("wallpaper_effect", Effect::ALL[i].id(), cx));
            }
        });
        div()
            .child(row("Theme", "Follow the system or pick one", appearance, t))
            .child(row("Accent", "", swatches, t))
            .child(row(
                "Frosted glass",
                "Blur the desktop behind the window (Windows 11 / macOS)",
                kit::switch("set-frosted", frosted, t).on_click(
                    cx.listener(move |this, _, _, cx| this.set("frosted", if frosted { "false" } else { "true" }, cx)),
                ),
                t,
            ))
            .child(row(
                "New-thread background",
                "An image behind the composer on the new-thread page",
                div()
                    .flex()
                    .gap(px(6.0))
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
            .child(row("Background effect", "", effect_control, t))
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
        div()
            .child(row(
                "Default permission level",
                "New threads start here; Bypass is chosen per thread and confirmed per project",
                default_mode,
                t,
            ))
            .child(row(
                "Projects allowed to bypass",
                if bypassed.is_empty() {
                    "None yet"
                } else {
                    "Agents may run without approvals here when you pick Bypass"
                },
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(t.text_muted)
                            .max_w(px(260.0))
                            .truncate()
                            .child(bypassed.join(", ")),
                    )
                    .when(!bypassed.is_empty(), |d| {
                        d.child(ui::button_ghost("bypass-reset", "Reset", t).on_click(cx.listener(|this, _, _, cx| {
                            for p in this.engine.store.projects().unwrap_or_default() {
                                let _ = this.engine.store.set_setting(&format!("bypass_ok.{}", p.id), "0");
                            }
                            cx.notify();
                        })))
                    }),
                t,
            ))
            .into_any_element()
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
        div()
            .child(row(
                "Local dictation",
                "NVIDIA Parakeet TDT 0.6B v3, on this device: audio never leaves it. Downloaded on demand and checked file by file (SHA-256).",
                control,
                t,
            ))
            .child(row("Microphone", "", devices, t))
            .child(row("How to dictate", "Hold the mic in the composer (or the terminal) to talk; click it to toggle. Text is inserted, never sent.", div(), t))
            .into_any_element()
    }

    fn keyboard(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let store = self.engine.store.clone();
        div()
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
                    "",
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(if recording {
                            div()
                                .px(px(8.0))
                                .h(px(22.0))
                                .flex()
                                .items_center()
                                .rounded(px(5.0))
                                .border_1()
                                .border_color(t.accent)
                                .text_size(px(12.0))
                                .text_color(t.accent)
                                .child("Press keys… (Esc cancels)")
                                .into_any_element()
                        } else {
                            kit::kbd(&crate::keymap::pretty(&key), t).into_any_element()
                        })
                        .child(ui::button_ghost(SharedString::from(format!("rec-{id}")), "Change", t).on_click(
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
            }))
            .into_any_element()
    }

    fn notifications(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let on = self.flag(crate::notify::NOTIFICATIONS, true);
        let sound = self.flag(crate::notify::SOUND, false);
        let keep = self.flag(crate::notify::KEEP_RUNNING, false);
        div()
            .child(row(
                "System notifications",
                "When a thread needs you, finishes or fails while Dove is in the background",
                kit::switch("set-notify", on, t).on_click(cx.listener(move |this, _, _, cx| {
                    this.set(crate::notify::NOTIFICATIONS, if on { "false" } else { "true" }, cx)
                })),
                t,
            ))
            .child(row(
                "Sound",
                "",
                kit::switch("set-sound", sound, t).on_click(cx.listener(move |this, _, _, cx| {
                    this.set(crate::notify::SOUND, if sound { "false" } else { "true" }, cx)
                })),
                t,
            ))
            .child(row(
                "Keep running in the background",
                if crate::notify::tray_supported() {
                    "Closing the window keeps Dove (and automations) running, with a tray icon to reopen or quit"
                } else {
                    "Closing the window minimizes Dove so automations keep running"
                },
                kit::switch("set-keep", keep, t).on_click(cx.listener(move |this, _, window, cx| {
                    crate::notify::set_keep_running(&this.engine, !keep, window, cx);
                    cx.notify();
                })),
                t,
            ))
            .into_any_element()
    }

    fn data(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let root = self.engine.store.root().to_path_buf();
        let size = self.data_size.map(|b| format!("{:.1} MB", b as f64 / 1e6)).unwrap_or_else(|| "measuring…".into());
        div()
            .child(row(
                "Data folder",
                "Projects, threads, transcripts, settings and the voice model",
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().text_size(px(12.5)).text_color(t.text_muted).child(size))
                    .child(
                        ui::button_ghost("open-data", "Open folder", t)
                            .child(icon(icons::ARROW_UP_RIGHT).size(px(12.0)).text_color(t.text_faint))
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_with_system(&root))),
                    ),
                t,
            ))
            .child(row(
                "Location",
                "Set DOVE_HOME to move it",
                div()
                    .text_size(px(12.0))
                    .font_family(t.font_mono.clone())
                    .text_color(t.text_faint)
                    .child(self.engine.store.root().display().to_string()),
                t,
            ))
            .child(row("Telemetry", "Dove sends no telemetry in this build", kit::badge("Off", t.text_faint), t))
            .into_any_element()
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
        let state = if !enabled {
            "Off"
        } else if status.connected {
            "Connected, phone can see this computer"
        } else if status.signed_in {
            "Connecting"
        } else if status.running {
            "Sign in to connect"
        } else {
            "Starting"
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
        div()
            .child(row(
                "Remote control",
                "Watch and drive this computer's sessions from the Dove phone app. While off, nothing leaves this machine.",
                kit::switch("set-remote", enabled, t).on_click({
                    let remote = remote.clone();
                    move |_, _, cx| remote.update(cx, |r, cx| r.set_enabled(!enabled, cx))
                }),
                t,
            ))
            .child(row(
                "Account",
                "Sign in on the phone with the same Woozlit account",
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(div().text_color(t.text_muted).child(account))
                    .child(account_button),
                t,
            ))
            .child(row("Status", "", div().text_color(t.text_muted).child(state), t))
            .when_some(status.device_id.clone(), |d, id| {
                d.child(row("Device", "", div().text_color(t.text_muted).text_size(px(12.0)).child(id), t))
            })
            .when_some(status.error.clone(), |d, e| {
                d.child(row("Last error", "", div().text_color(t.text_muted).child(e), t))
            })
            .into_any_element()
    }

    fn about(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .child(row("Version", "", div().text_color(t.text_muted).child(env!("CARGO_PKG_VERSION")), t))
            .child(row(
                "License",
                "Apache-2.0; third-party notices ship with the app",
                div().text_color(t.text_muted).child("Apache-2.0"),
                t,
            ))
            .child(row(
                "Source",
                "",
                ui::button_ghost("about-src", "Open repository", t)
                    .on_click(cx.listener(|_, _, _, cx| cx.open_url("https://github.com/dove-ade/dove"))),
                t,
            ))
            .into_any_element()
    }
}

fn field(input: &Entity<InputState>, w: f32, t: &Theme) -> impl IntoElement {
    div()
        .w(px(w))
        .h(px(30.0))
        .px(px(8.0))
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

fn row(label: &'static str, hint: &'static str, control: impl IntoElement, t: &Theme) -> gpui::Div {
    div()
        .py(px(14.0))
        .flex()
        .items_center()
        .gap(px(16.0))
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(div().text_size(px(14.0)).child(label))
                .when(!hint.is_empty(), |d| d.child(div().text_size(px(12.0)).text_color(t.text_faint).child(hint))),
        )
        .child(control)
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
        let nav = div().w(px(200.0)).flex_none().flex().flex_col().gap(px(2.0)).pt(px(56.0)).children(
            Section::ALL.into_iter().map(|s| {
                let selected = s == self.section;
                div()
                    .id(SharedString::from(format!("set-nav-{}", s.id())))
                    .h(px(32.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .text_size(px(13.5))
                    .when(selected, |d| d.bg(t.wash(0.10)).text_color(t.text))
                    .when(!selected, |d| d.text_color(t.text_muted).hover(|s| s.bg(t.wash(0.05))))
                    .child(icon(s.glyph()).size(px(15.0)))
                    .child(s.label())
                    .on_click(cx.listener(move |this, _, _, cx| this.select(s, cx)))
            }),
        );
        let body = match self.section {
            Section::General => self.general(&t, cx),
            Section::Appearance => self.appearance(&t, cx),
            Section::Composer => self.composer(&t, cx),
            Section::Voice => self.voice(&t, cx),
            Section::Keyboard => self.keyboard(&t, cx),
            Section::Notifications => self.notifications(&t, cx),
            Section::Remote => self.remote(&t, cx),
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
                div().w_full().max_w(px(960.0)).px(px(28.0)).pb(px(48.0)).flex().gap(px(28.0)).child(nav).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .pt(px(26.0))
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(px(24.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(self.section.label()),
                        )
                        .child(body),
                ),
            )
    }
}
