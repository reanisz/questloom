/**
 * 内蔵ブラウザペインの共通処理。
 *
 * ペインの実体は main ウィンドウの子 webview(Rust 側 `crate::browser`)で、
 * 描画は [`BrowserPane`](./components/BrowserPane.tsx)、状態は
 * [`useBoardStore`](./store.ts) が持つ。ここに置くのは、その両方から使う小物。
 */

import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect } from "react";

import { useBoardStore } from "./store";
import { toMessage } from "./tauri";
import type { UrlOpenMode } from "./types";

/**
 * ペイン操作(生成・矩形・表示・破棄)を発行順に直列化するキュー。
 *
 * 子 webview の生成と破棄は非同期なので、素朴に投げると
 * 「開く → 閉じる → 開く」が入れ違って**最後に閉じてしまう**ことがある
 * (React の StrictMode はまさにこの順で effect を二重に走らせる)。
 * 1 本の Promise チェーンに積めば、発行順どおりに適用される。
 */
let queue: Promise<unknown> = Promise.resolve();

/** ペイン操作をキューへ積む。前の操作が失敗しても後続は続行する。 */
export function enqueuePaneOp<T>(op: () => Promise<T>): Promise<T> {
  const next = queue.then(op, op);
  // キュー自身は失敗を握りつぶす(呼び出し側が受け取った Promise で扱う)。
  queue = next.then(
    () => undefined,
    () => undefined,
  );
  return next;
}

/**
 * このコンポーネントが表示されている間、内蔵ブラウザペインを隠す。
 *
 * 子 webview はネイティブの子ウィンドウなので z-index に従わず、ドロワー・モーダル・
 * 右クリックメニューの**上に**描かれてしまう。覆う UI 側からこのフックを呼んで、
 * 開いている間だけ webview を隠す(閉じないのでページの状態は残る)。
 */
export function useOccludePane(enabled = true): void {
  const occludePane = useBoardStore((state) => state.occludePane);
  useEffect(() => {
    if (!enabled) return;
    occludePane(1);
    return () => occludePane(-1);
  }, [enabled, occludePane]);
}

/** URL を OS の既定ブラウザで開く。失敗はバナーに出す。 */
export function openExternal(url: string): void {
  openUrl(url).catch((error: unknown) => useBoardStore.getState().setError(toMessage(error)));
}

/**
 * URL リソースを、設定 (`urlOpenMode`) に従って開く。
 *
 * 明示的な「外部で開く」「内蔵ブラウザで開く」はこれを通さず、
 * [`openExternal`] / `store.openPane` を直接呼ぶ。
 */
export function openByMode(url: string, mode: UrlOpenMode): void {
  if (mode === "external") {
    openExternal(url);
  } else {
    useBoardStore.getState().openPane(url);
  }
}
