//! OS ログイン時の自動起動(tauri-plugin-autostart)の同期。
//!
//! 設定値を「あるべき状態」とみなし、実際の登録状態がずれていれば合わせる。

use tauri::{AppHandle, Runtime};
use tauri_plugin_autostart::ManagerExt;

/// 設定値どおりに自動起動の登録状態を合わせる。
pub fn apply<R: Runtime>(app: &AppHandle<R>, enabled: bool) {
    let manager = app.autolaunch();
    match manager.is_enabled() {
        Ok(current) if current == enabled => {
            tracing::debug!(enabled, "自動起動の設定は既に一致しています");
            return;
        }
        Ok(_) => {}
        // 状態を読めなくても、設定どおりの登録は試みる。
        Err(error) => tracing::warn!(%error, "自動起動の状態を取得できませんでした"),
    }

    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    match result {
        Ok(()) => tracing::info!(enabled, "自動起動の設定を更新しました"),
        Err(error) => tracing::warn!(enabled, %error, "自動起動の設定を更新できませんでした"),
    }
}
