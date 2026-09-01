/**
 * チェックリスト節のテスト。
 *
 * 見ているのは 4 つ。
 * 1. 進捗表示 (`2/5`) と「全部埋まった」の見分け。
 * 2. チェックボックスの操作が `onToggle` へ届くこと。
 * 3. 本文のインライン編集 — Enter で確定、Esc で取り消し、**変わっていなければ投げない**こと。
 * 4. 追加フォームが連続入力できる形になっていること(送信後に入力欄が空で残る)。
 *
 * このコンポーネントはバックエンドに触らないので、`api` のモックは要らない。
 */

import { act } from "react";
import { describe, expect, it, vi } from "vitest";

import { mount, pressKey } from "../test-utils";
import type { ChecklistItem } from "../types";
import { ChecklistSection } from "./ChecklistSection";

function item(id: string, body: string, checked = false): ChecklistItem {
  return {
    id,
    taskId: "t1",
    body,
    checked,
    sortOrder: id,
    createdAt: "2026-09-01T00:00:00Z",
  };
}

/** `items` のチェック済み件数から進捗を作る(バックエンドの集計と同じ意味)。 */
function progressOf(items: ChecklistItem[]) {
  return {
    checklistDone: items.filter((i) => i.checked).length,
    checklistTotal: items.length,
  };
}

function setup(items: ChecklistItem[], handlers: Partial<Parameters<typeof ChecklistSection>[0]> = {}) {
  const calls = {
    onAdd: vi.fn(),
    onToggle: vi.fn(),
    onRename: vi.fn(),
    onRemove: vi.fn(),
  };
  const mounted = mount(
    <ChecklistSection items={items} progress={progressOf(items)} {...calls} {...handlers} />,
  );
  const { container } = mounted;
  const q = <T extends Element>(selector: string) => container.querySelector<T>(selector);
  const all = <T extends Element>(selector: string) =>
    Array.from(container.querySelectorAll<T>(selector));

  return {
    ...mounted,
    ...calls,
    rows: () => all('[data-testid="checklist-item"]'),
    progress: () => q('[data-testid="checklist-progress"]'),
    toggles: () => all<HTMLInputElement>('[data-testid="checklist-toggle"]'),
    bodies: () => all<HTMLButtonElement>('[data-testid="checklist-body"]'),
    edit: () => q<HTMLInputElement>('[data-testid="checklist-edit"]'),
    addInput: () => q<HTMLInputElement>('[data-testid="checklist-add"]'),
    /** 入力欄へ文字を入れる(React の onChange を通す)。 */
    type(input: HTMLInputElement, text: string) {
      act(() => {
        // value を直接書くと React が変化に気付かないので、ネイティブの setter を使う。
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    },
    /** 追加フォームの submit(= Enter 相当)。 */
    submitAdd() {
      const form = container.querySelector("form");
      if (!form) throw new Error("追加フォームがない");
      act(() => {
        form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
      });
    },
  };
}

describe("ChecklistSection", () => {
  it("項目が無いときは空の案内だけを出す", () => {
    const view = setup([]);
    expect(view.rows()).toHaveLength(0);
    expect(view.progress()).toBeNull();
    expect(view.container.textContent).toContain("まだありません");
    view.unmount();
  });

  it("進捗を 2/3 の形で出し、全部埋まったら見た目を変える", () => {
    const partial = setup([item("a", "1", true), item("b", "2", true), item("c", "3")]);
    expect(partial.progress()?.textContent).toBe("2/3");
    expect(partial.progress()?.className).not.toContain("badge-checklist-done");
    partial.unmount();

    const full = setup([item("a", "1", true), item("b", "2", true)]);
    expect(full.progress()?.textContent).toBe("2/2");
    expect(full.progress()?.className).toContain("badge-checklist-done");
    full.unmount();
  });

  it("チェック済みの項目には取り消し線のクラスが付く", () => {
    const view = setup([item("a", "済み", true), item("b", "まだ")]);
    expect(view.bodies()[0].className).toContain("checklist-body-done");
    expect(view.bodies()[1].className).not.toContain("checklist-body-done");
    expect(view.toggles()[0].checked).toBe(true);
    expect(view.toggles()[1].checked).toBe(false);
    view.unmount();
  });

  it("チェックボックスの操作が onToggle へ届く", () => {
    const target = item("a", "住所変更");
    const view = setup([target]);
    act(() => view.toggles()[0].click());
    expect(view.onToggle).toHaveBeenCalledWith(target, true);
    view.unmount();
  });

  it("✕ で onRemove を呼ぶ", () => {
    const target = item("a", "消す");
    const view = setup([target]);
    act(() => {
      view.container.querySelector<HTMLButtonElement>('[data-testid="checklist-remove"]')?.click();
    });
    expect(view.onRemove).toHaveBeenCalledWith(target);
    view.unmount();
  });

  it("本文をクリックすると編集欄になり、Enter で確定する", () => {
    const target = item("a", "もとの本文");
    const view = setup([target]);
    expect(view.edit()).toBeNull();

    act(() => view.bodies()[0].click());
    const input = view.edit();
    expect(input).not.toBeNull();
    expect(input?.value).toBe("もとの本文");

    view.type(input!, "  新しい本文  ");
    pressKey("Enter", input!);
    expect(view.onRename).toHaveBeenCalledWith(target, "新しい本文");
    // 確定したら編集欄は閉じる。
    expect(view.edit()).toBeNull();
    view.unmount();
  });

  it("中身が変わっていなければ確定しても投げない", () => {
    const view = setup([item("a", "そのまま")]);
    act(() => view.bodies()[0].click());
    pressKey("Enter", view.edit()!);
    expect(view.onRename).not.toHaveBeenCalled();
    view.unmount();
  });

  it("空にして確定したら削除ではなく取り消しとして扱う", () => {
    const view = setup([item("a", "残る")]);
    act(() => view.bodies()[0].click());
    view.type(view.edit()!, "   ");
    pressKey("Enter", view.edit()!);

    expect(view.onRename).not.toHaveBeenCalled();
    expect(view.onRemove).not.toHaveBeenCalled();
    expect(view.bodies()[0].textContent).toBe("残る");
    view.unmount();
  });

  it("Esc は編集を取り消し、下のレイヤーへ漏らさない", () => {
    const view = setup([item("a", "もとの本文")]);
    act(() => view.bodies()[0].click());
    view.type(view.edit()!, "書きかけ");

    // ドロワー・ダイアログの Esc レイヤー (keyboard.ts) は document で待ち構えている。
    // ここで漏らすと、編集をやめただけでドロワーごと閉じてしまう。
    const onDocumentKeyDown = vi.fn();
    document.addEventListener("keydown", onDocumentKeyDown);
    pressKey("Escape", view.edit()!);
    document.removeEventListener("keydown", onDocumentKeyDown);

    expect(view.onRename).not.toHaveBeenCalled();
    expect(view.edit()).toBeNull();
    expect(view.bodies()[0].textContent).toBe("もとの本文");
    expect(onDocumentKeyDown).not.toHaveBeenCalled();
    view.unmount();
  });

  it("blur でも確定する", () => {
    const target = item("a", "もとの本文");
    const view = setup([target]);
    act(() => view.bodies()[0].click());
    view.type(view.edit()!, "blur で確定");
    // React の onBlur は native の focusout に対応する(blur は bubble しない)。
    act(() => view.edit()?.dispatchEvent(new FocusEvent("focusout", { bubbles: true })));
    expect(view.onRename).toHaveBeenCalledWith(target, "blur で確定");
    view.unmount();
  });

  it("IME の変換確定の Enter では確定しない", () => {
    const view = setup([item("a", "もとの本文")]);
    act(() => view.bodies()[0].click());
    view.type(view.edit()!, "へんかんちゅう");
    // isComposing 付きの Enter は変換確定であって、編集の確定ではない。
    pressKey("Enter", view.edit()!, { isComposing: true });
    expect(view.onRename).not.toHaveBeenCalled();
    expect(view.edit()).not.toBeNull();
    view.unmount();
  });

  it("追加は trim して onAdd へ渡し、入力欄を空にして残す", () => {
    const view = setup([]);
    const input = view.addInput()!;
    expect(input.value).toBe("");

    view.type(input, "  住所変更  ");
    view.submitAdd();

    expect(view.onAdd).toHaveBeenCalledWith("住所変更");
    // 連続追加できるよう、入力欄は空で残る(畳まない)。
    expect(view.addInput()?.value).toBe("");
    expect(view.addInput()).not.toBeNull();
    view.unmount();
  });

  it("空のまま送信しても追加しない", () => {
    const view = setup([]);
    view.type(view.addInput()!, "   ");
    view.submitAdd();
    expect(view.onAdd).not.toHaveBeenCalled();
    view.unmount();
  });
});
