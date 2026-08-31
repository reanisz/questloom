/**
 * ヘッダの「AI」ボタンから開くモーダル。
 *
 * - タスク作成: 文章を渡し、抽出されたタスクを New 列へ作成する
 * - 自由指示: MCP 経由で AI 自身にタスクを操作させ、応答テキストを表示する
 *
 * 作成結果はドメインイベント (`tasks-changed`) で自動的にボードへ反映される。
 * 枠(scrim・ヘッダ・Esc・フォーカス管理)は [`ModalShell`] が持つ。
 */

import { useState } from "react";

import * as api from "../api";
import { onCtrlEnter } from "../keyboard";
import { useBoardStore } from "../store";
import type { AiCreateResult, AiTextResult } from "../types";
import { useAiRun, useAiStatus } from "../useAi";
import { AiProviderSelect } from "./AiProviderSelect";
import { ModalShell } from "./ModalShell";

/** ダイアログのモード。 */
type Mode = "createTasks" | "freeInstruction";

const MODES: readonly { key: Mode; label: string; hint: string }[] = [
  {
    key: "createTasks",
    label: "タスク作成",
    hint: "メモや議事録を貼ると、タスクを抽出して New 列に作成します。",
  },
  {
    key: "freeInstruction",
    label: "自由指示",
    hint: "内蔵 MCP サーバー経由で、AI にタスクの整理・更新を任せます。",
  },
];

type Result = AiCreateResult | AiTextResult;

function isCreateResult(result: Result): result is AiCreateResult {
  return "created" in result;
}

export function AiDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [mode, setMode] = useState<Mode>("createTasks");
  const [text, setText] = useState("");
  const [providerId, setProviderId] = useState("");
  const [providerError, setProviderError] = useState<string | null>(null);
  const { busy, error, result, start, cancel, reset } = useAiRun<Result>();
  const status = useAiStatus();
  const openTask = useBoardStore((state) => state.openTask);

  if (!open) return null;

  const submit = () => {
    const body = text.trim();
    if (!body || busy) return;
    void start(() =>
      mode === "createTasks"
        ? api.aiCreateTasks(body, providerId || undefined)
        : api.aiFreeInstruction(body, providerId || undefined),
    );
  };

  const hint = MODES.find((entry) => entry.key === mode)?.hint;

  return (
    <ModalShell
      title="AI に依頼"
      busy={busy}
      onClose={onClose}
      footer={
        <>
          {busy ? (
            <button type="button" className="btn btn-sm" onClick={cancel}>
              ■ キャンセル
            </button>
          ) : (
            <span className="muted ai-shortcut">Ctrl+Enter で実行</span>
          )}
          <button
            type="button"
            className="btn btn-primary"
            // プロバイダが 1 つも無いときは providerId が空のままになる。
            disabled={busy || !text.trim() || !providerId}
            onClick={submit}
          >
            実行
          </button>
        </>
      }
    >
      <div className="ai-controls">
        <div className="ai-modes" role="group" aria-label="モード">
          {MODES.map((entry) => (
            <button
              key={entry.key}
              type="button"
              className={mode === entry.key ? "btn btn-sm btn-primary" : "btn btn-sm"}
              aria-pressed={mode === entry.key}
              disabled={busy}
              onClick={() => {
                setMode(entry.key);
                reset();
              }}
            >
              {entry.label}
            </button>
          ))}
        </div>

        <AiProviderSelect
          value={providerId}
          onChange={setProviderId}
          disabled={busy}
          onError={setProviderError}
        />
      </div>

      {hint && <p className="ai-hint muted">{hint}</p>}
      {providerError && <p className="ai-error">{providerError}</p>}

      <textarea
        className="ai-input"
        rows={8}
        value={text}
        disabled={busy}
        data-autofocus
        aria-label={mode === "createTasks" ? "タスクの元になる文章" : "AI への指示"}
        placeholder={
          mode === "createTasks"
            ? "例) 来週の打ち合わせまでに資料を用意して、田中さんにレビューを依頼する。請求書は水曜まで。"
            : "例) Today に溜まっているタスクを見直して、今週中で良いものを This Week に移して。"
        }
        onChange={(event) => setText(event.target.value)}
        onKeyDown={onCtrlEnter(submit)}
      />

      {busy && (
        <p className="ai-running">
          <span className="spinner" aria-hidden="true" />
          実行中… {status?.state === "running" && status.message}
        </p>
      )}
      {error && <p className="ai-error">{error}</p>}

      {result && isCreateResult(result) && (
        <section className="ai-result">
          <h3>{result.created.length} 件のタスクを作成しました</h3>
          <ul className="ai-created">
            {result.created.map((task) => (
              <li key={task.id}>
                <button type="button" className="task-link" onClick={() => openTask(task.id)}>
                  <span className="task-link-title">{task.title}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
      {result && !isCreateResult(result) && (
        <section className="ai-result">
          <h3>
            応答
            {!result.mcpAttached && <span className="badge">MCP 未接続</span>}
          </h3>
          <p className="ai-response">{result.text || "(応答なし)"}</p>
        </section>
      )}
    </ModalShell>
  );
}
