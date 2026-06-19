# 04 · Rust 核心 + Tauri 外壳 · 施工蓝图

> 已选定方案①。本文把它落到可施工程度:workspace 布局、crate 边界、核心类型、async 任务模型、下载引擎、IPC、错误处理、性能手法。

---

## 1. Workspace 布局(Cargo 多 crate)

把"核心逻辑"和"外壳"彻底分开。核心是纯 Rust 库,不知道 Tauri/UI 的存在。

```
mc-launcher/
├── Cargo.toml                  # [workspace]
├── crates/
│   ├── mc-core/                # ★ 纯逻辑内核(无 UI、无 Tauri)— 占 80% 代码
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── version/        # 版本/组件系统
│   │   │   ├── download/       # 下载引擎
│   │   │   ├── auth/           # 认证
│   │   │   ├── java/           # Java 检测/下载
│   │   │   ├── instance/       # 实例管理
│   │   │   ├── launch/         # 启动链路(step 链)
│   │   │   ├── modplatform/    # Mod/整合包
│   │   │   ├── meta/           # 镜像源/元数据
│   │   │   └── error.rs
│   │   └── Cargo.toml
│   ├── mc-cli/                 # CLI 前端(测试/CI/无头用)— 直接调 mc-core
│   │   └── src/main.rs         #   mc-cli launch <instance> / fix / install ...
│   └── mc-types/               # 共享 DTO(serde,UI 和核心都用)
│       └── src/lib.rs          #   ts-rs 自动导出 TypeScript 类型给前端
├── desktop/                    # Tauri 应用(外壳)
│   ├── src/                    # Rust:tauri command + event 桥接,薄薄一层
│   │   ├── main.rs
│   │   ├── commands.rs         # #[tauri::command] 包装 mc-core 调用
│   │   └── events.rs           # 进度/日志流推送
│   ├── ui/                     # 前端(SolidJS/Svelte + Vite)
│   │   ├── src/
│   │   └── package.json
│   └── tauri.conf.json
└── docs/
```

**铁律**:`mc-core` 的 `Cargo.toml` **不依赖 tauri**。它只产出数据和事件流,谁来消费(CLI / Tauri / 测试)都行。这就是 PCL-CE 想用 `PCL.Core` 达到、但 Rust 能做得更彻底的「纯净内核」。

---

## 2. 核心异步模型

启动器是 **IO 密集**(下载、HTTP、文件),用 `tokio` 单一 async 运行时贯穿。CPU 密集的小块(批量 SHA1、解压)甩给 `rayon` 或 `tokio::task::spawn_blocking`,不堵 async 调度器。

```
tokio runtime (多线程)
  ├─ async 任务:HTTP 请求、文件 IO、进程管理
  └─ spawn_blocking / rayon:SHA1 批量校验、zip 解压(CPU 活)
```

**统一任务抽象**(对标 Prism 的 Task / PCL 的 Loader):

```rust
// 进度 + 取消 + 结果,用 channel 上报,不耦合 UI
pub struct TaskHandle<T> {
    progress: watch::Receiver<Progress>,   // UI 订阅进度
    cancel: CancellationToken,             // 随时可中止
    result: oneshot::Receiver<Result<T>>,  // 最终结果
}

pub struct Progress {
    pub stage: String,        // "下载 libraries"
    pub current: u64,
    pub total: u64,
    pub speed_bps: u64,       // 实时速度
}
```

> 关键:进度用 `tokio::sync::watch`,日志/事件用 `mpsc`。核心层只管往 channel 里推,Tauri 层把 channel 转成前端 event,CLI 层把它打到 stdout。**同一份核心,多种前端。**

---

## 3. 启动链路:Step 链(Rust 版)

把 [01-launch-chain](./01-launch-chain.md) 的 6 阶段实现成可组合、可中止的 step。

```rust
#[async_trait]
pub trait LaunchStep: Send + Sync {
    fn name(&self) -> &str;
    async fn execute(&self, ctx: &mut LaunchContext) -> Result<()>;
}

// 共享上下文,step 之间传数据
pub struct LaunchContext {
    pub instance: Instance,
    pub profile: Option<LaunchProfile>,   // 合并后的版本配置
    pub java: Option<JavaInstall>,
    pub session: Option<AuthSession>,     // token/uuid/name
    pub command: Option<Vec<String>>,     // 拼好的命令行
    pub progress: watch::Sender<Progress>,
    pub cancel: CancellationToken,
}

// 编排:顺序执行,任一失败即停;可并行的阶段用 join
pub async fn launch(instance: Instance, ...) -> Result<Child> {
    let steps: Vec<Box<dyn LaunchStep>> = vec![
        Box::new(CreateGameFolders),
        Box::new(ResolveProfile),       // 合并组件 → LaunchProfile
        // ↓ 这三步互不依赖,join 并发
        Box::new(EnsureFiles),          // 校验+下载 jar/libs/assets/natives
        Box::new(EnsureJava),           // 检测/下载 Java
        Box::new(RefreshAccount),       // 刷 token
        // ↑
        Box::new(ExtractNatives),
        Box::new(BuildCommandLine),
        Box::new(SpawnProcess),
    ];
    for step in steps {
        ctx.cancel.cancelled_or(step.execute(&mut ctx)).await?;
    }
    ...
}
```

并发优化:`EnsureFiles` / `EnsureJava` / `RefreshAccount` 无依赖,用 `tokio::try_join!` 同时跑,把"准备阶段"墙钟时间压到三者最大值而非之和。

---

## 4. 下载引擎(性能核心)

这是整个启动器最该压性能的地方。Rust 在这里相对 C#/VB.NET 优势最大。

```rust
pub struct Downloader {
    client: reqwest::Client,        // 全局复用!连接池 + HTTP/2 + keep-alive
    semaphore: Arc<Semaphore>,      // 并发上限(默认 64)
    limiter: Option<RateLimiter>,   // 可选限速
    mirror: MirrorResolver,         // 镜像源 URL 改写
}

pub struct DownloadItem {
    pub url: String,
    pub path: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}
```

设计要点(对应 [download.md](./modules/download.md)):

1. **reqwest::Client 全局单例**:连接池复用,几百个 library 不重复握手 TLS。这一条就能比"每次新建连接"快数倍。
2. **Semaphore 控并发**:`buffer_unordered` / `JoinSet` 跑 N 个并发,信号量封顶,避免打爆。
3. **校验内联到下载**:边下边算 SHA1(流式 `Sha1` digest 喂 chunk),下完即验,不二次读盘。
4. **镜像改写**:`MirrorResolver::rewrite(url) -> url`,集中管理 Mojang→BMCLAPI。
5. **补全 = 校验**:`verify_and_repair(instance)` 用 rayon 并行扫所有文件算 SHA1,缺/坏的组成下载批次。

```rust
// 批量校验:rayon 把上千文件摊到所有核心
use rayon::prelude::*;
let broken: Vec<_> = expected_files
    .par_iter()
    .filter(|f| !verify_sha1(&f.path, &f.sha1))   // CPU SHA 扩展指令
    .cloned()
    .collect();
downloader.download_all(broken).await?;
```

**性能预期**:千文件校验从 PCL(单线程/多线程混合)的数秒,降到亚秒级(取决于磁盘);下载吞吐打满带宽不是瓶颈。

---

## 5. 版本/组件系统

```rust
// 版本 json 用 serde 强类型解析(serde_json 极快)
#[derive(Deserialize)]
pub struct VersionJson {
    pub id: String,
    pub main_class: String,
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub arguments: Option<Arguments>,        // 1.13+
    pub minecraft_arguments: Option<String>, // 1.12-
    pub asset_index: AssetIndex,
    pub java_version: Option<JavaVersionReq>,
    pub inherits_from: Option<String>,       // 继承模型
    ...
}

// Gradle 坐标
pub struct GradleSpec { group, artifact, version, classifier, ext }
impl GradleSpec {
    pub fn to_path(&self) -> PathBuf { ... }
    pub fn from_str(s: &str) -> Result<Self> { ... }
}

// 规则求值:OS/arch/feature → bool
pub struct RuntimeContext { os: Os, arch: Arch, features: FeatureSet }
pub fn rules_allow(rules: &[Rule], ctx: &RuntimeContext) -> bool { ... }
```

合并:支持两种模型——先做 `inherits_from` 递归(与官启/HMCL 互通),组件模型可后续叠加。产物 `LaunchProfile` 可序列化便于 debug。

---

## 6. 认证(step 链复用同一框架)

```rust
#[async_trait]
trait AuthStep { async fn run(&self, data: &mut AuthData) -> Result<()>; }

// 微软链路 = 6 个 step 串行(见 auth.md)
let steps = [MsaDeviceCode, XboxAuth, Xsts, MinecraftLogin, Entitlements, Profile];

pub enum Account { Offline { name }, Msa { .. }, Yggdrasil { .. } }
// 三种账号统一出口
pub struct AuthSession { access_token, uuid, username, user_type, xuid }
```

- **device code flow 优先**(无浏览器/跨平台友好)。
- **token 加密存储**用 `keyring` crate(封装 Win DPAPI / macOS Keychain / Linux Secret Service),不自己滚加密。

---

## 7. IPC:核心 ↔ 前端

Tauri 层是**薄胶水**,只做三件事:命令转发、进度/日志流推送、类型导出。

```rust
// desktop/src/commands.rs —— 把 mc-core 调用包成 tauri command
#[tauri::command]
async fn launch_instance(id: String, app: AppHandle) -> Result<(), String> {
    let handle = mc_core::launch::launch(id).await.map_err(|e| e.to_string())?;
    // 把核心的 progress watch / log mpsc 转成前端 event
    tokio::spawn(forward_progress(handle.progress, app.clone()));
    Ok(())
}
```

流式数据(下载进度、游戏日志)用 **Tauri Channel / event**,不要轮询:

```
核心 mpsc/watch  ──forward──>  app.emit("launch://progress", payload)
                                app.emit("game://log", line)
前端: listen("launch://progress", e => updateBar(e.payload))
```

**类型安全跨语言**:`mc-types` 里的 DTO 用 `ts-rs` 自动导出 `.ts` 类型,前端直接 import,Rust 改字段前端编译即报错。零手写 interface、零 drift。

---

## 8. 错误处理

```rust
// 库层用 thiserror(结构化、可匹配)
#[derive(thiserror::Error, Debug)]
pub enum CoreError {
    #[error("下载失败 {url}: {source}")]
    Download { url: String, source: reqwest::Error },
    #[error("SHA1 校验失败: {path}")]
    Checksum { path: PathBuf },
    #[error("Xbox 认证错误 {code}: {hint}")]   // XSTS 错误码翻人话
    Xsts { code: u64, hint: String },
    #[error("未找到匹配的 Java {major}")]
    JavaNotFound { major: u8 },
    ...
}
```

- 核心层用 `thiserror` 给**可匹配**的结构化错误(UI 据此显示具体引导)。
- CLI/应用边界用 `anyhow` 兜底 + 上下文链。
- XSTS 这类错误码必须翻译成人话(见 auth.md)。

---

## 9. 关键依赖清单(crates)

| 用途 | crate | 备注 |
|------|-------|------|
| async 运行时 | `tokio` | 全核心统一 |
| HTTP | `reqwest`(rustls) | 连接池/HTTP2;rustls 免系统 OpenSSL |
| 多核并行 | `rayon` | 批量 SHA1 校验 |
| 哈希 | `sha1`/`sha2` | 启用 asm/硬件加速 feature |
| JSON | `serde` + `serde_json` | 版本 json 强类型 |
| 解压 | `zip` + `flate2` | natives 提取 |
| 错误 | `thiserror` + `anyhow` | 库/应用分层 |
| 取消 | `tokio-util`(CancellationToken) | 全程可中止 |
| 凭证加密 | `keyring` | 跨平台原生密钥库 |
| 类型导出 | `ts-rs` | DTO → TypeScript |
| 限速 | `governor` 或自实现 | 下载限速 |
| 日志 | `tracing` | 结构化日志 + 性能 span |
| 外壳 | `tauri` v2 | 仅 desktop crate |

---

## 10. 性能手法清单(把效率刻进设计)

1. **reqwest::Client 全局单例** — 连接池复用,省 TLS 握手(最大单点收益)。
2. **边下边校验** — 流式 SHA1,不二次读盘。
3. **rayon 并行校验** — 千文件摊满核心,启用 SHA 硬件指令。
4. **准备阶段三路并发** — 文件/Java/token 用 `try_join!`。
5. **assets 后台补** — 不阻塞启动,可玩了再后台补齐。
6. **零拷贝大 payload** — Tauri 传日志/进度用 event 而非大 JSON 返回值。
7. **rustls 而非 native-tls** — 免系统依赖、启动快、跨平台一致。
8. **共享 store + 硬链接** — 多实例共享 libraries/assets,省磁盘(Prism 做法)。
9. **冷启动指标化** — 用 `tracing` 给启动器自身 init 打 span,守住 <0.5s。
10. **spawn_blocking 隔离 CPU 活** — 解压/校验不堵 async 调度器。

---

## 11. 施工路线(里程碑)

```
M0  workspace 骨架 + mc-types + CLI 空壳                    (能 cargo run)
M1  版本 json 解析 + 命令行拼接                              (CLI 能打印出正确的 java 命令)
M2  下载引擎 + 校验补全                                      (CLI 能下完一个原版并校验通过)
M3  Java 检测 + SpawnProcess                                ★ CLI 能启动离线原版游戏  ← 第一个里程碑
M4  微软登录(device code)                                  (CLI 能正版启动)
M5  Tauri 外壳 + 前端基础 UI(实例列表/启动/进度/日志)       ★ 有图形界面能玩
M6  Forge/Fabric 安装 + 组件合并                            (能玩模组)
M7  镜像源(BMCLAPI)+ 多源 + DNS                            (国内可用)
M8  Mod/整合包(Modrinth)+ 实例管理 + 资源管理              (生态完整)
M9+ 主题/崩溃分析/联机/迁移 ...                              (差异化)
```

> **M3 是关键验证点**:不碰任何 UI,纯 `mc-core` + CLI 就能启动游戏。核心跑通了,套 UI 只是工程量,不是风险。这正是"核心与外壳分离"的最大好处——**风险前置、独立验证**。
```
