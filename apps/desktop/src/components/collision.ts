/**
 * ボードの衝突判定(どの droppable に重なっているとみなすか)。
 *
 * dnd-kit の既定 (`rectIntersection`) も、これまで使っていた `closestCorners` も、
 * **掴んでいる矩形**と droppable の矩形を比べる。カード 1 枚分の幅を持つ矩形で
 * 判定するため、カーソルが隣の列に入ってもしばらく前の列が勝ち続け、
 * ハイライトがカーソルより遅れて付いてくる。
 *
 * そこでポインタを最優先にする。カーソルが載っている droppable がそのまま
 * ドロップ先になるので、判定は見た目と一致し、遅れもなくなる。
 *
 * [`BoardView`](./BoardView.tsx) から切り出してあるのは、DOM も React も要らない
 * 純粋な関数で、単体テストできるようにするため([`dropPosition`](./dropPosition.ts) と同じ方針)。
 */

import {
  closestCorners,
  pointerWithin,
  rectIntersection,
  type CollisionDetection,
} from "@dnd-kit/core";

/**
 * ボードの衝突判定。次の順に試し、最初に候補が出たものを採る。
 *
 * 1. **ポインタが載っている droppable。** 列とその中のカードは必ず重なるが、
 *    `pointerWithin` は矩形の四隅までの距離が近い順に返すため、小さいカードが
 *    大きい列より先に来る(= カードの上ならカードが、列の余白なら列が勝つ)。
 * 2. ポインタがどの droppable にも載っていない(列の隙間・ボードの余白・
 *    ウィンドウ外へ出た)なら、掴んでいる矩形と重なっている droppable。
 * 3. それも無ければ最寄り。ドラッグ中は必ず 1 つ返るようにするための保険。
 */
export const boardCollisionDetection: CollisionDetection = (args) => {
  const byPointer = pointerWithin(args);
  if (byPointer.length > 0) return byPointer;
  const byRect = rectIntersection(args);
  if (byRect.length > 0) return byRect;
  return closestCorners(args);
};
