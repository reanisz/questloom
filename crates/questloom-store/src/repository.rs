//! [`TaskRepository`] の SQLite 実装。書き込みはすべてトランザクション内で行う。

use questloom_core::model::{ResourceId, Task, TaskId, TaskResource, TaskStatus, TaskUpdateEntry};
use questloom_core::repository::{RepoResult, TaskRepository};
use rusqlite::{params_from_iter, Connection, OptionalExtension, Row, Transaction};

use crate::error::StoreResult;
use crate::mapping::{
    resource_from_row, resource_params, task_from_row, task_params, update_from_row, update_params,
    RESOURCE_COLUMNS, TASK_COLUMNS, UPDATE_COLUMNS,
};
use crate::SqliteStore;

const INSERT_TASK: &str =
    "INSERT INTO tasks (id, title, description, status, scheduled_kind, scheduled_value,
         deadline, is_instant, origin, parent_id, sort_order, created_at, updated_at, done_at,
         deleted_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)";

/// 削除・復元も「`deleted_at` を含めた行の書き戻し」なので、この 1 本で足りる。
const UPDATE_TASK: &str =
    "UPDATE tasks SET title = ?2, description = ?3, status = ?4, scheduled_kind = ?5,
         scheduled_value = ?6, deadline = ?7, is_instant = ?8, origin = ?9, parent_id = ?10,
         sort_order = ?11, created_at = ?12, updated_at = ?13, done_at = ?14, deleted_at = ?15
     WHERE id = ?1";

const INSERT_RESOURCE: &str =
    "INSERT INTO task_resources (id, task_id, kind, value, label, is_primary, sort_order, created_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)";

const INSERT_UPDATE: &str = "INSERT INTO task_updates (id, task_id, body, origin, created_at)
     VALUES (?1, ?2, ?3, ?4, ?5)";

/// 生存しているタスクだけを対象にする条件(ソフトデリートの除外)。
///
/// `find_task` は復元のために削除済みも返す。除外するのはそれ以外の通常クエリ。
const ALIVE: &str = "deleted_at IS NULL";

/// 指定タスクの主リソースをすべて解除する。
const CLEAR_PRIMARY: &str =
    "UPDATE task_resources SET is_primary = 0 WHERE task_id = ?1 AND is_primary <> 0";

/// ストア層の処理を実行し、エラーをリポジトリ層のエラーへ移す。
///
/// `TaskRepository` の各メソッドはこれで包むだけで、実装本体は trait impl に直接書く
/// (別名のメソッドを作って委譲する二重定義を避けるため)。
fn repo<T>(body: impl FnOnce() -> StoreResult<T>) -> RepoResult<T> {
    body().map_err(Into::into)
}

fn query_all<T>(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
    map: impl Fn(&Row<'_>) -> rusqlite::Result<T>,
) -> StoreResult<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |row| map(row))?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
}

impl SqliteStore {
    /// 1 つのトランザクションで書き込みを行い、成功したらコミットする。
    fn write<T>(
        &self,
        body: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
    ) -> StoreResult<T> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let value = body(&tx)?;
        tx.commit()?;
        Ok(value)
    }
}

impl TaskRepository for SqliteStore {
    fn insert_task_with_resources(
        &self,
        task: &Task,
        resources: &[TaskResource],
    ) -> RepoResult<()> {
        repo(|| {
            self.write(|tx| {
                tx.execute(INSERT_TASK, params_from_iter(task_params(task)))?;
                for resource in resources {
                    tx.execute(INSERT_RESOURCE, params_from_iter(resource_params(resource)))?;
                }
                Ok(())
            })
        })
    }

    fn update_task(&self, task: &Task) -> RepoResult<bool> {
        repo(|| {
            self.write(|tx| tx.execute(UPDATE_TASK, params_from_iter(task_params(task))))
                .map(|affected| affected > 0)
        })
    }

    fn find_task(&self, id: TaskId) -> RepoResult<Option<Task>> {
        repo(|| {
            let conn = self.conn();
            let sql = format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1");
            conn.query_row(&sql, [id.to_string()], task_from_row)
                .optional()
                .map_err(Into::into)
        })
    }

    fn list_tasks(&self) -> RepoResult<Vec<Task>> {
        repo(|| {
            let conn = self.conn();
            let sql = format!(
                "SELECT {TASK_COLUMNS} FROM tasks WHERE {ALIVE} ORDER BY sort_order, created_at"
            );
            query_all(&conn, &sql, [], task_from_row)
        })
    }

    fn list_tasks_by_status(&self, status: TaskStatus) -> RepoResult<Vec<Task>> {
        repo(|| {
            let conn = self.conn();
            let sql = format!(
                "SELECT {TASK_COLUMNS} FROM tasks WHERE status = ?1 AND {ALIVE}
                 ORDER BY sort_order, created_at"
            );
            query_all(&conn, &sql, [status.as_str()], task_from_row)
        })
    }

    fn list_children(&self, parent_id: TaskId) -> RepoResult<Vec<Task>> {
        repo(|| {
            let conn = self.conn();
            let sql = format!(
                "SELECT {TASK_COLUMNS} FROM tasks WHERE parent_id = ?1 AND {ALIVE}
                 ORDER BY sort_order, created_at"
            );
            query_all(&conn, &sql, [parent_id.to_string()], task_from_row)
        })
    }

    fn list_deleted_tasks(&self) -> RepoResult<Vec<Task>> {
        repo(|| {
            let conn = self.conn();
            // deleted_at は RFC3339(UTC・ミリ秒)なので、文字列の降順が時刻の降順になる。
            let sql = format!(
                "SELECT {TASK_COLUMNS} FROM tasks WHERE deleted_at IS NOT NULL
                 ORDER BY deleted_at DESC, id DESC"
            );
            query_all(&conn, &sql, [], task_from_row)
        })
    }

    fn replace_primary_and_insert(&self, resource: &TaskResource) -> RepoResult<()> {
        repo(|| {
            self.write(|tx| {
                if resource.is_primary {
                    // 主リソースは 1 タスクに 1 つ。挿入と同じトランザクションで解除する。
                    tx.execute(CLEAR_PRIMARY, [resource.task_id.to_string()])?;
                }
                tx.execute(INSERT_RESOURCE, params_from_iter(resource_params(resource)))?;
                Ok(())
            })
        })
    }

    fn delete_resource(&self, id: ResourceId) -> RepoResult<bool> {
        repo(|| {
            self.write(|tx| {
                tx.execute("DELETE FROM task_resources WHERE id = ?1", [id.to_string()])
            })
            .map(|affected| affected > 0)
        })
    }

    fn list_resources(&self, task_id: TaskId) -> RepoResult<Vec<TaskResource>> {
        repo(|| {
            let conn = self.conn();
            let sql = format!(
                "SELECT {RESOURCE_COLUMNS} FROM task_resources WHERE task_id = ?1 ORDER BY sort_order, created_at"
            );
            query_all(&conn, &sql, [task_id.to_string()], resource_from_row)
        })
    }

    fn list_all_resources(&self) -> RepoResult<Vec<TaskResource>> {
        repo(|| {
            let conn = self.conn();
            // ボードの集計に使うので、削除済みタスクのリソースは混ぜない。
            let sql = format!(
                "SELECT {RESOURCE_COLUMNS} FROM task_resources r
                 WHERE EXISTS (SELECT 1 FROM tasks t WHERE t.id = r.task_id AND t.{ALIVE})
                 ORDER BY task_id, sort_order, created_at"
            );
            query_all(&conn, &sql, [], resource_from_row)
        })
    }

    fn insert_update(&self, entry: &TaskUpdateEntry) -> RepoResult<()> {
        repo(|| {
            self.write(|tx| {
                tx.execute(INSERT_UPDATE, params_from_iter(update_params(entry)))?;
                Ok(())
            })
        })
    }

    fn list_updates(&self, task_id: TaskId) -> RepoResult<Vec<TaskUpdateEntry>> {
        repo(|| {
            let conn = self.conn();
            let sql = format!(
                "SELECT {UPDATE_COLUMNS} FROM task_updates WHERE task_id = ?1 ORDER BY created_at, id"
            );
            query_all(&conn, &sql, [task_id.to_string()], update_from_row)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use questloom_core::model::{Origin, ResourceKind, Scheduled};
    use questloom_core::sort_order::FIRST_KEY;

    fn task() -> Task {
        let now = chrono::Utc::now();
        Task {
            id: TaskId::new(),
            title: "原子性".to_owned(),
            description: String::new(),
            status: TaskStatus::New,
            scheduled: Scheduled::None,
            deadline: None,
            is_instant: false,
            origin: Origin::User,
            parent_id: None,
            sort_order: FIRST_KEY.to_owned(),
            created_at: now,
            updated_at: now,
            done_at: None,
            deleted_at: None,
        }
    }

    fn resource(task_id: TaskId, value: &str, is_primary: bool) -> TaskResource {
        TaskResource {
            id: ResourceId::new(),
            task_id,
            kind: ResourceKind::Url,
            value: value.to_owned(),
            label: String::new(),
            is_primary,
            sort_order: FIRST_KEY.to_owned(),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn a_failed_resource_insert_rolls_back_the_task() {
        let store = SqliteStore::open_in_memory().unwrap();
        let task = task();
        let duplicate = resource(task.id, "https://example.com", true);

        // 同じ ID のリソースを 2 件渡すと主キー違反で失敗する。
        let error = store
            .insert_task_with_resources(&task, &[duplicate.clone(), duplicate])
            .expect_err("主キー違反になる");
        assert!(error.to_string().contains("SQLite"), "{error}");

        // タスクごと巻き戻り、中途半端な行は残らない。
        assert_eq!(store.find_task(task.id).unwrap(), None);
        assert!(store.list_all_resources().unwrap().is_empty());
    }

    /// 通常クエリからの削除済み除外はリポジトリの責務。
    #[test]
    fn soft_deleted_tasks_are_excluded_from_the_normal_queries() {
        let store = SqliteStore::open_in_memory().unwrap();
        let parent = task();
        let mut child = task();
        child.parent_id = Some(parent.id);
        store.insert_task_with_resources(&parent, &[]).unwrap();
        store
            .insert_task_with_resources(&child, &[resource(child.id, "https://example.com", true)])
            .unwrap();

        let mut deleted = child.clone();
        deleted.deleted_at = Some(chrono::Utc::now());
        assert!(store.update_task(&deleted).unwrap());

        assert_eq!(store.list_tasks().unwrap().len(), 1);
        assert_eq!(
            store.list_tasks_by_status(TaskStatus::New).unwrap().len(),
            1
        );
        assert!(store.list_children(parent.id).unwrap().is_empty());
        assert!(store.list_all_resources().unwrap().is_empty());

        // find_task は復元のために削除済みも返す。個別のリソース取得も残る。
        let found = store
            .find_task(child.id)
            .unwrap()
            .expect("行は消えていない");
        assert!(found.is_deleted());
        assert_eq!(store.list_resources(child.id).unwrap().len(), 1);

        // 削除済み一覧に出る。
        let listed = store.list_deleted_tasks().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, child.id);

        // 復元すれば元に戻る。
        assert!(store.update_task(&child).unwrap());
        assert_eq!(store.list_tasks().unwrap().len(), 2);
        assert_eq!(store.list_children(parent.id).unwrap().len(), 1);
        assert_eq!(store.list_all_resources().unwrap().len(), 1);
        assert!(store.list_deleted_tasks().unwrap().is_empty());
    }

    /// 削除済み一覧は「新しく消したものが先」。
    #[test]
    fn deleted_tasks_are_listed_newest_first() {
        let store = SqliteStore::open_in_memory().unwrap();
        let older = task();
        let newer = task();
        store.insert_task_with_resources(&older, &[]).unwrap();
        store.insert_task_with_resources(&newer, &[]).unwrap();

        let at = |raw: &str| {
            Some(
                chrono::DateTime::parse_from_rfc3339(raw)
                    .unwrap()
                    .with_timezone(&chrono::Utc),
            )
        };
        store
            .update_task(&Task {
                deleted_at: at("2026-09-01T00:00:00Z"),
                ..older.clone()
            })
            .unwrap();
        store
            .update_task(&Task {
                deleted_at: at("2026-09-05T00:00:00Z"),
                ..newer.clone()
            })
            .unwrap();

        let listed = store.list_deleted_tasks().unwrap();
        let ids: Vec<TaskId> = listed.iter().map(|t| t.id).collect();
        assert_eq!(ids, [newer.id, older.id]);
    }

    #[test]
    fn inserting_a_primary_resource_demotes_the_previous_one() {
        let store = SqliteStore::open_in_memory().unwrap();
        let task = task();
        let first = resource(task.id, "https://example.com/a", true);
        store
            .insert_task_with_resources(&task, std::slice::from_ref(&first))
            .unwrap();

        store
            .replace_primary_and_insert(&resource(task.id, "https://example.com/b", true))
            .unwrap();

        let resources = store.list_resources(task.id).unwrap();
        assert_eq!(resources.len(), 2);
        let primaries: Vec<&str> = resources
            .iter()
            .filter(|r| r.is_primary)
            .map(|r| r.value.as_str())
            .collect();
        assert_eq!(primaries, ["https://example.com/b"]);

        // 主リソースでない挿入は既存を解除しない。
        store
            .replace_primary_and_insert(&resource(task.id, "https://example.com/c", false))
            .unwrap();
        assert_eq!(
            store
                .list_resources(task.id)
                .unwrap()
                .iter()
                .filter(|r| r.is_primary)
                .count(),
            1
        );
    }
}
