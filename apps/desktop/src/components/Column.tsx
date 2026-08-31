/** ボードの 1 列。カードのドロップ先であり、下部にクイック追加フォームを持つ。 */

import { useDroppable } from "@dnd-kit/core";
import { SortableContext, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { useState } from "react";

import * as api from "../api";
import { useBoardStore } from "../store";
import type { BoardColumnKey, TaskCard } from "../types";
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
}

export function Column({ columnKey, label, cards, focused }: Props) {
  const mutate = useBoardStore((state) => state.mutate);
  const [draft, setDraft] = useState("");
  const [adding, setAdding] = useState(false);
  const { setNodeRef, isOver } = useDroppable({ id: `${COLUMN_DROPPABLE_PREFIX}${columnKey}` });

  const submit = async () => {
    const title = draft.trim();
    if (!title || adding) return;
    setAdding(true);
    // New で作成し、New 以外の列なら続けて move_task で列相当の状態・予定へ移す。
    // 列 → status/scheduled の変換をバックエンドの move_task に一元化するための手順。
    const ok = await mutate(async () => {
      const task = await api.createTask({ title });
      if (columnKey !== "new") {
        await api.moveTask(task.id, { column: columnKey, prevId: null, nextId: null });
      }
    });
    setAdding(false);
    if (ok) setDraft("");
  };

  return (
    <section
      id={columnDomId(columnKey)}
      className={`column${isOver ? " column-over" : ""}${focused ? " column-focused" : ""}`}
    >
      <header className="column-header">
        <h2>{label}</h2>
        <span className="column-count">{cards.length}</span>
      </header>

      <div className="column-body" ref={setNodeRef}>
        <SortableContext items={cards.map((card) => card.id)} strategy={verticalListSortingStrategy}>
          {cards.map((card) => (
            <TaskCardView key={card.id} card={card} />
          ))}
        </SortableContext>
        {cards.length === 0 && <p className="column-empty">タスクなし</p>}
      </div>

      <form
        className="quick-add"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <input
          type="text"
          value={draft}
          placeholder="+ タスクを追加"
          aria-label={`${label} にタスクを追加`}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") setDraft("");
          }}
        />
        {draft.trim() && (
          <button type="submit" className="btn btn-primary btn-sm" disabled={adding}>
            追加
          </button>
        )}
      </form>
    </section>
  );
}
