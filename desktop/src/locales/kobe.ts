// "kobe" 命名空间词条:kobeMC 后端账号登录 / 注册。zh 为真相源;en 缺项自动回退中文。
const dict = {
  zh: {
    title: "kobeMC 账号",
    subtitle: "登录我们自己的账号以使用临时领域(realms)与 mod 同步等服务端能力,和游戏内的 MC 账号相互独立。",
    emailPlaceholder: "邮箱",
    passwordPlaceholder: "密码",
    namePlaceholder: "昵称(可选)",
    loginAction: "登录",
    signupAction: "注册",
    logoutAction: "退出登录",
    working: "处理中…",
    switchToSignup: "还没有账号?去注册",
    switchToLogin: "已有账号?去登录",
    errEmptyCreds: "请填写邮箱和密码",
    errAuth: "操作失败:{{ err }}",
    "toast.loggedIn": "已登录 kobeMC:{{ name }}",
    "toast.signedUp": "已创建 kobeMC 账号:{{ name }}",
  } as Record<string, string>,
  en: {
    title: "kobeMC Account",
    subtitle:
      "Sign in to our own account to use temporary realms, mod sync and other server features — separate from your in-game Minecraft account.",
    emailPlaceholder: "Email",
    passwordPlaceholder: "Password",
    namePlaceholder: "Display name (optional)",
    loginAction: "Sign in",
    signupAction: "Sign up",
    logoutAction: "Sign out",
    working: "Working…",
    switchToSignup: "No account yet? Sign up",
    switchToLogin: "Already have an account? Sign in",
    errEmptyCreds: "Enter both email and password",
    errAuth: "Failed: {{ err }}",
    "toast.loggedIn": "Signed in to kobeMC: {{ name }}",
    "toast.signedUp": "Created kobeMC account: {{ name }}",
  } as Record<string, string>,
};

export default dict;
