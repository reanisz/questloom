/** アプリのルート。初回フェッチとイベント購読を行い、ボードとドロワーを描画する。 */

import { useEffect, useState } from "react";

import { listenOpenTask, listenTasksChanged } from "./api";
import { AiDialog } from "./components/AiDialog";
import { BoardView } from "./components/BoardView";
import { TaskDrawer } from "./components/TaskDrawer";
import { TitleBar } from "./components/TitleBar";
import { useBoardStore } from "./store";
import { useExpandedView } from "./viewMode";

function App() {
  const board = useBoardStore((state) => state.board);
  const ready = useBoardStore((state) => state.ready);
  const error = useBoardStore((state) => state.error);
  const refresh = useBoardStore((state) => state.refresh);
  const setError = useBoardStore((state) => state.setError);
  const openTask = useBoardStore((state) => state.openTask);
  const [expanded, setExpanded] = useExpandedView();
  const [aiOpen, setAiOpen] = useState(false);

  useEffect(() => {
    void refresh();
    // イベントのペイロードは見ず、再フェッチのトリガとしてのみ使う。
    const unlisten = listenTasksChanged(() => void refresh());
    return () => {
      void unlisten.then((off) => off()).catch(() => undefined);
    };
  }, [refresh]);

  useEffect(() => {
    // オーバーレイのクリックから「このタスクを開く」と言われたら詳細を開く。
    const unlisten = listenOpenTask((taskId) => openTask(taskId));
    return () => {
      void unlisten.then((off) => off()).catch(() => undefined);
    };
  }, [openTask]);

  return (
    <div className="app">
      <TitleBar />

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
            aria-pressed={expanded}
            title={
              expanded
                ? "New / Today / Doing / Done + 先送りレールの表示に戻す"
                : "先送りバケットも列として展開する"
            }
            onClick={() => setExpanded(!expanded)}
          >
            {expanded ? "▤ 通常表示" : "▦ 全列を展開"}
          </button>
        </div>
      </header>

      {error && (
        <div className="banner" role="alert">
          <span>{error}</span>
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => setError(null)}>
            ✕
          </button>
        </div>
      )}

      {board ? (
        <BoardView board={board} expanded={expanded} onExpand={() => setExpanded(true)} />
      ) : (
        <p className="placeholder">{ready ? "ボードを読み込めませんでした。" : "読み込み中…"}</p>
      )}

      <TaskDrawer />
      <AiDialog open={aiOpen} onClose={() => setAiOpen(false)} />
    </div>
  );
}

export default App;
