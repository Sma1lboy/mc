import type { AccountKind } from "../ipc/types";

/**
 * 账号类型 → 中文标签。各处(账号弹窗 / 右栏账号卡 / 经典启动账号卡)统一走这里,
 * 避免同一类型在不同界面出现 "微软" / "正版验证" 这种不一致叫法。
 */
const ACCOUNT_KIND_LABELS: Record<AccountKind, string> = {
  offline: "离线",
  microsoft: "微软",
  yggdrasil: "外置登录",
};

export function accountKindLabel(kind: AccountKind | string | undefined): string {
  if (!kind) return "";
  return ACCOUNT_KIND_LABELS[kind as AccountKind] ?? String(kind);
}
