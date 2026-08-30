/**
 * フロントの状態管理 (zustand)。
 *
 * 真実の情報源はバックエンド。書き込み系はすべて invoke → 再フェッチで反映する
 * (加えて `questloom://tasks-changed` でも再フェッチされる)。
 * ドラッグ&ドロップのみ、体感を保つために移動先へカードを先に動かす楽観的更新を行い、
 * 直後の再フェッチで正しい状態に上書きされる。
 */

import { create } from "zustand";

import * as api from "./api";
import type { Board, BoardColumnKey, TaskCard, TaskDetail, TaskId } from "./types";

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

  refresh: () => Promise<void>;
  openTask: (taskId: TaskId) => void;
  closeTask: () => void;
  setError: (error: string | null) => void;
  /** 書き込み系の共通ラッパー。失敗時はエラーを表示し false を返す。 */
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
      set({ error: api.toMessage(error), ready: true });
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

  async mutate(action) {
    try {
      await action();
      await get().refresh();
      return true;
    } catch (error) {
      set({ error: api.toMessage(error) });
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
