import { AbsoluteFill, interpolate, spring, useCurrentFrame, useVideoConfig } from "remotion"
import { colors } from "./colors"

// GlyphM —— 镜像 kobe 的 GlyphK 语法:圆角深色瓷砖 + 粗像素字形,逐行点亮、陶土光晕。
// 与 kobe 是同一套 app-icon 系统(一眼同组织);字母换成 M,中间 V 形用绿(MC 身份)。
// 像素值:0 空 / 1 陶土(主)/ 2 绿(口音)。7×7 的 M:外腿+顶角陶土,内 V 绿。
const M: ReadonlyArray<ReadonlyArray<0 | 1 | 2>> = [
  [1, 0, 0, 0, 0, 0, 1],
  [1, 2, 0, 0, 0, 2, 1],
  [1, 0, 2, 0, 2, 0, 1],
  [1, 0, 0, 2, 0, 0, 1],
  [1, 0, 0, 0, 0, 0, 1],
  [1, 0, 0, 0, 0, 0, 1],
  [1, 0, 0, 0, 0, 0, 1],
]

const PIXEL = 46
const GAP = 6
const COLS = M[0].length
const ROWS = M.length

const monoStack = '"JetBrains Mono", "IBM Plex Mono", "SF Mono", Menlo, Consolas, ui-monospace, monospace'

export const GlyphM: React.FC = () => {
  const frame = useCurrentFrame()
  const { fps } = useVideoConfig()

  const tileSpring = spring({ frame, fps, config: { damping: 18, stiffness: 110 } })
  const tileScale = interpolate(tileSpring, [0, 1], [0.92, 1])
  const tileOpacity = interpolate(tileSpring, [0, 1], [0, 1])

  // 稳定后光晕轻微呼吸。
  const glow = interpolate(frame % 90, [0, 45, 90], [0.35, 0.7, 0.35])

  return (
    <AbsoluteFill
      style={{
        backgroundColor: colors.bgSoft,
        alignItems: "center",
        justifyContent: "center",
        fontFamily: monoStack,
      }}
    >
      <div
        style={{
          width: 640,
          height: 640,
          borderRadius: 144,
          background: `linear-gradient(160deg, ${colors.panel}, ${colors.bg})`,
          boxShadow: [
            "inset 0 2px 0 rgba(255,255,255,0.05)",
            "inset 0 -2px 0 rgba(0,0,0,0.4)",
          ].join(", "),
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          transform: `scale(${tileScale})`,
          opacity: tileOpacity,
          position: "relative",
          overflow: "hidden",
        }}
      >
        {/* 陶土光晕(与 kobe 同) */}
        <div
          style={{
            position: "absolute",
            inset: 0,
            background: `radial-gradient(circle at 50% 50%, ${colors.terracotta}${Math.floor(glow * 80)
              .toString(16)
              .padStart(2, "0")}, transparent 55%)`,
            pointerEvents: "none",
          }}
        />

        {/* 像素 M */}
        <div
          style={{
            display: "grid",
            gridTemplateColumns: `repeat(${COLS}, ${PIXEL}px)`,
            gridTemplateRows: `repeat(${ROWS}, ${PIXEL}px)`,
            gap: GAP,
            position: "relative",
          }}
        >
          {M.flatMap((row, rIdx) =>
            row.map((cell, cIdx) => {
              if (cell === 0) return <div key={`${rIdx}-${cIdx}`} />
              const start = 8 + rIdx * 4
              const end = start + 10
              const reveal = interpolate(frame, [start, end], [0, 1], {
                extrapolateLeft: "clamp",
                extrapolateRight: "clamp",
              })
              const isGreen = cell === 2
              const fill = isGreen ? colors.green : colors.terracotta
              const glowColor = isGreen ? colors.green : colors.terracotta
              return (
                <div
                  key={`${rIdx}-${cIdx}`}
                  style={{
                    background: fill,
                    borderRadius: 6,
                    opacity: reveal,
                    transform: `scale(${0.6 + reveal * 0.4})`,
                    boxShadow: `inset 0 2px 0 rgba(255,255,255,0.18), 0 0 ${8 + glow * 14}px ${glowColor}80`,
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
