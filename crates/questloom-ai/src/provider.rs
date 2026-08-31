//! プロバイダ定義([`AiProvider`])から実行リクエストを組み立てる。
//!
//! 引数テンプレートのプレースホルダは以下。プロンプトはシェルを介さず
//! 引数配列のまま渡すため、引用符やメタ文字のエスケープは不要。
//!
//! | プレースホルダ | 置換内容 | 使う場所 |
//! |---|---|---|
//! | `{prompt}` | プロンプト本文 | `args` |
//! | `{mcp_url}` | 内蔵 MCP サーバーの URL | `mcp_args` |
//! | `{mcp_config}` | Claude Code の `--mcp-config` に渡す JSON | `mcp_args` |

use std::time::Duration;

use questloom_core::settings::AiProvider;

use crate::exec::{AiRequest, PromptDelivery};

/// 内蔵 MCP サーバーの接続情報。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpEndpoint {
    /// `http://127.0.0.1:<port>/mcp`。
    pub url: String,
    /// Bearer トークン。`None` なら認証なし。
    pub token: Option<String>,
}

impl McpEndpoint {
    /// Claude Code の `--mcp-config` に渡す JSON を組み立てる。
    #[must_use]
    pub fn claude_config_json(&self) -> String {
        let mut server = serde_json::json!({ "type": "http", "url": self.url });
        if let Some(token) = &self.token {
            server["headers"] = serde_json::json!({ "Authorization": format!("Bearer {token}") });
        }
        serde_json::json!({ "mcpServers": { "questloom": server } }).to_string()
    }
}

/// 組み立て済みの実行内容。
#[derive(Debug, Clone)]
pub struct PreparedRun {
    /// プロセス起動の指定。
    pub request: AiRequest,
    /// MCP 接続用の引数を付けられたか。
    pub mcp_attached: bool,
}

/// プロバイダ定義とプロンプトから実行リクエストを組み立てる。
///
/// `mcp` を渡すと、プロバイダが MCP に対応している限り接続用の引数を
/// `args` の**前**に挿入する(`codex -c ... exec <prompt>` のように、
/// サブコマンドより前に置く必要があるプロバイダがあるため)。
///
/// プロンプトの渡し方は [`PromptDelivery::detect`] で決める(`.cmd` シム経由の
/// CLI では、複数行のプロンプトを引数で渡せないため標準入力にする)。
#[must_use]
pub fn prepare(
    provider: &AiProvider,
    prompt: &str,
    mcp: Option<&McpEndpoint>,
    timeout: Duration,
) -> PreparedRun {
    prepare_with(
        provider,
        prompt,
        mcp,
        timeout,
        PromptDelivery::detect(&provider.command),
    )
}

/// プロンプトの渡し方を明示して組み立てる([`prepare`] の下位版)。
#[must_use]
pub fn prepare_with(
    provider: &AiProvider,
    prompt: &str,
    mcp: Option<&McpEndpoint>,
    timeout: Duration,
    delivery: PromptDelivery,
) -> PreparedRun {
    let mcp_args = mcp.and_then(|endpoint| mcp_args(provider, endpoint));
    let mcp_attached = mcp_args.is_some();

    let mut args = mcp_args.unwrap_or_default();
    let mut stdin = None;
    match delivery {
        PromptDelivery::Argument => args.extend(
            provider
                .args
                .iter()
                .map(|arg| arg.replace("{prompt}", prompt)),
        ),
        PromptDelivery::Stdin => {
            // `{prompt}` を含む引数は落とし、本文は標準入力から渡す。
            args.extend(
                provider
                    .args
                    .iter()
                    .filter(|arg| !arg.contains("{prompt}"))
                    .cloned(),
            );
            stdin = Some(prompt.to_owned());
        }
    }

    PreparedRun {
        request: AiRequest {
            command: provider.command.clone(),
            args,
            stdin,
            timeout,
        },
        mcp_attached,
    }
}

/// MCP 接続用の引数を組み立てる。非対応なら `None`。
///
/// トークンを設定しているのにヘッダを渡せないプロバイダ(codex 等)では、
/// 接続しても 401 になるだけなので MCP 接続自体を諦める。
#[must_use]
pub fn mcp_args(provider: &AiProvider, endpoint: &McpEndpoint) -> Option<Vec<String>> {
    if provider.mcp_args.is_empty() {
        return None;
    }
    if endpoint.token.is_some() && !provider.mcp_supports_token {
        tracing::warn!(
            provider = provider.id,
            "MCP トークンを渡せないプロバイダのため MCP 接続を省略します"
        );
        return None;
    }
    let config = endpoint.claude_config_json();
    Some(
        provider
            .mcp_args
            .iter()
            .map(|arg| {
                arg.replace("{mcp_url}", &endpoint.url)
                    .replace("{mcp_config}", &config)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::PromptDelivery;
    use questloom_core::settings::default_ai_providers;

    fn provider(id: &str) -> AiProvider {
        default_ai_providers()
            .into_iter()
            .find(|provider| provider.id == id)
            .expect("既定プロバイダ")
    }

    fn endpoint(token: Option<&str>) -> McpEndpoint {
        McpEndpoint {
            url: "http://127.0.0.1:39150/mcp".to_owned(),
            token: token.map(ToOwned::to_owned),
        }
    }

    /// 環境(CLI の有無)に左右されないよう、渡し方を固定して組み立てる。
    fn prepare(
        provider: &AiProvider,
        prompt: &str,
        mcp: Option<&McpEndpoint>,
        timeout: Duration,
    ) -> PreparedRun {
        prepare_with(provider, prompt, mcp, timeout, PromptDelivery::Argument)
    }

    #[test]
    fn substitutes_the_prompt_into_the_args() {
        let run = prepare(
            &provider("claude"),
            "タスクを 3 つ",
            None,
            Duration::from_secs(300),
        );
        assert_eq!(run.request.command, "claude");
        assert_eq!(run.request.args, ["-p", "タスクを 3 つ"]);
        assert_eq!(run.request.stdin, None);
        assert!(!run.mcp_attached);
    }

    #[test]
    fn stdin_delivery_drops_the_prompt_argument() {
        let run = prepare_with(
            &provider("claude"),
            "複数行の\nプロンプト",
            Some(&endpoint(None)),
            Duration::from_secs(300),
            PromptDelivery::Stdin,
        );
        // `-p` は残り、`{prompt}` の引数だけが落ちる。
        assert_eq!(run.request.args.last().unwrap(), "-p");
        assert!(!run
            .request
            .args
            .iter()
            .any(|arg| arg.contains("プロンプト")));
        assert_eq!(run.request.stdin.as_deref(), Some("複数行の\nプロンプト"));
        assert!(run.mcp_attached, "MCP 引数は改行を含まないので残る");
    }

    #[test]
    fn claude_gets_an_mcp_config_before_the_prompt() {
        let run = prepare(
            &provider("claude"),
            "整理して",
            Some(&endpoint(None)),
            Duration::from_secs(300),
        );
        assert!(run.mcp_attached);
        assert_eq!(run.request.args[0], "--mcp-config");
        let config: serde_json::Value = serde_json::from_str(&run.request.args[1]).unwrap();
        assert_eq!(
            config["mcpServers"]["questloom"]["url"],
            "http://127.0.0.1:39150/mcp"
        );
        assert_eq!(config["mcpServers"]["questloom"]["type"], "http");
        assert!(config["mcpServers"]["questloom"]["headers"].is_null());
        assert_eq!(run.request.args[2], "--allowedTools");
        // プロンプトは MCP 引数の後ろに来る。
        assert_eq!(run.request.args[4], "-p");
        assert_eq!(run.request.args[5], "整理して");
    }

    #[test]
    fn claude_passes_the_token_as_an_authorization_header() {
        let run = prepare(
            &provider("claude"),
            "整理して",
            Some(&endpoint(Some("s3cret"))),
            Duration::from_secs(300),
        );
        assert!(run.mcp_attached);
        let config: serde_json::Value = serde_json::from_str(&run.request.args[1]).unwrap();
        assert_eq!(
            config["mcpServers"]["questloom"]["headers"]["Authorization"],
            "Bearer s3cret"
        );
    }

    #[test]
    fn codex_gets_config_overrides_before_the_subcommand() {
        let run = prepare(
            &provider("codex"),
            "整理して",
            Some(&endpoint(None)),
            Duration::from_secs(300),
        );
        assert!(run.mcp_attached);
        assert!(run
            .request
            .args
            .iter()
            .any(|arg| arg == r#"mcp_servers.questloom.url="http://127.0.0.1:39150/mcp""#));
        let exec_at = run
            .request
            .args
            .iter()
            .position(|arg| arg == "exec")
            .expect("サブコマンドがある");
        assert!(exec_at > 0, "設定上書きは exec より前に置く");
        assert_eq!(run.request.args.last().unwrap(), "整理して");
    }

    #[test]
    fn providers_that_cannot_send_headers_skip_mcp_when_a_token_is_set() {
        let run = prepare(
            &provider("codex"),
            "整理して",
            Some(&endpoint(Some("s3cret"))),
            Duration::from_secs(300),
        );
        assert!(!run.mcp_attached);
        assert_eq!(run.request.args, ["exec", "整理して"]);
    }

    #[test]
    fn providers_without_mcp_args_are_not_wired_to_mcp() {
        let mut antigravity = provider("antigravity");
        antigravity.enabled = true;
        let run = prepare(
            &antigravity,
            "整理して",
            Some(&endpoint(None)),
            Duration::from_secs(300),
        );
        assert!(!run.mcp_attached);
        assert_eq!(run.request.args, ["-p", "整理して"]);
    }
}
