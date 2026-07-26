import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import Pet from "./pet/Pet";
import "./App.css";

// 同一份 SPA 同时被主窗口和宠物窗口加载；按窗口 label 决定渲染谁。
const isPet = (() => {
  try {
    return getCurrentWindow().label === "pet";
  } catch {
    return false;
  }
})();

if (isPet) document.documentElement.classList.add("pet-mode");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isPet ? <Pet /> : <App />}</React.StrictMode>,
);
