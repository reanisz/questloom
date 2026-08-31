//! 実ポートで MCP サーバーを起動し、HTTP レベルの疎通と Bearer 認証を確認する。

use questloom_mcp::{serve, McpServerConfig};
use serde_json::json;

mod common;

use common::service;

/// MCP の `initialize` リクエスト(JSON-RPC)。
fn initialize_body() -> String {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "questloom-mcp-test", "version": "0.1.0" },
        }
    })
    .to_string()
}

fn post(url: &str, token: Option<&str>) -> reqwest::RequestBuilder {
    let request = reqwest::Client::new()
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(
            reqwest::header::ACCEPT,
            "application/json, text/event-stream",
        )
        .body(initialize_body());
    match token {
        Some(token) => request.header(reqwest::header::AUTHORIZATION, format!("Bearer {token}")),
        None => request,
    }
}

#[tokio::test]
async fn initialize_over_streamable_http_returns_200() {
    // ポート 0 でエフェメラルポートを割り当てる。
    let handle = serve(
        service(),
        McpServerConfig {
            port: 0,
            token: None,
        },
    )
    .await
    .expect("MCP サーバーが起動する");

    assert!(
        handle.addr().ip().is_loopback(),
        "127.0.0.1 にだけバインドする"
    );
    let url = handle.url();
    assert!(url.ends_with("/mcp"), "{url}");

    let response = post(&url, None).send().await.expect("リクエストできる");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response.text().await.expect("本文を読める");
    assert!(
        body.contains("serverInfo"),
        "initialize の結果が返る: {body}"
    );
    assert!(body.contains("questloom"), "questloom として名乗る: {body}");

    handle.shutdown().await;
}

#[tokio::test]
async fn bearer_token_is_enforced() {
    let handle = serve(
        service(),
        McpServerConfig {
            port: 0,
            token: Some("s3cret".to_owned()),
        },
    )
    .await
    .expect("MCP サーバーが起動する");
    let url = handle.url();

    let response = post(&url, None).send().await.expect("リクエストできる");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

    let response = post(&url, Some("wrong"))
        .send()
        .await
        .expect("リクエストできる");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

    let response = post(&url, Some("s3cret"))
        .send()
        .await
        .expect("リクエストできる");
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    handle.shutdown().await;
}

#[tokio::test]
async fn a_busy_port_is_reported_as_an_error() {
    let first = serve(
        service(),
        McpServerConfig {
            port: 0,
            token: None,
        },
    )
    .await
    .expect("1 つ目は起動する");

    let error = serve(
        service(),
        McpServerConfig {
            port: first.addr().port(),
            token: None,
        },
    )
    .await
    .expect_err("同じポートは確保できない");
    assert!(
        error.to_string().contains("ポートを確保できません"),
        "{error}"
    );

    first.shutdown().await;
}
