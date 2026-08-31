//! グローバルショートカット(既定 Ctrl+Space)の登録。
//!
//! 押下でメインウィンドウをトグルする。ショートカット文字列は設定値
//! ([`CoreSettings::global_shortcut`](questloom_core::settings::CoreSettings::global_shortcut))
//! から読む。他アプリと衝突して登録できない場合でも、警告ログのみでアプリは動き続ける。

use std::str::FromStr;
use std::sync::Mutex;

use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::window;

/// いま登録できているショートカット。未登録(設定なし・解釈不能・登録失敗)は `None`。
///
/// [`apply`] は設定変更のたびに呼ばれるが、実際に変化したときだけ張り直す
/// ([`McpSupervisor::apply`](crate::mcp::McpSupervisor::apply) と同じ差分チェック)。
/// 無関係な設定を保存するたびに OS のホットキー登録を解除・再登録すると、
/// その瞬間だけ他アプリに奪われうるため。
static REGISTERED: Mutex<Option<Shortcut>> = Mutex::new(None);

/// 設定文字列から、登録すべきショートカットを決める。
///
/// 空文字列は「ショートカットなし」。解釈できない文字列も警告を出して `None` にする
/// (保存時に [`crate::settings::validate`] が弾くので、ここへ来るのは異常系のみ)。
fn desired(spec: &str) -> Option<Shortcut> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    match Shortcut::from_str(spec) {
        Ok(shortcut) => Some(shortcut),
        Err(error) => {
            tracing::warn!(spec, %error, "グローバルショートカットを解釈できませんでした");
            None
        }
    }
}

/// 指定のショートカットを登録し直す。
///
/// 現在の登録内容から変化が無ければ何もしない(冪等)。
pub fn apply<R: Runtime>(app: &AppHandle<R>, spec: &str) {
    let target = desired(spec);
    let mut registered = REGISTERED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *registered == target {
        return;
    }

    let manager = app.global_shortcut();
    if let Err(error) = manager.unregister_all() {
        tracing::warn!(%error, "既存のグローバルショートカットを解除できませんでした");
    }
    *registered = None;

    let Some(shortcut) = target else {
        tracing::info!("グローバルショートカットは設定されていません");
        return;
    };

    let result = manager.on_shortcut(shortcut, |app, _shortcut, event| {
        // 押下と離上の両方で呼ばれるため、押下のみを拾う。
        if event.state() == ShortcutState::Pressed {
            window::toggle_main(app);
        }
    });
    match result {
        Ok(()) => {
            *registered = Some(shortcut);
            tracing::info!(spec, "グローバルショートカットを登録しました");
        }
        // 失敗は記録しない。次回の apply で(設定が同じでも)もう一度試せるようにする。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_shortcut_for_empty_or_broken_specs() {
        assert_eq!(desired(""), None);
        assert_eq!(desired("   "), None);
        assert_eq!(desired("Ctrl+"), None);
        assert_eq!(desired("とても長い日本語"), None);
    }

    /// 同じ意味の文字列は同じショートカットになる。差分チェックが
    /// 「書き方が違うだけ」で張り直さないことの根拠。
    #[test]
    fn equivalent_specs_compare_equal() {
        assert_eq!(desired("Ctrl+Space"), desired(" Ctrl+Space "));
        assert_eq!(desired("Alt+Shift+Q"), desired("alt+shift+KeyQ"));
        assert_ne!(desired("Ctrl+Space"), desired("Alt+Shift+Q"));
    }
}
