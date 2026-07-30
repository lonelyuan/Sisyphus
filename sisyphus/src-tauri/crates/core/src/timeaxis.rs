//! 时间轴几何：刻度尺与折叠网格。
//!
//! **为什么这在 Core 而不在前端**：刻度与折叠的边界必须是*逻辑*日/周/月
//! （本地时区 + 可配置换日点，见 [`crate::clock`]）。前端只有 `Date`，
//! 它算出来的"日界"是 UTC 午夜或系统时区午夜，和 `time_rollups` 的桶对不上——
//! 这在线性视图里表现为网格线偏移（UTC+8 下偏 8 小时），
//! 而在折叠视图里会让**每一行的起点都是错的**，行与行之间不再可比。
//!
//! 另外，月/年不是固定毫秒数：`30 * DAY` / `365 * DAY` 在长跨度下会持续漂移，
//! DST 也会让某一天变成 23 或 25 小时。所有对齐都走 `clock`，不做固定倍数算术。
//!
//! 纯 chrono + serde，无副作用，安卓可编。

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Timelike};
use serde::Serialize;

use crate::clock;

const MINUTE: i64 = 60_000;
const HOUR: i64 = 3_600_000;
const DAY: i64 = 86_400_000;

/// 一条刻度。`tier` 0 = 主刻度（带标签），1 = 次刻度（只有短线）。
#[derive(Debug, Clone, Serialize)]
pub struct Tick {
    pub ms: i64,
    pub label: String,
    pub tier: u8,
    /// 是否恰好是逻辑日起点。前端据此加重强调；折叠时它就是行边界。
    pub day_start: bool,
}

/// 刻度单位。分钟/小时带步长（5 分钟、3 小时……），日以上是日历单位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Minute(i64),
    Hour(i64),
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl Unit {
    /// 名义长度，只用于**挑单位**，不用于定位（定位一律走 `clock`）。
    fn nominal_ms(self) -> i64 {
        match self {
            Unit::Minute(n) => n * MINUTE,
            Unit::Hour(n) => n * HOUR,
            Unit::Day => DAY,
            Unit::Week => 7 * DAY,
            Unit::Month => 2_629_746_000,
            Unit::Quarter => 7_889_238_000,
            Unit::Year => 31_557_600_000,
        }
    }

    pub fn as_str(self) -> String {
        match self {
            Unit::Minute(n) => format!("{n}min"),
            Unit::Hour(n) => format!("{n}h"),
            Unit::Day => "day".into(),
            Unit::Week => "week".into(),
            Unit::Month => "month".into(),
            Unit::Quarter => "quarter".into(),
            Unit::Year => "year".into(),
        }
    }

    /// 把 `ms` 对齐到所在单位的起点。
    ///
    /// 分钟/小时以**逻辑日起点**为原点累加，而不是以 epoch 为原点——
    /// 换日点为 4 时，小时刻度落在 4:00 / 5:00…，日界永远是一条刻度。
    fn align(self, ms: i64, boundary_hour: u32) -> i64 {
        match self {
            Unit::Minute(_) | Unit::Hour(_) => {
                let day = clock::day_start_at(ms, boundary_hour);
                let step = self.nominal_ms();
                day + (ms - day).div_euclid(step) * step
            }
            Unit::Day => clock::day_start_at(ms, boundary_hour),
            Unit::Week => clock::week_start_at(ms, boundary_hour),
            Unit::Month => clock::month_start_at(ms, boundary_hour),
            Unit::Quarter => clock::quarter_start_at(ms, boundary_hour),
            Unit::Year => clock::year_start_at(ms, boundary_hour),
        }
    }

    /// 下一个刻度。`ms` 应当已对齐。
    ///
    /// 分钟/小时在跨过日界时**吸附到日界**：DST 的 23/25 小时日不会让后续刻度整体漂移。
    fn next(self, ms: i64, boundary_hour: u32) -> i64 {
        match self {
            Unit::Minute(_) | Unit::Hour(_) => {
                let candidate = ms + self.nominal_ms();
                let day_end = clock::day_end_at(ms, boundary_hour);
                if candidate >= day_end {
                    day_end
                } else {
                    candidate
                }
            }
            Unit::Day => clock::day_end_at(ms, boundary_hour),
            Unit::Week => clock::week_end_at(ms, boundary_hour),
            Unit::Month => clock::month_end_at(ms, boundary_hour),
            Unit::Quarter => clock::quarter_end_at(ms, boundary_hour),
            Unit::Year => clock::year_end_at(ms, boundary_hour),
        }
    }

    fn label(self, ms: i64, boundary_hour: u32) -> String {
        let date = clock::logical_date_at(ms, boundary_hour);
        match self {
            Unit::Minute(_) | Unit::Hour(_) => {
                if ms == clock::day_start_at(ms, boundary_hour) {
                    format!("{}/{}", date.month(), date.day())
                } else {
                    let t = clock::local_at(ms);
                    format!("{:02}:{:02}", t.hour(), t.minute())
                }
            }
            Unit::Day | Unit::Week => format!("{}/{}", date.month(), date.day()),
            Unit::Month => {
                if date.month() == 1 {
                    format!("{}", date.year())
                } else {
                    format!("{}月", date.month())
                }
            }
            Unit::Quarter => format!("{}Q{}", date.year() % 100, (date.month() - 1) / 3 + 1),
            Unit::Year => format!("{}", date.year()),
        }
    }
}

/// 主刻度候选，按名义长度升序。
const CANDIDATES: [Unit; 13] = [
    Unit::Minute(1),
    Unit::Minute(5),
    Unit::Minute(15),
    Unit::Minute(30),
    Unit::Hour(1),
    Unit::Hour(3),
    Unit::Hour(6),
    Unit::Hour(12),
    Unit::Day,
    Unit::Week,
    Unit::Month,
    Unit::Quarter,
    Unit::Year,
];

/// 每个主单位对应的次刻度（DAW 里的 bars/beats 两级）。
fn minor_for(major: Unit) -> Option<Unit> {
    Some(match major {
        Unit::Minute(1) => return None,
        Unit::Minute(5) => Unit::Minute(1),
        Unit::Minute(15) => Unit::Minute(5),
        Unit::Minute(30) => Unit::Minute(10),
        Unit::Minute(_) => Unit::Minute(5),
        Unit::Hour(1) => Unit::Minute(15),
        Unit::Hour(3) => Unit::Hour(1),
        Unit::Hour(6) => Unit::Hour(1),
        Unit::Hour(12) => Unit::Hour(3),
        Unit::Hour(_) => Unit::Hour(1),
        Unit::Day => Unit::Hour(6),
        Unit::Week => Unit::Day,
        Unit::Month => Unit::Week,
        Unit::Quarter => Unit::Month,
        Unit::Year => Unit::Month,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct TickSet {
    pub unit: String,
    pub minor_unit: String,
    pub ticks: Vec<Tick>,
}

const MAX_MINOR: usize = 420;
const MAX_TICKS: usize = 900;

/// 生成可见窗口的刻度。`target_major` 是期望的主刻度数量（约 8–12 读起来最舒服）。
///
/// 两端各多给一个刻度，前端才能在边缘画出完整标签。
pub fn ticks(start_ms: i64, end_ms: i64, boundary_hour: u32, target_major: i64) -> TickSet {
    let span = (end_ms - start_ms).max(1);
    let target = target_major.max(2);
    let major = CANDIDATES
        .iter()
        .copied()
        .find(|unit| span / unit.nominal_ms() <= target)
        .unwrap_or(Unit::Year);

    let mut out: Vec<Tick> = Vec::new();
    let mut major_set: HashSet<i64> = HashSet::new();
    let mut cursor = major.align(start_ms, boundary_hour);
    for _ in 0..4_000 {
        out.push(Tick {
            ms: cursor,
            label: major.label(cursor, boundary_hour),
            tier: 0,
            day_start: cursor == clock::day_start_at(cursor, boundary_hour),
        });
        major_set.insert(cursor);
        if cursor > end_ms {
            break;
        }
        let next = major.next(cursor, boundary_hour);
        if next <= cursor {
            break;
        }
        cursor = next;
    }

    let minor_unit = minor_for(major);
    if let Some(minor) = minor_unit {
        let mut minors: Vec<Tick> = Vec::new();
        let mut cursor = minor.align(start_ms, boundary_hour);
        for _ in 0..4_000 {
            if !major_set.contains(&cursor) {
                minors.push(Tick {
                    ms: cursor,
                    label: String::new(),
                    tier: 1,
                    day_start: cursor == clock::day_start_at(cursor, boundary_hour),
                });
            }
            if cursor > end_ms {
                break;
            }
            let next = minor.next(cursor, boundary_hour);
            if next <= cursor {
                break;
            }
            cursor = next;
        }
        // 次刻度太密就整档丢掉，而不是画成一片灰。
        if minors.len() <= MAX_MINOR {
            out.extend(minors);
        }
    }

    out.sort_by_key(|t| t.ms);
    out.truncate(MAX_TICKS);
    TickSet {
        unit: major.as_str(),
        minor_unit: minor_unit.map(|u| u.as_str()).unwrap_or_default(),
        ticks: out,
    }
}

// ── 折叠（相位）网格 ──────────────────────────────────────────────────────────

/// 折叠档位。线性轴按周期取模后，一个周期占一行。
///
/// `Day` 就是时间生物学里的 actogram（作息栅格图）：行 = 日，横轴 = 日内时刻。
/// `Week` 的 7 列形态即传统日历。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fold {
    None,
    Day,
    Week,
    Month,
    Year,
}

impl Fold {
    /// 从档位名解析。刻意不实现 `FromStr`：非法值回落到 `None`（线性）而不是报错，
    /// 前端传了未知档位应当退化成线性视图，而不是让整个查询失败。
    pub fn parse(value: &str) -> Self {
        match value {
            "day" => Fold::Day,
            "week" => Fold::Week,
            "month" => Fold::Month,
            "year" => Fold::Year,
            _ => Fold::None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Fold::None => "none",
            Fold::Day => "day",
            Fold::Week => "week",
            Fold::Month => "month",
            Fold::Year => "year",
        }
    }

    /// 一行的列数。列宽固定，短月/短年右侧留空——这样列在行之间可比。
    pub fn cols(self) -> i32 {
        match self {
            Fold::None => 0,
            Fold::Day => 24,
            Fold::Week => 7,
            Fold::Month => 31,
            Fold::Year => 366,
        }
    }

    /// 列的单位：折叠成日时列是小时，其余都是日。
    pub fn col_unit(self) -> &'static str {
        match self {
            Fold::None => "none",
            Fold::Day => "hour",
            _ => "day",
        }
    }

    fn row_start(self, ms: i64, boundary_hour: u32) -> i64 {
        match self {
            Fold::None => ms,
            Fold::Day => clock::day_start_at(ms, boundary_hour),
            Fold::Week => clock::week_start_at(ms, boundary_hour),
            Fold::Month => clock::month_start_at(ms, boundary_hour),
            Fold::Year => clock::year_start_at(ms, boundary_hour),
        }
    }

    fn row_end(self, ms: i64, boundary_hour: u32) -> i64 {
        match self {
            Fold::None => ms,
            Fold::Day => clock::day_end_at(ms, boundary_hour),
            Fold::Week => clock::week_end_at(ms, boundary_hour),
            Fold::Month => clock::month_end_at(ms, boundary_hour),
            Fold::Year => clock::year_end_at(ms, boundary_hour),
        }
    }

    fn row_label(self, ms: i64, boundary_hour: u32) -> (String, String) {
        let date = clock::logical_date_at(ms, boundary_hour);
        const WEEKDAY: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];
        match self {
            Fold::None => (String::new(), String::new()),
            Fold::Day => (
                format!("{}/{}", date.month(), date.day()),
                WEEKDAY[clock::logical_weekday(ms, boundary_hour) as usize].to_string(),
            ),
            Fold::Week => (
                format!("{}/{}", date.month(), date.day()),
                format!("W{}", date.iso_week().week()),
            ),
            Fold::Month => (
                format!("{}月", date.month()),
                if date.month() == 1 {
                    format!("{}", date.year())
                } else {
                    String::new()
                },
            ),
            Fold::Year => (format!("{}", date.year()), String::new()),
        }
    }
}

/// 折叠后的一行（一个周期）。
#[derive(Debug, Clone, Serialize)]
pub struct FoldRow {
    pub index: i32,
    pub start_ms: i64,
    pub end_ms: i64,
    pub label: String,
    pub sub_label: String,
}

/// 折叠网格。前端只按 `row`/`col` 落位，不做任何时区算术。
#[derive(Debug, Clone, Serialize)]
pub struct FoldGrid {
    pub fold: String,
    pub cols: i32,
    pub col_unit: String,
    pub rows: Vec<FoldRow>,
    /// 行数超过 `max_rows` 时截断（防止年尺度折叠成日时行数爆掉）。
    pub truncated: bool,
}

/// 构造覆盖 `[start_ms, end_ms)` 的折叠网格。行边界一律是逻辑周期边界。
pub fn grid(
    fold: Fold,
    start_ms: i64,
    end_ms: i64,
    boundary_hour: u32,
    max_rows: usize,
) -> FoldGrid {
    let mut rows: Vec<FoldRow> = Vec::new();
    let mut truncated = false;
    if fold != Fold::None && end_ms > start_ms {
        let mut cursor = fold.row_start(start_ms, boundary_hour);
        let mut index = 0;
        while cursor < end_ms {
            if rows.len() >= max_rows {
                truncated = true;
                break;
            }
            let row_end = fold.row_end(cursor, boundary_hour);
            if row_end <= cursor {
                break;
            }
            let (label, sub_label) = fold.row_label(cursor, boundary_hour);
            rows.push(FoldRow {
                index,
                start_ms: cursor,
                end_ms: row_end,
                label,
                sub_label,
            });
            index += 1;
            cursor = row_end;
        }
    }
    FoldGrid {
        fold: fold.as_str().to_string(),
        cols: fold.cols(),
        col_unit: fold.col_unit().to_string(),
        rows,
        truncated,
    }
}

/// 逻辑日 → (行号, 列号) 映射。
///
/// 折叠视图里所有"日单元格"都用它定位；`Fold::Day` 时列号恒为 0，
/// 小时单元格的列号由小时序号直接给出。
pub fn day_coords(
    fold: Fold,
    rows: &[FoldRow],
    boundary_hour: u32,
) -> HashMap<i64, (i32, i32)> {
    let mut out = HashMap::new();
    for row in rows {
        let mut day = clock::day_start_at(row.start_ms, boundary_hour);
        // 一行最多 366 天（年折叠），给上限防脏边界死循环。
        for index in 0..400i32 {
            if day >= row.end_ms {
                break;
            }
            let column = match fold {
                Fold::Day => 0,
                Fold::Week => clock::logical_weekday(day, boundary_hour) as i32,
                _ => index,
            };
            out.insert(day, (row.index, column));
            let next = clock::day_end_at(day, boundary_hour);
            if next <= day {
                break;
            }
            day = next;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    #[test]
    fn day_ticks_land_on_logical_day_start_not_utc_midnight() {
        // 这就是前端 Math.floor(ms / DAY) * DAY 的 bug：UTC+8 下偏 8 小时，
        // 换日点为 4 时再偏 4 小时。刻度必须由 clock 给。
        for boundary in [0u32, 4] {
            let now = clock::now_ms();
            let start = now - 3 * DAY;
            let set = ticks(start, now, boundary, 10);
            let day_ticks: Vec<&Tick> = set.ticks.iter().filter(|t| t.day_start).collect();
            assert!(!day_ticks.is_empty(), "三天窗口内必须有日界刻度");
            for tick in day_ticks {
                assert_eq!(
                    tick.ms,
                    clock::day_start_at(tick.ms, boundary),
                    "日界刻度必须与 rollup 日桶起点一致 (boundary={boundary})"
                );
            }
        }
    }

    #[test]
    fn month_ticks_are_calendar_months() {
        // 2026-01-01 起 5 个月：刻度必须落在每月 1 日，而不是每 30 天。
        let jan = clock::year_start_at(
            clock::day_start_at(
                Local
                    .with_ymd_and_hms(2026, 3, 15, 12, 0, 0)
                    .unwrap()
                    .timestamp_millis(),
                0,
            ),
            0,
        );
        let set = ticks(jan, jan + 150 * DAY, 0, 6);
        assert_eq!(set.unit, "month");
        for tick in set.ticks.iter().filter(|t| t.tier == 0) {
            let date = clock::logical_date_at(tick.ms, 0);
            assert_eq!(date.day(), 1, "月刻度应落在 1 日，实际 {date}");
            assert_eq!(tick.ms, clock::month_start_at(tick.ms, 0));
        }
    }

    #[test]
    fn hour_ticks_snap_to_day_boundary() {
        let now = clock::now_ms();
        let set = ticks(now - 18 * HOUR, now, 4, 8);
        assert!(set.unit.ends_with('h') || set.unit.ends_with("min"));
        // 相邻刻度不得跨过日界还保持等距（DST / 换日点会让最后一格更短）。
        for tick in set.ticks.iter().filter(|t| t.day_start) {
            assert_eq!(tick.ms, clock::day_start_at(tick.ms, 4));
        }
    }

    #[test]
    fn tick_count_stays_bounded_at_life_scale() {
        let now = clock::now_ms();
        let set = ticks(now - 10 * 365 * DAY, now, 0, 10);
        assert!(set.ticks.len() <= MAX_TICKS, "刻度数量必须有界");
        assert!(set.ticks.iter().any(|t| t.tier == 0));
    }

    #[test]
    fn week_fold_rows_are_seven_days_and_columns_are_weekdays() {
        let now = clock::now_ms();
        let g = grid(Fold::Week, now - 21 * DAY, now, 0, 64);
        assert_eq!(g.cols, 7);
        assert_eq!(g.col_unit, "day");
        assert!(g.rows.len() >= 3);
        for row in &g.rows {
            assert_eq!(row.start_ms, clock::week_start_at(row.start_ms, 0));
            let days = (row.end_ms - row.start_ms) / HOUR;
            assert!((167..=169).contains(&days), "一周 168h±DST，实际 {days}h");
        }
        let coords = day_coords(Fold::Week, &g.rows, 0);
        // 每一行的周一都在第 0 列。
        for row in &g.rows {
            assert_eq!(coords.get(&row.start_ms).map(|c| c.1), Some(0));
        }
    }

    #[test]
    fn day_fold_rows_cover_each_logical_day_once() {
        let now = clock::now_ms();
        let g = grid(Fold::Day, now - 5 * DAY, now, 4, 64);
        assert_eq!(g.cols, 24);
        assert_eq!(g.col_unit, "hour");
        for row in &g.rows {
            assert_eq!(row.start_ms, clock::day_start_at(row.start_ms, 4));
            assert_eq!(row.end_ms, clock::day_end_at(row.start_ms, 4));
        }
        let coords = day_coords(Fold::Day, &g.rows, 4);
        assert_eq!(coords.len(), g.rows.len(), "日折叠时每行恰好一个日单元");
    }

    #[test]
    fn month_fold_row_has_calendar_month_span() {
        let g = grid(
            Fold::Month,
            Local
                .with_ymd_and_hms(2026, 2, 10, 0, 0, 0)
                .unwrap()
                .timestamp_millis(),
            Local
                .with_ymd_and_hms(2026, 4, 2, 0, 0, 0)
                .unwrap()
                .timestamp_millis(),
            0,
            64,
        );
        assert_eq!(g.cols, 31);
        assert_eq!(g.rows.len(), 3, "2 月 / 3 月 / 4 月");
        let feb = &g.rows[0];
        assert_eq!((feb.end_ms - feb.start_ms) / DAY, 28, "2026 年 2 月 28 天");
        let coords = day_coords(Fold::Month, &g.rows, 0);
        let last_feb_day = feb.end_ms - DAY;
        assert_eq!(
            coords.get(&clock::day_start_at(last_feb_day, 0)).map(|c| c.1),
            Some(27),
            "2 月最后一天在第 27 列"
        );
    }

    #[test]
    fn grid_truncates_instead_of_exploding() {
        let now = clock::now_ms();
        let g = grid(Fold::Day, now - 900 * DAY, now, 0, 120);
        assert_eq!(g.rows.len(), 120);
        assert!(g.truncated);
    }
}
