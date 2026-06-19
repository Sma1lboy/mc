# 参考 · PCL2(VB.NET .NET 4.8 / WPF)

> 国内最流行的 PCL 原版。架构是单体模块化,亮点是**加载器框架** + **国内化实践** + **崩溃分析**。

## 工程结构

| 工程 | 语言 | 状态 | 职责 |
|------|------|------|------|
| `Plain Craft Launcher 2` | VB.NET | 主线(4.3 万行) | 完整启动器 |
| `PCLCS` | C# | 孵化(<100 行) | 未来 C# 重构库 |
| `MeloongCore` | C#(submodule) | 基础库 | UI/动画/网络通用 |

## 核心模块(`Modules/`)

| 文件 | 行数 | 职责 |
|------|------|------|
| `Minecraft/ModMinecraft.vb` | 2526 | 版本/文件夹管理、实例隔离 |
| `Minecraft/ModLaunch.vb` | 2368 | 启动链路 + 4 类账号(Ms/Server/Legacy/Nide) |
| `Minecraft/ModDownload.vb` | 1386 | 下载引擎、多源(DlSourceLoader)、FileChecker、DlClientFix |
| `Minecraft/ModCrash.vb` | 1121 | 崩溃日志关键词分析 🌟 |
| `Minecraft/ModJava.vb` | 758 | Java 检测 |
| `Minecraft/ModModpack.vb` | 882 | 整合包(CF/MMC/MCBBS/Modrinth) |
| `Base/ModNet.vb` | 2010 | 网络:重试/代理/SSL/多源 |
| `Base/ModLoader.vb` | 712 | **加载器框架**(异步任务编排核心) |
| `Base/ModBase.vb` | 1536 | 路径/文件/日志/设置 |

## 加载器框架(架构灵魂)

```
LoaderBase
├─ LoaderTask<TIn,TOut>   单异步任务(输入/输出 + 异常 + 重试)
├─ LoaderCombo<T>         任务组合(顺序/并行 + 进度聚合)
└─ LoaderDownload         多线程下载

状态机: Waiting → Loading → Finished / Failed / Interrupted
事件: OnStateChangedUi(UI线程) / OnStateChangedThread(工作线程)
```

> 相当于 Prism 的 Task 框架,但更 UI 绑定。自研可借鉴它"进度自动聚合 + UI 自动更新"的思路。

## 国内化实践(最大价值)🌟

- **多源并联**:官方 + BMCLAPI 并行竞速,URL 自动改写
- **文件补全 DlClientFix**:libraries/assets/javaruntime 分别校验补全,assets 可后台
- **四类账号**:微软 / 外置(Authlib)/ 离线 / **统一通行证(Nide,网易)**
- **版本隔离**:`versions/<id>/PCL/`(Setup.ini + Logo + Mods),每版本可配不同登录服务器
- **崩溃分析**:1121 行关键词规则,把报错翻译成人话
- **设置系统**:150+ 条目,注册表存储,敏感项 DES 加密(偏弱)

## PCL2 vs PCL-CE

PCL-CE 是 PCL2 的现代化重构方向:VB.NET → C#、单体 → 核心库分离、运行时配置 → 源码生成、.NET 4.8 → .NET 8、纯 Windows → 为跨平台铺路(PCLCS 即此意图的早期形态)。
