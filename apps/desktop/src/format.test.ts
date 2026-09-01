/**
 * 表示整形のテスト。
 *
 * `format.ts` はローカルタイムゾーンで整形するので、期待値は
 * **ローカル時刻の成分から作った Date** を起点にして書く
 * (`new Date(2026, 8, 30, 9, 5)` = ローカルの 2026-09-30 09:05)。
 * こうすれば開発機 (JST) でも CI (UTC) でも同じ結果になる。
 */

import { describe, expect, it } from "vitest";

import {
  formatChecklist,
  formatDeadline,
  formatScheduled,
  formatStatus,
  formatTimestamp,
  fromDateTimeLocal,
  isChecklistComplete,
  isOverdue,
  toDateTimeLocal,
} from "./format";
import type { TaskStatus } from "./types";

/** ローカル時刻の成分から RFC3339 (UTC) を作る。 */
function localIso(
  year: number,
  month: number,
  day: number,
  hour = 0,
  minute = 0,
): string {
  return new Date(year, month - 1, day, hour, minute).toISOString();
}

describe("formatDeadline", () => {
  it("同じ年なら年を省く", () => {
    const year = new Date().getFullYear();
    const text = formatDeadline(localIso(year, 9, 30, 9, 5));
    expect(text).toMatch(/9\/30/);
    expect(text).toMatch(/09:05/);
    expect(text).not.toContain(String(year));
  });

  it("違う年なら年を出す", () => {
    const text = formatDeadline(localIso(2000, 1, 2, 3, 4));
    expect(text).toContain("2000");
    expect(text).toMatch(/1\/2/);
    expect(text).toMatch(/03:04/);
  });

  it("日付として読めない文字列はそのまま返す(表示を空にしない)", () => {
    expect(formatDeadline("いつか")).toBe("いつか");
    expect(formatDeadline("")).toBe("");
  });
});

describe("formatTimestamp", () => {
  it("年まで含めた絶対時刻にする", () => {
    const text = formatTimestamp(localIso(2026, 9, 30, 9, 5));
    expect(text).toContain("2026");
    expect(text).toMatch(/9\/30/);
    expect(text).toMatch(/09:05/);
  });

  it("読めない文字列はそのまま返す", () => {
    expect(formatTimestamp("not a date")).toBe("not a date");
  });
});

describe("isOverdue", () => {
  it("過ぎていれば true", () => {
    expect(isOverdue(new Date(Date.now() - 60_000).toISOString())).toBe(true);
  });

  it("まだなら false", () => {
    expect(isOverdue(new Date(Date.now() + 60_000).toISOString())).toBe(false);
  });

  it("締切なし・読めない値は false(赤くしない)", () => {
    expect(isOverdue(null)).toBe(false);
    expect(isOverdue("")).toBe(false);
    expect(isOverdue("そのうち")).toBe(false);
  });
});

describe("datetime-local との相互変換", () => {
  it("UTC の RFC3339 をローカルの入力値へ直す", () => {
    expect(toDateTimeLocal(localIso(2026, 9, 30, 9, 5))).toBe("2026-09-30T09:05");
    // 月・日・時・分はゼロ詰めする(<input type="datetime-local"> が要求する形)。
    expect(toDateTimeLocal(localIso(2026, 1, 2, 3, 4))).toBe("2026-01-02T03:04");
  });

  it("空・読めない値は空文字列(入力欄を空にする)", () => {
    expect(toDateTimeLocal(null)).toBe("");
    expect(toDateTimeLocal("")).toBe("");
    expect(toDateTimeLocal("いつか")).toBe("");
  });

  it("入力値を UTC の RFC3339 へ戻す", () => {
    expect(fromDateTimeLocal("2026-09-30T09:05")).toBe(localIso(2026, 9, 30, 9, 5));
  });

  it("空は null(締切なし)、読めない値も null", () => {
    expect(fromDateTimeLocal("")).toBeNull();
    expect(fromDateTimeLocal("2026-13-40T99:99")).toBeNull();
  });

  it("往復しても同じ時刻に戻る", () => {
    const iso = localIso(2026, 12, 31, 23, 59);
    expect(fromDateTimeLocal(toDateTimeLocal(iso))).toBe(iso);
  });
});

describe("formatScheduled", () => {
  it("種別ごとの日本語表記", () => {
    expect(formatScheduled({ kind: "date", value: "2026-09-30" })).toBe("2026-09-30 に実施");
    expect(formatScheduled({ kind: "week", value: "2026-W40" })).toBe("2026-W40 の週に実施");
    expect(formatScheduled({ kind: "none" })).toBe("予定なし");
  });
});

describe("formatStatus", () => {
  it("状態のラベルを引く", () => {
    expect(formatStatus("new")).toBe("New");
    expect(formatStatus("todo")).toBe("Todo");
    expect(formatStatus("doing")).toBe("Doing");
    expect(formatStatus("done")).toBe("Done");
  });

  it("知らない状態はそのまま出す(表示を undefined にしない)", () => {
    expect(formatStatus("archived" as TaskStatus)).toBe("archived");
  });
});

describe("formatChecklist / isChecklistComplete", () => {
  const progress = (checklistDone: number, checklistTotal: number) => ({
    checklistDone,
    checklistTotal,
  });

  it("進捗を done/total で出す", () => {
    expect(formatChecklist(progress(0, 5))).toBe("0/5");
    expect(formatChecklist(progress(2, 5))).toBe("2/5");
    expect(formatChecklist(progress(5, 5))).toBe("5/5");
  });

  it("全部埋まったときだけ完了とみなす", () => {
    expect(isChecklistComplete(progress(5, 5))).toBe(true);
    expect(isChecklistComplete(progress(4, 5))).toBe(false);
    expect(isChecklistComplete(progress(0, 1))).toBe(false);
  });

  it("項目が 0 件なら完了ではない(バッジ自体を出さない側の判断)", () => {
    expect(isChecklistComplete(progress(0, 0))).toBe(false);
  });
});
