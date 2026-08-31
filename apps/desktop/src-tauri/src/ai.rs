//! AI CLI 呼び出しの配線。
//!
//! 機能そのもの(プロバイダ解決・プロンプト・プロセス起動・応答の解釈・
//! [`TaskService`](questloom_core::service::TaskService) への反映・同時実行の制限)は
//! [`questloom_ai::AiService`] にある。ここでやるのは Tauri command としての公開、
//! State の取り出し、[`AI_STATUS`] イベントの emit、エラーの文字列化だけ。
//!
//! | command | 使う出力 | 反映先 |
//! |---|---|---|
//! | [`ai_create_tasks`] | JSON 配列 | New 列に通常タスクを作成 |
//! | [`ai_split_task`] | JSON 配列 | 元タスクの子タスクを作成 |
//! | [`ai_free_instruction`] | 自然文 | MCP 経由で AI 自身が操作。応答は UI に表示 |

use std::sync::Arc;

use questloom_ai::{AiCreateResult, AiFeature, AiProgress, AiRunner, AiService, AiTextResult};
use questloom_core::model::TaskId;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};

use crate::commands::CommandResult;
use crate::mcp::McpSupervisor;
use crate::state::AppState;

/// AI 実行の進捗を webview へ通知するイベント名。
pub const AI_STATUS: &str = "questloom://ai-status";

/// 実行状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AiState {
    /// 実行中。
    Running,
    /// 正常終了。
    Done,
    /// 失敗・キャンセル。
    Error,
}

/// [`AI_STATUS`] のペイロード。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    /// 実行状態。
    pub state: AiState,
    /// どの機能か。
    pub feature: AiFeature,
    /// 補足メッセージ(完了内容・エラー内容)。
    pub message: Option<String>,
}

impl From<&AiProgress> for AiStatus {
    fn from(progress: &AiProgress) -> Self {
        let state = match progress {
            AiProgress::Started { .. } => AiState::Running,
            AiProgress::Finished { .. } => AiState::Done,
            AiProgress::Failed { .. } => AiState::Error,
        };
        Self {
            state,
            feature: progress.feature(),
            message: progress.message().map(ToOwned::to_owned),
        }
    }
}

/// 進捗イベントを webview へ送るコールバックを作る。
fn status_sink<R: Runtime>(app: AppHandle<R>) -> impl Fn(AiProgress) + Send + Sync {
    move |progress| {
        if let Err(error) = app.emit(AI_STATUS, AiStatus::from(&progress)) {
            tracing::error!(%error, "AI 進捗イベントの emit に失敗しました");
        }
    }
}

/// エラーを文字列にし、ログへも残す。
fn fail(error: impl std::fmt::Display) -> String {
    let message = error.to_string();
    tracing::warn!(%message, "AI の実行でエラーが発生しました");
    message
}

/// State から AI サービスを組み立てる。
fn ai_service(state: &AppState, runner: &Arc<AiRunner>) -> AiService {
    AiService::new(Arc::clone(&state.service), Arc::clone(runner))
}

/// 文章からタスクを抽出して作成する。
#[tauri::command]
pub async fn ai_create_tasks(
    app: AppHandle,
    state: State<'_, AppState>,
    runner: State<'_, Arc<AiRunner>>,
    text: String,
    provider_id: Option<String>,
) -> CommandResult<AiCreateResult> {
    let ai = ai_service(&state, runner.inner());
    let settings = state.settings();
    let progress = status_sink(app);
    ai.create_tasks(&settings, &text, provider_id.as_deref(), &progress)
        .await
        .map_err(fail)
}

/// タスクをサブタスクへ分割・詳細化し、子タスクとして作成する。
#[tauri::command]
pub async fn ai_split_task(
    app: AppHandle,
    state: State<'_, AppState>,
    runner: State<'_, Arc<AiRunner>>,
    task_id: TaskId,
    instruction: Option<String>,
    provider_id: Option<String>,
) -> CommandResult<AiCreateResult> {
    let ai = ai_service(&state, runner.inner());
    let settings = state.settings();
    let progress = status_sink(app);
    ai.split_task(
        &settings,
        task_id,
        instruction.as_deref(),
        provider_id.as_deref(),
        &progress,
    )
    .await
    .map_err(fail)
}

/// 自由指示。MCP サーバーが動いていれば、その URL を CLI へ渡して自律操作させる。
#[tauri::command]
pub async fn ai_free_instruction(
    app: AppHandle,
    state: State<'_, AppState>,
    runner: State<'_, Arc<AiRunner>>,
    supervisor: State<'_, Arc<McpSupervisor>>,
    text: String,
    provider_id: Option<String>,
) -> CommandResult<AiTextResult> {
    let ai = ai_service(&state, runner.inner());
    let settings = state.settings();
    let endpoint = supervisor.endpoint().await;
    let progress = status_sink(app);
    ai.free_instruction(
        &settings,
        &text,
        provider_id.as_deref(),
        endpoint.as_ref(),
        &progress,
    )
    .await
    .map_err(fail)
}

/// 実行中の AI プロセスを kill する。
#[tauri::command]
pub fn ai_cancel(runner: State<'_, Arc<AiRunner>>) -> CommandResult<bool> {
    Ok(runner.cancel())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_payload_is_camel_case() {
        let json = serde_json::to_value(AiStatus {
            state: AiState::Running,
            feature: AiFeature::SplitTask,
            message: None,
        })
        .unwrap();
        assert_eq!(json["state"], "running");
        assert_eq!(json["feature"], "splitTask");
        assert!(json["message"].is_null());

        let json = serde_json::to_value(AiStatus {
            state: AiState::Error,
            feature: AiFeature::FreeInstruction,
            message: Some("失敗".to_owned()),
        })
        .unwrap();
        assert_eq!(json["state"], "error");
        assert_eq!(json["feature"], "freeInstruction");
        assert_eq!(json["message"], "失敗");
    }

    /// questloom-ai の進捗が、そのままフロントのペイロードへ写ること。
    #[test]
    fn progress_maps_onto_the_status_payload() {
        let status = AiStatus::from(&AiProgress::Started {
            feature: AiFeature::CreateTasks,
        });
        assert_eq!(status.state, AiState::Running);
        assert_eq!(status.feature, AiFeature::CreateTasks);
        assert_eq!(status.message, None);

        let status = AiStatus::from(&AiProgress::Finished {
            feature: AiFeature::CreateTasks,
            message: "3 件のタスクを作成しました".to_owned(),
        });
        assert_eq!(status.state, AiState::Done);
        assert_eq!(
            status.message.as_deref(),
            Some("3 件のタスクを作成しました")
        );

        let status = AiStatus::from(&AiProgress::Failed {
            feature: AiFeature::FreeInstruction,
            message: "失敗".to_owned(),
        });
        assert_eq!(status.state, AiState::Error);
        assert_eq!(status.feature, AiFeature::FreeInstruction);
    }

    #[test]
    fn event_name_is_valid_for_tauri() {
        assert!(AI_STATUS
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '/' | ':' | '_')));
    }

    /// キャンセルはランナーへ委譲するだけ。
    #[test]
    fn cancel_is_delegated_to_the_runner() {
        let runner = Arc::new(AiRunner::new());
        let ai = AiService::new(
            crate::state::test_support::service(Default::default()),
            Arc::clone(&runner),
        );
        assert!(!ai.cancel(), "実行中でなければ false");

        let guard = runner.begin(AiFeature::CreateTasks).unwrap();
        assert!(ai.cancel());
        assert!(guard.cancel_token().is_cancelled());
    }
}
