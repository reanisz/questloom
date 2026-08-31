/**
 * エントリポイント。1 つのバンドルをメインウィンドウ・オーバーレイ・plugin-host で共有し、
 * ウィンドウラベルで描画するコンポーネントを切り替える。
 */

import { getCurrentWindow } from "@tauri-apps/api/window";
import React from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import { OverlayApp } from "./components/OverlayApp";
import { startHost } from "./plugin-host/host";
import { PluginHostApp } from "./plugin-host/PluginHostApp";
import "./styles.css";

/** オーバーレイウィンドウのラベル(tauri.conf.json と揃えること)。 */
const OVERLAY_LABEL = "overlay";
/** TS プラグインを走らせる非表示ウィンドウのラベル(同上)。 */
const PLUGIN_HOST_LABEL = "plugin-host";

const label = getCurrentWindow().label;
const isOverlay = label === OVERLAY_LABEL;
const isPluginHost = label === PLUGIN_HOST_LABEL;

// 透過ウィンドウでは body の背景を消す(CSS 側で `.overlay-body` を透明にする)。
document.body.classList.add(isOverlay ? "overlay-body" : "main-body");

// プラグインの起動は React の外で行う(StrictMode の二重実行で二重 activate しないため)。
if (isPluginHost) startHost();

function root() {
  if (isPluginHost) return <PluginHostApp />;
  if (isOverlay) return <OverlayApp />;
  return <App />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{root()}</React.StrictMode>,
);
