/** 設定画面の「AI」節。既定プロバイダ・タイムアウトと、プロバイダ定義の表。 */

import {
  AI_TIMEOUT_RANGE,
  emptyProvider,
  type ProviderDraft,
  type SettingsDraft,
} from "../settings";
import { Field } from "./SettingsControls";

interface Props {
  draft: SettingsDraft;
  patch: (changes: Partial<SettingsDraft>) => void;
  /** プロバイダ 1 行だけを差し替える。 */
  patchProvider: (index: number, changes: Partial<ProviderDraft>) => void;
}

export function AiSection({ draft, patch, patchProvider }: Props) {
  return (
    <section className="settings-section">
      <h2>AI</h2>

      <Field label="既定のプロバイダ" hint="AI ダイアログでプロバイダを選ばなかったときに使います。">
        <select
          value={draft.aiDefaultProviderId}
          onChange={(event) => patch({ aiDefaultProviderId: event.target.value })}
        >
          {/* 一覧に無い id が保存されている場合でも選択を失わせない。 */}
          {!draft.aiProviders.some(
            (provider) => provider.id.trim() === draft.aiDefaultProviderId.trim(),
          ) && <option value={draft.aiDefaultProviderId}>{draft.aiDefaultProviderId}</option>}
          {draft.aiProviders.map((provider, index) => (
            <option key={`${provider.id}-${index}`} value={provider.id}>
              {provider.label || provider.id || `(${index + 1} 行目)`}
              {provider.enabled ? "" : "(無効)"}
            </option>
          ))}
        </select>
      </Field>

      <Field
        label="タイムアウト(秒)"
        hint={`${AI_TIMEOUT_RANGE.min}〜${AI_TIMEOUT_RANGE.max}。超えたら CLI のプロセスを終了します。`}
      >
        <input
          type="number"
          min={AI_TIMEOUT_RANGE.min}
          max={AI_TIMEOUT_RANGE.max}
          className="settings-number"
          value={draft.aiTimeoutSecs}
          onChange={(event) => patch({ aiTimeoutSecs: event.target.value })}
        />
      </Field>

      <h3>プロバイダ</h3>
      <p className="settings-hint">
        <code>command</code> は PATH から解決します。<code>args</code> の{" "}
        <code>{"{prompt}"}</code> がプロンプト本文に置き換わります(この要素が無い場合は
        標準入力から渡されます)。スペース区切り、または JSON 配列
        <code>{'["-p","{prompt}"]'}</code> で書けます。スペースを含む引数は{" "}
        <code>&quot;</code> で括ってください。
      </p>

      <div className="settings-table-wrap">
        <table className="settings-table">
          <thead>
            <tr>
              <th className="col-enabled">有効</th>
              <th className="col-id">id</th>
              <th className="col-label">表示名</th>
              <th className="col-command">command</th>
              <th className="col-args">args</th>
              <th className="col-remove" />
            </tr>
          </thead>
          <tbody>
            {draft.aiProviders.length === 0 && (
              <tr>
                <td colSpan={6} className="settings-table-empty">
                  プロバイダがありません。「+ 追加」で作成してください。
                </td>
              </tr>
            )}
            {draft.aiProviders.map((provider, index) => (
              <tr key={index}>
                <td className="col-enabled">
                  <input
                    type="checkbox"
                    aria-label={`${provider.label || provider.id} を有効にする`}
                    checked={provider.enabled}
                    onChange={(event) => patchProvider(index, { enabled: event.target.checked })}
                  />
                </td>
                <td className="col-id">
                  <input
                    type="text"
                    spellCheck={false}
                    aria-label="id"
                    value={provider.id}
                    onChange={(event) => {
                      const id = event.target.value;
                      // 既定プロバイダに指定されている行の id を変えたら追随させる。
                      if (draft.aiDefaultProviderId === provider.id) {
                        patch({ aiDefaultProviderId: id });
                      }
                      patchProvider(index, { id });
                    }}
                  />
                </td>
                <td className="col-label">
                  <input
                    type="text"
                    aria-label="表示名"
                    value={provider.label}
                    onChange={(event) => patchProvider(index, { label: event.target.value })}
                  />
                </td>
                <td className="col-command">
                  <input
                    type="text"
                    spellCheck={false}
                    aria-label="command"
                    value={provider.command}
                    onChange={(event) => patchProvider(index, { command: event.target.value })}
                  />
                </td>
                <td className="col-args">
                  <input
                    type="text"
                    spellCheck={false}
                    aria-label="args"
                    value={provider.argsText}
                    onChange={(event) => patchProvider(index, { argsText: event.target.value })}
                  />
                  {provider.mcpArgs.length > 0 && (
                    <p className="settings-hint">
                      MCP 接続時は前に <code>{provider.mcpArgs.join(" ")}</code> が付きます。
                    </p>
                  )}
                </td>
                <td className="col-remove">
                  <button
                    type="button"
                    className="btn btn-sm btn-ghost"
                    title="この行を削除"
                    aria-label={`${provider.label || provider.id} を削除`}
                    onClick={() =>
                      patch({ aiProviders: draft.aiProviders.filter((_, at) => at !== index) })
                    }
                  >
                    ✕
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <button
        type="button"
        className="btn btn-sm"
        onClick={() => patch({ aiProviders: [...draft.aiProviders, emptyProvider()] })}
      >
        + 追加
      </button>
    </section>
  );
}
