//! Races: every race so far, and one race's compare view (status, time,
//! cost, diff and check per contender, with Pick winner and Discard
//! others). State and actions live in `crate::race`; the rules in
//! `dove_core::race`.

use dove_core::race::{self, Mark, Phase, Race, Standing};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use super::page_with;
use crate::assets::{icon, icons};
use crate::design::{radius, space, ty};
use crate::kit::controls::{self as ctl, Tone};
use crate::kit::{self, page};
use crate::race::Arm;
use crate::shell::Shell;
use crate::theme::Theme;
use crate::ui;

const SUBTITLE: &str =
    "One prompt, several agents and machines, each in its own branch. Compare, then merge the winner.";

fn cost(u: Option<dove_core::proto::Usage>) -> (String, String) {
    match u {
        None => ("—".into(), "no usage yet".into()),
        Some(u) => {
            let c = match u.cost_usd {
                Some(c) if u.estimated => format!("~${c:.2}"),
                Some(c) => format!("${c:.2}"),
                None => "—".into(),
            };
            (c, format!("{} tok", ui::compact(u.total_tokens())))
        }
    }
}

impl Shell {
    pub fn render_races_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        // Reached without `navigate` too (`DOVE_ROUTE=races`).
        self.ensure_races();
        if let Some(id) = self.race.open.clone()
            && self.race.get(&id).is_some()
        {
            return self.render_race_compare(&id, &t, cx);
        }
        self.render_race_list(&t, cx)
    }

    fn new_race_button(&self, id: &'static str, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        ctl::button(t, id, "New race", Tone::Outlined)
            .child(icon(icons::BOLT).size(px(13.0)).text_color(t.text_muted))
            .on_click(cx.listener(|this, _, window, cx| {
                let draft = this.composer.read(cx).race_draft(cx);
                this.open_race_sheet(draft, window, cx);
            }))
            .into_any_element()
    }

    fn render_race_list(&mut self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let races = self.race.races().to_vec();
        let body = if races.is_empty() {
            page::card(t)
                .child(page::empty_state(
                    t,
                    icons::BOLT,
                    "No races yet",
                    "Type a prompt, then choose Race agents… in the composer's + menu to run it with several agents or machines at once.",
                    Some(self.new_race_button("races-new-empty", t, cx)),
                ))
                .into_any_element()
        } else {
            let rows = races.iter().enumerate().map(|(i, r)| {
                let id = r.id.clone();
                let project =
                    self.project(r.project).map(|p| p.name.clone()).unwrap_or_else(|| "Removed project".into());
                let (phase, tint) = self.phase_text(r, t);
                page::row_hover(t, page::table_row(t, i == 0).id(SharedString::from(format!("race-row-{i}"))), false)
                    .child(icon(icons::BOLT).size(px(14.0)).flex_none().text_color(t.text_faint))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(ty::BODY)
                            .text_color(t.text)
                            .child(r.title.clone()),
                    )
                    .child(div().flex().flex_none().gap(px(4.0)).children(r.contenders.iter().map(|c| {
                        icon(ui::harness_icon(c.harness)).size(px(13.0)).text_color(ui::harness_tint(
                            c.harness,
                            t,
                            t.text_muted,
                        ))
                    })))
                    .child(
                        div()
                            .w(px(140.0))
                            .flex_none()
                            .truncate()
                            .text_size(ty::META)
                            .text_color(t.text_muted)
                            .child(project),
                    )
                    .child(div().w(px(150.0)).flex_none().truncate().text_size(ty::META).text_color(tint).child(phase))
                    .child(
                        div()
                            .w(px(40.0))
                            .flex_none()
                            .text_right()
                            .text_size(ty::META)
                            .text_color(t.text_faint)
                            .child(ui::relative_time(r.created_ms)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.open_race(Some(id.clone()), window, cx)))
            });
            page::card(t).children(rows).into_any_element()
        };
        page_with("races", "Races", SUBTITLE, t, Some(self.new_race_button("races-new", t, cx)), body)
    }

    fn phase_text(&self, r: &Race, t: &Theme) -> (String, Hsla) {
        match r.phase(|id| self.race_status(id)) {
            Phase::Running { left } => (format!("{left} of {} working", r.contenders.len()), t.busy),
            Phase::Compare => ("Ready to compare".into(), t.accent),
            Phase::Decided => {
                let who = r
                    .winner
                    .and_then(|w| r.contender(w))
                    .map(|c| c.harness.label().to_string())
                    .unwrap_or_else(|| "picked".into());
                (format!("Winner: {who}"), t.success)
            }
        }
    }

    fn render_race_compare(&mut self, id: &str, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(r) = self.race.get(id).cloned() else { return div().into_any_element() };
        if self.race_live(id) {
            self.ensure_race_tick(cx);
        }
        let now = dove_core::store::now_ms();
        let marks = r.marks();
        let busy = self.race.busy.get(id).copied();
        let base = r.base_branch.clone().unwrap_or_else(|| "a detached HEAD".into());
        let project = self.project(r.project).map(|p| p.name.clone()).unwrap_or_else(|| "Removed project".into());
        let subtitle = format!(
            "{} contenders · {project} · from {base} at {} · started {} ago",
            r.contenders.len(),
            &r.base[..r.base.len().min(7)],
            race::duration_label(now - r.created_ms)
        );

        let back = ctl::button(t, "race-back", "All races", Tone::Quiet)
            .on_click(cx.listener(|this, _, window, cx| this.open_race(None, window, cx)));
        let forget_arm = Arm::Forget(id.to_string());
        let forget_armed = self.race.armed(&forget_arm);
        let forget = {
            let id = id.to_string();
            ctl::button(
                t,
                "race-forget",
                if forget_armed { "Click again to remove" } else { "Remove from list" },
                if forget_armed { Tone::Danger } else { Tone::Quiet },
            )
            .tooltip(ui::tip("Forget this race. Its threads and branches stay."))
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.race_confirm(Arm::Forget(id.clone()), cx) {
                    this.forget_race(id.clone(), window, cx);
                }
            }))
        };
        let header = div().flex().items_center().gap(px(space::XS)).child(forget).child(back).into_any_element();

        let prompt = page::card(t).px(px(space::LG)).py(px(space::MD)).child(
            div()
                .text_size(ty::BODY)
                .line_height(ty::lh::BODY)
                .text_color(t.text)
                .line_clamp(6)
                .child(dove_core::strip_persona(&r.prompt).to_string()),
        );

        // What happened last, and the decision strip.
        let note = self.race.note.get(id).cloned();
        let decision = r.winner.map(|w| {
            let losers = r.losers().count();
            let who = r.contender(w).map(|c| c.label()).unwrap_or_default();
            let branch = r.contender(w).map(|c| c.branch.clone()).unwrap_or_default();
            let arm = Arm::Discard(id.to_string());
            let armed = self.race.armed(&arm);
            let id = id.to_string();
            div()
                .flex()
                .items_center()
                .gap(px(space::MD))
                .px(px(space::LG))
                .py(px(space::MD))
                .rounded(px(radius::CARD))
                .bg(t.success.opacity(0.08))
                .border_1()
                .border_color(t.success.opacity(0.25))
                .child(icon(icons::GIT_MERGE).size(px(16.0)).text_color(t.success))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(ty::BODY)
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .child(format!("Winner: {who}")),
                        )
                        .child(div().text_size(ty::META).text_color(t.text_muted).child(format!(
                            "{branch} was merged into {base}{}",
                            r.merged_ms.map(|m| format!(", {} ago", race::duration_label(now - m))).unwrap_or_default()
                        ))),
                )
                .when(losers > 0, |d| {
                    d.child(
                        ctl::button(
                            t,
                            "race-discard",
                            if armed {
                                format!("Remove {losers} thread{} for good?", if losers == 1 { "" } else { "s" })
                            } else {
                                format!("Discard others ({losers})")
                            },
                            Tone::Danger,
                        )
                        .tooltip(ui::tip(
                            "Deletes the other contenders' threads, worktrees and branches. Their figures stay here.",
                        ))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.race_confirm(Arm::Discard(id.clone()), cx) {
                                this.discard_race_losers(id.clone(), window, cx);
                            }
                        })),
                    )
                })
        });

        let rows = r.contenders.iter().enumerate().map(|(i, c)| {
            let status = self.race_status(c.task);
            let standing = c.standing(status);
            let checking = self.race.checking.contains(&c.task);
            let expanded = self.race.expanded.contains(&c.task);
            let task = c.task;
            let rid = id.to_string();

            let (state_label, state_color, glyph): (&str, Hsla, AnyElement) = match standing {
                Standing::Running => {
                    ("Working", t.busy, kit::spinner(("race-spin", i), 12.0, t.busy).into_any_element())
                }
                Standing::NeedsInput => ("Needs you", t.warning, ui::dot(t.warning, 8.0).into_any_element()),
                Standing::Done => {
                    ("Done", t.success, icon(icons::CHECK).size(px(12.0)).text_color(t.success).into_any_element())
                }
                Standing::Failed => {
                    ("Failed", t.danger, icon(icons::CLOSE).size(px(12.0)).text_color(t.danger).into_any_element())
                }
                Standing::Discarded => (
                    "Discarded",
                    t.text_faint,
                    icon(icons::TRASH_BIN_MINIMALISTIC).size(px(12.0)).text_color(t.text_faint).into_any_element(),
                ),
            };
            let working = matches!(standing, Standing::Running | Standing::NeedsInput);

            let metric = |label: &'static str, value: AnyElement, sub: Option<String>| {
                div()
                    .min_w(px(110.0))
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .child(div().text_size(ty::CAPTION).text_color(t.text_faint).child(label))
                    .child(value)
                    .when_some(sub, |d, s| d.child(div().text_size(ty::CAPTION).text_color(t.text_muted).child(s)))
            };
            let text = |s: String, color: Hsla| div().text_size(ty::BODY).text_color(color).child(s).into_any_element();

            let time = metric(
                "Time",
                text(
                    race::duration_label(c.elapsed_ms(now)),
                    if c.finished_ms.is_some() { t.text } else { t.text_muted },
                ),
                Some(if c.finished_ms.is_some() { "to first finish".into() } else { "so far".into() }),
            );
            let (money, tokens) = cost(c.usage);
            let spend = metric("Cost", text(money, t.text), Some(tokens));
            let changes = match c.diff {
                Some(d) => metric(
                    "Changes",
                    div()
                        .flex()
                        .gap(px(6.0))
                        .text_size(ty::BODY)
                        .child(div().text_color(t.diff_add).child(format!("+{}", d.adds)))
                        .child(div().text_color(t.diff_del).child(format!("−{}", d.dels)))
                        .into_any_element(),
                    Some(format!("{} file{}", d.files, if d.files == 1 { "" } else { "s" })),
                ),
                None => metric("Changes", text("—".into(), t.text_muted), Some("not measured yet".into())),
            };
            let check = match (&r.check, &c.check, checking) {
                (None, _, _) => None,
                (Some(_), _, true) => Some(metric(
                    "Check",
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(kit::spinner(("race-check", i), 11.0, t.busy))
                        .child(div().text_size(ty::BODY).text_color(t.text_muted).child("Running…"))
                        .into_any_element(),
                    None,
                )),
                (Some(_), Some(k), false) => Some(metric(
                    "Check",
                    text(
                        if k.passed {
                            "Passed".into()
                        } else if k.timed_out {
                            "Timed out".into()
                        } else {
                            match k.code {
                                Some(code) => format!("Failed (exit {code})"),
                                None => "Failed".into(),
                            }
                        },
                        if k.passed { t.success } else { t.danger },
                    ),
                    Some(format!("in {}", race::duration_label(k.duration_ms as i64))),
                )),
                (Some(_), None, false) => Some(metric(
                    "Check",
                    text("Not run".into(), t.text_muted),
                    Some(if working { "runs when it finishes".into() } else { "—".into() }),
                )),
            };

            // Actions.
            let alive = !c.discarded;
            let open = ctl::button_sm(t, SharedString::from(format!("race-open-{i}")), "Open thread", Tone::Quiet);
            let open =
                if alive && status.is_some() {
                    open.on_click(cx.listener(move |this, _, window, cx| {
                        this.navigate(crate::shell::Route::Thread(task), window, cx)
                    }))
                } else {
                    ctl::disabled(open, "Its thread was removed")
                };
            let diff = ctl::button_sm(t, SharedString::from(format!("race-diff-{i}")), "View diff", Tone::Quiet)
                .child(icon(icons::FILE_DIFF).size(px(12.0)));
            let diff = if alive && status.is_some() {
                diff.on_click(cx.listener(move |this, _, window, cx| this.race_view_diff(task, window, cx)))
            } else {
                ctl::disabled(diff, "Its worktree was removed")
            };
            let run_check = (r.check.is_some() && alive).then(|| {
                let b = ctl::button_sm(
                    t,
                    SharedString::from(format!("race-check-{i}")),
                    if c.check.is_some() { "Run check again" } else { "Run check" },
                    Tone::Quiet,
                );
                if checking {
                    ctl::disabled(b, "The check is running")
                } else {
                    let rid = rid.clone();
                    b.tooltip(ui::tip(format!("Runs `{}` in its worktree", r.check.clone().unwrap_or_default())))
                        .on_click(cx.listener(move |this, _, _, cx| this.run_race_check(rid.clone(), task, cx)))
                }
            });
            let pick = (r.winner.is_none() && alive).then(|| {
                let arm = Arm::Pick(rid.clone(), task);
                let armed = self.race.armed(&arm);
                let into = r.base_branch.clone().unwrap_or_default();
                let b = ctl::button_sm(
                    t,
                    SharedString::from(format!("race-pick-{i}")),
                    if busy.is_some() {
                        busy.unwrap_or_default().to_string()
                    } else if armed {
                        format!("Merge into {into}?")
                    } else {
                        "Pick winner".into()
                    },
                    if armed { Tone::Solid } else { Tone::Filled },
                )
                .child(icon(icons::GIT_MERGE).size(px(12.0)));
                if busy.is_some() {
                    ctl::disabled(b, "A merge is running")
                } else if r.base_branch.is_none() {
                    ctl::disabled(b, format!("The race started on a detached HEAD; merge {} by hand", c.branch))
                } else if working {
                    ctl::disabled(b, "Still working: wait for it to finish")
                } else {
                    let rid = rid.clone();
                    b.tooltip(ui::tip(format!(
                        "Commits anything it left uncommitted, then merges {} into {into} in your project folder",
                        c.branch
                    )))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.race_confirm(Arm::Pick(rid.clone(), task), cx) {
                            this.pick_race_winner(rid.clone(), task, cx);
                        }
                    }))
                }
            });
            let output_toggle = c.check.as_ref().filter(|k| !k.tail.is_empty()).map(|_| {
                ctl::button_sm(
                    t,
                    SharedString::from(format!("race-out-{i}")),
                    if expanded { "Hide output" } else { "Show output" },
                    Tone::Quiet,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.race.expanded.remove(&task) {
                        this.race.expanded.insert(task);
                    }
                    cx.notify();
                }))
            });
            let output = c.check.as_ref().filter(|k| expanded && !k.tail.is_empty()).map(|k| {
                div()
                    .mt(px(space::XS))
                    .px(px(space::MD))
                    .py(px(space::SM))
                    .rounded(px(radius::ROW))
                    .bg(t.code_wash)
                    .font_family(t.font_mono.clone())
                    .text_size(ty::CAPTION)
                    .line_height(ty::lh::META)
                    .text_color(t.code_text)
                    .flex()
                    .flex_col()
                    .children(k.tail.iter().map(|l| div().min_w_0().truncate().child(l.clone())))
            });

            let meta = {
                let mut parts = Vec::new();
                if let Some(m) = c.model.as_deref().filter(|m| !m.is_empty()) {
                    parts.push(m.to_string());
                }
                parts.push(c.host.label());
                parts.push(c.branch.clone());
                parts.join(" · ")
            };

            div()
                .px(px(space::LG))
                .py(px(space::MD))
                .flex()
                .flex_col()
                .gap(px(space::MD))
                .when(i > 0, |d| d.border_t_1().border_color(page::row_divider(t)))
                .when(c.discarded, |d| d.opacity(0.6))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::SM))
                        .child(icon(ui::harness_icon(c.harness)).size(px(16.0)).text_color(ui::harness_tint(
                            c.harness,
                            t,
                            t.text_muted,
                        )))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(ty::BODY)
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(t.text)
                                        .child(c.harness.label()),
                                )
                                .child(div().text_size(ty::META).text_color(t.text_muted).truncate().child(meta)),
                        )
                        .children(marks.get(i).into_iter().flatten().map(|m| {
                            let color = match m {
                                Mark::Winner => t.success,
                                Mark::Fastest | Mark::Cheapest => t.accent,
                            };
                            div()
                                .flex_none()
                                .px(px(7.0))
                                .py(px(1.0))
                                .rounded(px(radius::ROW))
                                .bg(color.opacity(0.12))
                                .child(page::badge_tone(m.label(), color))
                        }))
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(glyph)
                                .child(div().text_size(ty::SMALL).text_color(state_color).child(state_label)),
                        ),
                )
                .child(
                    div().flex().flex_wrap().gap(px(space::XL)).child(time).child(spend).child(changes).children(check),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(space::XS))
                        .children(output_toggle)
                        .child(div().flex_1())
                        .child(open)
                        .child(diff)
                        .children(run_check)
                        .children(pick),
                )
                .children(output)
        });

        let check_line = r
            .check
            .as_ref()
            .map(|cmd| format!("Check: `{cmd}`, run on this computer in each worktree when its contender finishes."))
            .unwrap_or_else(|| "No check command. Compare by diff, time and cost, or open each thread.".into());

        let body = div()
            .flex()
            .flex_col()
            .child(page::section_label(t, "Prompt"))
            .child(div().mt(px(space::SM)).child(prompt))
            .when_some(note, |d, (text, error)| {
                d.child(div().mt(px(space::MD)).child(if error {
                    page::error_strip(t, text)
                } else {
                    page::info_strip(t, text)
                }))
            })
            .when_some(decision, |d, el| d.child(div().mt(px(space::MD)).child(el)))
            .child(page::section(t, "Contenders", page::card(t).children(rows)))
            .child(
                div()
                    .mt(px(space::SM))
                    .px(px(page::LABEL_INSET))
                    .text_size(ty::META)
                    .text_color(t.text_faint)
                    .child(check_line),
            );

        page_with("race-compare", &r.title, &subtitle, t, Some(header), body)
    }
}
