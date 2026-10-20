//! Settings → Integrations, MCP servers: the user's connected servers, the
//! catalog of popular ones, and the dialog for adding any server by command,
//! URL or pasted JSON. Sign-in for remote servers is Dove's own OAuth
//! (`mcp_oauth`); every server reaches every harness through Dove's MCP
//! server.

use std::collections::BTreeMap;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_base::input::{InputState, Textarea, TextareaState};

use super::{IntegrationsView, McpServer, Transport, field, small_icon, status};
use crate::assets::icons;
use crate::design::{radius, space, ty};
use crate::integrations_catalog::{self as catalog, Auth, Entry};
use crate::kit::controls::{self as ctl, Segment, Tone};
use crate::kit::{page, popover};
use crate::theme::Theme;
use crate::ui;

/// A server's live state in the view (not saved).
#[derive(Clone, Debug)]
pub(super) enum Status {
    Busy(&'static str),
    Ok(String),
    Failed(String),
}

pub(super) enum Dialog {
    /// Connect a catalog entry that takes a key (and maybe browser sign-in).
    Key { entry: &'static Entry, key: Entity<InputState>, error: Option<String> },
    /// Add or edit a server by hand.
    Custom(Box<CustomForm>),
}

pub(super) struct CustomForm {
    /// The id being edited; `None` adds a new server.
    editing: Option<String>,
    /// 0 local command, 1 remote URL, 2 paste JSON.
    mode: usize,
    name: Entity<InputState>,
    target: Entity<InputState>,
    args: Entity<InputState>,
    lines: Entity<TextareaState>,
    paste: Entity<TextareaState>,
    oauth: bool,
    error: Option<String>,
    test: Option<Status>,
}

/// Split a command line into arguments, honouring double and single quotes.
pub(super) fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `KEY=value` (env) or `Name: value` (headers) lines into a map.
pub(super) fn parse_lines(text: &str, sep: char) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let (k, v) = line.split_once(sep).ok_or_else(|| format!("\"{line}\" needs a {sep}"))?;
        let k = k.trim();
        if k.is_empty() {
            return Err(format!("\"{line}\" has no name"));
        }
        out.insert(k.to_string(), v.trim().to_string());
    }
    Ok(out)
}

fn join_lines(map: &BTreeMap<String, String>, sep: &str) -> String {
    map.iter().map(|(k, v)| format!("{k}{sep}{v}")).collect::<Vec<_>>().join("\n")
}

fn quote_args(args: &[String]) -> String {
    args.iter()
        .map(|a| if a.is_empty() || a.chars().any(char::is_whitespace) { format!("\"{a}\"") } else { a.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}

impl IntegrationsView {
    pub(super) fn set_status(&mut self, id: &str, status: Option<Status>, cx: &mut Context<Self>) {
        match status {
            Some(s) => {
                self.server_status.insert(id.to_string(), s);
            }
            None => {
                self.server_status.remove(id);
            }
        }
        cx.notify();
    }

    /// Browser sign-in for `id` against the MCP server at `url`. On success
    /// the server switches to OAuth; on failure the status says why.
    pub(super) fn sign_in(&mut self, id: String, url: String, cx: &mut Context<Self>) {
        if matches!(self.server_status.get(&id), Some(Status::Busy(_))) {
            return;
        }
        self.set_status(&id, Some(Status::Busy("Opening sign-in…")), cx);
        let begin = {
            let (id, url) = (id.clone(), url.clone());
            cx.background_spawn(async move { crate::mcp_oauth::begin(&id, &url, None, None) })
        };
        cx.spawn(async move |this, cx| {
            let pending = match begin.await {
                Ok(p) => p,
                Err(e) => {
                    let _ = this.update(cx, |v, cx| v.set_status(&id, Some(Status::Failed(e)), cx));
                    return;
                }
            };
            let url = pending.authorize_url.clone();
            if this
                .update(cx, |v, cx| {
                    cx.open_url(&url);
                    v.set_status(&id, Some(Status::Busy("Finish signing in in your browser…")), cx);
                })
                .is_err()
            {
                return;
            }
            let result = cx.background_spawn(async move { pending.finish() }).await;
            let _ = this.update(cx, |v, cx| {
                match result {
                    Ok(()) => {
                        if id == "composio" {
                            v.config.composio.account = true;
                            v.config.composio.enabled = true;
                        } else if let Some(s) = v.config.servers.iter_mut().find(|s| s.id == id) {
                            if let Transport::Http { oauth, .. } = &mut s.transport {
                                *oauth = true;
                            }
                            s.enabled = true;
                        }
                        v.set_status(&id, Some(Status::Ok("Signed in".into())), cx);
                    }
                    Err(e) => v.set_status(&id, Some(Status::Failed(e)), cx),
                }
                v.persist(cx);
            });
        })
        .detach();
    }

    /// Start the server, list its tools, and show the count.
    pub(super) fn test(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(server) = self.config.servers.iter().find(|s| s.id == id).cloned() else { return };
        self.set_status(&id, Some(Status::Busy("Connecting…")), cx);
        let task = cx.background_spawn(async move { crate::mcp_browser::test_server(&server.id, &server.transport) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                let status = match result {
                    Ok(n) => Status::Ok(format!("{n} tool{}", if n == 1 { "" } else { "s" })),
                    Err(e) => Status::Failed(e),
                };
                v.set_status(&id, Some(status), cx);
            });
        })
        .detach();
    }

    fn add_server(&mut self, server: McpServer, cx: &mut Context<Self>) {
        let id = server.id.clone();
        let sign_in = match &server.transport {
            Transport::Http { url, oauth: true, .. } if !crate::mcp_oauth::signed_in(&id) => Some(url.clone()),
            _ => None,
        };
        let local = !server.is_remote();
        self.config.servers.retain(|s| s.id != id);
        self.config.servers.push(server);
        self.persist(cx);
        if let Some(url) = sign_in {
            self.sign_in(id, url, cx);
        } else if local {
            self.test(id, cx);
        }
    }

    fn remove_server(&mut self, id: &str, cx: &mut Context<Self>) {
        self.config.servers.retain(|s| s.id != id);
        crate::mcp_oauth::sign_out(id);
        self.server_status.remove(id);
        self.persist(cx);
    }

    /// "Connect" on a catalog entry.
    fn connect(&mut self, entry: &'static Entry, window: &mut Window, cx: &mut Context<Self>) {
        match entry.auth {
            Auth::None | Auth::OAuth => {
                let id = super::unique_id(entry.id, &self.config.ids());
                self.add_server(entry.server(id, None), cx);
            }
            Auth::OAuthOrKey { label, .. } | Auth::Key { label, .. } | Auth::OptionalKey { label, .. } => {
                let style = ui::input_style(Theme::of(cx));
                let key = cx.new(|cx| {
                    let mut s = InputState::new(window, cx).placeholder(SharedString::from(label)).masked(true);
                    s.set_editor_style(style);
                    s
                });
                key.update(cx, |s, cx| s.focus(window, cx));
                self.dialog = Some(Dialog::Key { entry, key, error: None });
                cx.notify();
            }
        }
    }

    pub(super) fn open_custom(&mut self, editing: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let existing = editing.as_ref().and_then(|id| self.config.servers.iter().find(|s| s.id == *id)).cloned();
        let style = ui::input_style(Theme::of(cx));
        let input = |placeholder: &'static str, value: String, window: &mut Window, cx: &mut Context<Self>| {
            let style = style.clone();
            cx.new(|cx| {
                let mut s = InputState::new(window, cx).placeholder(placeholder);
                s.set_editor_style(style);
                s.set_value(value, window, cx);
                s
            })
        };
        let area = |placeholder: &'static str, value: String, rows: (usize, usize), window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut s = TextareaState::new(window, cx).auto_grow(rows.0, rows.1).placeholder(placeholder);
                s.set_value(value, window, cx);
                s
            })
        };
        let (mode, target, args, lines, oauth) = match existing.as_ref().map(|s| &s.transport) {
            Some(Transport::Http { url, headers, oauth }) => (1, url.clone(), String::new(), join_lines(headers, ": "), *oauth),
            Some(Transport::Stdio { command, args, env, .. }) => {
                (0, command.clone(), quote_args(args), join_lines(env, "="), false)
            }
            None => (0, String::new(), String::new(), String::new(), false),
        };
        let name = input("Name, e.g. My server", existing.as_ref().map(|s| s.name.clone()).unwrap_or_default(), window, cx);
        let form = CustomForm {
            editing,
            mode,
            target: input("npx, uvx, docker, or a path", target, window, cx),
            args: input("Arguments, e.g. -y @scope/server", args, window, cx),
            lines: area("KEY=value, one per line", lines, (2, 6), window, cx),
            paste: area("{\"mcpServers\": {\"name\": {\"command\": \"npx\", \"args\": [...]}}}", String::new(), (5, 14), window, cx),
            name: name.clone(),
            oauth,
            error: None,
            test: None,
        };
        name.update(cx, |s, cx| s.focus(window, cx));
        self.dialog = Some(Dialog::Custom(Box::new(form)));
        cx.notify();
    }

    /// The server the custom form describes, or what is missing.
    fn custom_server(&self, form: &CustomForm, cx: &gpui::App) -> Result<McpServer, String> {
        let name = form.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return Err("Give the server a name".into());
        }
        let target = form.target.read(cx).value().trim().to_string();
        let lines = form.lines.read(cx).value().to_string();
        let transport = if form.mode == 1 {
            let parsed = url::Url::parse(&target).map_err(|_| "Enter the server's full URL".to_string())?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err("The URL must start with https://".into());
            }
            Transport::Http { url: target, headers: parse_lines(&lines, ':')?, oauth: form.oauth }
        } else {
            if target.is_empty() {
                return Err("Enter the command that starts the server".into());
            }
            Transport::Stdio {
                command: target,
                args: split_args(form.args.read(cx).value().as_ref()),
                env: parse_lines(&lines, '=')?,
                cwd: None,
            }
        };
        let taken: Vec<String> =
            self.config.ids().into_iter().filter(|id| Some(id) != form.editing.as_ref()).collect();
        let id = form.editing.clone().unwrap_or_else(|| super::unique_id(&name, &taken));
        let catalog = form
            .editing
            .as_ref()
            .and_then(|id| self.config.servers.iter().find(|s| s.id == *id))
            .and_then(|s| s.catalog.clone());
        Ok(McpServer { id, name, catalog, enabled: true, transport })
    }

    fn save_custom(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::Custom(form)) = &self.dialog else { return };
        if form.mode == 2 {
            let text = form.paste.read(cx).value().to_string();
            match super::parse_pasted(&text, &self.config.ids()) {
                Ok(servers) => {
                    self.dialog = None;
                    for s in servers {
                        self.add_server(s, cx);
                    }
                }
                Err(e) => self.set_form_error(Some(e), cx),
            }
            return;
        }
        match self.custom_server(form, cx) {
            Ok(server) => {
                self.dialog = None;
                self.add_server(server, cx);
            }
            Err(e) => self.set_form_error(Some(e), cx),
        }
    }

    fn set_form_error(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        match &mut self.dialog {
            Some(Dialog::Custom(f)) => f.error = error,
            Some(Dialog::Key { error: e, .. }) => *e = error,
            None => {}
        }
        cx.notify();
    }

    fn test_custom(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::Custom(form)) = &self.dialog else { return };
        let server = match self.custom_server(form, cx) {
            Ok(s) => s,
            Err(e) => return self.set_form_error(Some(e), cx),
        };
        if server.uses_oauth() && !crate::mcp_oauth::signed_in(&server.id) {
            return self.set_form_error(Some("Save first, then sign in; Dove tests after sign-in".into()), cx);
        }
        if let Some(Dialog::Custom(f)) = &mut self.dialog {
            f.test = Some(Status::Busy("Connecting…"));
            f.error = None;
        }
        cx.notify();
        let task = cx.background_spawn(async move { crate::mcp_browser::test_server(&server.id, &server.transport) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                if let Some(Dialog::Custom(f)) = &mut v.dialog {
                    f.test = Some(match result {
                        Ok(n) => Status::Ok(format!("Connected: {n} tool{}", if n == 1 { "" } else { "s" })),
                        Err(e) => Status::Failed(e),
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_key(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::Key { entry, key, .. }) = &self.dialog else { return };
        let entry = *entry;
        let value = key.read(cx).value().trim().to_string();
        if value.is_empty() && !matches!(entry.auth, Auth::OptionalKey { .. }) {
            return self.set_form_error(Some("Paste a key, or sign in with your browser".into()), cx);
        }
        self.dialog = None;
        let id = super::unique_id(entry.id, &self.config.ids());
        let server = entry.server(id.clone(), Some(&value));
        self.add_server(server, cx);
        self.test(id, cx);
    }

    /// Connect the key dialog's entry with the GitHub CLI's token
    /// (`gh auth token`), read off the UI thread.
    fn use_gh_token(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::Key { entry, .. }) = &self.dialog else { return };
        let entry = *entry;
        let task = cx.background_spawn(async { dove_core::git::github::gh_token() });
        cx.spawn(async move |this, cx| {
            let token = task.await;
            let _ = this.update(cx, |v, cx| match token {
                Some(token) => {
                    v.dialog = None;
                    let id = super::unique_id(entry.id, &v.config.ids());
                    v.add_server(entry.server(id.clone(), Some(&token)), cx);
                    v.test(id, cx);
                }
                None => v.set_form_error(Some("The GitHub CLI is not signed in".into()), cx),
            });
        })
        .detach();
    }

    fn browser_sign_in_from_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::Key { entry, .. }) = &self.dialog else { return };
        let entry = *entry;
        self.dialog = None;
        let id = super::unique_id(entry.id, &self.config.ids());
        self.add_server(entry.server(id, None), cx);
    }

    // ----------------------------------------------------------------- render

    /// The MCP section (search and connected servers) and, separately, the
    /// catalog, which the page puts last because it is long.
    pub(super) fn render_mcp(&mut self, t: &Theme, cx: &mut Context<Self>) -> (AnyElement, Option<AnyElement>) {
        let query = self.search.read(cx).value().trim().to_lowercase();
        let matches = |name: &str, blurb: &str| {
            query.is_empty() || name.to_lowercase().contains(&query) || blurb.to_lowercase().contains(&query)
        };

        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(space::SM))
            .child(div().flex_1().min_w_0().child(page::search_field(t, &self.search)))
            .child(
                ctl::button_sm(t, "mcp-add-custom", "Add server", Tone::Filled)
                    .on_click(cx.listener(|this, _, window, cx| this.open_custom(None, window, cx))),
            );

        let connected: Vec<McpServer> = self
            .config
            .servers
            .iter()
            .filter(|s| {
                let blurb = s.catalog.as_deref().and_then(catalog::get).map(|e| e.blurb).unwrap_or("");
                matches(&s.name, blurb)
            })
            .cloned()
            .collect();
        let connected_card = (!connected.is_empty()).then(|| {
            page::card(t).children(connected.into_iter().enumerate().map(|(i, s)| self.server_row(t, i == 0, s, cx)))
        });

        let added: Vec<String> = self.config.servers.iter().filter_map(|s| s.catalog.clone()).collect();
        let discover: Vec<&'static Entry> = catalog::CATALOG
            .iter()
            .filter(|e| !added.iter().any(|a| a == e.id) && matches(e.name, e.blurb))
            .collect();
        let discover_card = (!discover.is_empty()).then(|| {
            page::card(t).children(discover.into_iter().enumerate().map(|(i, e)| {
                let meta = format!("{} · {}", e.blurb, if e.is_remote() { "Remote" } else { "Local" });
                page::row(t, i == 0)
                    .child(page::row_tile(t, e.icon, None))
                    .child(page::row_text(t, e.name, meta))
                    .child(
                        div().flex_none().flex().items_center().gap(px(4.0)).child(
                            small_icon(
                                SharedString::from(format!("mcp-docs-{}", e.id)),
                                icons::ARROW_UP_RIGHT,
                                "Open docs",
                                t,
                            )
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(e.docs))),
                        )
                        .child(
                            ctl::button_sm(t, SharedString::from(format!("mcp-connect-{}", e.id)), "Connect", Tone::Quiet)
                                .on_click(cx.listener(move |this, _, window, cx| this.connect(e, window, cx))),
                        ),
                    )
            }))
        });

        let head = div()
            .flex()
            .flex_col()
            .gap(px(space::MD))
            .child(toolbar)
            .when_some(connected_card, |d, card| d.child(page::section(t, "Connected", card)))
            .into_any_element();
        (head, discover_card.map(|card| page::section(t, "Discover", card).into_any_element()))
    }

    fn server_row(&self, t: &Theme, first: bool, s: McpServer, cx: &mut Context<Self>) -> gpui::Div {
        let entry = s.catalog.as_deref().and_then(catalog::get);
        let glyph = entry.map(|e| e.icon).unwrap_or(if s.is_remote() { icons::CLOUD } else { icons::TERMINAL });
        let needs_sign_in = s.uses_oauth() && !crate::mcp_oauth::signed_in(&s.id);
        let state = self.server_status.get(&s.id).cloned();
        let (ok, label): (bool, String) = match (&state, needs_sign_in) {
            (Some(Status::Busy(m)), _) => (false, (*m).to_string()),
            (Some(Status::Failed(e)), _) => (false, e.chars().take(90).collect()),
            (_, true) => (false, "Not signed in".into()),
            (Some(Status::Ok(m)), _) => (true, m.clone()),
            (None, _) if !s.enabled => (false, "Off".into()),
            (None, _) if s.uses_oauth() => (true, "Signed in".into()),
            (None, _) => (true, "Ready".into()),
        };
        let kind = match &s.transport {
            Transport::Http { url, .. } => {
                url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| url.clone())
            }
            Transport::Stdio { command, args, .. } => format!("{command} {}", quote_args(args)),
        };
        let id = s.id.clone();
        let on = s.enabled;
        let sign_in = match (&s.transport, needs_sign_in) {
            (Transport::Http { url, .. }, true) => Some(url.clone()),
            _ => None,
        };
        let busy = matches!(state, Some(Status::Busy(_)));
        let controls = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.0))
            .when_some(sign_in, |d, url| {
                let id = id.clone();
                d.child(
                    ctl::button_sm(t, SharedString::from(format!("mcp-signin-{id}")), "Sign in", Tone::Filled)
                        .on_click(cx.listener(move |this, _, _, cx| this.sign_in(id.clone(), url.clone(), cx))),
                )
            })
            .when(!busy && !needs_sign_in, |d| {
                let id = id.clone();
                d.child(
                    small_icon(SharedString::from(format!("mcp-test-{id}")), icons::REFRESH, "Test connection", t)
                        .on_click(cx.listener(move |this, _, _, cx| this.test(id.clone(), cx))),
                )
            })
            .child({
                let id = id.clone();
                small_icon(SharedString::from(format!("mcp-edit-{id}")), icons::PEN, "Edit", t)
                    .on_click(cx.listener(move |this, _, window, cx| this.open_custom(Some(id.clone()), window, cx)))
            })
            .child({
                let id = id.clone();
                small_icon(SharedString::from(format!("mcp-remove-{id}")), icons::TRASH_BIN_MINIMALISTIC, "Remove", t)
                    .on_click(cx.listener(move |this, _, _, cx| this.remove_server(&id, cx)))
            })
            .child(ctl::on_toggle(ctl::toggle(t, SharedString::from(format!("mcp-on-{id}")), on), cx, {
                let id = id.clone();
                move |this, _, cx| {
                    if let Some(s) = this.config.servers.iter_mut().find(|s| s.id == id) {
                        s.enabled = !on;
                    }
                    this.persist(cx);
                }
            }));
        page::row(t, first)
            .child(page::row_tile(t, glyph, None))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(page::row_text(t, s.name.clone(), format!("{kind} · tools as {}__*", s.id)))
                    .child(status(ok, label, t)),
            )
            .child(controls)
    }

    /// The open dialog, as a window-wide modal layer.
    pub(super) fn render_dialog(&mut self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let close = {
            let this = cx.entity().downgrade();
            move |_: &mut Window, cx: &mut gpui::App| {
                let _ = this.update(cx, |v, cx| {
                    v.dialog = None;
                    cx.notify();
                });
            }
        };
        let card = match self.dialog.as_ref()? {
            Dialog::Key { entry, key, error } => self.key_dialog(t, entry, key, error.clone(), cx),
            Dialog::Custom(_) => self.custom_dialog(t, cx),
        };
        Some(popover::modal("mcp-dialog", window.viewport_size(), t.scrim, Some(96.0), card, Some(Box::new(close))))
    }

    fn key_dialog(
        &self,
        t: &Theme,
        entry: &'static Entry,
        key: &Entity<InputState>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (_, _, label) = entry.auth.key().unwrap_or(("", "", "API key"));
        let message = match entry.auth {
            Auth::OAuthOrKey { .. } => format!("Sign in with your browser, or paste a {}.", label.to_lowercase()),
            Auth::OptionalKey { .. } => "Works without a key; a key raises the limits.".to_string(),
            _ => format!("Paste your {}. It stays in Dove's data folder.", label.to_lowercase()),
        };
        popover::dialog_card(t, 440.0)
            .child(popover::dialog_body(t, format!("Connect {}", entry.name), message))
            .child(
                div()
                    .px(px(20.0))
                    .flex()
                    .flex_col()
                    .gap(px(space::SM))
                    .child(field(key, 400.0, t))
                    .child(
                        div()
                            .id("mcp-key-docs")
                            .text_size(ty::META)
                            .text_color(t.accent)
                            .cursor_pointer()
                            .child(format!("Get a {} ↗", label.to_lowercase()))
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(entry.docs))),
                    )
                    .when_some(error, |d, e| d.child(page::error_strip(t, e))),
            )
            .child(
                popover::dialog_actions()
                    .child(
                        ctl::button_sm(t, "mcp-key-cancel", "Cancel", Tone::Quiet).on_click(cx.listener(|this, _, _, cx| {
                            this.dialog = None;
                            cx.notify();
                        })),
                    )
                    .when(entry.id == "github" && self.probe.gh_user.is_some(), |d| {
                        d.child(
                            ctl::button_sm(t, "mcp-key-gh", "Use GitHub CLI token", Tone::Quiet)
                                .on_click(cx.listener(|this, _, _, cx| this.use_gh_token(cx))),
                        )
                    })
                    .when(entry.auth.oauth(), |d| {
                        d.child(
                            ctl::button_sm(t, "mcp-key-browser", "Sign in with browser", Tone::Quiet)
                                .on_click(cx.listener(|this, _, _, cx| this.browser_sign_in_from_dialog(cx))),
                        )
                    })
                    .child(
                        ctl::button_sm(
                            t,
                            "mcp-key-save",
                            if matches!(entry.auth, Auth::OptionalKey { .. }) { "Connect" } else { "Use key" },
                            Tone::Filled,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.save_key(cx))),
                    ),
            )
            .into_any_element()
    }

    fn custom_dialog(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(Dialog::Custom(form)) = &self.dialog else { return div().into_any_element() };
        let this = cx.entity().downgrade();
        let modes = if form.editing.is_some() {
            vec![Segment::new("Local command"), Segment::new("Remote URL")]
        } else {
            vec![Segment::new("Local command"), Segment::new("Remote URL"), Segment::new("Paste JSON")]
        };
        let label = |text: &'static str| div().text_size(ty::META).text_color(t.text_muted).child(text);
        let area = |state: &Entity<TextareaState>| {
            div()
                .w_full()
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(radius::SELECT))
                .bg(t.wash(0.06))
                .text_size(ty::BODY)
                .font_family(t.font_mono.clone())
                .child(Textarea::new(state))
        };
        let body = match form.mode {
            2 => div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(label("Paste a config from a server's README (Claude Code, Cursor, Codex or OpenCode format)"))
                .child(area(&form.paste)),
            mode => div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(label("Name"))
                .child(field(&form.name, 400.0, t))
                .child(label(if mode == 1 { "Server URL" } else { "Command" }))
                .child(field(&form.target, 400.0, t))
                .when(mode == 0, |d| d.child(label("Arguments")).child(field(&form.args, 400.0, t)))
                .child(label(if mode == 1 {
                    "Headers, one per line (Authorization: Bearer …)"
                } else {
                    "Environment variables, one per line (KEY=value)"
                }))
                .child(area(&form.lines))
                .when(mode == 1, |d| {
                    let oauth = form.oauth;
                    d.child(
                        page::row(t, false)
                            .child(page::row_text(
                                t,
                                "Sign in with browser",
                                "For servers that use MCP sign-in (OAuth) instead of a key",
                            ))
                            .child(ctl::on_toggle(ctl::toggle(t, "mcp-custom-oauth", oauth), cx, move |this, _, cx| {
                                if let Some(Dialog::Custom(f)) = &mut this.dialog {
                                    f.oauth = !oauth;
                                }
                                cx.notify();
                            })),
                    )
                }),
        };
        let test = form.test.clone();
        popover::dialog_card(t, 480.0)
            .child(popover::dialog_body(
                t,
                if form.editing.is_some() { "Edit MCP server" } else { "Add an MCP server" },
                "Every agent Dove starts gets its tools, in any harness.",
            ))
            .child(
                div()
                    .px(px(20.0))
                    .flex()
                    .flex_col()
                    .gap(px(space::SM))
                    .child(ctl::segmented(t, "mcp-custom-mode", modes, form.mode, move |i, _, cx| {
                        let _ = this.update(cx, |v, cx| {
                            if let Some(Dialog::Custom(f)) = &mut v.dialog {
                                f.mode = i;
                                f.error = None;
                                f.test = None;
                            }
                            cx.notify();
                        });
                    }))
                    .child(body)
                    .when_some(test, |d, s| {
                        d.child(match s {
                            Status::Ok(m) => page::info_strip(t, m),
                            Status::Busy(m) => page::info_strip(t, m),
                            Status::Failed(e) => page::error_strip(t, e),
                        })
                    })
                    .when_some(form.error.clone(), |d, e| d.child(page::error_strip(t, e))),
            )
            .child(
                popover::dialog_actions()
                    .child(
                        ctl::button_sm(t, "mcp-custom-cancel", "Cancel", Tone::Quiet).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.dialog = None;
                                cx.notify();
                            },
                        )),
                    )
                    .when(form.mode != 2, |d| {
                        d.child(
                            ctl::button_sm(t, "mcp-custom-test", "Test", Tone::Quiet)
                                .on_click(cx.listener(|this, _, _, cx| this.test_custom(cx))),
                        )
                    })
                    .child(
                        ctl::button_sm(t, "mcp-custom-save", "Save", Tone::Filled)
                            .on_click(cx.listener(|this, _, _, cx| this.save_custom(cx))),
                    ),
            )
            .into_any_element()
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_quoted_args() {
        assert_eq!(split_args(r#"-y @scope/srv --dir "C:\My Files" ''"#), vec!["-y", "@scope/srv", "--dir", r"C:\My Files", ""]);
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn parses_env_and_header_lines() {
        let env = parse_lines("A=1\n# note\n\nB = two=2", '=').unwrap();
        assert_eq!(env.get("B").map(String::as_str), Some("two=2"));
        let h = parse_lines("Authorization: Bearer x", ':').unwrap();
        assert_eq!(h.get("Authorization").map(String::as_str), Some("Bearer x"));
        assert!(parse_lines("nope", '=').is_err());
    }

    #[test]
    fn args_round_trip() {
        let args = vec!["-y".to_string(), "a b".to_string(), "c".to_string()];
        assert_eq!(split_args(&quote_args(&args)), args);
    }
}

impl IntegrationsView {
    /// Sign in to a catalog server from another page (Settings › Memory):
    /// adds it when missing, else signs in again.
    pub fn sign_in_catalog(&mut self, catalog: &str, cx: &mut Context<Self>) {
        let Some(entry) = crate::integrations_catalog::get(catalog) else { return };
        if let Some(s) = self.config.servers.iter_mut().find(|s| s.catalog.as_deref() == Some(catalog)) {
            s.enabled = true;
            if let Transport::Http { url, .. } = &s.transport {
                let (id, url) = (s.id.clone(), url.clone());
                self.sign_in(id, url, cx);
            } else {
                self.persist(cx);
            }
            return;
        }
        let id = super::unique_id(entry.id, &self.config.ids());
        self.add_server(entry.server(id, None), cx);
    }

    /// Sign out of a catalog server (its token is dropped; the entry stays).
    pub fn sign_out_catalog(&mut self, catalog: &str, cx: &mut Context<Self>) {
        let ids: Vec<String> =
            self.config.servers.iter().filter(|s| s.catalog.as_deref() == Some(catalog)).map(|s| s.id.clone()).collect();
        for id in ids {
            crate::mcp_oauth::sign_out(&id);
            self.server_status.remove(&id);
        }
        self.persist(cx);
    }

    /// Changes whenever a catalog server is added, removed, switched or
    /// its sign-in state moves (for pages that follow it).
    pub fn catalog_key(&self, catalog: &str) -> String {
        let Some(s) = self.config.servers.iter().find(|s| s.catalog.as_deref() == Some(catalog)) else {
            return String::new();
        };
        let status = match self.server_status.get(&s.id) {
            Some(Status::Busy(t)) => format!("busy:{t}"),
            Some(Status::Ok(t)) => format!("ok:{t}"),
            Some(Status::Failed(e)) => format!("failed:{e}"),
            None => String::new(),
        };
        format!("{}|{}|{:?}|{status}", s.id, s.enabled, s.transport)
    }

    /// A catalog server's live state, for other pages: `(failed, text)`.
    pub fn catalog_status(&self, catalog: &str) -> Option<(bool, String)> {
        let s = self.config.servers.iter().find(|s| s.catalog.as_deref() == Some(catalog))?;
        match self.server_status.get(&s.id)? {
            Status::Busy(text) => Some((false, (*text).to_string())),
            Status::Failed(e) => Some((true, e.clone())),
            Status::Ok(_) => None,
        }
    }
}
