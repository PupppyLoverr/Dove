//! Settings → Machines: where agents run. This computer and the machines it
//! can already reach (WSL, SSH hosts, Docker containers, Cua Spaces), the
//! cloud providers a key unlocks (with their boxes), and paired devices.
//!
//! Everything here that touches the outside world (discovery, probes, the
//! providers' HTTP APIs, the keys file) runs on the background executor; the
//! view only holds results. A saved key is never read back into the UI: the
//! row says "Key saved" and offers Replace.

use std::collections::HashMap;

use dove_core::host::{self, Host};
use dove_core::hosts::{self, BoxInfo, BoxSpec, Keys, Probe, Provider};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, Focusable as _, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    px,
};
use gpui_base::input::{InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::design::{space, ty};
use crate::kit::controls::{self as ctl, Tone};
use crate::kit::page;
use crate::theme::Theme;
use crate::ui;

/// One discovered machine and what its probe found (`None` while checking).
struct HostRow {
    host: Host,
    probe: Option<Probe>,
}

/// A box action in flight, shown on its button.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BoxAction {
    Wake,
    Sleep,
    Delete,
}

impl BoxAction {
    fn busy_label(self) -> &'static str {
        match self {
            BoxAction::Wake => "Waking…",
            BoxAction::Sleep => "Sleeping…",
            BoxAction::Delete => "Deleting…",
        }
    }
}

/// One cloud provider's row state.
struct Cloud {
    provider: &'static dyn Provider,
    /// The key field, for providers that take a key.
    key: Option<Entity<InputState>>,
    has_key: bool,
    /// Showing the field again over a saved key.
    replacing: bool,
    saving: bool,
    /// Second click on Remove deletes the saved key.
    confirm_remove: bool,
    probe: Option<Probe>,
    testing: bool,
    boxes: Option<Vec<BoxInfo>>,
    listing: bool,
    creating: bool,
    busy: HashMap<String, BoxAction>,
    /// The box whose Delete was clicked once and now asks again.
    confirm_delete: Option<String>,
    error: Option<String>,
}

impl Cloud {
    /// Whether probing makes sense: a key is saved, or none is needed.
    fn can_probe(&self) -> bool {
        !self.provider.needs_key() || self.has_key
    }
}

pub struct MachinesView {
    loaded: bool,
    /// Bumped by every refresh, so late results of an older one are dropped.
    generation: u64,
    /// `None` while discovery runs.
    hosts: Option<Vec<HostRow>>,
    /// Paired devices; `None` while loading.
    paired: Option<Vec<dove_core::paired::Device>>,
    clouds: Vec<Cloud>,
    _subs: Vec<Subscription>,
}

impl MachinesView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let style = ui::input_style(Theme::of(cx));
        let mut subs = Vec::new();
        let mut clouds = Vec::new();
        for (i, provider) in hosts::providers().into_iter().enumerate() {
            let key = provider.needs_key().then(|| {
                let input = cx.new(|cx| {
                    let mut s = InputState::new(window, cx).placeholder("Paste an API key").masked(true);
                    s.set_editor_style(style.clone());
                    s
                });
                subs.push(cx.subscribe_in(&input, window, move |this, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::PressEnter { .. } => this.save_key(i, window, cx),
                    InputEvent::Change | InputEvent::Focus | InputEvent::Blur => cx.notify(),
                }));
                input
            });
            clouds.push(Cloud {
                provider,
                key,
                has_key: false,
                replacing: false,
                saving: false,
                confirm_remove: false,
                probe: None,
                testing: false,
                boxes: None,
                listing: false,
                creating: false,
                busy: HashMap::new(),
                confirm_delete: None,
                error: None,
            });
        }
        Self { loaded: false, generation: 0, hosts: None, paired: None, clouds, _subs: subs }
    }

    /// Start discovery and probes the first time the page is shown.
    pub fn ensure_loaded(&mut self, cx: &mut Context<Self>) {
        if !self.loaded {
            self.refresh(cx);
        }
    }

    pub fn is_refreshing(&self) -> bool {
        self.loaded
            && (self.hosts.is_none() || self.paired.is_none() || self.clouds.iter().any(|c| c.testing || c.listing))
    }

    /// Look for machines again, probe each, and re-check every provider.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.loaded = true;
        self.generation += 1;
        let generation = self.generation;
        self.hosts = None;

        // Machines: discovery, then one probe per machine so a slow SSH
        // host doesn't hold up the rest. Cloud boxes are listed under their
        // provider below, not here.
        let found = cx.background_spawn(async move {
            host::discover()
                .into_iter()
                .filter(|h| !matches!(h, Host::Sandbox { .. } | Host::Node { .. }))
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let found = found.await;
            let _ = this.update(cx, |v, cx| {
                if v.generation != generation {
                    return;
                }
                v.hosts = Some(
                    found
                        .into_iter()
                        .map(|host| {
                            // This computer needs no check (and `true` isn't
                            // a program on Windows).
                            let probe = host.is_local().then_some(Probe::Ready);
                            HostRow { host, probe }
                        })
                        .collect(),
                );
                let pending: Vec<(usize, Host)> = v
                    .hosts
                    .iter()
                    .flatten()
                    .enumerate()
                    .filter(|(_, r)| r.probe.is_none())
                    .map(|(i, r)| (i, r.host.clone()))
                    .collect();
                for (i, h) in pending {
                    v.probe_host(generation, i, h, cx);
                }
                cx.notify();
            });
        })
        .detach();

        // Paired devices (and whether this build can pair).
        self.paired = None;
        let paired = cx.background_spawn(async move { dove_core::paired::devices() });
        cx.spawn(async move |this, cx| {
            let devices = paired.await;
            let _ = this.update(cx, |v, cx| {
                if v.generation == generation {
                    v.paired = Some(devices);
                    cx.notify();
                }
            });
        })
        .detach();

        // Providers: which have keys, then probe those that can be.
        let keys = cx.background_spawn(async move { Keys::load() });
        cx.spawn(async move |this, cx| {
            let keys = keys.await;
            let _ = this.update(cx, |v, cx| {
                for i in 0..v.clouds.len() {
                    let c = &mut v.clouds[i];
                    c.has_key = keys.has(c.provider.id());
                    c.confirm_remove = false;
                    c.confirm_delete = None;
                    if c.can_probe() {
                        v.test(i, cx);
                    } else {
                        c.probe = None;
                        c.boxes = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn probe_host(&mut self, generation: u64, i: usize, h: Host, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move { host::probe(&h) });
        cx.spawn(async move |this, cx| {
            let probe = task.await;
            let _ = this.update(cx, |v, cx| {
                if v.generation != generation {
                    return;
                }
                if let Some(row) = v.hosts.as_mut().and_then(|rows| rows.get_mut(i)) {
                    row.probe = Some(probe);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    // ------------------------------------------------------------ providers

    /// Run the provider's probe; when it is ready, list its boxes.
    fn test(&mut self, i: usize, cx: &mut Context<Self>) {
        let c = &mut self.clouds[i];
        if c.testing || !c.can_probe() {
            return;
        }
        c.testing = true;
        c.error = None;
        let p = c.provider;
        let task = cx.background_spawn(async move { p.probe(&Keys::load()) });
        cx.spawn(async move |this, cx| {
            let probe = task.await;
            let _ = this.update(cx, |v, cx| {
                let c = &mut v.clouds[i];
                c.testing = false;
                let ready = probe.is_ready();
                c.probe = Some(probe);
                if ready {
                    v.list(i, cx);
                } else {
                    c.boxes = None;
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn list(&mut self, i: usize, cx: &mut Context<Self>) {
        let c = &mut self.clouds[i];
        if c.listing {
            return;
        }
        c.listing = true;
        let p = c.provider;
        let task = cx.background_spawn(async move { p.list(&Keys::load()).map_err(|e| format!("{e:#}")) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                let c = &mut v.clouds[i];
                c.listing = false;
                match result {
                    Ok(boxes) => c.boxes = Some(boxes),
                    Err(e) => {
                        c.boxes = None;
                        c.error = Some(format!("Could not list boxes: {e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn save_key(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let c = &mut self.clouds[i];
        let Some(input) = c.key.clone() else { return };
        let key = input.read(cx).value().trim().to_string();
        if key.is_empty() || c.saving {
            return;
        }
        // The key leaves the field at once; it is never shown again.
        input.update(cx, |s, cx| s.set_value("", window, cx));
        c.saving = true;
        c.error = None;
        let id = c.provider.id();
        let task = cx.background_spawn(async move { Keys::store(id, &key).map_err(|e| format!("{e:#}")) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                let c = &mut v.clouds[i];
                c.saving = false;
                match result {
                    Ok(()) => {
                        c.has_key = true;
                        c.replacing = false;
                        c.probe = None;
                        c.boxes = None;
                        v.test(i, cx);
                    }
                    Err(e) => c.error = Some(format!("Could not save the key: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn remove_key(&mut self, i: usize, cx: &mut Context<Self>) {
        let c = &mut self.clouds[i];
        if !c.confirm_remove {
            c.confirm_remove = true;
            cx.notify();
            return;
        }
        c.confirm_remove = false;
        c.saving = true;
        let id = c.provider.id();
        let task = cx.background_spawn(async move { Keys::store(id, "").map_err(|e| format!("{e:#}")) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                let c = &mut v.clouds[i];
                c.saving = false;
                match result {
                    Ok(()) => {
                        c.has_key = false;
                        c.replacing = false;
                        c.probe = None;
                        c.boxes = None;
                    }
                    Err(e) => c.error = Some(format!("Could not remove the key: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn create_box(&mut self, i: usize, cx: &mut Context<Self>) {
        let c = &mut self.clouds[i];
        if c.creating {
            return;
        }
        c.creating = true;
        c.error = None;
        let p = c.provider;
        let task =
            cx.background_spawn(
                async move { p.create(&Keys::load(), &BoxSpec::default()).map_err(|e| format!("{e:#}")) },
            );
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                let c = &mut v.clouds[i];
                c.creating = false;
                match result {
                    Ok(_) => v.list(i, cx),
                    Err(e) => c.error = Some(format!("Could not create a box: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn box_action(&mut self, i: usize, id: String, action: BoxAction, cx: &mut Context<Self>) {
        let c = &mut self.clouds[i];
        if c.busy.contains_key(&id) {
            return;
        }
        if action == BoxAction::Delete && c.confirm_delete.as_deref() != Some(id.as_str()) {
            c.confirm_delete = Some(id);
            cx.notify();
            return;
        }
        c.confirm_delete = None;
        c.error = None;
        c.busy.insert(id.clone(), action);
        let p = c.provider;
        let box_id = id.clone();
        let task = cx.background_spawn(async move {
            let keys = Keys::load();
            match action {
                BoxAction::Wake => p.wake(&keys, &box_id),
                BoxAction::Sleep => p.sleep(&keys, &box_id),
                BoxAction::Delete => p.delete(&keys, &box_id),
            }
            .map_err(|e| format!("{e:#}"))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                let c = &mut v.clouds[i];
                c.busy.remove(&id);
                if let Err(e) = result {
                    let verb = match action {
                        BoxAction::Wake => "wake",
                        BoxAction::Sleep => "put to sleep",
                        BoxAction::Delete => "delete",
                    };
                    c.error = Some(format!("Could not {verb} {id}: {e}"));
                }
                v.list(i, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    // ------------------------------------------------------------ render

    fn hosts_section(&self, t: &Theme) -> AnyElement {
        let Some(rows) = &self.hosts else {
            let card = page::card(t).child(
                page::row(t, true)
                    .child(page::row_tile(t, icons::LAPTOP, None))
                    .child(div().flex_1().flex().flex_col().gap(px(6.0)).child(page::skeleton_lines(t, 2))),
            );
            return page::section(t, "This computer & connected machines", card).into_any_element();
        };
        let mut card = page::card(t);
        for (i, row) in rows.iter().enumerate() {
            let (glyph, title, kind) = describe(&row.host);
            let (status_label, color, reason) = match &row.probe {
                None => ("Checking…", t.busy, None),
                Some(Probe::Ready) => ("Ready", t.success, None),
                Some(Probe::NeedsSetup(why)) => ("Needs setup", t.warning, Some(why.clone())),
                Some(Probe::Unavailable(why)) => ("Unavailable", t.text_faint, Some(why.clone())),
            };
            let description = match reason.filter(|r| !r.trim().is_empty()) {
                Some(r) => format!("{kind} · {r}"),
                None => kind.to_string(),
            };
            card = card.child(
                page::row(t, i == 0)
                    .child(page::row_tile(t, glyph, None))
                    .child(page::row_text(t, title, description))
                    .child(status(status_label, color, t)),
            );
        }
        if rows.len() <= 1 {
            card = card.child(page::row(t, false).child(page::info_strip(
                t,
                "No other machines found. Add hosts to ~/.ssh/config, start a Docker container, or use a cloud \
                 provider below; then press Refresh.",
            )));
        }
        page::section(t, "This computer & connected machines", card).into_any_element()
    }

    fn clouds_section(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let mut list = div().flex().flex_col().gap(px(space::SM)).child(page::info_strip(
            &t,
            format!(
                "Keys are kept in {}, readable only by you, and never reach an agent's environment.",
                Keys::path().display()
            ),
        ));
        for i in 0..self.clouds.len() {
            list = list.child(self.cloud_card(i, &t, window, cx));
        }
        page::section(&t, "Cloud providers", list).into_any_element()
    }

    fn cloud_card(&self, i: usize, t: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let c = &self.clouds[i];
        let p = c.provider;
        let pid = p.id();
        let id = |what: &str| SharedString::from(format!("mach-{pid}-{what}"));

        // Head: name, Unverified, status line; Test and the key page link.
        let (line, color): (String, Hsla) = if c.saving {
            ("Saving…".into(), t.busy)
        } else if c.testing {
            ("Checking…".into(), t.busy)
        } else {
            match &c.probe {
                Some(Probe::Ready) => match &c.boxes {
                    Some(b) if b.len() == 1 => ("Ready · 1 box".into(), t.success),
                    Some(b) => (format!("Ready · {} boxes", b.len()), t.success),
                    None => ("Ready".into(), t.success),
                },
                Some(Probe::NeedsSetup(why)) => (non_empty(why, "Needs setup"), t.warning),
                Some(Probe::Unavailable(why)) => (non_empty(why, "Unavailable"), t.text_faint),
                None if p.needs_key() && !c.has_key => ("Paste an API key to connect".into(), t.text_faint),
                None => ("Not checked yet".into(), t.text_faint),
            }
        };
        let title = div()
            .flex()
            .items_center()
            .gap(px(space::SM))
            .child(
                div()
                    .text_size(ty::BODY)
                    .line_height(ty::lh::BODY)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(t.text)
                    .child(p.label()),
            )
            .when(!p.verified(), |d| {
                d.child(
                    div()
                        .id(id("unverified"))
                        .child(page::badge_tone("Unverified", t.warning))
                        .tooltip(ui::tip("Not yet tested end to end with a real account")),
                )
            });
        let text = div().flex_1().min_w(px(180.0)).flex().flex_col().gap(px(1.0)).child(title).child(
            div()
                .flex()
                .items_start()
                .gap(px(6.0))
                .text_size(ty::META)
                .line_height(ty::lh::META)
                .text_color(t.text_muted)
                .child(div().mt(px(6.0)).flex_none().child(ui::dot(color, 6.0)))
                .child(div().flex_1().min_w_0().child(line)),
        );
        let test = ctl::button_sm(t, id("test"), if c.testing { "Testing…" } else { "Test" }, Tone::Filled);
        let test = if !c.can_probe() {
            ctl::disabled(test, "Save a key first")
        } else if c.testing || c.saving {
            ctl::disabled(test, "Already checking")
        } else {
            test.tooltip(ui::tip("Check the key and the connection now"))
                .on_click(cx.listener(move |this, _, _, cx| this.test(i, cx)))
        };
        let url = p.key_url();
        let link = (!url.is_empty()).then(|| {
            ctl::button_sm(t, id("link"), if p.needs_key() { "Get a key" } else { "Docs" }, Tone::Quiet)
                .child(icon(icons::ARROW_UP_RIGHT).size(px(12.0)))
                .tooltip(ui::tip(url))
                .on_click(cx.listener(move |_, _, _, cx| cx.open_url(url)))
        });
        let mut card = page::card(t).child(
            page::row(t, true)
                .child(page::row_tile(t, icons::CLOUD, None))
                .child(text)
                .child(div().flex_none().flex().items_center().gap(px(4.0)).children(link).child(test)),
        );

        // The key.
        if let Some(input) = &c.key {
            let control = if c.has_key && !c.replacing {
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .mr(px(4.0))
                            .text_size(ty::META)
                            .text_color(t.text_muted)
                            .child(icon(icons::KEY_MINIMALISTIC).size(px(13.0)).text_color(t.success))
                            .child("Key saved"),
                    )
                    .child(ctl::button_sm(t, id("replace"), "Replace", Tone::Filled).on_click(cx.listener(
                        move |this, _, window, cx| {
                            let c = &mut this.clouds[i];
                            c.replacing = true;
                            c.confirm_remove = false;
                            if let Some(input) = c.key.clone() {
                                input.update(cx, |s, cx| s.focus(window, cx));
                            }
                            cx.notify();
                        },
                    )))
                    .child(
                        ctl::button_sm(
                            t,
                            id("remove"),
                            if c.confirm_remove { "Remove for good" } else { "Remove" },
                            if c.confirm_remove { Tone::Danger } else { Tone::Quiet },
                        )
                        .tooltip(ui::tip(if c.confirm_remove {
                            "Click again to delete the saved key"
                        } else {
                            "Delete the saved key from this computer"
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| this.remove_key(i, cx))),
                    )
                    .when(c.confirm_remove, |d| {
                        d.child(ctl::button_sm(t, id("remove-cancel"), "Cancel", Tone::Quiet).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.clouds[i].confirm_remove = false;
                                cx.notify();
                            },
                        )))
                    })
            } else {
                let focused = input.read(cx).focus_handle(cx).is_focused(window);
                let empty = input.read(cx).value().trim().is_empty();
                let save = ctl::button_sm(t, id("save"), if c.saving { "Saving…" } else { "Save" }, Tone::Solid);
                let save = if empty {
                    ctl::disabled(save, "Paste a key first")
                } else if c.saving {
                    ctl::disabled(save, "Saving…")
                } else {
                    save.on_click(cx.listener(move |this, _, window, cx| this.save_key(i, window, cx)))
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(ctl::text_field(input, Some(220.0), focused, t).h(px(28.0)).text_size(ty::SMALL))
                    .child(save)
                    .when(c.replacing, |d| {
                        d.child(ctl::button_sm(t, id("replace-cancel"), "Cancel", Tone::Quiet).on_click(cx.listener(
                            move |this, _, window, cx| {
                                let c = &mut this.clouds[i];
                                c.replacing = false;
                                if let Some(input) = c.key.clone() {
                                    input.update(cx, |s, cx| s.set_value("", window, cx));
                                }
                                cx.notify();
                            },
                        )))
                    })
            };
            let hint = if c.has_key && !c.replacing {
                "Stored on this computer only"
            } else if c.replacing {
                "The new key replaces the saved one"
            } else {
                "Saved on this computer only, never shown again"
            };
            card = card.child(page::setting_row(t, false, "API key", hint, control));
        }

        // Boxes, once the provider answers.
        if matches!(c.probe, Some(Probe::Ready)) {
            match &c.boxes {
                None if c.listing => {
                    card = card
                        .child(page::row(t, false).child(
                            div().flex_1().text_size(ty::META).text_color(t.text_faint).child("Loading boxes…"),
                        ));
                }
                None => {}
                Some(boxes) => {
                    for b in boxes {
                        card = card.child(self.box_row(i, b, t, cx));
                    }
                    let new =
                        ctl::button_sm(t, id("new"), if c.creating { "Creating…" } else { "New box" }, Tone::Filled);
                    let new = if c.creating {
                        ctl::disabled(new, "Creating a box; this can take a minute")
                    } else {
                        new.child(icon(icons::PLUS).size(px(12.0)))
                            .on_click(cx.listener(move |this, _, _, cx| this.create_box(i, cx)))
                    };
                    card = card.child(page::setting_row(
                        t,
                        false,
                        if boxes.is_empty() { "No boxes yet" } else { "Another box" },
                        "Smallest size with the provider's default auto-sleep",
                        new,
                    ));
                }
            }
        }
        if let Some(e) = &c.error {
            card = card.child(page::row(t, false).child(page::error_strip(t, e.clone())));
        }
        card.into_any_element()
    }

    fn box_row(&self, i: usize, b: &BoxInfo, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let c = &self.clouds[i];
        let pid = c.provider.id();
        let bid = b.id.clone();
        let id = |what: &str| SharedString::from(format!("mach-{pid}-{what}-{bid}"));
        let busy = c.busy.get(&b.id).copied();
        let confirming = c.confirm_delete.as_deref() == Some(b.id.as_str());
        let title = if b.name.is_empty() { b.id.clone() } else { b.name.clone() };
        let description =
            if b.name.is_empty() || b.name == b.id { b.state.clone() } else { format!("{} · {}", b.state, b.id) };
        let mut actions = div().flex_none().flex().items_center().gap(px(4.0));
        if let Some(action) = busy {
            actions = actions.child(page::badge_tone(action.busy_label(), t.busy));
        } else if confirming {
            let box_id = b.id.clone();
            actions =
                actions
                    .child(ctl::button_sm(t, id("cancel"), "Cancel", Tone::Quiet).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.clouds[i].confirm_delete = None;
                            cx.notify();
                        },
                    )))
                    .child(
                        ctl::button_sm(t, id("delete-confirm"), "Delete for good", Tone::Danger)
                            .tooltip(ui::tip("The box and everything on it are removed. This cannot be undone."))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.box_action(i, box_id.clone(), BoxAction::Delete, cx)
                            })),
                    );
        } else {
            let (box_id, del_id) = (b.id.clone(), b.id.clone());
            actions = actions
                .child(if b.running {
                    ctl::button_sm(t, id("sleep"), "Sleep", Tone::Filled)
                        .tooltip(ui::tip("Suspend or stop the box; Keplorayx wakes it when a thread needs it"))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.box_action(i, box_id.clone(), BoxAction::Sleep, cx)),
                        )
                } else {
                    ctl::button_sm(t, id("wake"), "Wake", Tone::Filled).on_click(
                        cx.listener(move |this, _, _, cx| this.box_action(i, box_id.clone(), BoxAction::Wake, cx)),
                    )
                })
                .child(ctl::button_sm(t, id("delete"), "Delete", Tone::Quiet).on_click(
                    cx.listener(move |this, _, _, cx| this.box_action(i, del_id.clone(), BoxAction::Delete, cx)),
                ));
        }
        page::row(t, false)
            .pl(px(28.0))
            .child(ui::dot(if b.running { t.success } else { t.text_faint }, 6.0).flex_none())
            .child(page::row_text(t, title, description))
            .child(actions)
            .into_any_element()
    }

    /// Paired devices: other computers on the user's account running
    /// Keplorayx as a machine (or `dove node serve`), listed with whether
    /// they are online. Pairing itself is enabled once the build has it.
    fn paired_devices_section(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let pair = ctl::button_sm(t, "mach-pair", "Pair a device", Tone::Filled);
        let pair = if dove_core::paired::pairing_available() {
            pair.on_click(cx.listener(|this, _, window, cx| this.start_pairing(window, cx)))
        } else {
            ctl::disabled(pair, "Pairing is being set up")
        };
        let mut card = page::card(t).child(
            page::row(t, true)
                .child(page::row_tile(t, icons::LINK_CHAIN, None))
                .child(page::row_text(
                    t,
                    "Run agents on your other computers",
                    "Pair a Mac, a desktop or a home server signed in to the same Woozlit account. It connects \
                     out, so no ports are opened, and its threads keep running when this computer sleeps.",
                ))
                .child(pair),
        );
        match &self.paired {
            None if self.loaded => {
                card = card.child(
                    page::row(t, false)
                        .child(div().flex_1().text_size(ty::META).text_color(t.text_faint).child("Loading devices…")),
                );
            }
            None => {}
            Some(devices) => {
                for d in devices {
                    let (label, color) = if d.online { ("Online", t.success) } else { ("Offline", t.text_faint) };
                    let name = if d.name.is_empty() { d.id.clone() } else { d.name.clone() };
                    card = card.child(
                        page::row(t, false)
                            .child(page::row_tile(t, icons::MONITOR, None))
                            .child(page::row_text(t, name, "Paired device"))
                            .child(status(label, color, t)),
                    );
                }
            }
        }
        page::section(t, "Paired devices", card).into_any_element()
    }

    /// Start pairing a device. Reached only when
    /// `dove_core::paired::pairing_available()` is true; the pairing flow
    /// (code and QR) plugs in here.
    fn start_pairing(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
    }
}

/// A machine's row glyph, title and kind.
fn describe(host: &Host) -> (&'static str, String, &'static str) {
    match host {
        Host::Local => (icons::LAPTOP, format!("Local · {}", ui::host_name()), "This computer"),
        Host::Wsl { distro } => {
            (icons::TERMINAL, distro.clone().unwrap_or_else(|| "Default distro".into()), "Windows Subsystem for Linux")
        }
        Host::Ssh { alias } => (icons::SERVER, alias.clone(), "SSH host from ~/.ssh/config"),
        Host::CuaSpace { name } => (icons::BOX_SELECT, name.clone(), "Cua Space"),
        Host::Docker { container } => (icons::LAYERS, container.clone(), "Running Docker container"),
        Host::Sandbox { .. } => (icons::CLOUD, host.label(), "Cloud box"),
        Host::Node { .. } => (icons::MONITOR, host.label(), "Paired device"),
    }
}

fn non_empty(text: &str, fallback: &str) -> String {
    let text = text.trim();
    if text.is_empty() { fallback.to_string() } else { text.to_string() }
}

/// A status dot and a short label.
fn status(label: &'static str, color: Hsla, t: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .text_size(ty::META)
        .text_color(t.text_muted)
        .child(ui::dot(color, 6.0))
        .child(label)
}

impl Render for MachinesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("MachinesView");
        let t = Theme::of(cx).clone();
        div()
            .flex()
            .flex_col()
            .child(self.hosts_section(&t))
            .child(self.clouds_section(window, cx))
            .child(self.paired_devices_section(&t, cx))
    }
}
