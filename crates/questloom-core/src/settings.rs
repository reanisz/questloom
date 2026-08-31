//! コア設定モデル。`settings` テーブルの `core` 名前空間に JSON で保存される。

use chrono::Weekday;
use serde::{Deserialize, Serialize};

/// 設定の名前空間名(コア設定)。
pub const CORE_NAMESPACE: &str = "core";

/// 週の開始曜日。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WeekStart {
    /// 月曜始まり(既定)。ISO 8601 と一致する。
    #[default]
    Monday,
    /// 日曜始まり。
    Sunday,
}

impl WeekStart {
    /// [`chrono::Weekday`] へ変換する。
    #[must_use]
    pub const fn weekday(self) -> Weekday {
        match self {
            Self::Monday => Weekday::Mon,
            Self::Sunday => Weekday::Sun,
        }
    }
}

/// グローバルショートカットの既定値。
///
/// 文字列の解釈はシェル(デスクトップアプリ)側の責務。コアは値を保持するだけで、
/// ここでの検証・パースは行わない(コアを UI・Tauri から独立させるため)。
pub const DEFAULT_GLOBAL_SHORTCUT: &str = "Ctrl+Space";

/// 内蔵 MCP サーバーの既定ポート。127.0.0.1 のみにバインドする。
pub const DEFAULT_MCP_PORT: u16 = 39150;

/// コア設定。未知フィールドは無視し、欠けたフィールドは既定値で補う(前方互換)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CoreSettings {
    /// 週の開始曜日。バケット導出に用いる。
    pub week_start: WeekStart,
    /// バックアップの保持世代数。
    pub backup_generations: u32,
    /// オーバーレイ通知を表示するか。
    pub overlay_enabled: bool,
    /// メインウィンドウをトグルするグローバルショートカット。
    pub global_shortcut: String,
    /// OS へのログイン時に自動起動するか。
    pub autostart: bool,
    /// 内蔵 MCP サーバーを起動するか。
    pub mcp_enabled: bool,
    /// MCP サーバーの待受ポート(バインドは 127.0.0.1 のみ)。
    pub mcp_port: u16,
    /// MCP サーバーの Bearer トークン。`None` なら認証なし。
    ///
    /// 当面は設定ファイル内に平文で保持する。keyring への移設は将来の課題。
    pub mcp_token: Option<String>,
}

impl Default for CoreSettings {
    fn default() -> Self {
        Self {
            week_start: WeekStart::Monday,
            backup_generations: 14,
            overlay_enabled: true,
            global_shortcut: DEFAULT_GLOBAL_SHORTCUT.to_owned(),
            autostart: false,
            mcp_enabled: true,
            mcp_port: DEFAULT_MCP_PORT,
            mcp_token: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_docs() {
        let settings = CoreSettings::default();
        assert_eq!(settings.week_start, WeekStart::Monday);
        assert_eq!(settings.backup_generations, 14);
        assert!(settings.overlay_enabled);
        assert_eq!(settings.global_shortcut, "Ctrl+Space");
        assert!(!settings.autostart);
        assert!(settings.mcp_enabled);
        assert_eq!(settings.mcp_port, DEFAULT_MCP_PORT);
        assert_eq!(settings.mcp_token, None);
    }

    #[test]
    fn missing_and_unknown_fields_are_tolerated() {
        let parsed: CoreSettings =
            serde_json::from_str(r#"{"weekStart":"sunday","futureField":123}"#).unwrap();
        assert_eq!(parsed.week_start, WeekStart::Sunday);
        assert_eq!(parsed.backup_generations, 14);
        // Phase 1 以前に保存された JSON でも、追加フィールドは既定値で補われる。
        assert!(parsed.overlay_enabled);
        assert_eq!(parsed.global_shortcut, DEFAULT_GLOBAL_SHORTCUT);
        assert!(parsed.mcp_enabled);
        assert_eq!(parsed.mcp_port, DEFAULT_MCP_PORT);
    }

    #[test]
    fn json_is_camel_case() {
        let json = serde_json::to_value(CoreSettings::default()).unwrap();
        assert_eq!(json["overlayEnabled"], true);
        assert_eq!(json["globalShortcut"], "Ctrl+Space");
        assert_eq!(json["autostart"], false);
        assert_eq!(json["mcpEnabled"], true);
        assert_eq!(json["mcpPort"], 39150);
        assert!(json["mcpToken"].is_null());
    }

    #[test]
    fn empty_object_yields_defaults() {
        let parsed: CoreSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, CoreSettings::default());
    }
}
