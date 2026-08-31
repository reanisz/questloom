/** ボード上のタスクカード。ドラッグ可能で、クリックで詳細ドロワーを開く。 */

import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useRef } from "react";

import * as api from "../api";
import { formatDeadline, isOverdue } from "../format";
import { useBoardStore } from "../store";
import type { BoardColumnKey, TaskCard } from "../types";
import { PromoteMenu } from "./PromoteMenu";

/** ドラッグとクリックを区別する閾値 (px)。 */
const CLICK_SLOP = 5;

/** カードの見た目部分。ドラッグオーバーレイからも使う。 */
export function CardBody({ card, dragging }: { card: TaskCard; dragging?: boolean }) {
  const mutate = useBoardStore((state) => state.mutate);
  const overdue = isOverdue(card.deadline);

  return (
    <>
      <div className="card-title">
        {card.isInstant && (
          <span className="badge badge-instant" title="インスタントタスク">
            ⚡
          </span>
        )}
        <span>{card.title}</span>
      </div>

      {(card.deadline || card.childCount > 0 || card.resourceCount > 0) && (
        <div className="card-meta">
          {card.deadline && (
            <span className={overdue ? "badge badge-overdue" : "badge"} title="締切">
              ⏰ {formatDeadline(card.deadline)}
            </span>
          )}
          {card.childCount > 0 && (
            <span className="badge" title="子タスク数">
              ⛓ {card.childCount}
            </span>
          )}
          {card.resourceCount > 0 && (
            <span className="badge" title="関連リソース数">
              🔗 {card.resourceCount}
            </span>
          )}
        </div>
      )}

      {card.isInstant && !dragging && (
        <div className="card-actions">
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            onPointerDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation();
              void mutate(() => api.completeTask(card.id));
            }}
          >
            ✓ 完了
          </button>
          <PromoteMenu
            onSelect={(column: BoardColumnKey) =>
              void mutate(() => api.promoteTask(card.id, column))
            }
          />
        </div>
      )}
    </>
  );
}

interface CardProps {
  card: TaskCard;
  /**
   * 右クリックされた。カーソル位置を渡すので、呼び出し元がそこにメニューを出す。
   * 標準のコンテキストメニューの抑止はここで済ませてある。
   */
  onContextMenu?: (card: TaskCard, at: { x: number; y: number }) => void;
}

export function TaskCardView({ card, onContextMenu }: CardProps) {
  const openTask = useBoardStore((state) => state.openTask);
  const origin = useRef<{ x: number; y: number } | null>(null);
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: card.id,
  });

  return (
    <div
      ref={setNodeRef}
      data-testid="task-card"
      className={`card${card.isInstant ? " card-instant" : ""}${isDragging ? " card-dragging" : ""}`}
      style={{ transform: CSS.Translate.toString(transform), transition }}
      {...attributes}
      {...listeners}
      onPointerDown={(event) => {
        origin.current = { x: event.clientX, y: event.clientY };
        listeners?.onPointerDown?.(event);
      }}
      // 右クリックは標準メニューを止めて自前のメニューを出す。ドラッグ (dnd-kit の
      // PointerSensor) は左ボタンしか見ないので、右ボタンで掴んでしまう心配はない。
      onContextMenu={(event) => {
        if (!onContextMenu) return;
        event.preventDefault();
        event.stopPropagation();
        onContextMenu(card, { x: event.clientX, y: event.clientY });
      }}
      onClick={(event) => {
        // ドラッグ後のクリックで詳細が開かないよう、移動量で判定する。
        const start = origin.current;
        origin.current = null;
        if (start) {
          const moved = Math.hypot(event.clientX - start.x, event.clientY - start.y);
          if (moved > CLICK_SLOP) return;
        }
        openTask(card.id);
      }}
    >
      <CardBody card={card} />
    </div>
  );
}
