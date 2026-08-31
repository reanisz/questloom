//! questloom デスクトップアプリの Tauri シェル。配線のみを担い、ロジックは questloom-core に置く。
//!
//! 起動時に `%APPDATA%\questloom` の DB を開き(マイグレーション + バックアップ)、
//! [`TaskService`](questloom_core::service::TaskService) を State として保持する。
//! ドメインイベントは [`events::TASKS_CHANGED`] として webview へ中継される。
//!
//! ウィンドウは 2 つ。メインウィンドウ(ボード)は閉じるとトレイへ格納され、
//! オーバーレイウィンドウは New タスクがある間だけ表示される。
//!
//! 設定で有効なら、内蔵 MCP サーバー([`mcp`])を `127.0.0.1` で起動する。

pub mod autostart;
pub mod commands;
pub mod events;
pub mod mcp;
pub mod overlay;
pub mod settings;
pub mod shortcut;
pub mod state;
pub mod tray;
pub mod window;

use std::sync::Arc;

use tauri::{Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
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
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let handle = app.handle().clone();
            let data_dir = app.path().app_data_dir()?;
            let state = AppState::initialize(&data_dir)?;
            let service = Arc::clone(&state.service);
            let settings = service.settings();
            app.manage(state);
            app.manage(Arc::new(mcp::McpSupervisor::new(Arc::clone(&service))));

            tray::setup(&handle)?;

            events::spawn_bridge(handle.clone(), &service);
            events::spawn_day_watcher(Arc::clone(&service));
            overlay::spawn_watcher(handle.clone(), &service);
            settings::spawn_watcher(handle.clone(), &service);

            // 起動時点の設定・タスク状況を反映する。
            settings::apply(&handle, &settings);
            overlay::sync(&handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            // メインウィンドウの閉じるはトレイ格納。終了はトレイメニューからのみ。
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == window::MAIN_WINDOW {
                    api.prevent_close();
                    window::hide_main(window.app_handle());
                }
            }
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
            commands::show_main_window,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri アプリの起動に失敗しました");
}
