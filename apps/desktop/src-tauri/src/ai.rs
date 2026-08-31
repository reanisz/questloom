//! AI CLI 呼び出しの配線。
//!
//! 実行そのもの(プロセス起動・プロンプト・JSON 抽出)は [`questloom_ai`] にあり、
//! ここでは Tauri command としての公開、同時実行の制限、進捗イベントの emit、
//! 抽出結果の [`TaskService`] への反映だけを行う。
//!
//! 提供する機能は 3 つ。
//!
//! | command | 使う出力 | 反映先 |
//! |---|---|---|
//! | [`ai_create_tasks`] | JSON 配列 | New 列に通常タスクを作成 |
//! | [`ai_split_task`] | JSON 配列 | 元タスクの子タスクを作成 |
//! | [`ai_free_instruction`] | 自然文 | MCP 経由で AI 自身が操作。応答は UI に表示 |
//!
//! 実行中は 1 件だけを許し、[`AI_STATUS`] イベントで状態を通知する。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use questloom_ai::{exec, prompt, provider, AiError, TaskDraft};
use questloom_core::model::{Origin, TaskId, TaskStatus};
use questloom_core::service::{NewTask, TaskService};
use questloom_core::settings::{AiProvider, CoreSettings};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};
use tokio_util::sync::CancellationToken;

use crate::commands::CommandResult;
use crate::mcp::McpSupervisor;
use crate::state::AppState;

/// AI 実行の進捗を webview へ通知するイベント名。
pub const AI_STATUS: &str = "questloom://ai-status";

/// AI 機能の種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AiFeature {
    /// 文章からタスクを作る。
    CreateTasks,
    /// タスクを分割・詳細化する。
    SplitTask,
    /// 自由指示(MCP 経由の操作を含む)。
    FreeInstruction,
}

impl AiFeature {
    /// UI に出す日本語名。
    const fn label(self) -> &'static str {
        match self {
            Self::CreateTasks => "タスク作成",
            Self::SplitTask => "分割/詳細化",
            Self::FreeInstruction => "自由指示",
        }
    }
}

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

/// 作成されたタスクの要約。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTaskSummary {
    /// 作成されたタスクの ID。
    pub id: TaskId,
    /// タイトル。
    pub title: String,
    /// 詳細(空のこともある)。
    pub description: String,
}

/// タスクを作る系の結果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCreateResult {
    /// 使ったプロバイダの ID。
    pub provider_id: String,
    /// 作成されたタスク。
    pub created: Vec<AiTaskSummary>,
}

/// 応答テキストを返す系の結果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTextResult {
    /// 使ったプロバイダの ID。
    pub provider_id: String,
    /// CLI の応答(標準出力)。
    pub text: String,
    /// 内蔵 MCP サーバーへ接続させられたか。
    pub mcp_attached: bool,
}

/// 実行中の AI ジョブを 1 件だけ持つ。
///
/// Tauri の managed state として保持する。実行中に別の実行が来たら拒否する
/// (キューには積まない)。
#[derive(Debug, Default)]
pub struct AiRunner {
    current: Mutex<Option<Job>>,
}

#[derive(Debug)]
struct Job {
    feature: AiFeature,
    cancel: CancellationToken,
}

/// 実行中であることを表すガード。drop で実行枠を空ける。
#[derive(Debug)]
pub struct JobGuard {
    runner: Arc<AiRunner>,
    cancel: CancellationToken,
}

impl JobGuard {
    /// この実行のキャンセルトークン。
    #[must_use]
    pub const fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        *AiRunner::lock(&self.runner.current) = None;
    }
}

impl AiRunner {
    /// 空のランナーを作る。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            current: Mutex::new(None),
        }
    }

    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// 実行枠を確保する。既に実行中なら理由付きで失敗する。
    ///
    /// # Errors
    /// 別の AI 実行が進行中の場合。
    pub fn begin(self: &Arc<Self>, feature: AiFeature) -> Result<JobGuard, String> {
        let mut current = Self::lock(&self.current);
        if let Some(job) = current.as_ref() {
            return Err(format!(
                "AI の{}を実行中です。完了するかキャンセルしてから実行してください",
                job.feature.label()
            ));
        }
        let cancel = CancellationToken::new();
        *current = Some(Job {
            feature,
            cancel: cancel.clone(),
        });
        Ok(JobGuard {
            runner: Arc::clone(self),
            cancel,
        })
    }

    /// 実行中のジョブをキャンセルする。実行中でなければ `false`。
    pub fn cancel(&self) -> bool {
        let current = Self::lock(&self.current);
        match current.as_ref() {
            Some(job) => {
                job.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// 実行中の機能。
    #[must_use]
    pub fn running_feature(&self) -> Option<AiFeature> {
        Self::lock(&self.current).as_ref().map(|job| job.feature)
    }
}

/// 進捗イベントを webview へ送る。
fn emit<R: Runtime>(
    app: &AppHandle<R>,
    state: AiState,
    feature: AiFeature,
    message: Option<String>,
) {
    let payload = AiStatus {
        state,
        feature,
        message,
    };
    if let Err(error) = app.emit(AI_STATUS, payload) {
        tracing::error!(%error, "AI 進捗イベントの emit に失敗しました");
    }
}

/// エラーを文字列にし、ログへも残す。
fn fail(error: impl std::fmt::Display) -> String {
    let message = error.to_string();
    tracing::warn!(%message, "AI の実行でエラーが発生しました");
    message
}

/// 設定からプロバイダを解決する。
fn resolve_provider(settings: &CoreSettings, id: Option<&str>) -> Result<AiProvider, String> {
    settings.ai_provider(id).cloned().ok_or_else(|| {
        let available: Vec<&str> = settings
            .enabled_ai_providers()
            .map(|provider| provider.id.as_str())
            .collect();
        let requested = id.unwrap_or(&settings.ai_default_provider_id);
        fail(format!(
            "AI プロバイダ {requested:?} が見つからないか無効です(利用可能: {})",
            if available.is_empty() {
                "なし".to_owned()
            } else {
                available.join(" / ")
            }
        ))
    })
}

/// プロンプトを CLI に投げ、標準出力を受け取る。
async fn ask(
    provider: &AiProvider,
    prompt: &str,
    mcp: Option<&provider::McpEndpoint>,
    settings: &CoreSettings,
    guard: &JobGuard,
) -> Result<(String, bool), String> {
    let prepared = provider::prepare(
        provider,
        prompt,
        mcp,
        Duration::from_secs(settings.ai_timeout_secs),
    );
    let output = exec::run(&prepared.request, guard.cancel_token())
        .await
        .map_err(fail)?;
    if !output.stderr.trim().is_empty() {
        tracing::debug!(stderr = output.stderr, "AI CLI の stderr");
    }
    let stdout = output.into_stdout(&provider.command).map_err(fail)?;
    Ok((stdout, prepared.mcp_attached))
}

/// 抽出結果をタスクとして登録する。
fn create_from_drafts(
    service: &TaskService,
    drafts: Vec<TaskDraft>,
    parent_id: Option<TaskId>,
) -> Result<Vec<AiTaskSummary>, String> {
    let mut created = Vec::with_capacity(drafts.len());
    for draft in drafts {
        let task = service
            .create_task(NewTask {
                title: draft.title.clone(),
                description: draft.description.clone(),
                // AI が作るのは「まず New に積む通常タスク」。
                status: Some(TaskStatus::New),
                deadline: draft.deadline_utc(),
                is_instant: false,
                origin: Origin::Ai,
                parent_id,
                ..NewTask::default()
            })
            .map_err(fail)?;
        created.push(AiTaskSummary {
            id: task.id,
            title: task.title,
            description: task.description,
        });
    }
    if created.is_empty() {
        return Err(fail(AiError::NoTasks));
    }
    Ok(created)
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
    let feature = AiFeature::CreateTasks;
    let guard = Arc::clone(runner.inner()).begin(feature)?;
    emit(&app, AiState::Running, feature, None);

    let service = Arc::clone(&state.service);
    let settings = service.settings();
    let result = async {
        let text = text.trim();
        if text.is_empty() {
            return Err("文章が空です".to_owned());
        }
        let provider = resolve_provider(&settings, provider_id.as_deref())?;
        let prompt = prompt::create_tasks_prompt(text, service.today());
        let (stdout, _) = ask(&provider, &prompt, None, &settings, &guard).await?;
        let drafts = prompt::parse_task_drafts(&stdout).map_err(fail)?;
        Ok(AiCreateResult {
            provider_id: provider.id.clone(),
            created: create_from_drafts(&service, drafts, None)?,
        })
    }
    .await;

    report(&app, feature, &result, |value| {
        format!("{} 件のタスクを作成しました", value.created.len())
    });
    result
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
    let feature = AiFeature::SplitTask;
    let guard = Arc::clone(runner.inner()).begin(feature)?;
    emit(&app, AiState::Running, feature, None);

    let service = Arc::clone(&state.service);
    let settings = service.settings();
    let result = async {
        let provider = resolve_provider(&settings, provider_id.as_deref())?;
        let detail = service.task_detail(task_id).map_err(fail)?;
        let prompt = prompt::split_task_prompt(&detail, instruction.as_deref(), service.today());
        let (stdout, _) = ask(&provider, &prompt, None, &settings, &guard).await?;
        let drafts = prompt::parse_task_drafts(&stdout).map_err(fail)?;
        Ok(AiCreateResult {
            provider_id: provider.id.clone(),
            created: create_from_drafts(&service, drafts, Some(task_id))?,
        })
    }
    .await;

    report(&app, feature, &result, |value| {
        format!("{} 件の子タスクを作成しました", value.created.len())
    });
    result
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
    let feature = AiFeature::FreeInstruction;
    let guard = Arc::clone(runner.inner()).begin(feature)?;
    emit(&app, AiState::Running, feature, None);

    let service = Arc::clone(&state.service);
    let settings = service.settings();
    let endpoint = supervisor.endpoint().await;
    let result = async {
        let text = text.trim();
        if text.is_empty() {
            return Err("指示が空です".to_owned());
        }
        let provider = resolve_provider(&settings, provider_id.as_deref())?;
        // プロンプトの前置きを決めるため、MCP を実際に繋げられるかを先に確かめる。
        let will_attach = endpoint
            .as_ref()
            .and_then(|endpoint| provider::mcp_args(&provider, endpoint))
            .is_some();
        let prompt = prompt::free_instruction_prompt(text, will_attach);
        let (stdout, mcp_attached) =
            ask(&provider, &prompt, endpoint.as_ref(), &settings, &guard).await?;
        Ok(AiTextResult {
            provider_id: provider.id.clone(),
            text: stdout.trim().to_owned(),
            mcp_attached,
        })
    }
    .await;

    report(&app, feature, &result, |value| {
        if value.mcp_attached {
            "MCP 経由で実行しました".to_owned()
        } else {
            "MCP 未接続のまま実行しました".to_owned()
        }
    });
    result
}

/// 実行中の AI プロセスを kill する。
#[tauri::command]
pub fn ai_cancel(runner: State<'_, Arc<AiRunner>>) -> CommandResult<bool> {
    Ok(runner.cancel())
}

/// 結果に応じて完了・エラーのイベントを送る。
fn report<T, R: Runtime>(
    app: &AppHandle<R>,
    feature: AiFeature,
    result: &Result<T, String>,
    summary: impl FnOnce(&T) -> String,
) {
    match result {
        Ok(value) => emit(app, AiState::Done, feature, Some(summary(value))),
        Err(message) => emit(app, AiState::Error, feature, Some(message.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use questloom_core::settings::default_ai_providers;

    #[test]
    fn only_one_job_runs_at_a_time() {
        let runner = Arc::new(AiRunner::new());
        assert_eq!(runner.running_feature(), None);
        assert!(!runner.cancel(), "実行中でなければ何もしない");

        let guard = runner.begin(AiFeature::CreateTasks).unwrap();
        assert_eq!(runner.running_feature(), Some(AiFeature::CreateTasks));

        let error = runner.begin(AiFeature::FreeInstruction).unwrap_err();
        assert!(error.contains("タスク作成"), "{error}");
        assert!(error.contains("実行中"), "{error}");

        assert!(runner.cancel());
        assert!(guard.cancel_token().is_cancelled());

        // ガードを落とすと次の実行が通る。
        drop(guard);
        assert_eq!(runner.running_feature(), None);
        let guard = runner.begin(AiFeature::FreeInstruction).unwrap();
        assert!(!guard.cancel_token().is_cancelled());
    }

    #[test]
    fn resolves_providers_and_explains_failures() {
        let settings = CoreSettings::default();
        assert_eq!(resolve_provider(&settings, None).unwrap().id, "claude");
        assert_eq!(
            resolve_provider(&settings, Some("codex")).unwrap().id,
            "codex"
        );

        // 既定で無効の antigravity は選べない。
        let error = resolve_provider(&settings, Some("antigravity")).unwrap_err();
        assert!(error.contains("antigravity"), "{error}");
        assert!(error.contains("claude / codex"), "{error}");

        let settings = CoreSettings {
            ai_providers: default_ai_providers()
                .into_iter()
                .map(|provider| AiProvider {
                    enabled: false,
                    ..provider
                })
                .collect(),
            ..CoreSettings::default()
        };
        assert!(resolve_provider(&settings, None)
            .unwrap_err()
            .contains("なし"));
    }

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

    #[test]
    fn result_payloads_are_camel_case() {
        let json = serde_json::to_value(AiCreateResult {
            provider_id: "claude".to_owned(),
            created: vec![AiTaskSummary {
                id: TaskId::new(),
                title: "買い物".to_owned(),
                description: String::new(),
            }],
        })
        .unwrap();
        assert_eq!(json["providerId"], "claude");
        assert_eq!(json["created"][0]["title"], "買い物");

        let json = serde_json::to_value(AiTextResult {
            provider_id: "claude".to_owned(),
            text: "やりました".to_owned(),
            mcp_attached: true,
        })
        .unwrap();
        assert_eq!(json["mcpAttached"], true);
        assert_eq!(json["text"], "やりました");
    }

    #[test]
    fn event_name_is_valid_for_tauri() {
        assert!(AI_STATUS
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '/' | ':' | '_')));
    }

    #[test]
    fn drafts_become_new_tasks_with_ai_origin() {
        use questloom_core::clock::SystemClock;
        use questloom_core::repository::TaskRepository;
        use questloom_store::SqliteStore;

        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let repository: Arc<dyn TaskRepository> = store as Arc<dyn TaskRepository>;
        let service = TaskService::new(repository, Arc::new(SystemClock), CoreSettings::default());

        let parent = service
            .create_task(NewTask {
                title: "親".to_owned(),
                ..NewTask::default()
            })
            .unwrap();

        let created = create_from_drafts(
            &service,
            vec![
                TaskDraft {
                    title: "見積もり".to_owned(),
                    description: "概算".to_owned(),
                    deadline: Some("2026-09-30".to_owned()),
                },
                TaskDraft {
                    title: "発注".to_owned(),
                    ..TaskDraft::default()
                },
            ],
            Some(parent.id),
        )
        .unwrap();
        assert_eq!(created.len(), 2);

        let detail = service.task_detail(parent.id).unwrap();
        assert_eq!(detail.children.len(), 2);
        let child = &detail.children[0];
        assert_eq!(child.task.origin, Origin::Ai);
        assert_eq!(child.task.status, TaskStatus::New);
        assert!(!child.task.is_instant);
        assert!(child.task.deadline.is_some());

        // 1 件も作れなかった場合はエラーになる。
        assert!(create_from_drafts(&service, Vec::new(), None).is_err());
    }
}
