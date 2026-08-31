/**
 * 右クリックメニューの純粋なロジックのテスト。
 *
 * 見るのは 2 つ。
 * - 項目の出し分け(完了済み・インスタント・主リソースの有無で変わる)。
 * - はみ出し補正(右端・下端では折り返し、画面より大きいときは端に寄せる)。
 */

import { describe, expect, it } from "vitest";

import type { TaskCard, TaskResource } from "../types";
import { clampMenuPosition, contextMenuActions, type ContextMenuTarget } from "./contextMenu";

/** 判定に効く属性だけを持つカード。 */
function target(overrides: Partial<ContextMenuTarget> = {}): ContextMenuTarget {
  return { status: "new", isInstant: false, primaryResource: null, ...overrides };
}

/** 主リソース。kind 以外は判定に使われない。 */
function resource(kind: TaskResource["kind"]): TaskCard["primaryResource"] {
  return { kind, value: "https://example.com" } as TaskResource;
}

/** 400x300 の画面。 */
const viewport = { width: 400, height: 300 };
/** 100x120 のメニュー。 */
const size = { width: 100, height: 120 };

describe("contextMenuActions", () => {
  it("通常の未完了タスクでは 詳細 / 完了 / 移動 / 削除 をこの順で出す", () => {
    expect(contextMenuActions(target())).toEqual(["open", "complete", "move", "delete"]);
  });

  it("完了済みには「完了にする」を出さない", () => {
    expect(contextMenuActions(target({ status: "done" }))).toEqual(["open", "move", "delete"]);
  });

  it("インスタントタスクにだけ「昇格」を出す(完了の次、移動の前)", () => {
    expect(contextMenuActions(target({ isInstant: true }))).toEqual([
      "open",
      "complete",
      "promote",
      "move",
      "delete",
    ]);
  });

  it("完了済みのインスタントタスクでも昇格は出す", () => {
    expect(contextMenuActions(target({ isInstant: true, status: "done" }))).toEqual([
      "open",
      "promote",
      "move",
      "delete",
    ]);
  });

  it("監視中のタスクは未完了なので、通常タスクと同じ項目を出す", () => {
    expect(contextMenuActions(target({ status: "watching" }))).toEqual([
      "open",
      "complete",
      "move",
      "delete",
    ]);
  });

  it("主リソースが URL のときだけ「URL を開く」を出す", () => {
    expect(contextMenuActions(target({ primaryResource: resource("url") }))).toContain("url");
  });

  it("主リソースがファイルなら「URL を開く」は出さない", () => {
    expect(contextMenuActions(target({ primaryResource: resource("file") }))).not.toContain("url");
  });

  it("削除は常に最後", () => {
    for (const card of [
      target(),
      target({ status: "done" }),
      target({ isInstant: true, primaryResource: resource("url") }),
    ]) {
      const actions = contextMenuActions(card);
      expect(actions[actions.length - 1]).toBe("delete");
    }
  });
});

describe("clampMenuPosition", () => {
  it("余裕があればカーソル位置をそのまま使う", () => {
    expect(clampMenuPosition({ x: 50, y: 60 }, size, viewport)).toEqual({ x: 50, y: 60 });
  });

  it("右端では左へ折り返す", () => {
    expect(clampMenuPosition({ x: 380, y: 60 }, size, viewport)).toEqual({ x: 280, y: 60 });
  });

  it("下端では上へ折り返す", () => {
    expect(clampMenuPosition({ x: 50, y: 290 }, size, viewport)).toEqual({ x: 50, y: 170 });
  });

  it("右下の隅では両方向へ折り返す", () => {
    expect(clampMenuPosition({ x: 395, y: 295 }, size, viewport)).toEqual({ x: 295, y: 175 });
  });

  it("ちょうど収まる位置では折り返さない(境界)", () => {
    // x + width + margin === viewport.width。
    expect(clampMenuPosition({ x: 292, y: 60 }, size, viewport).x).toBe(292);
    // 1px はみ出したら折り返す。
    expect(clampMenuPosition({ x: 293, y: 60 }, size, viewport).x).toBe(193);
  });

  it("左上の隅では余白ぶんだけ押し戻す", () => {
    expect(clampMenuPosition({ x: 0, y: 2 }, size, viewport)).toEqual({ x: 8, y: 8 });
  });

  it("折り返しても入らない(画面より大きい)ときは端に寄せる", () => {
    const huge = { width: 500, height: 400 };
    expect(clampMenuPosition({ x: 200, y: 150 }, huge, viewport)).toEqual({ x: 8, y: 8 });
  });

  it("余白は差し替えられる", () => {
    expect(clampMenuPosition({ x: 0, y: 0 }, size, viewport, 0)).toEqual({ x: 0, y: 0 });
  });
});
