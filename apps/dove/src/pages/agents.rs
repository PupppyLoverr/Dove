//! Agents: every CLI Dove knows, where it lives (with an override), its
//! version, signing in (the agent's own login in a terminal), its last
//! model, and whether the composer offers it.

use dove_core::proto::HarnessKind;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_base::input::Input;

use super::{input, page};
use crate::assets::icon;
use crate::kit;
use crate::shell::Shell;
use crate::terminal::TerminalView;
use crate::theme::Theme;
use crate::ui;

/// The arguments that start an agent's sign-in flow.
fn login_args(kind: HarnessKind) -> Vec<String> {
    match kind {
        HarnessKind::Codex | HarnessKind::Cursor => vec!["login".into()],
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

    pub fn render_agents_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let installed: Vec<HarnessKind> = self.installed.iter().map(|i| i.kind).collect();
        let memory = crate::picker_memory::PickerMemory::load(self.engine.store.clone());
        let rows =
            div().flex().flex_col().rounded(px(12.0)).border_1().border_color(t.border).overflow_hidden().children(
                HarnessKind::ALL.iter().copied().filter(|k| *k != HarnessKind::Shell).enumerate().map(|(i, kind)| {
                    let found = installed.contains(&kind);
                    let supported = dove_core::harness_supported(kind);
                    let enabled = agent_enabled(&self.engine.store, kind);
                    let path = self.engine.program_for(kind);
                    let version = self.versions.get(&kind).cloned();
                    let last_model = memory.last(kind).map(|p| p.model.clone());
                    let editing =
                        self.pages.agent_path_edit.as_ref().filter(|(k, _)| *k == kind).map(|(_, f)| f.clone());
                    div()
                        .px(px(14.0))
                        .py(px(12.0))
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .when(i > 0, |d| d.border_t_1().border_color(t.border))
                        .when(!found, |d| d.opacity(0.65))
                        .child(icon(ui::harness_icon(kind)).size(px(20.0)).text_color(ui::harness_tint(
                            kind,
                            &t,
                            t.text_muted,
                        )))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(3.0))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(8.0))
                                        .child(
                                            div()
                                                .text_size(px(14.0))
                                                .font_weight(gpui::FontWeight::MEDIUM)
                                                .child(kind.label()),
                                        )
                                        .when_some(version, |d, v| d.child(kit::badge(v, t.text_faint)))
                                        .when(!supported, |d| d.child(kit::badge("terminal only", t.text_faint)))
                                        .when(!found, |d| d.child(kit::badge("not installed", t.warning))),
                                )
                                .child(match editing {
                                    Some(f) => div()
                                        .h(px(28.0))
                                        .px(px(8.0))
                                        .flex()
                                        .items_center()
                                        .rounded(px(6.0))
                                        .border_1()
                                        .border_color(t.border_strong)
                                        .text_size(px(12.5))
                                        .child(div().flex_1().child(Input::new(&f)))
                                        .into_any_element(),
                                    None => div()
                                        .id(SharedString::from(format!("ag-path-{}", kind.id())))
                                        .text_size(px(12.0))
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
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| this.edit_path(kind, window, cx)),
                                        )
                                        .into_any_element(),
                                })
                                .when_some(last_model, |d, m| {
                                    d.child(
                                        div()
                                            .text_size(px(11.5))
                                            .text_color(t.text_faint)
                                            .child(format!("Last model: {m}")),
                                    )
                                }),
                        )
                        .when(found, |d| {
                            d.child(
                                ui::button_ghost(SharedString::from(format!("ag-login-{}", kind.id())), "Sign in", &t)
                                    .on_click(cx.listener(move |this, _, _, cx| this.sign_in(kind, cx))),
                            )
                        })
                        .child(
                            kit::switch(SharedString::from(format!("ag-on-{}", kind.id())), enabled, &t)
                                .tooltip(ui::tip(if enabled {
                                    "Offered in the composer"
                                } else {
                                    "Hidden from the composer"
                                }))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let _ = this.engine.store.set_setting(
                                        &format!("agent_disabled.{}", kind.id()),
                                        if enabled { "true" } else { "false" },
                                    );
                                    let installed: Vec<_> = this
                                        .installed
                                        .iter()
                                        .filter(|i| agent_enabled(&this.engine.store, i.kind))
                                        .cloned()
                                        .collect();
                                    this.composer.update(cx, |c, cx| c.set_installed(installed, cx));
                                    cx.notify();
                                })),
                        )
                }),
            );
        let sheet = self.pages.agent_terminal.clone().map(|(kind, term)| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(13.0))
                                .child(format!("Signing in to {} · follow the prompts below", kind.label())),
                        )
                        .child(ui::button_ghost("ag-term-close", "Done", &t).on_click(cx.listener(
                            |this, _, _, cx| {
                                if let Some((_, term)) = this.pages.agent_terminal.take() {
                                    term.update(cx, |t, cx| t.kill(cx));
                                }
                                cx.notify();
                            },
                        ))),
                )
                .child(
                    div()
                        .h(px(340.0))
                        .rounded(px(10.0))
                        .overflow_hidden()
                        .border_1()
                        .border_color(t.border)
                        .child(term),
                )
        });
        let rescan = ui::button_ghost("ag-rescan", "Rescan PATH", &t)
            .on_click(cx.listener(|this, _, _, cx| this.rescan(cx)))
            .into_any_element();
        page(
            "agents-page",
            "Agents",
            &t,
            Some(rescan),
            div().flex().flex_col().gap(px(14.0)).children(sheet).child(rows),
        )
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
