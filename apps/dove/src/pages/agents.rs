//! Agents: every CLI Dove knows, where it lives (with an override), its
//! version, signing in (the agent's own login in a terminal), its last
//! model, and whether the composer offers it.

use dove_core::proto::HarnessKind;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use super::input;
use crate::design::{space, ty};
use crate::kit::controls::{self as ctl, Tone};
use crate::kit::{page, popover};
use crate::shell::Shell;
use crate::terminal::TerminalView;
use crate::theme::Theme;
use crate::ui;

/// The arguments that start an agent's sign-in flow.
fn login_args(kind: HarnessKind) -> Vec<String> {
    match kind {
        HarnessKind::Codex | HarnessKind::Cursor | HarnessKind::Kimi | HarnessKind::Amp => vec!["login".into()],
        HarnessKind::OpenCode | HarnessKind::Kilo => vec!["auth".into(), "login".into()],
        HarnessKind::Devin => vec!["auth".into(), "login".into()],
        // Claude, Gemini, Copilot, Qwen and others sign in from their own UI.
        _ => Vec::new(),
    }
}

pub fn agent_enabled(store: &dove_core::store::Store, kind: HarnessKind) -> bool {
    store.setting(&format!("agent_disabled.{}", kind.id())).as_deref() != Some("true")
}

impl Shell {
    fn sign_in(&mut self, kind: HarnessKind, cx: &mut Context<Self>) {
        let Some(program) = self.engine.program_for(kind) else { return };
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ".".into());
        let term = cx.new(|cx| TerminalView::for_command(home, program, login_args(kind), cx));
        self.pages.agent_terminal = Some((kind, term));
        cx.notify();
    }

    fn edit_path(&mut self, kind: HarnessKind, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.engine.store.setting(&format!("agent_path.{}", kind.id())).unwrap_or_default();
        let field = input("Full path to the binary (empty = find on PATH)", window, cx);
        field.update(cx, |s, cx| {
            s.set_value(current, window, cx);
            s.focus(window, cx);
        });
        let sub = cx.subscribe(&field, move |this, f, ev: &gpui_base::input::InputEvent, cx| {
            if let gpui_base::input::InputEvent::PressEnter { .. } = ev {
                let v = f.read(cx).value().trim().to_string();
                let _ = this.engine.store.set_setting(&format!("agent_path.{}", kind.id()), &v);
                this.pages.agent_path_edit = None;
                cx.notify();
            }
        });
        sub.detach();
        self.pages.agent_path_edit = Some((kind, field));
        cx.notify();
    }

    fn close_agent_terminal(&mut self, cx: &mut Context<Self>) {
        if let Some((_, term)) = self.pages.agent_terminal.take() {
            term.update(cx, |t, cx| t.kill(cx));
        }
        cx.notify();
    }

    /// The sign-in (or update) sheet: the agent's own login running in a
    /// terminal, over a scrim, until Done.
    fn render_agent_sheet(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (kind, term) = self.pages.agent_terminal.clone()?;
        let card = popover::dialog_card(t, 720.0)
            .id("agent-sheet")
            .role(gpui::Role::Dialog)
            .aria_label(SharedString::from(format!("Signing in to {}", kind.label())))
            .child(
                div()
                    .px(px(20.0))
                    .pt(px(16.0))
                    .pb(px(space::MD))
                    .flex()
                    .items_center()
                    .gap(px(space::MD))
                    .child(page::row_tile(t, ui::harness_icon(kind), Some(ui::harness_tint(kind, t, t.text_muted))))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(ty::TITLE)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(format!("Signing in to {} · follow the prompts below", kind.label())),
                    )
                    .child(
                        ctl::button(t, "ag-term-close", "Done", Tone::Solid)
                            .on_click(cx.listener(|this, _, _, cx| this.close_agent_terminal(cx))),
                    ),
            )
            .child(
                div()
                    .mx(px(space::LG))
                    .mb(px(space::LG))
                    .h(px(340.0))
                    .rounded(px(crate::design::radius::PANEL))
                    .overflow_hidden()
                    .border_1()
                    .border_color(t.border)
                    .child(term),
            );
        Some(popover::modal("agent-sheet-layer", window.viewport_size(), t.scrim, None, card.into_any_element(), None))
    }

    pub fn render_agents_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let installed: Vec<HarnessKind> = self.installed.iter().map(|i| i.kind).collect();
        let memory = crate::picker_memory::PickerMemory::load(self.engine.store.clone());
        let kinds: Vec<HarnessKind> = HarnessKind::ALL.iter().copied().filter(|k| *k != HarnessKind::Shell).collect();
        let found_n = kinds.iter().filter(|k| installed.contains(k)).count();
        let rows = page::card(&t).children(kinds.into_iter().enumerate().map(|(i, kind)| {
            let found = installed.contains(&kind);
            let supported = dove_core::harness_supported(kind);
            let enabled = agent_enabled(&self.engine.store, kind);
            let path = self.engine.program_for(kind);
            let version = self.versions.get(&kind).cloned();
            let last_model = memory.last(kind).map(|p| p.model.clone());
            let editing = self.pages.agent_path_edit.as_ref().filter(|(k, _)| *k == kind).map(|(_, f)| f.clone());
            let toggle = ctl::on_toggle(
                ctl::toggle(&t, SharedString::from(format!("ag-on-{}", kind.id())), enabled)
                    .tooltip(ui::tip(if enabled { "Offered in the composer" } else { "Hidden from the composer" })),
                cx,
                move |this, _, cx| {
                    let _ = this
                        .engine
                        .store
                        .set_setting(&format!("agent_disabled.{}", kind.id()), if enabled { "true" } else { "false" });
                    let installed: Vec<_> =
                        this.installed.iter().filter(|i| agent_enabled(&this.engine.store, i.kind)).cloned().collect();
                    this.composer.update(cx, |c, cx| c.set_installed(installed, cx));
                    cx.notify();
                },
            );
            page::row(&t, i == 0)
                .child(page::row_tile(&t, ui::harness_icon(kind), Some(ui::harness_tint(kind, &t, t.text_muted))))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(220.0))
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .when(!found, |d| d.opacity(0.7))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .text_size(ty::BODY)
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(t.text)
                                        .child(kind.label()),
                                )
                                .when_some(version, |d, v| d.child(page::badge(&t, v)))
                                .when(!supported, |d| d.child(page::badge(&t, "terminal only")))
                                .when(!found, |d| d.child(page::badge_tone("not installed", t.warning))),
                        )
                        .child(match editing {
                            Some(f) => {
                                ctl::text_field(&f, None, true, &t).h(px(28.0)).text_size(ty::SMALL).into_any_element()
                            }
                            None => div()
                                .id(SharedString::from(format!("ag-path-{}", kind.id())))
                                .text_size(ty::META)
                                .font_family(t.font_mono.clone())
                                .text_color(t.text_faint)
                                .truncate()
                                .cursor_pointer()
                                .hover(|s| s.text_color(t.text_muted))
                                .tooltip(ui::tip("Click to set a custom path"))
                                .child(
                                    path.map(|p| p.display().to_string())
                                        .unwrap_or_else(|| "Not on PATH · click to set a path".into()),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| this.edit_path(kind, window, cx)))
                                .into_any_element(),
                        })
                        .when_some(last_model, |d, m| {
                            d.child(
                                div().text_size(ty::CAPTION).text_color(t.text_faint).child(format!("Last model: {m}")),
                            )
                        }),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(space::SM))
                        .when(found, |d| {
                            d.child(
                                ctl::button_sm(
                                    &t,
                                    SharedString::from(format!("ag-login-{}", kind.id())),
                                    "Sign in",
                                    Tone::Filled,
                                )
                                .on_click(cx.listener(move |this, _, _, cx| this.sign_in(kind, cx))),
                            )
                        })
                        .child(toggle),
                )
        }));
        let rescan = ctl::button(&t, "ag-rescan", "Rescan PATH", Tone::Outlined)
            .on_click(cx.listener(|this, _, _, cx| this.rescan(cx)))
            .into_any_element();
        let sheet = self.render_agent_sheet(&t, window, cx);
        div()
            .size_full()
            .child(super::page_at(
                "agents-page",
                crate::design::size::PAGE_MAX,
                "Agents",
                "Every agent CLI Dove knows: where it lives, how to sign in, and whether the composer offers it.",
                &t,
                Some(rescan),
                page::section(&t, format!("{found_n} installed"), rows).mt(px(0.0)),
            ))
            .children(sheet)
            .into_any_element()
    }

    pub fn rescan(&mut self, cx: &mut Context<Self>) {
        let engine = self.engine.clone();
        let scan = cx.background_spawn(async move { engine.rescan_harnesses() });
        cx.spawn(async move |this, cx| {
            let found = scan.await;
            let _ = this.update(cx, |s, cx| {
                s.installed = found.clone();
                let enabled: Vec<_> = found.into_iter().filter(|i| agent_enabled(&s.engine.store, i.kind)).collect();
                s.composer.update(cx, |c, cx| c.set_installed(enabled, cx));
                cx.notify();
            });
        })
        .detach();
    }
}
