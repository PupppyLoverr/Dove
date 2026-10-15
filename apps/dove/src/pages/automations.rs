//! Automations: prompts that start threads on a schedule or on an event, and
//! the Inbox of their runs.

use dove_core::automation::{Trigger, WEEKDAYS};
use dove_core::proto::HarnessKind;
use dove_core::store::Automation;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_base::input::{Input, Textarea};

use super::{AutoEditor, AutoTab, input, page_with, textarea};
use crate::assets::{icon, icons};
use crate::design::ty;
use crate::kit;
use crate::shell::Shell;
use crate::theme::Theme;
use crate::ui;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TriggerKind {
    Every,
    Daily,
    Weekdays,
    Weekly,
    Cron,
    OnCommit,
    OnPr,
    OnReview,
}

impl TriggerKind {
    const ALL: [TriggerKind; 8] = [
        TriggerKind::Every,
        TriggerKind::Daily,
        TriggerKind::Weekdays,
        TriggerKind::Weekly,
        TriggerKind::Cron,
        TriggerKind::OnCommit,
        TriggerKind::OnPr,
        TriggerKind::OnReview,
    ];

    fn label(self) -> &'static str {
        match self {
            TriggerKind::Every => "Every N minutes",
            TriggerKind::Daily => "Daily",
            TriggerKind::Weekdays => "Weekdays",
            TriggerKind::Weekly => "Weekly",
            TriggerKind::Cron => "Cron",
            TriggerKind::OnCommit => "New commit",
            TriggerKind::OnPr => "PR opened",
            TriggerKind::OnReview => "Review requested",
        }
    }
}

fn parse_time(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

impl Shell {
    fn open_editor(&mut self, existing: Option<Automation>, window: &mut Window, cx: &mut Context<Self>) {
        let name = input("Name, e.g. Nightly dependency check", window, cx);
        let prompt = textarea("What should the agent do each time?", (3, 12), window, cx);
        let minutes = input("60", window, cx);
        let time = input("09:00", window, cx);
        let cron = input("0 9 * * 1-5", window, cx);
        let branch = input("main", window, cx);
        let mut ed = AutoEditor {
            id: None,
            name,
            prompt,
            project: self.home_project.or_else(|| self.projects.first().map(|p| p.id)),
            harness: self.composer.read(cx).harness(cx),
            persona: None,
            kind: TriggerKind::Weekdays,
            minutes,
            time,
            weekdays: vec![0],
            cron,
            branch,
            worktree: true,
            enabled: true,
            error: None,
        };
        let set =
            |e: &gpui::Entity<gpui_base::input::InputState>, v: String, window: &mut Window, cx: &mut Context<Self>| {
                e.update(cx, |s, cx| s.set_value(v, window, cx));
            };
        set(&ed.time, "09:00".into(), window, cx);
        set(&ed.minutes, "60".into(), window, cx);
        set(&ed.branch, "main".into(), window, cx);
        if let Some(a) = existing {
            ed.id = Some(a.id.clone());
            ed.project = Some(a.project_id);
            ed.harness = a.harness;
            ed.persona = self
                .engine
                .store
                .setting(&format!("automation_persona.{}", a.id))
                .filter(|id| self.personas.iter().any(|p| &p.id == id));
            ed.worktree = a.worktree;
            ed.enabled = a.enabled;
            set(&ed.name, a.name.clone(), window, cx);
            ed.prompt.update(cx, |s, cx| s.set_value(a.prompt.clone(), window, cx));
            match Trigger::parse(&a.trigger) {
                Some(Trigger::Every { minutes }) => {
                    ed.kind = TriggerKind::Every;
                    set(&ed.minutes, minutes.to_string(), window, cx);
                }
                Some(Trigger::At { hour, minute, weekdays }) => {
                    ed.kind = match weekdays.as_slice() {
                        [] => TriggerKind::Daily,
                        [0, 1, 2, 3, 4] => TriggerKind::Weekdays,
                        _ => TriggerKind::Weekly,
                    };
                    if ed.kind == TriggerKind::Weekly {
                        ed.weekdays = weekdays;
                    }
                    set(&ed.time, format!("{hour:02}:{minute:02}"), window, cx);
                }
                Some(Trigger::Cron { expr }) => {
                    ed.kind = TriggerKind::Cron;
                    set(&ed.cron, expr, window, cx);
                }
                Some(Trigger::OnCommit { branch }) => {
                    ed.kind = TriggerKind::OnCommit;
                    set(&ed.branch, branch, window, cx);
                }
                Some(Trigger::OnPullRequest { review_requested }) => {
                    ed.kind = if review_requested { TriggerKind::OnReview } else { TriggerKind::OnPr };
                }
                None => {}
            }
        }
        self.pages.editor = Some(ed);
        cx.notify();
    }

    fn save_editor(&mut self, cx: &mut Context<Self>) {
        let Some(ed) = self.pages.editor.as_mut() else { return };
        let name = ed.name.read(cx).value().trim().to_string();
        let prompt = ed.prompt.read(cx).value().trim().to_string();
        let fail = |ed: &mut AutoEditor, msg: &str| ed.error = Some(msg.to_string());
        if name.is_empty() || prompt.is_empty() {
            fail(ed, "Give it a name and a prompt");
            cx.notify();
            return;
        }
        let Some(project) = ed.project else {
            fail(ed, "Pick a project");
            cx.notify();
            return;
        };
        let time = parse_time(&ed.time.read(cx).value());
        let trigger = match ed.kind {
            TriggerKind::Every => match ed.minutes.read(cx).value().trim().parse::<u32>() {
                Ok(m) if m >= 5 => Trigger::Every { minutes: m },
                _ => {
                    fail(ed, "Every: at least 5 minutes");
                    cx.notify();
                    return;
                }
            },
            TriggerKind::Daily | TriggerKind::Weekdays | TriggerKind::Weekly => {
                let Some((hour, minute)) = time else {
                    fail(ed, "Time must look like 09:30");
                    cx.notify();
                    return;
                };
                let weekdays = match ed.kind {
                    TriggerKind::Daily => vec![],
                    TriggerKind::Weekdays => vec![0, 1, 2, 3, 4],
                    _ if ed.weekdays.is_empty() => {
                        fail(ed, "Pick at least one day");
                        cx.notify();
                        return;
                    }
                    _ => {
                        let mut d = ed.weekdays.clone();
                        d.sort_unstable();
                        d
                    }
                };
                Trigger::At { hour, minute, weekdays }
            }
            TriggerKind::Cron => {
                let expr = ed.cron.read(cx).value().trim().to_string();
                if dove_core::automation::Cron::parse(&expr).is_none() {
                    fail(ed, "Cron needs five fields: minute hour day month weekday");
                    cx.notify();
                    return;
                }
                Trigger::Cron { expr }
            }
            TriggerKind::OnCommit => {
                let branch = ed.branch.read(cx).value().trim().to_string();
                if branch.is_empty() {
                    fail(ed, "Which branch?");
                    cx.notify();
                    return;
                }
                Trigger::OnCommit { branch }
            }
            TriggerKind::OnPr => Trigger::OnPullRequest { review_requested: false },
            TriggerKind::OnReview => Trigger::OnPullRequest { review_requested: true },
        };
        let existing = ed.id.as_ref().and_then(|id| self.pages.automations.iter().find(|a| &a.id == id)).cloned();
        let a = Automation {
            id: ed.id.clone().unwrap_or_else(|| dove_core::proto::TaskId::new().to_string()),
            name,
            prompt,
            project_id: project,
            harness: ed.harness,
            model: None,
            trigger: trigger.to_json(),
            worktree: ed.worktree,
            enabled: ed.enabled,
            // A new schedule counts from now, so it never fires for the past.
            last_run: existing.as_ref().and_then(|a| a.last_run).or(Some(dove_core::store::now_ms())),
            state: existing.map(|a| a.state).unwrap_or_else(|| "{}".into()),
        };
        let _ =
            self.engine.store.set_setting(&format!("automation_persona.{}", a.id), ed.persona.as_deref().unwrap_or(""));
        match self.engine.store.save_automation(&a) {
            Ok(()) => {
                self.pages.editor = None;
                self.pages.refresh_automations(&self.engine);
            }
            Err(e) => {
                if let Some(ed) = self.pages.editor.as_mut() {
                    ed.error = Some(format!("{e:#}"));
                }
            }
        }
        cx.notify();
    }

    fn render_editor(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let ed = self.pages.editor.as_ref().expect("editor open");
        let field = |label: &'static str, el: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(div().text_size(ty::META).text_color(t.text_faint).child(label))
                .child(el)
        };
        let boxed = |el: AnyElement| {
            div()
                .min_h(px(32.0))
                .px(px(10.0))
                .py(px(6.0))
                .flex()
                .items_center()
                .rounded(px(8.0))
                .border_1()
                .border_color(t.border)
                .bg(t.wash(0.04))
                .text_size(ty::BODY)
                .child(div().flex_1().child(el))
                .into_any_element()
        };
        let kinds = div().flex().flex_wrap().gap(px(4.0)).children(TriggerKind::ALL.into_iter().map(|k| {
            let on = ed.kind == k;
            div()
                .id(SharedString::from(format!("tk-{k:?}")))
                .h(px(26.0))
                .px(px(10.0))
                .flex()
                .items_center()
                .rounded(px(7.0))
                .border_1()
                .border_color(if on { t.accent.opacity(0.6) } else { t.border })
                .when(on, |d| d.bg(t.accent.opacity(0.10)).text_color(t.text))
                .when(!on, |d| d.text_color(t.text_muted))
                .text_size(ty::SMALL)
                .cursor_pointer()
                .child(k.label())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(ed) = this.pages.editor.as_mut() {
                        ed.kind = k;
                    }
                    cx.notify();
                }))
        }));
        let detail: AnyElement = match ed.kind {
            TriggerKind::Every => field("Minutes between runs", boxed(Input::new(&ed.minutes).into_any_element())).into_any_element(),
            TriggerKind::Daily | TriggerKind::Weekdays => field("Time (local)", boxed(Input::new(&ed.time).into_any_element())).into_any_element(),
            TriggerKind::Weekly => div()
                .flex()
                .gap(px(12.0))
                .child(field("Time (local)", boxed(Input::new(&ed.time).into_any_element())))
                .child(field(
                    "Days",
                    div()
                        .flex()
                        .gap(px(4.0))
                        .children(WEEKDAYS.iter().enumerate().map(|(i, d)| {
                            let i = i as u32;
                            let on = ed.weekdays.contains(&i);
                            div()
                                .id(SharedString::from(format!("wd-{i}")))
                                .size(px(32.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(7.0))
                                .border_1()
                                .border_color(if on { t.accent.opacity(0.6) } else { t.border })
                                .when(on, |d| d.bg(t.accent.opacity(0.10)))
                                .text_size(ty::CAPTION)
                                .cursor_pointer()
                                .child(&d[..2])
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(ed) = this.pages.editor.as_mut() {
                                        match ed.weekdays.iter().position(|x| *x == i) {
                                            Some(p) => {
                                                ed.weekdays.remove(p);
                                            }
                                            None => ed.weekdays.push(i),
                                        }
                                    }
                                    cx.notify();
                                }))
                        }))
                        .into_any_element(),
                ))
                .into_any_element(),
            TriggerKind::Cron => field("Cron expression (minute hour day month weekday, local time)", boxed(Input::new(&ed.cron).into_any_element())).into_any_element(),
            TriggerKind::OnCommit => field("Branch to watch", boxed(Input::new(&ed.branch).into_any_element())).into_any_element(),
            TriggerKind::OnPr | TriggerKind::OnReview => div()
                .text_size(ty::SMALL)
                .text_color(t.text_faint)
                .child("Checked every 5 minutes through the GitHub CLI (gh). The PR's number, title and link are added to the prompt.")
                .into_any_element(),
        };
        let projects = div().flex().flex_wrap().gap(px(4.0)).children(self.projects.iter().map(|p| {
            let on = ed.project == Some(p.id);
            let id = p.id;
            ui::chip(SharedString::from(format!("ap-{id}")), t)
                .border_1()
                .border_color(if on { t.accent.opacity(0.6) } else { t.border })
                .when(on, |d| d.text_color(t.text))
                .child(p.name.clone())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(ed) = this.pages.editor.as_mut() {
                        ed.project = Some(id);
                    }
                    cx.notify();
                }))
        }));
        let agents = div().flex().flex_wrap().gap(px(4.0)).children(
            self.installed.iter().filter(|i| dove_core::harness_supported(i.kind)).map(|i| {
                let k: HarnessKind = i.kind;
                let on = ed.harness == k;
                ui::chip(SharedString::from(format!("ah-{}", k.id())), t)
                    .border_1()
                    .border_color(if on { t.accent.opacity(0.6) } else { t.border })
                    .when(on, |d| d.text_color(t.text))
                    .child(icon(ui::harness_icon(k)).size(px(13.0)).text_color(ui::harness_tint(k, t, t.text_muted)))
                    .child(k.label())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(ed) = this.pages.editor.as_mut() {
                            ed.harness = k;
                        }
                        cx.notify();
                    }))
            }),
        );
        div()
            .p(px(18.0))
            .flex()
            .flex_col()
            .gap(px(14.0))
            .rounded(px(14.0))
            .border_1()
            .border_color(t.border_strong)
            .bg(t.surface_raised)
            .child(div().text_size(ty::HEADING).font_weight(gpui::FontWeight::SEMIBOLD).child(if ed.id.is_some() {
                "Edit automation"
            } else {
                "New automation"
            }))
            .child(field("Name", boxed(Input::new(&ed.name).into_any_element())))
            .child(field("Prompt", boxed(Textarea::new(&ed.prompt).into_any_element())))
            .child(field("Project", projects.into_any_element()))
            .child(field("Agent", agents.into_any_element()))
            .when(!self.personas.is_empty(), |d| {
                let current = ed.persona.clone();
                d.child(field(
                    "Run as",
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(4.0))
                        .child(
                            ui::chip("auto-as-anyone", t)
                                .border_1()
                                .border_color(if current.is_none() { t.accent.opacity(0.6) } else { t.border })
                                .child("Anyone")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(ed) = this.pages.editor.as_mut() {
                                        ed.persona = None;
                                    }
                                    cx.notify();
                                })),
                        )
                        .children(self.personas.iter().map(|p| {
                            let on = current.as_deref() == Some(p.id.as_str());
                            let (pid, kind) = (p.id.clone(), p.harness);
                            ui::chip(SharedString::from(format!("auto-as-{}", p.id)), t)
                                .pl(px(5.0))
                                .border_1()
                                .border_color(if on { t.accent.opacity(0.6) } else { t.border })
                                .when(on, |c| c.text_color(t.text))
                                .child(crate::team::avatar(p, 16.0, t))
                                .child(p.name.clone())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(ed) = this.pages.editor.as_mut() {
                                        ed.persona = Some(pid.clone());
                                        ed.harness = kind;
                                    }
                                    cx.notify();
                                }))
                        }))
                        .into_any_element(),
                ))
            })
            .child(field("Trigger", kinds.into_any_element()))
            .child(detail)
            .child(
                div()
                    .flex()
                    .gap(px(16.0))
                    .child(kit::checkbox("auto-wt", ed.worktree, "Run in its own worktree", t).on_click(cx.listener(
                        |this, _, _, cx| {
                            if let Some(ed) = this.pages.editor.as_mut() {
                                ed.worktree = !ed.worktree;
                            }
                            cx.notify();
                        },
                    )))
                    .child(kit::checkbox("auto-enabled", ed.enabled, "Enabled", t).on_click(cx.listener(
                        |this, _, _, cx| {
                            if let Some(ed) = this.pages.editor.as_mut() {
                                ed.enabled = !ed.enabled;
                            }
                            cx.notify();
                        },
                    ))),
            )
            .when_some(ed.error.clone(), |d, e| d.child(div().text_size(ty::SMALL).text_color(t.danger).child(e)))
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(
                        ui::button_primary("auto-save", "Save", t)
                            .on_click(cx.listener(|this, _, _, cx| this.save_editor(cx))),
                    )
                    .child(ui::button_ghost("auto-cancel", "Cancel", t).on_click(cx.listener(|this, _, _, cx| {
                        this.pages.editor = None;
                        cx.notify();
                    }))),
            )
            .into_any_element()
    }

    pub fn render_automations(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let tab = self.pages.auto_tab;
        let unread = self.pages.runs.iter().filter(|r| r.unread).count();
        let tabs = kit::segmented(
            "auto-tabs",
            &["Automations", if unread > 0 { "Inbox •" } else { "Inbox" }],
            if tab == AutoTab::List { 0 } else { 1 },
            &t,
            {
                let weak = cx.weak_entity();
                move |i, _, cx| {
                    let _ = weak.update(cx, |s, cx| {
                        s.pages.auto_tab = if i == 0 { AutoTab::List } else { AutoTab::Inbox };
                        s.pages.refresh_automations(&s.engine);
                        cx.notify();
                    });
                }
            },
        );
        let new_btn = ui::button_primary("auto-new", "New automation", &t)
            .child(icon(icons::PLUS).size(px(13.0)).text_color(t.on_solid))
            .on_click(cx.listener(|this, _, window, cx| this.open_editor(None, window, cx)));
        let header = div().flex().items_center().gap(px(8.0)).child(tabs).child(new_btn).into_any_element();

        let body: AnyElement = match tab {
            AutoTab::List => {
                let list = div().flex().flex_col().gap(px(10.0)).children(
                    self.pages.automations.clone().into_iter().map(|a| {
                        let trigger = Trigger::parse(&a.trigger)
                            .map(|t| t.describe())
                            .unwrap_or_else(|| "Unknown trigger".into());
                        let project = self
                            .project(a.project_id)
                            .map(|p| p.name.clone())
                            .unwrap_or_else(|| "missing project".into());
                        let last = a
                            .last_run
                            .map(|r| format!("Last run {}", ui::relative_time(r)))
                            .unwrap_or_else(|| "Never run".into());
                        let (a1, a2, a3, a4) = (a.clone(), a.clone(), a.clone(), a.clone());
                        let confirm = self.pages.confirm_delete_auto.as_deref() == Some(a.id.as_str());
                        div()
                            .p(px(14.0))
                            .flex()
                            .items_center()
                            .gap(px(14.0))
                            .rounded(px(12.0))
                            .border_1()
                            .border_color(t.border)
                            .when(!a.enabled, |d| d.opacity(0.6))
                            .child(icon(ui::harness_icon(a.harness)).size(px(18.0)).text_color(ui::harness_tint(
                                a.harness,
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
                                            .text_size(ty::MD)
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .child(a.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(ty::SMALL)
                                            .text_color(t.text_muted)
                                            .truncate()
                                            .child(a.prompt.lines().next().unwrap_or_default().to_string()),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .gap(px(10.0))
                                            .text_size(ty::CAPTION)
                                            .text_color(t.text_faint)
                                            .child(trigger)
                                            .child(project)
                                            .child(last)
                                            .when(a.worktree, |d| d.child("worktree")),
                                    ),
                            )
                            .child(
                                kit::switch(SharedString::from(format!("auto-on-{}", a.id)), a.enabled, &t).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        let mut a = a1.clone();
                                        a.enabled = !a.enabled;
                                        let _ = this.engine.store.save_automation(&a);
                                        this.pages.refresh_automations(&this.engine);
                                        cx.notify();
                                    }),
                                ),
                            )
                            .child(
                                ui::button_ghost(SharedString::from(format!("auto-run-{}", a.id)), "Run now", &t)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.run_automation(&a2, None, window, cx);
                                        this.pages.refresh_automations(&this.engine);
                                    })),
                            )
                            .child(ui::icon_button(
                                SharedString::from(format!("auto-edit-{}", a.id)),
                                icons::PEN,
                                "Edit",
                                &t,
                                cx.listener(move |this, _, window, cx| this.open_editor(Some(a3.clone()), window, cx)),
                            ))
                            .child(if confirm {
                                ui::button_ghost(SharedString::from(format!("auto-del-yes-{}", a.id)), "Delete?", &t)
                                    .text_color(t.danger)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let _ = this.engine.store.delete_automation(&a4.id);
                                        this.pages.confirm_delete_auto = None;
                                        this.pages.refresh_automations(&this.engine);
                                        cx.notify();
                                    }))
                                    .into_any_element()
                            } else {
                                let id = a.id.clone();
                                ui::icon_button(
                                    SharedString::from(format!("auto-del-{}", a.id)),
                                    icons::TRASH_BIN_MINIMALISTIC,
                                    "Delete",
                                    &t,
                                    cx.listener(move |this, _, _, cx| {
                                        this.pages.confirm_delete_auto = Some(id.clone());
                                        cx.notify();
                                    }),
                                )
                                .into_any_element()
                            })
                    }),
                );
                let empty = (self.pages.automations.is_empty() && self.pages.editor.is_none()).then(|| {
                    kit::empty_state(
                        icons::CLOCK_CIRCLE,
                        "No automations yet",
                        "Run a prompt every morning, when commits land on main, or when someone asks you for a review. Each run is a normal thread.",
                        &t,
                    )
                });
                let keep = (!self.engine.store.setting(crate::notify::KEEP_RUNNING).is_some_and(|v| v == "true")).then(|| {
                    div()
                        .text_size(ty::META)
                        .text_color(t.text_faint)
                        .child("Automations run while Dove is open. To keep them running with the window closed, turn on \"Keep running in the background\" in Settings › Notifications.")
                });
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.0))
                    .children(self.pages.editor.as_ref().map(|_| self.render_editor(&t, cx)))
                    .children(empty)
                    .child(list)
                    .children(keep)
                    .into_any_element()
            }
            AutoTab::Inbox => {
                let runs = self.pages.runs.clone();
                if runs.is_empty() {
                    kit::empty_state(
                        icons::BELL,
                        "Inbox is empty",
                        "Each automation run lands here, unread until you open it.",
                        &t,
                    )
                    .into_any_element()
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .rounded(px(12.0))
                        .border_1()
                        .border_color(t.border)
                        .overflow_hidden()
                        .children(runs.into_iter().enumerate().map(|(i, r)| {
                            let name = self
                                .pages
                                .automations
                                .iter()
                                .find(|a| a.id == r.automation_id)
                                .map(|a| a.name.clone())
                                .unwrap_or_else(|| "Deleted automation".into());
                            let task = self.task(r.task_id).cloned();
                            let id = r.task_id;
                            div()
                                .id(SharedString::from(format!("run-{i}")))
                                .h(px(44.0))
                                .px(px(14.0))
                                .flex()
                                .items_center()
                                .gap(px(10.0))
                                .when(i > 0, |d| d.border_t_1().border_color(t.border))
                                .cursor_pointer()
                                .hover(|s| s.bg(t.wash(0.04)))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let _ = this.engine.store.mark_run_read(id);
                                    this.pages.refresh_automations(&this.engine);
                                    this.inbox_unread = this.pages.runs.iter().filter(|r| r.unread).count();
                                    this.open_task(id, window, cx);
                                }))
                                .child(if r.unread {
                                    ui::dot(t.accent, 8.0).into_any_element()
                                } else {
                                    div().w(px(8.0)).into_any_element()
                                })
                                .when_some(task.as_ref(), |d, task| {
                                    d.child(kit::status_glyph(
                                        SharedString::from(format!("run-st-{i}")),
                                        task.status,
                                        false,
                                        &t,
                                    ))
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .child(
                                            div()
                                                .truncate()
                                                .text_size(ty::BODY)
                                                .when(r.unread, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                                                .child(
                                                    task.map(|t| t.title).unwrap_or_else(|| "Thread deleted".into()),
                                                ),
                                        )
                                        .child(div().text_size(ty::CAPTION).text_color(t.text_faint).child(name)),
                                )
                                .child(
                                    div().text_size(ty::META).text_color(t.text_faint).child(ui::relative_time(r.at)),
                                )
                        }))
                        .into_any_element()
                }
            }
        };
        page_with(
            "automations-page",
            "Automations",
            "Prompts that start threads on a schedule or when something happens in a repo.",
            &t,
            Some(header),
            body,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_parse() {
        assert_eq!(parse_time("09:30"), Some((9, 30)));
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("nine"), None);
    }
}
