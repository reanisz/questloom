/**
 * questloom TypeScript プラグイン SDK。
 *
 * **プラグイン作者向けのリファレンスはこのファイル**。ここに書かれている型が、
 * プラグインが受け取る `ctx` と、宣言する manifest のすべてである。
 *
 * ## プラグインの置き場所
 *
 * `%APPDATA%\questloom\plugins\*.ts`(または `*.js`)に置くだけで読み込まれる。
 * 正確なパスは設定画面の「プラグイン」節に表示される。
 *
 * ## 最小のプラグイン
 *
 * ```ts
 * export default defineQuestloomPlugin({
 *   manifest: { id: "hello", name: "ハローワールド", version: "0.1.0" },
 *   activate(ctx) {
 *     ctx.log("読み込まれました");
 *     return () => ctx.log("片付けました"); // 省略可(dispose)
 *   },
 * });
 * ```
 *
 * `defineQuestloomPlugin` は**グローバル関数**としてホストが用意する
 * (import は不要。型だけを見たい場合はこのファイルを読むこと)。
 * 中身は id の検証とその場での型付けだけで、副作用は無い。
 *
 * ## 制限(重要)
 *
 * - **1 ファイル 1 プラグイン。`import` は使えない。** ロード時に esbuild-wasm の
 *   `transform` でトランスパイルするだけで、バンドル(モジュール解決)は行わない。
 *   必要なコードはファイル内に書くこと。
 * - `ctx.fetch` は webview の `fetch` なので **CORS の制約を受ける**。
 *   `Access-Control-Allow-Origin` を返さない API は呼べない。
 * - プラグインは **questloom が起動している間だけ**動く。常駐サービスではない。
 * - 型注釈は付けてよいが**型検査はされない**(トランスパイルのみ)。
 */

import type {
  BoardColumnKey,
  NewResource,
  NewTask,
  ResourceKind,
  Task,
  TaskCard,
  TaskDetail,
  TaskId,
  TaskPatch,
  TaskResource,
  TaskUpdateEntry,
} from "../types";

/* ------------------------------------------------------------------ manifest */

/**
 * 設定項目の入力種別。
 *
 * `secret` は文字列だが扱いが違う。値は `settings` テーブルではなく **OS の資格情報
 * ストア**(Windows の資格情報マネージャー)に入り、設定画面には「設定済み / 未設定」
 * しか出ない(一度書いた値は画面から読み出せない)。プラグインから見た形は
 * 他の項目と同じで、`ctx.settings.get()` が値を混ぜて返す。
 */
export type PluginSettingType = "string" | "number" | "boolean" | "secret";

/**
 * 設定スキーマの項目 1 件。
 *
 * ここに宣言した項目が、設定画面の「プラグイン」節にフォームとして自動で生えてくる。
 * `ctx.settings.get()` は保存値に `default` をマージしたオブジェクトを返す。
 */
export interface PluginSettingField {
  /** 設定オブジェクト内のキー。 */
  key: string;
  /** 設定画面に出すラベル。 */
  label: string;
  /** 入力の種別。 */
  type: PluginSettingType;
  /** 既定値。未保存のときに `ctx.settings.get()` が返す値。 */
  default?: string | number | boolean | null;
  /** 入力欄の下に出す補足説明。 */
  hint?: string;
}

/** プラグインの宣言。ロード時にホストが読み、設定画面にも表示される。 */
export interface PluginManifest {
  /**
   * 一意な識別子。設定名前空間 (`plugin:<id>`)、KV の名前空間、
   * 作成したタスクの `origin` (`plugin:<id>`) に使われる。
   *
   * 英数字・`-`・`_` のみ。同じ id のプラグインが 2 つあると後から読んだ方が拒否される。
   */
  id: string;
  /** 表示名。 */
  name: string;
  /** バージョン文字列(表示のみ。ホストは解釈しない)。 */
  version?: string;
  /** 説明(設定画面に出る)。 */
  description?: string;
  /**
   * `ctx.fetch` を許可するホスト名。**完全一致**で判定する
   * (`api.github.com` を許しても `evil.api.github.com` は許されない)。
   * 空(既定)ならこのプラグインは一切 fetch できない。
   */
  fetchDomains?: string[];
  /** 設定スキーマ。設定画面のフォームがここから生成される。 */
  settingsSchema?: PluginSettingField[];
}

/* ------------------------------------------------------------------- context */

/** プラグインの設定値。キーは `settingsSchema` の `key`。 */
export type PluginSettings = Record<string, unknown>;

/** タスク操作 API。作成・追記の `origin` には自動で `plugin:<id>` が入る。 */
export interface PluginTaskApi {
  /**
   * タスクを作る。`origin` は指定しても `plugin:<id>` で上書きされる。
   *
   * 既定は「New 列の通常タスク」。インスタントタスクにするなら `isInstant: true`。
   */
  createTask(input: NewTask): Promise<Task>;
  /**
   * タイトル・詳細・締切を更新する(差分のみ指定する)。
   *
   * `origin` を持たない command なので、更新してもタスクの出所は変わらない。
   * **ユーザーが書いた文章を勝手に上書きしないこと**(空のときだけ埋める等の
   * 判断はプラグイン側の責任)。
   */
  updateTask(taskId: TaskId, patch: TaskPatch): Promise<Task>;
  /** タスク詳細(リソース・履歴・親子込み)を取る。 */
  getTask(taskId: TaskId): Promise<TaskDetail>;
  /**
   * 全タスクを 1 本の配列で返す(`get_board` を平坦化したもの)。
   *
   * ボードと同じ絞り込みがかかる。**完了済みは「今日完了した分」しか含まれない**
   * (前日以前の完了は画面でも別扱いで、プラグインからは見えない)。
   */
  listTasks(): Promise<TaskCard[]>;
  /** タスクを完了にする(冪等)。 */
  completeTask(taskId: TaskId): Promise<Task>;
  /** アップデート履歴を追記する。 */
  addTaskUpdate(taskId: TaskId, body: string): Promise<TaskUpdateEntry>;
  /** 関連リソースを追加する。 */
  addResource(taskId: TaskId, resource: NewResource): Promise<TaskResource>;
  /** 全タスクの関連リソースを一括で取る(URL の走査に使う)。 */
  listAllResources(): Promise<PluginTaskResource[]>;
  /** 列を指定してタスクを移動する。 */
  moveTask(taskId: TaskId, column: BoardColumnKey): Promise<Task>;
}

/** `listAllResources` が返す 1 件。 */
export interface PluginTaskResource {
  taskId: TaskId;
  kind: ResourceKind;
  value: string;
  label: string;
  isPrimary: boolean;
}

/** 設定へのアクセス。 */
export interface PluginSettingsApi {
  /**
   * `settingsSchema` の `default` をマージ済みの現在値を返す。
   *
   * `type: "secret"` の項目は OS の資格情報ストアから読んで混ぜてある。
   */
  get(): Promise<PluginSettings>;
  /** 設定画面から保存されたときに呼ばれる。戻り値で購読を解除できる。 */
  onChange(handler: (settings: PluginSettings) => void): () => void;
}

/** プラグイン専用の KV ストレージ(SQLite の `plugin_kv` テーブル)。 */
export interface PluginKvApi {
  /** 値を読む。未保存なら `undefined`。 */
  get<T = unknown>(key: string): Promise<T | undefined>;
  /** 値を書く。`undefined` / `null` を渡すとキーを消す。 */
  set(key: string, value: unknown): Promise<void>;
  /** 保存済みのキーを昇順で返す。 */
  keys(): Promise<string[]>;
}

/** ログレベル。 */
export type PluginLogLevel = "debug" | "info" | "warn" | "error";

/**
 * ログ出力。questloom 本体の tracing に転送されるので、
 * `npm run tauri dev` のコンソールに出る(`console.log` は非表示ウィンドウに埋もれる)。
 */
export interface PluginLogger {
  /** info レベルで出す。 */
  (...args: unknown[]): void;
  debug(...args: unknown[]): void;
  info(...args: unknown[]): void;
  warn(...args: unknown[]): void;
  error(...args: unknown[]): void;
}

/** `activate` に渡される実行コンテキスト。 */
export interface PluginContext {
  /** 自分の manifest(宣言したものがそのまま入る)。 */
  readonly manifest: PluginManifest;
  /** タスク操作。 */
  readonly tasks: PluginTaskApi;
  /** 設定。 */
  readonly settings: PluginSettingsApi;
  /** KV ストレージ。 */
  readonly kv: PluginKvApi;
  /**
   * `manifest.fetchDomains` に宣言したホストにのみ許される `fetch`。
   * 宣言外の URL を渡すと例外になる。CORS の制約はそのまま受ける。
   */
  fetch(url: string, init?: RequestInit): Promise<Response>;
  /**
   * 定期実行を登録する。登録直後に 1 回実行し、以後 `intervalMinutes` 毎に呼ぶ。
   * `fn` が投げた例外はログに出るだけで、以後の実行は止まらない。
   * 戻り値を呼ぶと解除できる(dispose 時はホストが自動で解除する)。
   */
  schedule(intervalMinutes: number, fn: () => void | Promise<void>): () => void;
  /**
   * タスクの変更(作成・更新・移動・完了・日付変化)を購読する。
   * ペイロードは渡さないので、必要なら `ctx.tasks` で取り直すこと。
   */
  onTaskEvent(handler: () => void): () => void;
  /** ログ出力。 */
  readonly log: PluginLogger;
}

/* -------------------------------------------------------------------- plugin */

/** `activate` の戻り値。関数を返すと dispose 時に呼ばれる。 */
export type PluginDispose = void | (() => void | Promise<void>);

/** プラグイン本体。`export default defineQuestloomPlugin({...})` で書く。 */
export interface QuestloomPlugin {
  /** 宣言。 */
  manifest: PluginManifest;
  /**
   * 起動時に 1 回呼ばれる。ここで `ctx.schedule` / `ctx.onTaskEvent` を登録する。
   * 例外を投げるとこのプラグインだけが無効になり、他のプラグインと本体には影響しない。
   */
  activate(ctx: PluginContext): PluginDispose | Promise<PluginDispose>;
}

/** manifest の id として許す文字。 */
const ID_PATTERN = /^[A-Za-z0-9_-]+$/;

/** 設定項目として許す種別。 */
const SETTING_TYPES: readonly PluginSettingType[] = ["string", "number", "boolean", "secret"];

/**
 * プラグイン定義を検証してそのまま返す。
 *
 * 型付けと最低限の検証(id の形式、manifest の必須項目、設定スキーマの妥当性)だけを行う。
 * 副作用は無い。id の重複はホスト側がロード時に弾く。
 *
 * @throws 検証に失敗した場合。ホストがログに出して、そのプラグインだけを無効にする。
 */
export function defineQuestloomPlugin(plugin: QuestloomPlugin): QuestloomPlugin {
  if (!plugin || typeof plugin !== "object") {
    throw new Error("defineQuestloomPlugin にはプラグイン定義オブジェクトを渡してください。");
  }
  const manifest = plugin.manifest;
  if (!manifest || typeof manifest !== "object") {
    throw new Error("manifest がありません。");
  }
  if (typeof manifest.id !== "string" || !ID_PATTERN.test(manifest.id)) {
    throw new Error("manifest.id は英数字・-・_ からなる空でない文字列にしてください。");
  }
  if (typeof manifest.name !== "string" || manifest.name.trim() === "") {
    throw new Error(`manifest.name を指定してください (id: ${manifest.id})。`);
  }
  if (typeof plugin.activate !== "function") {
    throw new Error(`activate を関数として定義してください (id: ${manifest.id})。`);
  }
  for (const domain of manifest.fetchDomains ?? []) {
    if (typeof domain !== "string" || domain.trim() === "") {
      throw new Error(`manifest.fetchDomains にはホスト名の文字列を並べてください (id: ${manifest.id})。`);
    }
  }
  const seen = new Set<string>();
  for (const field of manifest.settingsSchema ?? []) {
    if (!field || typeof field.key !== "string" || field.key.trim() === "") {
      throw new Error(`settingsSchema の key を指定してください (id: ${manifest.id})。`);
    }
    if (seen.has(field.key)) {
      throw new Error(`settingsSchema の key "${field.key}" が重複しています (id: ${manifest.id})。`);
    }
    seen.add(field.key);
    if (!SETTING_TYPES.includes(field.type)) {
      throw new Error(
        `settingsSchema "${field.key}" の type は ${SETTING_TYPES.join(" / ")} のいずれかにしてください。`,
      );
    }
  }
  return plugin;
}

/**
 * `settingsSchema` の既定値に保存値を重ねた設定値を作る。
 *
 * スキーマに無いキーは(将来のスキーマ変更で値を失わないよう)そのまま残す。
 */
export function mergeSettings(
  schema: readonly PluginSettingField[] | undefined,
  stored: PluginSettings | null | undefined,
): PluginSettings {
  const merged: PluginSettings = {};
  for (const field of schema ?? []) {
    merged[field.key] = field.default ?? defaultForType(field.type);
  }
  for (const [key, value] of Object.entries(stored ?? {})) {
    if (value !== undefined) merged[key] = value;
  }
  return merged;
}

/** `default` が省略されたときに使う、型ごとの空値。 */
export function defaultForType(type: PluginSettingType): string | number | boolean {
  switch (type) {
    case "number":
      return 0;
    case "boolean":
      return false;
    default:
      return "";
  }
}

declare global {
  /**
   * ホストがグローバルに用意する `defineQuestloomPlugin`。
   * プラグインファイルからは import 無しで呼べる。
   */
  // eslint-disable-next-line no-var
  var defineQuestloomPlugin: (plugin: QuestloomPlugin) => QuestloomPlugin;
}
