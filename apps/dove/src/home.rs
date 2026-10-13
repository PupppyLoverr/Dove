//! The new-thread canvas: an optional wallpaper hero, a greeting, the
//! composer (its context row picks device, folder, branch and worktree),
//! suggestion chips and the detected-agents pill.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, px,
};

use crate::assets::{icon, icons};
use crate::shell::Shell;
use crate::theme::Theme;
use crate::ui;
use crate::wallpaper;

const SUGGESTIONS: &[(&str, &str)] = &[
    (
        "Explain this codebase",
        "Give me a tour of this codebase: the main parts, how they fit together, and where to start reading.",
    ),
    ("Find and fix a bug", "Look for a likely bug in this project, explain it, and fix it with a test."),
    ("Add tests", "Find the least-tested important code in this project and add focused tests for it."),
    ("Review my changes", "Review the uncommitted changes in this project. List bugs and risks, most severe first."),
];

impl Shell {
    pub fn render_home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        if self.projects.is_empty() {
            return self.render_onboarding(&t, cx);
        }

        // Wallpaper hero, painted behind the column with the composer cut out.
        let viewport_h = f32::from(window.viewport_size().height);
        let wall_path = self.engine.store.setting("wallpaper_path").filter(|p| std::path::Path::new(p).is_file());
        let effect = wallpaper::Effect::from_id(&self.engine.store.setting("wallpaper_effect").unwrap_or_default());
        let hero = wall_path.and_then(|p| {
            let image = wallpaper::prepare(std::path::Path::new(&p), effect, !t.appearance.is_dark(), cx)?;
            let opacity = if t.is_frost() { wallpaper::FROSTED_OPACITY } else { 1.0 };
            let h = wallpaper::hero_height(viewport_h);
            let layers = [false, true].map(|cutout| {
                let image = image.clone();
                let bounds_cell = self.composer_bounds.clone();
                div().absolute().inset_0().opacity(if cutout { 1.0 } else { wallpaper::CUTOUT_REVEAL_OPACITY }).child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, window, _| {
                            wallpaper::paint(image.clone(), bounds, bounds_cell.get(), cutout, window);
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
            });
            Some(div().absolute().top_0().left_0().right_0().h(px(h)).opacity(opacity).children(layers))
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

        let bounds_cell = self.composer_bounds.clone();
        let composer = div()
            .relative()
            .w_full()
            .child(self.composer.clone())
            .child(canvas(move |b, _, _| bounds_cell.set(Some(b)), |_, _, _, _| {}).absolute().inset_0());

        let suggestions = div().flex().flex_wrap().justify_center().gap(px(6.0)).children(
            SUGGESTIONS.iter().enumerate().map(|(i, (label, prompt))| {
                let prompt = prompt.to_string();
                ui::chip(SharedString::from(format!("suggest-{i}")), &t)
                    .border_1()
                    .border_color(t.border)
                    .child(*label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        crate::composer::set_text(&this.composer, &prompt, window, cx);
                        crate::composer::focus(&this.composer, window, cx);
                    }))
            }),
        );

        let column = div()
            .w_full()
            .max_w(px(780.0))
            .px(px(24.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(
                div()
                    .text_center()
                    .text_size(px(26.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child("What should we build?"),
            )
            .children(join_chip.map(|c| div().flex().justify_center().child(c)))
            .child(composer)
            .child(suggestions)
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .id("home-error")
                        .p(px(10.0))
                        .rounded(px(10.0))
                        .bg(t.danger.opacity(0.12))
                        .text_color(t.danger)
                        .text_size(px(13.0))
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
            .child(div().h(px((viewport_h * 0.26).clamp(80.0, 300.0))))
            .child(column)
            .child(div().flex_1())
            .child(self.render_agents_pill(&t, cx))
            .into_any_element()
    }

    fn render_agents_pill(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let ready: Vec<_> = self.installed.iter().filter(|i| dove_core::harness_supported(i.kind)).collect();
        let marks = div().flex().children(ready.iter().take(4).enumerate().map(|(n, i)| {
            div()
                .ml(px(if n == 0 { 0.0 } else { -5.0 }))
                .size(px(20.0))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(t.surface_raised)
                .border_1()
                .border_color(t.border)
                .child(icon(ui::harness_icon(i.kind)).size(px(12.0)).text_color(ui::harness_tint(i.kind, t, t.text)))
        }));
        let label = if self.installed.is_empty() {
            "Looking for agents…".to_string()
        } else {
            format!("{} agents ready · {}", ready.len(), ui::host_name())
        };
        let list = self.agents_open.then(|| {
            div().mb(px(8.0)).w(px(340.0)).child(ui::menu_card(
                t,
                div().flex().flex_col().children(self.installed.iter().map(|i| {
                    let ok = dove_core::harness_supported(i.kind);
                    div()
                        .h(px(34.0))
                        .px(px(14.0))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .text_size(px(14.0))
                        .when(!ok, |d| d.opacity(0.5))
                        .child(icon(ui::harness_icon(i.kind)).text_color(ui::harness_tint(i.kind, t, t.text)))
                        .child(div().flex_1().child(i.kind.label()))
                        .child(
                            div().text_size(px(12.0)).text_color(t.text_faint).child(
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
            .pb(px(18.0))
            .flex()
            .flex_col()
            .items_center()
            .children(list)
            .child(ui::frosted(
                18.0,
                16.0,
                div()
                    .id("agents-pill")
                    .h(px(36.0))
                    .pl(px(8.0))
                    .pr(px(12.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .rounded(px(18.0))
                    .bg(t.glass_overlay())
                    .border_1()
                    .border_color(t.border)
                    .text_size(px(13.0))
                    .cursor_pointer()
                    .child(marks)
                    .child(label)
                    .child(
                        icon(if self.agents_open { icons::ALT_ARROW_DOWN } else { icons::ALT_ARROW_UP })
                            .size(px(12.0))
                            .text_color(t.text_faint),
                    )
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
                    .text_size(px(16.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("Add a project to get started"),
            )
            .child(
                div()
                    .text_size(px(13.0))
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
