/**
 * クイック追加の開閉のテスト。
 *
 * 見ているのは「いつ入力欄になり、いつボタンへ戻るか」と「Esc が下のレイヤーへ
 * 漏れないか」の 2 点。追加そのもの(create_task → move_task)は Column 側の
 * 責務なので、ここでは `onAdd` を差し替えて呼ばれ方だけを見る。
 */

import { act } from "react";
import { describe, expect, it, vi } from "vitest";

import { mount, pressKey } from "../test-utils";
import { QuickAdd } from "./QuickAdd";

/** テスト対象を描画して、よく使う要素の取り出し口を返す。 */
function setup(onAdd: (title: string) => Promise<boolean> = async () => true) {
  const mounted = mount(<QuickAdd columnKey="new" label="New" onAdd={onAdd} />);
  const { container } = mounted;
  return {
    ...mounted,
    openButton: () =>
      container.querySelector<HTMLButtonElement>('[data-testid="quick-add-open-new"]'),
    input: () => container.querySelector<HTMLInputElement>('[data-testid="quick-add-new"]'),
    /** ボタンを押して入力欄を開く。 */
    open() {
      act(() => container.querySelector<HTMLButtonElement>("button")?.click());
    },
    /** 入力欄へ文字を入れる(React の onChange を通す)。 */
    type(text: string) {
      const input = container.querySelector<HTMLInputElement>('[data-testid="quick-add-new"]');
      if (!input) throw new Error("入力欄が開いていません");
      act(() => {
        // value を直接書くと React が変化に気付かないので、ネイティブの setter を使う。
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    },
    /** Enter 相当。フォームの submit を発火させる。 */
    submit() {
      const form = container.querySelector("form");
      if (!form) throw new Error("フォームが開いていません");
      act(() => {
        form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
      });
    },
  };
}

describe("QuickAdd", () => {
  it("既定ではテキストボタンだけを出す", () => {
    const view = setup();
    expect(view.openButton()).not.toBeNull();
    expect(view.openButton()?.textContent).toContain("タスクを追加");
    expect(view.input()).toBeNull();
    view.unmount();
  });

  it("押すと入力欄になり、自動でフォーカスする", () => {
    const view = setup();
    view.open();
    const input = view.input();
    expect(input).not.toBeNull();
    expect(document.activeElement).toBe(input);
    expect(view.openButton()).toBeNull();
    view.unmount();
  });

  it("送信すると onAdd を呼び、入力欄とフォーカスを保つ(連続入力)", async () => {
    const onAdd = vi.fn(async () => true);
    const view = setup(onAdd);
    view.open();
    view.type("やること");
    view.submit();
    // submit ハンドラ内の await を消化させる。
    await act(async () => {});

    expect(onAdd).toHaveBeenCalledWith("やること");
    const input = view.input();
    expect(input).not.toBeNull();
    expect(input?.value).toBe("");
    expect(document.activeElement).toBe(input);
    view.unmount();
  });

  it("空の送信では onAdd を呼ばない", async () => {
    const onAdd = vi.fn(async () => true);
    const view = setup(onAdd);
    view.open();
    view.type("   ");
    view.submit();
    await act(async () => {});

    expect(onAdd).not.toHaveBeenCalled();
    view.unmount();
  });

  it("失敗したら入力内容を残す(打ち直させない)", async () => {
    const view = setup(async () => false);
    view.open();
    view.type("やること");
    view.submit();
    await act(async () => {});

    expect(view.input()?.value).toBe("やること");
    view.unmount();
  });

  it("Esc でボタン表示へ戻り、Esc は document まで漏れない", () => {
    const view = setup();
    view.open();
    view.type("書きかけ");

    const onDocumentKeyDown = vi.fn();
    document.addEventListener("keydown", onDocumentKeyDown);
    const input = view.input();
    if (!input) throw new Error("入力欄が開いていません");
    pressKey("Escape", input);
    document.removeEventListener("keydown", onDocumentKeyDown);

    expect(onDocumentKeyDown).not.toHaveBeenCalled();
    expect(view.input()).toBeNull();
    expect(view.openButton()).not.toBeNull();
    view.unmount();
  });

  it("IME の変換中の Esc では畳まない", () => {
    const view = setup();
    view.open();
    view.type("へんかん");
    const input = view.input();
    if (!input) throw new Error("入力欄が開いていません");
    // KeyboardEvent の isComposing は init から立てる。
    pressKey("Escape", input, { isComposing: true } as KeyboardEventInit);

    expect(view.input()).not.toBeNull();
    view.unmount();
  });

  it("空のまま blur したらボタン表示へ戻る", () => {
    const view = setup();
    view.open();
    const input = view.input();
    act(() => input?.dispatchEvent(new FocusEvent("focusout", { bubbles: true })));

    expect(view.input()).toBeNull();
    expect(view.openButton()).not.toBeNull();
    view.unmount();
  });

  it("入力途中の blur では畳まず、文字も捨てない", () => {
    const view = setup();
    view.open();
    view.type("書きかけ");
    const input = view.input();
    act(() => input?.dispatchEvent(new FocusEvent("focusout", { bubbles: true })));

    expect(view.input()?.value).toBe("書きかけ");
    view.unmount();
  });
});
