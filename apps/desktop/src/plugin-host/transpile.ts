/**
 * プラグインソースのトランスパイル。
 *
 * esbuild-wasm の `transform` を使い、TypeScript を ESM の JavaScript にするだけ。
 * **バンドル(モジュール解決)は行わない**ため、プラグインは 1 ファイルで完結させる必要がある。
 *
 * wasm はアプリのアセットとして同梱する(CDN は参照しない)。
 */

import * as esbuild from "esbuild-wasm";
// Vite が wasm をアセットとして出力し、その URL を埋め込む。
import wasmUrl from "esbuild-wasm/esbuild.wasm?url";

/** 初期化は 1 回だけ。並行呼び出しでも同じ Promise を共有する。 */
let initialized: Promise<void> | null = null;

/** esbuild-wasm を初期化する(冪等)。 */
export function initTranspiler(): Promise<void> {
  initialized ??= esbuild
    .initialize({ wasmURL: wasmUrl })
    .catch((error: unknown) => {
      // 失敗したら次回に再試行できるよう、キャッシュを捨てる。
      initialized = null;
      throw error;
    });
  return initialized;
}

/**
 * プラグインソースを ESM の JavaScript へ変換する。
 *
 * @param fileName 診断メッセージに出すファイル名。拡張子で loader を選ぶ。
 * @param source ソース本文。
 * @returns 変換後の JavaScript。
 * @throws 構文エラーなど、esbuild が変換に失敗した場合。
 */
export async function transpilePlugin(fileName: string, source: string): Promise<string> {
  await initTranspiler();
  const result = await esbuild.transform(source, {
    loader: fileName.toLowerCase().endsWith(".js") ? "js" : "ts",
    format: "esm",
    target: "es2022",
    sourcefile: fileName,
    // 装飾子やクラスフィールドは素直な JS へ落とす。
    tsconfigRaw: { compilerOptions: { useDefineForClassFields: true } },
  });
  for (const warning of result.warnings) {
    console.warn(`[questloom] ${fileName}: ${warning.text}`);
  }
  return result.code;
}
