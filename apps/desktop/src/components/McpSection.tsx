/** 設定画面の「MCP サーバー」節。稼働状態の表示と Claude Code への登録コマンド。 */

import { useState } from "react";

import { claudeMcpCommand, MCP_PORT_RANGE, type SettingsDraft } from "../settings";
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

export function McpSection({ draft, patch, status, onReloadStatus }: Props) {
  const [showToken, setShowToken] = useState(false);
  const mcpUrl = status?.mcpUrl ?? `http://127.0.0.1:${draft.mcpPort || "?"}/mcp`;

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

      <Field
        label="トークン"
        hint="設定すると Authorization: Bearer <token> を要求します。空欄なら認証なし。"
      >
        <div className="settings-inline">
          <input
            type={showToken ? "text" : "password"}
            className="settings-text"
            spellCheck={false}
            autoComplete="off"
            value={draft.mcpToken}
            onChange={(event) => patch({ mcpToken: event.target.value })}
          />
          <button
            type="button"
            className="btn btn-sm"
            aria-pressed={showToken}
            onClick={() => setShowToken(!showToken)}
          >
            {showToken ? "隠す" : "表示"}
          </button>
        </div>
      </Field>

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
      <p className="settings-hint">保存後に、以下を PowerShell で実行すると接続できます。</p>
      <CopyableCode text={claudeMcpCommand(mcpUrl, draft.mcpToken)} />
    </section>
  );
}
