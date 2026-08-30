//! Tauri command。サービス層への薄い委譲のみを行い、エラーは境界で文字列化する。
//!
//! JS 側の引数は camelCase で渡す(Tauri v2 が snake_case へ変換する)。

use questloom_core::bucket::BoardColumn;
use questloom_core::model::{Origin, ResourceId, Task, TaskId, TaskResource, TaskUpdateEntry};
use questloom_core::service::{Board, MoveRequest, NewResource, NewTask, TaskDetail, TaskPatch};
use questloom_core::settings::CoreSettings;
use tauri::State;

use crate::state::AppState;

/// command の戻り値。エラーはフロントで扱いやすいよう文字列にする。
pub type CommandResult<T> = Result<T, String>;

fn fail(error: impl std::fmt::Display) -> String {
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
    Ok(state.service.settings())
}

/// コア設定を保存し、即座に反映する。
#[tauri::command]
pub fn set_settings(state: State<'_, AppState>, settings: CoreSettings) -> CommandResult<()> {
    state.save_settings(settings).map_err(fail)
}
