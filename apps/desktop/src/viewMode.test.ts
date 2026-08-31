/**
 * 表示モードの永続化のテスト。
 *
 * 本題は「localStorage が使えない環境でも落ちない」こと
 * (プライベートモードや、サイトデータをブロックしたブラウザでは
 * getItem / setItem が例外を投げる)。
 */

import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { renderHook } from "./test-utils";
import { useExpandedView, VIEW_MODE_KEY } from "./viewMode";

afterEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
});

describe("useExpandedView", () => {
  it("保存されていなければ通常表示から始まる", () => {
    const { result, unmount } = renderHook(() => useExpandedView());
    expect(result.current[0]).toBe(false);
    unmount();
  });

  it('"1" が保存されていれば展開表示から始まる', () => {
    localStorage.setItem(VIEW_MODE_KEY, "1");
    const { result, unmount } = renderHook(() => useExpandedView());
    expect(result.current[0]).toBe(true);
    unmount();
  });

  it('"1" 以外はすべて通常表示として読む', () => {
    localStorage.setItem(VIEW_MODE_KEY, "0");
    const first = renderHook(() => useExpandedView());
    expect(first.result.current[0]).toBe(false);
    first.unmount();

    localStorage.setItem(VIEW_MODE_KEY, "true");
    const second = renderHook(() => useExpandedView());
    expect(second.result.current[0]).toBe(false);
    second.unmount();
  });

  it("切り替えると localStorage に書く", () => {
    const { result, unmount } = renderHook(() => useExpandedView());

    act(() => result.current[1](true));
    expect(result.current[0]).toBe(true);
    expect(localStorage.getItem(VIEW_MODE_KEY)).toBe("1");

    act(() => result.current[1](false));
    expect(result.current[0]).toBe(false);
    expect(localStorage.getItem(VIEW_MODE_KEY)).toBe("0");

    unmount();
  });

  it("読み出しが例外を投げても通常表示で立ち上がる", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new DOMException("access denied", "SecurityError");
    });

    const { result, unmount } = renderHook(() => useExpandedView());
    expect(result.current[0]).toBe(false);
    unmount();
  });

  it("保存が例外を投げても表示の切り替えは通る", () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new DOMException("quota exceeded", "QuotaExceededError");
    });

    const { result, unmount } = renderHook(() => useExpandedView());
    act(() => result.current[1](true));

    expect(result.current[0]).toBe(true);
    expect(setItem).toHaveBeenCalledWith(VIEW_MODE_KEY, "1");
    unmount();
  });
});
