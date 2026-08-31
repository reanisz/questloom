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

/// AI CLI の既定プロバイダ ID。
pub const DEFAULT_AI_PROVIDER_ID: &str = "claude";

/// AI CLI 実行の既定タイムアウト(秒)。
pub const DEFAULT_AI_TIMEOUT_SECS: u64 = 300;

/// AI CLI プロバイダの定義。
///
/// コアは値を保持するだけで、実行方法(プロセス起動・プレースホルダ置換)は
/// `questloom-ai` の責務。プレースホルダは以下。
///
/// - `args` 内の `{prompt}` — プロンプト本文
/// - `mcp_args` 内の `{mcp_url}` — 内蔵 MCP サーバーの URL
/// - `mcp_args` 内の `{mcp_config}` — Claude Code の `--mcp-config` に渡す JSON
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiProvider {
    /// 識別子(設定内で一意)。
    pub id: String,
    /// UI に出す表示名。
    pub label: String,
    /// 実行ファイル名。PATH から解決する。
    pub command: String,
    /// 引数テンプレート。`{prompt}` が置換される。
    pub args: Vec<String>,
    /// このプロバイダを使えるようにするか。
    pub enabled: bool,
    /// MCP 接続時に `args` の前へ挿入する引数。空なら MCP 非対応。
    pub mcp_args: Vec<String>,
    /// MCP の Bearer トークン(認証ヘッダ)を CLI へ渡せるか。
    ///
    /// 偽のプロバイダは、トークンが設定されている間 MCP 接続を諦める。
    pub mcp_supports_token: bool,
}

impl Default for AiProvider {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            command: String::new(),
            args: Vec::new(),
            enabled: true,
            mcp_args: Vec::new(),
            mcp_supports_token: false,
        }
    }
}

/// 既定のプロバイダ定義(claude / codex / antigravity)。
#[must_use]
pub fn default_ai_providers() -> Vec<AiProvider> {
    vec![
        AiProvider {
            id: "claude".to_owned(),
            label: "Claude Code".to_owned(),
            command: "claude".to_owned(),
            args: vec!["-p".to_owned(), "{prompt}".to_owned()],
            enabled: true,
            // Claude Code は `--mcp-config` に JSON をそのまま渡せる(ヘッダ指定も可)。
            mcp_args: vec![
                "--mcp-config".to_owned(),
                "{mcp_config}".to_owned(),
                "--allowedTools".to_owned(),
                "mcp__questloom__*".to_owned(),
            ],
            mcp_supports_token: true,
        },
        AiProvider {
            id: "codex".to_owned(),
            label: "Codex".to_owned(),
            command: "codex".to_owned(),
            args: vec!["exec".to_owned(), "{prompt}".to_owned()],
            enabled: true,
            // codex は `-c` の設定上書きで Streamable HTTP の MCP サーバーを足せる
            // (`url` を持つエントリは rmcp クライアント経由になる)。
            // 認証ヘッダは環境変数経由でしか渡せないため、トークン設定時は非対応扱い。
            mcp_args: vec![
                "-c".to_owned(),
                "features.experimental_use_rmcp_client=true".to_owned(),
                "-c".to_owned(),
                "mcp_servers.questloom.url=\"{mcp_url}\"".to_owned(),
            ],
            mcp_supports_token: false,
        },
        AiProvider {
            // 引数仕様が確認できていないため、既定では無効にしておく。
            id: "antigravity".to_owned(),
            label: "Antigravity".to_owned(),
            command: "antigravity".to_owned(),
            args: vec!["-p".to_owned(), "{prompt}".to_owned()],
            enabled: false,
            mcp_args: Vec::new(),
            mcp_supports_token: false,
        },
    ]
}

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
    /// AI CLI プロバイダの一覧。
    pub ai_providers: Vec<AiProvider>,
    /// 既定で使うプロバイダの `id`。
    pub ai_default_provider_id: String,
    /// AI CLI 実行のタイムアウト(秒)。超えたらプロセスを kill する。
    pub ai_timeout_secs: u64,
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
            ai_providers: default_ai_providers(),
            ai_default_provider_id: DEFAULT_AI_PROVIDER_ID.to_owned(),
            ai_timeout_secs: DEFAULT_AI_TIMEOUT_SECS,
        }
    }
}

impl CoreSettings {
    /// `id` のプロバイダを探す。`None` なら既定プロバイダ。
    ///
    /// 無効化されたプロバイダは見つからない扱いにする。
    #[must_use]
    pub fn ai_provider(&self, id: Option<&str>) -> Option<&AiProvider> {
        let id = id.unwrap_or(&self.ai_default_provider_id);
        self.ai_providers
            .iter()
            .find(|provider| provider.id == id && provider.enabled)
    }

    /// 有効なプロバイダの一覧。
    pub fn enabled_ai_providers(&self) -> impl Iterator<Item = &AiProvider> {
        self.ai_providers.iter().filter(|provider| provider.enabled)
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
        assert_eq!(settings.ai_default_provider_id, "claude");
        assert_eq!(settings.ai_timeout_secs, 300);
    }

    #[test]
    fn default_ai_providers_match_docs() {
        let settings = CoreSettings::default();
        let ids: Vec<&str> = settings
            .ai_providers
            .iter()
            .map(|provider| provider.id.as_str())
            .collect();
        assert_eq!(ids, ["claude", "codex", "antigravity"]);

        let claude = settings.ai_provider(None).expect("既定は claude");
        assert_eq!(claude.command, "claude");
        assert_eq!(claude.args, ["-p", "{prompt}"]);
        assert!(claude.mcp_supports_token);
        assert!(claude.mcp_args.iter().any(|arg| arg == "{mcp_config}"));

        let codex = settings.ai_provider(Some("codex")).expect("codex は有効");
        assert_eq!(codex.args, ["exec", "{prompt}"]);
        assert!(!codex.mcp_supports_token);
        assert!(codex.mcp_args.iter().any(|arg| arg.contains("{mcp_url}")));

        // antigravity は仕様不明のため既定で無効。
        assert!(settings.ai_provider(Some("antigravity")).is_none());
        assert_eq!(settings.enabled_ai_providers().count(), 2);
        assert!(settings.ai_provider(Some("unknown")).is_none());
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
        // Phase 4 以前に保存された JSON でも、AI プロバイダは既定の 3 つで補われる。
        assert_eq!(parsed.ai_providers, default_ai_providers());
        assert_eq!(parsed.ai_timeout_secs, DEFAULT_AI_TIMEOUT_SECS);
    }

    #[test]
    fn ai_provider_json_is_camel_case_and_tolerates_missing_fields() {
        let json = serde_json::to_value(CoreSettings::default()).unwrap();
        assert_eq!(json["aiDefaultProviderId"], "claude");
        assert_eq!(json["aiTimeoutSecs"], 300);
        assert_eq!(json["aiProviders"][0]["id"], "claude");
        assert_eq!(json["aiProviders"][0]["command"], "claude");
        assert_eq!(json["aiProviders"][0]["enabled"], true);
        assert_eq!(json["aiProviders"][0]["mcpSupportsToken"], true);
        assert_eq!(json["aiProviders"][2]["enabled"], false);

        // 最小限のプロバイダ定義でも読める(mcpArgs 等は既定値)。
        let parsed: CoreSettings = serde_json::from_str(
            r#"{"aiProviders":[{"id":"x","label":"X","command":"x","args":["{prompt}"]}]}"#,
        )
        .unwrap();
        assert_eq!(parsed.ai_providers.len(), 1);
        assert!(parsed.ai_providers[0].enabled);
        assert!(parsed.ai_providers[0].mcp_args.is_empty());
        assert!(!parsed.ai_providers[0].mcp_supports_token);
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
