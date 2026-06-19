# 参考 · PCL-CE(C# .NET 8 核心 + VB.NET WPF UI)

> 国内 PCL 的 C# 重构版。亮点是**核心库工程化** + **源码生成配置** + **联机大厅**。

## 工程结构

| 工程 | 语言 | 职责 |
|------|------|------|
| `PCL.Core/` | C# | 无 UI 核心库(Apache 2.0):Minecraft / App / IO / UI / Link / Utils |
| `PCL.Core.SourceGenerators/` | C# | 编译期源码生成(配置、DI、生命周期) |
| `Plain Craft Launcher 2/` | VB.NET + WPF | UI 应用(Modules / Pages / Controls) |

## PCL.Core 模块

| 模块 | 内容 |
|------|------|
| `Minecraft/` | Java(JavaManager + 5 scanner)、Launch(LaunchEnvUtils 占位符)、IdentityModel(OAuth/PKCE/Yggdrasil)、ResourceProject(CF/Modrinth + 依赖解析)、GameCore |
| `App/` | Configuration(源码生成驱动)、EventBus、Localization、Lifecycle、Database(SQLite) |
| `IO/` | Net/Http(链式 HttpRequest)、Net/Dns(DoH + DNS 竞速)、Download(NDlFactory)、Storage/Cache |
| `Link/` 🌟 | McPing、Scaffolding(联机协议)、EasyTier(P2P 穿透/中继)、Lobby |
| `UI/` | 主题(ThemeService)、动画框架、控件、NColor |

## 源码生成器(差异化亮点)

| 生成器 | 作用 |
|--------|------|
| `ConfigGenerator`(28.5K) | 扫 `[ConfigItem]`/`[ConfigGroup]` → 生成配置类 getter/setter + 事件 + 加密 + 迁移 |
| `DependencyCollectorGenerator` | 生成 IoC 装配代码(无反射) |
| `LifecycleScopeGenerator` | 生成生命周期事件挂接 |

> 用源码生成把"配置系统"做成零样板、零反射,是很现代的工程手法,值得借鉴(对应语言可用 Rust macro / TS decorator / C# SourceGen)。

## 国内化要点

- 下载多源(BMCLAPI),`ModDownload`
- IdentityModel 支持外置登录(Yggdrasil)
- 联机大厅(EasyTier P2P + Scaffolding 协议)——这是 PCL-CE 的护城河
- 完整中文本地化

## 文件速查

| 功能 | 文件 |
|------|------|
| 启动主逻辑 | `Plain Craft Launcher 2/Modules/Minecraft/ModLaunch.cs`(154K) |
| 占位符替换 | `PCL.Core/Minecraft/Launch/Utils/LaunchEnvUtils.cs` |
| Java 管理 | `PCL.Core/Minecraft/Java/JavaManager.cs` |
| 微软登录 | `PCL.Core/Minecraft/IdentityModel/OAuth/Client.cs` |
| 依赖解析 | `PCL.Core/Minecraft/ResourceProject/ModDependencyResolver.cs` |
| 整合包 | `Plain Craft Launcher 2/Modules/Minecraft/ModModpack.cs`(77K) |
| 配置生成 | `PCL.Core.SourceGenerators/ConfigGenerator.cs` |
| HTTP | `PCL.Core/IO/Net/Http/HttpRequest.cs` |
| 联机 | `PCL.Core/Link/{McPing,Scaffolding,EasyTier}/` |
