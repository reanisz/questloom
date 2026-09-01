/** アプリのルート。初回フェッチとイベント購読を行い、ボードとドロワーを描画する。 */

import { useEffect, useState } from "react";

import { listenOpenTask, listenTasksChanged } from "./api";
import { AiDialog } from "./components/AiDialog";
import { BoardView } from "./components/BoardView";
import { BrowserPane } from "./components/BrowserPane";
import { DeletedTasksDialog } from "./components/DeletedTasksDialog";
import { SettingsPage } from "./components/SettingsPage";
import { TaskDrawer } from "./components/TaskDrawer";
import { TitleBar } from "./components/TitleBar";
import { useBoardStore } from "./store";
import { useTauriEvent } from "./useTauriEvent";
import { useExpandedView } from "./viewMode";

/** 表示中のページ。設定はモーダルではなくボードを置き換えるページとして出す。 */
type Page = "board" | "settings";

function App() {
  const board = useBoardStore((state) => state.board);
  const ready = useBoardStore((state) => state.ready);
  const error = useBoardStore((state) => state.error);
  const refresh = useBoardStore((state) => state.refresh);
  const setError = useBoardStore((state) => state.setError);
  const openTask = useBoardStore((state) => state.openTask);
  const paneUrl = useBoardStore((state) => state.paneUrl);
  const loadUrlOpenMode = useBoardStore((state) => state.loadUrlOpenMode);
  const [expanded, setExpanded] = useExpandedView();
  const [aiOpen, setAiOpen] = useState(false);
  const [deletedOpen, setDeletedOpen] = useState(false);
  const [page, setPage] = useState<Page>("board");

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // URL リソースの開き方だけはボード側でも要るので、コア設定から写しておく。
  // 設定画面から戻ったときにも読み直す(設定変更の通知イベントは無い)。
  useEffect(() => {
    if (page === "board") void loadUrlOpenMode();
  }, [loadUrlOpenMode, page]);

  // 書き込みの結果はすべてこのイベント経由で反映する(store.mutate は再フェッチしない)。
  // ペイロードは見ず、再フェッチのトリガとしてのみ使う。
  useTauriEvent(listenTasksChanged, () => void refresh());

  // オーバーレイのクリックから「このタスクを開く」と言われたら詳細を開く。
  useTauriEvent(listenOpenTask, openTask);

  return (
    <div className="app">
      <TitleBar />

      {error && (
        <div className="banner" role="alert">
          <span>{error}</span>
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => setError(null)}>
            ✕
          </button>
        </div>
      )}

      <div className="app-body">
        {/* 内蔵ブラウザペイン。開いている間だけ、ボードの左に場所を取る。 */}
        {paneUrl && <BrowserPane occluded={page !== "board"} />}

        <div className="app-main">
          {page === "settings" ? (
            <SettingsPage onClose={() => setPage("board")} />
          ) : (
            <>
              <header className="app-header">
                {board && <span className="muted">{board.today}</span>}
                <div className="app-header-actions">
                  <button
                    type="button"
                    className="btn btn-sm"
                    title="AI に依頼する (タスク作成 / 自由指示)"
                    onClick={() => setAiOpen(true)}
                  >
                    ✨ AI
                  </button>
                  <button
                    type="button"
                    className="btn btn-sm btn-ghost"
                    data-testid="open-deleted"
                    title="削除したタスクを見る / 復元する"
                    onClick={() => setDeletedOpen(true)}
                  >
                    🗑 削除済み
                  </button>
                  <button
                    type="button"
                    className="btn btn-sm btn-ghost"
                    data-testid="toggle-expanded"
                    aria-pressed={expanded}
                    title={
                      expanded
                        ? "New / Today / Doing / Done + 先送りレールの表示に戻す"
                        : "先送りバケット・Icebox・監視中も列として展開する"
                    }
                    onClick={() => setExpanded(!expanded)}
                  >
                    {expanded ? "▤ 通常表示" : "▦ 全列を展開"}
                  </button>
                  <button
                    type="button"
                    className="btn btn-sm btn-ghost btn-icon"
                    title="設定"
                    aria-label="設定"
                    onClick={() => setPage("settings")}
                  >
                    ⚙
                  </button>
                </div>
              </header>

              {board ? (
                <BoardView board={board} expanded={expanded} onExpand={() => setExpanded(true)} />
              ) : (
                <p className="placeholder">{ready ? "ボードを読み込めませんでした。" : "読み込み中…"}</p>
              )}

              <TaskDrawer />
              <AiDialog open={aiOpen} onClose={() => setAiOpen(false)} />
              {deletedOpen && <DeletedTasksDialog onClose={() => setDeletedOpen(false)} />}
            </>
          )}
        </div>
      </div>
    </div>
  );
}

export default App;
