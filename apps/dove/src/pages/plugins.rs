//! Settings › Plugins: everything agents can use beyond the harness itself,
//! on one page. Tabs (Built in, Apps, MCPs, Skills) with counts and one
//! search; a row per plugin with its switch or status and a chevron to its
//! details; a directory to add more (MCP servers, apps, skills); an Add menu.
//!
//! The details are the plates `IntegrationsView` already draws (and the
//! Skills and Memory views Settings owns), so each plugin's settings live
//! in one place.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use super::{BrowserChoice, IntegrationsView, TOOLKITS, status};
use crate::assets::{icon, icons};
use crate::design::{space, ty};
use crate::integrations_catalog::{self as catalog};
use crate::kit::controls::{self as ctl, Tone};
use crate::kit::page;
use crate::theme::Theme;
use crate::ui;

/// The page's tabs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    BuiltIn,
    Apps,
    Mcps,
    Skills,
}

/// A built-in plugin with a details page of its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    Memory,
    Ego,
    Chrome,
    Computer,
    GitHub,
    Paper,
    Composio,
}

impl Item {
    fn title(self) -> &'static str {
        match self {
            Item::Memory => "Memory",
            Item::Ego => "ego lite",
            Item::Chrome => "Keplorayx for Chrome",
            Item::Computer => "Computer use",
            Item::GitHub => "GitHub",
            Item::Paper => "Paper",
            Item::Composio => "Composio",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    List,
    Directory,
    Detail(Item),
}

#[derive(Default)]
pub struct State {
    pub tab: Tab,
    pub mode: Mode,
    pub add_open: bool,
}

/// The plates `IntegrationsView::render` built this frame, for the details.
pub struct Plates {
    pub github: gpui::Div,
    pub ego: gpui::Div,
    pub chrome: gpui::Div,
    pub paper: gpui::Div,
    pub computer: gpui::Div,
    pub composio: gpui::Div,
    pub mcp: AnyElement,
}

/// What a deep link or a settings search result opens.
pub fn target(id: &str) -> Option<(Tab, Mode)> {
    Some(match id {
        "plugins" | "integrations" => (Tab::BuiltIn, Mode::List),
        "mcp" | "mcps" => (Tab::Mcps, Mode::List),
        "connectors" | "composio" | "apps" => (Tab::Apps, Mode::List),
        "skills" | "skill" | "agent_office_skills" => (Tab::Skills, Mode::List),
        "memory" | "supermemory" => (Tab::BuiltIn, Mode::Detail(Item::Memory)),
        "ego" | "browser" => (Tab::BuiltIn, Mode::Detail(Item::Ego)),
        "chrome" | "extension" => (Tab::BuiltIn, Mode::Detail(Item::Chrome)),
        "computer" | "computer_use" => (Tab::BuiltIn, Mode::Detail(Item::Computer)),
        "github" => (Tab::BuiltIn, Mode::Detail(Item::GitHub)),
        "directory" => (Tab::BuiltIn, Mode::Directory),
        _ => return None,
    })
}

/// A Composio toolkit's glyph.
fn toolkit_icon(slug: &str) -> &'static str {
    match slug {
        "github" => icons::BRAND_GITHUB,
        "slack" => icons::BRAND_SLACK,
        "notion" => icons::BRAND_NOTION,
        "linear" => icons::BRAND_LINEAR,
        "jira" => icons::BRAND_ATLASSIAN,
        "figma" => icons::BRAND_FIGMA,
        "googlecalendar" => icons::CALENDAR,
        "googledrive" => icons::DOCUMENT,
        _ => icons::WIDGET,
    }
}

const TILE: f32 = 40.0;

/// A plugin's icon on a soft square.
fn tile(t: &Theme, glyph: &'static str) -> gpui::Div {
    div()
        .size(px(TILE))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(10.0))
        .bg(t.wash(0.06))
        .child(icon(glyph).size(px(19.0)).text_color(t.text))
}

/// Name over a one-line description.
fn name_block(t: &Theme, title: impl Into<SharedString>, blurb: impl Into<SharedString>) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .text_size(ty::BODY)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(t.text)
                .truncate()
                .child(title.into()),
        )
        .child(div().text_size(ty::META).text_color(t.text_muted).truncate().child(blurb.into()))
}

/// What a plugin's settings sit on: Settings' own flat card.
fn row_card(t: &Theme) -> gpui::Div {
    page::card(t)
}

/// Rows in Settings' look: flat, inset, a hairline between each.
fn hairlines(t: &Theme, rows: Vec<AnyElement>) -> AnyElement {
    page::card(t)
        .children(
            rows.into_iter()
                .enumerate()
                .map(|(i, r)| div().mx(px(space::LG)).when(i > 0, |d| d.border_t_1().border_color(t.border)).child(r)),
        )
        .into_any_element()
}

impl IntegrationsView {
    /// Open a tab, a plugin's details or the directory (deep links, search).
    pub fn open(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some((tab, mode)) = target(id) {
            self.plugins.tab = tab;
            self.plugins.mode = mode;
            self.plugins.add_open = false;
            cx.notify();
        }
    }

    /// "Browse directory".
    pub fn browse(&mut self, cx: &mut Context<Self>) {
        self.plugins.mode = Mode::Directory;
        self.plugins.add_open = false;
        cx.notify();
    }

    /// The header's Add button.
    pub fn toggle_add(&mut self, cx: &mut Context<Self>) {
        self.plugins.add_open = !self.plugins.add_open;
        cx.notify();
    }

    fn back_to_list(&mut self, cx: &mut Context<Self>) {
        self.plugins.mode = Mode::List;
        cx.notify();
    }

    fn show(&mut self, item: Item, cx: &mut Context<Self>) {
        self.plugins.mode = Mode::Detail(item);
        self.plugins.add_open = false;
        cx.notify();
    }

    fn query(&self, cx: &gpui::App) -> String {
        self.search.read(cx).value().trim().to_lowercase()
    }

    /// Composio toolkits picked for agents (when any are).
    fn picked_toolkits(&self) -> Vec<(&'static str, &'static str)> {
        TOOLKITS.iter().copied().filter(|(slug, _)| self.config.composio.toolkits.iter().any(|k| k == slug)).collect()
    }

    fn set_toolkit(&mut self, slug: &str, on: bool, cx: &mut Context<Self>) {
        let list = &mut self.config.composio.toolkits;
        list.retain(|k| k != slug);
        if on {
            list.push(slug.to_string());
            self.config.composio.enabled = true;
        }
        self.persist(cx);
    }

    // ----------------------------------------------------------------- render

    pub(super) fn render_plugins(&mut self, plates: Plates, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let query = self.query(cx);
        // The Skills view follows this page's search, and shows its
        // libraries while the directory is open.
        let directory = self.plugins.mode == Mode::Directory;
        if let Some(skills) = self.skills.as_ref().and_then(|w| w.upgrade()) {
            skills.update(cx, |s, cx| {
                s.set_embedded(true, cx);
                s.set_query(&query, cx);
                s.set_discover(directory, cx);
            });
        }
        let body = match self.plugins.mode {
            Mode::Detail(item) => self.render_detail(item, plates, &t, cx),
            Mode::Directory => self.render_directory(&query, &t, window, cx),
            Mode::List => self.render_list(plates, &query, &t, cx),
        };
        div()
            .relative()
            .pt(px(space::LG))
            .flex()
            .flex_col()
            .child(body)
            .children(self.plugins.add_open.then(|| self.render_add_menu(&t, cx)))
            .into_any_element()
    }

    fn tab_counts(&self, cx: &gpui::App) -> [usize; 4] {
        let skills = self.skills.as_ref().and_then(|w| w.upgrade()).map(|s| s.read(cx).count()).unwrap_or(0);
        [6, 1 + self.picked_toolkits().len(), self.config.servers.len(), skills]
    }

    fn render_tabs(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let counts = self.tab_counts(cx);
        let tabs = [(Tab::BuiltIn, "Built in"), (Tab::Apps, "Apps"), (Tab::Mcps, "MCPs"), (Tab::Skills, "Skills")];
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(space::MD))
            .border_b_1()
            .border_color(t.border)
            .child(div().flex().items_end().gap(px(4.0)).children(tabs.into_iter().zip(counts).map(
                |((tab, label), n)| {
                    // Underline tabs, as in Settings: the text carries the
                    // state, a 2px bar marks the open one.
                    let on = self.plugins.tab == tab;
                    let (ink_on, faint) = (t.text, t.text_faint);
                    div()
                        .id(SharedString::from(format!("plugins-tab-{label}")))
                        .role(gpui::Role::Tab)
                        .aria_selected(on)
                        .aria_label(label)
                        .h(px(34.0))
                        .px(px(8.0))
                        .flex()
                        .flex_col()
                        .justify_between()
                        .cursor_pointer()
                        .text_size(ty::BODY)
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .text_color(if on { t.text } else { t.text_muted })
                                .when(on, |d| d.font_weight(gpui::FontWeight::MEDIUM))
                                .when(!on, |d| d.hover(move |s| s.text_color(ink_on)))
                                .child(label)
                                .child(div().text_size(ty::META).text_color(faint).child(n.to_string())),
                        )
                        .child(div().h(px(2.0)).w_full().rounded_full().when(on, |d| d.bg(t.accent)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.plugins.tab = tab;
                            cx.notify();
                        }))
                },
            )))
            .child(div().flex_1())
            .child(page::search_field(t, &self.search).w(px(260.0)).max_w_full())
    }

    /// One plugin: icon, name and status; its switch or status on the
    /// right; the left side and the chevron open its details.
    #[allow(clippy::too_many_arguments)]
    fn plugin_row(
        &self,
        id: &'static str,
        glyph: &'static str,
        blurb: SharedString,
        right: Option<AnyElement>,
        item: Option<Item>,
        title: Option<&'static str>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = title.or(item.map(Item::title)).unwrap_or(id);
        let open = div()
            .id(SharedString::from(format!("plugin-open-{id}")))
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(14.0))
            .child(tile(t, glyph))
            .child(name_block(t, title, blurb))
            .when_some(item, |d, item| {
                d.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.show(item, cx)))
            });
        div()
            .w_full()
            .min_h(px(64.0))
            .py(px(12.0))
            .flex()
            .items_center()
            .gap(px(14.0))
            .child(open)
            .children(right)
            .when_some(item, |d, item| {
                d.child(
                    div()
                        .id(SharedString::from(format!("plugin-chevron-{id}")))
                        .size(px(26.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .hover({
                            let h = t.wash(0.08);
                            move |s| s.bg(h)
                        })
                        .tooltip(ui::tip(format!("{} settings", item.title())))
                        .child(icon(icons::ALT_ARROW_RIGHT).size(px(14.0)).text_color(t.text_faint))
                        .on_click(cx.listener(move |this, _, _, cx| this.show(item, cx))),
                )
            })
            .into_any_element()
    }

    fn render_list(&mut self, plates: Plates, query: &str, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let matches = |name: &str, blurb: &str| {
            query.is_empty() || name.to_lowercase().contains(query) || blurb.to_lowercase().contains(query)
        };
        let tabs = self.render_tabs(t, cx);
        let body: AnyElement = match self.plugins.tab {
            Tab::BuiltIn => {
                let rows = self.builtin_rows(t, cx);
                let rows: Vec<AnyElement> =
                    rows.into_iter().filter(|(name, blurb, _)| matches(name, blurb)).map(|(_, _, r)| r).collect();
                if rows.is_empty() {
                    page::info_strip(t, "No built-in plugins match").into_any_element()
                } else {
                    hairlines(t, rows)
                }
            }
            Tab::Apps => self.render_apps(query, t, cx),
            Tab::Mcps => div()
                .flex()
                .flex_col()
                .gap(px(space::SM))
                .child(row_card(t).overflow_hidden().child(plates.mcp))
                .child(
                    div().flex().gap(px(space::SM)).child(
                        ctl::button_sm(t, "plugins-mcp-custom", "Add a custom server", Tone::Quiet)
                            .on_click(cx.listener(|this, _, window, cx| this.open_custom(None, window, cx))),
                    ),
                )
                .into_any_element(),
            Tab::Skills => self.render_skills_tab(t, cx),
        };
        div().flex().flex_col().gap(px(space::LG)).child(tabs).child(body).into_any_element()
    }

    /// (name, blurb, row) for each built-in plugin, for the search.
    fn builtin_rows(&self, t: &Theme, cx: &mut Context<Self>) -> Vec<(String, String, AnyElement)> {
        let mut out = Vec::new();
        let mut push = |name: &str, blurb: &str, row: AnyElement| out.push((name.to_string(), blurb.to_string(), row));

        let memory_on = dove_core::memory::backend().is_some();
        let blurb = "Every agent remembers you and each project across sessions (Supermemory)";
        let row = self.plugin_row(
            "memory",
            icons::STAR,
            blurb.into(),
            Some(status(memory_on, if memory_on { "Connected" } else { "Not connected" }, t).into_any_element()),
            Some(Item::Memory),
            None,
            t,
            cx,
        );
        push("Memory", blurb, row);

        let ego = &self.probe.ego;
        let ready = matches!(self.ego_health, Some(crate::ego::Health::Ready));
        let failed = matches!(self.ego_health, Some(crate::ego::Health::Failed(_)));
        let version = ego.version().map(|v| format!(" · {v}")).unwrap_or_default();
        let (ok, label) = match (ego.cli.is_some(), ego.app.is_some()) {
            (true, _) if failed => (false, "Not responding".to_string()),
            (true, _) if ready => (true, format!("Ready{version}")),
            (true, _) => (true, format!("Installed{version}")),
            (false, true) => (false, "Finish onboarding".into()),
            (false, false) => (false, "Not installed".into()),
        };
        let choice = match self.config.browser {
            BrowserChoice::Ego => "Agents browse with ego lite only",
            BrowserChoice::Both => "Agents browse with ego lite first, Keplorayx's browser as a fallback",
            BrowserChoice::Keplorayx => "Agents use Keplorayx's browser; ego lite is off",
        };
        let row = self.plugin_row(
            "ego",
            icons::GLOBAL,
            choice.into(),
            Some(status(ok, label, t).into_any_element()),
            Some(Item::Ego),
            None,
            t,
            cx,
        );
        push("ego lite browser", choice, row);

        let browser = crate::browser::control::connected_browser();
        let blurb = "Agents work in your own Chrome, Edge or Brave, only on sites you allow";
        let label = match browser.as_deref() {
            Some(ua) if ua.contains("Edg/") => "Connected to Edge",
            Some(ua) if ua.contains("Brave") => "Connected to Brave",
            Some(_) => "Connected to Chrome",
            None => "Not connected",
        };
        let row = self.plugin_row(
            "chrome",
            icons::GLOBE,
            blurb.into(),
            Some(status(browser.is_some(), label, t).into_any_element()),
            Some(Item::Chrome),
            None,
            t,
            cx,
        );
        push("Keplorayx for Chrome extension browser", blurb, row);

        let on = self.config.computer.enabled;
        let installed = self.probe.cu_plugin.is_some() || self.probe.cua.is_some();
        let blurb = "Agents see and use your desktop apps in the background";
        let right: AnyElement = if installed {
            ctl::on_toggle(ctl::toggle(t, "plugin-cu-on", on), cx, move |this, _, cx| {
                this.config.computer.enabled = !on;
                this.persist(cx);
            })
            .into_any_element()
        } else {
            status(false, "Not installed", t).into_any_element()
        };
        let row =
            self.plugin_row("computer", icons::MONITOR, blurb.into(), Some(right), Some(Item::Computer), None, t, cx);
        push("Computer use desktop", blurb, row);

        let (ok, label) = match (&self.probe.gh, &self.probe.gh_user) {
            (Some(_), Some(user)) => (true, format!("Signed in as {user}")),
            (Some(_), None) => (false, "Not signed in".to_string()),
            (None, _) => (false, "Not installed".to_string()),
        };
        let blurb = "Pull requests, reviews and PR automations through the GitHub CLI";
        let row = self.plugin_row(
            "github",
            icons::BRAND_GITHUB,
            blurb.into(),
            Some(status(ok, label, t).into_any_element()),
            Some(Item::GitHub),
            None,
            t,
            cx,
        );
        push("GitHub", blurb, row);

        let paper_on = self.config.paper.enabled;
        let blurb = "Agents read and edit the design open in Paper Desktop";
        let row = self.plugin_row(
            "paper",
            icons::PEN,
            blurb.into(),
            Some(
                ctl::on_toggle(ctl::toggle(t, "plugin-paper-on", paper_on), cx, move |this, _, cx| {
                    this.config.paper.enabled = !paper_on;
                    this.persist(cx);
                    this.refresh(cx);
                })
                .into_any_element(),
            ),
            Some(Item::Paper),
            None,
            t,
            cx,
        );
        push("Paper design", blurb, row);
        out
    }

    fn render_apps(&mut self, query: &str, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let on = self.config.composio.enabled;
        let account = self.config.composio.account;
        let signed_in = crate::mcp_oauth::signed_in("composio");
        let has_key = !self.config.composio.api_key.is_empty();
        let label = match (on, account, signed_in, has_key) {
            (false, ..) => "Off",
            (true, true, true, _) => "Signed in",
            (true, true, false, _) => "Not signed in",
            (true, false, _, true) => "API key set",
            (true, false, _, false) => "Needs a key",
        };
        let ok = on && ((account && signed_in) || (!account && has_key));
        let composio = self.plugin_row(
            "composio",
            icons::WIDGET,
            "Gmail, Slack, Notion, Linear and 1000+ apps through your Composio account".into(),
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.0))
                    .child(status(ok, label, t))
                    .child(ctl::on_toggle(ctl::toggle(t, "plugin-composio-on", on), cx, move |this, _, cx| {
                        this.config.composio.enabled = !on;
                        this.persist(cx);
                    }))
                    .into_any_element(),
            ),
            Some(Item::Composio),
            None,
            t,
            cx,
        );
        let picked = self.picked_toolkits();
        let rows: Vec<AnyElement> = picked
            .into_iter()
            .filter(|(slug, name)| query.is_empty() || slug.contains(query) || name.to_lowercase().contains(query))
            .map(|(slug, name)| {
                let id: &'static str = slug;
                let blurb: SharedString = format!("{name} through Composio, for every agent").into();
                self.plugin_row(
                    id,
                    toolkit_icon(slug),
                    blurb,
                    Some(
                        ctl::on_toggle(ctl::toggle(t, SharedString::from(format!("plugin-tk-{slug}")), true), cx, {
                            move |this, _, cx| this.set_toolkit(slug, false, cx)
                        })
                        .into_any_element(),
                    ),
                    None,
                    Some(name),
                    t,
                    cx,
                )
            })
            .collect();
        let hint = if rows.is_empty() {
            "Agents can use any app you link in Composio. Pick specific apps in the directory to limit them."
        } else {
            "Agents are limited to these apps. Turn them all off to allow every app you link in Composio."
        };
        div()
            .flex()
            .flex_col()
            .gap(px(space::SM))
            .child(hairlines(t, std::iter::once(composio).chain(rows).collect()))
            .child(div().px(px(page::LABEL_INSET)).text_size(ty::META).text_color(t.text_faint).child(hint))
            .into_any_element()
    }

    fn render_skills_tab(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(skills) = self.skills.as_ref().and_then(|w| w.upgrade()) else {
            return page::info_strip(t, "Skills are loading…").into_any_element();
        };
        let (updating, has_git) = {
            let s = skills.read(cx);
            (s.updating(), s.has_git())
        };
        let update =
            ctl::button_sm(t, "plugins-skills-update", if updating { "Updating…" } else { "Update all" }, Tone::Quiet);
        let update: AnyElement = if has_git && !updating {
            let s = skills.clone();
            update.on_click(move |_, _, cx| s.update(cx, |v, cx| v.update_all(cx))).into_any_element()
        } else {
            ctl::disabled(update, if updating { "Checking GitHub" } else { "No skills from GitHub to update" })
                .into_any_element()
        };
        let s = skills.clone();
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::SM))
                    .child(div().flex_1())
                    .child(
                        ctl::button_sm(t, "plugins-skills-browse", "Find skills", Tone::Quiet)
                            .on_click(cx.listener(|this, _, _, cx| this.browse(cx))),
                    )
                    .child(update)
                    .child(ctl::button_sm(t, "plugins-skills-add", "Add library", Tone::Quiet).on_click(cx.listener(
                        move |this, _, window, cx| {
                            this.browse(cx);
                            s.update(cx, |v, cx| v.add_repository(window, cx));
                        },
                    ))),
            )
            .child(skills)
            .into_any_element()
    }

    fn back_bar(&self, title: &str, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let hover = t.wash(0.06);
        div()
            .flex()
            .items_center()
            .gap(px(space::SM))
            .child(
                div()
                    .id("plugins-back")
                    .h(px(28.0))
                    .pl(px(6.0))
                    .pr(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .text_size(ty::BODY)
                    .text_color(t.text_muted)
                    .hover(move |s| s.bg(hover))
                    .child(icon(icons::ALT_ARROW_LEFT).size(px(14.0)).text_color(t.text_muted))
                    .child("Plugins")
                    .on_click(cx.listener(|this, _, _, cx| this.back_to_list(cx))),
            )
            .child(div().text_color(t.text_faint).child("/"))
            .child(
                div()
                    .text_size(ty::BODY)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(t.text)
                    .child(title.to_string()),
            )
    }

    fn render_detail(&mut self, item: Item, plates: Plates, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let body: AnyElement = match item {
            Item::Memory => match self.memory.as_ref().and_then(|w| w.upgrade()) {
                Some(m) => m.into_any_element(),
                None => page::info_strip(t, "Memory is loading…").into_any_element(),
            },
            Item::Ego => row_card(t).overflow_hidden().child(plates.ego).into_any_element(),
            Item::Chrome => row_card(t).overflow_hidden().child(plates.chrome).into_any_element(),
            Item::Computer => row_card(t).overflow_hidden().child(plates.computer).into_any_element(),
            Item::GitHub => row_card(t).overflow_hidden().child(plates.github).into_any_element(),
            Item::Paper => row_card(t).overflow_hidden().child(plates.paper).into_any_element(),
            Item::Composio => row_card(t).overflow_hidden().child(plates.composio).into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(space::LG))
            .child(self.back_bar(item.title(), t, cx))
            .child(body)
            .into_any_element()
    }

    /// A directory entry: icon, name, blurb and a + (or "Added").
    fn dir_tile(
        &self,
        id: SharedString,
        glyph: &'static str,
        name: &'static str,
        blurb: &'static str,
        added: bool,
        t: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        let hover = t.wash(0.05);
        div()
            .id(id)
            .h(px(72.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(14.0))
            .rounded(px(10.0))
            .when(!added, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
            .child(tile(t, glyph))
            .child(name_block(t, name, blurb))
            .child(if added {
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .text_size(ty::META)
                    .text_color(t.text_faint)
                    .child(icon(icons::CHECK).size(px(13.0)).text_color(t.success))
                    .child("Added")
                    .into_any_element()
            } else {
                div()
                    .size(px(30.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .child(icon(icons::PLUS).size(px(18.0)).text_color(t.text))
                    .into_any_element()
            })
    }

    fn grid(children: Vec<AnyElement>) -> gpui::Div {
        div().w_full().grid().grid_cols(2).gap_x(px(space::LG)).gap_y(px(2.0)).children(children)
    }

    fn render_directory(&mut self, query: &str, t: &Theme, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let matches = |name: &str, blurb: &str| {
            query.is_empty() || name.to_lowercase().contains(query) || blurb.to_lowercase().contains(query)
        };
        let added: Vec<String> = self.config.servers.iter().filter_map(|s| s.catalog.clone()).collect();
        let servers: Vec<AnyElement> = catalog::CATALOG
            .iter()
            .filter(|e| matches(e.name, e.blurb))
            .map(|e| {
                let is_added = added.iter().any(|a| a == e.id);
                let tile = self.dir_tile(format!("dir-mcp-{}", e.id).into(), e.icon, e.name, e.blurb, is_added, t);
                if is_added {
                    tile.into_any_element()
                } else {
                    let place = if e.is_remote() { "a remote server" } else { "runs on this computer" };
                    tile.tooltip(ui::tip(format!("Connect {} ({place})", e.name)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.connect(e, window, cx);
                            this.plugins.tab = Tab::Mcps;
                        }))
                        .into_any_element()
                }
            })
            .collect();
        let apps: Vec<AnyElement> = TOOLKITS
            .iter()
            .filter(|(slug, name)| matches(name, slug))
            .map(|&(slug, name)| {
                let picked = self.config.composio.toolkits.iter().any(|k| k == slug);
                let tile = self.dir_tile(
                    format!("dir-app-{slug}").into(),
                    toolkit_icon(slug),
                    name,
                    "Through Composio",
                    picked,
                    t,
                );
                if picked {
                    tile.into_any_element()
                } else {
                    tile.tooltip(ui::tip(format!("Add {name}")))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_toolkit(slug, true, cx);
                            // Without a key or an account Composio can't run yet.
                            let c = &this.config.composio;
                            if c.api_key.is_empty() && !(c.account && crate::mcp_oauth::signed_in("composio")) {
                                this.show(Item::Composio, cx);
                            }
                        }))
                        .into_any_element()
                }
            })
            .collect();
        let mut out = div().flex().flex_col().gap(px(space::LG)).child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(space::MD))
                .child(self.back_bar("Directory", t, cx))
                .child(div().flex_1())
                .child(page::search_field(t, &self.search).w(px(260.0)).max_w_full()),
        );
        let heading = |label: &'static str| {
            div()
                .px(px(page::LABEL_INSET))
                .flex()
                .items_center()
                .gap(px(4.0))
                .text_size(ty::HEADING)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(t.text)
                .child(label)
        };
        if !servers.is_empty() {
            out = out.child(
                div().flex().flex_col().gap(px(space::SM)).child(heading("MCP servers")).child(Self::grid(servers)),
            );
        }
        if !apps.is_empty() {
            out = out.child(div().flex().flex_col().gap(px(space::SM)).child(heading("Apps")).child(Self::grid(apps)));
        }
        if let Some(skills) = self.skills.as_ref().and_then(|w| w.upgrade()) {
            out = out.child(div().flex().flex_col().gap(px(space::SM)).child(heading("Skills")).child(skills));
        }
        out.into_any_element()
    }

    fn render_add_menu(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let skills = self.skills.as_ref().and_then(|w| w.upgrade());
        let item = |id: &'static str, glyph: &'static str, label: &'static str| {
            crate::kit::popover::menu_item(t, id, Some(glyph), label, false, false, false)
        };
        div()
            .absolute()
            .top(px(-6.0))
            .right_0()
            .w(px(260.0))
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.plugins.add_open = false;
                cx.notify();
            }))
            .child(ui::menu_card(
                t,
                div()
                    .flex()
                    .flex_col()
                    .child(item("plugins-add-mcp", icons::WIDGET_ADD, "MCP server…").on_click(cx.listener(
                        |this, _, window, cx| {
                            this.plugins.add_open = false;
                            this.plugins.tab = Tab::Mcps;
                            this.plugins.mode = Mode::List;
                            this.open_custom(None, window, cx);
                        },
                    )))
                    .child(item("plugins-add-app", icons::WIDGET, "App through Composio").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.plugins.tab = Tab::Apps;
                            this.show(Item::Composio, cx);
                        },
                    )))
                    .child(item("plugins-add-skill", icons::MAGIC_STICK_3, "Skill library from GitHub…").on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.plugins.tab = Tab::Skills;
                            this.browse(cx);
                            if let Some(s) = skills.clone() {
                                s.update(cx, |v, cx| v.add_repository(window, cx));
                            }
                        }),
                    ))
                    .child(ui::separator(t))
                    .child(
                        item("plugins-add-browse", icons::MAGNIFER, "Browse the directory")
                            .on_click(cx.listener(|this, _, _, cx| this.browse(cx))),
                    ),
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_section_links_open_the_right_place() {
        assert_eq!(target("skills"), Some((Tab::Skills, Mode::List)));
        assert_eq!(target("composio"), Some((Tab::Apps, Mode::List)));
        assert_eq!(target("memory"), Some((Tab::BuiltIn, Mode::Detail(Item::Memory))));
        assert_eq!(target("mcp"), Some((Tab::Mcps, Mode::List)));
        assert_eq!(target("integrations"), Some((Tab::BuiltIn, Mode::List)));
        assert_eq!(target("nope"), None);
    }

    #[test]
    fn every_toolkit_has_a_glyph() {
        for (slug, _) in TOOLKITS {
            assert!(!toolkit_icon(slug).is_empty());
        }
    }
}
