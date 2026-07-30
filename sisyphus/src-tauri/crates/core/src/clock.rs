//! 全系统唯一的"一天"定义（本地时区 + 可配置换日点）。
//!
//! 修复的问题：此前"今日目标 / 今日娱乐时长"用 `Utc::now().format("%Y-%m-%d")` 算日期，
//! 而规则的 `time_of_day` 与 `daily@HH:MM` 用 `Local` —— UTC+8 用户的日界落在**早上 8 点**：
//! 早 7 点设的目标记进昨天，8 点整目标突然切换，今日统计从 8 点起算。
//!
//! 现在"今天"只有一个定义：本地时区，换日点可配（`settings.day_boundary_hour`，默认 0）。
//! 换日点 > 0 时（如 4）凌晨仍算作前一天，适合夜猫子。
//!
//! 纯 chrono + rusqlite，无副作用，安卓可编。

use chrono::{DateTime, Datelike, Days, Local, NaiveDate, TimeZone, Timelike};
use rusqlite::Connection;

/// 默认换日点：本地 0 点（日历日）。
pub const DEFAULT_BOUNDARY_HOUR: u32 = 0;

/// 读换日点（本地小时 0–23）。未配置或非法值回落到 [`DEFAULT_BOUNDARY_HOUR`]。
pub fn boundary_hour(conn: &Connection) -> u32 {
    crate::settings::get(conn, "day_boundary_hour")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|h| *h <= 23)
        .unwrap_or(DEFAULT_BOUNDARY_HOUR)
}

fn local_at(ms: i64) -> DateTime<Local> {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(|| Local.timestamp_millis_opt(0).unwrap())
}

/// `ms` 属于哪个**逻辑日**（本地时区；换日点之前算前一天）。
pub fn logical_date_at(ms: i64, boundary_hour: u32) -> NaiveDate {
    let t = local_at(ms);
    let d = t.date_naive();
    if t.hour() < boundary_hour {
        d.checked_sub_days(Days::new(1)).unwrap_or(d)
    } else {
        d
    }
}

/// 本地日期 + 小时 → epoch ms（DST 空洞时回落到该日可用的最早时刻）。
fn local_date_hour_ms(d: NaiveDate, hour: u32) -> i64 {
    for h in [hour, 0, 1, 2, 3] {
        if let Some(naive) = d.and_hms_opt(h, 0, 0) {
            if let Some(t) = Local
                .from_local_datetime(&naive)
                .earliest()
                .or_else(|| Local.from_local_datetime(&naive).latest())
            {
                return t.timestamp_millis();
            }
        }
    }
    0
}

/// `ms` 所在逻辑日的日期串（`"2026-07-30"`）。
pub fn day_str_at(ms: i64, boundary_hour: u32) -> String {
    logical_date_at(ms, boundary_hour)
        .format("%Y-%m-%d")
        .to_string()
}

/// `ms` 所在逻辑日的起点（epoch ms）。
pub fn day_start_at(ms: i64, boundary_hour: u32) -> i64 {
    local_date_hour_ms(logical_date_at(ms, boundary_hour), boundary_hour)
}

/// `ms` 所在逻辑日的终点（= 次日起点，半开区间上界）。
pub fn day_end_at(ms: i64, boundary_hour: u32) -> i64 {
    let d = logical_date_at(ms, boundary_hour);
    let next = d.checked_add_days(Days::new(1)).unwrap_or(d);
    local_date_hour_ms(next, boundary_hour)
}

/// `ms` 所在逻辑周的起点（周一为一周之始）。
pub fn week_start_at(ms: i64, boundary_hour: u32) -> i64 {
    let d = logical_date_at(ms, boundary_hour);
    let back = d.weekday().num_days_from_monday() as u64;
    let monday = d.checked_sub_days(Days::new(back)).unwrap_or(d);
    local_date_hour_ms(monday, boundary_hour)
}

/// `ms` 所在逻辑月的起点。
pub fn month_start_at(ms: i64, boundary_hour: u32) -> i64 {
    let d = logical_date_at(ms, boundary_hour);
    let first = NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or(d);
    local_date_hour_ms(first, boundary_hour)
}

// ── 便捷封装（读 settings 的换日点）────────────────────────────────────────────

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 今天的日期串。**所有"今日"语义都必须经这里**，不要再直接 format Utc。
pub fn today_str(conn: &Connection) -> String {
    day_str_at(now_ms(), boundary_hour(conn))
}

/// 今天的起点（epoch ms）。
pub fn today_start_ms(conn: &Connection) -> i64 {
    day_start_at(now_ms(), boundary_hour(conn))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_day_respects_boundary_hour() {
        // 构造本地时间 2026-07-30 02:30 —— 换日点 0 时属于 30 日，换日点 4 时属于 29 日。
        let naive = NaiveDate::from_ymd_opt(2026, 7, 30)
            .unwrap()
            .and_hms_opt(2, 30, 0)
            .unwrap();
        let ms = Local
            .from_local_datetime(&naive)
            .earliest()
            .unwrap()
            .timestamp_millis();
        assert_eq!(day_str_at(ms, 0), "2026-07-30");
        assert_eq!(day_str_at(ms, 4), "2026-07-29");
    }

    #[test]
    fn day_bounds_are_half_open_and_contain_the_instant() {
        let ms = now_ms();
        for h in [0u32, 4] {
            let start = day_start_at(ms, h);
            let end = day_end_at(ms, h);
            assert!(start <= ms && ms < end, "boundary_hour={h}");
            // 一天长度在 23–25 小时之间（容纳 DST）。
            let hours = (end - start) / 3_600_000;
            assert!((23..=25).contains(&hours), "day span {hours}h");
        }
    }

    #[test]
    fn week_and_month_starts_are_not_after_day_start() {
        let ms = now_ms();
        assert!(week_start_at(ms, 0) <= day_start_at(ms, 0));
        assert!(month_start_at(ms, 0) <= day_start_at(ms, 0));
    }
}
