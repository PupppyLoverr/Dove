//! Automations: prompts that start threads by themselves.
//!
//! A trigger is a schedule (every N minutes, daily/weekly at a time, or a
//! five-field cron expression, all in local time) or an event (a new commit
//! on a branch; a pull request opened or a review requested, through `gh`).
//! The UI calls [`due`] about once a minute while Dove runs; each due
//! automation starts a normal thread tagged with the automation.

use chrono::{DateTime, Datelike, Local, NaiveTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    /// Every `minutes` minutes.
    Every { minutes: u32 },
    /// At `hour:minute` on the listed weekdays (0 = Monday); empty = daily.
    At { hour: u32, minute: u32, weekdays: Vec<u32> },
    /// `minute hour day-of-month month day-of-week` (Sunday = 0 or 7).
    Cron { expr: String },
    /// A new commit lands on `branch`.
    OnCommit { branch: String },
    /// A PR is opened, or a review is requested from you.
    OnPullRequest { review_requested: bool },
}

impl Trigger {
    pub fn parse(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// "Every 30 minutes", "Weekdays at 09:00", "Cron 0 9 * * 1-5", …
    pub fn describe(&self) -> String {
        match self {
            Self::Every { minutes } if minutes % 60 == 0 && *minutes >= 60 => {
                let h = minutes / 60;
                if h == 1 { "Every hour".into() } else { format!("Every {h} hours") }
            }
            Self::Every { minutes } => format!("Every {minutes} minutes"),
            Self::At { hour, minute, weekdays } => {
                let days = match weekdays.as_slice() {
                    [] => "Daily".to_string(),
                    [0, 1, 2, 3, 4] => "Weekdays".to_string(),
                    [5, 6] => "Weekends".to_string(),
                    list => list.iter().map(|d| WEEKDAYS[*d as usize % 7]).collect::<Vec<_>>().join(", "),
                };
                format!("{days} at {hour:02}:{minute:02}")
            }
            Self::Cron { expr } => format!("Cron {expr}"),
            Self::OnCommit { branch } => format!("On new commits to {branch}"),
            Self::OnPullRequest { review_requested: true } => "When a review is requested from you".into(),
            Self::OnPullRequest { review_requested: false } => "When a pull request is opened".into(),
        }
    }

    pub fn is_schedule(&self) -> bool {
        matches!(self, Self::Every { .. } | Self::At { .. } | Self::Cron { .. })
    }

    /// The first fire time strictly after `after`, for schedules.
    pub fn next_after(&self, after: DateTime<Local>) -> Option<DateTime<Local>> {
        match self {
            Self::Every { minutes } => Some(after + chrono::Duration::minutes((*minutes).max(1) as i64)),
            Self::At { hour, minute, weekdays } => {
                let time = NaiveTime::from_hms_opt(*hour % 24, *minute % 60, 0)?;
                (0..8).find_map(|d| {
                    let date = (after + chrono::Duration::days(d)).date_naive();
                    let at = Local.from_local_datetime(&date.and_time(time)).earliest()?;
                    let wd = date.weekday().num_days_from_monday();
                    (at > after && (weekdays.is_empty() || weekdays.contains(&wd))).then_some(at)
                })
            }
            Self::Cron { expr } => Cron::parse(expr)?.next_after(after),
            _ => None,
        }
    }
}

pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// A parsed five-field cron expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Cron {
    minutes: Vec<u32>,
    hours: Vec<u32>,
    days: Vec<u32>,
    months: Vec<u32>,
    /// 0 = Sunday … 6 = Saturday.
    weekdays: Vec<u32>,
    any_day: bool,
    any_weekday: bool,
}

impl Cron {
    pub fn parse(expr: &str) -> Option<Self> {
        let f: Vec<&str> = expr.split_whitespace().collect();
        if f.len() != 5 {
            return None;
        }
        let mut weekdays = field(f[4], 0, 7)?;
        for d in &mut weekdays {
            if *d == 7 {
                *d = 0;
            }
        }
        weekdays.sort_unstable();
        weekdays.dedup();
        Some(Self {
            minutes: field(f[0], 0, 59)?,
            hours: field(f[1], 0, 23)?,
            days: field(f[2], 1, 31)?,
            months: field(f[3], 1, 12)?,
            weekdays,
            any_day: f[2] == "*",
            any_weekday: f[4] == "*",
        })
    }

    fn matches(&self, t: &DateTime<Local>) -> bool {
        let day_ok = {
            let dom = self.days.contains(&t.day());
            let dow = self.weekdays.contains(&t.weekday().num_days_from_sunday());
            // Standard cron: when both are restricted, either may match.
            match (self.any_day, self.any_weekday) {
                (true, true) => true,
                (true, false) => dow,
                (false, true) => dom,
                (false, false) => dom || dow,
            }
        };
        day_ok
            && self.months.contains(&t.month())
            && self.hours.contains(&t.hour())
            && self.minutes.contains(&t.minute())
    }

    /// Walk forward minute by minute (bounded to a year and a day).
    pub fn next_after(&self, after: DateTime<Local>) -> Option<DateTime<Local>> {
        let mut t = after.with_second(0)?.with_nanosecond(0)? + chrono::Duration::minutes(1);
        for _ in 0..(366 * 24 * 60) {
            if self.matches(&t) {
                return Some(t);
            }
            // Skip whole hours that cannot match.
            if !self.hours.contains(&t.hour()) {
                t = (t + chrono::Duration::hours(1)).with_minute(0)?;
                continue;
            }
            t += chrono::Duration::minutes(1);
        }
        None
    }
}

/// `*`, `5`, `1-5`, `*/15`, `0-30/10`, `1,3,5`.
fn field(spec: &str, lo: u32, hi: u32) -> Option<Vec<u32>> {
    let mut out = Vec::new();
    for part in spec.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().ok().filter(|s| *s > 0)?),
            None => (part, 1),
        };
        let (a, b) = if range == "*" {
            (lo, hi)
        } else if let Some((a, b)) = range.split_once('-') {
            (a.parse().ok()?, b.parse().ok()?)
        } else {
            let v: u32 = range.parse().ok()?;
            (v, if part.contains('/') { hi } else { v })
        };
        if a < lo || b > hi || a > b {
            return None;
        }
        out.extend((a..=b).step_by(step as usize));
    }
    out.sort_unstable();
    out.dedup();
    Some(out)
}

/// Whether a schedule fired between its last run and `now`.
pub fn schedule_due(trigger: &Trigger, last_run_ms: Option<i64>, created_ms: i64, now: DateTime<Local>) -> bool {
    let since = last_run_ms.unwrap_or(created_ms);
    let Some(since) = Local.timestamp_millis_opt(since).single() else { return false };
    trigger.next_after(since).is_some_and(|next| next <= now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).single().unwrap()
    }

    #[test]
    fn cron_fields_parse() {
        assert_eq!(field("*/15", 0, 59).unwrap(), vec![0, 15, 30, 45]);
        assert_eq!(field("1-5", 0, 7).unwrap(), vec![1, 2, 3, 4, 5]);
        assert_eq!(field("1,3,5", 0, 7).unwrap(), vec![1, 3, 5]);
        assert!(field("70", 0, 59).is_none());
        assert!(Cron::parse("* * *").is_none());
    }

    #[test]
    fn cron_next_fire() {
        // 2026-10-12 is a Monday.
        let c = Cron::parse("0 9 * * 1-5").unwrap();
        assert_eq!(c.next_after(at(2026, 10, 12, 8, 30)), Some(at(2026, 10, 12, 9, 0)));
        assert_eq!(c.next_after(at(2026, 10, 12, 9, 0)), Some(at(2026, 10, 13, 9, 0)));
        // Friday evening → Monday morning.
        assert_eq!(c.next_after(at(2026, 10, 16, 18, 0)), Some(at(2026, 10, 19, 9, 0)));
        let every = Cron::parse("*/30 * * * *").unwrap();
        assert_eq!(every.next_after(at(2026, 10, 12, 8, 31)), Some(at(2026, 10, 12, 9, 0)));
    }

    #[test]
    fn presets_fire_on_their_days() {
        let weekdays = Trigger::At { hour: 9, minute: 0, weekdays: vec![0, 1, 2, 3, 4] };
        assert_eq!(weekdays.next_after(at(2026, 10, 17, 10, 0)), Some(at(2026, 10, 19, 9, 0)));
        let daily = Trigger::At { hour: 23, minute: 30, weekdays: vec![] };
        assert_eq!(daily.next_after(at(2026, 10, 12, 23, 30)), Some(at(2026, 10, 13, 23, 30)));
        assert_eq!(weekdays.describe(), "Weekdays at 09:00");
        assert_eq!(Trigger::Every { minutes: 120 }.describe(), "Every 2 hours");
    }

    #[test]
    fn due_only_after_the_next_fire() {
        let t = Trigger::Every { minutes: 60 };
        let base = at(2026, 10, 12, 8, 0).timestamp_millis();
        assert!(!schedule_due(&t, Some(base), base, at(2026, 10, 12, 8, 59)));
        assert!(schedule_due(&t, Some(base), base, at(2026, 10, 12, 9, 0)));
    }

    #[test]
    fn triggers_round_trip_as_json() {
        let t = Trigger::OnCommit { branch: "main".into() };
        assert_eq!(Trigger::parse(&t.to_json()), Some(t));
    }
}
