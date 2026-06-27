// "shortcuts" 命名空间词条:全局键盘快捷键 + 帮助浮层。zh 为真相源;en 缺项回退中文。
const dict = {
  zh: {
    title: "键盘快捷键",
    subtitle: "在输入框 / 文本域聚焦时不触发,可放心打字。",

    groupNav: "导航",
    groupLaunch: "启动",
    groupGeneral: "通用",

    navHome: "首页",
    navLibrary: "库",
    navDiscover: "发现",
    navSettings: "设置",

    launchRecent: "启动第 1–9 个最近游玩的实例",
    toggleHelp: "显示 / 隐藏本帮助",
    closeHelp: "关闭本帮助",

    close: "知道了",
    open: "键盘快捷键",
  } as Record<string, string>,
  en: {
    title: "Keyboard shortcuts",
    subtitle: "Won't fire while an input or textarea is focused — type freely.",

    groupNav: "Navigation",
    groupLaunch: "Launch",
    groupGeneral: "General",

    navHome: "Home",
    navLibrary: "Library",
    navDiscover: "Discover",
    navSettings: "Settings",

    launchRecent: "Launch the 1st–9th most recently played instance",
    toggleHelp: "Show / hide this help",
    closeHelp: "Close this help",

    close: "Got it",
    open: "Keyboard shortcuts",
  } as Record<string, string>,
};

export default dict;
