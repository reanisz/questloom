/**
 * フロントの状態管理 (zustand)。
 *
 * 真実の情報源はバックエンド。**書き込みの結果は自前で再フェッチせず、
 * バックエンドが出す `questloom://tasks-changed` を受けて反映する**
 * (購読は App.tsx / OverlayApp.tsx)。書き込みごとに再フェッチも回すと
 * 1 回の変更で `get_board` が 2 回走るため、イベント駆動に一本化してある。
 * 取りこぼし (`missed > 0`) の場合もイベント自体は届くので再フェッチされる。
 *
 * 失敗時だけは、楽観的更新を巻き戻すためにその場で再フェッチする。
 * ドラッグ&ドロップのみ、体感を保つために移動先へカードを先に動かす楽観的更新を行い、
 * イベントで届く正しい状態に上書きされる。
 */

import { create } from "zustand";

import * as api from "./api";
import { toMessage } from "./tauri";
import type { Board, BoardColumnKey, TaskCard, TaskDetail, TaskId, UrlOpenMode } from "./types";

/** 再フェッチの世代番号。古い応答で新しい状態を上書きしないためのガード。 */
let generation = 0;

interface BoardState {
  board: Board | null;
  detail: TaskDetail | null;
  /** 詳細ドロワーで開いているタスク。null ならドロワーは閉じている。 */
  selectedId: TaskId | null;
  /** 初回ロードが完了したか。 */
  ready: boolean;
  /** 直近のエラーメッセージ(バックエンドの日本語文字列)。 */
  error: string | null;

  /**
   * 内蔵ブラウザペインで開いている URL。null なら閉じている。
   *
   * 実体は main ウィンドウの子 webview で、生成・破棄は
   * [`BrowserPane`](./components/BrowserPane.tsx) が担う。ここが持つのは
   * 「どの URL を開くか」だけ。
   */
  paneUrl: string | null;
  /**
   * ペインを覆う UI(ドロワー・モーダル・右クリックメニュー)の数。
   *
   * 子 webview はネイティブの子ウィンドウで、HTML より必ず前面に描かれる。
   * 0 でない間はペインを隠して、後ろに回り込んだ UI が操作できなくならないようにする。
   */
  paneOccluders: number;
  /** URL リソースをクリックしたときの既定の開き方(コア設定の写し)。 */
  urlOpenMode: UrlOpenMode;

  refresh: () => Promise<void>;
  openTask: (taskId: TaskId) => void;
  closeTask: () => void;
  setError: (error: string | null) => void;
  /** 内蔵ブラウザペインを開く(既に開いていれば URL を差し替える)。 */
  openPane: (url: string) => void;
  /** 内蔵ブラウザペインを閉じる。 */
  closePane: () => void;
  /** ペインを覆う UI の増減を伝える。 */
  occludePane: (delta: 1 | -1) => void;
  /** コア設定から `urlOpenMode` を読み直す。 */
  loadUrlOpenMode: () => Promise<void>;
  /**
   * 書き込み系の共通ラッパー。成功時の反映は tasks-changed イベントに任せる。
   * 失敗時はエラーを表示し、状態を読み直して false を返す。
   */
  mutate: (action: () => Promise<unknown>) => Promise<boolean>;
  /** ドロップ直後にカードをローカルで移動する(再フェッチで上書きされる)。 */
  applyLocalMove: (
    taskId: TaskId,
    column: BoardColumnKey,
    prevId: TaskId | null,
    nextId: TaskId | null,
  ) => void;
}

/** 指定カードを列から取り除いた新しい columns を返す。 */
function withoutCard(columns: Board["columns"], taskId: TaskId) {
  const next = {} as Board["columns"];
  let removed: TaskCard | null = null;
  for (const [key, cards] of Object.entries(columns) as [BoardColumnKey, TaskCard[]][]) {
    const found = cards.find((card) => card.id === taskId);
    if (found) removed = found;
    next[key] = found ? cards.filter((card) => card.id !== taskId) : cards;
  }
  return { columns: next, removed };
}

export const useBoardStore = create<BoardState>()((set, get) => ({
  board: null,
  detail: null,
  selectedId: null,
  ready: false,
  error: null,
  paneUrl: null,
  paneOccluders: 0,
  urlOpenMode: "external",

  async refresh() {
    const token = ++generation;
    const selectedId = get().selectedId;
    try {
      const [board, detail] = await Promise.all([
        api.getBoard(),
        selectedId ? api.getTask(selectedId).catch(() => null) : Promise.resolve(null),
      ]);
      if (token !== generation) return;
      set((state) => ({
        board,
        // フェッチ中に開き直された場合は詳細を捨てる。
        detail: state.selectedId === selectedId ? detail : state.detail,
        ready: true,
      }));
    } catch (error) {
      if (token !== generation) return;
      set({ error: toMessage(error), ready: true });
    }
  },

  openTask(taskId) {
    set({ selectedId: taskId, detail: null });
    void get().refresh();
  },

  closeTask() {
    set({ selectedId: null, detail: null });
  },

  setError(error) {
    set({ error });
  },

  openPane(url) {
    set({ paneUrl: url });
  },

  closePane() {
    set({ paneUrl: null });
  },

  occludePane(delta) {
    // 下限 0。開閉が入れ違っても負にしない(負になるとずっと隠れたままになる)。
    set((state) => ({ paneOccluders: Math.max(0, state.paneOccluders + delta) }));
  },

  async loadUrlOpenMode() {
    try {
      const settings = await api.getSettings();
      set({ urlOpenMode: settings.urlOpenMode });
    } catch {
      // 設定が読めなくても既定(外部ブラウザ)で動けるので、ここでは何も出さない。
    }
  },

  async mutate(action) {
    try {
      await action();
      // 成功時はここで再フェッチしない。バックエンドの tasks-changed が引き金になる。
      return true;
    } catch (error) {
      set({ error: toMessage(error) });
      // 失敗時はイベントが来ないので、楽観的更新を巻き戻すために読み直す。
      await get().refresh();
      return false;
    }
  },

  applyLocalMove(taskId, column, prevId, nextId) {
    const board = get().board;
    if (!board) return;
    const { columns, removed } = withoutCard(board.columns, taskId);
    if (!removed) return;

    const target = [...columns[column]];
    let index = target.length;
    if (prevId) {
      const at = target.findIndex((card) => card.id === prevId);
      if (at >= 0) index = at + 1;
    } else if (nextId) {
      const at = target.findIndex((card) => card.id === nextId);
      index = at >= 0 ? at : 0;
    } else {
      index = target.length;
    }
    target.splice(index, 0, removed);

    set({ board: { ...board, columns: { ...columns, [column]: target } } });
  },
}));
