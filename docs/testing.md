# questloom テスト戦略

2026-08-31 のテスト充実度調査に基づく。現状の棚卸し・e2e 方式の比較・推奨構成を記す。

**実装状況(2026-08-31)**: 下記の推奨構成 1〜3 と CI はすべて実装済み。
- フロントユニット: vitest + jsdom、106 テスト(`cd apps/desktop; npm test`)
- バックエンド e2e: `apps/desktop/src-tauri/tests/backend_e2e.rs`(`#[ignore]`。実 exe +
  一時プロファイルで MCP 往復。実行方法は CLAUDE.md 参照)
- GUI e2e スモーク: @wdio/tauri-service + tauri-driver(`apps/desktop/e2e/smoke.spec.ts`、
  起動→作成→削除→復元の 4 テスト。`npm run e2e`)
- CI: `.github/workflows/ci.yml`(rust: windows / frontend: ubuntu は push・PR、
  GUI e2e は手動 + 週次)
- テスト分離: `QUESTLOOM_DATA_DIR` / `QUESTLOOM_MCP_PORT` 環境変数で実プロファイルと分離

## 現状(調査時点)

Rust テスト 212 個 + プラグイン純関数テスト(`node --test examples/plugins/github.test.mjs`)26 個。
フロントエンド(React/TS)のテストは 0。CI は未整備。

| レイヤ | 数 | 守っているもの |
|---|---|---|
| questloom-core | 82 | バケット導出・サービス層・並び順キー・設定検証・serde 表現 |
| questloom-store | 30 | マイグレーション・CRUD 永続化・バックアップ・カスケード・フォールバック |
| questloom-mcp | 13 | ツールの統合往復(インメモリ SQLite + 実サービス)・HTTP/Bearer |
| questloom-ai | 43 | Windows プロセス起動とエスケープ・JSON 抽出・プロンプト |
| src-tauri | 44 | 契約テスト(ACL・イベント名・camelCase・ウィンドウ生成方式)・MCP 監督 |
| プラグイン(TS) | 26 | GitHub プラグインの純関数(URL 解析・CI 判定・通知判断) |

強み: JSON 契約・ACL・イベント名などの「食い違い検出」テストが厚い。

### 主な穴

1. **フロントのロジック全般**: `store.ts` の世代ガード・`applyLocalMove`、`keyboard.ts` の
   Esc レイヤースタック、`settings.ts` のドラフト変換/検証(バックエンドと「揃っている」ことの
   保証がない)、`format.ts` / `viewMode.ts`、`BoardView` の D&D 挿入位置計算(要純関数化)。
2. **Tauri command 層の結合**: webview → command → State の実経路(初期化順バグの類)。
3. **GUI 操作**: D&D・ドロワー・設定画面・オーバーレイ・3 ウィンドウ連携は手動頼み。
4. **CI が無い**。

## e2e 方式の比較(2026-08 時点の Web 調査)

| 方式 | 実バックエンド | CI (windows-latest) | 評価 |
|---|---|---|---|
| **A. @wdio/tauri-service**(公式推奨。official provider = tauri-driver + msedgedriver) | ○ 実アプリ丸ごと | ○ 公式 CI 例あり | **採用**。Windows では WebView2 版数を検出して msedgedriver を自動 DL(版数一致の自前管理が不要) |
| B. 手動 tauri-driver + Selenium/WebdriverIO | ○ | ○ | A の下位互換。選ぶ理由なし |
| C. Playwright + Vite dev + mockIPC | ×(UI のみ) | ○ | e2e ではなくフロント統合。必要になったら検討 |
| D. tauri-plugin-playwright(コミュニティ) | △ | △ | 個人メンテ依存のため見送り |
| **E. MCP HTTP 経由のバックエンド e2e** | ○(UI 以外) | ○ | **採用**。実アプリを起動し `127.0.0.1:39150/mcp` を叩けば GUI なしで「起動配線+実 DB」を検証できる questloom 固有の近道 |

参考: tauri-driver 2.0.6(2026-05)・@wdio/tauri-service 1.3.0(2026-08)とも活発にメンテ中。
出典: https://v2.tauri.app/develop/tests/webdriver/ / https://webdriver.io/docs/wdio-tauri-service/

### questloom 固有の考慮

- WebDriver が触れるのは webview 内 DOM のみ。トレイ・グローバルショートカットは対象外
  (登録状態は `get_runtime_status` 経由で間接検証)。
- overlay / plugin-host ウィンドウへの WebDriver セッション切替可否は未検証。
  e2e スモークは main ウィンドウのみを対象にする。overlay の表示判定は Rust テストが既に守っている。
- **前提課題**: テストが本物の `%APPDATA%\dev.reanisz.questloom` を汚さないための
  データディレクトリ分離(環境変数によるオーバーライド)が必要。

## 推奨構成(三段構え)

1. **フロントユニット(vitest + jsdom)** — コスト最小・効果大。
   対象: format / viewMode / settings(ドラフト変換・検証)/ keyboard(レイヤースタック)/
   store(世代ガード・applyLocalMove・mutate 巻き戻し。api は mock)。
   Tauri API が絡む箇所は `@tauri-apps/api/mocks` の `mockIPC`。
2. **MCP 経由バックエンド e2e** — 実アプリ(データディレクトリ分離)を起動し、
   MCP initialize → create → list → move/complete → delete の往復と DB 状態を検証。
   既存の `questloom-mcp/tests/http.rs`(プロセス内直結)では通らない src-tauri の起動配線を守る。
3. **GUI e2e スモーク(@wdio/tauri-service)** — 起動 → ボード描画 → タスク作成 → 削除、の 1 本から。
   UI 駆動のみならアプリのコード変更不要。IPC モック等が必要になったら `tauri-plugin-wdio` を
   `cfg(debug_assertions)` 限定で導入。

## CI(GitHub Actions)方針

- `rust` ジョブ: windows-latest(src-tauri が MSVC リンクを要するため)+ Swatinem/rust-cache。
  `cargo test --workspace` + clippy。
- `frontend` ジョブ: ubuntu-latest。`npm run build` + vitest + `node --test examples/plugins/*.test.mjs`。
- `e2e` ジョブ: windows-latest。tauri build --debug + wdio。コストが高いため nightly / 手動トリガーも可。
