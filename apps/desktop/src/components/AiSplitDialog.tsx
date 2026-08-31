/**
 * タスク詳細から開く「AI で分割/詳細化」の小さなダイアログ。
 *
 * 追加指示(「3 つ以内で」など)だけを受け取り、結果は元タスクの子タスクとして
 * 作成される。子タスクの一覧はドロワーの再フェッチで自動的に更新される。
 */

import { useEffect, useState } from "react";

import * as api from "../api";
import type { AiCreateResult, TaskId } from "../types";
import { useAiProviders, useAiRun } from "../useAi";

export function AiSplitDialog({
  taskId,
  onClose,
}: {
  taskId: TaskId;
  onClose: () => void;
}) {
  const [instruction, setInstruction] = useState("");
  const [providerId, setProviderId] = useState("");
  const { providers, defaultId, error: providerError } = useAiProviders(true);
  const { busy, error, result, start, cancel } = useAiRun<AiCreateResult>();

  useEffect(() => {
    if (!providerId && defaultId) setProviderId(defaultId);
  }, [defaultId, providerId]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [busy, onClose]);

  const submit = () => {
    if (busy) return;
    void start(() => api.aiSplitTask(taskId, instruction, providerId || undefined));
  };

  return (
    <>
      <div className="modal-scrim" onClick={() => !busy && onClose()} />
      <div className="modal modal-sm" role="dialog" aria-modal="true" aria-label="AI で分割/詳細化">
        <header className="modal-header">
          <h2>AI で分割/詳細化</h2>
          <button type="button" className="btn btn-ghost btn-sm" disabled={busy} onClick={onClose}>
            ✕ 閉じる
          </button>
        </header>

        <div className="modal-body">
          <p className="ai-hint muted">
            タイトル・詳細・履歴を渡して、サブタスクを提案させます。結果は子タスクとして追加されます。
          </p>

          <label className="ai-provider">
            <span className="muted">プロバイダ</span>
            <select
              value={providerId}
              disabled={busy || providers.length === 0}
              onChange={(event) => setProviderId(event.target.value)}
            >
              {providers.length === 0 && <option value="">(利用可能なプロバイダなし)</option>}
              {providers.map((provider) => (
                <option key={provider.id} value={provider.id}>
                  {provider.label}
                </option>
              ))}
            </select>
          </label>

          {providerError && <p className="ai-error">{providerError}</p>}

          <textarea
            className="ai-input"
            rows={3}
            value={instruction}
            disabled={busy}
            aria-label="追加指示"
            placeholder="追加指示 (任意) 例) 3 つ以内で、今日中に着手できる粒度で"
            onChange={(event) => setInstruction(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
                event.preventDefault();
                submit();
              }
            }}
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
        </div>

        <footer className="modal-footer">
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
        </footer>
      </div>
    </>
  );
}
