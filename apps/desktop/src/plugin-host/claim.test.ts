/**
 * ホストの id 重複裁定([`claimPluginId`])のテスト。
 *
 * ソースは Rust 側 (`plugin_host::merge_sources`) が
 * **ユーザー配置 → アプリ同梱**の順で返す。ホストは先に来た方に id を確保させる
 * だけなので、「同じ id ならユーザー版が勝つ」という規則はこの 2 つの組で決まる。
 * ロード本体 (`host.ts` の `load`) は Blob URL からの動的 import と esbuild-wasm に
 * 依存していて jsdom では動かせないため、裁定だけを純関数として切り出してある。
 */

import { describe, expect, it } from "vitest";

import type { LoadedPlugin } from "../types";
import { claimPluginId } from "./claim";

/** 台帳に載っている勝者 1 件。 */
function winner(fileName: string, builtin: boolean): LoadedPlugin {
  return { fileName, manifest: null, active: true, error: null, builtin, shadowsBuiltin: false };
}

describe("claimPluginId", () => {
  it("初出の id はそのまま確保する", () => {
    const claimed = new Map<string, LoadedPlugin>();
    expect(claimPluginId(claimed, "github", { builtin: true })).toBe("claimed");
  });

  it("ユーザー版が先に確保した id は、同梱版を隠す", () => {
    const user = winner("github.ts", false);
    const claimed = new Map([["github", user]]);

    expect(claimPluginId(claimed, "github", { builtin: true })).toBe("shadowed");
    // 勝った側に印が付く(設定画面が「同梱版を上書きしています」を出す)。
    expect(user.shadowsBuiltin).toBe(true);
  });

  it("ファイル名が違ってもユーザー版が勝つ(判定は id だけで行う)", () => {
    const user = winner("my-github.ts", false);
    const claimed = new Map([["github", user]]);

    expect(claimPluginId(claimed, "github", { builtin: true })).toBe("shadowed");
    expect(user.shadowsBuiltin).toBe(true);
  });

  it("ユーザー配置どうしの重複は従来どおりエラー扱い", () => {
    const first = winner("a.ts", false);
    const claimed = new Map([["github", first]]);

    expect(claimPluginId(claimed, "github", { builtin: false })).toBe("duplicate");
    expect(first.shadowsBuiltin).toBe(false);
  });

  it("同梱が先に確保していたらユーザー版でも重複扱い", () => {
    // Rust 側がユーザー配置を先に返す限り起きないが、順序が崩れたときに
    // 黙って二重起動しないことを固定しておく。
    const builtin = winner("github.ts", true);
    const claimed = new Map([["github", builtin]]);

    expect(claimPluginId(claimed, "github", { builtin: false })).toBe("duplicate");
    expect(builtin.shadowsBuiltin).toBe(false);
  });
});
