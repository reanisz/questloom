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
- `questloom-desktop` (src-tauri) → `questloom-core`, `questloom-store`
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

## 注意事項

- **Tauri v2 を使用する。** v1 の API・設定ファイル形式・ネット上の記事を参照しないこと。
  v1 と v2 では `tauri.conf.json` の構造、権限モデル(capabilities / permissions)、
  プラグインのパッケージ名(`tauri-plugin-*` の v2 系)がすべて異なる。
  参照するのは https://v2.tauri.app のドキュメントのみとする。
- 権限は capabilities(`apps/desktop/src-tauri/capabilities/`)で最小限に構成する。
- MCP・その他のリッスンは 127.0.0.1 のみにバインドする。
- GitHub PAT 等のシークレットは DB に置かず、Windows 資格情報マネージャー(keyring crate)に保存する。
- crate 間の依存は上記の依存方向を守る。特に `questloom-core` を汚染しないこと。
