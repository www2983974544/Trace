use std::sync::{Arc, Mutex};

use chrono::Local;
use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
#[cfg(mobile)]
use tauri::Manager;

use crate::{
    backup::{self, ImportResult},
    db::{self, AiReport, AppSettings, Entry, EntrySummary},
    export::{self, ExportPreview, PeriodType},
    gemini,
    lan_sync::{self, LanPeer, LanServerInfo, LanSyncOutcome, SyncServerHandle},
};

pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub sync_server: Mutex<Option<SyncServerHandle>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiChunkEvent<'a> {
    request_id: &'a str,
    delta: &'a str,
}

#[tauri::command]
pub fn get_today_date() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

#[tauri::command]
pub fn get_entry(state: State<'_, AppState>, date: String) -> Result<Option<Entry>, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁定失败".to_string())?;
    db::get_entry(&conn, &date)
}

#[tauri::command]
pub fn save_entry(
    state: State<'_, AppState>,
    date: String,
    content: String,
) -> Result<Option<Entry>, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁定失败".to_string())?;
    db::upsert_entry(&conn, &date, &content)
}

#[tauri::command]
pub fn get_month_recorded_dates(
    state: State<'_, AppState>,
    year: i32,
    month: u32,
) -> Result<Vec<String>, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁定失败".to_string())?;
    db::get_month_recorded_dates(&conn, year, month)
}

#[tauri::command]
pub fn get_recent_entries(
    state: State<'_, AppState>,
    exclude_date: Option<String>,
    limit: usize,
) -> Result<Vec<EntrySummary>, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁定失败".to_string())?;
    db::get_recent_entries(&conn, exclude_date.as_deref(), limit)
}

#[tauri::command]
pub fn preview_period_export(
    state: State<'_, AppState>,
    anchor_date: String,
    period: PeriodType,
    format: String,
) -> Result<ExportPreview, String> {
    let anchor = db::validate_date(&anchor_date)?;
    let range = export::resolve_period(anchor, period);
    let conn = state.db.lock().map_err(|_| "数据库锁定失败".to_string())?;
    let entries = db::get_entries_in_range(&conn, &range.start_date, &range.end_date)?;
    Ok(export::build_export(&range, &entries, &format))
}

#[tauri::command]
pub async fn save_text_to_file(
    app: AppHandle,
    default_filename: String,
    content: String,
) -> Result<Option<String>, String> {
    #[cfg(not(mobile))]
    {
        use tauri_plugin_dialog::DialogExt;

        let chosen = tauri::async_runtime::spawn_blocking(move || {
            app.dialog()
                .file()
                .set_file_name(&default_filename)
                .blocking_save_file()
        })
        .await
        .map_err(|e| format!("保存对话框执行失败：{e}"))?;

        let Some(file_path) = chosen else {
            return Ok(None);
        };
        let path = file_path
            .into_path()
            .map_err(|e| format!("无效的文件路径：{e}"))?;
        std::fs::write(&path, content.as_bytes())
            .map_err(|e| format!("写入文件失败：{e}"))?;
        Ok(Some(path.to_string_lossy().to_string()))
    }

    #[cfg(mobile)]
    {
        let resolver = app.path();
        let candidates = [
            resolver.download_dir(),
            resolver.document_dir(),
            resolver.app_data_dir(),
        ];
        let mut last_err = "无法获取可用的导出目录".to_string();
        for dir_res in candidates {
            let dir = match dir_res {
                Ok(d) => d,
                Err(e) => {
                    last_err = format!("获取导出目录失败：{e}");
                    continue;
                }
            };
            if let Err(e) = std::fs::create_dir_all(&dir) {
                last_err = format!("创建导出目录失败：{e}");
                continue;
            }
            let path = dir.join(&default_filename);
            match std::fs::write(&path, content.as_bytes()) {
                Ok(()) => return Ok(Some(path.to_string_lossy().to_string())),
                Err(e) => {
                    last_err = format!("写入文件失败：{e}");
                }
            }
        }
        Err(last_err)
    }
}

#[tauri::command]
pub async fn export_backup_file(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let json = {
        let conn = state.db.lock().map_err(|_| "数据库锁定失败".to_string())?;
        backup::export_backup_json(&conn)?
    };
    let filename = format!("trace-backup-{}.json", get_today_date());
    save_text_to_file(app, filename, json).await
}

#[tauri::command]
pub fn get_backup_json(state: State<'_, AppState>) -> Result<String, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁定失败".to_string())?;
    backup::export_backup_json(&conn)
}

#[tauri::command]
pub fn import_backup_data(
    state: State<'_, AppState>,
    json_content: String,
    overwrite: bool,
) -> Result<ImportResult, String> {
    let mut conn = state.db.lock().map_err(|_| "数据库锁定失败".to_string())?;
    backup::import_backup_json(&mut conn, &json_content, overwrite)
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁定失败".to_string())?;
    db::get_settings(&conn)
}

#[tauri::command]
pub fn save_settings(
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁定失败".to_string())?;
    db::save_settings(&conn, &settings)
}

#[tauri::command]
pub async fn analyze_period_with_gemini(
    app: AppHandle,
    state: State<'_, AppState>,
    request_id: String,
    anchor_date: String,
    period: PeriodType,
    prompt: String,
) -> Result<AiReport, String> {
    let anchor = db::validate_date(&anchor_date)?;
    let range = export::resolve_period(anchor, period);

    // Read settings & entries synchronously, dropping MutexGuard before `.await`.
    let (settings, preview) = {
        let conn = state.db.lock().map_err(|_| "数据库锁定失败".to_string())?;
        let settings = db::get_settings(&conn)?;
        let entries = db::get_entries_in_range(&conn, &range.start_date, &range.end_date)?;
        let preview = export::build_export(&range, &entries, "md");
        (settings, preview)
    };

    if preview.entry_count == 0 {
        return Err("所选周期内暂无日记记录，无法进行分析".to_string());
    }

    let user_prompt = prompt.trim();
    let full_prompt = if user_prompt.is_empty() {
        format!("请分析以下个人日记内容并给出清晰、有条理的总结与建议：\n\n{}", preview.content)
    } else {
        format!("{user_prompt}\n\n---\n\n{}", preview.content)
    };

    let result_text = gemini::stream_analyze(
        &settings.gemini_api_key,
        &settings.gemini_model,
        &settings.gemini_base_url,
        &full_prompt,
        |delta| {
            let _ = app.emit(
                "gemini-chunk",
                GeminiChunkEvent {
                    request_id: &request_id,
                    delta,
                },
            );
        },
    )
    .await?;

    let conn = state.db.lock().map_err(|_| "数据库锁定失败".to_string())?;
    db::save_ai_report(
        &conn,
        range.period_type.as_str(),
        &range.label,
        &range.start_date,
        &range.end_date,
        &settings.gemini_model,
        user_prompt,
        &result_text,
    )
}

#[tauri::command]
pub async fn test_gemini_connection(
    api_key: String,
    model: String,
    base_url: String,
) -> Result<gemini::GeminiTestResult, String> {
    gemini::test_connection(&api_key, &model, &base_url).await
}

#[tauri::command]
pub fn list_ai_reports(state: State<'_, AppState>, limit: usize) -> Result<Vec<AiReport>, String> {
    let conn = state.db.lock().map_err(|_| "数据库锁定失败".to_string())?;
    db::list_ai_reports(&conn, limit)
}

#[tauri::command]
pub fn delete_ai_report(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|_| "数据库锁定失败".to_string())?;
    db::delete_ai_report(&conn, id)
}

#[tauri::command]
pub async fn start_lan_sync_server(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<LanServerInfo, String> {
    {
        let mut guard = state
            .sync_server
            .lock()
            .map_err(|_| "快传状态锁定失败".to_string())?;
        if let Some(existing) = guard.as_ref() {
            if !existing.is_stopped() {
                return Ok(existing.info().clone());
            }
        }
        *guard = None;
    }

    let db = Arc::clone(&state.db);
    let app_handle = app.clone();
    let (handle, info) = lan_sync::start_server(db, move |evt| {
        let _ = app_handle.emit("lan-sync-received", evt);
    })
    .await?;

    let mut guard = state
        .sync_server
        .lock()
        .map_err(|_| "快传状态锁定失败".to_string())?;
    if let Some(prev) = guard.replace(handle) {
        prev.stop();
    }
    Ok(info)
}

#[tauri::command]
pub fn stop_lan_sync_server(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state
        .sync_server
        .lock()
        .map_err(|_| "快传状态锁定失败".to_string())?;
    if let Some(handle) = guard.take() {
        handle.stop();
    }
    Ok(())
}

#[tauri::command]
pub fn get_lan_sync_status(state: State<'_, AppState>) -> Result<Option<LanServerInfo>, String> {
    let mut guard = state
        .sync_server
        .lock()
        .map_err(|_| "快传状态锁定失败".to_string())?;
    if let Some(handle) = guard.as_ref() {
        if handle.is_stopped() {
            *guard = None;
            return Ok(None);
        }
        return Ok(Some(handle.info().clone()));
    }
    Ok(None)
}

#[tauri::command]
pub async fn discover_lan_peers(
    state: State<'_, AppState>,
    timeout_ms: Option<u64>,
) -> Result<Vec<LanPeer>, String> {
    let own_session_id = {
        let mut guard = state
            .sync_server
            .lock()
            .map_err(|_| "快传状态锁定失败".to_string())?;
        match guard.as_ref() {
            Some(handle) if !handle.is_stopped() => Some(handle.session_id().to_string()),
            Some(_) => {
                *guard = None;
                None
            }
            None => None,
        }
    };
    lan_sync::discover_peers(timeout_ms.unwrap_or(1800), own_session_id.as_deref()).await
}

#[tauri::command]
pub async fn sync_with_lan_peer(
    state: State<'_, AppState>,
    address: String,
    pin: String,
) -> Result<LanSyncOutcome, String> {
    let db = Arc::clone(&state.db);
    lan_sync::sync_with_peer(db, &address, &pin).await
}
