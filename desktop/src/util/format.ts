/** 通用展示格式化(此前 fmtSize / fmtDate 在多个页面各有一份,收拢为单一来源)。 */

/** 人类可读的字节大小;0 / null / 缺省返回空串。 */
export function fmtSize(bytes: number | null | undefined): string {
  if (!bytes) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let n = bytes;
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i += 1;
  }
  return `${n.toFixed(i > 0 && n < 10 ? 1 : 0)} ${units[i]}`;
}

/** ISO 时间 → 本地日期文本;无法解析时原样返回。 */
export function fmtDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString();
}
