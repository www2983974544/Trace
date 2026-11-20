use chrono::{Datelike, Duration, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

use crate::db::Entry;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PeriodType {
    Day,
    Week,
    Month,
    Year,
}

impl PeriodType {
    pub fn as_str(self) -> &'static str {
        match self {
            PeriodType::Day => "day",
            PeriodType::Week => "week",
            PeriodType::Month => "month",
            PeriodType::Year => "year",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PeriodRange {
    pub period_type: PeriodType,
    pub label: String,
    pub start_date: String,
    pub end_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreview {
    pub range: PeriodRange,
    pub entry_count: usize,
    pub char_count: usize,
    pub estimated_tokens: usize,
    pub filename: String,
    pub content: String,
}

pub fn weekday_zh(date: NaiveDate) -> &'static str {
    match date.weekday() {
        Weekday::Mon => "周一",
        Weekday::Tue => "周二",
        Weekday::Wed => "周三",
        Weekday::Thu => "周四",
        Weekday::Fri => "周五",
        Weekday::Sat => "周六",
        Weekday::Sun => "周日",
    }
}

pub fn resolve_period(anchor: NaiveDate, period: PeriodType) -> PeriodRange {
    match period {
        PeriodType::Day => {
            let s = anchor.format("%Y-%m-%d").to_string();
            PeriodRange {
                period_type: period,
                label: s.clone(),
                start_date: s.clone(),
                end_date: s,
            }
        }
        PeriodType::Week => {
            let week = anchor.week(Weekday::Mon);
            let start = week.first_day();
            let end = week.last_day();
            let iso = anchor.iso_week();
            PeriodRange {
                period_type: period,
                label: format!("{}-W{:02}", iso.year(), iso.week()),
                start_date: start.format("%Y-%m-%d").to_string(),
                end_date: end.format("%Y-%m-%d").to_string(),
            }
        }
        PeriodType::Month => {
            let year = anchor.year();
            let month = anchor.month();
            let start = NaiveDate::from_ymd_opt(year, month, 1).expect("valid first day of month");
            let next_month_start = if month == 12 {
                NaiveDate::from_ymd_opt(year + 1, 1, 1)
            } else {
                NaiveDate::from_ymd_opt(year, month + 1, 1)
            }
            .expect("valid next month start");
            let end = next_month_start - Duration::days(1);
            PeriodRange {
                period_type: period,
                label: format!("{year:04}-{month:02}"),
                start_date: start.format("%Y-%m-%d").to_string(),
                end_date: end.format("%Y-%m-%d").to_string(),
            }
        }
        PeriodType::Year => {
            let year = anchor.year();
            PeriodRange {
                period_type: period,
                label: format!("{year:04}"),
                start_date: format!("{year:04}-01-01"),
                end_date: format!("{year:04}-12-31"),
            }
        }
    }
}

pub fn build_export(range: &PeriodRange, entries: &[Entry], format: &str) -> ExportPreview {
    let ext = if format == "txt" { "txt" } else { "md" };
    let filename = format!("trace-{}.{ext}", range.label);
    if entries.is_empty() {
        return ExportPreview {
            range: range.clone(),
            entry_count: 0,
            char_count: 0,
            estimated_tokens: 0,
            filename,
            content: String::new(),
        };
    }

    let mut out = String::new();
    if ext == "txt" {
        out.push_str(&format!(
            "Trace 日记汇总（{} | {} 至 {} | 共 {} 篇）\n\n",
            range.label,
            range.start_date,
            range.end_date,
            entries.len()
        ));
    } else {
        out.push_str(&format!(
            "# Trace 日记汇总（{}）\n\n> 周期：{} 至 {} · 共 {} 篇\n\n",
            range.label,
            range.start_date,
            range.end_date,
            entries.len()
        ));
    }

    for (idx, entry) in entries.iter().enumerate() {
        if idx > 0 {
            out.push_str("\n\n");
        }
        let day_label = NaiveDate::parse_from_str(&entry.entry_date, "%Y-%m-%d")
            .map(weekday_zh)
            .unwrap_or("");
        if ext == "txt" {
            out.push_str(&format!("=== {} {} ===\n\n", entry.entry_date, day_label));
        } else {
            out.push_str(&format!("## {} {}\n\n", entry.entry_date, day_label));
        }
        out.push_str(entry.content.trim_end());
    }
    out.push('\n');

    let char_count = out.chars().count();
    let estimated_tokens = ((char_count as f64) * 0.65).ceil() as usize;

    ExportPreview {
        range: range.clone(),
        entry_count: entries.len(),
        char_count,
        estimated_tokens,
        filename,
        content: out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(date: &str, content: &str) -> Entry {
        Entry {
            id: 1,
            entry_date: date.to_string(),
            content: content.to_string(),
            word_count: content.chars().filter(|c| !c.is_whitespace()).count() as i64,
            created_at: 100,
            updated_at: 100,
        }
    }

    #[test]
    fn resolves_day_week_month_year_boundaries() {
        let jan1 = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let day = resolve_period(jan1, PeriodType::Day);
        assert_eq!(day.start_date, "2026-01-01");
        assert_eq!(day.end_date, "2026-01-01");

        let week = resolve_period(jan1, PeriodType::Week);
        assert_eq!(week.start_date, "2025-12-29");
        assert_eq!(week.end_date, "2026-01-04");
        assert_eq!(week.label, "2026-W01");

        let leap_feb = NaiveDate::from_ymd_opt(2024, 2, 15).unwrap();
        let m_leap = resolve_period(leap_feb, PeriodType::Month);
        assert_eq!(m_leap.start_date, "2024-02-01");
        assert_eq!(m_leap.end_date, "2024-02-29");

        let y = resolve_period(leap_feb, PeriodType::Year);
        assert_eq!(y.start_date, "2024-01-01");
        assert_eq!(y.end_date, "2024-12-31");
        assert_eq!(y.label, "2024");
    }

    #[test]
    fn builds_markdown_and_txt_exports() {
        let anchor = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        let range = resolve_period(anchor, PeriodType::Week);
        let entries = vec![
            sample_entry("2026-11-18", "周三完成策划定稿"),
            sample_entry("2026-11-20", "周五开始编码"),
        ];

        let md = build_export(&range, &entries, "md");
        assert_eq!(md.filename, "trace-2026-W47.md");
        assert_eq!(md.entry_count, 2);
        assert!(md.content.contains("## 2026-11-18 周三"));
        assert!(md.content.contains("## 2026-11-20 周五"));
        assert!(md.estimated_tokens > 0);

        let txt = build_export(&range, &entries, "txt");
        assert_eq!(txt.filename, "trace-2026-W47.txt");
        assert!(txt.content.contains("=== 2026-11-18 周三 ==="));

        let empty = build_export(&range, &[], "md");
        assert_eq!(empty.entry_count, 0);
        assert!(empty.content.is_empty());
    }
}
