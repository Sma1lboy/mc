import clsx from "clsx";
import { avatarTone, avatarInitial } from "../../util/avatar";
import { t } from "../../i18n";

/** 折叠区标题左侧的 caret(展开时旋转 90°)。 */
export function Caret({ open }: { open: boolean }) {
  return (
    <svg
      className={clsx("w-[10px] h-[10px] shrink-0 text-muted transition-transform duration-150", { "rotate-90": open })}
      viewBox="0 0 12 12"
      fill="none"
      aria-hidden="true"
    >
      <path d="M4 2.5 8 6 4 9.5" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

/** 成员 / 邀请头像方块瓦片(含可选在线 pip)。 */
export function PersonTile({ name, online, pip }: { name: string; online?: boolean; pip?: boolean }) {
  return (
    <span
      className={clsx(
        "relative w-[26px] h-[26px] shrink-0 grid place-items-center shadow-raised font-display text-[12px] text-[#1a1b12]",
        { "grayscale brightness-75": pip ? !online : false },
      )}
      style={{ backgroundColor: avatarTone(name) }}
      aria-hidden="true"
    >
      {avatarInitial(name)}
      {pip && (
        <span
          className={clsx(
            "absolute -right-[2px] -bottom-[2px] w-[8px] h-[8px] shadow-[0_0_0_2px_var(--color-panel)]",
            online ? "bg-accent" : "bg-faint",
          )}
        />
      )}
    </span>
  );
}

/** role 字符串 → 本地化标签。 */
export function roleLabel(role: string): string {
  return role === "owner"
    ? t("realm.roleOwner")
    : role === "admin"
      ? t("realm.roleAdmin")
      : t("realm.roleMember");
}

/**
 * LobbyBlock —— 领域里的「联机」块:一键开启 / 断开一个 EasyTier 虚拟局域网会话,运行时
 * 轮询状态展示本机虚拟 IP、在线对端与各自的「直连 / 中继 + 延迟」。开启需要管理员 / root
 * 授权(建 TUN);后端按平台提权拉起 easytier-core。EasyTier 未安装时后端返回清晰错误。
 */
