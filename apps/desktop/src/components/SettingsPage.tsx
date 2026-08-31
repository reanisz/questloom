/**
 * 設定ページ。ヘッダの歯車から開き、ボードを置き換えて表示する。
 *
 * 項目数が多いのでモーダルではなくページにし、左の節ナビゲーションで切り替える。
 * 自動保存はしない。「保存」を押したときだけ `set_settings` を一括で呼び、
 * 未保存のまま閉じようとしたら確認する。
 *
 * ここが持つのはドラフト・検証・保存だけで、各節の描画は
 * `GeneralSection` / `ShortcutSection` / `McpSection` / `AiSection` /
 * `PluginSettingsSection` に分けてある(プラグイン節だけは保存も独立)。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import * as api from "../api";
import { ESC_LAYER, useEscapeKey } from "../keyboard";
import {
  fromDraft,
  isDirty,
  issuesBySection,
  SECTIONS,
  toDraft,
  validateDraft,
  type ProviderDraft,
  type SectionKey,
  type SettingsDraft,
} from "../settings";
import { toMessage } from "../tauri";
import type { RuntimeStatus } from "../types";
import { AiSection } from "./AiSection";
import { GeneralSection } from "./GeneralSection";
import { McpSection } from "./McpSection";
import { PluginSettingsSection } from "./PluginSettingsSection";
import { ShortcutSection } from "./ShortcutSection";

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
      .catch((cause: unknown) => {
        if (alive) setLoadError(toMessage(cause));
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

  // ページなので最下層。ドロワーやダイアログが開いていればそちらが先に閉じる。
  useEscapeKey(requestClose, { priority: ESC_LAYER.page });

  /** ドラフトの一部を差し替える。 */
  const patch = useCallback((changes: Partial<SettingsDraft>) => {
    setNotice(null);
    setSaveError(null);
    setDraft((current) => (current ? { ...current, ...changes } : current));
  }, []);

  const patchProvider = useCallback((index: number, changes: Partial<ProviderDraft>) => {
    setNotice(null);
    setSaveError(null);
    setDraft((current) => {
      if (!current) return current;
      const providers = current.aiProviders.map((provider, at) =>
        at === index ? { ...provider, ...changes } : provider,
      );
      return { ...current, aiProviders: providers };
    });
  }, []);

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
      .catch((cause: unknown) => setSaveError(toMessage(cause)))
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
      .catch((cause: unknown) => setSaveError(toMessage(cause)));
  };

  if (loadError) {
    return (
      <div className="settings">
        <p className="placeholder">設定を読み込めませんでした: {loadError}</p>
      </div>
    );
  }
  if (!draft) return <p className="placeholder">読み込み中…</p>;

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
              className={section === entry.key ? "settings-nav-item is-active" : "settings-nav-item"}
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
          {section === "general" && <GeneralSection draft={draft} patch={patch} />}

          {section === "shortcut" && (
            <ShortcutSection draft={draft} patch={patch} status={status} />
          )}

          {section === "mcp" && (
            <McpSection draft={draft} patch={patch} status={status} onReloadStatus={loadStatus} />
          )}

          {section === "ai" && (
            <AiSection draft={draft} patch={patch} patchProvider={patchProvider} />
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
