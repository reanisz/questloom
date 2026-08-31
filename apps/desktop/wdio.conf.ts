/**
 * GUI e2e (WebdriverIO + @wdio/tauri-service) の設定。
 *
 * 対象は **アセット同梱の debug ビルド** (`npm run tauri build -- --debug --no-bundle` が作る
 * `target/debug/questloom-desktop.exe`)。`cargo run` / `tauri dev` の exe は devUrl を
 * 読みに行くので使えない。
 *
 * driver は `driverProvider: "external"`(= cargo でいれた `tauri-driver` + msedgedriver)。
 * 既定の `"embedded"` はアプリ側に `tauri-plugin-wdio-webdriver` を組み込む必要があるため使わない。
 * Windows の msedgedriver は WebView2 の版に合わせてサービスが自動 DL する
 * (`autoDownloadEdgeDriver`、既定 true)。
 *
 * **本物のデータを触らないこと**が最優先なので、起動する exe には
 * `QUESTLOOM_DATA_DIR`(実行ごとの一時ディレクトリ)と `QUESTLOOM_MCP_PORT`(空きポート)を
 * 必ず渡す。これで `%APPDATA%\dev.reanisz.questloom` と 39150 には一切触らない。
 */

import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { homedir, tmpdir } from "node:os";
import { delimiter, join, resolve } from "node:path";

/** 空いている TCP ポートを 1 つ borrow する(閉じてから使うので厳密には競合しうるが実用上十分)。 */
function findFreePort(): Promise<number> {
  return new Promise((ok, ng) => {
    const server = createServer();
    server.unref();
    server.on("error", ng);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (address === null || typeof address === "string") {
        server.close(() => ng(new Error("空きポートを取得できませんでした")));
        return;
      }
      const { port } = address;
      server.close(() => ok(port));
    });
  });
}

/**
 * 実行ごとの一時プロファイルと MCP ポート。
 *
 * wdio は launcher とワーカーの両方でこのファイルを読み込むので、決めた値を
 * `process.env` に書き戻して共有する(ワーカーは launcher の env を継承する)。
 * こうしないとワーカー側が別の一時ディレクトリを作ってしまう。
 */
const dataDir =
  process.env.QUESTLOOM_E2E_DATA_DIR ?? mkdtempSync(join(tmpdir(), "questloom-e2e-"));
const mcpPort = process.env.QUESTLOOM_E2E_MCP_PORT ?? String(await findFreePort());
process.env.QUESTLOOM_E2E_DATA_DIR = dataDir;
process.env.QUESTLOOM_E2E_MCP_PORT = mcpPort;

/** 起動するアプリに渡す上書き。tauri-driver 経由で exe まで継承される。 */
const appEnv = { QUESTLOOM_DATA_DIR: dataDir, QUESTLOOM_MCP_PORT: mcpPort };

// tauri-driver は `cargo install` で `~/.cargo/bin` に入るが、そこが PATH に無いシェルからも
// 走らせたいので明示的に足しておく(msedgedriver をサービスが PATH 経由で見つける都合上、
// PATH ごと渡すのが素直)。
const cargoBin = join(homedir(), ".cargo", "bin");
if (!(process.env.PATH ?? "").split(delimiter).includes(cargoBin)) {
  process.env.PATH = `${cargoBin}${delimiter}${process.env.PATH ?? ""}`;
}

// exe は Cargo workspace のルート(= apps/desktop から 2 つ上)の target/ にできる。
const appBinaryPath = resolve(import.meta.dirname, "../../target/debug/questloom-desktop.exe");

// ---- 後始末(プロセスの取りこぼし) ----

/**
 * 走り終わっても残りうるプロセスのイメージ名。
 *
 * @wdio/tauri-service は Windows で driver を `shell: true` で spawn する(= 掴んでいる pid が
 * cmd.exe のもの)ため、後片付けの `taskkill /T` が tauri-driver 本体まで届かず、
 * tauri-driver と msedgedriver が居残る。実行のたびに積み上がるので自分で片付ける。
 */
const STRAY_IMAGES = ["tauri-driver.exe", "msedgedriver.exe", "questloom-desktop.exe"];

/** イメージ名から動作中の pid を引く。 */
function pidsOf(image: string): number[] {
  try {
    const csv = execFileSync("tasklist", ["/FI", `IMAGENAME eq ${image}`, "/FO", "CSV", "/NH"], {
      encoding: "utf8",
      windowsHide: true,
    });
    return csv
      .split("\n")
      .map((line) => /^"[^"]*","(\d+)"/.exec(line.trim())?.[1])
      .filter((pid): pid is string => pid !== undefined)
      .map(Number);
  } catch {
    // 該当なしのときは tasklist が非 0 で終わることがある。
    return [];
  }
}

/**
 * 開始前から居たプロセス。**これらは絶対に殺さない**
 * (利用者が別途動かしている questloom 本体や、他のテストの driver を巻き込まないため)。
 */
const preexistingPids =
  process.platform === "win32" ? new Set(STRAY_IMAGES.flatMap(pidsOf)) : new Set<number>();

export const config: WebdriverIO.Config = {
  runner: "local",
  specs: ["./e2e/**/*.spec.ts"],

  // アプリを 1 つだけ起動する。並列に上げるとウィンドウの取り合いになる。
  maxInstances: 1,

  capabilities: [
    {
      browserName: "tauri",
      "tauri:options": { application: appBinaryPath },
    } as WebdriverIO.Capabilities,
  ],

  services: [
    [
      "@wdio/tauri-service",
      {
        appBinaryPath,
        driverProvider: "external",
        // 未導入なら cargo install する(初回は数分)。導入済みなら何もしない。
        autoInstallTauriDriver: true,
        autoDownloadEdgeDriver: true,
        // tauri-driver に渡した env は、そこから spawn される exe に継承される。
        // **この 2 つだけ**にすること。サービス側が `{ ...process.env, ...env }` で混ぜるので、
        // ここに PATH を載せると msedgedriver を自動 DL した後の PATH 追記を上書きしてしまい、
        // tauri-driver が native driver を見つけられず exit code 1 で即死する。
        env: appEnv,
        // 起動失敗の原因が見えるようアプリ側のログを拾う。
        captureBackendLogs: true,
        // WebView2 の初回起動とウィンドウ 3 枚の生成があるので既定 30s では足りないことがある。
        startTimeout: 120_000,
      },
    ],
  ],

  logLevel: "warn",
  bail: 0,
  waitforTimeout: 15_000,
  connectionRetryTimeout: 120_000,
  connectionRetryCount: 3,

  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: { ui: "bdd", timeout: 120_000 },

  /**
   * 居残ったプロセスを片付けてから一時プロファイルを消す。
   *
   * 順序が大事。exe が生きているうちは SQLite のファイルハンドルが開いていて、
   * Windows ではディレクトリを消せない。
   */
  onComplete() {
    if (process.platform === "win32") {
      for (const image of STRAY_IMAGES) {
        for (const pid of pidsOf(image)) {
          if (preexistingPids.has(pid)) continue;
          try {
            execFileSync("taskkill", ["/PID", String(pid), "/T", "/F"], {
              stdio: "ignore",
              windowsHide: true,
            });
          } catch {
            // 既に終わっていたなら何もしなくてよい。
          }
        }
      }
    }
    rmSync(dataDir, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  },
};
