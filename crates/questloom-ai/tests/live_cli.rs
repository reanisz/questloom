//! 実際に AI CLI を起動する手動確認用テスト。
//!
//! 課金が発生するため既定では走らせない。実行するときは明示的に指定する。
//!
//! ```powershell
//! cargo test -p questloom-ai --test live_cli -- --ignored --nocapture
//! ```

use std::time::Duration;

use questloom_ai::{exec, prompt, provider};
use questloom_core::settings::CoreSettings;
use tokio_util::sync::CancellationToken;

/// 既定のプロンプトで claude CLI を 1 回だけ叩き、JSON 抽出まで通ることを確認する。
#[tokio::test]
#[ignore = "実 CLI を呼ぶ(課金あり)。手動でのみ実行する"]
async fn claude_returns_parsable_task_json() {
    let settings = CoreSettings::default();
    let claude = settings.ai_provider(Some("claude")).expect("claude 設定");

    let today = chrono::Utc::now().date_naive();
    let text = prompt::create_tasks_prompt("明日までに牛乳を買う。", today);
    let run = provider::prepare(claude, &text, None, Duration::from_secs(180));

    let output = exec::run(&run.request, &CancellationToken::new())
        .await
        .expect("claude が起動して終了する");
    let stdout = output.into_stdout(&claude.command).expect("正常終了する");
    println!("--- stdout ---\n{stdout}\n--------------");

    let drafts = prompt::parse_task_drafts(&stdout).expect("JSON を取り出せる");
    assert!(!drafts.is_empty());
    println!("{drafts:#?}");
}

/// 未インストールの CLI は分かりやすいエラーになる。こちらは課金なしで常に走らせてよい。
#[tokio::test]
async fn a_missing_cli_reports_install_instructions() {
    let settings = CoreSettings::default();
    let mut codex = settings
        .ai_provider(Some("codex"))
        .expect("codex 設定")
        .clone();
    // このマシンに存在しない名前にして、解決失敗のパスを通す。
    codex.command = "questloom-not-installed-cli".to_owned();

    let run = provider::prepare(&codex, "ping", None, Duration::from_secs(10));
    let error = exec::run(&run.request, &CancellationToken::new())
        .await
        .expect_err("見つからない");
    let message = error.to_string();
    assert!(message.contains("questloom-not-installed-cli"), "{message}");
    assert!(message.contains("インストールと PATH"), "{message}");
}
