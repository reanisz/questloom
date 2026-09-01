/**
 * タスク詳細ドロワー。右からスライドインし、タイトル・説明・締切の編集、
 * 関連リソース、状態アップデート履歴、親子タスクを扱う。
 */

import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useEffect, useRef, useState } from "react";

import * as api from "../api";
import { openByMode, openExternal } from "../browserPane";
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
import type {
  BoardColumnKey,
  ResourceKind,
  TaskCard,
  TaskDetail,
  TaskId,
  UrlOpenMode,
} from "../types";
import { AiSplitDialog } from "./AiSplitDialog";
import { ChecklistSection } from "./ChecklistSection";
import { DeleteConfirmDialog } from "./DeleteConfirmDialog";
import { PromoteMenu } from "./PromoteMenu";

/**
 * リソース名のクリック。URL は設定 (`urlOpenMode`) に従って開き、
 * file はエクスプローラで場所を表示する。
 *
 * 行の ↗ / 🌐 ボタンは設定に関わらず、それぞれ外部ブラウザ・内蔵ペインで開く。
 */
function openResource(kind: ResourceKind, value: string, mode: UrlOpenMode) {
  if (kind === "url") {
    openByMode(value, mode);
  } else {
    revealItemInDir(value).catch((error: unknown) =>
      useBoardStore.getState().setError(toMessage(error)),
    );
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
function DrawerBody({
  detail,
  onSplit,
  onDelete,
}: {
  detail: TaskDetail;
  onSplit: () => void;
  onDelete: () => void;
}) {
  const mutate = useBoardStore((state) => state.mutate);
  const openTask = useBoardStore((state) => state.openTask);
  const openPane = useBoardStore((state) => state.openPane);
  const urlOpenMode = useBoardStore((state) => state.urlOpenMode);

  const [title, setTitle] = useState(detail.title);
  const [description, setDescription] = useState(detail.description);
  const [deadline, setDeadline] = useState(toDateTimeLocal(detail.deadline));
  const [note, setNote] = useState("");
  const [resourceKind, setResourceKind] = useState<ResourceKind>("url");
  const [resourceValue, setResourceValue] = useState("");
  const [resourceLabel, setResourceLabel] = useState("");

  /*
   * `internalAuto` のときは、詳細を開いた時点で主リソース(URL)をペインに出す。
   * このコンポーネントは `key={detail.id}` で作り直されるので、
   * 「タスクを開いたとき 1 回」= マウント時 1 回でよい。
   *
   * ドロワーはペインを隠さない(隠すと自動表示の意味が無い)。重ならないことは
   * CSS 側で担保している(`--browser-pane-width` を使ったドロワー幅の上限)。
   */
  const autoOpen = urlOpenMode === "internalAuto";
  const primaryUrl = detail.primaryResource?.kind === "url" ? detail.primaryResource.value : null;
  useEffect(() => {
    if (autoOpen && primaryUrl) openPane(primaryUrl);
    // 開いた瞬間の値だけを見る。以後の再フェッチでは開き直さない。
  }, []);

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
          <button
            type="button"
            className="btn btn-sm btn-danger drawer-delete"
            data-testid="drawer-delete"
            title="このタスクを削除する (あとで復元できます)"
            onClick={onDelete}
          >
            🗑 削除
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
                title={
                  resource.kind === "url"
                    ? `${resource.value}\n${
                        urlOpenMode === "external"
                          ? "既定のブラウザで開く"
                          : "内蔵ブラウザで開く"
                      } (設定の「URL リソースの開き方」)`
                    : `${resource.value}\nエクスプローラで場所を表示する`
                }
                onClick={() => openResource(resource.kind, resource.value, urlOpenMode)}
              >
                {resource.label || resource.value}
              </button>
              <span className="badge">{resource.kind === "url" ? "URL" : "ファイル"}</span>
              {resource.kind === "url" && (
                <>
                  <button
                    type="button"
                    className="btn btn-ghost btn-sm"
                    data-testid="resource-open-external"
                    aria-label="既定のブラウザで開く"
                    title={`${resource.value}\n既定のブラウザで開く`}
                    onClick={() => openExternal(resource.value)}
                  >
                    ↗
                  </button>
                  <button
                    type="button"
                    className="btn btn-ghost btn-sm"
                    data-testid="resource-open-internal"
                    aria-label="内蔵ブラウザで開く"
                    title={`${resource.value}\nquestloom の内蔵ブラウザペインで開く`}
                    onClick={() => openPane(resource.value)}
                  >
                    🌐
                  </button>
                </>
              )}
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

      <ChecklistSection
        items={detail.checklist}
        progress={detail}
        onAdd={(body) => void mutate(() => api.addChecklistItem(detail.id, body))}
        onToggle={(item, checked) =>
          void mutate(() => api.updateChecklistItem(detail.id, item.id, { checked }))
        }
        onRename={(item, body) =>
          void mutate(() => api.updateChecklistItem(detail.id, item.id, { body }))
        }
        onRemove={(item) => void mutate(() => api.removeChecklistItem(detail.id, item.id))}
      />

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
  const mutate = useBoardStore((state) => state.mutate);
  // ダイアログはドロワーの外に置く(スクロール領域の中だと重なりが崩れるため)。
  const [splitFor, setSplitFor] = useState<TaskId | null>(null);
  /** 削除確認の対象。表示中のタスクのタイトルを添えて確認する。 */
  const [confirmDelete, setConfirmDelete] = useState<{ id: TaskId; title: string } | null>(null);
  const [deleting, setDeleting] = useState(false);
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

  /** 削除を確定する。ボードの更新は tasks-changed 任せ、ドロワーは閉じる。 */
  const runDelete = async (taskId: TaskId) => {
    setDeleting(true);
    const ok = await mutate(() => api.deleteTask(taskId));
    setDeleting(false);
    setConfirmDelete(null);
    if (ok) closeTask();
  };

  return (
    <>
      <div className="drawer-scrim" onClick={closeTask} />
      <aside className="drawer" data-testid="task-drawer" aria-label="タスク詳細">
        <header className="drawer-header">
          <span className="muted">タスク詳細</span>
          <button type="button" className="btn btn-ghost btn-sm" onClick={closeTask}>
            ✕ 閉じる
          </button>
        </header>
        <div className="drawer-content">
          {detail ? (
            <DrawerBody
              key={detail.id}
              detail={detail}
              onSplit={() => setSplitFor(detail.id)}
              onDelete={() => setConfirmDelete({ id: detail.id, title: detail.title })}
            />
          ) : (
            <p className="muted">読み込み中…</p>
          )}
        </div>
      </aside>
      {splitFor && <AiSplitDialog taskId={splitFor} onClose={() => setSplitFor(null)} />}
      {confirmDelete && (
        <DeleteConfirmDialog
          title={confirmDelete.title}
          busy={deleting}
          onConfirm={() => void runDelete(confirmDelete.id)}
          onClose={() => setConfirmDelete(null)}
        />
      )}
    </>
  );
}
