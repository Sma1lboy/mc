import { Component } from "solid-js";
import { t } from "../i18n";

/**
 * ClassicMore —— 经典视图「更多」标签页:关于 / 链接 / 工具入口。
 * 这里先做关于卡 + 链接卡,后续填工具。
 */
const LINKS = (): { label: string; href: string; desc: string }[] => [
  { label: "Modrinth", href: "https://modrinth.com", desc: t("classic.more.link.modrinth") },
  { label: "CurseForge", href: "https://www.curseforge.com/minecraft", desc: t("classic.more.link.curseforge") },
  { label: "Minecraft Wiki", href: "https://zh.minecraft.wiki", desc: t("classic.more.link.wiki") },
  { label: "GitHub", href: "https://github.com/Sma1lboy/mc", desc: t("classic.more.link.github") },
];

const ClassicMore: Component = () => {
  return (
    <div class="h-full overflow-auto px-[24px] py-[20px] bg-transparent flex flex-col gap-[14px]">
      <div class="glass-card rounded-[5px] flex items-center gap-[16px] p-[20px]">
        <div class="w-[64px] h-[64px] flex-[0_0_64px] rounded-[12px] flex items-center justify-center text-[24px] font-extrabold text-white bg-[linear-gradient(135deg,var(--classic-blue-hover),var(--classic-blue))] shadow-classic">
          MC
        </div>
        <div>
          <div class="text-[20px] font-bold text-classic-text">kobeMC</div>
          <div class="text-[13px] text-classic-text2 mt-[4px]">
            {t("classic.more.classicView")}
          </div>
          <div class="text-[12px] text-classic-text3 mt-[2px]">
            {t("classic.more.devChannel")}
          </div>
        </div>
      </div>

      <div class="text-[13px] font-bold text-classic-text3 px-[2px] pt-[4px]">{t("classic.more.usefulLinks")}</div>
      <div class="grid grid-cols-2 gap-[12px]">
        {LINKS().map((l) => (
          <a
            class="flex items-center gap-[10px] px-[16px] py-[14px] glass-card glass-card--hover rounded-[5px] no-underline transition-[box-shadow,transform] duration-150 ease-[ease] hover:-translate-y-px cursor-pointer"
            href={l.href}
            target="_blank"
            rel="noreferrer"
          >
            <span class="text-[14px] font-semibold text-classic-text">{l.label}</span>
            <span class="flex-1 text-[12px] text-classic-text3">{l.desc}</span>
            <span class="text-classic-blue text-[14px]">↗</span>
          </a>
        ))}
      </div>
    </div>
  );
};

export default ClassicMore;
