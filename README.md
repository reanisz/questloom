# questloom

タスク管理・通知管理のデスクトップアプリ(Tauri v2 + React + Rust)。

Trello 風のボード(New / Today / Tomorrow / ThisWeek / NextWeek / Future / Doing / Done)で
タスクを管理し、タスクトレイ常駐・グローバルショートカット(既定 Ctrl+Space)・
New タスクのオーバーレイ通知を備える。内蔵 MCP サーバー経由で Claude Code / Codex などの
AI エージェントからタスクを操作でき、AI CLI 呼び出しや TypeScript プラグイン
(第一弾は GitHub PR 監視)による自動タスク生成に対応する。

## 動作環境・前提

- Windows 10/11(WebView2 ランタイム。Windows 11 は同梱済み)
- ビルドする場合はさらに:
  - [Rust](https://rustup.rs/) stable(`x86_64-pc-windows-msvc`)
  - Node.js(LTS 以降)+ npm
  - Visual Studio の「**C++ によるデスクトップ開発**」ワークロード
    (MSVC v143 + Windows SDK。無いとリンクエラーでビルドできない)

## ビルドとインストール

```powershell
git clone https://github.com/reanisz/questloom.git
cd questloom\apps\desktop
npm install
npm run tauri build
```

初回ビルドは 10 分前後かかる。成果物は 2 つ:

| 用途 | パス |
|---|---|
| インストーラ (NSIS) | `target\release\bundle\nsis\questloom-desktop_<version>_x64-setup.exe` |
| ポータブル実行ファイル | `target\release\questloom-desktop.exe` |

インストーラを実行するとスタートメニューに登録される。試すだけならポータブル exe の
直接起動でもよい(どちらもデータは共通の場所を使うので、後から切り替えても引き継がれる)。

### 開発起動(ビルドせずに試す)

```powershell
cd apps\desktop
npm install
npm run tauri dev
```

## 使い方のポイント

- **閉じるボタンはタスクトレイへの格納**。終了はトレイアイコン右クリック →「終了」。
- **Ctrl+Space** でどこからでもボードをトグル(設定で変更可)。
- New タスクがある間は画面左上に透過オーバーレイが出る。インスタントタスク(⚡)は
  そこからワンクリックで完了/主 URL を開ける。
- データは `%APPDATA%\dev.reanisz.questloom\data.db`(SQLite)。起動ごとに `backups\` へ
  バックアップが取られる(既定 14 世代)。
- 設定はヘッダ右端の歯車から(週の開始曜日・ショートカット・自動起動・MCP・AI プロバイダ等)。

### AI エージェントからの操作(MCP)

アプリ起動中は `http://127.0.0.1:39150/mcp` で MCP サーバーが待ち受ける。Claude Code なら:

```powershell
claude mcp add --transport http questloom http://127.0.0.1:39150/mcp
```

### プラグイン

`%APPDATA%\dev.reanisz.questloom\plugins\` に `.ts` / `.js` を置くと読み込まれる。
サンプルは [examples/plugins/](examples/plugins/)(`hello.ts`、GitHub PR 監視の `github.ts`)。
導入手順・設定は [CLAUDE.md](CLAUDE.md) の「GitHub プラグイン」節を参照。

## ライセンス

[MIT License](LICENSE)

## ドキュメント

- [アーキテクチャ設計](docs/architecture.md)
- [データモデル設計](docs/data-model.md)
- [実装ロードマップ](docs/roadmap.md)
- [テスト戦略](docs/testing.md)
- [開発ガイド (CLAUDE.md)](CLAUDE.md) — ビルド/テストコマンドの一覧、設計上の注意
