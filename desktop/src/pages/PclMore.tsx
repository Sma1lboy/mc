import { Component } from "solid-js";
import "./PclMore.css";

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
    <div class="pcl-more">
      <div class="pcl-more-card pcl-more-about">
        <div class="pcl-more-logo">MC</div>
        <div>
          <div class="pcl-more-title">MC Launcher</div>
          <div class="pcl-more-sub">Rust 核心 + Tauri 外壳 · PCL 风格界面</div>
          <div class="pcl-more-ver">开发版 · 双布局(Modrinth / PCL)</div>
        </div>
      </div>

      <div class="pcl-more-h">常用链接</div>
      <div class="pcl-more-links">
        {LINKS.map((l) => (
          <a class="pcl-more-link" href={l.href} target="_blank" rel="noreferrer">
            <span class="pcl-more-link-label">{l.label}</span>
            <span class="pcl-more-link-desc">{l.desc}</span>
            <span class="pcl-more-link-arrow">↗</span>
          </a>
        ))}
      </div>
    </div>
  );
};

export default PclMore;
