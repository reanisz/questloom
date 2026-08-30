//! SQLite の行とドメインモデルの相互変換。
//!
//! 日時は RFC3339 (UTC, ミリ秒) のテキストで保存する。この表現は辞書順が
//! 時系列順と一致するため、`ORDER BY` にもそのまま使える。

use std::error::Error as StdError;

use chrono::{DateTime, SecondsFormat, Utc};
use questloom_core::model::{
    Origin, ResourceId, ResourceKind, Scheduled, Task, TaskId, TaskResource, TaskStatus,
    TaskUpdateEntry, UpdateId,
};
use rusqlite::types::Type;
use rusqlite::{Error as SqlError, Row};

/// 全カラムを固定順で読み出すための SELECT 句(`tasks`)。
pub const TASK_COLUMNS: &str = "id, title, description, status, scheduled_kind, scheduled_value, \
     deadline, is_instant, origin, parent_id, sort_order, created_at, updated_at, done_at";

/// 全カラムを固定順で読み出すための SELECT 句(`task_resources`)。
pub const RESOURCE_COLUMNS: &str =
    "id, task_id, kind, value, label, is_primary, sort_order, created_at";

/// 全カラムを固定順で読み出すための SELECT 句(`task_updates`)。
pub const UPDATE_COLUMNS: &str = "id, task_id, body, origin, created_at";

/// 日時を DB 表現へ変換する。
#[must_use]
pub fn time_to_sql(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
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
