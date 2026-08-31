/**
 * 独自タイトルバー(`decorations: false` の代替)。
 *
 * ドラッグ移動とダブルクリックでの最大化トグルは `data-tauri-drag-region` に任せる
 * (Tauri 側が mousedown を見て start_dragging / internal_toggle_maximize を呼ぶ)ため、
 * React では onDoubleClick を実装しない。実装すると二重にトグルされてしまう。
 */

import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useState } from "react";

import { useTauriEvent } from "../useTauriEvent";

/** ウィンドウのリサイズ購読(`useTauriEvent` に渡すため参照を固定する)。 */
const subscribeResized = (handler: () => void) => getCurrentWindow().onResized(handler);

/** 最大化状態を購読し、最大化 / 元に戻すアイコンを切り替えるためのフック。 */
function useMaximized(): boolean {
  const [maximized, setMaximized] = useState(false);

  const sync = useCallback(() => {
    void getCurrentWindow()
      .isMaximized()
      .then(setMaximized)
      .catch(() => undefined);
  }, []);

  useEffect(sync, [sync]);
  useTauriEvent(subscribeResized, sync);

  return maximized;
}

/** タイトルバーボタンのアイコン。線幅 1px の Windows 11 風グリフ。 */
function Glyph({ kind }: { kind: "minimize" | "maximize" | "restore" | "close" }) {
  const common = {
    width: 10,
    height: 10,
    viewBox: "0 0 10 10",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1,
    "aria-hidden": true,
  } as const;

  switch (kind) {
    case "minimize":
      return (
        <svg {...common}>
          <path d="M0.5 5.5 H9.5" />
        </svg>
      );
    case "maximize":
      return (
        <svg {...common}>
          <rect x="0.5" y="0.5" width="9" height="9" rx="1" />
        </svg>
      );
    case "restore":
      return (
        <svg {...common}>
          <rect x="0.5" y="2.5" width="7" height="7" rx="1" />
          <path d="M2.5 2.5 V1.5 A1 1 0 0 1 3.5 0.5 H8.5 A1 1 0 0 1 9.5 1.5 V6.5 A1 1 0 0 1 8.5 7.5 H7.5" />
        </svg>
      );
    case "close":
      return (
        <svg {...common}>
          <path d="M0.7 0.7 L9.3 9.3 M9.3 0.7 L0.7 9.3" />
        </svg>
      );
  }
}

export function TitleBar() {
  const maximized = useMaximized();

  return (
    <div className="titlebar" data-testid="titlebar" data-tauri-drag-region>
      <span className="titlebar-brand" data-tauri-drag-region>
        questloom
      </span>
      <div className="titlebar-drag" data-tauri-drag-region />
      <div className="titlebar-buttons">
        <button
          type="button"
          className="titlebar-btn"
          title="最小化"
          aria-label="最小化"
          onClick={() => void getCurrentWindow().minimize()}
        >
          <Glyph kind="minimize" />
        </button>
        <button
          type="button"
          className="titlebar-btn"
          title={maximized ? "元のサイズに戻す" : "最大化"}
          aria-label={maximized ? "元のサイズに戻す" : "最大化"}
          onClick={() => void getCurrentWindow().toggleMaximize()}
        >
          <Glyph kind={maximized ? "restore" : "maximize"} />
        </button>
        <button
          type="button"
          className="titlebar-btn titlebar-btn-close"
          title="閉じる"
          aria-label="閉じる"
          onClick={() => void getCurrentWindow().close()}
        >
          <Glyph kind="close" />
        </button>
      </div>
    </div>
  );
}
