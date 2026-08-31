//! Tauri command。サービス層への薄い委譲のみを行い、エラーは境界で文字列化する。
//!
//! JS 側の引数は camelCase で渡す(Tauri v2 が snake_case へ変換する)。

use std::sync::Arc;

use questloom_core::bucket::BoardColumn;
use questloom_core::model::{Origin, ResourceId, Task, TaskId, TaskResource, TaskUpdateEntry};
use questloom_core::service::{Board, MoveRequest, NewResource, NewTask, TaskDetail, TaskPatch};
use questloom_core::settings::CoreSettings;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::mcp::McpSupervisor;
use crate::state::AppState;
use crate::{settings, shortcut, window};

/// command の戻り値。エラーはフロントで扱いやすいよう文字列にする。
pub type CommandResult<T> = Result<T, String>;

/// command のエラーをフロントへ返す文字列にし、あわせて警告ログへ残す。
///
/// 以前は [`crate::ai`] と [`crate::plugin_host`] にも同じものがあった。
/// エラーの文字列化と「必ずログに残す」という約束は 1 箇所に置く。
pub fn fail(error: impl std::fmt::Display) -> String {
    let message = error.to_string();
    tracing::warn!(%message, "command でエラーが発生しました");
    message
}

/// ボード全体を、バケット導出済みの構造で返す。
#[tauri::command]
pub fn get_board(state: State<'_, AppState>) -> CommandResult<Board> {
    state.service.board().map_err(fail)
}

/// タスク詳細(リソース・履歴・親子込み)を返す。
#[tauri::command]
pub fn get_task(state: State<'_, AppState>, task_id: TaskId) -> CommandResult<TaskDetail> {
    state.service.task_detail(task_id).map_err(fail)
}

/// タスクを作成する。
#[tauri::command]
pub fn create_task(state: State<'_, AppState>, input: NewTask) -> CommandResult<Task> {
    state.service.create_task(input).map_err(fail)
}

/// タスクの内容を更新する。
#[tauri::command]
pub fn update_task(
    state: State<'_, AppState>,
    task_id: TaskId,
    patch: TaskPatch,
) -> CommandResult<Task> {
    state.service.update_task(task_id, patch).map_err(fail)
}

/// タスクの状態・予定・並び順を変更する。
#[tauri::command]
pub fn move_task(
    state: State<'_, AppState>,
    task_id: TaskId,
    request: MoveRequest,
) -> CommandResult<Task> {
    state.service.move_task(task_id, request).map_err(fail)
}

/// タスクを完了にする。
#[tauri::command]
pub fn complete_task(state: State<'_, AppState>, task_id: TaskId) -> CommandResult<Task> {
    state.service.complete_task(task_id).map_err(fail)
}

/// インスタントタスクを通常タスクへ昇格する。
#[tauri::command]
pub fn promote_task(
    state: State<'_, AppState>,
    task_id: TaskId,
    column: Option<BoardColumn>,
) -> CommandResult<Task> {
    state.service.promote_task(task_id, column).map_err(fail)
}

/// アップデート履歴を追記する。
#[tauri::command]
pub fn add_task_update(
    state: State<'_, AppState>,
    task_id: TaskId,
    body: String,
    origin: Option<Origin>,
) -> CommandResult<TaskUpdateEntry> {
    state
        .service
        .add_task_update(task_id, body, origin.unwrap_or(Origin::User))
        .map_err(fail)
}

/// 関連リソースを追加する。
#[tauri::command]
pub fn add_resource(
    state: State<'_, AppState>,
    task_id: TaskId,
    resource: NewResource,
) -> CommandResult<TaskResource> {
    state.service.add_resource(task_id, resource).map_err(fail)
}

/// 関連リソースを削除する。
#[tauri::command]
pub fn remove_resource(
    state: State<'_, AppState>,
    task_id: TaskId,
    resource_id: ResourceId,
) -> CommandResult<()> {
    state
        .service
        .remove_resource(task_id, resource_id)
        .map_err(fail)
}

/// 親タスクを設定・解除する(循環は禁止)。
#[tauri::command]
pub fn set_parent(
    state: State<'_, AppState>,
    task_id: TaskId,
    parent_id: Option<TaskId>,
) -> CommandResult<Task> {
    state.service.set_parent(task_id, parent_id).map_err(fail)
}

/// コア設定を返す。
#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> CommandResult<CoreSettings> {
    Ok(state.settings())
}

/// 出荷時のコア設定を返す。設定画面の「既定値に戻す」で使う。
///
/// 返すだけで保存はしない(フォームに読み込ませ、保存操作は利用者に委ねる)。
#[tauri::command]
pub fn get_default_settings() -> CommandResult<CoreSettings> {
    Ok(CoreSettings::default())
}

/// コア設定を検証して保存し、即座に反映する。
///
/// 不正な値(解釈できないショートカット文字列など)は保存せずエラーを返す。
/// ショートカット・自動起動・オーバーレイ表示は、保存時に発行される
/// `SettingsChanged` イベントを購読している watcher が反映する。
#[tauri::command]
pub fn set_settings(state: State<'_, AppState>, settings: CoreSettings) -> CommandResult<()> {
    settings::validate(&settings).map_err(fail)?;
    state.save_settings(settings).map_err(fail)
}

/// デスクトップ側の稼働状態。設定画面での確認用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    /// 内蔵 MCP サーバーが起動しているか。
    pub mcp_running: bool,
    /// 起動中の MCP エンドポイント URL。停止中は `None`。
    pub mcp_url: Option<String>,
    /// 起動中の MCP サーバーが Bearer トークンを要求するか。
    pub mcp_token_required: bool,
    /// 設定中のグローバルショートカットを実際に登録できているか。
    pub shortcut_registered: bool,
}

/// MCP サーバーとグローバルショートカットの現在の稼働状態を返す。
///
/// 設定値ではなく「いま動いているもの」を映す。保存に失敗しうる要素
/// (ポート衝突・ショートカットの奪い合い)を利用者が確認できるようにするため。
#[tauri::command]
pub async fn get_runtime_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<RuntimeStatus> {
    let shortcut_spec = state.settings().global_shortcut;
    // State の借用を await をまたいで持ち越さないよう、Arc を取り出しておく。
    let supervisor = app
        .try_state::<Arc<McpSupervisor>>()
        .map(|state| Arc::clone(state.inner()));
    let endpoint = match supervisor {
        Some(supervisor) => supervisor.endpoint().await,
        None => None,
    };

    Ok(RuntimeStatus {
        mcp_running: endpoint.is_some(),
        mcp_token_required: endpoint
            .as_ref()
            .is_some_and(|endpoint| endpoint.token.is_some()),
        mcp_url: endpoint.map(|endpoint| endpoint.url),
        shortcut_registered: shortcut::is_registered(&app, &shortcut_spec),
    })
}

/// メインウィンドウを表示・フォーカスし、指定があればそのタスクの詳細を開かせる。
///
/// オーバーレイから通常タスクをクリックしたときに使う。
#[tauri::command]
pub fn show_main_window(app: AppHandle, task_id: Option<TaskId>) -> CommandResult<()> {
    window::show_main(&app, task_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 設定画面が読む形(camelCase)を固定する。
    #[test]
    fn runtime_status_json_is_camel_case() {
        let json = serde_json::to_value(RuntimeStatus {
            mcp_running: true,
            mcp_url: Some("http://127.0.0.1:39150/mcp".to_owned()),
            mcp_token_required: false,
            shortcut_registered: true,
        })
        .unwrap();
        assert_eq!(json["mcpRunning"], true);
        assert_eq!(json["mcpUrl"], "http://127.0.0.1:39150/mcp");
        assert_eq!(json["mcpTokenRequired"], false);
        assert_eq!(json["shortcutRegistered"], true);
        assert_eq!(json.as_object().map(serde_json::Map::len), Some(4));
    }

    /// 停止中は `mcpUrl` が `null` になる(フロントは null 判定で「停止中」を出す)。
    #[test]
    fn runtime_status_reports_a_stopped_server_as_null_url() {
        let json = serde_json::to_value(RuntimeStatus {
            mcp_running: false,
            mcp_url: None,
            mcp_token_required: false,
            shortcut_registered: false,
        })
        .unwrap();
        assert_eq!(json["mcpRunning"], false);
        assert!(json["mcpUrl"].is_null());
    }

    /// フロントへ返す文字列は元のエラーそのまま。
    #[test]
    fn fail_keeps_the_original_message() {
        assert_eq!(fail("タスクが見つかりません"), "タスクが見つかりません");
    }
}
