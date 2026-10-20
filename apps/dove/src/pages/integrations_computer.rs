//! Settings → Integrations → Dove Computer Use: the plugin's plate.
//!
//! Install, Update and Uninstall of the pinned dove-driver release, the
//! driver's own `health_report`, the per-OS permissions walkthrough (shown
//! on first install and on demand), the approval mode, and the optional
//! local grounder. Everything that touches the network or the disk runs in
//! the background and only after a click.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};

use super::{IntegrationsView, head, line, plate, small_icon, status, toggle};
use crate::assets::icons;
use crate::computer_plugin::{self as plugin, Health, PermissionMode, StepAction};
use crate::design::ty;
use crate::kit::controls::{self as ctl, Tone};
use crate::kit::page;
use crate::theme::Theme;

/// A background job on the plugin or the grounder.
#[derive(Clone, Default)]
pub(super) enum Job {
    #[default]
    Idle,
    Working(&'static str),
    Failed(String),
}

/// The plate's live state, kept across [`IntegrationsView::refresh`].
#[derive(Default)]
pub(super) struct State {
    job: Job,
    grounder_job: Job,
    /// The last `health_report`, for the driver found at that time.
    health: Option<Health>,
    checking: bool,
    /// The permissions walkthrough is open.
    walkthrough: bool,
    /// The installed grounder weights.
    grounder: Option<std::path::PathBuf>,
    /// The QA hook ran (debug builds).
    qa_done: bool,
}

impl IntegrationsView {
    /// After each probe: read the grounder and, once per driver, its health.
    pub(super) fn computer_probed(&mut self, cx: &mut Context<Self>) {
        self.cu.grounder = plugin::grounder_installed();
        // Debug builds, QA against a fake release: `DOVE_COMPUTER_USE_QA` set
        // to `install`, `grounder` or `uninstall` presses that button once.
        if cfg!(debug_assertions) && !std::mem::replace(&mut self.cu.qa_done, true) {
            match std::env::var("DOVE_COMPUTER_USE_QA").as_deref() {
                Ok("install") if self.probe.cu_plugin.is_none() => self.run_cu(true, cx),
                Ok("grounder") if self.cu.grounder.is_none() => self.set_grounder(true, cx),
                Ok("uninstall") if self.probe.cu_plugin.is_some() => self.run_cu(false, cx),
                _ => {}
            }
        }
        if self.probe.cua.is_some() && self.cu.health.is_none() && !self.cu.checking {
            self.check_cua(cx);
        }
    }

    /// Install or update the plugin (`install`), or remove it. Only ever
    /// started from the plate's buttons.
    fn run_cu(&mut self, install: bool, cx: &mut Context<Self>) {
        if matches!(self.cu.job, Job::Working(_)) {
            return;
        }
        self.cu.job = Job::Working(if install { "Downloading and verifying dove-driver…" } else { "Removing…" });
        let task = cx
            .background_spawn(async move { if install { plugin::install().map(|_| ()) } else { plugin::uninstall() } });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                match result {
                    Ok(()) => {
                        v.cu.job = Job::Idle;
                        v.cu.health = None;
                        // Installing turns it on; removing takes the tools away.
                        v.config.computer.enabled = install;
                        if install {
                            v.cu.walkthrough = !v.config.computer.walkthrough_done;
                        } else {
                            v.config.computer.grounder = false;
                            v.cu.walkthrough = false;
                        }
                        v.persist(cx);
                    }
                    Err(e) => v.cu.job = Job::Failed(e),
                }
                v.refresh(cx);
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn check_cua(&mut self, cx: &mut Context<Self>) {
        let Some(bin) = self.probe.cua.clone() else { return };
        self.cu.checking = true;
        let settings = self.config.computer.clone();
        let task = cx.background_spawn(async move { crate::mcp_browser::cua_health(&bin, &settings) });
        cx.spawn(async move |this, cx| {
            let health = task.await;
            let _ = this.update(cx, |v, cx| {
                v.cu.checking = false;
                v.cu.health = Some(health);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Turn the local grounder on (downloading its weights) or off
    /// (removing them).
    fn set_grounder(&mut self, on: bool, cx: &mut Context<Self>) {
        if matches!(self.cu.grounder_job, Job::Working(_)) {
            return;
        }
        if !on {
            self.config.computer.grounder = false;
            self.cu.grounder_job = match plugin::remove_grounder() {
                Ok(()) => Job::Idle,
                Err(e) => Job::Failed(e),
            };
            self.cu.grounder = plugin::grounder_installed();
            self.persist(cx);
            return;
        }
        self.cu.grounder_job = Job::Working("Downloading the grounder…");
        let task = cx.background_spawn(async move { plugin::install_grounder() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| match result {
                Ok(path) => {
                    v.cu.grounder_job = Job::Idle;
                    v.cu.grounder = Some(path);
                    v.config.computer.grounder = true;
                    v.persist(cx);
                }
                Err(e) => {
                    v.cu.grounder_job = Job::Failed(e);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_computer(&mut self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let m = plugin::manifest();
        let on = self.config.computer.enabled;
        let found = self.probe.cua.is_some();
        let installed = self.probe.cu_plugin.clone();
        let busy = matches!(self.cu.job, Job::Working(_));
        let available = plugin::available();
        let asset = m.asset();
        let mb = |bytes: Option<u64>| bytes.map(|b| format!(", {:.0} MB", b as f64 / 1_048_576.0)).unwrap_or_default();

        let (left, switch) = toggle(
            on,
            Some(match (&installed, found) {
                (Some(p), _) => (true, if p.outdated() { "Update available" } else { "Installed" }),
                (None, true) => (true, "Developer build"),
                (None, false) => (false, "Not installed"),
            }),
            "int-cua",
            t,
        );
        let hint: SharedString = match (&self.cu.job, &installed, &self.probe.cua_version) {
            (Job::Working(m), _, _) => (*m).into(),
            (Job::Failed(e), _, _) => format!("Could not finish: {e}").into(),
            (_, Some(p), _) => format!("Agents see and use your desktop apps through dove-driver {}", p.version).into(),
            (_, None, Some(v)) => format!("Agents see and use your desktop apps through {v}").into(),
            _ => "Agents see and use your desktop apps in the background, from each window's accessibility tree".into(),
        };
        let install_hint = if available {
            format!(
                "Downloads dove-driver {}{} from its GitHub releases and checks its SHA-256 against the manifest built into Dove",
                m.version,
                mb(asset.and_then(|a| a.size))
            )
        } else {
            format!("dove-driver {} is not published for this platform yet", m.version)
        };

        let mut card = plate(t)
            .child(head(
                icons::MONITOR,
                "Dove Computer Use",
                hint,
                left.when(found, |d| {
                    d.child(ctl::on_toggle(switch, cx, move |this, _, cx| {
                        this.config.computer.enabled = !on;
                        this.persist(cx);
                    }))
                }),
                t,
            ))
            .child(match &installed {
                None => line(
                    "Install",
                    install_hint,
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .when(available, |d| {
                            d.child(
                                ctl::button_sm(
                                    t,
                                    "int-cu-install",
                                    if busy { "Installing…" } else { "Install" },
                                    Tone::Filled,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.run_cu(true, cx))),
                            )
                        })
                        .child(
                            small_icon("int-refresh", icons::REFRESH, "Detect again", t)
                                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                        ),
                    t,
                ),
                Some(p) => line(
                    "Plugin",
                    if p.outdated() {
                        format!(
                            "dove-driver {} installed; {} is available{}",
                            p.version,
                            m.version,
                            mb(asset.and_then(|a| a.size))
                        )
                    } else {
                        format!("dove-driver {} in {}", p.version, p.dir().display())
                    },
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .when(p.outdated() && available, |d| {
                            d.child(
                                ctl::button_sm(
                                    t,
                                    "int-cu-update",
                                    if busy { "Updating…" } else { "Update" },
                                    Tone::Filled,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.run_cu(true, cx))),
                            )
                        })
                        .child(
                            ctl::button_sm(t, "int-cu-remove", "Uninstall", Tone::Quiet)
                                .on_click(cx.listener(|this, _, _, cx| this.run_cu(false, cx))),
                        ),
                    t,
                ),
            });

        let Some(path) = self.probe.cua.clone() else { return card };

        // The driver's own health_report.
        let (health_ok, health_label, health_hint): (bool, &str, String) = match (&self.cu.health, self.cu.checking) {
            (_, true) => (false, "Checking", "Asking the driver for its health report…".into()),
            (None, false) => (false, "Unknown", "Not checked yet".into()),
            (Some(h), false) => (
                h.ok(),
                match h.overall.as_str() {
                    _ if h.error.is_some() => "Not answering",
                    "ok" => "Ready",
                    "degraded" => "Needs attention",
                    "failed" => "Not working",
                    _ => "Unknown",
                },
                h.summary(),
            ),
        };
        let health_hint = if installed.is_none() {
            format!("{health_hint} · using {} (developer override)", path.display())
        } else {
            health_hint
        };
        card = card.child(line(
            "Status",
            health_hint,
            div().flex().items_center().gap(px(10.0)).child(status(health_ok, health_label, t)).child(
                ctl::button_sm(
                    t,
                    "int-cua-check",
                    if self.cu.checking { "Checking…" } else { "Check again" },
                    Tone::Quiet,
                )
                .on_click(cx.listener(|this, _, _, cx| this.check_cua(cx))),
            ),
            t,
        ));

        // Permissions walkthrough.
        let open = self.cu.walkthrough;
        card = card.child(line(
            "Permissions",
            if cfg!(target_os = "macos") {
                "dove-driver needs Accessibility, and Screen Recording for screenshots"
            } else if cfg!(windows) {
                "Runs as you: it cannot control apps started as administrator"
            } else {
                "Needs the AT-SPI accessibility bus (on by default in GNOME and KDE)"
            },
            ctl::button_sm(t, "int-cu-walk", if open { "Hide steps" } else { "Set up" }, Tone::Quiet).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.cu.walkthrough = !open;
                    cx.notify();
                }),
            ),
            t,
        ));
        if open {
            for (i, step) in plugin::walkthrough().iter().enumerate() {
                let control = div().flex().items_center().when_some(step.action, |d, (label, action)| {
                    d.child(
                        ctl::button_sm(t, SharedString::from(format!("int-cu-step-{i}")), label, Tone::Filled)
                            .on_click(cx.listener(move |this, _, _, cx| match action {
                                StepAction::Open(url) => cx.open_url(url),
                                StepAction::EnableAtSpi => match plugin::enable_at_spi() {
                                    Ok(()) => this.check_cua(cx),
                                    Err(e) => {
                                        this.cu.health = Some(Health::failed(format!("Could not enable AT-SPI: {e}")));
                                        cx.notify();
                                    }
                                },
                            })),
                    )
                });
                card = card.child(
                    page::setting_row(t, false, format!("{}. {}", i + 1, step.title), step.body, control).pl(px(40.0)),
                );
            }
            card = card.child(
                page::row(t, false).pl(px(40.0)).justify_end().child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .child(
                            ctl::button_sm(t, "int-cu-walk-check", "Check permissions", Tone::Quiet)
                                .on_click(cx.listener(|this, _, _, cx| this.check_cua(cx))),
                        )
                        .child(ctl::button_sm(t, "int-cu-walk-done", "Done", Tone::Filled).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.cu.walkthrough = false;
                                this.config.computer.walkthrough_done = true;
                                this.persist(cx);
                            },
                        ))),
                ),
            );
        }

        // Approval mode: the driver's own permission modes.
        let mode = self.config.computer.permission_mode;
        let this = cx.entity().downgrade();
        card = card.child(line(
            "Approvals",
            match mode {
                PermissionMode::Standard => "dove-driver asks before an agent acts on an app the first time",
                PermissionMode::Bounded => "Agents act within the driver's capability manifest without asking",
            },
            ctl::segmented(
                t,
                "int-cu-mode",
                vec![ctl::Segment::new("Ask first"), ctl::Segment::new("Bounded")],
                match mode {
                    PermissionMode::Standard => 0,
                    PermissionMode::Bounded => 1,
                },
                move |i, _, cx| {
                    let _ = this.update(cx, |v, cx| {
                        v.config.computer.permission_mode =
                            if i == 0 { PermissionMode::Standard } else { PermissionMode::Bounded };
                        v.persist(cx);
                    });
                },
            ),
            t,
        ));

        // Local grounder: resolves `intent` steps on the device.
        let grounder = m.grounder();
        let g_on = self.config.computer.grounder && self.cu.grounder.is_some();
        let g_busy = matches!(self.cu.grounder_job, Job::Working(_));
        let g_hint: String = match (&self.cu.grounder_job, grounder) {
            (Job::Working(m), _) => (*m).into(),
            (Job::Failed(e), _) => format!("Could not finish: {e}"),
            (_, None) => {
                format!("Resolves intent steps on this device. Not in the dove-driver {} release yet", m.version)
            }
            (_, Some(_)) if g_on => "Resolves intent steps on this device, at no token cost".into(),
            (_, Some(g)) => {
                format!("Resolves intent steps on this device. Turning it on downloads its weights{}", mb(g.size))
            }
        };
        let (g_left, g_switch) = toggle(g_on, None, "int-cu-grounder", t);
        card.child(line(
            "Local grounder",
            g_hint,
            g_left.child(if grounder.is_some() && !g_busy {
                ctl::on_toggle(g_switch, cx, move |this, _, cx| this.set_grounder(!g_on, cx))
            } else {
                ctl::disabled(g_switch, if g_busy { "Downloading…" } else { "Not in this release yet" })
            }),
            t,
        ))
        .when(on && self.cu.health.as_ref().is_some_and(|h| !h.ok() && h.error.is_none()) && !open, |d| {
            d.child(
                div()
                    .px(px(14.0))
                    .pb(px(10.0))
                    .text_size(ty::META)
                    .child(page::info_strip(t, "Some checks failed. Open Set up for the steps on this computer.")),
            )
        })
    }
}
