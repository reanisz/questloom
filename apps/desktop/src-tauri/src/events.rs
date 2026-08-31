//! ドメインイベントの購読ヘルパ、webview への橋渡し、日付変化の監視。
//!
//! `TaskService` の broadcast を購読する側は 3 箇所(ここ・[`overlay`](crate::overlay)・
//! [`settings`](crate::settings))あり、いずれも「recv → Lagged は warn して継続 →
//! Closed で終了」という同じループを持つ。その定型は [`spawn_domain_watcher`] に集約し、
//! 各所は受け取った [`DomainSignal`] の扱いだけを書く。

use std::sync::Arc;
use std::time::Duration;

use questloom_core::events::DomainEvent;
use questloom_core::service::TaskService;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::broadcast::error::RecvError;

pub use crate::contract::TASKS_CHANGED;

/// 日付変化を監視する間隔。
const DAY_WATCH_INTERVAL: Duration = Duration::from_secs(60);

/// ドメインイベント購読者が受け取る通知。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainSignal {
    /// イベントを 1 件受け取った。
    Event(DomainEvent),
    /// 購読が追いつかず取りこぼした(件数付き)。何が落ちたか分からないので、
    /// 状態を持つ購読者は「全部変わったかもしれない」として再同期すること。
    Lagged(u64),
}

/// ドメインイベントを購読し、1 件ごとに `on_signal` を呼ぶタスクを開始する。
///
/// `name` は取りこぼし・終了時のログに出す購読者の名前。
/// 送信側(`TaskService`)が落ちたら購読を終了する。
pub fn spawn_domain_watcher<F>(service: &Arc<TaskService>, name: &'static str, mut on_signal: F)
where
    F: FnMut(DomainSignal) + Send + 'static,
{
    let mut receiver = service.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            let signal = match receiver.recv().await {
                Ok(event) => DomainSignal::Event(event),
                Err(RecvError::Lagged(missed)) => {
                    tracing::warn!(watcher = name, missed, "ドメインイベントを取りこぼしました");
                    DomainSignal::Lagged(missed)
                }
                Err(RecvError::Closed) => {
                    tracing::debug!(watcher = name, "ドメインイベントの購読を終了します");
                    break;
                }
            };
            on_signal(signal);
        }
    });
}

/// [`TASKS_CHANGED`] のペイロード。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TasksChanged {
    /// 発生したドメインイベント。取りこぼしがあった場合は `None`。
    pub event: Option<DomainEvent>,
    /// 購読が追いつかず取りこぼしたイベント数。
    pub missed: u64,
}

impl From<DomainSignal> for TasksChanged {
    fn from(signal: DomainSignal) -> Self {
        match signal {
            DomainSignal::Event(event) => Self {
                event: Some(event),
                missed: 0,
            },
            DomainSignal::Lagged(missed) => Self {
                event: None,
                missed,
            },
        }
    }
}

/// ドメインイベントを購読し、webview へ中継するタスクを開始する。
///
/// **種類で絞らない**。`SettingsChanged` / `DayChanged` でも [`TASKS_CHANGED`] を出すので、
/// フロントは設定変更・日付変化でもボードを取り直す(バケット導出が今日基準のため)。
pub fn spawn_bridge<R: Runtime>(app: AppHandle<R>, service: &Arc<TaskService>) {
    spawn_domain_watcher(service, "bridge", move |signal| {
        if let Err(error) = app.emit(TASKS_CHANGED, TasksChanged::from(signal)) {
            tracing::error!(%error, "イベントの emit に失敗しました");
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
    use questloom_core::model::TaskId;

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

    /// 取りこぼしの件数がペイロードへ載ること(フロントの契約)。
    #[test]
    fn signal_maps_onto_the_payload() {
        let event = DomainEvent::TaskCreated {
            task_id: TaskId::new(),
        };
        let payload = TasksChanged::from(DomainSignal::Event(event.clone()));
        assert_eq!(payload.event, Some(event));
        assert_eq!(payload.missed, 0);

        let payload = TasksChanged::from(DomainSignal::Lagged(7));
        assert_eq!(payload.event, None);
        assert_eq!(payload.missed, 7);
    }
}
