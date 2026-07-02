import type { Story, StoryDefault } from "@ladle/react";
import type { UIMessage } from "ai";
import { Panel } from "../components";
import { AskUserOptions } from "./AskUserOptions";
import { AssistantText, ActivityGroup, ToolChip, type Part, type ToolPart } from "./ChatParts";

/* ============================================================================
 * agentChat.stories —— 整合包助手对话 UI 的隔离预览(Ladle)。
 *
 * 目的:脱离 Tauri 后端,只喂 mock 的 UIMessage / ToolUIPart,单独渲染每个碎片与
 * AskUserOptions 的每个状态,便于纯视觉调参。提交等接后端的动作在 Ladle 里无后端,
 * AskUserOptions 内部走 submitAskUserAnswer(在非 Tauri 环境仅打日志/静默失败),不崩即可。
 * ========================================================================== */

export default {
  title: "Agent / Chat",
} satisfies StoryDefault;

// —— mock 构造工具 ——————————————————————————————————————————————————

// ToolPart 是按 state 判别的联合(不同 state 要求不同字段);mock 里字段是运行时给全的,
// 类型层用 unknown 桥接,避免为每个 state 手写精确形状。
type ToolOver = Record<string, unknown>;

/** 造一个 ask_user_question 工具 part(可覆盖 state / input / output)。 */
function askPart(over: ToolOver): ToolPart {
  return {
    type: "tool-ask_user_question",
    toolCallId: "call_ask_1",
    state: "input-available",
    ...over,
  } as unknown as ToolPart;
}

/** 造一个普通工具 part(用于工具芯片 / 活动块)。 */
function toolPart(over: ToolOver): ToolPart {
  return {
    type: "tool-search_mods",
    toolCallId: "call_tool_1",
    state: "output-available",
    input: {},
    output: {},
    ...over,
  } as unknown as ToolPart;
}

const SINGLE_OPTIONS = [
  { id: "tech", label: "科技(Tech)", description: "机械动力、应用能源等自动化科技模组" },
  { id: "magic", label: "魔法(Magic)", description: "神秘时代、植物魔法等魔法向模组" },
  { id: "adventure", label: "冒险 / RPG", description: "地牢、任务、Boss 与探索为主" },
];

const MULTI_OPTIONS = [
  { id: "shaders", label: "光影支持", description: "内置 Iris + 一套推荐光影" },
  { id: "perf", label: "性能优化", description: "Sodium / Lithium / FerriteCore" },
  { id: "map", label: "小地图", description: "JourneyMap 或 Xaero's" },
  { id: "storage", label: "存储管理", description: "整理箱子与物流" },
];

// —— AskUserOptions 各状态 —————————————————————————————————————————————

/** input-streaming:参数还在流,渲染骨架。 */
export const AskInputStreaming: Story = () => (
  <AskUserOptions
    msgId="m1"
    globalStreaming={true}
    part={askPart({ state: "input-streaming", input: {} })}
  />
);
AskInputStreaming.storyName = "AskUserOptions · input-streaming (骨架)";

/** input-available 单选:可点选,单选点一个即高亮。 */
export const AskSingleSelect: Story = () => (
  <AskUserOptions
    msgId="m1"
    globalStreaming={false}
    part={askPart({
      state: "input-available",
      input: {
        question: "你想要什么类型的整合包?",
        options: SINGLE_OPTIONS,
        multi_select: false,
      },
    })}
  />
);
AskSingleSelect.storyName = "AskUserOptions · input-available 单选";

/** input-available 多选:可勾多个 + 自定义输入 + 提交按钮。 */
export const AskMultiSelect: Story = () => (
  <AskUserOptions
    msgId="m1"
    globalStreaming={false}
    part={askPart({
      state: "input-available",
      input: {
        question: "还想要哪些附加特性?(可多选)",
        options: MULTI_OPTIONS,
        multi_select: true,
      },
    })}
  />
);
AskMultiSelect.storyName = "AskUserOptions · input-available 多选";

/** output-available:已作答,高亮回显用户选过的项(只读)。 */
export const AskAnswered: Story = () => (
  <AskUserOptions
    msgId="m1"
    globalStreaming={false}
    part={askPart({
      state: "output-available",
      input: {
        question: "还想要哪些附加特性?(可多选)",
        options: MULTI_OPTIONS,
        multi_select: true,
      },
      output: { selected: ["性能优化", "光影支持"] },
    })}
  />
);
AskAnswered.storyName = "AskUserOptions · output-available (已作答)";

// —— 工具芯片 —————————————————————————————————————————————————————

export const ToolChipStates: Story = () => (
  <div className="flex flex-col gap-[8px]">
    <ToolChip part={toolPart({ state: "input-streaming", input: { query: "create" } })} />
    <ToolChip
      part={toolPart({ state: "output-available", input: { query: "sodium", loader: "fabric" } })}
    />
    <ToolChip
      part={toolPart({
        type: "tool-add_mod",
        state: "output-error",
        input: { slug: "does-not-exist" },
        errorText: "找不到该模组:does-not-exist(404)",
      })}
    />
  </div>
);
ToolChipStates.storyName = "工具芯片 · 调用中 / 完成 / 出错";

// —— 折叠活动块 —————————————————————————————————————————————————————

const ACTIVITY_PARTS: Part[] = [
  { type: "reasoning", text: "先确认用户的目标版本与加载器,再搜索合适的性能与光影模组。" } as Part,
  toolPart({ toolCallId: "a1", type: "tool-search_mods", input: { query: "sodium" } }),
  toolPart({ toolCallId: "a2", type: "tool-add_mod", input: { slug: "sodium" } }),
];

export const ActivityCollapsed: Story = () => <ActivityGroup parts={ACTIVITY_PARTS} />;
ActivityCollapsed.storyName = "活动块 · 已完成(默认收起)";

export const ActivityRunning: Story = () => (
  <ActivityGroup
    parts={[
      { type: "reasoning", text: "正在搜索合适的科技模组…" } as Part,
      toolPart({ toolCallId: "r1", type: "tool-search_mods", state: "input-available", input: { query: "create" } }),
    ]}
  />
);
ActivityRunning.storyName = "活动块 · 进行中(自动展开)";

// —— 助手文本 —————————————————————————————————————————————————————

const MARKDOWN = `我为你挑了一套 **多人生存科技** 整合包思路:

- 核心:Create(机械动力)+ Applied Energistics 2
- 性能:Sodium / Lithium / FerriteCore
- 体验:JourneyMap 小地图、JEI 物品查询

\`\`\`json
{ "loader": "fabric", "mc": "1.20.1" }
\`\`\`

需要我直接开始搜索并添加这些模组吗?`;

export const AssistantMarkdown: Story = () => <AssistantText text={MARKDOWN} live={false} />;
AssistantMarkdown.storyName = "助手文本 · Markdown 渲染";

export const AssistantStreaming: Story = () => (
  <AssistantText text="正在为你规划整合包" live={true} />
);
AssistantStreaming.storyName = "助手文本 · 流式(尾部光标)";

// —— 完整助手消息(reasoning + tool + text 组合) —————————————————————————

const FULL_MESSAGE: UIMessage = {
  id: "assistant-1",
  role: "assistant",
  parts: [
    { type: "reasoning", text: "用户想要多人生存科技包,先搜核心科技模组,再补性能模组。" } as Part,
    toolPart({ toolCallId: "f1", type: "tool-search_mods", input: { query: "create" } }),
    toolPart({ toolCallId: "f2", type: "tool-add_mod", input: { slug: "create" } }),
    { type: "text", text: MARKDOWN } as Part,
  ],
};

/** 一条完整助手消息:活动块(思考+工具)收起在上,最终文本答案在下——贴近真实气泡。 */
export const FullAssistantMessage: Story = () => {
  const activity = FULL_MESSAGE.parts.slice(0, 3);
  const text = FULL_MESSAGE.parts[3];
  return (
    <div className="flex justify-start">
      <Panel variant="sunken" className="max-w-[85%] min-w-0 px-[14px] py-[11px]">
        <ActivityGroup parts={activity} />
        {text.type === "text" && <AssistantText text={text.text} live={false} />}
      </Panel>
    </div>
  );
};
FullAssistantMessage.storyName = "完整助手消息 · 活动块 + 文本";
