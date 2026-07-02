import type { GlobalProvider } from "@ladle/react";

// 先装 Tauri 后端桩:让故事里直接 import 的真实组件/页面调用 invoke() 时拿到 mock 数据,
// 而不是崩溃——这样「主 UI 存在,这边只需引入 + 上下文即可模拟」,不维护第二套 UI。
import "./tauriMock";

// 与 src/main.tsx 相同的顺序加载设计系统:字体 → 令牌(CSS 变量)→ Tailwind 入口 + 桥接。
// 这样故事里的组件拿到和真实 app 一致的观感(色阶 / 阴影 / 字体)。
import "../src/theme/fonts.css";
import "../src/theme/tokens.css";
import "../src/theme/tailwind.css";
// theme.ts 在导入时即把 DEFAULT_THEME(深色 + 熔岩橙)注入 <html>,补齐 --a-*/--n-* 派生变量。
import "../src/theme/theme";

/**
 * Ladle 全局 Provider —— 给每个故事套上和 app 外壳一致的深色面板背景。
 * 组件用 bg-panel-* / text-fg 等令牌类,需要一个坐落在 tokens 之上的容器才好看。
 */
export const Provider: GlobalProvider = ({ children }) => (
  <div
    style={{
      background: "var(--bg-window)",
      color: "var(--text)",
      minHeight: "100vh",
      padding: "24px",
      fontFamily: "var(--font)",
    }}
  >
    <div style={{ maxWidth: 820, margin: "0 auto" }}>{children}</div>
  </div>
);
