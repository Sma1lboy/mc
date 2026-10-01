// "account" 命名空间词条。zh 为真相源;en 缺项自动回退中文。
const dict = {
  zh: {
    // 弹窗标题(随步骤切换)
    titleAdd: "添加账号",
    titleOffline: "离线登录",
    titleMsa: "微软登录",
    titleYggdrasil: "外置登录",
    close: "关闭",

    // 已有账号列表
    listLoadFailed: "账号列表加载失败",
    removeAccount: "移除账号",
    remove: "移除",
    orAddNew: "或添加新账号:",

    // 登录方式卡片
    msaTitle: "微软账号",
    msaDesc: "使用拥有 Minecraft Java 版的微软账号",
    offlineTitle: "离线账号",
    offlineDesc: "仅输入用户名,单机游玩",
    yggdrasilTitle: "外置登录",
    yggdrasilDesc: "第三方皮肤站(LittleSkin 等),自动注入 authlib-injector",

    // 微软设备码
    fetchingCode: "正在获取登录代码…",
    msaInstruction: "在微软登录页输入以下代码,然后按网页提示完成登录。",
    copyCode: "复制登录代码",
    codeCopied: "已复制登录代码",
    codeCopyFailed: "未能复制。请选中上方代码,手动复制。",
    pageOpenFailed: "未能打开浏览器。请手动访问上方验证地址。",
    retryMsa: "重新获取代码",
    yggBaseLabel: "皮肤站 API 地址",
    yggUserLabel: "邮箱或用户名",
    yggPassLabel: "密码",
    verificationUri: "验证地址:",
    waitingAuth: "等待你在浏览器中完成授权…",

    // 离线表单
    offlineNameLabel: "离线用户名",
    offlineNamePlaceholder: "输入用户名,例如 Steve…",

    // 外置登录表单
    yggBasePlaceholder: "皮肤站 API 地址,如 https://littleskin.cn/api/yggdrasil",
    yggUserPlaceholder: "邮箱 / 用户名",
    yggPassPlaceholder: "密码",

    // 按钮
    back: "返回",
    addOffline: "添加离线账号",
    adding: "添加中…",
    login: "登录",
    loggingIn: "登录中…",

    // toast
    removed: "已移除账号:{{ name }}",
    removeFailed: "移除失败:{{ err }}",
    loggedIn: "已登录:{{ name }}",
    offlineAdded: "已添加离线账号:{{ name }}",
    loggedInYggdrasil: "已登录(外置):{{ name }}",

    // 微软 client_id 错误说明
    msaClientIdError:
      "此版本的微软登录配置不可用。请从官方发布页检查新版,或向维护者报告下方错误。自行构建时,检查 ",
    msaClientIdErrorMid: " 的 ",
    msaClientIdErrorEnd: ",重启应用后再试。",

    // ContextBar
    sectionCurrent: "当前账号",
    sectionFriends: "好友",
    sectionNews: "动态",
    contextAria: "上下文信息",
    contextLoadFailed: "账号载入失败",
    noAccount: "尚未添加账号",
    noAccountHint: "登录微软正版,或添加一个离线账号",
    loginOrAdd: "登录 / 添加账号",
    switchAccount: "切换账号",
    switchFailed: "切换账号失败",
    friendsEmpty: "暂无好友",
    friendsHint: "联机/社交功能开发中",
    newsEmpty: "暂无动态",
    newsHint: "敬请期待",
  } as Record<string, string>,
  en: {
    titleAdd: "Add Account",
    titleOffline: "Offline Login",
    titleMsa: "Microsoft Login",
    titleYggdrasil: "External Login",
    close: "Close",

    listLoadFailed: "Failed to load account list",
    removeAccount: "Remove account",
    remove: "Remove",
    orAddNew: "Or add a new account:",

    msaTitle: "Microsoft Account",
    msaDesc: "Use a Microsoft account that owns Minecraft Java Edition",
    offlineTitle: "Offline Account",
    offlineDesc: "Just a username, singleplayer only",
    yggdrasilTitle: "External Login",
    yggdrasilDesc: "Third-party skin sites (LittleSkin, etc.) — authlib-injector auto-injected",

    fetchingCode: "Fetching login code…",
    msaInstruction: "Enter this code on the Microsoft sign-in page, then follow the steps there.",
    copyCode: "Copy sign-in code",
    codeCopied: "Sign-in code copied",
    codeCopyFailed: "Couldn't copy. Select the code above and copy it manually.",
    pageOpenFailed: "Couldn't open your browser. Visit the verification URL above manually.",
    retryMsa: "Get a new code",
    yggBaseLabel: "Skin site API URL",
    yggUserLabel: "Email or username",
    yggPassLabel: "Password",
    verificationUri: "Verification URL: ",
    waitingAuth: "Waiting for you to authorize in the browser…",

    offlineNameLabel: "Offline username",
    offlineNamePlaceholder: "Enter a username, e.g. Steve…",

    yggBasePlaceholder: "Skin site API URL, e.g. https://littleskin.cn/api/yggdrasil",
    yggUserPlaceholder: "Email / username",
    yggPassPlaceholder: "Password",

    back: "Back",
    addOffline: "Add offline account",
    adding: "Adding…",
    login: "Login",
    loggingIn: "Logging in…",

    removed: "Removed account: {{ name }}",
    removeFailed: "Remove failed: {{ err }}",
    loggedIn: "Logged in: {{ name }}",
    offlineAdded: "Offline account added: {{ name }}",
    loggedInYggdrasil: "Logged in (external): {{ name }}",

    msaClientIdError:
      "Microsoft sign-in is unavailable in this build. Check the official releases for an update or report the error below to the maintainer. If you built the app yourself, check ",
    msaClientIdErrorMid: " — the ",
    msaClientIdErrorEnd: " field — then restart the app and try again.",

    sectionCurrent: "Playing as",
    sectionFriends: "Friends",
    sectionNews: "News",
    contextAria: "Context info",
    contextLoadFailed: "Failed to load accounts",
    noAccount: "No account yet",
    noAccountHint: "Log in with Microsoft, or add an offline account",
    loginOrAdd: "Log in / add account",
    switchAccount: "Switch account",
    switchFailed: "Failed to switch account",
    friendsEmpty: "No friends yet",
    friendsHint: "Multiplayer/social features in progress",
    newsEmpty: "No news yet",
    newsHint: "Coming soon",
  } as Record<string, string>,
};

export default dict;
