//! Usage: tokens and cost by day, agent and project, with CSV export.

use std::collections::BTreeMap;

use dove_core::proto::{HarnessKind, RateWindow, Usage};
use dove_core::quota::{Provider, Quota};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, div, px,
};

use super::page_with;
use crate::assets::icons;
use crate::design::{space, ty};
use crate::kit::controls::{self as ctl, Segment, Tone};
use crate::kit::page;
use crate::shell::Shell;
use crate::theme::Theme;
use crate::ui;

fn cost(u: &Usage) -> String {
    match u.cost_usd {
        Some(c) => format!("${c:.2}"),
        None => "—".into(),
    }
}

/// "in 3h 12m" until unix second `reset`, from `now` (unix seconds).
fn resets_in(reset: i64, now: i64) -> String {
    let secs = (reset - now).max(0);
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("in {}m", secs / 60),
        3600..=86_399 => format!("in {}h {}m", secs / 3600, (secs % 3600) / 60),
        _ => format!("in {}d {}h", secs / 86_400, (secs % 86_400) / 3600),
    }
}

/// "just now", "4 min ago".
fn age(ms: i64, now_ms: i64) -> String {
    match (now_ms - ms).max(0) / 60_000 {
        0 => "just now".into(),
        m => format!("{m} min ago"),
    }
}

/// One plan window: its label, a bar, the percent used and when it resets.
fn limit_meter(w: &RateWindow, now: i64, t: &Theme) -> gpui::Div {
    let used = w.used_percent.unwrap_or(0.0).clamp(0.0, 100.0);
    let fraction = (used / 100.0) as f32;
    let color = if w.blocked { t.danger } else { crate::kit::ring_color(fraction, t) };
    let reset = match (w.blocked, w.resets_at) {
        (true, Some(r)) => format!("Blocked · resets {}", resets_in(r, now)),
        (true, None) => "Blocked".into(),
        (false, Some(r)) => format!("Resets {}", resets_in(r, now)),
        (false, None) => String::new(),
    };
    div()
        .flex()
        .items_center()
        .gap(px(space::MD))
        .min_h(px(22.0))
        .child(div().w(px(120.0)).flex_none().truncate().text_size(ty::META).text_color(t.text_muted).child(w.label()))
        .child(
            div()
                .flex_1()
                .min_w(px(60.0))
                .h(px(4.0))
                .rounded_full()
                .overflow_hidden()
                .bg(t.wash(0.10))
                .when(used > 0.0, |d| {
                    d.child(div().h_full().w(gpui::relative(fraction.max(0.02))).rounded_full().bg(color.opacity(0.9)))
                }),
        )
        .child(
            div()
                .w(px(44.0))
                .flex_none()
                .text_right()
                .text_size(ty::META)
                .text_color(color)
                .child(w.used_percent.map(|p| format!("{p:.0}%")).unwrap_or_else(|| "—".into())),
        )
        .child(
            div()
                .w(px(150.0))
                .flex_none()
                .truncate()
                .text_size(ty::META)
                .text_color(if w.blocked { t.danger } else { t.text_faint })
                .child(reset),
        )
}

/// One provider's row: who, which plan, and its meters (or why there are none).
fn quota_row(q: &Quota, first: bool, now: i64, t: &Theme) -> gpui::Div {
    let kind = match q.provider {
        Provider::Claude => HarnessKind::ClaudeCode,
        Provider::Codex => HarnessKind::Codex,
    };
    let who = div()
        .w(px(150.0))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(space::SM))
        .child(page::row_tile(t, ui::harness_icon(kind), Some(ui::harness_tint(kind, t, t.text_muted))))
        .child(
            div()
                .text_size(ty::BODY)
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(t.text)
                .child(q.provider.label()),
        )
        .children(q.plan.clone().map(|p| page::badge(t, p)));
    let detail = match q.message() {
        Some(m) => div().text_size(ty::META).text_color(t.text_faint).child(m),
        None => div()
            .flex()
            .flex_col()
            .gap(px(space::XS))
            .children(q.windows.iter().map(|w| limit_meter(w, now, t)))
            .children(q.notes.iter().map(|n| div().text_size(ty::META).text_color(t.text_faint).child(n.clone()))),
    };
    page::row(t, first).items_start().child(who.min_h(px(22.0))).child(div().flex_1().min_w(px(280.0)).child(detail))
}

impl Shell {
    /// What is left of the Claude and Codex plans, read from their CLIs'
    /// own sign-ins (`dove_core::quota`).
    fn render_plan_limits(&self, t: &Theme) -> gpui::Div {
        let now_ms = dove_core::store::now_ms();
        let card = match &self.pages.quota {
            None => page::card(t).child(div().p(px(space::LG)).child(page::skeleton_lines(t, 3))),
            Some(list) => {
                page::card(t).children(list.iter().enumerate().map(|(i, q)| quota_row(q, i == 0, now_ms / 1000, t)))
            }
        };
        let updated = self.pages.quota.as_ref().and_then(|l| l.iter().map(|q| q.fetched_at).min());
        div()
            .mb(px(page::SECTION_GAP - space::MD))
            .flex()
            .flex_col()
            .gap(px(space::SM))
            .child(page::section_label(t, "Plan limits"))
            .child(card)
            .child(div().px(px(page::LABEL_INSET)).text_size(ty::META).text_color(t.text_faint).child(
                match (updated, self.pages.quota_loading) {
                    (_, true) if self.pages.quota.is_none() => "Reading your plan limits…".to_string(),
                    (Some(at), _) => format!(
                        "Read with Claude Code's and Codex's own sign-ins on this device · updated {}",
                        age(at, now_ms)
                    ),
                    (None, _) => String::new(),
                },
            ))
    }

    fn export_usage_csv(&mut self, cx: &mut Context<Self>) {
        let rows = self.pages.usage_rows.clone();
        let names: std::collections::HashMap<_, _> = self.projects.iter().map(|p| (p.id, p.name.clone())).collect();
        let dir = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ".".into());
        let pick = cx.prompt_for_new_path(&dir, Some("dove-usage.csv"));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = pick.await else { return };
            let mut csv = String::from("date,agent,project,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,cost_usd,estimated\n");
            for r in rows.iter() {
                let date = chrono_date(r.day);
                let project = names.get(&r.project_id).cloned().unwrap_or_default().replace(',', " ");
                let u = &r.usage;
                csv.push_str(&format!(
                    "{date},{},{project},{},{},{},{},{},{}\n",
                    r.harness,
                    u.input_tokens,
                    u.output_tokens,
                    u.cache_read_tokens,
                    u.cache_write_tokens,
                    u.cost_usd.map(|c| format!("{c:.4}")).unwrap_or_default(),
                    u.estimated
                ));
            }
            let result = std::fs::write(&path, csv);
            let _ = this.update(cx, |s, cx| {
                s.pages.usage_note = Some(match result {
                    Ok(()) => format!("Saved {}", path.display()),
                    Err(e) => format!("Could not save: {e}"),
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub fn render_usage_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = Theme::of(cx).clone();
        let rows = self.pages.usage_rows.clone();
        let mut total = Usage::default();
        let mut by_day: BTreeMap<i64, Usage> = BTreeMap::new();
        let mut by_agent: BTreeMap<String, Usage> = BTreeMap::new();
        let mut by_project: BTreeMap<String, Usage> = BTreeMap::new();
        for r in rows.iter() {
            total += r.usage;
            *by_day.entry(r.day).or_default() += r.usage;
            let agent = dove_core::proto::HarnessKind::from_id(&r.harness)
                .map(|k| k.label().to_string())
                .unwrap_or_else(|| r.harness.clone());
            *by_agent.entry(agent).or_default() += r.usage;
            let project =
                self.project(r.project_id).map(|p| p.name.clone()).unwrap_or_else(|| "Removed project".into());
            *by_project.entry(project).or_default() += r.usage;
        }
        let max_day = by_day.values().map(|u| u.total_tokens()).max().unwrap_or(1).max(1);
        let days = self.pages.usage_days;
        let range = ctl::segmented(
            &t,
            "usage-range",
            vec![Segment::new("7 days"), Segment::new("30 days"), Segment::new("90 days")],
            match days {
                7 => 0,
                30 => 1,
                _ => 2,
            },
            {
                let weak = cx.weak_entity();
                move |i, _, cx| {
                    let _ = weak.update(cx, |s, cx| {
                        s.pages.usage_days = [7, 30, 90][i.min(2)];
                        let engine = s.engine.clone();
                        s.pages.refresh_usage(&engine, cx);
                    });
                }
            },
        );
        let header = div()
            .flex()
            .items_center()
            .gap(px(space::SM))
            .child(range)
            .child(
                ctl::button(&t, "usage-csv", "Export CSV", Tone::Outlined)
                    .on_click(cx.listener(|this, _, _, cx| this.export_usage_csv(cx))),
            )
            .into_any_element();

        let stats = div()
            .flex()
            .flex_wrap()
            .gap(px(space::SM))
            .child(page::stat(&t, "Tokens", ui::compact(total.total_tokens()), None))
            .child(page::stat(&t, "Output", ui::compact(total.output_tokens), None))
            .child(page::stat(&t, "Cache reads", ui::compact(total.cache_read_tokens), None))
            .child(page::stat(&t, "Cost (reported)", cost(&total), None));

        // One slot per day of the range, so a day or two of data reads as
        // a day or two (not one bar across the card); empty days show a stub.
        let slots = day_slots(dove_core::store::now_ms(), days);
        let active_days = by_day.values().filter(|u| u.total_tokens() > 0).count();
        let chart = page::card(&t)
            .child(
                div()
                    .h(px(132.0))
                    .px(px(space::LG))
                    .pt(px(space::LG))
                    .pb(px(space::SM))
                    .flex()
                    .items_end()
                    .justify_between()
                    .gap(px(if days > 30 { 1.0 } else { 3.0 }))
                    .children(slots.iter().map(|day| {
                        let u = by_day.get(day).copied().unwrap_or_default();
                        let used = u.total_tokens() > 0;
                        let h = if used { (u.total_tokens() as f32 / max_day as f32 * 96.0).max(3.0) } else { 2.0 };
                        div()
                            .id(("usage-bar", *day as usize))
                            .flex_1()
                            .max_w(px(28.0))
                            .h(px(h))
                            .rounded_t(px(3.0))
                            .bg(if used { t.accent.opacity(0.6) } else { t.wash(0.08) })
                            .when(used, |d| d.hover(|s| s.bg(t.accent)))
                            .tooltip(ui::tip(if used {
                                format!(
                                    "{} · {} tokens · {}",
                                    chrono_date(*day),
                                    ui::compact(u.total_tokens()),
                                    cost(&u)
                                )
                            } else {
                                format!("{} · no usage", chrono_date(*day))
                            }))
                    })),
            )
            .child(
                div()
                    .px(px(space::LG))
                    .pb(px(space::MD))
                    .flex()
                    .justify_between()
                    .text_size(ty::MICRO)
                    .text_color(t.text_faint)
                    .child(slots.first().map(|d| chrono_date(*d)).unwrap_or_default())
                    .when(active_days < 3, |d| {
                        d.child(format!(
                            "{active_days} {} with usage so far: the chart fills in as you work",
                            if active_days == 1 { "day" } else { "days" }
                        ))
                    })
                    .child("Today"),
            );

        let table = |data: &BTreeMap<String, Usage>| {
            let mut v: Vec<_> = data.iter().collect();
            v.sort_by_key(|e| std::cmp::Reverse(e.1.total_tokens()));
            page::card(&t)
                .child(
                    page::table_head(&t)
                        .child(div().flex_1().child("Name"))
                        .child(div().w(px(80.0)).text_right().child("Tokens"))
                        .child(div().w(px(70.0)).text_right().child("Cost")),
                )
                .children(v.into_iter().map(|(name, u)| {
                    page::table_row(&t, false)
                        .text_size(ty::BODY)
                        .child(div().flex_1().min_w_0().truncate().text_color(t.text).child(name.clone()))
                        .child(
                            div()
                                .w(px(80.0))
                                .text_right()
                                .text_color(t.text_muted)
                                .child(ui::compact(u.total_tokens())),
                        )
                        .child(div().w(px(70.0)).text_right().text_color(t.text_faint).child(cost(u)))
                }))
        };
        // What each teammate's threads have used, all time.
        let mut by_teammate: BTreeMap<String, Usage> = BTreeMap::new();
        for p in &self.personas {
            let mut sum = Usage::default();
            for (task, pid) in &self.persona_of {
                if pid == &p.id {
                    sum += self.engine.store.task_usage(*task).unwrap_or_default();
                }
            }
            if sum.total_tokens() > 0 {
                by_teammate.insert(p.name.clone(), sum);
            }
        }
        let half = |label: &'static str, card: gpui::Div| {
            div()
                .flex_1()
                .min_w(px(300.0))
                .flex()
                .flex_col()
                .gap(px(space::SM))
                .child(page::section_label(&t, label))
                .child(card)
        };
        let body = if rows.is_empty() {
            page::card(&t).child(page::empty_state(
                &t,
                icons::CLOCK_CIRCLE,
                "No usage yet",
                "Token counts and costs that agents report show up here.",
                None,
            ))
        } else {
            div()
                .flex()
                .flex_col()
                .child(stats)
                .child(page::section(&t, "By day", chart))
                .child(
                    div()
                        .mt(px(page::SECTION_GAP))
                        .flex()
                        .flex_wrap()
                        .gap(px(space::LG))
                        .items_start()
                        .child(half("By agent", table(&by_agent)))
                        .child(half("By project", table(&by_project))),
                )
                .when(!by_teammate.is_empty(), |d| {
                    d.child(page::section(&t, "By teammate (all time)", table(&by_teammate)))
                })
        };
        let note =
            self.pages.usage_note.clone().map(|n| {
                if n.starts_with("Could not save") { page::error_strip(&t, n) } else { page::info_strip(&t, n) }
            });
        page_with(
            "usage-page",
            "Usage",
            "Tokens and spend by day, agent and project. Counted on this device.",
            &t,
            Some(header),
            div()
                .flex()
                .flex_col()
                .gap(px(space::MD))
                .children(note)
                .child(self.render_plan_limits(&t))
                .child(body)
                .child(
                    div()
                        .mt(px(space::SM))
                        .px(px(page::LABEL_INSET))
                        .text_size(ty::META)
                        .text_color(t.text_faint)
                        .child("Costs are what each agent reports; subscription plans often report none."),
                ),
        )
    }
}

/// The UTC midnights of the last `days` days, oldest first, ending today.
pub fn day_slots(now_ms: i64, days: u32) -> Vec<i64> {
    let today = now_ms.div_euclid(86_400_000) * 86_400_000;
    (0..days as i64).rev().map(|i| today - i * 86_400_000).collect()
}

/// "2026-10-12" for a UTC-midnight timestamp.
pub fn chrono_date(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_and_age_phrases() {
        assert_eq!(resets_in(100, 130), "now");
        assert_eq!(resets_in(4_000, 100), "in 1h 5m");
        assert_eq!(resets_in(100 + 2 * 86_400 + 3 * 3600, 100), "in 2d 3h");
        assert_eq!(age(0, 30_000), "just now");
        assert_eq!(age(0, 5 * 60_000), "5 min ago");
    }

    #[test]
    fn dates_from_millis() {
        assert_eq!(chrono_date(0), "1970-01-01");
        assert_eq!(chrono_date(1_791_763_200_000), "2026-10-12");
    }

    #[test]
    fn a_range_has_one_slot_per_day_ending_today() {
        let noon = 1_791_763_200_000 + 12 * 3_600_000;
        let slots = day_slots(noon, 7);
        assert_eq!(slots.len(), 7);
        assert_eq!(chrono_date(*slots.last().unwrap()), "2026-10-12");
        assert_eq!(chrono_date(slots[0]), "2026-10-06");
    }
}
