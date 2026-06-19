# 参考 · 路径/文件系统处理可复用清单

> 从三个参考实现(PrismLauncher `FileSystem.cpp` 55KB、PCL-CE `PCL.Core/IO`、PCL2 `ModBase.vb`)提炼的路径处理逻辑。
> ✅ = 已移植进 `crates/mc-core/src/fs.rs`(或 `paths.rs`)· ⬜ = 仍可继续 copy。

## ✅ 已移植(mc-core)

| 能力 | 函数(我们的) | 来源 | 说明 |
|------|---------------|------|------|
| 文件名净化 | `sanitize_filename` | Prism `RemoveInvalidFilenameChars` | 非法字符 `<>:"/\|?*` + 控制符 + Windows 保留名(CON/PRN/COM1…)+ 结尾点/空格 |
| 唯一目录名 | `dir_name_from_string` | Prism `DirNameFromString` | 净化 + 冲突时 `-2/-3` 追加 |
| **问题路径检测** | `check_problematic_path` | Prism `checkProblemticPathJava` + PCL precheck | `!`→Error(破坏 Java classpath)、非 ASCII/中文→Warn、空格→Warn |
| 原子写入 | `write_atomic` | Prism `write`(QSaveFile) | temp+fsync+rename,防崩溃截断;已接入版本 json/config/账号/asset index |
| 路径规范化 | `normalize` / `path_depth` | Prism `NormalizePath`/`pathDepth` | 词法解析 `.`/`..`,不碰 FS |
| 子路径判定 | `is_subpath` | — | 防 `..` 逃逸 |
| PATH 查找 | `resolve_executable` | Prism `ResolveExecutable` | 名字→PATH(Windows 带 PATHEXT) |
| 跨盘移动 | `move_with_fallback` | Prism `move`/`moveByCopy` | rename 失败→copy+delete |
| 磁盘空间 | `available_space` + `nearest_existent_ancestor` | Prism `statFS` | 安装前预检(对未创建路径走最近存在祖先) |
| **共享存储** | `share_file`(`ShareMethod`) | Prism `create_link`/`clone` | hardlink→reflink→copy,实例间共享 libraries/assets 零额外磁盘 |
| 目录递归复制 | `copy_dir` | Prism `copy` | override/move 的基石 |
| 整合包覆写 | `override_folder` | Prism/PCL `overrideFolder` | overrides 叠加到实例 |
| zip-slip 防护 | `safe_join` | PCL "Directory Traversal Prevention" | 拒绝逃逸 base 的归档条目 |
| 官启兼容 | `ensure_launcher_profiles` | PCL `launcher_profiles.json 生成` | Forge/旧 installer 需要;已接入 install |
| 启动期路径守卫 | `launch()` 起始检查 | PCL `McLaunchPrecheck` | `!` 路径直接拒绝启动并报中文原因 |

## ⬜ 仍可继续 copy(按价值/难度)

### 高价值
| 能力 | 来源 | 难度 | 价值 |
|------|------|------|------|
| **legacy assets 重建** | Prism `reconstructAssets` / PCL2 | medium | <1.7 老版本把 hash 对象复制成真实文件名到 `virtual/resources`(否则老版本无声音/无语言文件)。我们已有 `assets_virtual_dir` 路径,缺重建逻辑 |
| **版本隔离模式** | PCL2 `PathIndie vs PathVersion` | medium | saves/mods/logs/resourcepacks 可配置「隔离到版本目录」或「共享 MC 根」。对应 docs/02 的 B6,体验关键 |
| **递归 share 整树** | Prism `clone`/`create_link` class | medium | 把整个 libraries/ 或 assets/ 目录树按 `share_file` 批量链接(带 max-depth),实例克隆秒级完成 |
| **回收站删除** | Prism `trash` | easy | 删实例/文件进系统回收站而非永久删除(安全;契合工作区"禁止无确认删除") |
| **modpack 格式探测** | PCL-CE/PCL2 | medium | 从压缩包结构判定 CurseForge/Modrinth/MMC/MCBBS/HMCL,整合包导入前置 |

### 中价值
| 能力 | 来源 | 难度 | 价值 |
|------|------|------|------|
| 文件系统类型识别 | Prism `FilesystemType`/`getFilesystemTypeFuzzy` | easy | 决定能否 reflink/symlink(FAT 不支持 symlink);`share_file` 已用回退兜底,显式探测可少试错 |
| symlink 能力检测 | Prism `canLinkOnFS` | easy | 同上 |
| 平台标准目录 | Prism `getDesktopDir`/`getApplicationsDir` | easy | 创建桌面快捷方式、导出位置 |
| 桌面快捷方式 | Prism `createShortcut`(.lnk/.desktop/.app) | hard | 为实例建直接启动的快捷方式(docs/02 D8) |
| shell 参数转义 | Prism `quoteArgs` | easy | wrapper command/启动脚本导出时安全转义 |
| 平台感知路径相等 | PCL-CE | easy | Windows 大小写不敏感比较 |
| 目录权限预检 | PCL-CE/PCL2 | easy | 添加自定义 MC 目录时检查可读写 |
| 归档多格式解压 | PCL-CE(ZIP/JAR/GZ/TAR/BZIP2 + 进度) | medium | 整合包/installer 解压;我们现仅 zip natives |
| 编码探测读写 | PCL-CE | medium | 老配置/日志非 UTF-8 兼容 |
| 系统保护目录检测 | PCL-CE | easy | 阻止在 C:\Windows 等创建实例 |

### 低价值 / 平台特定
- Windows 8.3 短名转换(非 ASCII 长路径回退)`getPathNameInLocal8bit` — medium
- Windows 文件属性复制 `copyFileAttributes` — easy
- Windows 提权链接 `ExternalLinkFileProcess`(UAC) — hard
- 内存/可用空间检测(系统级) — 部分已由 `available_space` 覆盖

## 推荐下一步顺序
1. **版本隔离模式**(PathIndie)——体验刚需,改动集中在 `GamePaths`+`InstanceConfig`。
2. **legacy assets 重建**——让 1.6- 老版本可玩。
3. **递归 share 整树 + trash**——实例克隆/删除的完整闭环(`share_file` 已就绪)。
4. **modpack 格式探测 + 多格式解压**——打开整合包导入(docs/02 B12)。
