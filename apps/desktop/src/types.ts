/**
 * バックエンド (questloom-core) の JSON 契約に対応する型定義。
 *
 * 契約テスト `crates/questloom-core/src/service.rs::board_and_detail_json_shape` が
 * この形を固定している。JSON は全面 camelCase、enum も camelCase。
 */

// プラグインの manifest 型は SDK(プラグイン作者向けリファレンス)を唯一の定義とする。
import type { PluginManifest } from "./plugin-host/sdk";

export type { PluginManifest, PluginSettingField, PluginSettingType } from "./plugin-host/sdk";

/** タスクの識別子 (UUID v7)。 */
export type TaskId = string;
/** 関連リソースの識別子 (UUID v7)。 */
export type ResourceId = string;
/** アップデート履歴の識別子 (UUID v7)。 */
export type UpdateId = string;

/**
 * タスクの状態。
 *
 * `watching` は「外部の変化待ち」。ユーザー以外の origin(mcp / ai / plugin:*)による
 * 履歴追記・子タスク作成を受けると、バックエンドが自動的に `new` へ戻す(起床)。
 */
export type TaskStatus = "new" | "todo" | "doing" | "done" | "watching";

/** Todo タスクの表示バケット(導出値)。 */
export type Bucket = "today" | "tomorrow" | "thisWeek" | "nextWeek" | "future";

/** ボードの列。ドラッグ&ドロップ先の指定に用いる。 */
export type BoardColumnKey =
  | "new"
  | "today"
  | "tomorrow"
  | "thisWeek"
  | "nextWeek"
  | "future"
  | "watching"
  | "doing"
  | "done";

/** 発生元。`plugin:<id>` もありうるため string を許す。 */
export type Origin = "user" | "mcp" | "ai" | "system" | (string & {});

/** 関連リソースの種別。 */
export type ResourceKind = "url" | "file";

/** タスクの予定。`{"kind":"date","value":"2026-08-31"}` などの外部タグ表現。 */
export type Scheduled =
  | { kind: "date"; value: string }
  | { kind: "week"; value: string }
  | { kind: "none" };

/** 週の開始曜日。 */
export type WeekStart = "monday" | "sunday";

/**
 * URL の関連リソースをクリックしたときの既定の開き方。
 *
 * - `external` — OS の既定ブラウザ(既定)
 * - `internal` — アプリ内蔵のブラウザペイン
 * - `internalAuto` — 内蔵ペイン + タスク詳細を開いたら主リソースを自動表示
 *
 * 「内蔵で開く」「外部で開く」の明示的な操作はモードに関わらず常に使える。
 */
export type UrlOpenMode = "external" | "internal" | "internalAuto";

/** タスク本体。 */
export interface Task {
  id: TaskId;
  title: string;
  description: string;
  status: TaskStatus;
  scheduled: Scheduled;
  /** 締切 (RFC3339 / UTC)。 */
  deadline: string | null;
  isInstant: boolean;
  origin: Origin;
  parentId: TaskId | null;
  sortOrder: string;
  createdAt: string;
  updatedAt: string;
  doneAt: string | null;
  /** ソフトデリート時刻 (RFC3339 / UTC)。null なら生存している。 */
  deletedAt: string | null;
}

/** タスクの関連リソース。 */
export interface TaskResource {
  id: ResourceId;
  taskId: TaskId;
  kind: ResourceKind;
  value: string;
  label: string;
  isPrimary: boolean;
  sortOrder: string;
  createdAt: string;
}

/** 状態アップデートのヒストリー 1 件。 */
export interface TaskUpdateEntry {
  id: UpdateId;
  taskId: TaskId;
  body: string;
  origin: Origin;
  createdAt: string;
}

/** ボードのカード。Task を平坦化して集計値を足したもの。 */
export interface TaskCard extends Task {
  /** Todo 以外は null。 */
  bucket: Bucket | null;
  childCount: number;
  resourceCount: number;
  primaryResource: TaskResource | null;
}

/** 列ごとのカード配列。各列は sortOrder 昇順。 */
export type BoardColumns = Record<BoardColumnKey, TaskCard[]>;

/** ボード全体。 */
export interface Board {
  /** サービスが認識している今日の日付 (YYYY-MM-DD)。 */
  today: string;
  weekStart: WeekStart;
  columns: BoardColumns;
}

/** タスク詳細。TaskCard を平坦化して関連情報を足したもの。 */
export interface TaskDetail extends TaskCard {
  resources: TaskResource[];
  /** 古い順。 */
  updates: TaskUpdateEntry[];
  parent: TaskCard | null;
  children: TaskCard[];
}

/** `create_task` の入力。`title` のみ必須。 */
export interface NewTask {
  title: string;
  description?: string;
  status?: TaskStatus;
  scheduled?: Scheduled;
  deadline?: string | null;
  isInstant?: boolean;
  origin?: Origin;
  parentId?: TaskId | null;
  resources?: NewResource[];
}

/** `update_task` の差分。締切のクリアは `clearDeadline: true`。 */
export interface TaskPatch {
  title?: string;
  description?: string;
  deadline?: string | null;
  clearDeadline?: boolean;
  scheduled?: Scheduled;
  isInstant?: boolean;
}

/** `move_task` の指定。両端が null なら列末尾。 */
export interface MoveRequest {
  column: BoardColumnKey;
  prevId?: TaskId | null;
  nextId?: TaskId | null;
}

/** `add_resource` の入力。 */
export interface NewResource {
  kind: ResourceKind;
  value: string;
  label?: string;
  isPrimary?: boolean;
}

/** AI CLI プロバイダの定義。 */
export interface AiProvider {
  /** 識別子(設定内で一意)。 */
  id: string;
  /** UI に出す表示名。 */
  label: string;
  /** 実行ファイル名(PATH から解決される)。 */
  command: string;
  /** 引数テンプレート。`{prompt}` が置換される。 */
  args: string[];
  enabled: boolean;
  /** MCP 接続時に前置される引数。空なら MCP 非対応。 */
  mcpArgs: string[];
  /** MCP トークン(認証ヘッダ)を渡せるか。 */
  mcpSupportsToken: boolean;
}

/** AI 機能の種別。 */
export type AiFeature = "createTasks" | "splitTask" | "freeInstruction";

/** AI 実行の状態。 */
export type AiState = "running" | "done" | "error";

/** `questloom://ai-status` のペイロード。 */
export interface AiStatus {
  state: AiState;
  feature: AiFeature;
  message: string | null;
}

/** AI が作成したタスク 1 件。 */
export interface AiTaskSummary {
  id: TaskId;
  title: string;
  description: string;
}

/** タスクを作る系 (`ai_create_tasks` / `ai_split_task`) の結果。 */
export interface AiCreateResult {
  providerId: string;
  created: AiTaskSummary[];
}

/** 自由指示 (`ai_free_instruction`) の結果。 */
export interface AiTextResult {
  providerId: string;
  text: string;
  /** 内蔵 MCP サーバーへ接続させられたか。 */
  mcpAttached: boolean;
}

/** コア設定。 */
export interface CoreSettings {
  weekStart: WeekStart;
  backupGenerations: number;
  /** New タスクがあるときにオーバーレイ通知を出すか。 */
  overlayEnabled: boolean;
  /** メインウィンドウをトグルするグローバルショートカット(例: "Ctrl+Space")。 */
  globalShortcut: string;
  /** OS ログイン時に自動起動するか。 */
  autostart: boolean;
  /** URL の関連リソースをクリックしたときの既定の開き方。 */
  urlOpenMode: UrlOpenMode;
  /** 内蔵 MCP サーバーを起動するか。 */
  mcpEnabled: boolean;
  /** MCP サーバーの待受ポート(バインドは 127.0.0.1 のみ)。 */
  mcpPort: number;
  /*
   * MCP サーバーの Bearer トークンはここには**無い**。実体は OS の資格情報ストア
   * (Windows の資格情報マネージャー)にあり、`get_mcp_token_status` /
   * `set_mcp_token` で扱う。値の読み出し経路はフロントには存在しない。
   */
  /** AI CLI プロバイダの一覧。 */
  aiProviders: AiProvider[];
  /** 既定で使うプロバイダの id。 */
  aiDefaultProviderId: string;
  /** AI CLI 実行のタイムアウト(秒)。 */
  aiTimeoutSecs: number;
}

/** デスクトップ側の稼働状態 (`get_runtime_status`)。設定画面での確認用。 */
export interface RuntimeStatus {
  /** 内蔵 MCP サーバーが起動しているか。 */
  mcpRunning: boolean;
  /** 起動中の MCP エンドポイント URL(停止中は null)。 */
  mcpUrl: string | null;
  /** 起動中の MCP サーバーが Bearer トークンを要求するか。 */
  mcpTokenRequired: boolean;
  /** 設定中のグローバルショートカットを実際に登録できているか。 */
  shortcutRegistered: boolean;
}

/**
 * `plugin_list_sources` が返すプラグインソース 1 件。
 * `fileName` はプラグインディレクトリ直下のファイル名(区切り文字を含むものは Rust 側が弾く)。
 */
export interface PluginSourceFile {
  fileName: string;
  source: string;
  /** 最終更新時刻 (RFC3339 / UTC)。取得できなければ null。 */
  modifiedAt: string | null;
}

/**
 * plugin-host が公開するロード結果 1 件(`plugin_list_loaded` /
 * `questloom://plugins-loaded`)。設定画面のプラグイン一覧はこれを描画する。
 */
export interface LoadedPlugin {
  fileName: string;
  /** 読み取れた manifest。ロードに失敗した場合は null。 */
  manifest: PluginManifest | null;
  /** `activate` まで成功したか。 */
  active: boolean;
  /** 失敗した場合のメッセージ。 */
  error: string | null;
}

/** 昇格先として選べる列(New / Doing / Done は選ばせない)。 */
export const PROMOTE_COLUMNS = [
  "today",
  "tomorrow",
  "thisWeek",
  "nextWeek",
  "future",
] as const satisfies readonly BoardColumnKey[];

/** 表示順に並べた全列と日本語ラベル。 */
export const BOARD_COLUMNS: readonly { key: BoardColumnKey; label: string }[] = [
  { key: "new", label: "New" },
  { key: "today", label: "Today" },
  { key: "tomorrow", label: "Tomorrow" },
  { key: "thisWeek", label: "This Week" },
  { key: "nextWeek", label: "Next Week" },
  { key: "future", label: "Future" },
  { key: "watching", label: "監視中" },
  { key: "doing", label: "Doing" },
  { key: "done", label: "Done" },
];

/**
 * 通常表示で列として並べる列。
 * 先送りバケットはレールのドロップボックスへ追い出し、横スクロールなしで収まるようにする。
 */
export const PRIMARY_COLUMNS = [
  "new",
  "today",
  "doing",
  "done",
] as const satisfies readonly BoardColumnKey[];

/**
 * 先送りバケット。通常表示ではレールのコンパクトなドロップボックスとして表示する。
 *
 * `watching`(外部の変化待ち)は時間バケットではないが、「今すぐ気にしなくてよいことの
 * 置き場」という点は同じなのでレールに並べる。列としての展開は展開表示のときだけ。
 */
export const DEFER_COLUMNS = [
  "tomorrow",
  "thisWeek",
  "nextWeek",
  "future",
  "watching",
] as const satisfies readonly BoardColumnKey[];

/** 列ヘッダ・レールのラベルに添える控えめな記号。無い列は undefined。 */
const COLUMN_ICONS: Partial<Record<BoardColumnKey, string>> = {
  watching: "👁",
};

/** 列キーの記号(あれば)。Watching であることを一目で分かるようにするためだけのもの。 */
export function columnIcon(key: BoardColumnKey): string | undefined {
  return COLUMN_ICONS[key];
}

/** 通常表示で列として表示する列か。 */
export function isPrimaryColumn(key: BoardColumnKey): boolean {
  return (PRIMARY_COLUMNS as readonly BoardColumnKey[]).includes(key);
}

/** 列キーの日本語ラベルを引く。 */
export function columnLabel(key: BoardColumnKey): string {
  return BOARD_COLUMNS.find((column) => column.key === key)?.label ?? key;
}
