//! Settings → Plugins: MCP servers (from the catalog or added by hand), the
//! built-in tools (GitHub CLI, ego lite, Dove for Chrome, Paper, the Dove
//! Computer Use plugin) and apps through Composio. One view draws every tab
//! (`plugins.rs` lays the page out), so they share one copy of the config
//! and never overwrite each other's changes.
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
        let url =
            spec.get("url").or_else(|| spec.get("httpUrl")).or_else(|| spec.get("serverUrl")).and_then(|u| u.as_str());
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
    /// Dove Computer Use (computer_plugin): on/off, approvals, grounder.
    pub computer: crate::computer_plugin::Settings,
    /// Catalog and custom MCP servers, in the order the user added them.
    pub servers: Vec<McpServer>,
    /// Older builds' "Use ego lite only" switch, read once into `browser`
    /// and kept in step with it so they still read the same choice.
    pub ego: Toggle,
    /// Which browser agents use.
    pub browser: BrowserChoice,
    /// Let agents run `ego-browser` without a prompt, through each
    /// harness's own allow rules (off by default).
    pub ego_allow: bool,
}

/// The agents' browser (Settings → Plugins → ego lite).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BrowserChoice {
    /// ego lite first, Dove's browser tools kept as a fallback.
    #[default]
    Both,
    /// ego lite only: Dove's `browser_*` tools are hidden so agents use the
    /// `ego-browser` command (dove-browser skill).
    Ego,
    /// Dove's own browser; agents are not pointed at ego lite.
    Dove,
}

impl Config {
    pub fn ids(&self) -> Vec<String> {
        self.servers.iter().map(|s| s.id.clone()).collect()
    }

    /// Whether agents may run `ego-browser` without asking: the switch,
    /// unless agents are not pointed at ego lite at all.
    pub fn ego_allowed(&self) -> bool {
        self.ego_allow && self.browser != BrowserChoice::Dove
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
    std::fs::read_to_string(config_path()).ok().and_then(|s| parse(&s)).unwrap_or_default()
}

/// Read `integrations.json`, filling in what older versions did not write.
pub fn parse(text: &str) -> Option<Config> {
    let raw: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut config: Config = serde_json::from_value(raw.clone()).ok()?;
    // Before the three-way choice, "Use ego lite only" was `ego.enabled`.
    if raw.get("browser").is_none() && config.ego.enabled {
        config.browser = BrowserChoice::Ego;
    }
    config.ego.enabled = config.browser == BrowserChoice::Ego;
    // Ids are tool prefixes: drop duplicates and Dove's own prefixes that a
    // hand-edited file may carry, keeping the first of each.
    let mut taken: Vec<String> = Vec::new();
    for s in &mut config.servers {
        if taken.contains(&s.id) || RESERVED.contains(&s.id.as_str()) || slug(&s.id) != s.id {
            s.id = unique_id(&s.id, &taken);
        }
        taken.push(s.id.clone());
    }
    Some(config)
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

/// The computer-use driver: the installed Dove Computer Use plugin first,
/// then a `cua-driver` (or a break-glass `dove-driver` build) on `PATH` or
/// where their installers put it (a developer override).
pub fn cua_driver() -> Option<PathBuf> {
    crate::computer_plugin::installed().map(|i| i.bin).or_else(dev_driver)
}

/// A driver the user installed themselves, outside Dove's plugin folder.
pub fn dev_driver() -> Option<PathBuf> {
    let exes: &[&str] =
        if cfg!(windows) { &["cua-driver.exe", "dove-driver.exe"] } else { &["cua-driver", "dove-driver"] };
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
    exes.iter().find_map(|exe| dirs.iter().map(|d| d.join(exe)).find(|p| p.is_file()))
}

/// `cua-driver --version`, trimmed.
pub fn cua_version(bin: &std::path::Path) -> Option<String> {
    let out = std::process::Command::new(bin).arg("--version").output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// The `ego-browser` command ego lite registers on onboarding, if found.
pub fn ego_browser() -> Option<PathBuf> {
    crate::ego::detect().cli
}

/// The Paper CLI, which Paper Desktop installs to `~/.paper/bin` when it
/// first opens. `paper mcp` relays MCP over stdio, even while Desktop is
/// closed, so Dove prefers it to Desktop's HTTP endpoint.
pub fn paper_cli() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)?;
    let bin = home.join(".paper").join("bin").join(if cfg!(windows) { "paper.exe" } else { "paper" });
    bin.is_file().then_some(bin)
}

/// Whether Paper Desktop's (legacy) HTTP MCP server is listening.
pub fn paper_running() -> bool {
    std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], 29979).into(), Duration::from_millis(250)).is_ok()
}

#[derive(Clone, Default)]
struct Probe {
    cua: Option<PathBuf>,
    /// The Dove Computer Use plugin, when installed.
    cu_plugin: Option<crate::computer_plugin::Installed>,
    cua_version: Option<String>,
    paper: bool,
    /// The Paper CLI (`paper mcp`), when Paper Desktop installed it.
    paper_cli: Option<PathBuf>,
    /// ego lite: the app, its version and the `ego-browser` command.
    ego: crate::ego::Status,
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

#[path = "integrations_computer.rs"]
mod computer;
#[path = "integrations_mcp.rs"]
mod mcp;
#[path = "plugins.rs"]
pub mod plugins;

pub struct IntegrationsView {
    /// Settings › Plugins: the tab, list / directory / detail, the Add menu.
    plugins: plugins::State,
    /// The views Plugins shows for skills and memory (owned by Settings).
    skills: Option<gpui::WeakEntity<crate::pages::skills::SkillsView>>,
    memory: Option<gpui::WeakEntity<crate::pages::memory_settings::MemoryView>>,
    config: Config,
    api_key: Entity<InputState>,
    toolkits: Entity<InputState>,
    /// Filters connected servers and the catalog.
    search: Entity<InputState>,
    /// Live per-server state (signing in, tested, failed), by id.
    server_status: std::collections::HashMap<String, mcp::Status>,
    dialog: Option<mcp::Dialog>,
    probe: Probe,
    copied: bool,
    /// The last ego lite runtime check, and whether one is running.
    ego_health: Option<crate::ego::Health>,
    ego_testing: bool,
    gh_step: GhStep,
    /// The Computer use plate's jobs, health report and walkthrough.
    cu: computer::State,
    /// Computer use: apps agents may never control, comma separated.
    cu_denied: Entity<InputState>,
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
            let mut s = InputState::new(window, cx).placeholder("Search plugins");
            s.set_editor_style(ui::input_style(Theme::of(cx)));
            s
        });
        let cu_denied = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("e.g. 1Password, KeePass, Bitwarden");
            s.set_editor_style(ui::input_style(Theme::of(cx)));
            s.set_value(config.computer.denied_apps.join(", "), window, cx);
            s
        });
        let subs = vec![
            cx.subscribe(&cu_denied, |this, s, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change | InputEvent::Blur) {
                    let list: Vec<String> = s
                        .read(cx)
                        .value()
                        .split([',', '\n', ';'])
                        .map(|a| a.trim().to_string())
                        .filter(|a| !a.is_empty())
                        .collect();
                    if list != this.config.computer.denied_apps {
                        this.config.computer.denied_apps = list;
                        this.persist(cx);
                    }
                }
            }),
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
            plugins: plugins::State::default(),
            skills: None,
            memory: None,
            config,
            api_key,
            toolkits,
            search,
            server_status: Default::default(),
            dialog: None,
            probe: Probe::default(),
            copied: false,
            ego_health: None,
            ego_testing: false,
            gh_step: GhStep::Idle,
            cu: computer::State::default(),
            cu_denied,
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

    /// The Skills and Memory views Plugins shows in its Skills tab and
    /// Memory details.
    pub fn embed(
        &mut self,
        skills: &Entity<crate::pages::skills::SkillsView>,
        memory: &Entity<crate::pages::memory_settings::MemoryView>,
    ) {
        self.skills = Some(skills.downgrade());
        self.memory = Some(memory.downgrade());
    }

    fn persist(&mut self, cx: &mut Context<Self>) {
        let _ = save(&self.config);
        dove_core::set_allow_ego_browser(self.config.ego_allowed());
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
            Probe {
                cua,
                cu_plugin: crate::computer_plugin::installed(),
                cua_version,
                paper: paper_running(),
                paper_cli: paper_cli(),
                ego: crate::ego::detect(),
                gh,
                gh_user,
            }
        });
        cx.spawn(async move |this, cx| {
            let probe = task.await;
            let _ = this.update(cx, |v, cx| {
                v.probe = probe;
                v.computer_probed(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Run ego lite's own runtime check (it may start ego lite).
    fn test_ego(&mut self, cx: &mut Context<Self>) {
        let Some(cli) = self.probe.ego.cli.clone() else { return };
        if self.ego_testing {
            return;
        }
        self.ego_testing = true;
        let task = cx.background_spawn(async move { crate::ego::health(&cli, Duration::from_secs(30)) });
        cx.spawn(async move |this, cx| {
            let health = task.await;
            let _ = this.update(cx, |v, cx| {
                v.ego_health = Some(health);
                v.ego_testing = false;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
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
        let composio_on = c.enabled;

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
                (Some(_), None) => "Sign in once so Keplorayx can list, open and merge pull requests".into(),
                (None, _) => "Keplorayx opens, reviews and merges pull requests through the GitHub CLI (gh)".into(),
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
                "Keplorayx for Chrome",
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
                    "Paired. To switch browsers or extensions, make a new code"
                } else {
                    "Open the Keplorayx side panel in Chrome and paste this once"
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
                        .child(
                            small_icon(
                                "int-new-pairing",
                                icons::REFRESH,
                                "New pairing code (unpairs the current browser)",
                                &t,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                crate::browser::control::new_pairing_code();
                                this.copied = false;
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
                                ctl::button_sm(&t, "int-composio-signout", "Sign out", Tone::Quiet).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        crate::mcp_oauth::sign_out("composio");
                                        this.server_status.remove("composio");
                                        cx.notify();
                                    }),
                                )
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
                        (true, _) => "From composio.dev → Settings. Kept only in Keplorayx's data folder",
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
        let paper_cli = self.probe.paper_cli.is_some();
        let (left, switch) = toggle(
            paper_on,
            Some(match (paper_cli, self.probe.paper) {
                (true, _) => (true, "Paper CLI"),
                (false, true) => (true, "Running"),
                (false, false) => (false, "Not running"),
            }),
            "int-paper",
            &t,
        );
        let paper = plate(&t).child(head(
            icons::PEN,
            "Paper",
            if paper_cli {
                "Agents read and edit the file open in Paper Desktop through the Paper CLI"
            } else if self.probe.paper {
                "Agents can read and edit the design open in Paper Desktop"
            } else {
                "Open Paper Desktop (paper.design) once; it installs the CLI agents use to read and edit designs"
            },
            left.child(ctl::on_toggle(switch, cx, move |this, _, cx| {
                this.config.paper.enabled = !paper_on;
                this.persist(cx);
                this.refresh(cx);
            })),
            &t,
        ));

        // Computer use: the Dove Computer Use plugin (integrations_computer.rs)
        let computer = self.render_computer(&t, cx);

        let mcp = self.render_mcp(&t, cx);
        let dialog = self.render_dialog(&t, window, cx);
        // ego lite: the agents' browser
        let choice = self.config.browser;
        let ego_found = self.probe.ego.cli.is_some();
        let ego_app = self.probe.ego.app.clone();
        let ego_version = self.probe.ego.version().map(str::to_string);
        let ego_stale = self.probe.ego.stale.clone();
        let ego_state: SharedString = match (&self.probe.ego.cli, &ego_app, &self.ego_health) {
            (Some(_), _, Some(crate::ego::Health::Ready)) => {
                format!("Ready{}", ego_version.as_deref().map(|v| format!(" · {v}")).unwrap_or_default()).into()
            }
            (Some(_), _, Some(crate::ego::Health::Failed(_))) => "Not responding".into(),
            (Some(_), _, None) => {
                format!("Installed{}", ego_version.as_deref().map(|v| format!(" · {v}")).unwrap_or_default()).into()
            }
            (None, Some(_), _) => "Finish onboarding".into(),
            (None, None, _) if ego_stale.is_some() => "Removed".into(),
            _ => "Not installed".into(),
        };
        let ego_ok = ego_found && !matches!(self.ego_health, Some(crate::ego::Health::Failed(_)));
        let this = cx.entity().downgrade();
        let ego = plate(&t)
            .child(head(
                icons::GLOBAL,
                "ego lite",
                if ego_found {
                    "Agents browse in your own Chromium with your sign-ins, each task in its own space you can watch"
                } else {
                    "A browser built for agents. Install it and every harness uses it through the dove-browser skill"
                },
                status(ego_ok, ego_state, &t),
                &t,
            ))
            .when(ego_found, |d| {
                let testing = self.ego_testing;
                let detail: SharedString = match &self.ego_health {
                    Some(crate::ego::Health::Ready) => "ego-browser answered: the runtime is ready".into(),
                    Some(crate::ego::Health::Failed(e)) => {
                        format!("ego-browser did not answer ({e}). Open ego lite, then test again").into()
                    }
                    None => "Runs ego lite's own check (it may start ego lite)".into(),
                };
                d.child(line(
                    "Health",
                    detail,
                    ctl::button_sm(&t, "int-ego-test", if testing { "Testing…" } else { "Test" }, Tone::Quiet)
                        .on_click(cx.listener(|this, _, _, cx| this.test_ego(cx))),
                    &t,
                ))
            })
            .when(!ego_found && ego_app.is_some(), |d| {
                let app = ego_app.clone().unwrap();
                d.child(line(
                    "Onboarding",
                    format!(
                        "ego lite{} is installed at {}, but its ego-browser command is not registered yet. Open ego lite, finish onboarding, then detect again",
                        app.version.as_deref().map(|v| format!(" {v}")).unwrap_or_default(),
                        app.path.display()
                    ),
                    small_icon("int-ego-redetect", icons::REFRESH, "Detect again", &t)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    &t,
                ))
            })
            .child(line(
                "Agents browse with",
                match choice {
                    BrowserChoice::Ego => "ego lite only. Keplorayx's own browser tools are hidden from agents",
                    BrowserChoice::Both => "ego lite first, with Keplorayx's browser as a fallback",
                    BrowserChoice::Dove => "Keplorayx's browser in the side panel; agents are not pointed at ego lite",
                },
                ctl::segmented(
                    &t,
                    "int-ego-choice",
                    vec![ctl::Segment::new("ego lite"), ctl::Segment::new("Keplorayx browser"), ctl::Segment::new("Both")],
                    match choice {
                        BrowserChoice::Ego => 0,
                        BrowserChoice::Dove => 1,
                        BrowserChoice::Both => 2,
                    },
                    move |i, _, cx| {
                        let _ = this.update(cx, |v, cx| {
                            v.config.browser = [BrowserChoice::Ego, BrowserChoice::Dove, BrowserChoice::Both][i.min(2)];
                            v.config.ego.enabled = v.config.browser == BrowserChoice::Ego;
                            v.persist(cx);
                        });
                    },
                ),
                &t,
            ))
            .when(choice != BrowserChoice::Dove, |d| {
                let on = self.config.ego_allow;
                d.child(line(
                    "Run without asking",
                    "Claude Code runs ego-browser without a prompt; a command chained to it still asks. \
                     Codex keeps asking, and OpenCode follows its own permission rules",
                    ctl::on_toggle(ctl::toggle(&t, "int-ego-allow", on), cx, move |this, _, cx| {
                        this.config.ego_allow = !on;
                        this.persist(cx);
                    }),
                    &t,
                ))
            })
            .when(!ego_found && ego_app.is_none(), |d| {
                d.child(line(
                    "Install",
                    if ego_stale.is_some() {
                        "ego lite was removed from this computer (its last version folder is gone). Install it again, finish onboarding, then detect again. Keplorayx never downloads it for you"
                    } else {
                        "Download ego lite, finish its onboarding, then detect again. Keplorayx never downloads it for you"
                    },
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
                ))
            });
        let plates = plugins::Plates { github, ego, chrome, paper, computer, composio, mcp };
        div()
            .flex()
            .flex_col()
            .child(self.render_plugins(plates, window, cx))
            .children(dialog)
            .child(
                div()
                    .px(px(page::LABEL_INSET))
                    .pt(px(space::LG))
                    .text_size(ty::META)
                    .text_color(t.text_faint)
                    .child("Changes apply to agents started from now on."),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_files_load_and_migrate() {
        // Before MCP servers: Composio, Paper and computer use only, plus the
        // "Use ego lite only" switch.
        let old = r#"{"composio":{"enabled":true,"api_key":"ak_x","toolkits":["github"]},
                      "paper":{"enabled":true},"computer":{"enabled":false},"ego":{"enabled":true}}"#;
        let c = parse(old).unwrap();
        assert!(c.composio.enabled && !c.composio.account);
        assert_eq!(c.composio.toolkits, ["github"]);
        assert!(c.paper.enabled && !c.computer.enabled);
        assert!(c.servers.is_empty());
        assert_eq!(c.browser, BrowserChoice::Ego);
        assert_eq!(parse("{}").unwrap(), Config::default());
        assert_eq!(parse("{}").unwrap().browser, BrowserChoice::Both);
        // A saved choice wins over the old switch.
        let c = parse(r#"{"ego":{"enabled":true},"browser":"dove"}"#).unwrap();
        assert_eq!(c.browser, BrowserChoice::Dove);
        assert!(!c.ego.enabled);
        assert!(parse("not json").is_none());
    }

    #[test]
    fn servers_round_trip() {
        let mut c = Config::default();
        c.servers.push(McpServer {
            id: "linear".into(),
            name: "Linear".into(),
            catalog: Some("linear".into()),
            enabled: true,
            transport: Transport::Http {
                url: "https://mcp.linear.app/mcp".into(),
                headers: BTreeMap::new(),
                oauth: true,
            },
        });
        c.servers.push(McpServer {
            id: "local".into(),
            name: "Local".into(),
            catalog: None,
            enabled: false,
            transport: Transport::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "pkg".into()],
                env: BTreeMap::from([("TOKEN".into(), "t".into())]),
                cwd: Some("/tmp".into()),
            },
        });
        c.browser = BrowserChoice::Dove;
        let text = serde_json::to_string_pretty(&c).unwrap();
        assert!(text.contains("\"type\": \"http\"") && text.contains("\"type\": \"stdio\""));
        assert_eq!(parse(&text).unwrap(), c);
        assert!(c.any_enabled());
        // `enabled` defaults to on for hand-written entries.
        let c = parse(r#"{"servers":[{"id":"x","name":"X","type":"stdio","command":"x"}]}"#).unwrap();
        assert!(c.servers[0].enabled);
    }

    #[test]
    fn ego_allow_is_off_by_default_and_saved() {
        assert!(!parse("{}").unwrap().ego_allow);
        let c = parse(r#"{"browser":"ego","ego_allow":true}"#).unwrap();
        assert!(c.ego_allow && c.ego_allowed());
        assert_eq!(parse(&serde_json::to_string(&c).unwrap()).unwrap(), c);
        // Agents not pointed at ego lite get no rule for it.
        let c = parse(r#"{"browser":"dove","ego_allow":true}"#).unwrap();
        assert!(c.ego_allow && !c.ego_allowed());
    }

    #[test]
    fn bad_ids_are_fixed_on_load() {
        let c = parse(
            r#"{"servers":[{"id":"a","name":"A","type":"stdio","command":"a"},
                           {"id":"a","name":"A2","type":"stdio","command":"a"},
                           {"id":"composio","name":"C","type":"stdio","command":"c"},
                           {"id":"Bad Id","name":"B","type":"stdio","command":"b"}]}"#,
        )
        .unwrap();
        assert_eq!(c.ids(), ["a", "a-2", "composio-2", "bad-id"]);
    }

    #[test]
    fn pasted_json_shapes() {
        let claude = r#"{"mcpServers":{"Linear":{"url":"https://mcp.linear.app/mcp","headers":{"Authorization":"Bearer k"}},
                         "pw":{"command":"npx","args":["-y","@playwright/mcp"],"env":{"A":"1"}}}}"#;
        let got = parse_pasted(claude, &["linear".into()]).unwrap();
        assert_eq!(got.len(), 2);
        let linear = got.iter().find(|s| s.name == "Linear").unwrap();
        assert_eq!(linear.id, "linear-2");
        assert!(matches!(&linear.transport, Transport::Http { headers, .. } if headers["Authorization"] == "Bearer k"));
        let pw = got.iter().find(|s| s.name == "pw").unwrap();
        assert!(matches!(&pw.transport, Transport::Stdio { command, args, env, .. }
            if command == "npx" && args == &["-y", "@playwright/mcp"] && env["A"] == "1"));
        // OpenCode writes the command as one array.
        let oc = r#"{"mcp":{"x":{"type":"local","command":["node","s.js"],"environment":{"B":"2"}}}}"#;
        let got = parse_pasted(oc, &[]).unwrap();
        assert!(matches!(&got[0].transport, Transport::Stdio { command, args, env, .. }
            if command == "node" && args == &["s.js"] && env["B"] == "2"));
        assert!(parse_pasted("{}", &[]).is_err());
        assert!(parse_pasted(r#"{"mcpServers":{"x":{}}}"#, &[]).is_err());
    }

    #[test]
    fn ids_are_slugs_and_unique() {
        assert_eq!(slug("  My Server! v2 "), "my-server-v2");
        assert_eq!(slug("***"), "server");
        assert_eq!(unique_id("Paper", &[]), "paper-2");
        assert_eq!(unique_id("figma", &["figma".into(), "figma-2".into()]), "figma-3");
    }

    #[test]
    fn composio_key_kinds() {
        assert_eq!(ComposioKey::of("ak_1").header(), "x-api-key");
        assert_eq!(ComposioKey::of("uak_1").header(), "x-user-api-key");
        assert_eq!(ComposioKey::of("ck_1").header(), "x-consumer-api-key");
    }
}
