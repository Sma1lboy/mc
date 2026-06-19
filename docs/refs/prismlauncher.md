# 参考 · PrismLauncher(C++ / Qt)

> 国际主流跨平台启动器,架构抽象最干净。自研学它的**可扩展设计**。

## 架构分层(`launcher/`)

| 模块 | 职责 |
|------|------|
| `launch/` | 启动框架:LaunchTask(步骤队列编排)、LaunchStep(步骤基类)、LogModel |
| `minecraft/` | MC 业务:实例、版本组件、认证、配置 |
| `minecraft/launch/` | MC 专属启动步骤(AutoInstallJava、ExtractNatives、LauncherPartLaunch…) |
| `net/` | 网络:NetJob、Download、Sink、Validator、HttpMetaCache |
| `meta/` | 版本元数据服务:Index、Version、VersionList(离线推荐 + 依赖) |
| `modplatform/` | Mod 平台:ResourceAPI 抽象 + modrinth/flame/ftb/technic/atlauncher |
| `java/` | Java 检测(JavaChecker)+ 下载 |
| `tasks/` | 任务框架:Task / ConcurrentTask / SequentialTask |
| `ui/` | UI(与逻辑解耦) |

## 三个值得抄的设计

1. **步骤链(LaunchStep)**:启动拆成 ~20 个可组合、可中止的 step,`onStepFinished` 自动推进下一个。新增功能 = 加一个 step。
2. **组件系统(PackProfile + Component)**:实例是有序组件列表,每个组件 `applyTo(LaunchProfile)` 合并。比 inheritsFrom 更灵活。
3. **任务框架三件套**:所有异步操作继承 Task,统一状态机/进度/中止/信号。NetJob 是 ConcurrentTask 的特化。

## 启动步骤序列(MinecraftInstance::createLaunchTask)

```
TextPrint → CreateGameFolders → LookupServerAddress → MinecraftLoadAndCheck
→ AutoInstallJava → CheckJava → VerifyJavaInstall → PreLaunchCommand
→ ClaimAccount → ComponentUpdateTask(或 EnsureOfflineLibraries)
→ ModMinecraftJar → ScanModFolders → EnsureAvailableMemory → PrintInstanceInfo
→ ExtractNatives → ReconstructAssets → LauncherPartLaunch → PostLaunchCommand
```

## 文件速查

| 功能 | 文件 |
|------|------|
| 启动编排 | `launch/LaunchTask.{h,cpp}` |
| 起进程 | `minecraft/launch/LauncherPartLaunch.cpp` |
| 版本组件 | `minecraft/{PackProfile,Component,LaunchProfile,VersionFile}.h` |
| json 解析 | `minecraft/{MojangVersionFormat,OneSixVersionFormat}.cpp` |
| 库/坐标/规则 | `minecraft/{Library,GradleSpecifier,Rule}.h` |
| 下载 | `net/{NetJob,Download,FileSink,Validator,HttpMetaCache}.h` |
| 认证 | `minecraft/auth/AuthFlow.h` + `auth/steps/*` |
| 账号 | `minecraft/auth/{AccountData,AccountList,MinecraftAccount}.h` |
| Mod 平台 | `modplatform/{ResourceAPI,ModIndex}.h` + 各平台子目录 |
| Java | `java/{JavaChecker,JavaUtils,JavaVersion}.h` |
| 实例 | `BaseInstance.h`、`minecraft/MinecraftInstance.h` |
| 世界 | `minecraft/{World,WorldList}.h` |
| 任务 | `tasks/{Task,ConcurrentTask}.h` |
