/**
 * Tauri command 呼び出しの土台。
 *
 * フロント全体で `invoke` を直接触るのはこのモジュールだけにする。
 * バックエンドは日本語の文字列で reject するため、ここで Error へ正規化して
 * 「どこで受けても `error.message` が読める」状態に揃える。
 */

import { invoke } from "@tauri-apps/api/core";

/**
 * reject 値(多くは日本語文字列)を人が読めるメッセージへ正規化する。
 *
 * 文字列はそのまま、Error はその `message`、それ以外は `String()` で落とす。
 */
export function toMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

/**
 * Tauri command を呼ぶ。
 *
 * 引数は camelCase で渡す(Tauri v2 が snake_case へ変換する)。
 * reject 値は [`toMessage`] で正規化して Error に包み直す。
 */
export async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw new Error(toMessage(error));
  }
}
