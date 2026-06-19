# 模块 · 从零建实例 + 多组件版本模型 + 加载器核心

> 「从零配一个整合包」与「下载不同的核心(Forge/Fabric/Quilt/NeoForge)」是同一件事的两面:
> 一个实例的「版本」该如何表达、加载器如何作为一层插进去、从零创建的流程长什么样。
> 配套:[version-system.md](./version-system.md)(json 解析/合并/规则)、[modpack-import.md](./modpack-import.md)
> (导入复用同一套建实例 + 装加载器)。

## 0. 两种版本模型

| | 单叶 + `inheritsFrom`(mc-core 现状) | 多组件 `mmc-pack.json`(MultiMC/Prism) |
|---|---|---|
| 版本定义 | 一个 loader json,`inheritsFrom` 指向 vanilla | **有序组件列表** `[{uid,version},...]` |
| 合并序 | 由父指针隐式决定 | 由列表顺序显式决定 |
| 加载器身份 | 从 id/inheritsFrom 子串**猜**(`infer_loader`) | uid 一等公民 + 冲突矩阵 |
| 依赖(intermediary/lwjgl) | 寄望 loader json 里已含 | requires/conflicts 图,**自动注入** |
| 改 MC 版本 | 不能联动 loader | `ImportantChanged` 级联重算 |

mc-core 现在是**扁平**的:`launch::install_*` 各写一个 `inheritsFrom` vanilla 的 loader json。合并数学(`profile.rs` 的 `from_chain`)是**对的且可复用**,只是输入排序模型(父指针)太死,表达不了「intermediary 夹在 vanilla 与 loader 之间」。

## 1. Prism/MultiMC 组件模型(参考)

实例目录的版本定义 = `mmc-pack.json`:`{formatVersion:1, components:[...]}`,**有序**(顺序即合并优先级,后盖前)。每个组件:

```rust
// 见 modpack-formats.md §4 的完整结构
MmcComponent { uid, version, important, dependency_only, disabled,
               cached_version, cached_name, cached_requires, cached_conflicts, cached_volatile }
MmcRequire   { uid, equals_version /*JSON "equals"*/, suggests }   // 跨组件依赖边,按 uid 去重
```

- `uid` 选 meta 服务器的版本文件(或本地 `patches/<uid>.json` 覆盖)。`net.minecraft`/所选 loader = `important`(不可删)。
- `dependency_only` 组件(如 `net.fabricmc.intermediary`、`org.lwjgl3`)为满足 require 自动注入、`volatile` 时不再被需要就自动移除。
- 加载器 uid 与冲突矩阵(`KNOWN_MODLOADERS`):`net.neoforged`/`net.minecraftforge`/`net.fabricmc.fabric-loader`/`org.quiltmc.quilt-loader`/`com.mumfrey.liteloader` 互斥;`org.lwjgl*`/`net.minecraft` 非 loader。Quilt 蕴含 Fabric;1.20.1 NeoForge 蕴含 Forge。

### 从零创建(Prism `VanillaInstanceCreationTask`)

整个「从零」流程极短:

```
buildingFromScratch()
setComponentVersion("net.minecraft", mc, important=true)   // 锚:追加 net.minecraft 组件
if 带加载器: setComponentVersion(loaderUid, loaderVer)      // 再追加一个 loader 组件(可删)
saveNow()                                                  // 写 mmc-pack.json
ComponentUpdateTask(Resolution):                           // 懒解析
  loadComponents() → 每组件加载 meta 版本文件(或本地 patch),刷 cached_requires/conflicts
  resolveDependencies():
    gatherRequirements(并 per-uid:最小位置、equals 须一致否则硬冲突、suggests 取最大)
    trivialRemovals(无人依赖的 dependency_only+volatile 移除)
    inject 缺失 require 为 dependency_only,插在首个依赖者之前;改版本满足 equals;循环到稳定
    finalizeComponents(): 缺依赖/版本不符/双 loader 共存 → 警告/错误
getProfile(): 按列表序对每组件 applyTo(LaunchProfile)(库按坐标 upsert、mainClass 末者胜、args 追加)
```

> Prism 作者自评依赖版本硬编码(lwjgl=2.9.1/3.1.2、intermediary=MC 版本)是 HACK(「该用图」)。**别照抄硬编码**——
> 优先用 require 的 equals/suggests,intermediary==mc 作为有原则的规则。

## 2. 给 mc-core 的建议(精简版组件模型)

采用组件模型,但要**精简**——它是当前最大的结构缺口,且一举解锁整合包导入、加载器增删、多 loader。

1. **加 `crate::version::pack`**:`PackProfile{format_version, components: Vec<Component>}`,落地为实例目录里的 `mmc-pack.json`(或合进 `instance.json` 的 `components` 字段免第二文件)。
2. **`Component`/`Require` 结构** + 小 `KNOWN_LOADERS` 表(uid→`LoaderKind` + 冲突 uid),让加载器识别从「id 子串猜」(`instance/mod.rs::infer_loader`)变成「列表里哪个已知 loader uid」。
3. **现有 `loader/` 安装器变组件产出**:`install_*` 不再只回一个可启动 id,而是产出一个 `Component`(uid+version)+ 它拉到的 meta 版本文件。`install_fabric/quilt` 已取到现成 profile json → 存为 loader 组件的 resolved 版本文件;`install_forge/neoforge` 跑完 installer 后,生成的 `versions/<id>/<id>.json` 即该组件的(custom/冻结)版本文件。
4. **从零 API**:`InstanceCreate` 做 VanillaCreationTask 流程——`new_pack_profile().set_component("net.minecraft", mc, important=true)`;可选 `set_component(loader_uid, ver)`;`resolve()`;下载;**暂存到 temp 目录,成功后原子 move**(对齐 Prism `executeTask`),别直写 `versions/<id>`。
5. **`from_chain` 升级为按组件合并**:`profile::from_components(&[(Component, VersionJson)]) -> LaunchProfile`,**复用现有 `upsert_library`/末者胜逻辑**,只把排序来源从父指针换成显式列表。保留 `load_chain` 读仍用 `inheritsFrom` 的单 loader json。
6. **解析器(纯数据,无 Qt 任务)**:`gather_requirements`/`trivial_removals`/`trivial_changes`/inject,循环。intermediary/hashed 解析成 MC 版本(有原则,非硬编码)。
7. **携带 requires/conflicts**:取 loader meta/profile json 时解析并存其 requires(fabric→intermediary==mc + minecraft suggests mc;forge/neoforge→minecraft==mc)。今天没人读依赖边,加 fabric 全靠 json 里碰巧已含 intermediary。离线可用故存 `cached_requires/conflicts`。
8. **导入直接复用**:.mrpack/CF/MMC 导入 = 「从 manifest 建组件列表」——`net.minecraft`=包 MC 版本,每个声明 loader 一个组件(写成冻结/custom patch 防漂),再跑同一解析器补 intermediary/lwjgl。这是现在就采纳组件模型最划算的理由。

**不要照抄**:Prism 的 reactor/Qt 信号架构、`QAbstractListModel`、防抖保存定时器、lwjgl 硬编码表。`PackProfile` 用普通 serde 结构 + 显式 `save()`;解析是对已加载版本文件的同步函数(IO 像 `load_chain` 那样注入)。jarMods/agents/customJar 组件按需再说。

## 3. 不同加载器核心怎么进来

| 核心 | uid | 安装 | 版本文件来源 |
|------|-----|------|--------------|
| Vanilla | `net.minecraft` | `launch::install_version` | Mojang version json |
| Forge | `net.minecraftforge` | 跑 installer | 生成的 `versions/<id>/<id>.json`(custom/冻结) |
| NeoForge | `net.neoforged` | 跑 installer | 同上;id 含 1.20.1 特判 |
| Fabric | `net.fabricmc.fabric-loader` | 拉 loader meta | 现成 profile json;require intermediary==mc |
| Quilt | `org.quiltmc.quilt-loader` | 拉 loader meta | 现成 profile json;require hashed==mc |

从 UI/导入拿到 `(LoaderKind, version)` → 调对应 `loader::install_*` → 得一个组件 → 进 `PackProfile`。**导入与从零走同一条装加载器路径**(见 [modpack-import.md](./modpack-import.md) §3 引擎第 7 步)。

## 4. 与本启动器目录模型的衔接

mc-core 是「version == instance」(`versions/<id>/` 既是实例根又是游戏目录,配置在 `instance.json`),而 Prism 是 `instances/<n>/{instance.cfg, mmc-pack.json, minecraft/}`。所以:

- **导入 MultiMC**:`instance.cfg`→`instance.json`、`mmc-pack.json`→mc-core 版本/loader 模型(想无损往返就保留组件列表)、把 `minecraft/` 内容**拍平**进 `versions/<id>/`。
- **从零/导出**:反向合成 `minecraft/` 子目录 + `instance.cfg` + `mmc-pack.json`。
- **暂存-提交**:任何建实例都应暂存到 temp、成功才原子 move,不直写最终目录(Prism `InstanceCreationTask` 语义)。

## 5. 当前 mc-core 差距

- **无组件模型**:版本 = 单叶 json + `inheritsFrom`(`version/mod.rs::load_chain`);无 `mmc-pack.json`、无可编辑的有序 `{uid,version}` 列表,表达不了「vanilla + forge + intermediary」为独立可单独增删的层。
- **无从零/配整合包路径**:无 `VanillaCreationTask` 等价;实例只能靠扫 `versions/` 发现(`list_instances`);`install_*` 顺带产出可启动 id,但没有「建实例 → 增删 loader」的生命周期。
- **无依赖图**:`Require`/requires/conflicts/`dependency_only` 注入/`volatile` 移除/双 loader 冲突检测全缺;加 Fabric 全靠 meta json 已含 intermediary。
- **加载器身份是启发式**:`infer_loader` 靠 id/inheritsFrom 子串猜,无 uid 注册表、无冲突矩阵。
- **加载器版本只冻结不重选**:`install_*` 原样写,不重解析;无 floating/recommended、无 customize/revert、改 MC 版本不级联。
- **合并按父指针非显式列表**:注入的 intermediary 夹不进「vanilla 与 loader 之间」,层不能重排/禁用(合并数学 `from_chain` 对、可复用,只是输入排序太死)。

## 6. 自研要点

1. 先落 `PackProfile`/`Component`/`Require` 的 serde 结构 + `KNOWN_LOADERS` 表,把 `infer_loader` 换成「查组件列表里的已知 loader uid」。
2. `from_components` 复用 `from_chain` 的合并逻辑,只换排序来源。
3. `install_*` 改为产出 `Component`(+ 其版本文件);从零/导入都经它装核心。
4. 解析器做成纯函数(对已加载版本文件),可单测;intermediary==mc 作规则,别硬编码 lwjgl 表。
5. 暂存-提交建实例;导入直接复用 §2.8 的「manifest → 组件列表 → 解析器」。
