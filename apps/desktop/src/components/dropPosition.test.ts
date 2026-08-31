/**
 * D&D 着地点計算のテスト。
 *
 * ドロップ先のカード「より上か下か」の判定は、掴んでいる矩形の**中心**と
 * 相手の矩形の**中心**を比べる。中心どうしが同じ高さのときは「上」に倒す
 * (= 相手の手前へ挿す)ので、境界の 1px でも取り違えないよう固定しておく。
 */

import { describe, expect, it } from "vitest";

import type { Board, BoardColumnKey, TaskCard } from "../types";
import { COLUMN_DROPPABLE_PREFIX } from "./Column";
import { columnOf, locate, resolveDropPosition, type DropRect } from "./dropPosition";

/** id だけ意味のあるカード。計算は id と並び順しか見ない。 */
const card = (id: string) => ({ id }) as TaskCard;

/** 指定した列だけカードを持つボードの columns。 */
function columns(filled: Partial<Record<BoardColumnKey, string[]>>): Board["columns"] {
  const all: Board["columns"] = {
    new: [],
    today: [],
    tomorrow: [],
    thisWeek: [],
    nextWeek: [],
    future: [],
    doing: [],
    done: [],
  };
  for (const [key, ids] of Object.entries(filled) as [BoardColumnKey, string[]][]) {
    all[key] = ids.map(card);
  }
  return all;
}

/** 高さ 100 のカードの矩形。 */
const rect = (top: number): DropRect => ({ top, height: 100 });

/** 列そのものへ落とすときの droppable id。 */
const columnId = (key: BoardColumnKey) => `${COLUMN_DROPPABLE_PREFIX}${key}`;

describe("locate", () => {
  it("カードのいる列を返す", () => {
    const board = columns({ today: ["a"], doing: ["b"] });
    expect(locate(board, "a")).toBe("today");
    expect(locate(board, "b")).toBe("doing");
  });

  it("どこにも無ければ null", () => {
    expect(locate(columns({ today: ["a"] }), "z")).toBeNull();
  });
});

describe("columnOf", () => {
  const board = columns({ today: ["a"], doing: ["b"] });

  it("列の droppable id はその列", () => {
    expect(columnOf(board, columnId("future"))).toBe("future");
  });

  it("カード id はそのカードのいる列(= ハイライトすべき列)", () => {
    expect(columnOf(board, "b")).toBe("doing");
  });

  it("知らない列キー・知らないカードは null", () => {
    expect(columnOf(board, `${COLUMN_DROPPABLE_PREFIX}archive`)).toBeNull();
    expect(columnOf(board, "z")).toBeNull();
  });
});

describe("resolveDropPosition — 列へ落とす", () => {
  it("列の余白へ落としたら末尾へ", () => {
    const board = columns({ today: ["a"], doing: ["x", "y"] });
    expect(resolveDropPosition(board, "a", columnId("doing"), rect(0), rect(0))).toEqual({
      column: "doing",
      prevId: "y",
      nextId: null,
    });
  });

  it("空の列へ落としたら前後とも null", () => {
    const board = columns({ today: ["a"] });
    expect(resolveDropPosition(board, "a", columnId("future"), rect(0), rect(0))).toEqual({
      column: "future",
      prevId: null,
      nextId: null,
    });
  });

  it("同じ列の余白へ落とすと、自分を除いた末尾に付く", () => {
    const board = columns({ today: ["a", "b", "c"] });
    expect(resolveDropPosition(board, "a", columnId("today"), rect(0), rect(0))).toEqual({
      column: "today",
      prevId: "c",
      nextId: null,
    });
    // すでに末尾にいるなら動かさない。
    expect(resolveDropPosition(board, "c", columnId("today"), rect(0), rect(0))).toBeNull();
  });

  it("知らない列キーは無視する", () => {
    const board = columns({ today: ["a"] });
    expect(
      resolveDropPosition(board, "a", `${COLUMN_DROPPABLE_PREFIX}archive`, rect(0), rect(0)),
    ).toBeNull();
  });
});

describe("resolveDropPosition — カードへ重ねる", () => {
  const board = columns({ today: ["a", "b", "c"], doing: ["x", "y"] });

  it("相手の中心より上にいるなら手前へ挿す", () => {
    // 掴んでいる矩形の中心 50 < 相手の中心 150。
    expect(resolveDropPosition(board, "a", "y", rect(0), rect(100))).toEqual({
      column: "doing",
      prevId: "x",
      nextId: "y",
    });
  });

  it("相手の中心より下にいるなら後ろへ挿す", () => {
    // 掴んでいる矩形の中心 250 > 相手の中心 150。
    expect(resolveDropPosition(board, "a", "y", rect(200), rect(100))).toEqual({
      column: "doing",
      prevId: "y",
      nextId: null,
    });
  });

  it("中心がちょうど重なったら手前へ挿す(境界は「上」に倒す)", () => {
    expect(resolveDropPosition(board, "a", "x", rect(100), rect(100))).toEqual({
      column: "doing",
      prevId: null,
      nextId: "x",
    });
  });

  it("矩形が取れないときも手前へ挿す", () => {
    expect(resolveDropPosition(board, "a", "x", null, rect(0))).toEqual({
      column: "doing",
      prevId: null,
      nextId: "x",
    });
  });

  it("移動対象を除いた並びで前後を数える(自分自身を prevId にしない)", () => {
    // a を c の後ろへ。除外後の並びは [b, c] なので prev=c / next=null。
    expect(resolveDropPosition(board, "a", "c", rect(300), rect(200))).toEqual({
      column: "today",
      prevId: "c",
      nextId: null,
    });
    // a を b の手前へ…は現在位置と同じなので動かさない。
    expect(resolveDropPosition(board, "a", "b", rect(0), rect(100))).toBeNull();
  });

  it("列内で 1 つ下へずらす", () => {
    // a を b の後ろへ。除外後の並びは [b, c]。
    expect(resolveDropPosition(board, "a", "b", rect(200), rect(100))).toEqual({
      column: "today",
      prevId: "b",
      nextId: "c",
    });
  });

  it("自分自身に重なったら末尾扱いだが、末尾にいるなら動かさない", () => {
    expect(resolveDropPosition(board, "a", "a", rect(0), rect(0))).toEqual({
      column: "today",
      prevId: "c",
      nextId: null,
    });
    expect(resolveDropPosition(board, "c", "c", rect(0), rect(0))).toBeNull();
  });
});

describe("resolveDropPosition — 移動しない場合", () => {
  const board = columns({ today: ["a", "b"] });

  it("掴んだカードがボードに無ければ null", () => {
    expect(resolveDropPosition(board, "z", "a", rect(0), rect(0))).toBeNull();
  });

  it("重なった相手がボードに無ければ null", () => {
    expect(resolveDropPosition(board, "a", "z", rect(0), rect(0))).toBeNull();
  });

  it("同じ列で前後関係が変わらなければ null(無駄な move_task を出さない)", () => {
    // a を a の位置へ(b の手前)。
    expect(resolveDropPosition(board, "a", "b", rect(0), rect(100))).toBeNull();
    // b を b の位置へ(a の後ろ)。
    expect(resolveDropPosition(board, "b", "a", rect(200), rect(100))).toBeNull();
  });

  it("1 枚しかない列の中で動かしても null", () => {
    const single = columns({ today: ["only"] });
    expect(resolveDropPosition(single, "only", "only", rect(0), rect(0))).toBeNull();
    expect(resolveDropPosition(single, "only", columnId("today"), rect(0), rect(0))).toBeNull();
  });
});
