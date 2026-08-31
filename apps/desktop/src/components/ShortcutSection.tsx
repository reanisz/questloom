/** 設定画面の「ショートカットとオーバーレイ」節。 */

import type { SettingsDraft } from "../settings";
import type { RuntimeStatus } from "../types";
import { Field, Toggle } from "./SettingsControls";

interface Props {
  draft: SettingsDraft;
  patch: (changes: Partial<SettingsDraft>) => void;
  /** 稼働状態。取得できなかった場合は null。 */
  status: RuntimeStatus | null;
}

export function ShortcutSection({ draft, patch, status }: Props) {
  return (
    <section className="settings-section">
      <h2>ショートカットとオーバーレイ</h2>
      <Field
        label="グローバルショートカット"
        hint='メインウィンドウの表示・非表示を切り替えます。例: "Ctrl+Space" / "Alt+Shift+Q"。修飾キーは Ctrl / Alt / Shift / Super、空欄ならショートカットなし。'
      >
        <input
          type="text"
          className="settings-text"
          placeholder="Ctrl+Space"
          spellCheck={false}
          value={draft.globalShortcut}
          onChange={(event) => patch({ globalShortcut: event.target.value })}
        />
      </Field>

      <Field label="登録状態">
        <p className="settings-status">
          {status === null ? (
            <span className="muted">取得できませんでした</span>
          ) : status.shortcutRegistered ? (
            <span className="settings-ok">● 登録済み</span>
          ) : (
            <span className="settings-warn">
              ● 未登録(未設定か、他のアプリが使用中の可能性があります)
            </span>
          )}
        </p>
      </Field>

      <Toggle
        label="New タスクがあるときにオーバーレイ通知を表示する"
        hint="メインディスプレイの左上に、最前面の小さな一覧を出します。"
        checked={draft.overlayEnabled}
        onChange={(overlayEnabled) => patch({ overlayEnabled })}
      />
    </section>
  );
}
