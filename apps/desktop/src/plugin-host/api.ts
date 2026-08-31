/**
 * プラグイン基盤の Tauri command ラッパー。
 *
 * ここは `apps/desktop/src-tauri/src/plugin_host.rs` と 1 対 1 で対応する。
 * plugin-host ウィンドウとメインウィンドウ(設定画面)の両方から使う。
 *
 * **どの command をどちらのウィンドウから呼べるかは capability で決まる。**
 * `plugin_directory` / `plugin_set_settings` / `plugin_list_loaded` は設定画面専用で、
 * plugin-host からは ACL に拒否される(`src-tauri/capabilities/plugin-host.json`)。
 */

import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";

import { call } from "../tauri";
import type { LoadedPlugin, PluginSourceFile } from "../types";
import type { PluginSettings, PluginTaskResource } from "./sdk";

/*
 * イベント名は Rust 側の `apps/desktop/src-tauri/src/contract.rs` が発行元で、
 * ここはその写し。codegen はしていないので、**片方を変えたらもう片方も直すこと**
 * (タスク・AI 系のイベント名は `src/api.ts` 側にある)。
 */

/** ホストがロード結果を公開したときのイベント名。 */
export const PLUGINS_LOADED = "questloom://plugins-loaded";

/** 全プラグインの再読み込み要求イベント名。 */
export const PLUGINS_RELOAD = "questloom://plugins-reload";

/** プラグイン設定が保存されたときのイベント名。 */
export const PLUGIN_SETTINGS_CHANGED = "questloom://plugin-settings-changed";

/** プラグインディレクトリの絶対パス(無ければ作成される)。 */
export const pluginDirectory = () => call<string>("plugin_directory");

/** プラグインディレクトリ直下の `*.ts` / `*.js` をソース込みで列挙する。 */
export const pluginListSources = () => call<PluginSourceFile[]>("plugin_list_sources");

/** プラグイン専用 KV を読む。 */
export const pluginKvGet = (pluginId: string, key: string) =>
  call<unknown>("plugin_kv_get", { pluginId, key });

/** プラグイン専用 KV を書く。`null` でキー削除。 */
export const pluginKvSet = (pluginId: string, key: string, value: unknown) =>
  call<void>("plugin_kv_set", { pluginId, key, value: value ?? null });

/** プラグイン専用 KV のキー一覧。 */
export const pluginKvKeys = (pluginId: string) => call<string[]>("plugin_kv_keys", { pluginId });

/** プラグイン設定(名前空間 `plugin:<id>`)の保存値。未保存なら `{}`。 */
export const pluginGetSettings = (pluginId: string) =>
  call<PluginSettings>("plugin_get_settings", { pluginId });

/** プラグイン設定を保存する。保存時に `plugin-settings-changed` が発行される。 */
export const pluginSetSettings = (pluginId: string, value: PluginSettings) =>
  call<void>("plugin_set_settings", { pluginId, value });

/** 全タスクの関連リソース。 */
export const pluginListTaskResources = () =>
  call<PluginTaskResource[]>("plugin_list_task_resources");

/** プラグインのログを questloom 本体の tracing へ転送する。 */
export const pluginLog = (pluginId: string, level: string, message: string) =>
  call<void>("plugin_log", { pluginId, level, message });

/** URL が許可ドメインに含まれるか(判定は Rust 側の実装を唯一の正とする)。 */
export const pluginFetchAllowed = (url: string, domains: string[]) =>
  call<boolean>("plugin_fetch_allowed", { url, domains });

/** ホストがロード結果を公開する。 */
export const pluginPublishLoaded = (plugins: LoadedPlugin[]) =>
  call<void>("plugin_publish_loaded", { plugins });

/** 最後に公開されたロード結果を読む(設定画面が開いたときに使う)。 */
export const pluginListLoaded = () => call<LoadedPlugin[]>("plugin_list_loaded");

/** 全プラグインの再読み込みを要求する。 */
export const requestPluginReload = () => emit(PLUGINS_RELOAD);

/** ロード結果の更新を購読する。 */
export function listenPluginsLoaded(
  handler: (plugins: LoadedPlugin[]) => void,
): Promise<UnlistenFn> {
  return listen<LoadedPlugin[]>(PLUGINS_LOADED, (event) => handler(event.payload));
}

/** 再読み込み要求を購読する(plugin-host 専用)。 */
export function listenPluginReload(handler: () => void): Promise<UnlistenFn> {
  return listen(PLUGINS_RELOAD, () => handler());
}

/** プラグイン設定の変更を購読する(plugin-host 専用)。 */
export function listenPluginSettingsChanged(
  handler: (pluginId: string) => void,
): Promise<UnlistenFn> {
  return listen<{ pluginId: string }>(PLUGIN_SETTINGS_CHANGED, (event) =>
    handler(event.payload.pluginId),
  );
}
