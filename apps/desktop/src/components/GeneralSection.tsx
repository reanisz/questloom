/** 設定画面の「一般」節。週の開始曜日・自動起動・バックアップ世代数。 */

import type { SettingsDraft } from "../settings";
import type { WeekStart } from "../types";
import { Field, Toggle } from "./SettingsControls";

interface Props {
  draft: SettingsDraft;
  /** ドラフトの一部を差し替える。 */
  patch: (changes: Partial<SettingsDraft>) => void;
}

export function GeneralSection({ draft, patch }: Props) {
  return (
    <section className="settings-section">
      <h2>一般</h2>
      <Field label="週の開始曜日" hint="This Week / Next Week の区切りに使います。">
        <select
          value={draft.weekStart}
          onChange={(event) => patch({ weekStart: event.target.value as WeekStart })}
        >
          <option value="monday">月曜</option>
          <option value="sunday">日曜</option>
        </select>
      </Field>

      <Toggle
        label="OS のログイン時に自動起動する"
        hint="保存すると、すぐにスタートアップ登録へ反映されます。"
        checked={draft.autostart}
        onChange={(autostart) => patch({ autostart })}
      />

      <Field
        label="バックアップ世代数"
        hint="起動時に DB をバックアップし、この件数より古いものから消します。"
      >
        <input
          type="number"
          min={1}
          className="settings-number"
          value={draft.backupGenerations}
          onChange={(event) => patch({ backupGenerations: event.target.value })}
        />
      </Field>
    </section>
  );
}
