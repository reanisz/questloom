//! questloom デスクトップアプリの Tauri シェル。配線のみを担い、ロジックは questloom-core に置く。
//!
//! 起動時に `%APPDATA%\questloom` の DB を開き(マイグレーション + バックアップ)、
//! [`TaskService`](questloom_core::service::TaskService) を State として保持する。
//! ドメインイベントは [`events::TASKS_CHANGED`] として webview へ中継される。

pub mod commands;
pub mod events;
pub mod state;

use std::sync::Arc;

use tauri::Manager;
use tracing_subscriber::EnvFilter;

use crate::state::AppState;

/// ログを初期化する。`RUST_LOG` があればそれに従う。
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // 二重初期化(テストや複数回呼び出し)でも落ちないよう結果は無視する。
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

/// アプリのエントリポイント。
///
/// # Panics
/// データディレクトリの解決・DB の初期化・Tauri の起動に失敗した場合(いずれも起動時のみ)。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let state = AppState::initialize(&data_dir)?;
            let service = Arc::clone(&state.service);

            events::spawn_bridge(app.handle().clone(), &service);
            events::spawn_day_watcher(service);

            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_board,
            commands::get_task,
            commands::create_task,
            commands::update_task,
            commands::move_task,
            commands::complete_task,
            commands::promote_task,
            commands::add_task_update,
            commands::add_resource,
            commands::remove_resource,
            commands::set_parent,
            commands::get_settings,
            commands::set_settings,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri アプリの起動に失敗しました");
}
