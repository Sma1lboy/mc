import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

// 本地字体(Blocky Craft:像素标题 + 像素徽标 + Noto 正文),最先加载避免 FOUT。
import "./theme/fonts.css";
// 全局设计令牌(CSS 变量)必须最先加载,后续主题脚本只覆盖 --a-*/--n-*。
import "./theme/tokens.css";
// Tailwind 入口 + 令牌桥接(放 tokens 之后:preflight/utilities 引用上面的变量)。
import "./theme/tailwind.css";
import { installClientLogForwarding, log } from "./util/log";

declare global {
  interface Window {
    __MC_E2E_START__?: () => Promise<void>;
  }
}

async function mountApp(): Promise<void> {
  // 延迟导入让 E2E 能在产品模块执行前安装外部服务 mocks；普通构建仍立即调用。
  const { default: App } = await import("./App");
  const root = document.getElementById("root");
  if (!root) {
    throw new Error("未找到挂载点 #root,请检查 index.html。");
  }

  // 渲染期异常写到页面便于排查(否则 React 会吞成白屏)。
  try {
    createRoot(root).render(
      <StrictMode>
        <App />
      </StrictMode>,
    );
  } catch (e) {
    root.innerHTML =
      '<pre style="color:#ff6b6b;padding:20px;white-space:pre-wrap;font:13px monospace">RENDER ERROR:\n' +
      String((e as Error)?.stack || e) +
      "</pre>";
    log.error("mount failed", e);
  }
}

// 尽早挂全局错误转发:未捕获异常/console.error 会一并落进统一日志(client 前缀)。
installClientLogForwarding();

if (import.meta.env.VITE_E2E === "1") {
  await import("@wdio/tauri-plugin");
  window.__MC_E2E_START__ = mountApp;
} else {
  await mountApp();
}
