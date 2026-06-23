// 共享类名常量:同语义的内联 class 串集中一处,避免各文件各写一份、改一处漏一处导致漂移。

/** accent 紧凑操作按钮(列表行内「添加 / 安装 / 更新」,28px 高)。 */
export const ACCENT_BTN_COMPACT =
  "shrink-0 h-[28px] px-[12px] rounded-ctl bg-a-4 text-white text-[12px] font-semibold cursor-pointer " +
  "transition-opacity duration-[var(--dur)] ease-app hover:opacity-90 disabled:opacity-50 disabled:cursor-default";

/** accent 标准操作按钮(空态 CTA 等,34px 高)。 */
export const ACCENT_BTN =
  "h-[34px] px-[16px] rounded-ctl bg-a-4 text-white text-[13px] font-semibold cursor-pointer " +
  "transition-opacity duration-[var(--dur)] ease-app hover:opacity-90 disabled:opacity-50 disabled:cursor-default";
