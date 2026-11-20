use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::db::{self, AiReport, Entry};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPayload {
    pub schema_version: u32,
    pub exported_at: i64,
    pub entries: Vec<Entry>,
    pub ai_reports: Vec<AiReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported_entries: usize,
    pub updated_entries: usize,
    pub skipped_entries: usize,
    pub imported_reports: usize,
}

pub fn export_backup_json(conn: &Connection) -> Result<String, String> {
    let mut entry_stmt = conn
        .prepare(
            "SELECT id, entry_date, content, word_count, created_at, updated_at
             FROM entries ORDER BY entry_date ASC",
        )
        .map_err(|e| format!("Failed to prepare entries backup: {e}"))?;
    let entries = entry_stmt
        .query_map([], |row| {
            Ok(Entry {
                id: row.get(0)?,
                entry_date: row.get(1)?,
                content: row.get(2)?,
                word_count: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })
        .map_err(|e| format!("Failed to read entries for backup: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect entries for backup: {e}"))?;

    let ai_reports = db::list_ai_reports(conn, 1000)?;
    let payload = BackupPayload {
        schema_version: 1,
        exported_at: Utc::now().timestamp(),
        entries,
        ai_reports,
    };

    serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("Failed to serialize backup JSON: {e}"))
}

pub fn import_backup_json(
    conn: &mut Connection,
    json: &str,
    overwrite: bool,
) -> Result<ImportResult, String> {
    let payload: BackupPayload = serde_json::from_str(json)
        .map_err(|e| format!("无效的备份 JSON 文件：{e}"))?;
    if payload.schema_version != 1 {
        return Err(format!("不支持的备份版本：{}", payload.schema_version));
    }
    for entry in &payload.entries {
        db::validate_date(&entry.entry_date)?;
    }

    let tx = conn
        .transaction()
        .map_err(|e| format!("Failed to begin import transaction: {e}"))?;

    let mut result = ImportResult {
        imported_entries: 0,
        updated_entries: 0,
        skipped_entries: 0,
        imported_reports: 0,
    };

    if overwrite {
        tx.execute("DELETE FROM entries", [])
            .map_err(|e| format!("Failed to clear entries: {e}"))?;
        tx.execute("DELETE FROM ai_reports", [])
            .map_err(|e| format!("Failed to clear ai_reports: {e}"))?;
    }

    for entry in &payload.entries {
        if entry.content.trim().is_empty() {
            continue;
        }
        let words = db::count_words(&entry.content);
        if overwrite {
            tx.execute(
                "INSERT INTO entries (entry_date, content, word_count, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    entry.entry_date,
                    entry.content,
                    words,
                    entry.created_at,
                    entry.updated_at
                ],
            )
            .map_err(|e| format!("Failed to import entry {}: {e}", entry.entry_date))?;
            result.imported_entries += 1;
        } else {
            let existing_updated: Option<i64> = tx
                .query_row(
                    "SELECT updated_at FROM entries WHERE entry_date = ?1",
                    params![entry.entry_date],
                    |row| row.get(0),
                )
                .ok();
            match existing_updated {
                None => {
                    tx.execute(
                        "INSERT INTO entries (entry_date, content, word_count, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            entry.entry_date,
                            entry.content,
                            words,
                            entry.created_at,
                            entry.updated_at
                        ],
                    )
                    .map_err(|e| format!("Failed to insert entry {}: {e}", entry.entry_date))?;
                    result.imported_entries += 1;
                }
                Some(cur_updated) if entry.updated_at > cur_updated => {
                    tx.execute(
                        "UPDATE entries SET content = ?2, word_count = ?3, updated_at = ?4
                         WHERE entry_date = ?1",
                        params![entry.entry_date, entry.content, words, entry.updated_at],
                    )
                    .map_err(|e| format!("Failed to update entry {}: {e}", entry.entry_date))?;
                    result.updated_entries += 1;
                }
                Some(_) => {
                    result.skipped_entries += 1;
                }
            }
        }
    }

    for report in &payload.ai_reports {
        let exists: bool = if overwrite {
            false
        } else {
            tx.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM ai_reports
                    WHERE period_type = ?1 AND start_date = ?2 AND end_date = ?3 AND created_at = ?4
                )",
                params![
                    report.period_type,
                    report.start_date,
                    report.end_date,
                    report.created_at
                ],
                |row| row.get(0),
            )
            .unwrap_or(false)
        };
        if !exists {
            tx.execute(
                "INSERT INTO ai_reports (period_type, period_label, start_date, end_date, model, prompt, content, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    report.period_type,
                    report.period_label,
                    report.start_date,
                    report.end_date,
                    report.model,
                    report.prompt,
                    report.content,
                    report.created_at
                ],
            )
            .map_err(|e| format!("Failed to import AI report: {e}"))?;
            result.imported_reports += 1;
        }
    }

    tx.commit()
        .map_err(|e| format!("Failed to commit import transaction: {e}"))?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        conn
    }

    #[test]
    fn backup_roundtrip_merge_and_overwrite() {
        let src = mem_db();
        db::upsert_entry(&src, "2026-11-19", "周四日记").unwrap();
        db::upsert_entry(&src, "2026-11-20", "周五日记").unwrap();
        db::save_ai_report(
            &src,
            "week",
            "2026-W47",
            "2026-11-16",
            "2026-11-22",
            "gemini-3.8-flash",
            "总结本周",
            "分析结果",
        )
        .unwrap();

        let json = export_backup_json(&src).unwrap();

        let mut dst = mem_db();
        db::upsert_entry(&dst, "2026-11-18", "本地旧日记").unwrap();
        let merged = import_backup_json(&mut dst, &json, false).unwrap();
        assert_eq!(merged.imported_entries, 2);
        assert_eq!(merged.imported_reports, 1);
        assert!(db::get_entry(&dst, "2026-11-18").unwrap().is_some());

        let overwritten = import_backup_json(&mut dst, &json, true).unwrap();
        assert_eq!(overwritten.imported_entries, 2);
        assert!(db::get_entry(&dst, "2026-11-18").unwrap().is_none());
        assert_eq!(db::list_ai_reports(&dst, 10).unwrap().len(), 1);
    }

    #[test]
    fn invalid_backup_leaves_existing_data_untouched() {
        let mut dst = mem_db();
        db::upsert_entry(&dst, "2026-11-20", "不能丢失").unwrap();
        let err = import_backup_json(&mut dst, "{not-valid-json", true);
        assert!(err.is_err());
        assert_eq!(
            db::get_entry(&dst, "2026-11-20").unwrap().unwrap().content,
            "不能丢失"
        );
    }
}
