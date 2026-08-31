/**
 * questloom TypeScript プラグインのサンプル。
 *
 * 使い方: このファイルを設定画面の「プラグイン」節に表示されるフォルダ
 * (既定では `%APPDATA%\dev.reanisz.questloom\plugins\`)へコピーし、
 * 「プラグインを再読み込み」を押す。ログは questloom 本体の tracing に出る
 * (`npm run tauri dev` のコンソール)。
 *
 * API の型は `apps/desktop/src/plugin-host/sdk.ts` を参照。
 * `defineQuestloomPlugin` はホストがグローバルに用意するので import は不要。
 *
 * 制限: 1 ファイル完結(import 不可)、fetch は manifest 宣言ドメインのみ、
 * questloom の起動中だけ動く。
 */

export default defineQuestloomPlugin({
  manifest: {
    id: "hello",
    name: "ハローワールド",
    version: "0.1.0",
    description: "プラグイン基盤の動作確認用。ログを出すだけで、タスクは一切変更しない。",
    // fetch は使わないので許可ドメインは宣言しない(= ctx.fetch は常に拒否される)。
    fetchDomains: [],
    settingsSchema: [
      {
        key: "greeting",
        label: "あいさつ",
        type: "string",
        default: "こんにちは",
        hint: "起動時とポーリング時にログへ出す文字列。",
      },
      {
        key: "pollIntervalMinutes",
        label: "ポーリング間隔(分)",
        type: "number",
        default: 5,
        hint: "この間隔でログを出すだけ。何も取得しない。",
      },
      {
        key: "enabled",
        label: "ポーリングを行う",
        type: "boolean",
        default: true,
      },
    ],
  },

  async activate(ctx) {
    const settings = await ctx.settings.get();
    const greeting = String(settings.greeting ?? "こんにちは");
    const interval = Number(settings.pollIntervalMinutes ?? 5);

    ctx.log(`${greeting}! hello プラグインを読み込みました。`);

    // 起動回数を KV に記録する(KV の疎通確認を兼ねる)。
    const runs = ((await ctx.kv.get<number>("runs")) ?? 0) + 1;
    await ctx.kv.set("runs", runs);
    ctx.log.debug(`これまでの起動回数: ${runs}`);

    // タスクの一覧と関連リソースを 1 回だけ数えてみる(読み取りのみ)。
    const tasks = await ctx.tasks.listTasks();
    const resources = await ctx.tasks.listAllResources();
    ctx.log(`タスク ${tasks.length} 件 / 関連リソース ${resources.length} 件を確認しました。`);

    // 設定画面から保存されたら通知が来る。
    const offSettings = ctx.settings.onChange((next) => {
      ctx.log(`設定が変わりました: ${JSON.stringify(next)}`);
    });

    // タスクの変更通知(ペイロードは無いので、必要なら取り直す)。
    ctx.onTaskEvent(() => {
      ctx.log.debug("タスクが変更されました。");
    });

    if (settings.enabled !== false) {
      // 登録直後に 1 回、以後は指定間隔で呼ばれる。ここでは何もしない。
      ctx.schedule(interval, () => {
        ctx.log.debug(`${greeting}(ポーリング。実際の処理は無し)`);
      });
    }

    // 戻り値の関数は再読み込み・終了時に呼ばれる(省略可)。
    return () => {
      offSettings();
      ctx.log("hello プラグインを停止しました。");
    };
  },
});
