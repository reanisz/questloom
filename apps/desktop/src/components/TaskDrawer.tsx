/**
 * タスク詳細ドロワー。右からスライドインし、タイトル・説明・締切の編集、
 * 関連リソース、状態アップデート履歴、親子タスクを扱う。
 */

import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { useEffect, useRef, useState } from "react";

import * as api from "../api";
import {
  formatScheduled,
  formatStatus,
  formatTimestamp,
  fromDateTimeLocal,
  isOverdue,
  toDateTimeLocal,
} from "../format";
import { ESC_LAYER, onCtrlEnter, useEscapeKey } from "../keyboard";
import { useBoardStore } from "../store";
import { toMessage } from "../tauri";
import type { BoardColumnKey, ResourceKind, TaskCard, TaskDetail, TaskId } from "../types";
import { AiSplitDialog } from "./AiSplitDialog";
import { PromoteMenu } from "./PromoteMenu";

/** リソースを既定のアプリで開く。file はエクスプローラで場所を表示する。 */
async function openResource(kind: ResourceKind, value: string) {
  if (kind === "url") {
    await openUrl(value);
  } else {
    await revealItemInDir(value);
  }
}

/** 親・子タスクへのリンク行。 */
function TaskLink({ card, onOpen }: { card: TaskCard; onOpen: () => void }) {
  return (
    <button type="button" className="task-link" onClick={onOpen}>
      {card.isInstant && <span className="badge badge-instant">⚡</span>}
      <span className="task-link-title">{card.title}</span>
      <span className="task-link-status">{formatStatus(card.status)}</span>
    </button>
  );
}

/** ドロワーの中身。タスクが切り替わったら `key` で作り直して編集中の値をリセットする。 */
function DrawerBody({ detail, onSplit }: { detail: TaskDetail; onSplit: () => void }) {
  const mutate = useBoardStore((state) => state.mutate);
  const openTask = useBoardStore((state) => state.openTask);

  const [title, setTitle] = useState(detail.title);
  const [description, setDescription] = useState(detail.description);
  const [deadline, setDeadline] = useState(toDateTimeLocal(detail.deadline));
  const [note, setNote] = useState("");
  const [resourceKind, setResourceKind] = useState<ResourceKind>("url");
  const [resourceValue, setResourceValue] = useState("");
  const [resourceLabel, setResourceLabel] = useState("");

  const saveTitle = () => {
    const next = title.trim();
    if (!next || next === detail.title) {
      setTitle(detail.title);
      return;
    }
    void mutate(() => api.updateTask(detail.id, { title: next }));
  };

  const saveDescription = () => {
    if (description === detail.description) return;
    void mutate(() => api.updateTask(detail.id, { description }));
  };

  const saveDeadline = (value: string) => {
    setDeadline(value);
    const iso = fromDateTimeLocal(value);
    if (iso === null) {
      void mutate(() => api.updateTask(detail.id, { clearDeadline: true }));
    } else {
      void mutate(() => api.updateTask(detail.id, { deadline: iso }));
    }
  };

  const addResource = () => {
    const value = resourceValue.trim();
    if (!value) return;
    void mutate(async () => {
      await api.addResource(detail.id, {
        kind: resourceKind,
        value,
        label: resourceLabel.trim(),
      });
      setResourceValue("");
      setResourceLabel("");
    });
  };

  const addNote = () => {
    const body = note.trim();
    if (!body) return;
    void mutate(async () => {
      await api.addTaskUpdate(detail.id, body, "user");
      setNote("");
    });
  };

  return (
    <>
      <section className="drawer-section">
        <input
          className="drawer-title"
          value={title}
          aria-label="タイトル"
          onChange={(event) => setTitle(event.target.value)}
          onBlur={saveTitle}
          onKeyDown={(event) => {
            if (event.key === "Enter") event.currentTarget.blur();
            if (event.key === "Escape") setTitle(detail.title);
          }}
        />
        <div className="drawer-actions">
          {detail.status !== "done" && (
            <button
              type="button"
              className="btn btn-primary btn-sm"
              onClick={() => void mutate(() => api.completeTask(detail.id))}
            >
              ✓ 完了
            </button>
          )}
          {detail.isInstant && (
            <PromoteMenu
              className="btn btn-sm"
              onSelect={(column: BoardColumnKey) =>
                void mutate(() => api.promoteTask(detail.id, column))
              }
            />
          )}
          <button
            type="button"
            className="btn btn-sm"
            title="AI にサブタスクへの分割・詳細化を依頼する"
            onClick={onSplit}
          >
            ✨ AI で分割/詳細化
          </button>
        </div>
        <dl className="drawer-facts">
          <div>
            <dt>状態</dt>
            <dd>
              {formatStatus(detail.status)}
              {detail.isInstant && <span className="badge badge-instant">⚡ インスタント</span>}
            </dd>
          </div>
          <div>
            <dt>予定</dt>
            <dd>{formatScheduled(detail.scheduled)}</dd>
          </div>
          <div>
            <dt>作成</dt>
            <dd>{formatTimestamp(detail.createdAt)}</dd>
          </div>
        </dl>
      </section>

      <section className="drawer-section">
        <h3>締切</h3>
        <div className="deadline-row">
          <input
            type="datetime-local"
            value={deadline}
            aria-label="締切"
            className={isOverdue(detail.deadline) ? "input-overdue" : undefined}
            onChange={(event) => saveDeadline(event.target.value)}
          />
          {deadline && (
            <button type="button" className="btn btn-ghost btn-sm" onClick={() => saveDeadline("")}>
              クリア
            </button>
          )}
        </div>
      </section>

      <section className="drawer-section">
        <h3>詳細</h3>
        <textarea
          className="drawer-description"
          value={description}
          rows={6}
          placeholder="詳細を書く (Markdown)"
          aria-label="詳細"
          onChange={(event) => setDescription(event.target.value)}
          onBlur={saveDescription}
        />
      </section>

      <section className="drawer-section">
        <h3>関連リソース</h3>
        {detail.resources.length === 0 && <p className="muted">まだありません。</p>}
        <ul className="resource-list">
          {detail.resources.map((resource) => (
            <li key={resource.id} className="resource">
              <span
                className={resource.isPrimary ? "star star-on" : "star"}
                title={resource.isPrimary ? "主リソース" : undefined}
              >
                {resource.isPrimary ? "★" : "☆"}
              </span>
              <button
                type="button"
                className="resource-open"
                title={resource.value}
                onClick={() => {
                  openResource(resource.kind, resource.value).catch((error: unknown) =>
                    useBoardStore.getState().setError(toMessage(error)),
                  );
                }}
              >
                {resource.label || resource.value}
              </button>
              <span className="badge">{resource.kind === "url" ? "URL" : "ファイル"}</span>
              <button
                type="button"
                className="btn btn-ghost btn-sm"
                aria-label="リソースを削除"
                onClick={() => void mutate(() => api.removeResource(detail.id, resource.id))}
              >
                ✕
              </button>
            </li>
          ))}
        </ul>
        <form
          className="resource-form"
          onSubmit={(event) => {
            event.preventDefault();
            addResource();
          }}
        >
          <select
            value={resourceKind}
            aria-label="リソース種別"
            onChange={(event) => setResourceKind(event.target.value as ResourceKind)}
          >
            <option value="url">URL</option>
            <option value="file">ファイル</option>
          </select>
          <input
            type="text"
            value={resourceValue}
            placeholder={resourceKind === "url" ? "https://..." : "C:\\path\\to\\file"}
            aria-label="リソースの値"
            onChange={(event) => setResourceValue(event.target.value)}
          />
          <input
            type="text"
            value={resourceLabel}
            placeholder="ラベル (任意)"
            aria-label="リソースのラベル"
            onChange={(event) => setResourceLabel(event.target.value)}
          />
          <button type="submit" className="btn btn-sm" disabled={!resourceValue.trim()}>
            追加
          </button>
        </form>
      </section>

      <section className="drawer-section">
        <h3>親タスク</h3>
        {detail.parent ? (
          <div className="parent-row">
            <TaskLink card={detail.parent} onOpen={() => openTask(detail.parent!.id)} />
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={() => void mutate(() => api.setParent(detail.id, null))}
            >
              解除
            </button>
          </div>
        ) : (
          <p className="muted">ありません。</p>
        )}

        <h3>子タスク ({detail.children.length})</h3>
        {detail.children.length === 0 ? (
          <p className="muted">ありません。</p>
        ) : (
          <ul className="child-list">
            {detail.children.map((child) => (
              <li key={child.id}>
                <TaskLink card={child} onOpen={() => openTask(child.id)} />
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="drawer-section">
        <h3>アップデート履歴</h3>
        {detail.updates.length === 0 && <p className="muted">まだありません。</p>}
        <ol className="update-list">
          {detail.updates.map((update) => (
            <li key={update.id}>
              <div className="update-head">
                <span className="update-origin">{update.origin}</span>
                <time>{formatTimestamp(update.createdAt)}</time>
              </div>
              <p className="update-body">{update.body}</p>
            </li>
          ))}
        </ol>
        <form
          className="update-form"
          onSubmit={(event) => {
            event.preventDefault();
            addNote();
          }}
        >
          <textarea
            value={note}
            rows={3}
            placeholder="進捗メモを追記"
            aria-label="アップデートの本文"
            onChange={(event) => setNote(event.target.value)}
            onKeyDown={onCtrlEnter(addNote)}
          />
          <button type="submit" className="btn btn-primary btn-sm" disabled={!note.trim()}>
            追記
          </button>
        </form>
      </section>
    </>
  );
}

export function TaskDrawer() {
  const selectedId = useBoardStore((state) => state.selectedId);
  const detail = useBoardStore((state) => state.detail);
  const closeTask = useBoardStore((state) => state.closeTask);
  // ダイアログはドロワーの外に置く(スクロール領域の中だと重なりが崩れるため)。
  const [splitFor, setSplitFor] = useState<TaskId | null>(null);
  /** 開く前にフォーカスしていた要素。閉じたら戻す。 */
  const restoreTo = useRef<HTMLElement | null>(null);
  /** 直前に開いていたか。開閉の瞬間だけフォーカスを動かすための記憶。 */
  const wasOpen = useRef(false);

  // ダイアログやメニューを開いている間の Esc は、最前面のそれだけを閉じる。
  useEscapeKey(closeTask, { enabled: selectedId != null, priority: ESC_LAYER.drawer });

  // ドロワーはフォーカストラップまではしないが、閉じたら呼び出し元へフォーカスを返す
  // (カードから開いたなら、そのカードへキーボード操作が戻る)。
  // 中のリンクでタスクを切り替えたときは動かさない(開きっぱなしのため)。
  useEffect(() => {
    const open = selectedId != null;
    if (open && !wasOpen.current) {
      const opener = document.activeElement;
      restoreTo.current = opener instanceof HTMLElement ? opener : null;
    } else if (!open && wasOpen.current) {
      const element = restoreTo.current;
      restoreTo.current = null;
      if (element?.isConnected) element.focus();
    }
    wasOpen.current = open;
  }, [selectedId]);

  if (!selectedId) return null;

  return (
    <>
      <div className="drawer-scrim" onClick={closeTask} />
      <aside className="drawer" aria-label="タスク詳細">
        <header className="drawer-header">
          <span className="muted">タスク詳細</span>
          <button type="button" className="btn btn-ghost btn-sm" onClick={closeTask}>
            ✕ 閉じる
          </button>
        </header>
        <div className="drawer-content">
          {detail ? (
            <DrawerBody key={detail.id} detail={detail} onSplit={() => setSplitFor(detail.id)} />
          ) : (
            <p className="muted">読み込み中…</p>
          )}
        </div>
      </aside>
      {splitFor && <AiSplitDialog taskId={splitFor} onClose={() => setSplitFor(null)} />}
    </>
  );
}
