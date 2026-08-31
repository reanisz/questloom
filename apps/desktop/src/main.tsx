/**
 * エントリポイント。1 つのバンドルをメインウィンドウとオーバーレイで共有し、
 * ウィンドウラベルで描画するコンポーネントを切り替える。
 */

import { getCurrentWindow } from "@tauri-apps/api/window";
import React from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import { OverlayApp } from "./components/OverlayApp";
import "./styles.css";

/** オーバーレイウィンドウのラベル(tauri.conf.json と揃えること)。 */
const OVERLAY_LABEL = "overlay";

const isOverlay = getCurrentWindow().label === OVERLAY_LABEL;
// 透過ウィンドウでは body の背景を消す(CSS 側で `.overlay-body` を透明にする)。
document.body.classList.add(isOverlay ? "overlay-body" : "main-body");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isOverlay ? <OverlayApp /> : <App />}</React.StrictMode>,
);
