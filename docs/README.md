# 自研 Minecraft 启动器 · 归档文档

本目录是对 `ref/` 下三个开源启动器(PrismLauncher / PCL-CE / PCL2)的**链路**与**功能特性**梳理归档,作为自研启动器的设计蓝图。

## 参考实现

| 实现 | 语言/框架 | 定位 | 价值 |
|------|-----------|------|------|
| **PrismLauncher** | C++ / Qt | 国际主流、跨平台、模块化最清晰 | 学**架构抽象**(步骤链、组件系统、任务框架、ResourceAPI) |
| **PCL-CE** | C# (.NET 8) + VB.NET / WPF | 国内 PCL 的 C# 重构版,核心库 `PCL.Core` | 学**现代核心库设计**(源码生成配置、IdentityModel、联机大厅) |
| **PCL2** | VB.NET (.NET 4.8) / WPF | 国内最流行的原版 | 学**国内化实践**(BMCLAPI 多源、文件补全、易用性、加载器框架) |

## 文档导航

| 文档 | 内容 |
|------|------|
| [00-overview.md](./00-overview.md) | 启动器是什么、三实现横向对比、自研技术选型建议 |
| [01-launch-chain.md](./01-launch-chain.md) | **核心**:从点击启动到游戏进程的完整链路(统一抽象) |
| [02-features.md](./02-features.md) | **核心**:全功能特性清单(MVP → 完整版分级) |
| [03-tech-stack.md](./03-tech-stack.md) | **选型**:性能优先的技术栈畅想(主推 Rust 核心 + Tauri) |
| [04-rust-tauri-design.md](./04-rust-tauri-design.md) | **施工蓝图**:Rust+Tauri 的 workspace 布局、核心类型、async 模型、IPC、性能手法、里程碑 |
| [05-ui-design-pcl.md](./05-ui-design-pcl.md) | **UI 设计**:PCL 风格落地(设计 token、可调主题色、组件、动画、SolidJS 结构) |
| [06-ui-layout-synthesis.md](./06-ui-layout-synthesis.md) | **UI 布局**:Modrinth 三区骨架 + PCL 主题引擎融合(深色默认、图标栏、dashboard) |
| [07-directory-model-portability.md](./07-directory-model-portability.md) | **决策**:启动器独立于实例 + 多根目录自动检测 + 便携模式 |
| [08-data-layer-and-lite-server.md](./08-data-layer-and-lite-server.md) | **路线图**:数据层 feature 清单(是否值得做)+ lite 服务器设计 + 本地测试环境 |
| [09-engineering-reference.md](./09-engineering-reference.md) | **工程参考**:框架/库清单、46 模块地图、**存储层每处怎么存** + 优化清单(P0-P3) |
| [modules/version-system.md](./modules/version-system.md) | 版本/组件系统:json 解析、多 loader 合并、库与 native、规则过滤 |
| [modules/download.md](./modules/download.md) | 下载系统:任务框架、多源镜像、并发、校验、缓存、补全 |
| [modules/auth.md](./modules/auth.md) | 账号认证:微软 OAuth、离线、外置登录(Yggdrasil) |
| [modules/java.md](./modules/java.md) | Java 管理:检测、版本匹配、自动下载 |
| [modules/mod-platform.md](./modules/mod-platform.md) | Mod/整合包:CurseForge/Modrinth、依赖解析、整合包导入 |
| [modules/instance.md](./modules/instance.md) | 实例管理:版本隔离、目录结构、世界/资源管理 |
| [refs/prismlauncher.md](./refs/prismlauncher.md) | PrismLauncher 架构笔记(含文件速查表) |
| [refs/pcl-ce.md](./refs/pcl-ce.md) | PCL-CE 架构笔记 |
| [refs/pcl2.md](./refs/pcl2.md) | PCL2 架构笔记 |
| [refs/path-handling-catalog.md](./refs/path-handling-catalog.md) | 路径/文件系统处理可复用清单(已移植 ✅ / 仍可 copy ⬜) |

## 怎么用这套文档

1. 先读 `00-overview` 建立全局认知,确定技术选型。
2. `01-launch-chain` 是启动器的骨架——任何实现都绕不开这条链路,按它搭主流程。
3. `02-features` 是需求清单,按 MVP/进阶/完整分级排期。
4. 各 `modules/*` 是骨架上每个关节的设计细节,实现到某模块时再深入读。
5. `refs/*` 是三个实现的"答案",卡住时去对应源码找参考(文件路径已标注)。
