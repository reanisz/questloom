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
├── examples/
│   └── plugins/                     # TS プラグイン(hello.ts / github.ts + github.test.mjs)
├── apps/
│   └── desktop/                     # Tauri デスクトップアプリ
│       ├── src/                     # フロントエンド (React 19 + TypeScript)
│       │   └── plugin-host/         # TS プラグインのホストと SDK 型定義
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
- `questloom-desktop` (src-tauri) → `questloom-core`, `questloom-store`, `questloom-mcp`, `questloom-ai`
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

「プロバイダ解決 → プロンプト生成 → CLI 実行 → 応答の解釈 → `TaskService` への反映」は
`questloom_ai::AiService`(`service.rs`)、同時実行 1 件の制御とキャンセルは
`questloom_ai::AiRunner`(`runner.rs`)にある。src-tauri 側に残るのは command 定義・
State の取り出し・`questloom://ai-status` の emit・エラーの文字列化だけで、進捗は
`AiProgress` のコールバックとして受け取る。

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

## TypeScript プラグイン

`%APPDATA%\dev.reanisz.questloom\plugins\` に `*.ts` / `*.js` を置くだけで読み込まれる軽量プラグイン層
(Phase 6a の基盤。GitHub 統合は Phase 6b)。実行場所は**非表示の `plugin-host` webview ウィンドウ**で、
プラグインのライフサイクルはすべてホスト JS が持つ(Rust 側はファイル列挙・永続化・ログ転送のみ)。

| 役割 | 場所 |
|---|---|
| Tauri command(列挙・KV・設定・リソース・ログ・ドメイン判定・レジストリ) | `apps/desktop/src-tauri/src/plugin_host.rs` |
| ホスト(ロード・activate・dispose・リロード) | `apps/desktop/src/plugin-host/host.ts` |
| **SDK の型定義(プラグイン作者向けリファレンス)** | `apps/desktop/src/plugin-host/sdk.ts` |
| トランスパイル (esbuild-wasm) | `apps/desktop/src/plugin-host/transpile.ts` |
| サンプル | `examples/plugins/hello.ts`(最小)、`examples/plugins/github.ts`(実用例) |

### プラグインの形

```ts
export default defineQuestloomPlugin({
  manifest: {
    id: "github",                     // 一意。設定名前空間・KV・origin (`plugin:<id>`) になる
    name: "GitHub 統合",
    version: "0.1.0",
    fetchDomains: ["api.github.com"], // ctx.fetch を許すホスト名(完全一致)
    settingsSchema: [                 // 設定画面に項目が自動で生える
      { key: "pat", label: "Personal Access Token", type: "secret", default: "" },
      { key: "pollIntervalMinutes", label: "ポーリング間隔(分)", type: "number", default: 5 },
      { key: "enabled", label: "有効", type: "boolean", default: true },
    ],
  },
  activate(ctx) { /* 戻り値に関数を返すと dispose 時に呼ばれる */ },
});
```

`defineQuestloomPlugin` は**ホストがグローバルに用意する**(import 不要)。
`ctx` が提供するのは `tasks` / `settings` / `kv` / `fetch` / `schedule` / `onTaskEvent` / `log`。
詳細は `sdk.ts` の JSDoc を参照。

### 既知の制限

- **1 ファイル 1 プラグイン。`import` は使えない。** ロード時に esbuild-wasm の `transform` を
  かけるだけで、バンドル(モジュール解決)はしない。
- **型検査はされない**(トランスパイルのみ)。型注釈は書けるが誤りは実行時まで分からない。
- `ctx.fetch` は webview の `fetch` なので **CORS の制約を受ける**。
  `Access-Control-Allow-Origin` を返さない API は呼べない。
- **questloom が起動している間だけ動く**。常駐サービスではなく、アプリ終了でポーリングも止まる。
- `fetchDomains` はサブドメインのワイルドカードを持たない(完全一致のみ)。
  判定の実体は Rust 側 `plugin_host::is_fetch_allowed`(テストもそこにある)。
- シークレット項目 (`type: "secret"`) は現状 **DB の `settings` テーブルに平文で入る**。
  資格情報マネージャーへの移行は今後の課題。
- ホスト JS とプラグインは同じ JS realm で動くため、`fetchDomains` は
  セキュリティ境界ではなく**事故防止のガードレール**として扱う。

### 開発時の確認

- プラグインの `ctx.log` は `plugin_log` command 経由で **本体の tracing に流れる**
  (`npm run tauri dev` のコンソールに `plugin="<id>"` 付きで出る)。
  `console.log` は非表示ウィンドウのコンソールに埋もれるので使わないこと。
- ファイルを足したり書き換えたら、設定画面の「プラグインを再読み込み」
  (`questloom://plugins-reload` を emit)で dispose → 再ロードされる。

### GitHub プラグイン(Phase 6b)

`examples/plugins/github.ts` が TS プラグイン層のパイロット実装。
未完了タスクに紐づいた GitHub PR を監視し、新しいコメント・CI 失敗を検知したら
「PR を確認する: owner/repo#123」というインスタントの子タスクを New に作る。

#### 導入

1. `examples/plugins/github.ts` を `%APPDATA%\dev.reanisz.questloom\plugins\` へコピーする。
2. 設定画面の「プラグイン」節で「プラグインを再読み込み」を押す。
3. 同じ節に生えるフォームで **Personal Access Token** を入れて保存する
   (PAT 未設定のうちは「PAT が未設定のためスキップします」とログに出るだけで何もしない)。

| 設定 | 既定 | 内容 |
|---|---|---|
| `pat` (secret) | `""` | GitHub PAT。PR を読めるだけの最小権限で発行する |
| `pollIntervalMinutes` (number) | `5` | ポーリング間隔。保存すると即座に張り直して 1 回走る |
| `enabled` (boolean) | `true` | 偽ならポーリングしない |

#### 動作

1. `get_board` + `plugin_list_task_resources` で全タスクの関連リソースを走査し、
   `https://github.com/<owner>/<repo>/pull/<番号>` を検出する。
   **Done のタスクと、このプラグイン自身が作ったタスク (`origin == plugin:github`) は対象外**。
2. PR ごとに直列で(レート制限にやさしく)REST API を叩く。
   すべて `Authorization: Bearer <pat>` + `X-GitHub-Api-Version: 2022-11-28` +
   `If-None-Match`(ETag)付き。1 PR あたり最大 5 リクエスト
   (PR 本体 / issue コメント / レビューコメント / check-runs / combined status)。
3. 通知の条件は「前回以降の新規コメント(`/user` で取った自分の login のものは除く)」と
   「CI 失敗への**遷移**」。head SHA が同じ同一の失敗は 1 度しか通知しない。
4. 通知は KV に持つ直近の通知タスク id で重複を防ぐ。そのタスクが未完了で残っていれば
   新規作成せず `add_task_update` で追記する。
5. PR がマージ/クローズされたら、どのタスクからも参照されなくなったら、KV の状態を捨てる。
6. 403/429(レート制限)に当たったらそのラウンドを打ち切ってログを出すだけ。
   PR 単位で try/catch するので、1 件の失敗で全体は止まらない。

KV(`plugin_kv` の `github` 名前空間)に持つのは、PR ごとの
`pr:<owner>/<repo>#<番号>` キー(最終コメント時刻/id、CI の head SHA・判定・通知済み状態、
エンドポイントごとの ETag、作成済み通知タスクの id)と、自分の login (`selfLogin`)。
PR を初めて観測したラウンドではコメントの通知はせず「今」を起点に記録するだけにする
(過去ログを丸ごと通知しないため)。CI が既に赤い場合だけは初回でも通知する。

#### 検証

```powershell
# 判定ロジック(純関数)のテスト
node --test examples/plugins/github.test.mjs

# 型検査(グローバル宣言を効かせるため sdk.ts も一緒に渡す)
cd apps/desktop
./node_modules/.bin/tsc --noEmit --strict --noUnusedLocals --noUnusedParameters `
  --target ES2020 --module ESNext --moduleResolution bundler `
  --lib ES2020,DOM,DOM.Iterable --skipLibCheck `
  ../../examples/plugins/github.ts src/plugin-host/sdk.ts
```

`examples` は `apps/desktop/tsconfig.json` の `include` に**入れない**
(フロント本体のビルドを汚さないため)。型検査は上のように単発で回す。

#### 既知の制限

- **PAT は `settings` テーブルに平文で保存される。** `type: "secret"` は設定画面で
  伏せ字にするだけで、暗号化も資格情報マネージャー連携もしていない。
  DB (`%APPDATA%\dev.reanisz.questloom`) を読める者は PAT を読める。keyring 化は今後の課題。
- **CORS 前提。** `ctx.fetch` は webview の `fetch` なので api.github.com 側の CORS 応答に依存する。
  現状 api.github.com は `Access-Control-Allow-Origin: *` を返し、プリフライトの
  `Access-Control-Allow-Headers` に `Authorization` / `If-None-Match` / `X-GitHub-Api-Version` を、
  `Access-Control-Expose-Headers` に `ETag` / `X-RateLimit-*` / `Retry-After` を含むので通る。
  GitHub がこれを変えたら Rust 側にプロキシ command を足す必要がある。
- GitHub Enterprise Server(自前ホスト)には未対応。API ベース URL は `api.github.com` 固定。
- コメントは 1 ページ(100 件)しか読まない。1 回のポーリング間隔に 100 件を超える更新があると
  次回に回る(`since` が進むので取りこぼしはしない)。
- questloom が起動している間だけ動く。閉じている間の変化は次の起動時にまとめて拾う。

## 設定画面

ヘッダ右端の歯車ボタンから、ボードを置き換えるページとして開く(`apps/desktop/src/components/SettingsPage.tsx`。
Esc / 閉じるでボードへ戻る)。節は 一般 / ショートカットとオーバーレイ / MCP サーバー / AI / プラグイン の 5 つ。
プラグイン節(`components/PluginSettingsSection.tsx`)だけはコア設定と独立で、
manifest の `settingsSchema` からフォームを自動生成し、プラグインごとの保存ボタンで
`plugin_set_settings` を呼ぶ(下記の一括「保存」は使わない)。
自動保存はせず「保存」ボタンで `set_settings` を一括呼び出しする(未保存のまま閉じるときは確認)。
検証はフロント (`apps/desktop/src/settings.ts`) とバックエンドの両方で行い、不正なら保存しない。
バックエンドの検証は `questloom_core::settings::CoreSettings::validate`(値の範囲・AI プロバイダ定義の
整合性)に、`apps/desktop/src-tauri/src/settings.rs::validate` がショートカット文字列のパースを
足したもの。設定の実体(`CoreSettings`)は `AppState` が保持し、`TaskService` はボード表示に要る
`BoardSettings`(週開始曜日)だけを持つ。稼働状態(MCP の URL・ショートカットの登録可否)は `get_runtime_status` で取得する。

## 注意事項

- **Tauri v2 を使用する。** v1 の API・設定ファイル形式・ネット上の記事を参照しないこと。
  v1 と v2 では `tauri.conf.json` の構造、権限モデル(capabilities / permissions)、
  プラグインのパッケージ名(`tauri-plugin-*` の v2 系)がすべて異なる。
  参照するのは https://v2.tauri.app のドキュメントのみとする。
- 権限は capabilities(`apps/desktop/src-tauri/capabilities/`)で最小限に構成する。
- MCP・その他のリッスンは 127.0.0.1 のみにバインドする。
- GitHub PAT 等のシークレットは、最終的には DB ではなく Windows 資格情報マネージャー
  (keyring crate)に置く方針。**ただし現状は未実装**で、プラグイン設定の `type: "secret"` は
  `settings` テーブルに平文で入る(上記「GitHub プラグイン」の既知の制限を参照)。
- crate 間の依存は上記の依存方向を守る。特に `questloom-core` を汚染しないこと。
