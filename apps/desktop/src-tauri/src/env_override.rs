//! 環境変数によるデータディレクトリ・MCP ポートの上書き。
//!
//! テスト(バックエンド e2e)が実アプリを起動するとき、本物の
//! `%APPDATA%\dev.reanisz.questloom` を汚さず、本物の 39150 番ポートとも衝突しないための逃げ道。
//! どちらの環境変数も未設定(または空白のみ)なら従来どおりの解決に落ちる。
//!
//! | 環境変数 | 効果 |
//! |---|---|
//! | [`DATA_DIR_ENV`] | `app_data_dir()` の代わりにこのパスを使う |
//! | [`MCP_PORT_ENV`] | コア設定の `mcpPort` を無視してこのポートで待ち受ける |
//!
//! 解決規則そのものは環境変数を読まない純関数([`resolve_data_dir`] /
//! [`resolve_mcp_port`])に切り出してあり、テストはそちらを固定する
//! (`std::env::set_var` はプロセス全体を触るので、並行するテストから使わない)。

use std::path::PathBuf;

/// データディレクトリを上書きする環境変数名。
pub const DATA_DIR_ENV: &str = "QUESTLOOM_DATA_DIR";

/// MCP サーバーの待受ポートを上書きする環境変数名(コア設定より優先)。
pub const MCP_PORT_ENV: &str = "QUESTLOOM_MCP_PORT";

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
