/**
 * 設定画面の「プラグイン」節。
 *
 * plugin-host が公開したロード結果 (`plugin_list_loaded` /
 * `questloom://plugins-loaded`) を一覧し、各プラグインの `settingsSchema` から
 * フォームを自動生成する。
 *
 * 保存はコア設定とは独立で、プラグインごとの「保存」ボタンで
 * `plugin_set_settings` を呼ぶ(コア設定の保存ボタンは触らない)。
 *
 * **`type: "secret"` の項目だけは別経路。** 値は `settings` テーブルではなく OS の
 * 資格情報ストアに入るので、`plugin_secret_set` で保存し、画面には「設定済み /
 * 未設定」しか出さない(値は読み出せない)。
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

/** シークレット項目か。 */
function isSecret(field: PluginSettingField): boolean {
  return field.type === "secret";
}

/**
 * 保存値をフォーム用のドラフトへ落とす。
 *
 * シークレットは値を持たないので対象外(下の `SecretField` が別に扱う)。
 */
function toDraft(schema: readonly PluginSettingField[], settings: PluginSettings): Draft {
  const draft: Draft = {};
  for (const field of schema) {
    if (isSecret(field)) continue;
    const value = settings[field.key] ?? field.default ?? defaultForType(field.type);
    draft[field.key] = field.type === "boolean" ? Boolean(value) : String(value ?? "");
  }
  return draft;
}

/**
 * ドラフトを保存用の値へ戻す。数値として読めないものは既定値へ落とす。
 *
 * **シークレットは決して含めない。** ここへ混ぜると `settings` テーブルに平文で
 * 書き戻ってしまう。
 */
function fromDraft(schema: readonly PluginSettingField[], draft: Draft): PluginSettings {
  const value: PluginSettings = {};
  for (const field of schema) {
    if (isSecret(field)) continue;
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

/**
 * シークレット 1 項目の入力。
 *
 * 現在の値は読めないので、出せるのは「設定済み / 未設定」と、新しい値の入力欄と、
 * 「クリア」の予約だけ。実際の保存はカードの「保存」ボタンでまとめて行う。
 */
function SecretField({
  field,
  configured,
  input,
  cleared,
  onInput,
  onToggleClear,
}: {
  field: PluginSettingField;
  /** 設定済みか。確認中は null。 */
  configured: boolean | null;
  input: string;
  cleared: boolean;
  onInput: (value: string) => void;
  onToggleClear: () => void;
}) {
  return (
    <div className="settings-field">
      <span className="settings-field-label">{field.label || field.key}</span>
      <div className="settings-field-control">
        <p className="settings-status">
          {configured === null ? (
            <span className="muted">確認中…</span>
          ) : cleared ? (
            <span className="settings-warn">● 保存すると削除されます</span>
          ) : configured ? (
            <span className="settings-ok">● 設定済み</span>
          ) : (
            <span className="muted">○ 未設定</span>
          )}
        </p>
        <div className="settings-inline">
          <input
            type="password"
            className="settings-text"
            spellCheck={false}
            autoComplete="off"
            placeholder={configured ? "新しい値(空欄なら変更しない)" : "値を入力"}
            value={input}
            onChange={(event) => onInput(event.target.value)}
          />
          <button
            type="button"
            className="btn btn-sm"
            aria-pressed={cleared}
            disabled={configured !== true && !cleared}
            onClick={onToggleClear}
          >
            {cleared ? "取り消し" : "クリア"}
          </button>
        </div>
        <p className="settings-hint">
          値は Windows の資格情報マネージャーに保存され、ここからは読み出せません。
        </p>
        {field.hint && <p className="settings-hint">{field.hint}</p>}
      </div>
    </div>
  );
}

/** プラグイン 1 件のカード。設定フォームと保存ボタンを持つ。 */
function PluginCard({ plugin }: { plugin: LoadedPlugin }) {
  const manifest = plugin.manifest;
  const schema = manifest?.settingsSchema ?? [];
  const pluginId = manifest?.id ?? null;

  const secretFields = schema.filter(isSecret);

  const [draft, setDraft] = useState<Draft | null>(null);
  const [saving, setSaving] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** シークレットが設定済みか。未取得のキーは null 扱い。 */
  const [secretStatus, setSecretStatus] = useState<Record<string, boolean>>({});
  const [secretLoaded, setSecretLoaded] = useState(false);
  /** 入力された新しい値。空なら「変更しない」。 */
  const [secretInput, setSecretInput] = useState<Record<string, string>>({});
  /** 保存時に削除するキー。 */
  const [secretCleared, setSecretCleared] = useState<Record<string, boolean>>({});

  /** シークレットの設定状態を取り直す。 */
  const loadSecretStatus = useCallback(async (): Promise<Record<string, boolean>> => {
    if (!pluginId) return {};
    const entries = await Promise.all(
      secretFields.map(async (field) => [field.key, await papi.pluginSecretStatus(pluginId, field.key)] as const),
    );
    return Object.fromEntries(entries);
    // secretFields は manifest 由来で、プラグインが読み直されない限り変わらない。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pluginId]);

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
    loadSecretStatus()
      .then((status) => {
        if (!alive) return;
        setSecretStatus(status);
        setSecretLoaded(true);
      })
      .catch((cause: unknown) => {
        if (alive) setError(toMessage(cause));
      });
    return () => {
      alive = false;
    };
    // schema は manifest 由来で、プラグインが読み直されない限り変わらない。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pluginId, loadSecretStatus]);

  const save = useCallback(() => {
    if (!pluginId || !draft || saving) return;
    setSaving(true);
    setError(null);
    setNotice(null);
    // 非シークレットをまとめて保存してから、変更のあるシークレットだけを個別に書く。
    papi
      .pluginSetSettings(pluginId, fromDraft(schema, draft))
      .then(async () => {
        for (const field of secretFields) {
          const value = (secretInput[field.key] ?? "").trim();
          if (value !== "") {
            await papi.pluginSecretSet(pluginId, field.key, value);
          } else if (secretCleared[field.key]) {
            await papi.pluginSecretSet(pluginId, field.key, null);
          }
        }
      })
      .then(() => papi.pluginGetSettings(pluginId))
      .then(async (stored) => {
        setDraft(toDraft(schema, mergeSettings(schema, stored)));
        setSecretInput({});
        setSecretCleared({});
        setSecretStatus(await loadSecretStatus());
        setNotice("保存しました。");
      })
      .catch((cause: unknown) => setError(toMessage(cause)))
      .finally(() => setSaving(false));
    // secretFields / secretInput / secretCleared は下の依存に含めてある。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pluginId, draft, saving, schema, secretInput, secretCleared, loadSecretStatus]);

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
            <span className="plugin-badge">{plugin.builtin ? "標準" : "ユーザー"}</span>
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

      {plugin.builtin && (
        <p className="settings-hint muted">
          アプリに同梱されている標準プラグインです。更新するとこの版も更新されます。
        </p>
      )}

      {plugin.shadowsBuiltin && (
        <p className="settings-hint">
          標準版(アプリ同梱)を上書きしています。
          プラグインフォルダの <code>{plugin.fileName}</code> を削除すると同梱版に戻ります。
        </p>
      )}

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
          {schema.map((field) =>
            isSecret(field) ? (
              <SecretField
                key={field.key}
                field={field}
                configured={secretLoaded ? Boolean(secretStatus[field.key]) : null}
                input={secretInput[field.key] ?? ""}
                cleared={Boolean(secretCleared[field.key])}
                onInput={(value) => {
                  setNotice(null);
                  setSecretInput((current) => ({ ...current, [field.key]: value }));
                }}
                onToggleClear={() => {
                  setNotice(null);
                  setSecretCleared((current) => ({
                    ...current,
                    [field.key]: !current[field.key],
                  }));
                }}
              />
            ) : (
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
            ),
          )}

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
        「標準」プラグインはアプリに同梱されていて、そのまま使えます。加えて、下のフォルダに{" "}
        <code>.ts</code> / <code>.js</code> を置くと、起動時と「再読み込み」で自動的に
        読み込まれます(同じ id なら置いた方が優先されます)。
        プラグインは questloom が起動している間だけ動きます。
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
          {/* 同梱版と同名のユーザー版が並びうるので、key には置き場も混ぜる。 */}
          {plugins.map((plugin) => (
            <PluginCard
              key={`${plugin.builtin ? "builtin" : "user"}:${plugin.fileName}`}
              plugin={plugin}
            />
          ))}
        </div>
      )}
    </section>
  );
}
