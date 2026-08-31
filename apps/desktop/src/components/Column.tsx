/** ボードの 1 列。カードのドロップ先であり、下部にクイック追加を持つ。 */

import { useDroppable } from "@dnd-kit/core";
import { SortableContext, verticalListSortingStrategy } from "@dnd-kit/sortable";

import * as api from "../api";
import { useBoardStore } from "../store";
import type { BoardColumnKey, TaskCard } from "../types";
import { QuickAdd } from "./QuickAdd";
import { TaskCardView } from "./TaskCardView";

/** 列の droppable id。カード id と衝突しないよう接頭辞を付ける。 */
export const COLUMN_DROPPABLE_PREFIX = "column:";

/** 列を DOM から引くための id。レールから展開したバケットをスクロールで見せるのに使う。 */
export const columnDomId = (key: BoardColumnKey) => `board-column-${key}`;

interface Props {
  columnKey: BoardColumnKey;
  label: string;
  cards: TaskCard[];
  /** レールから開かれた直後の一時的な強調。 */
  focused?: boolean;
  /**
   * 掴んでいるカードがこの列に落ちる状態か。
   *
   * `useDroppable().isOver` は「列そのものに重なっている」ときしか真にならず、
   * 列のカードに重なっている間(= 列の上半分)は落ちる先が分からなくなる。
   * そこで判定は [`BoardView`](./BoardView.tsx) が `over` から一元的に行う。
   */
  over?: boolean;
}

export function Column({ columnKey, label, cards, focused, over }: Props) {
  const mutate = useBoardStore((state) => state.mutate);
  // ドロップ先は列全体。ヘッダやクイック追加の上が死角にならないようにする。
  const { setNodeRef } = useDroppable({ id: `${COLUMN_DROPPABLE_PREFIX}${columnKey}` });

  // New で作成し、New 以外の列なら続けて move_task で列相当の状態・予定へ移す。
  // 列 → status/scheduled の変換をバックエンドの move_task に一元化するための手順。
  const add = (title: string) =>
    mutate(async () => {
      const task = await api.createTask({ title });
      if (columnKey !== "new") {
        await api.moveTask(task.id, { column: columnKey, prevId: null, nextId: null });
      }
    });

  return (
    <section
      ref={setNodeRef}
      id={columnDomId(columnKey)}
      data-testid={`column-${columnKey}`}
      className={`column${over ? " column-over" : ""}${focused ? " column-focused" : ""}`}
    >
      <header className="column-header">
        <h2>{label}</h2>
        <span className="column-count">{cards.length}</span>
      </header>

      <div className="column-body">
        <SortableContext items={cards.map((card) => card.id)} strategy={verticalListSortingStrategy}>
          {cards.map((card) => (
            <TaskCardView key={card.id} card={card} />
          ))}
        </SortableContext>
        {cards.length === 0 && <p className="column-empty">タスクなし</p>}
      </div>

      <QuickAdd columnKey={columnKey} label={label} onAdd={add} />
    </section>
  );
}
