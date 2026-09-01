/**
 * 列の一覧まわりの取り決めのテスト。
 *
 * 列を足すたびに直す場所が 3 つ(`BOARD_COLUMNS` / `PRIMARY_COLUMNS` / `DEFER_COLUMNS`)
 * あり、どれか 1 つを忘れると「ボードのどこにも出ない列」ができてしまう。
 * ここではその取りこぼしを機械的に防ぐ。
 */

import { describe, expect, it } from "vitest";

import {
  BOARD_COLUMNS,
  columnIcon,
  columnLabel,
  DEFER_COLUMNS,
  isPrimaryColumn,
  PRIMARY_COLUMNS,
  PROMOTE_COLUMNS,
  type BoardColumnKey,
} from "./types";

const allKeys = BOARD_COLUMNS.map((column) => column.key);

describe("列の一覧", () => {
  it("通常表示の列 + 先送りレール = 全列(どこにも出ない列を作らない)", () => {
    const shown = [...PRIMARY_COLUMNS, ...DEFER_COLUMNS];
    expect([...shown].sort()).toEqual([...allKeys].sort());
    // 二重に出る列も無い。
    expect(new Set(shown).size).toBe(shown.length);
  });

  it("監視中と Icebox は先送りレール側(通常表示では列にしない)", () => {
    for (const key of ["watching", "icebox"] as const) {
      expect(DEFER_COLUMNS).toContain<BoardColumnKey>(key);
      expect(isPrimaryColumn(key)).toBe(false);
    }
  });

  it("レールの並びは Tomorrow / This Week / Next Week / Future / Icebox / 監視中", () => {
    expect([...DEFER_COLUMNS]).toEqual([
      "tomorrow",
      "thisWeek",
      "nextWeek",
      "future",
      "icebox",
      "watching",
    ]);
  });

  it("展開表示では Icebox が一番左(バックエンドの列順と一致)", () => {
    expect(allKeys[0]).toBe("icebox");
    expect(allKeys).toEqual([
      "icebox",
      "new",
      "today",
      "tomorrow",
      "thisWeek",
      "nextWeek",
      "future",
      "watching",
      "doing",
      "done",
    ]);
  });

  it("昇格先は Todo 系だけ(New / Watching / Icebox / Doing / Done は選ばせない)", () => {
    expect([...PROMOTE_COLUMNS]).toEqual(["today", "tomorrow", "thisWeek", "nextWeek", "future"]);
    for (const key of ["new", "watching", "icebox", "doing", "done"] as const) {
      expect(PROMOTE_COLUMNS).not.toContain<BoardColumnKey>(key);
    }
  });

  it("列キーは重複せず、すべてラベルを持つ", () => {
    expect(new Set(allKeys).size).toBe(allKeys.length);
    for (const key of allKeys) {
      expect(columnLabel(key)).not.toBe("");
    }
    expect(columnLabel("watching")).toBe("監視中");
    expect(columnLabel("icebox")).toBe("Icebox");
  });

  it("記号を持つのは監視中だけ(他の列は素のラベル)", () => {
    expect(columnIcon("watching")).toBe("👁");
    for (const key of allKeys.filter((k) => k !== "watching")) {
      expect(columnIcon(key)).toBeUndefined();
    }
  });
});
