//! Settings → Integrations: Dove for Chrome pairing, Composio (BYOK), Paper,
//! and computer use through the open-source `cua-driver`.
//!
//! The choices live in `<dove data>/integrations.json` (mode 0600, since it
//! holds the user's Composio key). `dove --mcp-browser` reads that file when
//! a harness starts it and aggregates the enabled servers into Dove's own
//! MCP server, so every harness Dove starts gets them without per-harness
//! config.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use gpui_base::input::{Input, InputEvent, InputState};
use serde::{Deserialize, Serialize};

use crate::assets::{icon, icons};
use crate::design::{radius, size, space, ty};
use crate::kit::controls::{self as ctl, Tone};
use crate::kit::page;
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
    /// Signed in with a Composio account (OAuth to Composio Connect)
    /// instead of a pasted key.
    pub account: bool,
}

/// An MCP server the user added from the catalog or by hand. Its tools reach
/// every harness as `<id>__<tool>` through Dove's own MCP server.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServer {
    /// Tool prefix and OAuth token key: `[a-z0-9-]`, unique.
    pub id: String,
    pub name: String,
    /// The catalog entry it came from, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(flatten)]
    pub transport: Transport,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Transport {
    /// A local command speaking MCP on stdin/stdout.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    /// A remote streamable-HTTP server. Keys go in `headers`
    /// (`Authorization: Bearer …`); `oauth` uses the token Dove signed in for.
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        oauth: bool,
    },
}

impl McpServer {
    pub fn is_remote(&self) -> bool {
        matches!(self.transport, Transport::Http { .. })
    }

    pub fn uses_oauth(&self) -> bool {
        matches!(self.transport, Transport::Http { oauth: true, .. })
    }
}

/// A tool prefix from a display name: lowercase `[a-z0-9-]`, at most 24
/// characters, never empty.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 24 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "server".into() } else { out }
}

/// Ids Dove's own server already uses as tool prefixes.
const RESERVED: [&str; 5] = ["composio", "paper", "computer", "browser", "chrome"];

/// `base`, or `base-2`, `base-3`… so it is unique among `taken` and never
/// collides with Dove's own prefixes.
pub fn unique_id(base: &str, taken: &[String]) -> String {
    let base = slug(base);
    let free = |id: &str| !RESERVED.contains(&id) && !taken.iter().any(|t| t == id);
    if free(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}-{n}")).find(|id| free(id)).unwrap_or(base)
}

/// Read a pasted `{"mcpServers": {…}}` block (Claude Code / Cursor / Gemini
/// shape), or a bare `{name: {…}}` map, or one server object with a name.
pub fn parse_pasted(json: &str, taken: &[String]) -> Result<Vec<McpServer>, String> {
    let v: serde_json::Value = serde_json::from_str(json.trim()).map_err(|e| format!("Not valid JSON: {e}"))?;
    let map = v.get("mcpServers").or_else(|| v.get("mcp")).or_else(|| v.get("servers")).unwrap_or(&v);
    let map = map.as_object().ok_or("Expected an object of servers")?;
    let mut taken = taken.to_vec();
    let mut out = Vec::new();
    for (name, spec) in map {
        let str_map = |key: &str| -> BTreeMap<String, String> {
            spec.get(key)
                .and_then(|m| m.as_object())
                .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
                .unwrap_or_default()
        };
        let url = spec.get("url").or_else(|| spec.get("httpUrl")).or_else(|| spec.get("serverUrl")).and_then(|u| u.as_str());
        let transport = if let Some(url) = url {
            let mut headers = str_map("headers");
            headers.extend(str_map("http_headers"));
            Transport::Http { url: url.to_string(), headers, oauth: false }
        } else {
            // OpenCode writes `command` as one array; everyone else splits it.
            let (command, mut args) = match spec.get("command") {
                Some(serde_json::Value::Array(parts)) => {
                    let mut parts = parts.iter().filter_map(|p| p.as_str().map(str::to_string));
                    (parts.next().unwrap_or_default(), parts.collect::<Vec<_>>())
                }
                Some(c) => (c.as_str().unwrap_or_default().to_string(), Vec::new()),
                None => (String::new(), Vec::new()),
            };
            if command.is_empty() {
                return Err(format!("{name}: needs a command or a url"));
            }
            args.extend(
                spec.get("args")
                    .and_then(|a| a.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect::<Vec<_>>())
                    .unwrap_or_default(),
            );
            let mut env = str_map("env");
            env.extend(str_map("environment"));
            let cwd = spec.get("cwd").and_then(|c| c.as_str()).map(str::to_string);
            Transport::Stdio { command, args, env, cwd }
        };
        let id = unique_id(name, &taken);
        taken.push(id.clone());
        out.push(McpServer { id, name: name.clone(), catalog: None, enabled: true, transport });
    }
    if out.is_empty() {
        return Err("No servers found".into());
    }
    Ok(out)
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
    /// Catalog and custom MCP servers, in the order the user added them.
    pub servers: Vec<McpServer>,
    /// ego lite as the agents' only browser: hides Dove's `browser_*` tools
    /// so agents use the `ego-browser` command (dove-browser skill).
    pub ego: Toggle,
}

impl Config {
    pub fn ids(&self) -> Vec<String> {
        self.servers.iter().map(|s| s.id.clone()).collect()
    }

    /// Whether anything here needs Dove's MCP server running.
    pub fn any_enabled(&self) -> bool {
        self.composio.enabled || self.paper.enabled || self.computer.enabled || self.servers.iter().any(|s| s.enabled)
    }
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

/// The `ego-browser` command ego lite registers on onboarding, if found.
pub fn ego_browser() -> Option<PathBuf> {
    dove_core::harness_process::resolve("ego-browser")
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
    /// ego lite's `ego-browser` command.
    ego: Option<PathBuf>,
    /// The GitHub CLI, and the account it is signed in to.
    gh: Option<PathBuf>,
    gh_user: Option<String>,
}

/// The GitHub CLI's setup, while Dove installs it or opens its sign-in.
#[derive(Clone, Default)]
enum GhStep {
    #[default]
    Idle,
    Installing,
    /// The sign-in terminal is open; detect again once it is done.
    SigningIn,
    Failed(String),
}

#[path = "integrations_mcp.rs"]
mod mcp;

pub struct IntegrationsView {
    config: Config,
    api_key: Entity<InputState>,
    toolkits: Entity<InputState>,
    /// Filters connected servers and the catalog.
    search: Entity<InputState>,
    /// Live per-server state (signing in, tested, failed), by id.
    server_status: std::collections::HashMap<String, mcp::Status>,
    dialog: Option<mcp::Dialog>,
    probe: Probe,
    checking: bool,
    copied: bool,
    gh_step: GhStep,
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
        let search = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Search servers");
            s.set_editor_style(ui::input_style(Theme::of(cx)));
            s
        });
        let subs = vec![
            cx.subscribe(&search, |_, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            }),
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
        let mut view = Self {
            config,
            api_key,
            toolkits,
            search,
            server_status: Default::default(),
            dialog: None,
            probe: Probe::default(),
            checking: false,
            copied: false,
            gh_step: GhStep::Idle,
            _subs: subs,
        };
        view.refresh(cx);
        // Connection state changes outside GPUI (the extension's socket): wait
        // on its watch instead of polling, so an idle view schedules nothing.
        let mut changes = crate::browser::control::connection_watch();
        cx.spawn(async move |this, cx| {
            while changes.changed().await.is_ok() {
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
        // The first integration turned on with the agent browser off still
        // needs Dove's server registered for the agents started next.
        crate::mcp_browser::register_without_browser(&data_root());
        // Supermemory connected or removed: memory recall and capture follow.
        let root = data_root();
        cx.background_spawn(async move { crate::supermemory::refresh(&root) }).detach();
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move {
            let cua = cua_driver();
            let cua_version = cua.as_deref().and_then(cua_version);
            let gh = dove_core::git::github::gh_path();
            let gh_user = gh.as_ref().and_then(|_| dove_core::git::github::gh_user());
            Probe { cua, cua_version, cua_health: None, paper: paper_running(), ego: ego_browser(), gh, gh_user }
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

    /// Install the GitHub CLI with the system package manager, then detect.
    fn install_gh(&mut self, cx: &mut Context<Self>) {
        if matches!(self.gh_step, GhStep::Installing) {
            return;
        }
        self.gh_step = GhStep::Installing;
        let task = cx.background_spawn(async move { dove_core::git::github::install_gh() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                v.gh_step = match result {
                    Ok(()) => GhStep::Idle,
                    Err(e) => GhStep::Failed(format!("{e:#}")),
                };
                v.refresh(cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// Open `gh auth login` in a terminal; the browser finishes sign-in.
    fn sign_in_gh(&mut self, cx: &mut Context<Self>) {
        self.gh_step = match dove_core::git::github::open_gh_login() {
            Ok(()) => GhStep::SigningIn,
            Err(e) => GhStep::Failed(format!("{e:#}")),
        };
        cx.notify();
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

/// One integration: a kit card whose first row is the switch row.
fn plate(t: &Theme) -> gpui::Div {
    page::card(t)
}

/// The integration's head row: its glyph tile, name and hint, and the
/// status and switch on the right.
fn head(
    glyph: &'static str,
    title: &'static str,
    hint: impl Into<SharedString>,
    right: impl IntoElement,
    t: &Theme,
) -> gpui::Div {
    page::row(t, true)
        .child(page::row_tile(t, glyph, None))
        .child(page::row_text(t, title, hint))
        .child(div().flex_none().max_w_full().flex().items_center().justify_end().child(right))
}

/// A detail row inside a plate: label and hint on the left, the control on
/// the right (wrapping under the text on a narrow card).
fn line(label: &'static str, hint: impl Into<SharedString>, control: impl IntoElement, t: &Theme) -> gpui::Div {
    page::setting_row(t, false, label, hint, control)
}

fn status(on: bool, label: impl Into<SharedString>, t: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .text_size(ty::META)
        .text_color(if on { t.text_muted } else { t.text_faint })
        .child(div().size(px(6.0)).rounded_full().bg(if on { t.success } else { t.wash(0.22) }))
        .child(label.into())
}

/// Status text and a switch, the right side of every plate head.
fn toggle(
    on: bool,
    state: Option<(bool, &'static str)>,
    id: &'static str,
    t: &Theme,
) -> (gpui::Div, gpui::Stateful<gpui::Div>) {
    let left =
        div().flex().items_center().gap(px(14.0)).when_some(state, |d, (ok, label)| d.child(status(ok, label, t)));
    (left, ctl::toggle(t, id, on))
}

/// A toolkit pick: a pill, filled with the accent when chosen.
fn pick(id: SharedString, label: &'static str, on: bool, t: &Theme) -> gpui::Stateful<gpui::Div> {
    let hover = t.wash(0.10);
    div()
        .id(gpui::ElementId::Name(id))
        .role(gpui::Role::CheckBox)
        .aria_toggled(if on { gpui::Toggled::True } else { gpui::Toggled::False })
        .aria_label(label)
        .h(px(26.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .gap(px(5.0))
        .rounded_full()
        .text_size(ty::SMALL)
        .cursor_pointer()
        .map(|d| {
            if on {
                d.bg(t.accent).text_color(t.on_solid)
            } else {
                d.bg(t.wash(0.06)).text_color(t.text_muted).hover(move |s| s.bg(hover))
            }
        })
        .when(on, |d| d.child(icon(icons::CHECK).size(px(11.0)).text_color(t.on_solid)))
        .child(label)
}

fn small_icon(
    id: impl Into<gpui::ElementId>,
    glyph: &'static str,
    label: &'static str,
    t: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let hover = t.wash(0.08);
    div()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(label)
        .size(px(28.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(radius::RADIUS))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(ui::tip(label))
        .child(icon(glyph).size(px(14.0)).text_color(t.text_muted))
}

fn field(input: &Entity<InputState>, w: f32, t: &Theme) -> impl IntoElement {
    div()
        .w(px(w))
        .max_w_full()
        .h(px(size::FIELD))
        .px(px(10.0))
        .flex()
        .items_center()
        .rounded(px(radius::SELECT))
        .bg(t.wash(0.06))
        .text_size(ty::BODY)
        .child(div().flex_1().min_w_0().child(Input::new(input)))
}

impl Render for IntegrationsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("IntegrationsView");
        let t = Theme::of(cx).clone();
        let browser = crate::browser::control::connected_browser();
        let code = crate::browser::control::pairing_code();
        let browser_name = browser.as_deref().map(|ua| {
            if ua.contains("Edg/") {
                "Edge"
            } else if ua.contains("Brave") {
                "Brave"
            } else {
                "Chrome"
            }
        });

        let c = self.config.composio.clone();
        let key_kind = ComposioKey::of(&c.api_key);
        let scoped = !c.api_key.is_empty() && key_kind != ComposioKey::Consumer;
        let paper_on = self.config.paper.enabled;
        let cua_on = self.config.computer.enabled;
        let composio_on = c.enabled;
        let cua_installed = self.probe.cua.is_some();

        // GitHub CLI: pull requests, PR reviews and PR automations.
        let gh_installed = self.probe.gh.is_some();
        let gh_state = match (&self.probe.gh, &self.probe.gh_user) {
            (Some(_), Some(user)) => (true, format!("Signed in as {user}")),
            (Some(_), None) => (false, "Not signed in".to_string()),
            (None, _) => (false, "Not installed".to_string()),
        };
        let installing = matches!(self.gh_step, GhStep::Installing);
        let gh_hint: SharedString = match &self.gh_step {
            GhStep::Installing if cfg!(windows) => {
                "Installing with winget. Windows may ask to allow the installer".into()
            }
            GhStep::Installing => "Installing with Homebrew…".into(),
            GhStep::SigningIn => "Finish signing in in the terminal and browser, then detect again".into(),
            GhStep::Failed(e) => format!("Could not finish: {e}").into(),
            GhStep::Idle => match (&self.probe.gh, &self.probe.gh_user) {
                (Some(_), Some(_)) => "Pull requests, reviews and PR automations use this account".into(),
                (Some(_), None) => "Sign in once so Dove can list, open and merge pull requests".into(),
                (None, _) => "Dove opens, reviews and merges pull requests through the GitHub CLI (gh)".into(),
            },
        };
        let gh_action = if !gh_installed {
            Some(if cfg!(any(windows, target_os = "macos")) {
                ctl::button_sm(&t, "int-gh-install", if installing { "Installing…" } else { "Install" }, Tone::Filled)
                    .on_click(cx.listener(|this, _, _, cx| this.install_gh(cx)))
                    .into_any_element()
            } else {
                ctl::button_sm(&t, "int-gh-docs", "Install guide", Tone::Filled)
                    .on_click(cx.listener(|_, _, _, cx| cx.open_url("https://cli.github.com")))
                    .into_any_element()
            })
        } else if self.probe.gh_user.is_none() {
            Some(
                ctl::button_sm(&t, "int-gh-login", "Sign in", Tone::Filled)
                    .on_click(cx.listener(|this, _, _, cx| this.sign_in_gh(cx)))
                    .into_any_element(),
            )
        } else {
            None
        };
        let github = plate(&t)
            .child(head(icons::GIT_BRANCH, "GitHub", gh_hint, status(gh_state.0, gh_state.1, &t), &t))
            .child(line(
                "GitHub CLI",
                self.probe.gh.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| {
                    if cfg!(windows) {
                        "One click: installs GitHub.cli with winget".into()
                    } else if cfg!(target_os = "macos") {
                        "One click: installs gh with Homebrew".into()
                    } else {
                        "Install gh with your package manager".into()
                    }
                }),
                div().flex().items_center().gap(px(4.0)).children(gh_action).child(
                    small_icon("int-gh-refresh", icons::REFRESH, "Detect again", &t).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.gh_step = GhStep::Idle;
                            this.refresh(cx);
                        },
                    )),
                ),
                &t,
            ));

        // Dove for Chrome
        let chrome = plate(&t)
            .child(head(
                icons::GLOBE,
                "Dove for Chrome",
                "Agents work in your own browser with your logins, only on sites you allow",
                status(
                    browser.is_some(),
                    browser_name.map(|b| format!("Connected to {b}")).unwrap_or_else(|| "Not connected".into()),
                    &t,
                ),
                &t,
            ))
            .child(line(
                "Pairing code",
                if browser.is_some() {
                    "Paired. Copy it again only to pair another browser"
                } else {
                    "Open the Dove side panel in Chrome and paste this once"
                },
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        div()
                            .h(px(size::FIELD))
                            .px(px(10.0))
                            .flex()
                            .items_center()
                            .max_w(px(240.0))
                            .rounded(px(radius::SELECT))
                            .bg(t.wash(0.06))
                            .text_size(ty::META)
                            .font_family(t.font_mono.clone())
                            .text_color(t.text_muted)
                            .child(
                                div()
                                    .truncate()
                                    .child(code.clone().unwrap_or_else(|| "Turn on the agent browser to pair".into())),
                            ),
                    )
                    .when_some(code, |d, code| {
                        d.child(
                            small_icon(
                                "int-copy",
                                if self.copied { icons::CHECK } else { icons::COPY },
                                if self.copied { "Copied" } else { "Copy pairing code" },
                                &t,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                                this.copied = true;
                                cx.spawn(async move |this, cx| {
                                    cx.background_executor().timer(Duration::from_millis(1600)).await;
                                    let _ = this.update(cx, |v, cx| {
                                        v.copied = false;
                                        cx.notify();
                                    });
                                })
                                .detach();
                                cx.notify();
                            })),
                        )
                    }),
                &t,
            ));

        // Composio
        let (left, switch) = toggle(composio_on, None, "int-composio", &t);
        let composio = plate(&t)
            .child(head(
                icons::WIDGET,
                "Composio",
                "Gmail, GitHub, Slack, Notion and 1000+ apps through your own Composio account",
                left.child(ctl::on_toggle(switch, cx, move |this, _, cx| {
                    this.config.composio.enabled = !composio_on;
                    this.persist(cx);
                })),
                &t,
            ))
            .when(composio_on, |d| {
                let this = cx.entity().downgrade();
                let account = c.account;
                let signed_in = crate::mcp_oauth::signed_in("composio");
                let state = self.server_status.get("composio").cloned();
                d.child(line(
                    "Connect with",
                    "Sign in with your Composio account, or paste a key",
                    ctl::segmented(
                        &t,
                        "int-composio-mode",
                        vec![ctl::Segment::new("API key"), ctl::Segment::new("Composio account")],
                        usize::from(account),
                        move |i, _, cx| {
                            let _ = this.update(cx, |v, cx| {
                                v.config.composio.account = i == 1;
                                v.persist(cx);
                            });
                        },
                    ),
                    &t,
                ))
                .when(account, |d| {
                    let (ok, label) = match &state {
                        Some(mcp::Status::Busy(m)) => (false, (*m).to_string()),
                        Some(mcp::Status::Failed(e)) => (false, e.chars().take(90).collect()),
                        _ if signed_in => (true, "Signed in. Agents can use every app you link in Composio".into()),
                        _ => (false, "Not signed in".to_string()),
                    };
                    d.child(line(
                        "Composio account",
                        label.clone(),
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(status(ok, if ok { "Connected" } else { "Not connected" }, &t))
                            .child(if signed_in {
                                ctl::button_sm(&t, "int-composio-signout", "Sign out", Tone::Quiet)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        crate::mcp_oauth::sign_out("composio");
                                        this.server_status.remove("composio");
                                        cx.notify();
                                    }))
                            } else {
                                ctl::button_sm(&t, "int-composio-signin", "Sign in", Tone::Filled).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.sign_in("composio".into(), COMPOSIO_CONNECT_URL.into(), cx)
                                    }),
                                )
                            }),
                        &t,
                    ))
                })
            })
            .when(composio_on && !c.account, |d| {
                d.child(line(
                    "API key",
                    match (c.api_key.is_empty(), key_kind) {
                        (true, _) => "From composio.dev → Settings. Kept only in Dove's data folder",
                        (false, ComposioKey::Consumer) => "Connect key. Agents pick apps as they need them",
                        (false, ComposioKey::Project) => "Project key. Limited to the toolkits below",
                        (false, ComposioKey::User) => "User key. Limited to the toolkits below",
                    },
                    field(&self.api_key, 260.0, &t),
                    &t,
                ))
                .child(
                    page::row(&t, false).child(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap(px(10.0))
                            .child(page::row_text(
                                &t,
                                "Toolkits",
                                if scoped {
                                    "Pick none to allow every toolkit"
                                } else {
                                    "Applies to project (ak_) and user (uak_) keys"
                                },
                            ))
                            .child(div().flex().flex_wrap().gap(px(6.0)).children(TOOLKITS.iter().map(
                                |(slug, name)| {
                                    let on = c.toolkits.iter().any(|k| k == slug);
                                    let slug = slug.to_string();
                                    pick(SharedString::from(format!("int-tk-{slug}")), name, on, &t).on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            let list = &mut this.config.composio.toolkits;
                                            if let Some(i) = list.iter().position(|k| *k == slug) {
                                                list.remove(i);
                                            } else {
                                                list.push(slug.clone());
                                            }
                                            this.persist(cx);
                                        }),
                                    )
                                },
                            )))
                            .child(field(&self.toolkits, 320.0, &t)),
                    ),
                )
            });

        // Paper
        let (left, switch) = toggle(
            paper_on,
            Some((self.probe.paper, if self.probe.paper { "Running" } else { "Not running" })),
            "int-paper",
            &t,
        );
        let paper = plate(&t).child(head(
            icons::PEN,
            "Paper",
            if self.probe.paper {
                "Agents can read and edit the design open in Paper Desktop"
            } else {
                "Open Paper Desktop (paper.design) so agents can read and edit designs"
            },
            left.child(ctl::on_toggle(switch, cx, move |this, _, cx| {
                this.config.paper.enabled = !paper_on;
                this.persist(cx);
                this.refresh(cx);
            })),
            &t,
        ));

        // Computer use
        let (left, switch) = toggle(
            cua_on,
            Some((cua_installed, if cua_installed { "Installed" } else { "Not installed" })),
            "int-cua",
            &t,
        );
        let computer = plate(&t)
            .child(head(
                icons::MONITOR,
                "Computer use",
                match (&self.probe.cua, &self.probe.cua_version) {
                    (Some(_), Some(v)) => format!("Agents see and use your desktop apps through {v}"),
                    (Some(_), None) => "Agents see and use your desktop apps through cua-driver".to_string(),
                    (None, _) => format!("Uses the open-source cua-driver {CUA_VERSION} or later, running locally"),
                },
                left.child(ctl::on_toggle(switch, cx, move |this, _, cx| {
                    this.config.computer.enabled = !cua_on;
                    this.persist(cx);
                })),
                &t,
            ))
            .child(if let Some(path) = self.probe.cua.clone() {
                line(
                    "Permissions",
                    self.probe.cua_health.clone().unwrap_or_else(|| {
                        if cfg!(target_os = "macos") {
                            "Needs Accessibility and Screen Recording in System Settings".into()
                        } else {
                            format!("{}", path.display())
                        }
                    }),
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .child(
                            ctl::button_sm(
                                &t,
                                "int-cua-check",
                                if self.checking { "Checking…" } else { "Check" },
                                Tone::Filled,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.check_cua(cx))),
                        )
                        .child(
                            small_icon("int-refresh", icons::REFRESH, "Detect again", &t)
                                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                        ),
                    &t,
                )
            } else {
                line(
                    "Install",
                    "Install cua-driver from github.com/trycua/cua, then detect again",
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .child(
                            small_icon("int-cua-docs", icons::ARROW_UP_RIGHT, "Open install guide", &t)
                                .on_click(cx.listener(|_, _, _, cx| cx.open_url("https://github.com/trycua/cua"))),
                        )
                        .child(
                            small_icon("int-refresh", icons::REFRESH, "Detect again", &t)
                                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                        ),
                    &t,
                )
            });

        let mcp = self.render_mcp(&t, cx);
        let dialog = self.render_dialog(&t, window, cx);
        // ego lite: the agents' browser
        let ego_on = self.config.ego.enabled;
        let ego_found = self.probe.ego.is_some();
        let ego = plate(&t)
            .child(head(
                icons::GLOBAL,
                "ego lite",
                if ego_found {
                    "Agents browse in your own Chromium with your sign-ins, each task in its own space you can watch"
                } else {
                    "A browser built for agents. Install it and every harness uses it through the dove-browser skill"
                },
                status(ego_found, if ego_found { "Installed" } else { "Not installed" }, &t),
                &t,
            ))
            .child(if ego_found {
                let (left, switch) = toggle(ego_on, None, "int-ego-only", &t);
                line(
                    "Use ego lite only",
                    "Hide Dove's own browser tools so agents always use ego lite",
                    left.child(ctl::on_toggle(switch, cx, move |this, _, cx| {
                        this.config.ego.enabled = !ego_on;
                        this.persist(cx);
                    })),
                    &t,
                )
            } else {
                line(
                    "Install",
                    "Download ego lite, finish its onboarding, then detect again",
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .child(
                            ctl::button_sm(&t, "int-ego-get", "Get ego lite", Tone::Filled)
                                .on_click(cx.listener(|_, _, _, cx| cx.open_url("https://lite.ego.app/"))),
                        )
                        .child(
                            small_icon("int-ego-refresh", icons::REFRESH, "Detect again", &t)
                                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                        ),
                    &t,
                )
            });
        let built_in = div()
            .flex()
            .flex_col()
            .gap(px(space::MD))
            .child(github)
            .child(ego)
            .child(chrome)
            .child(composio)
            .child(paper)
            .child(computer);
        div()
            .pt(px(space::LG))
            .flex()
            .flex_col()
            .gap(px(space::MD))
            .child(page::section(&t, "MCP servers", mcp))
            .child(page::section(&t, "Built in", built_in))
            .children(dialog)
            .child(
                div()
                    .px(px(page::LABEL_INSET))
                    .pt(px(space::XXS))
                    .text_size(ty::META)
                    .text_color(t.text_faint)
                    .child("Changes apply to agents started from now on."),
            )
            .into_any_element()
    }
}
