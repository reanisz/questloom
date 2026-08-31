/**
 * 設定画面の共通パーツ。
 *
 * 節ごとのコンポーネント (`GeneralSection` などと `PluginSettingsSection`) から使う、
 * 見た目だけの小物を集めたモジュール。状態は持たない(コピー済み表示を除く)。
 */

import { useState, type ReactNode } from "react";

/** クリップボードへコピーする。Tauri の webview では権限が無いこともあるので握りつぶさない。 */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // セキュアコンテキスト外などで clipboard API が使えない場合の保険。
    try {
      const area = document.createElement("textarea");
      area.value = text;
      area.style.position = "fixed";
      area.style.opacity = "0";
      document.body.appendChild(area);
      area.select();
      const ok = document.execCommand("copy");
      document.body.removeChild(area);
      return ok;
    } catch {
      return false;
    }
  }
}

/** ラベル付きの 1 行。説明文は入力の下に小さく出す。 */
export function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className="settings-field">
      <span className="settings-field-label">{label}</span>
      <div className="settings-field-control">
        {children}
        {hint && <p className="settings-hint">{hint}</p>}
      </div>
    </div>
  );
}

/** チェックボックス 1 個の行。 */
export function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <div className="settings-field">
      <span className="settings-field-label" />
      <div className="settings-field-control">
        <label className="settings-check">
          <input
            type="checkbox"
            checked={checked}
            onChange={(event) => onChange(event.target.checked)}
          />
          <span>{label}</span>
        </label>
        {hint && <p className="settings-hint">{hint}</p>}
      </div>
    </div>
  );
}

/** コピーボタン付きのコード表示。 */
export function CopyableCode({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  return (
    <div className="settings-code">
      <code>{text}</code>
      <button
        type="button"
        className="btn btn-sm"
        onClick={() => {
          void copyText(text).then((ok) => {
            setCopied(ok);
            if (ok) window.setTimeout(() => setCopied(false), 1600);
          });
        }}
      >
        {copied ? "✓ コピー済み" : "コピー"}
      </button>
    </div>
  );
}
