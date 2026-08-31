/**
 * テスト専用の React マウントヘルパ(vitest + jsdom)。
 *
 * フック(`useEscapeKey` / `useExpandedView`)は React の外では動かないので、
 * 実際に小さなコンポーネントを描画して観察する。@testing-library は入れず、
 * `react-dom/client` + React 19 の `act` だけで済ませている。
 *
 * **このモジュールは `*.test.tsx` からのみ import すること。** アプリの
 * エントリからは辿られないため、`vite build` の成果物には入らない。
 */

import { act, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";

// React に「テスト環境なので act() の外の更新は警告してよい」と伝える。
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** [`mount`] が返す操作口。 */
export interface Mounted {
  /** マウント先の DOM。 */
  container: HTMLElement;
  /** 別の要素で描画し直す(props を変えて再レンダーするのに使う)。 */
  rerender: (element: ReactElement) => void;
  /** アンマウントして DOM も片付ける。 */
  unmount: () => void;
}

/** 要素を新しいルートへ描画する。呼び出しごとに独立したルートになる。 */
export function mount(element: ReactElement): Mounted {
  const container = document.createElement("div");
  document.body.appendChild(container);
  let root!: Root;
  act(() => {
    root = createRoot(container);
    root.render(element);
  });

  return {
    container,
    rerender(next) {
      act(() => root.render(next));
    },
    unmount() {
      act(() => root.unmount());
      container.remove();
    },
  };
}

/** [`renderHook`] が返す観察口。 */
export interface RenderedHook<T> extends Omit<Mounted, "rerender"> {
  /** 直近のレンダーでフックが返した値。 */
  result: { current: T };
  /** 再レンダーさせる。 */
  rerender: () => void;
}

/** フックだけを描画して、戻り値を観察できるようにする。 */
export function renderHook<T>(hook: () => T): RenderedHook<T> {
  const result: { current: T } = { current: undefined as T };

  function Probe() {
    result.current = hook();
    return null;
  }

  // key を変えないので、rerender は同じインスタンスの再レンダーになる(state は保たれる)。
  const mounted = mount(<Probe />);
  return {
    container: mounted.container,
    result,
    rerender: () => mounted.rerender(<Probe />),
    unmount: mounted.unmount,
  };
}

/** キーを 1 つ押す。`target` 省略時は document へ直接送る。 */
export function pressKey(key: string, target: EventTarget = document, init?: KeyboardEventInit) {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
  act(() => {
    target.dispatchEvent(event);
  });
  return event;
}
