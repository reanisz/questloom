//! ストアのエラー型。

use std::path::PathBuf;

use questloom_core::repository::RepositoryError;

/// [`StoreError`] を返す `Result` の別名。
pub type StoreResult<T> = Result<T, StoreError>;

/// SQLite 永続化層のエラー。
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite のエラー。
    #[error("SQLite エラー: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// ファイル操作のエラー。
    #[error("ファイル操作に失敗しました ({path}): {source}")]
    Io {
        /// 対象パス。
        path: PathBuf,
        /// 元エラー。
        #[source]
        source: std::io::Error,
    },

    /// JSON のシリアライズ/デシリアライズに失敗した。
    #[error("設定 JSON の変換に失敗しました (namespace={namespace}): {source}")]
    Json {
        /// 名前空間。
        namespace: String,
        /// 元エラー。
        #[source]
        source: serde_json::Error,
    },

    /// DB のスキーマがこのバイナリより新しい(前進のみのため対応不可)。
    #[error(
        "DB のスキーマバージョン {found} はこのバイナリ ({supported}) より新しいため開けません"
    )]
    SchemaTooNew {
        /// DB 側のバージョン。
        found: i64,
        /// バイナリが対応する最大バージョン。
        supported: i64,
    },

    /// 保存されている値がドメインモデルへ変換できない。
    #[error("保存データの変換に失敗しました: {0}")]
    Decode(String),
}

impl From<StoreError> for RepositoryError {
    fn from(error: StoreError) -> Self {
        Self::with_source("永続化層でエラーが発生しました", error)
    }
}
