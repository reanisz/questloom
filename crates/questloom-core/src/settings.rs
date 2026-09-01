//! コア設定モデル。`settings` テーブルの `core` 名前空間に JSON で保存される。
//!
//! 値の検証は [`CoreSettings::validate`] にある。UI・Tauri に依存しない規則
//! (数値の範囲・プロバイダ定義の整合性)はすべてここで判定し、シェル側は
//! それにショートカット文字列のパースを足すだけでよい。

use std::collections::HashSet;

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

/// URL の関連リソースをクリックしたときの既定の開き方。
///
/// 「内蔵ブラウザで開く / 既定のブラウザで開く」の**明示的な**操作は、この設定に
/// 関わらずどちらも使える(右クリックメニューとリソース行のボタン)。ここで決めるのは
/// あくまで**クリックしたときの既定**と、詳細を開いたときの自動表示。
///
/// 内蔵ブラウザの実体はシェル側(`questloom-desktop` の `browser` モジュール)にあり、
/// コアは利用者の選択を保持するだけ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UrlOpenMode {
    /// OS の既定ブラウザで開く(既定)。
    #[default]
    External,
    /// アプリ内蔵のブラウザペインで開く。
    Internal,
    /// 内蔵ペインで開き、さらにタスク詳細を開いたとき主リソースが URL なら自動で表示する。
    InternalAuto,
}

impl UrlOpenMode {
    /// 内蔵ペインを既定の開き先にするモードか。
    #[must_use]
    pub const fn uses_internal_pane(self) -> bool {
        matches!(self, Self::Internal | Self::InternalAuto)
    }

    /// タスク詳細を開いたときに主リソースを自動で表示するか。
    #[must_use]
    pub const fn opens_automatically(self) -> bool {
        matches!(self, Self::InternalAuto)
    }
}

/// ボード表示に関わる設定だけを抜き出したもの。
///
/// [`TaskService`](crate::service::TaskService) が保持するのはこれだけで、
/// 設定全体([`CoreSettings`])の読み書き・配布はシェル(アプリ)側の責務。
/// サービスがボード以外の設定(MCP・AI・ショートカット等)を抱えないようにするため。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardSettings {
    /// 週の開始曜日。バケット導出に用いる。
    pub week_start: WeekStart,
}

impl From<&CoreSettings> for BoardSettings {
    fn from(settings: &CoreSettings) -> Self {
        Self {
            week_start: settings.week_start,
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

/// MCP サーバーに許すポートの下限(well-known ポートは避ける)。
pub const MIN_MCP_PORT: u16 = 1024;

/// AI CLI のタイムアウトに許す範囲(秒)。
pub const AI_TIMEOUT_RANGE: std::ops::RangeInclusive<u64> = 10..=3600;

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
            // `--skip-git-repo-check`: codex exec は信頼済みディレクトリ(git リポジトリ等)の
            // 外では実行を拒否する。questloom はアプリの作業ディレクトリから spawn するため、
            // このフラグが無いと「Not inside a trusted directory」で失敗する。
            args: vec![
                "exec".to_owned(),
                "--skip-git-repo-check".to_owned(),
                "{prompt}".to_owned(),
            ],
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

/// 保存済み設定に残った「旧バージョンの既定値」を現行の既定値へ引き上げる。
///
/// 既定値の修正はフィールド既定(serde default)では既存ユーザーに届かないため、
/// **旧既定と完全一致する場合に限って**書き換える(ユーザーが編集した値には触れない)。
/// 変更があれば true を返す(呼び出し側が保存し直す)。
///
/// 現在の対象:
/// - codex プロバイダの `args` が旧既定 `["exec", "{prompt}"]` のままなら
///   `--skip-git-repo-check` を挿入する(codex exec は信頼済みディレクトリの外では
///   このフラグなしに実行を拒否するため)。
#[must_use]
pub fn upgrade_stale_defaults(settings: &mut CoreSettings) -> bool {
    let mut changed = false;
    for provider in &mut settings.ai_providers {
        if provider.id == "codex"
            && provider.command == "codex"
            && provider.args == ["exec", "{prompt}"]
        {
            provider.args = vec![
                "exec".to_owned(),
                "--skip-git-repo-check".to_owned(),
                "{prompt}".to_owned(),
            ];
            changed = true;
        }
    }
    changed
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
    /// URL の関連リソースをクリックしたときの既定の開き方。
    pub url_open_mode: UrlOpenMode,
    /// 内蔵 MCP サーバーを起動するか。
    pub mcp_enabled: bool,
    /// MCP サーバーの待受ポート(バインドは 127.0.0.1 のみ)。
    pub mcp_port: u16,
    /// **旧バージョンが平文で持っていた** MCP サーバーの Bearer トークン。
    ///
    /// トークンの実体は OS の資格情報ストアへ移した(シェル側の
    /// `crate::secrets` / `crate::state::AppState`)。このフィールドは
    ///
    /// - 既存の設定 JSON を読めるようにするため deserialize だけ受け付け、
    /// - `skip_serializing` により**二度と書き戻さない**
    ///
    /// という移行専用の受け皿。起動時に値があればシェルが資格情報ストアへ移送し、
    /// 設定 JSON からは消える。新しいコードがここを読むのは移送処理だけ。
    #[serde(rename = "mcpToken", skip_serializing)]
    pub legacy_mcp_token: Option<String>,
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
            url_open_mode: UrlOpenMode::External,
            mcp_enabled: true,
            mcp_port: DEFAULT_MCP_PORT,
            legacy_mcp_token: None,
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

    /// 保存前に値を検証する。
    ///
    /// UI・Tauri に依存しない規則だけを見る。ショートカット文字列が解釈できるか
    /// (`Ctrl+Space` など)は OS/フレームワーク依存なのでシェル側で追加検証する。
    ///
    /// # Errors
    /// 値が不正な場合。メッセージはそのまま UI に出せる日本語。
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.backup_generations < 1 {
            return Err(SettingsError::BackupGenerations);
        }
        if self.mcp_port < MIN_MCP_PORT {
            return Err(SettingsError::McpPort);
        }
        if !AI_TIMEOUT_RANGE.contains(&self.ai_timeout_secs) {
            return Err(SettingsError::AiTimeout);
        }

        let mut seen = HashSet::new();
        for provider in &self.ai_providers {
            let id = provider.id.trim();
            if id.is_empty() {
                return Err(SettingsError::EmptyProviderId);
            }
            if !seen.insert(id) {
                return Err(SettingsError::DuplicateProviderId { id: id.to_owned() });
            }
            if provider.label.trim().is_empty() {
                return Err(SettingsError::EmptyProviderLabel { id: id.to_owned() });
            }
            if provider.command.trim().is_empty() {
                return Err(SettingsError::EmptyProviderCommand { id: id.to_owned() });
            }
        }

        if self.ai_provider(None).is_none() {
            return Err(SettingsError::DefaultProviderUnavailable {
                id: self.ai_default_provider_id.clone(),
            });
        }

        Ok(())
    }
}

/// [`CoreSettings::validate`] が見つけた不正な設定値。
///
/// メッセージはそのまま設定画面に出せる日本語にしてある。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    /// バックアップ世代数が 1 未満。
    #[error("バックアップ世代数は 1 以上にしてください。")]
    BackupGenerations,

    /// MCP ポートが範囲外。
    #[error("MCP ポートは {MIN_MCP_PORT}〜65535 の範囲で指定してください。")]
    McpPort,

    /// AI のタイムアウトが範囲外。
    #[error("AI のタイムアウトは {}〜{} 秒の範囲で指定してください。", AI_TIMEOUT_RANGE.start(), AI_TIMEOUT_RANGE.end())]
    AiTimeout,

    /// プロバイダの id が空。
    #[error("AI プロバイダの id を入力してください。")]
    EmptyProviderId,

    /// プロバイダの id が重複している。
    #[error("AI プロバイダの id \"{id}\" が重複しています。")]
    DuplicateProviderId {
        /// 重複した id。
        id: String,
    },

    /// プロバイダの表示名が空。
    #[error("AI プロバイダ \"{id}\" の表示名を入力してください。")]
    EmptyProviderLabel {
        /// 対象プロバイダの id。
        id: String,
    },

    /// プロバイダの command が空。
    #[error("AI プロバイダ \"{id}\" の command を入力してください。")]
    EmptyProviderCommand {
        /// 対象プロバイダの id。
        id: String,
    },

    /// 既定プロバイダが存在しないか無効。
    #[error("既定の AI プロバイダ \"{id}\" が存在しないか無効です。")]
    DefaultProviderUnavailable {
        /// 既定プロバイダとして指定されている id。
        id: String,
    },
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
        assert_eq!(settings.url_open_mode, UrlOpenMode::External);
        assert!(settings.mcp_enabled);
        assert_eq!(settings.mcp_port, DEFAULT_MCP_PORT);
        assert_eq!(settings.legacy_mcp_token, None);
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
        assert_eq!(codex.args, ["exec", "--skip-git-repo-check", "{prompt}"]);
        assert!(!codex.mcp_supports_token);
        assert!(codex.mcp_args.iter().any(|arg| arg.contains("{mcp_url}")));

        // antigravity は仕様不明のため既定で無効。
        assert!(settings.ai_provider(Some("antigravity")).is_none());
        assert_eq!(settings.enabled_ai_providers().count(), 2);
        assert!(settings.ai_provider(Some("unknown")).is_none());
    }

    #[test]
    fn stale_codex_default_gains_skip_git_repo_check() {
        // 旧既定のまま → フラグが挿入される。
        let mut settings = CoreSettings::default();
        settings.ai_providers[1].args = vec!["exec".to_owned(), "{prompt}".to_owned()];
        assert!(upgrade_stale_defaults(&mut settings));
        assert_eq!(
            settings.ai_providers[1].args,
            ["exec", "--skip-git-repo-check", "{prompt}"]
        );
        // 2 回目は何もしない(冪等)。
        assert!(!upgrade_stale_defaults(&mut settings));
    }

    #[test]
    fn user_edited_codex_args_are_left_alone() {
        let mut settings = CoreSettings::default();
        settings.ai_providers[1].args = vec![
            "exec".to_owned(),
            "--json".to_owned(),
            "{prompt}".to_owned(),
        ];
        assert!(!upgrade_stale_defaults(&mut settings));
        assert_eq!(
            settings.ai_providers[1].args,
            ["exec", "--json", "{prompt}"]
        );
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
        // 内蔵ブラウザを足す前に保存された JSON でも、既定は「外部ブラウザ」。
        assert_eq!(parsed.url_open_mode, UrlOpenMode::External);
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
        assert_eq!(json["urlOpenMode"], "external");
        assert_eq!(json["mcpEnabled"], true);
        assert_eq!(json["mcpPort"], 39150);
    }

    /// URL の開き方は camelCase の文字列で往復し、未知の値は保存を拒む。
    #[test]
    fn url_open_mode_round_trips_as_camel_case() {
        for (json, mode) in [
            ("external", UrlOpenMode::External),
            ("internal", UrlOpenMode::Internal),
            ("internalAuto", UrlOpenMode::InternalAuto),
        ] {
            let parsed: CoreSettings =
                serde_json::from_str(&format!(r#"{{"urlOpenMode":"{json}"}}"#)).unwrap();
            assert_eq!(parsed.url_open_mode, mode);
            assert_eq!(
                serde_json::to_value(parsed).unwrap()["urlOpenMode"],
                serde_json::Value::String(json.to_owned())
            );
        }

        // 綴りを間違えた値は既定へ落とさず、読み込みごと失敗させる
        // (設定ファイルを直接編集したときに気づけるように)。
        assert!(serde_json::from_str::<CoreSettings>(r#"{"urlOpenMode":"inline"}"#).is_err());
    }

    #[test]
    fn url_open_mode_answers_what_the_ui_asks() {
        assert!(!UrlOpenMode::External.uses_internal_pane());
        assert!(UrlOpenMode::Internal.uses_internal_pane());
        assert!(UrlOpenMode::InternalAuto.uses_internal_pane());

        assert!(!UrlOpenMode::External.opens_automatically());
        assert!(!UrlOpenMode::Internal.opens_automatically());
        assert!(UrlOpenMode::InternalAuto.opens_automatically());
    }

    /// MCP トークンは **設定 JSON に書き戻さない**(実体は OS の資格情報ストア)。
    ///
    /// 読む方は旧バージョンの JSON のために残してあるので、
    /// 「読める・けれど書かない」を両方固定する。
    #[test]
    fn the_legacy_mcp_token_is_read_but_never_written_back() {
        let parsed: CoreSettings =
            serde_json::from_str(r#"{"mcpToken":"s3cret"}"#).expect("旧 JSON を読める");
        assert_eq!(parsed.legacy_mcp_token.as_deref(), Some("s3cret"));

        // 読み込んだ値をそのまま serialize しても、トークンは出て行かない。
        let json = serde_json::to_value(&parsed).unwrap();
        assert!(
            json.get("mcpToken").is_none(),
            "設定 JSON にトークンを書き戻してはいけない: {json}"
        );
        assert!(!json.to_string().contains("s3cret"), "{json}");

        // 既定値でもキー自体が現れない。
        let json = serde_json::to_value(CoreSettings::default()).unwrap();
        assert!(json.get("mcpToken").is_none(), "{json}");
    }

    #[test]
    fn empty_object_yields_defaults() {
        let parsed: CoreSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, CoreSettings::default());
    }

    #[test]
    fn board_settings_are_derived_from_core_settings() {
        assert_eq!(BoardSettings::default().week_start, WeekStart::Monday);
        let settings = CoreSettings {
            week_start: WeekStart::Sunday,
            ..CoreSettings::default()
        };
        assert_eq!(
            BoardSettings::from(&settings),
            BoardSettings {
                week_start: WeekStart::Sunday
            }
        );
    }

    // ---- 検証 ----

    #[test]
    fn defaults_are_valid() {
        assert_eq!(CoreSettings::default().validate(), Ok(()));
    }

    #[test]
    fn numeric_ranges_are_checked() {
        assert_eq!(
            CoreSettings {
                backup_generations: 0,
                ..CoreSettings::default()
            }
            .validate(),
            Err(SettingsError::BackupGenerations)
        );
        assert_eq!(
            CoreSettings {
                mcp_port: 80,
                ..CoreSettings::default()
            }
            .validate(),
            Err(SettingsError::McpPort)
        );
        assert!(CoreSettings {
            mcp_port: MIN_MCP_PORT,
            ..CoreSettings::default()
        }
        .validate()
        .is_ok());
        assert_eq!(
            CoreSettings {
                ai_timeout_secs: 9,
                ..CoreSettings::default()
            }
            .validate(),
            Err(SettingsError::AiTimeout)
        );
        assert_eq!(
            CoreSettings {
                ai_timeout_secs: 3601,
                ..CoreSettings::default()
            }
            .validate(),
            Err(SettingsError::AiTimeout)
        );
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

        assert!(matches!(
            CoreSettings {
                ai_providers: vec![provider("a"), provider("a")],
                ai_default_provider_id: "a".to_owned(),
                ..CoreSettings::default()
            }
            .validate(),
            Err(SettingsError::DuplicateProviderId { .. })
        ));

        assert!(matches!(
            CoreSettings {
                ai_providers: vec![AiProvider {
                    id: "  ".to_owned(),
                    ..provider("a")
                }],
                ai_default_provider_id: "a".to_owned(),
                ..CoreSettings::default()
            }
            .validate(),
            Err(SettingsError::EmptyProviderId)
        ));

        assert!(matches!(
            CoreSettings {
                ai_providers: vec![AiProvider {
                    label: String::new(),
                    ..provider("a")
                }],
                ai_default_provider_id: "a".to_owned(),
                ..CoreSettings::default()
            }
            .validate(),
            Err(SettingsError::EmptyProviderLabel { .. })
        ));

        assert!(matches!(
            CoreSettings {
                ai_providers: vec![AiProvider {
                    command: String::new(),
                    ..provider("a")
                }],
                ai_default_provider_id: "a".to_owned(),
                ..CoreSettings::default()
            }
            .validate(),
            Err(SettingsError::EmptyProviderCommand { .. })
        ));

        // 既定プロバイダが無効なら保存させない。
        assert!(matches!(
            CoreSettings {
                ai_providers: vec![AiProvider {
                    enabled: false,
                    ..provider("a")
                }],
                ai_default_provider_id: "a".to_owned(),
                ..CoreSettings::default()
            }
            .validate(),
            Err(SettingsError::DefaultProviderUnavailable { .. })
        ));

        assert_eq!(
            CoreSettings {
                ai_providers: vec![provider("a"), provider("b")],
                ai_default_provider_id: "b".to_owned(),
                ..CoreSettings::default()
            }
            .validate(),
            Ok(())
        );
    }

    #[test]
    fn error_messages_name_the_offending_value() {
        let error = CoreSettings {
            ai_default_provider_id: "missing".to_owned(),
            ..CoreSettings::default()
        }
        .validate()
        .unwrap_err();
        assert!(error.to_string().contains("missing"), "{error}");
        assert!(SettingsError::McpPort.to_string().contains("1024"));
        assert!(SettingsError::AiTimeout.to_string().contains("3600"));
    }
}
