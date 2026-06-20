import { Component } from "solid-js";

/**
 * PclMore —— PCL CE「更多」标签页:关于 / 链接 / 工具入口。
 * PCL 的「更多」是关于页 + 百宝箱;这里先做关于卡 + 链接卡,后续填工具。
 */
const LINKS: { label: string; href: string; desc: string }[] = [
  { label: "Modrinth", href: "https://modrinth.com", desc: "开源模组与整合包平台" },
  { label: "CurseForge", href: "https://www.curseforge.com/minecraft", desc: "最大的模组下载站" },
  { label: "Minecraft Wiki", href: "https://zh.minecraft.wiki", desc: "中文百科" },
  { label: "PCL 原版", href: "https://github.com/Hex-Dragon/PCL2", desc: "本风格的灵感来源" },
];

const PclMore: Component = () => {
  return (
    <div class="h-full overflow-auto px-[24px] py-[20px] bg-pcl-gray-bg flex flex-col gap-[14px]">
      <div class="bg-pcl-card rounded-[5px] shadow-pcl flex items-center gap-[16px] p-[20px]">
        <div class="w-[64px] h-[64px] flex-[0_0_64px] rounded-[12px] flex items-center justify-center text-[24px] font-extrabold text-white bg-[linear-gradient(135deg,var(--pcl-blue-hover),var(--pcl-blue))] shadow-pcl">
          MC
        </div>
        <div>
          <div class="text-[20px] font-bold text-pcl-text">MC Launcher</div>
          <div class="text-[13px] text-pcl-text2 mt-[4px]">
            Rust 核心 + Tauri 外壳 · PCL 风格界面
          </div>
          <div class="text-[12px] text-pcl-text3 mt-[2px]">
            开发版 · 双布局(Modrinth / PCL)
          </div>
        </div>
      </div>

      <div class="text-[13px] font-bold text-pcl-text3 px-[2px] pt-[4px]">常用链接</div>
      <div class="grid grid-cols-2 gap-[12px]">
        {LINKS.map((l) => (
          <a
            class="flex items-center gap-[10px] px-[16px] py-[14px] bg-pcl-card rounded-[5px] shadow-pcl no-underline transition-[box-shadow,transform] duration-150 ease-[ease] hover:shadow-pcl-strong hover:-translate-y-px"
            href={l.href}
            target="_blank"
            rel="noreferrer"
          >
            <span class="text-[14px] font-semibold text-pcl-text">{l.label}</span>
            <span class="flex-1 text-[12px] text-pcl-text3">{l.desc}</span>
            <span class="text-pcl-blue text-[14px]">↗</span>
          </a>
        ))}
      </div>
    </div>
  );
};

export default PclMore;
