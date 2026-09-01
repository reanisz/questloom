/**
 * GUI e2e スモーク。実アプリ(アセット同梱の debug ビルド)を起動して、
 * 「ボードが出る → 作る → 開く → 消す → 復元する → 右クリックで消す → 復元する」を
 * 1 本で通す。
 *
 * 触るのは基本 **main ウィンドウだけ**。overlay / plugin-host も同じバンドルを読むので
 * ウィンドウハンドルは 3 つ返りうる。main の判別は独自タイトルバー
 * (`[data-testid="titlebar"]` は `App` にしか無い)で行う。
 * 例外は最後の内蔵ブラウザペインの spec で、そこだけは 4 枚目の webview
 * (外部ページ)へ切り替えて ACL と Esc の中継を確かめ、最後に main へ戻る。
 *
 * データディレクトリと MCP ポートの分離は `wdio.conf.ts` の責務。
 * ここでは「まっさらなプロファイルで起動している」前提に乗る。
 *
 * `describe` / `it` / `browser` / `$` / `$$` / `expect` は wdio が注入するグローバル。
 */

import { createServer as createHttpServer } from "node:http";

/** 作って消して戻すタスクのタイトル。 */
const TITLE = "e2e smoke";

/** ボードに必ずある列(見出しラベル付き)。 */
const COLUMNS = [
  { key: "new", label: "New" },
  { key: "today", label: "Today" },
  { key: "doing", label: "Doing" },
  { key: "done", label: "Done" },
] as const;

/** UI の往復(invoke → イベント → 再フェッチ)を待つ上限。 */
const SETTLE_TIMEOUT = 20_000;

/**
 * main ウィンドウのハンドルへ切り替える。既にそこなら何もしない。
 *
 * 起動直後はどのウィンドウもまだ描画前でありうるので、見つかるまで繰り返す
 * (1 周しただけだと「まだ空の DOM」を見て取り違える)。
 */
async function switchToMainWindow(): Promise<void> {
  let handles: string[] = [];
  await browser.waitUntil(
    async () => {
      handles = await browser.getWindowHandles();
      for (const handle of handles) {
        await browser.switchToWindow(handle);
        // 独自タイトルバーがあるのは main だけ(overlay は OverlayApp、
        // plugin-host は PluginHostApp を描画する)。
        const isMain = await browser.execute(
          () => document.querySelector('[data-testid="titlebar"]') !== null,
        );
        if (isMain) return true;
      }
      return false;
    },
    {
      timeout: 60_000,
      interval: 500,
      timeoutMsg: `main ウィンドウが見つかりません (handles=${handles.length})`,
    },
  );
}

/**
 * `selector` にマッチする要素のうち、テキストに `text` を含む最初のものを返す。
 * 無ければ null。
 *
 * wdio の `*=` 記法は属性セレクタと併用しづらいので、自前で拾って絞る。
 * DOM が入れ替わった直後は stale になりうるので、参照は都度取り直すこと。
 */
async function findByText(selector: string, text: string) {
  for (const element of await $$(selector)) {
    // 走査中に再描画が入ると stale element になる。次の待機ループで拾い直せばよいので飛ばす。
    const content = await element.getText().catch(() => null);
    if (content !== null && content.includes(text)) return element;
  }
  return null;
}

/** `selector` にマッチしテキストに `text` を含む要素が、現れる / 消えるまで待つ。 */
async function waitForText(
  selector: string,
  text: string,
  present: boolean,
  message: string,
): Promise<void> {
  await browser.waitUntil(async () => ((await findByText(selector, text)) !== null) === present, {
    timeout: SETTLE_TIMEOUT,
    interval: 250,
    timeoutMsg: message,
  });
}

/** New 列のカードのセレクタ。 */
const NEW_CARDS = '[data-testid="column-new"] [data-testid="task-card"]';

/* ------------------------------------------------------- Tauri command の直呼び */

/**
 * main ウィンドウの ACL で許されている command を webview 越しに呼ぶ。
 *
 * UI からは辿れない配線(シークレットの保存先など)を確かめるための逃げ道。
 * `__TAURI_INTERNALS__.invoke` は Tauri v2 が webview に生やすもの。
 */
async function invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  return (await browser.execute(
    (name: string, payload: Record<string, unknown>) =>
      (window as any).__TAURI_INTERNALS__.invoke(name, payload),
    command,
    args,
  )) as T;
}

/* ---------------------------------------------------- 内蔵 MCP の最小クライアント */

/**
 * このアプリが待ち受けている MCP のエンドポイント。
 *
 * ポートは `wdio.conf.ts` が空きポートを取って `QUESTLOOM_E2E_MCP_PORT` に書き戻し、
 * `QUESTLOOM_MCP_PORT` としてアプリへ渡している(本物の 39150 は使わない)。
 */
const MCP_URL = `http://127.0.0.1:${process.env.QUESTLOOM_E2E_MCP_PORT ?? ""}/mcp`;

/** `initialize` で受け取るセッション id。以降のリクエストに付ける。 */
let mcpSession: string | null = null;

/**
 * Streamable HTTP の応答を JSON として読む。
 *
 * 結果は SSE (`data: {...}`) で返りうるので、中身のある最初の `data:` 行を拾う。
 */
async function readMcpJson(response: Response): Promise<Record<string, unknown>> {
  const body = await response.text();
  const line = body
    .split("\n")
    .map((raw) => raw.trim())
    .filter((raw) => raw.startsWith("data:"))
    .map((raw) => raw.slice("data:".length).trim())
    .find((data) => data.length > 0);
  return JSON.parse(line ?? body.trim()) as Record<string, unknown>;
}

async function postMcp(body: unknown): Promise<Response> {
  const headers: Record<string, string> = {
    "content-type": "application/json",
    accept: "application/json, text/event-stream",
  };
  if (mcpSession) headers["mcp-session-id"] = mcpSession;
  return fetch(MCP_URL, { method: "POST", headers, body: JSON.stringify(body) });
}

/** MCP のハンドシェイク。1 度だけ行う。 */
async function initMcp(): Promise<void> {
  if (mcpSession) return;
  const response = await postMcp({
    jsonrpc: "2.0",
    id: 1,
    method: "initialize",
    params: {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "questloom-gui-e2e", version: "0.1.0" },
    },
  });
  if (!response.ok) throw new Error(`MCP の initialize に失敗しました (${response.status})`);
  mcpSession = response.headers.get("mcp-session-id");
  await readMcpJson(response);
  await postMcp({ jsonrpc: "2.0", method: "notifications/initialized" });
}

/** MCP のツールを呼び、結果テキスト(questloom はいつも JSON を返す)をパースして返す。 */
async function callMcp(tool: string, args: Record<string, unknown>): Promise<any> {
  await initMcp();
  const response = await postMcp({
    jsonrpc: "2.0",
    id: 2,
    method: "tools/call",
    params: { name: tool, arguments: args },
  });
  const payload = (await readMcpJson(response)) as any;
  if (payload.result?.isError === true) {
    throw new Error(`${tool} がエラーを返しました: ${JSON.stringify(payload)}`);
  }
  const text = payload.result?.content?.[0]?.text;
  if (typeof text !== "string") throw new Error(`${tool} の応答が読めません: ${JSON.stringify(payload)}`);
  return JSON.parse(text);
}

describe("questloom GUI スモーク", () => {
  before(async () => {
    await switchToMainWindow();
    // 初回描画は get_board の往復待ちなので、ボードが出るまで待ってから始める。
    await $('[data-testid="column-new"]').waitForExist({ timeout: 60_000 });
  });

  it("起動するとタイトルバーとボードの主要な列が描画される", async () => {
    await expect($('[data-testid="titlebar"]')).toBeExisting();

    for (const { key, label } of COLUMNS) {
      const column = $(`[data-testid="column-${key}"]`);
      await expect(column).toBeExisting();
      // 見出しは CSS の `text-transform` で大文字に描かれる(getText は描画後の文字を返す)。
      await expect(column.$("h2")).toHaveText(label, { ignoreCase: true });
    }
  });

  it("New 列のクイック追加でタスクを作るとカードが現れる", async () => {
    // クイック追加は常設の入力欄ではなくテキストボタン。押して入力欄に変える。
    const open = $('[data-testid="quick-add-open-new"]');
    await open.waitForDisplayed();
    await open.click();

    const input = $('[data-testid="quick-add-new"]');
    await input.waitForDisplayed();
    await input.setValue(TITLE);
    await browser.keys("Enter");

    await waitForText(NEW_CARDS, TITLE, true, `New 列に「${TITLE}」が出ません`);
  });

  it("カードをクリックすると詳細ドロワーが開き、削除するとカードが消える", async () => {
    const card = await findByText(NEW_CARDS, TITLE);
    if (!card) throw new Error(`「${TITLE}」のカードが見つかりません`);
    await card.click();

    const drawer = $('[data-testid="task-drawer"]');
    await drawer.waitForDisplayed();
    // ドロワーの中身は get_task の往復待ち。削除ボタンが出たら読み込み済み。
    const deleteButton = $('[data-testid="drawer-delete"]');
    await deleteButton.waitForDisplayed();
    await deleteButton.click();

    // 確認ダイアログ(ModalShell の aria-label で引く)。
    await $('div[role="dialog"][aria-label="タスクを削除"]').waitForDisplayed();
    await $('[data-testid="confirm-delete"]').click();

    await drawer.waitForExist({ reverse: true, timeout: SETTLE_TIMEOUT });
    await waitForText(NEW_CARDS, TITLE, false, `New 列から「${TITLE}」が消えません`);
  });

  it("「削除済み」から復元すると New 列に戻る", async () => {
    await $('[data-testid="open-deleted"]').click();

    const dialog = $('div[role="dialog"][aria-label="削除済みのタスク"]');
    await dialog.waitForDisplayed();

    const rows = '[data-testid="deleted-row"]';
    await waitForText(rows, TITLE, true, `削除済み一覧に「${TITLE}」が出ません`);

    const row = await findByText(rows, TITLE);
    if (!row) throw new Error(`削除済みの「${TITLE}」が見つかりません`);
    await row.$('[data-testid="restore-task"]').click();

    // 一覧は tasks-changed で取り直されるので、復元した行は消える。
    await waitForText(rows, TITLE, false, `削除済み一覧から「${TITLE}」が消えません`);

    // ダイアログを閉じてボードを確認する。
    await browser.keys("Escape");
    await dialog.waitForExist({ reverse: true, timeout: SETTLE_TIMEOUT });

    await waitForText(NEW_CARDS, TITLE, true, `New 列に「${TITLE}」が戻りません`);
  });

  it("カードの右クリックメニューから削除し、「削除済み」から復元できる", async () => {
    const card = await findByText(NEW_CARDS, TITLE);
    if (!card) throw new Error(`「${TITLE}」のカードが見つかりません`);
    // 右クリック。標準メニューはフロント側が preventDefault で止めている。
    await card.click({ button: "right" });

    await $('[data-testid="task-context-menu"]').waitForDisplayed();
    await $('[data-testid="context-delete"]').click();

    // 確認ダイアログはドロワーの削除と同じもの (DeleteConfirmDialog)。
    await $('div[role="dialog"][aria-label="タスクを削除"]').waitForDisplayed();
    await $('[data-testid="confirm-delete"]').click();

    await waitForText(NEW_CARDS, TITLE, false, `New 列から「${TITLE}」が消えません`);

    // 後片付けを兼ねて復元し、ボードを元の状態へ戻す。
    await $('[data-testid="open-deleted"]').click();
    const dialog = $('div[role="dialog"][aria-label="削除済みのタスク"]');
    await dialog.waitForDisplayed();

    const rows = '[data-testid="deleted-row"]';
    await waitForText(rows, TITLE, true, `削除済み一覧に「${TITLE}」が出ません`);
    const row = await findByText(rows, TITLE);
    if (!row) throw new Error(`削除済みの「${TITLE}」が見つかりません`);
    await row.$('[data-testid="restore-task"]').click();

    await browser.keys("Escape");
    await dialog.waitForExist({ reverse: true, timeout: SETTLE_TIMEOUT });
    await waitForText(NEW_CARDS, TITLE, true, `New 列に「${TITLE}」が戻りません`);
  });

  /**
   * Watching(監視中)の往復。
   *
   * 「右クリック → 移動 → 監視中」で先送りレールへ引っ込め、MCP からの履歴追記
   * (= ユーザー以外の変化)で New に戻ってくることを見る。起床はサービス層の規則だが、
   * ここでは**実アプリの UI が実際に New へ描き直す**ところまで通す。
   */
  it("監視中へ移すとレールに入り、MCP の履歴追記で New へ戻る", async () => {
    const card = await findByText(NEW_CARDS, TITLE);
    if (!card) throw new Error(`「${TITLE}」のカードが見つかりません`);
    await card.click({ button: "right" });

    await $('[data-testid="task-context-menu"]').waitForDisplayed();
    await $('[data-testid="context-move"]').click();
    await $('[data-testid="context-move-watching"]').click();

    // New から消え、通常表示のレールの「監視中」ボックスに数えられる。
    await waitForText(NEW_CARDS, TITLE, false, `New 列から「${TITLE}」が消えません`);
    const box = $('[data-testid="defer-box-watching"]');
    await box.waitForDisplayed();
    await browser.waitUntil(async () => (await box.getText()).includes("1"), {
      timeout: SETTLE_TIMEOUT,
      interval: 250,
      timeoutMsg: "監視中ボックスの件数が 1 になりません",
    });

    // MCP から見ても watching にいる。
    const listed = await callMcp("list_tasks", { column: "watching" });
    const task = listed.tasks.find((item: { title: string }) => item.title === TITLE);
    if (!task) throw new Error(`MCP の watching 列に「${TITLE}」がいません`);

    // ユーザー以外の origin の追記で起床する。
    await callMcp("add_task_update", { task_id: task.id, body: "外部で変化がありました" });

    // ボードは tasks-changed を受けて描き直され、カードが New に戻る。
    await waitForText(NEW_CARDS, TITLE, true, `起床した「${TITLE}」が New 列に現れません`);
    await browser.waitUntil(async () => (await box.getText()).includes("0"), {
      timeout: SETTLE_TIMEOUT,
      interval: 250,
      timeoutMsg: "監視中ボックスの件数が 0 に戻りません",
    });
  });

  /**
   * タスク内チェックリストの往復。
   *
   * 「ドロワーで 2 件足す → 1 件チェックする → 閉じるとカードに `☑ 1/2` が出る」まで。
   * command は main ウィンドウにしか配っていないので、ACL の配線が抜けていれば
   * ここで invoke が拒否されて落ちる。
   */
  it("ドロワーでチェックリストを足してチェックすると、カードにバッジが出る", async () => {
    const card = await findByText(NEW_CARDS, TITLE);
    if (!card) throw new Error(`「${TITLE}」のカードが見つかりません`);
    await card.click();

    const drawer = $('[data-testid="task-drawer"]');
    await drawer.waitForDisplayed();
    // ドロワーの中身は get_task の往復待ち。追加欄が出たら読み込み済み。
    const input = $('[data-testid="checklist-add"]');
    await input.waitForDisplayed();

    // Enter で追加。入力欄は残るので、続けてもう 1 件足せる。
    await input.setValue("住所変更");
    await browser.keys("Enter");
    await browser.waitUntil(async () => (await $$('[data-testid="checklist-item"]')).length === 1, {
      timeout: SETTLE_TIMEOUT,
      interval: 250,
      timeoutMsg: "チェックリストに 1 件目が出ません",
    });

    await $('[data-testid="checklist-add"]').setValue("電気の停止");
    await browser.keys("Enter");
    await browser.waitUntil(async () => (await $$('[data-testid="checklist-item"]')).length === 2, {
      timeout: SETTLE_TIMEOUT,
      interval: 250,
      timeoutMsg: "チェックリストに 2 件目が出ません",
    });

    const progress = $('[data-testid="checklist-progress"]');
    await progress.waitForDisplayed();
    await browser.waitUntil(async () => (await progress.getText()) === "0/2", {
      timeout: SETTLE_TIMEOUT,
      interval: 250,
      timeoutMsg: "進捗が 0/2 になりません",
    });

    // 1 件目をチェックすると進捗が進む。タスク自体は完了しない(New のまま)。
    const toggles = await $$('[data-testid="checklist-toggle"]');
    await toggles[0].click();
    await browser.waitUntil(async () => (await progress.getText()) === "1/2", {
      timeout: SETTLE_TIMEOUT,
      interval: 250,
      timeoutMsg: "進捗が 1/2 になりません",
    });

    // ドロワーを閉じて、ボードのカードにバッジが出ていることを見る。
    await browser.keys("Escape");
    await drawer.waitForExist({ reverse: true, timeout: SETTLE_TIMEOUT });

    await browser.waitUntil(
      async () => {
        const target = await findByText(NEW_CARDS, TITLE);
        if (!target) return false;
        const badge = await target.$('[data-testid="checklist-badge"]');
        if (!(await badge.isExisting())) return false;
        return (await badge.getText()).includes("1/2");
      },
      {
        timeout: SETTLE_TIMEOUT,
        interval: 250,
        timeoutMsg: `「${TITLE}」のカードに ☑ 1/2 のバッジが出ません`,
      },
    );

    // チェックを埋めてもタスクは完了しない(Done へ動かない)。
    const listed = await callMcp("list_tasks", { column: "new" });
    const task = listed.tasks.find((item: { title: string }) => item.title === TITLE);
    if (!task) throw new Error(`チェック後も New にいるはずの「${TITLE}」がいません`);
  });

  /**
   * 展開表示は 10 列(Icebox と監視中を含む)。既定のウィンドウ幅 1280px で横スクロールを
   * 出さないことを実寸で見る(`--column-min-expanded` の見積もりが崩れたら落ちる)。
   */
  it("全列を展開しても横スクロールが出ない", async () => {
    const toggle = $('[data-testid="toggle-expanded"]');
    await toggle.click();
    await $('[data-testid="column-watching"]').waitForDisplayed();

    for (const key of [
      "icebox",
      "new",
      "tomorrow",
      "thisWeek",
      "nextWeek",
      "future",
      "watching",
      "done",
    ]) {
      await expect($(`[data-testid="column-${key}"]`)).toBeExisting();
    }

    const overflow = await browser.execute(() => {
      const board = document.querySelector(".board");
      if (!board) return null;
      const first = board.querySelector("section.column");
      return {
        scrollWidth: board.scrollWidth,
        clientWidth: board.clientWidth,
        columns: board.querySelectorAll("section.column").length,
        firstColumn: first?.getAttribute("data-testid") ?? null,
      };
    });
    if (!overflow) throw new Error("ボードが見つかりません");
    expect(overflow.columns).toBe(10);
    // Icebox は一番左。
    expect(overflow.firstColumn).toBe("column-icebox");
    expect(overflow.scrollWidth).toBeLessThanOrEqual(overflow.clientWidth);

    // 通常表示へ戻して次回以降に影響を残さない(表示モードは localStorage に残る)。
    await toggle.click();
    await $('[data-testid="defer-box-watching"]').waitForDisplayed();
    // レールにも Icebox のボックスが出る。
    await expect($('[data-testid="defer-box-icebox"]')).toBeExisting();
  });

  /**
   * プラグインのシークレットが資格情報ストアへ入り、DB にも UI にも値が残らない。
   *
   * `wdio.conf.ts` が一時プロファイルへ置いた `e2esecret` プラグインを使い、
   *
   * - 書き込み → 「設定済み」になり、値を返す経路が無いこと、
   * - main ウィンドウから `plugin_secret_get`(値の読み出し)が ACL で拒まれること、
   * - `plugin_set_settings` の JSON にシークレットが混ざらないこと、
   * - **旧バージョンが平文で残した値が、読み直しで資格情報ストアへ移り
   *   設定 JSON から消えること**(`plugin_host::migrate_plugin_secrets` を
   *   実プロセス・実 webview・本物の資格情報マネージャーで通す)
   *
   * を見る。資格情報エントリはユーザー単位でグローバルなので、service 名は
   * `wdio.conf.ts` が実行ごとに分けたうえで、このテストの最後に必ず消す。
   */
  it("プラグインのシークレットは資格情報ストアに入り、値は読み出せない", async () => {
    const PLUGIN = "e2esecret";
    const PAT = "gui-e2e-pat";

    // プラグインがロードされていること(= plugin-host が動いていること)を先に確かめる。
    await browser.waitUntil(
      async () => {
        const loaded = await invoke<{ manifest: { id: string } | null }[]>("plugin_list_loaded");
        return loaded.some((plugin) => plugin.manifest?.id === PLUGIN);
      },
      { timeout: SETTLE_TIMEOUT, interval: 250, timeoutMsg: `${PLUGIN} がロードされません` },
    );

    // 未設定から始める(前回の残骸があっても消しておく)。
    await invoke("plugin_secret_set", { pluginId: PLUGIN, key: "pat", value: null });
    expect(await invoke<boolean>("plugin_secret_status", { pluginId: PLUGIN, key: "pat" })).toBe(
      false,
    );

    // 書き込むと「設定済み」になる。返るのは状態だけで、値は返らない。
    expect(
      await invoke<boolean>("plugin_secret_set", { pluginId: PLUGIN, key: "pat", value: PAT }),
    ).toBe(true);
    expect(await invoke<boolean>("plugin_secret_status", { pluginId: PLUGIN, key: "pat" })).toBe(
      true,
    );

    // 値を読む command は plugin-host 専用。main からは ACL に拒まれる。
    await expect(invoke("plugin_secret_get", { pluginId: PLUGIN, key: "pat" })).rejects.toThrow();

    // プラグイン設定の JSON にはシークレットが入らない。
    await invoke("plugin_set_settings", { pluginId: PLUGIN, value: { note: "残るべき値" } });
    const stored = await invoke<Record<string, unknown>>("plugin_get_settings", {
      pluginId: PLUGIN,
    });
    expect(stored.pat).toBeUndefined();
    expect(stored.note).toBe("残るべき値");

    // 名前空間を抜け出すキーは境界で弾く。
    await expect(
      invoke("plugin_secret_status", { pluginId: "../core", key: "mcp-token" }),
    ).rejects.toThrow();

    // ---- 旧バージョンからの移送 ----

    // 「平文が settings に残っている」状態を作り直す。
    await invoke("plugin_secret_set", { pluginId: PLUGIN, key: "pat", value: null });
    await invoke("plugin_set_settings", {
      pluginId: PLUGIN,
      value: { pat: PAT, note: "残るべき値" },
    });
    expect(await invoke<boolean>("plugin_secret_status", { pluginId: PLUGIN, key: "pat" })).toBe(
      false,
    );

    // 読み直させると、公開された manifest を見て移送が走る。
    await invoke("plugin:event|emit", { event: "questloom://plugins-reload", payload: null });
    await browser.waitUntil(
      () => invoke<boolean>("plugin_secret_status", { pluginId: PLUGIN, key: "pat" }),
      { timeout: SETTLE_TIMEOUT, interval: 250, timeoutMsg: "PAT が資格情報ストアへ移りません" },
    );

    // 平文は設定から消え、他の項目は残る。
    const migrated = await invoke<Record<string, unknown>>("plugin_get_settings", {
      pluginId: PLUGIN,
    });
    expect(migrated.pat).toBeUndefined();
    expect(migrated.note).toBe("残るべき値");

    // 後始末: このテストが作った資格情報エントリを消す。
    expect(
      await invoke<boolean>("plugin_secret_set", { pluginId: PLUGIN, key: "pat", value: null }),
    ).toBe(false);
  });

  /**
   * MCP トークンも同じ扱い(設定済みかどうかだけが見える)。
   *
   * トークンを設定すると MCP サーバーが張り直され、`get_runtime_status` の
   * `mcpTokenRequired` が真になる。最後に必ず解除してエントリを残さない。
   */
  it("MCP トークンは資格情報ストアに入り、稼働状態にだけ現れる", async () => {
    expect(await invoke<boolean>("get_mcp_token_status")).toBe(false);

    expect(await invoke<boolean>("set_mcp_token", { token: "  gui-e2e-token  " })).toBe(true);
    expect(await invoke<boolean>("get_mcp_token_status")).toBe(true);

    // コア設定にはトークンが載らない。
    const settings = await invoke<Record<string, unknown>>("get_settings");
    expect(settings.mcpToken).toBeUndefined();

    // サーバーが張り直され、認証ありになる。
    await browser.waitUntil(
      async () => (await invoke<{ mcpTokenRequired: boolean }>("get_runtime_status")).mcpTokenRequired,
      {
        timeout: SETTLE_TIMEOUT,
        interval: 250,
        timeoutMsg: "MCP サーバーがトークン認証ありで張り直されません",
      },
    );

    // 後始末: 解除して元の「認証なし」に戻す(MCP を使う他のテストを壊さないため)。
    expect(await invoke<boolean>("set_mcp_token", { token: null })).toBe(false);
    await browser.waitUntil(
      async () =>
        !(await invoke<{ mcpTokenRequired: boolean }>("get_runtime_status")).mcpTokenRequired,
      {
        timeout: SETTLE_TIMEOUT,
        interval: 250,
        timeoutMsg: "MCP サーバーが認証なしへ戻りません",
      },
    );
  });

  /**
   * 内蔵ブラウザペイン(4 枚目の webview・外部ページ)の ACL と Esc の中継。
   *
   * ここだけは main 以外のハンドルへ切り替える。ページは実行ごとに立てる
   * ローカルの HTTP サーバー(`http://127.0.0.1:<空きポート>`)で、外部ネットワークには出ない。
   * questloom から見れば `*.localhost` ではない普通の http なので、
   * Tauri からは**リモート生成元** (`Origin::Remote`) に見える = 本番と同じ条件になる。
   *
   * 見るのは 3 つ。
   *
   * 1. **`browser_pane_escape` 以外の command は ACL に拒まれる**
   *    (`get_board` / `get_settings` / `browser_pane_open` / `delete_task`)。
   * 2. `browser_pane_escape` は通り、Esc は**最前面のレイヤー**へ配られる
   *    (モーダルを開いておくと、閉じるのはモーダルだけ)。
   * 3. ページの中で押した Esc が main へ中継され、ドロワーとペイン(紐づき)が閉じる。
   */
  it("ブラウザペインは Esc だけを中継でき、他の command は ACL に拒まれる", async () => {
    const PANE_TITLE = "e2e browser pane";
    const server = createHttpServer((_request, response) => {
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      response.end("<!doctype html><title>questloom e2e</title><h1>pane</h1>");
    });
    await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok));
    const address = server.address();
    if (address === null || typeof address === "string") throw new Error("ポートを取得できません");
    const pageUrl = `http://127.0.0.1:${address.port}/`;

    try {
      // internalAuto: 主リソースが URL のタスクを開くと、ドロワーと一緒にペインが出る。
      const settings = await invoke<Record<string, unknown>>("get_settings");
      await invoke("set_settings", { settings: { ...settings, urlOpenMode: "internalAuto" } });
      // ボード側の `urlOpenMode` は「設定画面から戻ったとき」に読み直される
      // (設定変更の通知イベントは無い)。開いて閉じて反映させる。
      await $('button[aria-label="設定"]').click();
      await $(".settings-header").waitForDisplayed();
      await browser.keys("Escape");
      await $('[data-testid="column-new"]').waitForDisplayed();

      // MCP は使わない。直前の spec がトークンを付け外ししてサーバーを張り直しており、
      // 使い回しているセッション id が失効している。
      const created = await invoke<{ id: string }>("create_task", {
        input: {
          title: PANE_TITLE,
          resources: [{ kind: "url", value: pageUrl, label: "e2e", isPrimary: true }],
        },
      });
      await waitForText(NEW_CARDS, PANE_TITLE, true, `New 列に「${PANE_TITLE}」が出ません`);

      const card = await findByText(NEW_CARDS, PANE_TITLE);
      if (!card) throw new Error(`「${PANE_TITLE}」のカードが見つかりません`);
      await card.click();
      await $('[data-testid="task-drawer"]').waitForDisplayed();
      // ペインの枠(React 側)が出たら、子 webview の生成要求も飛んでいる。
      await $('[data-testid="browser-pane"]').waitForExist({ timeout: SETTLE_TIMEOUT });

      // ---- ペインの webview へ切り替える ----
      const mainHandle = await browser.getWindowHandle();
      let paneHandle: string | null = null;
      await browser.waitUntil(
        async () => {
          for (const handle of await browser.getWindowHandles()) {
            if (handle === mainHandle) continue;
            await browser.switchToWindow(handle);
            const href = await browser.execute(() => window.location.href).catch(() => "");
            if (typeof href === "string" && href.startsWith(pageUrl)) {
              paneHandle = handle;
              return true;
            }
          }
          return false;
        },
        { timeout: SETTLE_TIMEOUT, interval: 500, timeoutMsg: "ペインの webview が見つかりません" },
      );
      if (paneHandle === null) throw new Error("ペインの webview が見つかりません");

      /**
       * ペインの中から command を呼び、結果を `OK:<返り値>` / `ERR:<理由>` の文字列で返す。
       *
       * 例外を投げずに文字列へ畳むのは、`browser.execute` が返す値だけで
       * 「通った / 拒まれた」を区別したいため。
       */
      const tryInvoke = (command: string) =>
        browser.execute(
          (name: string) =>
            (window as any).__TAURI_INTERNALS__.invoke(name).then(
              (value: unknown) => `OK:${JSON.stringify(value)?.slice(0, 80)}`,
              (error: unknown) => `ERR:${String((error as { message?: string })?.message ?? error)}`,
            ),
          command,
        );

      // 1. Esc の中継以外は ACL に拒まれる。
      //    (デバッグビルドの拒否理由は "<command> not allowed on window ..., webview ...")
      for (const command of ["get_board", "get_settings", "browser_pane_open", "delete_task"]) {
        const result = await tryInvoke(command);
        if (typeof result !== "string" || !result.includes("not allowed")) {
          throw new Error(`${command} はペインから拒否されるべき (返り値: ${String(result)})`);
        }
      }

      // 2. `browser_pane_escape` は通る。
      //    ペインを閉じずに確かめるため、main 側でモーダルを開いておく
      //    (Esc は最前面のレイヤー = モーダルへ配られ、ドロワーとペインは残る)。
      await browser.switchToWindow(mainHandle);
      // ドロワーがヘッダに重なっているので、hit-test を通さず JS でクリックする。
      await browser.execute(() =>
        document.querySelector<HTMLButtonElement>('[data-testid="open-deleted"]')?.click(),
      );
      const deleted = $('div[role="dialog"][aria-label="削除済みのタスク"]');
      await deleted.waitForDisplayed();

      await browser.switchToWindow(paneHandle);
      const escaped = await tryInvoke("browser_pane_escape");
      if (typeof escaped !== "string" || !escaped.startsWith("OK")) {
        throw new Error(`browser_pane_escape は通るべき (返り値: ${String(escaped)})`);
      }

      await browser.switchToWindow(mainHandle);
      await deleted.waitForExist({ reverse: true, timeout: SETTLE_TIMEOUT });
      // モーダルだけが閉じ、ドロワーとペインは残っている。
      await expect($('[data-testid="task-drawer"]')).toBeExisting();
      await expect($('[data-testid="browser-pane"]')).toBeExisting();

      // 3. ページの中で押した Esc が main へ中継され、ドロワーとペイン(紐づき)が閉じる。
      //    2 で 1 発使っているので、Rust 側の間引き (200ms) が明けるのを待つ。
      await browser.pause(500);
      await browser.switchToWindow(paneHandle);
      await browser.execute(() => {
        window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      });

      await browser.switchToWindow(mainHandle);
      await $('[data-testid="task-drawer"]').waitForExist({
        reverse: true,
        timeout: SETTLE_TIMEOUT,
      });
      await $('[data-testid="browser-pane"]').waitForExist({
        reverse: true,
        timeout: SETTLE_TIMEOUT,
      });

      // 後始末: 設定を戻し、作ったタスクを消す。
      await invoke("set_settings", { settings });
      await invoke("delete_task", { taskId: created.id });
      await waitForText(NEW_CARDS, PANE_TITLE, false, `New 列から「${PANE_TITLE}」が消えません`);
    } finally {
      await new Promise<void>((ok) => server.close(() => ok()));
      await switchToMainWindow();
    }
  });
});
