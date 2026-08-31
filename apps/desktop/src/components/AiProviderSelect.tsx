/**
 * AI プロバイダの選択セレクト。
 *
 * プロバイダ一覧の取得(`get_settings`)と「未選択なら既定プロバイダに追随する」
 * 挙動をここに閉じ込める。選択値は呼び出し側が持つ(実行時にそのまま渡すため)。
 *
 * 一覧の取得に失敗したときのメッセージは、ダイアログごとに置き場所が違うので
 * `onError` で呼び出し側へ返し、描画は任せる。
 */

import { useEffect } from "react";

import { useAiProviders } from "../useAi";

interface Props {
  /** 偽の間は一覧を取りに行かない(ダイアログを開いたときだけ読む)。 */
  enabled?: boolean;
  /** 選択中のプロバイダ id。空文字列なら未選択。 */
  value: string;
  /** 選択が変わったとき、および既定プロバイダへ追随したときに呼ばれる。 */
  onChange: (providerId: string) => void;
  /** 実行中などで操作させたくないとき。 */
  disabled?: boolean;
  /** 一覧の取得エラー(無ければ null)。 */
  onError?: (message: string | null) => void;
}

export function AiProviderSelect({ enabled = true, value, onChange, disabled, onError }: Props) {
  const { providers, defaultId, error } = useAiProviders(enabled);

  useEffect(() => {
    if (!value && defaultId) onChange(defaultId);
  }, [defaultId, value, onChange]);

  useEffect(() => {
    onError?.(error);
  }, [error, onError]);

  return (
    <label className="ai-provider">
      <span className="muted">プロバイダ</span>
      <select
        value={value}
        disabled={disabled || providers.length === 0}
        onChange={(event) => onChange(event.target.value)}
      >
        {providers.length === 0 && <option value="">(利用可能なプロバイダなし)</option>}
        {providers.map((provider) => (
          <option key={provider.id} value={provider.id}>
            {provider.label}
          </option>
        ))}
      </select>
    </label>
  );
}
