/**
 * 設定ドラフトの変換と検証のテスト。
 *
 * ## バックエンドとの対応
 *
 * 検証規則は `apps/desktop/src-tauri/src/settings.rs::validate`
 * (= `questloom_core::settings::CoreSettings::validate` + ショートカットのパース)と
 * 揃っていなければならない。**片方だけ通る設定があると、保存ボタンを押した瞬間に
 * バックエンドが弾いて理由が UI に出ない**という一番わかりにくい壊れ方をする。
 *
 * そこで検証のテスト名は `SettingsError` のバリアント名を頭に付け、
 * どのバックエンド規則の写しなのかを名指しする。
 * バックエンドにしか無い規則(ショートカット文字列のパース)は
 * 「フロントは見ない」ことをテストで固定する。
 */

import { describe, expect, it } from "vitest";

import {
  AI_TIMEOUT_RANGE,
  claudeMcpCommand,
  MCP_TOKEN_PLACEHOLDER,
  emptyProvider,
  formatArgs,
  fromDraft,
  isDirty,
  issuesBySection,
  MCP_PORT_RANGE,
  parseArgs,
  toDraft,
  validateDraft,
  type SettingsDraft,
} from "./settings";
import type { CoreSettings } from "./types";

/** バックエンドの `CoreSettings::default()` に相当する既定値。 */
function defaults(): CoreSettings {
  return {
    weekStart: "monday",
    backupGenerations: 14,
    overlayEnabled: true,
    globalShortcut: "Ctrl+Space",
    autostart: false,
    urlOpenMode: "external",
    mcpEnabled: true,
    mcpPort: 39150,
    aiProviders: [
      {
        id: "claude",
        label: "Claude Code",
        command: "claude",
        args: ["-p", "{prompt}"],
        enabled: true,
        mcpArgs: ["--mcp-config", "{mcp_config}"],
        mcpSupportsToken: true,
      },
      {
        id: "codex",
        label: "Codex",
        command: "codex",
        args: ["exec", "{prompt}"],
        enabled: true,
        mcpArgs: [],
        mcpSupportsToken: false,
      },
    ],
    aiDefaultProviderId: "claude",
    aiTimeoutSecs: 300,
  };
}

/**
 * URL リソースの開き方は、そのままドラフトを往復する
 * (数値と違って編集途中の文字列にならないので、変換は素通し)。
 */
describe("urlOpenMode", () => {
  it("既定は外部ブラウザ", () => {
    expect(toDraft(defaults()).urlOpenMode).toBe("external");
  });

  it("ドラフトを往復しても失われない", () => {
    for (const mode of ["external", "internal", "internalAuto"] as const) {
      const settings: CoreSettings = { ...defaults(), urlOpenMode: mode };
      expect(fromDraft(toDraft(settings)).urlOpenMode).toBe(mode);
    }
  });

  it("変更は未保存として検出される", () => {
    const baseline = toDraft(defaults());
    expect(isDirty({ ...baseline, urlOpenMode: "internal" }, baseline)).toBe(true);
  });
});

/** 既定のドラフトに差分をあてる。 */
function draft(patch: Partial<SettingsDraft> = {}): SettingsDraft {
  return { ...toDraft(defaults()), ...patch };
}

/** 検証結果のメッセージをまとめて読む。 */
function messages(value: SettingsDraft): string {
  return validateDraft(value)
    .map((issue) => `${issue.section}: ${issue.message}`)
    .join("\n");
}

// ---- 引数テキストの相互変換 ----

describe("parseArgs / formatArgs", () => {
  it("スペース区切りを配列にする", () => {
    expect(parseArgs("-p {prompt}")).toEqual(["-p", "{prompt}"]);
    expect(parseArgs("  exec   {prompt}  ")).toEqual(["exec", "{prompt}"]);
    expect(parseArgs("")).toEqual([]);
    expect(parseArgs("   ")).toEqual([]);
  });

  it('引用符でスペースを含む引数を 1 つにまとめ、"" はリテラルの " になる', () => {
    expect(parseArgs('-c "a b"')).toEqual(["-c", "a b"]);
    expect(parseArgs('"" x')).toEqual(["", "x"]);
    expect(parseArgs('-c "say ""hi"""')).toEqual(["-c", 'say "hi"']);
  });

  it("閉じていない引用符は解釈できない", () => {
    expect(parseArgs('-c "a b')).toBeNull();
  });

  it("[ で始まるものは JSON 配列として読み、失敗してもスペース区切りへ落とさない", () => {
    expect(parseArgs('["exec", "{prompt}"]')).toEqual(["exec", "{prompt}"]);
    expect(parseArgs('["exec", ')).toBeNull();
    // 文字列以外を含む JSON 配列も拒む。
    expect(parseArgs("[1, 2]")).toBeNull();
  });

  it("JSON として読むのは [ で始まるものだけ(それ以外はスペース区切り)", () => {
    expect(parseArgs('{"a": 1}')).toEqual(["{a:", "1}"]);
  });

  it("formatArgs → parseArgs で元の配列に戻る", () => {
    for (const args of [
      [],
      ["-p", "{prompt}"],
      ["-c", "a b"],
      ["", "x"],
      ['say "hi"'],
      ["mcp_servers.questloom.url=\"{mcp_url}\""],
    ]) {
      expect(parseArgs(formatArgs(args))).toEqual(args);
    }
  });
});

// ---- ドラフト⇄CoreSettings ----

describe("toDraft / fromDraft", () => {
  it("往復しても設定が変わらない", () => {
    const settings = defaults();
    expect(fromDraft(toDraft(settings))).toEqual(settings);
  });

  it("数値は編集途中の文字列として持ち、保存時にだけ数値へ戻す", () => {
    const value = toDraft(defaults());
    expect(value.mcpPort).toBe("39150");
    expect(value.backupGenerations).toBe("14");
    expect(value.aiTimeoutSecs).toBe("300");
    expect(fromDraft(value).mcpPort).toBe(39150);
  });

  it("MCP トークンはドラフトにもコア設定にも載らない(実体は資格情報ストア)", () => {
    // 平文の混入を防ぐため、往復のどちらにも `mcpToken` が現れないことを固定する。
    expect(toDraft(defaults())).not.toHaveProperty("mcpToken");
    expect(fromDraft(draft())).not.toHaveProperty("mcpToken");

    // 旧バージョンの JSON が紛れ込んでも、保存する値には持ち越さない。
    const legacy = { ...defaults(), mcpToken: "s3cret" } as unknown as CoreSettings;
    expect(JSON.stringify(fromDraft(toDraft(legacy)))).not.toContain("s3cret");
  });

  it("画面で編集しない mcpArgs / mcpSupportsToken を保存時に失わない", () => {
    const saved = fromDraft(toDraft(defaults()));
    expect(saved.aiProviders[0].mcpArgs).toEqual(["--mcp-config", "{mcp_config}"]);
    expect(saved.aiProviders[0].mcpSupportsToken).toBe(true);
    expect(saved.aiProviders[1].mcpArgs).toEqual([]);
  });

  it("id・表示名・command・既定プロバイダの前後空白は落とす", () => {
    const value = draft({ aiDefaultProviderId: "  claude  " });
    value.aiProviders[0] = {
      ...value.aiProviders[0],
      id: " claude ",
      label: " Claude Code ",
      command: " claude ",
    };
    const saved = fromDraft(value);
    expect(saved.aiProviders[0].id).toBe("claude");
    expect(saved.aiProviders[0].label).toBe("Claude Code");
    expect(saved.aiProviders[0].command).toBe("claude");
    expect(saved.aiDefaultProviderId).toBe("claude");
  });
});

// ---- 検証(バックエンドと 1 対 1) ----

describe("validateDraft — settings.rs::validate と同じ規則を張る", () => {
  it("既定の設定は通る(バックエンドの defaults_are_valid に対応)", () => {
    expect(validateDraft(draft())).toEqual([]);
  });

  it("SettingsError::BackupGenerations — 世代数は 1 以上の整数", () => {
    expect(validateDraft(draft({ backupGenerations: "1" }))).toEqual([]);
    for (const broken of ["0", "", "  ", "-1", "1.5", "いち", "1e3"]) {
      const issues = validateDraft(draft({ backupGenerations: broken }));
      expect(issues, broken).toHaveLength(1);
      expect(issues[0].section).toBe("general");
    }
  });

  it("SettingsError::McpPort — ポートは 1024〜65535 の整数", () => {
    expect(MCP_PORT_RANGE).toEqual({ min: 1024, max: 65535 });
    for (const ok of ["1024", "39150", "65535"]) {
      expect(validateDraft(draft({ mcpPort: ok })), ok).toEqual([]);
    }
    for (const broken of ["1023", "80", "0", "65536", "", "abc", "-1", "3915 0"]) {
      const issues = validateDraft(draft({ mcpPort: broken }));
      expect(issues, broken).toHaveLength(1);
      expect(issues[0].section).toBe("mcp");
    }
  });

  it("SettingsError::AiTimeout — タイムアウトは 10〜3600 秒", () => {
    expect(AI_TIMEOUT_RANGE).toEqual({ min: 10, max: 3600 });
    for (const ok of ["10", "300", "3600"]) {
      expect(validateDraft(draft({ aiTimeoutSecs: ok })), ok).toEqual([]);
    }
    for (const broken of ["9", "3601", "0", "", "五分"]) {
      const issues = validateDraft(draft({ aiTimeoutSecs: broken }));
      expect(issues, broken).toHaveLength(1);
      expect(issues[0].section).toBe("ai");
    }
  });

  it("SettingsError::EmptyProviderId — プロバイダの id は必須", () => {
    const value = draft();
    value.aiProviders[1] = { ...value.aiProviders[1], id: "   " };
    expect(messages(value)).toContain("2 行目の id");
  });

  it("SettingsError::DuplicateProviderId — id は重複させない", () => {
    const value = draft();
    value.aiProviders[1] = { ...value.aiProviders[1], id: "claude" };
    expect(messages(value)).toContain('id "claude" が重複');
    expect(validateDraft(value)).toHaveLength(1);
  });

  it("SettingsError::EmptyProviderLabel — 表示名は必須", () => {
    const value = draft();
    value.aiProviders[0] = { ...value.aiProviders[0], label: " " };
    expect(messages(value)).toContain('"claude" の表示名');
  });

  it("SettingsError::EmptyProviderCommand — command は必須", () => {
    const value = draft();
    value.aiProviders[0] = { ...value.aiProviders[0], command: "" };
    expect(messages(value)).toContain('"claude" の command');
  });

  it("SettingsError::DefaultProviderUnavailable — 既定 id は一覧にあり、かつ有効", () => {
    expect(messages(draft({ aiDefaultProviderId: "gemini" }))).toContain("一覧にない id");

    const disabled = draft();
    disabled.aiProviders[0] = { ...disabled.aiProviders[0], enabled: false };
    expect(messages(disabled)).toContain('"claude" が無効');
  });

  it("args の解釈不能はフロント固有の規則(保存時に配列へ直せないため)", () => {
    const value = draft();
    value.aiProviders[0] = { ...value.aiProviders[0], argsText: '-p "{prompt}' };
    expect(messages(value)).toContain("args を解釈できません");
  });

  it("グローバルショートカットのパースはバックエンド専任(フロントは見ない)", () => {
    // Tauri のパーサが要るためコア/フロントには置けない。ここで弾かないことを固定しておく
    // (弾いたつもりで規則が二重化していないことの確認)。
    expect(validateDraft(draft({ globalShortcut: "Ctrl+" }))).toEqual([]);
    expect(validateDraft(draft({ globalShortcut: "" }))).toEqual([]);
  });

  it("問題は溜めて返す(最初の 1 件で打ち切らない)", () => {
    const issues = validateDraft(
      draft({ backupGenerations: "0", mcpPort: "80", aiTimeoutSecs: "1" }),
    );
    expect(issues).toHaveLength(3);
    expect(issuesBySection(issues)).toEqual({
      general: 1,
      shortcut: 0,
      mcp: 1,
      ai: 1,
      plugins: 0,
    });
  });
});

// ---- 画面の小道具 ----

describe("issuesBySection", () => {
  it("問題が無ければ全節 0(プラグイン節は常に対象外)", () => {
    expect(issuesBySection([])).toEqual({
      general: 0,
      shortcut: 0,
      mcp: 0,
      ai: 0,
      plugins: 0,
    });
  });
});

describe("isDirty", () => {
  it("同じ内容なら未変更", () => {
    expect(isDirty(draft(), draft())).toBe(false);
  });

  it("どこか 1 つでも違えば変更あり", () => {
    expect(isDirty(draft({ mcpPort: "39151" }), draft())).toBe(true);
    const providers = draft();
    providers.aiProviders[0] = { ...providers.aiProviders[0], enabled: false };
    expect(isDirty(providers, draft())).toBe(true);
  });
});

describe("emptyProvider", () => {
  it("追加した直後の行は id / 表示名 / command が未入力として弾かれる", () => {
    const value = draft();
    value.aiProviders.push(emptyProvider());
    const issues = validateDraft(value);
    expect(issues).toHaveLength(3);
    expect(issues.every((issue) => issue.section === "ai")).toBe(true);
    // 既定の args テンプレートは解釈できる形にしておく。
    expect(parseArgs(emptyProvider().argsText)).toEqual(["-p", "{prompt}"]);
  });
});

describe("claudeMcpCommand", () => {
  it("トークン未設定なら --header を付けない", () => {
    expect(claudeMcpCommand("http://127.0.0.1:39150/mcp", false)).toBe(
      "claude mcp add --transport http questloom http://127.0.0.1:39150/mcp",
    );
  });

  it("トークン設定済みなら Authorization ヘッダの形だけを見せる(値は差し込めない)", () => {
    const command = claudeMcpCommand("http://127.0.0.1:39150/mcp", true);
    expect(command).toContain(`--header "Authorization: Bearer ${MCP_TOKEN_PLACEHOLDER}"`);
    // 値は資格情報ストアにあり、フロントからは読めない。
    expect(MCP_TOKEN_PLACEHOLDER).toBe("<設定したトークン>");
  });
});
