/**
 * 衝突判定のテスト。
 *
 * ここが直したい不具合の核心なので、報告された 2 つの症状をそのまま置いてある。
 * 1. **列の上半分(カードが並んでいる領域)にカーソルを置いても反応しない。**
 *    → その位置ではカードが勝ち、`over` がカード id になる。列のハイライトは
 *      BoardView が [`columnOf`](./dropPosition.ts) で親の列へ読み替える。
 * 2. **ハイライトがワンテンポ遅れる。**
 *    → 掴んでいる矩形ではなくカーソルで決まること(= まだ元の列に大きく重なって
 *      いても、カーソルが隣の列に入った時点で切り替わること)を固定する。
 */

import type { ClientRect, CollisionDetection } from "@dnd-kit/core";
import { describe, expect, it } from "vitest";

import { boardCollisionDetection } from "./collision";

type Args = Parameters<CollisionDetection>[0];

/** 左上と大きさから ClientRect を作る。 */
function rect(left: number, top: number, width: number, height: number): ClientRect {
  return { left, top, width, height, right: left + width, bottom: top + height };
}

/**
 * ボードを模した droppable の一式。
 *
 * 幅 200 の列が 2 つ(x=0..200 / x=220..420)。左の列には上半分にカード 2 枚が
 * 並び、下半分は余白。実際の DOM と同じく**カードの矩形は列の矩形に完全に含まれる**。
 */
const RECTS: Record<string, ClientRect> = {
  "column:new": rect(0, 0, 200, 600),
  "card-a": rect(10, 10, 180, 60),
  "card-b": rect(10, 80, 180, 60),
  "column:today": rect(220, 0, 200, 600),
};

/** 指定の座標にカーソルがあり、掴んでいる矩形が `dragged` にある状況を作る。 */
function detect(pointer: { x: number; y: number } | null, dragged: ClientRect) {
  const droppableRects = new Map(Object.entries(RECTS));
  const args = {
    active: { id: "dragged", data: { current: undefined }, rect: { current: {} } },
    collisionRect: dragged,
    droppableRects,
    droppableContainers: Object.keys(RECTS).map((id) => ({ id })),
    pointerCoordinates: pointer,
  } as unknown as Args;
  return boardCollisionDetection(args).map((collision) => String(collision.id));
}

describe("boardCollisionDetection", () => {
  it("カードの上ではカードが勝つ(列に完全に含まれていても負けない)", () => {
    // 列の上半分 = card-b の真ん中。
    const [first] = detect({ x: 100, y: 110 }, rect(60, 60, 180, 60));
    expect(first).toBe("card-b");
  });

  it("列の余白ではその列が勝つ", () => {
    const [first] = detect({ x: 100, y: 400 }, rect(60, 350, 180, 60));
    expect(first).toBe("column:new");
  });

  it("カーソルが隣の列に入った時点で切り替わる(掴んでいる矩形が元の列に残っていても)", () => {
    // 掴んでいる矩形の大半 (x=60..240) はまだ左の列。カーソルだけ右の列に入った状態。
    const dragged = rect(60, 300, 180, 60);
    const [first] = detect({ x: 230, y: 330 }, dragged);
    expect(first).toBe("column:today");

    // 参考: 同じ状況でも矩形で決める判定なら左の列のままになる(= ワンテンポ遅れる)。
    const [byRect] = detect(null, dragged);
    expect(byRect).toBe("column:new");
  });

  it("列の隙間ではポインタが載っていないので矩形の重なりで決める", () => {
    // x=210 は列と列の間。掴んでいる矩形は右の列に多くかかっている。
    const [first] = detect({ x: 210, y: 300 }, rect(140, 300, 180, 60));
    expect(first).toBe("column:today");
  });

  it("どこにも重なっていなくても必ず 1 つ返す(最寄りに落ちる)", () => {
    const found = detect({ x: 900, y: 900 }, rect(880, 880, 180, 60));
    expect(found.length).toBeGreaterThan(0);
    expect(found[0]).toBe("column:today");
  });
});
