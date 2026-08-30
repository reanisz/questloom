/**
 * ボード全体。列間移動・列内並び替えのドラッグ&ドロップを担う。
 *
 * ドロップ時にドロップ先の前後カード id (`prevId` / `nextId`) を求めて `move_task` に渡す。
 * 両方 null なら列末尾。並び順キーの生成はバックエンドが行う。
 */

import {
  closestCorners,
  DndContext,
  DragOverlay,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DragStartEvent,
} from "@dnd-kit/core";
import { useState } from "react";

import * as api from "../api";
import { useBoardStore } from "../store";
import { BOARD_COLUMNS, type Board, type BoardColumnKey, type TaskCard, type TaskId } from "../types";
import { Column, COLUMN_DROPPABLE_PREFIX } from "./Column";
import { CardBody } from "./TaskCardView";

/** カードが属する列を探す。 */
function locate(columns: Board["columns"], taskId: TaskId): BoardColumnKey | null {
  for (const { key } of BOARD_COLUMNS) {
    if (columns[key].some((card) => card.id === taskId)) return key;
  }
  return null;
}

export function BoardView({ board }: { board: Board }) {
  const mutate = useBoardStore((state) => state.mutate);
  const applyLocalMove = useBoardStore((state) => state.applyLocalMove);
  const [activeCard, setActiveCard] = useState<TaskCard | null>(null);

  // 数 px 動かすまではドラッグを開始しない(カードのクリックを潰さないため)。
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }));

  const onDragStart = (event: DragStartEvent) => {
    const id = String(event.active.id);
    const column = locate(board.columns, id);
    setActiveCard(column ? (board.columns[column].find((card) => card.id === id) ?? null) : null);
  };

  const onDragEnd = ({ active, over }: DragEndEvent) => {
    setActiveCard(null);
    if (!over) return;

    const activeId = String(active.id);
    const overId = String(over.id);
    const from = locate(board.columns, activeId);
    if (!from) return;

    const to = overId.startsWith(COLUMN_DROPPABLE_PREFIX)
      ? (overId.slice(COLUMN_DROPPABLE_PREFIX.length) as BoardColumnKey)
      : locate(board.columns, overId);
    if (!to) return;

    // 移動対象を除いた移動先の並び。ここへの挿入位置から前後 id を求める。
    const items = board.columns[to].map((card) => card.id).filter((id) => id !== activeId);

    let insertAt: number;
    if (overId.startsWith(COLUMN_DROPPABLE_PREFIX) || overId === activeId) {
      insertAt = items.length;
    } else {
      const overIndex = items.indexOf(overId);
      if (overIndex < 0) return;
      // ドラッグ中の矩形の中心が対象カードの中心より下なら後ろへ挿入する。
      const dragged = active.rect.current.translated;
      const below =
        dragged != null && dragged.top + dragged.height / 2 > over.rect.top + over.rect.height / 2;
      insertAt = overIndex + (below ? 1 : 0);
    }

    const prevId = items[insertAt - 1] ?? null;
    const nextId = items[insertAt] ?? null;

    // 同じ列で位置が変わらないなら何もしない。
    if (from === to) {
      const current = board.columns[from].map((card) => card.id);
      const at = current.indexOf(activeId);
      if ((current[at - 1] ?? null) === prevId && (current[at + 1] ?? null) === nextId) return;
    }

    applyLocalMove(activeId, to, prevId, nextId);
    void mutate(() => api.moveTask(activeId, { column: to, prevId, nextId }));
  };

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCorners}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onDragCancel={() => setActiveCard(null)}
    >
      <div className="board">
        {BOARD_COLUMNS.map(({ key, label }) => (
          <Column key={key} columnKey={key} label={label} cards={board.columns[key]} />
        ))}
      </div>
      <DragOverlay dropAnimation={null}>
        {activeCard && (
          <div className={`card card-overlay${activeCard.isInstant ? " card-instant" : ""}`}>
            <CardBody card={activeCard} dragging />
          </div>
        )}
      </DragOverlay>
    </DndContext>
  );
}
