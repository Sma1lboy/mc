import type { Story, StoryDefault } from "@ladle/react";
import {
  ModpackCard,
  ModpackListItem,
  InstanceRow,
  Button,
  type ModpackHit,
  type InstanceRowData,
} from ".";

/* ============================================================================
 * cards.stories —— 复合卡片 / 行的隔离预览(Ladle)。
 *
 * ModpackCard / ModpackListItem 是纯 props(喂 mock ModpackHit)。InstanceRow 会
 * 订阅 store 切片(运行 / 更新 / 社交开关),但只读默认值即可渲染;⋮ 菜单与删除
 * 确认弹窗的动作在无后端下点了也不崩(回调是 no-op)。
 * ========================================================================== */

export default {
  title: "Components / Cards & Rows",
} satisfies StoryDefault;

// —— mock 数据 ————————————————————————————————————————————————————

const HIT_FULL: ModpackHit = {
  id: "AABBCCDD",
  slug: "create-above-and-beyond",
  title: "Create: Above and Beyond",
  description:
    "一套以 Create 机械动力为核心的科技进度整合包,从采集到自动化一条龙,配套任务书引导。",
  author: "simibubi",
  downloads: 4820000,
  gallery_url:
    "https://cdn.modrinth.com/data/placeholder/images/cover.png", // 网络图,Ladle 里取不到会走占位
  categories: ["technology", "adventure", "quests"],
};

const HIT_NO_IMAGE: ModpackHit = {
  id: "EEFF0011",
  slug: "vanilla-plus",
  title: "Vanilla Plus",
  description: "轻量原版增强,只加实用改进,不改变核心玩法。",
  author: "someone",
  downloads: 12800,
  categories: ["utility", "optimization"],
};

const INSTANCE_BASE: InstanceRowData = {
  id: "inst-1",
  name: "科技生存 1.20.1",
  mc_version: "1.20.1",
  loader: "fabric",
  loader_version: "0.15.7",
  last_played: Date.now() - 1000 * 60 * 5,
  running: false,
};

// —— ModpackCard ———————————————————————————————————————————————————

export const CardWithCover: Story = () => (
  <div className="max-w-[320px]">
    <ModpackCard hit={HIT_FULL} onClick={() => {}} />
  </div>
);
CardWithCover.storyName = "ModpackCard · 带封面(网络图回退占位)";

export const CardNoCover: Story = () => (
  <div className="max-w-[320px]">
    <ModpackCard hit={HIT_NO_IMAGE} onClick={() => {}} />
  </div>
);
CardNoCover.storyName = "ModpackCard · 无封面(草方块 + 首字母)";

export const CardGrid: Story = () => (
  <div className="grid grid-cols-2 gap-[16px]">
    <ModpackCard hit={HIT_FULL} onClick={() => {}} />
    <ModpackCard hit={HIT_NO_IMAGE} onClick={() => {}} />
  </div>
);
CardGrid.storyName = "ModpackCard · 网格布局";

// —— ModpackListItem ——————————————————————————————————————————————

export const ListItems: Story = () => (
  <div className="flex flex-col gap-[10px]">
    <ModpackListItem hit={HIT_FULL} onClick={() => {}} />
    <ModpackListItem
      hit={HIT_NO_IMAGE}
      onClick={() => {}}
      action={<Button variant="ghost">安装</Button>}
    />
    <ModpackListItem hit={HIT_FULL} onClick={() => {}} progress={0.4} />
    <ModpackListItem hit={HIT_NO_IMAGE} onClick={() => {}} progress={null} />
  </div>
);
ListItems.storyName = "ModpackListItem · 普通 / 带操作 / 进度";

// —— InstanceRow ——————————————————————————————————————————————————

export const InstanceRows: Story = () => (
  <div className="flex flex-col gap-[10px]">
    <InstanceRow instance={INSTANCE_BASE} onPlay={() => {}} onOpen={() => {}} />
    <InstanceRow
      instance={{ ...INSTANCE_BASE, id: "inst-2", name: "魔法冒险", loader: "forge", last_played: 0 }}
      onPlay={() => {}}
    />
    <InstanceRow
      instance={{
        ...INSTANCE_BASE,
        id: "inst-3",
        name: "带用户标签的实例",
        tags: ["主力", "光影"],
      }}
      onPlay={() => {}}
    />
  </div>
);
InstanceRows.storyName = "InstanceRow · 常规 / 从未游玩 / 带标签";

export const InstanceRowSelectable: Story = () => (
  <div className="flex flex-col gap-[10px]">
    <InstanceRow instance={INSTANCE_BASE} selectable selected onToggleSelect={() => {}} />
    <InstanceRow
      instance={{ ...INSTANCE_BASE, id: "inst-2", name: "未选中的实例" }}
      selectable
      selected={false}
      onToggleSelect={() => {}}
    />
  </div>
);
InstanceRowSelectable.storyName = "InstanceRow · 多选模式(选中 / 未选)";
