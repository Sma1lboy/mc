// format.ts —— 纯函数工具集。组件 / 页面共享的格式化逻辑。
// 设计原则: 无副作用, 无依赖 (不引入 dayjs 等, 保持 bundle 轻量, 契合 03 技术栈"小体积"目标)。

/**
 * 把一个 epoch 毫秒时间戳格式化为相对时间, 如 "5 minutes ago" / "just now" / "in 3 days"。
 * 用于 InstanceRow 的 "Played ... ago"。
 *
 * @param epochMs Unix 毫秒时间戳。允许传 0 / NaN / 负数 / 未来时间, 都做了兜底。
 * @returns 人类可读的相对时间字符串。
 */
export function formatRelativeTime(epochMs: number): string {
  // 兜底: 非法输入 (后端 last_played 可能为 0 表示"从未游玩")。
  if (!Number.isFinite(epochMs) || epochMs <= 0) return "never";

  const now = Date.now();
  const diffMs = now - epochMs;
  const future = diffMs < 0;
  // 用绝对值统一计算, 最后按方向拼前缀/后缀 (支持"in X" 与 "X ago")。
  const abs = Math.abs(diffMs);

  const sec = Math.floor(abs / 1000);
  if (sec < 45) return "just now";

  // 时间梯度表: [单位上限秒数, 该单位每个对应的秒数, 单数词]。
  // 命中第一个 value < 上限的梯度。
  const units: [limit: number, divisor: number, name: string][] = [
    [90, 60, "minute"], // < 90s → "a minute"
    [45 * 60, 60, "minute"], // < 45min → N minutes
    [90 * 60, 60 * 60, "hour"], // < 90min → "an hour"
    [22 * 3600, 3600, "hour"], // < 22h → N hours
    [36 * 3600, 86400, "day"], // < 36h → "a day"
    [25 * 86400, 86400, "day"], // < 25d → N days
    [45 * 86400, 86400 * 30, "month"], // < 45d → "a month"
    [320 * 86400, 86400 * 30, "month"], // < ~10.5mo → N months
    [548 * 86400, 86400 * 365, "year"], // < 1.5y → "a year"
    [Infinity, 86400 * 365, "year"], // else → N years
  ];

  for (const [limit, divisor, name] of units) {
    if (sec < limit) {
      const value = Math.round(sec / divisor);
      const label = value <= 1 ? `1 ${name}` : `${value} ${name}s`;
      return future ? `in ${label}` : `${label} ago`;
    }
  }
  // 理论不可达 (Infinity 兜底), 保险返回。
  return future ? "in the future" : "a long time ago";
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
