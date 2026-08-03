//! Cron helpers for backup policy schedules (UTC).

use std::str::FromStr;

use chrono::{DateTime, Utc};
use cron::Schedule;

/// Normalize a user cron string to the form expected by the `cron` crate.
///
/// Accepts:
/// - 5 fields: `min hour dom month dow` (standard crontab) → seconds forced to `0`
/// - 6 fields: `sec min hour dom month dow`
/// - 7 fields: with optional year
pub fn normalize_cron_expression(expr: &str) -> Result<String, String> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Err("schedule must not be empty".to_string());
    }
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    match fields.len() {
        5 => Ok(format!("0 {}", fields.join(" "))),
        6 | 7 => Ok(fields.join(" ")),
        n => Err(format!(
            "schedule must have 5 or 6 fields (got {n}); example: '0 2 * * *' (daily 02:00 UTC)"
        )),
    }
}

pub fn parse_schedule(expr: &str) -> Result<Schedule, String> {
    let normalized = normalize_cron_expression(expr)?;
    Schedule::from_str(&normalized).map_err(|err| format!("invalid schedule '{expr}': {err}"))
}

/// Next fire time strictly after `after` (UTC).
pub fn next_run_after(expr: &str, after: DateTime<Utc>) -> Result<DateTime<Utc>, String> {
    let schedule = parse_schedule(expr)?;
    schedule
        .after(&after)
        .next()
        .ok_or_else(|| format!("schedule '{expr}' has no upcoming fire time"))
}

/// Validate that `expr` is a usable cron schedule.
pub fn validate_schedule(expr: &str) -> Result<(), String> {
    parse_schedule(expr).map(|_| ())
}

/// DNS-1123-safe run name: `{policy}-{YYYYMMDDHHMMSS}`, truncated to 63 chars.
pub fn backup_run_name(policy_name: &str, at: DateTime<Utc>) -> String {
    let stamp = at.format("%Y%m%d%H%M%S").to_string();
    let base = format!("{policy_name}-{stamp}");
    if base.len() <= 63 {
        return base;
    }
    let keep = 63usize.saturating_sub(1 + stamp.len());
    let prefix: String = policy_name.chars().take(keep).collect();
    format!("{prefix}-{stamp}")
}

/// Whether a scheduled tick is due.
///
/// - If `next_run_at` is set and `<= now` → due.
/// - If `next_run_at` is unset → not due (caller should seed `next_run_at`).
pub fn schedule_is_due(now: DateTime<Utc>, next_run_at: Option<DateTime<Utc>>) -> bool {
    match next_run_at {
        Some(next) => next <= now,
        None => false,
    }
}

/// Advance past every fire time `<= now` starting from `from` (exclusive via `next_run_after`).
/// Returns the last tick at or before `now`, or `None` if none.
pub fn last_tick_at_or_before(
    expr: &str,
    from: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, String> {
    let mut cursor = from;
    let mut last = None;
    // Bound iterations so a pathological schedule cannot hang reconcile.
    for _ in 0..10_000 {
        let next = next_run_after(expr, cursor)?;
        if next > now {
            break;
        }
        last = Some(next);
        cursor = next;
    }
    Ok(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn normalize_five_field_prepends_seconds() {
        assert_eq!(
            normalize_cron_expression("0 2 * * *").expect("ok"),
            "0 0 2 * * *"
        );
    }

    #[test]
    fn parse_daily_two_am() {
        parse_schedule("0 2 * * *").expect("valid");
    }

    #[test]
    fn reject_empty() {
        assert!(normalize_cron_expression("").is_err());
    }

    #[test]
    fn next_run_after_is_deterministic() {
        let after = Utc.with_ymd_and_hms(2026, 8, 3, 1, 0, 0).unwrap();
        let next = next_run_after("0 2 * * *", after).expect("next");
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 8, 3, 2, 0, 0).unwrap());
    }

    #[test]
    fn validate_rejects_garbage() {
        assert!(validate_schedule("not a cron").is_err());
    }

    #[test]
    fn schedule_due_when_next_passed() {
        let now = Utc.with_ymd_and_hms(2026, 8, 3, 3, 0, 0).unwrap();
        let next = Utc.with_ymd_and_hms(2026, 8, 3, 2, 0, 0).unwrap();
        assert!(schedule_is_due(now, Some(next)));
        assert!(!schedule_is_due(now, None));
    }

    #[test]
    fn last_tick_skips_missed_hours() {
        let from = Utc.with_ymd_and_hms(2026, 8, 3, 0, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 8, 3, 3, 30, 0).unwrap();
        let last = last_tick_at_or_before("0 * * * *", from, now)
            .expect("ok")
            .expect("tick");
        assert_eq!(last, Utc.with_ymd_and_hms(2026, 8, 3, 3, 0, 0).unwrap());
    }

    #[test]
    fn backup_run_name_truncates() {
        let long = "a".repeat(80);
        let at = Utc.with_ymd_and_hms(2026, 8, 3, 2, 0, 0).unwrap();
        let name = backup_run_name(&long, at);
        assert!(name.len() <= 63);
        assert!(name.ends_with("-20260803020000"));
    }
}
