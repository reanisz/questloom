/**
 * 設定画面の「MCP サーバー」節。稼働状態の表示と Claude Code への登録コマンド。
 *
 * トークンだけはコア設定と別扱い。実体は OS の資格情報ストア(Windows の資格情報
 * マネージャー)にあり、`get_settings` にも `set_settings` にも載らない。
 * そのため
 *
 * - **値は読み出せない**(表示切替も無い。見せるのは「設定済み / 未設定」だけ)、
 * - 保存は下の「保存」ボタンとは独立に、この節の操作で即座に行う
 *
 * という形にしてある。設定に成功すると `SettingsChanged` が飛んで MCP サーバーが
 * 張り直されるので、稼働状態も取り直す。
 */

import { useCallback, useEffect, useState } from "react";

import * as api from "../api";
import { claudeMcpCommand, MCP_PORT_RANGE, type SettingsDraft } from "../settings";
import { toMessage } from "../tauri";
import type { RuntimeStatus } from "../types";
import { CopyableCode, Field, Toggle } from "./SettingsControls";

interface Props {
  draft: SettingsDraft;
  patch: (changes: Partial<SettingsDraft>) => void;
  /** 稼働状態。取得できなかった場合は null。 */
  status: RuntimeStatus | null;
  /** 稼働状態を取り直す。 */
  onReloadStatus: () => void;
}

/** トークンの設定・解除。値の読み出しは無い。 */
function TokenField({ onChanged }: { onChanged: (configured: boolean) => void }) {
  /** 設定済みか。読み込み中は null。 */
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    api
      .getMcpTokenStatus()
      .then((value) => {
        if (!alive) return;
        setConfigured(value);
        onChanged(value);
      })
      .catch((cause: unknown) => {
        if (alive) setError(toMessage(cause));
      });
    return () => {
      alive = false;
    };
    // 初回のみ。以後の更新は下の apply が反映する。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const apply = useCallback(
    (token: string | null) => {
      if (busy) return;
      setBusy(true);
      setError(null);
      setNotice(null);
      api
        .setMcpToken(token)
        .then((value) => {
          setConfigured(value);
          setInput("");
          setNotice(value ? "トークンを設定しました。" : "トークンを解除しました。");
          onChanged(value);
        })
        .catch((cause: unknown) => setError(toMessage(cause)))
        .finally(() => setBusy(false));
    },
    [busy, onChanged],
  );

  return (
    <Field
      label="トークン"
      hint="設定すると Authorization: Bearer <token> を要求します。未設定なら認証なし。値は Windows の資格情報マネージャーに保存され、ここからは読み出せません。"
    >
      <p className="settings-status">
        {configured === null ? (
          <span className="muted">確認中…</span>
        ) : configured ? (
          <span className="settings-ok">● 設定済み</span>
        ) : (
          <span className="muted">○ 未設定(認証なし)</span>
        )}
      </p>
      <div className="settings-inline">
        <input
          type="password"
          className="settings-text"
          spellCheck={false}
          autoComplete="off"
          placeholder={configured ? "新しいトークン(空欄なら変更しない)" : "トークンを入力"}
          value={input}
          onChange={(event) => {
            setNotice(null);
            setInput(event.target.value);
          }}
        />
        <button
          type="button"
          className="btn btn-sm btn-primary"
          disabled={busy || input.trim() === ""}
          onClick={() => apply(input)}
        >
          {configured ? "変更" : "設定"}
        </button>
        <button
          type="button"
          className="btn btn-sm"
          disabled={busy || configured !== true}
          onClick={() => apply(null)}
        >
          クリア
        </button>
      </div>
      {error && <p className="settings-error">{error}</p>}
      {notice && <p className="settings-ok">{notice}</p>}
    </Field>
  );
}

export function McpSection({ draft, patch, status, onReloadStatus }: Props) {
  const [tokenConfigured, setTokenConfigured] = useState(false);
  const mcpUrl = status?.mcpUrl ?? `http://127.0.0.1:${draft.mcpPort || "?"}/mcp`;

  /** トークンを変えるとサーバーが張り直されるので、稼働状態も取り直す。 */
  const onTokenChanged = useCallback(
    (configured: boolean) => {
      setTokenConfigured(configured);
      onReloadStatus();
    },
    [onReloadStatus],
  );

  return (
    <section className="settings-section">
      <h2>MCP サーバー</h2>
      <p className="settings-lead muted">
        Claude Code などの AI エージェントから、この questloom のタスクを操作させるための
        内蔵サーバーです。待受は 127.0.0.1 のみです。
      </p>

      <Toggle
        label="内蔵 MCP サーバーを起動する"
        checked={draft.mcpEnabled}
        onChange={(mcpEnabled) => patch({ mcpEnabled })}
      />

      <Field label="ポート" hint={`${MCP_PORT_RANGE.min}〜${MCP_PORT_RANGE.max}。既定は 39150。`}>
        <input
          type="number"
          min={MCP_PORT_RANGE.min}
          max={MCP_PORT_RANGE.max}
          className="settings-number"
          value={draft.mcpPort}
          onChange={(event) => patch({ mcpPort: event.target.value })}
        />
      </Field>

      <TokenField onChanged={onTokenChanged} />

      <Field label="稼働状態">
        <p className="settings-status">
          {status === null ? (
            <span className="muted">取得できませんでした</span>
          ) : status.mcpRunning ? (
            <span className="settings-ok">
              ● 稼働中 {status.mcpUrl}
              <span className="muted">
                {status.mcpTokenRequired ? "(トークン認証あり)" : "(認証なし)"}
              </span>
            </span>
          ) : (
            <span className="settings-warn">
              ● 停止中(無効か、ポートが使用中の可能性があります)
            </span>
          )}
          <button type="button" className="btn btn-sm btn-ghost" onClick={onReloadStatus}>
            再取得
          </button>
        </p>
      </Field>

      <h3>Claude Code への登録</h3>
      <p className="settings-hint">
        保存後に、以下を PowerShell で実行すると接続できます。
        {tokenConfigured && "トークンの値は控えたものに置き換えてください。"}
      </p>
      <CopyableCode text={claudeMcpCommand(mcpUrl, tokenConfigured)} />
    </section>
  );
}
