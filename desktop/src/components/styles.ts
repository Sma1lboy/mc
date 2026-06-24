// 共享类名常量:同语义的内联 class 串集中一处,避免各文件各写一份、改一处漏一处导致漂移。

/** accent 紧凑操作按钮(列表行内「添加 / 安装 / 更新」,28px 高)。 */
export const ACCENT_BTN_COMPACT =
  "shrink-0 h-[28px] px-[12px] rounded-ctl bg-a-4 text-white text-[12px] font-semibold cursor-pointer " +
  "transition-opacity duration-[var(--dur)] ease-app hover:opacity-90 disabled:opacity-50 disabled:cursor-default";

/** accent 标准操作按钮(空态 CTA 等,34px 高)。 */
export const ACCENT_BTN =
  "h-[34px] px-[16px] rounded-ctl bg-a-4 text-white text-[13px] font-semibold cursor-pointer " +
  "transition-opacity duration-[var(--dur)] ease-app hover:opacity-90 disabled:opacity-50 disabled:cursor-default";

/**
 * 加载器图标徽标的品牌底色(按 data-loader 属性着色,与主题无关的固定品牌色)。
 * 追加在基础底色后即可(vanilla 默认用 bg-a-4)。
 * 集中一处,避免 forge/fabric 等色值在多页内联重复、改一处漏三处。
 */
export const LOADER_BADGE_TINT =
  "data-[loader=forge]:bg-[#c96a1c] data-[loader=neoforge]:bg-[#c96a1c] " +
  "data-[loader=fabric]:bg-[#a87b3f] data-[loader=quilt]:bg-[#a87b3f]";
