import { useEffect } from "react";
import type { Story, StoryDefault } from "@ladle/react";
import Home from "./Home";
import { refreshInstances } from "../store";

/* ============================================================================
 * pages.stories —— 真实页面在 Ladle 里的整装预览。
 *
 * 关键:不重建 UI。直接 import 真实页面组件,靠 .ladle 的 Tauri 桩(tauriMock.ts)喂
 * mock 后端数据 + 必要时给 store 灌一下上下文,页面就原样渲染出来。要更多数据就改
 * 那一处中心 mock,不在故事里造第二套 UI。
 * ========================================================================== */

export default {
  title: "Pages",
} satisfies StoryDefault;

/** 首页:import 真实 <Home/>,拉一次实例(经 Tauri 桩返回 mock)即渲染。 */
export const HomePage: Story = () => {
  useEffect(() => {
    void refreshInstances(); // → 桩返回 mock 实例;首页的发现搜索也走桩返回 mock 整合包
  }, []);
  return (
    <div className="h-[760px] w-full overflow-hidden border border-titlebar shadow-input">
      <Home />
    </div>
  );
};
HomePage.storyName = "首页 Home(真实页面 + mock 后端)";
