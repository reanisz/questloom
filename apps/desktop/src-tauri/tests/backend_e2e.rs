//! 実アプリを起動して、内蔵 MCP サーバー越しにタスク操作を一往復させるバックエンド e2e。
//!
//! `crates/questloom-mcp/tests/http.rs` はサービスを直接組み立ててサーバーを張るので、
//! src-tauri の**起動配線**(setup フックの順序・`AppState` の初期化・`McpSupervisor` への
//! 設定反映・実ファイルの DB)は通らない。ここではビルド済みの実行ファイルをそのまま
//! 起動し、外から HTTP を叩いて、最後に一時ディレクトリの DB を読んで確かめる。
//!
//! 本物の `%APPDATA%\dev.reanisz.questloom` とポート 39150 を避けるため、
//! `QUESTLOOM_DATA_DIR` / `QUESTLOOM_MCP_PORT`(`questloom_desktop_lib::env_override`)を使う。
//!
//! **ビルド済みの exe が前提**なので `#[ignore]` を付けてある。実行方法:
//!
//! ```powershell
//! cargo build -p questloom-desktop
//! cargo test -p questloom-desktop --test backend_e2e -- --ignored
//! ```
//!
//! GUI 環境で走らせる前提(メインウィンドウが一瞬表示される)。

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use questloom_desktop_lib::env_override::{DATA_DIR_ENV, MCP_PORT_ENV};
use questloom_desktop_lib::state::AppState;
use serde_json::{json, Value};

/// 起動を待つ上限。初回はウィンドウ生成と WebView2 の立ち上げがあるので長めに取る。
const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);
/// ヘルスチェックの間隔。
const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// MCP のプロトコル版。`questloom-mcp/tests/http.rs` と揃える。
const PROTOCOL_VERSION: &str = "2025-06-18";
/// 作成するタスクのタイトル。DB を読み直すときの目印にもする。
const TITLE: &str = "バックエンド e2e のタスク";

// ---- 起動した実アプリ ----

/// 起動中の実アプリ。落ちても(panic しても)必ず kill する。
struct SpawnedApp {
    child: Option<Child>,
}

impl SpawnedApp {
    /// 実行ファイルを一時データディレクトリ + 指定ポートで起動する。
    fn spawn(exe: &Path, data_dir: &Path, port: u16) -> Self {
        let child = Command::new(exe)
            .env(DATA_DIR_ENV, data_dir)
            .env(MCP_PORT_ENV, port.to_string())
            // 失敗したときに原因が見えるよう、アプリのログはそのまま流す。
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|error| panic!("{} を起動できません: {error}", exe.display()));
        Self { child: Some(child) }
    }

    /// プロセスを kill して終了を待つ(DB ファイルのハンドルを手放させるため)。
    ///
    /// 冪等。二度呼んでも、Drop から呼ばれても安全。
    fn stop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let _ = child.kill();
        // wait を省くとゾンビが残り、Windows では DB ファイルのハンドルも解放されない。
        let _ = child.wait();
    }

    /// まだ動いているか(kill 前に落ちていないことの確認に使う)。
    fn is_running(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }
}

impl Drop for SpawnedApp {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---- MCP クライアント(Streamable HTTP の最小実装) ----

/// 実アプリの MCP エンドポイントを叩く最小クライアント。
struct McpClient {
    client: reqwest::Client,
    url: String,
    /// `initialize` の応答で受け取るセッション id。以降のリクエストに付ける。
    session: Option<String>,
}

impl McpClient {
    fn new(url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            url,
            session: None,
        }
    }

    /// JSON-RPC を 1 本投げる。
    async fn post(&self, body: &Value) -> reqwest::Result<reqwest::Response> {
        let mut request = self
            .client
            .post(&self.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .body(body.to_string());
        if let Some(session) = &self.session {
            request = request.header("mcp-session-id", session.clone());
        }
        request.send().await
    }

    /// サーバーが応答するまで待って `initialize` する。
    async fn initialize(&mut self, app: &mut SpawnedApp) {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "questloom-backend-e2e", "version": "0.1.0" },
            }
        });

        let deadline = std::time::Instant::now() + STARTUP_TIMEOUT;
        loop {
            assert!(
                app.is_running(),
                "アプリが起動前に終了しました(ログを確認してください)"
            );
            if let Ok(response) = self.post(&body).await {
                if response.status().is_success() {
                    self.session = response
                        .headers()
                        .get("mcp-session-id")
                        .and_then(|value| value.to_str().ok())
                        .map(ToOwned::to_owned);
                    let payload = read_json(response).await;
                    assert_eq!(
                        payload["result"]["serverInfo"]["name"], "questloom",
                        "questloom として名乗る: {payload}"
                    );
                    break;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{:?} 以内に MCP サーバーが立ち上がりませんでした ({})",
                STARTUP_TIMEOUT,
                self.url
            );
            tokio::time::sleep(POLL_INTERVAL).await;
        }

        // 初期化完了の通知。ここまでが MCP のハンドシェイク。
        let response = self
            .post(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await
            .expect("initialized を送れる");
        assert!(response.status().is_success(), "{:?}", response.status());
    }

    /// ツールを呼び、結果テキスト(questloom はいつも JSON を返す)をパースして返す。
    async fn call(&self, tool: &str, arguments: Value) -> Value {
        let response = self
            .post(&json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": { "name": tool, "arguments": arguments },
            }))
            .await
            .unwrap_or_else(|error| panic!("{tool} を呼べる: {error}"));
        assert!(
            response.status().is_success(),
            "{tool}: {:?}",
            response.status()
        );

        let payload = read_json(response).await;
        let result = &payload["result"];
        assert_ne!(
            result["isError"],
            Value::Bool(true),
            "{tool} がツールエラーを返しました: {payload}"
        );
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{tool} の応答にテキストがない: {payload}"));
        serde_json::from_str(text)
            .unwrap_or_else(|error| panic!("{tool} の応答が JSON でない ({error}): {text}"))
    }
}

/// 応答本文を JSON として読む。
///
/// Streamable HTTP は結果を SSE (`data: {...}`) で返しうる。先頭には接続を張った合図の
/// 空の `data:`(retry 付き)が来るので、中身のある最初の `data:` 行を拾う。
async fn read_json(response: reqwest::Response) -> Value {
    let body = response.text().await.expect("本文を読める");
    let payload = body
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .find(|data| !data.is_empty())
        .unwrap_or_else(|| body.trim());
    serde_json::from_str(payload)
        .unwrap_or_else(|error| panic!("応答を JSON として読めません ({error}): {body}"))
}

// ---- ヘルパ ----

/// ビルド済みの実行ファイルのパス(workspace の `target/debug`)。
fn desktop_exe() -> PathBuf {
    // src-tauri/../../../ が workspace ルート。
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("workspace ルートをたどれる")
        .to_path_buf();
    let exe = workspace
        .join("target")
        .join("debug")
        .join(format!("questloom-desktop{}", std::env::consts::EXE_SUFFIX));
    assert!(
        exe.is_file(),
        "{} がありません。先に `cargo build -p questloom-desktop` を実行してください",
        exe.display()
    );
    exe
}

/// 空いている TCP ポートを 1 つ借りる。
///
/// 借りたそばから手放すので厳密には競合しうるが、本物の 39150 を奪うよりはよい。
fn free_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("エフェメラルポートを取れる");
    listener.local_addr().expect("アドレスを読める").port()
}

/// `list_tasks` の結果から、指定 id のタスクを探す。
fn find_task<'a>(list: &'a Value, task_id: &str) -> Option<&'a Value> {
    list["tasks"]
        .as_array()
        .expect("tasks は配列")
        .iter()
        .find(|task| task["id"] == task_id)
}

// ---- テスト本体 ----

/// 実アプリを起動し、MCP 経由で作成 → 一覧 → 移動 → 完了 → 削除 → 復元まで往復して、
/// 最後に一時ディレクトリの DB にタスクが残っていることを確かめる。
#[tokio::test]
#[ignore = "ビルド済みの target/debug/questloom-desktop を要する (--ignored で実行)"]
async fn the_real_app_serves_tasks_over_mcp() {
    let exe = desktop_exe();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let port = free_port();

    let mut app = SpawnedApp::spawn(&exe, dir.path(), port);
    let mut mcp = McpClient::new(format!("http://127.0.0.1:{port}/mcp"));
    mcp.initialize(&mut app).await;

    // 作成: 列を指定したので通常タスクとして Today に入る。
    let created = mcp
        .call(
            "create_task",
            json!({ "title": TITLE, "description": "T1 の疎通確認", "column": "today" }),
        )
        .await;
    let task_id = created["id"].as_str().expect("id が返る").to_owned();
    assert_eq!(created["column"], "today");
    assert_eq!(created["origin"], "mcp", "MCP 経由の作成は origin=mcp");
    assert_eq!(created["isInstant"], false, "列指定なら通常タスク");

    // 一覧: 作ったばかりのタスクが見える。
    let listed = mcp.call("list_tasks", json!({})).await;
    let found = find_task(&listed, &task_id).expect("一覧に出る");
    assert_eq!(found["title"], TITLE);
    assert_eq!(found["column"], "today");

    // 移動: Doing へ。
    let moved = mcp
        .call(
            "move_task",
            json!({ "task_id": task_id, "column": "doing" }),
        )
        .await;
    assert_eq!(moved["column"], "doing");
    assert_eq!(moved["status"], "doing");

    // 完了。
    let completed = mcp
        .call("complete_task", json!({ "task_id": task_id }))
        .await;
    assert_eq!(completed["status"], "done");
    assert_eq!(completed["column"], "done");

    // 削除(ソフトデリート)。ボードから消える。
    let deleted = mcp.call("delete_task", json!({ "task_id": task_id })).await;
    assert_eq!(deleted["deleted"], true);
    let listed = mcp.call("list_tasks", json!({})).await;
    assert!(
        find_task(&listed, &task_id).is_none(),
        "削除したタスクは一覧から消える: {listed}"
    );

    // 復元。もとの列(Done)へ戻る。
    let restored = mcp
        .call("restore_task", json!({ "task_id": task_id }))
        .await;
    assert_eq!(restored["status"], "done");
    let listed = mcp.call("list_tasks", json!({ "column": "done" })).await;
    assert!(
        find_task(&listed, &task_id).is_some(),
        "復元したタスクは Done に戻る: {listed}"
    );

    assert!(app.is_running(), "往復のあいだアプリは生きている");
    app.stop();

    // 一時ディレクトリに実 DB が作られ、往復の結果が残っていること。
    let db = dir.path().join("data.db");
    assert!(db.is_file(), "{} が作られる", db.display());
    assert!(
        dir.path().join("backups").is_dir(),
        "起動時バックアップのディレクトリも作られる"
    );

    {
        // 本物の起動経路と同じ手順で開き直して中身を読む。
        let state = AppState::initialize(dir.path()).expect("DB を開き直せる");
        let board = state.service.board().expect("ボードを取れる");
        let card = board
            .columns
            .done
            .iter()
            .find(|card| card.task.id.to_string() == task_id)
            .expect("Done に残っている");
        assert_eq!(card.task.title, TITLE);
        assert_eq!(card.task.origin.to_string(), "mcp");
        assert!(card.task.deleted_at.is_none(), "復元済みなので生きている");
    }

    // TempDir の後始末で一時ディレクトリごと消える(プロセスは stop 済み)。
    dir.close().expect("一時ディレクトリを片付けられる");
}
