import { useEffect } from "react";
import type { Story, StoryDefault } from "@ladle/react";
import App from "./App";
import { refreshInstances, setCurrentRoot, setCurrentPage } from "./store";

/* ============================================================================
 * app.stories —— 整机视图:真正的 <App/>(Rail 侧栏 + TopBar + 页面)整壳渲染。
 *
 * 不重建 UI:直接 import 真实 App,靠 .ladle 的 Tauri 桩喂 mock 后端 + 种一点 store
 * 上下文,整个窗口就原样出来,还能点 Rail 在页面间切换(home/discover/library/settings)。
 * 用 fixed inset-0 撑满(绕开 Provider 的 820px 限宽容器),贴近真实窗口。
 * ========================================================================== */

export default {
  title: "App",
} satisfies StoryDefault;

export const FullApp: Story = () => {
  useEffect(() => {
    setCurrentRoot("/mock/minecraft");
    setCurrentPage("home");
    void refreshInstances(); // → Tauri 桩返回 mock 实例
  }, []);
  return (
    <div style={{ position: "fixed", inset: 0, background: "var(--bg-window)" }}>
      <App />
    </div>
  );
};
FullApp.storyName = "整机视图(Rail + TopBar + 页面)";
