//! メインウィンドウの表示制御。
//!
//! トレイ・グローバルショートカット・オーバーレイのいずれからも同じ入口を通す。
//! 閉じる操作は終了ではなくトレイ格納(hide)で、実際の終了はトレイメニューからのみ。

use questloom_core::model::TaskId;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};

/// メインウィンドウのラベル(`tauri.conf.json` と一致させること)。
pub const MAIN_WINDOW: &str = "main";

/// メインウィンドウでタスク詳細を開かせるイベント名。
///
/// オーバーレイから通常タスクをクリックしたときに、[`TASKS_CHANGED`](crate::events::TASKS_CHANGED)
/// とは別経路でメインウィンドウのみへ送る。
pub const OPEN_TASK: &str = "questloom://open-task";

/// [`OPEN_TASK`] のペイロード。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTask {
    /// 開くタスク。
    pub task_id: TaskId,
}

/// メインウィンドウを取得する。まだ生成されていない場合は `None`。
#[must_use]
pub fn main_window<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.get_webview_window(MAIN_WINDOW)
}

/// メインウィンドウを表示してフォーカスし、必要ならタスク詳細を開かせる。
pub fn show_main<R: Runtime>(app: &AppHandle<R>, task_id: Option<TaskId>) {
    let Some(window) = main_window(app) else {
        tracing::warn!("メインウィンドウが見つかりません");
        return;
    };
    if window.is_minimized().unwrap_or(false) {
        let _ = window.unminimize();
    }
    if let Err(error) = window.show() {
        tracing::warn!(%error, "メインウィンドウを表示できませんでした");
    }
    if let Err(error) = window.set_focus() {
        tracing::warn!(%error, "メインウィンドウをフォーカスできませんでした");
    }
    if let Some(task_id) = task_id {
        if let Err(error) = app.emit_to(MAIN_WINDOW, OPEN_TASK, OpenTask { task_id }) {
            tracing::warn!(%error, "タスクを開く通知に失敗しました");
        }
    }
}

/// メインウィンドウを隠す(トレイ格納)。
pub fn hide_main<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = main_window(app) else {
        return;
    };
    if let Err(error) = window.hide() {
        tracing::warn!(%error, "メインウィンドウを隠せませんでした");
    }
}

/// メインウィンドウの表示状態をトグルする。
///
/// 表示中でもフォーカスが無い(最小化・背面)場合は隠さず前面に出す。
pub fn toggle_main<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = main_window(app) else {
        tracing::warn!("メインウィンドウが見つかりません");
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    let focused = window.is_focused().unwrap_or(false);
    if visible && !minimized && focused {
        hide_main(app);
    } else {
        show_main(app, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_name_is_valid_for_tauri() {
        // Tauri v2 が許容するのは英数字と `-` `/` `:` `_` のみ。
        assert!(OPEN_TASK
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '/' | ':' | '_')));
    }

    #[test]
    fn open_task_payload_is_camel_case() {
        let id = TaskId::new();
        let json = serde_json::to_value(OpenTask { task_id: id }).unwrap();
        assert_eq!(json["taskId"], id.to_string());
    }
}
