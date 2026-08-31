//! コア設定のデスクトップ側への反映。
//!
//! 設定は `set_settings` で保存されると
//! [`DomainEvent::SettingsChanged`] が飛ぶので、それを購読して
//! ショートカットの再登録と自動起動の同期を行う。
//! オーバーレイの表示可否は [`crate::overlay`] の watcher 側で再評価される。

use std::sync::Arc;

use questloom_core::events::DomainEvent;
use questloom_core::service::TaskService;
use questloom_core::settings::CoreSettings;
use tauri::{AppHandle, Runtime};
use tokio::sync::broadcast::error::RecvError;

use crate::{autostart, shortcut};

/// 設定値をデスクトップ側(ショートカット・自動起動)へ反映する。
pub fn apply<R: Runtime>(app: &AppHandle<R>, settings: &CoreSettings) {
    shortcut::apply(app, &settings.global_shortcut);
    autostart::apply(app, settings.autostart);
}

/// 設定変更イベントを購読し、反映するタスクを開始する。
pub fn spawn_watcher<R: Runtime>(app: AppHandle<R>, service: &Arc<TaskService>) {
    let mut receiver = service.subscribe();
    let service = Arc::clone(service);
    tauri::async_runtime::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(DomainEvent::SettingsChanged) => apply(&app, &service.settings()),
                Ok(_) => {}
                Err(RecvError::Lagged(missed)) => {
                    // 取りこぼした中に設定変更が含まれうるので、念のため反映し直す。
                    tracing::warn!(missed, "設定監視でイベントを取りこぼしました");
                    apply(&app, &service.settings());
                }
                Err(RecvError::Closed) => {
                    tracing::debug!("設定変更の購読を終了します");
                    break;
                }
            }
        }
    });
}
