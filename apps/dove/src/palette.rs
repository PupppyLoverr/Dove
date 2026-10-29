//! The command palette (Ctrl+K): threads, projects, agents, actions,
//! settings rows, and files in the active thread's worktree (type `@`).
//! Fuzzy-ish matching: every query word must appear, earlier and tighter
//! matches rank higher. It sits on Zeron's `palette_card` over a light
//! scrim and enters with the dialog motion (opacity + 2 px settle).

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
use crate::kit::{self, popover};
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
    /// Rehearse a taught skill: (name, status label).
    Taught(String, &'static str),
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
            PaletteItem::Taught(name, status) => format!("Rehearse skill: {name} ({status})"),
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
            PaletteItem::Taught(..) => "Skill",
        }
    }
}

pub enum PaletteEvent {
    Pick(PaletteItem),
    /// Start a new thread with this text in the composer (Tab).
    Compose(String),
    Close,
}

/// Which kind of result the palette lists. ← and → move between them
/// while the query is empty; the tabs take a click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    All,
    Threads,
    Projects,
    Files,
    Actions,
    Settings,
}

impl Scope {
    pub const ALL: [Scope; 6] = [Self::All, Self::Threads, Self::Projects, Self::Files, Self::Actions, Self::Settings];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Threads => "Threads",
            Self::Projects => "Projects",
            Self::Files => "Files",
            Self::Actions => "Actions",
            Self::Settings => "Settings",
        }
    }

    fn admits(self, item: &PaletteItem) -> bool {
        match self {
            Self::All => !matches!(item, PaletteItem::File(_)),
            Self::Threads => matches!(item, PaletteItem::Thread(..)),
            Self::Projects => matches!(item, PaletteItem::Project(..)),
            Self::Files => matches!(item, PaletteItem::File(_)),
            Self::Actions => matches!(item, PaletteItem::Action(..) | PaletteItem::Agent(_) | PaletteItem::Taught(..)),
            Self::Settings => matches!(item, PaletteItem::Setting(..)),
        }
    }
}

/// The heading a row sits under in the unfiltered "All" list.
fn section_of(item: &PaletteItem) -> &'static str {
    match item {
        PaletteItem::Thread(..) => "Recent",
        PaletteItem::Project(..) => "Projects",
        PaletteItem::File(_) => "Files",
        PaletteItem::Setting(..) => "Settings",
        _ => "Actions",
    }
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
    scope: Scope,
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
            scope: Scope::All,
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
        let files_q = q.strip_prefix('@').or((self.scope == Scope::Files).then_some(q));
        if let Some(files_q) = files_q {
            let mut scored: Vec<(i32, &String)> =
                self.files.iter().filter_map(|f| score(f, files_q).map(|s| (s, f))).collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.len().cmp(&b.1.len())));
            self.shown = scored.into_iter().take(MAX_ROWS).map(|(_, f)| PaletteItem::File(f.clone())).collect();
            return;
        }
        let scope = self.scope;
        if q.is_empty() {
            // In "All", settings are found by searching, not listed up
            // front; recent threads lead, then the actions.
            let mut shown: Vec<PaletteItem> = self
                .items
                .iter()
                .filter(|i| scope.admits(i) && (scope == Scope::Settings || !matches!(i, PaletteItem::Setting(..))))
                .cloned()
                .collect();
            if scope == Scope::All {
                shown.sort_by_key(|i| match section_of(i) {
                    "Recent" => 0,
                    "Actions" => 1,
                    _ => 2,
                });
            }
            shown.truncate(MAX_ROWS);
            self.shown = shown;
            return;
        }
        let mut scored: Vec<(i32, &PaletteItem)> = self
            .items
            .iter()
            .filter(|i| scope.admits(i))
            .filter_map(|i| score(&i.label(), q).map(|s| (s, i)))
            .collect();
        scored.sort_by_key(|e| std::cmp::Reverse(e.0));
        self.shown = scored.into_iter().take(MAX_ROWS).map(|(_, i)| i.clone()).collect();
    }

    fn pick(&mut self, cx: &mut Context<Self>) {
        match self.shown.get(self.highlight).cloned() {
            Some(item) => cx.emit(PaletteEvent::Pick(item)),
            None => cx.emit(PaletteEvent::Close),
        }
    }

    pub fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        self.scope = scope;
        self.filter();
        self.scroll.scroll_to_item(0);
        cx.notify();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.shown.len();
        let empty = self.query.is_empty();
        let at = Scope::ALL.iter().position(|s| *s == self.scope).unwrap_or(0);
        match ev.keystroke.key.as_str() {
            "left" if empty => {
                self.set_scope(Scope::ALL[(at + Scope::ALL.len() - 1) % Scope::ALL.len()], cx);
            }
            "right" if empty => {
                self.set_scope(Scope::ALL[(at + 1) % Scope::ALL.len()], cx);
            }
            "tab" => cx.emit(PaletteEvent::Compose(self.query.trim().to_string())),
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
            PaletteItem::Taught(..) => {
                icon(icons::CHECKLIST).size(px(15.0)).text_color(t.text_muted).into_any_element()
            }
        }
    }
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf_trace::count_render("Palette");
        let t = Theme::of(cx).clone();
        let viewport = window.viewport_size();
        let width = 680.0_f32.min(f32::from(viewport.width) - 32.0).max(280.0);
        let list_max = (f32::from(viewport.height) - 260.0).clamp(120.0, 420.0);
        let hover = t.wash(0.07);
        let active = popover::row_active_bg(&t);
        let header = div()
            .min_h(px(52.0))
            .flex_none()
            .px(px(18.0))
            .py(px(8.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(icon(icons::PALETTE_SEARCH).size(px(16.0)).flex_none().text_color(t.text_muted))
            .child(div().flex_1().min_w_0().text_size(ty::MD).child(Input::new(&self.input)))
            .child(
                div()
                    .id("palette-close")
                    .size(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(crate::design::radius::ROW))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(icon(icons::CLOSE).size(px(14.0)).text_color(t.text_muted))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Close))),
            );
        let tabs = div()
            .flex_none()
            .px(px(12.0))
            .pb(px(8.0))
            .flex()
            .items_center()
            .gap(px(2.0))
            .border_b_1()
            .border_color(t.border)
            .children(Scope::ALL.into_iter().map(|scope| {
                let on = scope == self.scope;
                div()
                    .id(SharedString::from(format!("pal-tab-{}", scope.label())))
                    .role(gpui::Role::Tab)
                    .aria_selected(on)
                    .h(px(26.0))
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .rounded(px(crate::design::radius::ROW))
                    .text_size(ty::SMALL)
                    .cursor_pointer()
                    .when(on, |d| d.bg(active).text_color(t.text).font_weight(gpui::FontWeight::MEDIUM))
                    .when(!on, |d| d.text_color(t.text_muted).hover(move |s| s.bg(hover)))
                    .child(scope.label())
                    .on_click(cx.listener(move |this, _, _, cx| this.set_scope(scope, cx)))
            }));
        let list = div()
            .id("palette-list")
            .max_h(px(list_max))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .when(self.shown.is_empty(), |d| {
                d.child(div().px(px(10.0)).py(px(14.0)).text_size(ty::BODY).text_color(t.text_faint).child(
                    if self.query.starts_with('@') && self.files.is_empty() {
                        "No files (open a thread first)"
                    } else {
                        "Nothing matches"
                    },
                ))
            })
            .children(self.shown.iter().enumerate().flat_map(|(i, item)| {
                let selected = i == self.highlight;
                let item2 = item.clone();
                // Headings only in the unfiltered "All" list.
                let heading = (self.scope == Scope::All && self.query.is_empty())
                    .then(|| section_of(item))
                    .filter(|h| i == 0 || section_of(&self.shown[i - 1]) != *h)
                    .map(|h| {
                        div()
                            .px(px(10.0))
                            .pt(px(if i == 0 { 4.0 } else { 10.0 }))
                            .pb(px(4.0))
                            .text_size(ty::CAPTION)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.text_faint)
                            .child(h)
                            .into_any_element()
                    });
                let row = div()
                    .id(("pal-row", i))
                    .role(gpui::Role::ListBoxOption)
                    .aria_selected(selected)
                    .h(px(36.0))
                    .flex_none()
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .rounded(px(popover::PALETTE_ITEM_RADIUS))
                    .cursor_pointer()
                    .text_color(if selected { t.text } else { t.text.opacity(0.9) })
                    .when(selected, |d| d.bg(active))
                    .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                    .child(div().w(px(16.0)).flex_none().flex().justify_center().child(Self::glyph(item, &t)))
                    .child(div().flex_1().min_w_0().truncate().text_size(ty::BODY).child(item.label()))
                    .child(
                        div()
                            .flex_none()
                            .text_size(ty::CAPTION)
                            .text_color(t.text_faint)
                            .when(selected, |d| d.child(icon(icons::RETURN).size(px(13.0)).text_color(t.text_muted)))
                            .when(!selected, |d| d.child(item.kind())),
                    )
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(PaletteEvent::Pick(item2.clone()))))
                    .into_any_element();
                heading.into_iter().chain([row])
            }));
        let footer = div()
            .flex_none()
            .px(px(16.0))
            .py(px(7.0))
            .border_t_1()
            .border_color(t.border)
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(12.0))
            .child(popover::key_hint(&t, "Esc", "Close"))
            .child(popover::key_hint(&t, "← →", "Change type"))
            .child(popover::key_hint(&t, "↑ ↓", "Navigate"))
            .child(popover::key_hint(&t, "↵", "Open"))
            .child(div().flex_1())
            .child(popover::key_hint(&t, "Tab", "New thread with this"));
        let card = popover::palette_card(&t, width)
            .id("palette")
            .role(gpui::Role::Dialog)
            .aria_label("Command palette")
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::on_key))
            .child(header)
            .child(tabs)
            .child(list)
            .child(footer);
        let weak = cx.weak_entity();
        div().size_full().child(popover::modal(
            "palette-layer",
            viewport,
            popover::scrim(&t, popover::PALETTE_SCRIM),
            Some(90.0),
            ui::frosted(popover::PALETTE_RADIUS, 22.0, card).into_any_element(),
            Some(Box::new(move |_, cx| {
                let _ = weak.update(cx, |_, cx| cx.emit(PaletteEvent::Close));
            })),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_admit_their_own_kind() {
        let thread = PaletteItem::Thread(TaskId::new(), "Fix it".into(), TaskStatus::Idle);
        assert!(Scope::All.admits(&thread) && Scope::Threads.admits(&thread));
        assert!(!Scope::Settings.admits(&thread) && !Scope::Files.admits(&thread));
        assert!(!Scope::All.admits(&PaletteItem::File("a.rs".into())));
        assert_eq!(section_of(&PaletteItem::Action("New", "x", "new")), "Actions");
    }

    #[test]
    fn scoring_prefers_early_word_matches() {
        assert!(score("New thread", "new").unwrap() > score("Renew thread", "new").unwrap());
        assert!(score("Toggle side panel", "side pan").is_some());
        assert!(score("Settings", "xyz").is_none());
    }
}
