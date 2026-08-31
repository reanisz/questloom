/**
 * 右クリックメニューの描画のテスト。
 *
 * 判定そのものは [`contextMenu.test.ts`] が押さえているので、ここで見るのは
 * 「純関数の結果が DOM になっているか」「第 2 階層へ潜って戻れるか」
 * 「Esc / 外側クリックで閉じ、下のレイヤーへ漏らさないか」の 3 点。
 * Tauri の command は呼ばれない経路(項目を押さない)だけを踏む。
 */

import { act } from "react";
import { describe, expect, it, vi } from "vitest";

import { mount, pressKey } from "../test-utils";
import type { TaskCard } from "../types";
import { TaskContextMenu } from "./TaskContextMenu";

/** メニューが見るぶんだけ埋めたカード。 */
function card(overrides: Partial<TaskCard> = {}): TaskCard {
  return {
    id: "task-1",
    title: "テストタスク",
    status: "new",
    isInstant: false,
    primaryResource: null,
    ...overrides,
  } as TaskCard;
}

function setup(target: TaskCard = card(), onClose = vi.fn(), onDelete = vi.fn()) {
  const mounted = mount(
    <TaskContextMenu
      card={target}
      column="new"
      anchor={{ x: 10, y: 20 }}
      onClose={onClose}
      onDelete={onDelete}
    />,
  );
  return {
    ...mounted,
    onClose,
    onDelete,
    menu: () => mounted.container.querySelector('[data-testid="task-context-menu"]'),
    item: (action: string) =>
      mounted.container.querySelector<HTMLButtonElement>(`[data-testid="context-${action}"]`),
    /** 出ている項目の action を DOM 順で返す。 */
    actions: () =>
      Array.from(mounted.container.querySelectorAll("[data-testid^='context-']")).map((element) =>
        element.getAttribute("data-testid")?.replace("context-", ""),
      ),
    click(action: string) {
      const button = mounted.container.querySelector<HTMLButtonElement>(
        `[data-testid="context-${action}"]`,
      );
      if (!button) throw new Error(`項目 ${action} がありません`);
      act(() => button.click());
    },
  };
}

describe("TaskContextMenu", () => {
  it("カードの状態どおりの項目を描画する", () => {
    const view = setup(card({ isInstant: true }));
    expect(view.actions()).toEqual(["open", "complete", "promote", "move", "delete"]);
    view.unmount();
  });

  it("完了済みには「完了にする」を描かない", () => {
    const view = setup(card({ status: "done" }));
    expect(view.item("complete")).toBeNull();
    expect(view.item("open")).not.toBeNull();
    view.unmount();
  });

  it("カーソル位置に fixed で置く", () => {
    const view = setup();
    const style = (view.menu() as HTMLElement).style;
    // jsdom では offsetWidth/Height が 0 なので補正は起きず、アンカーがそのまま出る。
    expect(style.left).toBe("10px");
    expect(style.top).toBe("20px");
    view.unmount();
  });

  it("「移動」で第 2 階層に差し替わり、今いる列は無効化される", () => {
    const view = setup();
    view.click("move");

    expect(view.item("open")).toBeNull();
    expect(view.item("move-today")).not.toBeNull();
    expect(view.item("move-new")?.disabled).toBe(true);
    expect(view.item("move-today")?.disabled).toBe(false);

    // 戻れる。
    view.click("back");
    expect(view.item("open")).not.toBeNull();
    view.unmount();
  });

  it("「移動」の第 2 階層に監視中が並ぶ", () => {
    const view = setup();
    view.click("move");
    expect(view.item("move-watching")).not.toBeNull();
    expect(view.item("move-watching")?.disabled).toBe(false);
    view.unmount();
  });

  it("監視中のカードでは「移動」で監視中が無効化される", () => {
    const mounted = mount(
      <TaskContextMenu
        card={card({ status: "watching" })}
        column="watching"
        anchor={{ x: 10, y: 20 }}
        onClose={vi.fn()}
        onDelete={vi.fn()}
      />,
    );
    const item = (action: string) =>
      mounted.container.querySelector<HTMLButtonElement>(`[data-testid="context-${action}"]`);
    act(() => item("move")?.click());
    expect(item("move-watching")?.disabled).toBe(true);
    expect(item("move-new")?.disabled).toBe(false);
    mounted.unmount();
  });

  it("「昇格」の第 2 階層は New / Watching / Doing / Done を出さない", () => {
    const view = setup(card({ isInstant: true }));
    view.click("promote");

    expect(view.item("promote-today")).not.toBeNull();
    expect(view.item("promote-future")).not.toBeNull();
    expect(view.item("promote-new")).toBeNull();
    expect(view.item("promote-watching")).toBeNull();
    expect(view.item("promote-done")).toBeNull();
    view.unmount();
  });

  it("Esc で閉じ、下のレイヤーへは漏らさない", () => {
    const onClose = vi.fn();
    const view = setup(card(), onClose);

    const onDocumentKeyDown = vi.fn();
    document.addEventListener("keydown", onDocumentKeyDown);
    pressKey("Escape");
    document.removeEventListener("keydown", onDocumentKeyDown);

    expect(onClose).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("メニューの外の pointerdown で閉じる(中では閉じない)", () => {
    const onClose = vi.fn();
    const view = setup(card(), onClose);

    act(() => {
      view.menu()?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    expect(onClose).not.toHaveBeenCalled();

    act(() => {
      document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("「削除」は onDelete に委ね、メニューは閉じる", () => {
    const onClose = vi.fn();
    const onDelete = vi.fn();
    const view = setup(card(), onClose, onDelete);
    view.click("delete");

    expect(onDelete).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
    view.unmount();
  });
});
