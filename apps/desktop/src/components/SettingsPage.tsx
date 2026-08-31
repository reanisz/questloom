/**
 * 設定ページ。ヘッダの歯車から開き、ボードを置き換えて表示する。
 *
 * 項目数が多いのでモーダルではなくページにし、左の節ナビゲーションで切り替える。
 * 自動保存はしない。「保存」を押したときだけ `set_settings` を一括で呼び、
 * 未保存のまま閉じようとしたら確認する。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import * as api from "../api";
import {
  AI_TIMEOUT_RANGE,
  claudeMcpCommand,
  emptyProvider,
  fromDraft,
  isDirty,
  issuesBySection,
  MCP_PORT_RANGE,
  SECTIONS,
  toDraft,
  validateDraft,
  type ProviderDraft,
  type SectionKey,
  type SettingsDraft,
} from "../settings";
import type { RuntimeStatus, WeekStart } from "../types";
import { PluginSettingsSection } from "./PluginSettingsSection";

/** クリップボードへコピーする。Tauri の webview では権限が無いこともあるので握りつぶさない。 */
async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // セキュアコンテキスト外などで clipboard API が使えない場合の保険。
    try {
      const area = document.createElement("textarea");
      area.value = text;
      area.style.position = "fixed";
      area.style.opacity = "0";
      document.body.appendChild(area);
      area.select();
      const ok = document.execCommand("copy");
      document.body.removeChild(area);
      return ok;
    } catch {
      return false;
    }
  }
}

/** ラベル付きの 1 行。説明文は入力の下に小さく出す。 */
function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="settings-field">
      <span className="settings-field-label">{label}</span>
      <div className="settings-field-control">
        {children}
        {hint && <p className="settings-hint">{hint}</p>}
      </div>
    </div>
  );
}

/** チェックボックス 1 個の行。 */
function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <div className="settings-field">
      <span className="settings-field-label" />
      <div className="settings-field-control">
        <label className="settings-check">
          <input
            type="checkbox"
            checked={checked}
            onChange={(event) => onChange(event.target.checked)}
          />
          <span>{label}</span>
        </label>
        {hint && <p className="settings-hint">{hint}</p>}
      </div>
    </div>
  );
}

/** コピーボタン付きのコード表示。 */
function CopyableCode({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  return (
    <div className="settings-code">
      <code>{text}</code>
      <button
        type="button"
        className="btn btn-sm"
        onClick={() => {
          void copyText(text).then((ok) => {
            setCopied(ok);
            if (ok) window.setTimeout(() => setCopied(false), 1600);
          });
        }}
      >
        {copied ? "✓ コピー済み" : "コピー"}
      </button>
    </div>
  );
}

export function SettingsPage({ onClose }: { onClose: () => void }) {
  const [section, setSection] = useState<SectionKey>("general");
  const [draft, setDraft] = useState<SettingsDraft | null>(null);
  /** 保存済みの内容。未保存判定の基準。 */
  const [baseline, setBaseline] = useState<SettingsDraft | null>(null);
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [showToken, setShowToken] = useState(false);
  /** 保存を試みたあとだけ、検証の指摘を表示する。 */
  const [showIssues, setShowIssues] = useState(false);

  const dirty = draft !== null && baseline !== null && isDirty(draft, baseline);
  const issues = useMemo(() => (draft ? validateDraft(draft) : []), [draft]);
  const counts = useMemo(() => issuesBySection(issues), [issues]);

  const loadStatus = useCallback(() => {
    api
      .getRuntimeStatus()
      .then(setStatus)
      .catch(() => setStatus(null));
  }, []);

  useEffect(() => {
    let alive = true;
    api
      .getSettings()
      .then((settings) => {
        if (!alive) return;
        setDraft(toDraft(settings));
        setBaseline(toDraft(settings));
        setLoadError(null);
      })
      .catch((cause) => {
        if (alive) setLoadError(api.toMessage(cause));
      });
    loadStatus();
    return () => {
      alive = false;
    };
  }, [loadStatus]);

  // 未保存の変更があるときだけ確認する。ハンドラは Esc からも使う。
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;
  const requestClose = useCallback(() => {
    if (dirtyRef.current && !window.confirm("未保存の変更があります。破棄して閉じますか?")) return;
    onClose();
  }, [onClose]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") requestClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [requestClose]);

  /** ドラフトの一部を差し替える。 */
  const patch = useCallback((changes: Partial<SettingsDraft>) => {
    setNotice(null);
    setSaveError(null);
    setDraft((current) => (current ? { ...current, ...changes } : current));
  }, []);

  const patchProvider = useCallback(
    (index: number, changes: Partial<ProviderDraft>) => {
      setNotice(null);
      setSaveError(null);
      setDraft((current) => {
        if (!current) return current;
        const providers = current.aiProviders.map((provider, at) =>
          at === index ? { ...provider, ...changes } : provider,
        );
        return { ...current, aiProviders: providers };
      });
    },
    [],
  );

  const save = () => {
    if (!draft || saving) return;
    const found = validateDraft(draft);
    setShowIssues(true);
    if (found.length > 0) {
      setNotice(null);
      setSaveError("入力に問題があります。下の指摘を確認してください。");
      setSection(found[0].section);
      return;
    }

    setSaving(true);
    setSaveError(null);
    setNotice(null);
    api
      .setSettings(fromDraft(draft))
      // 保存後の正規化(トリム等)を映すため、必ず読み直す。
      .then(() => api.getSettings())
      .then((settings) => {
        setDraft(toDraft(settings));
        setBaseline(toDraft(settings));
        setShowIssues(false);
        setNotice("設定を保存しました。");
        loadStatus();
      })
      .catch((cause) => setSaveError(api.toMessage(cause)))
      .finally(() => setSaving(false));
  };

  const restoreDefaults = () => {
    if (!window.confirm("すべての設定を既定値に戻しますか?(保存を押すまで反映されません)")) return;
    api
      .getDefaultSettings()
      .then((settings) => {
        setDraft(toDraft(settings));
        setShowIssues(false);
        setSaveError(null);
        setNotice("既定値を読み込みました。「保存」で反映されます。");
      })
      .catch((cause) => setSaveError(api.toMessage(cause)));
  };

  if (loadError) {
    return (
      <div className="settings">
        <p className="placeholder">設定を読み込めませんでした: {loadError}</p>
      </div>
    );
  }
  if (!draft) return <p className="placeholder">読み込み中…</p>;

  const mcpUrl = status?.mcpUrl ?? `http://127.0.0.1:${draft.mcpPort || "?"}/mcp`;

  return (
    <div className="settings">
      <header className="settings-header">
        <h1>設定</h1>
        {dirty && <span className="settings-dirty">● 未保存の変更があります</span>}
        <button type="button" className="btn btn-sm btn-ghost" onClick={requestClose}>
          ✕ 閉じる (Esc)
        </button>
      </header>

      <div className="settings-body">
        <nav className="settings-nav" aria-label="設定の節">
          {SECTIONS.map((entry) => (
            <button
              key={entry.key}
              type="button"
              className={
                section === entry.key ? "settings-nav-item is-active" : "settings-nav-item"
              }
              aria-current={section === entry.key}
              onClick={() => setSection(entry.key)}
            >
              <span className="settings-nav-label">
                {entry.label}
                {showIssues && counts[entry.key] > 0 && (
                  <span className="settings-nav-badge" title="入力に問題があります">
                    {counts[entry.key]}
                  </span>
                )}
              </span>
              <span className="settings-nav-hint">{entry.hint}</span>
            </button>
          ))}
        </nav>

        <div className="settings-pane">
          {section === "general" && (
            <section className="settings-section">
              <h2>一般</h2>
              <Field label="週の開始曜日" hint="This Week / Next Week の区切りに使います。">
                <select
                  value={draft.weekStart}
                  onChange={(event) => patch({ weekStart: event.target.value as WeekStart })}
                >
                  <option value="monday">月曜</option>
                  <option value="sunday">日曜</option>
                </select>
              </Field>

              <Toggle
                label="OS のログイン時に自動起動する"
                hint="保存すると、すぐにスタートアップ登録へ反映されます。"
                checked={draft.autostart}
                onChange={(autostart) => patch({ autostart })}
              />

              <Field
                label="バックアップ世代数"
                hint="起動時に DB をバックアップし、この件数より古いものから消します。"
              >
                <input
                  type="number"
                  min={1}
                  className="settings-number"
                  value={draft.backupGenerations}
                  onChange={(event) => patch({ backupGenerations: event.target.value })}
                />
              </Field>
            </section>
          )}

          {section === "shortcut" && (
            <section className="settings-section">
              <h2>ショートカットとオーバーレイ</h2>
              <Field
                label="グローバルショートカット"
                hint='メインウィンドウの表示・非表示を切り替えます。例: "Ctrl+Space" / "Alt+Shift+Q"。修飾キーは Ctrl / Alt / Shift / Super、空欄ならショートカットなし。'
              >
                <input
                  type="text"
                  className="settings-text"
                  placeholder="Ctrl+Space"
                  spellCheck={false}
                  value={draft.globalShortcut}
                  onChange={(event) => patch({ globalShortcut: event.target.value })}
                />
              </Field>

              <Field label="登録状態">
                <p className="settings-status">
                  {status === null ? (
                    <span className="muted">取得できませんでした</span>
                  ) : status.shortcutRegistered ? (
                    <span className="settings-ok">● 登録済み</span>
                  ) : (
                    <span className="settings-warn">
                      ● 未登録(未設定か、他のアプリが使用中の可能性があります)
                    </span>
                  )}
                </p>
              </Field>

              <Toggle
                label="New タスクがあるときにオーバーレイ通知を表示する"
                hint="メインディスプレイの左上に、最前面の小さな一覧を出します。"
                checked={draft.overlayEnabled}
                onChange={(overlayEnabled) => patch({ overlayEnabled })}
              />
            </section>
          )}

          {section === "mcp" && (
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
                  <button type="button" className="btn btn-sm btn-ghost" onClick={loadStatus}>
                    再取得
                  </button>
                </p>
              </Field>

              <h3>Claude Code への登録</h3>
              <p className="settings-hint">
                保存後に、以下を PowerShell で実行すると接続できます。
              </p>
              <CopyableCode text={claudeMcpCommand(mcpUrl, draft.mcpToken)} />
            </section>
          )}

          {section === "ai" && (
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
                            onChange={(event) =>
                              patchProvider(index, { enabled: event.target.checked })
                            }
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
                            onChange={(event) =>
                              patchProvider(index, { label: event.target.value })
                            }
                          />
                        </td>
                        <td className="col-command">
                          <input
                            type="text"
                            spellCheck={false}
                            aria-label="command"
                            value={provider.command}
                            onChange={(event) =>
                              patchProvider(index, { command: event.target.value })
                            }
                          />
                        </td>
                        <td className="col-args">
                          <input
                            type="text"
                            spellCheck={false}
                            aria-label="args"
                            value={provider.argsText}
                            onChange={(event) =>
                              patchProvider(index, { argsText: event.target.value })
                            }
                          />
                          {provider.mcpArgs.length > 0 && (
                            <p className="settings-hint">
                              MCP 接続時は前に{" "}
                              <code>{provider.mcpArgs.join(" ")}</code> が付きます。
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
                              patch({
                                aiProviders: draft.aiProviders.filter((_, at) => at !== index),
                              })
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
          )}

          {/* プラグイン節はコア設定と独立して保存する(下の「保存」ボタンは無関係)。 */}
          {section === "plugins" && <PluginSettingsSection />}
        </div>
      </div>

      {showIssues && issues.length > 0 && (
        <ul className="settings-issues" role="alert">
          {issues.map((issue, index) => (
            <li key={index}>{issue.message}</li>
          ))}
        </ul>
      )}

      <footer className="settings-footer">
        <button type="button" className="btn btn-sm btn-ghost" onClick={restoreDefaults}>
          既定値に戻す
        </button>
        <div className="settings-footer-feedback">
          {saveError && <span className="settings-error">{saveError}</span>}
          {notice && <span className="settings-ok">{notice}</span>}
        </div>
        <button type="button" className="btn" onClick={requestClose}>
          閉じる
        </button>
        <button
          type="button"
          className="btn btn-primary"
          disabled={saving || !dirty}
          onClick={save}
        >
          {saving ? "保存中…" : "保存"}
        </button>
      </footer>
    </div>
  );
}
