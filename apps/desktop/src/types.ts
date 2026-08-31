/**
 * バックエンド (questloom-core) の JSON 契約に対応する型定義。
 *
 * 契約テスト `crates/questloom-core/src/service.rs::board_and_detail_json_shape` が
 * この形を固定している。JSON は全面 camelCase、enum も camelCase。
 */

/** タスクの識別子 (UUID v7)。 */
export type TaskId = string;
/** 関連リソースの識別子 (UUID v7)。 */
export type ResourceId = string;
/** アップデート履歴の識別子 (UUID v7)。 */
export type UpdateId = string;

/** タスクの状態。 */
export type TaskStatus = "new" | "todo" | "doing" | "done";

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

/** 先送りバケット。通常表示ではレールのコンパクトなドロップボックスとして表示する。 */
export const DEFER_COLUMNS = [
  "tomorrow",
  "thisWeek",
  "nextWeek",
  "future",
] as const satisfies readonly BoardColumnKey[];

/** 通常表示で列として表示する列か。 */
export function isPrimaryColumn(key: BoardColumnKey): boolean {
  return (PRIMARY_COLUMNS as readonly BoardColumnKey[]).includes(key);
}

/** 列キーの日本語ラベルを引く。 */
export function columnLabel(key: BoardColumnKey): string {
  return BOARD_COLUMNS.find((column) => column.key === key)?.label ?? key;
}
