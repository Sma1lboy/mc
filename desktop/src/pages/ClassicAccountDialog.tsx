// 经典布局的账号弹窗即共用的、主题中性的 AccountDialog —— 它用桥接令牌着色,在经典
// (浅色)布局下自动呈现蓝色/浅色样式,且自带账号切换 + 移除 + 添加。历史上这里曾有一份
// 经典专属实现,现已与工作台布局合并到 components/AccountDialog,避免两份登录逻辑漂移。
export { AccountDialog as default } from "../components/AccountDialog";
