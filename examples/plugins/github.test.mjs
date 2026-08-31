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
  buildPrDescription,
  buildReasons,
  collectDescriptionTargets,
  collectPullRequestTargets,
  decideNoticeAction,
  mergeCi,
  parsePullRequestUrl,
  planNotification,
  pullRequestApiPath,
  selectNewComments,
  shouldFillDescription,
  shouldNotifyCiFailure,
  summarizeCheckRuns,
  summarizeCombinedStatus,
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
