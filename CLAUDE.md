# CLAUDE.md

## プロジェクト概要

questloom はタスク管理・通知管理のデスクトップアプリ。当面は Windows 向けスタンドアロンだが、
将来モバイル・Web・CLI 等へ展開する可能性を見越し、コアロジックを UI から分離した Rust workspace 構成をとる。
UI は Trello 風のボード(Icebox / New / Today / Tomorrow / ThisWeek / NextWeek / Future /
Watching / Doing / Done)で、時間バケットは DB に保存せず `scheduled_*` から導出する。同じ考え方で **Done 列に出るのは
「今日完了した分」だけ**で、前日以前の完了は列フッタの「過去の完了 n 件…」から見る
(下記「Done 列」節)。`Watching`(監視中)は「外部の変化待ち」で、
ユーザー以外の origin による変化を受けると自動的に New へ戻る(下記「Watching」節)。
`Icebox`(棚上げ)は「いつやるかの判断ごと後回し」で、Watching と違い**自動では起きない**
(下記「Icebox」節)。加えて、内蔵 MCP サーバー経由で
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
| フロントのユニットテスト (vitest + jsdom) | `cd apps/desktop; npm test` |
| GUI e2e スモーク (WebdriverIO) | `cd apps/desktop; npm run tauri build -- --debug --no-bundle; npm run e2e` |
| プラグイン(TS)の純関数テスト | `node --test examples/plugins/github.test.mjs` |
| アプリの開発起動(ウィンドウが立ち上がる) | `cd apps/desktop; npm run tauri dev` |
| リリースバンドル作成 | `cd apps/desktop; npm run tauri build` |

補足:

- パッケージマネージャは **npm**(pnpm は未インストール)。
- PowerShell 5.1 では `&&` / `||` は使えない。`;` か `if ($?) { ... }` で繋ぐこと。
- 初回の Tauri ビルドは数分〜10 分以上かかる。タイムアウトを長めに設定すること。
- `apps/desktop/src-tauri` はルート workspace のメンバー。src-tauri/Cargo.toml に
  独自の `[workspace]` を書かないこと。

### テスト

三段構えの方針と現状の棚卸しは `docs/testing.md`。CI は `.github/workflows/ci.yml`
(rust ジョブ = windows-latest で `cargo test --workspace` + clippy、frontend ジョブ =
ubuntu-latest で `npm run build` → `npm test` → プラグインの `node --test`、
e2e ジョブ = windows-latest で GUI e2e。**手動 `workflow_dispatch` と週次 schedule のときだけ**
走り、push / PR では動かない)。

- **フロントのユニットテスト**は vitest + jsdom。`apps/desktop/src/**/*.test.ts(x)` に置き、
  `npm test` (= `vitest run`) で走る。テストは `npm run build` の tsc でも型検査される。
  React フックを触るテストは `src/test-utils.tsx` の `mount` / `renderHook` / `pressKey` を使う。
- **バックエンド e2e** (`apps/desktop/src-tauri/tests/backend_e2e.rs`) は実アプリを起動して
  MCP 越しに往復させる。ビルド済み exe が前提なので `#[ignore]` 付き。

  ```powershell
  cargo build -p questloom-desktop
  cargo test -p questloom-desktop --test backend_e2e -- --ignored
  ```

  3 本目
  (`a_plaintext_token_is_migrated_to_the_credential_manager_by_the_real_app`)は
  **本物の資格情報マネージャーに書き込む**。service 名を実行ごとに分け、成否によらず
  最後にエントリを消す。`secrets::tests::keyring_store_round_trips_against_the_real_backend`
  (`cargo test -p questloom-desktop --lib secrets -- --ignored`)も同様。

- **GUI e2e スモーク** (`apps/desktop/e2e/smoke.spec.ts`) は WebdriverIO +
  [`@wdio/tauri-service`](https://webdriver.io/docs/wdio-tauri-service/) で実アプリを操作する。
  「起動 → ボード描画 → New へタスク作成 → カードから詳細ドロワー → 削除 →
  『削除済み』から復元 → カードを右クリック → メニューから削除 → 復元 →
  監視中へ移して MCP の履歴追記で起床 → 全列展開の幅 →
  プラグインのシークレット(書き込み・ACL・平文からの移送)→ MCP トークン」を
  main ウィンドウだけで 1 本通す(9 テスト)。設定は `apps/desktop/wdio.conf.ts`。

  **前提は「フロント同梱の debug ビルド」**。`npm run tauri dev` / `cargo run` が作る exe は
  `devUrl`(Vite dev サーバー)を読みに行くので使えない。

  ```powershell
  cd apps/desktop
  npm run tauri build -- --debug --no-bundle   # target/debug/questloom-desktop.exe を作る
  npm run e2e                                  # = wdio run ./wdio.conf.ts
  ```

  - **フロントを変更したら exe を作り直すこと。** アセットは exe に焼き込まれているので、
    ビルドし直さないと古い画面のままテストされる。
  - driver は `driverProvider: "external"`(= `cargo install tauri-driver --locked` で入る
    `tauri-driver`)。未導入なら `autoInstallTauriDriver` が自動で入れる(初回は数分)。
    Windows の `msedgedriver` は WebView2 の版に合わせてサービスが自動 DL する。
  - **本物のデータは触らない。** `wdio.conf.ts` が実行ごとに一時ディレクトリ・空きポート・
    資格情報 service 名を取り、`QUESTLOOM_DATA_DIR` / `QUESTLOOM_MCP_PORT` /
    `QUESTLOOM_KEYRING_SERVICE` として渡す(下記参照)。**WebView2 のプロファイル
    (localStorage)も `QUESTLOOM_DATA_DIR` に従う**ので、`questloom.board.expanded` /
    `questloom.overlay.collapsed` を書き換える spec も利用者の実プロファイルには届かない。
    終了時に一時ディレクトリと、居残った `tauri-driver` / `msedgedriver` /
    `questloom-desktop` を片付ける(**起動前から居たプロセスには手を出さない**)。
    **資格情報エントリだけは spec 側で消す**(Node からは消せないので、シークレットを
    書いたテストは最後に必ず `null` を書いて解除する)。
  - シークレットの spec 用に、`wdio.conf.ts` が一時プロファイルの `plugins/` へ
    `e2esecret.ts`(secret 項目を 1 つ持つ最小プラグイン)を置く。
    プラグインディレクトリも `QUESTLOOM_DATA_DIR` に従うので、利用者の本物の
    プラグインは読み込まれない。
  - UI から辿れない配線(シークレット・稼働状態)は、spec の `invoke` ヘルパで
    `window.__TAURI_INTERNALS__.invoke` を直接呼んで確かめる。main の ACL で
    許されていない command(`plugin_secret_get`)が拒まれることもここで見る。
  - セレクタは最小限の `data-testid`(`titlebar` / `column-<key>` /
    `quick-add-open-<key>`(クイック追加を開くテキストボタン)/ `quick-add-<key>`(開いた後の入力欄)/
    `task-card` / `task-drawer` / `drawer-delete` / `confirm-delete` / `open-deleted` /
    `deleted-row` / `restore-task` / `task-context-menu` / `context-<action>`(右クリック
    メニューの項目。`open` / `complete` / `promote` / `move` / `url` / `url-internal` / `delete` /
    `back` / `promote-<column>` / `move-<column>`)/ `defer-box-<key>`(先送りレールの
    ボックス)/ `toggle-expanded`(全列展開のトグル))。
    ダイアログは `ModalShell` の `aria-label` で引く。
  - Watching の往復と展開表示の 2 本は、内蔵 MCP を spec から直接叩く
    (`QUESTLOOM_E2E_MCP_PORT` を読んで `http://127.0.0.1:<port>/mcp` へ JSON-RPC)。
    「UI で監視中へ移す → MCP で履歴追記 → New に戻る」を実アプリで通し、
    展開表示では 10 列(Icebox が一番左)が並び、`.board` の
    `scrollWidth <= clientWidth` であることを実寸で確かめる。
  - **利用者が questloom を起動したままだと exe を差し替えられない**
    (`tauri build` が `failed to remove file ... os error 5` で落ちる)。そのときは
    リンク済みの `target/debug/deps/questloom_desktop.exe` を別の場所へ
    `questloom-desktop.exe` という名前でコピーし(名前を保つと後片付けの対象に入る)、
    `QUESTLOOM_E2E_APP_BINARY` にそのパスを入れて `npm run e2e` を走らせる。
  - overlay / plugin-host も同じバンドルを読むためウィンドウハンドルは複数返る。
    spec の冒頭で `[data-testid="titlebar"]`(= `App` にしか無い)を持つものへ切り替える。

### テスト用の環境変数

実アプリを本物のデータ・ポートから切り離して起動するための上書き
(`apps/desktop/src-tauri/src/env_override.rs`)。どちらも未設定なら従来どおり。

- `QUESTLOOM_DATA_DIR` — `app_data_dir()`(`%APPDATA%\dev.reanisz.questloom`)の代わりに
  このディレクトリを使う。分離される範囲は **DB・バックアップ・`plugins/`・WebView2 プロファイル**。
  - `plugins/` の探索先もここに従う(`plugin_host::plugins_dir` は `AppState::data_dir` を
    基準にする)。`app_data_dir()` を直接引くと、一時プロファイルのはずのテストが
    利用者の本物のプラグインを読み込んでしまう。**ただしアプリ同梱の標準プラグインは
    `<resource_dir>/plugins` にあるので、これを指定しても読み込まれる**
    (シークレットが見えなければ何もしないので、`QUESTLOOM_KEYRING_SERVICE` を
    併せて渡していれば無害)。
  - **WebView2 のユーザーデータフォルダは `<dir>\webview` になる**
    (`env_override::webview_data_dir`)。ここに localStorage
    (`questloom.board.expanded` / `questloom.overlay.collapsed`)・Cookie・キャッシュが入る。
    Tauri の既定は `%LOCALAPPDATA%\dev.reanisz.questloom` で、**データディレクトリとは
    別系統**なので、これを引き込まないとテストが利用者の実 localStorage を読み書きしてしまう。
    渡し方は Tauri v2 の `WebviewWindowBuilder::data_directory` /
    `WebviewBuilder::data_directory`。**webview を作る箇所すべて**
    (`lib.rs::create_windows` の main / overlay / plugin-host と、
    `browser.rs::browser_pane_open` の browser-pane)で渡すこと。渡し忘れた webview だけが
    実プロファイルに残る。`WEBVIEW2_USER_DATA_FOLDER` 環境変数は効かない
    (Tauri が Windows では必ずフォルダを明示するので、WebView2 ローダーは env を見ない)。
    未設定なら `data_directory` を一切呼ばず、従来どおり Tauri の既定に落ちる。
- `QUESTLOOM_MCP_PORT` — コア設定の `mcpPort` を無視してこのポートで待ち受ける
  (本物の 39150 と衝突させないため)。`u16` として読めない値は無視して設定値に落ちる。
- `QUESTLOOM_KEYRING_SERVICE` — シークレットの service 名(既定 `questloom`)を差し替える。
  **資格情報エントリはデータディレクトリと違ってユーザー単位でグローバル**なので、
  `QUESTLOOM_DATA_DIR` だけでは本物のエントリと分離できない。実アプリを起動する
  テスト・検証では必ず一緒に渡すこと(`wdio.conf.ts` と `tests/backend_e2e.rs` は
  実行ごとにユニークな名前を渡している)。

## 設計ドキュメント

実装前に必ず参照すること。設計変更が必要な場合はドキュメント側も更新する。

- `docs/architecture.md` — 技術スタック、crate 構成、サービス層・MCP・AI・プラグインの設計、セキュリティ方針
- `docs/data-model.md` — SQLite スキーマ、タスクの状態とバケット導出規則、データ保全方針
- `docs/roadmap.md` — フェーズ分割(Phase 0 スキャフォールド 〜 Phase 6 プラグイン)。
  各フェーズは「動作確認できる状態」で完了とし、フェーズごとにコミットを分ける。

## Watching(監視中)

`TaskStatus::Watching` / `BoardColumn::Watching` は「今すぐ作業はしないが、外の変化を
待っている」状態。仕様の確定版は `docs/data-model.md`(「タスクの状態とバケットの考え方」)。

- 時間バケットは持たない(New / Doing / Done と同じく `resolve` は**予定を保持**する)。
  Watching から出入りしても `scheduled` は失われない。
- **起床ルール**: ユーザー以外の origin(`mcp` / `ai` / `plugin:*` / `system`)による変化を
  受けると、サービス層が `status` を `new` へ戻し(予定は保持、New 列の末尾)、
  `TaskWoken` + `TaskMoved` を発行する。起床すればオーバーレイも既存経路で出る。
- 起床の**トリガーは 4 つだけ**。いずれも呼び出し元の origin が API に載っている操作。
  1. `add_task_update`(origin が非 User)
  2. `create_task`(origin が非 User)で `parent_id` が Watching のタスク → 親を起こす
  3. `add_checklist_item`(origin が非 User)
  4. `update_checklist_item`(origin が非 User)— **チェック・本文のどちらの変更でも**
- チェックリストの `remove_checklist_item` / `reorder_checklist_item` は**トリガーにしない**。
  「消えた」「並びが変わった」は待っていた変化ではないため(reorder はそもそも origin を取らない)。
- `add_resource` / `remove_resource` は**トリガーにしない**。これらの API は origin を
  受け取っておらず、渡せるようにしても「メインウィンドウの手動追加」と「プラグインの追加」を
  区別できない(呼び出し側の自己申告になる)。実用シナリオは履歴追記と子タスク作成でほぼ
  覆えるため、API を広げるより起床経路を絞る方を選んだ。
- `update_task` もトリガーにしない。フロントのユーザー編集と同じ command なので、
  自分で書き換えただけで起きてしまう。
- ユーザー操作(ドラッグ・右クリックメニューの「移動」)での出入りは通常の `move_task`。
  **昇格先 (`PROMOTE_COLUMNS`) には含めない**(昇格は Todo 系のみという現仕様を維持)。
- UI: 通常表示では先送りレールの「監視中」ボックス(`DEFER_COLUMNS`)、展開表示では 8 列目。
  列ヘッダとレールのラベルに 👁 を添えるだけで、カード自体の装飾はしない。
- DB は `status` が自由な TEXT なのでマイグレーション不要
  (`migrations.rs` の `a_v3_database_accepts_new_statuses_without_a_migration` が担保)。

## Icebox(棚上げ)

`TaskStatus::Icebox` / `BoardColumn::Icebox` は「いつやるかの判断ごと後回しにする」置き場。
仕様の確定版は `docs/data-model.md`(「タスクの状態とバケットの考え方」)。

- Future(Todo の「いつか」)と違い **Todo ですらない**。時間バケットは持たず、
  `resolve` は New / Watching / Doing / Done と同じく**予定を保持**する
  (Icebox から出し入れしても `scheduled` は失われない)。
- **自動起床は無い。** 外部の変化を待っているのは Watching だけで、Icebox は
  `add_task_update` などを外部 origin で受けても動かない(`wake` は `Watching` しか見ない)。
  出すのは常に手動、または MCP / AI の明示的な `move_task`。
- **昇格先 (`PROMOTE_COLUMNS`) には含めない**(昇格は Todo 系のみ)。
  右クリックメニューの「移動」からは選べる(現在列なら無効化)。
- UI: 通常表示では先送りレールの「Icebox」ボックス(`DEFER_COLUMNS` の Future と監視中の間)、
  展開表示では**一番左**の列。列挙順・ボード JSON 上の位置も先頭
  (`BoardColumns::iter` / `BOARD_COLUMNS` / 契約テスト `board_json_puts_icebox_first`)。
- DB は Watching と同じくマイグレーション不要。

## Done 列(今日の完了だけ)

**ボードの Done 列に出るのは「今日完了した分」だけ**で、前日以前に完了したタスクは
列フッタの「過去の完了 n 件…」リンク → ダイアログ(`components/ArchivedDoneDialog.tsx`)で見る。
時間バケットと同じく**保存された状態ではなく表示時の導出**なので、データは 1 行も動かない。
日付が変われば既存の `DayChanged` → 再フェッチでひとりでに消える。

- 判定は `TaskService::board()`。`done_at` の**ローカル日付**が `Clock::today()` より前なら
  列から外し、`Board::archivedDoneCount` に件数だけ載せる。`done_at` を持たない完了タスク
  (サービス経由では作れないが古いデータにはありうる)は隠さない。
- UTC ↔ ローカル日付の変換は **`Clock` の責務**(`local_date` / `local_day_start`)。
  サービス層が `chrono::Local` を直接触ると、`FixedClock` で今日を固定してもタイムゾーン
  絡みの判定だけ実機に引きずられる。`FixedClock::with_offset_hours` でその境界も試せる。
- 中身は `list_archived_done`(`TaskService::list_archived_done` → `ArchivedDone`)。
  完了が新しい順に **最大 `ARCHIVED_DONE_LIMIT`(200)件**で、総件数は `total`。
  ページングは無く、切り詰めたときはダイアログが「全 n 件のうち…」と断る。
  削除済みは含まない。SQL は `repository.rs` の `DONE_BEFORE`(`done_at` は固定長の
  RFC3339 テキストなので文字列比較でよい)。
- **`get_board` 越しに見えるのも今日の完了まで**(オーバーレイ・plugin-host も同じ)。
  完了済みを全部見たい呼び出し側は `TaskService::full_board()` を使う。使っているのは
  MCP の `list_tasks` だけで、AI には従来どおり全件を見せる。
- `list_archived_done` command は **main ウィンドウだけ**に配る(削除・復元と同じ扱い)。

## タスク内チェックリスト

「子タスクにするほどではない項目」の置き場。スキーマは `task_checklist_items`
(**マイグレーション v3**)で、仕様の確定版は `docs/data-model.md`。

- モデルは `ChecklistItem { id, task_id, body, checked, sort_order, created_at }`。
  `id` は `ChecklistItemId`(UUID v7 の newtype)。並びは他と同じ fractional key。
- `TaskDetail` に `checklist: Vec<ChecklistItem>`(`sort_order` 昇順)、`TaskCard` に
  `checklistDone` / `checklistTotal`。**ボードの集計は 1 クエリ**
  (`list_all_checklist_items` → `HashMap` で数える。リソースの `resource_index` と同じ形)。
  削除済みタスクの項目は集計から落ちる。
- **チェックが全部埋まってもタスクは自動完了しない。** 進捗の表示だけ。
- サービス層の 4 つはすべて task の `updated_at` を進め、`TaskChecklistChanged` を発行する。
  削除済み・存在しないタスクへの操作は既存どおり拒否。

  | メソッド | origin | 起床 |
  |---|---|---|
  | `add_checklist_item(task_id, body, origin)` | 取る | **する**(非 User) |
  | `update_checklist_item(task_id, item_id, body?, checked?, origin)` | 取る | **する**(非 User。チェック・本文どちらでも) |
  | `remove_checklist_item(task_id, item_id, origin)` | 取る | しない |
  | `reorder_checklist_item(task_id, item_id, prev_id?, next_id?)` | 取らない | しない |

- 項目 id は**タスクをまたげない**。別タスクの `item_id` を渡すと
  `ChecklistItemNotFound` で弾く(`require_checklist_item`)。
- 削除は冪等では**ない**(無ければ `ChecklistItemNotFound`)。タスクと違いソフトデリートもしない。
- `TaskChecklistChanged` はオーバーレイの `affects_visibility` に**入れない**。New の件数を
  動かさないため。起床した場合は直後に New 列への `TaskMoved` が続くのでそちらで拾われる。
- Tauri command(`add_` / `update_` / `remove_` / `reorder_checklist_item`)は
  **origin を `User` に固定**し、**main ウィンドウだけ**に配る。プラグイン・AI が触る道は
  MCP のツール(origin は `Mcp`)で、そちらは起床する。
- UI はドロワーの「チェックリスト」節(`components/ChecklistSection.tsx`。リソースと親タスクの間)。
  ヘッダに進捗 `2/5`、各行はチェックボックス + 本文(クリックでインライン編集、Enter/blur で確定、
  Esc で取り消し)+ 削除 ✕、下部に常設の追加入力欄(Enter で連続追加)。
  カードには `checklistTotal > 0` のとき `☑ 2/5` バッジ。
  **並び替えの UI はまだ無い**(`reorder_checklist_item` command と `api.reorderChecklistItem`
  は用意済み。ドラッグ&ドロップは後日)。
- `ChecklistSection` は**バックエンドに触らない表示専用**にしてある(書き込みは TaskDrawer が
  `api` + `mutate` で行う)。おかげで Tauri の invoke 抜きに vitest で回せる。

## 内蔵 MCP サーバー

`crates/questloom-mcp` が、公式 Rust SDK([rmcp](https://docs.rs/rmcp) 3.x)の
**Streamable HTTP** transport を axum に載せた MCP サーバーを提供する。
アプリ起動時、設定 `mcpEnabled` が真なら自動的に立ち上がる。

- エンドポイント: **`http://127.0.0.1:39150/mcp`**(バインドは 127.0.0.1 のみ)
- 関連設定(`CoreSettings`): `mcpEnabled`(既定 true)、`mcpPort`(既定 39150)。
  設定を変更すると `SettingsChanged` を受けてサーバーが張り直される。
- **Bearer トークンはコア設定に入っていない。** 実体は Windows 資格情報マネージャー
  (service `questloom` / エントリ `core/mcp-token`。「シークレットの保存先」節を参照)で、
  設定すると `Authorization: Bearer <token>` を要求し、不一致は 401 になる。
  読み書きは `get_mcp_token_status`(設定済みか否かだけ)/ `set_mcp_token`(設定・解除)の
  2 つの command で、**値をアプリ側へ読み出す経路は無い**。変更すると
  `SettingsChanged` が飛んでサーバーが張り直される。
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
`icebox` / `new` / `today` / `tomorrow` / `thisWeek` / `nextWeek` / `future` / `watching` /
`doing` / `done`(`status` は `new` / `todo` / `doing` / `done` / `watching` / `icebox`)。

| ツール | 引数 | 内容 |
|---|---|---|
| `list_tasks` | `status?`, `column?` | ボードのタスク一覧(id, title, status, column, bucket, isInstant, deadline, scheduled)。**`done` は完了済み全件**(画面の Done 列は今日の分だけ)|
| `get_task` | `task_id` | 詳細(関連リソース・アップデート履歴・親子込み) |
| `create_task` | `title`, `description?`, `column?`, `deadline?`, `is_instant?`, `parent_id?`, `resources?` | 作成。既定は **インスタントタスクを New へ**。`column` 指定時は通常タスクとしてその列へ |
| `update_task` | `task_id`, `title?`, `description?`, `deadline?`, `clear_deadline?` | タイトル・詳細・締切の更新 |
| `move_task` | `task_id`, `column` | 指定列の末尾へ移動(時間バケット列は予定も設定される。`watching` で「外部の変化待ち」、`icebox` で「棚上げ」にできる。どちらも予定は保持) |
| `complete_task` | `task_id` | 完了にする(冪等) |
| `delete_task` | `task_id` | 削除する(ソフトデリート。ボードから消えるだけで復元可能。子タスクへはカスケードしない。冪等) |
| `restore_task` | `task_id` | 削除済みタスクを復元する(現ステータス列の末尾へ。冪等) |
| `promote_task` | `task_id`, `column?` | インスタントタスクを通常タスクへ昇格(既定 `today`) |
| `add_task_update` | `task_id`, `body` | アップデート履歴を追記(対象が `watching` なら New へ起床させる) |
| `add_resource` | `task_id`, `kind`, `value`, `label?`, `is_primary?` | 関連リソース(`url` / `file`)を追加 |
| `set_primary_resource` | `task_id`, `resource_id`, `is_primary?` | 既存リソースを主リソースにする(既定 true)/ 解除する。主は 1 タスクに 1 つで、付け替えると前の主は外れる。冪等 |
| `add_checklist_item` | `task_id`, `body` | チェックリストの末尾へ項目を追加(空白のみは拒否。対象が `watching` なら New へ起床させる) |
| `set_checklist_item` | `task_id`, `item_id`, `checked?`, `body?` | チェックリスト項目のチェック・本文を変更(同上。省略した項目は変えない) |
| `remove_checklist_item` | `task_id`, `item_id` | チェックリスト項目を削除(**起床させない**。冪等ではない) |

`item_id` は `get_task` の返り値 `checklist[]` から取る。
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

アプリ同梱の**標準プラグイン**(`<resource_dir>\plugins\`)と、
`%APPDATA%\dev.reanisz.questloom\plugins\` に `*.ts` / `*.js` を置くだけで読み込まれる
**利用者配置**のプラグインからなる軽量プラグイン層
(Phase 6a の基盤。GitHub 統合は Phase 6b)。実行場所は**非表示の `plugin-host` webview ウィンドウ**で、
プラグインのライフサイクルはすべてホスト JS が持つ(Rust 側はファイル列挙・永続化・ログ転送のみ)。

| 役割 | 場所 |
|---|---|
| Tauri command(列挙・KV・設定・リソース・ログ・ドメイン判定・レジストリ) | `apps/desktop/src-tauri/src/plugin_host.rs` |
| ホスト(ロード・activate・dispose・リロード) | `apps/desktop/src/plugin-host/host.ts` |
| id の重複裁定(標準 / ユーザーの優先規則) | `apps/desktop/src/plugin-host/claim.ts` |
| **SDK の型定義(プラグイン作者向けリファレンス)** | `apps/desktop/src/plugin-host/sdk.ts` |
| トランスパイル (esbuild-wasm) | `apps/desktop/src/plugin-host/transpile.ts` |
| サンプル | `examples/plugins/hello.ts`(最小)、`examples/plugins/github.ts`(実用例 = 標準プラグイン) |

### 標準プラグイン(アプリ同梱)

**GitHub 統合 (`examples/plugins/github.ts`) はアプリに同梱される。** インストール直後から
設定画面に「標準」として並び、PAT を入れるだけで動く。プラグインフォルダへのコピーは不要
(コピー方式はプロファイル側の版が古いまま滞留するのでやめた)。

- **実体は `examples/plugins/github.ts` ただ 1 つ。** `tauri.conf.json` の
  `bundle.resources` が `"../../../examples/plugins/github.ts": "plugins/github.ts"` と
  マッピングしてそのまま配るので、コピーも同期処理も無い。同梱するファイルを増やすときも
  ここへ足す(`src-tauri/resources/` は**作らない**。`lib.rs` の
  `builtin_plugins_are_bundled_straight_from_examples` が見ている)。`hello.ts` は同梱しない。
- 読み込み元は `<resource_dir>/plugins`。**`npm run tauri dev` でも読める**
  (`tauri-build` が `bundle.resources` を cargo の出力ディレクトリ = exe の隣にも
  コピーし、Windows の `resource_dir` は exe のあるディレクトリを指すため)。
  したがって dev 専用のフォールバックは持たない。
- **同じ id なら利用者配置が勝つ。** `plugin_list_sources` はユーザー配置 → 同梱の順で
  返し、ホストは先に来た方に id を確保させる(`claim.ts`)。隠された同梱版は
  エラーにも一覧にもせず読み飛ばし、勝ったユーザー版に `shadowsBuiltin` を立てる。
  設定画面はそのカードに「標準版(アプリ同梱)を上書きしています」と出す。
  ファイルを消せば同梱版に戻る。
- 設定・KV・シークレットの名前空間は id ベース (`plugin:<id>`) なので、
  標準版とユーザー版のどちらでロードされても引き継がれる。
- ファイル名の検証・トラバーサル対策 (`is_plugin_file_name`) は同梱側にも同じく効く。

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
`ctx.tasks` は `createTask` / `updateTask` / `getTask` / `listTasks` / `completeTask` /
`addTaskUpdate` / `addResource` / `listAllResources` / `moveTask`。
`updateTask` はタイトル・詳細・締切の差分更新で、`origin` を持たない command なので
更新してもタスクの出所は変わらない(**ユーザーが書いた文章を上書きしない**判断はプラグイン側の責任)。
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
- シークレット項目 (`type: "secret"`) の値は **Windows 資格情報マネージャー**に入る
  (`plugin:<id>/<key>`。「シークレットの保存先」節を参照)。プラグインから見た形は
  他の項目と同じで、`ctx.settings.get()` が値を混ぜて返す。ただし**設定画面には
  「設定済み / 未設定」しか出ない**(一度書いた値は画面から読み出せない)。
- ホスト JS とプラグインは同じ JS realm で動くため、`fetchDomains` は
  セキュリティ境界ではなく**事故防止のガードレール**として扱う。同じ理由で、
  シークレットの ACL 分離(`plugin_secret_get` は plugin-host のみ)も
  **プラグイン同士を隔てるものではない**(同じ realm なので互いの値に手が届く)。
  外(DB を読める第三者・バックアップ・ログ)からシークレットを守るための仕組みである。

### 開発時の確認

- プラグインの `ctx.log` は `plugin_log` command 経由で **本体の tracing に流れる**
  (`npm run tauri dev` のコンソールに `plugin="<id>"` 付きで出る)。
  `console.log` は非表示ウィンドウのコンソールに埋もれるので使わないこと。
- ファイルを足したり書き換えたら、設定画面の「プラグインを再読み込み」
  (`questloom://plugins-reload` を emit)で dispose → 再ロードされる。

### GitHub プラグイン(Phase 6b)

`examples/plugins/github.ts` が TS プラグイン層のパイロット実装。機能は 3 つ。

- **PR の監視**(PAT 必須)。未完了タスクに紐づいた GitHub PR を監視し、
  新しいコメント・CI 失敗を検知したら「PR を確認する: owner/repo#123」という
  インスタントの子タスクを New に作る。
- **受信箱の取り込み**(PAT 必須)。自分宛のレビュー依頼と、自分へのメンションを
  「レビュー依頼: owner/repo#123」「メンション: owner/repo#123」というインスタントタスクとして
  New に作る。**どのタスクからも追いかけていないものだけ**が対象なので PR の監視と重複しない。
- **description の自動記入**(PAT 任意)。PR の URL が付いたタスクの description が
  空なら、PR のタイトル・状態・作者・本文の先頭を書き込む。

#### 導入

**アプリ同梱の標準プラグインなので、コピーは要らない。** 設定画面の「プラグイン」節に
「標準」バッジ付きで並んでいるので、そこに生えるフォームで **Personal Access Token** を
入れて保存するだけでよい(PAT 未設定のうちは PR の監視は「PAT が未設定のためスキップします」
とログに出るだけで何もしない。description の自動記入は PAT 無しでも public な PR に対して動く)。

手を入れたい場合は `examples/plugins/github.ts` をプラグインフォルダへコピーして編集する。
同じ id なので同梱版を隠して読み込まれ、カードに「標準版を上書きしています」と出る。
書き換えたら「プラグインを再読み込み」を押す。ファイルを消せば同梱版に戻る。

| 設定 | 既定 | 内容 |
|---|---|---|
| `pat` (secret) | `""` | GitHub PAT。PR を読めるだけの最小権限で発行する。空なら認証なしで叩く。値は Windows 資格情報マネージャー(`plugin:github/pat`)に入り、設定画面には「設定済み / 未設定」しか出ない |
| `pollIntervalMinutes` (number) | `5` | ポーリング間隔。保存すると即座に張り直して 1 回走る |
| `watchReviewRequests` (boolean) | `true` | 偽ならレビュー依頼を取り込まない |
| `watchMentions` (boolean) | `true` | 偽ならメンションを取り込まない |
| `enabled` (boolean) | `true` | 偽なら 3 つの機能とも動かない |

#### 動作(PR の監視)

1. `get_board` + `plugin_list_task_resources` で全タスクの関連リソースを走査し、
   `https://github.com/<owner>/<repo>/pull/<番号>` を検出する。
   **Done のタスクと、このプラグイン自身が作ったタスク (`origin == plugin:github`) は対象外**。
2. PR ごとに直列で(レート制限にやさしく)REST API を叩く。
   すべて `Authorization: Bearer <pat>` + `X-GitHub-Api-Version: 2022-11-28` +
   `If-None-Match`(ETag)付き。1 PR あたり最大 5 リクエスト
   (PR 本体 / issue コメント / レビューコメント / check-runs / combined status)。
3. 通知の条件は「前回以降の新規コメント(`/user` で取った自分の login のものは除く)」と
   「CI 失敗への**遷移**」。head SHA が同じ同一の失敗は 1 度しか通知しない。
4. 通知のしかたは `planNotification` が決める。
   - **PR を参照しているタスクが「監視中」(`watching`)なら、通知タスクを作らず
     そのタスク自身へ `addTaskUpdate` する**。プラグインの追記は origin が
     `plugin:github` なので、本体の起床ルールがそのタスクを New へ戻す。
   - 監視中が無ければ従来どおり。KV に持つ直近の通知タスク id で重複を防ぎ、
     そのタスクが未完了で残っていれば新規作成せず `add_task_update` で追記する。
   - 起床では `noticeTaskId` を触らないので、起きた次のラウンド(もう `watching` ではない)
     からは自然に従来ルートへ戻る。
5. PR がマージ/クローズされたら、どのタスクからも参照されなくなったら、KV の状態を捨てる。
6. 403/429(レート制限)に当たったらそのラウンドを打ち切ってログを出すだけ。
   PR 単位で try/catch するので、1 件の失敗で全体は止まらない。

KV(`plugin_kv` の `github` 名前空間)に持つのは、PR ごとの
`pr:<owner>/<repo>#<番号>` キー(最終コメント時刻/id、CI の head SHA・判定・通知済み状態、
エンドポイントごとの ETag、作成済み通知タスクの id)と、自分の login (`selfLogin`)。
PR を初めて観測したラウンドではコメントの通知はせず「今」を起点に記録するだけにする
(過去ログを丸ごと通知しないため)。CI が既に赤い場合だけは初回でも通知する。

#### 動作(レビュー依頼・メンションの取り込み)

PR の監視とは独立に、**1 ラウンドにつき検索 API を 2 回**だけ叩く
(見張っている PR が 0 件でも走る。認証時の検索レート制限は 30 req/min なので余裕がある)。

1. クエリはどちらも `@me` を使う。**`@me` は認証しているユーザー自身に解決される**ので
   `/user` で login を引かなくてよい(未認証で投げると 422)。
   - レビュー依頼: `type:pr state:open review-requested:@me`
     — 自分に直接来た依頼に加えて**自分が属するチームへの依頼**も含む。
   - メンション: `state:open mentions:@me updated:><前回確認時刻>`
     — `mentions:` に「いつメンションされたか」の条件は無いので更新時刻の窓で切る。
2. `sort=updated&order=desc` + `per_page=50` の**1 ページで打ち切る**。
   **検索 API は `ETag` を返さない**(`Cache-Control: no-cache`)ので条件付きリクエストはしない。
3. **既に未完了タスクの関連リソースで追いかけている issue/PR は取り込まない**
   (`collectTrackedKeys`。PR の監視と同じ集合を見るので二重通知にならない)。
   このプラグイン自身が作ったタスク (`origin == plugin:github`) の参照も数えない。
   issue と PR は番号空間を共有するので、`owner/repo#番号` のキーで両方をまとめて扱う。
4. 通知は `decideNoticeAction` 経由。KV の直近の通知タスクが未完了で残っていれば
   新規作成せず `add_task_update` で追記する。作るのは New のインスタントタスクで、
   対象 URL を主リソースに、description にタイトル・出典・理由・URL を並べる。
   両方に同時に当たったらレビュー依頼を主に見立てて 1 件にまとめる。
5. 403/429 に当たったらそのラウンドを打ち切る。それ以外の失敗は 1 件ずつ握りつぶし、
   PR の監視には波及させない。

KV に持つのは `inbox:<owner>/<repo>#<番号>`
(`reviewNotified` / `mentionNotifiedAt` / `noticeTaskId` / `seenAt`)と、
メンション検索の起点 `inboxMentionSince`。

- **レビュー依頼は生きている間 1 度だけ**通知する(re-request で `updated` が動いても再通知しない)。
  依頼が解消されて検索結果から消えたら `reviewNotified` を解除し、再依頼をまた知らせる。
- **メンションは `mentionNotifiedAt` より新しい更新のときだけ**通知する。
  `inboxMentionSince` が未設定の初回ラウンドは「今」を記録するだけで通知しない(過去分の洪水防止)。
- 掃除は `planInboxPrune`。レビュー依頼の集合に居れば残し、最後の観測から 30 日
  (`INBOX_TTL_DAYS`)過ぎたら捨てる。メンションは「結果に出てこない = 消えた」と言えないので
  クローズの検知は `state:open` クエリ + TTL に任せている。

#### 動作(description の自動記入)

1. トリガーは `ctx.onTaskEvent`(1.5 秒デバウンス)と、ポーリングラウンドの先頭
   (イベントの取りこぼし対策)。**PAT の有無に関係なく走る**。
2. 対象は「description が空(空白のみを含む)で、PR の URL を関連リソースに持つタスク」。
   1 タスクにつきリソース順で最初に見つかった PR を 1 件だけ使う。
   このプラグインが作ったタスク (`origin == plugin:github`) と削除済みタスクは対象外。
   Done も対象に含める(ただし `get_board` が返す Done は**今日完了した分まで**なので、
   前日以前に完了したタスクには結局書き込まない)。
3. `GET /repos/{owner}/{repo}/pulls/{番号}` を 1 回だけ叩き(PAT があれば `Authorization` 付き、
   無ければ認証なし)、次の形を `update_task` で書き込む。本文が空なら 2 行目までで終わる。

   ```
   <PR タイトル>
   <owner>/<repo>#<番号> (open|merged|closed) by <作者>

   <PR 本文の先頭 400 文字。切ったら末尾に「…」>
   ```

4. 書き込んだら KV に `desc:<taskId>` = `{ pr: "<owner>/<repo>#<番号>", at: "<RFC 3339>" }` を残し、
   **そのタスクには二度と触らない**。記入後にユーザーが編集・削除しても埋め直さない。
   ボードから消えたタスクの記録はポーリングのラウンドで掃除する。
5. 403 / 404(private・削除済み・認証なしでは見えない PR)は debug ログを出して黙って飛ばす。
   レート制限に当たったらそのラウンドを打ち切る。ここでの失敗は PR の監視に波及しない。

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

- **PAT は Windows 資格情報マネージャー**(service `questloom` / エントリ
  `plugin:github/pat`)に入る。DB (`%APPDATA%\dev.reanisz.questloom`) には残らない。
  以前のバージョンが `settings` テーブルへ平文で書いた PAT は、次のプラグイン
  ロード時に自動で移送され JSON からは消える(「シークレットの保存先」節を参照)。
  ただし**同じ realm で動く他のプラグインからは `ctx.settings.get()` 越しに見えない
  だけで、ホスト JS を経由すれば手が届く**。プラグイン同士の隔離ではないことに注意。
- **CORS 前提。** `ctx.fetch` は webview の `fetch` なので api.github.com 側の CORS 応答に依存する。
  現状 api.github.com は `Access-Control-Allow-Origin: *` を返し、プリフライトの
  `Access-Control-Allow-Headers` に `Authorization` / `If-None-Match` / `X-GitHub-Api-Version` を、
  `Access-Control-Expose-Headers` に `ETag` / `X-RateLimit-*` / `Retry-After` を含むので通る。
  GitHub がこれを変えたら Rust 側にプロキシ command を足す必要がある。
- GitHub Enterprise Server(自前ホスト)には未対応。API ベース URL は `api.github.com` 固定。
- コメントは 1 ページ(100 件)しか読まない。1 回のポーリング間隔に 100 件を超える更新があると
  次回に回る(`since` が進むので取りこぼしはしない)。
- questloom が起動している間だけ動く。閉じている間の変化は次の起動時にまとめて拾う。
- description の自動記入は **1 タスクにつき 1 回きり**。PR が更新されても追随しないし、
  ユーザーが description を消しても埋め直さない(上書き事故を避けるための割り切り)。
  PAT 無しの認証なしリクエストは GitHub のレート制限が厳しい(IP あたり 60 req/h)。
- 受信箱の取り込みは**検索 API の粒度までしか分からない**。
  - メンションは「自分がメンションされている open な issue/PR が更新された」で引くので、
    **昔メンションされたスレッドが動いただけでも当たる**。
    通知タスクの再利用(未完了なら追記)で騒がしさを抑えているだけで、絞り込みではない。
  - 誰にメンションされたかは分からない。検索結果から取れるのは issue/PR の**作成者**までで、
    コメント内のメンションだと投稿者は別人でありうる。
  - 検索は 1 ページ(50 件)で打ち切る。レビュー依頼はこれを超えると 51 件目以降が
    ずっと取り込まれない(メンションは `updated:>` の起点が進むので次回に回る)。
  - **検索 API は ETag を返さない**ので毎ラウンド必ず 2 リクエスト消費する。
  - `inbox:` の状態は最後の観測から 30 日で捨てる。それより長く放置された
    メンションが再度動くと、もう一度通知される。

## シークレットの保存先(資格情報マネージャー)

**シークレットは DB に置かない。** 実体は OS の資格情報ストア(Windows なら資格情報
マネージャー)で、[keyring](https://docs.rs/keyring) crate 越しに読み書きする。
`settings` テーブルには値も痕跡も入らない。

| 項目 | エントリ名 |
|---|---|
| 内蔵 MCP サーバーの Bearer トークン | `core/mcp-token` |
| プラグインの `type: "secret"` 項目 | `plugin:<id>/<key>` |

- service 名は **`questloom`**。テスト・検証で本物のエントリを汚さないよう、
  `QUESTLOOM_KEYRING_SERVICE` で丸ごと差し替えられる(下記「テスト用の環境変数」)。
- 実装は `apps/desktop/src-tauri/src/secrets.rs`。`SecretStore` trait +
  keyring 実装 (`KeyringSecretStore`) + テスト用インメモリ実装 (`MemorySecretStore`)。
  `questloom-core` は OS 非依存を保つため、この層はシェル(src-tauri)にだけ置く。
- エントリ名は `SecretKey::mcp_token()` / `SecretKey::plugin(id, key)` からしか作れない。
  区画に使えるのは英数字・`-`・`_`・`.` のみで、webview から渡された id / key は
  境界で必ず検査する。
- **平文フォールバックはしない。** 資格情報ストアが使えない環境では読み書きとも
  エラーにして UI に出す(黙って DB へ落とさない)。
- 一度書いた値は **UI からは読み出せない**。設定画面が見るのは「設定済み / 未設定」だけで、
  値を読む command (`plugin_secret_get`) は plugin-host ウィンドウにしか配っていない。

### 旧バージョンからの移送(1 回限り)

平文で `settings` に残っている値は、次のタイミングで自動的に資格情報ストアへ移り、
JSON からは消える。移送に失敗した場合は error ログを出して**平文をそのまま残し**、
次回にやり直す(認証が黙って外れる方が危ないため)。

| 対象 | いつ | どこ |
|---|---|---|
| `mcpToken` | 起動時(`AppState::initialize`) | `state.rs::adopt_mcp_token` |
| プラグインの secret 項目 | ホストがロード結果を公開したとき (`plugin_publish_loaded`) | `plugin_host.rs::migrate_plugin_secrets` |

プラグイン側を **manifest 駆動**にしてあるのは、どのキーがシークレットかは
`settingsSchema` を持つホストからしか分からないため。`plugin:github` の `pat` だけを
ハードコードすると同梱の例以外のプラグインが救われないので、公開された manifest を
見て移送する。書き込み自体は Rust 側で完結する(値の出どころも DB)ので、
plugin-host に設定の書き込み権限を渡す必要はない。

`CoreSettings.mcp_token` は `legacy_mcp_token` に改名し、`#[serde(rename = "mcpToken",
skip_serializing)]` を付けてある。**読めるが二度と書き戻さない**移行専用の受け皿で、
新しいコードがここを読むのは移送処理だけ。

## 内蔵ブラウザペイン

関連リソースの URL を、外部ブラウザではなく **main ウィンドウの中**で開ける
(`apps/desktop/src-tauri/src/browser.rs` + `apps/desktop/src/components/BrowserPane.tsx`)。

- 実体は **main ウィンドウの子 webview**(ラベル `browser-pane`)。Tauri v2 の
  マルチ webview(`tauri` の `unstable` feature + `Window::add_child`)で重ねる。
  ウィンドウは増やさない。`tauri.conf.json` にも定義しない(URL が実行時にしか決まらない)。
- React が描くのは**枠だけ**。ヘッダ(URL / ↗ 外部で開く / ✕ 閉じる)と空の箱を出し、
  箱の実寸(`getBoundingClientRect`)を論理ピクセルで Rust へ送って webview を重ねる。
  ResizeObserver + `resize` で追従する。
- 子 webview は**ネイティブの子ウィンドウ**で、HTML の z-index に従わず常に前面に描かれる。
  重なる UI は開いている間だけペインを隠す(`browser_pane_set_visible`)。
  隠す側は `ModalShell`(= 全モーダル)・`TaskContextMenu`・設定画面で、
  数は store の `paneOccluders` が数える。**ドロワーだけは隠さない**
  (`internalAuto` の「詳細を見ながらページを見る」を殺さないため)。代わりに
  CSS 変数 `--browser-pane-width` からドロワー幅の上限を作り、重ならないようにしている。

| command | 呼べる webview | 内容 |
|---|---|---|
| `browser_pane_open(url, bounds?)` (`async`) | main | 生成 or URL 差し替え。`bounds` はフロントが測った矩形 |
| `browser_pane_close()` (`async`) | main | 子 webview を破棄(冪等) |
| `browser_pane_set_bounds(bounds)` (`async`) | main | 矩形を更新(論理 px。`{x, y, width, height}`) |
| `browser_pane_set_visible(visible)` (`async`) | main | 表示・非表示(閉じないのでページの状態は残る) |
| `browser_pane_escape()` | **browser-pane のみ** | ペイン内の Esc を main へ中継(下記) |

`browser_pane_open` が **`async` なのは必須**。`Window::add_child` は Windows で
メインスレッドから呼ぶとデッドロックする(WebView2 の既知の問題)。

### ペインの中で押された Esc

子 webview は独立した WebView2 なので、そこにフォーカスがある間のキー入力は
main の `document` に届かない(= ドロワーが Esc で閉じない)。そこで:

1. ペイン生成時に `WebviewBuilder::initialization_script` で小さな JS
   (`browser.rs` の `ESCAPE_SCRIPT`)を注入する。capture フェーズで `keydown` を見て、
   Escape なら `browser_pane_escape` を invoke する。
   **`preventDefault` はしない**(ページ自身の Esc 処理を妨げない)。
2. `browser_pane_escape` は `questloom://browser-pane-escape` を main へ emit するだけ。
   連打は Rust 側(`EscapeThrottle`、**200ms に 1 回**)で間引く。JS 側にガードを置いても
   悪意あるページは直接 `invoke` を呼べるので防御にならない。
3. main では `BrowserPane` がこれを購読し、`keyboard.ts` の `dispatchEscape()` で
   **キーボードの Esc とまったく同じレイヤースタック**へ流す。最前面のレイヤー
   (モーダル > ドロワー)が閉じ、レイヤーが 1 つも無ければ(= ペインだけ)`closePane()`。

注入スクリプトが動くのは **main frame だけ**(Tauri が `__TAURI_INTERNALS__` を
子フレームへ注入しないため)。ページ内 iframe にフォーカスがある間の Esc は届かない。

### セキュリティ(外部ページに Esc 以外を渡さない)

守りは 3 枚。どれか 1 枚が破れても `browser_pane_escape` より先へは届かない。

1. **capability は webview ラベルで配る。** Tauri の `resolve_access` は
   「webview ラベルが一致 **または** ウィンドウラベルが一致」で通すので、
   `"windows": ["main"]` のままだと **main ウィンドウの子 webview(外部ページ)に
   main の全権限が渡ってしまう**。そのため capabilities は 4 つとも
   `"webviews": [...]` で書き、`browser-pane` を載せるのは
   `capabilities/browser-pane.json` **1 枚だけ**にする。
2. **リモート生成元へ開けるのは 1 command だけ。** 外部 URL からの invoke は `Origin::Remote`
   になり、`remote` 節を持たない capability とは照合されない。`remote` 節を持つのは
   `capabilities/browser-pane.json` だけで、その `permissions` は
   `allow-browser-pane-escape` **のみ**。ここに 2 つ目を足さないこと(利用者がペインで
   開いた任意のページが呼べるようになる)。
3. **URL を絞る。** `http` / `https` のみ。加えて `*.localhost` を弾く
   (Windows では questloom 自身が `http://tauri.localhost` から配信されるため、
   そこを開くと Tauri から「ローカル生成元」に見えて 1 と 2 をまとめて回避されうる)。

`dangerousRemoteDomainIpcAccess` は使わない。1〜3 を一括で無効化する設定なので持たない。
1 と 2 は `src-tauri/src/lib.rs` の `capabilities_are_granted_by_webview_label` /
`the_browser_pane_capability_grants_only_the_escape_command` /
`the_browser_pane_remote_urls_cover_http_and_https_only`
(`capabilities/` を実際に走査する)、3 は `browser::tests` が見る。

`remote.urls` は `["http://*:*", "https://*:*"]`。**`:*` を落とさないこと。**
URLPattern はポート成分を書かないと「スキームの既定ポートだけ」に絞られるので、
`https://*` と書くと `https://host:8443/` のページでだけ Esc が効かなくなる。

### 開き方の設定

`CoreSettings.urlOpenMode`(既定 `external`)で、**URL リソースをクリックしたとき**の
挙動を選ぶ。設定画面「一般」節のラジオ 3 択。

| 値 | 挙動 |
|---|---|
| `external` | OS の既定ブラウザで開く(従来どおり) |
| `internal` | 内蔵ブラウザペインで開く |
| `internalAuto` | `internal` に加えて、**タスク詳細を開いたとき主リソースが URL なら自動でペインに出す** |

明示的な操作は設定に関わらず両方使える。右クリックメニューの
「🔗 URL を開く」(外部)と「🌐 内蔵ブラウザで開く」(`context-url-internal`)、
ドロワーのリソース行の ↗ / 🌐 ボタン。**オーバーレイは常に外部ブラウザ**(変更なし)。

## 設定画面

ヘッダ右端の歯車ボタンから、ボードを置き換えるページとして開く(`apps/desktop/src/components/SettingsPage.tsx`。
Esc / 閉じるでボードへ戻る)。節は 一般 / ショートカットとオーバーレイ / MCP サーバー / AI / プラグイン の 5 つ。
プラグイン節(`components/PluginSettingsSection.tsx`)だけはコア設定と独立で、
manifest の `settingsSchema` からフォームを自動生成し、プラグインごとの保存ボタンで
`plugin_set_settings` を呼ぶ(下記の一括「保存」は使わない)。カードには出所の
「標準」/「ユーザー」バッジが付き、ユーザー版が標準版を隠している場合は
「標準版(アプリ同梱)を上書きしています」という注記が出る。
自動保存はせず「保存」ボタンで `set_settings` を一括呼び出しする(未保存のまま閉じるときは確認)。

**シークレットはどちらの「保存」にも載らない**(値は資格情報マネージャーにあり、
`CoreSettings` にも `plugin:<id>` の JSON にも入らない)。画面の形は共通で、
「設定済み / 未設定」のインジケータ + 新しい値の入力欄 + クリアの 3 点セット。
既存の値は読み出せないので、入力欄が空なら「変更しない」の意味になる。

- MCP のトークン(`components/McpSection.tsx`)は「設定 / 変更 / クリア」の操作で
  即座に `set_mcp_token` を呼ぶ(下の一括保存とは無関係)。成功すると MCP サーバーが
  張り直されるので、稼働状態も取り直す。Claude Code への登録コマンド例には値を
  差し込めないので、`--header` の形だけを見せる。
- プラグインの secret 項目はカードの「保存」で `plugin_secret_set` を呼ぶ
  (非シークレットの `plugin_set_settings` とは別の呼び出し)。

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

### ウィンドウの生成は setup フックの中で行う

`tauri.conf.json` の 3 つのウィンドウ定義は**すべて `"create": false`** にしてあり、生成は
`src-tauri/src/lib.rs` の `create_windows`(setup フック内、`app.manage` の後)が
`WebviewWindowBuilder::from_config` で行う。

Tauri v2 は「conf 定義のウィンドウを生成 → ユーザーの setup フック」の順に動くため、
`create: true`(既定)のままだと webview の最初の `invoke` が `app.manage(AppState)` を
追い越し、`state not managed for field 'state' on command 'get_board'` になる。
dev サーバー経由ではフロントのロードが遅くて表面化しないが、アセット同梱ビルド
(`npm run build; cargo run -p questloom-desktop --features tauri/custom-protocol`)では
毎回初回描画が失敗する。フロントは `tasks-changed` 駆動なので、次のイベントが来るまで
ボードが空のまま残る。

**ウィンドウの属性(サイズ・可視性・フォーカス等)は conf 側に置いたままにすること。**
Rust 側は定義をそのまま `from_config` に渡すだけで、capability の対応づけも
従来どおりラベルで決まる。`create: false` が保たれていることは
`src-tauri/src/lib.rs` の `windows_are_created_by_the_setup_hook` テストが見る。

ウィンドウはこの 3 つだけ。**4 枚目の webview として `browser-pane`**(内蔵ブラウザ)が
main ウィンドウの中に重なることがあるが、これはウィンドウではなく
`Window::add_child` による子 webview で、conf にも定義しない。
上記「内蔵ブラウザペイン」節を参照。

### メインウィンドウのすりガラス (windowEffects)

main ウィンドウは `tauri.conf.json` で `"transparent": true` +
`"windowEffects": { "effects": ["acrylic"] }` を持つ。効果は
`WebviewWindowBuilder::from_config` が build 時に適用するので、Rust 側の追加配線は要らない。
`window_vibrancy` の acrylic は Windows 11 build 22523 以降なら DWM の
`DWMWA_SYSTEMBACKDROP_TYPE`(ラグの無い新しい経路)、Windows 10 v1809〜Win11 21H2 では
旧 `SetWindowCompositionAttribute`(リサイズ/ドラッグが重い既知の問題)になる。
CSS 側は `styles.css` の `--app-tint` を body の地色に敷き、`prefers-reduced-transparency`
(Windows の「透明効果」オフ)では不透明色 `--bg` に落として従来のダークテーマへ戻す。

### webview 別の command 許可(ACL)

アプリ独自 command も **Tauri の ACL の対象にしている**。`build.rs` が
`tauri_build::AppManifest::commands` に `src-tauri/src/app_commands.rs` の `APP_COMMANDS`
(`tauri::generate_handler!` と 1 対 1)を渡し、`permissions/autogenerated/<command>.toml` に
`allow-<command>` / `deny-<command>` を生成する。これを capability で webview ごとに配る。

**割り当ては `"windows"` ではなく `"webviews"` で書くこと。** Tauri は
「webview ラベルが一致 または ウィンドウラベルが一致」で許可するので、`"windows": ["main"]`
だと main ウィンドウに重なる子 webview(= 内蔵ブラウザの外部ページ)へ権限が漏れる。

| webview | 許可 |
|---|---|
| `main` | `plugin_secret_get` 以外の全 command(ボード・ドロワー・設定画面・AI・プラグイン設定)。タスクの削除・復元 (`delete_task` / `restore_task` / `list_deleted_tasks`)、過去の完了 (`list_archived_done`)、チェックリスト (`add_checklist_item` / `update_checklist_item` / `remove_checklist_item` / `reorder_checklist_item`)、MCP トークン (`get_mcp_token_status` / `set_mcp_token`)、内蔵ブラウザ (`browser_pane_*`)、主リソースの付け替え (`set_primary_resource`) は**ここだけ** |
| `overlay` | `get_board` / `complete_task` / `show_main_window` のみ |
| `plugin-host` | `plugin_*`(設定書き込み `plugin_set_settings`、設定画面専用の `plugin_directory` / `plugin_list_loaded` / `plugin_secret_set` / `plugin_secret_status` を除く)+ シークレットの読み出し `plugin_secret_get` + `ctx.tasks` が使うタスク操作(`get_board` / `get_task` / `create_task` / `update_task` / `move_task` / `complete_task` / `add_task_update` / `add_resource`) |
| `browser-pane` | **`browser_pane_escape` だけ**(`capabilities/browser-pane.json`)。外部ページが載るので、ここに 2 つ目を足さないこと |

plugin-host では第三者のプラグインコードが動くので、`get_settings` / `set_settings` /
`get_runtime_status`(MCP の URL が載る)/ `get_mcp_token_status` / `set_mcp_token` /
`ai_*` / `browser_pane_*` / タスクの削除・復元 / `list_archived_done` /
チェックリストの `*_checklist_item` は**渡さない**。
チェックリストの command は origin を `User` に固定しているので、plugin-host に配ると
**プラグインが「利用者の操作」を騙れる**(= 監視中タスクを起こさずに中身を書き換えられる)。
プラグイン・AI がチェックリストを触る道は MCP のツール(origin は `Mcp`)。許可されていない
command を invoke すると Tauri が拒否する。command を足したら `APP_COMMANDS` と
`capabilities/default.json` の両方に足すこと(食い違いは `src-tauri/src/lib.rs` の
テストが検出する)。

シークレットだけは許可の向きが逆で、**値を読む `plugin_secret_get` は plugin-host にしか
渡さない**(プラグインコードは値が無いと動かない)。設定画面は書き込み
(`plugin_secret_set`)と状態確認(`plugin_secret_status`)だけを持ち、値は読めない。
main に配らない command は `lib.rs` の `NOT_FOR_MAIN`(`plugin_secret_get` と
`browser_pane_escape`)に列挙してある。

### CSP

`tauri.conf.json` の `app.security.csp` は `default-src 'self'` を土台に、次だけを緩めている。

- `script-src 'self' 'wasm-unsafe-eval' blob:` / `worker-src 'self' blob:` —
  プラグインのトランスパイルに使う **esbuild-wasm**(WASM のコンパイルと Web Worker)と、
  プラグインを Blob URL から `import()` するホストのため。どちらか欠けると
  プラグインが 1 件もロードできなくなる。
- `style-src 'self' 'unsafe-inline'` — dnd-kit がドラッグ中に付ける `style` 属性のため。
- `connect-src 'self' ipc: http://ipc.localhost *` — `ipc:` 系は Tauri の IPC、
  `*` は**プラグインの `ctx.fetch`**。許可ドメインは利用者がプラグインを置いた時点で決まるので
  ビルド時の CSP には書けない。fetch 先の制限は manifest の `fetchDomains` と
  Rust 側 `plugin_host::is_fetch_allowed` が担う。

CSP が効くのは**アセットを同梱したビルドだけ**(`npm run tauri dev` は Vite dev サーバーから
読むので CSP ヘッダが付かない)。CSP を変えたら
`npm run build; cargo run -p questloom-desktop --features tauri/custom-protocol` で確認すること。
- MCP・その他のリッスンは 127.0.0.1 のみにバインドする。
- **シークレット(MCP トークン・GitHub PAT 等)は DB に置かない。** 実体は Windows
  資格情報マネージャー(keyring crate、service `questloom`)で、`settings` テーブルには
  値も痕跡も入らない。詳細は「シークレットの保存先」節を参照。
- crate 間の依存は上記の依存方向を守る。特に `questloom-core` を汚染しないこと。
