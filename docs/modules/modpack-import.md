# 模块 · 整合包导入(可插拔架构)

> 「从别人的整合包配置一个本地实例」。本文是这套功能的**架构核心**:把导入做成
> **一个统一接口 + 每种格式一个可插拔模块 + 一个共享执行引擎 + 一个按优先级排序的探测分发器**。
> 加一种新格式 = 写一个模块 + 在注册表里加一行,**不动引擎**。这是项目主人的第一诉求。
>
> 配套文档:[modpack-formats.md](./modpack-formats.md)(每种格式的精确 Rust 结构)·
> [modpack-export.md](./modpack-export.md)(导出/让别人下载)·
> [content-providers.md](./content-providers.md)(平台 Provider 抽象,导入/导出共用)·
> [instance-and-components.md](./instance-and-components.md)(从零创建 + 组件 + 加载器核心)。

## 0. 参考实现怎么做的

| 实现 | 抽象方式 | 文件 |
|------|----------|------|
| **Prism** | 虚基类 `InstanceCreationTask`(`executeTask()` 固定流程,只暴露 `updateInstance()`/`createInstance()` 两个虚函数)+ 手写分发器 `InstanceImportTask`(按文件名优先级嗅探 → `switch(ModpackType)`)。**无注册表**,是 enum + if/else + switch。 | `InstanceCreationTask.{h,cpp}`、`InstanceImportTask.{h,cpp}`、`modplatform/{modrinth,flame}/*InstanceCreationTask` |
| **PCL2** | 更不 OO:一个 `Select Case PackType` 阶梯调 `InstallPackX()`。但**格式最全**(含国内 MCBBS / HMCL)。 | `Modules/Minecraft/ModModpack.vb` |
| **PCL-CE** | 接口化(`PCL.Core/Minecraft/ResourceProject`),Provider 用闭包注入。 | `ResourceProject/` |

我们取 Prism 的「**一个固定引擎 + 少量每格式扩展点 + 优先级嗅探**」语义,但用 Rust 的 `Vec<Box<dyn ModpackImporter>>`
注册表表达,并把 Prism 单体的 `createInstance()` 拆成 **`plan()`(纯解析)+ 引擎执行副作用**——`plan()` 可对着 fixture
manifest 做单元测试(对齐 mc-core 既有 `pick_version` 的可测纯逻辑风格)。

## 1. 统一接口:`trait ModpackImporter`

```rust
// crates/mc-core/src/modpack/import/mod.rs
pub trait ModpackImporter: Send + Sync {
    /// 稳定 id,也是 DetectMatch.format 的取值。
    fn id(&self) -> &'static str;

    /// 只读嗅探(对应 Prism detectInstance lambda):在已打开的归档索引里找本格式的
    /// 标记文件,命中则报告包根(archive_root)。不得解压/下载。
    fn detect(&self, archive: &dyn ArchiveIndex) -> Option<DetectMatch>;

    /// 解析本格式的 manifest(已解压到 staging)为与来源无关的 ImportPlan。
    /// 纯函数式:只读 staging、不联网、不改实例 —— 等价 parseManifest/loadManifest。
    fn plan(&self, staging: &Path, m: &DetectMatch) -> Result<ImportPlan>;

    /// 可选第二趟:把 UnresolvedRef 解析成具体下载源。默认空操作。
    /// 只有「给的是 id 而非 URL」的格式(CurseForge、部分 MCBBS)覆盖它,
    /// 委托给 content-providers 的 ResourceProvider 批量查文件 + 探测 blocked。
    fn resolve<'a>(&'a self, dl: &'a Downloader, plan: &'a mut ImportPlan)
        -> Pin<Box<dyn Future<Output = Result<Vec<BlockedFile>>> + Send + 'a>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// detect() 用的归档只读视图——让 importer 脱离具体 zip 类型,可用假对象做单测。
pub trait ArchiveIndex {
    fn entries(&self) -> &[String];
    fn read_small(&self, name: &str) -> Option<Vec<u8>>; // 供 CF/MCBBS 读 manifest 内容判别
}
```

> **async 约定**:沿用仓库「不引 `async_trait`」的决定(见 `modplatform/mod.rs`)——`detect()`/`plan()`
> 同步;只有 `resolve()` 联网,用 `Pin<Box<dyn Future>>`(对齐 `instance/install_mod.rs` 的 `install_rec`)。

## 2. 与来源无关的中间模型:`ImportPlan`

每种格式的 `plan()` 都产出**同一个** `ImportPlan`;引擎此后什么都不需要再知道。**这是关键的缝。**

```rust
pub struct ImportPlan {
    pub pack_name: String,
    pub pack_version: Option<String>,
    pub mc_version: String,
    pub loader: Option<(LoaderKind, String)>,   // (Fabric,"0.15.7")…; None = 原版
    pub files: Vec<PlannedFile>,
    pub unresolved: Vec<UnresolvedRef>,          // mrpack/multimc 为空;CF 填它
    pub override_roots: Vec<String>,             // 顺序拷进游戏目录;mrpack=["overrides","client-overrides"],CF/MCBBS=["overrides"],MMC=[".minecraft"]
    pub recommended_ram_mib: Option<u64>,
    pub managed: Option<ManagedPack>,            // provider id/version,供日后「更新整合包」
}

pub struct PlannedFile {
    pub rel_path: String,        // 相对游戏目录,如 "mods/sodium.jar";引擎用前必过 safe_join
    pub sources: Vec<String>,    // 有序候选 URL(多源!)——引擎依次尝试
    pub sha1: Option<String>,
    pub sha512: Option<String>,  // 有些格式(mrpack)只给 sha512
    pub size: Option<u64>,
    pub required: bool,          // 可选项:可跳过 / 落 ".disabled"
}

/// 需要二次解析才有 URL 的引用(CurseForge projectID/fileID)。
pub struct UnresolvedRef { pub project_id: String, pub file_id: String, pub target_dir: String, pub required: bool }

/// 无第三方下载链接的 CF 文件(法律上不可再分发)——回传给 UI 让用户手动下,引擎跳过它。
pub struct BlockedFile { pub name: String, pub website_url: String, pub target_dir: String, pub required: bool }

/// 跨格式「这实例来自哪个平台的哪个包/版本」的溯源记录(MultiMC 的 ManagedPack* 是它的鼻祖)。
pub struct ManagedPack { pub platform: String, pub project_id: String, pub version_id: Option<String> }
```

`DetectMatch { format, archive_root, confidence }`:`archive_root` 是包在归档内的真实根(MultiMC 嵌套目录 / Technic 映射到 `minecraft`),
让探测与解压解耦。

## 3. 共享执行引擎:`ImportEngine`

引擎实现一次,所有格式复用——它 = Prism `InstanceCreationTask::executeTask` + `InstanceImportTask` 管线,
但**适配到本启动器「version == instance」的 `versions/<id>/` 目录模型**(不照搬 Prism 的 `instances/` 布局)。

```
import(src, opts):
  1. 取归档:ImportSource::Url → 先用现有 Downloader 下载整包(对齐 Prism「先下后探」);LocalFile 跳过
  2. 打开 zip 一次 → 建 ArchiveIndex → dispatch():按注册顺序跑各 importer.detect(),取最高 confidence
  3. 按 DetectMatch.archive_root 把对应子树解压到 staging 临时目录;修权限
  4. importer.plan(staging) → ImportPlan
  5. importer.resolve(dl, &mut plan) → 填 unresolved 的 sources + 收集 BlockedFile(默认空操作)
  6. 建实例目录 versions/<id>/(走现有 Instance/GamePaths),写 instance.json(name/icon/内存=recommended_ram)+ managed 溯源
  7. 装加载器:取 plan.loader → 调现有 loader::install_{fabric,forge,neoforge,quilt};原版 → launch::install_version
  8. 下文件:PlannedFile → DownloadItem → Downloader::download_all(并发+校验);多源故障转移在此集中实现
  9. 铺 overrides:逐个 override_root 经 safe_join 拷进游戏目录(同一个路径安全闸)
  10. 可选/blocked 处理:引擎决定跳过 vs 落 ".disabled";BlockedFile 列表进 ImportOutcome 回 UI
```

```rust
pub struct ImportEngine { dl: Downloader, importers: Vec<Box<dyn ModpackImporter>> }
impl ImportEngine {
    pub fn with_defaults(dl: Downloader) -> Self { /* 注册全部内建 importer,顺序即优先级 */ }
    pub async fn import(&self, src: ImportSource, opts: ImportOptions) -> Result<ImportOutcome>;
    /// 纯分发步骤:按注册顺序跑 detect(),取最高 confidence。可单独测试。
    pub fn dispatch(&self, archive: &dyn ArchiveIndex) -> Option<(usize, DetectMatch)>;
}
pub enum ImportSource { LocalFile(PathBuf), Url(String) }
pub struct ImportOptions { pub dest_root: PathBuf, pub instance_id: Option<String>, pub icon: Option<PathBuf>, pub managed: Option<ManagedPack> }
pub struct ImportOutcome { pub instance_id: String, pub blocked: Vec<BlockedFile>, pub skipped_optional: Vec<String> }

/// 唯一的路径安全闸——拒绝绝对路径/盘符/规整后逃出 game_dir 的路径(处理 "../"、"..\\"、"./"、Windows 分隔符)。
/// 引擎里写一次,所有格式的 PlannedFile.rel_path 与每个 override 拷贝都走它。
fn safe_join(game_dir: &Path, rel: &str) -> Result<PathBuf>;
```

> Prism 的交互式 `OptionalModDialog`/`BlockedModsDialog` 在我们这里**退化为回传给 Tauri 层的数据**:
> 逻辑留核心,UI 保持薄。`ImportOptions.managed`/`instance_id` 对应 Prism dispatcher 的 `extra_info`
> (provider 发起的安装 vs 裸 zip 拖入,以及就地更新已存在实例)。

## 4. 探测分发(优先级 + 内容判别)

把 Prism 的「唯一命名优先、`manifest.json` 最后、按内容区分」沉淀为**注册顺序 + detect() 内容检查**:

| 序 | 格式 | 标记 | 备注 |
|----|------|------|------|
| 1 | mcbbs | `mcbbs.packmeta` | 必须在 manifest.json 之前(PCL2 #1) |
| 2 | multimc | basename == `mmc-pack.json` 或 `instance.cfg` | 在 manifest.json 之前(PCL2 #4194);捕获嵌套 root |
| 3 | modrinth | `modrinth.index.json` | |
| 4 | curseforge | `manifest.json` 且内容 `manifestType=="minecraftModpack"` 且**无** `addons` | manifest.json 用户里**最后**(它会出现在 overrides 内) |
| 4b | mcbbs(再) | `manifest.json` 且内容**有** `addons`/`launchInfo` | CF 与 MCBBS 同名,靠**内容**区分 |
| 5 | packwiz | `pack.toml` | |
| 6 | technic | `bin/modpack.jar` \| `bin/version.json` | archive_root 映射到 `minecraft` |
| 7 | atlauncher | `instance.json` + 签名 | 最低优先级 |

两条从参考实现学到的硬规则(写进 `detect()`):
1. **唯一命名标记先于 `manifest.json`**——后者会出现在 `overrides/` 里造成误判。
2. **CurseForge vs MCBBS 必须读 `manifest.json` 内容判别**(`addons` 存在 = MCBBS),不能只看文件名;
   所以 `detect()` 返回 `confidence`(不是 bool),让根级标记得分高于深层标记。

```
dispatch(archive): 打开 zip 一次 → 建 ArchiveIndex → 按注册顺序跑 detect()
  → 第一个返回 Some 的胜出(早停,= Prism 的 stop=true);用 confidence 时取最大,平局按注册序
  → 全不中 → Err("not a recognized modpack")
新增格式 = 写一个实现 ModpackImporter 的模块 + 在 with_defaults 里按优先级插一行。引擎零改动。
```

## 5. 共享 vs 每格式(职责切分)

**共享引擎(写一次,全格式复用)**:取包 → 探测分发 → 按 root 解压 → 建实例(`versions/<id>/`)→ 装加载器(调现有 `loader/`)→ 下文件(`Downloader` + **集中式多源故障转移**)→ `safe_join` 铺 overrides → 可选/blocked 策略 → 更新差量(对比留存的旧 manifest 调度删除,= Prism `scheduleToDelete`)。

**每格式模块(只装格式知识)**:`detect()`(标记 + 包根)+ `plan()`(解析 manifest,**唯一懂 schema 的地方**)+ 可选 `resolve()`(只有给 id 的格式)+ 各自的 `override_roots` 与目标目录约定。

| 格式 | 自带 URL? | resolve() | override 根 | 加载器来源 | 特殊点 |
|------|-----------|-----------|-------------|------------|--------|
| **modrinth (.mrpack)** | ✅(多源)| 无 | overrides + client-overrides | `dependencies` map 键 | sha512;env(client/server,optional) |
| **curseforge** | ❌ 只有 projectID/fileID | ✅ 批量查 Flame | overrides + 非 mod 重分类 | `modLoaders[].id` 前缀(neoforge 1.20.1 特判) | **blocked 文件**手动下;`detect` 读内容避开 MCBBS |
| **multimc/prism** | — 预装好的实例 | 无 | `.minecraft` | mmc-pack 组件 | 无远程文件;捕获嵌套 root |
| **mcbbs(国内)** | 混合 | ✅(部分,共用 CF) | overrides | `addons[]` 数组 | 内容判别(addons/launchInfo);`fileApi` 镜像基址 |
| **packwiz** | ✅(.pw.toml 给 url+hash) | 无 | (元文件图) | `[versions]` | TOML 两级:pack.toml→index.toml→.pw.toml;强哈希 |
| **technic** | 远程 | — | 映射 minecraft | 解压后嗅探 | Solder/zip 两变体;远程拉取 |

## 6. 两种来源:本地归档 vs 远程 PackProvider

不是所有「整合包」都是本地 zip。MCBBS 长尾分析揭示需要**两种来源,同一条安装管线**:

- **LocalArchiveImporter**(.mrpack / CurseForge / MultiMC / MCBBS / packwiz):上面的 `ModpackImporter` + zip 探测。
- **RemotePackProvider**(ATLauncher / Technic / Solder):没有本地 manifest,从服务器拉(ATLauncher `Configs.json`、Technic Solder `/modpack/{pack}/{version}`),然后喂**同一个** `ImportPlan` 管线。

两者都产出 `ImportPlan`,引擎第 6–10 步完全共享。建议 ATLauncher/Technic 走「平台浏览器选包 → RemotePackProvider 取 plan」入口,而非本地文件 importer。

## 7. 建议模块布局

```
crates/mc-core/src/modpack/
  mod.rs            新顶层模块:pub mod import; pub mod export; + 共享类型(ManagedPack 等)
  import/
    mod.rs          trait ModpackImporter / ArchiveIndex / ImportPlan 等 plan 类型 / 注册表与分发优先级
    engine.rs       ImportEngine:共享 executeTask 等价管线(format-independent);safe_join 在此
    archive.rs      zip 打开 + ArchiveIndex 实现 + 按 archive_root 解压到 staging + 修权限(唯一碰 zip crate 处)
    modrinth.rs     .mrpack importer
    curseforge.rs   CurseForge importer(覆盖 resolve() 调 provider)
    multimc.rs      MultiMC/Prism importer
    mcbbs.rs        MCBBS(国内)importer
    packwiz.rs      packwiz importer
    technic.rs      Technic importer(较低优先级)
    atlauncher.rs   ATLauncher importer(最低优先级)
    tests.rs        分发优先级(假 ArchiveIndex)+ path-safety + 各格式 plan() golden 测试
```

## 8. 当前 mc-core 差距

- **完全无整合包导入**:`modplatform/modrinth.rs` 只有 search + list-versions;无 zip 处理、无探测分发、无 manifest 解析、无 from-pack 建实例流程。
- **无路径安全闸**(`safe_join` 无等价物);现有本地 mod/pack 管理只操作已可信的盘上文件,不防 manifest 注入路径。
- **下载器无多源/sha512**:见 [download.md](./download.md) §4/§5 —— 是导入的硬前置(`PlannedFile.sources` + sha512 校验)。
- **无 CurseForge backend** 和 **无 Provider 抽象**:CF 的 `resolve()` 与 MCBBS 部分文件都依赖它,见 [content-providers.md](./content-providers.md)。
- **无组件/多 loader 模型**:导入把 loader 写进 `ImportPlan.loader` 调现有安装器即可(单 loader 场景够用),但若要无损往返 MultiMC 组件图,见 [instance-and-components.md](./instance-and-components.md)。

## 9. 自研要点

1. **先落地引擎 + `trait ModpackImporter` + `ImportPlan`**,再写第一个 importer(modrinth,自带 URL + sha512,最简单),验证整条管线。
2. **`plan()` 必须纯**(解析 → ImportPlan,无副作用),对 fixture manifest 单测;副作用全在引擎。
3. **多源故障转移 + sha512 在引擎集中实现**一次(Prism 是每文件 lambda 散落,我们收口)。
4. **path-safety 不可妥协**:一个 `safe_join`,所有 `rel_path` 与 override 拷贝都走它。
5. **CurseForge 第二:** 它带来 Provider 抽象 + blocked-mod 手动流 + 多源,装好它后 MCBBS 几乎白送。
6. **逻辑留核心,交互回传数据**:OptionalMod/BlockedMod 是 `ImportOutcome` 里的数据,UI 薄。
