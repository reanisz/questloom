//! 永続化の抽象。実装は questloom-store が提供する。

use std::error::Error as StdError;
use std::fmt;

use crate::model::{ResourceId, Task, TaskId, TaskResource, TaskStatus, TaskUpdateEntry};

/// リポジトリ操作の `Result` 別名。
pub type RepoResult<T> = Result<T, RepositoryError>;

/// 永続化層で発生したエラー。core は具体的なストレージ実装を知らないため、
/// メッセージと元エラーだけを保持する。
#[derive(Debug)]
pub struct RepositoryError {
    message: String,
    source: Option<Box<dyn StdError + Send + Sync + 'static>>,
}

impl RepositoryError {
    /// メッセージのみのエラーを作る。
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    /// 元エラーを保持したエラーを作る。
    pub fn with_source(
        message: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.source {
            Some(source) => write!(f, "{}: {source}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl StdError for RepositoryError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_ref()
            .map(|source| source.as_ref() as &(dyn StdError + 'static))
    }
}

/// タスク・リソース・アップデート履歴の永続化。
///
/// 同期 API。実装側は内部で必要な排他制御(接続の Mutex など)を行うこと。
/// 書き込み系メソッドはそれぞれトランザクション内で実行される。
pub trait TaskRepository: Send + Sync + 'static {
    /// タスクを 1 件挿入する。
    ///
    /// # Errors
    /// 永続化に失敗した場合。
    fn insert_task(&self, task: &Task) -> RepoResult<()>;

    /// タスクを 1 件更新する。存在しない場合は `Ok(false)`。
    ///
    /// # Errors
    /// 永続化に失敗した場合。
    fn update_task(&self, task: &Task) -> RepoResult<bool>;

    /// タスクを 1 件取得する。
    ///
    /// # Errors
    /// 永続化に失敗した場合。
    fn find_task(&self, id: TaskId) -> RepoResult<Option<Task>>;

    /// 全タスクを `sort_order` 昇順で返す。
    ///
    /// # Errors
    /// 永続化に失敗した場合。
    fn list_tasks(&self) -> RepoResult<Vec<Task>>;

    /// 指定ステータスのタスクを `sort_order` 昇順で返す。
    ///
    /// # Errors
    /// 永続化に失敗した場合。
    fn list_tasks_by_status(&self, status: TaskStatus) -> RepoResult<Vec<Task>>;

    /// 子タスクを `sort_order` 昇順で返す。
    ///
    /// # Errors
    /// 永続化に失敗した場合。
    fn list_children(&self, parent_id: TaskId) -> RepoResult<Vec<Task>>;

    /// リソースを 1 件挿入する。
    ///
    /// # Errors
    /// 永続化に失敗した場合。
    fn insert_resource(&self, resource: &TaskResource) -> RepoResult<()>;

    /// リソースを 1 件更新する。存在しない場合は `Ok(false)`。
    ///
    /// # Errors
    /// 永続化に失敗した場合。
    fn update_resource(&self, resource: &TaskResource) -> RepoResult<bool>;

    /// リソースを 1 件削除する。存在しない場合は `Ok(false)`。
    ///
    /// # Errors
    /// 永続化に失敗した場合。
    fn delete_resource(&self, id: ResourceId) -> RepoResult<bool>;

    /// タスクのリソースを `sort_order` 昇順で返す。
    ///
    /// # Errors
    /// 永続化に失敗した場合。
    fn list_resources(&self, task_id: TaskId) -> RepoResult<Vec<TaskResource>>;

    /// 全リソースを `(task_id, sort_order)` 昇順で返す(ボード表示用)。
    ///
    /// # Errors
    /// 永続化に失敗した場合。
    fn list_all_resources(&self) -> RepoResult<Vec<TaskResource>>;

    /// アップデート履歴を 1 件挿入する。
    ///
    /// # Errors
    /// 永続化に失敗した場合。
    fn insert_update(&self, entry: &TaskUpdateEntry) -> RepoResult<()>;

    /// タスクのアップデート履歴を古い順に返す。
    ///
    /// # Errors
    /// 永続化に失敗した場合。
    fn list_updates(&self, task_id: TaskId) -> RepoResult<Vec<TaskUpdateEntry>>;
}
