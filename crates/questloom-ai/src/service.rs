//! AI 機能のオーケストレーション。
//!
//! 「プロバイダ解決 → プロンプト生成 → CLI 実行 → 応答の解釈 → [`TaskService`] への反映」
//! までをここで完結させる。UI(Tauri)側に残るのは command としての公開と、
//! [`AiProgress`] をアプリのイベントへ載せ替えることだけ。
//!
//! | メソッド | 使う出力 | 反映先 |
//! |---|---|---|
//! | [`AiService::create_tasks`] | JSON 配列 | New 列に通常タスクを作成 |
//! | [`AiService::split_task`] | JSON 配列 | 元タスクの子タスクを作成 |
//! | [`AiService::free_instruction`] | 自然文 | MCP 経由で AI 自身が操作。応答は呼び出し元へ |
//!
//! 同時実行は [`AiRunner`] により 1 件に制限される。

use std::sync::Arc;
use std::time::Duration;

use questloom_core::model::{Origin, TaskId, TaskStatus};
use questloom_core::service::{NewTask, TaskService};
use questloom_core::settings::{AiProvider, CoreSettings};
use serde::Serialize;

use crate::error::{AiError, AiResult};
use crate::prompt::TaskDraft;
use crate::provider::McpEndpoint;
use crate::runner::{AiFeature, AiRunner, JobGuard};
use crate::{exec, prompt, provider};

/// 実行の進捗。呼び出し元(UI)へ通知するために使う。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiProgress {
    /// 実行を開始した。
    Started {
        /// 実行中の機能。
        feature: AiFeature,
    },
    /// 正常に終わった。
    Finished {
        /// 終わった機能。
        feature: AiFeature,
        /// 結果の要約(UI に出せる日本語)。
        message: String,
    },
    /// 失敗・キャンセルで終わった。
    Failed {
        /// 失敗した機能。
        feature: AiFeature,
        /// エラーメッセージ(UI に出せる日本語)。
        message: String,
    },
}

impl AiProgress {
    /// どの機能の進捗か。
    #[must_use]
    pub const fn feature(&self) -> AiFeature {
        match *self {
            Self::Started { feature }
            | Self::Finished { feature, .. }
            | Self::Failed { feature, .. } => feature,
        }
    }

    /// 補足メッセージ。開始時は無い。
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Started { .. } => None,
            Self::Finished { message, .. } | Self::Failed { message, .. } => Some(message),
        }
    }
}

/// 進捗の受け取り口。実行中に複数回呼ばれる。
pub type ProgressSink<'a> = &'a (dyn Fn(AiProgress) + Send + Sync);

/// 何もしない進捗コールバック(テスト・進捗表示の要らない呼び出し向け)。
pub fn ignore_progress() -> impl Fn(AiProgress) + Send + Sync {
    |_| {}
}

/// 作成されたタスクの要約。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCreateResult {
    /// 使ったプロバイダの ID。
    pub provider_id: String,
    /// 作成されたタスク。
    pub created: Vec<AiTaskSummary>,
}

/// 応答テキストを返す系の結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTextResult {
    /// 使ったプロバイダの ID。
    pub provider_id: String,
    /// CLI の応答(標準出力)。
    pub text: String,
    /// 内蔵 MCP サーバーへ接続させられたか。
    pub mcp_attached: bool,
}

/// AI 機能をタスク管理へ結びつけるサービス。
#[derive(Debug)]
pub struct AiService {
    tasks: Arc<TaskService>,
    runner: Arc<AiRunner>,
}

impl AiService {
    /// タスクサービスとランナーを束ねる。
    #[must_use]
    pub const fn new(tasks: Arc<TaskService>, runner: Arc<AiRunner>) -> Self {
        Self { tasks, runner }
    }

    /// 実行中の AI プロセスをキャンセルする。実行中でなければ `false`。
    pub fn cancel(&self) -> bool {
        self.runner.cancel()
    }

    /// 文章からタスクを抽出し、New 列へ通常タスクとして作成する。
    ///
    /// # Errors
    /// 実行中の場合、文章が空の場合、CLI の失敗、応答を解釈できない場合、
    /// 1 件も抽出できなかった場合、またはタスク作成の失敗。
    pub async fn create_tasks(
        &self,
        settings: &CoreSettings,
        text: &str,
        provider_id: Option<&str>,
        progress: ProgressSink<'_>,
    ) -> AiResult<AiCreateResult> {
        let feature = AiFeature::CreateTasks;
        // 実行枠を取れなかった場合は進捗を出さない(何も始まっていないため)。
        let guard = self.runner.begin(feature)?;
        progress(AiProgress::Started { feature });

        let result = async {
            let text = text.trim();
            if text.is_empty() {
                return Err(AiError::EmptyInput { label: "文章" });
            }
            let provider = provider::resolve(settings, provider_id)?;
            let prompt = prompt::create_tasks_prompt(text, self.tasks.today());
            let (stdout, _) = ask(&provider, &prompt, None, settings, &guard).await?;
            let drafts = prompt::parse_task_drafts(&stdout)?;
            Ok(AiCreateResult {
                provider_id: provider.id.clone(),
                created: create_from_drafts(&self.tasks, drafts, None)?,
            })
        }
        .await;

        report(feature, &result, progress, |value| {
            format!("{} 件のタスクを作成しました", value.created.len())
        });
        result
    }

    /// タスクをサブタスクへ分割・詳細化し、子タスクとして作成する。
    ///
    /// # Errors
    /// 実行中の場合、対象タスクが無い場合、CLI の失敗、応答を解釈できない場合、
    /// 1 件も抽出できなかった場合、またはタスク作成の失敗。
    pub async fn split_task(
        &self,
        settings: &CoreSettings,
        task_id: TaskId,
        instruction: Option<&str>,
        provider_id: Option<&str>,
        progress: ProgressSink<'_>,
    ) -> AiResult<AiCreateResult> {
        let feature = AiFeature::SplitTask;
        let guard = self.runner.begin(feature)?;
        progress(AiProgress::Started { feature });

        let result = async {
            let provider = provider::resolve(settings, provider_id)?;
            let detail = self.tasks.task_detail(task_id)?;
            let prompt = prompt::split_task_prompt(&detail, instruction, self.tasks.today());
            let (stdout, _) = ask(&provider, &prompt, None, settings, &guard).await?;
            let drafts = prompt::parse_task_drafts(&stdout)?;
            Ok(AiCreateResult {
                provider_id: provider.id.clone(),
                created: create_from_drafts(&self.tasks, drafts, Some(task_id))?,
            })
        }
        .await;

        report(feature, &result, progress, |value| {
            format!("{} 件の子タスクを作成しました", value.created.len())
        });
        result
    }

    /// 自由指示。`mcp` を渡すと、対応プロバイダには MCP 経由で自律操作させる。
    ///
    /// # Errors
    /// 実行中の場合、指示が空の場合、または CLI の失敗。
    pub async fn free_instruction(
        &self,
        settings: &CoreSettings,
        text: &str,
        provider_id: Option<&str>,
        mcp: Option<&McpEndpoint>,
        progress: ProgressSink<'_>,
    ) -> AiResult<AiTextResult> {
        let feature = AiFeature::FreeInstruction;
        let guard = self.runner.begin(feature)?;
        progress(AiProgress::Started { feature });

        let result = async {
            let text = text.trim();
            if text.is_empty() {
                return Err(AiError::EmptyInput { label: "指示" });
            }
            let provider = provider::resolve(settings, provider_id)?;
            // プロンプトの前置きを決めるため、MCP を実際に繋げられるかを先に確かめる。
            let will_attach = mcp
                .and_then(|endpoint| provider::mcp_args(&provider, endpoint))
                .is_some();
            let prompt = prompt::free_instruction_prompt(text, will_attach);
            let (stdout, mcp_attached) = ask(&provider, &prompt, mcp, settings, &guard).await?;
            Ok(AiTextResult {
                provider_id: provider.id.clone(),
                text: stdout.trim().to_owned(),
                mcp_attached,
            })
        }
        .await;

        report(feature, &result, progress, |value| {
            if value.mcp_attached {
                "MCP 経由で実行しました".to_owned()
            } else {
                "MCP 未接続のまま実行しました".to_owned()
            }
        });
        result
    }
}

/// 結果に応じて完了・失敗の進捗を送る。
fn report<T>(
    feature: AiFeature,
    result: &AiResult<T>,
    progress: ProgressSink<'_>,
    summary: impl FnOnce(&T) -> String,
) {
    let event = match result {
        Ok(value) => AiProgress::Finished {
            feature,
            message: summary(value),
        },
        Err(error) => AiProgress::Failed {
            feature,
            message: error.to_string(),
        },
    };
    progress(event);
}

/// プロンプトを CLI に投げ、標準出力と「MCP を繋げたか」を返す。
async fn ask(
    provider: &AiProvider,
    prompt: &str,
    mcp: Option<&McpEndpoint>,
    settings: &CoreSettings,
    guard: &JobGuard,
) -> AiResult<(String, bool)> {
    let prepared = provider::prepare(
        provider,
        prompt,
        mcp,
        Duration::from_secs(settings.ai_timeout_secs),
    );
    let output = exec::run(&prepared.request, guard.cancel_token()).await?;
    if !output.stderr.trim().is_empty() {
        tracing::debug!(stderr = output.stderr, "AI CLI の stderr");
    }
    Ok((
        output.into_stdout(&provider.command)?,
        prepared.mcp_attached,
    ))
}

/// AI が抽出した下書きをタスクとして登録する。
///
/// AI が作るのは「まず New に積む通常タスク」。発生元は [`Origin::Ai`]。
///
/// # Errors
/// タスク作成に失敗した場合、または 1 件も作れなかった場合 [`AiError::NoTasks`]。
pub fn create_from_drafts(
    tasks: &TaskService,
    drafts: Vec<TaskDraft>,
    parent_id: Option<TaskId>,
) -> AiResult<Vec<AiTaskSummary>> {
    let mut created = Vec::with_capacity(drafts.len());
    for draft in drafts {
        let task = tasks.create_task(NewTask {
            title: draft.title.clone(),
            description: draft.description.clone(),
            status: Some(TaskStatus::New),
            deadline: draft.deadline_utc(),
            is_instant: false,
            origin: Origin::Ai,
            parent_id,
            ..NewTask::default()
        })?;
        created.push(AiTaskSummary {
            id: task.id,
            title: task.title,
            description: task.description,
        });
    }
    if created.is_empty() {
        return Err(AiError::NoTasks);
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;
    use questloom_core::clock::SystemClock;
    use questloom_core::repository::TaskRepository;
    use questloom_core::settings::BoardSettings;
    use questloom_store::SqliteStore;

    fn service() -> Arc<TaskService> {
        let store = Arc::new(SqliteStore::open_in_memory().expect("インメモリ DB"));
        let repository: Arc<dyn TaskRepository> = store as Arc<dyn TaskRepository>;
        Arc::new(TaskService::new(
            repository,
            Arc::new(SystemClock),
            BoardSettings::default(),
        ))
    }

    #[test]
    fn drafts_become_new_tasks_with_ai_origin() {
        let tasks = service();
        let parent = tasks
            .create_task(NewTask {
                title: "親".to_owned(),
                ..NewTask::default()
            })
            .unwrap();

        let created = create_from_drafts(
            &tasks,
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

        let detail = tasks.task_detail(parent.id).unwrap();
        assert_eq!(detail.children.len(), 2);
        let child = &detail.children[0];
        assert_eq!(child.task.origin, Origin::Ai);
        assert_eq!(child.task.status, TaskStatus::New);
        assert!(!child.task.is_instant);
        assert!(child.task.deadline.is_some());

        // 1 件も作れなかった場合はエラーになる。
        assert!(matches!(
            create_from_drafts(&tasks, Vec::new(), None),
            Err(AiError::NoTasks)
        ));
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

    #[tokio::test]
    async fn empty_input_is_rejected_before_spawning_anything() {
        let ai = AiService::new(service(), Arc::new(AiRunner::new()));
        let settings = CoreSettings::default();
        let seen = std::sync::Mutex::new(Vec::new());
        let progress = |event: AiProgress| seen.lock().unwrap().push(event);

        let error = ai
            .create_tasks(&settings, "   ", None, &progress)
            .await
            .expect_err("空の文章は弾く");
        assert_eq!(error.to_string(), "文章が空です");

        let error = ai
            .free_instruction(&settings, "", None, None, &progress)
            .await
            .expect_err("空の指示は弾く");
        assert_eq!(error.to_string(), "指示が空です");

        // 開始と失敗の両方が通知され、実行枠は解放されている。
        let seen = seen.into_inner().unwrap();
        assert_eq!(
            seen[0],
            AiProgress::Started {
                feature: AiFeature::CreateTasks
            }
        );
        assert_eq!(
            seen[1],
            AiProgress::Failed {
                feature: AiFeature::CreateTasks,
                message: "文章が空です".to_owned()
            }
        );
        assert_eq!(seen[2].feature(), AiFeature::FreeInstruction);
        assert_eq!(seen[3].message(), Some("指示が空です"));
        assert!(!ai.cancel(), "実行枠は空いている");
    }

    #[tokio::test]
    async fn a_second_run_is_rejected_while_one_is_in_flight() {
        let runner = Arc::new(AiRunner::new());
        let ai = AiService::new(service(), Arc::clone(&runner));
        let _guard = runner.begin(AiFeature::SplitTask).expect("実行枠を取る");

        let progress = ignore_progress();
        let error = ai
            .create_tasks(&CoreSettings::default(), "文章", None, &progress)
            .await
            .expect_err("同時実行は拒否される");
        assert!(matches!(
            error,
            AiError::Busy {
                running: AiFeature::SplitTask
            }
        ));
    }
}
