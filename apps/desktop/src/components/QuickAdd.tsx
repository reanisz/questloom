/**
 * 列下部のクイック追加。
 *
 * 常設の入力欄ではなく、通常は控えめなテキストボタン (`+ タスクを追加`) だけを置き、
 * 押されたときに入力欄へ切り替える。列がカードで埋まっているときに入力欄が
 * 視線を奪わないようにするため。
 *
 * ## 開閉と blur の扱い
 *
 * - 開いたら自動でフォーカスする。
 * - Enter (フォームの submit) で追加し、**入力欄もフォーカスも残す**(連続入力)。
 * - Esc、または**空のまま**フォーカスが外れたらボタン表示へ戻す。
 * - **文字が残っている状態の blur では畳まない。** 畳むと入力途中の文字が黙って
 *   消えるうえ、「追加」ボタンを押したときの blur でボタン自身が消えて click が
 *   届かなくなる(押下時の blur は `onMouseDown` の preventDefault でも防いでいる)。
 *
 * Esc は `keyboard.ts` の Esc レイヤーへ**渡さない**。空の入力欄の Esc は
 * [`isTypingTarget`] の対象外なので、そのままだと下のドロワー・ダイアログが
 * 道連れで閉じてしまう。
 */

import { useEffect, useRef, useState } from "react";

import type { BoardColumnKey } from "../types";

interface Props {
  columnKey: BoardColumnKey;
  /** 列の表示名。読み上げ用のラベルに使う。 */
  label: string;
  /** 追加を実行する。成功したら true(= 入力欄を空にして次の入力を待つ)。 */
  onAdd: (title: string) => Promise<boolean>;
}

export function QuickAdd({ columnKey, label, onAdd }: Props) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [adding, setAdding] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  // 開いた直後にフォーカスを移す(マウント時の 1 回だけ)。
  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  const close = () => {
    setOpen(false);
    setDraft("");
  };

  const submit = async () => {
    const title = draft.trim();
    if (!title || adding) return;
    setAdding(true);
    const ok = await onAdd(title);
    setAdding(false);
    if (ok) setDraft("");
    // 連続して打てるようフォーカスを戻す(「追加」ボタン経由で外れていた場合の保険)。
    inputRef.current?.focus();
  };

  if (!open) {
    return (
      <div className="quick-add">
        <button
          type="button"
          className="quick-add-open"
          data-testid={`quick-add-open-${columnKey}`}
          aria-label={`${label} にタスクを追加`}
          onClick={() => setOpen(true)}
        >
          + タスクを追加
        </button>
      </div>
    );
  }

  return (
    <form
      className="quick-add"
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      {/*
        Enter の送信はフォームの submit に任せる。自前の keydown で拾うと、
        IME の変換確定の Enter まで送信になってしまう。
      */}
      <input
        ref={inputRef}
        type="text"
        value={draft}
        data-testid={`quick-add-${columnKey}`}
        placeholder="タスク名"
        aria-label={`${label} にタスクを追加`}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key !== "Escape" || event.nativeEvent.isComposing) return;
          // ここで止める。Esc レイヤー(keyboard.ts)は document で待ち構えている。
          event.stopPropagation();
          close();
        }}
        onBlur={() => {
          if (!draft.trim()) close();
        }}
      />
      <button
        type="submit"
        className="btn btn-primary btn-sm"
        disabled={adding || !draft.trim()}
        // 押しても入力欄のフォーカスを外さない(blur で畳まれるのを避ける)。
        onMouseDown={(event) => event.preventDefault()}
      >
        追加
      </button>
    </form>
  );
}
