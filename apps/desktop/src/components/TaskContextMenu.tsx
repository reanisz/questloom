/**
 * タスクカードの右クリックメニュー。
 *
 * ネイティブメニューではなく HTML で描く(項目の出し分けをカードの状態から決めたいのと、
 * ボードの見た目=ガラスパネルに揃えたいため)。位置はカーソル基準で、画面端では
 * [`clampMenuPosition`](./contextMenu.ts) が折り返す。
 *
 * ## 階層
 *
 * 「昇格」と「移動」は列を選ばせる必要がある。横に開くフライアウトにすると、
 * 画面端での再クランプ・ホバーの猶予・親項目のハイライト維持…と面倒が増えるので、
 * **同じパネルの中身を第 2 階層に差し替える**方式にした。パネルは 1 枚のままなので、
 * はみ出し補正も「中身が変わったら測り直す」だけで済む。
 *
 * ## 閉じる
 *
 * 項目の選択・メニュー外の pointerdown・Esc で閉じる。別のカードの右クリックは
 * 「外の pointerdown で閉じる → contextmenu で開き直す」の順に起きるので、
 * 呼び出し元 ([`BoardView`](./BoardView.tsx)) が新しい座標で開き直せばよい。
 * Esc は [`ESC_LAYER.popup`] なので、後ろのドロワーやダイアログは道連れにならない。
 */

import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";

import * as api from "../api";
import { ESC_LAYER, useEscapeKey } from "../keyboard";
import { useBoardStore } from "../store";
import { toMessage } from "../tauri";
import {
  BOARD_COLUMNS,
  columnLabel,
  PROMOTE_COLUMNS,
  type BoardColumnKey,
  type TaskCard,
} from "../types";
import {
  clampMenuPosition,
  contextMenuActions,
  type ContextMenuAction,
  type Point,
} from "./contextMenu";

/** 表示中の階層。`root` 以外は列を選ばせる第 2 階層。 */
type Level = "root" | "promote" | "move";

interface Props {
  card: TaskCard;
  /** カードが今いる列。「移動」でその列を無効化する。 */
  column: BoardColumnKey;
  /** 右クリックした位置 (clientX / clientY)。 */
  anchor: Point;
  onClose: () => void;
  /** 「削除」。確認ダイアログを挟むので、実行は呼び出し元に委ねる。 */
  onDelete: () => void;
}

/** メニューの 1 行。 */
function Item({
  action,
  onSelect,
  danger,
  submenu,
  disabled,
  children,
}: {
  /** `data-testid` に使う識別子(`context-<action>`)。 */
  action: string;
  onSelect: () => void;
  danger?: boolean;
  /** 第 2 階層を開く項目。右端に ▸ を出す。 */
  submenu?: boolean;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      className={`context-menu-item${danger ? " context-menu-item-danger" : ""}`}
      data-testid={`context-${action}`}
      disabled={disabled}
      onClick={(event) => {
        event.stopPropagation();
        onSelect();
      }}
    >
      <span className="context-menu-label">{children}</span>
      {submenu && <span className="context-menu-arrow">▸</span>}
    </button>
  );
}

export function TaskContextMenu({ card, column, anchor, onClose, onDelete }: Props) {
  const mutate = useBoardStore((state) => state.mutate);
  const openTask = useBoardStore((state) => state.openTask);
  const setError = useBoardStore((state) => state.setError);

  const [level, setLevel] = useState<Level>("root");
  /** 補正後の左上座標。測るまでは null で、その間は隠しておく(ちらつき防止)。 */
  const [position, setPosition] = useState<Point | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  // 描画後の実寸で位置を決め直す。paint 前に走るので画面には補正後だけが出る。
  // 第 2 階層へ切り替えると高さが変わるので、level も依存に入れる。
  useLayoutEffect(() => {
    const element = rootRef.current;
    if (!element) return;
    setPosition(
      clampMenuPosition(
        anchor,
        { width: element.offsetWidth, height: element.offsetHeight },
        { width: window.innerWidth, height: window.innerHeight },
      ),
    );
  }, [anchor, level]);

  // ポップアップなので Esc はここで止める(後ろのドロワーまで閉じない)。
  useEscapeKey(onClose, { priority: ESC_LAYER.popup });

  // メニューの外を押したら閉じる。開いた契機の pointerdown は既に済んでいるので、
  // 「開いた瞬間に閉じる」ことはない。
  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) onClose();
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [onClose]);

  /** 項目を選んだときの定型。メニューを畳んでから実行する。 */
  const run = (action: () => void) => {
    onClose();
    action();
  };

  const renderItem = (action: ContextMenuAction): ReactNode => {
    switch (action) {
      case "open":
        return (
          <Item key={action} action="open" onSelect={() => run(() => openTask(card.id))}>
            詳細を開く
          </Item>
        );
      case "complete":
        return (
          <Item
            key={action}
            action="complete"
            onSelect={() => run(() => void mutate(() => api.completeTask(card.id)))}
          >
            ✓ 完了にする
          </Item>
        );
      case "promote":
        return (
          <Item key={action} action="promote" submenu onSelect={() => setLevel("promote")}>
            ⚡ 昇格
          </Item>
        );
      case "move":
        return (
          <Item key={action} action="move" submenu onSelect={() => setLevel("move")}>
            移動
          </Item>
        );
      case "url":
        return (
          <Item
            key={action}
            action="url"
            onSelect={() =>
              run(() => {
                const value = card.primaryResource?.value;
                if (!value) return;
                openUrl(value).catch((error: unknown) => setError(toMessage(error)));
              })
            }
          >
            🔗 URL を開く
          </Item>
        );
      case "delete":
        return (
          <div key={action} className="context-menu-group">
            <div className="context-menu-sep" role="separator" />
            <Item action="delete" danger onSelect={() => run(onDelete)}>
              🗑 削除
            </Item>
          </div>
        );
    }
  };

  /** 第 2 階層の見出し行(← で root へ戻る)。 */
  const back = (label: string) => (
    <button
      type="button"
      className="context-menu-back"
      data-testid="context-back"
      onClick={(event) => {
        event.stopPropagation();
        setLevel("root");
      }}
    >
      ← {label}
    </button>
  );

  return (
    <div
      ref={rootRef}
      className="context-menu"
      data-testid="task-context-menu"
      role="menu"
      aria-label={`${card.title} の操作`}
      style={{
        left: position?.x ?? anchor.x,
        top: position?.y ?? anchor.y,
        visibility: position ? undefined : "hidden",
      }}
      // メニューの上での右クリックで、標準メニューを出させない。
      onContextMenu={(event) => event.preventDefault()}
      // ドラッグの開始や、カードのクリック(詳細を開く)へ漏らさない。
      onPointerDown={(event) => event.stopPropagation()}
      onClick={(event) => event.stopPropagation()}
    >
      {level === "root" && contextMenuActions(card).map(renderItem)}

      {level === "promote" && (
        <>
          {back("昇格先")}
          {PROMOTE_COLUMNS.map((key) => (
            <Item
              key={key}
              action={`promote-${key}`}
              onSelect={() => run(() => void mutate(() => api.promoteTask(card.id, key)))}
            >
              {columnLabel(key)}
            </Item>
          ))}
        </>
      )}

      {level === "move" && (
        <>
          {back("移動先")}
          {BOARD_COLUMNS.map(({ key, label }) => (
            <Item
              key={key}
              action={`move-${key}`}
              disabled={key === column}
              onSelect={() =>
                run(() =>
                  void mutate(() =>
                    api.moveTask(card.id, { column: key, prevId: null, nextId: null }),
                  ),
                )
              }
            >
              {label}
            </Item>
          ))}
        </>
      )}
    </div>
  );
}
