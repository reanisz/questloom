/**
 * オーバーレイ通知の折りたたみのテスト。
 *
 * 見ているのは 4 点。
 * 1. ヘッダを押すと折りたたまれ、インジケータを押すと戻ること。
 * 2. 折りたたみ状態が localStorage に残り、次の起動でも復元されること。
 * 3. 折りたたみ中も件数がライブ更新されること。
 * 4. **件数が増えても勝手に展開しないこと**(ユーザーの選択を尊重する)。
 *
 * Tauri には触れず、`../api` をモジュールごと差し替える(`ArchivedDoneDialog.test.tsx` と
 * 同じ切り方)。ウィンドウのリサイズは `@tauri-apps/api/window` を差し替えて観察する。
 */

import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import * as api from "../api";
import { mount } from "../test-utils";
import type { Board, TaskCard } from "../types";
import { OVERLAY_COLLAPSED_KEY, OverlayApp } from "./OverlayApp";

vi.mock("../api");

// vi.mock のファクトリはこのファイルの本体より先に走るので、hoisted で先に作る。
const { setSize } = vi.hoisted(() => ({
  setSize: vi.fn((_size: { width: number; height: number }) => Promise.resolve()),
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setSize }) }));

// jsdom には ResizeObserver が無い。observe された要素は測らないので空実装でよい。
class NoopResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as { ResizeObserver?: unknown }).ResizeObserver = NoopResizeObserver;

/** 一覧が読むぶんだけ埋めたカード。 */
function card(id: string): TaskCard {
  return { id, title: `タスク ${id}`, isInstant: true } as TaskCard;
}

/** New 列だけ埋めたボード。 */
function board(count: number): Board {
  const tasks = Array.from({ length: count }, (_, index) => card(`t${index + 1}`));
  return { columns: { new: tasks } } as unknown as Board;
}

/** `tasks-changed` のハンドラ。テストから発火して再フェッチさせる。 */
let fireTasksChanged: (() => void) | null = null;

beforeEach(() => {
  vi.mocked(api.getBoard).mockResolvedValue(board(1));
  vi.mocked(api.listenTasksChanged).mockImplementation((handler) => {
    fireTasksChanged = handler;
    return Promise.resolve(() => undefined);
  });
});

afterEach(() => {
  fireTasksChanged = null;
  setSize.mockClear();
  localStorage.clear();
  vi.restoreAllMocks();
});

/** 描画して最初のフェッチを消化する。 */
async function render() {
  const mounted = mount(<OverlayApp />);
  await act(async () => {});

  const find = (testId: string) =>
    mounted.container.querySelector<HTMLElement>(`[data-testid="${testId}"]`);

  return {
    ...mounted,
    head: () => find("overlay-collapse"),
    indicator: () => find("overlay-indicator"),
    rows: () => mounted.container.querySelectorAll(".overlay-row"),
    text: () => mounted.container.textContent ?? "",
    /** バックエンドの件数を変えて `tasks-changed` を流す。 */
    async change(count: number) {
      vi.mocked(api.getBoard).mockResolvedValue(board(count));
      await act(async () => {
        fireTasksChanged?.();
      });
    },
  };
}

describe("OverlayApp", () => {
  it("既定では展開していて、一覧を出す", async () => {
    const view = await render();
    expect(view.head()).not.toBeNull();
    expect(view.indicator()).toBeNull();
    expect(view.rows()).toHaveLength(1);
    view.unmount();
  });

  it("ヘッダを押すと折りたたみ、インジケータを押すと戻る", async () => {
    const view = await render();

    act(() => view.head()?.click());
    expect(view.head()).toBeNull();
    expect(view.rows()).toHaveLength(0);
    expect(view.indicator()?.textContent).toContain("1");

    act(() => view.indicator()?.click());
    expect(view.indicator()).toBeNull();
    expect(view.rows()).toHaveLength(1);

    view.unmount();
  });

  it("折りたたみ状態を localStorage に書き、次の描画で復元する", async () => {
    const first = await render();
    act(() => first.head()?.click());
    expect(localStorage.getItem(OVERLAY_COLLAPSED_KEY)).toBe("1");
    first.unmount();

    // 再起動相当。折りたたんだまま立ち上がる。
    const second = await render();
    expect(second.indicator()).not.toBeNull();
    expect(second.head()).toBeNull();

    act(() => second.indicator()?.click());
    expect(localStorage.getItem(OVERLAY_COLLAPSED_KEY)).toBe("0");
    second.unmount();
  });

  it("localStorage が読めなくても展開状態で立ち上がる", async () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("access denied", "SecurityError");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("quota exceeded", "QuotaExceededError");
    });

    const view = await render();
    expect(view.head()).not.toBeNull();
    // 保存が投げても折りたたみ自体は通る。
    act(() => view.head()?.click());
    expect(view.indicator()).not.toBeNull();
    view.unmount();
  });

  it("折りたたみ中も件数はライブ更新される", async () => {
    const view = await render();
    act(() => view.head()?.click());
    expect(view.indicator()?.textContent).toContain("1");

    await view.change(4);
    expect(view.indicator()?.textContent).toContain("4");

    await view.change(2);
    expect(view.indicator()?.textContent).toContain("2");

    view.unmount();
  });

  it("件数が増えても勝手には展開しない", async () => {
    const view = await render();
    act(() => view.head()?.click());

    await view.change(3);
    expect(view.indicator()).not.toBeNull();
    expect(view.head()).toBeNull();
    expect(view.rows()).toHaveLength(0);
    expect(localStorage.getItem(OVERLAY_COLLAPSED_KEY)).toBe("1");

    view.unmount();
  });

  it("件数が増えたときだけパルスを付ける", async () => {
    const view = await render();
    act(() => view.head()?.click());
    const count = () =>
      view.container.querySelector<HTMLElement>('[data-testid="overlay-indicator-count"]');
    expect(count()?.className).not.toContain("is-pulse");

    await view.change(3);
    expect(count()?.className).toContain("is-pulse");

    view.unmount();
  });

  it("New タスクが 0 件なら何も描かない", async () => {
    vi.mocked(api.getBoard).mockResolvedValue(board(0));
    const view = await render();
    expect(view.head()).toBeNull();
    expect(view.indicator()).toBeNull();
    view.unmount();
  });

  it("折りたたむとウィンドウを幅ごと縮める", async () => {
    const view = await render();
    const expanded = setSize.mock.calls.at(-1)?.[0];
    expect(expanded?.width).toBe(360);

    setSize.mockClear();
    act(() => view.head()?.click());
    const collapsed = setSize.mock.calls.at(-1)?.[0];
    // jsdom は実寸を返さないので、幅が展開時の固定値から外れることだけを見る。
    expect(collapsed?.width).toBeLessThan(360);

    view.unmount();
  });
});
