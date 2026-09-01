/**
 * 内蔵ブラウザペイン。ボードの左側に外部ページを表示する。
 *
 * ## HTML と子 webview の分担
 *
 * ページ本体はネイティブの子 webview(Rust 側 `crate::browser`)で、React が描くのは
 * **枠だけ**。ヘッダ(URL・外部で開く・閉じる)と、webview を置く場所を示す空の箱を
 * 出し、その箱の実寸を `browser_pane_set_bounds` に送って webview を重ねる。
 *
 * 子 webview はネイティブの子ウィンドウなので、
 *
 * - CSS では位置も重なり順も決められない(実寸を測って Rust へ送る)、
 * - HTML の**どの要素よりも前面**に描かれる(ドロワーやモーダルが隠れる)
 *
 * という制約がある。後者は [`useOccludePane`](../browserPane.ts) で、覆う UI が
 * 開いている間だけ webview を隠して回避している。
 *
 * 矩形は `getBoundingClientRect()`(CSS ピクセル = ウィンドウのクライアント領域基準)を
 * そのまま論理ピクセルとして渡す。main ウィンドウは `decorations: false` で
 * webview がクライアント領域いっぱいなので、両者の原点は一致する。DPI の換算は
 * Tauri が `LogicalPosition` / `LogicalSize` から行う。
 *
 * ## ペインの中で押された Esc
 *
 * 子 webview のキー入力はこちらの `document` には届かない。Rust 側がペインへ注入した
 * スクリプトが `browser_pane_escape` を呼び、その結果が
 * `questloom://browser-pane-escape` として届くので、それを
 * [`dispatchEscape`](../keyboard.ts) で**キーボードの Esc と同じレイヤースタックへ**流す。
 * ドロワーやモーダルが開いていればそちらが先に閉じ、ペインしか無ければペインを閉じる。
 */

import { useEffect, useLayoutEffect, useRef } from "react";

import * as api from "../api";
import { enqueuePaneOp, openExternal } from "../browserPane";
import { dispatchEscape } from "../keyboard";
import { useBoardStore } from "../store";
import { toMessage } from "../tauri";
import { useTauriEvent } from "../useTauriEvent";

interface Props {
  /**
   * ボード以外のページ(設定画面)を出しているか。
   *
   * 真の間はペインを隠す。ドロワー・モーダル・メニューによる遮蔽は
   * store の `paneOccluders` 側で数える。
   */
  occluded?: boolean;
}

export function BrowserPane({ occluded = false }: Props) {
  const url = useBoardStore((state) => state.paneUrl);
  const occluders = useBoardStore((state) => state.paneOccluders);
  const closePane = useBoardStore((state) => state.closePane);
  const setError = useBoardStore((state) => state.setError);

  /** webview を重ねる箱。ここの実寸がそのまま bounds になる。 */
  const bodyRef = useRef<HTMLDivElement>(null);
  /** 生成済みか。生成前の矩形更新は無意味なので送らない。 */
  const created = useRef(false);

  const hidden = occluded || occluders > 0;

  /** 箱の現在の矩形。まだ測れないうちは null。 */
  const measure = (): api.PaneBounds | null => {
    const element = bodyRef.current;
    if (!element) return null;
    const rect = element.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return null;
    return { x: rect.left, y: rect.top, width: rect.width, height: rect.height };
  };

  const pushBounds = () => {
    if (!created.current) return;
    const bounds = measure();
    if (bounds) void enqueuePaneOp(() => api.setBrowserPaneBounds(bounds));
  };

  // 生成(と URL の差し替え)。レイアウト確定後の実寸を渡すので、初回から正しい位置に出る。
  useLayoutEffect(() => {
    if (!url) return;
    const bounds = measure() ?? undefined;
    enqueuePaneOp(() => api.openBrowserPane(url, bounds))
      .then(() => {
        created.current = true;
        // 生成を待つ間にウィンドウが変わっているかもしれないので測り直す。
        pushBounds();
      })
      .catch((error: unknown) => {
        setError(toMessage(error));
        closePane();
      });
    // measure / pushBounds が触るのは ref だけなので、依存は url だけでよい。
  }, [url]);

  // 閉じるのはアンマウント時だけ。store の closePane がアンマウントを起こす。
  useEffect(() => {
    return () => {
      created.current = false;
      void enqueuePaneOp(() => api.closeBrowserPane());
    };
  }, []);

  // レイアウトの変化(ウィンドウのリサイズ・ボードの表示切り替え)に追従する。
  useEffect(() => {
    const element = bodyRef.current;
    if (!element) return;
    const observer = new ResizeObserver(() => pushBounds());
    observer.observe(element);
    window.addEventListener("resize", pushBounds);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", pushBounds);
    };
  }, []);

  // 覆う UI が開いている間は隠す。戻すときは矩形も合わせ直す
  // (隠れている間にウィンドウが変わっていることがある)。
  useEffect(() => {
    void enqueuePaneOp(() => api.setBrowserPaneVisible(!hidden));
    if (!hidden) pushBounds();
  }, [hidden]);

  // ペイン内で押された Esc。まず既存のレイヤー(ドロワー・モーダル)へ配り、
  // 配る相手がいなければペイン自身を閉じる。
  useTauriEvent(api.listenBrowserPaneEscape, () => {
    if (!dispatchEscape()) closePane();
  });

  if (!url) return null;

  return (
    <aside className="browser-pane" data-testid="browser-pane" aria-label="内蔵ブラウザ">
      <header className="browser-pane-header">
        <span className="browser-pane-url" title={url}>
          {url}
        </span>
        <button
          type="button"
          className="btn btn-ghost btn-sm"
          data-testid="browser-pane-external"
          aria-label="既定のブラウザで開く"
          title="既定のブラウザで開く"
          onClick={() => openExternal(url)}
        >
          ↗
        </button>
        <button
          type="button"
          className="btn btn-ghost btn-sm"
          data-testid="browser-pane-close"
          aria-label="内蔵ブラウザを閉じる"
          title="内蔵ブラウザを閉じる"
          onClick={closePane}
        >
          ✕
        </button>
      </header>
      {/* 子 webview がここに重なる。中身は空でよい(隠れている間だけ見える背景)。 */}
      <div className="browser-pane-body" ref={bodyRef} />
    </aside>
  );
}
