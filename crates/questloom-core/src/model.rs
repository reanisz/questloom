//! ドメインモデル。DB スキーマ (docs/data-model.md) と 1:1 に対応する。

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// ID 型を定義するマクロ。すべて UUID v7(時系列ソート可能)。
macro_rules! define_id {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// 新しい UUID v7 を採番する。
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// 既存の UUID から構築する。
            #[must_use]
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// 内部の UUID を返す。
            #[must_use]
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }
    };
}

define_id!(TaskId, "タスクの識別子 (UUID v7)。");
define_id!(ResourceId, "関連リソースの識別子 (UUID v7)。");
define_id!(UpdateId, "アップデート履歴の識別子 (UUID v7)。");

/// タスクの状態。DB の `tasks.status` に対応する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    /// 受信箱。オーバーレイ表示の対象。
    New,
    /// 着手予定。時間バケットを持つ。
    Todo,
    /// 着手中。
    Doing,
    /// 完了。
    Done,
}

impl TaskStatus {
    /// DB に保存する文字列表現。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Todo => "todo",
            Self::Doing => "doing",
            Self::Done => "done",
        }
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 文字列から列挙値へのパースに失敗したことを表す。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("ドメイン値のパースに失敗しました: {kind} = {value:?}")]
pub struct ParseDomainError {
    /// パース対象の型名。
    pub kind: &'static str,
    /// 入力文字列。
    pub value: String,
}

impl ParseDomainError {
    fn new(kind: &'static str, value: impl Into<String>) -> Self {
        Self {
            kind,
            value: value.into(),
        }
    }
}

impl FromStr for TaskStatus {
    type Err = ParseDomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "new" => Ok(Self::New),
            "todo" => Ok(Self::Todo),
            "doing" => Ok(Self::Doing),
            "done" => Ok(Self::Done),
            other => Err(ParseDomainError::new("TaskStatus", other)),
        }
    }
}

/// ISO 週相当の週キー。`YYYY-Www` 形式で表現する。
///
/// 週の開始曜日が月曜のとき ISO 8601 の週番号と完全に一致する。
/// 週開始が日曜に設定されている場合は「その週の 4 日目(=水曜)が属する ISO 週」を
/// ラベルとして採用する([`crate::bucket::week_key_of`] 参照)。
/// `(year, week)` の辞書順比較は日付順と一致する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct WeekKey {
    /// 週が属する年 (ISO week-numbering year)。
    pub year: i32,
    /// 1 始まりの週番号。
    pub week: u32,
}

impl WeekKey {
    /// 年と週番号から構築する。
    #[must_use]
    pub const fn new(year: i32, week: u32) -> Self {
        Self { year, week }
    }
}

impl fmt::Display for WeekKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-W{:02}", self.year, self.week)
    }
}

impl FromStr for WeekKey {
    type Err = ParseDomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseDomainError::new("WeekKey", s);
        let (year, week) = s.split_once("-W").ok_or_else(err)?;
        if year.len() != 4 || week.len() != 2 {
            return Err(err());
        }
        let year: i32 = year.parse().map_err(|_| err())?;
        let week: u32 = week.parse().map_err(|_| err())?;
        if !(1..=53).contains(&week) {
            return Err(err());
        }
        Ok(Self { year, week })
    }
}

impl TryFrom<String> for WeekKey {
    type Error = ParseDomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<WeekKey> for String {
    fn from(value: WeekKey) -> Self {
        value.to_string()
    }
}

/// タスクの予定。`todo` 状態のときに意味を持つ。
///
/// JSON では `{"kind":"date","value":"2026-08-31"}` /
/// `{"kind":"week","value":"2026-W36"}` / `{"kind":"none"}` として表現される。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Scheduled {
    /// この日にやる。
    Date(NaiveDate),
    /// この週にやる。
    Week(WeekKey),
    /// いつかやる。
    #[default]
    None,
}

/// [`Scheduled`] の種別。DB の `scheduled_kind` に対応する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledKind {
    /// `date` (ISO 日付)。
    Date,
    /// `week` (ISO 週)。
    Week,
    /// 予定なし。DB では NULL。
    None,
}

impl Scheduled {
    /// 種別を返す。
    #[must_use]
    pub const fn kind(&self) -> ScheduledKind {
        match self {
            Self::Date(_) => ScheduledKind::Date,
            Self::Week(_) => ScheduledKind::Week,
            Self::None => ScheduledKind::None,
        }
    }

    /// DB の `(scheduled_kind, scheduled_value)` 相当の組を返す。
    #[must_use]
    pub fn to_columns(self) -> (Option<&'static str>, Option<String>) {
        match self {
            Self::Date(d) => (Some("date"), Some(d.format("%Y-%m-%d").to_string())),
            Self::Week(w) => (Some("week"), Some(w.to_string())),
            Self::None => (None, None),
        }
    }

    /// DB の `(scheduled_kind, scheduled_value)` から復元する。
    ///
    /// # Errors
    /// 種別・値が不正な場合 [`ParseDomainError`] を返す。
    pub fn from_columns(kind: Option<&str>, value: Option<&str>) -> Result<Self, ParseDomainError> {
        match (kind, value) {
            (None | Some("none"), _) => Ok(Self::None),
            (Some("date"), Some(v)) => NaiveDate::parse_from_str(v, "%Y-%m-%d")
                .map(Self::Date)
                .map_err(|_| ParseDomainError::new("Scheduled::Date", v)),
            (Some("week"), Some(v)) => v.parse().map(Self::Week),
            (Some(other), _) => Err(ParseDomainError::new("Scheduled", other)),
        }
    }
}

/// タスク・アップデートの発生元。
///
/// JSON / DB のどちらでも `user` / `mcp` / `ai` / `plugin:<id>` / `system` の
/// 文字列として表現される。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Origin {
    /// ユーザー操作。
    #[default]
    User,
    /// 内蔵 MCP サーバー経由。
    Mcp,
    /// AI CLI 経由。
    Ai,
    /// プラグイン (`plugin:<id>`)。
    Plugin(String),
    /// システムによる自動記録。
    System,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User => f.write_str("user"),
            Self::Mcp => f.write_str("mcp"),
            Self::Ai => f.write_str("ai"),
            Self::Plugin(id) => write!(f, "plugin:{id}"),
            Self::System => f.write_str("system"),
        }
    }
}

impl FromStr for Origin {
    type Err = ParseDomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(Self::User),
            "mcp" => Ok(Self::Mcp),
            "ai" => Ok(Self::Ai),
            "system" => Ok(Self::System),
            other => other
                .strip_prefix("plugin:")
                .filter(|id| !id.is_empty())
                .map(|id| Self::Plugin(id.to_owned()))
                .ok_or_else(|| ParseDomainError::new("Origin", other)),
        }
    }
}

impl Serialize for Origin {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Origin {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// 関連リソースの種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceKind {
    /// URL。
    Url,
    /// ローカルファイルパス。
    File,
}

impl ResourceKind {
    /// DB に保存する文字列表現。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Url => "url",
            Self::File => "file",
        }
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ResourceKind {
    type Err = ParseDomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "url" => Ok(Self::Url),
            "file" => Ok(Self::File),
            other => Err(ParseDomainError::new("ResourceKind", other)),
        }
    }
}

/// タスク本体。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    /// 識別子。
    pub id: TaskId,
    /// タイトル。
    pub title: String,
    /// 詳細 (Markdown、複数行)。
    pub description: String,
    /// 状態。
    pub status: TaskStatus,
    /// 予定 (`status == Todo` のときに意味を持つ)。
    pub scheduled: Scheduled,
    /// 締切。予定とは独立。
    pub deadline: Option<DateTime<Utc>>,
    /// インスタントタスクか。
    pub is_instant: bool,
    /// 発生元。
    pub origin: Origin,
    /// 親タスク。
    pub parent_id: Option<TaskId>,
    /// 同一リスト内の並び順 (fractional key)。
    pub sort_order: String,
    /// 作成時刻 (UTC)。
    pub created_at: DateTime<Utc>,
    /// 最終更新時刻 (UTC)。
    pub updated_at: DateTime<Utc>,
    /// 完了時刻 (UTC)。
    pub done_at: Option<DateTime<Utc>>,
}

/// タスクの関連リソース。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResource {
    /// 識別子。
    pub id: ResourceId,
    /// 所属タスク。
    pub task_id: TaskId,
    /// 種別。
    pub kind: ResourceKind,
    /// URL またはファイルパス。
    pub value: String,
    /// 表示ラベル。
    pub label: String,
    /// 主リソース(オーバーレイのワンクリック起動対象)。
    pub is_primary: bool,
    /// 並び順 (fractional key)。
    pub sort_order: String,
    /// 作成時刻 (UTC)。
    pub created_at: DateTime<Utc>,
}

/// 状態アップデートのヒストリー 1 件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskUpdateEntry {
    /// 識別子。
    pub id: UpdateId,
    /// 所属タスク。
    pub task_id: TaskId,
    /// 本文 (Markdown)。
    pub body: String,
    /// 発生元。
    pub origin: Origin,
    /// 作成時刻 (UTC)。
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_status_roundtrip() {
        for status in [
            TaskStatus::New,
            TaskStatus::Todo,
            TaskStatus::Doing,
            TaskStatus::Done,
        ] {
            assert_eq!(status.as_str().parse::<TaskStatus>().unwrap(), status);
        }
        assert!("bogus".parse::<TaskStatus>().is_err());
    }

    #[test]
    fn origin_roundtrip() {
        for origin in [
            Origin::User,
            Origin::Mcp,
            Origin::Ai,
            Origin::System,
            Origin::Plugin("github".to_owned()),
        ] {
            assert_eq!(origin.to_string().parse::<Origin>().unwrap(), origin);
        }
        assert_eq!(Origin::Plugin("github".into()).to_string(), "plugin:github");
        assert!("plugin:".parse::<Origin>().is_err());
        assert!("nope".parse::<Origin>().is_err());
    }

    #[test]
    fn origin_serde_is_a_plain_string() {
        let json = serde_json::to_string(&Origin::Plugin("github".into())).unwrap();
        assert_eq!(json, "\"plugin:github\"");
        let back: Origin = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Origin::Plugin("github".into()));
    }

    #[test]
    fn week_key_roundtrip() {
        let key = WeekKey::new(2026, 1);
        assert_eq!(key.to_string(), "2026-W01");
        assert_eq!("2026-W01".parse::<WeekKey>().unwrap(), key);
        assert_eq!(
            serde_json::to_string(&key).unwrap(),
            "\"2026-W01\"".to_owned()
        );
        assert!("2026-W00".parse::<WeekKey>().is_err());
        assert!("2026-W54".parse::<WeekKey>().is_err());
        assert!("2026W01".parse::<WeekKey>().is_err());
        assert!("26-W01".parse::<WeekKey>().is_err());
    }

    #[test]
    fn week_key_ordering_is_chronological() {
        assert!(WeekKey::new(2025, 52) < WeekKey::new(2026, 1));
        assert!(WeekKey::new(2026, 1) < WeekKey::new(2026, 2));
    }

    #[test]
    fn scheduled_columns_roundtrip() {
        let cases = [
            Scheduled::None,
            Scheduled::Date(NaiveDate::from_ymd_opt(2026, 8, 31).unwrap()),
            Scheduled::Week(WeekKey::new(2026, 36)),
        ];
        for case in cases {
            let (kind, value) = case.to_columns();
            let back = Scheduled::from_columns(kind, value.as_deref()).unwrap();
            assert_eq!(back, case);
        }
        assert!(Scheduled::from_columns(Some("bogus"), None).is_err());
        assert!(Scheduled::from_columns(Some("date"), Some("nope")).is_err());
    }

    #[test]
    fn scheduled_json_shape() {
        let json = serde_json::to_string(&Scheduled::Week(WeekKey::new(2026, 36))).unwrap();
        assert_eq!(json, r#"{"kind":"week","value":"2026-W36"}"#);
        let json = serde_json::to_string(&Scheduled::None).unwrap();
        assert_eq!(json, r#"{"kind":"none"}"#);
        let parsed: Scheduled = serde_json::from_str(r#"{"kind":"none"}"#).unwrap();
        assert_eq!(parsed, Scheduled::None);
    }

    #[test]
    fn ids_are_uuid_v7_and_roundtrip() {
        let id = TaskId::new();
        assert_eq!(id.as_uuid().get_version_num(), 7);
        assert_eq!(id.to_string().parse::<TaskId>().unwrap(), id);
    }
}
