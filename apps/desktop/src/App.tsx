/** アプリのルート。初回フェッチとイベント購読を行い、ボードとドロワーを描画する。 */

import { useEffect } from "react";

import { listenTasksChanged } from "./api";
import { BoardView } from "./components/BoardView";
import { TaskDrawer } from "./components/TaskDrawer";
import { useBoardStore } from "./store";

function App() {
  const board = useBoardStore((state) => state.board);
  const ready = useBoardStore((state) => state.ready);
  const error = useBoardStore((state) => state.error);
  const refresh = useBoardStore((state) => state.refresh);
  const setError = useBoardStore((state) => state.setError);

  useEffect(() => {
    void refresh();
    // イベントのペイロードは見ず、再フェッチのトリガとしてのみ使う。
    const unlisten = listenTasksChanged(() => void refresh());
    return () => {
      void unlisten.then((off) => off()).catch(() => undefined);
    };
  }, [refresh]);

  return (
    <div className="app">
      <header className="app-header">
        <h1>questloom</h1>
        {board && <span className="muted">{board.today}</span>}
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
        <BoardView board={board} />
      ) : (
        <p className="placeholder">{ready ? "ボードを読み込めませんでした。" : "読み込み中…"}</p>
      )}

      <TaskDrawer />
    </div>
  );
}

export default App;
