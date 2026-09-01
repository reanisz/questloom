//! 環境変数によるデータディレクトリ・MCP ポート・資格情報 service 名の上書き。
//!
//! テスト(バックエンド e2e)が実アプリを起動するとき、本物の
//! `%APPDATA%\dev.reanisz.questloom` を汚さず、本物の 39150 番ポートとも衝突せず、
//! 本物の資格情報エントリも触らないための逃げ道。
//! いずれの環境変数も未設定(または空白のみ)なら従来どおりの解決に落ちる。
//!
//! | 環境変数 | 効果 |
//! |---|---|
//! | [`DATA_DIR_ENV`] | `app_data_dir()` の代わりにこのパスを使う。WebView2 のプロファイルも `<dir>\webview` へ移す |
//! | [`MCP_PORT_ENV`] | コア設定の `mcpPort` を無視してこのポートで待ち受ける |
//! | [`KEYRING_SERVICE_ENV`] | シークレットの service 名(既定 `questloom`)を差し替える |
//!
//! 解決規則そのものは環境変数を読まない純関数([`resolve_data_dir`] /
//! [`resolve_webview_data_dir`] / [`resolve_mcp_port`] / [`resolve_keyring_service`])に
//! 切り出してあり、テストはそちらを固定する(`std::env::set_var` はプロセス全体を触るので、
//! 並行するテストから使わない)。

use std::path::PathBuf;

use crate::secrets::DEFAULT_SERVICE;

/// データディレクトリを上書きする環境変数名。
pub const DATA_DIR_ENV: &str = "QUESTLOOM_DATA_DIR";

/// MCP サーバーの待受ポートを上書きする環境変数名(コア設定より優先)。
pub const MCP_PORT_ENV: &str = "QUESTLOOM_MCP_PORT";

/// 資格情報ストアの service 名を上書きする環境変数名。
///
/// 資格情報マネージャーのエントリはデータディレクトリと違ってユーザー単位で
/// グローバルなので、`QUESTLOOM_DATA_DIR` だけでは本物のエントリと分離できない。
/// テスト・検証ではこれも一緒に渡すこと。
pub const KEYRING_SERVICE_ENV: &str = "QUESTLOOM_KEYRING_SERVICE";

/// 環境変数で指定されたデータディレクトリ。未設定なら `None`(= 既定の解決を使う)。
#[must_use]
pub fn data_dir() -> Option<PathBuf> {
    resolve_data_dir(std::env::var(DATA_DIR_ENV).ok().as_deref())
}

/// [`data_dir`] の解決規則。空白のみは「未設定」として扱う。
#[must_use]
pub fn resolve_data_dir(raw: Option<&str>) -> Option<PathBuf> {
    let value = raw?.trim();
    (!value.is_empty()).then(|| PathBuf::from(value))
}

/// WebView2 のユーザーデータフォルダに使う、データディレクトリ配下のサブディレクトリ名。
pub const WEBVIEW_SUBDIR: &str = "webview";

/// webview のユーザーデータフォルダ。未設定なら `None`(= Tauri の既定に任せる)。
///
/// [`DATA_DIR_ENV`] は DB・バックアップ・`plugins/` しか動かさない。WebView2 の
/// プロファイル(localStorage・Cookie・キャッシュ)は Tauri が既定で
/// `%LOCALAPPDATA%\<identifier>` に置くので、そのままだとテスト起動が
/// **利用者の実 localStorage を読み書きしてしまう**(逆方向の汚染も起きる)。
/// データディレクトリを上書きしているときは、プロファイルもその下へ引き込む。
///
/// 返した値は `WebviewWindowBuilder::data_directory` /
/// `WebviewBuilder::data_directory` に渡す。**全 webview(main / overlay /
/// plugin-host / browser-pane)に同じ値を渡すこと。** 渡し忘れた webview だけが
/// 既定の実プロファイルに残る。
#[must_use]
pub fn webview_data_dir() -> Option<PathBuf> {
    resolve_webview_data_dir(std::env::var(DATA_DIR_ENV).ok().as_deref())
}

/// [`webview_data_dir`] の解決規則。[`resolve_data_dir`] と同じ入力から導出する。
#[must_use]
pub fn resolve_webview_data_dir(raw: Option<&str>) -> Option<PathBuf> {
    Some(resolve_data_dir(raw)?.join(WEBVIEW_SUBDIR))
}

/// コア設定の待受ポートに、環境変数による上書きを適用する。
#[must_use]
pub fn mcp_port(configured: u16) -> u16 {
    resolve_mcp_port(std::env::var(MCP_PORT_ENV).ok().as_deref(), configured)
}

/// [`mcp_port`] の解決規則。
///
/// 未設定・空白のみ・`u16` として読めない値は無視して `configured` に落ちる
/// (壊れた環境変数でアプリが起動しなくなるより、設定どおりに動く方がまし)。
#[must_use]
pub fn resolve_mcp_port(raw: Option<&str>, configured: u16) -> u16 {
    let Some(value) = raw else {
        return configured;
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return configured;
    }
    match trimmed.parse::<u16>() {
        Ok(port) => port,
        Err(error) => {
            tracing::warn!(
                %error,
                value = trimmed,
                "{MCP_PORT_ENV} を解釈できないため設定値を使います"
            );
            configured
        }
    }
}

/// 資格情報ストアの service 名。未設定なら [`DEFAULT_SERVICE`]。
#[must_use]
pub fn keyring_service() -> String {
    resolve_keyring_service(std::env::var(KEYRING_SERVICE_ENV).ok().as_deref())
}

/// [`keyring_service`] の解決規則。空白のみは「未設定」として扱う。
#[must_use]
pub fn resolve_keyring_service(raw: Option<&str>) -> String {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_SERVICE)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_data_dir_falls_back_to_the_default() {
        assert_eq!(resolve_data_dir(None), None);
        assert_eq!(resolve_data_dir(Some("")), None);
        assert_eq!(resolve_data_dir(Some("   \t")), None);
    }

    #[test]
    fn a_data_dir_override_is_taken_as_a_path() {
        assert_eq!(
            resolve_data_dir(Some(r"C:\temp\questloom-test")),
            Some(PathBuf::from(r"C:\temp\questloom-test"))
        );
        // 前後の空白は落とすが、パスの途中の空白は残す。
        assert_eq!(
            resolve_data_dir(Some("  C:\\temp\\quest loom  ")),
            Some(PathBuf::from("C:\\temp\\quest loom"))
        );
    }

    /// 未設定なら webview のプロファイルも既定のまま
    /// (= `%LOCALAPPDATA%\dev.reanisz.questloom`。既存ユーザーの localStorage を動かさない)。
    #[test]
    fn an_unset_data_dir_leaves_the_webview_profile_alone() {
        assert_eq!(resolve_webview_data_dir(None), None);
        assert_eq!(resolve_webview_data_dir(Some("")), None);
        assert_eq!(resolve_webview_data_dir(Some("   \t")), None);
    }

    /// 上書き時はデータディレクトリの直下([`WEBVIEW_SUBDIR`])に入る。
    #[test]
    fn a_data_dir_override_moves_the_webview_profile_under_it() {
        assert_eq!(
            resolve_webview_data_dir(Some(r"C:\temp\questloom-test")),
            Some(PathBuf::from(r"C:\temp\questloom-test\webview"))
        );
        // データディレクトリ本体と同じ入力から導く(食い違うと分離が片肺になる)。
        let raw = Some("  C:\\temp\\quest loom  ");
        assert_eq!(
            resolve_webview_data_dir(raw),
            resolve_data_dir(raw).map(|dir| dir.join(WEBVIEW_SUBDIR))
        );
    }

    #[test]
    fn an_unset_port_keeps_the_configured_one() {
        assert_eq!(resolve_mcp_port(None, 39150), 39150);
        assert_eq!(resolve_mcp_port(Some(""), 39150), 39150);
        assert_eq!(resolve_mcp_port(Some("  "), 39150), 39150);
    }

    #[test]
    fn a_port_override_wins_over_the_configured_one() {
        assert_eq!(resolve_mcp_port(Some("45123"), 39150), 45123);
        assert_eq!(resolve_mcp_port(Some(" 45123\n"), 39150), 45123);
        // 0 は「OS に任せる」の意味を持つのでそのまま通す。
        assert_eq!(resolve_mcp_port(Some("0"), 39150), 0);
    }

    #[test]
    fn the_keyring_service_falls_back_to_the_default() {
        assert_eq!(resolve_keyring_service(None), DEFAULT_SERVICE);
        assert_eq!(resolve_keyring_service(Some("")), DEFAULT_SERVICE);
        assert_eq!(resolve_keyring_service(Some("  \t")), DEFAULT_SERVICE);
    }

    #[test]
    fn a_keyring_service_override_replaces_the_default() {
        assert_eq!(
            resolve_keyring_service(Some("questloom-e2e-123")),
            "questloom-e2e-123"
        );
        assert_eq!(
            resolve_keyring_service(Some("  questloom-test  ")),
            "questloom-test"
        );
    }

    #[test]
    fn a_broken_port_override_is_ignored() {
        for broken in ["abc", "-1", "70000", "45123.0", "45123 45124"] {
            assert_eq!(
                resolve_mcp_port(Some(broken), 39150),
                39150,
                "{broken} は無視して設定値に落ちる"
            );
        }
    }
}
