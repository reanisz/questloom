/** 設定画面の「一般」節。週の開始曜日・自動起動・URL の開き方・バックアップ世代数。 */

import type { SettingsDraft } from "../settings";
import type { UrlOpenMode, WeekStart } from "../types";
import { Field, Toggle } from "./SettingsControls";

interface Props {
  draft: SettingsDraft;
  /** ドラフトの一部を差し替える。 */
  patch: (changes: Partial<SettingsDraft>) => void;
}

/** URL リソースをクリックしたときの開き方(表示順)。 */
const URL_OPEN_MODES: readonly { value: UrlOpenMode; label: string; hint: string }[] = [
  {
    value: "external",
    label: "既定のブラウザで開く",
    hint: "OS のブラウザに渡します。",
  },
  {
    value: "internal",
    label: "内蔵ブラウザで開く",
    hint: "ウィンドウ左側のペインに表示します。",
  },
  {
    value: "internalAuto",
    label: "内蔵ブラウザで開く + 詳細を開いたら自動表示",
    hint: "タスク詳細を開いたとき、主リソースが URL なら自動でペインに出します。",
  },
];

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
        label="URL リソースの開き方"
        hint="右クリックメニューとリソース行の「外部で開く」「内蔵ブラウザで開く」は、この設定に関わらず使えます。"
      >
        <div className="settings-radios" role="radiogroup" aria-label="URL リソースの開き方">
          {URL_OPEN_MODES.map((mode) => (
            <label key={mode.value} className="settings-check">
              <input
                type="radio"
                name="urlOpenMode"
                value={mode.value}
                checked={draft.urlOpenMode === mode.value}
                onChange={() => patch({ urlOpenMode: mode.value })}
              />
              <span>
                {mode.label}
                <span className="settings-radio-hint">{mode.hint}</span>
              </span>
            </label>
          ))}
        </div>
      </Field>

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
