//! Automations: prompts that start threads on a schedule or on an event, and
//! the Inbox of their runs.

use dove_core::automation::{Trigger, WEEKDAYS};
use dove_core::proto::HarnessKind;
use dove_core::store::{Automation, AutomationRun};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, uniform_list,
};
use gpui_base::input::{Input, Textarea};

use super::{AutoEditor, AutoTab, PageDialog, input, textarea};
use crate::assets::{icon, icons};
use crate::design::{radius, size, space, ty};
use crate::kit::controls::{self as ctl, Segment, Tone};
use crate::kit::{self, page};
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
        let full = |input: AnyElement| {
            div()
                .w_full()
                .min_h(px(size::FIELD))
                .px(px(10.0))
                .py(px(6.0))
                .flex()
                .items_center()
                .rounded(px(radius::SELECT))
                .bg(t.wash(0.06))
                .text_size(ty::BODY)
                .child(div().flex_1().min_w_0().child(input))
        };
        // A row whose control sits under its label, full width.
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

        let project_ids: Vec<_> = self.projects.iter().map(|p| p.id).collect();
        let project_ix = ed.project.and_then(|id| project_ids.iter().position(|x| *x == id)).unwrap_or(usize::MAX);
        let project = ctl::select("auto-project", "Project", |s: &mut Shell| &mut s.pages.auto_sel_project)
            .options(
                self.projects.iter().map(|p| ctl::SelectOption::new(p.name.clone()).glyph(icons::FOLDER)),
                project_ix,
            )
            .placeholder("Pick a project")
            .width(220.0)
            .on_select(move |this, i, _, cx| {
                if let Some(ed) = this.pages.editor.as_mut() {
                    ed.project = project_ids.get(i).copied().or(ed.project);
                }
                cx.notify();
            })
            .render(t, &self.pages.auto_sel_project, cx);

        let agents: Vec<HarnessKind> =
            self.installed.iter().map(|i| i.kind).filter(|k| dove_core::harness_supported(*k)).collect();
        let agent_ix = agents.iter().position(|k| *k == ed.harness).unwrap_or(usize::MAX);
        let agent = ctl::select("auto-agent", "Agent", |s: &mut Shell| &mut s.pages.auto_sel_agent)
            .options(agents.iter().map(|k| ctl::SelectOption::new(k.label()).glyph(ui::harness_icon(*k))), agent_ix)
            .placeholder(ed.harness.label())
            .width(220.0)
            .on_select(move |this, i, _, cx| {
                if let (Some(ed), Some(k)) = (this.pages.editor.as_mut(), agents.get(i)) {
                    ed.harness = *k;
                }
                cx.notify();
            })
            .render(t, &self.pages.auto_sel_agent, cx);

        let persona = (!self.personas.is_empty()).then(|| {
            let people: Vec<(String, HarnessKind)> = self.personas.iter().map(|p| (p.id.clone(), p.harness)).collect();
            let ix = ed.persona.as_ref().and_then(|id| people.iter().position(|(p, _)| p == id)).map_or(0, |i| i + 1);
            let labels = std::iter::once(ctl::SelectOption::new("Anyone")).chain(self.personas.iter().map(|p| {
                let mark = if p.avatar.trim().is_empty() { String::new() } else { format!("{} ", p.avatar) };
                ctl::SelectOption::new(format!("{mark}{}", p.name)).detail(p.harness.label())
            }));
            ctl::select("auto-persona", "Run as", |s: &mut Shell| &mut s.pages.auto_sel_persona)
                .options(labels, ix)
                .width(220.0)
                .menu_width(260.0)
                .on_select(move |this, i, _, cx| {
                    if let Some(ed) = this.pages.editor.as_mut() {
                        match i.checked_sub(1).and_then(|i| people.get(i)) {
                            Some((id, kind)) => {
                                ed.persona = Some(id.clone());
                                ed.harness = *kind;
                            }
                            None => ed.persona = None,
                        }
                    }
                    cx.notify();
                })
                .render(t, &self.pages.auto_sel_persona, cx)
        });

        let trigger_ix = TriggerKind::ALL.iter().position(|k| *k == ed.kind).unwrap_or(0);
        let trigger = ctl::select("auto-trigger", "Trigger", |s: &mut Shell| &mut s.pages.auto_sel_trigger)
            .options(super::options(TriggerKind::ALL.iter().map(|k| k.label())), trigger_ix)
            .width(220.0)
            .on_select(|this, i, _, cx| {
                if let (Some(ed), Some(k)) = (this.pages.editor.as_mut(), TriggerKind::ALL.get(i)) {
                    ed.kind = *k;
                }
                cx.notify();
            })
            .render(t, &self.pages.auto_sel_trigger, cx);

        let narrow = |input: AnyElement| full(input).w(px(220.0));
        let detail: AnyElement = match ed.kind {
            TriggerKind::Every => page::setting_row(t, false, "Minutes between runs", "At least 5.", narrow(Input::new(&ed.minutes).into_any_element())).into_any_element(),
            TriggerKind::Daily | TriggerKind::Weekdays => page::setting_row(t, false, "Time (local)", "Like 09:30.", narrow(Input::new(&ed.time).into_any_element())).into_any_element(),
            TriggerKind::Weekly => div()
                .flex()
                .flex_col()
                .child(page::setting_row(t, false, "Time (local)", "Like 09:30.", narrow(Input::new(&ed.time).into_any_element())))
                .child(page::row(t, false)
                .child(page::row_text(t, "Days", "Runs at that time on each chosen day."))
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(space::SM))
                        .child(div().flex().gap(px(4.0)).children(WEEKDAYS.iter().enumerate().map(|(i, d)| {
                            let i = i as u32;
                            let on = ed.weekdays.contains(&i);
                            let hover = t.wash(0.08);
                            div()
                                .id(SharedString::from(format!("wd-{i}")))
                                .role(gpui::Role::CheckBox)
                                .aria_toggled(if on { gpui::Toggled::True } else { gpui::Toggled::False })
                                .aria_label(SharedString::from(d.to_string()))
                                .size(px(30.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(radius::SELECT))
                                .text_size(ty::CAPTION)
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .cursor_pointer()
                                .map(|d| if on {
                                    d.bg(t.accent).text_color(t.on_solid)
                                } else {
                                    d.bg(t.wash(0.06)).text_color(t.text_muted).hover(move |s| s.bg(hover))
                                })
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
                        }))),
                ))
                .into_any_element(),
            TriggerKind::Cron => stacked(false, "Cron expression (minute hour day month weekday, local time)", "", full(Input::new(&ed.cron).into_any_element()).into_any_element()).into_any_element(),
            TriggerKind::OnCommit => page::setting_row(t, false, "Branch to watch", "", narrow(Input::new(&ed.branch).into_any_element())).into_any_element(),
            TriggerKind::OnPr | TriggerKind::OnReview => page::row(t, false)
                .child(page::info_strip(t, "Checked every 5 minutes through the GitHub CLI (gh). The PR's number, title and link are added to the prompt."))
                .into_any_element(),
        };

        let worktree = ctl::on_toggle(ctl::toggle(t, "auto-wt", ed.worktree), cx, |this, _, cx| {
            if let Some(ed) = this.pages.editor.as_mut() {
                ed.worktree = !ed.worktree;
            }
            cx.notify();
        });
        let enabled = ctl::on_toggle(ctl::toggle(t, "auto-enabled", ed.enabled), cx, |this, _, cx| {
            if let Some(ed) = this.pages.editor.as_mut() {
                ed.enabled = !ed.enabled;
            }
            cx.notify();
        });

        let what = page::card(t)
            .child(stacked(true, "Name", "", full(Input::new(&ed.name).into_any_element()).into_any_element()))
            .child(stacked(
                false,
                "Prompt",
                "What the agent does each time.",
                full(Textarea::new(&ed.prompt).into_any_element()).into_any_element(),
            ))
            .child(page::setting_row(t, false, "Project", "Where each run starts.", project))
            .child(page::setting_row(t, false, "Agent", "Supported agents on this device.", agent))
            .children(persona.map(|p| page::setting_row(t, false, "Run as", "The teammate each run starts as.", p)));
        let when = page::card(t)
            .child(page::setting_row(t, true, "Trigger", "", trigger))
            .child(detail)
            .child(page::setting_row(t, false, "Run in its own worktree", "Each run gets a fresh branch.", worktree))
            .child(page::setting_row(t, false, "Enabled", "Paused automations keep their settings.", enabled));
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(page::LABEL_INSET))
                    .text_size(ty::TITLE)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(if ed.id.is_some() { "Edit automation" } else { "New automation" }),
            )
            .child(page::section(t, "What it does", what).mt(px(space::MD)))
            .child(page::section(t, "When it runs", when))
            .when_some(ed.error.clone(), |d, e| d.child(div().mt(px(space::MD)).child(page::error_strip(t, e))))
            .child(
                div()
                    .mt(px(space::LG))
                    .flex()
                    .justify_end()
                    .gap(px(space::SM))
                    .child(ctl::button(t, "auto-cancel", "Cancel", Tone::Outlined).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.pages.editor = None;
                            cx.notify();
                        },
                    )))
                    .child(
                        ctl::button(t, "auto-save", "Save", Tone::Solid)
                            .on_click(cx.listener(|this, _, _, cx| this.save_editor(cx))),
                    ),
            )
            .into_any_element()
    }

    /// One automation in the list card.
    fn automation_row(&self, a: &Automation, first: bool, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let trigger = Trigger::parse(&a.trigger).map(|t| t.describe()).unwrap_or_else(|| "Unknown trigger".into());
        let project = self.project(a.project_id).map(|p| p.name.clone()).unwrap_or_else(|| "missing project".into());
        let last =
            a.last_run.map(|r| format!("Last run {}", ui::relative_time(r))).unwrap_or_else(|| "Never run".into());
        let (a1, a2, a3) = (a.clone(), a.clone(), a.clone());
        let id = a.id.clone();
        let toggle = ctl::on_toggle(
            ctl::toggle(t, SharedString::from(format!("auto-on-{}", a.id)), a.enabled),
            cx,
            move |this, _, cx| {
                let mut a = a1.clone();
                a.enabled = !a.enabled;
                let _ = this.engine.store.save_automation(&a);
                this.pages.refresh_automations(&this.engine);
                cx.notify();
            },
        );
        let meta = [trigger, project, last]
            .into_iter()
            .chain(a.worktree.then(|| "worktree".to_string()))
            .collect::<Vec<_>>()
            .join(" · ");
        page::row(t, first)
            .child(page::row_tile(t, ui::harness_icon(a.harness), Some(ui::harness_tint(a.harness, t, t.text_muted))))
            .child(
                div()
                    .flex_1()
                    .min_w(px(200.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .when(!a.enabled, |d| d.opacity(0.6))
                    .child(
                        div()
                            .text_size(ty::BODY)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(t.text)
                            .child(a.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(ty::META)
                            .text_color(t.text_muted)
                            .truncate()
                            .child(a.prompt.lines().next().unwrap_or_default().to_string()),
                    )
                    .child(div().text_size(ty::CAPTION).text_color(t.text_faint).truncate().child(meta)),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(space::XS))
                    .child(toggle)
                    .child(
                        ctl::button_sm(t, SharedString::from(format!("auto-run-{}", a.id)), "Run now", Tone::Filled)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.run_automation(&a2, None, window, cx);
                                this.pages.refresh_automations(&this.engine);
                            })),
                    )
                    .child(ui::icon_button(
                        SharedString::from(format!("auto-edit-{}", a.id)),
                        icons::PEN,
                        "Edit",
                        t,
                        cx.listener(move |this, _, window, cx| this.open_editor(Some(a3.clone()), window, cx)),
                    ))
                    .child(ui::icon_button(
                        SharedString::from(format!("auto-del-{}", a.id)),
                        icons::TRASH_BIN_MINIMALISTIC,
                        "Delete",
                        t,
                        cx.listener(move |this, _, window, cx| {
                            this.open_page_dialog(PageDialog::DeleteAutomation(id.clone()), window, cx)
                        }),
                    )),
            )
            .into_any_element()
    }

    /// One 40 px Inbox row.
    fn run_row(&self, ix: usize, r: &AutomationRun, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let name = self
            .pages
            .automations
            .iter()
            .find(|a| a.id == r.automation_id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "Deleted automation".into());
        let task = self.task(r.task_id).cloned();
        let id = r.task_id;
        page::row_hover(t, page::table_row(t, ix == 0).id(SharedString::from(format!("run-{ix}"))), false)
            .on_click(cx.listener(move |this, _, window, cx| {
                let _ = this.engine.store.mark_run_read(id);
                this.pages.refresh_automations(&this.engine);
                this.inbox_unread = this.pages.runs.iter().filter(|r| r.unread).count();
                this.open_task(id, window, cx);
            }))
            .child(if r.unread {
                ui::dot(t.accent, 8.0).into_any_element()
            } else {
                div().w(px(8.0)).flex_none().into_any_element()
            })
            .child(
                div().w(px(14.0)).flex_none().flex().justify_center().children(
                    task.as_ref().map(|task| {
                        kit::status_glyph(SharedString::from(format!("run-st-{ix}")), task.status, false, t)
                    }),
                ),
            )
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
                            .line_height(ty::lh::BODY)
                            .text_color(t.text)
                            .when(r.unread, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                            .child(task.map(|t| t.title).unwrap_or_else(|| "Thread deleted".into())),
                    )
                    .child(div().truncate().text_size(ty::CAPTION).text_color(t.text_faint).child(name)),
            )
            .child(div().flex_none().text_size(ty::META).text_color(t.text_faint).child(ui::relative_time(r.at)))
            .into_any_element()
    }

    pub fn render_automations(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let tab = self.pages.auto_tab;
        let unread = self.pages.runs.iter().filter(|r| r.unread).count();
        let tabs = ctl::segmented(
            &t,
            "auto-tabs",
            vec![Segment::new("Automations"), Segment::new(if unread > 0 { "Inbox •" } else { "Inbox" })],
            if tab == AutoTab::List { 0 } else { 1 },
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
        let new_btn = ctl::button(&t, "auto-new", "New automation", Tone::Solid)
            .child(icon(icons::PLUS).size(px(13.0)).text_color(t.on_solid))
            .on_click(cx.listener(|this, _, window, cx| this.open_editor(None, window, cx)));
        let header = div().flex().items_center().gap(px(space::SM)).child(tabs).child(new_btn).into_any_element();
        const TITLE: &str = "Automations";
        const SUBTITLE: &str = "Prompts that start threads on a schedule or when something happens in a repo.";

        let page: AnyElement = match tab {
            AutoTab::List => {
                let automations = self.pages.automations.clone();
                let list = (!automations.is_empty()).then(|| {
                    page::card(&t)
                        .children(automations.iter().enumerate().map(|(i, a)| self.automation_row(a, i == 0, &t, cx)))
                });
                let empty = (automations.is_empty() && self.pages.editor.is_none()).then(|| {
                    page::card(&t).child(page::empty_state(
                        &t,
                        icons::CLOCK_CIRCLE,
                        "No automations yet",
                        "Run a prompt every morning, when commits land on main, or when someone asks you for a review. Each run is a normal thread.",
                        None,
                    ))
                });
                let keep = (!self.engine.store.setting(crate::notify::KEEP_RUNNING).is_some_and(|v| v == "true")).then(|| {
                    page::info_strip(&t, "Automations run while Dove is open. To keep them running with the window closed, turn on \"Keep running in the background\" in Settings › Notifications.")
                });
                super::page_with(
                    "automations-page",
                    TITLE,
                    SUBTITLE,
                    &t,
                    Some(header),
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(space::LG))
                        .children(self.pages.editor.as_ref().map(|_| self.render_editor(&t, cx)))
                        .children(empty)
                        .children(list)
                        .children(keep),
                )
            }
            AutoTab::Inbox => {
                let runs = std::rc::Rc::new(self.pages.runs.clone());
                let count = runs.len();
                let body: AnyElement = if runs.is_empty() {
                    page::empty_state(
                        &t,
                        icons::BELL,
                        "Inbox is empty",
                        "Each automation run lands here, unread until you open it.",
                        None,
                    )
                    .into_any_element()
                } else {
                    let theme = t.clone();
                    let rows = runs.clone();
                    uniform_list(
                        "inbox-list",
                        count,
                        cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                            range.filter_map(|ix| rows.get(ix).map(|r| this.run_row(ix, r, &theme, cx))).collect()
                        }),
                    )
                    .h(px(count as f32 * page::TABLE_ROW))
                    .flex_shrink(1.0)
                    .min_h_0()
                    .into_any_element()
                };
                let (root, column) = page::page_fill("automations-inbox", page::PAGE_WIDE);
                root.child(crate::motion::fade_in(
                    "automations-inbox-in",
                    column
                        .child(page::page_head(&t, TITLE, SUBTITLE, Some(header)))
                        .child(page::card(&t).mt(px(24.0)).flex_shrink(1.0).min_h_0().child(body)),
                ))
                .into_any_element()
            }
        };
        page
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
