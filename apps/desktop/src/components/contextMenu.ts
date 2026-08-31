/**
 * タスクカードの右クリックメニューの、DOM に依らない部分。
 *
 * 「どの項目を出すか」と「どこに置くか」は分岐が多く、実機で総当たりするのが面倒なので
 * 純関数として切り出して単体テストする ([`contextMenu.test.ts`])。
 * 描画・イベント処理は [`TaskContextMenu`](./TaskContextMenu.tsx) 側。
 */

import type { TaskCard } from "../types";

/** メニュー項目の識別子。DOM では `data-testid="context-<action>"` になる。 */
export type ContextMenuAction = "open" | "complete" | "promote" | "move" | "url" | "delete";

/** 出し分けの判断に要るカードの属性だけ。テストから作りやすくするために絞ってある。 */
export type ContextMenuTarget = Pick<TaskCard, "status" | "isInstant" | "primaryResource">;

/** メニューの座標(いずれも viewport 基準の px)。 */
export interface Point {
  x: number;
  y: number;
}

/** 矩形の大きさ (px)。 */
export interface Size {
  width: number;
  height: number;
}

/** 画面端との最小の隙間 (px)。 */
export const MENU_MARGIN = 8;

/**
 * カードの状態から、出す項目を**表示順**で返す。
 *
 * - 完了済みのタスクに「完了にする」は出さない。
 * - 「昇格」はインスタントタスクだけ(通常タスクは既に列を持っている)。
 * - 「URL を開く」は主リソースが URL のときだけ(ファイルはドロワーから開く)。
 */
export function contextMenuActions(card: ContextMenuTarget): ContextMenuAction[] {
  const actions: ContextMenuAction[] = ["open"];
  if (card.status !== "done") actions.push("complete");
  if (card.isInstant) actions.push("promote");
  actions.push("move");
  if (card.primaryResource?.kind === "url") actions.push("url");
  actions.push("delete");
  return actions;
}

/**
 * 1 軸ぶんの配置。カーソルの手前(右 / 下)を基本とし、はみ出すなら反対側へ折り返す。
 * 折り返しても入らない(メニューが画面より大きい)場合だけ、端に寄せて溢れさせる。
 */
function place(anchor: number, size: number, viewport: number, margin: number): number {
  if (anchor + size + margin <= viewport) return Math.max(margin, anchor);
  if (anchor - size >= margin) return anchor - size;
  return Math.max(margin, viewport - size - margin);
}

/**
 * カーソル位置 `anchor` に置いたメニューが画面からはみ出さないよう補正した左上座標を返す。
 *
 * 縦横は独立に決める(右端では左へ、下端では上へ、隅では両方)。
 */
export function clampMenuPosition(
  anchor: Point,
  size: Size,
  viewport: Size,
  margin: number = MENU_MARGIN,
): Point {
  return {
    x: place(anchor.x, size.width, viewport.width, margin),
    y: place(anchor.y, size.height, viewport.height, margin),
  };
}
