import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

// Tauri v2 前端构建配置:固定端口 1420 供 Tauri webview 加载,
// 关闭 Vite 自带的清屏以便和 cargo/tauri 的日志共存。
export default defineConfig({
  // root 即 desktop/ 本目录(index.html 所在处)。
  root: ".",
  plugins: [solid()],
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
  },
});
