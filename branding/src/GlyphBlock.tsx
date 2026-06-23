import { AbsoluteFill, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion"
import { colors } from "./colors"

// 折中版 —— kobe 同款圆角像素瓷砖,里面放「像素草方块正面」(Minecraft 经典纹理:
// 上绿草、下陶土泥,带颗粒)。既是家族图标系统(瓷砖+像素+陶土光晕),又是 MC 方块 motif。
const monoStack = '"JetBrains Mono", "IBM Plex Mono", "SF Mono", Menlo, Consolas, ui-monospace, monospace'

// 8×8 草方块纹理。g/G/l = 绿(常/深/浅);t/d/L = 陶土(常/深/浅)。
// 上 2 行草,第 3 行草檐滴落,其余泥 + 颗粒。
const PATTERN: ReadonlyArray<string> = [
  "lgglgglg",
  "gGgglgGg",
  "gtgttgtt",
  "ttdttttL",
  "Ltttdttt",
  "tttLttdt",
  "tdtttLtt",
  "ttttdttt",
]

const CMAP: Record<string, string> = {
  g: colors.green,
  G: colors.greenDark,
  l: colors.greenLight,
  t: colors.terracotta,
  d: colors.terracottaDark,
  L: colors.terracottaLight,
}

const PIXEL = 40
const GAP = 5
const COLS = 8
const ROWS = PATTERN.length

export const GlyphBlock: React.FC = () => {
  const frame = useCurrentFrame()
  const { fps } = useVideoConfig()

  const tileSpring = spring({ frame, fps, config: { damping: 18, stiffness: 110 } })
  const tileScale = interpolate(tileSpring, [0, 1], [0.92, 1])
  const tileOpacity = interpolate(tileSpring, [0, 1], [0, 1])
  const glow = interpolate(frame % 90, [0, 45, 90], [0.35, 0.7, 0.35])

  return (
    <AbsoluteFill
      style={{ backgroundColor: colors.bgSoft, alignItems: "center", justifyContent: "center", fontFamily: monoStack }}
    >
      <div
        style={{
          width: 640,
          height: 640,
          borderRadius: 144,
          background: `linear-gradient(160deg, ${colors.panel}, ${colors.bg})`,
          boxShadow: "inset 0 2px 0 rgba(255,255,255,0.05), inset 0 -2px 0 rgba(0,0,0,0.4)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          transform: `scale(${tileScale})`,
          opacity: tileOpacity,
          position: "relative",
          overflow: "hidden",
        }}
      >
        <div
          style={{
            position: "absolute",
            inset: 0,
            background: `radial-gradient(circle at 50% 45%, ${colors.terracotta}${Math.floor(glow * 70)
              .toString(16)
              .padStart(2, "0")}, transparent 58%)`,
            pointerEvents: "none",
          }}
        />

        <div
          style={{
            display: "grid",
            gridTemplateColumns: `repeat(${COLS}, ${PIXEL}px)`,
            gridTemplateRows: `repeat(${ROWS}, ${PIXEL}px)`,
            gap: GAP,
            position: "relative",
            borderRadius: 10,
            overflow: "hidden",
          }}
        >
          {PATTERN.flatMap((row, rIdx) =>
            row.split("").map((ch, cIdx) => {
              const fill = CMAP[ch] ?? colors.terracotta
              const start = 8 + rIdx * 4
              const end = start + 10
              const reveal = interpolate(frame, [start, end], [0, 1], {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
              })
              return (
                <div
                  key={`${rIdx}-${cIdx}`}
                  style={{
                    background: fill,
                    opacity: reveal,
                    transform: `scale(${0.7 + reveal * 0.3})`,
                    boxShadow: "inset 0 2px 0 rgba(255,255,255,0.14), inset 0 -2px 0 rgba(0,0,0,0.18)",
                  }}
                />
              )
            }),
          )}
        </div>
      </div>
    </AbsoluteFill>
  )
}
