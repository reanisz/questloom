//! questloom デスクトップアプリの Tauri シェル。配線のみを担い、ロジックは questloom-core に置く。
//!
//! 起動時に `%APPDATA%\questloom` の DB を開き(マイグレーション + バックアップ)、
//! [`TaskService`](questloom_core::service::TaskService) を State として保持する。
//! データディレクトリと MCP ポートは環境変数で上書きできる([`env_override`])。
//! ドメインイベントは [`contract::TASKS_CHANGED`] として webview へ中継される。
//!
//! ウィンドウは 3 つ。メインウィンドウ(ボード)は閉じるとトレイへ格納され、
//! オーバーレイウィンドウは New タスクがある間だけ表示される。
//! plugin-host ウィンドウは常に非表示で、TS プラグイン([`plugin_host`])を実行する。
//!
//! **3 つとも `tauri.conf.json` で `"create": false` にし、生成は setup フックの中で
//! [`create_windows`] が行う。** Tauri は「conf 定義のウィンドウを生成 → setup フック」の順で
//! 動くため、既定のままだと webview の最初の `invoke` が [`AppState`] の `manage` を
//! 追い越し、`state not managed for field 'state'` で初回描画が失敗しうる
//! (アセット同梱ビルドでは毎回起きる)。属性は conf 側に残したままなので、
//! ウィンドウの見た目・capability の対応づけは従来どおり conf と
//! `capabilities/*.json` で決まる。
//!
//! **ウィンドウごとの command 許可**は `capabilities/*.json` で決まる。
//! アプリ独自 command の一覧は [`app_commands::APP_COMMANDS`] が唯一の定義で、
//! `build.rs` がそこから permission を生成する。下の
//! [`tauri::generate_handler!`] とは 1 対 1 で対応させること
//! (対応が崩れていないことは `tests::app_commands_match_the_capabilities` が見る)。
//!
//! 設定で有効なら、内蔵 MCP サーバー([`mcp`])を `127.0.0.1` で起動する。
//! AI CLI の呼び出し([`ai`])は、実行中の MCP サーバーがあればその URL を CLI に渡す。

pub mod ai;
/// アプリ独自 Tauri command の一覧。`build.rs` と共有する
/// (`include!` される都合でモジュールドキュメントを持てないため、説明はここに置く)。
pub mod app_commands;
pub mod autostart;
pub mod commands;
pub mod contract;
pub mod env_override;
pub mod events;
pub mod mcp;
pub mod overlay;
pub mod plugin_host;
pub mod settings;
pub mod shortcut;
pub mod state;
pub mod tray;
pub mod window;

use std::sync::Arc;
use std::time::Duration;

use tauri::{Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use tracing_subscriber::EnvFilter;

use crate::mcp::McpSupervisor;
use crate::state::AppState;

/// 終了時に MCP サーバーの停止を待つ上限。
///
/// 停止は axum の graceful shutdown なので、接続中の MCP クライアントがいると
/// 待たされうる。終了操作が固まる方が害なので、待つのは一瞬だけにする。
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

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
            // テストからは QUESTLOOM_DATA_DIR で本物の %APPDATA% を避けられる(env_override 参照)。
            let data_dir = match env_override::data_dir() {
                Some(dir) => {
                    tracing::info!(
                        path = %dir.display(),
                        "{} でデータディレクトリを上書きします",
                        env_override::DATA_DIR_ENV
                    );
                    dir
                }
                None => app.path().app_data_dir()?,
            };
            let state = AppState::initialize(&data_dir)?;
            let service = Arc::clone(&state.service);
            let settings = state.settings();
            app.manage(state);
            app.manage(Arc::new(mcp::McpSupervisor::new(Arc::clone(&service))));
            app.manage(Arc::new(questloom_ai::AiRunner::new()));
            // TS プラグインのライフサイクルは plugin-host webview 上の JS が持つ。
            // Rust 側はそのロード結果を受け取るレジストリだけを用意する。
            app.manage(plugin_host::PluginRegistry::new());

            // State が出揃ってからウィンドウを作る。逆順にすると webview の初回 invoke が
            // manage を追い越して "state not managed" になる(モジュールドキュメント参照)。
            create_windows(app)?;

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
            commands::delete_task,
            commands::restore_task,
            commands::list_deleted_tasks,
            commands::get_settings,
            commands::get_default_settings,
            commands::set_settings,
            commands::get_runtime_status,
            commands::show_main_window,
            ai::ai_create_tasks,
            ai::ai_split_task,
            ai::ai_free_instruction,
            ai::ai_cancel,
            plugin_host::plugin_directory,
            plugin_host::plugin_list_sources,
            plugin_host::plugin_kv_get,
            plugin_host::plugin_kv_set,
            plugin_host::plugin_kv_keys,
            plugin_host::plugin_get_settings,
            plugin_host::plugin_set_settings,
            plugin_host::plugin_list_task_resources,
            plugin_host::plugin_log,
            plugin_host::plugin_fetch_allowed,
            plugin_host::plugin_publish_loaded,
            plugin_host::plugin_list_loaded,
        ])
        .build(tauri::generate_context!())
        .expect("Tauri アプリの起動に失敗しました")
        .run(|app, event| {
            // 終了時は MCP サーバーを畳んでからプロセスを落とす(ポートを掴んだままにしない)。
            if matches!(event, RunEvent::Exit) {
                stop_mcp(app);
            }
        });
}

/// `tauri.conf.json` のウィンドウ定義(`app.windows`)からウィンドウを生成する。
///
/// 定義はすべて `"create": false` にしてあるので、Tauri は自動生成しない。
/// ここでは定義をそのまま [`tauri::WebviewWindowBuilder::from_config`] に渡すだけで、
/// サイズ・可視性・フォーカス等の属性は conf 側の記述がそのまま効く。
///
/// # Errors
/// ウィンドウの生成に失敗した場合。
fn create_windows(app: &tauri::App) -> tauri::Result<()> {
    // build() が app を借りるので、定義は先に取り出しておく。
    let windows = app.config().app.windows.clone();
    for config in &windows {
        tracing::debug!(label = %config.label, "ウィンドウを生成します");
        tauri::WebviewWindowBuilder::from_config(app.handle(), config)?.build()?;
    }
    Ok(())
}

/// 内蔵 MCP サーバーを止める。停止に手間取っても終了を待たせすぎない。
fn stop_mcp(app: &tauri::AppHandle) {
    let Some(supervisor) = app.try_state::<Arc<McpSupervisor>>() else {
        return;
    };
    let supervisor = Arc::clone(supervisor.inner());
    let stopped = tauri::async_runtime::block_on(async move {
        tokio::time::timeout(SHUTDOWN_TIMEOUT, supervisor.stop())
            .await
            .is_ok()
    });
    if !stopped {
        tracing::warn!("MCP サーバーの停止を待ちきれませんでした。そのまま終了します");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::app_commands::{allow_permission, APP_COMMANDS};

    /// capability ファイル 1 件が持つ、アプリ独自 command の permission。
    ///
    /// `core:` / `opener:` のようにプラグインの接頭辞が付くものは対象外。
    fn app_permissions(capability: &str) -> BTreeSet<String> {
        let value: serde_json::Value =
            serde_json::from_str(capability).expect("capability は JSON として読める");
        value["permissions"]
            .as_array()
            .expect("permissions は配列")
            .iter()
            .filter_map(|item| item.as_str())
            .filter(|item| !item.contains(':'))
            .map(ToOwned::to_owned)
            .collect()
    }

    fn allowed(commands: &[&str]) -> BTreeSet<String> {
        commands.iter().map(|c| allow_permission(c)).collect()
    }

    const CONFIG: &str = include_str!("../tauri.conf.json");

    /// ウィンドウは Tauri に自動生成させず、`setup` の中で作る。
    ///
    /// 自動生成(`create` 既定 true)に戻すと、webview の最初の `invoke` が
    /// `app.manage(AppState)` を追い越して `state not managed` になる。
    /// アセット同梱ビルドでは初回描画が必ず失敗するので、ここで固定しておく。
    #[test]
    fn windows_are_created_by_the_setup_hook() {
        let value: serde_json::Value =
            serde_json::from_str(CONFIG).expect("tauri.conf.json は JSON として読める");
        let windows = value["app"]["windows"]
            .as_array()
            .expect("app.windows は配列");
        assert_eq!(windows.len(), 3, "main / overlay / plugin-host の 3 つ");
        for window in windows {
            assert_eq!(
                window["create"],
                serde_json::Value::Bool(false),
                "{} は create: false にすること(生成は crate::create_windows)",
                window["label"]
            );
        }
    }

    const MAIN: &str = include_str!("../capabilities/default.json");
    const OVERLAY: &str = include_str!("../capabilities/overlay.json");
    const PLUGIN_HOST: &str = include_str!("../capabilities/plugin-host.json");

    /// capability が参照する command permission は、必ず `APP_COMMANDS` から生成されたもの。
    ///
    /// 名前を間違えるとビルド時に `Permission ... not found` で落ちるが、
    /// 逆(command を消したのに capability に残る)も含めてここで押さえる。
    #[test]
    fn capabilities_only_reference_generated_permissions() {
        let known = allowed(APP_COMMANDS);
        for (name, capability) in [
            ("default", MAIN),
            ("overlay", OVERLAY),
            ("plugin-host", PLUGIN_HOST),
        ] {
            for permission in app_permissions(capability) {
                assert!(
                    known.contains(&permission),
                    "{name} が知らない permission {permission} を参照している"
                );
            }
        }
    }

    /// メインウィンドウ(ボード・ドロワー・設定画面)は全 command を使える。
    #[test]
    fn app_commands_match_the_capabilities() {
        assert_eq!(
            app_permissions(MAIN),
            allowed(APP_COMMANDS),
            "main の capability は APP_COMMANDS と一致させること"
        );
    }

    /// オーバーレイは New タスクの一覧・完了・メインウィンドウ表示だけ。
    #[test]
    fn the_overlay_window_gets_only_what_it_calls() {
        assert_eq!(
            app_permissions(OVERLAY),
            allowed(&["get_board", "complete_task", "show_main_window"])
        );
    }

    /// plugin-host は「プラグイン基盤 + ctx.tasks が呼ぶタスク操作」まで。
    ///
    /// ここで動くのは第三者のプラグインコードなので、設定・AI・稼働状態は渡さない。
    #[test]
    fn the_plugin_host_window_gets_only_the_plugin_surface() {
        assert_eq!(
            app_permissions(PLUGIN_HOST),
            allowed(&[
                "plugin_list_sources",
                "plugin_kv_get",
                "plugin_kv_set",
                "plugin_kv_keys",
                "plugin_get_settings",
                "plugin_list_task_resources",
                "plugin_log",
                "plugin_fetch_allowed",
                "plugin_publish_loaded",
                "get_board",
                "get_task",
                "create_task",
                "move_task",
                "complete_task",
                "add_task_update",
                "add_resource",
            ])
        );
    }

    /// 管理系の command は plugin-host / overlay から絶対に見えないこと。
    ///
    /// タスクの削除・復元(`delete_task` / `restore_task` / `list_deleted_tasks`)も
    /// **main だけ**にする。オーバーレイは完了させるだけ、プラグインには
    /// 他人のタスクを消す手段を与えない。
    #[test]
    fn management_commands_are_never_exposed_to_untrusted_windows() {
        let forbidden = allowed(&[
            "delete_task",
            "restore_task",
            "list_deleted_tasks",
            "get_settings",
            "get_default_settings",
            "set_settings",
            "get_runtime_status",
            "ai_create_tasks",
            "ai_split_task",
            "ai_free_instruction",
            "ai_cancel",
            "plugin_set_settings",
            "plugin_directory",
            "plugin_list_loaded",
        ]);
        for (name, capability) in [("overlay", OVERLAY), ("plugin-host", PLUGIN_HOST)] {
            let granted = app_permissions(capability);
            for permission in &forbidden {
                assert!(
                    !granted.contains(permission),
                    "{name} に {permission} を渡してはいけない"
                );
            }
        }
    }
}
