// "agent" 命名空间词条:整合包助手流式对话页。zh 为真相源;en 缺项自动回退中文。
const dict = {
  zh: {
    title: "整合包助手",
    subtitle: "描述你想要的整合包,助手会帮你搜索、挑选与配置。",
    newChat: "新对话",
    placeholder: "描述你想要的整合包(Enter 发送,Shift+Enter 换行)",
    send: "发送",
    streaming: "思考中…",
    reasoning: "思考过程",
    toolCall: "调用工具",
    toolArgs: "点击查看参数",
    copyCode: "复制",
    copied: "已复制",
    copyFailed: "复制失败",
    emptyTitle: "开始和整合包助手对话",
    emptyHint: "例如:「帮我找一个适合多人生存的科技整合包」。",
    brainLabel: "大脑",
    brainRust: "Rust",
    brainTs: "TS",

    // —— 上下文入口(发现页 / 新建实例)——
    discoverCta: "AI 帮我组整合包",
    discoverEmptyCta: "找不到?让 AI 帮你组一个",
    newInstanceCta: "或者:让 AI 生成整合包",
    // 提示词片段(拼成一句自然语言诉求,空字段优雅省略)。
    promptVersion: "Minecraft {{ version }}",
    promptLoader: "{{ loader }} 加载器",
    promptJoin: "、",
    promptConstraints: "(目标:{{ specs }})",
    discoverPrompt: "帮我做一个整合包:{{ query }}{{ constraints }}",
    discoverPromptOpen: "帮我推荐一个耐玩的整合包{{ constraints }}",
    instancePrompt: "帮我为新建的实例生成一个整合包{{ constraints }}",
  } as Record<string, string>,
  en: {
    title: "Modpack Assistant",
    subtitle: "Describe the modpack you want — the assistant searches, picks and configures it for you.",
    newChat: "New chat",
    placeholder: "Describe the modpack you want (Enter to send, Shift+Enter for a newline)",
    send: "Send",
    streaming: "Thinking…",
    reasoning: "Reasoning",
    toolCall: "Tool call",
    toolArgs: "Click to view arguments",
    copyCode: "Copy",
    copied: "Copied",
    copyFailed: "Copy failed",
    emptyTitle: "Start chatting with the Modpack Assistant",
    emptyHint: "e.g. \"Find me a tech modpack that's good for multiplayer survival.\"",
    brainLabel: "Brain",
    brainRust: "Rust",
    brainTs: "TS",

    // —— Contextual entry points (Discover / New instance) ——
    discoverCta: "Ask AI to build a pack",
    discoverEmptyCta: "Can't find it? Let AI build one",
    newInstanceCta: "Or: let AI build a modpack",
    // Prompt fragments (assembled into one natural-language ask; empty fields drop out).
    promptVersion: "Minecraft {{ version }}",
    promptLoader: "{{ loader }} loader",
    promptJoin: ", ",
    promptConstraints: " (target: {{ specs }})",
    discoverPrompt: "Help me build a modpack: {{ query }}{{ constraints }}",
    discoverPromptOpen: "Recommend and build a modpack worth playing{{ constraints }}",
    instancePrompt: "Build a modpack for my new instance{{ constraints }}",
  } as Record<string, string>,
};

export default dict;
