//! Usage: tokens and cost by day, agent and project, with CSV export.

use std::collections::BTreeMap;

use dove_core::proto::Usage;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, div, px,
};

use super::page_with;
use crate::assets::icons;
use crate::design::ty;
use crate::kit;
use crate::shell::Shell;
use crate::theme::Theme;
use crate::ui;

fn cost(u: &Usage) -> String {
    match u.cost_usd {
        Some(c) => format!("${c:.2}"),
        None => "—".into(),
    }
}

impl Shell {
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
        let range = kit::segmented(
            "usage-range",
            &["7 days", "30 days", "90 days"],
            match days {
                7 => 0,
                30 => 1,
                _ => 2,
            },
            &t,
            {
                let weak = cx.weak_entity();
                move |i, _, cx| {
                    let _ = weak.update(cx, |s, cx| {
                        s.pages.usage_days = [7, 30, 90][i];
                        let engine = s.engine.clone();
                        s.pages.refresh_usage(&engine, cx);
                    });
                }
            },
        );
        let header = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(range)
            .child(
                ui::button_ghost("usage-csv", "Export CSV", &t)
                    .on_click(cx.listener(|this, _, _, cx| this.export_usage_csv(cx))),
            )
            .into_any_element();

        let stat = |label: &str, value: String| {
            div()
                .flex_1()
                .p(px(14.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .rounded(px(12.0))
                .border_1()
                .border_color(t.border)
                .child(div().text_size(ty::META).text_color(t.text_faint).child(label.to_string()))
                .child(div().text_size(ty::PAGE).font_weight(gpui::FontWeight::SEMIBOLD).child(value))
        };
        let stats = div()
            .flex()
            .gap(px(10.0))
            .child(stat("Tokens", ui::compact(total.total_tokens())))
            .child(stat("Output", ui::compact(total.output_tokens)))
            .child(stat("Cache reads", ui::compact(total.cache_read_tokens)))
            .child(stat("Cost (reported)", cost(&total)));

        let chart = div()
            .h(px(120.0))
            .p(px(12.0))
            .flex()
            .items_end()
            .gap(px(3.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(t.border)
            .children(by_day.iter().map(|(day, u)| {
                let h = (u.total_tokens() as f32 / max_day as f32 * 90.0).max(2.0);
                div()
                    .id(("usage-bar", *day as usize))
                    .flex_1()
                    .h(px(h))
                    .rounded_t(px(3.0))
                    .bg(t.accent.opacity(0.6))
                    .tooltip(ui::tip(format!(
                        "{} · {} tokens · {}",
                        chrono_date(*day),
                        ui::compact(u.total_tokens()),
                        cost(u)
                    )))
            }));

        let table = |title: &str, data: &BTreeMap<String, Usage>| {
            let mut v: Vec<_> = data.iter().collect();
            v.sort_by_key(|e| std::cmp::Reverse(e.1.total_tokens()));
            div()
                .flex_1()
                .flex()
                .flex_col()
                .rounded(px(12.0))
                .border_1()
                .border_color(t.border)
                .overflow_hidden()
                .child(
                    div()
                        .px(px(12.0))
                        .py(px(8.0))
                        .text_size(ty::META)
                        .text_color(t.text_faint)
                        .child(title.to_string()),
                )
                .children(v.into_iter().map(|(name, u)| {
                    div()
                        .h(px(34.0))
                        .px(px(12.0))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .border_t_1()
                        .border_color(t.border)
                        .text_size(ty::BODY)
                        .child(div().flex_1().truncate().child(name.clone()))
                        .child(div().text_color(t.text_muted).child(ui::compact(u.total_tokens())))
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
        let body = if rows.is_empty() {
            div().child(kit::empty_state(
                icons::CLOCK_CIRCLE,
                "No usage yet",
                "Token counts and costs that agents report show up here.",
                &t,
            ))
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(14.0))
                .child(stats)
                .child(chart)
                .child(
                    div()
                        .flex()
                        .gap(px(10.0))
                        .items_start()
                        .child(table("By agent", &by_agent))
                        .child(table("By project", &by_project)),
                )
                .when(!by_teammate.is_empty(), |d| {
                    d.child(div().flex().child(table("By teammate (all time)", &by_teammate)))
                })
        };
        page_with(
            "usage-page",
            "Usage",
            "Tokens and spend by day, agent and project. Counted on this device.",
            &t,
            Some(header),
            body.when_some(self.pages.usage_note.clone(), |d, n| {
                d.child(div().text_size(ty::SMALL).text_color(t.text_muted).child(n))
            })
            .child(
                div()
                    .text_size(ty::CAPTION)
                    .text_color(t.text_faint)
                    .child("Costs are what each agent reports; subscription plans often report none."),
            ),
        )
    }
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
    fn dates_from_millis() {
        assert_eq!(chrono_date(0), "1970-01-01");
        assert_eq!(chrono_date(1_791_763_200_000), "2026-10-12");
    }
}
