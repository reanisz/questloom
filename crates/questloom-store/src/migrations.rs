//! バージョン付きマイグレーション(前進のみ)。
//!
//! `schema_version` テーブルに現在のバージョンを 1 行だけ保持する。
//! 新しいスキーマ変更は [`MIGRATIONS`] の末尾に追加し、既存の要素は書き換えない。

use rusqlite::{Connection, OptionalExtension};

use crate::error::{StoreError, StoreResult};

/// このバイナリが対応するスキーマバージョン。
pub const CURRENT_SCHEMA_VERSION: i64 = 3;

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

/// v2: タスクのソフトデリート (`tasks.deleted_at`)。NULL = 生存。
///
/// 既存行はすべて NULL(= 生存)になるので、データの書き換えは不要。
const V2: &str = r"
ALTER TABLE tasks ADD COLUMN deleted_at TEXT;
";

/// v3: タスク内チェックリスト (`task_checklist_items`)。
///
/// 新しいテーブルを足すだけなので、既存行の書き換えは不要
/// (チェックリストを持たないタスクは 0 件のまま)。
const V3: &str = r"
CREATE TABLE task_checklist_items (
    id          TEXT PRIMARY KEY,
    task_id     TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    body        TEXT NOT NULL,
    checked     INTEGER NOT NULL DEFAULT 0,
    sort_order  TEXT NOT NULL,
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_checklist_task ON task_checklist_items(task_id);
";

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: V1,
    },
    Migration {
        version: 2,
        sql: V2,
    },
    Migration {
        version: 3,
        sql: V3,
    },
];

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

    /// `status` は自由な TEXT なので、Watching の追加にマイグレーションは要らない。
    ///
    /// v2 のまま置かれていた既存 DB(= マイグレーション無し)に `watching` を
    /// 書き込み、読み戻せることで確かめる。CHECK 制約や enum テーブルを後から
    /// 足すと、この前提が崩れて既存 DB が壊れる。
    #[test]
    fn a_v2_database_accepts_the_watching_status_without_a_migration() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(migrate(&mut conn).unwrap(), CURRENT_SCHEMA_VERSION);
        // ここから先はスキーマを一切変えずに書き込む。
        conn.execute(
            "INSERT INTO tasks (id, title, status, sort_order, created_at, updated_at)
             VALUES ('t1', '見張る', 'watching', 'a0', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("watching を弾く制約が無い");

        let status: String = conn
            .query_row("SELECT status FROM tasks WHERE id = 't1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(status, "watching");
        // 追加のマイグレーションは発生しない。
        assert_eq!(migrate(&mut conn).unwrap(), CURRENT_SCHEMA_VERSION);
    }

    /// 途中バージョンの DB を、そのバージョンだけ適用した状態として用意する。
    fn database_at(version: i64) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        for migration in MIGRATIONS.iter().filter(|m| m.version <= version) {
            conn.execute_batch(migration.sql).unwrap();
        }
        conn.execute_batch("CREATE TABLE schema_version (version INTEGER NOT NULL);")
            .unwrap();
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [version],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (id, title, status, sort_order, created_at, updated_at)
             VALUES ('t1', '既存タスク', 'new', 'a0', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn
    }

    /// テーブルが存在するか。
    fn has_table(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
            > 0
    }

    /// v1 のまま置かれていた DB を開いても、行を失わずに現行バージョンまで上がること。
    ///
    /// v1 → v2(`deleted_at`)→ v3(`task_checklist_items`)を一気に通す。
    #[test]
    fn a_v1_database_is_upgraded_to_the_current_version_without_losing_rows() {
        let mut conn = database_at(1);
        assert!(!has_table(&conn, "task_checklist_items"));

        assert_eq!(migrate(&mut conn).unwrap(), CURRENT_SCHEMA_VERSION);

        // 既存行は残り、deleted_at は NULL(= 生存)になる。
        let (title, deleted_at) = conn
            .query_row(
                "SELECT title, deleted_at FROM tasks WHERE id = 't1'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .unwrap();
        assert_eq!(title, "既存タスク");
        assert_eq!(deleted_at, None);

        // チェックリストのテーブルが増え、既存タスクは 0 件で始まる。
        assert!(has_table(&conn, "task_checklist_items"));
        let items: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM task_checklist_items WHERE task_id = 't1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(items, 0);

        // 上げ直しても壊れない。
        assert_eq!(migrate(&mut conn).unwrap(), CURRENT_SCHEMA_VERSION);
    }

    /// v2 の DB(= ソフトデリートまで入った既存ユーザー)も v3 へ上がること。
    #[test]
    fn a_v2_database_gains_the_checklist_table() {
        let mut conn = database_at(2);
        assert!(!has_table(&conn, "task_checklist_items"));

        assert_eq!(migrate(&mut conn).unwrap(), 3);
        assert!(has_table(&conn, "task_checklist_items"));

        // タスクを消すと、そのチェックリストも一緒に消える (ON DELETE CASCADE)。
        conn.execute("PRAGMA foreign_keys = ON", []).unwrap();
        conn.execute(
            "INSERT INTO task_checklist_items (id, task_id, body, checked, sort_order, created_at)
             VALUES ('c1', 't1', '項目', 0, 'a0', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM tasks WHERE id = 't1'", [])
            .unwrap();
        let items: i64 = conn
            .query_row("SELECT COUNT(*) FROM task_checklist_items", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(items, 0, "タスクの物理削除でカスケードする");
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
