import { Composition } from "remotion"
import { IsoCube, IsoCubeTile, IsoCubeIcon } from "./IsoCube"
import { GlyphM } from "./GlyphM"
import { GlyphBlock } from "./GlyphBlock"

// 两个 composition:
//   iso-cube       —— logo mark,透明底(出 PNG 带透明 / 之后转内联 SVG 用在侧栏顶栏)
//   iso-cube-tile  —— app 图标:圆角深色块 + 方块(出图标 / 开屏动效)
export const RemotionRoot: React.FC = () => {
  return (
    <>
      <Composition id="iso-cube" component={IsoCube} durationInFrames={90} fps={30} width={512} height={512} />
      <Composition id="iso-cube-tile" component={IsoCubeTile} durationInFrames={90} fps={30} width={640} height={640} />
      <Composition id="iso-cube-icon" component={IsoCubeIcon} durationInFrames={90} fps={30} width={640} height={640} />
      <Composition id="glyph-m" component={GlyphM} durationInFrames={150} fps={30} width={800} height={800} />
      <Composition id="glyph-block" component={GlyphBlock} durationInFrames={150} fps={30} width={800} height={800} />
    </>
  )
}
