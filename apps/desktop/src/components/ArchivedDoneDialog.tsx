/**
 * 過去の完了(前日以前に完了したタスク)の一覧。Done 列のフッタから開く。
 *
 * ボードの Done 列は「今日完了した分」だけを見せる導出なので、それ以前の完了は
 * 消えたわけではなくここに溜まる(データは動いていない)。
 *
 * 一覧は**開いたときにフェッチ**し、`questloom://tasks-changed` で取り直す
 * ([`DeletedTasksDialog`] と同じ作法)。ここからの操作は「詳細を開く」だけで、
 * 復元・再オープン・削除といった操作は詳細ドロワー側に任せる。
 * 枠(scrim・ヘッダ・Esc・フォーカス管理)は [`ModalShell`] が持つ。
 */

import { useCallback, useEffect, useState } from "react";

import * as api from "../api";
import { listenTasksChanged } from "../api";
import { formatTimestamp } from "../format";
import { useBoardStore } from "../store";
import { toMessage } from "../tauri";
import type { ArchivedDone } from "../types";
import { useTauriEvent } from "../useTauriEvent";
import { ModalShell } from "./ModalShell";

export function ArchivedDoneDialog({ onClose }: { onClose: () => void }) {
  const [listed, setListed] = useState<ArchivedDone | null>(null);
  const [error, setError] = useState<string | null>(null);
  const openTask = useBoardStore((state) => state.openTask);

  const load = useCallback(() => {
    api
      .listArchivedDone()
      .then((next) => {
        setListed(next);
        setError(null);
      })
      .catch((cause: unknown) => setError(toMessage(cause)));
  }, []);

  useEffect(load, [load]);
  // 他ウィンドウ・MCP・AI からの変更でも一覧を取り直す。
  useTauriEvent(listenTasksChanged, load);

  // 詳細はボードの上のドロワーで開くので、このダイアログは閉じる。
  const open = (taskId: string) => {
    openTask(taskId);
    onClose();
  };

  return (
    <ModalShell title="過去の完了" onClose={onClose}>
      {error && <p className="ai-error">{error}</p>}

      {listed === null && !error && <p className="muted">読み込み中…</p>}

      {listed?.tasks.length === 0 && <p className="muted">前日以前に完了したタスクはありません。</p>}

      {listed && listed.tasks.length > 0 && (
        <>
          <ul className="deleted-list">
            {listed.tasks.map((card) => (
              <li key={card.id} className="deleted-row" data-testid="archived-done-row">
                <button
                  type="button"
                  className="deleted-main archived-open"
                  title="タスクの詳細を開く"
                  onClick={() => open(card.id)}
                >
                  <span className="deleted-title">{card.title}</span>
                  <span className="deleted-meta muted">
                    {card.doneAt && <time>{formatTimestamp(card.doneAt)} に完了</time>}
                  </span>
                </button>
              </li>
            ))}
          </ul>
          {listed.total > listed.tasks.length && (
            <p className="muted">
              全 {listed.total} 件のうち、新しい {listed.limit} 件を表示しています。
            </p>
          )}
        </>
      )}
    </ModalShell>
  );
}
