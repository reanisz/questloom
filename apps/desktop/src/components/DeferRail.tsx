/**
 * 先送りバケット (Tomorrow / This Week / Next Week / Future) の細いサイドレール。
 *
 * 通常表示ではこの 4 バケットを列として展開せず、ラベル + 件数だけのドロップボックスにする。
 * ボックスは列と同じ droppable id (`column:<key>`) を持つので、BoardView の onDragEnd は
 * 列へのドロップと同じ経路で処理でき、ドロップ位置はバケット末尾になる。
 * クリックすると全列展開表示へ切り替え、そのバケットの列を強調する。
 */

import { useDroppable } from "@dnd-kit/core";

import { columnLabel, DEFER_COLUMNS, type BoardColumnKey, type BoardColumns } from "../types";
import { COLUMN_DROPPABLE_PREFIX } from "./Column";

interface BoxProps {
  columnKey: BoardColumnKey;
  count: number;
  /** ドラッグ中はドロップ可能であることを示すハイライトを出す。 */
  dragging: boolean;
  /** 掴んでいるカードがこのボックスに落ちる状態か(判定は BoardView が持つ)。 */
  over: boolean;
  onOpen: (key: BoardColumnKey) => void;
}

function DeferBox({ columnKey, count, dragging, over, onOpen }: BoxProps) {
  const { setNodeRef } = useDroppable({ id: `${COLUMN_DROPPABLE_PREFIX}${columnKey}` });
  const label = columnLabel(columnKey);

  return (
    <button
      type="button"
      ref={setNodeRef}
      className={`defer-box${dragging ? " defer-box-armed" : ""}${over ? " defer-box-over" : ""}`}
      title={`${label} を展開表示で開く(ドラッグして先送りもできます)`}
      onClick={() => onOpen(columnKey)}
    >
      <span className="defer-box-label">{label}</span>
      <span className={`defer-box-count${count > 0 ? " defer-box-count-filled" : ""}`}>{count}</span>
    </button>
  );
}

interface Props {
  columns: BoardColumns;
  dragging: boolean;
  /** 現在の着地先の列。レールのボックスと一致するものだけを光らせる。 */
  overColumn: BoardColumnKey | null;
  onOpen: (key: BoardColumnKey) => void;
}

export function DeferRail({ columns, dragging, overColumn, onOpen }: Props) {
  return (
    <aside className="defer-rail" aria-label="先送り">
      <div className="defer-rail-title">先送り</div>
      {DEFER_COLUMNS.map((key) => (
        <DeferBox
          key={key}
          columnKey={key}
          count={columns[key].length}
          dragging={dragging}
          over={overColumn === key}
          onOpen={onOpen}
        />
      ))}
    </aside>
  );
}
