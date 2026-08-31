# questloom アーキテクチャ設計

## 概要

questloom はタスク管理・通知管理ツール。当面は Windows 向けスタンドアロンのデスクトップアプリだが、
将来的に複数アプリケーション(モバイル、Web、CLI 等)からなるソリューションに拡張する可能性を見越し、
コアロジックを UI から分離した Rust workspace 構成をとる。

## 技術スタック

| レイヤ | 技術 | 備考 |
|---|---|---|
| デスクトップシェル | Tauri v2 | v1 は使用しない |
| フロントエンド | React 19 + TypeScript + Vite | |
| ドラッグ&ドロップ | dnd-kit | Trello 風ボード用 |
| フロント状態管理 | zustand | 軽量。Tauri command/event で Rust 側と同期 |
| バックエンド | Rust (stable, MSVC) | |
| 永続化 | SQLite (rusqlite, bundled) | WAL モード。詳細は data-model.md |
| MCP サーバー | rmcp (公式 Rust SDK) + axum | Streamable HTTP, localhost のみ |
| グローバルショートカット | tauri-plugin-global-shortcut | 既定 Ctrl+Space(設定変更可) |
| トレイ | Tauri v2 TrayIcon API | |
| 自動起動 | tauri-plugin-autostart | 設定で ON/OFF |

## ディレクトリ構成

```
questloom/
├── Cargo.toml                     # Rust workspace ルート
├── docs/                          # 設計ドキュメント
├── apps/
│   └── desktop/                   # Tauri デスクトップアプリ
│       ├── src/                   # フロントエンド (React + TS)
│       ├── src-tauri/             # Tauri シェル crate (questloom-desktop)
│       ├── package.json
│       └── vite.config.ts
└── crates/
    ├── questloom-core/            # ドメインモデル + サービス層(Tauri 非依存)
    ├── questloom-store/           # SQLite 永続化・マイグレーション・バックアップ
    ├── questloom-mcp/             # 内蔵 MCP サーバー
    ├── questloom-ai/              # AI CLI 呼び出し (claude/codex/antigravity)
    ├── questloom-plugin-api/      # プラグイン trait・イベント型定義
    └── plugins/
        └── questloom-plugin-github/  # GitHub 統合プラグイン
```

原則:
- `questloom-core` は UI・Tauri・HTTP に依存しない純粋なドメイン層。将来別アプリから再利用する。
- `apps/desktop/src-tauri` は「配線」だけを担う薄いシェル。Tauri command はサービス層への委譲に留める。
- crate 間の依存方向: desktop → (mcp, ai, plugins) → core ← store。core は他の questloom crate に依存しない
  (store は core のモデルを実装するため core に依存する)。

## 主要コンポーネント

### サービス層 (questloom-core)

- `TaskService`: タスクの CRUD・状態遷移・並び替え・昇格(インスタント→通常)。
- ドメインイベント: `TaskCreated` / `TaskUpdated` / `TaskMoved` / `TaskCompleted` / `DayChanged` 等を
  broadcast チャネルで発行。UI 更新・オーバーレイ・プラグインはこれを購読する。
- 時間バケット (Today/Tomorrow/…) は **保存値ではなく導出値**(data-model.md 参照)。
  日付が変わってもデータ変更なしで正しいリストに現れるため、アプリ停止中のロールオーバー漏れが発生しない。
  `DayChanged` イベントは表示更新・通知のトリガとしてのみ使う(1分毎に日付変化を監視)。

### ウィンドウ構成 (apps/desktop)

1. **メインウィンドウ**: Trello 風ボード。閉じるボタンはトレイへの格納(終了はトレイメニューから)。
   - Windows 標準のタイトルバーは使わず、`decorations: false` + 独自スタイルのタイトルバー
     (ドラッグ領域 + 最小化/最大化/閉じるボタン)にする。
   - Tomorrow / This Week / Next Week / Future は「すぐに気にしなくてよいことの先送り先」なので、
     通常表示では列として展開せず、**ドロップ先の箱 + 件数インジケーター**として小さく表示する
     (通常表示の列は New / Today / Doing / Done)。全列を展開する表示モードも用意する。
   - 通常表示は横スクロールなしで全体が収まることを優先する。
   - カードのドラッグで状態・バケットを変更。
   - タスク詳細はカードクリックでドロワー/モーダル表示: タイトル、詳細(複数行)、締切、関連リソース、
     状態更新ヒストリー、親子タスクへのリンク、AI ボタン(分割/詳細化)。
   - インスタントタスクはカード上で視覚的に区別(バッジ・配色)し、「昇格」操作を持つ。
2. **オーバーレイウィンドウ**: New タスク存在時のみ表示。透過・装飾なし・常に最前面・タスクバー非表示・
   フォーカスを奪わない。メインディスプレイ左上に配置。
   - インスタントタスクに主リソース URL があればワンクリックで開くボタンを表示。
   - インスタントタスクはワンクリック完了ボタンを表示。
   - 通常の New タスクはクリックでメインウィンドウの該当タスクを開く。
3. **設定画面**: メインウィンドウ内のページとして実装。

### MCP サーバー (questloom-mcp)

- rmcp の Streamable HTTP transport を axum に載せ、`127.0.0.1:<port>`(既定 39150、設定変更可)で待受。
- 任意の Bearer トークン認証(設定で有効化)。
- ツール(初期セット): `list_tasks`, `get_task`, `create_task`(既定でインスタントタスク),
  `update_task`, `move_task`, `complete_task`, `promote_task`, `add_task_update`, `add_resource`。
- Claude Code / Codex からは `http://127.0.0.1:<port>/mcp` を登録して利用する。

### AI 呼び出し (questloom-ai)

- 各 CLI の即時モードをコマンドテンプレートとして設定に持つ:
  - claude code: `claude -p "<prompt>"`
  - codex: `codex exec "<prompt>"`
  - antigravity: 設定でテンプレート指定
- stdout を受け取り、構造化出力(JSON)をパースする用途と、MCP 経由で自律操作させる用途の 2 系統。
- 機能:
  1. 自由指示: 文章からのタスク作成、MCP 経由でのタスク整理・更新。
  2. タスク詳細の「このタスクを分割/詳細化する」ボタン → 子タスク群を提案・作成。
- 実行は非同期 spawn + タイムアウト。実行中は UI に進捗表示。

### プラグインシステム(2 層構成)

プラグインは 2 層に分ける。API の概念(タスク操作・イベント購読・設定・KV・ポーリング)は
両層と MCP ツールで揃え、操作面が発散しないようにする。

#### 第 1 層: Rust プラグイン (questloom-plugin-api) — 構想(未実装)

コンパイル時に組み込む in-process Rust プラグイン(trait ベース)。コア機能・重い統合・
ネイティブ性能が必要なもの向け。DLL 等の動的ロードは行わない(Rust の ABI 不安定のため。
必要になったら外部プロセス + MCP / WASM を検討する)。
**現状 questloom-plugin-api は空のプレースホルダで、以下は構想段階のコード例**
(TS プラグイン層を先行させたため。roadmap.md 参照)。

```rust
trait Plugin {
    fn id(&self) -> &'static str;
    fn init(&mut self, ctx: PluginContext) -> Result<()>;
    fn on_event(&mut self, event: &DomainEvent);   // イベント購読
}
```

- `PluginContext` が提供するもの: `TaskService` ハンドル、プラグイン専用の設定名前空間(型付き JSON)、
  プラグイン専用 KV ストレージ、ポーリング用スケジューラ登録。
- 主機能のコアプラグイン化は当面行わない。

#### 第 2 層: TypeScript プラグイン(軽量・気軽に書ける)

`%APPDATA%\questloom\plugins\*.ts` にファイルを置くだけで読み込まれる軽量プラグイン。
HTTP ポーリング系の統合・軽い自動化・実験向け。追加ランタイム不要
(Tauri 同梱の webview で実行する)。

- 実行場所: 専用の非表示「plugin-host」webview ウィンドウ。独自の capability セットで権限を絞る。
- API: 型付き SDK(`defineQuestloomPlugin({...})`)。タスク操作 invoke、ドメインイベント購読、
  設定名前空間、KV、`tauri-plugin-http` によるドメインスコープ付き fetch、ポーリング登録。
- manifest(名前・必要権限・fetch 許可ドメイン・設定スキーマ)をプラグイン先頭で宣言する。
  設定スキーマから設定画面の項目を自動生成する。
- TS → JS はロード時に esbuild-wasm でトランスパイル。ファイル置き換えでホットリロード可。
- セキュリティ初期方針: ローカルファイルアクセスなし、fetch 先は manifest 宣言ドメインのみ。

#### GitHub プラグイン(TS プラグインのパイロット実装)

TS プラグイン層の実証として GitHub 統合を TS で実装する。

- 設定: PAT、ポーリング間隔(既定 5 分、変更可)。
- 動作: 全タスクの関連リソースから GitHub PR URL を検出し、REST API(ETag 利用)でポーリング。
  新規コメント・CI 失敗を検知したら「PR を確認する」インスタント子タスク(親: 元タスク、
  主リソース: PR URL)を New に作成。KV ストレージに前回状態を保持して重複作成を防ぐ。

### 設定管理

- 設定は store の `settings` テーブルに名前空間ごとの JSON で保存(コア設定 + プラグイン設定)。
- Rust 側は serde による型付き構造体 + デフォルト値。未知フィールドは無視し、前方互換を保つ。
- 主な設定項目: 週の開始曜日(既定: 月曜)、グローバルショートカット、オーバーレイ有効/無効、
  自動起動、MCP ポート/トークン、AI CLI テンプレート、GitHub プラグイン(PAT・間隔)。

## セキュリティ方針

- MCP・その他のリッスンは 127.0.0.1 のみにバインドする。
- PAT 等のシークレットは、最終的には Windows 資格情報マネージャー(keyring crate)に保存する方針。
  **ただし現状は未実装**で、PAT・MCP トークンは settings テーブルに平文で保存されている
  (既知の制限。CLAUDE.md 参照)。
- Tauri の capability は最小権限で構成する。
