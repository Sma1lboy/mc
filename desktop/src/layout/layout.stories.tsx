import type { Story, StoryDefault } from "@ladle/react";
import TopBar from "./TopBar";

/* ============================================================================
 * layout.stories —— 外壳布局片段的隔离预览(Ladle)。
 *
 * TopBar 是自绘标题栏(48px 石质顶栏)。它读 store 切片(运行数 / 社交开关 / 登录态,
 * 均取默认值渲染),窗口按钮的 Tauri API 只在点击时惰性调用、非 Tauri 环境静默忽略;
 * 内嵌的下载队列 / 通知 / 好友入口的后端拉取都在点击后才发生。故打开态外壳可直接预览。
 * ========================================================================== */

export default {
  title: "Layout / TopBar",
} satisfies StoryDefault;

/** 顶栏满宽:套一个 -mx 负边距 + 深色底,还原它在窗口顶部的观感。 */
export const Bar: Story = () => (
  <div className="-mx-[24px] -mt-[24px]">
    <TopBar />
  </div>
);
Bar.storyName = "TopBar · 自绘标题栏(默认态)";

/* ----------------------------------------------------------------------------
 * 跳过(SKIPPED)—— 其余 layout 片段强依赖 store 实时页面状态 / 路由,空壳无意义:
 *   · AppShell —— 整个应用外壳(侧栏 + 页面路由 + 全局 overlay 挂载),需完整 store。
 *   · Rail / ContextBar —— 侧栏导航与右侧上下文栏由 currentPage / 选中实例驱动,
 *     已在 NavItem / EmptyState 等原语层覆盖其视觉构件。
 * -------------------------------------------------------------------------- */
