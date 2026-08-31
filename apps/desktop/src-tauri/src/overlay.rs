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

use questloom_core::model::TaskStatus;
use questloom_core::service::TaskService;
use tauri::{AppHandle, LogicalPosition, Manager, Runtime, WebviewWindow};
use tokio::sync::broadcast::error::RecvError;

use crate::state::AppState;

/// オーバーレイウィンドウのラベル(`tauri.conf.json` と一致させること)。
pub const OVERLAY_WINDOW: &str = "overlay";

/// メインディスプレイ左上からのマージン(論理ピクセル)。
const MARGIN: f64 = 12.0;

/// New タスク件数と設定に応じて、オーバーレイの表示 / 非表示を切り替える。
pub fn sync<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AppState>() else {
        // 初期化前に呼ばれた場合は何もしない。
        return;
    };
    let visible = should_show(&state.service);
    apply(app, visible);
}

/// オーバーレイを表示すべきか(オーバーレイ有効 かつ New タスクが 1 件以上)。
fn should_show(service: &TaskService) -> bool {
    if !service.settings().overlay_enabled {
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

/// ドメインイベントを購読し、そのたびに表示判定をやり直すタスクを開始する。
///
/// 判定はタスク件数と設定にしか依存しないため、イベントの種類は区別しない。
pub fn spawn_watcher<R: Runtime>(app: AppHandle<R>, service: &Arc<TaskService>) {
    let mut receiver = service.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(_) => sync(&app),
                Err(RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "オーバーレイ更新でイベントを取りこぼしました");
                    sync(&app);
                }
                Err(RecvError::Closed) => {
                    tracing::debug!("オーバーレイの購読を終了します");
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use questloom_core::clock::SystemClock;
    use questloom_core::repository::TaskRepository;
    use questloom_core::service::NewTask;
    use questloom_core::settings::CoreSettings;
    use questloom_store::SqliteStore;

    fn service(settings: CoreSettings) -> TaskService {
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let repo: Arc<dyn TaskRepository> = store as Arc<dyn TaskRepository>;
        TaskService::new(repo, Arc::new(SystemClock), settings)
    }

    #[test]
    fn hidden_without_new_tasks() {
        let service = service(CoreSettings::default());
        assert!(!should_show(&service));

        service
            .create_task(NewTask {
                title: "新着".to_owned(),
                ..NewTask::default()
            })
            .unwrap();
        assert!(should_show(&service));
    }

    #[test]
    fn hidden_when_overlay_is_disabled() {
        let service = service(CoreSettings {
            overlay_enabled: false,
            ..CoreSettings::default()
        });
        service
            .create_task(NewTask {
                title: "新着".to_owned(),
                ..NewTask::default()
            })
            .unwrap();
        assert!(!should_show(&service));
    }
}
