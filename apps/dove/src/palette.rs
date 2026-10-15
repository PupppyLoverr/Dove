//! The command palette (Ctrl+K): threads, projects, agents, actions, and
//! files in the active thread's worktree (type `@`). Fuzzy-ish matching:
//! every query word must appear, earlier and tighter matches rank higher.

use std::path::PathBuf;
use std::sync::Arc;

use dove_core::proto::{HarnessKind, ProjectId, TaskId, TaskStatus};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, px,
};
use gpui_base::input::{Input, InputEvent, InputState};

use crate::assets::{icon, icons};
use crate::design::ty;
use crate::kit;
use crate::theme::Theme;
use crate::ui;

#[derive(Clone, Debug)]
pub enum PaletteItem {
    /// (label, glyph, action id)
    Action(&'static str, &'static str, &'static str),
    Thread(TaskId, String, TaskStatus),
    Project(ProjectId, String),
    Agent(HarnessKind),
    File(String),
    /// One setting: (row id, title, section label).
    Setting(&'static str, &'static str, &'static str),
}

impl PaletteItem {
    fn label(&self) -> String {
        match self {
            PaletteItem::Action(l, _, _) => (*l).to_string(),
            PaletteItem::Thread(_, t, _) => t.clone(),
            PaletteItem::Project(_, n) => n.clone(),
            PaletteItem::Agent(k) => format!("New thread with {}", k.label()),
            PaletteItem::File(p) => p.clone(),
            PaletteItem::Setting(_, title, section) => format!("{section}: {title}"),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            PaletteItem::Action(..) => "Action",
            PaletteItem::Thread(..) => "Thread",
            PaletteItem::Project(..) => "Project",
            PaletteItem::Agent(..) => "Agent",
            PaletteItem::File(..) => "File",
            PaletteItem::Setting(..) => "Setting",
        }
    }
}

pub enum PaletteEvent {
    Pick(PaletteItem),
    Close,
}

/// Score `text` against every word of `query`; `None` when a word is missing.
pub fn score(text: &str, query: &str) -> Option<i32> {
    let text = text.to_lowercase();
    let mut total = 0;
    for word in query.split_whitespace() {
        let w = word.to_lowercase();
        let at = text.find(&w)?;
        // Earlier matches and matches at word starts rank higher.
        let boundary = at == 0 || !text.as_bytes()[at - 1].is_ascii_alphanumeric();
        total += 100 - (at as i32).min(80) + if boundary { 30 } else { 0 };
    }
    Some(total)
}

const MAX_ROWS: usize = 40;

pub struct Palette {
    items: Vec<PaletteItem>,
    files: Arc<Vec<String>>,
    input: Entity<InputState>,
    query: String,
    shown: Vec<PaletteItem>,
    highlight: usize,
    scroll: ScrollHandle,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl EventEmitter<PaletteEvent> for Palette {}

impl Focusable for Palette {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Palette {
    pub fn new(items: Vec<PaletteItem>, root: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let style = ui::input_style(Theme::of(cx));
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Search threads, projects, actions… (@ for files)");
            s.set_editor_style(style);
            s
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        let subs = vec![cx.subscribe_in(&input, window, |this, input, ev: &InputEvent, _, cx| match ev {
            InputEvent::Change => {
                this.query = input.read(cx).value().to_string();
                this.filter();
                cx.notify();
            }
            InputEvent::PressEnter { .. } => this.pick(cx),
            InputEvent::Blur => {}
            _ => {}
        })];
        if let Some(root) = root {
            let load = cx.background_spawn(async move { dove_core::git::list_files(&root).unwrap_or_default() });
            cx.spawn(async move |this, cx| {
                let files = load.await;
                let _ = this.update(cx, |p, cx| {
                    p.files = Arc::new(files);
                    p.filter();
                    cx.notify();
                });
            })
            .detach();
        }
        let mut this = Self {
            items,
            files: Arc::default(),
            input,
            query: String::new(),
            shown: Vec::new(),
            highlight: 0,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            _subs: subs,
        };
        this.filter();
        this
    }

    fn filter(&mut self) {
        let q = self.query.trim();
        self.highlight = 0;
        if let Some(files_q) = q.strip_prefix('@') {
            let mut scored: Vec<(i32, &String)> =
                self.files.iter().filter_map(|f| score(f, files_q).map(|s| (s, f))).collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.len().cmp(&b.1.len())));
            self.shown = scored.into_iter().take(MAX_ROWS).map(|(_, f)| PaletteItem::File(f.clone())).collect();
            return;
        }
        if q.is_empty() {
            // Settings are found by searching, not listed up front.
            self.shown =
                self.items.iter().filter(|i| !matches!(i, PaletteItem::Setting(..))).take(MAX_ROWS).cloned().collect();
            return;
        }
        let mut scored: Vec<(i32, &PaletteItem)> =
            self.items.iter().filter_map(|i| score(&i.label(), q).map(|s| (s, i))).collect();
        scored.sort_by_key(|e| std::cmp::Reverse(e.0));
        self.shown = scored.into_iter().take(MAX_ROWS).map(|(_, i)| i.clone()).collect();
    }

    fn pick(&mut self, cx: &mut Context<Self>) {
        match self.shown.get(self.highlight).cloned() {
            Some(item) => cx.emit(PaletteEvent::Pick(item)),
            None => cx.emit(PaletteEvent::Close),
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.shown.len();
        match ev.keystroke.key.as_str() {
            "down" if n > 0 => {
                self.highlight = (self.highlight + 1) % n;
                self.scroll.scroll_to_item(self.highlight);
            }
            "up" if n > 0 => {
                self.highlight = (self.highlight + n - 1) % n;
                self.scroll.scroll_to_item(self.highlight);
            }
            "escape" => cx.emit(PaletteEvent::Close),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn glyph(item: &PaletteItem, t: &Theme) -> AnyElement {
        match item {
            PaletteItem::Action(_, g, _) => icon(g).size(px(15.0)).text_color(t.text_muted).into_any_element(),
            PaletteItem::Thread(id, _, status) => {
                kit::status_glyph(SharedString::from(format!("pal-{id}")), *status, false, t)
            }
            PaletteItem::Project(..) => icon(icons::FOLDER).size(px(15.0)).text_color(t.text_muted).into_any_element(),
            PaletteItem::Agent(k) => icon(ui::harness_icon(*k))
                .size(px(15.0))
                .text_color(ui::harness_tint(*k, t, t.text_muted))
                .into_any_element(),
            PaletteItem::File(_) => icon(icons::DOCUMENT).size(px(15.0)).text_color(t.text_muted).into_any_element(),
            PaletteItem::Setting(..) => {
                icon(icons::SETTINGS).size(px(15.0)).text_color(t.text_muted).into_any_element()
            }
        }
    }
}

impl Render for Palette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("Palette");
        let t = Theme::of(cx).clone();
        ui::frosted(
            14.0,
            22.0,
            div()
                .id("palette")
                .track_focus(&self.focus)
                .capture_key_down(cx.listener(Self::on_key))
                .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Close)))
                .w(px(600.0))
                .max_h(px(480.0))
                .flex()
                .flex_col()
                .rounded(px(14.0))
                .bg(t.glass_overlay())
                .border_1()
                .border_color(t.border_strong)
                .shadow_lg()
                .child(
                    div()
                        .h(px(48.0))
                        .px(px(14.0))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .border_b_1()
                        .border_color(t.border)
                        .text_size(ty::TITLE)
                        .child(icon(icons::MAGNIFER).size(px(16.0)).text_color(t.text_faint))
                        .child(div().flex_1().child(Input::new(&self.input)))
                        .child(kit::kbd("Esc", &t)),
                )
                .child(
                    div()
                        .id("palette-list")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .py(px(6.0))
                        .when(self.shown.is_empty(), |d| {
                            d.child(div().px(px(16.0)).py(px(10.0)).text_size(ty::BODY).text_color(t.text_faint).child(
                                if self.query.starts_with('@') && self.files.is_empty() {
                                    "No files (open a thread first)"
                                } else {
                                    "Nothing matches"
                                },
                            ))
                        })
                        .children(self.shown.iter().enumerate().map(|(i, item)| {
                            let selected = i == self.highlight;
                            let item2 = item.clone();
                            div()
                                .id(("pal-row", i))
                                .mx(px(6.0))
                                .h(px(36.0))
                                .px(px(10.0))
                                .flex()
                                .items_center()
                                .gap(px(10.0))
                                .rounded(px(8.0))
                                .cursor_pointer()
                                .when(selected, |d| d.bg(t.wash(0.10)))
                                .hover(|s| s.bg(t.wash(0.07)))
                                .child(div().w(px(16.0)).flex().justify_center().child(Self::glyph(item, &t)))
                                .child(div().flex_1().min_w_0().truncate().text_size(ty::BODY).child(item.label()))
                                .child(div().text_size(ty::CAPTION).text_color(t.text_faint).child(item.kind()))
                                .on_click(cx.listener(move |_, _, _, cx| cx.emit(PaletteEvent::Pick(item2.clone()))))
                        })),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoring_prefers_early_word_matches() {
        assert!(score("New thread", "new").unwrap() > score("Renew thread", "new").unwrap());
        assert!(score("Toggle side panel", "side pan").is_some());
        assert!(score("Settings", "xyz").is_none());
    }
}
