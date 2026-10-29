//! Help: keyboard shortcuts (with the user's overrides) and how Dove works.

use gpui::{AnyElement, Context, ParentElement as _, Styled as _, div, px};

use super::page_with;
use crate::design::{space, ty};
use crate::kit::{self, page};
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
        "Type @ in the composer for files, / for the agent's commands and Keplorayx's prompt templates.",
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
        let shortcuts = page::card(&t).children(crate::keymap::shortcuts().into_iter().enumerate().map(|(i, s)| {
            let key = crate::keymap::binding(&store, &s);
            page::table_row(&t, i == 0)
                .child(div().flex_1().min_w_0().truncate().text_size(ty::BODY).text_color(t.text).child(s.label))
                .child(kit::kbd(&crate::keymap::pretty(&key), &t))
        }));
        let tips = page::card(&t).children(
            TIPS.iter()
                .enumerate()
                .map(|(i, (title, body))| page::row(&t, i == 0).child(page::row_text(&t, *title, *body))),
        );
        let column = |label: &'static str, card: gpui::Div| {
            div()
                .flex_1()
                .min_w(px(320.0))
                .flex()
                .flex_col()
                .gap(px(space::SM))
                .child(page::section_label(&t, label))
                .child(card)
        };
        page_with(
            "help-page",
            "Help & shortcuts",
            "How Keplorayx works, and every key it listens for.",
            &t,
            None,
            div()
                .flex()
                .flex_wrap()
                .gap(px(space::XL))
                .items_start()
                .child(column("Keyboard (change them in Settings › Keyboard)", shortcuts))
                .child(column("How Keplorayx works", tips)),
        )
    }
}
