/**
 * タスク詳細ドロワーのチェックリスト節。
 *
 * バックエンドには触らない**表示だけのコンポーネント**にしてある。実際の書き込みは
 * 呼び出し元(TaskDrawer)が `api` + `mutate` で行い、ここは操作を props のコールバックへ
 * 流すだけ。おかげで Tauri の invoke を持ち込まずにテストできる。
 *
 * 並び替えの UI はまだ無い(`reorder_checklist_item` command は用意済み)。
 * 現状は「末尾へ追加 → チェック → 本文を直す → 消す」までを扱う。
 */

import { useRef, useState } from "react";

import { formatChecklist, isChecklistComplete } from "../format";
import type { ChecklistItem } from "../types";

/**
 * チェックリストの 1 行。チェックボックス + 本文 + 削除。
 *
 * 本文はクリックでインライン編集に変わり、Enter か blur で確定、Esc で取り消す。
 * 確定は「中身が変わっていたら」だけ投げる(開いて閉じただけで書き込まない)。
 */
export function ChecklistRow({
  item,
  onToggle,
  onRename,
  onRemove,
}: {
  item: ChecklistItem;
  onToggle: (checked: boolean) => void;
  onRename: (body: string) => void;
  onRemove: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(item.body);

  const commit = () => {
    setEditing(false);
    const next = draft.trim();
    // 空にしたのは「消したい」ではなく「やめた」とみなして元へ戻す(削除は ✕ だけ)。
    if (!next || next === item.body) {
      setDraft(item.body);
      return;
    }
    onRename(next);
  };

  const cancel = () => {
    setDraft(item.body);
    setEditing(false);
  };

  return (
    <li className="checklist-item" data-testid="checklist-item">
      <input
        type="checkbox"
        checked={item.checked}
        data-testid="checklist-toggle"
        aria-label={item.body}
        onChange={(event) => onToggle(event.target.checked)}
      />
      {editing ? (
        <input
          type="text"
          className="checklist-edit"
          value={draft}
          autoFocus
          data-testid="checklist-edit"
          aria-label="チェックリスト項目の本文"
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            // IME の変換確定の Enter で確定してしまわないようにする。
            if (event.nativeEvent.isComposing) return;
            if (event.key === "Enter") {
              event.preventDefault();
              commit();
            }
            if (event.key === "Escape") {
              // ここで止める。漏らすと下のドロワーが道連れで閉じる(keyboard.ts の Esc レイヤー)。
              event.stopPropagation();
              cancel();
            }
          }}
        />
      ) : (
        <button
          type="button"
          className={item.checked ? "checklist-body checklist-body-done" : "checklist-body"}
          data-testid="checklist-body"
          title="クリックして編集"
          onClick={() => {
            setDraft(item.body);
            setEditing(true);
          }}
        >
          {item.body}
        </button>
      )}
      <button
        type="button"
        className="btn btn-ghost btn-sm"
        data-testid="checklist-remove"
        aria-label="チェックリスト項目を削除"
        onClick={onRemove}
      >
        ✕
      </button>
    </li>
  );
}

interface Props {
  /** 表示する項目(`sortOrder` 昇順で渡されている前提)。 */
  items: ChecklistItem[];
  /** ヘッダの進捗表示に使う集計。バックエンドが返す `checklistDone` / `checklistTotal`。 */
  progress: { checklistDone: number; checklistTotal: number };
  /** 末尾へ 1 件追加する。呼ばれる時点で本文は trim 済み・非空。 */
  onAdd: (body: string) => void;
  onToggle: (item: ChecklistItem, checked: boolean) => void;
  /** 本文を変更する。中身が実際に変わったときだけ呼ばれる。 */
  onRename: (item: ChecklistItem, body: string) => void;
  onRemove: (item: ChecklistItem) => void;
}

export function ChecklistSection({ items, progress, onAdd, onToggle, onRename, onRemove }: Props) {
  const [draft, setDraft] = useState("");
  const input = useRef<HTMLInputElement>(null);

  const submit = () => {
    const body = draft.trim();
    if (!body) return;
    onAdd(body);
    // QuickAdd と同じく、入力欄もフォーカスも残して連続追加できるようにする。
    setDraft("");
    input.current?.focus();
  };

  return (
    <section className="drawer-section">
      <div className="checklist-head">
        <h3>チェックリスト</h3>
        {progress.checklistTotal > 0 && (
          <span
            className={isChecklistComplete(progress) ? "badge badge-checklist-done" : "badge"}
            data-testid="checklist-progress"
          >
            {formatChecklist(progress)}
          </span>
        )}
      </div>

      {items.length === 0 && <p className="muted">まだありません。</p>}
      <ul className="checklist">
        {items.map((item) => (
          <ChecklistRow
            key={item.id}
            item={item}
            onToggle={(checked) => onToggle(item, checked)}
            onRename={(body) => onRename(item, body)}
            onRemove={() => onRemove(item)}
          />
        ))}
      </ul>

      {/*
        Enter の送信はフォームの submit に任せる。自前の keydown で拾うと、
        IME の変換確定の Enter まで送信になってしまう(QuickAdd と同じ理由)。
      */}
      <form
        className="checklist-form"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <input
          ref={input}
          type="text"
          value={draft}
          data-testid="checklist-add"
          placeholder="項目を追加"
          aria-label="チェックリストに項目を追加"
          onChange={(event) => setDraft(event.target.value)}
        />
        <button
          type="submit"
          className="btn btn-sm"
          disabled={!draft.trim()}
          // 押しても入力欄のフォーカスを外さない(連続追加のため)。
          onMouseDown={(event) => event.preventDefault()}
        >
          追加
        </button>
      </form>
    </section>
  );
}
