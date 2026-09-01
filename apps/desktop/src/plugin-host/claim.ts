/**
 * プラグイン id の重複裁定。
 *
 * ソースは Rust 側 (`plugin_host::merge_sources`) が
 * **利用者配置 → アプリ同梱**の順、つまりロード順 = 優先順で返す。ホストは
 * 先に来た方に id を確保させるだけで、「同じ id ならユーザーのカスタマイズ版が
 * 標準プラグインに勝つ」という規則を表現する。
 *
 * ホスト本体 ([`./host`]) から切り出してあるのは、host.ts が esbuild-wasm を
 * 引き込む(= jsdom では import すらできない)ため。ここだけなら純関数として
 * テストできる。
 */

import type { LoadedPlugin } from "../types";

/**
 * id の重複を裁いた結果。
 *
 * - `claimed` — この id は初出。ロードを続けてよい。
 * - `shadowed` — 同じ id をユーザー配置版が既に確保している同梱版。エラーではなく
 *   「利用者がカスタマイズ版を置いた」という正常な状態なので、静かに読み飛ばして
 *   勝った側に `shadowsBuiltin` を立てる。
 * - `duplicate` — それ以外の重複(同じ置き場に同じ id が 2 つ等)。設定・KV を
 *   共有してしまうため、後から来た方をエラーとして拒否する。
 */
export type ClaimResult = "claimed" | "shadowed" | "duplicate";

/**
 * プラグイン id を台帳へ確保できるか判定する。
 *
 * 負けたのが同梱版であれば、勝ったユーザー版に `shadowsBuiltin` を立てる
 * (設定画面が「標準版を上書きしています」と出す)。
 *
 * @param claimed id → その id を確保しているロード結果。勝った側を書き換えるので、
 *   呼び出し側が結果一覧に載せている**同じオブジェクト**を渡すこと。
 * @param id 判定するプラグイン id。
 * @param incoming これから読み込もうとしているソースの出所。
 */
export function claimPluginId(
  claimed: Map<string, LoadedPlugin>,
  id: string,
  incoming: { builtin: boolean },
): ClaimResult {
  const winner = claimed.get(id);
  if (!winner) return "claimed";
  if (incoming.builtin && !winner.builtin) {
    winner.shadowsBuiltin = true;
    return "shadowed";
  }
  return "duplicate";
}
