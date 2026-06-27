// "layout" 命名空间词条。zh 为真相源;en 缺项自动回退中文。
const dict = {
  zh: {
    navHome: "主页",
    navDiscover: "发现",
    navLibrary: "库",
    navSettings: "设置",
    primaryNav: "主导航",
    recentInstances: "最近实例",
    newInstance: "新增实例",
    loginOrAddAccount: "登录 / 添加账号",
    minimize: "最小化",
    close: "关闭",
    noInstanceRunning: "无实例运行",
    running: "{{ n }} 个运行中",
  } as Record<string, string>,
  en: {
    navHome: "Home",
    navDiscover: "Discover",
    navLibrary: "Library",
    navSettings: "Settings",
    primaryNav: "Primary navigation",
    recentInstances: "Recent instances",
    newInstance: "New instance",
    loginOrAddAccount: "Sign in / add account",
    minimize: "Minimize",
    close: "Close",
    noInstanceRunning: "No instance running",
    running: "{{ n }} running",
  } as Record<string, string>,
};

export default dict;
