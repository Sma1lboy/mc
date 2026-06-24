import { JSX, Show, mergeProps } from "solid-js";

/* ============================================================================
 * components/InstanceIcon.tsx —— 实例图标(自定义图 / MC 像素占位)
 *
 * 有自定义 icon → 直接渲染图片;否则按实例名/ID 生成一枚确定性的 Minecraft 风
 * 像素方块:8×8 网格、竖向对称(类 GitHub identicon),配 MC 方块色板(草、泥、
 * 石、钻石、红石…)。同名永远得到同一张图,不同名各异。用 SVG + crispEdges 出
 * 硬边像素感,跟随容器尺寸铺满。
 * ========================================================================== */

// MC 方块色板:每项 [亮色, 暗色] —— 暗色作底,亮色画像素,出「方块面」质感。
const PALETTES: [string, string][] = [
  ["#6aa84f", "#3e6b29"], // 草方块 grass
  ["#9c6b43", "#5e3d28"], // 泥土 dirt
  ["#8f8f8f", "#5c5c5c"], // 石头 stone
  ["#bb904f", "#7a5e34"], // 橡木 oak
  ["#54d6cb", "#2f9a92"], // 钻石 diamond
  ["#c0392b", "#8a2820"], // 红石 redstone
  ["#e2b652", "#a8842f"], // 金 gold
  ["#33c46f", "#1f8f4e"], // 绿宝石 emerald
  ["#4474d6", "#284c95"], // 青金石 lapis
  ["#9b59b6", "#6c3483"], // 紫水晶 amethyst
];

// 字符串 → 32 位无符号哈希(FNV-1a),确定性、与平台无关。
function hash32(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

// 由种子生成 8×8 竖向对称的像素位图 + 选定色板。
function pixelSprite(seed: string): { cells: boolean[]; light: string; dark: string } {
  const h = hash32(seed);
  const [light, dark] = PALETTES[h % PALETTES.length];
  // 取一个独立的位流决定每格亮灭:左半 4 列(8 行)= 32 位,正好一个哈希值。
  let bits = hash32(seed + "#pix");
  const cells: boolean[] = new Array(64).fill(false);
  for (let y = 0; y < 8; y++) {
    for (let x = 0; x < 4; x++) {
      const on = (bits & 1) === 1;
      bits >>>= 1;
      cells[y * 8 + x] = on;
      cells[y * 8 + (7 - x)] = on; // 镜像到右半
    }
  }
  return { cells, light, dark };
}

export interface InstanceIconProps {
  /** 实例名(优先)/ ID,作为像素图种子。 */
  name?: string;
  /** 自定义图标(文件 URL);存在则覆盖像素占位。 */
  icon?: string;
  /** 像素图无障碍标签;默认装饰性。 */
  alt?: string;
}

/**
 * <InstanceIcon name icon /> —— 铺满父容器(父负责尺寸/圆角/裁剪)。
 * 自定义图走 <img object-cover>;占位走 MC 像素 SVG。
 */
export function InstanceIcon(props: InstanceIconProps): JSX.Element {
  const merged = mergeProps({ alt: "" }, props);
  const seed = () => merged.name?.trim() || "?";
  const sprite = () => pixelSprite(seed());

  return (
    <Show
      when={merged.icon}
      fallback={
        <svg
          viewBox="0 0 8 8"
          class="w-full h-full block"
          shape-rendering="crispEdges"
          preserveAspectRatio="none"
          role={merged.alt ? "img" : undefined}
          aria-hidden={merged.alt ? undefined : "true"}
          aria-label={merged.alt || undefined}
        >
          <rect x="0" y="0" width="8" height="8" fill={sprite().dark} />
          {sprite().cells.map((on, i) =>
            on ? (
              <rect x={i % 8} y={Math.floor(i / 8)} width="1" height="1" fill={sprite().light} />
            ) : null,
          )}
        </svg>
      }
    >
      <img src={merged.icon} alt={merged.alt} loading="lazy" class="w-full h-full object-cover block" />
    </Show>
  );
}

export default InstanceIcon;
