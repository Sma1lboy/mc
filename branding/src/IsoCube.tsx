import { AbsoluteFill, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion"
import { colors } from "./colors"

// 等距体素方块 —— Minecraft 草方块语法重配色:顶面绿(草)、两侧陶土(土,主色)。
// 几何:200×200 视框内居中的标准等距立方体(7 个可见顶点,3 个菱形面)。
const P = {
  top: "100,20 178,64 100,108 22,64",
  left: "22,64 100,108 100,196 22,152",
  right: "178,64 100,108 100,196 178,152",
  // 草檐:两侧面顶部一条窄绿(草方块经典细节)
  leftGrass: "22,64 100,108 100,124 22,80",
  rightGrass: "178,64 100,108 100,124 178,80",
}

export const IsoCube: React.FC = () => {
  const frame = useCurrentFrame()
  const { fps } = useVideoConfig()

  // 整体弹入(轻微回弹)。
  const appear = spring({ frame, fps, config: { damping: 13, stiffness: 95 } })
  const scale = interpolate(appear, [0, 1], [0.62, 1])
  const baseOpacity = interpolate(appear, [0, 1], [0, 1])

  // 三面错峰组装(左 → 右 → 顶,顶面最后「落」上去)。
  const faceIn = (start: number) =>
    interpolate(frame, [start, start + 12], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" })
  const leftIn = faceIn(4)
  const rightIn = faceIn(9)
  const topIn = faceIn(15)
  const mIn = faceIn(24)

  // 组装完成后的轻微上下浮动。
  const settle = interpolate(frame, [28, 42], [0, 1], { extrapolateLeft: "clamp", extrapolateRight: "clamp" })
  const bob = Math.sin(frame / 22) * 5 * settle

  return (
    <AbsoluteFill style={{ alignItems: "center", justifyContent: "center", backgroundColor: "transparent" }}>
      <svg
        width="100%"
        height="100%"
        viewBox="20 17 160 182"
        style={{ transform: `translateY(${bob}px) scale(${scale})`, opacity: baseOpacity, overflow: "visible" }}
      >
        {/* 左面(陶土,主) */}
        <polygon
          points={P.left}
          fill={colors.terracotta}
          opacity={leftIn}
          style={{ transform: `translateX(${(1 - leftIn) * -22}px)`, transformBox: "fill-box", transformOrigin: "center" }}
        />
        {/* 右面(陶土深) */}
        <polygon
          points={P.right}
          fill={colors.terracottaDark}
          opacity={rightIn}
          style={{ transform: `translateX(${(1 - rightIn) * 22}px)`, transformBox: "fill-box", transformOrigin: "center" }}
        />
        {/* 草檐:侧面顶部窄绿 */}
        <polygon points={P.leftGrass} fill={colors.greenDark} opacity={Math.min(leftIn, topIn)} />
        <polygon points={P.rightGrass} fill={colors.green} opacity={Math.min(rightIn, topIn)} style={{ filter: "brightness(0.85)" }} />

        {/* 顶面(绿,草) */}
        <polygon
          points={P.top}
          fill={colors.green}
          opacity={topIn}
          style={{ transform: `translateY(${(1 - topIn) * -26}px)`, transformBox: "fill-box", transformOrigin: "center" }}
        />

        {/* 顶面高光边 + 前棱明暗,强化体素体积感 */}
        <polygon points={P.top} fill="none" stroke="rgba(255,255,255,0.40)" strokeWidth="1.4" strokeLinejoin="round" opacity={topIn} />
        <line x1="100" y1="108" x2="100" y2="196" stroke="rgba(255,255,255,0.14)" strokeWidth="1.4" opacity={Math.min(leftIn, rightIn)} />

        {/* 中央 M 徽标(品牌字母,奶白)——组装完成后淡入 */}
        <path
          d="M70,170 L70,130 L100,150 L130,130 L130,170"
          fill="none"
          stroke={colors.fg}
          strokeWidth={13}
          strokeLinejoin="round"
          strokeLinecap="round"
          opacity={mIn}
        />
      </svg>
    </AbsoluteFill>
  )
}

// 满幅应用图标:渐变底铺满整帧 + 居中方块(圆角交给系统遮罩,避免双层圆角)。
export const IsoCubeIcon: React.FC = () => {
  return (
    <AbsoluteFill style={{ background: `linear-gradient(160deg, ${colors.panel}, ${colors.bg})` }}>
      <IsoCube />
    </AbsoluteFill>
  )
}

// app 图标:圆角深色块衬底 + 方块,用于桌面图标 / 开屏。
export const IsoCubeTile: React.FC = () => {
  const frame = useCurrentFrame()
  const { fps } = useVideoConfig()
  const appear = spring({ frame, fps, config: { damping: 16, stiffness: 110 } })
  const scale = interpolate(appear, [0, 1], [0.9, 1])
  const opacity = interpolate(appear, [0, 1], [0, 1])

  return (
    <AbsoluteFill style={{ alignItems: "center", justifyContent: "center", backgroundColor: colors.bgSoft }}>
      <div
        style={{
          width: 560,
          height: 560,
          borderRadius: 128,
          background: `linear-gradient(160deg, ${colors.panel}, ${colors.bg})`,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          transform: `scale(${scale})`,
          opacity,
          boxShadow: "inset 0 2px 0 rgba(255,255,255,0.06), inset 0 -3px 0 rgba(0,0,0,0.4)",
          position: "relative",
        }}
      >
        <div style={{ position: "absolute", inset: 0, display: "flex", alignItems: "center", justifyContent: "center" }}>
          <IsoCube />
        </div>
      </div>
    </AbsoluteFill>
  )
}
