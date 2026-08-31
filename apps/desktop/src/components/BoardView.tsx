/**
 * ボード全体。列間移動・列内並び替えのドラッグ&ドロップを担う。
 *
 * ドロップ時にドロップ先の前後カード id (`prevId` / `nextId`) を求めて `move_task` に渡す。
 * 両方 null なら列末尾。並び順キーの生成はバックエンドが行う。
 * 着地点の計算そのものは、単体テストできるよう
 * [`resolveDropPosition`](./dropPosition.ts) に純関数として切り出してある。
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
import { useEffect, useState } from "react";

import * as api from "../api";
import { useBoardStore } from "../store";
import {
  BOARD_COLUMNS,
  isPrimaryColumn,
  type Board,
  type BoardColumnKey,
  type TaskCard,
} from "../types";
import { Column, columnDomId } from "./Column";
import { DeferRail } from "./DeferRail";
import { locate, resolveDropPosition } from "./dropPosition";
import { CardBody } from "./TaskCardView";

/** レールから展開したバケットを強調しておく時間 (ms)。 */
const FOCUS_HIGHLIGHT_MS = 1600;

interface Props {
  board: Board;
  /** 全 8 列を列として表示するか。false なら 4 列 + 先送りレール。 */
  expanded: boolean;
  /** レールのバケットが選ばれたときに展開表示へ切り替える。 */
  onExpand: () => void;
}

export function BoardView({ board, expanded, onExpand }: Props) {
  const mutate = useBoardStore((state) => state.mutate);
  const applyLocalMove = useBoardStore((state) => state.applyLocalMove);
  const [activeCard, setActiveCard] = useState<TaskCard | null>(null);
  const [focused, setFocused] = useState<BoardColumnKey | null>(null);

  const columns = expanded ? BOARD_COLUMNS : BOARD_COLUMNS.filter(({ key }) => isPrimaryColumn(key));

  // 展開直後にその列を視界へ入れ、少し経ったら強調を消す。
  useEffect(() => {
    if (!focused) return;
    document
      .getElementById(columnDomId(focused))
      ?.scrollIntoView({ behavior: "smooth", inline: "nearest", block: "nearest" });
    const timer = setTimeout(() => setFocused(null), FOCUS_HIGHLIGHT_MS);
    return () => clearTimeout(timer);
  }, [focused]);

  const openBucket = (key: BoardColumnKey) => {
    onExpand();
    setFocused(key);
  };

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
    const landing = resolveDropPosition(
      board.columns,
      activeId,
      String(over.id),
      active.rect.current.translated,
      over.rect,
    );
    if (!landing) return;

    const { column, prevId, nextId } = landing;
    applyLocalMove(activeId, column, prevId, nextId);
    void mutate(() => api.moveTask(activeId, { column, prevId, nextId }));
  };

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCorners}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onDragCancel={() => setActiveCard(null)}
    >
      <div className={`board${expanded ? " board-expanded" : ""}`}>
        {columns.map(({ key, label }) => (
          <Column
            key={key}
            columnKey={key}
            label={label}
            cards={board.columns[key]}
            focused={focused === key}
          />
        ))}
        {!expanded && (
          <DeferRail columns={board.columns} dragging={activeCard != null} onOpen={openBucket} />
        )}
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
