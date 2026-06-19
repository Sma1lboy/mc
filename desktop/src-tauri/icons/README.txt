应用图标占位说明
=================

构建打包前需在本目录放置一张真实的 PNG 图标,文件名必须为:

    icon.png

tauri.conf.json 的 bundle.icon 引用了 ["icons/icon.png"]。

建议提供 1024x1024 的 RGBA PNG。若后续 bundle 阶段报缺少特定平台图标
(.ico / .icns / Square*Logo.png 等),可用以下命令从 icon.png 一键生成全套:

    pnpm tauri icon icons/icon.png

(本仓库约定由维护者统一安装与构建,这里仅放占位说明,不生成二进制图片。)
