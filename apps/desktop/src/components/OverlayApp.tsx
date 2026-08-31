/**
 * オーバーレイ通知ウィンドウのルート。
 *
 * 表示 / 非表示そのものは Rust 側(overlay.rs)が New タスク件数を見て制御するため、
 * ここは「今の New タスクを描く」ことだけに集中する。
 * ウィンドウ高さは内容に合わせて自分で調整する(幅は tauri.conf.json の 360px 固定)。
 */

import { LogicalSize } from "@tauri-apps/api/dpi";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { completeTask, getBoard, listenTasksChanged, showMainWindow } from "../api";
import type { TaskCard } from "../types";
import { useTauriEvent } from "../useTauriEvent";

/** ウィンドウ幅(論理ピクセル)。tauri.conf.json の overlay と揃えること。 */
const OVERLAY_WIDTH = 360;
/** 一覧に出す最大件数。これを超えた分は「他 n 件」にまとめる。 */
const MAX_ROWS = 5;

/** 内容の高さに合わせてウィンドウをリサイズするフック。 */
function useAutoHeight(ref: React.RefObject<HTMLDivElement | null>): void {
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const appWindow = getCurrentWindow();
    const apply = () => {
      const height = Math.max(1, Math.ceil(element.getBoundingClientRect().height));
      void appWindow.setSize(new LogicalSize(OVERLAY_WIDTH, height)).catch(() => undefined);
    };

    apply();
    const observer = new ResizeObserver(apply);
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);
}

/** New タスクを購読するフック。真実の情報源はバックエンド。 */
function useNewTasks(): TaskCard[] {
  const [tasks, setTasks] = useState<TaskCard[]>([]);

  const refresh = useCallback(() => {
    void getBoard()
      .then((board) => setTasks(board.columns.new))
      // 取得に失敗しても通知を出しっぱなしにしないよう、空にして畳む。
      .catch(() => setTasks([]));
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  useTauriEvent(listenTasksChanged, refresh);

  return tasks;
}

/** 主リソースが URL のときだけ、その URL を返す。 */
function primaryUrl(task: TaskCard): string | null {
  const resource = task.primaryResource;
  return resource && resource.kind === "url" ? resource.value : null;
}

function TaskRow({ task }: { task: TaskCard }) {
  const url = primaryUrl(task);

  return (
    <div
      className={`overlay-row${task.isInstant ? " is-instant" : ""}`}
      role="button"
      tabIndex={0}
      title={task.title}
      onClick={() => void showMainWindow(task.id)}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") void showMainWindow(task.id);
      }}
    >
      {task.isInstant && (
        <span className="overlay-bolt" aria-label="インスタントタスク">
          ⚡
        </span>
      )}
      <span className="overlay-title">{task.title}</span>

      {task.isInstant && (
        <span className="overlay-actions">
          {url && (
            <button
              type="button"
              className="overlay-btn"
              title={url}
              onClick={(event) => {
                event.stopPropagation();
                void openUrl(url).catch(() => undefined);
              }}
            >
              開く
            </button>
          )}
          <button
            type="button"
            className="overlay-btn overlay-btn-done"
            title="完了にする"
            onClick={(event) => {
              event.stopPropagation();
              void completeTask(task.id).catch(() => undefined);
            }}
          >
            ✓ 完了
          </button>
        </span>
      )}
    </div>
  );
}

export function OverlayApp() {
  const tasks = useNewTasks();
  const cardRef = useRef<HTMLDivElement>(null);
  useAutoHeight(cardRef);

  const shown = tasks.slice(0, MAX_ROWS);
  const rest = tasks.length - shown.length;

  return (
    <div className="overlay" ref={cardRef}>
      {tasks.length > 0 && (
        <div className="overlay-card">
          <div className="overlay-head">
            <span className="overlay-dot" aria-hidden />
            <span className="overlay-heading">New タスク</span>
            <span className="overlay-count">{tasks.length}</span>
          </div>

          <div className="overlay-list">
            {shown.map((task) => (
              <TaskRow key={task.id} task={task} />
            ))}
          </div>

          {rest > 0 && (
            <button type="button" className="overlay-more" onClick={() => void showMainWindow()}>
              他 {rest} 件
            </button>
          )}
        </div>
      )}
    </div>
  );
}
