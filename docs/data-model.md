# questloom データモデル設計

## 方針: データを失わないための工夫

1. SQLite + WAL モード(`journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`)。
2. すべての書き込みはトランザクション内で行う。
3. スキーマは `schema_version` によるバージョン付きマイグレーション(前進のみ)。
4. バックアップ: 起動時 + 日次で SQLite Online Backup API により
   `backups/questloom-YYYYMMDD-HHMMSS.db` へコピー。世代数は設定可能(既定 14)。
5. データ配置: `%APPDATA%\questloom\`(`data.db`, `backups/`, `logs/`)。

## ドメインモデル

### タスクの状態とバケットの考え方

タスクの状態 (`status`) は 4 つ: `new` / `todo` / `doing` / `done`。

Todo 内の時間バケット (Today/Tomorrow/ThisWeek/NextWeek/Future) は **DB に保存せず、
`scheduled_*` カラムから表示時に導出する**。これにより「日付や週が変わったら自動的に正しい
リストへ移動」はデータ更新なしで常に成立し、アプリが停止していた期間のロールオーバー処理が不要になる。

`todo` タスクは予定 (`scheduled_kind`) を 1 つ持つ:

| scheduled_kind | scheduled_value | 意味 |
|---|---|---|
| `date` | ISO 日付 (YYYY-MM-DD) | この日にやる |
| `week` | ISO 週 (YYYY-Www) | この週にやる |
| `none` | NULL | いつかやる (Future) |

バケット導出規則(`today` は現在日、週の開始曜日は設定値):

- `date(d)`: `d <= today` → **Today**(過ぎた予定は今日に繰り上がる) / `d == today+1` → **Tomorrow**
  / d が今週内 → **ThisWeek** / d が来週内 → **NextWeek** / それ以降 → **Future**
- `week(w)`: `w < 今週` → **Today** / `w == 今週` → **ThisWeek** / `w == 来週` → **NextWeek** / それ以降 → **Future**
- `none` → **Future**

ドラッグ操作とのマッピング: Today 列へ → `date(today)`、Tomorrow 列へ → `date(today+1)`、
ThisWeek 列へ → `week(今週)`、NextWeek 列へ → `week(来週)`、Future 列へ → `none`。

## テーブル定義

```sql
CREATE TABLE tasks (
    id              TEXT PRIMARY KEY,          -- UUID v7(時系列ソート可能)
    title           TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    status          TEXT NOT NULL,             -- 'new' | 'todo' | 'doing' | 'done'
    scheduled_kind  TEXT,                      -- 'date' | 'week' | NULL(status='todo' のときのみ意味を持つ)
    scheduled_value TEXT,                      -- 'YYYY-MM-DD' | 'YYYY-Www' | NULL
    deadline        TEXT,                      -- RFC3339。予定(scheduled)とは独立した締切
    is_instant      INTEGER NOT NULL DEFAULT 0,
    origin          TEXT NOT NULL DEFAULT 'user', -- 'user' | 'mcp' | 'ai' | 'plugin:<id>'
    parent_id       TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    sort_order      TEXT NOT NULL,             -- 同一リスト内の並び順(辞書順の fractional key)
    created_at      TEXT NOT NULL,             -- RFC3339 (UTC)
    updated_at      TEXT NOT NULL,
    done_at         TEXT
);
CREATE INDEX idx_tasks_status ON tasks(status);
CREATE INDEX idx_tasks_parent ON tasks(parent_id);

CREATE TABLE task_resources (
    id          TEXT PRIMARY KEY,              -- UUID v7
    task_id     TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,                 -- 'url' | 'file'
    value       TEXT NOT NULL,                 -- URL またはファイルパス
    label       TEXT NOT NULL DEFAULT '',
    is_primary  INTEGER NOT NULL DEFAULT 0,    -- 主リソース(オーバーレイのワンクリック起動対象)
    sort_order  TEXT NOT NULL,
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_resources_task ON task_resources(task_id);

-- 状態アップデートのヒストリー(ユーザー/AI が書く進捗メモ + システム記録)
CREATE TABLE task_updates (
    id          TEXT PRIMARY KEY,              -- UUID v7
    task_id     TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    body        TEXT NOT NULL,                 -- Markdown
    origin      TEXT NOT NULL DEFAULT 'user',  -- 'user' | 'mcp' | 'ai' | 'plugin:<id>' | 'system'
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_updates_task ON task_updates(task_id);

-- 名前空間付き設定(コア: 'core'、プラグイン: 'plugin:<id>')
CREATE TABLE settings (
    namespace   TEXT PRIMARY KEY,
    value       TEXT NOT NULL,                 -- JSON
    updated_at  TEXT NOT NULL
);

-- プラグイン用 KV ストレージ(GitHub プラグインのポーリング状態など)
CREATE TABLE plugin_kv (
    plugin_id   TEXT NOT NULL,
    key         TEXT NOT NULL,
    value       TEXT NOT NULL,                 -- JSON
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (plugin_id, key)
);

CREATE TABLE schema_version (
    version     INTEGER NOT NULL
);
```

補足:

- `sort_order` は fractional indexing(例: "a0", "a0V", …)で、ドラッグ&ドロップの並び替えを
  1 行の UPDATE で済ませる。
- 親子リンクは `parent_id` の単純な 1:N。循環はサービス層で禁止する。
- インスタントタスクの「昇格」= `is_instant` を 0 にして `status`/`scheduled` を設定する操作。
- タスク削除はまず考えない(Done で十分)。実装する場合も履歴保全のためソフトデリートを検討する。
- シークレット(GitHub PAT 等)はこの DB には保存しない(keyring を使用)。
