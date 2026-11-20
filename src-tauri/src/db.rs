use chrono::{NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: i64,
    pub entry_date: String,
    pub content: String,
    pub word_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    pub id: i64,
    pub entry_date: String,
    pub preview: String,
    pub word_count: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiReport {
    pub id: i64,
    pub period_type: String,
    pub period_label: String,
    pub start_date: String,
    pub end_date: String,
    pub model: String,
    pub prompt: String,
    pub content: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub gemini_api_key: String,
    pub gemini_model: String,
    pub gemini_base_url: String,
    pub theme: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            gemini_api_key: String::new(),
            gemini_model: "gemini-3.8-flash".to_string(),
            gemini_base_url: String::new(),
            theme: "system".to_string(),
        }
    }
}

pub fn init_db(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS entries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            entry_date TEXT NOT NULL UNIQUE,
            content TEXT NOT NULL,
            word_count INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_entries_date ON entries(entry_date DESC);

        CREATE TABLE IF NOT EXISTS ai_reports (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            period_type TEXT NOT NULL,
            period_label TEXT NOT NULL,
            start_date TEXT NOT NULL,
            end_date TEXT NOT NULL,
            model TEXT NOT NULL,
            prompt TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_ai_reports_created ON ai_reports(created_at DESC);

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        ",
    )
    .map_err(|e| format!("Failed to initialize SQLite database: {e}"))
}

pub fn validate_date(date: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| format!("Invalid date format '{date}', expected YYYY-MM-DD"))
}

pub fn count_words(text: &str) -> i64 {
    text.chars().filter(|c| !c.is_whitespace()).count() as i64
}

pub fn get_entry(conn: &Connection, date: &str) -> Result<Option<Entry>, String> {
    validate_date(date)?;
    conn.query_row(
        "SELECT id, entry_date, content, word_count, created_at, updated_at
         FROM entries WHERE entry_date = ?1",
        params![date],
        |row| {
            Ok(Entry {
                id: row.get(0)?,
                entry_date: row.get(1)?,
                content: row.get(2)?,
                word_count: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(|e| format!("Failed to read entry {date}: {e}"))
}

pub fn upsert_entry(conn: &Connection, date: &str, content: &str) -> Result<Option<Entry>, String> {
    validate_date(date)?;
    if content.trim().is_empty() {
        conn.execute("DELETE FROM entries WHERE entry_date = ?1", params![date])
            .map_err(|e| format!("Failed to clear empty entry {date}: {e}"))?;
        return Ok(None);
    }

    let words = count_words(content);
    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT INTO entries (entry_date, content, word_count, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(entry_date) DO UPDATE SET
             content = excluded.content,
             word_count = excluded.word_count,
             updated_at = excluded.updated_at",
        params![date, content, words, now],
    )
    .map_err(|e| format!("Failed to save entry {date}: {e}"))?;

    get_entry(conn, date)
}

pub fn get_month_recorded_dates(
    conn: &Connection,
    year: i32,
    month: u32,
) -> Result<Vec<String>, String> {
    if !(1..=12).contains(&month) {
        return Err(format!("Invalid month: {month}"));
    }
    let prefix = format!("{year:04}-{month:02}-%");
    let mut stmt = conn
        .prepare("SELECT entry_date FROM entries WHERE entry_date LIKE ?1 ORDER BY entry_date ASC")
        .map_err(|e| format!("Failed to prepare month query: {e}"))?;
    let rows = stmt
        .query_map(params![prefix], |row| row.get::<_, String>(0))
        .map_err(|e| format!("Failed to query month dates: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect month dates: {e}"))
}

pub fn get_recent_entries(
    conn: &Connection,
    exclude_date: Option<&str>,
    limit: usize,
) -> Result<Vec<EntrySummary>, String> {
    let cap = limit.min(200) as i64;
    let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<EntrySummary> {
        Ok(EntrySummary {
            id: row.get(0)?,
            entry_date: row.get(1)?,
            preview: row.get(2)?,
            word_count: row.get(3)?,
            updated_at: row.get(4)?,
        })
    };

    if let Some(excluded) = exclude_date {
        let mut stmt = conn
            .prepare(
                "SELECT id, entry_date, substr(content, 1, 140), word_count, updated_at
                 FROM entries
                 WHERE entry_date != ?1
                 ORDER BY entry_date DESC
                 LIMIT ?2",
            )
            .map_err(|e| format!("Failed to prepare recent entries query: {e}"))?;
        let rows = stmt
            .query_map(params![excluded, cap], map_row)
            .map_err(|e| format!("Failed to query recent entries: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read recent entries: {e}"))
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT id, entry_date, substr(content, 1, 140), word_count, updated_at
                 FROM entries
                 ORDER BY entry_date DESC
                 LIMIT ?1",
            )
            .map_err(|e| format!("Failed to prepare recent entries query: {e}"))?;
        let rows = stmt
            .query_map(params![cap], map_row)
            .map_err(|e| format!("Failed to query recent entries: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read recent entries: {e}"))
    }
}

pub fn get_entries_in_range(
    conn: &Connection,
    start_date: &str,
    end_date: &str,
) -> Result<Vec<Entry>, String> {
    validate_date(start_date)?;
    validate_date(end_date)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, entry_date, content, word_count, created_at, updated_at
             FROM entries
             WHERE entry_date >= ?1 AND entry_date <= ?2
             ORDER BY entry_date ASC",
        )
        .map_err(|e| format!("Failed to prepare range query: {e}"))?;
    let rows = stmt
        .query_map(params![start_date, end_date], |row| {
            Ok(Entry {
                id: row.get(0)?,
                entry_date: row.get(1)?,
                content: row.get(2)?,
                word_count: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })
        .map_err(|e| format!("Failed to query entries in range: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect range entries: {e}"))
}

pub fn save_ai_report(
    conn: &Connection,
    period_type: &str,
    period_label: &str,
    start_date: &str,
    end_date: &str,
    model: &str,
    prompt: &str,
    content: &str,
) -> Result<AiReport, String> {
    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT INTO ai_reports (period_type, period_label, start_date, end_date, model, prompt, content, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            period_type,
            period_label,
            start_date,
            end_date,
            model,
            prompt,
            content,
            now
        ],
    )
    .map_err(|e| format!("Failed to save AI report: {e}"))?;
    let id = conn.last_insert_rowid();
    Ok(AiReport {
        id,
        period_type: period_type.to_string(),
        period_label: period_label.to_string(),
        start_date: start_date.to_string(),
        end_date: end_date.to_string(),
        model: model.to_string(),
        prompt: prompt.to_string(),
        content: content.to_string(),
        created_at: now,
    })
}

pub fn list_ai_reports(conn: &Connection, limit: usize) -> Result<Vec<AiReport>, String> {
    let cap = limit.min(100) as i64;
    let mut stmt = conn
        .prepare(
            "SELECT id, period_type, period_label, start_date, end_date, model, prompt, content, created_at
             FROM ai_reports
             ORDER BY created_at DESC, id DESC
             LIMIT ?1",
        )
        .map_err(|e| format!("Failed to prepare AI report list: {e}"))?;
    let rows = stmt
        .query_map(params![cap], |row| {
            Ok(AiReport {
                id: row.get(0)?,
                period_type: row.get(1)?,
                period_label: row.get(2)?,
                start_date: row.get(3)?,
                end_date: row.get(4)?,
                model: row.get(5)?,
                prompt: row.get(6)?,
                content: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .map_err(|e| format!("Failed to list AI reports: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect AI reports: {e}"))
}

pub fn delete_ai_report(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM ai_reports WHERE id = ?1", params![id])
        .map_err(|e| format!("Failed to delete AI report {id}: {e}"))?;
    Ok(())
}

const SETTINGS_KEY: &str = "app_settings";

pub fn get_settings(conn: &Connection) -> Result<AppSettings, String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![SETTINGS_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Failed to read settings: {e}"))?;

    match raw {
        Some(json) => serde_json::from_str::<AppSettings>(&json)
            .map_err(|e| format!("Corrupted settings JSON: {e}")),
        None => Ok(AppSettings::default()),
    }
}

pub fn save_settings(conn: &Connection, settings: &AppSettings) -> Result<AppSettings, String> {
    let normalized = AppSettings {
        gemini_api_key: settings.gemini_api_key.trim().to_string(),
        gemini_model: if settings.gemini_model.trim().is_empty() {
            "gemini-3.8-flash".to_string()
        } else {
            settings.gemini_model.trim().to_string()
        },
        gemini_base_url: settings.gemini_base_url.trim().to_string(),
        theme: match settings.theme.as_str() {
            "light" | "dark" | "system" => settings.theme.clone(),
            _ => "system".to_string(),
        },
    };
    let json = serde_json::to_string(&normalized)
        .map_err(|e| format!("Failed to serialize settings: {e}"))?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![SETTINGS_KEY, json],
    )
    .map_err(|e| format!("Failed to save settings: {e}"))?;
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        conn
    }

    #[test]
    fn upsert_enforces_single_entry_per_date_and_clears_when_blank() {
        let conn = mem_db();
        let e1 = upsert_entry(&conn, "2026-11-20", "第一行内容").unwrap().unwrap();
        assert_eq!(e1.word_count, 5);

        let e2 = upsert_entry(&conn, "2026-11-20", "更新后的正文内容").unwrap().unwrap();
        assert_eq!(e2.id, e1.id);
        assert_eq!(e2.word_count, 8);

        let cleared = upsert_entry(&conn, "2026-11-20", "   \n\t ").unwrap();
        assert!(cleared.is_none());
        assert!(get_entry(&conn, "2026-11-20").unwrap().is_none());
    }

    #[test]
    fn month_dates_and_recent_summary_projection_work() {
        let conn = mem_db();
        let long_text = "今天".repeat(120);
        upsert_entry(&conn, "2026-11-18", &long_text).unwrap();
        upsert_entry(&conn, "2026-11-20", "今日记录").unwrap();
        upsert_entry(&conn, "2026-10-31", "上月记录").unwrap();

        let nov = get_month_recorded_dates(&conn, 2026, 11).unwrap();
        assert_eq!(nov, vec!["2026-11-18", "2026-11-20"]);

        let recent = get_recent_entries(&conn, Some("2026-11-20"), 10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].entry_date, "2026-11-18");
        assert_eq!(recent[0].preview.chars().count(), 140);
        assert_eq!(recent[1].entry_date, "2026-10-31");
    }
}
