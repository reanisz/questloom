//! questloom の内蔵 MCP サーバー。127.0.0.1 でタスク操作ツールを公開する。
//!
//! 公式 Rust SDK([rmcp](https://docs.rs/rmcp))の Streamable HTTP transport を
//! axum に載せ、`http://127.0.0.1:<port>/mcp` を待ち受ける。
//! Claude Code / Codex などのエージェントはこの URL を登録して
//! [`tools`] のツール群からタスクを操作する。
//!
//! ```no_run
//! # use std::sync::Arc;
//! # async fn example(
//! #     service: Arc<questloom_core::service::TaskService>,
//! # ) -> Result<(), questloom_mcp::McpServerError> {
//! let handle = questloom_mcp::serve(service, questloom_mcp::McpServerConfig::default()).await?;
//! println!("{}", handle.url()); // http://127.0.0.1:39150/mcp
//! handle.shutdown().await;
//! # Ok(())
//! # }
//! ```

pub mod server;
pub mod tools;

pub use server::{serve, McpServerConfig, McpServerError, McpServerHandle, MCP_PATH};
pub use tools::QuestloomTools;
