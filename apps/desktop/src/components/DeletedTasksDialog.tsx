/**
 * 削除済みタスクの一覧と復元。ヘッダの「削除済み」ボタンから開く。
 *
 * 削除はソフトデリート(`deleted_at` を立てるだけ)なので、ここから
 * いつでも元のステータス列の末尾へ戻せる。物理削除の手段は提供しない。
 *
 * 一覧は**開いたときにフェッチ**し、`questloom://tasks-changed` で取り直す
 * (復元してもこのイベントが飛ぶので、自分の操作の結果もここに乗る)。
 * 枠(scrim・ヘッダ・Esc・フォーカス管理)は [`ModalShell`] が持つ。
 */

import { useCallback, useEffect, useState } from "react";

import * as api from "../api";
import { listenTasksChanged } from "../api";
import { formatTimestamp } from "../format";
import { useBoardStore } from "../store";
import { toMessage } from "../tauri";
import { columnLabel, type BoardColumnKey, type TaskCard } from "../types";
import { useTauriEvent } from "../useTauriEvent";
import { ModalShell } from "./ModalShell";

/**
 * 削除時にいた列。バケットは表示のたびに導出されるので、
 * Todo なら導出済みの `bucket` が、それ以外はステータスがそのまま列になる。
 */
function originColumn(card: TaskCard): BoardColumnKey {
  return (card.bucket ?? card.status) as BoardColumnKey;
}

export function DeletedTasksDialog({ onClose }: { onClose: () => void }) {
  const [cards, setCards] = useState<TaskCard[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** 復元中のタスク。二度押しを防ぐ。 */
  const [restoring, setRestoring] = useState<string | null>(null);
  const mutate = useBoardStore((state) => state.mutate);

  const load = useCallback(() => {
    api
      .listDeletedTasks()
      .then((listed) => {
        setCards(listed);
        setError(null);
      })
      .catch((cause: unknown) => setError(toMessage(cause)));
  }, []);

  useEffect(load, [load]);
  // 復元(と他ウィンドウ・MCP からの削除)を拾って一覧を取り直す。
  useTauriEvent(listenTasksChanged, load);

  const restore = async (card: TaskCard) => {
    setRestoring(card.id);
    await mutate(() => api.restoreTask(card.id));
    setRestoring(null);
  };

  return (
    <ModalShell title="削除済みのタスク" onClose={onClose}>
      {error && <p className="ai-error">{error}</p>}

      {cards === null && !error && <p className="muted">読み込み中…</p>}

      {cards?.length === 0 && <p className="muted">削除済みのタスクはありません。</p>}

      {cards && cards.length > 0 && (
        <ul className="deleted-list">
          {cards.map((card) => (
            <li key={card.id} className="deleted-row" data-testid="deleted-row">
              <div className="deleted-main">
                <span className="deleted-title">{card.title}</span>
                <span className="deleted-meta muted">
                  <span className="badge">{columnLabel(originColumn(card))}</span>
                  {card.deletedAt && <time>{formatTimestamp(card.deletedAt)} に削除</time>}
                </span>
              </div>
              <button
                type="button"
                className="btn btn-sm"
                data-testid="restore-task"
                disabled={restoring === card.id}
                onClick={() => void restore(card)}
              >
                復元
              </button>
            </li>
          ))}
        </ul>
      )}
    </ModalShell>
  );
}
