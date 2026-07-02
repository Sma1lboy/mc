import { useState } from "react";
import type { Story, StoryDefault } from "@ladle/react";
import {
  Button,
  Panel,
  Heading,
  PixelLabel,
  Checkbox,
  Toggle,
  Segmented,
  Slider,
  Chip,
  Tag,
  Spinner,
  EmptyState,
  ErrorState,
  Icon,
  ModpackCard,
  ModpackListItem,
  InstanceRow,
  type ModpackHit,
  type InstanceRowData,
} from ".";

/* ============================================================================
 * overview.stories —— 组件总览(kitchen-sink):一页滚动铺开所有展示型组件,
 * 方便一眼扫过整套设计系统 / 对比风格。逐组件的各状态见其它 stories。
 * ========================================================================== */

export default {
  title: "Components / Overview",
} satisfies StoryDefault;

const HIT: ModpackHit = {
  id: "AABBCCDD",
  slug: "create-above-and-beyond",
  title: "Create: Above and Beyond",
  description: "以 Create 机械动力为核心的科技进度整合包,采集到自动化一条龙。",
  author: "simibubi",
  downloads: 4820000,
  categories: ["technology", "quests"],
};

const INSTANCE: InstanceRowData = {
  id: "inst-1",
  name: "科技生存 1.20.1",
  mc_version: "1.20.1",
  loader: "fabric",
  loader_version: "0.15.7",
  last_played: Date.now() - 1000 * 60 * 5,
  running: false,
};

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="flex flex-col gap-[10px]">
      <Heading size="section" as="h2">{title}</Heading>
      <div className="flex flex-wrap items-start gap-[12px]">{children}</div>
    </section>
  );
}

export const AllComponents: Story = () => {
  const [chk, setChk] = useState(true);
  const [tog, setTog] = useState(true);
  const [tab, setTab] = useState<"mods" | "worlds">("mods");
  const [mem, setMem] = useState(4096);

  return (
    <div className="flex flex-col gap-[28px] max-w-[900px]">
      <Section title="按钮 Button">
        <Button variant="primary"><Icon name="power" size={15} />启动</Button>
        <Button variant="ghost"><Icon name="download" size={15} />下载</Button>
        <Button variant="danger">删除</Button>
        <Button variant="primary" disabled>禁用</Button>
      </Section>

      <Section title="面板 Panel(倒角)">
        <Panel variant="sunken" className="px-[16px] py-[12px] text-[13px]">sunken</Panel>
        <Panel variant="raised" className="px-[16px] py-[12px] text-[13px]">raised</Panel>
        <Panel variant="input" className="px-[16px] py-[12px] text-[13px]">input</Panel>
        <Panel variant="raised" stone className="px-[16px] py-[12px] text-[13px]">stone</Panel>
      </Section>

      <Section title="排版 Typography">
        <Heading size="page" as="h1">Page 标题</Heading>
        <Heading size="section" as="h2">Section 标题</Heading>
        <PixelLabel className="bg-accent text-accent-text px-[8px] py-[4px]">PIXEL</PixelLabel>
      </Section>

      <Section title="表单控件 Controls">
        <Checkbox checked={chk} onChange={setChk} label="性能优化" />
        <Toggle checked={tog} onChange={setTog} title="开关" />
        <Segmented
          value={tab}
          onChange={setTab}
          options={[
            { value: "mods", label: "模组" },
            { value: "worlds", label: "世界" },
          ]}
        />
        <div className="w-[240px]"><Slider value={mem} min={1024} max={16384} step={256} onInput={setMem} label="内存" /></div>
      </Section>

      <Section title="标记 Chip / Tag">
        <Chip active onClick={() => {}}>已选</Chip>
        <Chip onClick={() => {}}>未选</Chip>
        <Tag>只读标签</Tag>
      </Section>

      <Section title="反馈 Feedback">
        <Spinner size={20} />
        <div className="w-[280px]"><EmptyState title="暂无内容" hint="这里还什么都没有" /></div>
        <div className="w-[280px]"><ErrorState message="加载失败" onRetry={() => {}} /></div>
      </Section>

      <Section title="图标 Icon">
        {(["power", "download", "gear", "search", "user", "bell", "check", "close", "info", "warn"] as const).map((n) => (
          <div key={n} className="grid place-items-center w-[36px] h-[36px] bg-panel-2 shadow-input text-sub" title={n}>
            <Icon name={n} size={18} />
          </div>
        ))}
      </Section>

      <Section title="卡片 / 行 Cards & Rows">
        <div className="w-[300px]"><ModpackCard hit={HIT} onClick={() => {}} /></div>
        <div className="w-[380px] flex flex-col gap-[8px]">
          <ModpackListItem hit={HIT} onClick={() => {}} />
          <InstanceRow instance={INSTANCE} onOpen={() => {}} />
        </div>
      </Section>
    </div>
  );
};
AllComponents.storyName = "组件总览(全部展示型组件)";
