/**
 * キーボード操作の共通実装。
 *
 * ## Esc のレイヤースタック
 *
 * ドロワー・モーダル・ポップアップメニューはそれぞれ「Esc で閉じる」を持つが、
 * 個別に `document.addEventListener("keydown")` を張ると、重なって表示している
 * ときに 1 回の Esc で全部閉じてしまう。そこでリスナはこのモジュールが 1 本だけ
 * 張り、開いている閉じ手をモジュールスコープのスタックで管理して、
 * **最前面のレイヤーだけ**に Esc を配る。
 *
 * - マウント(正確には `enabled` が真になった時点)で push、アンマウントで pop。
 * - 同じ priority ならあとから開いた方が上。priority は重なり順 ([`ESC_LAYER`]) に対応し、
 *   「ドロワーを開いたままモーダルを出す」ような順序の入れ替わりでも見た目と一致させる。
 * - 文字入力中(値の入った input / textarea、または select にフォーカス)の Esc は
 *   どのレイヤーへも配らない。入力途中のテキストをダイアログごと失わせないため。
 *
 * 内蔵ブラウザペイン(別 webview)で押された Esc はこの document には届かないので、
 * Rust から中継されたイベントを [`dispatchEscape`] で同じスタックへ流し込む。
 */

import { useEffect, useRef, type KeyboardEvent as ReactKeyboardEvent } from "react";

/**
 * Esc を受け取る優先度。値は styles.css の z-index の重なり順に対応する。
 *
 * 同じ値なら「あとから開いた方」が優先される。
 */
export const ESC_LAYER = {
  /** ボードを置き換えるページ(設定画面など)。最下層。 */
  page: 0,
  /** タスク詳細ドロワー (z-index 50/51)。 */
  drawer: 10,
  /** モーダルダイアログ (z-index 60/61)。 */
  modal: 20,
  /** ポップアップメニュー。常に呼び出し元より前面にある。 */
  popup: 30,
} as const;

/** [`useEscapeKey`] のオプション。 */
export interface EscapeKeyOptions {
  /**
   * 偽の間はレイヤーに参加しない(Esc は 1 つ下のレイヤーへ渡る)。
   * 既定は真。開閉を props で持つコンポーネントから使う。
   */
  enabled?: boolean;
  /** 重なり順。既定は [`ESC_LAYER.modal`]。 */
  priority?: number;
}

/** スタックに積まれた 1 レイヤー。 */
interface EscapeLayer {
  priority: number;
  /** push した順番。同 priority の比較に使う。 */
  seq: number;
  /** 最新の onClose を読むための参照(再購読なしで差し替えられるように)。 */
  handler: { current: () => void };
}

const layers: EscapeLayer[] = [];
let sequence = 0;
let listening = false;

/** テキストを持ちうる input の type。checkbox などは対象外。 */
const TEXT_INPUT_TYPES = new Set([
  "text",
  "search",
  "url",
  "tel",
  "email",
  "password",
  "number",
  "date",
  "datetime-local",
  "month",
  "week",
  "time",
]);

/**
 * Esc を握りつぶすべき入力中か。
 *
 * - 値の入った input / textarea: 入力を捨てさせないため閉じない
 *   (要素自身の Escape ハンドラ、例えば「編集前に戻す」は動く)。
 * - select: Esc はドロップダウンの取り消しに使われる。
 */
function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target instanceof HTMLSelectElement) return true;
  if (target instanceof HTMLTextAreaElement) return target.value !== "";
  if (target instanceof HTMLInputElement) {
    return TEXT_INPUT_TYPES.has(target.type) && target.value !== "";
  }
  return false;
}

/** 最前面のレイヤー(priority が最大、同点ならあとから積まれた方)。 */
function topLayer(): EscapeLayer | null {
  let top: EscapeLayer | null = null;
  for (const layer of layers) {
    if (!top || layer.priority > top.priority) {
      top = layer;
    } else if (layer.priority === top.priority && layer.seq > top.seq) {
      top = layer;
    }
  }
  return top;
}

function onKeyDown(event: KeyboardEvent): void {
  if (event.key !== "Escape" || event.defaultPrevented) return;
  if (isTypingTarget(event.target)) return;
  dispatchEscape();
}

/**
 * 最前面のレイヤーへ Esc を配る。配る相手がいれば真、レイヤーが空なら偽を返す。
 *
 * キーボードから来た Esc(このモジュールの document リスナ)と、
 * **内蔵ブラウザペインから中継された Esc** の共通の出口。ペイン側のキー入力は
 * 別 webview なので document には届かず、Rust の `browser_pane_escape` command が
 * `questloom://browser-pane-escape` として送り直したものを
 * [`BrowserPane`](./components/BrowserPane.tsx) がここへ流し込む。
 *
 * **入力中(`isTypingTarget`)の判定はしない。** あれは「questloom の入力欄に
 * 書きかけのテキストがある」ときの話で、フォーカスが別 webview へ移っている
 * ペイン由来の Esc には当てはまらない(document 側のリスナは呼ぶ前に判定済み)。
 */
export function dispatchEscape(): boolean {
  const layer = topLayer();
  if (!layer) return false;
  layer.handler.current();
  return true;
}

/** レイヤーが 1 つでもある間だけ document のリスナを張る。 */
function syncListener(): void {
  const wanted = layers.length > 0;
  if (wanted === listening) return;
  listening = wanted;
  if (wanted) document.addEventListener("keydown", onKeyDown);
  else document.removeEventListener("keydown", onKeyDown);
}

/**
 * Esc で閉じるレイヤーを登録する。最前面のレイヤーだけが `onClose` を呼ばれる。
 *
 * `onClose` は毎レンダー差し替わってよい(参照で保持するため再購読は起きない)。
 * 「実行中は閉じない」といった条件は `onClose` の中で判定すること。そうすれば
 * Esc はこのレイヤーで止まり、下のドロワーなどが道連れに閉じない。
 */
export function useEscapeKey(onClose: () => void, options?: EscapeKeyOptions): void {
  const enabled = options?.enabled ?? true;
  const priority = options?.priority ?? ESC_LAYER.modal;

  const handler = useRef(onClose);
  handler.current = onClose;

  useEffect(() => {
    if (!enabled) return;
    sequence += 1;
    const layer: EscapeLayer = { priority, seq: sequence, handler };
    layers.push(layer);
    syncListener();
    return () => {
      const at = layers.indexOf(layer);
      if (at >= 0) layers.splice(at, 1);
      syncListener();
    };
  }, [enabled, priority]);
}

/**
 * Ctrl+Enter(macOS では Cmd+Enter)で送信する onKeyDown ハンドラを作る。
 *
 * テキストエリアの改行と送信を両立させるための定型。
 */
export function onCtrlEnter<T>(submit: () => void) {
  return (event: ReactKeyboardEvent<T>): void => {
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      submit();
    }
  };
}
