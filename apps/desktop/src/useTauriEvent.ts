/**
 * Tauri イベント購読の共通フック。
 *
 * `listen()` は `Promise<UnlistenFn>` を返すため、素朴に書くと
 * 「解除関数が届く前にアンマウントされたら解除し損ねる」定型が各所に散らばる。
 * ここに 1 つだけ置いて使い回す。
 */

import { useEffect, useRef } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";

/**
 * イベントを購読し、アンマウント時に解除する。
 *
 * `subscribe` は**安定した参照**を渡すこと(モジュールスコープの関数か
 * `useCallback` の戻り値)。これが変わるたびに購読を張り直す。
 * `handler` は毎レンダー差し替わってよい(参照で保持するため再購読は起きない)。
 *
 * ```ts
 * useTauriEvent(listenTasksChanged, () => void refresh());
 * ```
 */
export function useTauriEvent<A extends unknown[]>(
  subscribe: (handler: (...args: A) => void) => Promise<UnlistenFn>,
  handler: (...args: A) => void,
): void {
  const latest = useRef(handler);
  latest.current = handler;

  useEffect(() => {
    let stopped = false;
    let off: UnlistenFn | null = null;

    void subscribe((...args: A) => latest.current(...args))
      .then((unlisten) => {
        // 購読が張られる前にアンマウントされていたら、届いた時点で解除する。
        if (stopped) unlisten();
        else off = unlisten;
      })
      .catch(() => undefined);

    return () => {
      stopped = true;
      off?.();
      off = null;
    };
  }, [subscribe]);
}
