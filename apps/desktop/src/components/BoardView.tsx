/**
 * ボード全体。列間移動・列内並び替えのドラッグ&ドロップを担う。
 *
 * ドロップ時にドロップ先の前後カード id (`prevId` / `nextId`) を求めて `move_task` に渡す。
 * 両方 null なら列末尾。並び順キーの生成はバックエンドが行う。
 * 着地点の計算そのものは、単体テストできるよう
 * [`resolveDropPosition`](./dropPosition.ts) に純関数として切り出してある。
 *
 * ## 衝突判定とハイライト
 *
 * 判定は**ポインタ優先** ([`boardCollisionDetection`](./collision.ts))。以前使っていた
 * `closestCorners` は「掴んでいる矩形の四隅」と droppable の四隅の距離で決めるため、
 * カード 1 枚分の幅を持つ矩形が列の境界をまたぐまで判定が切り替わらず、
 * ハイライトがカーソルより遅れて付いてくる。
 *
 * ハイライトする列も、`useDroppable().isOver`(= 列そのものに重なっているときだけ真)
 * ではなく `over` から求めた着地列 ([`columnOf`]) で決める。dnd-kit の sortable は
 * **別のコンテナのカードに重なっている間はカードをずらさない**
 * (`disableTransforms = overIndex !== -1 && activeIndex === -1`)ので、
 * カードが詰まった領域では列のハイライトだけが唯一の手がかりになる。
 */

import {
  DndContext,
  DragOverlay,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DragOverEvent,
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
  type TaskId,
} from "../types";
import { ArchivedDoneDialog } from "./ArchivedDoneDialog";
import { boardCollisionDetection } from "./collision";
import { Column, columnDomId } from "./Column";
import { type Point } from "./contextMenu";
import { DeferRail } from "./DeferRail";
import { DeleteConfirmDialog } from "./DeleteConfirmDialog";
import { columnOf, locate, resolveDropPosition } from "./dropPosition";
import { CardBody } from "./TaskCardView";
import { TaskContextMenu } from "./TaskContextMenu";

/** レールから展開したバケットを強調しておく時間 (ms)。 */
const FOCUS_HIGHLIGHT_MS = 1600;

/** 開いている右クリックメニュー。 */
interface MenuState {
  card: TaskCard;
  column: BoardColumnKey;
  anchor: Point;
}

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
  const [overColumn, setOverColumn] = useState<BoardColumnKey | null>(null);
  const [focused, setFocused] = useState<BoardColumnKey | null>(null);
  /** カードの右クリックメニュー。ボード全体で高々 1 つ。 */
  const [menu, setMenu] = useState<MenuState | null>(null);
  /** 右クリックメニューの「削除」の確認待ち。ドロワーのそれとは独立。 */
  const [confirmDelete, setConfirmDelete] = useState<{ id: TaskId; title: string } | null>(null);
  const [deleting, setDeleting] = useState(false);
  /** Done 列のフッタから開く「過去の完了」一覧。 */
  const [archivedOpen, setArchivedOpen] = useState(false);

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

  /** 削除を確定する。ボードの更新は tasks-changed 任せ。 */
  const runDelete = async (taskId: TaskId) => {
    setDeleting(true);
    await mutate(() => api.deleteTask(taskId));
    setDeleting(false);
    setConfirmDelete(null);
  };

  const onDragStart = (event: DragStartEvent) => {
    setMenu(null);
    const id = String(event.active.id);
    const column = locate(board.columns, id);
    setActiveCard(column ? (board.columns[column].find((card) => card.id === id) ?? null) : null);
  };

  // 着地する列。カードに重なっていてもその親の列を光らせる。
  const onDragOver = ({ over }: DragOverEvent) => {
    setOverColumn(over ? columnOf(board.columns, String(over.id)) : null);
  };

  const stopDragging = () => {
    setActiveCard(null);
    setOverColumn(null);
  };

  const onDragEnd = ({ active, over }: DragEndEvent) => {
    stopDragging();
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
      collisionDetection={boardCollisionDetection}
      onDragStart={onDragStart}
      onDragOver={onDragOver}
      onDragEnd={onDragEnd}
      onDragCancel={stopDragging}
    >
      <div className={`board${expanded ? " board-expanded" : ""}`}>
        {columns.map(({ key, label }) => (
          <Column
            key={key}
            columnKey={key}
            label={label}
            cards={board.columns[key]}
            focused={focused === key}
            over={overColumn === key}
            // Done 列は今日完了した分しか出さないので、それ以前への入口を足す。
            footer={
              key === "done" && board.archivedDoneCount > 0 ? (
                <button
                  type="button"
                  className="column-link"
                  data-testid="open-archived-done"
                  title="前日以前に完了したタスクを見る"
                  onClick={() => setArchivedOpen(true)}
                >
                  過去の完了 {board.archivedDoneCount} 件…
                </button>
              ) : undefined
            }
            onCardContextMenu={(card, column, anchor) => setMenu({ card, column, anchor })}
          />
        ))}
        {!expanded && (
          <DeferRail
            columns={board.columns}
            dragging={activeCard != null}
            overColumn={overColumn}
            onOpen={openBucket}
          />
        )}
      </div>
      <DragOverlay dropAnimation={null}>
        {activeCard && (
          <div className={`card card-overlay${activeCard.isInstant ? " card-instant" : ""}`}>
            <CardBody card={activeCard} dragging />
          </div>
        )}
      </DragOverlay>
      {menu && (
        // 開き直しでも状態(第 2 階層など)を持ち越さないよう、カードと位置で作り直す。
        <TaskContextMenu
          key={`${menu.card.id}:${menu.anchor.x}:${menu.anchor.y}`}
          card={menu.card}
          column={menu.column}
          anchor={menu.anchor}
          onClose={() => setMenu(null)}
          onDelete={() => setConfirmDelete({ id: menu.card.id, title: menu.card.title })}
        />
      )}
      {archivedOpen && <ArchivedDoneDialog onClose={() => setArchivedOpen(false)} />}
      {confirmDelete && (
        <DeleteConfirmDialog
          title={confirmDelete.title}
          busy={deleting}
          onConfirm={() => void runDelete(confirmDelete.id)}
          onClose={() => setConfirmDelete(null)}
        />
      )}
    </DndContext>
  );
}
