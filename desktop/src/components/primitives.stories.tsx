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
  BlockIcon,
  NavItem,
  Icon,
  type IconName,
  EmptyState,
  ErrorState,
  SearchBox,
  Card,
  Select,
  toast,
  ToastContainer,
} from ".";

/* ============================================================================
 * primitives.stories —— Blocky Craft 基础控件的隔离预览(Ladle)。
 *
 * 全是纯展示 / 受控原语:props 进、回调出,不碰 Tauri / store。每个故事喂 mock
 * props + 本地 useState 兜受控值,方便脱离 app 单独调样式与各状态。
 * ========================================================================== */

export default {
  title: "Components / Primitives",
} satisfies StoryDefault;

/** 小工具:给一组样例套统一的纵向排布 + 分组标题。 */
function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-[8px]">
      <div className="text-[11px] uppercase tracking-[1px] text-muted">{label}</div>
      <div className="flex flex-wrap items-center gap-[12px]">{children}</div>
    </div>
  );
}

// —— Button ————————————————————————————————————————————————————————

export const Buttons: Story = () => (
  <div className="flex flex-col gap-[20px]">
    <Row label="variants">
      <Button variant="primary">开始游戏</Button>
      <Button variant="ghost">取消</Button>
      <Button variant="danger">删除实例</Button>
    </Row>
    <Row label="disabled">
      <Button variant="primary" disabled>
        开始游戏
      </Button>
      <Button variant="ghost" disabled>
        取消
      </Button>
      <Button variant="danger" disabled>
        删除实例
      </Button>
    </Row>
    <Row label="with icon">
      <Button variant="primary">
        <Icon name="power" size={15} />
        启动
      </Button>
      <Button variant="ghost">
        <Icon name="download" size={15} />
        下载
      </Button>
    </Row>
  </div>
);
Buttons.storyName = "Button · 变体 / 禁用 / 带图标";

// —— Panel ————————————————————————————————————————————————————————

export const Panels: Story = () => (
  <div className="grid grid-cols-2 gap-[16px]">
    {(["sunken", "raised", "pressed", "input"] as const).map((v) => (
      <Panel key={v} variant={v} className="p-[18px] text-[13px] text-sub">
        variant="{v}"
      </Panel>
    ))}
    <Panel stone className="p-[18px] text-[13px] text-sub col-span-2">
      stone(叠石质纹理侧栏 / 大面板)
    </Panel>
  </div>
);
Panels.storyName = "Panel · 四种倒角 + stone";

// —— Typography ————————————————————————————————————————————————————

export const Typography: Story = () => (
  <div className="flex flex-col gap-[16px]">
    <Row label="heading sizes">
      <div className="flex flex-col gap-[8px]">
        <Heading size="page">方块工坊 Blocky Craft</Heading>
        <Heading size="section">我的整合包</Heading>
        <Heading size="sub">最近游玩</Heading>
        <Heading size="mini">附加特性</Heading>
      </div>
    </Row>
    <Row label="pixel label">
      <PixelLabel>PLAY</PixelLabel>
      <PixelLabel className="text-[14px]">CONTINUE</PixelLabel>
      <PixelLabel className="text-accent">MODRINTH</PixelLabel>
    </Row>
  </div>
);
Typography.storyName = "Typography · Heading / PixelLabel";

// —— Checkbox / Toggle —————————————————————————————————————————————

export const Checkboxes: Story = () => {
  const [a, setA] = useState(true);
  const [b, setB] = useState(false);
  return (
    <div className="flex flex-col gap-[12px] items-start">
      <Checkbox checked={a} onChange={setA} label="性能优化(Sodium / Lithium)" />
      <Checkbox checked={b} onChange={setB} label="光影支持(Iris)" />
      <Checkbox checked={true} onChange={() => {}} label="已禁用(选中)" disabled />
      <Checkbox checked={false} onChange={() => {}} label="已禁用(未选)" disabled />
    </div>
  );
};
Checkboxes.storyName = "Checkbox · 受控 / 禁用";

export const Toggles: Story = () => {
  const [on, setOn] = useState(true);
  return (
    <div className="flex items-center gap-[16px]">
      <Toggle checked={on} onChange={setOn} title="启用" />
      <Toggle checked={false} onChange={() => {}} title="关闭" />
      <Toggle checked={true} onChange={() => {}} disabled title="禁用(开)" />
      <Toggle checked={false} onChange={() => {}} disabled title="禁用(关)" />
    </div>
  );
};
Toggles.storyName = "Toggle · 开 / 关 / 禁用";

// —— Segmented —————————————————————————————————————————————————————

export const SegmentedControl: Story = () => {
  const [tab, setTab] = useState("mods");
  const [source, setSource] = useState("modrinth");
  return (
    <div className="flex flex-col gap-[16px] items-start">
      <Segmented
        value={tab}
        onChange={setTab}
        options={[
          { value: "mods", label: "模组" },
          { value: "resourcepacks", label: "资源包" },
          { value: "shaders", label: "光影" },
          { value: "datapacks", label: "数据包" },
        ]}
        ariaLabel="内容类型"
      />
      <Segmented
        pixel
        value={source}
        onChange={setSource}
        options={[
          { value: "modrinth", label: "Modrinth" },
          { value: "curseforge", label: "CurseForge" },
        ]}
        ariaLabel="下载源"
      />
    </div>
  );
};
SegmentedControl.storyName = "Segmented · 内容类型 / 下载源(pixel)";

// —— Slider ————————————————————————————————————————————————————————

export const Sliders: Story = () => {
  const [mem, setMem] = useState(4096);
  const [par, setPar] = useState(8);
  return (
    <div className="flex flex-col gap-[20px] max-w-[360px]">
      <Slider
        value={mem}
        min={1024}
        max={16384}
        step={256}
        onInput={setMem}
        label="内存分配"
        display={(v) => `${(v / 1024).toFixed(1)}G`}
      />
      <Slider value={par} min={1} max={16} onInput={setPar} label="下载并发" />
      <Slider value={50} min={0} max={100} onInput={() => {}} label="禁用" disabled />
    </div>
  );
};
Sliders.storyName = "Slider · 内存 / 并发 / 禁用";

// —— Chip / Tag ————————————————————————————————————————————————————

export const Chips: Story = () => {
  const [active, setActive] = useState("tech");
  return (
    <div className="flex flex-col gap-[16px]">
      <Row label="filter chips (single active)">
        {[
          { id: "tech", label: "科技" },
          { id: "magic", label: "魔法" },
          { id: "adventure", label: "冒险" },
        ].map((c) => (
          <Chip key={c.id} active={active === c.id} onClick={() => setActive(c.id)}>
            {c.label}
          </Chip>
        ))}
      </Row>
      <Row label="selected filters (removable)">
        <Chip onRemove={() => {}}>Fabric</Chip>
        <Chip onRemove={() => {}}>1.20.1</Chip>
        <Chip onRemove={() => {}}>开源</Chip>
      </Row>
      <Row label="static tags">
        <Tag>Fabric</Tag>
        <Tag>Optimization</Tag>
        <Tag>Adventure</Tag>
      </Row>
    </div>
  );
};
Chips.storyName = "Chip / Tag · 可选 / 可移除 / 静态";

// —— Spinner ———————————————————————————————————————————————————————

export const Spinners: Story = () => (
  <div className="flex items-center gap-[20px]">
    <Spinner size={16} />
    <Spinner size={24} />
    <Spinner size={36} />
  </div>
);
Spinners.storyName = "Spinner · 盲文旋转(多尺寸)";

// —— BlockIcon / NavItem —————————————————————————————————————————————

export const BlockIcons: Story = () => (
  <div className="flex items-center gap-[16px]">
    <BlockIcon className="w-[32px] h-[32px]" />
    <BlockIcon className="w-[48px] h-[48px]" />
    <BlockIcon className="w-[64px] h-[64px]" />
  </div>
);
BlockIcons.storyName = "BlockIcon · 草方块占位";

export const NavItems: Story = () => {
  const [active, setActive] = useState("home");
  const items: { id: string; icon: IconName; title: string }[] = [
    { id: "home", icon: "grid", title: "首页" },
    { id: "discover", icon: "search", title: "发现" },
    { id: "library", icon: "download", title: "库" },
    { id: "settings", icon: "gear", title: "设置" },
  ];
  return (
    <Panel stone className="inline-flex flex-col gap-[8px] p-[8px]">
      {items.map((it) => (
        <NavItem
          key={it.id}
          active={active === it.id}
          title={it.title}
          onClick={() => setActive(it.id)}
        >
          <Icon name={it.icon} size={22} />
        </NavItem>
      ))}
    </Panel>
  );
};
NavItems.storyName = "NavItem · 侧栏导航(选中态)";

// —— Icon 全集 —————————————————————————————————————————————————————

const ALL_ICONS: IconName[] = [
  "power",
  "download",
  "gear",
  "grid",
  "close",
  "check",
  "info",
  "warn",
  "error",
  "search",
  "microsoft",
  "user",
  "users",
  "bell",
  "link",
];

export const Icons: Story = () => (
  <div className="grid grid-cols-5 gap-[12px]">
    {ALL_ICONS.map((name) => (
      <Panel
        key={name}
        variant="sunken"
        className="flex flex-col items-center gap-[8px] p-[12px] text-fg"
      >
        <Icon name={name} size={24} label={name} />
        <span className="text-[10px] text-muted">{name}</span>
      </Panel>
    ))}
  </div>
);
Icons.storyName = "Icon · 全部图标网格";

// —— EmptyState / ErrorState ————————————————————————————————————————

export const EmptyStates: Story = () => (
  <div className="flex flex-col gap-[16px]">
    <EmptyState
      title="还没有整合包"
      hint="到「发现」页搜索并安装,或从本地导入一个整合包。"
      action={<Button variant="primary">去发现</Button>}
    />
    <EmptyState title="没有匹配的搜索结果" compact />
  </div>
);
EmptyStates.storyName = "EmptyState · 有操作 / 紧凑";

export const ErrorStates: Story = () => (
  <div className="flex flex-col gap-[16px]">
    <ErrorState message="加载整合包列表失败:网络超时" onRetry={() => {}} />
    <ErrorState compact />
  </div>
);
ErrorStates.storyName = "ErrorState · 带重试 / 默认文案";

// —— SearchBox —————————————————————————————————————————————————————

export const SearchBoxes: Story = () => {
  const [q, setQ] = useState("");
  const [q2, setQ2] = useState("sodium");
  return (
    <div className="flex flex-col gap-[16px] max-w-[420px]">
      <SearchBox value={q} onInput={setQ} placeholder="搜索整合包…" />
      <SearchBox value={q2} onInput={setQ2} placeholder="搜索模组…" />
    </div>
  );
};
SearchBoxes.storyName = "SearchBox · 空 / 有值(可清除)";

// —— Card ——————————————————————————————————————————————————————————

export const Cards: Story = () => (
  <div className="grid grid-cols-2 gap-[16px]">
    <Card>
      <Heading size="sub">静态卡片</Heading>
      <div className="text-[13px] text-sub mt-[6px]">普通容器,无 hover 动画。</div>
    </Card>
    <Card hover onClick={() => {}}>
      <Heading size="sub">可点卡片</Heading>
      <div className="text-[13px] text-sub mt-[6px]">hover 上移 + 阴影加深。</div>
    </Card>
  </div>
);
Cards.storyName = "Card · 静态 / 可点(hover)";

// —— Select ————————————————————————————————————————————————————————

export const Selects: Story = () => {
  const [v, setV] = useState("fabric");
  return (
    <div className="max-w-[280px]">
      <Select
        value={v}
        onChange={setV}
        options={[
          { value: "fabric", label: "Fabric" },
          { value: "forge", label: "Forge" },
          { value: "neoforge", label: "NeoForge" },
          { value: "quilt", label: "Quilt" },
        ]}
        placeholder="选择加载器"
      />
    </div>
  );
};
Selects.storyName = "Select · 加载器下拉";

// —— Toast —————————————————————————————————————————————————————————

/** Toast 是全局单例通道:点按钮 push 一条,右下角由 <ToastContainer/> 渲染 + 退场。 */
export const Toasts: Story = () => (
  <div className="flex flex-wrap gap-[12px]">
    <Button variant="ghost" onClick={() => toast.info("已刷新实例列表")}>
      info
    </Button>
    <Button variant="ghost" onClick={() => toast.success("整合包安装完成")}>
      success
    </Button>
    <Button variant="ghost" onClick={() => toast.warn("有 2 个模组存在版本冲突")}>
      warn
    </Button>
    <Button variant="ghost" onClick={() => toast.error("启动失败:找不到 Java")}>
      error
    </Button>
    <ToastContainer />
  </div>
);
Toasts.storyName = "Toast · 四种类型(点按钮触发)";
