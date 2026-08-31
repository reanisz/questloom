/**
 * 設定画面の「プラグイン」節。
 *
 * plugin-host が公開したロード結果 (`plugin_list_loaded` /
 * `questloom://plugins-loaded`) を一覧し、各プラグインの `settingsSchema` から
 * フォームを自動生成する。
 *
 * 保存はコア設定とは独立で、プラグインごとの「保存」ボタンで
 * `plugin_set_settings` を呼ぶ(コア設定の保存ボタンは触らない)。
 */

import { useCallback, useEffect, useState } from "react";

import * as papi from "../plugin-host/api";
import {
  defaultForType,
  mergeSettings,
  type PluginSettingField,
  type PluginSettings,
} from "../plugin-host/sdk";
import { toMessage } from "../tauri";
import type { LoadedPlugin } from "../types";
import { useTauriEvent } from "../useTauriEvent";

/** 編集中の値。数値も編集途中の文字列で持つ(打ち直しできるようにするため)。 */
type Draft = Record<string, string | boolean>;

/** 保存値をフォーム用のドラフトへ落とす。 */
function toDraft(schema: readonly PluginSettingField[], settings: PluginSettings): Draft {
  const draft: Draft = {};
  for (const field of schema) {
    const value = settings[field.key] ?? field.default ?? defaultForType(field.type);
    draft[field.key] = field.type === "boolean" ? Boolean(value) : String(value ?? "");
  }
  return draft;
}

/** ドラフトを保存用の値へ戻す。数値として読めないものは既定値へ落とす。 */
function fromDraft(schema: readonly PluginSettingField[], draft: Draft): PluginSettings {
  const value: PluginSettings = {};
  for (const field of schema) {
    const raw = draft[field.key];
    if (field.type === "boolean") {
      value[field.key] = Boolean(raw);
    } else if (field.type === "number") {
      const parsed = Number(String(raw).trim());
      value[field.key] = Number.isFinite(parsed) ? parsed : (field.default ?? 0);
    } else {
      value[field.key] = String(raw ?? "");
    }
  }
  return value;
}

/** プラグイン 1 件のカード。設定フォームと保存ボタンを持つ。 */
function PluginCard({ plugin }: { plugin: LoadedPlugin }) {
  const manifest = plugin.manifest;
  const schema = manifest?.settingsSchema ?? [];
  const pluginId = manifest?.id ?? null;

  const [draft, setDraft] = useState<Draft | null>(null);
  const [saving, setSaving] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [revealed, setRevealed] = useState<Record<string, boolean>>({});

  useEffect(() => {
    if (!pluginId || schema.length === 0) return;
    let alive = true;
    papi
      .pluginGetSettings(pluginId)
      .then((stored) => {
        if (alive) setDraft(toDraft(schema, mergeSettings(schema, stored)));
      })
      .catch((cause: unknown) => {
        if (alive) setError(toMessage(cause));
      });
    return () => {
      alive = false;
    };
    // schema は manifest 由来で、プラグインが読み直されない限り変わらない。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pluginId]);

  const save = useCallback(() => {
    if (!pluginId || !draft || saving) return;
    setSaving(true);
    setError(null);
    setNotice(null);
    papi
      .pluginSetSettings(pluginId, fromDraft(schema, draft))
      .then(() => papi.pluginGetSettings(pluginId))
      .then((stored) => {
        setDraft(toDraft(schema, mergeSettings(schema, stored)));
        setNotice("保存しました。");
      })
      .catch((cause: unknown) => setError(toMessage(cause)))
      .finally(() => setSaving(false));
  }, [pluginId, draft, saving, schema]);

  const patch = (key: string, value: string | boolean) => {
    setNotice(null);
    setDraft((current) => (current ? { ...current, [key]: value } : current));
  };

  return (
    <article className="plugin-card">
      <header className="plugin-card-head">
        <div>
          <h3>
            {manifest?.name ?? plugin.fileName}
            {manifest?.version && <span className="muted"> v{manifest.version}</span>}
          </h3>
          <p className="settings-hint">
            <code>{plugin.fileName}</code>
            {manifest && <> / id: <code>{manifest.id}</code></>}
          </p>
        </div>
        <span className={plugin.active ? "settings-ok" : "settings-warn"}>
          {plugin.active ? "● 有効" : "● 無効"}
        </span>
      </header>

      {manifest?.description && <p className="settings-lead muted">{manifest.description}</p>}

      {plugin.error && (
        <pre className="plugin-card-error" role="alert">
          {plugin.error}
        </pre>
      )}

      {manifest && manifest.fetchDomains && manifest.fetchDomains.length > 0 && (
        <p className="settings-hint">
          fetch 許可ドメイン: {manifest.fetchDomains.map((domain) => (
            <code key={domain}>{domain}</code>
          ))}
        </p>
      )}

      {schema.length > 0 && draft && (
        <>
          {schema.map((field) => (
            <div className="settings-field" key={field.key}>
              <span className="settings-field-label">{field.label || field.key}</span>
              <div className="settings-field-control">
                {field.type === "boolean" ? (
                  <label className="settings-check">
                    <input
                      type="checkbox"
                      checked={Boolean(draft[field.key])}
                      onChange={(event) => patch(field.key, event.target.checked)}
                    />
                    <span>有効</span>
                  </label>
                ) : field.type === "number" ? (
                  <input
                    type="number"
                    className="settings-number"
                    value={String(draft[field.key] ?? "")}
                    onChange={(event) => patch(field.key, event.target.value)}
                  />
                ) : field.type === "secret" ? (
                  <div className="settings-inline">
                    <input
                      type={revealed[field.key] ? "text" : "password"}
                      className="settings-text"
                      spellCheck={false}
                      autoComplete="off"
                      value={String(draft[field.key] ?? "")}
                      onChange={(event) => patch(field.key, event.target.value)}
                    />
                    <button
                      type="button"
                      className="btn btn-sm"
                      aria-pressed={Boolean(revealed[field.key])}
                      onClick={() =>
                        setRevealed((current) => ({
                          ...current,
                          [field.key]: !current[field.key],
                        }))
                      }
                    >
                      {revealed[field.key] ? "隠す" : "表示"}
                    </button>
                  </div>
                ) : (
                  <input
                    type="text"
                    className="settings-text"
                    value={String(draft[field.key] ?? "")}
                    onChange={(event) => patch(field.key, event.target.value)}
                  />
                )}
                {field.hint && <p className="settings-hint">{field.hint}</p>}
              </div>
            </div>
          ))}

          <div className="plugin-card-actions">
            {error && <span className="settings-error">{error}</span>}
            {notice && <span className="settings-ok">{notice}</span>}
            <button type="button" className="btn btn-sm btn-primary" disabled={saving} onClick={save}>
              {saving ? "保存中…" : "このプラグインの設定を保存"}
            </button>
          </div>
        </>
      )}

      {manifest && schema.length === 0 && (
        <p className="settings-hint muted">このプラグインに設定項目はありません。</p>
      )}
    </article>
  );
}

export function PluginSettingsSection() {
  const [plugins, setPlugins] = useState<LoadedPlugin[] | null>(null);
  const [directory, setDirectory] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reloading, setReloading] = useState(false);

  const refresh = useCallback(() => {
    papi
      .pluginListLoaded()
      .then(setPlugins)
      .catch((cause: unknown) =>
        setError(toMessage(cause)),
      );
  }, []);

  useEffect(() => {
    refresh();
    papi.pluginDirectory().then(setDirectory).catch(() => setDirectory(null));
  }, [refresh]);

  // ホストが読み直したら一覧を差し替える。
  useTauriEvent(papi.listenPluginsLoaded, (loaded) => {
    setPlugins(loaded);
    setReloading(false);
  });

  const reload = () => {
    setReloading(true);
    setError(null);
    void papi
      .requestPluginReload()
      .catch((cause: unknown) => {
        setReloading(false);
        setError(toMessage(cause));
      });
    // ホストが応答しない場合に「再読み込み中」で固まらないよう保険をかける。
    window.setTimeout(() => setReloading(false), 8000);
  };

  return (
    <section className="settings-section">
      <h2>プラグイン</h2>
      <p className="settings-lead muted">
        下のフォルダに <code>.ts</code> / <code>.js</code> を置くと、起動時と「再読み込み」で
        自動的に読み込まれます。プラグインは questloom が起動している間だけ動きます。
      </p>

      <div className="settings-field">
        <span className="settings-field-label">プラグインフォルダ</span>
        <div className="settings-field-control">
          <div className="settings-code">
            <code>{directory ?? "取得できませんでした"}</code>
          </div>
          <p className="settings-hint">
            ファイルを追加・編集したら「再読み込み」を押してください。
          </p>
        </div>
      </div>

      <div className="settings-inline">
        <button type="button" className="btn btn-sm" disabled={reloading} onClick={reload}>
          {reloading ? "再読み込み中…" : "プラグインを再読み込み"}
        </button>
        <button type="button" className="btn btn-sm btn-ghost" onClick={refresh}>
          一覧を再取得
        </button>
      </div>

      {error && <p className="settings-error">{error}</p>}

      {plugins === null ? (
        <p className="settings-hint muted">読み込み中…</p>
      ) : plugins.length === 0 ? (
        <p className="settings-hint muted">
          読み込まれたプラグインはありません。
        </p>
      ) : (
        <div className="plugin-list">
          {plugins.map((plugin) => (
            <PluginCard key={plugin.fileName} plugin={plugin} />
          ))}
        </div>
      )}
    </section>
  );
}
