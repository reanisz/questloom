//! バージョン付きマイグレーション(前進のみ)。
//!
//! `schema_version` テーブルに現在のバージョンを 1 行だけ保持する。
//! 新しいスキーマ変更は [`MIGRATIONS`] の末尾に追加し、既存の要素は書き換えない。

use rusqlite::{Connection, OptionalExtension};

use crate::error::{StoreError, StoreResult};

/// このバイナリが対応するスキーマバージョン。
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

/// 1 段階のマイグレーション。
struct Migration {
    version: i64,
    sql: &'static str,
}

/// v1: docs/data-model.md の初期スキーマ。
const V1: &str = r"
CREATE TABLE tasks (
    id              TEXT PRIMARY KEY,
    title           TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    status          TEXT NOT NULL,
    scheduled_kind  TEXT,
    scheduled_value TEXT,
    deadline        TEXT,
    is_instant      INTEGER NOT NULL DEFAULT 0,
    origin          TEXT NOT NULL DEFAULT 'user',
    parent_id       TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    sort_order      TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    done_at         TEXT
);
CREATE INDEX idx_tasks_status ON tasks(status);
CREATE INDEX idx_tasks_parent ON tasks(parent_id);

CREATE TABLE task_resources (
    id          TEXT PRIMARY KEY,
    task_id     TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,
    value       TEXT NOT NULL,
    label       TEXT NOT NULL DEFAULT '',
    is_primary  INTEGER NOT NULL DEFAULT 0,
    sort_order  TEXT NOT NULL,
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_resources_task ON task_resources(task_id);

CREATE TABLE task_updates (
    id          TEXT PRIMARY KEY,
    task_id     TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    body        TEXT NOT NULL,
    origin      TEXT NOT NULL DEFAULT 'user',
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_updates_task ON task_updates(task_id);

CREATE TABLE settings (
    namespace   TEXT PRIMARY KEY,
    value       TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE plugin_kv (
    plugin_id   TEXT NOT NULL,
    key         TEXT NOT NULL,
    value       TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (plugin_id, key)
);
";

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: V1,
}];

/// 現在のスキーマバージョンを返す。未初期化なら 0。
///
/// # Errors
/// SQLite のエラー。
pub fn current_version(conn: &Connection) -> StoreResult<i64> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);")?;
    let version = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get::<_, i64>(0)
        })
        .optional()?;
    Ok(version.unwrap_or(0))
}

/// 未適用のマイグレーションをすべて適用する。各段階はトランザクション内で実行される。
///
/// # Errors
/// SQLite のエラー、または DB のバージョンがこのバイナリより新しい場合。
pub fn migrate(conn: &mut Connection) -> StoreResult<i64> {
    let mut version = current_version(conn)?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(StoreError::SchemaTooNew {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }

    let pending: Vec<&Migration> = MIGRATIONS.iter().filter(|m| m.version > version).collect();
    for migration in pending {
        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)?;
        tx.execute("DELETE FROM schema_version", [])?;
        tx.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [migration.version],
        )?;
        tx.commit()?;
        tracing::info!(version = migration.version, "スキーマを移行しました");
        version = migration.version;
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_ordered_and_reach_the_current_version() {
        let mut expected = 0;
        for migration in MIGRATIONS {
            assert!(
                migration.version > expected,
                "マイグレーションは昇順である必要がある"
            );
            expected = migration.version;
        }
        assert_eq!(expected, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn migrate_is_idempotent() {
        let mut conn = Connection::open_in_memory().expect("インメモリ DB を開ける");
        assert_eq!(current_version(&conn).unwrap(), 0);
        assert_eq!(migrate(&mut conn).unwrap(), CURRENT_SCHEMA_VERSION);
        assert_eq!(migrate(&mut conn).unwrap(), CURRENT_SCHEMA_VERSION);
        assert_eq!(current_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);

        // 1 行だけ保持されている。
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn rejects_newer_schema() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn.execute("UPDATE schema_version SET version = 999", [])
            .unwrap();
        assert!(matches!(
            migrate(&mut conn),
            Err(StoreError::SchemaTooNew { found: 999, .. })
        ));
    }
}
