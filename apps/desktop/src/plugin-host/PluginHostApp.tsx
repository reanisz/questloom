/**
 * plugin-host ウィンドウの中身。
 *
 * このウィンドウは常に非表示なので UI としての意味は無いが、
 * 開発中に webview のインスペクタから状態を確かめられるよう、
 * ロード結果を素朴な一覧として描画しておく。
 *
 * プラグインの起動自体は React の外([`startHost`])で行う。StrictMode の
 * 二重実行や再レンダーでプラグインが二重に activate されないようにするため。
 */

import { useEffect, useState } from "react";

import { subscribeHost, type HostState } from "./host";

export function PluginHostApp() {
  const [state, setState] = useState<HostState>({
    loading: true,
    plugins: [],
    loadedAt: null,
    hostError: null,
  });

  useEffect(() => subscribeHost(setState), []);

  return (
    <div className="plugin-host-debug">
      <h1>questloom plugin host</h1>
      <p>
        {state.loading ? "読み込み中…" : `${state.plugins.length} 件`}
        {state.loadedAt && ` / ${state.loadedAt}`}
      </p>
      {state.hostError && <p role="alert">ホストのエラー: {state.hostError}</p>}
      <ul>
        {state.plugins.map((plugin) => (
          <li key={plugin.fileName}>
            <strong>{plugin.fileName}</strong>{" "}
            {plugin.manifest ? `${plugin.manifest.id}@${plugin.manifest.version ?? "?"}` : "(未解決)"}{" "}
            {plugin.active ? "有効" : "無効"}
            {plugin.error && <pre>{plugin.error}</pre>}
          </li>
        ))}
      </ul>
    </div>
  );
}
