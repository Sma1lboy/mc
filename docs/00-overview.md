# 00 · 总览与技术选型

## 1. Minecraft 启动器到底做什么

一个 MC 启动器本质是「**把正确的文件准备好,拼一条正确的 `java` 命令行,然后把游戏进程拉起来**」。它不碰游戏本体逻辑,只负责四件事:

1. **管版本**:解析 Mojang 的 `version.json`,叠加 Forge/Fabric 等 loader,算出最终启动配置。
2. **管文件**:下载并校验 client jar、libraries(依赖库)、assets(资源)、natives(本地库),缺啥补啥。
3. **管账号**:微软正版登录 / 离线 / 第三方外置登录,拿到 `accessToken` + `uuid` + `username`。
4. **管 Java**:找到/下载合适版本的 JRE,把上面三者拼成命令行启动。

外加一圈增值功能:实例隔离、Mod/整合包安装、世界/资源包管理、崩溃分析、联机大厅、主题个性化。

## 2. 启动器的「链路」一句话版

```
点击启动
  → 解析实例版本(vanilla + loader 合并) 
  → 检查/补全文件(jar/libs/assets/natives)
  → 解压 natives
  → 检查/下载 Java
  → 刷新账号 token
  → 拼接 java 命令行(JVM 参数 + classpath + mainClass + 游戏参数 + 认证参数)
  → 启动进程 + 监听日志/崩溃
```

完整展开见 [01-launch-chain.md](./01-launch-chain.md)。

## 3. 三个参考实现横向对比

| 维度 | PrismLauncher | PCL-CE | PCL2 |
|------|---------------|--------|------|
| 语言 | C++ / Qt | C#(核心) + VB.NET(UI) | VB.NET |
| 平台 | Win/macOS/Linux | Windows | Windows |
| 架构风格 | 高度抽象、面向接口、任务框架 | 核心库分离 + 源码生成 | 单体模块化 + 加载器框架 |
| 启动流程抽象 | **步骤链**(LaunchStep 队列) | 流程式 | 加载器(Loader)编排 |
| 版本系统 | **组件化**(PackProfile + Component) | 版本继承(inheritsFrom) | 版本继承 |
| 下载 | NetJob + Sink + Validator + HttpMetaCache | IO/Download 框架 + 多源 | 多源并联 + FileChecker |
| 国内镜像 | 无(社区可配) | **BMCLAPI 内置** | **BMCLAPI/McIM 内置** |
| 认证 | MSA 步骤链 + 离线 | IdentityModel(OAuth/PKCE/Yggdrasil) | MS/外置/离线/统一通行证 |
| Mod 平台 | Modrinth/Flame/FTB/Technic/ATL | CurseForge/Modrinth | CurseForge/Modrinth |
| 特色 | 跨平台、整合包生态、元数据服务 | 联机大厅(EasyTier P2P)、主题、源码生成配置 | 联机大厅、版本隔离、易用性 |
| 学什么 | **抽象与可扩展性** | **核心库工程化** | **国内化与体验** |

## 4. 自研技术选型建议

没有标准答案,取决于目标平台和团队栈。三条典型路线:

| 路线 | 技术栈 | 适合 | 参考 |
|------|--------|------|------|
| **跨平台原生** | Rust / C++ + Tauri / Qt | 想要小体积、跨平台、性能 | PrismLauncher 架构 |
| **跨平台 Web 系** | Electron / Tauri + TS,或 .NET MAUI / Avalonia | 团队熟 Web/C#,要快速做 UI | PCL-CE 核心库思路 |
| **Windows 优先** | C# / .NET + WPF | 只做 Windows、对标 PCL | PCL-CE / PCL2 |

**通用建议**(无论选哪条):

1. **核心与 UI 分离**:把「版本/下载/认证/Java/启动」做成无 UI 的核心库(像 `PCL.Core`),UI 只调它。便于测试、换 UI、跨平台。
2. **启动用步骤链**:把启动拆成可组合、可中止、可复用的 step(像 Prism 的 LaunchStep),不要写成一个 2000 行的大函数(PCL2 的 ModLaunch.vb 就是反例的体量)。
3. **下载用任务框架**:统一的异步 Task/Job 抽象 + Sink(写文件/内存) + Validator(校验) + 缓存,所有网络操作复用。
4. **国内做的话必接 BMCLAPI**:多源并联、URL 改写、文件补全,这是 PCL 系流行的根本原因。
5. **认证做成步骤链**:微软登录是 5~6 步的串行流程,做成可插拔的 step,离线/外置登录共用框架。

## 5. 关键外部依赖与协议(自研必须对接的)

| 名称 | 用途 | 地址/说明 |
|------|------|-----------|
| Mojang Version Manifest | 版本列表 | `https://piston-meta.mojang.com/mc/game/version_manifest_v2.json` |
| Mojang Piston Meta | 单版本 json、assets index、client jar | `piston-meta.mojang.com` / `piston-data.mojang.com` |
| Microsoft OAuth | 正版登录 | `login.live.com` / `login.microsoftonline.com` |
| Xbox Live | 登录链路中转 | `user.auth.xboxlive.com` / `xsts.auth.xboxlive.com` |
| Minecraft Services | profile / entitlements | `api.minecraftservices.com` |
| BMCLAPI | 国内镜像(可镜像上面大部分) | `bmclapi2.bangbang93.com` |
| CurseForge API | Mod/整合包 | 需 API key |
| Modrinth API | Mod/整合包 | `api.modrinth.com/v2`,开放 |
| Adoptium / Azul / Microsoft | JRE 下载 | 各厂商 API,或 Mojang 自带 java runtime manifest |
| Authlib-Injector | 外置登录 | 各第三方皮肤站 / 私服 |

> 详细的链路和模块设计见后续文档。
