/* @refresh reload */
import { render } from "solid-js/web";

// 全局设计令牌(CSS 变量)必须最先加载,后续主题脚本只覆盖 --a-*/--n-*。
import "./theme/tokens.css";
// Tailwind 入口 + 令牌桥接(放 tokens 之后:preflight/utilities 引用上面的变量)。
import "./theme/tailwind.css";
import App from "./App";
import { installClientLogForwarding, log } from "./util/log";

// 尽早挂全局错误转发:未捕获异常/console.error 会一并落进统一日志(client 前缀)。
installClientLogForwarding();

const root = document.getElementById("root");

if (!root) {
  // 理论上 index.html 一定有 #root;缺失则属构建/模板错误,显式报错便于排查。
  throw new Error('未找到挂载点 #root,请检查 index.html。');
}

// SolidJS 渲染:render 返回 dispose,这里是应用根,无需手动卸载。
// 渲染期异常会被吞掉导致白屏;用 try/catch 把错误显式写到页面便于排查。
try {
  render(() => <App />, root);
} catch (e) {
  root.innerHTML =
    '<pre style="color:#ff6b6b;padding:20px;white-space:pre-wrap;font:13px monospace">RENDER ERROR:\n' +
    String((e as Error)?.stack || e) +
    "</pre>";
  log.error("mount failed", e);
}
