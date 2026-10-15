//! Team: teammate personas with a name, role, standing instructions,
//! and persistent cross-thread memory (`~/.dove/team/<id>/NOTES.md`).
//!
//! Inspired by multi-agent teammate workflows (Lead, Builder, Reviewer,
//! Designer, Verifier, Scribe), any installed harness can run under its own
//! individual persona for all of its threads.

use dove_core::proto::{HarnessKind, TaskStatus};
use dove_core::store::{Persona, Task};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_base::input::{Input, InputState, Textarea, TextareaState};

use crate::assets::{icon, icons};
use crate::design::{radius, space, ty};
use crate::kit::controls::{self as ctl, Tone};
use crate::kit::{self, page};
use crate::shell::{Route, Shell};
use crate::theme::Theme;
use crate::ui;

/// Emoji a teammate can wear instead of its initial.
pub const AVATARS: &[&str] = &[
    "🧭", "⚡", "🔍", "🎨", "🧪", "📐", "🛡", "🚀", "🎯", "🛠", "🔥", "🌿", "🦊", "🐙", "🦉", "🐝", "🐳", "🦋", "🐢",
    "🦁", "🐼", "🦜", "🐺", "🦔",
];

/// Tile hues offered in the editor.
pub const HUES: &[f32] = &[0.58, 0.47, 0.03, 0.86, 0.38, 0.14, 0.68, 0.76, 0.95, 0.09];

/// Teammate archetypes for one-click creation or seeding a starter team.
pub const TEMPLATES: &[(&str, &str, &str, f32, &str)] = &[
    (
        "Lead",
        "Lead architect",
        "🧭",
        0.58,
        "Break down goals before writing code. Map the architecture, identify invariants and edge cases, \
         coordinate multi-file changes cleanly, and verify every assumption before concluding.",
    ),
    (
        "Builder",
        "Full-stack engineer",
        "⚡",
        0.47,
        "Ship production-grade, minimal code end to end. Reuse existing primitives before introducing new \
         ones, avoid speculative abstractions, and verify that everything compiles and passes tests.",
    ),
    (
        "Reviewer",
        "Code & security auditor",
        "🔍",
        0.03,
        "Review before you change anything. Look for correctness bugs first, then security risks, race \
         conditions, and missing tests. Report findings most severe first with exact file and line.",
    ),
    (
        "Designer",
        "UI/UX & generative canvas",
        "🎨",
        0.86,
        "Craft minimal, high-craft interfaces with zero AI slop. Use crisp 1px hairlines, clear typographic \
         hierarchy, and cohesive tokens. Prototype and verify interactive layouts in the live Canvas.",
    ),
    (
        "Verifier",
        "QA & test engineer",
        "🧪",
        0.38,
        "Write deterministic, fast unit and integration tests. Reproduce bugs with a failing test first, \
         exercise edge cases, and keep memory and binary footprint lean.",
    ),
    (
        "Scribe",
        "Technical writer",
        "📐",
        0.14,
        "Write docs engineers can follow on their first read: short sentences, concrete examples, the \
         common path first. Keep READMEs, architecture notes, and comments in step with the code.",
    ),
];

/// The teammate's tile: its emoji, or its initial, on its hue.
pub fn avatar(p: &Persona, size: f32, t: &Theme) -> Div {
    let label = if p.avatar.trim().is_empty() {
        p.name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into())
    } else {
        p.avatar.clone()
    };
    kit::avatar_tile(label, p.hue, size, t)
}

/// "Lead architect · Claude Code"
pub fn subtitle(p: &Persona) -> String {
    match p.role.trim() {
        "" => p.harness.label().to_string(),
        role => format!("{role} · {}", p.harness.label()),
    }
}

/// The teammate being created or edited.
pub struct TeamEditor {
    pub id: Option<String>,
    pub name: gpui::Entity<InputState>,
    pub role: gpui::Entity<InputState>,
    pub instructions: gpui::Entity<TextareaState>,
    pub avatar: String,
    pub hue: f32,
    pub harness: HarnessKind,
    pub model: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
}

/// What a teammate is doing, from its threads.
#[derive(Default, Clone, Copy)]
pub struct Activity {
    pub threads: usize,
    pub running: usize,
    pub needs_you: usize,
}

impl Shell {
    pub fn persona(&self, id: &str) -> Option<&Persona> {
        self.personas.iter().find(|p| p.id == id)
    }

    /// The teammate running a thread, if any.
    pub fn persona_for(&self, task: dove_core::proto::TaskId) -> Option<&Persona> {
        self.persona_of.get(&task).and_then(|id| self.persona(id))
    }

    pub fn persona_tasks(&self, id: &str) -> Vec<Task> {
        let mut v: Vec<Task> =
            self.tasks.iter().filter(|t| self.persona_of.get(&t.id).is_some_and(|p| p == id)).cloned().collect();
        crate::sidebar::attention_sort(&mut v);
        v
    }

    pub fn persona_activity(&self, id: &str) -> Activity {
        let mut a = Activity::default();
        for t in self.tasks.iter().filter(|t| self.persona_of.get(&t.id).is_some_and(|p| p == id)) {
            a.threads += 1;
            match t.status {
                TaskStatus::Running => a.running += 1,
                TaskStatus::NeedsInput => a.needs_you += 1,
                _ => {}
            }
        }
        a
    }

    /// Open the Team page on one teammate.
    pub fn open_teammate(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.pages.team_edit = None;
        self.pages.team_notes_edit = None;
        self.pages.team_focus = id.or_else(|| self.personas.first().map(|p| p.id.clone()));
        self.load_team_notes();
        self.navigate(Route::Team, window, cx);
    }

    fn load_team_notes(&mut self) {
        self.pages.team_notes = self.pages.team_focus.as_ref().map(|id| {
            let path = self.engine.store.persona_notes_path(id);
            std::fs::read_to_string(path).unwrap_or_default()
        });
    }

    fn toggle_notes_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pages.team_notes_edit.is_some() {
            self.pages.team_notes_edit = None;
            cx.notify();
            return;
        }
        let current = self.pages.team_notes.clone().unwrap_or_default();
        let area = crate::pages::textarea(
            "What this teammate should remember from one thread to the next…",
            (6, 16),
            window,
            cx,
        );
        area.update(cx, |s, cx| {
            s.set_value(current, window, cx);
            s.focus(window, cx);
        });
        self.pages.team_notes_edit = Some(area);
        cx.notify();
    }

    fn save_team_notes(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.pages.team_focus.clone() else { return };
        let Some(area) = self.pages.team_notes_edit.take() else { return };
        let text = area.read(cx).value().to_string();
        let path = self.engine.store.persona_notes_path(&id);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, &text);
        self.pages.team_notes = Some(text);
        cx.notify();
    }

    /// Seed the default starter team (Lead, Builder, Reviewer, Designer)
    /// bound to installed harnesses.
    pub fn seed_starter_team(&mut self, cx: &mut Context<Self>) {
        let avail: Vec<HarnessKind> = self
            .installed
            .iter()
            .map(|i| i.kind)
            .filter(|k| *k != HarnessKind::Shell && dove_core::harness_supported(*k))
            .collect();
        let fallback = avail.first().copied().unwrap_or(HarnessKind::ClaudeCode);
        for (idx, (name, role, avatar, hue, brief)) in TEMPLATES.iter().take(4).enumerate() {
            if self.personas.iter().any(|p| p.name.eq_ignore_ascii_case(name)) {
                continue;
            }
            let harness = avail.get(idx % avail.len().max(1)).copied().unwrap_or(fallback);
            let p = Persona {
                id: uuid::Uuid::now_v7().to_string(),
                name: (*name).into(),
                role: (*role).into(),
                avatar: (*avatar).into(),
                hue: *hue,
                harness,
                model: None,
                instructions: (*brief).into(),
                created_at: dove_core::store::now_ms() + idx as i64,
            };
            let _ = self.engine.store.save_persona(&p);
        }
        self.reload(cx);
        self.pages.team_focus = self.personas.first().map(|p| p.id.clone());
        self.load_team_notes();
        cx.notify();
    }

    /// Start a new thread as `id`: opens the Home composer with the teammate
    /// pre-selected.
    pub fn start_as(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let persona = self.persona(id).cloned();
        self.composer.update(cx, |c, cx| c.use_persona(persona, cx));
        self.navigate(Route::Home, window, cx);
        crate::composer::focus(&self.composer, window, cx);
    }

    /// Dispatch a prompt directly from the teammate's workspace page.
    fn dispatch_as(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = self.pages.team_prompt.read(cx).value().trim().to_string();
        if prompt.is_empty() {
            self.start_as(id, window, cx);
            return;
        }
        let Some(persona) = self.persona(id).cloned() else { return };
        self.pages.team_prompt.update(cx, |s, cx| s.set_value("", window, cx));
        self.composer.update(cx, |c, cx| c.use_persona(Some(persona.clone()), cx));
        crate::composer::set_text(&self.composer, &prompt, window, cx);
        self.navigate(Route::Home, window, cx);
        self.composer.update(cx, |c, cx| c.submit(window, cx));
    }

    /// Open the editor: a blank teammate, a template, or an existing one.
    pub fn edit_teammate(&mut self, existing: Option<Persona>, window: &mut Window, cx: &mut Context<Self>) {
        let template = existing.clone().unwrap_or_else(|| {
            let harness = self.installed.first().map(|i| i.kind).unwrap_or(HarnessKind::ClaudeCode);
            Persona {
                id: String::new(),
                name: String::new(),
                role: String::new(),
                avatar: String::new(),
                hue: HUES[self.personas.len() % HUES.len()],
                harness,
                model: None,
                instructions: String::new(),
                created_at: dove_core::store::now_ms(),
            }
        });
        self.open_team_editor(template, existing.is_some(), window, cx);
    }

    fn open_team_editor(&mut self, p: Persona, existing: bool, window: &mut Window, cx: &mut Context<Self>) {
        let name = crate::pages::input("Name, like Ava or Reviewer", window, cx);
        let role = crate::pages::input("Role, like Lead architect or Design engineer", window, cx);
        let instructions = crate::pages::textarea(
            "How this teammate works: architectural standards, tone, and rules it follows on every thread",
            (6, 16),
            window,
            cx,
        );
        name.update(cx, |s, cx| {
            s.set_value(p.name.clone(), window, cx);
            s.focus(window, cx);
        });
        role.update(cx, |s, cx| s.set_value(p.role.clone(), window, cx));
        instructions.update(cx, |s, cx| s.set_value(p.instructions.clone(), window, cx));
        self.pages.team_edit = Some(TeamEditor {
            id: existing.then(|| p.id.clone()),
            name,
            role,
            instructions,
            avatar: p.avatar,
            hue: p.hue,
            harness: p.harness,
            model: p.model,
            error: None,
            created_at: p.created_at,
        });
        if self.route != Route::Team {
            self.navigate(Route::Team, window, cx);
        }
        cx.notify();
    }

    fn use_template(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (name, role, avatar, hue, brief) = TEMPLATES[i];
        let harness = self.installed.first().map(|i| i.kind).unwrap_or(HarnessKind::ClaudeCode);
        let p = Persona {
            id: String::new(),
            name: name.into(),
            role: role.into(),
            avatar: avatar.into(),
            hue,
            harness,
            model: None,
            instructions: brief.into(),
            created_at: dove_core::store::now_ms(),
        };
        self.open_team_editor(p, false, window, cx);
    }

    fn save_teammate(&mut self, cx: &mut Context<Self>) {
        let Some(ed) = self.pages.team_edit.as_mut() else { return };
        let name = ed.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            ed.error = Some("Give your teammate a name.".into());
            cx.notify();
            return;
        }
        let id = ed.id.clone().unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        let p = Persona {
            id: id.clone(),
            name,
            role: ed.role.read(cx).value().trim().to_string(),
            avatar: ed.avatar.clone(),
            hue: ed.hue,
            harness: ed.harness,
            model: ed.model.clone(),
            instructions: ed.instructions.read(cx).value().trim().to_string(),
            created_at: ed.created_at,
        };
        if let Err(e) = self.engine.store.save_persona(&p) {
            ed.error = Some(format!("Could not save: {e}"));
            cx.notify();
            return;
        }
        self.pages.team_edit = None;
        self.pages.team_focus = Some(id);
        self.reload(cx);
        self.load_team_notes();
        cx.notify();
    }

    pub(crate) fn delete_teammate(&mut self, id: &str, cx: &mut Context<Self>) {
        let _ = self.engine.store.delete_persona(id);
        self.pages.team_edit = None;
        self.pages.team_focus = None;
        if self.pages.team_expanded.as_deref() == Some(id) {
            self.pages.team_expanded = None;
        }
        self.reload(cx);
        self.pages.team_focus = self.personas.first().map(|p| p.id.clone());
        self.load_team_notes();
        cx.notify();
    }

    // ------------------------------------------------------------ sidebar

    /// The left sidebar's Team section: each teammate persona with live status,
    /// quick thread launch (`+`), and inline expandable thread list.
    pub fn render_team_section(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let on_team = self.route == Route::Team;
        let total = self.personas.len();
        let header = div()
            .group("team-head")
            .h(px(26.0))
            .mx(px(8.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .text_size(ty::CAPTION)
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(t.text_faint)
            .child(
                div()
                    .id("team-title")
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.text_color(t.text_muted))
                    .child("Team")
                    .when(total > 0, |d| {
                        d.child(
                            div()
                                .px(px(5.0))
                                .h(px(15.0))
                                .flex()
                                .items_center()
                                .rounded(px(4.0))
                                .bg(t.wash(0.06))
                                .text_size(ty::BADGE)
                                .text_color(t.text_faint)
                                .child(total.to_string()),
                        )
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.open_teammate(None, window, cx))),
            )
            .child(
                // Pulled out by its padding so the glyph, not the hit
                // area, ends on the sidebar's trailing line.
                div()
                    .id("team-add")
                    .size(px(20.0))
                    .mr(px(-3.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.wash(0.10)))
                    .tooltip(ui::tip("New teammate"))
                    .child(icon(icons::PLUS).size(px(14.0)).text_color(t.text_faint))
                    .on_click(cx.listener(|this, _, window, cx| this.edit_teammate(None, window, cx))),
            );
        let mut list = div().flex().flex_col().gap(px(1.0)).child(header);
        if self.personas.is_empty() {
            list = list.child(
                div()
                    .id("team-empty")
                    .h(px(30.0))
                    .mx(px(8.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .text_size(ty::BODY)
                    .text_color(t.text_faint)
                    .hover(|s| s.bg(t.wash(0.05)).text_color(t.text_muted))
                    .child(icon(icons::PLUS).size(px(14.0)))
                    .child("Add a teammate")
                    .on_click(cx.listener(|this, _, window, cx| this.open_teammate(None, window, cx))),
            );
        }
        for p in &self.personas {
            let a = self.persona_activity(&p.id);
            let id = p.id.clone();
            let id_expand = p.id.clone();
            let id_start = p.id.clone();
            let selected = on_team && self.pages.team_focus.as_deref() == Some(p.id.as_str());
            let expanded = self.pages.team_expanded.as_deref() == Some(p.id.as_str());
            let row_group = SharedString::from(format!("team-row-{}", p.id));
            let state: AnyElement = if a.needs_you > 0 {
                kit::badge(a.needs_you.to_string(), t.warning).into_any_element()
            } else if a.running > 0 {
                kit::spinner(SharedString::from(format!("team-run-{}", p.id)), 11.0, t.busy).into_any_element()
            } else if a.threads > 0 {
                div().text_size(ty::MICRO).text_color(t.text_faint).child(a.threads.to_string()).into_any_element()
            } else {
                icon(ui::harness_icon(p.harness))
                    .size(px(11.5))
                    .text_color(ui::harness_tint(p.harness, t, t.text_faint))
                    .into_any_element()
            };
            list = list.child(
                div()
                    .id(SharedString::from(format!("team-{}", p.id)))
                    .group(row_group.clone())
                    .h(px(30.0))
                    .mx(px(8.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .text_size(ty::BODY)
                    .when(selected, |d| d.bg(ui::selected_bg(t)).shadow(ui::selected_ring(t)).text_color(t.text))
                    .when(!selected, |d| d.text_color(t.text_muted).hover(|s| s.bg(t.wash(0.05)).text_color(t.text)))
                    .tooltip(ui::tip(subtitle(p)))
                    .child(avatar(p, 19.0, t))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .child(div().truncate().child(p.name.clone()))
                            .when(!p.role.is_empty(), |d| {
                                d.child(
                                    div()
                                        .truncate()
                                        .text_size(ty::MICRO)
                                        .text_color(t.text_faint)
                                        .child(p.role.clone()),
                                )
                            }),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("team-new-{}", p.id)))
                            .size(px(19.0))
                            .flex_none()
                            .rounded(px(5.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .invisible()
                            .group_hover(row_group.clone(), |s| s.visible())
                            .hover(|s| s.bg(t.wash(0.12)))
                            .tooltip(ui::tip(format!("New thread as {}", p.name)))
                            .child(icon(icons::PLUS).size(px(11.5)).text_color(t.text_muted))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.start_as(&id_start, window, cx);
                            })),
                    )
                    .when(a.threads > 0, |d| {
                        d.child(
                            div()
                                .id(SharedString::from(format!("team-exp-{}", p.id)))
                                .size(px(18.0))
                                .flex_none()
                                .rounded(px(4.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(|s| s.bg(t.wash(0.10)))
                                .tooltip(ui::tip(if expanded { "Hide threads" } else { "Show threads" }))
                                .child(
                                    icon(if expanded { icons::ALT_ARROW_DOWN } else { icons::ALT_ARROW_RIGHT })
                                        .size(px(10.5))
                                        .text_color(t.text_faint),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    if this.pages.team_expanded.as_deref() == Some(id_expand.as_str()) {
                                        this.pages.team_expanded = None;
                                    } else {
                                        this.pages.team_expanded = Some(id_expand.clone());
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .child(state)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.route == Route::Team && this.pages.team_focus.as_deref() == Some(id.as_str()) {
                            if this.pages.team_expanded.as_deref() == Some(id.as_str()) {
                                this.pages.team_expanded = None;
                            } else {
                                this.pages.team_expanded = Some(id.clone());
                            }
                            cx.notify();
                        } else {
                            this.open_teammate(Some(id.clone()), window, cx);
                        }
                    })),
            );
            if expanded {
                let tasks = self.persona_tasks(&p.id);
                for task in tasks.into_iter().take(6) {
                    let tid = task.id;
                    let active_task = self.route == Route::Thread(tid);
                    list = list.child(
                        div()
                            .id(SharedString::from(format!("team-sub-{tid}")))
                            .h(px(26.0))
                            .ml(px(24.0))
                            .mr(px(8.0))
                            .px(px(8.0))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .rounded(px(6.0))
                            .border_l_1()
                            .border_color(t.border)
                            .cursor_pointer()
                            .text_size(ty::META)
                            .when(active_task, |d| d.bg(t.wash(0.09)).text_color(t.text))
                            .when(!active_task, |d| {
                                d.text_color(t.text_muted).hover(|s| s.bg(t.wash(0.04)).text_color(t.text))
                            })
                            .child(kit::status_glyph(
                                SharedString::from(format!("team-st-{tid}")),
                                task.status,
                                task.unread,
                                t,
                            ))
                            .child(div().flex_1().min_w_0().truncate().child(task.title.clone()))
                            .on_click(cx.listener(move |this, _, window, cx| this.open_task(tid, window, cx))),
                    );
                }
            }
        }
        list.into_any_element()
    }

    // ------------------------------------------------------------ page

    pub fn render_team_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        if self.pages.team_focus.is_none() && self.pages.team_edit.is_none() {
            self.pages.team_focus = self.personas.first().map(|p| p.id.clone());
            self.load_team_notes();
        }
        let header_right = div()
            .flex()
            .items_center()
            .gap(px(space::SM))
            .when(self.personas.len() < 4, |d| {
                d.child(
                    ctl::button(&t, "team-seed-top", "Add starter team", Tone::Outlined)
                        .child(icon(icons::MAGIC_STICK_3).size(px(12.5)).text_color(t.accent))
                        .on_click(cx.listener(|this, _, _, cx| this.seed_starter_team(cx))),
                )
            })
            .child(
                ctl::button(&t, "team-new", "New teammate", Tone::Solid)
                    .on_click(cx.listener(|this, _, window, cx| this.edit_teammate(None, window, cx))),
            )
            .into_any_element();
        let body: AnyElement = if let Some(ed) = &self.pages.team_edit {
            self.render_team_editor(ed, &t, cx)
        } else if self.personas.is_empty() {
            self.render_team_empty(&t, cx)
        } else {
            div()
                .flex()
                .gap(px(space::XL))
                .items_start()
                .child(self.render_team_list(&t, cx))
                .child(div().flex_1().min_w_0().children(self.render_team_detail(&t, cx)))
                .into_any_element()
        };
        div()
            .size_full()
            .child(crate::pages::page_with(
                "team-page",
                "Team",
                "Agents with a name, a role and a standing brief. Each runs on its own agent and keeps notes from one thread to the next.",
                &t,
                Some(header_right),
                body,
            ))
            .into_any_element()
    }

    fn render_team_empty(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let hero = page::card(t).child(
            page::row(t, true)
                .child(page::row_tile(t, icons::BOT, Some(t.accent)))
                .child(page::row_text(
                    t,
                    "Assemble a team in one click",
                    "Create Lead, Builder, Reviewer, and Designer personas pre-configured for your installed agents.",
                ))
                .child(
                    ctl::button(t, "team-seed-hero", "Add starter team", Tone::Solid)
                        .on_click(cx.listener(|this, _, _, cx| this.seed_starter_team(cx))),
                ),
        );
        let hover = t.wash(0.075);
        let templates = div().flex().flex_wrap().gap(px(space::MD)).children(TEMPLATES.iter().enumerate().map(
            |(i, (name, role, avatar, hue, brief))| {
                page::card(t)
                    .id(SharedString::from(format!("team-tpl-{i}")))
                    .w(px(280.0))
                    .flex_grow(1.0)
                    .p(px(space::LG))
                    .gap(px(10.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(kit::avatar_tile(*avatar, *hue, 34.0, t))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_size(ty::BODY)
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(t.text)
                                            .child(*name),
                                    )
                                    .child(div().text_size(ty::META).text_color(t.text_muted).child(*role)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(ty::META)
                            .line_height(ty::lh::META)
                            .text_color(t.text_muted)
                            .line_clamp(3)
                            .child(*brief),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.use_template(i, window, cx)))
            },
        ));
        div()
            .flex()
            .flex_col()
            .child(hero)
            .child(page::section(t, "Or start from a role", templates))
            .into_any_element()
    }

    fn render_team_list(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let rows = page::card(t).p(px(space::XS)).gap(px(2.0)).children(self.personas.iter().map(|p| {
            let a = self.persona_activity(&p.id);
            let on = self.pages.team_focus.as_deref() == Some(p.id.as_str());
            let id = p.id.clone();
            let hover = t.wash(0.05);
            div()
                .id(SharedString::from(format!("team-row-{}", p.id)))
                .role(gpui::Role::Tab)
                .aria_selected(on)
                .h(px(52.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .gap(px(11.0))
                .rounded(px(radius::CARD - space::XS))
                .cursor_pointer()
                .when(on, |d| d.bg(t.wash(0.09)))
                .when(!on, |d| d.hover(move |s| s.bg(hover)))
                .child(avatar(p, 32.0, t))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(ty::BODY)
                                .font_weight(if on { gpui::FontWeight::MEDIUM } else { gpui::FontWeight::NORMAL })
                                .text_color(t.text)
                                .truncate()
                                .child(p.name.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.0))
                                .text_size(ty::CAPTION)
                                .text_color(t.text_faint)
                                .child(icon(ui::harness_icon(p.harness)).size(px(11.0)).text_color(ui::harness_tint(
                                    p.harness,
                                    t,
                                    t.text_faint,
                                )))
                                .child(div().truncate().child(if p.role.is_empty() {
                                    p.harness.label().to_string()
                                } else {
                                    p.role.clone()
                                })),
                        ),
                )
                .when(a.needs_you > 0, |d| d.child(ui::dot(t.warning, 7.0)))
                .when(a.needs_you == 0 && a.running > 0, |d| {
                    d.child(kit::spinner(SharedString::from(format!("team-list-run-{}", p.id)), 11.0, t.busy))
                })
                .when(a.needs_you == 0 && a.running == 0 && a.threads > 0, |d| {
                    d.child(div().text_size(ty::MICRO).text_color(t.text_faint).child(a.threads.to_string()))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.pages.team_focus = Some(id.clone());
                    this.pages.team_notes_edit = None;
                    this.load_team_notes();
                    cx.notify();
                }))
        }));
        div()
            .w(px(260.0))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(space::SM))
            .child(page::section_label(t, "Teammates"))
            .child(rows)
            .child(page::section_label(t, "Quick presets").mt(px(space::MD)))
            .child(div().px(px(space::XS)).flex().flex_wrap().gap(px(4.0)).children(TEMPLATES.iter().enumerate().map(
                |(i, (name, _, av, _, _))| {
                    ui::chip(SharedString::from(format!("tpl-quick-{i}")), t)
                        .child(format!("{av} {name}"))
                        .on_click(cx.listener(move |this, _, window, cx| this.use_template(i, window, cx)))
                },
            )))
            .into_any_element()
    }

    fn render_team_detail(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.persona(self.pages.team_focus.as_deref()?)?.clone();
        let tasks = self.persona_tasks(&p.id);
        let a = self.persona_activity(&p.id);
        let id = p.id.clone();
        let id_dispatch = p.id.clone();
        let model = p.model.clone().unwrap_or_else(|| "Default model".into());
        let head = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(14.0))
            .child(avatar(&p, 52.0, t))
            .child(
                div()
                    .flex_1()
                    .min_w(px(200.0))
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(
                        div()
                            .text_size(ty::PAGE)
                            .line_height(ty::lh::PAGE)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.text)
                            .child(p.name.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(6.0))
                            .text_size(ty::SMALL)
                            .text_color(t.text_muted)
                            .when(!p.role.is_empty(), |d| d.child(p.role.clone()).child("·"))
                            .child(icon(ui::harness_icon(p.harness)).size(px(12.0)).text_color(ui::harness_tint(
                                p.harness,
                                t,
                                t.text_faint,
                            )))
                            .child(p.harness.label())
                            .child("·")
                            .child(model),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(ctl::button(t, "tm-edit", "Edit", Tone::Outlined).on_click(cx.listener({
                        let p = p.clone();
                        move |this, _, window, cx| this.edit_teammate(Some(p.clone()), window, cx)
                    })))
                    .child(ctl::button(t, "tm-start", "Open in composer", Tone::Solid).on_click(cx.listener({
                        let id = id.clone();
                        move |this, _, window, cx| this.start_as(&id, window, cx)
                    }))),
            );

        // Quick-dispatch bar right inside the teammate's workspace
        let dispatch_bar = div()
            .h(px(44.0))
            .pl(px(space::MD))
            .pr(px(6.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .rounded(px(radius::CARD))
            .bg(page::card_fill(t))
            .border_1()
            .border_color(t.border)
            .text_size(ty::BODY)
            .child(icon(ui::harness_icon(p.harness)).size(px(14.0)).text_color(ui::harness_tint(
                p.harness,
                t,
                t.text_muted,
            )))
            .child(div().flex_1().min_w_0().child(Input::new(&self.pages.team_prompt)))
            .child(
                ctl::button_sm(t, "tm-dispatch-go", format!("Ask {}", p.name), Tone::Solid)
                    .on_click(cx.listener(move |this, _, window, cx| this.dispatch_as(&id_dispatch, window, cx))),
            );

        let stats = div().flex().flex_wrap().gap(px(space::SM)).children(
            [
                ("Threads", a.threads, None),
                ("Working", a.running, Some(t.busy)),
                ("Needs you", a.needs_you, Some(t.warning)),
            ]
            .map(|(label, n, tint)| page::stat(t, label, n.to_string(), tint.filter(|_| n > 0))),
        );
        let brief = page::card(t).child(
            div()
                .px(px(space::LG))
                .py(px(space::MD))
                .text_size(ty::BODY)
                .line_height(ty::lh::BODY)
                .text_color(if p.instructions.is_empty() { t.text_faint } else { t.text_muted })
                .child(if p.instructions.is_empty() {
                    "No standing instructions yet. Click Edit to define how this teammate works.".to_string()
                } else {
                    p.instructions.clone()
                }),
        );
        let threads = if tasks.is_empty() {
            page::card(t).child(
                div()
                    .px(px(space::LG))
                    .py(px(space::MD))
                    .text_size(ty::BODY)
                    .text_color(t.text_faint)
                    .child(format!("{} has not run a thread yet. Use the dispatch bar above to start one.", p.name)),
            )
        } else {
            page::card(t).children(tasks.iter().take(15).enumerate().map(|(i, task)| {
                let tid = task.id;
                let proj = self.project(task.project_id).map(|pr| pr.name.clone()).unwrap_or_default();
                page::row_hover(t, page::table_row(t, i == 0).id(SharedString::from(format!("tm-task-{tid}"))), false)
                    .child(div().w(px(12.0)).flex_none().flex().justify_center().child(kit::status_glyph(
                        SharedString::from(format!("tm-st-{tid}")),
                        task.status,
                        task.unread,
                        t,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(ty::BODY)
                            .text_color(t.text)
                            .child(task.title.clone()),
                    )
                    .when(!proj.is_empty(), |d| {
                        d.child(div().flex_none().text_size(ty::CAPTION).text_color(t.text_faint).child(proj))
                    })
                    .child(
                        div()
                            .flex_none()
                            .text_size(ty::CAPTION)
                            .text_color(t.text_faint)
                            .child(ui::relative_time(task.updated_at)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.open_task(tid, window, cx)))
            }))
        };
        let notes_text = self.pages.team_notes.clone().unwrap_or_default();
        let notes_path = self.engine.store.persona_notes_path(&p.id);
        let editing_notes = self.pages.team_notes_edit.clone();
        let notes_actions = div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .when(editing_notes.is_some(), |d| {
                d.child(
                    ctl::button_sm(t, "tm-notes-save", "Save notes", Tone::Solid)
                        .on_click(cx.listener(|this, _, _, cx| this.save_team_notes(cx))),
                )
                .child(
                    ctl::button_sm(t, "tm-notes-cancel", "Cancel", Tone::Quiet)
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_notes_editor(window, cx))),
                )
            })
            .when(editing_notes.is_none(), |d| {
                d.child(
                    ctl::button_sm(t, "tm-notes-edit", "Edit notes", Tone::Quiet)
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_notes_editor(window, cx))),
                )
            })
            .when(notes_path.exists(), |d| {
                d.child(
                    ctl::button_sm(t, "tm-notes-open", "Open file", Tone::Quiet)
                        .on_click(cx.listener(move |_, _, _, cx| cx.open_with_system(&notes_path))),
                )
            });
        let notes_body = if let Some(area) = editing_notes {
            page::card(t)
                .child(div().px(px(space::LG)).py(px(space::MD)).text_size(ty::BODY).child(Textarea::new(&area)))
                .border_1()
                .border_color(t.accent.opacity(0.5))
        } else {
            page::card(t).child(
                div()
                    .px(px(space::LG))
                    .py(px(space::MD))
                    .text_size(ty::BODY)
                    .line_height(ty::lh::BODY)
                    .text_color(if notes_text.trim().is_empty() { t.text_faint } else { t.text_muted })
                    .child(if notes_text.trim().is_empty() {
                        "Notes appear here as this teammate learns across threads, or click Edit notes to seed project conventions and memory."
                            .to_string()
                    } else {
                        notes_text
                    }),
            )
        };
        let notes = div()
            .mt(px(page::SECTION_GAP))
            .flex()
            .flex_col()
            .gap(px(space::SM))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::SM))
                    .child(div().flex_1().child(page::section_label(t, "Persistent Memory (NOTES.md)")))
                    .child(notes_actions),
            )
            .child(notes_body);
        Some(
            div()
                .flex()
                .flex_col()
                .child(head)
                .child(div().mt(px(space::LG)).child(dispatch_bar))
                .child(div().mt(px(space::MD)).child(stats))
                .child(page::section(t, "Threads", threads))
                .child(page::section(t, "Standing instructions (Brief)", brief))
                .child(notes)
                .into_any_element(),
        )
    }

    fn render_team_editor(&self, ed: &TeamEditor, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let preview = Persona {
            id: String::new(),
            name: ed.name.read(cx).value().to_string(),
            role: ed.role.read(cx).value().to_string(),
            avatar: ed.avatar.clone(),
            hue: ed.hue,
            harness: ed.harness,
            model: ed.model.clone(),
            instructions: String::new(),
            created_at: 0,
        };
        let initial = Persona { avatar: String::new(), ..preview.clone() };
        let avatar_picker = div()
            .flex()
            .flex_wrap()
            .gap(px(6.0))
            .child(self.avatar_choice(
                "tm-av-letter",
                avatar(&initial, 30.0, t),
                ed.avatar.is_empty(),
                String::new(),
                t,
                cx,
            ))
            .children(AVATARS.iter().enumerate().map(|(i, a)| {
                let p = Persona { avatar: (*a).to_string(), ..preview.clone() };
                self.avatar_choice(
                    SharedString::from(format!("tm-av-{i}")),
                    avatar(&p, 30.0, t),
                    ed.avatar == *a,
                    (*a).to_string(),
                    t,
                    cx,
                )
            }));
        let hues = div().flex().flex_wrap().gap(px(space::SM)).children(HUES.iter().enumerate().map(|(i, h)| {
            let on = (ed.hue - h).abs() < 0.001;
            let h = *h;
            ctl::swatch(
                t,
                SharedString::from(format!("tm-hue-{i}")),
                gpui::hsla(h, 0.55, 0.55, 1.0),
                on,
                format!("Hue {}", i + 1),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(ed) = this.pages.team_edit.as_mut() {
                    ed.hue = h;
                }
                cx.notify();
            }))
        }));
        let mut agents: Vec<HarnessKind> = self
            .installed
            .iter()
            .map(|i| i.kind)
            .filter(|k| *k != HarnessKind::Shell && dove_core::harness_supported(*k))
            .collect();
        if !agents.contains(&ed.harness) {
            agents.insert(0, ed.harness);
        }
        let picked = |on: bool, chip: gpui::Stateful<gpui::Div>| {
            chip.when(on, |d| d.border_color(t.accent).bg(t.accent.opacity(0.10)).text_color(t.text))
        };
        let agent_picker = div().flex().flex_wrap().gap(px(6.0)).children(agents.into_iter().map(|k| {
            picked(k == ed.harness, ui::chip(SharedString::from(format!("tm-agent-{}", k.id())), t))
                .child(icon(ui::harness_icon(k)).size(px(13.0)).text_color(ui::harness_tint(k, t, t.text_muted)))
                .child(k.label())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(ed) = this.pages.team_edit.as_mut()
                        && ed.harness != k
                    {
                        ed.harness = k;
                        ed.model = None;
                    }
                    cx.notify();
                }))
        }));
        let models = dove_core::models::catalog(ed.harness);
        let model_picker = div()
            .flex()
            .flex_wrap()
            .gap(px(6.0))
            .child(picked(ed.model.is_none(), ui::chip("tm-model-default", t)).child("Agent default").on_click(
                cx.listener(|this, _, _, cx| {
                    if let Some(ed) = this.pages.team_edit.as_mut() {
                        ed.model = None;
                    }
                    cx.notify();
                }),
            ))
            .children(models.into_iter().take(10).map(|m| {
                let on = ed.model.as_deref() == Some(m.id.as_str());
                let mid = m.id.clone();
                picked(on, ui::chip(SharedString::from(format!("tm-model-{}", m.id)), t))
                    .child(m.label.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(ed) = this.pages.team_edit.as_mut() {
                            ed.model = Some(mid.clone());
                        }
                        cx.notify();
                    }))
            }));
        let existing = ed.id.clone();
        let field = |input: AnyElement| {
            div()
                .w_full()
                .min_h(px(crate::design::size::FIELD))
                .px(px(10.0))
                .py(px(6.0))
                .flex()
                .items_center()
                .rounded(px(radius::SELECT))
                .bg(t.wash(0.06))
                .text_size(ty::BODY)
                .child(div().flex_1().min_w_0().child(input))
        };
        let stacked = |first: bool, title: &'static str, hint: &'static str, control: AnyElement| {
            page::row(t, first).child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(space::SM))
                    .child(page::row_text(t, title, hint))
                    .child(control),
            )
        };
        let identity = page::card(t)
            .child(
                page::row(t, true).child(avatar(&preview, 56.0, t)).child(
                    div()
                        .flex_1()
                        .min_w(px(240.0))
                        .flex()
                        .flex_wrap()
                        .gap(px(space::MD))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(160.0))
                                .flex()
                                .flex_col()
                                .gap(px(6.0))
                                .child(page::row_text(t, "Name", ""))
                                .child(field(Input::new(&ed.name).into_any_element())),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(160.0))
                                .flex()
                                .flex_col()
                                .gap(px(6.0))
                                .child(page::row_text(t, "Role", ""))
                                .child(field(Input::new(&ed.role).into_any_element())),
                        ),
                ),
            )
            .child(stacked(
                false,
                "Look",
                "An emoji or the name's initial, on a hue.",
                div().flex().flex_col().gap(px(10.0)).child(avatar_picker).child(hues).into_any_element(),
            ));
        let runs = page::card(t).child(stacked(true, "Runs on", "", agent_picker.into_any_element())).child(stacked(
            false,
            "Model",
            "",
            model_picker.into_any_element(),
        ));
        let brief = page::card(t).child(stacked(
            true,
            "Standing instructions (Brief)",
            "Sent ahead of the first message of every thread this teammate runs, with a pointer to its NOTES.md.",
            field(Textarea::new(&ed.instructions).into_any_element()).into_any_element(),
        ));
        let footer = div()
            .mt(px(space::LG))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(space::SM))
            .when_some(existing.clone(), |d, id| {
                d.child(ctl::button(t, "tm-delete", "Delete…", Tone::Danger).on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.open_page_dialog(crate::pages::PageDialog::DeleteTeammate(id.clone()), window, cx)
                    },
                )))
            })
            .child(div().flex_1())
            .child(ctl::button(t, "tm-cancel", "Cancel", Tone::Outlined).on_click(cx.listener(|this, _, _, cx| {
                this.pages.team_edit = None;
                cx.notify();
            })))
            .child(
                ctl::button(t, "tm-save", if existing.is_some() { "Save" } else { "Add to team" }, Tone::Solid)
                    .on_click(cx.listener(|this, _, _, cx| this.save_teammate(cx))),
            );
        div()
            .max_w(px(crate::design::size::PAGE_MAX))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(page::LABEL_INSET))
                    .text_size(ty::TITLE)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(if existing.is_some() { "Edit teammate" } else { "New teammate" }),
            )
            .child(page::section(t, "Identity", identity).mt(px(space::MD)))
            .child(page::section(t, "Agent", runs))
            .child(page::section(t, "Brief", brief))
            .when_some(ed.error.clone(), |d, e| d.child(div().mt(px(space::MD)).child(page::error_strip(t, e))))
            .child(footer)
            .into_any_element()
    }

    fn avatar_choice(
        &self,
        id: impl Into<SharedString>,
        tile: Div,
        on: bool,
        value: String,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hover = t.wash(0.06);
        div()
            .id(gpui::ElementId::Name(id.into()))
            .role(gpui::Role::RadioButton)
            .aria_selected(on)
            .p(px(2.0))
            .rounded(px(10.0))
            .border_2()
            .border_color(if on { t.accent } else { gpui::transparent_black() })
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .child(tile)
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(ed) = this.pages.team_edit.as_mut() {
                    ed.avatar = value.clone();
                }
                cx.notify();
            }))
            .into_any_element()
    }
}
