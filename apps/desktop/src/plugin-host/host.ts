/**
 * TypeScript プラグインのホスト。非表示の `plugin-host` webview 上で動く。
 *
 * ライフサイクルはすべてここが持つ(Rust 側は列挙・永続化・ログ転送だけ)。
 *
 * ```
 * startHost()
 *   ├ グローバル defineQuestloomPlugin を用意
 *   ├ questloom://plugins-reload / plugin-settings-changed を購読
 *   └ load()
 *       1. plugin_list_sources() でソースを列挙
 *       2. ソースごとに: トランスパイル → Blob URL → import() → default export を検証
 *          (id が重複したら後から来た方を拒否する)
 *       3. 生き残った各プラグインの activate(ctx) を呼ぶ
 *       4. plugin_publish_loaded() で結果を公開(設定画面が読む)
 * reloadPlugins()  ← questloom://plugins-reload
 *   └ dispose() → load()
 * ```
 *
 * **エラー分離**: load / activate / ポーリング / イベントハンドラで出た例外はすべて
 * ここで捕捉してログに出す。1 つのプラグインの失敗が他のプラグインや本体へ
 * 波及しないようにするため。
 */

import * as api from "../api";
import type { LoadedPlugin, PluginSourceFile, TaskCard } from "../types";
import * as papi from "./api";
import {
  defineQuestloomPlugin,
  mergeSettings,
  type PluginContext,
  type PluginLogger,
  type PluginLogLevel,
  type PluginManifest,
  type PluginSettings,
  type QuestloomPlugin,
} from "./sdk";
import { transpilePlugin } from "./transpile";

/** ホスト自身のログに使う擬似プラグイン id。 */
const HOST_ID = "_host";

/** ホストの現在の状態。デバッグ表示用に購読できる。 */
export interface HostState {
  /** ロード中か。 */
  loading: boolean;
  /** ロード結果(ファイル名昇順)。 */
  plugins: LoadedPlugin[];
  /** 最後にロードを終えた時刻 (ISO 8601)。 */
  loadedAt: string | null;
  /** ホスト自体が動けなかった場合のメッセージ。 */
  hostError: string | null;
}

/** 起動中のプラグイン 1 件の内部状態。 */
interface ActivePlugin {
  fileName: string;
  manifest: PluginManifest;
  /** ctx が登録した後始末と、activate の戻り値の dispose。 */
  cleanups: (() => void | Promise<void>)[];
  /** 設定変更の購読者。 */
  settingsHandlers: Set<(settings: PluginSettings) => void>;
  /** モジュールの Blob URL。dispose 時に revoke する。 */
  objectUrl: string;
}

const active: ActivePlugin[] = [];
const listeners = new Set<(state: HostState) => void>();

let state: HostState = { loading: false, plugins: [], loadedAt: null, hostError: null };
/** ロードの直列化。reload を連打されても順番に処理する。 */
let queue: Promise<void> = Promise.resolve();
let started = false;

/** ホストの状態を購読する。登録時に現在値を 1 回渡す。戻り値で解除。 */
export function subscribeHost(handler: (state: HostState) => void): () => void {
  listeners.add(handler);
  handler(state);
  return () => {
    listeners.delete(handler);
  };
}

function setState(changes: Partial<HostState>): void {
  state = { ...state, ...changes };
  for (const listener of listeners) listener(state);
}

/** 値を安全に文字列化する(循環参照でも落ちない)。 */
function stringify(value: unknown): string {
  if (typeof value === "string") return value;
  if (value instanceof Error) return value.stack ?? `${value.name}: ${value.message}`;
  try {
    return JSON.stringify(value) ?? String(value);
  } catch {
    return String(value);
  }
}

/** 例外を人が読める文字列にする。 */
function describeError(error: unknown): string {
  return stringify(error);
}

/** questloom 本体の tracing へログを送る。失敗してもホストは止めない。 */
function forwardLog(pluginId: string, level: PluginLogLevel, message: string): void {
  void papi.pluginLog(pluginId, level, message).catch(() => {
    console.warn(`[questloom:${pluginId}] ${message}`);
  });
}

/** ホスト自身のログ。 */
function hostLog(level: PluginLogLevel, message: string): void {
  forwardLog(HOST_ID, level, message);
}

/* ------------------------------------------------------------------ context */

/** プラグイン用のロガーを作る。 */
function createLogger(pluginId: string): PluginLogger {
  const emit = (level: PluginLogLevel, args: unknown[]) =>
    forwardLog(pluginId, level, args.map(stringify).join(" "));
  const logger = ((...args: unknown[]) => emit("info", args)) as PluginLogger;
  logger.debug = (...args: unknown[]) => emit("debug", args);
  logger.info = (...args: unknown[]) => emit("info", args);
  logger.warn = (...args: unknown[]) => emit("warn", args);
  logger.error = (...args: unknown[]) => emit("error", args);
  return logger;
}

/** プラグイン 1 件分の `ctx` を組み立てる。 */
function createContext(entry: ActivePlugin): PluginContext {
  const id = entry.manifest.id;
  // プラグインが作ったタスク・履歴の出所を固定する(プラグイン側の指定は上書きする)。
  const origin = `plugin:${id}`;
  const log = createLogger(id);

  return {
    manifest: entry.manifest,

    // 本体と同じ型付きラッパー (src/api.ts) を使う。origin だけ引数で固定する。
    tasks: {
      createTask: (input) => api.createTask({ ...input, origin }),
      getTask: (taskId) => api.getTask(taskId),
      listTasks: async () => {
        const board = await api.getBoard();
        return Object.values(board.columns).flat() as TaskCard[];
      },
      completeTask: (taskId) => api.completeTask(taskId),
      addTaskUpdate: (taskId, body) => api.addTaskUpdate(taskId, body, origin),
      addResource: (taskId, resource) => api.addResource(taskId, resource),
      listAllResources: () => papi.pluginListTaskResources(),
      moveTask: (taskId, column) => api.moveTask(taskId, { column }),
    },

    settings: {
      get: async () =>
        mergeSettings(entry.manifest.settingsSchema, await papi.pluginGetSettings(id)),
      onChange: (handler) => {
        entry.settingsHandlers.add(handler);
        return () => {
          entry.settingsHandlers.delete(handler);
        };
      },
    },

    kv: {
      get: async <T,>(key: string): Promise<T | undefined> => {
        const value = await papi.pluginKvGet(id, key);
        return (value ?? undefined) as T | undefined;
      },
      set: (key, value) => papi.pluginKvSet(id, key, value ?? null),
      keys: () => papi.pluginKvKeys(id),
    },

    fetch: async (url, init) => {
      const domains = entry.manifest.fetchDomains ?? [];
      // 判定は Rust 側の実装を唯一の正とする(テストもそちらにある)。
      if (!(await papi.pluginFetchAllowed(url, domains))) {
        throw new Error(
          `プラグイン "${id}" は ${url} を取得できません。manifest.fetchDomains に` +
            `許可するホスト名を宣言してください(現在: ` +
            `${domains.length === 0 ? "宣言なし" : domains.join(", ")})。`,
        );
      }
      return fetch(url, init);
    },

    schedule: (intervalMinutes, fn) => {
      const run = () => {
        try {
          const result = fn();
          if (result instanceof Promise) {
            result.catch((error: unknown) => {
              log.error(`ポーリングが失敗しました: ${describeError(error)}`);
            });
          }
        } catch (error) {
          log.error(`ポーリングが失敗しました: ${describeError(error)}`);
        }
      };
      // 次の間隔まで何も起きないのを避けるため、登録直後に 1 回走らせる。
      run();
      const minutes = Number.isFinite(intervalMinutes) && intervalMinutes > 0 ? intervalMinutes : 5;
      const timer = window.setInterval(run, minutes * 60_000);
      const stop = () => {
        window.clearInterval(timer);
      };
      entry.cleanups.push(stop);
      return stop;
    },

    onTaskEvent: (handler) => {
      let off: (() => void) | null = null;
      let stopped = false;
      void api
        .listenTasksChanged(() => {
          try {
            handler();
          } catch (error) {
            log.error(`onTaskEvent が失敗しました: ${describeError(error)}`);
          }
        })
        .then((unlisten) => {
          // 購読が張られる前に dispose された場合はすぐ解除する。
          if (stopped) unlisten();
          else off = unlisten;
        })
        .catch((error: unknown) => {
          log.error(`タスクイベントを購読できませんでした: ${describeError(error)}`);
        });
      const stop = () => {
        stopped = true;
        off?.();
        off = null;
      };
      entry.cleanups.push(stop);
      return stop;
    },

    log,
  };
}

/* --------------------------------------------------------------- load/unload */

/** トランスパイル済みのソースを Blob URL 経由で読み込み、default export を検証して返す。 */
async function importPlugin(
  fileName: string,
  source: string,
): Promise<{ plugin: QuestloomPlugin; objectUrl: string }> {
  const code = await transpilePlugin(fileName, source);
  const objectUrl = URL.createObjectURL(new Blob([code], { type: "text/javascript" }));
  try {
    const module: unknown = await import(/* @vite-ignore */ objectUrl);
    const exported = (module as { default?: unknown }).default;
    if (!exported) {
      throw new Error(
        "default export がありません。`export default defineQuestloomPlugin({...})` を書いてください。",
      );
    }
    // グローバルを通さず素のオブジェクトを返された場合に備え、ここでも検証する。
    return { plugin: defineQuestloomPlugin(exported as QuestloomPlugin), objectUrl };
  } catch (error) {
    URL.revokeObjectURL(objectUrl);
    throw error;
  }
}

/** すべてのプラグインを読み込み、activate まで行う。 */
async function load(): Promise<void> {
  setState({ loading: true, hostError: null });

  let sources: PluginSourceFile[];
  try {
    sources = await papi.pluginListSources();
  } catch (error) {
    const message = describeError(error);
    hostLog("error", `プラグインを列挙できませんでした: ${message}`);
    setState({
      loading: false,
      hostError: message,
      plugins: [],
      loadedAt: new Date().toISOString(),
    });
    void papi.pluginPublishLoaded([]).catch(() => undefined);
    return;
  }

  const results: LoadedPlugin[] = [];
  const claimed = new Set<string>();

  for (const source of sources) {
    let imported: { plugin: QuestloomPlugin; objectUrl: string };
    try {
      imported = await importPlugin(source.fileName, source.source);
    } catch (error) {
      results.push({
        fileName: source.fileName,
        manifest: null,
        active: false,
        error: describeError(error),
      });
      continue;
    }

    const manifest = imported.plugin.manifest;
    if (claimed.has(manifest.id)) {
      URL.revokeObjectURL(imported.objectUrl);
      results.push({
        fileName: source.fileName,
        manifest,
        active: false,
        error: `プラグイン id "${manifest.id}" が重複しています(先に読み込まれた方を使います)。`,
      });
      continue;
    }
    claimed.add(manifest.id);

    const entry: ActivePlugin = {
      fileName: source.fileName,
      manifest,
      cleanups: [],
      settingsHandlers: new Set(),
      objectUrl: imported.objectUrl,
    };
    // activate が途中で失敗しても登録済みのものを片付けられるよう、先に積んでおく。
    active.push(entry);

    try {
      const dispose = await imported.plugin.activate(createContext(entry));
      if (typeof dispose === "function") entry.cleanups.push(dispose);
      results.push({ fileName: source.fileName, manifest, active: true, error: null });
    } catch (error) {
      results.push({
        fileName: source.fileName,
        manifest,
        active: false,
        error: describeError(error),
      });
    }
  }

  setState({ loading: false, plugins: results, loadedAt: new Date().toISOString() });
  hostLog(
    "info",
    `プラグインを ${results.filter((item) => item.active).length}/${results.length} 件有効にしました。`,
  );
  void papi.pluginPublishLoaded(results).catch((error: unknown) => {
    hostLog("warn", `ロード結果を公開できませんでした: ${describeError(error)}`);
  });
}

/** すべてのプラグインを停止し、登録物を片付ける。 */
async function disposeAll(): Promise<void> {
  const entries = active.splice(0, active.length);
  for (const entry of entries) {
    // 後から登録したものから順に片付ける。
    for (const cleanup of entry.cleanups.reverse()) {
      try {
        await cleanup();
      } catch (error) {
        hostLog("warn", `${entry.manifest.id} の後始末に失敗しました: ${describeError(error)}`);
      }
    }
    entry.settingsHandlers.clear();
    URL.revokeObjectURL(entry.objectUrl);
  }
}

/** 処理を直列化して実行する。 */
function enqueue(task: () => Promise<void>): Promise<void> {
  queue = queue.then(task).catch((error: unknown) => {
    hostLog("error", `プラグイン処理で予期しない例外: ${describeError(error)}`);
  });
  return queue;
}

/** 全プラグインを dispose してから読み直す。 */
export function reloadPlugins(): Promise<void> {
  return enqueue(async () => {
    hostLog("info", "プラグインを再読み込みします。");
    await disposeAll();
    await load();
  });
}

/** 設定が変わったプラグインの `onChange` を呼ぶ。 */
async function notifySettingsChanged(pluginId: string): Promise<void> {
  const entry = active.find((item) => item.manifest.id === pluginId);
  if (!entry || entry.settingsHandlers.size === 0) return;
  let settings: PluginSettings;
  try {
    settings = mergeSettings(
      entry.manifest.settingsSchema,
      await papi.pluginGetSettings(pluginId),
    );
  } catch (error) {
    hostLog("warn", `${pluginId} の設定を読めませんでした: ${describeError(error)}`);
    return;
  }
  for (const handler of entry.settingsHandlers) {
    try {
      handler(settings);
    } catch (error) {
      hostLog("warn", `${pluginId} の onChange が失敗しました: ${describeError(error)}`);
    }
  }
}

/**
 * ホストを起動する。2 回目以降の呼び出しは無視する
 * (React の StrictMode で二重に呼ばれてもプラグインを二重起動しないため)。
 */
export function startHost(): void {
  if (started) return;
  started = true;

  // プラグインは import 無しでこれを呼ぶ。型は plugin-host/sdk.ts を参照のこと。
  globalThis.defineQuestloomPlugin = defineQuestloomPlugin;

  void papi.listenPluginReload(() => void reloadPlugins()).catch((error: unknown) => {
    hostLog("error", `再読み込みイベントを購読できませんでした: ${describeError(error)}`);
  });

  void papi
    .listenPluginSettingsChanged((pluginId) => void notifySettingsChanged(pluginId))
    .catch((error: unknown) => {
      hostLog("error", `設定変更イベントを購読できませんでした: ${describeError(error)}`);
    });

  void enqueue(load);
}
