/**
 * 「過去の完了」ダイアログのテスト。
 *
 * 見ているのは 3 点。
 * 1. 開いたら `list_archived_done` を引いて一覧にすること。
 * 2. 上限で切り詰められたときに、その旨を出すこと(黙って隠さない)。
 * 3. 行を押したら詳細ドロワーへ渡して自分は閉じること(操作はドロワー側の責務)。
 *
 * Tauri の `invoke` には触れず、`../api` をモジュールごと差し替える
 * (`store.test.ts` と同じ切り方)。
 */

import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as api from "../api";
import { useBoardStore } from "../store";
import { mount } from "../test-utils";
import type { ArchivedDone, TaskCard } from "../types";
import { ArchivedDoneDialog } from "./ArchivedDoneDialog";

vi.mock("../api");

/** 一覧が読むぶんだけ埋めたカード。 */
function card(id: string, doneAt: string): TaskCard {
  return { id, title: `タスク ${id}`, doneAt } as TaskCard;
}

function listed(overrides: Partial<ArchivedDone> = {}): ArchivedDone {
  return {
    tasks: [card("t1", "2026-08-31T10:00:00Z"), card("t2", "2026-08-30T10:00:00Z")],
    total: 2,
    limit: 200,
    ...overrides,
  };
}

/** ダイアログを開いて、最初のフェッチを消化する。 */
async function open(result: ArchivedDone = listed()) {
  vi.mocked(api.listArchivedDone).mockResolvedValue(result);
  const onClose = vi.fn();
  const mounted = mount(<ArchivedDoneDialog onClose={onClose} />);
  await act(async () => {});
  return {
    ...mounted,
    onClose,
    rows: () =>
      Array.from(mounted.container.querySelectorAll('[data-testid="archived-done-row"]')),
    text: () => mounted.container.textContent ?? "",
  };
}

beforeEach(() => {
  vi.mocked(api.listenTasksChanged).mockResolvedValue(() => undefined);
  useBoardStore.setState({ openTask: vi.fn() });
});

describe("ArchivedDoneDialog", () => {
  it("過去の完了を完了時刻つきで並べる", async () => {
    const view = await open();
    expect(api.listArchivedDone).toHaveBeenCalled();
    expect(view.rows()).toHaveLength(2);
    expect(view.rows()[0]?.textContent).toContain("タスク t1");
    expect(view.rows()[0]?.textContent).toContain("に完了");
    view.unmount();
  });

  it("1 件も無ければその旨を出す", async () => {
    const view = await open(listed({ tasks: [], total: 0 }));
    expect(view.rows()).toHaveLength(0);
    expect(view.text()).toContain("前日以前に完了したタスクはありません");
    view.unmount();
  });

  it("上限で切り詰められたら総件数を添える", async () => {
    const view = await open(listed({ total: 512, limit: 200 }));
    expect(view.text()).toContain("全 512 件");
    view.unmount();
  });

  it("行を押すと詳細を開いて自分は閉じる", async () => {
    const view = await open();
    act(() => view.rows()[0]?.querySelector("button")?.click());

    expect(useBoardStore.getState().openTask).toHaveBeenCalledWith("t1");
    expect(view.onClose).toHaveBeenCalled();
    view.unmount();
  });
});
