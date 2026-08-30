//! questloom の SQLite 永続化層。スキーマのマイグレーションとバックアップを担当する。
//!
//! - 接続時に `journal_mode=WAL` / `synchronous=NORMAL` / `foreign_keys=ON` を設定する。
//! - スキーマは `schema_version` による前進のみのマイグレーション([`migrations`])。
//! - 書き込みはすべてトランザクション内で行う。
//! - [`backup`] は SQLite Online Backup API による世代付きバックアップを提供する。

pub mod backup;
mod error;
mod mapping;
pub mod migrations;
mod repository;

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Serialize;

pub use error::{StoreError, StoreResult};
pub use migrations::CURRENT_SCHEMA_VERSION;

/// SQLite による [`questloom_core::repository::TaskRepository`] の実装。
///
/// 接続は内部の [`Mutex`] で保護されており、`&self` から安全に使える。
#[derive(Debug)]
pub struct SqliteStore {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl SqliteStore {
    /// DB ファイルを開き(必要なら作成し)、PRAGMA 設定とマイグレーションを行う。
    ///
    /// # Errors
    /// 親ディレクトリの作成に失敗、SQLite のエラー、
    /// または DB のスキーマがこのバイナリより新しい場合。
    pub fn open(path: impl Into<PathBuf>) -> StoreResult<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
        }
        let conn = Connection::open(&path)?;
        Self::from_connection(conn, path)
    }

    /// インメモリ DB を開く(テスト用)。
    ///
    /// # Errors
    /// SQLite のエラー。
    pub fn open_in_memory() -> StoreResult<Self> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(conn, PathBuf::from(":memory:"))
    }

    fn from_connection(mut conn: Connection, path: PathBuf) -> StoreResult<Self> {
        configure(&conn)?;
        migrations::migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            path,
        })
    }

    /// DB ファイルのパス。
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 現在のスキーマバージョン。
    ///
    /// # Errors
    /// SQLite のエラー。
    pub fn schema_version(&self) -> StoreResult<i64> {
        migrations::current_version(&self.conn())
    }

    /// 名前空間付き設定を生の JSON 文字列として読む。
    ///
    /// # Errors
    /// SQLite のエラー。
    pub fn get_settings_json(&self, namespace: &str) -> StoreResult<Option<String>> {
        let conn = self.conn();
        let value = conn
            .query_row(
                "SELECT value FROM settings WHERE namespace = ?1",
                [namespace],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(value)
    }

    /// 名前空間付き設定を生の JSON 文字列として書く(upsert)。
    ///
    /// # Errors
    /// SQLite のエラー。
    pub fn set_settings_json(&self, namespace: &str, value: &str) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO settings (namespace, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(namespace) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            rusqlite::params![namespace, value, mapping::time_to_sql(Utc::now())],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// 名前空間付き設定を型付きで読む。未保存・パース不能なら既定値を返す。
    ///
    /// 壊れた設定でアプリが起動できなくなるのを避けるため、パース失敗は
    /// 警告ログを出して既定値にフォールバックする。
    ///
    /// # Errors
    /// SQLite のエラー。
    pub fn get_settings<T: DeserializeOwned + Default>(&self, namespace: &str) -> StoreResult<T> {
        let Some(raw) = self.get_settings_json(namespace)? else {
            return Ok(T::default());
        };
        match serde_json::from_str(&raw) {
            Ok(value) => Ok(value),
            Err(error) => {
                tracing::warn!(
                    namespace,
                    %error,
                    "設定 JSON を解釈できないため既定値を使用します"
                );
                Ok(T::default())
            }
        }
    }

    /// 名前空間付き設定を型付きで書く。
    ///
    /// # Errors
    /// JSON 化に失敗した場合、または SQLite のエラー。
    pub fn set_settings<T: Serialize>(&self, namespace: &str, value: &T) -> StoreResult<()> {
        let json = serde_json::to_string(value).map_err(|source| StoreError::Json {
            namespace: namespace.to_owned(),
            source,
        })?;
        self.set_settings_json(namespace, &json)
    }

    /// WAL のチェックポイントを実行し、`-wal` の内容を本体へ取り込む。
    ///
    /// # Errors
    /// SQLite のエラー。
    pub fn checkpoint(&self) -> StoreResult<()> {
        self.conn()
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    /// 内部の接続を借りる。Mutex が毒されていても処理を継続する。
    pub(crate) fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// 接続に対する PRAGMA 設定。
fn configure(conn: &Connection) -> StoreResult<()> {
    // journal_mode は値を返すため execute ではなく query_row で実行する。
    // インメモリ DB では "memory" が返るが、それは正常。
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    tracing::debug!(journal_mode = %mode, "SQLite の journal_mode を設定しました");
    conn.execute_batch(
        "PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use questloom_core::settings::{CoreSettings, WeekStart, CORE_NAMESPACE};

    #[test]
    fn open_applies_pragmas_and_migrations() {
        let store = SqliteStore::open_in_memory().expect("DB を開ける");
        assert_eq!(store.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);

        let conn = store.conn();
        let foreign_keys: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1);
    }

    #[test]
    fn settings_roundtrip() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(
            store.get_settings::<CoreSettings>(CORE_NAMESPACE).unwrap(),
            CoreSettings::default()
        );

        let settings = CoreSettings {
            week_start: WeekStart::Sunday,
            backup_generations: 7,
        };
        store.set_settings(CORE_NAMESPACE, &settings).unwrap();
        assert_eq!(
            store.get_settings::<CoreSettings>(CORE_NAMESPACE).unwrap(),
            settings
        );

        // 上書きできる。
        store
            .set_settings(CORE_NAMESPACE, &CoreSettings::default())
            .unwrap();
        assert_eq!(
            store.get_settings::<CoreSettings>(CORE_NAMESPACE).unwrap(),
            CoreSettings::default()
        );
    }

    #[test]
    fn settings_namespaces_are_independent() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .set_settings_json("core", r#"{"weekStart":"sunday"}"#)
            .unwrap();
        store
            .set_settings_json("plugin:github", r#"{"intervalMinutes":5}"#)
            .unwrap();
        assert_eq!(
            store.get_settings_json("core").unwrap().as_deref(),
            Some(r#"{"weekStart":"sunday"}"#)
        );
        assert_eq!(
            store.get_settings_json("plugin:github").unwrap().as_deref(),
            Some(r#"{"intervalMinutes":5}"#)
        );
        assert_eq!(store.get_settings_json("missing").unwrap(), None);
    }

    #[test]
    fn broken_settings_fall_back_to_defaults() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.set_settings_json(CORE_NAMESPACE, "not json").unwrap();
        assert_eq!(
            store.get_settings::<CoreSettings>(CORE_NAMESPACE).unwrap(),
            CoreSettings::default()
        );
    }
}
