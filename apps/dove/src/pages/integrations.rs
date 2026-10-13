//! Settings → Integrations: Dove for Chrome pairing, Composio (BYOK), Paper,
//! and computer use through the open-source `cua-driver`.
//!
//! The choices live in `<dove data>/integrations.json` (mode 0600, since it
//! holds the user's Composio key). `dove --mcp-browser` reads that file when
//! a harness starts it and aggregates the enabled servers into Dove's own
//! MCP server, so every harness Dove starts gets them without per-harness
//! config.

use std::path::PathBuf;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use gpui_base::input::{Input, InputEvent, InputState};
use serde::{Deserialize, Serialize};

use crate::kit;
use crate::theme::Theme;
use crate::ui;

/// Composio's shared MCP endpoint (Composio Connect, consumer keys).
pub const COMPOSIO_CONNECT_URL: &str = "https://connect.composio.dev/mcp";
/// REST v3.1 session creation, which returns a toolkit-scoped MCP URL.
pub const COMPOSIO_SESSION_URL: &str = "https://backend.composio.dev/api/v3.1/tool_router/session";
/// Paper Desktop serves MCP here while it is open.
pub const PAPER_MCP_URL: &str = "http://127.0.0.1:29979/mcp";
/// The cua-driver release Dove is written against (same pin as Cosmos).
pub const CUA_VERSION: &str = "0.34.0";

/// Toolkits offered as one-click picks; any other slug can be typed.
const TOOLKITS: [(&str, &str); 10] = [
    ("github", "GitHub"),
    ("gmail", "Gmail"),
    ("googlecalendar", "Calendar"),
    ("googledrive", "Drive"),
    ("slack", "Slack"),
    ("notion", "Notion"),
    ("linear", "Linear"),
    ("jira", "Jira"),
    ("figma", "Figma"),
    ("discord", "Discord"),
];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Composio {
    pub enabled: bool,
    pub api_key: String,
    /// Toolkit slugs to allow; empty means Composio's unrestricted default.
    pub toolkits: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Toggle {
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    pub composio: Composio,
    pub paper: Toggle,
    pub computer: Toggle,
}

/// Which credential the user pasted, by Composio's documented prefixes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposioKey {
    /// `ak_…`, sent as `x-api-key`; sessions can be toolkit-scoped.
    Project,
    /// `uak_…`, sent as `x-user-api-key`; sessions can be toolkit-scoped.
    User,
    /// Anything else: a Composio Connect consumer key (`x-consumer-api-key`).
    Consumer,
}

impl ComposioKey {
    pub fn of(key: &str) -> Self {
        if key.starts_with("uak_") {
            Self::User
        } else if key.starts_with("ak_") {
            Self::Project
        } else {
            Self::Consumer
        }
    }

    pub fn header(self) -> &'static str {
        match self {
            Self::Project => "x-api-key",
            Self::User => "x-user-api-key",
            Self::Consumer => "x-consumer-api-key",
        }
    }
}

/// The data folder: `DOVE_DATA` when the bridge is started by Dove, else
/// `~/.dove` (or `DOVE_HOME`).
pub fn data_root() -> PathBuf {
    std::env::var_os("DOVE_DATA").map(PathBuf::from).unwrap_or_else(dove_core::store::default_root)
}

pub fn config_path() -> PathBuf {
    data_root().join("integrations.json")
}

pub fn load() -> Config {
    std::fs::read_to_string(config_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(config: &Config) -> std::io::Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = serde_json::to_string_pretty(config).unwrap_or_default();
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
        f.write_all(body.as_bytes())?;
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, body)
    }
}

/// The installed `cua-driver` binary, if any: `PATH` first, then the
/// locations its installers use.
pub fn cua_driver() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "cua-driver.exe" } else { "cua-driver" };
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from) {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".cua/bin"));
        dirs.push(home.join(".cargo/bin"));
    }
    dirs.extend(
        ["/opt/homebrew/bin", "/usr/local/bin", "/Applications/CuaDriver.app/Contents/MacOS"].map(PathBuf::from),
    );
    dirs.into_iter().map(|d| d.join(exe)).find(|p| p.is_file())
}

/// `cua-driver --version`, trimmed.
pub fn cua_version(bin: &std::path::Path) -> Option<String> {
    let out = std::process::Command::new(bin).arg("--version").output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// Whether Paper Desktop's MCP server is listening.
pub fn paper_running() -> bool {
    std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], 29979).into(), Duration::from_millis(250)).is_ok()
}

#[derive(Clone, Default)]
struct Probe {
    cua: Option<PathBuf>,
    cua_version: Option<String>,
    /// The driver's own `health_report` summary, after "Check permissions".
    cua_health: Option<String>,
    paper: bool,
}

pub struct IntegrationsView {
    config: Config,
    api_key: Entity<InputState>,
    toolkits: Entity<InputState>,
    probe: Probe,
    checking: bool,
    copied: bool,
    _subs: Vec<Subscription>,
}

impl IntegrationsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = load();
        let style = ui::input_style(Theme::of(cx));
        let api_key = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Paste your Composio API key").masked(true);
            s.set_editor_style(style.clone());
            s.set_value(config.composio.api_key.clone(), window, cx);
            s
        });
        let custom: Vec<&String> =
            config.composio.toolkits.iter().filter(|k| !TOOLKITS.iter().any(|(slug, _)| slug == k)).collect();
        let custom = custom.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
        let toolkits = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Other toolkit slugs, comma separated");
            s.set_editor_style(style);
            s.set_value(custom, window, cx);
            s
        });
        let subs = vec![
            cx.subscribe(&api_key, |this, s, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change | InputEvent::Blur) {
                    this.config.composio.api_key = s.read(cx).value().trim().to_string();
                    this.persist(cx);
                }
            }),
            cx.subscribe(&toolkits, |this, s, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change | InputEvent::Blur) {
                    let typed: Vec<String> = s
                        .read(cx)
                        .value()
                        .split([',', ' '])
                        .map(|t| t.trim().to_ascii_lowercase())
                        .filter(|t| {
                            !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                        })
                        .collect();
                    let picked =
                        this.config.composio.toolkits.iter().filter(|k| TOOLKITS.iter().any(|(s, _)| s == k)).cloned();
                    let mut all: Vec<String> = picked.chain(typed).collect();
                    all.dedup();
                    this.config.composio.toolkits = all;
                    this.persist(cx);
                }
            }),
        ];
        let mut view =
            Self { config, api_key, toolkits, probe: Probe::default(), checking: false, copied: false, _subs: subs };
        view.refresh(cx);
        // Connection state changes outside GPUI (the extension's socket); poll it.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        view
    }

    fn persist(&mut self, cx: &mut Context<Self>) {
        let _ = save(&self.config);
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move {
            let cua = cua_driver();
            let cua_version = cua.as_deref().and_then(cua_version);
            Probe { cua, cua_version, cua_health: None, paper: paper_running() }
        });
        cx.spawn(async move |this, cx| {
            let probe = task.await;
            let _ = this.update(cx, |v, cx| {
                let health = v.probe.cua_health.take();
                v.probe = Probe { cua_health: health, ..probe };
                cx.notify();
            });
        })
        .detach();
    }

    fn check_cua(&mut self, cx: &mut Context<Self>) {
        let Some(bin) = self.probe.cua.clone() else { return };
        self.checking = true;
        let task = cx.background_spawn(async move { crate::mcp_browser::cua_health(&bin) });
        cx.spawn(async move |this, cx| {
            let health = task.await;
            let _ = this.update(cx, |v, cx| {
                v.checking = false;
                v.probe.cua_health = Some(health);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

fn heading(title: &'static str, t: &Theme) -> gpui::Div {
    div()
        .pt(px(22.0))
        .pb(px(2.0))
        .text_size(px(12.0))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(t.text_faint)
        .child(title)
}

fn row(label: &'static str, hint: impl Into<SharedString>, control: impl IntoElement, t: &Theme) -> gpui::Div {
    let hint: SharedString = hint.into();
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
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(div().text_size(px(14.0)).child(label))
                .when(!hint.is_empty(), |d| d.child(div().text_size(px(12.0)).text_color(t.text_faint).child(hint))),
        )
        .child(control)
}

fn status(on: bool, label: impl Into<SharedString>, t: &Theme) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_size(px(12.5))
        .text_color(if on { t.text } else { t.text_faint })
        .child(div().size(px(6.0)).rounded_full().bg(if on { t.text } else { t.wash(0.25) }))
        .child(label.into())
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

impl Render for IntegrationsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::of(cx).clone();
        let browser = crate::browser::control::connected_browser();
        let code = crate::browser::control::pairing_code();
        let browser_label = browser
            .as_deref()
            .map(|ua| {
                if ua.contains("Edg/") {
                    "Edge"
                } else if ua.contains("Brave") {
                    "Brave"
                } else {
                    "Chrome"
                }
            })
            .map(|b| format!("Connected to {b}"))
            .unwrap_or_else(|| "Not connected".into());

        let c = self.config.composio.clone();
        let key_kind = ComposioKey::of(&c.api_key);
        let scoped = !c.api_key.is_empty() && key_kind != ComposioKey::Consumer;
        let picks =
            div().flex().flex_wrap().gap(px(14.0)).py(px(12.0)).children(TOOLKITS.iter().map(|(slug, name)| {
                let on = c.toolkits.iter().any(|k| k == slug);
                let slug = slug.to_string();
                kit::checkbox(SharedString::from(format!("int-tk-{slug}")), on, *name, &t).on_click(cx.listener(
                    move |this, _, _, cx| {
                        let list = &mut this.config.composio.toolkits;
                        if let Some(i) = list.iter().position(|k| *k == slug) {
                            list.remove(i);
                        } else {
                            list.push(slug.clone());
                        }
                        this.persist(cx);
                    },
                ))
            }));

        let cua_hint = match (&self.probe.cua, &self.probe.cua_version) {
            (Some(p), v) => format!("{} · {}", v.clone().unwrap_or_else(|| "cua-driver".into()), p.display()),
            (None, _) => format!("Install cua-driver {CUA_VERSION}+ from github.com/trycua/cua, then reopen this page"),
        };
        let paper_on = self.config.paper.enabled;
        let cua_on = self.config.computer.enabled;
        let composio_on = c.enabled;

        div()
            .flex()
            .flex_col()
            .child(heading("Dove for Chrome", &t))
            .child(row(
                "Browser extension",
                "Agents get chrome_* tools that act in your own browser, only on sites you allow there",
                status(browser.is_some(), browser_label, &t),
                &t,
            ))
            .child(row(
                "Pairing code",
                "Paste into the extension's side panel once. It stays on this machine",
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .max_w(px(220.0))
                            .truncate()
                            .text_size(px(12.0))
                            .font_family(t.font_mono.clone())
                            .text_color(t.text_faint)
                            .child(
                                code.as_deref()
                                    .map(|c| format!("{}…", &c[..c.len().min(18)]))
                                    .unwrap_or_else(|| "Turn on the agent browser to pair".into()),
                            ),
                    )
                    .when_some(code, |d, code| {
                        d.child(ui::button_ghost("int-copy", if self.copied { "Copied" } else { "Copy" }, &t).on_click(
                            cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                                this.copied = true;
                                cx.notify();
                            }),
                        ))
                    }),
                &t,
            ))
            .child(heading("Composio", &t))
            .child(row(
                "Composio tools",
                "Gmail, GitHub, Slack, Notion and 1000+ apps through your own Composio account",
                kit::switch("int-composio", composio_on, &t).on_click(cx.listener(move |this, _, _, cx| {
                    this.config.composio.enabled = !composio_on;
                    this.persist(cx);
                })),
                &t,
            ))
            .child(row(
                "API key",
                match (c.api_key.is_empty(), key_kind) {
                    (true, _) => "From composio.dev → Settings. Stored only in Dove's data folder",
                    (false, ComposioKey::Consumer) => "Composio Connect key: agents pick apps on demand",
                    (false, ComposioKey::Project) => "Project key: Dove opens a session limited to the toolkits below",
                    (false, ComposioKey::User) => "User key: Dove opens a session limited to the toolkits below",
                },
                field(&self.api_key, 260.0, &t),
                &t,
            ))
            .child(
                div()
                    .py(px(14.0))
                    .border_b_1()
                    .border_color(t.border)
                    .child(div().text_size(px(14.0)).child("Toolkits"))
                    .child(div().text_size(px(12.0)).text_color(t.text_faint).child(if scoped {
                        "None picked means every toolkit"
                    } else {
                        "Toolkit limits apply with a project (ak_) or user (uak_) key; Connect keys choose per request"
                    }))
                    .child(picks)
                    .child(field(&self.toolkits, 360.0, &t)),
            )
            .child(heading("Paper", &t))
            .child(row(
                "Paper design files",
                if self.probe.paper {
                    "Paper Desktop is open; agents can read and edit the open file"
                } else {
                    "Open Paper Desktop (paper.design) so its local MCP server is running"
                },
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.0))
                    .child(status(self.probe.paper, if self.probe.paper { "Running" } else { "Not running" }, &t))
                    .child(kit::switch("int-paper", paper_on, &t).on_click(cx.listener(move |this, _, _, cx| {
                        this.config.paper.enabled = !paper_on;
                        this.persist(cx);
                        this.refresh(cx);
                    }))),
                &t,
            ))
            .child(heading("Computer use", &t))
            .child(row(
                "Cua Driver",
                cua_hint,
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.0))
                    .child(status(
                        self.probe.cua.is_some(),
                        if self.probe.cua.is_some() { "Installed" } else { "Not installed" },
                        &t,
                    ))
                    .child(kit::switch("int-cua", cua_on, &t).on_click(cx.listener(move |this, _, _, cx| {
                        this.config.computer.enabled = !cua_on;
                        this.persist(cx);
                    }))),
                &t,
            ))
            .child(row(
                "Permissions",
                self.probe.cua_health.clone().unwrap_or_else(|| {
                    if cfg!(target_os = "macos") {
                        "macOS needs Accessibility and Screen Recording for CuaDriver".into()
                    } else {
                        "Reported by the driver's own health check".into()
                    }
                }),
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(
                        ui::button_ghost("int-refresh", "Refresh", &t)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )
                    .when(self.probe.cua.is_some(), |d| {
                        d.child(
                            ui::button_ghost("int-cua-check", if self.checking { "Checking…" } else { "Check" }, &t)
                                .on_click(cx.listener(|this, _, _, cx| this.check_cua(cx))),
                        )
                    }),
                &t,
            ))
            .child(
                div()
                    .pt(px(14.0))
                    .text_size(px(12.0))
                    .text_color(t.text_faint)
                    .child("Changes apply to agents started from now on."),
            )
            .into_any_element()
    }
}
