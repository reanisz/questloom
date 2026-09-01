//! SQLite の行とドメインモデルの相互変換。
//!
//! 日時は RFC3339 (UTC, ミリ秒) のテキストで保存する。この表現は辞書順が
//! 時系列順と一致するため、`ORDER BY` にもそのまま使える。

use std::error::Error as StdError;

use chrono::{DateTime, SecondsFormat, Utc};
use questloom_core::model::{
    ChecklistItem, ChecklistItemId, Origin, ResourceId, ResourceKind, Scheduled, Task, TaskId,
    TaskResource, TaskStatus, TaskUpdateEntry, UpdateId,
};
use rusqlite::types::{Type, Value};
use rusqlite::{Error as SqlError, Row};

/// 全カラムを固定順で読み出すための SELECT 句(`tasks`)。
pub const TASK_COLUMNS: &str = "id, title, description, status, scheduled_kind, scheduled_value, \
     deadline, is_instant, origin, parent_id, sort_order, created_at, updated_at, done_at, \
     deleted_at";

/// 全カラムを固定順で読み出すための SELECT 句(`task_resources`)。
pub const RESOURCE_COLUMNS: &str =
    "id, task_id, kind, value, label, is_primary, sort_order, created_at";

/// 全カラムを固定順で読み出すための SELECT 句(`task_updates`)。
pub const UPDATE_COLUMNS: &str = "id, task_id, body, origin, created_at";

/// 全カラムを固定順で読み出すための SELECT 句(`task_checklist_items`)。
pub const CHECKLIST_COLUMNS: &str = "id, task_id, body, checked, sort_order, created_at";

/// 日時を DB 表現へ変換する。
#[must_use]
pub fn time_to_sql(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn text(value: impl Into<String>) -> Value {
    Value::Text(value.into())
}

fn nullable_text(value: Option<impl Into<String>>) -> Value {
    value.map_or(Value::Null, |value| Value::Text(value.into()))
}

/// [`TASK_COLUMNS`] と同じ順で `tasks` のバインド値を並べる。
///
/// INSERT と UPDATE で同じ並びを使う(UPDATE は `?1` を `WHERE id` に使い、
/// 残りを SET に並べる)ため、15 カラムの列挙はこの 1 箇所だけにする。
#[must_use]
pub fn task_params(task: &Task) -> [Value; 15] {
    let (kind, value) = task.scheduled.to_columns();
    [
        text(task.id.to_string()),
        text(task.title.clone()),
        text(task.description.clone()),
        text(task.status.as_str()),
        nullable_text(kind),
        nullable_text(value),
        nullable_text(task.deadline.map(time_to_sql)),
        Value::Integer(i64::from(task.is_instant)),
        text(task.origin.to_string()),
        nullable_text(task.parent_id.map(|id| id.to_string())),
        text(task.sort_order.clone()),
        text(time_to_sql(task.created_at)),
        text(time_to_sql(task.updated_at)),
        nullable_text(task.done_at.map(time_to_sql)),
        nullable_text(task.deleted_at.map(time_to_sql)),
    ]
}

/// [`RESOURCE_COLUMNS`] と同じ順で `task_resources` のバインド値を並べる。
#[must_use]
pub fn resource_params(resource: &TaskResource) -> [Value; 8] {
    [
        text(resource.id.to_string()),
        text(resource.task_id.to_string()),
        text(resource.kind.as_str()),
        text(resource.value.clone()),
        text(resource.label.clone()),
        Value::Integer(i64::from(resource.is_primary)),
        text(resource.sort_order.clone()),
        text(time_to_sql(resource.created_at)),
    ]
}

/// [`UPDATE_COLUMNS`] と同じ順で `task_updates` のバインド値を並べる。
#[must_use]
pub fn update_params(entry: &TaskUpdateEntry) -> [Value; 5] {
    [
        text(entry.id.to_string()),
        text(entry.task_id.to_string()),
        text(entry.body.clone()),
        text(entry.origin.to_string()),
        text(time_to_sql(entry.created_at)),
    ]
}

/// [`CHECKLIST_COLUMNS`] と同じ順で `task_checklist_items` のバインド値を並べる。
///
/// INSERT と UPDATE で同じ並びを使う(UPDATE は `?1` を `WHERE id` に使う)。
#[must_use]
pub fn checklist_params(item: &ChecklistItem) -> [Value; 6] {
    [
        text(item.id.to_string()),
        text(item.task_id.to_string()),
        text(item.body.clone()),
        Value::Integer(i64::from(item.checked)),
        text(item.sort_order.clone()),
        text(time_to_sql(item.created_at)),
    ]
}

fn conversion_error(index: usize, error: impl StdError + Send + Sync + 'static) -> SqlError {
    SqlError::FromSqlConversionFailure(index, Type::Text, Box::new(error))
}

fn time_from_sql(index: usize, raw: &str) -> Result<DateTime<Utc>, SqlError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| conversion_error(index, error))
}

fn parse_at<T>(index: usize, raw: &str) -> Result<T, SqlError>
where
    T: std::str::FromStr,
    T::Err: StdError + Send + Sync + 'static,
{
    raw.parse::<T>()
        .map_err(|error| conversion_error(index, error))
}

/// [`TASK_COLUMNS`] の順で並んだ行から [`Task`] を復元する。
///
/// # Errors
/// カラムの読み出しまたはドメイン値への変換に失敗した場合。
pub fn task_from_row(row: &Row<'_>) -> Result<Task, SqlError> {
    let scheduled_kind: Option<String> = row.get(4)?;
    let scheduled_value: Option<String> = row.get(5)?;
    let scheduled = Scheduled::from_columns(scheduled_kind.as_deref(), scheduled_value.as_deref())
        .map_err(|error| conversion_error(4, error))?;

    let deadline: Option<String> = row.get(6)?;
    let parent_id: Option<String> = row.get(9)?;
    let done_at: Option<String> = row.get(13)?;
    let deleted_at: Option<String> = row.get(14)?;

    Ok(Task {
        id: parse_at(0, &row.get::<_, String>(0)?)?,
        title: row.get(1)?,
        description: row.get(2)?,
        status: parse_at::<TaskStatus>(3, &row.get::<_, String>(3)?)?,
        scheduled,
        deadline: deadline
            .as_deref()
            .map(|raw| time_from_sql(6, raw))
            .transpose()?,
        is_instant: row.get::<_, i64>(7)? != 0,
        origin: parse_at::<Origin>(8, &row.get::<_, String>(8)?)?,
        parent_id: parent_id
            .as_deref()
            .map(|raw| parse_at::<TaskId>(9, raw))
            .transpose()?,
        sort_order: row.get(10)?,
        created_at: time_from_sql(11, &row.get::<_, String>(11)?)?,
        updated_at: time_from_sql(12, &row.get::<_, String>(12)?)?,
        done_at: done_at
            .as_deref()
            .map(|raw| time_from_sql(13, raw))
            .transpose()?,
        deleted_at: deleted_at
            .as_deref()
            .map(|raw| time_from_sql(14, raw))
            .transpose()?,
    })
}

/// [`RESOURCE_COLUMNS`] の順で並んだ行から [`TaskResource`] を復元する。
///
/// # Errors
/// カラムの読み出しまたはドメイン値への変換に失敗した場合。
pub fn resource_from_row(row: &Row<'_>) -> Result<TaskResource, SqlError> {
    Ok(TaskResource {
        id: parse_at::<ResourceId>(0, &row.get::<_, String>(0)?)?,
        task_id: parse_at::<TaskId>(1, &row.get::<_, String>(1)?)?,
        kind: parse_at::<ResourceKind>(2, &row.get::<_, String>(2)?)?,
        value: row.get(3)?,
        label: row.get(4)?,
        is_primary: row.get::<_, i64>(5)? != 0,
        sort_order: row.get(6)?,
        created_at: time_from_sql(7, &row.get::<_, String>(7)?)?,
    })
}

/// [`UPDATE_COLUMNS`] の順で並んだ行から [`TaskUpdateEntry`] を復元する。
///
/// # Errors
/// カラムの読み出しまたはドメイン値への変換に失敗した場合。
pub fn update_from_row(row: &Row<'_>) -> Result<TaskUpdateEntry, SqlError> {
    Ok(TaskUpdateEntry {
        id: parse_at::<UpdateId>(0, &row.get::<_, String>(0)?)?,
        task_id: parse_at::<TaskId>(1, &row.get::<_, String>(1)?)?,
        body: row.get(2)?,
        origin: parse_at::<Origin>(3, &row.get::<_, String>(3)?)?,
        created_at: time_from_sql(4, &row.get::<_, String>(4)?)?,
    })
}

/// [`CHECKLIST_COLUMNS`] の順で並んだ行から [`ChecklistItem`] を復元する。
///
/// # Errors
/// カラムの読み出しまたはドメイン値への変換に失敗した場合。
pub fn checklist_from_row(row: &Row<'_>) -> Result<ChecklistItem, SqlError> {
    Ok(ChecklistItem {
        id: parse_at::<ChecklistItemId>(0, &row.get::<_, String>(0)?)?,
        task_id: parse_at::<TaskId>(1, &row.get::<_, String>(1)?)?,
        body: row.get(2)?,
        checked: row.get::<_, i64>(3)? != 0,
        sort_order: row.get(4)?,
        created_at: time_from_sql(5, &row.get::<_, String>(5)?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params_from_iter, Connection};

    /// [`task_params`] で書いた行を、そのまま読み戻すための最小の DB。
    fn store_task(task: &Task) -> Connection {
        let conn = Connection::open_in_memory().expect("インメモリ DB");
        conn.execute_batch(
            "CREATE TABLE tasks (id TEXT, title TEXT, description TEXT, status TEXT,
                 scheduled_kind TEXT, scheduled_value TEXT, deadline TEXT, is_instant INTEGER,
                 origin TEXT, parent_id TEXT, sort_order TEXT, created_at TEXT,
                 updated_at TEXT, done_at TEXT, deleted_at TEXT);",
        )
        .expect("テーブルを作れる");
        conn.execute(
            "INSERT INTO tasks VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params_from_iter(task_params(task)),
        )
        .expect("挿入できる");
        conn
    }

    fn read_task(conn: &Connection) -> Result<Task, SqlError> {
        let sql = format!("SELECT {TASK_COLUMNS} FROM tasks");
        conn.query_row(&sql, [], task_from_row)
    }

    fn sample() -> Task {
        let now = DateTime::parse_from_rfc3339("2026-09-02T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        Task {
            id: TaskId::new(),
            title: "書き戻し".to_owned(),
            description: "詳細".to_owned(),
            status: TaskStatus::Todo,
            scheduled: questloom_core::model::Scheduled::Date(
                chrono::NaiveDate::from_ymd_opt(2026, 9, 3).unwrap(),
            ),
            deadline: Some(now),
            is_instant: true,
            origin: Origin::Plugin("github".to_owned()),
            parent_id: Some(TaskId::new()),
            sort_order: "a0".to_owned(),
            created_at: now,
            updated_at: now,
            done_at: None,
            deleted_at: None,
        }
    }

    #[test]
    fn params_and_row_mapping_roundtrip() {
        let task = sample();
        let conn = store_task(&task);
        assert_eq!(read_task(&conn).expect("読み戻せる"), task);
    }

    /// どの状態も文字列として往復すること
    /// (Watching / Icebox を足しても DB 側の変更は不要)。
    #[test]
    fn every_status_roundtrips() {
        for status in [
            TaskStatus::New,
            TaskStatus::Todo,
            TaskStatus::Doing,
            TaskStatus::Done,
            TaskStatus::Watching,
            TaskStatus::Icebox,
        ] {
            let task = Task { status, ..sample() };
            let conn = store_task(&task);
            assert_eq!(read_task(&conn).expect("読み戻せる").status, status);
        }
    }

    /// ソフトデリート時刻も往復すること。
    #[test]
    fn deleted_at_roundtrips() {
        let deleted_at = DateTime::parse_from_rfc3339("2026-09-05T08:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let task = Task {
            deleted_at: Some(deleted_at),
            ..sample()
        };
        let conn = store_task(&task);
        let read = read_task(&conn).expect("読み戻せる");
        assert_eq!(read.deleted_at, Some(deleted_at));
        assert!(read.is_deleted());
    }

    /// チェックリスト項目も、書いた並びのまま読み戻せること。
    #[test]
    fn checklist_params_and_row_mapping_roundtrip() {
        let item = ChecklistItem {
            id: ChecklistItemId::new(),
            task_id: TaskId::new(),
            body: "住所変更".to_owned(),
            checked: true,
            sort_order: "a0".to_owned(),
            created_at: DateTime::parse_from_rfc3339("2026-09-02T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        };
        let conn = Connection::open_in_memory().expect("インメモリ DB");
        conn.execute_batch(
            "CREATE TABLE task_checklist_items (id TEXT, task_id TEXT, body TEXT,
                 checked INTEGER, sort_order TEXT, created_at TEXT);",
        )
        .expect("テーブルを作れる");
        conn.execute(
            "INSERT INTO task_checklist_items VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params_from_iter(checklist_params(&item)),
        )
        .expect("挿入できる");

        let sql = format!("SELECT {CHECKLIST_COLUMNS} FROM task_checklist_items");
        let read = conn
            .query_row(&sql, [], checklist_from_row)
            .expect("読み戻せる");
        assert_eq!(read, item);

        // checked は 0/1 の整数。0 なら偽として読める。
        conn.execute("UPDATE task_checklist_items SET checked = 0", [])
            .unwrap();
        assert!(
            !conn
                .query_row(&sql, [], checklist_from_row)
                .unwrap()
                .checked
        );
    }

    #[test]
    fn unknown_status_is_reported_as_a_conversion_failure() {
        let conn = store_task(&sample());
        conn.execute("UPDATE tasks SET status = 'archived'", [])
            .unwrap();
        let error = read_task(&conn).expect_err("status を解釈できない");
        assert!(
            matches!(error, SqlError::FromSqlConversionFailure(3, _, _)),
            "{error:?}"
        );
        assert!(error.to_string().contains("archived"), "{error}");
    }

    #[test]
    fn unknown_origin_is_reported_as_a_conversion_failure() {
        let conn = store_task(&sample());
        // origin は `plugin:<id>` 以外の未知の値を受け付けない。
        conn.execute("UPDATE tasks SET origin = ''", []).unwrap();
        let error = read_task(&conn).expect_err("origin を解釈できない");
        assert!(
            matches!(error, SqlError::FromSqlConversionFailure(8, _, _)),
            "{error:?}"
        );
    }

    #[test]
    fn broken_timestamps_and_schedules_are_reported() {
        let conn = store_task(&sample());
        conn.execute("UPDATE tasks SET created_at = '昨日'", [])
            .unwrap();
        assert!(matches!(
            read_task(&conn).expect_err("日時を解釈できない"),
            SqlError::FromSqlConversionFailure(11, _, _)
        ));

        let conn = store_task(&sample());
        conn.execute("UPDATE tasks SET scheduled_kind = 'month'", [])
            .unwrap();
        assert!(matches!(
            read_task(&conn).expect_err("予定を解釈できない"),
            SqlError::FromSqlConversionFailure(4, _, _)
        ));
    }
}
