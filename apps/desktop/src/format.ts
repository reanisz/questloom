/** 日付・予定の表示整形。バックエンドは RFC3339 (UTC) を返すのでローカル時刻へ直して表示する。 */

import type { Scheduled, TaskStatus } from "./types";

const dateTimeFormat = new Intl.DateTimeFormat("ja-JP", {
  month: "numeric",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
});

const dateTimeWithYearFormat = new Intl.DateTimeFormat("ja-JP", {
  year: "numeric",
  month: "numeric",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
});

/** 締切バッジ用の短い表記(同じ年なら年を省略)。 */
export function formatDeadline(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const sameYear = date.getFullYear() === new Date().getFullYear();
  return (sameYear ? dateTimeFormat : dateTimeWithYearFormat).format(date);
}

/** 履歴などの絶対時刻表記。 */
export function formatTimestamp(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return dateTimeWithYearFormat.format(date);
}

/** 締切を過ぎているか。 */
export function isOverdue(iso: string | null): boolean {
  if (!iso) return false;
  const date = new Date(iso);
  return !Number.isNaN(date.getTime()) && date.getTime() < Date.now();
}

/** RFC3339 (UTC) を `<input type="datetime-local">` 用のローカル文字列へ変換する。 */
export function toDateTimeLocal(iso: string | null): string {
  if (!iso) return "";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}` +
    `T${pad(date.getHours())}:${pad(date.getMinutes())}`
  );
}

/** `<input type="datetime-local">` の値を RFC3339 (UTC) へ変換する。空なら null。 */
export function fromDateTimeLocal(value: string): string | null {
  if (!value) return null;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date.toISOString();
}

/** 予定の日本語表記。 */
export function formatScheduled(scheduled: Scheduled): string {
  switch (scheduled.kind) {
    case "date":
      return `${scheduled.value} に実施`;
    case "week":
      return `${scheduled.value} の週に実施`;
    default:
      return "予定なし";
  }
}

/**
 * チェックリストの進捗表記 (`2/5`)。
 *
 * カードのバッジとドロワーのヘッダで同じ表記を使うため、ここに置く。
 * バックエンドは常に `done <= total` を返すが、壊れた値でも表示だけは
 * 破綻させない(負や超過はそのまま出す)。
 */
export function formatChecklist(progress: { checklistDone: number; checklistTotal: number }): string {
  return `${progress.checklistDone}/${progress.checklistTotal}`;
}

/** チェックリストが全部埋まっているか。項目が 0 件なら偽。 */
export function isChecklistComplete(progress: {
  checklistDone: number;
  checklistTotal: number;
}): boolean {
  return progress.checklistTotal > 0 && progress.checklistDone >= progress.checklistTotal;
}

const STATUS_LABELS: Record<TaskStatus, string> = {
  new: "New",
  todo: "Todo",
  doing: "Doing",
  done: "Done",
  watching: "監視中",
  icebox: "Icebox",
};

/** 状態の表示ラベル。 */
export function formatStatus(status: TaskStatus): string {
  return STATUS_LABELS[status] ?? status;
}
