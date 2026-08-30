//! [`TaskRepository`] の SQLite 実装。書き込みはすべてトランザクション内で行う。

use questloom_core::model::{ResourceId, Task, TaskId, TaskResource, TaskStatus, TaskUpdateEntry};
use questloom_core::repository::{RepoResult, TaskRepository};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::StoreResult;
use crate::mapping::{
    resource_from_row, task_from_row, time_to_sql, update_from_row, RESOURCE_COLUMNS, TASK_COLUMNS,
    UPDATE_COLUMNS,
};
use crate::SqliteStore;

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
    fn insert_task_inner(&self, task: &Task) -> StoreResult<()> {
        let (kind, value) = task.scheduled.to_columns();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO tasks (id, title, description, status, scheduled_kind, scheduled_value,
                 deadline, is_instant, origin, parent_id, sort_order, created_at, updated_at, done_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                task.id.to_string(),
                task.title,
                task.description,
                task.status.as_str(),
                kind,
                value,
                task.deadline.map(time_to_sql),
                i64::from(task.is_instant),
                task.origin.to_string(),
                task.parent_id.map(|id| id.to_string()),
                task.sort_order,
                time_to_sql(task.created_at),
                time_to_sql(task.updated_at),
                task.done_at.map(time_to_sql),
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn update_task_inner(&self, task: &Task) -> StoreResult<bool> {
        let (kind, value) = task.scheduled.to_columns();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let affected = tx.execute(
            "UPDATE tasks SET title = ?2, description = ?3, status = ?4, scheduled_kind = ?5,
                 scheduled_value = ?6, deadline = ?7, is_instant = ?8, origin = ?9, parent_id = ?10,
                 sort_order = ?11, created_at = ?12, updated_at = ?13, done_at = ?14
             WHERE id = ?1",
            params![
                task.id.to_string(),
                task.title,
                task.description,
                task.status.as_str(),
                kind,
                value,
                task.deadline.map(time_to_sql),
                i64::from(task.is_instant),
                task.origin.to_string(),
                task.parent_id.map(|id| id.to_string()),
                task.sort_order,
                time_to_sql(task.created_at),
                time_to_sql(task.updated_at),
                task.done_at.map(time_to_sql),
            ],
        )?;
        tx.commit()?;
        Ok(affected > 0)
    }

    fn find_task_inner(&self, id: TaskId) -> StoreResult<Option<Task>> {
        let conn = self.conn();
        let sql = format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1");
        let task = conn
            .query_row(&sql, [id.to_string()], task_from_row)
            .optional()?;
        Ok(task)
    }

    fn list_tasks_inner(&self) -> StoreResult<Vec<Task>> {
        let conn = self.conn();
        let sql = format!("SELECT {TASK_COLUMNS} FROM tasks ORDER BY sort_order, created_at");
        query_all(&conn, &sql, [], task_from_row)
    }

    fn list_tasks_by_status_inner(&self, status: TaskStatus) -> StoreResult<Vec<Task>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE status = ?1 ORDER BY sort_order, created_at"
        );
        query_all(&conn, &sql, [status.as_str()], task_from_row)
    }

    fn list_children_inner(&self, parent_id: TaskId) -> StoreResult<Vec<Task>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE parent_id = ?1 ORDER BY sort_order, created_at"
        );
        query_all(&conn, &sql, [parent_id.to_string()], task_from_row)
    }

    fn insert_resource_inner(&self, resource: &TaskResource) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO task_resources (id, task_id, kind, value, label, is_primary, sort_order, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                resource.id.to_string(),
                resource.task_id.to_string(),
                resource.kind.as_str(),
                resource.value,
                resource.label,
                i64::from(resource.is_primary),
                resource.sort_order,
                time_to_sql(resource.created_at),
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn update_resource_inner(&self, resource: &TaskResource) -> StoreResult<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let affected = tx.execute(
            "UPDATE task_resources SET task_id = ?2, kind = ?3, value = ?4, label = ?5,
                 is_primary = ?6, sort_order = ?7, created_at = ?8
             WHERE id = ?1",
            params![
                resource.id.to_string(),
                resource.task_id.to_string(),
                resource.kind.as_str(),
                resource.value,
                resource.label,
                i64::from(resource.is_primary),
                resource.sort_order,
                time_to_sql(resource.created_at),
            ],
        )?;
        tx.commit()?;
        Ok(affected > 0)
    }

    fn delete_resource_inner(&self, id: ResourceId) -> StoreResult<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let affected = tx.execute("DELETE FROM task_resources WHERE id = ?1", [id.to_string()])?;
        tx.commit()?;
        Ok(affected > 0)
    }

    fn list_resources_inner(&self, task_id: TaskId) -> StoreResult<Vec<TaskResource>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {RESOURCE_COLUMNS} FROM task_resources WHERE task_id = ?1 ORDER BY sort_order, created_at"
        );
        query_all(&conn, &sql, [task_id.to_string()], resource_from_row)
    }

    fn list_all_resources_inner(&self) -> StoreResult<Vec<TaskResource>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {RESOURCE_COLUMNS} FROM task_resources ORDER BY task_id, sort_order, created_at"
        );
        query_all(&conn, &sql, [], resource_from_row)
    }

    fn insert_update_inner(&self, entry: &TaskUpdateEntry) -> StoreResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO task_updates (id, task_id, body, origin, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                entry.id.to_string(),
                entry.task_id.to_string(),
                entry.body,
                entry.origin.to_string(),
                time_to_sql(entry.created_at),
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn list_updates_inner(&self, task_id: TaskId) -> StoreResult<Vec<TaskUpdateEntry>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {UPDATE_COLUMNS} FROM task_updates WHERE task_id = ?1 ORDER BY created_at, id"
        );
        query_all(&conn, &sql, [task_id.to_string()], update_from_row)
    }
}

impl TaskRepository for SqliteStore {
    fn insert_task(&self, task: &Task) -> RepoResult<()> {
        self.insert_task_inner(task).map_err(Into::into)
    }

    fn update_task(&self, task: &Task) -> RepoResult<bool> {
        self.update_task_inner(task).map_err(Into::into)
    }

    fn find_task(&self, id: TaskId) -> RepoResult<Option<Task>> {
        self.find_task_inner(id).map_err(Into::into)
    }

    fn list_tasks(&self) -> RepoResult<Vec<Task>> {
        self.list_tasks_inner().map_err(Into::into)
    }

    fn list_tasks_by_status(&self, status: TaskStatus) -> RepoResult<Vec<Task>> {
        self.list_tasks_by_status_inner(status).map_err(Into::into)
    }

    fn list_children(&self, parent_id: TaskId) -> RepoResult<Vec<Task>> {
        self.list_children_inner(parent_id).map_err(Into::into)
    }

    fn insert_resource(&self, resource: &TaskResource) -> RepoResult<()> {
        self.insert_resource_inner(resource).map_err(Into::into)
    }

    fn update_resource(&self, resource: &TaskResource) -> RepoResult<bool> {
        self.update_resource_inner(resource).map_err(Into::into)
    }

    fn delete_resource(&self, id: ResourceId) -> RepoResult<bool> {
        self.delete_resource_inner(id).map_err(Into::into)
    }

    fn list_resources(&self, task_id: TaskId) -> RepoResult<Vec<TaskResource>> {
        self.list_resources_inner(task_id).map_err(Into::into)
    }

    fn list_all_resources(&self) -> RepoResult<Vec<TaskResource>> {
        self.list_all_resources_inner().map_err(Into::into)
    }

    fn insert_update(&self, entry: &TaskUpdateEntry) -> RepoResult<()> {
        self.insert_update_inner(entry).map_err(Into::into)
    }

    fn list_updates(&self, task_id: TaskId) -> RepoResult<Vec<TaskUpdateEntry>> {
        self.list_updates_inner(task_id).map_err(Into::into)
    }
}
