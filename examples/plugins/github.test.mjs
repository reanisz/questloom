/**
 * `github.ts` の判定ロジック(純関数)の検証。
 *
 * 実行:
 *
 * ```powershell
 * node --test examples/plugins/github.test.mjs
 * ```
 *
 * Node 22.18+ / 24 は `.ts` を型注釈の除去だけで読めるので、追加のビルドは要らない。
 * プラグイン本体は読み込み時に `defineQuestloomPlugin` を呼ぶため、
 * import の前にグローバルへスタブを置いてから動的 import する。
 *
 * 型そのものの検査は tsc に任せる(グローバル宣言を効かせるため sdk.ts も渡す):
 *
 * ```powershell
 * cd apps/desktop
 * ./node_modules/.bin/tsc --noEmit --strict --noUnusedLocals --noUnusedParameters `
 *   --target ES2020 --module ESNext --moduleResolution bundler `
 *   --lib ES2020,DOM,DOM.Iterable --skipLibCheck `
 *   ../../examples/plugins/github.ts src/plugin-host/sdk.ts
 * ```
 */

import assert from "node:assert/strict";
import { describe, it } from "node:test";

globalThis.defineQuestloomPlugin = (plugin) => plugin;

const {
  buildInboxDescription,
  buildInboxTitle,
  buildMentionQuery,
  buildPrDescription,
  buildReasons,
  buildReviewRequestQuery,
  collectDescriptionTargets,
  collectPullRequestTargets,
  collectTrackedKeys,
  decideInboxNotification,
  decideNoticeAction,
  mergeCi,
  mergeInboxItems,
  nextInboxState,
  parseIssueOrPullUrl,
  parsePullRequestUrl,
  planInboxPrune,
  planNotification,
  pullRequestApiPath,
  searchApiPath,
  selectInboxCandidates,
  selectNewComments,
  shouldFillDescription,
  shouldNotifyCiFailure,
  summarizeCheckRuns,
  summarizeCombinedStatus,
  toInboxItems,
  truncateBody,
} = await import("./github.ts");

describe("parsePullRequestUrl", () => {
  it("PR の URL を owner / repo / 番号に分解する", () => {
    assert.deepEqual(parsePullRequestUrl("https://github.com/rust-lang/rust/pull/1234"), {
      owner: "rust-lang",
      repo: "rust",
      number: 1234,
      key: "rust-lang/rust#1234",
      url: "https://github.com/rust-lang/rust/pull/1234",
    });
  });

  it("末尾のパス・クエリ・フラグメントが付いていても拾う", () => {
    for (const suffix of ["/files", "/commits/abc", "#issuecomment-42", "?w=1"]) {
      const ref = parsePullRequestUrl(`https://github.com/o/r/pull/7${suffix}`);
      assert.equal(ref?.key, "o/r#7", suffix);
      // 主リソースには正規化した URL を使う。
      assert.equal(ref?.url, "https://github.com/o/r/pull/7");
    }
  });

  it("www 付き・http でも拾い、キーは小文字にそろえる", () => {
    assert.equal(parsePullRequestUrl("http://www.github.com/Foo/Bar-Baz/pull/9")?.key, "foo/bar-baz#9");
  });

  it("PR 以外・別ホストは拾わない", () => {
    const rejected = [
      "https://github.com/o/r/issues/1",
      "https://github.com/o/r/pull/",
      "https://github.com/o/r/pull/abc",
      "https://github.com/o/r",
      "https://gitlab.com/o/r/pull/1",
      "https://evil.github.com/o/r/pull/1",
      "https://github.com.evil.test/o/r/pull/1",
      "C:/tmp/notes.txt",
      "",
    ];
    for (const url of rejected) {
      assert.equal(parsePullRequestUrl(url), null, url);
    }
  });
});

describe("collectPullRequestTargets", () => {
  const origin = "plugin:github";

  it("未完了タスクの PR URL だけを集め、同じ PR はまとめる", () => {
    const tasks = [
      { id: "t1", status: "todo", origin: "user" },
      { id: "t2", status: "new", origin: "user" },
      { id: "t3", status: "done", origin: "user" },
    ];
    const resources = [
      { taskId: "t1", kind: "url", value: "https://github.com/o/r/pull/1" },
      { taskId: "t2", kind: "url", value: "https://github.com/o/r/pull/1/files" },
      { taskId: "t2", kind: "url", value: "https://github.com/o/r/pull/2" },
      // Done タスクのリソースは対象外。
      { taskId: "t3", kind: "url", value: "https://github.com/o/r/pull/3" },
      // URL 以外・PR 以外は無視。
      { taskId: "t1", kind: "file", value: "https://github.com/o/r/pull/4" },
      { taskId: "t1", kind: "url", value: "https://example.com/" },
    ];
    const targets = collectPullRequestTargets(tasks, resources, origin);
    assert.deepEqual(
      targets.map((target) => [target.ref.key, target.taskIds]),
      [
        ["o/r#1", ["t1", "t2"]],
        ["o/r#2", ["t2"]],
      ],
    );
  });

  it("自分が作った通知タスクは監視対象にしない(自己増殖の防止)", () => {
    const tasks = [{ id: "n1", status: "new", origin }];
    const resources = [{ taskId: "n1", kind: "url", value: "https://github.com/o/r/pull/1" }];
    assert.deepEqual(collectPullRequestTargets(tasks, resources, origin), []);
  });

  it("ボードに無いタスクのリソースは無視する", () => {
    const resources = [{ taskId: "ghost", kind: "url", value: "https://github.com/o/r/pull/1" }];
    assert.deepEqual(collectPullRequestTargets([], resources, origin), []);
  });

  it("監視中のタスクは未完了なので監視対象に残る(起床させる相手になる)", () => {
    const tasks = [{ id: "w1", status: "watching", origin: "user" }];
    const resources = [{ taskId: "w1", kind: "url", value: "https://github.com/o/r/pull/1" }];
    assert.deepEqual(
      collectPullRequestTargets(tasks, resources, origin).map((t) => t.taskIds),
      [["w1"]],
    );
  });
});

describe("selectNewComments", () => {
  const comments = [
    { id: 1, createdAt: "2026-08-31T00:00:00Z", login: "alice", kind: "issue" },
    { id: 2, createdAt: "2026-08-31T01:00:00Z", login: "me", kind: "issue" },
    { id: 3, createdAt: "2026-08-31T02:00:00Z", login: "bob", kind: "review" },
  ];

  it("前回時刻より後のものだけを、自分のコメントを除いて返す", () => {
    const picked = selectNewComments(comments, "2026-08-31T00:00:00Z", 1, "me");
    assert.deepEqual(
      picked.map((comment) => comment.id),
      [3],
    );
  });

  it("同時刻は id で判定する(同じ秒に複数付いても取りこぼさない)", () => {
    const same = [
      { id: 10, createdAt: "2026-08-31T00:00:00Z", login: "bob", kind: "issue" },
      { id: 11, createdAt: "2026-08-31T00:00:00Z", login: "bob", kind: "issue" },
    ];
    const picked = selectNewComments(same, "2026-08-31T00:00:00Z", 10, null);
    assert.deepEqual(
      picked.map((comment) => comment.id),
      [11],
    );
  });

  it("ミリ秒付きの起点と秒精度の時刻を文字列比較しない", () => {
    // 文字列比較だと "…:52Z" > "…:52.500Z" になってしまう。
    const later = [{ id: 1, createdAt: "2026-08-31T00:00:52Z", login: "bob", kind: "issue" }];
    assert.deepEqual(selectNewComments(later, "2026-08-31T00:00:52.500Z", null, null), []);
  });

  it("結果は作成時刻の昇順(最後の 1 件が最新)", () => {
    const shuffled = [comments[2], comments[0], comments[1]];
    const picked = selectNewComments(shuffled, null, null, null);
    assert.deepEqual(
      picked.map((comment) => comment.id),
      [1, 2, 3],
    );
  });
});

describe("summarizeCheckRuns / summarizeCombinedStatus / mergeCi", () => {
  it("失敗系の conclusion を failure として拾い、ジョブ名を残す", () => {
    for (const conclusion of ["failure", "timed_out", "cancelled", "action_required"]) {
      const summary = summarizeCheckRuns([{ name: "build", status: "completed", conclusion }]);
      assert.deepEqual(summary, { state: "failure", failed: ["build"] }, conclusion);
    }
  });

  it("未完了があれば pending、成功系だけなら success、空なら none", () => {
    assert.equal(summarizeCheckRuns([{ name: "a", status: "in_progress" }]).state, "pending");
    assert.equal(
      summarizeCheckRuns([
        { name: "a", status: "completed", conclusion: "success" },
        { name: "b", status: "completed", conclusion: "skipped" },
        { name: "c", status: "completed", conclusion: "neutral" },
      ]).state,
      "success",
    );
    assert.equal(summarizeCheckRuns([]).state, "none");
  });

  it("失敗は未完了より優先する(赤いまま回っていても失敗を見逃さない)", () => {
    const summary = summarizeCheckRuns([
      { name: "a", status: "in_progress" },
      { name: "b", status: "completed", conclusion: "failure" },
    ]);
    assert.deepEqual(summary, { state: "failure", failed: ["b"] });
  });

  it("combined status を 4 段階に丸める", () => {
    assert.deepEqual(
      summarizeCombinedStatus({
        state: "failure",
        statuses: [
          { context: "ci/circleci", state: "failure" },
          { context: "ci/ok", state: "success" },
        ],
      }),
      { state: "failure", failed: ["ci/circleci"] },
    );
    assert.equal(summarizeCombinedStatus({ state: "pending", statuses: [] }).state, "pending");
    assert.equal(summarizeCombinedStatus({ state: "success", statuses: [] }).state, "success");
    assert.equal(summarizeCombinedStatus(null).state, "none");
  });

  it("check-runs と combined status のうち深刻な方を採る", () => {
    assert.deepEqual(
      mergeCi({ state: "success", failed: [] }, { state: "failure", failed: ["ci/x"] }),
      { state: "failure", failed: ["ci/x"] },
    );
    assert.equal(mergeCi({ state: "pending", failed: [] }, { state: "success", failed: [] }).state, "pending");
    assert.equal(mergeCi(null, null).state, "none");
  });
});

describe("shouldNotifyCiFailure", () => {
  const failure = { state: "failure", failed: ["build"] };

  it("失敗していなければ通知しない", () => {
    assert.equal(
      shouldNotifyCiFailure({ ciSha: "a", ciNotified: null }, "a", { state: "pending", failed: [] }),
      false,
    );
  });

  it("同じ head SHA の同じ失敗は 1 度しか通知しない", () => {
    assert.equal(shouldNotifyCiFailure({ ciSha: "a", ciNotified: null }, "a", failure), true);
    assert.equal(shouldNotifyCiFailure({ ciSha: "a", ciNotified: "failure" }, "a", failure), false);
  });

  it("head SHA が進んだら、前回も失敗でも新しい失敗として通知する", () => {
    assert.equal(shouldNotifyCiFailure({ ciSha: "a", ciNotified: "failure" }, "b", failure), true);
  });

  it("初めて観測した PR が既に赤ければ通知する", () => {
    assert.equal(shouldNotifyCiFailure({ ciSha: null, ciNotified: null }, "a", failure), true);
  });

  it("成功に戻ってからまた落ちたら通知する", () => {
    assert.equal(shouldNotifyCiFailure({ ciSha: "a", ciNotified: null }, "a", failure), true);
  });
});

describe("decideNoticeAction", () => {
  const statuses = new Map([
    ["open", "new"],
    ["closed", "done"],
  ]);

  it("通知タスクがまだ無ければ作る", () => {
    assert.deepEqual(decideNoticeAction(null, statuses), { kind: "create" });
  });

  it("未完了の通知タスクが残っていれば追記する(重複作成の防止)", () => {
    assert.deepEqual(decideNoticeAction("open", statuses), { kind: "append", taskId: "open" });
  });

  it("完了済み・削除済みなら作り直す", () => {
    assert.deepEqual(decideNoticeAction("closed", statuses), { kind: "create" });
    assert.deepEqual(decideNoticeAction("vanished", statuses), { kind: "create" });
  });
});

describe("planNotification", () => {
  it("参照元が監視中なら、通知タスクを作らずそのタスクを起こす", () => {
    const statuses = new Map([["t1", "watching"]]);
    assert.deepEqual(planNotification(["t1"], statuses, null), {
      kind: "wake",
      taskIds: ["t1"],
    });
  });

  it("監視中のタスクが複数あれば全部起こす(監視中でないものは触らない)", () => {
    const statuses = new Map([
      ["t1", "watching"],
      ["t2", "todo"],
      ["t3", "watching"],
    ]);
    assert.deepEqual(planNotification(["t1", "t2", "t3"], statuses, null), {
      kind: "wake",
      taskIds: ["t1", "t3"],
    });
  });

  it("監視中が 1 つでもあれば、通知タスクが残っていても起床を優先する", () => {
    const statuses = new Map([
      ["t1", "watching"],
      ["notice", "new"],
    ]);
    assert.deepEqual(planNotification(["t1"], statuses, "notice"), {
      kind: "wake",
      taskIds: ["t1"],
    });
  });

  it("監視中が無ければ従来どおり(未完了の通知タスクへ追記 / 無ければ作成)", () => {
    const statuses = new Map([
      ["t1", "todo"],
      ["notice", "new"],
    ]);
    assert.deepEqual(planNotification(["t1"], statuses, "notice"), {
      kind: "append",
      taskId: "notice",
    });
    assert.deepEqual(planNotification(["t1"], statuses, null), { kind: "create" });
  });

  /**
   * 起床したタスクは次のラウンドでは `new` なので、そのまま従来ルートへ戻る。
   * 起床では noticeTaskId を触らないので、残っている通知タスクへの追記も効き続ける。
   */
  it("起きたあと(watching でなくなった後)は従来ルートに戻る", () => {
    const woken = new Map([
      ["t1", "new"],
      ["notice", "new"],
    ]);
    assert.deepEqual(planNotification(["t1"], woken, "notice"), {
      kind: "append",
      taskId: "notice",
    });
  });

  it("ボードから消えたタスクは監視中とみなさない", () => {
    assert.deepEqual(planNotification(["ghost"], new Map(), null), { kind: "create" });
  });
});

describe("buildReasons", () => {
  it("コメントと CI の理由を並べる", () => {
    const comments = [
      { id: 1, createdAt: "2026-08-31T00:00:00Z", login: "a", kind: "issue" },
      { id: 2, createdAt: "2026-08-31T00:01:00Z", login: "b", kind: "review" },
    ];
    assert.deepEqual(buildReasons(comments, { state: "failure", failed: ["build", "test"] }), [
      "新しいコメントが 2 件(うちレビューコメント 1 件)",
      "CI が失敗: build, test",
    ]);
  });

  it("何も無ければ空(= 通知を作らない)", () => {
    assert.deepEqual(buildReasons([], null), []);
    assert.deepEqual(buildReasons([], { state: "success", failed: [] }), []);
  });
});

describe("pullRequestApiPath", () => {
  it("PR の URL から REST API のパスを作る", () => {
    const ref = parsePullRequestUrl("https://github.com/rust-lang/rust/pull/1234/files");
    assert.equal(pullRequestApiPath(ref), "/repos/rust-lang/rust/pulls/1234");
  });

  it("大文字小文字は URL のまま(キーだけが小文字)", () => {
    const ref = parsePullRequestUrl("https://github.com/Foo/Bar-Baz/pull/9");
    assert.equal(pullRequestApiPath(ref), "/repos/Foo/Bar-Baz/pulls/9");
  });
});

describe("shouldFillDescription", () => {
  const origin = "plugin:github";
  const base = { id: "t1", origin: "user", description: "", deletedAt: null };

  it("詳細が空のタスクは埋める", () => {
    assert.equal(shouldFillDescription(base, origin, false), true);
    assert.equal(shouldFillDescription({ ...base, description: "   \n " }, origin, false), true);
  });

  it("ユーザーが書いた文章は上書きしない", () => {
    assert.equal(shouldFillDescription({ ...base, description: "手で書いたメモ" }, origin, false), false);
  });

  it("一度記入したタスクは二度と触らない(消されても埋め直さない)", () => {
    assert.equal(shouldFillDescription(base, origin, true), false);
  });

  it("自分が作った通知タスクと削除済みタスクは対象外", () => {
    assert.equal(shouldFillDescription({ ...base, origin }, origin, false), false);
    assert.equal(
      shouldFillDescription({ ...base, deletedAt: "2026-09-01T00:00:00Z" }, origin, false),
      false,
    );
  });

  it("Done でも埋める(過去の PR でも手掛かりは残したい)", () => {
    assert.equal(shouldFillDescription({ ...base, status: "done" }, origin, false), true);
  });
});

describe("collectDescriptionTargets", () => {
  const origin = "plugin:github";

  it("詳細が空で PR URL を持つタスクを、タスクごとに 1 件だけ集める", () => {
    const tasks = [
      { id: "t1", origin: "user", description: "", deletedAt: null },
      { id: "t2", origin: "user", description: "既に書いてある", deletedAt: null },
      { id: "t3", origin: "user", description: "", deletedAt: null },
      { id: "t4", origin, description: "", deletedAt: null },
    ];
    const resources = [
      // 最初に見つかった PR を使う。
      { taskId: "t1", kind: "url", value: "https://github.com/o/r/pull/1" },
      { taskId: "t1", kind: "url", value: "https://github.com/o/r/pull/2" },
      // 詳細が埋まっているタスクは対象外。
      { taskId: "t2", kind: "url", value: "https://github.com/o/r/pull/3" },
      // PR 以外・URL 以外は無視。
      { taskId: "t3", kind: "url", value: "https://example.com/" },
      { taskId: "t3", kind: "file", value: "https://github.com/o/r/pull/4" },
      // 自分が作った通知タスクは対象外。
      { taskId: "t4", kind: "url", value: "https://github.com/o/r/pull/5" },
    ];
    assert.deepEqual(
      collectDescriptionTargets(tasks, resources, origin, new Set()).map((t) => [t.taskId, t.ref.key]),
      [["t1", "o/r#1"]],
    );
  });

  it("記入済みのタスクは外す", () => {
    const tasks = [{ id: "t1", origin: "user", description: "", deletedAt: null }];
    const resources = [{ taskId: "t1", kind: "url", value: "https://github.com/o/r/pull/1" }];
    assert.deepEqual(collectDescriptionTargets(tasks, resources, origin, new Set(["t1"])), []);
  });

  it("ボードに無いタスクのリソースは無視する", () => {
    const resources = [{ taskId: "ghost", kind: "url", value: "https://github.com/o/r/pull/1" }];
    assert.deepEqual(collectDescriptionTargets([], resources, origin, new Set()), []);
  });
});

describe("truncateBody", () => {
  it("改行を LF にそろえて前後の空白を落とす", () => {
    assert.equal(truncateBody("\r\n  一行目\r\n二行目  \r\n"), "一行目\n二行目");
  });

  it("上限を超えたら切って … を付ける", () => {
    assert.equal(truncateBody("abcdefghij", 4), "abcd…");
    // 切れ目の空白は残さない。
    assert.equal(truncateBody("abc defghij", 4), "abc…");
  });

  it("上限ちょうどなら切らない", () => {
    assert.equal(truncateBody("abcd", 4), "abcd");
  });
});

describe("buildPrDescription", () => {
  const ref = parsePullRequestUrl("https://github.com/rust-lang/rust/pull/42");

  it("タイトル・出典・本文を並べる", () => {
    const description = buildPrDescription(
      { title: "Fix the thing", state: "open", user: { login: "alice" }, body: "本文です。" },
      ref,
    );
    assert.equal(description, "Fix the thing\nrust-lang/rust#42 (open) by alice\n\n本文です。");
  });

  it("本文が空なら本文部分を省く", () => {
    const description = buildPrDescription(
      { title: "Fix", state: "open", user: { login: "alice" }, body: null },
      ref,
    );
    assert.equal(description, "Fix\nrust-lang/rust#42 (open) by alice");
  });

  it("merged フラグを state より優先する", () => {
    assert.match(
      buildPrDescription({ title: "T", state: "closed", merged: true }, ref),
      /^T\nrust-lang\/rust#42 \(merged\)$/,
    );
    assert.match(
      buildPrDescription({ title: "T", state: "closed", merged: false }, ref),
      /\(closed\)$/,
    );
  });

  it("タイトルや作者が取れなくても壊れない", () => {
    assert.equal(buildPrDescription({}, ref), "rust-lang/rust#42\nrust-lang/rust#42 (open)");
  });

  it("長い本文は 400 文字で切る", () => {
    const body = "あ".repeat(500);
    const description = buildPrDescription({ title: "T", body }, ref);
    const tail = description.split("\n\n")[1];
    assert.equal(tail, `${"あ".repeat(400)}…`);
  });
});

/* ==================== 受信箱(レビュー依頼・メンション)==================== */

/** `InboxItem` を組み立てる小道具。 */
const inboxItem = (url, over = {}) => ({
  ref: parseIssueOrPullUrl(url),
  title: "タイトル",
  author: "alice",
  updatedAt: "2026-09-01T00:00:00Z",
  kinds: ["mention"],
  ...over,
});

describe("parseIssueOrPullUrl", () => {
  it("PR の URL は isPullRequest: true で返す", () => {
    assert.deepEqual(parseIssueOrPullUrl("https://github.com/o/r/pull/7/files"), {
      owner: "o",
      repo: "r",
      number: 7,
      key: "o/r#7",
      url: "https://github.com/o/r/pull/7",
      isPullRequest: true,
    });
  });

  it("issue の URL は isPullRequest: false で返し、URL も /issues/ に正規化する", () => {
    assert.deepEqual(parseIssueOrPullUrl("https://github.com/o/r/issues/7#issuecomment-1"), {
      owner: "o",
      repo: "r",
      number: 7,
      key: "o/r#7",
      url: "https://github.com/o/r/issues/7",
      isPullRequest: false,
    });
  });

  it("issue と PR は同じ番号空間なのでキーがそろう(追跡済み判定に効く)", () => {
    assert.equal(
      parseIssueOrPullUrl("https://github.com/Foo/Bar/issues/9").key,
      parseIssueOrPullUrl("https://github.com/foo/bar/pull/9").key,
    );
  });

  it("issue でも PR でもない URL は拾わない", () => {
    for (const url of [
      "https://github.com/o/r/discussions/1",
      "https://github.com/o/r/commit/abc",
      "https://gitlab.com/o/r/issues/1",
      "https://github.com.evil.test/o/r/issues/1",
      "",
    ]) {
      assert.equal(parseIssueOrPullUrl(url), null, url);
    }
  });

  it("parsePullRequestUrl は issue を弾き、isPullRequest を含まない形で返す", () => {
    assert.equal(parsePullRequestUrl("https://github.com/o/r/issues/7"), null);
    assert.deepEqual(Object.keys(parsePullRequestUrl("https://github.com/o/r/pull/7")), [
      "owner",
      "repo",
      "number",
      "key",
      "url",
    ]);
  });
});

describe("検索クエリ", () => {
  it("レビュー依頼は @me で引く(login を取りに行かなくて済む)", () => {
    assert.equal(buildReviewRequestQuery(), "type:pr state:open review-requested:@me");
  });

  it("メンションは前回確認時刻からの更新に絞る", () => {
    assert.equal(
      buildMentionQuery("2026-09-01T00:00:00Z"),
      "state:open mentions:@me updated:>2026-09-01T00:00:00Z",
    );
  });

  it("パスはクエリを URL エンコードし、更新順・1 ページで打ち切る", () => {
    assert.equal(
      searchApiPath("state:open mentions:@me updated:>2026-09-01T00:00:00Z", 50),
      "/search/issues?q=state%3Aopen%20mentions%3A%40me%20updated%3A%3E2026-09-01T00%3A00%3A00Z" +
        "&per_page=50&sort=updated&order=desc",
    );
  });

  it("件数を省くと既定(50)になる", () => {
    assert.match(searchApiPath("type:pr"), /&per_page=50&/);
  });
});

describe("toInboxItems", () => {
  it("検索結果を判定に必要な形へ落とす", () => {
    const items = toInboxItems(
      [
        {
          html_url: "https://github.com/o/r/pull/1",
          title: "  直す  ",
          user: { login: "bob" },
          updated_at: "2026-09-01T10:00:00Z",
          pull_request: {},
        },
      ],
      "review",
    );
    assert.deepEqual(items, [
      {
        ref: parseIssueOrPullUrl("https://github.com/o/r/pull/1"),
        title: "直す",
        author: "bob",
        updatedAt: "2026-09-01T10:00:00Z",
        kinds: ["review"],
      },
    ]);
  });

  it("URL を解析できないものは落とし、タイトル・作者が無くても壊れない", () => {
    const items = toInboxItems(
      [
        { html_url: "https://github.com/o/r/discussions/1" },
        { html_url: "https://github.com/o/r/issues/2" },
      ],
      "mention",
    );
    assert.equal(items.length, 1);
    assert.deepEqual([items[0].title, items[0].author, items[0].updatedAt], ["", "", ""]);
  });
});

describe("mergeInboxItems", () => {
  it("同じ issue/PR は 1 件にまとめ、kinds を合流させる", () => {
    const merged = mergeInboxItems(
      [inboxItem("https://github.com/o/r/pull/1", { kinds: ["review"] })],
      [inboxItem("https://github.com/o/r/pull/1", { kinds: ["mention"] })],
    );
    assert.equal(merged.length, 1);
    assert.deepEqual(merged[0].kinds, ["review", "mention"]);
  });

  it("新しい updated_at を採る", () => {
    const merged = mergeInboxItems(
      [
        inboxItem("https://github.com/o/r/pull/1", {
          kinds: ["review"],
          updatedAt: "2026-09-01T00:00:00Z",
        }),
      ],
      [
        inboxItem("https://github.com/o/r/pull/1", {
          kinds: ["mention"],
          updatedAt: "2026-09-02T00:00:00Z",
        }),
      ],
    );
    assert.equal(merged[0].updatedAt, "2026-09-02T00:00:00Z");
  });

  it("読めない updated_at は読める方に置き換わる", () => {
    const merged = mergeInboxItems(
      [inboxItem("https://github.com/o/r/pull/1", { kinds: ["review"], updatedAt: "" })],
      [
        inboxItem("https://github.com/o/r/pull/1", {
          kinds: ["mention"],
          updatedAt: "2026-09-02T00:00:00Z",
        }),
      ],
    );
    assert.equal(merged[0].updatedAt, "2026-09-02T00:00:00Z");
  });

  it("別々の issue/PR はキーの昇順で並ぶ", () => {
    const merged = mergeInboxItems(
      [inboxItem("https://github.com/o/r/pull/2")],
      [inboxItem("https://github.com/o/r/issues/1")],
    );
    assert.deepEqual(
      merged.map((item) => item.ref.key),
      ["o/r#1", "o/r#2"],
    );
  });
});

describe("collectTrackedKeys / selectInboxCandidates", () => {
  const origin = "plugin:github";

  it("未完了タスクが参照している issue / PR を追跡済みとみなす", () => {
    const tasks = [
      { id: "t1", status: "todo", origin: "user" },
      { id: "t2", status: "done", origin: "user" },
      { id: "t3", status: "new", origin },
    ];
    const resources = [
      { taskId: "t1", kind: "url", value: "https://github.com/o/r/pull/1" },
      { taskId: "t1", kind: "url", value: "https://github.com/o/r/issues/2" },
      // Done タスク・自分が作ったタスク・URL 以外・ボードに無いタスクは数えない。
      { taskId: "t2", kind: "url", value: "https://github.com/o/r/pull/3" },
      { taskId: "t3", kind: "url", value: "https://github.com/o/r/pull/4" },
      { taskId: "t1", kind: "file", value: "https://github.com/o/r/pull/5" },
      { taskId: "ghost", kind: "url", value: "https://github.com/o/r/pull/6" },
    ];
    assert.deepEqual([...collectTrackedKeys(tasks, resources, origin)].sort(), [
      "o/r#1",
      "o/r#2",
    ]);
  });

  it("追跡済みの issue / PR は受信箱に取り込まない(PR 監視との二重通知を防ぐ)", () => {
    const items = [
      inboxItem("https://github.com/o/r/pull/1"),
      inboxItem("https://github.com/o/r/issues/2"),
    ];
    assert.deepEqual(
      selectInboxCandidates(items, new Set(["o/r#1"])).map((item) => item.ref.key),
      ["o/r#2"],
    );
  });
});

describe("decideInboxNotification", () => {
  const review = (over = {}) =>
    inboxItem("https://github.com/o/r/pull/1", { kinds: ["review"], ...over });

  it("初めて見たレビュー依頼は通知する(理由に作者を載せる)", () => {
    const decision = decideInboxNotification(null, review());
    assert.equal(decision.review, true);
    assert.equal(decision.mention, false);
    assert.deepEqual(decision.reasons, ["alice の PR のレビューを依頼されています"]);
  });

  it("通知済みのレビュー依頼は繰り返さない(re-request で updated が動いても黙る)", () => {
    const state = {
      reviewNotified: true,
      mentionNotifiedAt: null,
      noticeTaskId: "n1",
      seenAt: "2026-09-01T00:00:00Z",
    };
    const decision = decideInboxNotification(state, review({ updatedAt: "2026-09-09T00:00:00Z" }));
    assert.deepEqual(decision.reasons, []);
    assert.equal(decision.review, false);
  });

  it("初めて見たメンションは通知する(issue か PR かを文言に出す)", () => {
    assert.deepEqual(decideInboxNotification(null, inboxItem("https://github.com/o/r/issues/1")).reasons, [
      "alice の issue でメンションされています",
    ]);
    assert.deepEqual(decideInboxNotification(null, inboxItem("https://github.com/o/r/pull/1")).reasons, [
      "alice の PR でメンションされています",
    ]);
  });

  it("メンションは前回通知した更新より新しいときだけ通知する", () => {
    const state = {
      reviewNotified: false,
      mentionNotifiedAt: "2026-09-01T00:00:00Z",
      noticeTaskId: null,
      seenAt: "2026-09-01T00:00:00Z",
    };
    // 同時刻・過去は通知しない。
    assert.equal(decideInboxNotification(state, inboxItem("https://github.com/o/r/pull/1")).mention, false);
    assert.equal(
      decideInboxNotification(
        state,
        inboxItem("https://github.com/o/r/pull/1", { updatedAt: "2026-08-31T00:00:00Z" }),
      ).mention,
      false,
    );
    // 新しければ通知する。
    assert.equal(
      decideInboxNotification(
        state,
        inboxItem("https://github.com/o/r/pull/1", { updatedAt: "2026-09-01T00:00:01Z" }),
      ).mention,
      true,
    );
  });

  it("レビュー依頼とメンションが同時に当たったら理由を 2 つ並べる", () => {
    const both = inboxItem("https://github.com/o/r/pull/1", { kinds: ["review", "mention"] });
    const decision = decideInboxNotification(null, both);
    assert.deepEqual(decision, {
      review: true,
      mention: true,
      reasons: [
        "alice の PR のレビューを依頼されています",
        "alice の PR でメンションされています",
      ],
    });
  });

  it("作者が取れなくても文言が壊れない", () => {
    const decision = decideInboxNotification(null, review({ author: "" }));
    assert.deepEqual(decision.reasons, ["レビューを依頼されています"]);
  });
});

describe("buildInboxTitle / buildInboxDescription", () => {
  it("レビュー依頼を主に見立てる(両方に当たったときも)", () => {
    const item = inboxItem("https://github.com/o/r/pull/1", { kinds: ["review", "mention"] });
    assert.equal(buildInboxTitle(item, { review: true, mention: true, reasons: [] }), "レビュー依頼: o/r#1");
    assert.equal(buildInboxTitle(item, { review: false, mention: true, reasons: [] }), "メンション: o/r#1");
  });

  it("タイトル・出典・理由・URL を並べる", () => {
    const item = inboxItem("https://github.com/o/r/issues/3", { title: "落ちる", author: "bob" });
    assert.equal(
      buildInboxDescription(item, ["bob の issue でメンションされています"]),
      "落ちる\no/r#3 (issue) by bob\n\nbob の issue でメンションされています\n\nhttps://github.com/o/r/issues/3",
    );
  });

  it("タイトルや作者が取れなくても壊れない", () => {
    const item = inboxItem("https://github.com/o/r/pull/3", { title: "", author: "" });
    assert.equal(buildInboxDescription(item, []), "o/r#3\no/r#3 (pr)\n\nhttps://github.com/o/r/pull/3");
  });
});

describe("nextInboxState", () => {
  const now = "2026-09-02T00:00:00Z";
  const item = inboxItem("https://github.com/o/r/pull/1", { updatedAt: "2026-09-01T12:00:00Z" });

  it("通知した内容だけを記録し、観測時刻を進める", () => {
    assert.deepEqual(
      nextInboxState(null, item, { review: true, mention: true, reasons: ["x"] }, "n1", now),
      {
        reviewNotified: true,
        mentionNotifiedAt: "2026-09-01T12:00:00Z",
        noticeTaskId: "n1",
        seenAt: now,
      },
    );
  });

  it("通知しなかったときも観測時刻は進める(TTL 掃除の対象から外す)", () => {
    const previous = {
      reviewNotified: true,
      mentionNotifiedAt: "2026-08-01T00:00:00Z",
      noticeTaskId: "n1",
      seenAt: "2026-08-01T00:00:00Z",
    };
    assert.deepEqual(
      nextInboxState(previous, item, { review: false, mention: false, reasons: [] }, "n1", now),
      {
        reviewNotified: true,
        mentionNotifiedAt: "2026-08-01T00:00:00Z",
        noticeTaskId: "n1",
        seenAt: now,
      },
    );
  });

  it("一度立てたレビュー通知済みは下ろさない(下ろすのは掃除の仕事)", () => {
    const previous = {
      reviewNotified: true,
      mentionNotifiedAt: null,
      noticeTaskId: null,
      seenAt: now,
    };
    assert.equal(
      nextInboxState(previous, item, { review: false, mention: true, reasons: ["x"] }, null, now)
        .reviewNotified,
      true,
    );
  });

  it("更新時刻が取れないメンションは「今」を起点にする", () => {
    const blank = inboxItem("https://github.com/o/r/pull/1", { updatedAt: "" });
    assert.equal(
      nextInboxState(null, blank, { review: false, mention: true, reasons: ["x"] }, null, now)
        .mentionNotifiedAt,
      now,
    );
  });
});

describe("planInboxPrune", () => {
  const nowMs = Date.parse("2026-09-30T00:00:00Z");
  const state = (over = {}) => ({
    reviewNotified: false,
    mentionNotifiedAt: null,
    noticeTaskId: null,
    seenAt: "2026-09-29T00:00:00Z",
    ...over,
  });

  it("まだレビュー依頼として生きているなら残す", () => {
    assert.equal(planInboxPrune(state({ reviewNotified: true }), true, true, nowMs), "keep");
  });

  it("依頼が解消されたら通知済みだけ解除する(再依頼をまた知らせるため)", () => {
    assert.equal(planInboxPrune(state({ reviewNotified: true }), true, false, nowMs), "clear-review");
  });

  it("レビュー依頼の検索をしていないラウンドでは解除しない", () => {
    assert.equal(planInboxPrune(state({ reviewNotified: true }), false, false, nowMs), "keep");
  });

  it("最後の観測から TTL を過ぎたら捨てる", () => {
    assert.equal(
      planInboxPrune(state({ seenAt: "2026-08-01T00:00:00Z" }), true, false, nowMs),
      "delete",
    );
    // レビュー依頼として生きている限り TTL では消さない。
    assert.equal(
      planInboxPrune(state({ seenAt: "2026-08-01T00:00:00Z" }), true, true, nowMs),
      "keep",
    );
  });

  it("TTL 以内のメンション状態は残す", () => {
    assert.equal(planInboxPrune(state(), true, false, nowMs), "keep");
  });

  it("壊れた値・読めない観測時刻は捨てる", () => {
    assert.equal(planInboxPrune(null, true, false, nowMs), "delete");
    assert.equal(planInboxPrune(state({ seenAt: "???" }), true, false, nowMs), "delete");
  });
});
