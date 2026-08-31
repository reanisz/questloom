//! コア設定のデスクトップ側への反映と、保存前の検証。
//!
//! 設定は `set_settings` で保存されると
//! [`DomainEvent::SettingsChanged`] が飛ぶので、それを購読して
//! ショートカットの再登録・自動起動の同期・MCP サーバーの再起動を行う。
//! オーバーレイの表示可否は [`crate::overlay`] の watcher 側で再評価される。
//!
//! [`validate`] は、コアには置けない(UI・Tauri 非依存を保つため)
//! ショートカット文字列のパースを含む検証をシェル側で行う。

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;

use questloom_core::events::DomainEvent;
use questloom_core::service::TaskService;
use questloom_core::settings::CoreSettings;
use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::Shortcut;
use tokio::sync::broadcast::error::RecvError;

use crate::{autostart, mcp, shortcut};

/// MCP サーバーに許すポートの下限(well-known ポートは避ける)。
pub const MIN_MCP_PORT: u16 = 1024;

/// AI CLI のタイムアウトに許す範囲(秒)。
pub const AI_TIMEOUT_RANGE: std::ops::RangeInclusive<u64> = 10..=3600;

/// 保存前にコア設定を検証する。
///
/// フロント側にも同じ規則の検証があるが、MCP 経由や将来の呼び出し元でも
/// 壊れた設定が保存されないよう、境界であるここでも必ず検査する。
///
/// # Errors
/// 値が不正な場合、フロントへそのまま出せる日本語のメッセージを返す。
pub fn validate(settings: &CoreSettings) -> Result<(), String> {
    if settings.backup_generations < 1 {
        return Err("バックアップ世代数は 1 以上にしてください。".to_owned());
    }
    if settings.mcp_port < MIN_MCP_PORT {
        return Err(format!(
            "MCP ポートは {MIN_MCP_PORT}〜65535 の範囲で指定してください。"
        ));
    }
    if !AI_TIMEOUT_RANGE.contains(&settings.ai_timeout_secs) {
        return Err(format!(
            "AI のタイムアウトは {}〜{} 秒の範囲で指定してください。",
            AI_TIMEOUT_RANGE.start(),
            AI_TIMEOUT_RANGE.end()
        ));
    }

    // 空文字列は「ショートカットなし」として許す。
    let spec = settings.global_shortcut.trim();
    if !spec.is_empty() && Shortcut::from_str(spec).is_err() {
        return Err(format!(
            "グローバルショートカット \"{spec}\" を解釈できません(例: Ctrl+Space, Alt+Shift+Q)。"
        ));
    }

    let mut seen = HashSet::new();
    for provider in &settings.ai_providers {
        let id = provider.id.trim();
        if id.is_empty() {
            return Err("AI プロバイダの id を入力してください。".to_owned());
        }
        if !seen.insert(id) {
            return Err(format!("AI プロバイダの id \"{id}\" が重複しています。"));
        }
        if provider.label.trim().is_empty() {
            return Err(format!(
                "AI プロバイダ \"{id}\" の表示名を入力してください。"
            ));
        }
        if provider.command.trim().is_empty() {
            return Err(format!(
                "AI プロバイダ \"{id}\" の command を入力してください。"
            ));
        }
    }

    if settings.ai_provider(None).is_none() {
        return Err(format!(
            "既定の AI プロバイダ \"{}\" が存在しないか無効です。",
            settings.ai_default_provider_id
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

#[cfg(test)]
mod tests {
    use super::*;
    use questloom_core::settings::AiProvider;

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

    #[test]
    fn numeric_ranges_are_checked() {
        assert!(validate(&CoreSettings {
            backup_generations: 0,
            ..CoreSettings::default()
        })
        .is_err());
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
        assert!(validate(&CoreSettings {
            ai_timeout_secs: 9,
            ..CoreSettings::default()
        })
        .is_err());
        assert!(validate(&CoreSettings {
            ai_timeout_secs: 3601,
            ..CoreSettings::default()
        })
        .is_err());
    }

    #[test]
    fn providers_need_unique_ids_and_a_usable_default() {
        let provider = |id: &str| AiProvider {
            id: id.to_owned(),
            label: id.to_owned(),
            command: id.to_owned(),
            args: vec!["{prompt}".to_owned()],
            enabled: true,
            ..AiProvider::default()
        };

        assert!(validate(&CoreSettings {
            ai_providers: vec![provider("a"), provider("a")],
            ai_default_provider_id: "a".to_owned(),
            ..CoreSettings::default()
        })
        .is_err());

        assert!(validate(&CoreSettings {
            ai_providers: vec![AiProvider {
                command: String::new(),
                ..provider("a")
            }],
            ai_default_provider_id: "a".to_owned(),
            ..CoreSettings::default()
        })
        .is_err());

        // 既定プロバイダが無効なら保存させない。
        assert!(validate(&CoreSettings {
            ai_providers: vec![AiProvider {
                enabled: false,
                ..provider("a")
            }],
            ai_default_provider_id: "a".to_owned(),
            ..CoreSettings::default()
        })
        .is_err());

        assert!(validate(&CoreSettings {
            ai_providers: vec![provider("a"), provider("b")],
            ai_default_provider_id: "b".to_owned(),
            ..CoreSettings::default()
        })
        .is_ok());
    }
}
