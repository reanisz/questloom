/**
 * Esc レイヤースタックのテスト。
 *
 * 守りたいのは「1 回の Esc で開いているものが全部閉じない」こと。
 * リスナは `keyboard.ts` が document へ 1 本だけ張り、開いている閉じ手を
 * モジュールスコープのスタックで持つ設計なので、**テスト間で状態が漏れないよう
 * 必ず unmount する**(afterEach でまとめて片付ける)。
 */

import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ESC_LAYER, onCtrlEnter, useEscapeKey } from "./keyboard";
import { mount, pressKey, type Mounted } from "./test-utils";

/** Esc で閉じるレイヤー 1 枚。 */
function Layer({
  onClose,
  enabled,
  priority,
}: {
  onClose: () => void;
  enabled?: boolean;
  priority?: number;
}) {
  useEscapeKey(onClose, { enabled, priority });
  return null;
}

const opened: Mounted[] = [];

/** レイヤーを 1 枚開く(afterEach で必ず閉じる)。 */
function open(props: Parameters<typeof Layer>[0]): Mounted {
  const mounted = mount(<Layer {...props} />);
  opened.push(mounted);
  return mounted;
}

/** 入力要素を作ってフォーカスする。 */
function focusInput(tag: "input" | "textarea" | "select", value: string, type = "text") {
  const element = document.createElement(tag);
  if (element instanceof HTMLInputElement) element.type = type;
  if (element instanceof HTMLSelectElement) {
    const option = document.createElement("option");
    option.value = value;
    element.append(option);
  }
  if (!(element instanceof HTMLSelectElement)) element.value = value;
  document.body.append(element);
  element.focus();
  return element;
}

afterEach(() => {
  while (opened.length > 0) opened.pop()?.unmount();
  document.body.replaceChildren();
});

describe("useEscapeKey", () => {
  it("開いているレイヤーが無ければ何も起きない", () => {
    expect(() => pressKey("Escape")).not.toThrow();
  });

  it("Escape 以外は配らない", () => {
    const close = vi.fn();
    open({ onClose: close });
    pressKey("Enter");
    pressKey("Esc"); // 旧 IE 表記。key は "Escape" だけを見る。
    expect(close).not.toHaveBeenCalled();
  });

  it("最前面の 1 枚だけが閉じる", () => {
    const first = vi.fn();
    const second = vi.fn();
    open({ onClose: first });
    open({ onClose: second });

    pressKey("Escape");
    expect(second).toHaveBeenCalledTimes(1);
    expect(first).not.toHaveBeenCalled();
  });

  it("上のレイヤーが閉じたら次は下のレイヤーへ配られる", () => {
    const drawer = vi.fn();
    const modal = vi.fn();
    open({ onClose: drawer, priority: ESC_LAYER.drawer });
    const top = open({ onClose: modal, priority: ESC_LAYER.modal });

    pressKey("Escape");
    expect(modal).toHaveBeenCalledTimes(1);

    top.unmount();
    pressKey("Escape");
    expect(drawer).toHaveBeenCalledTimes(1);
    expect(modal).toHaveBeenCalledTimes(1);
  });

  it("priority が高い方が、あとから開いたものより優先される", () => {
    const popup = vi.fn();
    const page = vi.fn();
    // 先にポップアップ(前面)、あとから設定ページ(最下層)を開く。
    open({ onClose: popup, priority: ESC_LAYER.popup });
    open({ onClose: page, priority: ESC_LAYER.page });

    pressKey("Escape");
    expect(popup).toHaveBeenCalledTimes(1);
    expect(page).not.toHaveBeenCalled();
  });

  it("同じ priority ならあとから開いた方が上", () => {
    const older = vi.fn();
    const newer = vi.fn();
    open({ onClose: older, priority: ESC_LAYER.modal });
    open({ onClose: newer, priority: ESC_LAYER.modal });

    pressKey("Escape");
    expect(newer).toHaveBeenCalledTimes(1);
    expect(older).not.toHaveBeenCalled();
  });

  it("enabled が偽の間はスタックに参加せず、Esc は 1 つ下へ渡る", () => {
    const closed = vi.fn();
    const disabled = vi.fn();
    open({ onClose: closed });
    const toggled = open({ onClose: disabled, enabled: false });

    pressKey("Escape");
    expect(closed).toHaveBeenCalledTimes(1);
    expect(disabled).not.toHaveBeenCalled();

    // 有効化すると最前面になる。
    toggled.rerender(<Layer onClose={disabled} enabled />);
    pressKey("Escape");
    expect(disabled).toHaveBeenCalledTimes(1);
    expect(closed).toHaveBeenCalledTimes(1);

    // 無効化すると抜ける。
    toggled.rerender(<Layer onClose={disabled} enabled={false} />);
    pressKey("Escape");
    expect(disabled).toHaveBeenCalledTimes(1);
    expect(closed).toHaveBeenCalledTimes(2);
  });

  it("onClose は毎レンダー差し替えられる(再購読は起きない)", () => {
    function Renamed() {
      const [count, setCount] = useState(0);
      useEscapeKey(() => setCount((value) => value + 1));
      return <span>{count}</span>;
    }
    const mounted = mount(<Renamed />);
    opened.push(mounted);

    pressKey("Escape");
    pressKey("Escape");
    expect(mounted.container.textContent).toBe("2");
  });

  it("手前で処理された Esc (defaultPrevented) は配らない", () => {
    const close = vi.fn();
    open({ onClose: close });

    // 要素自身の Escape ハンドラ(「編集前に戻す」など)が先に処理したケース。
    const owner = document.createElement("div");
    owner.addEventListener("keydown", (event) => event.preventDefault());
    document.body.append(owner);

    pressKey("Escape", owner);
    expect(close).not.toHaveBeenCalled();
  });

  describe("入力中の Esc は握りつぶす(入力途中のテキストを失わせない)", () => {
    it("値の入った input / textarea では閉じない", () => {
      const close = vi.fn();
      open({ onClose: close });

      for (const tag of ["input", "textarea"] as const) {
        pressKey("Escape", focusInput(tag, "書きかけ"));
        expect(close, tag).not.toHaveBeenCalled();
      }
    });

    it("空の input / textarea では閉じる", () => {
      const close = vi.fn();
      open({ onClose: close });

      pressKey("Escape", focusInput("input", ""));
      expect(close).toHaveBeenCalledTimes(1);
      pressKey("Escape", focusInput("textarea", ""));
      expect(close).toHaveBeenCalledTimes(2);
    });

    it("select は値の有無によらず閉じない(Esc はドロップダウンの取り消し)", () => {
      const close = vi.fn();
      open({ onClose: close });
      pressKey("Escape", focusInput("select", ""));
      expect(close).not.toHaveBeenCalled();
    });

    it("contenteditable では閉じない", () => {
      const close = vi.fn();
      open({ onClose: close });

      const editable = document.createElement("div");
      editable.contentEditable = "true";
      // jsdom は isContentEditable を実装していない(常に false)ので、ブラウザ側の
      // 振る舞いを立てておく。判定に使っているのはこのプロパティ。
      Object.defineProperty(editable, "isContentEditable", { value: true });
      document.body.append(editable);
      pressKey("Escape", editable);
      expect(close).not.toHaveBeenCalled();
    });

    it("テキストを持たない input(チェックボックス等)では閉じる", () => {
      const close = vi.fn();
      open({ onClose: close });

      pressKey("Escape", focusInput("input", "on", "checkbox"));
      expect(close).toHaveBeenCalledTimes(1);
      pressKey("Escape", focusInput("input", "#ff0000", "color"));
      expect(close).toHaveBeenCalledTimes(2);
    });

    it("入力欄以外(ボタン)では閉じる", () => {
      const close = vi.fn();
      open({ onClose: close });

      const button = document.createElement("button");
      document.body.append(button);
      pressKey("Escape", button);
      expect(close).toHaveBeenCalledTimes(1);
    });
  });
});

describe("onCtrlEnter", () => {
  /** React の合成イベントの代わりに、必要な分だけ持つ偽物を作る。 */
  function event(init: { key: string; ctrlKey?: boolean; metaKey?: boolean }) {
    return {
      key: init.key,
      ctrlKey: init.ctrlKey ?? false,
      metaKey: init.metaKey ?? false,
      preventDefault: vi.fn(),
    };
  }

  it("Ctrl+Enter / Cmd+Enter で送信し、既定の動作は止める", () => {
    for (const modifier of ["ctrlKey", "metaKey"] as const) {
      const submit = vi.fn();
      const keyEvent = event({ key: "Enter", [modifier]: true });
      onCtrlEnter(submit)(keyEvent as never);
      expect(submit, modifier).toHaveBeenCalledTimes(1);
      expect(keyEvent.preventDefault).toHaveBeenCalledTimes(1);
    }
  });

  it("修飾なしの Enter は素通しする(テキストエリアの改行を殺さない)", () => {
    const submit = vi.fn();
    const keyEvent = event({ key: "Enter" });
    onCtrlEnter(submit)(keyEvent as never);
    expect(submit).not.toHaveBeenCalled();
    expect(keyEvent.preventDefault).not.toHaveBeenCalled();
  });

  it("Enter 以外は無視する", () => {
    const submit = vi.fn();
    onCtrlEnter(submit)(event({ key: "a", ctrlKey: true }) as never);
    expect(submit).not.toHaveBeenCalled();
  });
});
