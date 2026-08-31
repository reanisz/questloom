//! グローバルショートカット(既定 Ctrl+Space)の登録。
//!
//! 押下でメインウィンドウをトグルする。ショートカット文字列は設定値
//! ([`CoreSettings::global_shortcut`](questloom_core::settings::CoreSettings::global_shortcut))
//! から読む。他アプリと衝突して登録できない場合でも、警告ログのみでアプリは動き続ける。

use std::str::FromStr;

use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::window;

/// 現在の登録をすべて解除し、指定のショートカットを登録し直す。
///
/// 空文字列の場合は「ショートカットなし」として何も登録しない。
pub fn apply<R: Runtime>(app: &AppHandle<R>, spec: &str) {
    let manager = app.global_shortcut();
    if let Err(error) = manager.unregister_all() {
        tracing::warn!(%error, "既存のグローバルショートカットを解除できませんでした");
    }

    let spec = spec.trim();
    if spec.is_empty() {
        tracing::info!("グローバルショートカットは設定されていません");
        return;
    }

    let shortcut = match Shortcut::from_str(spec) {
        Ok(shortcut) => shortcut,
        Err(error) => {
            tracing::warn!(spec, %error, "グローバルショートカットを解釈できませんでした");
            return;
        }
    };

    let result = manager.on_shortcut(shortcut, |app, _shortcut, event| {
        // 押下と離上の両方で呼ばれるため、押下のみを拾う。
        if event.state() == ShortcutState::Pressed {
            window::toggle_main(app);
        }
    });
    match result {
        Ok(()) => tracing::info!(spec, "グローバルショートカットを登録しました"),
        Err(error) => tracing::warn!(
            spec,
            %error,
            "グローバルショートカットを登録できませんでした(他アプリが使用中の可能性があります)"
        ),
    }
}

/// 指定のショートカットが実際に登録できているか。
///
/// 空文字列(ショートカットなし)や解釈できない文字列は「登録されていない」とみなす。
/// 設定画面の稼働状態表示に使う。
pub fn is_registered<R: Runtime>(app: &AppHandle<R>, spec: &str) -> bool {
    let spec = spec.trim();
    if spec.is_empty() {
        return false;
    }
    Shortcut::from_str(spec).is_ok_and(|shortcut| app.global_shortcut().is_registered(shortcut))
}
