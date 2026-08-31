//! 実行中の AI ジョブを 1 件だけ許すランナー。
//!
//! AI CLI は重く、同時に走らせても待たされるだけなので、実行中の要求は
//! キューに積まず拒否する。キャンセルは [`JobGuard::cancel_token`] を通じて
//! [`exec::run`](crate::exec::run) へ伝わる。

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::error::{AiError, AiResult};

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
    /// UI・メッセージに出す日本語名。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::CreateTasks => "タスク作成",
            Self::SplitTask => "分割/詳細化",
            Self::FreeInstruction => "自由指示",
        }
    }
}

/// 実行中の AI ジョブを 1 件だけ持つ。
///
/// 実行中に別の実行が来たら [`AiError::Busy`] で拒否する(キューには積まない)。
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

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 実行枠を確保する。
    ///
    /// # Errors
    /// 別の AI 実行が進行中の場合 [`AiError::Busy`]。
    pub fn begin(self: &Arc<Self>, feature: AiFeature) -> AiResult<JobGuard> {
        let mut current = Self::lock(&self.current);
        if let Some(job) = current.as_ref() {
            return Err(AiError::Busy {
                running: job.feature,
            });
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

    /// 実行中の機能。実行中でなければ `None`。
    ///
    /// 現時点の呼び出し元は crate 内のテストだけ。デスクトップ側は進捗を
    /// `questloom://ai-status` イベントで受け取っていて、状態を問い合わせる必要が
    /// 無いため配線していない。ランナーの状態を外から確かめられる唯一の口として
    /// 公開したままにしておく(将来 `get_runtime_status` に載せる余地もある)。
    #[must_use]
    pub fn running_feature(&self) -> Option<AiFeature> {
        Self::lock(&self.current).as_ref().map(|job| job.feature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_job_runs_at_a_time() {
        let runner = Arc::new(AiRunner::new());
        assert_eq!(runner.running_feature(), None);
        assert!(!runner.cancel(), "実行中でなければ何もしない");

        let guard = runner.begin(AiFeature::CreateTasks).unwrap();
        assert_eq!(runner.running_feature(), Some(AiFeature::CreateTasks));

        let error = runner.begin(AiFeature::FreeInstruction).unwrap_err();
        assert!(matches!(
            error,
            AiError::Busy {
                running: AiFeature::CreateTasks
            }
        ));
        let message = error.to_string();
        assert!(message.contains("タスク作成"), "{message}");
        assert!(message.contains("実行中"), "{message}");

        assert!(runner.cancel());
        assert!(guard.cancel_token().is_cancelled());

        // ガードを落とすと次の実行が通る。
        drop(guard);
        assert_eq!(runner.running_feature(), None);
        let guard = runner.begin(AiFeature::FreeInstruction).unwrap();
        assert!(!guard.cancel_token().is_cancelled());
    }

    #[test]
    fn features_are_serialized_in_camel_case() {
        assert_eq!(
            serde_json::to_value(AiFeature::SplitTask).unwrap(),
            serde_json::json!("splitTask")
        );
        assert_eq!(
            serde_json::to_value(AiFeature::FreeInstruction).unwrap(),
            serde_json::json!("freeInstruction")
        );
        assert_eq!(
            serde_json::to_value(AiFeature::CreateTasks).unwrap(),
            serde_json::json!("createTasks")
        );
    }
}
