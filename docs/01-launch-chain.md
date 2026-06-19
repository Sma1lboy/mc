# 01 · 启动链路(Launch Chain)

> 这是启动器的骨架。三个参考实现叫法不同(Prism = LaunchStep 队列、PCL = Loader 编排),但链路本质一致。本文给出**统一抽象**,并标注三家的对应实现。

## 0. 全景图

```
用户点击「启动」
│
├─ ① 准备阶段(Pre-Launch)
│   ├─ 创建游戏目录(.minecraft、server-resource-packs…)
│   ├─ 解析实例版本 → 合并组件 → 得到 LaunchProfile
│   ├─ (可选) DNS 解析「启动即加入的服务器」
│   └─ 执行用户自定义「启动前命令」
│
├─ ② 文件就绪阶段(Ensure Files)
│   ├─ 校验 client jar
│   ├─ 校验/下载 libraries(依赖库)
│   ├─ 校验/下载 assets(资源,按 hash 寻址)
│   ├─ 校验/下载 natives,并解压到临时 natives 目录
│   └─ (jar mod 场景) 把 mod 合并进 minecraft.jar
│
├─ ③ Java 就绪阶段
│   ├─ 确定本版本需要的 Java major(8/17/21…)
│   ├─ 在已知 Java 中匹配;没有则自动下载 JRE
│   └─ 校验 Java 可执行、架构匹配、内存足够
│
├─ ④ 账号就绪阶段(非离线)
│   ├─ 检查 accessToken 是否过期
│   ├─ 过期则用 refreshToken 刷新(微软链路)
│   └─ 拿到 {accessToken, uuid, username, userType, xuid}
│
├─ ⑤ 命令行拼接阶段
│   └─ java [JVM 参数] -cp [classpath] [mainClass] [游戏参数] [认证参数]
│
└─ ⑥ 启动与监控阶段
    ├─ 启动子进程,设置工作目录 = 游戏目录
    ├─ 重定向 stdout/stderr → 日志窗口(解析 log4j XML)
    ├─ 过滤敏感信息(token/uuid 打码)
    ├─ 监听退出码 + 崩溃关键词分析
    ├─ 执行用户自定义「启动后命令」
    └─ (可选) 启动器自身隐藏/最小化/退出
```

## 1. 准备阶段

**职责**:把"要启动哪个版本"翻译成"一份完整的运行配置"。

- **合并组件**:实例 = vanilla + (Forge/Fabric/Quilt/NeoForge/OptiFine…) + 自定义。按顺序把每个组件的 `libraries / mainClass / arguments / traits` 叠加,得到最终 `LaunchProfile`。后装的 loader 会**覆盖** `mainClass`、**追加** libraries 和参数。详见 [version-system.md](./modules/version-system.md)。
- **版本继承**:PCL 系用 `inheritsFrom` 字段递归解析父版本;Prism 用显式组件列表。两者等价。

| 实现 | 对应 |
|------|------|
| Prism | `CreateGameFolders`、`LookupServerAddress`、`MinecraftLoadAndCheck` → `PackProfile::applyTo` |
| PCL-CE | `ModLaunch.McLaunchPrecheck`、`GameCore`、`LaunchEnvUtils` |
| PCL2 | `ModLaunch.vb` 预检测段、`ModMinecraft.vb` 版本解析 |

## 2. 文件就绪阶段

**职责**:保证启动所需的所有文件都在本地且完整。

- **校验逻辑**:对每个文件检查「存在 + 大小 + SHA1」,任一不符 → 进下载队列。PCL 把这套叫 `FileChecker` / `DlClientFix`(补全)。
- **assets 寻址**:assets 不按文件名,按内容 hash 存:`assets/objects/<hash前2位>/<完整hash>`。需要先下 `assets/indexes/<id>.json` 拿到映射表。老版本(<1.6)是 legacy 布局,需特殊处理。
- **natives**:平台相关的 `.dll/.so/.dylib` 打包在带 classifier 的 jar 里(如 `natives-windows`),启动前解压到临时目录,通过 `-Djava.library.path` 指过去。
- **下载**:全部走统一下载框架(并发 + 镜像 + 重试 + 校验),详见 [download.md](./modules/download.md)。

| 实现 | 对应 |
|------|------|
| Prism | `ComponentUpdateTask`、`ExtractNatives`、`ReconstructAssets`、`ModMinecraftJar`、`EnsureOfflineLibraries` |
| PCL-CE | `ModDownload`、`ModAssets`、`ModLibrary`(native 提取) |
| PCL2 | `ModDownload.DlClientFix`、`ModLibrary.vb` |

## 3. Java 就绪阶段

**职责**:确定并准备一个能跑这个版本的 JRE。

- **版本匹配**:MC 1.17- 需 Java 8,1.17~1.20.4 需 17,1.20.5+ 需 21。版本 json 的 `javaVersion.majorVersion` 字段会给推荐值。
- **检测**:扫注册表(Windows)、PATH、常见安装路径、`where`/`which`,对每个候选跑 `java -version` 解析。
- **自动下载**:本地无匹配版本时,从 Mojang java runtime manifest 或 Adoptium/Azul 下载并解压到实例目录。

详见 [java.md](./modules/java.md)。

| 实现 | 对应 |
|------|------|
| Prism | `AutoInstallJava`、`CheckJava`、`VerifyJavaInstall`、`java/JavaChecker` |
| PCL-CE | `PCL.Core/Minecraft/Java/JavaManager`(5 种 scanner) |
| PCL2 | `ModJava.vb` |

## 4. 账号就绪阶段

**职责**:拿到有效的认证三元组(token / uuid / username)。

- 离线模式:用户名 MD5 生成假 UUID,无网络。
- 微软模式:检查 token 过期 → 用 refreshToken 走刷新链路(不需要重新弹浏览器)。
- 详见 [auth.md](./modules/auth.md)。

| 实现 | 对应 |
|------|------|
| Prism | `ClaimAccount` + `AuthFlow`(refresh) |
| PCL-CE | `IdentityModel`(OAuth/Yggdrasil) |
| PCL2 | `ModLaunch.vb` 中 `McLoginMs/Server/Legacy/Nide` |

## 5. 命令行拼接阶段(最关键)

最终命令形如:

```
<java> \
  <JVM 参数>            # -Xmx2G、GC、-Djava.library.path=<natives>、log4j 配置、loader 注入参数
  -cp <classpath>      # 所有 library jar + client jar,平台分隔符(: 或 ;)拼接
  <mainClass>          # 如 net.minecraft.client.main.Main,或 loader 的 BootstrapLauncher
  <游戏参数>            # --version、--gameDir、--assetsDir、--assetIndex、窗口分辨率…
  <认证参数>            # --username、--uuid、--accessToken、--userType、--xuid
```

**占位符替换**是核心机制——版本 json 里参数写成 `${...}` 模板,启动前用实际值替换:

| 占位符 | 含义 |
|--------|------|
| `${auth_player_name}` | 玩家名 |
| `${auth_uuid}` | 玩家 UUID |
| `${auth_access_token}` | 访问令牌 |
| `${auth_xuid}` | Xbox UID(微软登录) |
| `${user_type}` | 账户类型(msa/legacy) |
| `${version_name}` / `${version_type}` | 版本号 / 类型 |
| `${game_directory}` | 游戏工作目录 |
| `${assets_root}` / `${assets_index_name}` | 资源目录 / 索引名 |
| `${classpath}` | 拼好的 classpath |
| `${natives_directory}` | natives 解压目录 |
| `${library_directory}` | 库根目录(新版 loader 用) |
| `${launcher_name}` / `${launcher_version}` | 启动器标识 |

**注意点**:
- 新版 `arguments.jvm` / `arguments.game` 是带 rules 的数组(1.13+);老版是 `minecraftArguments` 单字符串。两种都要支持。
- 数组元素可能带 OS/feature 规则(如全屏、demo 模式),需按 [Rule 过滤](./modules/version-system.md#rule)。
- Forge/NeoForge 会注入额外 JVM 参数(模块系统 `-p`、`--add-modules` 等),来自 loader 组件的 json。

| 实现 | 对应 |
|------|------|
| Prism | `LaunchProfile` 收集 → `LauncherPartLaunch` 生成启动脚本 |
| PCL-CE | `LaunchEnvUtils` 占位符替换 |
| PCL2 | `ModLaunch.vb` 参数构建段 |

## 6. 启动与监控阶段

- 用子进程 API 启动(QProcess / Process),工作目录设为游戏目录。
- 实时读取 stdout/stderr,解析 log4j 的 XML 日志事件,显示到日志窗口。
- **敏感信息打码**:日志里出现的 accessToken、真实 UUID 要替换成 `***`。
- **崩溃分析**:进程非零退出时,扫日志关键词(OutOfMemory、缺 mod、版本不匹配、显卡驱动…)给出人话诊断。PCL 的 `ModCrash` 是这块的重点参考(1000+ 行关键词规则)。
- 启动器自身行为可配:保持打开 / 隐藏 / 最小化 / 退出。

| 实现 | 对应 |
|------|------|
| Prism | `LaunchTask` + `LogModel`,`setCensorFilter` 打码 |
| PCL-CE | `ModLaunch` + `ModCrash` |
| PCL2 | `ModWatcher.vb` + `ModCrash.vb`(1121 行崩溃分析) |

## 7. 自研落地建议

1. **把每个阶段做成一个 `LaunchStep`**,实现 `execute() / abort()`,放进一个顺序执行的队列。任一步失败则整体失败并报错。
2. **step 之间用一个共享上下文对象**传递数据(LaunchProfile、Java 路径、AuthSession、生成的命令行)。
3. **全程可中止**:每步检查取消信号,长任务(下载)能立即停。
4. **阶段 ②③④ 可并行**:文件下载、Java 检测、token 刷新互不依赖,能并发就并发,缩短等待。
5. **离线模式跳过 ④,并把 ② 退化为"只校验不下载"**(Prism 的 `EnsureOfflineLibraries`)。
