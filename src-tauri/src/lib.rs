mod backup;
mod commands;
mod db;
mod export;
mod gemini;

use std::sync::Mutex;

use rusqlite::Connection;
use tauri::Manager;

use crate::commands::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("Failed to resolve app data dir: {e}"))?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("trace.db");
            let conn = Connection::open(db_path)?;
            db::init_db(&conn).map_err(std::io::Error::other)?;
            app.manage(AppState {
                db: Mutex::new(conn),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_today_date,
            commands::get_entry,
            commands::save_entry,
            commands::get_month_recorded_dates,
            commands::get_recent_entries,
            commands::preview_period_export,
            commands::save_text_to_file,
            commands::export_backup_file,
            commands::get_backup_json,
            commands::import_backup_data,
            commands::get_settings,
            commands::save_settings,
            commands::test_gemini_connection,
            commands::analyze_period_with_gemini,
            commands::list_ai_reports,
            commands::delete_ai_report,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Trace");
}
