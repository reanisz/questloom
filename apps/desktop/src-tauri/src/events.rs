//! ドメインイベントの webview への橋渡しと、日付変化の監視。

use std::sync::Arc;
use std::time::Duration;

use questloom_core::events::DomainEvent;
use questloom_core::service::TaskService;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::broadcast::error::RecvError;

/// タスク関連の変更を webview へ通知するイベント名。
///
/// フロントはこれを受け取ったら `get_board` などで再フェッチする。
pub const TASKS_CHANGED: &str = "questloom://tasks-changed";

/// 日付変化を監視する間隔。
const DAY_WATCH_INTERVAL: Duration = Duration::from_secs(60);

/// [`TASKS_CHANGED`] のペイロード。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TasksChanged {
    /// 発生したドメインイベント。取りこぼしがあった場合は `None`。
    pub event: Option<DomainEvent>,
    /// 購読が追いつかず取りこぼしたイベント数。
    pub missed: u64,
}

/// ドメインイベントを購読し、webview へ中継するタスクを開始する。
pub fn spawn_bridge<R: Runtime>(app: AppHandle<R>, service: &Arc<TaskService>) {
    let mut receiver = service.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            let payload = match receiver.recv().await {
                Ok(event) => TasksChanged {
                    event: Some(event),
                    missed: 0,
                },
                Err(RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "ドメインイベントを取りこぼしました");
                    TasksChanged {
                        event: None,
                        missed,
                    }
                }
                Err(RecvError::Closed) => {
                    tracing::debug!("ドメインイベントの購読を終了します");
                    break;
                }
            };
            if let Err(error) = app.emit(TASKS_CHANGED, payload) {
                tracing::error!(%error, "イベントの emit に失敗しました");
            }
        }
    });
}

/// 1 分毎に日付変化を監視し、変わったら [`DomainEvent::DayChanged`] を発行するタスクを開始する。
pub fn spawn_day_watcher(service: Arc<TaskService>) {
    tauri::async_runtime::spawn(async move {
        let mut current = service.today();
        let mut ticker = tokio::time::interval(DAY_WATCH_INTERVAL);
        // 1 回目の tick は即座に返るため読み捨てる。
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let today = service.today();
            if today != current {
                tracing::info!(%today, "日付が変わりました");
                current = today;
                service.notify_day_changed(today);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_is_camel_case() {
        let json = serde_json::to_value(TasksChanged {
            event: None,
            missed: 3,
        })
        .unwrap();
        assert_eq!(json["missed"], 3);
        assert!(json["event"].is_null());
    }

    #[test]
    fn event_name_is_valid_for_tauri() {
        // Tauri v2 が許容するのは英数字と `-` `/` `:` `_` のみ。
        assert!(TASKS_CHANGED
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '/' | ':' | '_')));
    }
}
