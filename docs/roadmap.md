# questloom 実装ロードマップ

各フェーズは「動作確認できる状態」で完了とし、フェーズごとにコミットを分ける。

## Phase 0: スキャフォールド

- [x] Cargo workspace + `apps/desktop`(Tauri v2 + React + TS + Vite)+ 空の `crates/*` を作成
- [x] `cargo build` / `npm run tauri dev` が通り、ウィンドウが表示される
- [x] `.gitignore`, `CLAUDE.md`(ビルドコマンド・構成の説明)を整備

## Phase 1: コア機能(タスク管理)

- [x] questloom-core: ドメインモデル、TaskService、バケット導出ロジック(単体テスト必須)
- [x] questloom-store: SQLite 永続化、マイグレーション、WAL、バックアップ
- [x] Trello 風ボード UI(New / Today / Tomorrow / ThisWeek / NextWeek / Future / Doing / Done)
  - ドラッグ&ドロップで状態・予定変更、列内並び替え
- [x] タスク詳細ドロワー: タイトル・詳細・締切・関連リソース・ヒストリー・親子リンク表示
- [x] インスタントタスクの見た目区別と「昇格」操作
- [x] 日付変化の監視(1 分毎)と `DayChanged` による表示更新

## Phase 2: トレイ常駐・オーバーレイ通知

- [x] タスクトレイ常駐(閉じる=トレイ格納、トレイメニューから終了)
- [x] グローバルショートカット Ctrl+Space でメインウィンドウをトグル
- [x] New タスク存在時のオーバーレイ(透過・最前面・メインディスプレイ左上)
  - インスタントタスク: URL ワンクリック起動、ワンクリック完了
- [x] 自動起動設定(tauri-plugin-autostart)

## Phase 3: MCP サーバー

- [x] questloom-mcp: rmcp + axum の Streamable HTTP サーバー(127.0.0.1、ポート設定可)
- [x] タスク操作ツール一式(architecture.md 参照)
- [ ] Claude Code から接続してタスク作成できることを確認(プロトコルレベルの initialize / tools/list は
      検証済み。実クライアントからの接続確認は未実施)

## Phase 4: AI 呼び出し

- [x] questloom-ai: CLI 即時モードの spawn 基盤(テンプレート設定、タイムアウト、進捗表示)
- [x] 文章からのタスク作成/自由指示(MCP 経由の操作を含む)
- [x] タスク詳細の「分割/詳細化」ボタン

## Phase 5: 設定画面

- [x] 設定インフラ(settings テーブル + 型付き構造体)は Phase 1 から用意
- [x] グラフィカルな設定ページ: 週開始曜日、ショートカット、オーバーレイ、自動起動、
      MCP ポート/トークン、AI CLI テンプレート

## Phase 6: プラグインシステム + GitHub 統合

プラグインは 2 層構成(architecture.md 参照)。GitHub 統合は TS プラグインのパイロットとして実装する。

- [x] TS プラグインホスト: plugin-host webview ウィンドウ、`%APPDATA%\questloom\plugins\` の読み込み、
      esbuild-wasm によるトランスパイル、型付き SDK(タスク操作・イベント・設定・KV・fetch・ポーリング)、
      manifest(権限・fetch 許可ドメイン・設定スキーマ)
- [ ] Rust 側: questloom-plugin-api の Plugin trait、PluginContext、イベント購読、スケジューラ、KV
      (コア/重量級プラグイン用。必要になったタイミングで整備する)
- [x] GitHub 統合(TS): PR URL 検出、PAT、設定可能な間隔でポーリング、
      新規コメント/CI 失敗 →「PR を確認する」インスタント子タスクを New に作成
- [x] 設定画面にプラグイン設定セクション(manifest の設定スキーマから自動生成)を追加
