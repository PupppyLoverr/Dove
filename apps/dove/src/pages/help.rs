//! Help: keyboard shortcuts (with the user's overrides) and how Dove works.

use gpui::{AnyElement, Context, ParentElement as _, Styled as _, div, px};

use super::page_with;
use crate::kit;
use crate::shell::Shell;
use crate::theme::Theme;

const TIPS: &[(&str, &str)] = &[
    (
        "Chat ⇄ Terminal",
        "Ctrl+` hands a thread to the agent's own terminal UI on the same session, and back. Mid-turn, it switches when the turn ends.",
    ),
    (
        "Team",
        "Create persistent teammates (Lead, Builder, Reviewer, Designer) in the left sidebar. Each teammate keeps its own harness, model, standing brief, and NOTES.md memory across all its threads.",
    ),
    (
        "Live Canvas & Generative UI",
        "Ask any agent to design an app, website, or mobile screen, or type /design or /mobile. Interactive HTML, React/TSX, and native UI cards render live in chat and in the side panel Canvas.",
    ),
    (
        "Approvals",
        "When an agent asks to run something, press A to allow, S to allow for the session, D to deny (with the composer empty).",
    ),
    (
        "Mentions and commands",
        "Type @ in the composer for files, / for the agent's commands and Dove's prompt templates.",
    ),
    (
        "Steer or queue",
        "While an agent works, Enter steers agents that support it and queues the message for the rest.",
    ),
    ("Review changes", "Click a line number in Changes to leave a comment; send them all as one review."),
    ("Kanban drafts", "Write prompts as drafts and drag them to In progress when you are ready."),
    ("Automations", "Run a prompt on a schedule or when commits land; each run is a thread in the Inbox."),
];

impl Shell {
    pub fn render_help_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let store = self.engine.store.clone();
        let shortcuts =
            div().flex().flex_col().rounded(px(12.0)).border_1().border_color(t.border).overflow_hidden().children(
                crate::keymap::shortcuts().into_iter().enumerate().map(|(i, s)| {
                    let key = crate::keymap::binding(&store, &s);
                    div()
                        .h(px(36.0))
                        .px(px(14.0))
                        .flex()
                        .items_center()
                        .when_border(i > 0, &t)
                        .child(div().flex_1().text_size(px(13.5)).child(s.label))
                        .child(kit::kbd(&crate::keymap::pretty(&key), &t))
                }),
            );
        let tips = div().flex().flex_col().gap(px(10.0)).children(TIPS.iter().map(|(title, body)| {
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(div().text_size(px(13.5)).font_weight(gpui::FontWeight::MEDIUM).child(*title))
                .child(div().text_size(px(13.0)).text_color(t.text_muted).child(*body))
        }));
        page_with("help-page", "Help & shortcuts", "How Dove works, and every key it listens for.",
            &t,
            None,
            div()
                .flex()
                .gap(px(24.0))
                .items_start()
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(t.text_faint)
                                .child("Keyboard (change them in Settings › Keyboard)"),
                        )
                        .child(shortcuts),
                )
                .child(div().flex_1().child(tips)),
        )
    }
}

trait BorderTop {
    fn when_border(self, on: bool, t: &Theme) -> Self;
}

impl BorderTop for gpui::Div {
    fn when_border(self, on: bool, t: &Theme) -> Self {
        if on { self.border_t_1().border_color(t.border) } else { self }
    }
}
