// format.ts —— 纯函数工具集。组件 / 页面共享的格式化逻辑。
// 设计原则: 无副作用, 无依赖 (不引入 dayjs 等, 保持 bundle 轻量, 契合 03 技术栈"小体积"目标)。

/**
 * 把一个 epoch 毫秒时间戳格式化为中文相对时间, 如 "5 分钟前" / "刚刚" / "3 天后"。
 * 用于 InstanceRow / InstanceDetail 的 "上次 ..." 元信息。
 *
 * @param epochMs Unix 毫秒时间戳。允许传 0 / NaN / 负数 / 未来时间, 都做了兜底。
 * @returns 中文相对时间字符串;非法/为 0 时返回哨兵 "never"(由调用方渲染为"从未游玩")。
 */
export function formatRelativeTime(epochMs: number): string {
  // 兜底: 非法输入 (后端 last_played 可能为 0 表示"从未游玩")。
  if (!Number.isFinite(epochMs) || epochMs <= 0) return "never";

  const now = Date.now();
  const diffMs = now - epochMs;
  const future = diffMs < 0;
  // 用绝对值统一计算, 最后按方向拼后缀 (前/后)。
  const abs = Math.abs(diffMs);

  const sec = Math.floor(abs / 1000);
  const suffix = future ? "后" : "前";
  if (sec < 45) return "刚刚";

  // 时间梯度表: [单位上限秒数, 该单位每个对应的秒数, 中文单位]。
  // 命中第一个 value < 上限的梯度;中文无单复数, 直接 "N 单位前/后"。
  const units: [limit: number, divisor: number, name: string][] = [
    [45 * 60, 60, "分钟"], // < 45min → N 分钟
    [22 * 3600, 3600, "小时"], // < 22h → N 小时
    [25 * 86400, 86400, "天"], // < 25d → N 天
    [320 * 86400, 86400 * 30, "个月"], // < ~10.5mo → N 个月
    [Infinity, 86400 * 365, "年"], // else → N 年
  ];

  for (const [limit, divisor, name] of units) {
    if (sec < limit) {
      const value = Math.max(1, Math.round(sec / divisor));
      return `${value} ${name}${suffix}`;
    }
  }
  // 理论不可达 (Infinity 兜底), 保险返回。
  return future ? "未来" : "很久以前";
}

/**
 * 把一个整数缩写成带 k/M/B 后缀的紧凑字符串, 如 2_650_000 -> "2.65M", 461_400 -> "461.4k"。
 * 用于 ModpackCard 的下载数显示。
 *
 * 规则:
 *  - < 1000 直接显示整数。
 *  - 1k ~ 999.9k 用 "k", 1M+ 用 "M", 1B+ 用 "B"。
 *  - 有效数字控制在 3 位左右 (2.65M / 46.1k / 461k), 避免 "461.40k" 这种冗余。
 *
 * @param n 任意非负数 (负数取绝对值再加负号; 非法值返回 "0")。
 */
export function formatCount(n: number): string {
  if (!Number.isFinite(n)) return "0";
  const negative = n < 0;
  let value = Math.abs(n);

  const sign = negative ? "-" : "";

  if (value < 1000) {
    // 小数值直接整数显示。
    return sign + String(Math.round(value));
  }

  const units: [threshold: number, suffix: string][] = [
    [1_000_000_000, "B"],
    [1_000_000, "M"],
    [1_000, "k"],
  ];

  for (const [threshold, suffix] of units) {
    if (value >= threshold) {
      const scaled = value / threshold;
      // 动态小数位: 让总有效位约为 3。
      //  scaled >= 100 (如 461.4k) → 1 位小数 "461.4"
      //  scaled >= 10  (如 46.12k) → 1 位小数 "46.1"
      //  scaled <  10  (如 2.654M) → 2 位小数 "2.65"
      let digits: number;
      if (scaled >= 100) digits = 1;
      else if (scaled >= 10) digits = 1;
      else digits = 2;

      let str = scaled.toFixed(digits);
      // 去掉尾随 0 与多余小数点, 如 "2.00" -> "2", "461.0" -> "461"。
      str = str.replace(/\.?0+$/, "");
      return sign + str + suffix;
    }
  }
  return sign + String(Math.round(value));
}
