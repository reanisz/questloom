/**
 * ボードストアのテスト。
 *
 * 見ているのは 3 つ。
 * 1. `applyLocalMove` の挿入位置(ドロップ直後の楽観的更新)。
 * 2. `refresh` の世代ガード — **遅れて返ってきた古い応答が、新しい状態を上書きしない**こと。
 * 3. `mutate` の巻き戻し — 失敗時はイベントが来ないので自分で読み直すこと。
 *
 * Tauri の `invoke` には触れず、`./api` をモジュールごと差し替える
 * (`api.ts` は `invoke` の型付きラッパでしかないため、ここで境界を切るのが素直)。
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

import * as api from "./api";
import { useBoardStore } from "./store";
import type { Board, BoardColumnKey, TaskCard, TaskDetail, TaskId } from "./types";

vi.mock("./api");

/** id だけ意味のあるカードを作る。 */
function card(id: TaskId): TaskCard {
  return {
    id,
    title: id,
    description: "",
    status: "todo",
    scheduled: { kind: "none" },
    deadline: null,
    isInstant: false,
    origin: "user",
    parentId: null,
    sortOrder: id,
    createdAt: "2026-08-31T00:00:00Z",
    updatedAt: "2026-08-31T00:00:00Z",
    doneAt: null,
    deletedAt: null,
    bucket: null,
    childCount: 0,
    resourceCount: 0,
    primaryResource: null,
  };
}

/** 指定した列だけカードを持つボード。 */
function board(columns: Partial<Record<BoardColumnKey, string[]>>): Board {
  const empty: Record<BoardColumnKey, TaskCard[]> = {
    new: [],
    today: [],
    tomorrow: [],
    thisWeek: [],
    nextWeek: [],
    future: [],
    doing: [],
    done: [],
  };
  for (const [key, ids] of Object.entries(columns) as [BoardColumnKey, string[]][]) {
    empty[key] = ids.map(card);
  }
  return { today: "2026-08-31", weekStart: "monday", columns: empty };
}

/** 列の並びを id だけで読む。 */
function ids(column: BoardColumnKey): string[] {
  return (useBoardStore.getState().board?.columns[column] ?? []).map((item) => item.id);
}

/** ストアを初期状態に戻す。 */
function reset(initial: Partial<ReturnType<typeof useBoardStore.getState>> = {}) {
  useBoardStore.setState({
    board: null,
    detail: null,
    selectedId: null,
    ready: false,
    error: null,
    ...initial,
  });
}

beforeEach(() => {
  vi.mocked(api.getBoard).mockReset();
  vi.mocked(api.getTask).mockReset();
  reset();
});

describe("applyLocalMove", () => {
  beforeEach(() => {
    reset({ board: board({ today: ["a", "b", "c"], doing: ["x", "y"] }) });
  });

  it("prevId の直後へ差し込む", () => {
    useBoardStore.getState().applyLocalMove("c", "today", "a", "b");
    expect(ids("today")).toEqual(["a", "c", "b"]);
  });

  it("prevId が null なら nextId の直前へ差し込む", () => {
    useBoardStore.getState().applyLocalMove("c", "today", null, "a");
    expect(ids("today")).toEqual(["c", "a", "b"]);
  });

  it("前後とも null なら列の末尾へ", () => {
    useBoardStore.getState().applyLocalMove("a", "today", null, null);
    expect(ids("today")).toEqual(["b", "c", "a"]);
  });

  it("列をまたぐと移動元から消えて移動先に入る", () => {
    useBoardStore.getState().applyLocalMove("b", "doing", "x", "y");
    expect(ids("today")).toEqual(["a", "c"]);
    expect(ids("doing")).toEqual(["x", "b", "y"]);
  });

  it("prevId が移動先に無ければ末尾へ落とす", () => {
    useBoardStore.getState().applyLocalMove("a", "doing", "存在しない", null);
    expect(ids("doing")).toEqual(["x", "y", "a"]);
  });

  it("nextId が移動先に無ければ先頭へ落とす", () => {
    useBoardStore.getState().applyLocalMove("a", "doing", null, "存在しない");
    expect(ids("doing")).toEqual(["a", "x", "y"]);
  });

  it("ボードに無いカードは何も動かさない", () => {
    useBoardStore.getState().applyLocalMove("z", "doing", null, null);
    expect(ids("today")).toEqual(["a", "b", "c"]);
    expect(ids("doing")).toEqual(["x", "y"]);
  });

  it("ボード未取得なら何もしない(落ちない)", () => {
    reset();
    useBoardStore.getState().applyLocalMove("a", "doing", null, null);
    expect(useBoardStore.getState().board).toBeNull();
  });
});

describe("refresh の世代ガード", () => {
  it("先に投げた遅い応答は、あとから返った新しい状態を上書きしない", async () => {
    const slow = board({ today: ["古い"] });
    const fresh = board({ today: ["新しい"] });

    let releaseSlow!: (value: Board) => void;
    vi.mocked(api.getBoard)
      .mockImplementationOnce(
        () =>
          new Promise<Board>((resolve) => {
            releaseSlow = resolve;
          }),
      )
      .mockResolvedValueOnce(fresh);

    const first = useBoardStore.getState().refresh();
    const second = useBoardStore.getState().refresh();

    await second;
    expect(ids("today")).toEqual(["新しい"]);

    // 1 本目がようやく返ってくる。世代が古いので捨てられる。
    releaseSlow(slow);
    await first;
    expect(ids("today")).toEqual(["新しい"]);
  });

  it("古い応答のエラーで新しい状態を壊さない", async () => {
    let rejectSlow!: (error: unknown) => void;
    vi.mocked(api.getBoard)
      .mockImplementationOnce(
        () =>
          new Promise<Board>((_resolve, reject) => {
            rejectSlow = reject;
          }),
      )
      .mockResolvedValueOnce(board({ today: ["新しい"] }));

    const first = useBoardStore.getState().refresh();
    await useBoardStore.getState().refresh();

    rejectSlow("接続できません");
    await first;
    expect(useBoardStore.getState().error).toBeNull();
    expect(ids("today")).toEqual(["新しい"]);
  });

  it("最新の応答なら反映し、ready を立てる", async () => {
    vi.mocked(api.getBoard).mockResolvedValue(board({ new: ["n"] }));
    await useBoardStore.getState().refresh();
    expect(ids("new")).toEqual(["n"]);
    expect(useBoardStore.getState().ready).toBe(true);
    expect(useBoardStore.getState().error).toBeNull();
  });

  it("最新の応答が失敗したらエラーを出しつつ ready にする", async () => {
    vi.mocked(api.getBoard).mockRejectedValue(new Error("ボードを取得できません"));
    await useBoardStore.getState().refresh();
    expect(useBoardStore.getState().error).toBe("ボードを取得できません");
    expect(useBoardStore.getState().ready).toBe(true);
  });

  it("開いているタスクがあれば詳細も一緒に取り直す", async () => {
    const detail = { ...card("a"), resources: [], updates: [], parent: null, children: [] };
    reset({ selectedId: "a" });
    vi.mocked(api.getBoard).mockResolvedValue(board({ today: ["a"] }));
    vi.mocked(api.getTask).mockResolvedValue(detail as TaskDetail);

    await useBoardStore.getState().refresh();
    expect(api.getTask).toHaveBeenCalledWith("a");
    expect(useBoardStore.getState().detail).toEqual(detail);
  });

  it("詳細の取得だけ失敗してもボードは反映する", async () => {
    reset({ selectedId: "a" });
    vi.mocked(api.getBoard).mockResolvedValue(board({ today: ["a"] }));
    vi.mocked(api.getTask).mockRejectedValue(new Error("消えている"));

    await useBoardStore.getState().refresh();
    expect(ids("today")).toEqual(["a"]);
    expect(useBoardStore.getState().detail).toBeNull();
    expect(useBoardStore.getState().error).toBeNull();
  });

  it("フェッチ中に開き直されたら、届いた詳細は捨てる", async () => {
    reset({ selectedId: "a" });
    vi.mocked(api.getBoard).mockResolvedValue(board({ today: ["a", "b"] }));
    vi.mocked(api.getTask).mockImplementation(async (taskId) => {
      // 応答を待っている間にユーザーが別のタスクを開いた。
      useBoardStore.setState({ selectedId: "b" });
      return { ...card(taskId), resources: [], updates: [], parent: null, children: [] };
    });

    await useBoardStore.getState().refresh();
    expect(useBoardStore.getState().selectedId).toBe("b");
    expect(useBoardStore.getState().detail).toBeNull();
  });
});

describe("mutate", () => {
  it("成功したら再フェッチしない(反映は tasks-changed に任せる)", async () => {
    const ok = await useBoardStore.getState().mutate(async () => "done");
    expect(ok).toBe(true);
    expect(api.getBoard).not.toHaveBeenCalled();
    expect(useBoardStore.getState().error).toBeNull();
  });

  it("失敗したらエラーを出し、楽観的更新を巻き戻すために読み直す", async () => {
    reset({ board: board({ today: ["a", "b"] }) });
    vi.mocked(api.getBoard).mockResolvedValue(board({ today: ["b", "a"] }));

    const ok = await useBoardStore
      .getState()
      .mutate(async () => {
        // 楽観的更新をしたあとで書き込みが失敗した、という筋書き。
        useBoardStore.getState().applyLocalMove("a", "today", "b", null);
        throw new Error("移動できません");
      });

    expect(ok).toBe(false);
    expect(useBoardStore.getState().error).toBe("移動できません");
    expect(api.getBoard).toHaveBeenCalledTimes(1);
    expect(ids("today")).toEqual(["b", "a"]);
  });

  it("文字列で reject されてもメッセージとして読める", async () => {
    vi.mocked(api.getBoard).mockResolvedValue(board({}));
    await useBoardStore.getState().mutate(() => Promise.reject("タスクが見つかりません"));
    expect(useBoardStore.getState().error).toBe("タスクが見つかりません");
  });
});

describe("openTask / closeTask", () => {
  it("開くと詳細を捨てて読み直す", async () => {
    vi.mocked(api.getBoard).mockResolvedValue(board({ today: ["a"] }));
    vi.mocked(api.getTask).mockResolvedValue({
      ...card("a"),
      resources: [],
      updates: [],
      parent: null,
      children: [],
    } as TaskDetail);

    useBoardStore.getState().openTask("a");
    expect(useBoardStore.getState().selectedId).toBe("a");
    expect(useBoardStore.getState().detail).toBeNull();

    // refresh は openTask の中で投げっぱなしなので、決着を待ってから見る。
    await vi.waitFor(() => expect(useBoardStore.getState().detail).not.toBeNull());
  });

  it("閉じると選択も詳細も落とす", () => {
    reset({ selectedId: "a", detail: {} as TaskDetail });
    useBoardStore.getState().closeTask();
    expect(useBoardStore.getState().selectedId).toBeNull();
    expect(useBoardStore.getState().detail).toBeNull();
  });
});
