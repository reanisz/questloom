/**
 * オーバーレイ通知ウィンドウのルート。
 *
 * 表示 / 非表示そのものは Rust 側(overlay.rs)が New タスク件数を見て制御するため、
 * ここは「今の New タスクを描く」ことだけに集中する。
 * ウィンドウの大きさは内容の実測に合わせて自分で調整する(展開中の幅は
 * tauri.conf.json と揃えた 360px 固定、折りたたみ中はインジケータの実寸)。
 *
 * ヘッダを押すと折りたたまれ、小さなインジケータだけになる。状態は localStorage に
 * 残るので再起動しても維持される。**新しいタスクが増えても勝手には展開しない**
 * (件数だけライブに更新し、増えた瞬間に軽くパルスさせる)。
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
/** `.overlay` の padding(論理ピクセル)。styles.css と揃えること。 */
const OVERLAY_PADDING = 6;

/**
 * 折りたたみ状態の localStorage キー。値は "1"(折りたたみ)/ "0"(展開)。
 *
 * バックエンドに持たせるほどの情報ではないので、`viewMode.ts` と同じ作法で
 * localStorage に置く(使えない環境でも落ちないよう例外は握り潰す)。
 */
export const OVERLAY_COLLAPSED_KEY = "questloom.overlay.collapsed";

function loadCollapsed(): boolean {
  try {
    return localStorage.getItem(OVERLAY_COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}

function saveCollapsed(collapsed: boolean): void {
  try {
    localStorage.setItem(OVERLAY_COLLAPSED_KEY, collapsed ? "1" : "0");
  } catch {
    // 保存できなくても表示は続けられるので無視する。
  }
}

/** 折りたたみ状態を localStorage に永続化しつつ保持する。 */
function useCollapsed(): [boolean, (collapsed: boolean) => void] {
  const [collapsed, setCollapsedState] = useState<boolean>(loadCollapsed);

  const setCollapsed = useCallback((next: boolean) => {
    setCollapsedState(next);
    saveCollapsed(next);
  }, []);

  return [collapsed, setCollapsed];
}

/**
 * 内容の実寸に合わせてウィンドウをリサイズするフック。
 *
 * 高さは常にルート要素(`.overlay`)の実測。幅は展開中は [`OVERLAY_WIDTH`] 固定だが、
 * 折りたたみ中はインジケータの実寸に合わせる(ルート要素はウィンドウ幅いっぱいに
 * 広がるので、幅だけは中身の要素を測る)。
 */
function useAutoSize(
  rootRef: React.RefObject<HTMLDivElement | null>,
  indicatorRef: React.RefObject<HTMLElement | null>,
  collapsed: boolean,
): void {
  useLayoutEffect(() => {
    const element = rootRef.current;
    if (!element) return;
    const appWindow = getCurrentWindow();
    const apply = () => {
      const height = Math.max(1, Math.ceil(element.getBoundingClientRect().height));
      const indicator = collapsed ? indicatorRef.current : null;
      const width = indicator
        ? Math.max(1, Math.ceil(indicator.getBoundingClientRect().width)) + OVERLAY_PADDING * 2
        : OVERLAY_WIDTH;
      void appWindow.setSize(new LogicalSize(width, height)).catch(() => undefined);
    };

    apply();
    const observer = new ResizeObserver(apply);
    observer.observe(element);
    // ルートはウィンドウ幅に張り付いて動かないので、幅の変化はインジケータ側で拾う。
    if (collapsed && indicatorRef.current) observer.observe(indicatorRef.current);
    return () => observer.disconnect();
  }, [rootRef, indicatorRef, collapsed]);
}

/**
 * 件数が増えるたびに変わる値を返す(パルスアニメーションの再生用)。
 *
 * これを `key` にすると要素が作り直され、CSS アニメーションが頭から流れる。
 * 初回描画では 0 のまま = アニメーションしない。減ったときも動かさない。
 */
function usePulse(count: number): number {
  const [pulse, setPulse] = useState(0);
  const previous = useRef(count);

  if (previous.current !== count) {
    // 0 件からの復帰はインジケータが現れるところなので数えない
    // (初回フェッチの 0 → n もここで吸収される)。
    const increased = previous.current > 0 && count > previous.current;
    previous.current = count;
    if (increased) setPulse((value) => value + 1);
  }

  return pulse;
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

/** 折りたたみ中の小さなインジケータ。押すと展開する。 */
function Indicator({
  count,
  pulse,
  onExpand,
  indicatorRef,
}: {
  count: number;
  pulse: number;
  onExpand: () => void;
  indicatorRef: React.RefObject<HTMLButtonElement | null>;
}) {
  return (
    <button
      type="button"
      ref={indicatorRef}
      className="overlay-indicator"
      data-testid="overlay-indicator"
      title="New タスクを開く"
      aria-expanded={false}
      aria-label={`New タスク ${count} 件。クリックで展開`}
      onClick={onExpand}
    >
      <span className="overlay-bolt" aria-hidden>
        ⚡
      </span>
      {/* key を変えることでパルスを再生する(初回は pulse === 0 なので動かない)。 */}
      <span
        key={pulse}
        className={`overlay-indicator-count${pulse > 0 ? " is-pulse" : ""}`}
        data-testid="overlay-indicator-count"
      >
        {count}
      </span>
    </button>
  );
}

export function OverlayApp() {
  const tasks = useNewTasks();
  const rootRef = useRef<HTMLDivElement>(null);
  const indicatorRef = useRef<HTMLButtonElement>(null);
  const [collapsed, setCollapsed] = useCollapsed();
  const pulse = usePulse(tasks.length);
  // 中身が無いときは展開時と同じ幅にしておく(次に出るのは展開状態かもしれない)。
  useAutoSize(rootRef, indicatorRef, collapsed && tasks.length > 0);

  const shown = tasks.slice(0, MAX_ROWS);
  const rest = tasks.length - shown.length;

  return (
    <div className="overlay" ref={rootRef}>
      {tasks.length > 0 &&
        (collapsed ? (
          <Indicator
            count={tasks.length}
            pulse={pulse}
            indicatorRef={indicatorRef}
            onExpand={() => setCollapsed(false)}
          />
        ) : (
          <div className="overlay-card">
            {/*
             * ヘッダには他に押せるものが無いので、行まるごとを折りたたみトグルにする。
             * 行の操作(開く / 完了)は下の一覧側なので干渉しない。
             */}
            <button
              type="button"
              className="overlay-head"
              data-testid="overlay-collapse"
              title="折りたたむ"
              aria-expanded
              onClick={() => setCollapsed(true)}
            >
              <span className="overlay-dot" aria-hidden />
              <span className="overlay-heading">New タスク</span>
              <span className="overlay-count">{tasks.length}</span>
              <span className="overlay-chevron" aria-hidden>
                ▾
              </span>
            </button>

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
        ))}
    </div>
  );
}
