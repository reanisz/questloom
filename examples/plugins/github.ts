/**
 * questloom GitHub 統合プラグイン(TypeScript プラグイン層のパイロット実装)。
 *
 * ## 何をするか
 *
 * 未完了タスクの関連リソースに貼られた GitHub PR の URL を見張り、
 *
 * - 前回確認以降に付いた**新しいコメント**(自分のコメントは除く)
 * - **CI の失敗への遷移**(同じ失敗を繰り返し通知しない)
 *
 * を検知したら「PR を確認する: owner/repo#123」というインスタントの子タスクを New に作る。
 * 既に未完了の通知タスクがあれば、新しく作らずアップデート履歴へ追記する。
 *
 * もう一つ、**description の自動記入**をする。PR の URL が付いたタスクの description が
 * 空なら、PR のタイトル・状態・作者・本文の先頭を書き込む。タスクイベント
 * (`ctx.onTaskEvent`)で即座に反応し、ポーリングでも取りこぼしを拾う。
 * 一度記入したタスクは KV に記録して二度と触らない(ユーザーの文章を上書きしないため)。
 * こちらは **PAT が無くても動く**(認証なしで public リポジトリの PR は読める)。
 *
 * ## 使い方
 *
 * 1. このファイルを設定画面の「プラグイン」節に出ているフォルダ
 *    (既定では `%APPDATA%\dev.reanisz.questloom\plugins\`)へコピーする。
 * 2. 「プラグインを再読み込み」を押す。
 * 3. 設定画面のプラグイン節で **Personal Access Token** を入れて保存する。
 *    必要なスコープは PR を読めるだけ(public のみなら fine-grained の Pull requests: Read、
 *    private も見るならそのリポジトリを対象に含めること)。
 *    PR の監視には PAT が要る。description の自動記入だけなら未設定でもよい。
 *
 * ## 実装メモ
 *
 * - 1 ファイル完結。`import` は使えないので GitHub API の型もここで宣言している。
 *   API の型は `apps/desktop/src/plugin-host/sdk.ts` を参照(`defineQuestloomPlugin` はグローバル)。
 * - `ctx.fetch` は webview の `fetch` なので CORS の制約を受ける。api.github.com は
 *   `Access-Control-Allow-Origin: *` を返し、プリフライトの `Access-Control-Allow-Headers` に
 *   `Authorization` / `If-None-Match` / `X-GitHub-Api-Version` を、
 *   `Access-Control-Expose-Headers` に `ETag` / `X-RateLimit-*` / `Retry-After` を含むため、
 *   認証付き GET と ETag による条件付きリクエストがそのまま通る。
 * - PR ごとの前回状態は `ctx.kv` に持つ(キーは `pr:<owner>/<repo>#<num>`)。
 *   PR がクローズ/マージされたら破棄し、どのタスクからも参照されなくなったキーも掃除する。
 * - description を記入したタスクは `desc:<taskId>` に記録する
 *   (値は `{ pr, at }`)。ボードから消えたタスクの記録は掃除する。
 * - 判定ロジックは純関数に切り出してファイル末尾で `export` している
 *   (ホストは default export しか見ないので無害。`examples/plugins/github.test.mjs` が検証する)。
 */

/* ================================================================ 型と定数 */

/** `ctx` の型。SDK を import できないので、グローバル宣言から引き出す。 */
type Ctx = Parameters<Parameters<typeof defineQuestloomPlugin>[0]["activate"]>[0];

/** GitHub REST API のベース URL(manifest の `fetchDomains` と一致させること)。 */
const API_BASE = "https://api.github.com";

/** 固定する REST API のバージョン。 */
const API_VERSION = "2022-11-28";

/** KV に置く PR 状態のキー接頭辞。 */
const PR_KEY_PREFIX = "pr:";

/** description を記入済みのタスクを覚えておく KV キーの接頭辞。 */
const DESC_KEY_PREFIX = "desc:";

/** description に載せる PR 本文の最大文字数。超えたら切って「…」を付ける。 */
const DESCRIPTION_BODY_LIMIT = 400;

/**
 * タスクイベントから description 記入までの待ち時間(ミリ秒)。
 *
 * リソース追加の直後は他の更新も連続しがちなので、少し待ってまとめて処理する。
 */
const FILL_DEBOUNCE_MS = 1_500;

/** 自分の login をキャッシュする KV キー。 */
const SELF_LOGIN_KEY = "selfLogin";

/** 1 回のリクエストで取る最大件数(GitHub の上限)。 */
const PER_PAGE = 100;

/** 既定のポーリング間隔(分)。 */
const DEFAULT_INTERVAL_MINUTES = 5;

/**
 * PR の URL。`https://github.com/<owner>/<repo>/pull/<番号>` を拾う。
 * 末尾に `/files` や `#issuecomment-...` が付いていても良い。
 */
const PR_URL_PATTERN =
  /^https?:\/\/(?:www\.)?github\.com\/([A-Za-z0-9][A-Za-z0-9-]*)\/([A-Za-z0-9._-]+)\/pull\/(\d+)(?:[/?#].*)?$/;

/** 失敗とみなす check-run の `conclusion`。 */
const FAILED_CONCLUSIONS = new Set([
  "failure",
  "timed_out",
  "cancelled",
  "action_required",
  "startup_failure",
]);

/** CI の状態を 4 段階に丸めたもの。深刻な順に failure > pending > success > none。 */
type CiState = "none" | "success" | "pending" | "failure";

/** CI の状態と、失敗しているジョブ名。 */
interface CiSummary {
  state: CiState;
  /** 失敗しているジョブ/コンテキストの名前(表示用。多くても数件に切る)。 */
  failed: string[];
}

/** URL から取り出した PR の参照。 */
interface PullRequestRef {
  owner: string;
  repo: string;
  number: number;
  /** KV と重複排除に使う正規化キー(`owner/repo#123`、小文字)。 */
  key: string;
  /** 正規化した PR の URL(通知タスクの主リソースに使う)。 */
  url: string;
}

/** 監視対象の PR 1 件と、それを参照している未完了タスク。 */
interface PullRequestTarget {
  ref: PullRequestRef;
  /** この PR を参照している未完了タスクの id(昇順)。先頭を通知タスクの親にする。 */
  taskIds: string[];
}

/** コメント 1 件を比較用に正規化したもの。 */
interface CommentInfo {
  id: number;
  /** 作成時刻 (RFC 3339)。 */
  createdAt: string;
  /** 投稿者の login(取れなければ空文字)。 */
  login: string;
  /** issue コメントかレビューコメントか。 */
  kind: "issue" | "review";
}

/** KV に持つ PR ごとの前回状態。 */
interface PrState {
  /** 一度でも観測したか。初回は通知せず現在値を記録するだけにする。 */
  initialized: boolean;
  /** 通知済みの最新コメントの作成時刻 (RFC 3339)。`since` パラメータにも使う。 */
  lastCommentAt: string | null;
  /** 同じ作成時刻のコメントを取りこぼさないための最新コメント id。 */
  lastCommentId: number | null;
  /** CI の判定に使った head SHA。 */
  ciSha: string | null;
  /** check-runs から導いた状態(304 のときはこれを使い回す)。 */
  ciChecks: CiSummary | null;
  /** combined status から導いた状態(同上)。 */
  ciCombined: CiSummary | null;
  /** 最後に**通知した** CI の状態。同じ失敗を繰り返し通知しないために持つ。 */
  ciNotified: CiState | null;
  /** 直近で作った通知タスクの id。未完了なら再利用して追記する。 */
  noticeTaskId: string | null;
  /** エンドポイント(パス)ごとの ETag。 */
  etags: Record<string, string>;
}

/* ------------------------------------------------- GitHub API のレスポンス */

interface GhUser {
  login?: string;
}

interface GhPullRequest {
  state?: string;
  merged?: boolean;
  head?: { sha?: string };
  /** PR のタイトル(description の自動記入で使う)。 */
  title?: string;
  /** PR の本文。空のことも null のこともある。 */
  body?: string | null;
  /** 作成者。 */
  user?: GhUser | null;
}

interface GhComment {
  id?: number;
  created_at?: string;
  user?: GhUser | null;
}

interface GhCheckRun {
  name?: string;
  status?: string;
  conclusion?: string | null;
}

interface GhCheckRuns {
  check_runs?: GhCheckRun[];
}

interface GhStatusContext {
  context?: string;
  state?: string;
}

interface GhCombinedStatus {
  state?: string;
  statuses?: GhStatusContext[];
}

/** 2xx 以外が返ってきたことを表す例外。ステータスで扱いを変えるために持つ。 */
class HttpError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "HttpError";
    this.status = status;
  }
}

/** レート制限に当たったことを表す例外。捕まえたらそのラウンドを打ち切る。 */
class RateLimitError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "RateLimitError";
  }
}

/* ============================================================== 純粋な判定 */

/**
 * GitHub の PR URL を解析する。PR URL でなければ `null`。
 *
 * `issues` や `commit` の URL、github.com 以外のホストは弾く。
 */
function parsePullRequestUrl(url: string): PullRequestRef | null {
  if (typeof url !== "string") return null;
  const match = PR_URL_PATTERN.exec(url.trim());
  if (!match) return null;
  const owner = match[1];
  // `repo.git` のような末尾は URL としては来ないが、念のため落としておく。
  const repo = match[2].replace(/\.git$/i, "");
  const number = Number(match[3]);
  if (!repo || !Number.isSafeInteger(number) || number <= 0) return null;
  return {
    owner,
    repo,
    number,
    key: `${owner.toLowerCase()}/${repo.toLowerCase()}#${number}`,
    url: `https://github.com/${owner}/${repo}/pull/${number}`,
  };
}

/** `collectPullRequestTargets` が受け取るタスク(TaskCard の必要な部分だけ)。 */
interface TargetTaskLike {
  id: string;
  status: string;
  origin: string;
}

/** `collectPullRequestTargets` が受け取る関連リソース。 */
interface TargetResourceLike {
  taskId: string;
  kind: string;
  value: string;
}

/**
 * 全タスクの関連リソースから、監視対象の PR を組み立てる。
 *
 * - **Done のタスクは対象外**(片付いた PR を見張り続けない)。
 * - このプラグインが作った通知タスク (`origin === pluginOrigin`) も対象外。
 *   通知タスク自身が PR URL を主リソースに持つので、除かないと自己増殖の元になる。
 * - 同じ PR を複数のタスクが参照していても 1 件にまとめ、タスク id 昇順の先頭を親に使う。
 */
function collectPullRequestTargets(
  tasks: readonly TargetTaskLike[],
  resources: readonly TargetResourceLike[],
  pluginOrigin: string,
): PullRequestTarget[] {
  const watched = new Set<string>();
  for (const task of tasks) {
    if (task.status === "done") continue;
    if (task.origin === pluginOrigin) continue;
    watched.add(task.id);
  }

  const byKey = new Map<string, PullRequestTarget>();
  for (const resource of resources) {
    if (resource.kind !== "url") continue;
    if (!watched.has(resource.taskId)) continue;
    const ref = parsePullRequestUrl(resource.value);
    if (!ref) continue;
    const found = byKey.get(ref.key);
    if (found) {
      if (!found.taskIds.includes(resource.taskId)) found.taskIds.push(resource.taskId);
    } else {
      byKey.set(ref.key, { ref, taskIds: [resource.taskId] });
    }
  }

  const targets = [...byKey.values()];
  for (const target of targets) target.taskIds.sort();
  targets.sort((a, b) => (a.ref.key < b.ref.key ? -1 : a.ref.key > b.ref.key ? 1 : 0));
  return targets;
}

/* ------------------------------------------- description の自動記入(純関数) */

/** PR 本体を取りに行く REST API のパス(`GET /repos/{owner}/{repo}/pulls/{number}`)。 */
function pullRequestApiPath(ref: PullRequestRef): string {
  return `/repos/${ref.owner}/${ref.repo}/pulls/${ref.number}`;
}

/** `shouldFillDescription` / `collectDescriptionTargets` が受け取るタスク。 */
interface FillTaskLike {
  id: string;
  origin: string;
  description: string;
  /** ソフトデリート時刻。生きているタスクは `null`。 */
  deletedAt?: string | null;
}

/** description を記入する対象 1 件。 */
interface DescriptionTarget {
  taskId: string;
  ref: PullRequestRef;
}

/**
 * このタスクの description を PR の情報で埋めてよいか。
 *
 * **ユーザーが書いた文章は絶対に上書きしない**のが要件なので、条件は厳しくとる。
 *
 * - 既に何か書かれていれば触らない(空白だけなら空とみなす)。
 * - 一度記入したタスクは二度と触らない (`alreadyFilled`)。
 *   記入後にユーザーが消したとしても、それは「空にした」という意思表示なので埋め直さない。
 * - このプラグインが作った通知タスクは対象外(description は自分で組み立てている)。
 * - 削除済み(ソフトデリート)のタスクも対象外。
 *
 * Done のタスクは除いていない。過去の PR でも手掛かりが残る方が嬉しいため。
 */
function shouldFillDescription(
  task: FillTaskLike,
  pluginOrigin: string,
  alreadyFilled: boolean,
): boolean {
  if (alreadyFilled) return false;
  if (task.origin === pluginOrigin) return false;
  if (task.deletedAt) return false;
  return String(task.description ?? "").trim() === "";
}

/**
 * description を埋めるべきタスクと、その元になる PR を組み立てる。
 *
 * 1 タスクにつき 1 件(関連リソースの並び順で最初に見つかった PR)。戻りはタスク id の昇順。
 */
function collectDescriptionTargets(
  tasks: readonly FillTaskLike[],
  resources: readonly TargetResourceLike[],
  pluginOrigin: string,
  filledTaskIds: ReadonlySet<string>,
): DescriptionTarget[] {
  const eligible = new Set<string>();
  for (const task of tasks) {
    if (shouldFillDescription(task, pluginOrigin, filledTaskIds.has(task.id))) {
      eligible.add(task.id);
    }
  }

  const picked = new Map<string, DescriptionTarget>();
  for (const resource of resources) {
    if (resource.kind !== "url") continue;
    if (!eligible.has(resource.taskId) || picked.has(resource.taskId)) continue;
    const ref = parsePullRequestUrl(resource.value);
    if (!ref) continue;
    picked.set(resource.taskId, { taskId: resource.taskId, ref });
  }

  const targets = [...picked.values()];
  targets.sort((a, b) => (a.taskId < b.taskId ? -1 : a.taskId > b.taskId ? 1 : 0));
  return targets;
}

/** PR の状態を 1 語で表す(`merged` は `state` に出ないので `merged` フラグを優先する)。 */
function prStateLabel(pr: GhPullRequest): string {
  if (pr.merged === true) return "merged";
  const state = (pr.state ?? "").trim().toLowerCase();
  return state === "" ? "open" : state;
}

/** PR 本文を description に載せられる長さへ切り詰める。改行は LF にそろえる。 */
function truncateBody(body: string, limit: number = DESCRIPTION_BODY_LIMIT): string {
  const normalized = body.replace(/\r\n?/g, "\n").trim();
  if (normalized.length <= limit) return normalized;
  return `${normalized.slice(0, limit).trimEnd()}…`;
}

/**
 * PR の情報から description を組み立てる。
 *
 * ```
 * <PR タイトル>
 * <owner>/<repo>#<番号> (open) by <作者>
 *
 * <本文の先頭>
 * ```
 *
 * 本文が空なら 2 行目までで終わる。PR の URL は関連リソースに残っているので載せない。
 */
function buildPrDescription(pr: GhPullRequest, ref: PullRequestRef): string {
  const slug = `${ref.owner}/${ref.repo}#${ref.number}`;
  const title = (pr.title ?? "").trim() || slug;
  const author = (pr.user?.login ?? "").trim();
  const meta = `${slug} (${prStateLabel(pr)})${author ? ` by ${author}` : ""}`;
  const body = truncateBody(String(pr.body ?? ""));
  return body ? `${title}\n${meta}\n\n${body}` : `${title}\n${meta}`;
}

/** RFC 3339 文字列をミリ秒に直す。読めなければ `NaN`。 */
function toEpoch(value: string | null | undefined): number {
  if (!value) return Number.NaN;
  return Date.parse(value);
}

/**
 * 前回確認以降に増えたコメントだけを取り出す。
 *
 * - 自分 (`selfLogin`) のコメントは除く。
 * - 作成時刻が `sinceAt` より後、または同時刻で id が `sinceId` より大きいものを新規とみなす
 *   (GitHub の `since` は「更新時刻」基準なので、編集された古いコメントが混ざる)。
 * - 戻りは作成時刻→id の昇順。
 */
function selectNewComments(
  comments: readonly CommentInfo[],
  sinceAt: string | null,
  sinceId: number | null,
  selfLogin: string | null,
): CommentInfo[] {
  const border = toEpoch(sinceAt);
  const self = selfLogin ? selfLogin.toLowerCase() : null;
  const picked = comments.filter((comment) => {
    if (self && comment.login.toLowerCase() === self) return false;
    if (Number.isNaN(border)) return true;
    const at = toEpoch(comment.createdAt);
    if (Number.isNaN(at)) return false;
    if (at > border) return true;
    return at === border && sinceId != null && comment.id > sinceId;
  });
  picked.sort((a, b) => toEpoch(a.createdAt) - toEpoch(b.createdAt) || a.id - b.id);
  return picked;
}

/** check-runs のレスポンスを 4 段階に丸める。 */
function summarizeCheckRuns(runs: readonly GhCheckRun[]): CiSummary {
  const failed: string[] = [];
  let pending = false;
  let seen = false;
  for (const run of runs) {
    seen = true;
    if (run.status && run.status !== "completed") {
      pending = true;
      continue;
    }
    const conclusion = (run.conclusion ?? "").toLowerCase();
    if (conclusion === "") {
      // completed なのに conclusion が無いのは状態が確定していないということ。
      pending = true;
    } else if (FAILED_CONCLUSIONS.has(conclusion)) {
      failed.push(run.name?.trim() || conclusion);
    }
    // success / neutral / skipped は成功扱い。
  }
  if (failed.length > 0) return { state: "failure", failed };
  if (pending) return { state: "pending", failed: [] };
  return { state: seen ? "success" : "none", failed: [] };
}

/** combined status のレスポンスを 4 段階に丸める。 */
function summarizeCombinedStatus(status: GhCombinedStatus | null): CiSummary {
  if (!status) return { state: "none", failed: [] };
  const contexts = status.statuses ?? [];
  const failed = contexts
    .filter((item) => item.state === "failure" || item.state === "error")
    .map((item) => item.context?.trim() || "status");
  const state = (status.state ?? "").toLowerCase();
  if (state === "failure" || state === "error") {
    return { state: "failure", failed: failed.length > 0 ? failed : ["status"] };
  }
  if (state === "pending") return { state: "pending", failed: [] };
  if (state === "success") return { state: "success", failed: [] };
  return { state: contexts.length > 0 ? "pending" : "none", failed: [] };
}

/** 深刻な方を採る(failure > pending > success > none)。 */
function mergeCi(a: CiSummary | null, b: CiSummary | null): CiSummary {
  const order: Record<CiState, number> = { none: 0, success: 1, pending: 2, failure: 3 };
  const left = a ?? { state: "none" as CiState, failed: [] };
  const right = b ?? { state: "none" as CiState, failed: [] };
  const state = order[left.state] >= order[right.state] ? left.state : right.state;
  // 失敗ジョブ名は両方から集める(重複は落とす)。
  const failed = [...new Set([...left.failed, ...right.failed])];
  return { state, failed: state === "failure" ? failed : [] };
}

/**
 * CI 失敗の通知を出すべきか。
 *
 * 「失敗への**遷移**」だけを通知する。
 * head SHA が変わっていない同じ失敗を繰り返し通知しないのが目的。
 * head SHA が変わっていれば、前回も失敗だったとしても新しい失敗として扱う。
 * PR を初めて観測したとき (`ciSha === null`) に既に赤ければ通知する
 * (コメントと違い、赤いまま放置されている PR は最初に知りたいはずなので)。
 */
function shouldNotifyCiFailure(
  previous: Pick<PrState, "ciSha" | "ciNotified">,
  headSha: string | null,
  ci: CiSummary,
): boolean {
  if (ci.state !== "failure") return false;
  if (previous.ciSha !== null && headSha !== null && previous.ciSha !== headSha) return true;
  return previous.ciNotified !== "failure";
}

/** 通知タスクを「新規作成」するか「既存へ追記」するか。 */
type NoticeAction = { kind: "create" } | { kind: "append"; taskId: string };

/**
 * 直近の通知タスクが未完了で残っていれば追記、そうでなければ新規作成。
 *
 * `taskStatuses` はボード上の全タスクの状態(消えたタスクはキーごと無い)。
 */
function decideNoticeAction(
  noticeTaskId: string | null,
  taskStatuses: ReadonlyMap<string, string>,
): NoticeAction {
  if (!noticeTaskId) return { kind: "create" };
  const status = taskStatuses.get(noticeTaskId);
  if (status === undefined || status === "done") return { kind: "create" };
  return { kind: "append", taskId: noticeTaskId };
}

/** 通知の理由(description / アップデート本文)を組み立てる。 */
function buildReasons(newComments: readonly CommentInfo[], ci: CiSummary | null): string[] {
  const reasons: string[] = [];
  if (newComments.length > 0) {
    const review = newComments.filter((comment) => comment.kind === "review").length;
    const detail = review > 0 ? `(うちレビューコメント ${review} 件)` : "";
    reasons.push(`新しいコメントが ${newComments.length} 件${detail}`);
  }
  if (ci && ci.state === "failure") {
    const names = ci.failed.slice(0, 5).join(", ");
    reasons.push(names ? `CI が失敗: ${names}` : "CI が失敗");
  }
  return reasons;
}

/* ================================================================ 実行部分 */

/** KV のキーを作る。 */
function kvKey(ref: PullRequestRef): string {
  return `${PR_KEY_PREFIX}${ref.key}`;
}

/** まだ何も観測していない状態。 */
function emptyState(): PrState {
  return {
    initialized: false,
    lastCommentAt: null,
    lastCommentId: null,
    ciSha: null,
    ciChecks: null,
    ciCombined: null,
    ciNotified: null,
    noticeTaskId: null,
    etags: {},
  };
}

/** 例外を人が読める文字列にする。 */
function describeError(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

/** GitHub の `since` に渡せるよう、秒精度の RFC 3339 に丸める。 */
function toGitHubTimestamp(value: Date | string): string {
  const date = typeof value === "string" ? new Date(value) : value;
  return `${date.toISOString().slice(0, 19)}Z`;
}

/** 1 回のリクエストの結果。 */
interface GhResult<T> {
  /** 304 だったか(= 前回から変わっていない)。 */
  notModified: boolean;
  /** 本文。304 のときは `null`。 */
  data: T | null;
}

/** GitHub REST API を叩く小さなクライアント。 */
interface GhClient {
  /**
   * `path`(先頭 `/`)を GET する。`etags` に前回の ETag があれば
   * `If-None-Match` を付け、200 なら新しい ETag を書き戻す。
   *
   * @throws {RateLimitError} レート制限に当たった場合。
   * @throws {HttpError} その他の非 2xx。
   */
  get<T>(path: string, etags: Record<string, string>): Promise<GhResult<T>>;
}

/**
 * クライアントを作る。
 *
 * `pat` が空文字なら `Authorization` を付けずに投げる。認証なしでも public な PR は
 * 読めるので、description の自動記入は PAT 未設定でも動く(レート制限は厳しくなる)。
 */
function createClient(ctx: Ctx, pat: string): GhClient {
  return {
    async get<T>(path: string, etags: Record<string, string>): Promise<GhResult<T>> {
      const headers: Record<string, string> = {
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": API_VERSION,
      };
      if (pat) headers.Authorization = `Bearer ${pat}`;
      const known = etags[path];
      if (known) headers["If-None-Match"] = known;

      // ブラウザの HTTP キャッシュに介入されると 304 が 200 に化けるので、
      // 条件付きリクエストは自前の ETag だけで回す。
      const response = await ctx.fetch(`${API_BASE}${path}`, {
        method: "GET",
        headers,
        cache: "no-store",
      });

      if (response.status === 304) return { notModified: true, data: null };

      if (response.status === 403 || response.status === 429) {
        const remaining = response.headers.get("x-ratelimit-remaining");
        const reset = response.headers.get("x-ratelimit-reset");
        const retryAfter = response.headers.get("retry-after");
        if (remaining === "0" || retryAfter !== null) {
          const when = reset ? new Date(Number(reset) * 1000).toISOString() : "不明";
          throw new RateLimitError(
            `GitHub API のレート制限に達しました(リセット: ${when}${
              retryAfter ? `, Retry-After: ${retryAfter}s` : ""
            })。次回のポーリングまで待ちます。`,
          );
        }
        throw new HttpError(
          response.status,
          `GET ${path} が ${response.status} を返しました(権限不足の可能性)。`,
        );
      }

      if (!response.ok) {
        throw new HttpError(
          response.status,
          `GET ${path} が ${response.status} ${response.statusText} を返しました。`,
        );
      }

      const etag = response.headers.get("etag");
      if (etag) etags[path] = etag;
      else delete etags[path];

      return { notModified: false, data: (await response.json()) as T };
    },
  };
}

/**
 * 自分の login を取る(1 回だけ取って KV にキャッシュする)。
 *
 * 自分のコメントを新着から除くためだけに使うので、取れなくても続行する。
 */
async function loadSelfLogin(ctx: Ctx, client: GhClient): Promise<string | null> {
  const cached = await ctx.kv.get<string>(SELF_LOGIN_KEY);
  if (typeof cached === "string" && cached !== "") return cached;
  try {
    const result = await client.get<GhUser>("/user", {});
    const login = result.data?.login ?? null;
    if (login) {
      await ctx.kv.set(SELF_LOGIN_KEY, login);
      ctx.log.debug(`自分の login を ${login} として記録しました。`);
    }
    return login;
  } catch (error) {
    if (error instanceof RateLimitError) throw error;
    ctx.log.warn(
      `自分の login を取得できませんでした(自分のコメントも新着として扱います): ${describeError(error)}`,
    );
    return null;
  }
}

/** issue コメント/レビューコメントのレスポンスを比較用に正規化する。 */
function toCommentInfos(raw: readonly GhComment[], kind: "issue" | "review"): CommentInfo[] {
  const infos: CommentInfo[] = [];
  for (const comment of raw) {
    if (typeof comment.id !== "number" || !comment.created_at) continue;
    infos.push({
      id: comment.id,
      createdAt: comment.created_at,
      login: comment.user?.login ?? "",
      kind,
    });
  }
  return infos;
}

/** PR 1 件を確認して、必要なら通知タスクを作る/追記する。 */
async function pollOne(
  ctx: Ctx,
  client: GhClient,
  target: PullRequestTarget,
  taskStatuses: ReadonlyMap<string, string>,
  selfLogin: string | null,
): Promise<void> {
  const { ref } = target;
  const key = kvKey(ref);
  const stored = await ctx.kv.get<PrState>(key);
  const state: PrState = { ...emptyState(), ...(stored ?? {}) };
  if (!state.etags || typeof state.etags !== "object") state.etags = {};

  const base = `/repos/${ref.owner}/${ref.repo}`;
  // このラウンドで使ったパス。使わなくなった ETag はここを基準に捨てる。
  const usedPaths: string[] = [];

  /* --- 1. PR 本体(状態と head SHA) --------------------------------- */
  const prPath = `${base}/pulls/${ref.number}`;
  usedPaths.push(prPath);
  const pr = await client.get<GhPullRequest>(prPath, state.etags);
  let headSha = state.ciSha;
  if (!pr.notModified && pr.data) {
    const prState = (pr.data.state ?? "open").toLowerCase();
    if (prState !== "open") {
      // マージ/クローズ済みは見張る意味が無いので状態ごと捨てる。
      await ctx.kv.set(key, null);
      ctx.log(`${ref.key} は ${pr.data.merged ? "マージ" : "クローズ"}済みです。状態を破棄しました。`);
      return;
    }
    headSha = pr.data.head?.sha ?? null;
  }

  /* --- 2. 初回は現在値を記録するだけ(通知しない) -------------------- */
  if (!state.initialized) {
    state.initialized = true;
    // 過去のコメントを全部読み直さずに済むよう、「今」を起点にする。
    state.lastCommentAt = toGitHubTimestamp(new Date());
    state.lastCommentId = null;
  }

  /* --- 3. コメント ---------------------------------------------------- */
  // `since` があるので、ふつうは 0 件か数件しか返らない。URL が変わらない間は ETag が効く。
  const since = state.lastCommentAt ? `&since=${encodeURIComponent(state.lastCommentAt)}` : "";
  const issuePath = `${base}/issues/${ref.number}/comments?per_page=${PER_PAGE}${since}`;
  const reviewPath = `${base}/pulls/${ref.number}/comments?per_page=${PER_PAGE}${since}`;
  usedPaths.push(issuePath, reviewPath);

  const issueComments = await client.get<GhComment[]>(issuePath, state.etags);
  const reviewComments = await client.get<GhComment[]>(reviewPath, state.etags);
  const fetched: CommentInfo[] = [
    ...toCommentInfos(issueComments.data ?? [], "issue"),
    ...toCommentInfos(reviewComments.data ?? [], "review"),
  ];
  // 1 ページ目しか見ないので、100 件を超える更新は次回に回る(`since` が進むので取りこぼさない)。
  const newComments = selectNewComments(
    fetched,
    state.lastCommentAt,
    state.lastCommentId,
    selfLogin,
  );
  // 時刻を進めるときは自分のコメントも数に入れる(入れないと毎回同じものを取り直すことになる)。
  const advanced = selectNewComments(fetched, state.lastCommentAt, state.lastCommentId, null);

  /* --- 4. CI ---------------------------------------------------------- */
  let ci: CiSummary = { state: "none", failed: [] };
  if (headSha) {
    if (state.ciSha !== headSha) {
      // head が進んだら前回の判定は無効。通知済みフラグも解除して新しい失敗を拾えるようにする。
      state.ciChecks = null;
      state.ciCombined = null;
      state.ciNotified = null;
    }
    const checksPath = `${base}/commits/${headSha}/check-runs?per_page=${PER_PAGE}`;
    const statusPath = `${base}/commits/${headSha}/status?per_page=${PER_PAGE}`;
    usedPaths.push(checksPath, statusPath);

    const checks = await client.get<GhCheckRuns>(checksPath, state.etags);
    if (!checks.notModified) {
      state.ciChecks = summarizeCheckRuns(checks.data?.check_runs ?? []);
    }
    const combined = await client.get<GhCombinedStatus>(statusPath, state.etags);
    if (!combined.notModified) {
      state.ciCombined = summarizeCombinedStatus(combined.data);
    }
    ci = mergeCi(state.ciChecks, state.ciCombined);
  }

  const notifyCi = shouldNotifyCiFailure(state, headSha, ci);
  state.ciSha = headSha;
  // 復旧したら通知済みフラグを解除する(次に落ちたときまた通知できるように)。
  if (ci.state !== "failure") state.ciNotified = null;

  /* --- 5. 通知 -------------------------------------------------------- */
  const reasons = buildReasons(newComments, notifyCi ? ci : null);
  if (reasons.length > 0) {
    const body = `${ref.owner}/${ref.repo}#${ref.number}: ${reasons.join(" / ")}`;
    const action = decideNoticeAction(state.noticeTaskId, taskStatuses);
    if (action.kind === "append") {
      await ctx.tasks.addTaskUpdate(action.taskId, body);
      ctx.log(`既存の通知タスクへ追記しました: ${body}`);
    } else {
      const created = await ctx.tasks.createTask({
        title: `PR を確認する: ${ref.owner}/${ref.repo}#${ref.number}`,
        description: `${reasons.join("\n")}\n\n${ref.url}`,
        status: "new",
        isInstant: true,
        parentId: target.taskIds[0] ?? null,
        resources: [
          {
            kind: "url",
            value: ref.url,
            label: `${ref.owner}/${ref.repo}#${ref.number}`,
            isPrimary: true,
          },
        ],
      });
      state.noticeTaskId = created.id;
      ctx.log(`通知タスクを作成しました: ${body}`);
    }
    if (notifyCi) state.ciNotified = "failure";
  }

  /* --- 6. 状態の保存 -------------------------------------------------- */
  const newest = advanced[advanced.length - 1];
  if (newest) {
    state.lastCommentAt = toGitHubTimestamp(newest.createdAt);
    state.lastCommentId = newest.id;
  }
  // 今回使わなかったパスの ETag(古い `since` 付き URL など)は捨てる。
  const kept: Record<string, string> = {};
  for (const path of usedPaths) {
    const etag = state.etags[path];
    if (etag) kept[path] = etag;
  }
  state.etags = kept;
  await ctx.kv.set(key, state);
}

/** どのタスクからも参照されなくなった PR の状態を KV から捨てる。 */
async function pruneStates(ctx: Ctx, targets: readonly PullRequestTarget[]): Promise<void> {
  const alive = new Set(targets.map((target) => kvKey(target.ref)));
  let keys: string[];
  try {
    keys = await ctx.kv.keys();
  } catch (error) {
    ctx.log.warn(`KV のキーを列挙できませんでした: ${describeError(error)}`);
    return;
  }
  for (const key of keys) {
    if (!key.startsWith(PR_KEY_PREFIX) || alive.has(key)) continue;
    await ctx.kv.set(key, null);
    ctx.log.debug(`参照されなくなった状態を破棄しました: ${key}`);
  }
}

/* ------------------------------------------- description の自動記入(実行部) */

/** KV に残す「記入済み」の記録。 */
interface DescriptionRecord {
  /** 記入元の PR (`owner/repo#番号`)。 */
  pr: string;
  /** 記入した時刻 (RFC 3339)。 */
  at: string;
}

/** 記入済み記録の KV キー。 */
function descKey(taskId: string): string {
  return `${DESC_KEY_PREFIX}${taskId}`;
}

/**
 * 記入済みのタスク id を読む。
 *
 * 読めなかったときは `null` を返す。**「まだ記入していない」と誤認して
 * ユーザーの文章を上書きするより、そのラウンドを諦める方が安全**なため。
 */
async function loadFilledTaskIds(ctx: Ctx): Promise<Set<string> | null> {
  try {
    const keys = await ctx.kv.keys();
    return new Set(
      keys
        .filter((key) => key.startsWith(DESC_KEY_PREFIX))
        .map((key) => key.slice(DESC_KEY_PREFIX.length)),
    );
  } catch (error) {
    ctx.log.warn(`記入済みの記録を読めませんでした: ${describeError(error)}`);
    return null;
  }
}

/** ボードから消えたタスクの記入済み記録を捨てる。 */
async function pruneDescriptionRecords(
  ctx: Ctx,
  tasks: readonly { id: string }[],
): Promise<void> {
  const alive = new Set(tasks.map((task) => task.id));
  let keys: string[];
  try {
    keys = await ctx.kv.keys();
  } catch {
    return; // 列挙できないだけなら次回に回す(loadFilledTaskIds が既に警告している)。
  }
  for (const key of keys) {
    if (!key.startsWith(DESC_KEY_PREFIX)) continue;
    if (alive.has(key.slice(DESC_KEY_PREFIX.length))) continue;
    await ctx.kv.set(key, null);
    ctx.log.debug(`消えたタスクの記入済み記録を破棄しました: ${key}`);
  }
}

/**
 * description が空のタスクを PR の情報で埋める。
 *
 * PR 監視と違って **PAT が無くても動く**(認証なしで public な PR は読める)ので、
 * ポーリング本体より前に、PAT の有無と関係なく走らせる。
 *
 * @param prune ボードから消えたタスクの記録も掃除するか(ポーリングのときだけ真)。
 */
async function fillDescriptions(
  ctx: Ctx,
  settings: Record<string, unknown>,
  prune: boolean,
): Promise<void> {
  if (settings.enabled === false) return;

  const tasks = await ctx.tasks.listTasks();
  if (prune) await pruneDescriptionRecords(ctx, tasks);

  const filled = await loadFilledTaskIds(ctx);
  if (!filled) return;

  const pluginOrigin = `plugin:${ctx.manifest.id}`;
  const resources = await ctx.tasks.listAllResources();
  const targets = collectDescriptionTargets(tasks, resources, pluginOrigin, filled);
  if (targets.length === 0) return;

  const client = createClient(ctx, String(settings.pat ?? "").trim());
  for (const target of targets) {
    try {
      // ETag は使わない(1 タスクにつき 1 回きりの取得なので使い回す相手がいない)。
      const result = await client.get<GhPullRequest>(pullRequestApiPath(target.ref), {});
      if (!result.data) continue;
      await ctx.tasks.updateTask(target.taskId, {
        description: buildPrDescription(result.data, target.ref),
      });
      const record: DescriptionRecord = { pr: target.ref.key, at: new Date().toISOString() };
      await ctx.kv.set(descKey(target.taskId), record);
      ctx.log(`タスク ${target.taskId} の詳細を ${target.ref.key} から記入しました。`);
    } catch (error) {
      if (error instanceof RateLimitError) {
        ctx.log.warn(`${error.message} (残りの description 記入は中断しました)`);
        return;
      }
      if (error instanceof HttpError && (error.status === 403 || error.status === 404)) {
        // private / 削除済み / 認証なしでは見えない PR。触れないだけなので静かに流す。
        ctx.log.debug(`${target.ref.key} は取得できませんでした (${error.status})。`);
        continue;
      }
      ctx.log.warn(`${target.ref.key} の PR 情報を取得できませんでした: ${describeError(error)}`);
    }
  }
}

/** ポーリング 1 ラウンド。 */
async function poll(ctx: Ctx): Promise<void> {
  const settings = await ctx.settings.get();
  if (settings.enabled === false) {
    ctx.log.debug("無効化されているためスキップします。");
    return;
  }

  // イベントを取りこぼしていた分をここで拾う。PAT の有無に関係なく走らせる。
  // ここで失敗しても PR の監視は続ける(2 つの機能は互いに独立)。
  try {
    await fillDescriptions(ctx, settings, true);
  } catch (error) {
    ctx.log.warn(`description の自動記入に失敗しました: ${describeError(error)}`);
  }

  const pat = String(settings.pat ?? "").trim();
  if (!pat) {
    ctx.log("PAT が未設定のためスキップします(設定画面のプラグイン節で設定してください)。");
    return;
  }

  const tasks = await ctx.tasks.listTasks();
  const resources = await ctx.tasks.listAllResources();
  const pluginOrigin = `plugin:${ctx.manifest.id}`;
  const targets = collectPullRequestTargets(tasks, resources, pluginOrigin);
  const taskStatuses = new Map(tasks.map((task) => [task.id, task.status] as const));

  await pruneStates(ctx, targets);

  if (targets.length === 0) {
    ctx.log.debug("監視対象の PR はありません。");
    return;
  }

  const client = createClient(ctx, pat);
  let selfLogin: string | null;
  try {
    selfLogin = await loadSelfLogin(ctx, client);
  } catch (error) {
    ctx.log.warn(describeError(error));
    return;
  }

  ctx.log.debug(`${targets.length} 件の PR を確認します。`);
  for (const target of targets) {
    try {
      // PR 数が多くても直列でよい(レート制限にやさしく、失敗も 1 件で閉じ込められる)。
      await pollOne(ctx, client, target, taskStatuses, selfLogin);
    } catch (error) {
      if (error instanceof RateLimitError) {
        ctx.log.warn(`${error.message} (残り ${targets.length} 件の確認は中断しました)`);
        return;
      }
      ctx.log.warn(`${target.ref.key} の確認に失敗しました: ${describeError(error)}`);
    }
  }
}

/* ============================================================== プラグイン */

export default defineQuestloomPlugin({
  manifest: {
    id: "github",
    name: "GitHub 統合",
    version: "0.1.0",
    description:
      "未完了タスクに紐づいた GitHub PR を監視し、新しいコメントや CI 失敗を検知したら" +
      "「PR を確認する」子タスクを New に作る。PR の URL が付いたタスクの詳細が空なら、" +
      "PR のタイトル・状態・本文の先頭を自動で書き込む(こちらは PAT 不要)。",
    // ctx.fetch はここに書いたホストにしか出られない(完全一致)。
    fetchDomains: ["api.github.com"],
    settingsSchema: [
      {
        key: "pat",
        label: "Personal Access Token",
        type: "secret",
        default: "",
        hint:
          "PR を読める最小限の権限で発行すること。現状は DB に平文で保存される。" +
          "未設定でも description の自動記入(public な PR のみ)は動く。",
      },
      {
        key: "pollIntervalMinutes",
        label: "ポーリング間隔(分)",
        type: "number",
        default: DEFAULT_INTERVAL_MINUTES,
        hint: "短くしすぎると GitHub のレート制限に当たる。PR 1 件あたり 1 回 5 リクエスト程度。",
      },
      {
        key: "enabled",
        label: "有効",
        type: "boolean",
        default: true,
      },
    ],
  },

  async activate(ctx) {
    /** 実行中フラグ。前回のポーリングが終わる前に次を走らせない。 */
    let running = false;
    /** 現在のスケジュールを止める関数。設定変更で張り直すために持つ。 */
    let stopSchedule: (() => void) | null = null;

    const runOnce = async (): Promise<void> => {
      if (running) {
        ctx.log.debug("前回のポーリングが終わっていないので今回は見送ります。");
        return;
      }
      running = true;
      try {
        await poll(ctx);
      } finally {
        running = false;
      }
    };

    /** 設定に合わせてスケジュールを張り直す(登録直後に 1 回走る)。 */
    const reschedule = (settings: Record<string, unknown>): void => {
      stopSchedule?.();
      stopSchedule = null;
      if (settings.enabled === false) {
        ctx.log("無効化されています。ポーリングは行いません。");
        return;
      }
      const raw = Number(settings.pollIntervalMinutes);
      const interval = Number.isFinite(raw) && raw > 0 ? raw : DEFAULT_INTERVAL_MINUTES;
      ctx.log(`${interval} 分ごとに PR を確認します。`);
      stopSchedule = ctx.schedule(interval, runOnce);
    };

    /* --- description の自動記入(タスクイベント駆動) ------------------- */

    /** 記入処理が走っている間は真。自分の更新で再入するのを防ぐ。 */
    let filling = false;
    /** 記入中に届いたイベント。終わったらもう一度だけ回す。 */
    let fillAgain = false;
    /** デバウンス用のタイマー(webview では number だが、環境差を吸収しておく)。 */
    let fillTimer: ReturnType<typeof setTimeout> | null = null;

    const fillNow = async (): Promise<void> => {
      if (filling) {
        fillAgain = true;
        return;
      }
      filling = true;
      try {
        await fillDescriptions(ctx, await ctx.settings.get(), false);
      } catch (error) {
        ctx.log.warn(`description の自動記入に失敗しました: ${describeError(error)}`);
      } finally {
        filling = false;
      }
      if (fillAgain) {
        fillAgain = false;
        scheduleFill();
      }
    };

    /** 少し待ってから記入を走らせる(連続する変更をまとめるため)。 */
    function scheduleFill(): void {
      if (fillTimer !== null) clearTimeout(fillTimer);
      fillTimer = setTimeout(() => {
        fillTimer = null;
        void fillNow();
      }, FILL_DEBOUNCE_MS);
    }

    // リソースが付いた直後に反応する。取りこぼしはポーリングが拾う。
    const offTasks = ctx.onTaskEvent(scheduleFill);

    const offSettings = ctx.settings.onChange((next) => {
      // PAT が差し替わった可能性があるので、自分の login のキャッシュは捨てる。
      void ctx.kv.set(SELF_LOGIN_KEY, null).catch((error: unknown) => {
        ctx.log.warn(`login キャッシュを消せませんでした: ${describeError(error)}`);
      });
      reschedule(next);
    });

    reschedule(await ctx.settings.get());

    return () => {
      offSettings();
      offTasks();
      if (fillTimer !== null) clearTimeout(fillTimer);
      stopSchedule?.();
      ctx.log("GitHub プラグインを停止しました。");
    };
  },
});

/* ------------------------------------------------------------------ テスト用 */

// ホストは default export しか見ないので、名前付き export を足しても動作には影響しない。
// `examples/plugins/github.test.mjs` がここを検証する。
export {
  buildPrDescription,
  buildReasons,
  collectDescriptionTargets,
  collectPullRequestTargets,
  decideNoticeAction,
  mergeCi,
  parsePullRequestUrl,
  pullRequestApiPath,
  selectNewComments,
  shouldFillDescription,
  shouldNotifyCiFailure,
  summarizeCheckRuns,
  summarizeCombinedStatus,
  truncateBody,
};
