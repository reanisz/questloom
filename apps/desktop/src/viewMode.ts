/**
 * ボードの表示モード(通常 / 全列展開)の保存。
 *
 * バックエンドに持たせるほどの情報ではないため localStorage に置く。
 * localStorage が使えない環境(プライベートモード等)でも落ちないよう例外は握り潰す。
 */

import { useCallback, useState } from "react";

/** 表示モードの localStorage キー。値は "1"(展開)/ "0"(通常)。 */
export const VIEW_MODE_KEY = "questloom.board.expanded";

function load(): boolean {
  try {
    return localStorage.getItem(VIEW_MODE_KEY) === "1";
  } catch {
    return false;
  }
}

function save(expanded: boolean): void {
  try {
    localStorage.setItem(VIEW_MODE_KEY, expanded ? "1" : "0");
  } catch {
    // 保存できなくても表示は続けられるので無視する。
  }
}

/** 全列展開モードかどうかを、localStorage に永続化しつつ保持する。 */
export function useExpandedView(): [boolean, (expanded: boolean) => void] {
  const [expanded, setExpandedState] = useState<boolean>(load);

  const setExpanded = useCallback((next: boolean) => {
    setExpandedState(next);
    save(next);
  }, []);

  return [expanded, setExpanded];
}
