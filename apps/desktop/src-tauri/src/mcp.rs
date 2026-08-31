//! 内蔵 MCP サーバーの起動・停止・再起動。
//!
//! 起動時とコア設定の変更時に [`McpSupervisor::apply`] が呼ばれ、
//! `mcpEnabled` / `mcpPort` / `mcpToken` の変化に追随してサーバーを張り直す。
//! ポート使用中などで起動に失敗しても、ログを出すだけでアプリは動き続ける。
//!
//! 待受ポートは `QUESTLOOM_MCP_PORT` が設定されていればそちらが優先される
//! ([`crate::env_override`])。テストが本物の 39150 を奪わないための逃げ道。

use std::sync::Arc;

use questloom_ai::McpEndpoint;
use questloom_core::service::TaskService;
use questloom_core::settings::CoreSettings;
use questloom_mcp::{McpServerConfig, McpServerHandle};
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::Mutex;

/// 起動中のサーバーと、それを起動したときの構成。
struct Running {
    config: McpServerConfig,
    handle: McpServerHandle,
}

/// MCP サーバーのライフサイクルを管理する。
///
/// Tauri の managed state として保持し、設定変更のたびに [`apply`](Self::apply) を呼ぶ。
pub struct McpSupervisor {
    service: Arc<TaskService>,
    /// 起動中のサーバー。停止中は `None`。
    running: Mutex<Option<Running>>,
}

impl McpSupervisor {
    /// サービスを束ねたスーパーバイザを作る。この時点ではまだ起動しない。
    #[must_use]
    pub const fn new(service: Arc<TaskService>) -> Self {
        Self {
            service,
            running: Mutex::const_new(None),
        }
    }

    /// 設定に合わせてサーバーを起動・停止・再起動する。
    ///
    /// 構成が変わっていなければ何もしない(冪等)。
    pub async fn apply(&self, settings: &CoreSettings) {
        let desired = settings.mcp_enabled.then(|| McpServerConfig {
            // 環境変数が指定されていればコア設定より優先する(crate::env_override 参照)。
            port: crate::env_override::mcp_port(settings.mcp_port),
            token: settings.mcp_token.clone(),
        });

        let mut running = self.running.lock().await;
        if desired.as_ref() == running.as_ref().map(|current| &current.config) {
            return;
        }

        if let Some(current) = running.take() {
            let url = current.handle.url();
            current.handle.shutdown().await;
            tracing::info!(url, "MCP サーバーを停止しました");
        }

        let Some(config) = desired else {
            return;
        };
        match questloom_mcp::serve(Arc::clone(&self.service), config.clone()).await {
            Ok(handle) => *running = Some(Running { config, handle }),
            // 起動に失敗してもアプリは使えるべきなので、ログだけ出して続行する。
            Err(error) => tracing::error!(%error, "MCP サーバーの起動に失敗しました"),
        }
    }

    /// 起動中なら、AI CLI に渡すための接続情報を返す。
    pub async fn endpoint(&self) -> Option<McpEndpoint> {
        let running = self.running.lock().await;
        running.as_ref().map(|current| McpEndpoint {
            url: current.handle.url(),
            token: current
                .config
                .token
                .as_deref()
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(ToOwned::to_owned),
        })
    }

    /// サーバーを停止する。
    ///
    /// アプリ終了時に `lib.rs` の [`RunEvent::Exit`](tauri::RunEvent::Exit) から
    /// タイムアウト付きで呼ばれる。停止済みでも安全に呼べる(冪等)。
    pub async fn stop(&self) {
        let mut running = self.running.lock().await;
        if let Some(current) = running.take() {
            current.handle.shutdown().await;
            tracing::info!("MCP サーバーを停止しました");
        }
    }
}

/// 設定を MCP サーバーへ反映する。実際の起動・停止は非同期タスクで行う。
pub fn apply<R: Runtime>(app: &AppHandle<R>, settings: &CoreSettings) {
    let Some(supervisor) = app.try_state::<Arc<McpSupervisor>>() else {
        tracing::warn!("MCP スーパーバイザが未登録のため設定を反映できません");
        return;
    };
    let supervisor = Arc::clone(supervisor.inner());
    let settings = settings.clone();
    tauri::async_runtime::spawn(async move {
        supervisor.apply(&settings).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::test_support;
    use questloom_core::settings::BoardSettings;

    fn supervisor() -> McpSupervisor {
        McpSupervisor::new(test_support::service(BoardSettings::default()))
    }

    /// ポート 0 を使い、実ポートを占有せずに起動・再起動・停止の流れを確認する。
    #[tokio::test]
    async fn applies_start_restart_and_stop() {
        let supervisor = supervisor();
        let settings = CoreSettings {
            mcp_enabled: true,
            mcp_port: 0,
            mcp_token: None,
            ..CoreSettings::default()
        };

        supervisor.apply(&settings).await;
        let first = supervisor
            .running
            .lock()
            .await
            .as_ref()
            .map(|current| current.handle.addr());
        assert!(first.is_some(), "起動している");

        // 同じ設定なら張り直さない。
        supervisor.apply(&settings).await;
        assert_eq!(
            supervisor
                .running
                .lock()
                .await
                .as_ref()
                .map(|current| current.handle.addr()),
            first
        );

        // トークンが変わったら再起動する。
        supervisor
            .apply(&CoreSettings {
                mcp_token: Some("s3cret".to_owned()),
                ..settings.clone()
            })
            .await;
        assert_ne!(
            supervisor
                .running
                .lock()
                .await
                .as_ref()
                .map(|current| current.handle.addr()),
            first,
            "別のエフェメラルポートで張り直される"
        );

        // 無効化したら停止する。
        supervisor
            .apply(&CoreSettings {
                mcp_enabled: false,
                ..settings
            })
            .await;
        assert!(supervisor.running.lock().await.is_none());

        // 停止済みでも stop は安全に呼べる。
        supervisor.stop().await;
    }

    #[tokio::test]
    async fn endpoint_reports_the_running_url_and_token() {
        let supervisor = supervisor();
        assert_eq!(supervisor.endpoint().await, None);

        supervisor
            .apply(&CoreSettings {
                mcp_enabled: true,
                mcp_port: 0,
                // 空白のみのトークンは「未設定」と同じ扱い(サーバー側と揃える)。
                mcp_token: Some("  ".to_owned()),
                ..CoreSettings::default()
            })
            .await;
        let endpoint = supervisor.endpoint().await.expect("起動している");
        assert!(endpoint.url.starts_with("http://127.0.0.1:"));
        assert!(endpoint.url.ends_with("/mcp"));
        assert_eq!(endpoint.token, None);

        supervisor
            .apply(&CoreSettings {
                mcp_enabled: true,
                mcp_port: 0,
                mcp_token: Some("s3cret".to_owned()),
                ..CoreSettings::default()
            })
            .await;
        assert_eq!(
            supervisor.endpoint().await.and_then(|end| end.token),
            Some("s3cret".to_owned())
        );
        supervisor.stop().await;
    }

    #[tokio::test]
    async fn a_failed_start_leaves_the_server_stopped() {
        let blocker = questloom_mcp::serve(
            supervisor().service,
            McpServerConfig {
                port: 0,
                token: None,
            },
        )
        .await
        .expect("ブロッカーが起動する");

        let supervisor = supervisor();
        supervisor
            .apply(&CoreSettings {
                mcp_enabled: true,
                mcp_port: blocker.addr().port(),
                ..CoreSettings::default()
            })
            .await;
        assert!(supervisor.running.lock().await.is_none());

        blocker.shutdown().await;
    }
}
