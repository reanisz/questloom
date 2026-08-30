/**
 * Tauri command の薄いラッパー。
 *
 * 引数は camelCase で渡す(Tauri v2 が snake_case へ変換する)。
 * エラーは日本語文字列で reject されるため、そのまま Error に包み直す。
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  Board,
  BoardColumnKey,
  CoreSettings,
  MoveRequest,
  NewResource,
  NewTask,
  Origin,
  ResourceId,
  Task,
  TaskDetail,
  TaskId,
  TaskPatch,
  TaskResource,
  TaskUpdateEntry,
} from "./types";

/** タスク関連の変更通知イベント名。ペイロードは見ずに再フェッチすればよい。 */
export const TASKS_CHANGED = "questloom://tasks-changed";

/** reject 値(多くは日本語文字列)を Error へ正規化する。 */
export function toMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw new Error(toMessage(error));
  }
}

/** ボード全体を、バケット導出済みの構造で取得する。 */
export const getBoard = () => call<Board>("get_board");

/** タスク詳細(リソース・履歴・親子込み)を取得する。 */
export const getTask = (taskId: TaskId) => call<TaskDetail>("get_task", { taskId });

/** タスクを作成する。`title` のみ必須。 */
export const createTask = (input: NewTask) => call<Task>("create_task", { input });

/** タスクの内容を更新する。締切のクリアは `clearDeadline: true`。 */
export const updateTask = (taskId: TaskId, patch: TaskPatch) =>
  call<Task>("update_task", { taskId, patch });

/** 状態・予定・並び順を変更する。 */
export const moveTask = (taskId: TaskId, request: MoveRequest) =>
  call<Task>("move_task", { taskId, request });

/** タスクを完了にする(冪等)。 */
export const completeTask = (taskId: TaskId) => call<Task>("complete_task", { taskId });

/** インスタントタスクを通常タスクへ昇格する。列省略時は Today。 */
export const promoteTask = (taskId: TaskId, column?: BoardColumnKey) =>
  call<Task>("promote_task", { taskId, column: column ?? null });

/** アップデート履歴を追記する。 */
export const addTaskUpdate = (taskId: TaskId, body: string, origin: Origin = "user") =>
  call<TaskUpdateEntry>("add_task_update", { taskId, body, origin });

/** 関連リソースを追加する。 */
export const addResource = (taskId: TaskId, resource: NewResource) =>
  call<TaskResource>("add_resource", { taskId, resource });

/** 関連リソースを削除する。 */
export const removeResource = (taskId: TaskId, resourceId: ResourceId) =>
  call<void>("remove_resource", { taskId, resourceId });

/** 親タスクを設定・解除する(循環は禁止)。 */
export const setParent = (taskId: TaskId, parentId: TaskId | null) =>
  call<Task>("set_parent", { taskId, parentId });

/** コア設定を取得する。 */
export const getSettings = () => call<CoreSettings>("get_settings");

/** コア設定を保存する。 */
export const setSettings = (settings: CoreSettings) => call<void>("set_settings", { settings });

/** タスク変更イベントを購読する。ペイロードは使わず再フェッチのトリガとしてのみ扱う。 */
export function listenTasksChanged(handler: () => void): Promise<UnlistenFn> {
  return listen(TASKS_CHANGED, () => handler());
}
