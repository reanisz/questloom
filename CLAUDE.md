# CLAUDE.md

## プロジェクト概要

questloom はタスク管理・通知管理のデスクトップアプリ。当面は Windows 向けスタンドアロンだが、
将来モバイル・Web・CLI 等へ展開する可能性を見越し、コアロジックを UI から分離した Rust workspace 構成をとる。
UI は Trello 風のボード(New / Today / Tomorrow / ThisWeek / NextWeek / Future / Doing / Done)で、
時間バケットは DB に保存せず `scheduled_*` から導出する。加えて、内蔵 MCP サーバー経由で
Claude Code / Codex などの AI エージェントからタスクを操作でき、AI CLI 呼び出しやプラグイン
(第一弾は GitHub PR 監視)による自動タスク生成を備える。

## ディレクトリ構成

```
questloom/
├── Cargo.toml                       # Rust workspace ルート(共通依存は workspace.dependencies で一元管理)
├── CLAUDE.md
├── README.md
├── docs/                            # 設計ドキュメント(実装前に必ず参照)
├── apps/
│   └── desktop/                     # Tauri デスクトップアプリ
│       ├── src/                     # フロントエンド (React 19 + TypeScript)
│       ├── src-tauri/               # Tauri シェル crate (questloom-desktop)
│       ├── index.html
│       ├── package.json
│       └── vite.config.ts
└── crates/
    ├── questloom-core/              # ドメインモデル + サービス層(Tauri・UI・HTTP 非依存)
    ├── questloom-store/             # SQLite 永続化・マイグレーション・バックアップ
    ├── questloom-mcp/               # 内蔵 MCP サーバー
    ├── questloom-ai/                # AI CLI 呼び出し (claude / codex / antigravity)
    ├── questloom-plugin-api/        # プラグイン trait・イベント型定義
    └── plugins/
        └── questloom-plugin-github/ # GitHub 統合プラグイン
```

### 依存方向

- `questloom-core` は他の questloom crate に依存しない。UI・Tauri・HTTP にも依存させないこと。
- `questloom-store` / `questloom-mcp` / `questloom-ai` / `questloom-plugin-api` → `questloom-core`
- `questloom-plugin-github` → `questloom-plugin-api`, `questloom-core`
- `questloom-desktop` (src-tauri) → `questloom-core`, `questloom-store`, `questloom-mcp`
- src-tauri は「配線」だけを担う薄いシェル。Tauri command はサービス層への委譲に留める。

## ビルド / 開発コマンド

### 前提(Windows)

- Rust stable (`x86_64-pc-windows-msvc`)、Node.js、WebView2 ランタイム。
- **Visual Studio の「C++ によるデスクトップ開発」ワークロード**(MSVC v143 ツールセットのヘッダ・
  ライブラリ + Windows SDK)。これが無いと `link.exe` が `msvcrt.lib` を見つけられず、
  `cl.exe` が `excpt.h` を見つけられずに、Rust のビルドスクリプトの時点で失敗する。
  導入状況は `cd apps/desktop; npm run tauri info` の Environment セクションで確認できる。

Rust ツールチェーンが PATH に無いシェルでは、先に以下を実行する(PowerShell)。

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
```

| 目的 | コマンド |
|---|---|
| Rust 全体の型チェック | `cargo check --workspace` |
| Rust 全体のビルド | `cargo build --workspace` |
| デスクトップアプリのみビルド | `cargo build -p questloom-desktop` |
| テスト | `cargo test --workspace` |
| Lint / フォーマット | `cargo clippy --workspace --all-targets`, `cargo fmt --all` |
| フロント依存のインストール | `cd apps/desktop; npm install` |
| フロントのみのビルド(型チェック込み) | `cd apps/desktop; npm run build` |
| アプリの開発起動(ウィンドウが立ち上がる) | `cd apps/desktop; npm run tauri dev` |
| リリースバンドル作成 | `cd apps/desktop; npm run tauri build` |

補足:

- パッケージマネージャは **npm**(pnpm は未インストール)。
- PowerShell 5.1 では `&&` / `||` は使えない。`;` か `if ($?) { ... }` で繋ぐこと。
- 初回の Tauri ビルドは数分〜10 分以上かかる。タイムアウトを長めに設定すること。
- `apps/desktop/src-tauri` はルート workspace のメンバー。src-tauri/Cargo.toml に
  独自の `[workspace]` を書かないこと。

## 設計ドキュメント

実装前に必ず参照すること。設計変更が必要な場合はドキュメント側も更新する。

- `docs/architecture.md` — 技術スタック、crate 構成、サービス層・MCP・AI・プラグインの設計、セキュリティ方針
- `docs/data-model.md` — SQLite スキーマ、タスクの状態とバケット導出規則、データ保全方針
- `docs/roadmap.md` — フェーズ分割(Phase 0 スキャフォールド 〜 Phase 6 プラグイン)。
  各フェーズは「動作確認できる状態」で完了とし、フェーズごとにコミットを分ける。

## 内蔵 MCP サーバー

`crates/questloom-mcp` が、公式 Rust SDK([rmcp](https://docs.rs/rmcp) 3.x)の
**Streamable HTTP** transport を axum に載せた MCP サーバーを提供する。
アプリ起動時、設定 `mcpEnabled` が真なら自動的に立ち上がる。

- エンドポイント: **`http://127.0.0.1:39150/mcp`**(バインドは 127.0.0.1 のみ)
- 関連設定(`CoreSettings`): `mcpEnabled`(既定 true)、`mcpPort`(既定 39150)、
  `mcpToken`(既定 null。設定すると `Authorization: Bearer <token>` を要求し、
  不一致は 401)。設定を変更すると `SettingsChanged` を受けてサーバーが張り直される。
- ポート使用中などで起動に失敗した場合は error ログを出すだけで、アプリは動き続ける。

Claude Code への登録:

```powershell
claude mcp add --transport http questloom http://127.0.0.1:39150/mcp
# トークンを設定している場合
claude mcp add --transport http questloom http://127.0.0.1:39150/mcp --header "Authorization: Bearer <token>"
```

### ツール一覧

引数名は snake_case、返り値の JSON は questloom-core の serde 表現(camelCase、
日付・週・時刻は文字列)に従う。`column` は
`new` / `today` / `tomorrow` / `thisWeek` / `nextWeek` / `future` / `doing` / `done`。

| ツール | 引数 | 内容 |
|---|---|---|
| `list_tasks` | `status?`, `column?` | ボードのタスク一覧(id, title, status, column, bucket, isInstant, deadline, scheduled) |
| `get_task` | `task_id` | 詳細(関連リソース・アップデート履歴・親子込み) |
| `create_task` | `title`, `description?`, `column?`, `deadline?`, `is_instant?`, `parent_id?`, `resources?` | 作成。既定は **インスタントタスクを New へ**。`column` 指定時は通常タスクとしてその列へ |
| `update_task` | `task_id`, `title?`, `description?`, `deadline?`, `clear_deadline?` | タイトル・詳細・締切の更新 |
| `move_task` | `task_id`, `column` | 指定列の末尾へ移動(時間バケット列は予定も設定される) |
| `complete_task` | `task_id` | 完了にする(冪等) |
| `promote_task` | `task_id`, `column?` | インスタントタスクを通常タスクへ昇格(既定 `today`) |
| `add_task_update` | `task_id`, `body` | アップデート履歴を追記 |
| `add_resource` | `task_id`, `kind`, `value`, `label?`, `is_primary?` | 関連リソース(`url` / `file`)を追加 |

MCP 経由で作られたタスク・履歴の `origin` は `mcp` になる。
`deadline` は RFC 3339 文字列(例 `2026-09-30T09:00:00Z`)。

## AI 呼び出し(AI CLI 連携)

`crates/questloom-ai` が外部 AI CLI を非同期に spawn し、`apps/desktop/src-tauri/src/ai.rs` が
Tauri command として配線する。ヘッダの「✨ AI」ボタンと、タスク詳細の
「✨ AI で分割/詳細化」ボタンから使う。

### プロバイダ設定(`CoreSettings`)

| 設定 | 既定 | 内容 |
|---|---|---|
| `aiProviders` | claude / codex / antigravity | プロバイダ定義の配列(下表) |
| `aiDefaultProviderId` | `"claude"` | プロバイダ未指定時に使う `id` |
| `aiTimeoutSecs` | `300` | これを超えたらプロセスを kill する |

`AiProvider` は `{ id, label, command, args, enabled, mcpArgs, mcpSupportsToken }`。
`command` は PATH から解決し、`args` の `{prompt}` にプロンプトが入る。
`mcpArgs` は MCP 接続時に `args` の**前**へ挿入され、`{mcp_url}`(エンドポイント URL)と
`{mcp_config}`(Claude Code の `--mcp-config` に渡す JSON。トークン設定時は
`Authorization` ヘッダ込み)が置換される。`mcpArgs` が空なら MCP 非対応、
`mcpSupportsToken` が偽のプロバイダは MCP トークン設定中は MCP 接続を省略する。

既定のプロバイダ定義:

| id | command | args | MCP | enabled |
|---|---|---|---|---|
| `claude` | `claude` | `-p {prompt}` | `--mcp-config {mcp_config} --allowedTools mcp__questloom__*` | true |
| `codex` | `codex` | `exec {prompt}` | `-c features.experimental_use_rmcp_client=true -c mcp_servers.questloom.url="{mcp_url}"` | true |
| `antigravity` | `antigravity` | `-p {prompt}` | なし | **false**(引数仕様が未確認のため) |

### 機能と command

| command | 内容 |
|---|---|
| `ai_create_tasks(text, providerId?)` | 文章からタスクを抽出(JSON 配列)し、New 列に通常タスクを作成 |
| `ai_split_task(taskId, instruction?, providerId?)` | タスクを分割・詳細化し、子タスクを作成 |
| `ai_free_instruction(text, providerId?)` | MCP 経由で AI 自身に操作させ、応答テキストを返す |
| `ai_cancel()` | 実行中のプロセスを kill する |

- AI が作るタスク・履歴の `origin` は `ai`。
- **同時実行は 1 件のみ**。実行中の要求はキューに積まず拒否する。
- 進捗は `questloom://ai-status` イベント(`{state: "running"|"done"|"error", feature, message}`)で通知。
- 構造化出力は「最初にパースできた JSON 値」を採用するため、前後の説明文や
  ```` ```json ```` のコードフェンスがあっても読み取れる。

### Windows での実行時の注意

- 実行ファイルは PATH + `PATHEXT` を自前で走査して解決する(`CreateProcessW` は `.exe` しか
  探さないため、npm が作る `.cmd` シムを直接起動できない)。`.cmd` / `.bat` の場合は
  `cmd.exe /D /S /C` 経由で起動し、引数は `"` 括り + `""` エスケープで
  cmd とバッチの `%*` 再展開を通しても壊れないようにする。
- ただし **改行を含む引数は cmd.exe を通せない**ため、シム経由のプロバイダでは
  プロンプトを標準入力から渡す(`{prompt}` の引数は落とす)。
- `CREATE_NO_WINDOW` を付けてコンソールウィンドウを出さない。
- PowerShell 専用シム(`.ps1` のみ)の CLI は未対応。

## 設定画面

ヘッダ右端の歯車ボタンから、ボードを置き換えるページとして開く(`apps/desktop/src/components/SettingsPage.tsx`。
Esc / 閉じるでボードへ戻る)。節は 一般 / ショートカットとオーバーレイ / MCP サーバー / AI の 4 つ。
自動保存はせず「保存」ボタンで `set_settings` を一括呼び出しする(未保存のまま閉じるときは確認)。
検証はフロント (`apps/desktop/src/settings.ts`) とバックエンド
(`apps/desktop/src-tauri/src/settings.rs::validate`、ショートカット文字列のパースを含む)の両方で行い、
不正なら保存しない。稼働状態(MCP の URL・ショートカットの登録可否)は `get_runtime_status` で取得する。

## 注意事項

- **Tauri v2 を使用する。** v1 の API・設定ファイル形式・ネット上の記事を参照しないこと。
  v1 と v2 では `tauri.conf.json` の構造、権限モデル(capabilities / permissions)、
  プラグインのパッケージ名(`tauri-plugin-*` の v2 系)がすべて異なる。
  参照するのは https://v2.tauri.app のドキュメントのみとする。
- 権限は capabilities(`apps/desktop/src-tauri/capabilities/`)で最小限に構成する。
- MCP・その他のリッスンは 127.0.0.1 のみにバインドする。
- GitHub PAT 等のシークレットは DB に置かず、Windows 資格情報マネージャー(keyring crate)に保存する。
- crate 間の依存は上記の依存方向を守る。特に `questloom-core` を汚染しないこと。
