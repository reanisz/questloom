/**
 * ドラッグ&ドロップの着地点計算。
 *
 * dnd-kit の `onDragEnd` が渡してくる「掴んでいるカード」「重なっている相手」から、
 * バックエンドの `move_task` に渡す `column` / `prevId` / `nextId` を求める。
 * DOM も dnd-kit も触らない純関数にしてあるのは、ここが**ボードで一番間違えやすい
 * 計算**(前後どちらへ差し込むか、自分自身を除いた並びで数えているか)だから。
 *
 * 表示側の都合は [`BoardView`](./BoardView.tsx) に残し、ここは並びの計算だけを持つ。
 */

import { BOARD_COLUMNS, type Board, type BoardColumnKey, type TaskId } from "../types";
import { COLUMN_DROPPABLE_PREFIX } from "./Column";

/** 上下の判定に要る矩形。dnd-kit の rect のうち使う分だけ。 */
export interface DropRect {
  top: number;
  height: number;
}

/** `move_task` に渡す移動先。 */
export interface DropPosition {
  column: BoardColumnKey;
  /** 挿入位置の 1 つ前のカード。先頭なら null。 */
  prevId: TaskId | null;
  /** 挿入位置の 1 つ後のカード。末尾なら null。 */
  nextId: TaskId | null;
}

/** カードが属する列を探す。どの列にも無ければ null。 */
export function locate(columns: Board["columns"], taskId: TaskId): BoardColumnKey | null {
  for (const { key } of BOARD_COLUMNS) {
    if (columns[key].some((card) => card.id === taskId)) return key;
  }
  return null;
}

/** droppable の id が列を指しているなら、その列キー。カード id なら null。 */
function columnFromDroppableId(overId: string): BoardColumnKey | null {
  if (!overId.startsWith(COLUMN_DROPPABLE_PREFIX)) return null;
  const key = overId.slice(COLUMN_DROPPABLE_PREFIX.length);
  return BOARD_COLUMNS.some((column) => column.key === key) ? (key as BoardColumnKey) : null;
}

/**
 * ドロップ先の id から、着地する列を求める。
 *
 * 列そのもの (`column:<key>`) でも、その列のカード id でも同じ列を返す。
 * 「どの列に落ちるか」のハイライトはこれで決める(`useDroppable().isOver` は
 * 列そのものに重なっているときしか真にならず、カードの上では消えてしまう)。
 */
export function columnOf(columns: Board["columns"], overId: string): BoardColumnKey | null {
  return overId.startsWith(COLUMN_DROPPABLE_PREFIX)
    ? columnFromDroppableId(overId)
    : locate(columns, overId);
}

/**
 * ドロップ先の列と、その中での挿入位置(前後のカード id)を求める。
 *
 * 戻り値が `null` なら「移動しない」。次のいずれか。
 * - 掴んだカードがボードに無い / ドロップ先の列を特定できない
 * - 同じ列で前後関係が変わらない(= 何もしないのが正しい)
 *
 * @param columns ボードの現在の並び。
 * @param activeId 掴んでいるカードの id。
 * @param overId 重なっている相手の id。列そのものなら `column:` 接頭辞が付く。
 * @param dragged 掴んでいるカードの現在の矩形。取れなければ null(= 対象カードの前へ挿す)。
 * @param over 重なっている相手の矩形。列へ落とした場合は使われない。
 */
export function resolveDropPosition(
  columns: Board["columns"],
  activeId: TaskId,
  overId: string,
  dragged: DropRect | null,
  over: DropRect,
): DropPosition | null {
  const from = locate(columns, activeId);
  if (!from) return null;

  const onColumn = overId.startsWith(COLUMN_DROPPABLE_PREFIX);
  const to = columnOf(columns, overId);
  if (!to) return null;

  // 移動対象を除いた移動先の並び。ここへの挿入位置から前後 id を求める。
  const items = columns[to].map((card) => card.id).filter((id) => id !== activeId);

  let insertAt: number;
  if (onColumn || overId === activeId) {
    // 列の余白へ落とした(または自分自身に重なった)なら末尾へ。
    insertAt = items.length;
  } else {
    const overIndex = items.indexOf(overId);
    if (overIndex < 0) return null;
    // ドラッグ中の矩形の中心が対象カードの中心より下なら後ろへ挿入する。
    const below = dragged != null && dragged.top + dragged.height / 2 > over.top + over.height / 2;
    insertAt = overIndex + (below ? 1 : 0);
  }

  const prevId = items[insertAt - 1] ?? null;
  const nextId = items[insertAt] ?? null;

  // 同じ列で位置が変わらないなら何もしない。
  if (from === to) {
    const current = columns[from].map((card) => card.id);
    const at = current.indexOf(activeId);
    if ((current[at - 1] ?? null) === prevId && (current[at + 1] ?? null) === nextId) return null;
  }

  return { column: to, prevId, nextId };
}
