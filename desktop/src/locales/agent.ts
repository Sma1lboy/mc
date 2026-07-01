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
    emptyTitle: "开始和整合包助手对话",
    emptyHint: "例如:「帮我找一个适合多人生存的科技整合包」。",
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
    emptyTitle: "Start chatting with the Modpack Assistant",
    emptyHint: "e.g. \"Find me a tech modpack that's good for multiplayer survival.\"",
  } as Record<string, string>,
};

export default dict;
