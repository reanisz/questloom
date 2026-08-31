/**
 * AI 機能まわりの共有フック。
 *
 * プロバイダ一覧はコア設定 (`get_settings`) から取り、
 * 実行状態は `questloom://ai-status` イベントで受け取る。
 */

import { useCallback, useEffect, useState } from "react";

import * as api from "./api";
import { toMessage } from "./tauri";
import type { AiProvider, AiStatus } from "./types";
import { useTauriEvent } from "./useTauriEvent";

/** 有効なプロバイダと既定 ID。ダイアログを開いたときに読み込む。 */
export function useAiProviders(enabled: boolean) {
  const [providers, setProviders] = useState<AiProvider[]>([]);
  const [defaultId, setDefaultId] = useState<string>("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    api
      .getSettings()
      .then((settings) => {
        if (!alive) return;
        setProviders(settings.aiProviders.filter((provider) => provider.enabled));
        setDefaultId(settings.aiDefaultProviderId);
        setError(null);
      })
      .catch((cause) => {
        if (alive) setError(toMessage(cause));
      });
    return () => {
      alive = false;
    };
  }, [enabled]);

  return { providers, defaultId, error };
}

/**
 * 直近の AI 実行状態。`running` の間だけ実行中とみなす。
 *
 * このウィンドウが投げた実行だけでなく、バックエンドからの通知をそのまま映す。
 */
export function useAiStatus(): AiStatus | null {
  const [status, setStatus] = useState<AiStatus | null>(null);

  useTauriEvent(api.listenAiStatus, setStatus);

  return status;
}

/** AI 実行を 1 つ抱えるための状態。ボタンの活殺と結果表示に使う。 */
export function useAiRun<T>() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<T | null>(null);

  const start = useCallback(async (action: () => Promise<T>) => {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      setResult(await action());
    } catch (cause) {
      setError(toMessage(cause));
    } finally {
      setBusy(false);
    }
  }, []);

  const cancel = useCallback(() => {
    api.aiCancel().catch((cause) => setError(toMessage(cause)));
  }, []);

  const reset = useCallback(() => {
    setError(null);
    setResult(null);
  }, []);

  return { busy, error, result, start, cancel, reset };
}
