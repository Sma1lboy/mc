import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri v2 前端构建配置:固定端口 1420 供 Tauri webview 加载,
// 关闭 Vite 自带的清屏以便和 cargo/tauri 的日志共存。
export default defineConfig({
  // root 即 desktop/ 本目录(index.html 所在处)。
  root: ".",
  // tailwindcss() 先跑做 CSS 处理,再交给 react 的 JSX 变换。
  plugins: [tailwindcss(), react()],
  // Tauri 期望前端固定跑在 1420;strictPort 保证端口被占用时直接失败而非漂移。
  server: {
    port: 1420,
    strictPort: true,
  },
  // 让 Tauri 的 Rust 日志不被 Vite 清掉。
  clearScreen: false,
  build: {
    // Tauri webview 基于较新的 WebKit/WebView2,直接用 esnext 产物,体积更小。
    target: "esnext",
    outDir: "dist",
    emptyOutDir: true,
    // 生产构建不需要 sourcemap(可按需打开);此处保持精简。
    sourcemap: false,
    // 把大块第三方库拆出主包,避免单 chunk 超过 500kB 的告警。
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes("node_modules")) return;
          if (id.includes("/react") || id.includes("/scheduler")) return "react";
          if (id.includes("/streamdown") || id.includes("/shiki") || id.includes("/marked")
            || id.includes("/katex") || id.includes("/remark") || id.includes("/rehype")
            || id.includes("/micromark") || id.includes("/mdast") || id.includes("/hast")
            || id.includes("/unified") || id.includes("/unist") || id.includes("/vfile"))
            return "markdown";
          if (id.includes("/motion") || id.includes("/framer-motion")) return "motion";
          if (id.includes("/lucide-react")) return "icons";
        },
      },
    },
  },
});
