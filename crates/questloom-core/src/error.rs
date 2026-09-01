//! ドメインエラー型。

use crate::model::{ChecklistItemId, ParseDomainError, ResourceId, TaskId};
use crate::repository::RepositoryError;
use crate::sort_order::SortKeyError;

/// [`CoreError`] を返す `Result` の別名。
pub type CoreResult<T> = Result<T, CoreError>;

/// サービス層が返すドメインエラー。
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// 指定 ID のタスクが存在しない。
    #[error("タスクが見つかりません: {0}")]
    TaskNotFound(TaskId),

    /// 削除済み(ソフトデリート済み)のタスクを操作しようとした。
    ///
    /// 復元 ([`TaskService::restore_task`](crate::service::TaskService::restore_task))
    /// 以外の操作は受け付けない。
    #[error("タスクは削除済みです: {0}")]
    TaskDeleted(TaskId),

    /// 指定 ID のリソースが存在しない。
    #[error("リソースが見つかりません: {0}")]
    ResourceNotFound(ResourceId),

    /// 自分自身を親にしようとした。
    #[error("タスクを自分自身の親にはできません: {0}")]
    SelfParent(TaskId),

    /// 親子リンクが循環する。
    #[error("親子リンクが循環します: {child} -> {parent}")]
    ParentCycle {
        /// 子タスク。
        child: TaskId,
        /// 親にしようとしたタスク。
        parent: TaskId,
    },

    /// 指定 ID のチェックリスト項目が(そのタスクに)存在しない。
    #[error("チェックリスト項目が見つかりません: {0}")]
    ChecklistItemNotFound(ChecklistItemId),

    /// タイトルが空。
    #[error("タイトルは空にできません")]
    EmptyTitle,

    /// チェックリスト項目の本文が空。
    #[error("チェックリスト項目の本文は空にできません")]
    EmptyChecklistBody,

    /// インスタントタスクでないものを昇格しようとした。
    #[error("インスタントタスクではありません: {0}")]
    NotInstant(TaskId),

    /// 並び順キーの生成に失敗した。
    #[error(transparent)]
    SortKey(#[from] SortKeyError),

    /// 永続化層のエラー。
    #[error(transparent)]
    Repository(#[from] RepositoryError),

    /// ドメイン値のパースに失敗した。
    #[error(transparent)]
    Parse(#[from] ParseDomainError),
}
