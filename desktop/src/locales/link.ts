// "link" 命名空间词条:kobeMC 账号下拉里的「关联账号」区(把微软身份绑到 kobeMC 用户)。
// zh 为真相源;en 缺项自动回退中文。
const dict = {
  zh: {
    title: "关联账号",
    hint: "把你的微软账号绑定到当前 kobeMC 账号,服务端据此识别你的正版身份。",
    providerMicrosoft: "微软",
    providerCredential: "邮箱密码",
    bind: "绑定",
    unlink: "解绑",
    alreadyLinked: "已绑定",
    noMsAccount: "还没有微软账号,请先在「账号」里添加一个微软账号。",
    linked: "已绑定微软账号",
    unlinked: "已解绑",
    opError: "操作失败:{{ err }}",
  } as Record<string, string>,
  en: {
    title: "Linked accounts",
    hint: "Bind your Microsoft account to this kobeMC account so the server can recognise your premium identity.",
    providerMicrosoft: "Microsoft",
    providerCredential: "Email & password",
    bind: "Link",
    unlink: "Unlink",
    alreadyLinked: "Linked",
    noMsAccount: "No Microsoft account yet — add one under Accounts first.",
    linked: "Microsoft account linked",
    unlinked: "Unlinked",
    opError: "Failed: {{ err }}",
  } as Record<string, string>,
};

export default dict;
