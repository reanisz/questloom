/**
 * GUI e2e スモーク。実アプリ(アセット同梱の debug ビルド)を起動して、
 * 「ボードが出る → 作る → 開く → 消す → 復元する」を 1 本で通す。
 *
 * 触るのは **main ウィンドウだけ**。overlay / plugin-host も同じバンドルを読むので
 * ウィンドウハンドルは 3 つ返りうる。main の判別は独自タイトルバー
 * (`[data-testid="titlebar"]` は `App` にしか無い)で行う。
 *
 * データディレクトリと MCP ポートの分離は `wdio.conf.ts` の責務。
 * ここでは「まっさらなプロファイルで起動している」前提に乗る。
 *
 * `describe` / `it` / `browser` / `$` / `$$` / `expect` は wdio が注入するグローバル。
 */

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

/** main ウィンドウのハンドルへ切り替える。既にそこなら何もしない。 */
async function switchToMainWindow(): Promise<void> {
  const handles = await browser.getWindowHandles();
  for (const handle of handles) {
    await browser.switchToWindow(handle);
    // 独自タイトルバーがあるのは main だけ(overlay は OverlayApp、
    // plugin-host は PluginHostApp を描画する)。
    const isMain = await browser.execute(
      () => document.querySelector('[data-testid="titlebar"]') !== null,
    );
    if (isMain) return;
  }
  throw new Error(`main ウィンドウが見つかりません (handles=${handles.length})`);
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
});
