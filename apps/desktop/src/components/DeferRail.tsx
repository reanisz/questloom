/**
 * 先送りバケット (Tomorrow / This Week / Next Week / Future / Icebox / 監視中) の
 * 細いサイドレール。
 *
 * 通常表示ではこの 6 つを列として展開せず、ラベル + 件数だけのドロップボックスにする。
 * ボックスは列と同じ droppable id (`column:<key>`) を持つので、BoardView の onDragEnd は
 * 列へのドロップと同じ経路で処理でき、ドロップ位置はバケット末尾になる。
 * クリックすると全列展開表示へ切り替え、そのバケットの列を強調する。
 */

import { useDroppable } from "@dnd-kit/core";

import {
  columnIcon,
  columnLabel,
  DEFER_COLUMNS,
  type BoardColumnKey,
  type BoardColumns,
} from "../types";
import { COLUMN_DROPPABLE_PREFIX } from "./Column";

/** ドロップの意味を説明する語。時間バケット以外は「先送り」ではないので言い回しを変える。 */
const DROP_HINTS: Partial<Record<BoardColumnKey, string>> = {
  watching: "外部の変化待ちにもできます",
  icebox: "棚上げもできます",
};

/** ボックスの説明文。 */
function boxTitle(columnKey: BoardColumnKey, label: string): string {
  const hint = DROP_HINTS[columnKey] ?? "先送りもできます";
  return `${label} を展開表示で開く(ドラッグして${hint})`;
}

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
  const icon = columnIcon(columnKey);

  return (
    <button
      type="button"
      ref={setNodeRef}
      data-testid={`defer-box-${columnKey}`}
      className={`defer-box${dragging ? " defer-box-armed" : ""}${over ? " defer-box-over" : ""}`}
      title={boxTitle(columnKey, label)}
      onClick={() => onOpen(columnKey)}
    >
      <span className="defer-box-label">
        {icon && (
          <span className="column-icon" aria-hidden="true">
            {icon}
          </span>
        )}
        {label}
      </span>
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
    <aside className="defer-rail" aria-label="先送りと監視中">
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
