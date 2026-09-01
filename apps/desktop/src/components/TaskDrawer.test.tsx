/**
 * 関連リソース一覧(ドロワーの「関連リソース」節)のテスト。
 *
 * 見ているのは 3 つ。
 * 1. 主リソースのトグルが ★/☆ とラベルを付け替えること。
 * 2. ☆ を押すと「主にする」、★ を押すと「解除」が届くこと(表示だけの飾りではない)。
 * 3. 主リソースが 1 つも無い一覧でも壊れないこと(バックエンドが空を許す)。
 *
 * `ResourceList` は props のコールバックしか呼ばないので、
 * Tauri の command には触れない(`../api` も踏まない)。
 */

import { act } from "react";
import { describe, expect, it, vi } from "vitest";

import { mount } from "../test-utils";
import type { TaskResource } from "../types";
import { primaryToggle, ResourceList } from "./TaskDrawer";

function resource(id: string, isPrimary: boolean): TaskResource {
  return {
    id,
    taskId: "t1",
    kind: "url",
    value: `https://example.com/${id}`,
    label: "",
    isPrimary,
    sortOrder: id,
    createdAt: "2026-09-01T00:00:00Z",
  };
}

function setup(resources: TaskResource[]) {
  const onSetPrimary = vi.fn();
  const onRemove = vi.fn();
  const onOpenPane = vi.fn();
  const mounted = mount(
    <ResourceList
      resources={resources}
      urlOpenMode="external"
      onOpenPane={onOpenPane}
      onSetPrimary={onSetPrimary}
      onRemove={onRemove}
    />,
  );
  return {
    ...mounted,
    onSetPrimary,
    onRemove,
    onOpenPane,
    stars: () =>
      Array.from(
        mounted.container.querySelectorAll<HTMLButtonElement>(
          '[data-testid="resource-primary-toggle"]',
        ),
      ),
  };
}

describe("primaryToggle", () => {
  it("★ は解除、☆ は昇格として振る舞う", () => {
    expect(primaryToggle(true)).toEqual({
      symbol: "★",
      className: "star star-on",
      label: "主リソースを解除",
      next: false,
    });
    expect(primaryToggle(false)).toEqual({
      symbol: "☆",
      className: "star",
      label: "主リソースにする",
      next: true,
    });
  });
});

describe("ResourceList", () => {
  it("主リソースだけ ★ になり、ラベルと aria-pressed も入れ替わる", () => {
    const view = setup([resource("a", true), resource("b", false)]);
    const [primary, other] = view.stars();

    expect(primary.textContent).toBe("★");
    expect(primary.className).toContain("star-on");
    expect(primary.getAttribute("aria-label")).toBe("主リソースを解除");
    expect(primary.getAttribute("aria-pressed")).toBe("true");
    // ツールチップはラベルと同じ文言(「主リソース」の表示だけで終わらせない)。
    expect(primary.title).toBe("主リソースを解除");

    expect(other.textContent).toBe("☆");
    expect(other.className).not.toContain("star-on");
    expect(other.getAttribute("aria-label")).toBe("主リソースにする");
    expect(other.getAttribute("aria-pressed")).toBe("false");
    view.unmount();
  });

  it("☆ を押すと主に昇格、★ を押すと解除を送る", () => {
    const primary = resource("a", true);
    const other = resource("b", false);
    const view = setup([primary, other]);

    act(() => view.stars()[1].click());
    expect(view.onSetPrimary).toHaveBeenLastCalledWith(other, true);

    act(() => view.stars()[0].click());
    expect(view.onSetPrimary).toHaveBeenLastCalledWith(primary, false);
    expect(view.onSetPrimary).toHaveBeenCalledTimes(2);
    view.unmount();
  });

  it("主リソースが無くても描画でき、✕ は削除へ届く", () => {
    const only = resource("a", false);
    const view = setup([only]);
    expect(view.stars()).toHaveLength(1);
    expect(view.stars()[0].textContent).toBe("☆");

    act(() => {
      view.container.querySelector<HTMLButtonElement>('[aria-label="リソースを削除"]')?.click();
    });
    expect(view.onRemove).toHaveBeenCalledWith(only);
    view.unmount();
  });
});
