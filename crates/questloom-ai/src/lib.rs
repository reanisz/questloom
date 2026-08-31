//! questloom の AI 呼び出し層。外部 AI CLI を非同期に spawn し、結果を構造化して返す。
//!
//! この crate は「CLI をどう起動し、応答をどう読むか」だけを担う。UI・Tauri・
//! タスクの永続化には依存せず、[`questloom_core`] のモデルと設定だけを参照する。
//!
//! - [`exec`] — プロセス起動、タイムアウト、キャンセル、Windows のシム(`.cmd`)対応
//! - [`provider`] — プロバイダ定義の解決と、実行引数の組み立て(MCP 接続引数を含む)
//! - [`prompt`] — 3 機能のプロンプト設計と応答 JSON の解釈
//! - [`json`] — 説明文やコードフェンスに埋もれた JSON を取り出すヘルパ
//! - [`runner`] — 同時実行を 1 件に制限するランナーとキャンセル
//! - [`service`] — 上記を束ね、結果を [`TaskService`](questloom_core::service::TaskService)
//!   へ反映する [`AiService`]
//!
//! ```no_run
//! # async fn demo() -> Result<(), questloom_ai::AiError> {
//! use std::time::Duration;
//! use questloom_ai::{exec, prompt, provider};
//! use questloom_core::settings::CoreSettings;
//!
//! let settings = CoreSettings::default();
//! let claude = settings.ai_provider(None).expect("既定プロバイダ");
//! let text = prompt::create_tasks_prompt("水曜までに請求書を出す", chrono::Utc::now().date_naive());
//! let run = provider::prepare(claude, &text, None, Duration::from_secs(300));
//! let output = exec::run(&run.request, &Default::default()).await?;
//! let drafts = prompt::parse_task_drafts(&output.into_stdout(&claude.command)?)?;
//! # let _ = drafts;
//! # Ok(())
//! # }
//! ```

pub mod error;
pub mod exec;
pub mod json;
pub mod prompt;
pub mod provider;
pub mod runner;
pub mod service;

pub use error::{AiError, AiResult};
pub use exec::{run, AiOutput, AiRequest, PromptDelivery};
pub use json::{extract_first_json, parse_first_json};
pub use prompt::{
    create_tasks_prompt, free_instruction_prompt, parse_task_drafts, split_task_prompt, TaskDraft,
};
pub use provider::{prepare, prepare_with, resolve, McpEndpoint, PreparedRun};
pub use runner::{AiFeature, AiRunner, JobGuard};
pub use service::{
    create_from_drafts, ignore_progress, AiCreateResult, AiProgress, AiService, AiTaskSummary,
    AiTextResult, ProgressSink,
};
