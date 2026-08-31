//! ドメインイベント。`tokio::sync::broadcast` で配信される。
//!
//! UI・オーバーレイ・プラグインはこれを購読して再取得・再描画を行う。
//! ペイロードは「何がどう変わったか」を最小限で伝えるだけに留め、
//! 詳細はサービス層から取り直す方針。

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::bucket::Bucket;
use crate::model::{TaskId, TaskStatus};

/// ドメインイベント。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DomainEvent {
    /// タスクが作成された。
    #[serde(rename_all = "camelCase")]
    TaskCreated {
        /// 対象タスク。
        task_id: TaskId,
    },
    /// タスクの内容(タイトル・詳細・締切など)が更新された。
    #[serde(rename_all = "camelCase")]
    TaskUpdated {
        /// 対象タスク。
        task_id: TaskId,
    },
    /// タスクの状態・予定・並び順が変わった。
    #[serde(rename_all = "camelCase")]
    TaskMoved {
        /// 対象タスク。
        task_id: TaskId,
        /// 移動後のステータス。
        status: TaskStatus,
        /// 移動後のバケット(Todo 以外は `None`)。
        bucket: Option<Bucket>,
    },
    /// タスクが完了した。
    #[serde(rename_all = "camelCase")]
    TaskCompleted {
        /// 対象タスク。
        task_id: TaskId,
    },
    /// インスタントタスクが通常タスクへ昇格した。
    #[serde(rename_all = "camelCase")]
    TaskPromoted {
        /// 対象タスク。
        task_id: TaskId,
    },
    /// タスクが削除された(ソフトデリート)。
    #[serde(rename_all = "camelCase")]
    TaskDeleted {
        /// 対象タスク。
        task_id: TaskId,
    },
    /// 削除済みタスクが復元された。
    #[serde(rename_all = "camelCase")]
    TaskRestored {
        /// 対象タスク。
        task_id: TaskId,
    },
    /// アップデート履歴が追記された。
    #[serde(rename_all = "camelCase")]
    TaskUpdateAdded {
        /// 対象タスク。
        task_id: TaskId,
    },
    /// 関連リソースが追加・削除された。
    #[serde(rename_all = "camelCase")]
    TaskResourcesChanged {
        /// 対象タスク。
        task_id: TaskId,
    },
    /// 親子リンクが変更された。
    #[serde(rename_all = "camelCase")]
    TaskParentChanged {
        /// 対象タスク。
        task_id: TaskId,
        /// 新しい親タスク。
        parent_id: Option<TaskId>,
    },
    /// 日付が変わった(1 分毎の監視で検出)。表示更新・通知のトリガにのみ使う。
    #[serde(rename_all = "camelCase")]
    DayChanged {
        /// 新しい日付(ローカル)。
        date: NaiveDate,
    },
    /// コア設定が変更された。
    SettingsChanged,
}

impl DomainEvent {
    /// このイベントが対象とするタスク(あれば)。
    #[must_use]
    pub const fn task_id(&self) -> Option<TaskId> {
        match self {
            Self::TaskCreated { task_id }
            | Self::TaskUpdated { task_id }
            | Self::TaskMoved { task_id, .. }
            | Self::TaskCompleted { task_id }
            | Self::TaskPromoted { task_id }
            | Self::TaskDeleted { task_id }
            | Self::TaskRestored { task_id }
            | Self::TaskUpdateAdded { task_id }
            | Self::TaskResourcesChanged { task_id }
            | Self::TaskParentChanged { task_id, .. } => Some(*task_id),
            Self::DayChanged { .. } | Self::SettingsChanged => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_json_shape() {
        let id = TaskId::new();
        let json = serde_json::to_value(DomainEvent::TaskCreated { task_id: id }).unwrap();
        assert_eq!(json["type"], "taskCreated");
        assert_eq!(json["taskId"], id.to_string());

        let json = serde_json::to_value(DomainEvent::TaskMoved {
            task_id: id,
            status: TaskStatus::Todo,
            bucket: Some(Bucket::ThisWeek),
        })
        .unwrap();
        assert_eq!(json["type"], "taskMoved");
        assert_eq!(json["status"], "todo");
        assert_eq!(json["bucket"], "thisWeek");

        let json = serde_json::to_value(DomainEvent::TaskDeleted { task_id: id }).unwrap();
        assert_eq!(json["type"], "taskDeleted");
        assert_eq!(json["taskId"], id.to_string());

        let json = serde_json::to_value(DomainEvent::TaskRestored { task_id: id }).unwrap();
        assert_eq!(json["type"], "taskRestored");
        assert_eq!(json["taskId"], id.to_string());
    }

    #[test]
    fn day_changed_has_no_task() {
        let event = DomainEvent::DayChanged {
            date: NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
        };
        assert_eq!(event.task_id(), None);
    }
}
