//! オーバーレイ通知ウィンドウの表示制御。
//!
//! 「New タスクが 1 件以上ある間だけ表示する」という判断は Rust 側で行い、
//! ウィンドウの中身(件数・行)はフロント側が [`TASKS_CHANGED`](crate::events::TASKS_CHANGED)
//! を受けて自分で取り直す。ウィンドウ高さもフロントが内容に合わせて調整する。
//!
//! ウィンドウは `tauri.conf.json` で `focus: false` として生成されるため、
//! `show()` は Windows では `SW_SHOWNOACTIVATE` になり、フォーカスを奪わない。
//! ここで `set_focus()` を呼んではいけない。

use std::sync::Arc;

use questloom_core::events::DomainEvent;
use questloom_core::model::TaskStatus;
use questloom_core::service::TaskService;
use tauri::{AppHandle, LogicalPosition, Manager, Runtime, WebviewWindow};

use crate::events::{spawn_domain_watcher, DomainSignal};
use crate::state::AppState;

/// オーバーレイウィンドウのラベル(`tauri.conf.json` と一致させること)。
pub const OVERLAY_WINDOW: &str = "overlay";

/// メインディスプレイ左上からのマージン(論理ピクセル)。
///
/// `tauri.conf.json` の overlay ウィンドウの初期位置 (`x: 12`, `y: 12`) と同じ値。
/// あちらは起動直後の暫定位置で、実際の配置は [`place`] がメインディスプレイの
/// 原点を見て毎回やり直す。JSON にはコメントが書けないので、
/// **片方を変えたらもう片方も直すこと**をここに書いておく。
const MARGIN: f64 = 12.0;

/// New タスク件数と設定に応じて、オーバーレイの表示 / 非表示を切り替える。
pub fn sync<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AppState>() else {
        // 初期化前に呼ばれた場合は何もしない。
        return;
    };
    let visible = should_show(&state.service, state.settings().overlay_enabled);
    apply(app, visible);
}

/// オーバーレイを表示すべきか(オーバーレイ有効 かつ New タスクが 1 件以上)。
fn should_show(service: &TaskService, overlay_enabled: bool) -> bool {
    if !overlay_enabled {
        return false;
    }
    match service.list_by_status(TaskStatus::New) {
        Ok(tasks) => !tasks.is_empty(),
        Err(error) => {
            tracing::warn!(%error, "New タスクを取得できませんでした");
            false
        }
    }
}

/// 表示状態を反映する。
fn apply<R: Runtime>(app: &AppHandle<R>, visible: bool) {
    let Some(window) = app.get_webview_window(OVERLAY_WINDOW) else {
        tracing::warn!("オーバーレイウィンドウが見つかりません");
        return;
    };
    let current = window.is_visible().unwrap_or(false);
    if visible == current {
        return;
    }
    if visible {
        place(&window);
        if let Err(error) = window.show() {
            tracing::warn!(%error, "オーバーレイを表示できませんでした");
        }
    } else if let Err(error) = window.hide() {
        tracing::warn!(%error, "オーバーレイを隠せませんでした");
    }
}

/// メインディスプレイの左上へ配置する。
fn place<R: Runtime>(window: &WebviewWindow<R>) {
    let monitor = match window.primary_monitor() {
        Ok(Some(monitor)) => monitor,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%error, "メインディスプレイを取得できませんでした");
            return;
        }
    };
    let scale = monitor.scale_factor();
    let origin = monitor.position().to_logical::<f64>(scale);
    let position = LogicalPosition::new(origin.x + MARGIN, origin.y + MARGIN);
    if let Err(error) = window.set_position(position) {
        tracing::warn!(%error, "オーバーレイを配置できませんでした");
    }
}

/// 表示判定をやり直す必要があるイベントか。
///
/// 判定材料は「New タスクが 1 件以上あるか」と「オーバーレイが有効か」だけなので、
/// New 件数を動かしうるタスク系イベントと `SettingsChanged` に絞る。
/// 削除・復元 (`TaskDeleted` / `TaskRestored`) も New 件数を動かすので含める
/// (New タスクを消したらオーバーレイも消えるように)。
/// `DayChanged` も含めるのは、日付をまたいだ直後の取りこぼしを避けるための保険。
///
/// タイトルやリソースの変更(`TaskUpdated` など)は表示 / 非表示を変えないので無視する。
/// オーバーレイの**中身**はフロントが `questloom://tasks-changed` を受けて取り直すため、
/// ここで弾いても表示内容が古くなることはない。
const fn affects_visibility(event: &DomainEvent) -> bool {
    matches!(
        event,
        DomainEvent::TaskCreated { .. }
            | DomainEvent::TaskMoved { .. }
            | DomainEvent::TaskCompleted { .. }
            | DomainEvent::TaskPromoted { .. }
            | DomainEvent::TaskDeleted { .. }
            | DomainEvent::TaskRestored { .. }
            | DomainEvent::DayChanged { .. }
            | DomainEvent::SettingsChanged
    )
}

/// ドメインイベントを購読し、表示判定をやり直すタスクを開始する。
///
/// 判定は DB へのクエリを伴うので、[`affects_visibility`] が真のイベントだけに反応する
/// (AI の一括作成のようなバーストで、無関係なイベントごとにクエリが走らないように)。
pub fn spawn_watcher<R: Runtime>(app: AppHandle<R>, service: &Arc<TaskService>) {
    spawn_domain_watcher(service, "overlay", move |signal| match signal {
        DomainSignal::Event(event) => {
            if affects_visibility(&event) {
                sync(&app);
            }
        }
        // 何が落ちたか分からないので、取りこぼし時は必ず judge し直す。
        DomainSignal::Lagged(_) => sync(&app),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::test_support;
    use questloom_core::model::TaskId;
    use questloom_core::service::NewTask;
    use questloom_core::settings::BoardSettings;

    #[test]
    fn hidden_without_new_tasks() {
        let service = test_support::service(BoardSettings::default());
        assert!(!should_show(&service, true));

        service
            .create_task(NewTask {
                title: "新着".to_owned(),
                ..NewTask::default()
            })
            .unwrap();
        assert!(should_show(&service, true));
    }

    /// 表示 / 非表示を動かしうるイベントだけを拾うこと。
    #[test]
    fn only_visibility_relevant_events_trigger_a_resync() {
        let id = TaskId::new();
        for event in [
            DomainEvent::TaskCreated { task_id: id },
            DomainEvent::TaskCompleted { task_id: id },
            DomainEvent::TaskPromoted { task_id: id },
            DomainEvent::TaskDeleted { task_id: id },
            DomainEvent::TaskRestored { task_id: id },
            DomainEvent::TaskMoved {
                task_id: id,
                status: TaskStatus::Todo,
                bucket: None,
            },
            DomainEvent::DayChanged {
                date: chrono::NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            },
            DomainEvent::SettingsChanged,
        ] {
            assert!(affects_visibility(&event), "{event:?} は再評価が要る");
        }

        for event in [
            DomainEvent::TaskUpdated { task_id: id },
            DomainEvent::TaskUpdateAdded { task_id: id },
            DomainEvent::TaskResourcesChanged { task_id: id },
            DomainEvent::TaskParentChanged {
                task_id: id,
                parent_id: None,
            },
        ] {
            assert!(!affects_visibility(&event), "{event:?} で再評価は要らない");
        }
    }

    /// New タスクを削除したらオーバーレイは消え、復元したら戻る。
    #[test]
    fn deleting_the_last_new_task_hides_the_overlay() {
        let service = test_support::service(BoardSettings::default());
        let task = service
            .create_task(NewTask {
                title: "新着".to_owned(),
                ..NewTask::default()
            })
            .unwrap();
        assert!(should_show(&service, true));

        service.delete_task(task.id).unwrap();
        assert!(!should_show(&service, true));

        service.restore_task(task.id).unwrap();
        assert!(should_show(&service, true));
    }

    #[test]
    fn hidden_when_overlay_is_disabled() {
        let service = test_support::service(BoardSettings::default());
        service
            .create_task(NewTask {
                title: "新着".to_owned(),
                ..NewTask::default()
            })
            .unwrap();
        assert!(!should_show(&service, false));
    }
}
