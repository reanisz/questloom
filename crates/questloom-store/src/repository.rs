//! [`TaskRepository`] の SQLite 実装。書き込みはすべてトランザクション内で行う。

use chrono::{DateTime, Utc};
use questloom_core::model::{
    ChecklistItem, ChecklistItemId, ResourceId, Task, TaskId, TaskResource, TaskStatus,
    TaskUpdateEntry,
};
use questloom_core::repository::{RepoResult, TaskRepository};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row, Transaction};

use crate::error::StoreResult;
use crate::mapping::{
    checklist_from_row, checklist_params, resource_from_row, resource_params, task_from_row,
    task_params, time_to_sql, update_from_row, update_params, CHECKLIST_COLUMNS, RESOURCE_COLUMNS,
    TASK_COLUMNS, UPDATE_COLUMNS,
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

const INSERT_CHECKLIST: &str =
    "INSERT INTO task_checklist_items (id, task_id, body, checked, sort_order, created_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6)";

/// `task_id` と `created_at` も書き戻すが、値は変わらない
/// (`checklist_params` の並びを INSERT と共有するため)。
const UPDATE_CHECKLIST: &str =
    "UPDATE task_checklist_items SET task_id = ?2, body = ?3, checked = ?4, sort_order = ?5,
         created_at = ?6
     WHERE id = ?1";

/// 生存しているタスクだけを対象にする条件(ソフトデリートの除外)。
///
/// `find_task` は復元のために削除済みも返す。除外するのはそれ以外の通常クエリ。
const ALIVE: &str = "deleted_at IS NULL";

/// 「`?2` の時刻より前に完了した、生存しているタスク」の条件(`?1` は `status`)。
///
/// `done_at` は RFC3339(UTC・ミリ秒)の固定長テキストなので、文字列の大小比較が
/// そのまま時刻の前後比較になる(`ORDER BY done_at` も同じ理由で使える)。
const DONE_BEFORE: &str =
    "status = ?1 AND done_at IS NOT NULL AND done_at < ?2 AND deleted_at IS NULL";

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

    fn list_done_before(&self, before: DateTime<Utc>, limit: usize) -> RepoResult<Vec<Task>> {
        repo(|| {
            let conn = self.conn();
            let sql = format!(
                "SELECT {TASK_COLUMNS} FROM tasks WHERE {DONE_BEFORE}
                 ORDER BY done_at DESC, id DESC LIMIT ?3"
            );
            // usize が i64 に収まらないほどの limit は来ない(来ても上限として無害)。
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            query_all(
                &conn,
                &sql,
                params![TaskStatus::Done.as_str(), time_to_sql(before), limit],
                task_from_row,
            )
        })
    }

    fn count_done_before(&self, before: DateTime<Utc>) -> RepoResult<usize> {
        repo(|| {
            let conn = self.conn();
            let sql = format!("SELECT COUNT(*) FROM tasks WHERE {DONE_BEFORE}");
            let count: i64 = conn.query_row(
                &sql,
                params![TaskStatus::Done.as_str(), time_to_sql(before)],
                |row| row.get(0),
            )?;
            Ok(usize::try_from(count).unwrap_or(0))
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

    fn list_checklist_items(&self, task_id: TaskId) -> RepoResult<Vec<ChecklistItem>> {
        repo(|| {
            let conn = self.conn();
            let sql = format!(
                "SELECT {CHECKLIST_COLUMNS} FROM task_checklist_items WHERE task_id = ?1
                 ORDER BY sort_order, created_at"
            );
            query_all(&conn, &sql, [task_id.to_string()], checklist_from_row)
        })
    }

    fn list_all_checklist_items(&self) -> RepoResult<Vec<ChecklistItem>> {
        repo(|| {
            let conn = self.conn();
            // ボードの集計に使うので、削除済みタスクの項目は混ぜない
            // (list_all_resources と同じ条件)。
            let sql = format!(
                "SELECT {CHECKLIST_COLUMNS} FROM task_checklist_items c
                 WHERE EXISTS (SELECT 1 FROM tasks t WHERE t.id = c.task_id AND t.{ALIVE})
                 ORDER BY task_id, sort_order, created_at"
            );
            query_all(&conn, &sql, [], checklist_from_row)
        })
    }

    fn insert_checklist_item(&self, item: &ChecklistItem) -> RepoResult<()> {
        repo(|| {
            self.write(|tx| {
                tx.execute(INSERT_CHECKLIST, params_from_iter(checklist_params(item)))?;
                Ok(())
            })
        })
    }

    fn update_checklist_item(&self, item: &ChecklistItem) -> RepoResult<bool> {
        repo(|| {
            self.write(|tx| tx.execute(UPDATE_CHECKLIST, params_from_iter(checklist_params(item))))
                .map(|affected| affected > 0)
        })
    }

    fn delete_checklist_item(&self, id: ChecklistItemId) -> RepoResult<bool> {
        repo(|| {
            self.write(|tx| {
                tx.execute(
                    "DELETE FROM task_checklist_items WHERE id = ?1",
                    [id.to_string()],
                )
            })
            .map(|affected| affected > 0)
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

    /// 過去の完了だけを、新しい順・件数制限つきで拾えること。
    #[test]
    fn done_before_lists_the_newest_completions_within_the_limit() {
        let store = SqliteStore::open_in_memory().unwrap();
        let at = |raw: &str| {
            chrono::DateTime::parse_from_rfc3339(raw)
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let done = |raw: &str| Task {
            status: TaskStatus::Done,
            done_at: Some(at(raw)),
            ..task()
        };

        let oldest = done("2026-09-01T10:00:00Z");
        let newer = done("2026-09-02T10:00:00Z");
        // 境界そのもの(cutoff と同時刻)は「今日の分」なので含めない。
        let boundary = done("2026-09-03T00:00:00Z");
        // 完了しているが削除済み。
        let removed = Task {
            deleted_at: Some(at("2026-09-02T12:00:00Z")),
            ..done("2026-09-01T12:00:00Z")
        };
        // 未完了。done_at も持たない。
        let open = task();
        for task in [&oldest, &newer, &boundary, &removed, &open] {
            store.insert_task_with_resources(task, &[]).unwrap();
        }

        let cutoff = at("2026-09-03T00:00:00Z");
        assert_eq!(store.count_done_before(cutoff).unwrap(), 2);
        let listed = store.list_done_before(cutoff, 10).unwrap();
        assert_eq!(
            listed.iter().map(|t| t.id).collect::<Vec<_>>(),
            [newer.id, oldest.id],
            "完了が新しい順"
        );

        // 上限で切り詰めても、切り詰めるのは古い方。
        let limited = store.list_done_before(cutoff, 1).unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].id, newer.id);
        assert!(store.list_done_before(cutoff, 0).unwrap().is_empty());
    }

    fn checklist_item(task_id: TaskId, body: &str, sort_order: &str) -> ChecklistItem {
        ChecklistItem {
            id: ChecklistItemId::new(),
            task_id,
            body: body.to_owned(),
            checked: false,
            sort_order: sort_order.to_owned(),
            created_at: chrono::Utc::now(),
        }
    }

    /// 挿入 → 一覧(並び順)→ 更新 → 削除の一連。
    #[test]
    fn checklist_items_round_trip_in_sort_order() {
        let store = SqliteStore::open_in_memory().unwrap();
        let task = task();
        store.insert_task_with_resources(&task, &[]).unwrap();

        // わざと並び順の逆に入れて、一覧が sort_order 昇順で返ることを見る。
        let second = checklist_item(task.id, "2 番目", "a1");
        let first = checklist_item(task.id, "1 番目", "a0");
        store.insert_checklist_item(&second).unwrap();
        store.insert_checklist_item(&first).unwrap();

        let listed = store.list_checklist_items(task.id).unwrap();
        assert_eq!(
            listed.iter().map(|i| i.body.as_str()).collect::<Vec<_>>(),
            ["1 番目", "2 番目"]
        );
        assert!(listed.iter().all(|item| !item.checked));

        // 更新(チェックと本文と並び順)。
        let updated = ChecklistItem {
            checked: true,
            body: "書き換えた".to_owned(),
            sort_order: "a2".to_owned(),
            ..first.clone()
        };
        assert!(store.update_checklist_item(&updated).unwrap());
        let listed = store.list_checklist_items(task.id).unwrap();
        assert_eq!(
            listed.iter().map(|i| i.body.as_str()).collect::<Vec<_>>(),
            ["2 番目", "書き換えた"]
        );
        assert!(listed[1].checked);

        // 存在しない項目の更新・削除は false。
        let missing = checklist_item(task.id, "無い", "a9");
        assert!(!store.update_checklist_item(&missing).unwrap());
        assert!(!store.delete_checklist_item(missing.id).unwrap());

        assert!(store.delete_checklist_item(first.id).unwrap());
        assert_eq!(store.list_checklist_items(task.id).unwrap().len(), 1);
    }

    /// 全件取得はタスクごとにまとまり、削除済みタスクの項目は落ちる。
    #[test]
    fn all_checklist_items_skip_deleted_tasks() {
        let store = SqliteStore::open_in_memory().unwrap();
        let kept = task();
        let removed = task();
        store.insert_task_with_resources(&kept, &[]).unwrap();
        store.insert_task_with_resources(&removed, &[]).unwrap();
        store
            .insert_checklist_item(&checklist_item(kept.id, "残る", "a0"))
            .unwrap();
        store
            .insert_checklist_item(&checklist_item(removed.id, "消える", "a0"))
            .unwrap();
        assert_eq!(store.list_all_checklist_items().unwrap().len(), 2);

        store
            .update_task(&Task {
                deleted_at: Some(chrono::Utc::now()),
                ..removed.clone()
            })
            .unwrap();

        let listed = store.list_all_checklist_items().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].task_id, kept.id);
        // 個別取得は残る(復元すれば戻るため)。
        assert_eq!(store.list_checklist_items(removed.id).unwrap().len(), 1);
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
