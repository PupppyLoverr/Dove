//! Agents' sign-in and update sheet (the agent's own login in a terminal,
//! over any page) and rescanning PATH. Settings › Agents lists the agents.

use dove_core::proto::HarnessKind;
use gpui::{
    AnyElement, AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

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
    pub(crate) fn sign_in(&mut self, kind: HarnessKind, cx: &mut Context<Self>) {
        let Some(program) = self.engine.program_for(kind) else { return };
        let home = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ".".into());
        let term = cx.new(|cx| TerminalView::for_command(home, program, login_args(kind), cx));
        self.pages.agent_terminal = Some((kind, term));
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
    pub(crate) fn render_agent_sheet(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
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
