/**
 * タスク詳細から開く「AI で分割/詳細化」の小さなダイアログ。
 *
 * 追加指示(「3 つ以内で」など)だけを受け取り、結果は元タスクの子タスクとして
 * 作成される。子タスクの一覧はタスク変更イベントによる再フェッチで更新される。
 * 枠(scrim・ヘッダ・Esc・フォーカス管理)は [`ModalShell`] が持つ。
 */

import { useState } from "react";

import * as api from "../api";
import { onCtrlEnter } from "../keyboard";
import type { AiCreateResult, TaskId } from "../types";
import { useAiRun } from "../useAi";
import { AiProviderSelect } from "./AiProviderSelect";
import { ModalShell } from "./ModalShell";

export function AiSplitDialog({ taskId, onClose }: { taskId: TaskId; onClose: () => void }) {
  const [instruction, setInstruction] = useState("");
  const [providerId, setProviderId] = useState("");
  const [providerError, setProviderError] = useState<string | null>(null);
  const { busy, error, result, start, cancel } = useAiRun<AiCreateResult>();

  const submit = () => {
    if (busy) return;
    void start(() => api.aiSplitTask(taskId, instruction, providerId || undefined));
  };

  return (
    <ModalShell
      title="AI で分割/詳細化"
      className="modal-sm"
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
          <button type="button" className="btn btn-primary" disabled={busy} onClick={submit}>
            実行
          </button>
        </>
      }
    >
      <p className="ai-hint muted">
        タイトル・詳細・履歴を渡して、サブタスクを提案させます。結果は子タスクとして追加されます。
      </p>

      <AiProviderSelect
        value={providerId}
        onChange={setProviderId}
        disabled={busy}
        onError={setProviderError}
      />

      {providerError && <p className="ai-error">{providerError}</p>}

      <textarea
        className="ai-input"
        rows={3}
        value={instruction}
        disabled={busy}
        data-autofocus
        aria-label="追加指示"
        placeholder="追加指示 (任意) 例) 3 つ以内で、今日中に着手できる粒度で"
        onChange={(event) => setInstruction(event.target.value)}
        onKeyDown={onCtrlEnter(submit)}
      />

      {busy && (
        <p className="ai-running">
          <span className="spinner" aria-hidden="true" />
          分割中…
        </p>
      )}
      {error && <p className="ai-error">{error}</p>}
      {result && (
        <p className="ai-result-line">{result.created.length} 件の子タスクを作成しました。</p>
      )}
    </ModalShell>
  );
}
