//! axum に載せた Streamable HTTP の MCP サーバー。
//!
//! バインド先は必ず `127.0.0.1`。トークンを設定した場合は
//! `Authorization: Bearer <token>` を検証し、一致しないリクエストは 401 で弾く。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use questloom_core::service::TaskService;
use questloom_core::settings::DEFAULT_MCP_PORT;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::tools::QuestloomTools;

/// MCP エンドポイントのパス。`http://127.0.0.1:<port>/mcp` として公開される。
pub const MCP_PATH: &str = "/mcp";

/// MCP サーバーの起動に失敗したことを表す。
#[derive(Debug, thiserror::Error)]
pub enum McpServerError {
    /// ポートを確保できなかった(使用中など)。
    #[error("MCP サーバーのポートを確保できません ({addr}): {source}")]
    Bind {
        /// バインドしようとしたアドレス。
        addr: SocketAddr,
        /// 元エラー。
        #[source]
        source: std::io::Error,
    },
}

/// MCP サーバーの構成。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    /// 待受ポート。`0` を渡すと OS がエフェメラルポートを割り当てる(テスト用)。
    pub port: u16,
    /// Bearer トークン。`None`(または空文字)なら認証なし。
    pub token: Option<String>,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_MCP_PORT,
            token: None,
        }
    }
}

impl McpServerConfig {
    /// 実際に使うトークン。空白のみは「未設定」として扱う。
    fn effective_token(&self) -> Option<String> {
        self.token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(ToOwned::to_owned)
    }
}

/// 起動中の MCP サーバーのハンドル。
///
/// [`shutdown`](Self::shutdown) を呼ぶまで、バックグラウンドの tokio タスクで動き続ける。
#[derive(Debug)]
pub struct McpServerHandle {
    addr: SocketAddr,
    cancel: CancellationToken,
    join: JoinHandle<()>,
}

impl McpServerHandle {
    /// 実際に待ち受けているアドレス。
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// クライアントに登録してもらうエンドポイント URL。
    #[must_use]
    pub fn url(&self) -> String {
        format!("http://{}{MCP_PATH}", self.addr)
    }

    /// サーバーを停止し、バックグラウンドタスクの終了を待つ。
    pub async fn shutdown(self) {
        self.cancel.cancel();
        if let Err(error) = self.join.await {
            if !error.is_cancelled() {
                tracing::warn!(%error, "MCP サーバーの停止待ちに失敗しました");
            }
        }
    }
}

/// MCP サーバーを `127.0.0.1:<port>` で起動する。
///
/// 起動後すぐに戻り、サーバー本体はバックグラウンドの tokio タスクで動く。
/// 停止は [`McpServerHandle::shutdown`] で行う。
///
/// # Errors
/// 指定ポートを確保できない場合(使用中など)。
pub async fn serve(
    service: Arc<TaskService>,
    config: McpServerConfig,
) -> Result<McpServerHandle, McpServerError> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, config.port));
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|source| McpServerError::Bind { addr, source })?;
    let addr = listener
        .local_addr()
        .map_err(|source| McpServerError::Bind { addr, source })?;

    let cancel = CancellationToken::new();
    let http = StreamableHttpService::new(
        move || Ok(QuestloomTools::new(Arc::clone(&service))),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().with_cancellation_token(cancel.child_token()),
    );

    let token = config.effective_token();
    let authenticated = token.is_some();
    let mut router = Router::new().nest_service(MCP_PATH, http);
    if let Some(token) = token {
        router = router.layer(middleware::from_fn_with_state(
            Arc::new(token),
            require_bearer,
        ));
    }

    let shutdown = cancel.clone();
    let join = tokio::spawn(async move {
        let server = axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled().await });
        if let Err(error) = server.await {
            tracing::error!(%error, "MCP サーバーが異常終了しました");
        }
    });

    let handle = McpServerHandle { addr, cancel, join };
    tracing::info!(
        url = handle.url(),
        authenticated,
        "MCP サーバーを起動しました"
    );
    Ok(handle)
}

/// `Authorization: Bearer <token>` を検証する middleware。
async fn require_bearer(
    State(expected): State<Arc<String>>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim);

    if presented == Some(expected.as_str()) {
        return next.run(request).await;
    }
    tracing::warn!("MCP: Bearer トークンが一致しないリクエストを拒否しました");
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        "Unauthorized\n",
    )
        .into_response()
}
