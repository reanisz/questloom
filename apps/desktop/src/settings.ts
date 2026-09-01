/**
 * 設定画面のフォーム状態(ドラフト)と、その検証。
 *
 * 数値・引数配列は「編集途中の文字列」をそのまま保持し、保存直前にだけ
 * [`CoreSettings`] へ変換する。こうしないと入力中の空文字列が 0 や NaN に
 * 化けて、桁の打ち直しができなくなる。
 *
 * 検証規則は `apps/desktop/src-tauri/src/settings.rs::validate` と揃えてある
 * (ショートカット文字列のパースだけはバックエンド専任)。
 */

import type { AiProvider, CoreSettings, UrlOpenMode, WeekStart } from "./types";

/**
 * 設定画面の節。
 *
 * `plugins` だけはコア設定 (`CoreSettings`) と無関係で、保存も独立している
 * (プラグインごとの `plugin_set_settings`)。そのためドラフト・検証の対象外。
 */
export type SectionKey = "general" | "shortcut" | "mcp" | "ai" | "plugins";

/** 節の表示順とラベル。 */
export const SECTIONS: readonly { key: SectionKey; label: string; hint: string }[] = [
  { key: "general", label: "一般", hint: "週の始まり・自動起動・バックアップ" },
  { key: "shortcut", label: "ショートカットとオーバーレイ", hint: "呼び出しキーと通知表示" },
  { key: "mcp", label: "MCP サーバー", hint: "AI エージェントからの接続" },
  { key: "ai", label: "AI", hint: "呼び出す CLI プロバイダ" },
  { key: "plugins", label: "プラグイン", hint: "TypeScript プラグインと個別設定" },
];

/** MCP ポートの許容範囲(バックエンドと同じ)。 */
export const MCP_PORT_RANGE = { min: 1024, max: 65535 } as const;

/** AI タイムアウトの許容範囲(秒。バックエンドと同じ)。 */
export const AI_TIMEOUT_RANGE = { min: 10, max: 3600 } as const;

/** プロバイダ 1 件のフォーム状態。`args` だけ編集しやすい文字列で持つ。 */
export interface ProviderDraft {
  id: string;
  label: string;
  command: string;
  /** スペース区切り(`"` で括ればスペースを含められる)または JSON 配列。 */
  argsText: string;
  enabled: boolean;
  /** 画面では編集しないが、保存時に失わないよう保持する。 */
  mcpArgs: string[];
  /** 同上。 */
  mcpSupportsToken: boolean;
}

/** 設定画面のフォーム状態。 */
export interface SettingsDraft {
  weekStart: WeekStart;
  backupGenerations: string;
  overlayEnabled: boolean;
  globalShortcut: string;
  autostart: boolean;
  urlOpenMode: UrlOpenMode;
  mcpEnabled: boolean;
  mcpPort: string;
  /*
   * MCP のトークンはドラフトに載せない。実体は OS の資格情報ストアにあり、
   * 保存も「保存」ボタンとは独立に `set_mcp_token` で即座に行う
   * (`components/McpSection.tsx`)。
   */
  aiProviders: ProviderDraft[];
  aiDefaultProviderId: string;
  aiTimeoutSecs: string;
}

/** 検証で見つかった問題。`section` は該当する節へ誘導するために使う。 */
export interface SettingsIssue {
  section: SectionKey;
  message: string;
}

/**
 * 引数テンプレートを 1 行のテキストへ落とす。
 *
 * スペースや `"` を含む要素は `"` で括る(空要素も `""` として残す)。
 */
export function formatArgs(args: readonly string[]): string {
  return args
    .map((arg) => (arg === "" || /[\s"]/.test(arg) ? `"${arg.replace(/"/g, '""')}"` : arg))
    .join(" ");
}

/**
 * 引数テキストを配列へ戻す。解釈できない場合は `null`。
 *
 * `[` で始まるものは JSON の文字列配列として読む。それ以外はスペース区切りで、
 * `"` で括った範囲はひとつの引数として扱う(括り内の `""` はリテラルの `"`)。
 */
export function parseArgs(text: string): string[] | null {
  const trimmed = text.trim();
  if (trimmed === "") return [];

  if (trimmed.startsWith("[")) {
    try {
      const parsed: unknown = JSON.parse(trimmed);
      if (Array.isArray(parsed) && parsed.every((item) => typeof item === "string")) {
        return parsed as string[];
      }
    } catch {
      // 下のスペース区切りへは落とさない。JSON のつもりなら JSON として直させる。
    }
    return null;
  }

  const args: string[] = [];
  let current = "";
  let started = false;
  let quoted = false;
  for (let index = 0; index < trimmed.length; index += 1) {
    const char = trimmed[index];
    if (char === '"') {
      if (quoted && trimmed[index + 1] === '"') {
        current += '"';
        index += 1;
      } else {
        quoted = !quoted;
      }
      started = true;
      continue;
    }
    if (!quoted && /\s/.test(char)) {
      if (started) {
        args.push(current);
        current = "";
        started = false;
      }
      continue;
    }
    current += char;
    started = true;
  }
  if (quoted) return null;
  if (started) args.push(current);
  return args;
}

/** 保存済みの設定をフォーム状態へ変換する。 */
export function toDraft(settings: CoreSettings): SettingsDraft {
  return {
    weekStart: settings.weekStart,
    backupGenerations: String(settings.backupGenerations),
    overlayEnabled: settings.overlayEnabled,
    globalShortcut: settings.globalShortcut,
    autostart: settings.autostart,
    urlOpenMode: settings.urlOpenMode,
    mcpEnabled: settings.mcpEnabled,
    mcpPort: String(settings.mcpPort),
    aiProviders: settings.aiProviders.map((provider) => ({
      id: provider.id,
      label: provider.label,
      command: provider.command,
      argsText: formatArgs(provider.args),
      enabled: provider.enabled,
      mcpArgs: provider.mcpArgs,
      mcpSupportsToken: provider.mcpSupportsToken,
    })),
    aiDefaultProviderId: settings.aiDefaultProviderId,
    aiTimeoutSecs: String(settings.aiTimeoutSecs),
  };
}

/**
 * フォーム状態を保存用の設定へ変換する。
 *
 * [`validateDraft`] を通したドラフトにのみ使うこと(数値のパースを前提にする)。
 */
export function fromDraft(draft: SettingsDraft): CoreSettings {
  const providers: AiProvider[] = draft.aiProviders.map((provider) => ({
    id: provider.id.trim(),
    label: provider.label.trim(),
    command: provider.command.trim(),
    args: parseArgs(provider.argsText) ?? [],
    enabled: provider.enabled,
    mcpArgs: provider.mcpArgs,
    mcpSupportsToken: provider.mcpSupportsToken,
  }));

  return {
    weekStart: draft.weekStart,
    backupGenerations: Number(draft.backupGenerations),
    overlayEnabled: draft.overlayEnabled,
    globalShortcut: draft.globalShortcut.trim(),
    autostart: draft.autostart,
    urlOpenMode: draft.urlOpenMode,
    mcpEnabled: draft.mcpEnabled,
    mcpPort: Number(draft.mcpPort),
    aiProviders: providers,
    aiDefaultProviderId: draft.aiDefaultProviderId.trim(),
    aiTimeoutSecs: Number(draft.aiTimeoutSecs),
  };
}

/** 十進の整数として読めるか(空・小数・符号付きは弾く)。 */
function toInteger(text: string): number | null {
  const trimmed = text.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const value = Number(trimmed);
  return Number.isSafeInteger(value) ? value : null;
}

/** フォーム状態を検証する。空配列なら保存してよい。 */
export function validateDraft(draft: SettingsDraft): SettingsIssue[] {
  const issues: SettingsIssue[] = [];

  const generations = toInteger(draft.backupGenerations);
  if (generations === null || generations < 1) {
    issues.push({ section: "general", message: "バックアップ世代数は 1 以上の整数にしてください。" });
  }

  const port = toInteger(draft.mcpPort);
  if (port === null || port < MCP_PORT_RANGE.min || port > MCP_PORT_RANGE.max) {
    issues.push({
      section: "mcp",
      message: `MCP ポートは ${MCP_PORT_RANGE.min}〜${MCP_PORT_RANGE.max} の整数で指定してください。`,
    });
  }

  const timeout = toInteger(draft.aiTimeoutSecs);
  if (timeout === null || timeout < AI_TIMEOUT_RANGE.min || timeout > AI_TIMEOUT_RANGE.max) {
    issues.push({
      section: "ai",
      message: `AI のタイムアウトは ${AI_TIMEOUT_RANGE.min}〜${AI_TIMEOUT_RANGE.max} 秒の整数で指定してください。`,
    });
  }

  const seen = new Set<string>();
  draft.aiProviders.forEach((provider, index) => {
    const id = provider.id.trim();
    const name = id || `${index + 1} 行目`;
    if (id === "") {
      issues.push({ section: "ai", message: `AI プロバイダ ${index + 1} 行目の id を入力してください。` });
    } else if (seen.has(id)) {
      issues.push({ section: "ai", message: `AI プロバイダの id "${id}" が重複しています。` });
    } else {
      seen.add(id);
    }
    if (provider.label.trim() === "") {
      issues.push({ section: "ai", message: `AI プロバイダ "${name}" の表示名を入力してください。` });
    }
    if (provider.command.trim() === "") {
      issues.push({ section: "ai", message: `AI プロバイダ "${name}" の command を入力してください。` });
    }
    if (parseArgs(provider.argsText) === null) {
      issues.push({
        section: "ai",
        message: `AI プロバイダ "${name}" の args を解釈できません(引用符の対応か JSON 配列を確認してください)。`,
      });
    }
  });

  const defaultId = draft.aiDefaultProviderId.trim();
  const chosen = draft.aiProviders.find((provider) => provider.id.trim() === defaultId);
  if (!chosen) {
    issues.push({ section: "ai", message: "既定のプロバイダに、一覧にない id が指定されています。" });
  } else if (!chosen.enabled) {
    issues.push({ section: "ai", message: `既定のプロバイダ "${defaultId}" が無効になっています。` });
  }

  return issues;
}

/** 未保存の変更があるか。 */
export function isDirty(draft: SettingsDraft, baseline: SettingsDraft): boolean {
  return JSON.stringify(draft) !== JSON.stringify(baseline);
}

/** 節ごとの問題件数(ナビゲーションの印に使う)。 */
export function issuesBySection(issues: readonly SettingsIssue[]): Record<SectionKey, number> {
  // プラグイン節はコア設定の検証対象外なので常に 0。
  const counts: Record<SectionKey, number> = { general: 0, shortcut: 0, mcp: 0, ai: 0, plugins: 0 };
  for (const issue of issues) counts[issue.section] += 1;
  return counts;
}

/** 追加ボタンで作る空のプロバイダ行。 */
export function emptyProvider(): ProviderDraft {
  return {
    id: "",
    label: "",
    command: "",
    argsText: "-p {prompt}",
    enabled: true,
    mcpArgs: [],
    mcpSupportsToken: false,
  };
}

/** トークンありのコマンド例で、実際の値の代わりに置く目印。 */
export const MCP_TOKEN_PLACEHOLDER = "<設定したトークン>";

/**
 * Claude Code へこの MCP サーバーを登録するコマンド例。
 *
 * トークンは資格情報ストアにあり**アプリから読み出せない**ので、設定済みの場合も
 * 値は差し込めない。`--header` の形だけを見せて、値は [`MCP_TOKEN_PLACEHOLDER`] を
 * 自分で置き換えてもらう。
 */
export function claudeMcpCommand(url: string, tokenConfigured: boolean): string {
  const base = `claude mcp add --transport http questloom ${url}`;
  return tokenConfigured
    ? `${base} --header "Authorization: Bearer ${MCP_TOKEN_PLACEHOLDER}"`
    : base;
}
