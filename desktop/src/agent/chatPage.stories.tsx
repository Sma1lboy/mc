import { useEffect } from "react";
import type { Story, StoryDefault } from "@ladle/react";
import type { UIMessage } from "ai";
import ChatPage from "./ChatPage";
import { useChatStore } from "./chatStore";

/* ============================================================================
 * chatPage.stories —— 整装的聊天页面(把整条 UI 拼在一起看)。
 *
 * 不是单个碎片,而是给 store 灌一段 mock UIMessage[] 后直接渲染真正的 <ChatPage/>:
 * 头部 + 可滚动消息流(收拢活动块 / 工具芯片 / ask_user 选项)+ 底部输入条,一屏看全。
 * 无 Tauri:发送 / 新对话等动作在 Ladle 里静默失败,纯看装配后的视觉。
 * ========================================================================== */

export default {
  title: "Agent / Chat",
} satisfies StoryDefault;

function mkMsg(id: string, role: "user" | "assistant", parts: unknown[]): UIMessage {
  return { id, role, parts } as unknown as UIMessage;
}

const OPTIONS = [
  { id: "adventure", label: "冒险 / RPG", description: "地牢、任务、Boss 与探索" },
  { id: "tech", label: "科技 / 自动化", description: "机械动力、应用能源" },
  { id: "magic", label: "魔法", description: "神秘时代、植物魔法" },
];

// 一段完整对话:提问 → 助手(思考+多次搜索 收拢)+ 追问(已答)→ 用户回答 → 助手搜底包 + 再追问(可答)。
const FLOW: UIMessage[] = [
  mkMsg("u1", "user", [{ type: "text", text: "帮我组个耐玩的整合包" }]),
  mkMsg("a1", "assistant", [
    { type: "reasoning", text: "用户没给方向,先用 ask_user 让他挑一个再动手。" },
    { type: "text", text: "**先定个方向吧** —— 你更想要哪种体验?" },
    {
      type: "tool-ask_user_question",
      toolCallId: "ask_dir",
      state: "output-available",
      input: { question: "想要什么类型的整合包?", options: OPTIONS, multi_select: false },
      output: { selected: ["冒险 / RPG"] },
    },
  ]),
  mkMsg("u2", "user", [{ type: "text", text: "冒险 / RPG" }]),
  mkMsg("a2", "assistant", [
    { type: "reasoning", text: "搜几个 RPG 向底包,匹配度不高就换关键词多搜几轮。" },
    { type: "text", text: "冒险方向不错,我找几个合适的底包:" },
    { type: "tool-search_base_modpacks", toolCallId: "s1", state: "output-available", input: { query: "rpg adventure" }, output: { candidates: [] } },
    { type: "tool-search_base_modpacks", toolCallId: "s2", state: "output-available", input: { query: "dungeons quests" }, output: { candidates: [] } },
    { type: "tool-search_base_modpacks", toolCallId: "s3", state: "output-available", input: { query: "exploration pack" }, output: { candidates: [] } },
    { type: "text", text: "找到几个口碑不错的,选一个开工:" },
    {
      type: "tool-ask_user_question",
      toolCallId: "ask_pack",
      state: "input-available",
      input: {
        question: "用哪个底包?",
        options: [
          { id: "rad", label: "Roguelike Adventures and Dungeons", description: "3.2M 下载 · 地牢 + 职业 + 探索" },
          { id: "bmc", label: "Better MC [FORGE]", description: "1.1M 下载 · 大而全的冒险整合" },
        ],
        multi_select: false,
      },
    },
  ]),
];

/** 整装聊天页面:store 灌 mock 后渲染真正的 ChatPage,一屏看全装配效果。 */
export const FullChatPage: Story = () => {
  useEffect(() => {
    useChatStore.setState({ messages: FLOW, streaming: false, error: null });
    return () => useChatStore.setState({ messages: [], error: null });
  }, []);
  return (
    <div className="h-[680px] w-full overflow-hidden border border-titlebar shadow-input">
      <ChatPage />
    </div>
  );
};
FullChatPage.storyName = "完整聊天页面(装配 UI)";
