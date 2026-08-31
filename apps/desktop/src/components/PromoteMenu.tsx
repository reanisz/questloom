/** インスタントタスクの「昇格」用の小さな列選択メニュー。 */

import { useCallback, useEffect, useRef, useState } from "react";

import { ESC_LAYER, useEscapeKey } from "../keyboard";
import { columnLabel, PROMOTE_COLUMNS, type BoardColumnKey } from "../types";

interface Props {
  /** 列を選んだときに呼ばれる。 */
  onSelect: (column: BoardColumnKey) => void;
  /** トリガーボタンの追加クラス。 */
  className?: string;
}

export function PromoteMenu({ onSelect, className }: Props) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  const close = useCallback(() => setOpen(false), []);

  // メニューは常に呼び出し元より前面。開いている間の Esc はここで止め、
  // 後ろのドロワーやカードまで一緒に閉じないようにする。
  useEscapeKey(close, { enabled: open, priority: ESC_LAYER.popup });

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  return (
    <div className="promote" ref={rootRef} onPointerDown={(event) => event.stopPropagation()}>
      <button
        type="button"
        className={className ?? "btn btn-ghost btn-sm"}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={(event) => {
          event.stopPropagation();
          setOpen((value) => !value);
        }}
      >
        昇格
      </button>
      {open && (
        <div className="promote-menu" role="menu">
          {PROMOTE_COLUMNS.map((column) => (
            <button
              key={column}
              type="button"
              role="menuitem"
              className="promote-item"
              onClick={(event) => {
                event.stopPropagation();
                setOpen(false);
                onSelect(column);
              }}
            >
              {columnLabel(column)}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
