//! The new-thread canvas: a full-bleed wallpaper, the composer, and the
//! detected-agents pill. With no threads open the window has no title bar;
//! the shell floats a small navigation cluster over this page instead.

use dove_core::proto::HarnessKind;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, px,
};

use crate::assets::{icon, icons};
use crate::design::ty;
use crate::shell::Shell;
use crate::theme::Theme;
use crate::ui;
use crate::wallpaper;

const SUGGESTIONS: &[(&str, &str, &str)] = &[
    (
        "Explain codebase",
        icons::FOLDER_TREE,
        "Give me a tour of this codebase: the main parts, how they fit together, and where to start reading.",
    ),
    (
        "Design an app screen",
        icons::PALETTE_SEARCH,
        "Design an interactive screen in Dove's Canvas (`show_canvas`) with React and Tailwind, plus an inline `show_ui` summary card in chat: ",
    ),
    (
        "Mobile UI mockup",
        icons::SMARTPHONE,
        "Design a mobile app flow in Dove's Canvas (`device: \"mobile\"`) and an inline `show_ui` phone mockup card: ",
    ),
    (
        "Find & fix a bug",
        icons::ACTION_DEBUG,
        "Look for a likely bug in this project, explain it, and fix it with a test.",
    ),
    (
        "Review my changes",
        icons::FILE_CODE,
        "Review the uncommitted changes in this project. List bugs and risks, most severe first.",
    ),
];

impl Shell {
    pub fn render_home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        if self.projects.is_empty() {
            return self.render_onboarding(&t, cx);
        }

        // Wallpaper fills the page. The composer is cut out of it.
        let viewport_h = f32::from(window.viewport_size().height);
        let wall_path = self.engine.store.setting("wallpaper_path").filter(|p| std::path::Path::new(p).is_file());
        let effect = wallpaper::Effect::from_id(&self.engine.store.setting("wallpaper_effect").unwrap_or_default());
        let animate = self.engine.store.setting("wallpaper_animate").as_deref() == Some("true");
        if !animate || wall_path.is_none() {
            wallpaper::release_animation(window);
        }
        let hero = wall_path.and_then(|p| {
            let scale = window.scale_factor();
            let width = f32::from(window.viewport_size().width) - if self.sidebar_open { self.sidebar_w } else { 0.0 };
            let target = (width * scale, wallpaper::hero_height(viewport_h) * scale);
            let path = std::path::Path::new(&p);
            let light = !t.appearance.is_dark();
            let (image, motion) = if animate {
                wallpaper::animate(path, effect, light, target, window, cx)?
            } else {
                (wallpaper::prepare(path, effect, light, target, cx)?, wallpaper::Motion::STILL)
            };
            let opacity = motion.opacity * if t.is_frost() { wallpaper::FROSTED_OPACITY } else { 1.0 };
            let layers = [false, true].map(|cutout| {
                let image = image.clone();
                let bounds_cell = self.composer_bounds.clone();
                div().absolute().inset_0().opacity(if cutout { 1.0 } else { wallpaper::CUTOUT_REVEAL_OPACITY }).child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            wallpaper::paint(image.clone(), bounds, bounds_cell.get(), cutout, motion, window);
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
            });
            // Zeron's hero: the top 72% of the page (at most 760 px), fading
            // into the canvas below the composer.
            Some(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(wallpaper::hero_height(viewport_h)))
                    .opacity(opacity)
                    .children(layers),
            )
        });

        let join_chip = self.join.as_ref().map(|(_, j)| {
            ui::chip("home-join", &t)
                .child(icon(icons::GIT_BRANCH).size(px(14.0)).text_color(t.accent))
                .child(format!("Side chat on {}", j.branch.clone().unwrap_or_else(|| "this branch".into())))
                .child(icon(icons::CLOSE).size(px(12.0)).text_color(t.text_faint))
                .tooltip(ui::tip("Shares the worktree of the thread you came from"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.join = None;
                    cx.notify();
                }))
        });

        let active_persona = self.composer.read(cx).persona.clone();
        let bounds_cell = self.composer_bounds.clone();
        let composer = div()
            .relative()
            .w_full()
            .child(self.composer.clone())
            .child(canvas(move |b, _, _| bounds_cell.set(Some(b)), |_, _, _, _| {}).absolute().inset_0());

        let suggestions = div().flex().flex_wrap().justify_center().gap(px(6.0)).children(
            SUGGESTIONS.iter().enumerate().map(|(i, (label, glyph, prompt))| {
                let prompt = prompt.to_string();
                ui::chip(SharedString::from(format!("suggest-{i}")), &t)
                    .border_1()
                    .border_color(t.border)
                    .child(icon(glyph).size(px(12.5)).text_color(t.text_faint))
                    .child(*label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        crate::composer::set_text(&this.composer, &prompt, window, cx);
                        crate::composer::focus(&this.composer, window, cx);
                    }))
            }),
        );

        let heading = match &active_persona {
            Some(p) => format!("What should {} work on?", p.name),
            None => "What should we build?".to_string(),
        };
        let subheading = active_persona.as_ref().map(|p| {
            if p.role.is_empty() {
                format!("Running on {}", p.harness.label())
            } else {
                format!("{} · {}", p.role, p.harness.label())
            }
        });

        let scenic = hero.is_some();
        let column = div()
            .w_full()
            .max_w(px(820.0))
            .px(px(32.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .when(!scenic, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(4.0))
                        .pb(px(8.0))
                        .child(
                            div()
                                .text_center()
                                .text_size(ty::DISPLAY)
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(t.text)
                                .child(heading),
                        )
                        .when_some(subheading, |d, sub| {
                            d.child(div().text_size(ty::BODY).text_color(t.text_muted).child(sub))
                        }),
                )
            })
            .children(join_chip.map(|c| div().flex().justify_end().child(c)))
            .child(composer)
            .when(!scenic, |d| d.child(suggestions))
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .id("home-error")
                        .p(px(10.0))
                        .rounded(px(10.0))
                        .bg(t.danger.opacity(0.12))
                        .text_color(t.danger)
                        .text_size(ty::BODY)
                        .cursor_pointer()
                        .child(e)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.error = None;
                            cx.notify();
                        })),
                )
            });

        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .children(hero)
            // The bar sits a little above the middle, as in Zeron: the
            // artwork owns the top, the bar the center.
            .child(div().h(px(if scenic {
                (viewport_h * 0.33).clamp(56.0, 380.0)
            } else {
                (viewport_h * 0.22).clamp(48.0, 240.0)
            })))
            .child(column)
            .child(div().flex_1())
            .child(self.render_agents_pill(&t, cx))
            .into_any_element()
    }

    fn render_agents_pill(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if !self.updates.is_empty() {
            return self.render_updates_pill(t, cx);
        }
        let ready: Vec<_> = self.installed.iter().filter(|i| dove_core::harness_supported(i.kind)).collect();
        let has_marks = !ready.is_empty();
        let marks = stacked_marks(ready.iter().take(4).map(|i| i.kind), t);
        let label = if self.installed.is_empty() {
            "Looking for agents…".to_string()
        } else {
            format!("{} agents ready · {}", ready.len(), ui::host_name())
        };
        let list = self.agents_open.then(|| {
            div().mb(px(8.0)).w(px(PILL_MENU_W)).child(ui::menu_card(
                t,
                div().flex().flex_col().children(self.installed.iter().map(|i| {
                    let ok = dove_core::harness_supported(i.kind);
                    div()
                        .h(px(34.0))
                        .px(px(14.0))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .text_size(ty::MD)
                        .when(!ok, |d| d.opacity(0.5))
                        .child(icon(ui::harness_icon(i.kind)).text_color(ui::harness_tint(i.kind, t, t.text)))
                        .child(div().flex_1().child(i.kind.label()))
                        .child(
                            div().text_size(ty::META).text_color(t.text_faint).child(
                                self.versions
                                    .get(&i.kind)
                                    .cloned()
                                    .unwrap_or_else(|| if ok { "ready".into() } else { "not supported yet".into() }),
                            ),
                        )
                })),
            ))
        });
        div()
            .pb(px(20.0))
            .flex()
            .flex_col()
            .items_center()
            .children(list)
            .child(ui::frosted(
                PILL_H / 2.0,
                16.0,
                pill_body("agents-pill", t)
                    .when(!has_marks, |d| d.pl(px(14.0)))
                    .when(has_marks, |d| d.child(marks))
                    .child(div().flex_none().whitespace_nowrap().child(label))
                    .child(pill_chevron(self.agents_open, t))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.agents_open = !this.agents_open;
                        cx.notify();
                    })),
            ))
            .into_any_element()
    }

    /// Zeron's agent-update island: overlapping marks and "3 agent updates
    /// · HOST"; open, a row per agent with its versions and an Update action.
    fn render_updates_pill(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let n = self.updates.len();
        let title = if n == 1 {
            let u = &self.updates[0];
            format!("{} {} available · {}", u.kind.label(), u.latest, ui::host_name())
        } else {
            format!("{n} agent updates · {}", ui::host_name())
        };
        let marks = stacked_marks(self.updates.iter().take(3).map(|u| u.kind), t);
        let list = self.agents_open.then(|| {
            div().mb(px(8.0)).w(px(PILL_MENU_W)).child(ui::menu_card(
                t,
                div().flex().flex_col().children(self.updates.iter().map(|u| {
                    let kind = u.kind;
                    div()
                        .h(px(56.0))
                        .px(px(14.0))
                        .flex()
                        .items_center()
                        .gap(px(12.0))
                        .child(mark(kind, t))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(div().text_size(ty::BODY).text_color(t.text).child(kind.label()))
                                .child(
                                    div()
                                        .text_size(ty::META)
                                        .text_color(t.text_faint)
                                        .child(format!("{} → {}", u.installed, u.latest)),
                                ),
                        )
                        .child(
                            ui::button_primary(SharedString::from(format!("update-{}", kind.id())), "Update", t)
                                .h(px(26.0))
                                .text_size(ty::META)
                                .on_click(cx.listener(move |this, _, window, cx| this.run_update(kind, window, cx))),
                        )
                })),
            ))
        });
        div()
            .pb(px(20.0))
            .flex()
            .flex_col()
            .items_center()
            .children(list)
            .child(ui::frosted(
                PILL_H / 2.0,
                16.0,
                pill_body("updates-pill", t)
                    .child(marks)
                    .child(div().flex_none().whitespace_nowrap().child(title))
                    .child(pill_chevron(self.agents_open, t))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.agents_open = !this.agents_open;
                        cx.notify();
                    })),
            ))
            .into_any_element()
    }

    fn render_onboarding(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .child(icon(icons::FOLDER_WITH_FILES).size(px(36.0)).text_color(t.text.opacity(0.28)))
            .child(
                div()
                    .mt(px(18.0))
                    .text_size(ty::HEADING)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("Add a project to get started"),
            )
            .child(
                div()
                    .text_size(ty::BODY)
                    .text_color(t.text_muted)
                    .child("A project is a folder on this device, usually a git repository."),
            )
            .child(
                ui::button_primary("onboard-add", "Add a project", t)
                    .mt(px(16.0))
                    .on_click(cx.listener(|this, _, _, cx| this.add_project_dialog(cx))),
            )
            .into_any_element()
    }
}

/// The island at the foot of the home page: one height, corner and inset
/// for both the agents and the updates variants.
const PILL_H: f32 = 36.0;
const PILL_MENU_W: f32 = 352.0;
/// Agent marks in the island: discs that overlap a little, so the stack
/// has an exact width and never runs into the label after it.
const MARK: f32 = 22.0;
const MARK_STEP: f32 = 17.0;

fn pill_body(id: &'static str, t: &Theme) -> gpui::Stateful<gpui::Div> {
    let hover = t.border_strong;
    div()
        .id(id)
        .h(px(PILL_H))
        // The first disc sits concentric with the pill's round end.
        .pl(px((PILL_H - MARK) / 2.0 - 1.0))
        .pr(px(14.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.0))
        .rounded(px(PILL_H / 2.0))
        .bg(t.glass_overlay())
        .border_1()
        .border_color(t.border)
        .text_size(ty::SMALL)
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(t.text)
        .cursor_pointer()
        .hover(move |s| s.border_color(hover))
}

fn pill_chevron(open: bool, t: &Theme) -> impl IntoElement {
    icon(if open { icons::ALT_ARROW_DOWN } else { icons::ALT_ARROW_UP })
        .size(px(12.0))
        .flex_none()
        .text_color(t.text_faint)
}

fn mark(kind: HarnessKind, t: &Theme) -> gpui::Div {
    div()
        .size(px(MARK))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(t.surface_raised)
        .border_1()
        .border_color(t.border_strong)
        .child(icon(ui::harness_icon(kind)).size(px(13.0)).text_color(ui::harness_tint(kind, t, t.text)))
}

/// Overlapping agent marks with a fixed footprint. Each disc is placed
/// rather than pulled left with a negative margin, so the label can't slide
/// underneath the stack.
fn stacked_marks(kinds: impl Iterator<Item = HarnessKind>, t: &Theme) -> gpui::Div {
    let kinds: Vec<HarnessKind> = kinds.collect();
    let width = if kinds.is_empty() { 0.0 } else { MARK + MARK_STEP * (kinds.len() - 1) as f32 };
    div().relative().flex_none().w(px(width)).h(px(MARK)).children(
        kinds
            .into_iter()
            .enumerate()
            .map(|(i, kind)| div().absolute().top_0().left(px(MARK_STEP * i as f32)).child(mark(kind, t))),
    )
}
