//! ドメインエラー型。

use crate::model::{ParseDomainError, ResourceId, TaskId};
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

    /// タイトルが空。
    #[error("タイトルは空にできません")]
    EmptyTitle,

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
