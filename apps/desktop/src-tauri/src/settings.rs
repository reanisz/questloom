//! コア設定のデスクトップ側への反映と、保存前の検証。
//!
//! 設定は `set_settings` で保存されると
//! [`DomainEvent::SettingsChanged`] が飛ぶので、それを購読して
//! ショートカットの再登録・自動起動の同期・MCP サーバーの再起動を行う。
//! オーバーレイの表示可否は [`crate::overlay`] の watcher 側で再評価される。
//!
//! [`validate`] は、コア側の検証([`CoreSettings::validate`])に、コアには
//! 置けない(UI・Tauri 非依存を保つため)ショートカット文字列のパースを足す。

use std::str::FromStr;
use std::sync::Arc;

use questloom_core::events::DomainEvent;
use questloom_core::service::TaskService;
use questloom_core::settings::CoreSettings;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::Shortcut;

use crate::events::{spawn_domain_watcher, DomainSignal};
use crate::state::AppState;
use crate::{autostart, mcp, shortcut};

pub use questloom_core::settings::{AI_TIMEOUT_RANGE, MIN_MCP_PORT};

/// 保存前にコア設定を検証する。
///
/// フロント側にも同じ規則の検証があるが、MCP 経由や将来の呼び出し元でも
/// 壊れた設定が保存されないよう、境界であるここでも必ず検査する。
///
/// 値の範囲・プロバイダ定義の整合性は [`CoreSettings::validate`] が見る。
/// ここが足すのは、Tauri のショートカット文字列を実際にパースできるかだけ。
///
/// # Errors
/// 値が不正な場合、フロントへそのまま出せる日本語のメッセージを返す。
pub fn validate(settings: &CoreSettings) -> Result<(), String> {
    settings.validate().map_err(|error| error.to_string())?;

    // 空文字列は「ショートカットなし」として許す。
    let spec = settings.global_shortcut.trim();
    if !spec.is_empty() && Shortcut::from_str(spec).is_err() {
        return Err(format!(
            "グローバルショートカット \"{spec}\" を解釈できません(例: Ctrl+Space, Alt+Shift+Q)。"
        ));
    }

    Ok(())
}

/// 設定値をデスクトップ側(ショートカット・自動起動・MCP サーバー)へ反映する。
pub fn apply<R: Runtime>(app: &AppHandle<R>, settings: &CoreSettings) {
    shortcut::apply(app, &settings.global_shortcut);
    autostart::apply(app, settings.autostart);
    mcp::apply(app, settings);
}

/// 現在保持しているコア設定をデスクトップ側へ反映する。
fn apply_current<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AppState>() else {
        tracing::warn!("アプリ状態が未登録のため設定を反映できません");
        return;
    };
    apply(app, &state.settings());
}

/// 設定変更イベントを購読し、反映するタスクを開始する。
///
/// 設定の実体は [`AppState`] が持つので、イベントは「読み直す合図」として使う。
pub fn spawn_watcher<R: Runtime>(app: AppHandle<R>, service: &Arc<TaskService>) {
    spawn_domain_watcher(service, "settings", move |signal| match signal {
        DomainSignal::Event(DomainEvent::SettingsChanged)
        // 取りこぼした中に設定変更が含まれうるので、念のため反映し直す。
        | DomainSignal::Lagged(_) => apply_current(&app),
        DomainSignal::Event(_) => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert_eq!(validate(&CoreSettings::default()), Ok(()));
    }

    #[test]
    fn shortcut_must_parse_but_may_be_empty() {
        let empty = CoreSettings {
            global_shortcut: "  ".to_owned(),
            ..CoreSettings::default()
        };
        assert_eq!(validate(&empty), Ok(()), "空はショートカットなしとして許す");

        let ok = CoreSettings {
            global_shortcut: "Alt+Shift+Q".to_owned(),
            ..CoreSettings::default()
        };
        assert_eq!(validate(&ok), Ok(()));

        let broken = CoreSettings {
            global_shortcut: "Ctrl+".to_owned(),
            ..CoreSettings::default()
        };
        assert!(validate(&broken).is_err());

        let nonsense = CoreSettings {
            global_shortcut: "とても長い日本語".to_owned(),
            ..CoreSettings::default()
        };
        assert!(validate(&nonsense).is_err());
    }

    /// 値の範囲・プロバイダ定義の検証本体は questloom-core にある
    /// (`settings::tests` を参照)。ここではそれが合成されていることだけ確かめる。
    #[test]
    fn core_rules_are_included() {
        let broken = CoreSettings {
            backup_generations: 0,
            ..CoreSettings::default()
        };
        assert_eq!(
            validate(&broken),
            Err(broken.validate().unwrap_err().to_string())
        );

        assert!(validate(&CoreSettings {
            mcp_port: 80,
            ..CoreSettings::default()
        })
        .is_err());
        assert!(validate(&CoreSettings {
            mcp_port: MIN_MCP_PORT,
            ..CoreSettings::default()
        })
        .is_ok());
        assert!(AI_TIMEOUT_RANGE.contains(&CoreSettings::default().ai_timeout_secs));
    }
}
