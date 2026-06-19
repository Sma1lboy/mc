# 模块 · 整合包导出(让别人能下载你的包)

> 「把你配好的实例打成一个可分享、别人能一键下载安装的整合包」。与导入对称的可插拔设计:
> **一个共享导出引擎 + 每种目标格式一个 `ExportTarget` 模块**。
> 配套:[modpack-import.md](./modpack-import.md)、[modpack-formats.md](./modpack-formats.md)、
> [content-providers.md](./content-providers.md)(反查哈希靠它)。

## 0. 核心难题:Resolvable vs Override

导出的全部技术含量在这一句:**对每个文件,判定它能否表达成一个平台下载 URL。**

- **Resolvable**:文件的哈希在平台(Modrinth/CurseForge)能反查到对应版本 → 写进**索引**(URL/id 形式),**不**进包。包因此又小又合规。
- **Override**:反查不到 → 原样塞进 `overrides/` 打包带走。

Prism 的关键技巧是 `setExcludeFiles(resolvedFiles.keys())`:打 zip 时**排除**已 resolved 的文件,
让它们只在索引里出现、不在 overrides 里重复。这一行是整个导出的承重点,务必保留。

```
未匹配 = override 是安全默认。
CF 额外坑:匹配到的版本若 isAvailable==false,会被丢弃 → 既不在索引也不在 overrides(数据丢失边界,需警告)。
```

## 1. 五阶段管线(Prism 两个导出 Task 共享同一骨架)

Prism 没有导出基类——`ModrinthPackExportTask` 与 `FlamePackExportTask` 是各自独立的 `Task`,
却跑**完全相同**的 5 阶段。我们把它抽成**一个引擎**:

```
1. collectFiles   遍历 game_root ∩ UI 选择;按 target.accepts() 过滤(目录前缀 × 扩展名)
2. collectHashes  对候选算哈希(sha512 / murmur2);先试本地元数据「免费 resolve」(见 §3)
3. resolve        把剩余哈希批量反查:provider.resolve_by_hashes(algo, hashes)
4.(仅 CF)backfill  provider.get_projects() 补 slug/name/authors(给 modlist.html)
5. buildZip       写 overrides/(排除 resolved 键)+ 注入索引文件(内存字节,放 zip 根)
失败/取消 → 删半成品,不留损坏的 .mrpack/.zip
```

文件门控(`accepts`):Modrinth 只收 `mods/ coremods/ resourcepacks/ texturepacks/ shaderpacks/` × `{jar,litemod,zip}`;CF 收 `{jar,zip}` + resourcepacks 特判。非门控文件(config、options.txt、saves…)直接进 overrides,不哈希。

**永不导出**的硬忽略:`logs/ crash-reports/ .cache/ .fabric/ .quilt/ .DS_Store thumbs.db *.pw.toml` + 各资源的 `.index` 元数据目录。用户取消勾选持久化到 `<instance>/.packignore`。

## 2. 可插拔目标:`trait ExportTarget`

引擎跑一次;每个格式模块只声明「门控 + 哈希算法 + 分类策略 + 索引写法 + 打包方式」。

```rust
// crates/mc-core/src/export/mod.rs
pub trait ExportTarget: Send + Sync {
    fn id(&self) -> &'static str;                 // "modrinth" | "curseforge" | "modlist"
    fn output_extension(&self) -> &'static str;   // "mrpack" | "zip" | "html"/"md"/...

    fn provider(&self) -> Option<ProviderId>;     // 反查用哪个平台;None = 无 resolve 阶段(纯 modlist)
    fn hash_algo(&self) -> Option<HashAlgo>;       // mrpack: Sha512;CurseForge: Murmur2

    fn accepts(&self, relative: &Path) -> bool;    // 文件门控(前缀 × 扩展名)

    /// 已 resolved 的文件能否对本格式作远程引用?
    /// mrpack: 仅当下载 host 在 mrpack 白名单内;CurseForge: 仅当 isAvailable。
    /// 返回 false → 即便 resolved 也强制塞进 overrides/。
    fn allow_remote(&self, _r: &ResolvedFile) -> bool { true }

    /// 基于 resolved 集 + meta + 实例 loader 图序列化索引,返回要注入归档的 (文件名, 字节)。
    /// mrpack=1 个;curseforge=2 个(manifest.json + modlist.html);modlist=1 个文本。
    fn write_index(&self, input: &ExportInput<'_>, set: &ClassifiedSet) -> Result<Vec<(String, Vec<u8>)>>;

    fn packaging(&self) -> Packaging { Packaging::ZipWithOverrides }
}

pub enum Packaging { ZipWithOverrides, SingleTextFile }
pub enum FileClass { Resolvable(ResolvedFile), Override(PathBuf), Skipped }
pub struct ClassifiedSet { pub resolved: Vec<(PathBuf, ResolvedFile)>, pub overrides: Vec<PathBuf> }

pub struct ModpackExporter { providers: Arc<ProviderRegistry> }
impl ModpackExporter {
    pub async fn export(&self, target: &dyn ExportTarget, input: ExportInput<'_>,
                        progress: &mut dyn FnMut(ExportPhase, u64, u64)) -> Result<PathBuf>;
}
```

各格式只是声明的差异:

| 目标 | provider | hash_algo | allow_remote 策略 | 索引 | 打包 |
|------|----------|-----------|-------------------|------|------|
| **modrinth** | Modrinth | Sha512 | host ∈ mrpack 白名单 | `modrinth.index.json`(env/side + dependencies 取自 loader 图) | zip + overrides/ |
| **curseforge** | CurseForge | Murmur2 | `isAvailable` | `manifest.json` + `modlist.html`(`modLoaders[].id="fabric-x"`) | zip + overrides/ |
| **modlist** | None | None | — | 单文本(HTML/MD/TXT/JSON/CSV/自定义模板) | 单文件(跳过 2–4 阶段) |

> `ExportToModList` 证明「导出」不止打包:把它做成 `Packaging::SingleTextFile` + `provider=None` 的 `ExportTarget`,
> resolve 阶段自动跳过——一个 trait,两种打包,引擎无特判。`OptionalData{Authors|Url|Version|FileName}` 选列,
> 各格式各自转义(HTML 实体、Markdown 标点、CSV 多作者引号)。

## 3. 本地元数据「免费 resolve」(关键优化)

若一个 jar 当初就是从平台装的、并记录了来源,导出时**跳过联网反查**:

- Modrinth:本地 mod 元数据 `url` 的 host 在 `MODRINTH_MRPACK_HOSTS` 白名单 → 直接写索引。
- CurseForge:本地元数据 `provider==FLAME` 带 `project_id/file_id` → 直接成 ResolvedFile。

这要求**安装时记录来源**(provider/project_id/file_id/version_id/url/sha1/sha512)到 mod 的 sidecar 元数据
(Prism 的 `.index/<slug>.pw.toml`)。当前 mc-core `instance/mods.rs` 的 `ModInfo` 只解析 jar manifest
(name/version/mod_id/authors),**不存来源**——这是开启免费 resolve 的前置改造。

## 4. 哈希反查 = Provider 能力

反查是导出离不开、却最不统一的能力(Modrinth sha512 批量 vs CurseForge murmur2 指纹 + 二次 `/mods/files`)。
所以它藏在 `ResourceProvider::resolve_by_hashes(algo, hashes)` + `caps().hash_algos` 后(见 [content-providers.md](./content-providers.md)):
引擎读 target 的 `hash_algo()`、断言 provider 支持它,再调统一接口,自身保持 algo-无关。

- Modrinth:`POST /version_files {hashes, algorithm:"sha512"}`;响应按哈希键回,并**二次核对** `file.hashes.sha512==请求哈希`(一个版本可含多文件)。
- CurseForge:本地算 murmur2(seed 1,**滤字节 9/10/13/32**)→ `POST /fingerprints` → `exactMatches[].file`;再 `POST /mods` 补 slug/name/authors。

## 5. 共享 vs 每格式

**共享引擎**:文件发现 + 选择过滤 + 三种哈希(sha1/sha512/murmur2)+ 免费 resolve 趟 + 批量反查 + 分类(resolved/override,套 `allow_remote`)+ 打 zip(排除 resolved 键 + 注入索引)/ 写单文本 + 进度/取消 + **path-safety**(全部相对 game_root,逃逸即跳,镜像导入侧的解压安全)。

**每格式模块(很薄)**:`accepts()` 门控、哈希算法、`provider()`、`allow_remote()` 策略、`write_index()` schema、打包模式。

## 6. 建议模块布局

```
crates/mc-core/src/export/
  mod.rs         trait ExportTarget + ModpackExporter 引擎 + ExportInput/ClassifiedSet/FileClass/Packaging + 目标注册表
  walk.rs        共享 game_root 遍历 + 选择过滤 + 哈希(sha1/sha512/murmur2)+ path-safety
  zip.rs         ExportToZipTask 等价:写 overrides/ 排除 resolved 键 + 注入额外文件;失败删半成品
  modrinth.rs    ModrinthExportTarget(modrinth.index.json,sha512,mrpack host 白名单,.mrpack)
  curseforge.rs  CurseForgeExportTarget(manifest.json + modlist.html,murmur2,isAvailable 门,.zip)
  modlist.rs     ModListExportTarget(HTML/MD/TXT/JSON/CSV/模板;Packaging::SingleTextFile)
```

裸备份复用 `zip.rs`:空前缀 + 无排除 + 无额外文件 = 实例备份 zip。一个引擎,两个调用方。

## 7. 当前 mc-core 差距

- **无哈希反查**:`modrinth.rs` 只有 search/get_versions/get_project,缺 `versions_from_hashes`(`POST /version_files`)——导出反查的命脉。
- **无 sha512 / murmur2**:`checksum.rs` 仅 sha1。导出 Modrinth 需 sha1+sha512,CF 需 murmur2(滤空白变体)。见 [download.md](./download.md) §5。
- **无 CurseForge backend / Provider 抽象**:CF 导出与反查都依赖,见 [content-providers.md](./content-providers.md)。
- **无 zip 写出**:`download/` 只下不写;无 `ExportToZipTask` 等价(前缀 + 排除 + 内存额外文件 + 半成品清理)。
- **本地 mod 无来源溯源**:`ModInfo` 不存 provider/project_id/file_id/url/sha → 免费 resolve 无从谈起。
- **无 include/exclude 选择 / 忽略规则 / `.packignore`**,无索引/manifest 生成器,无 mod 列表导出。

## 8. 自研要点

1. 先做 `walk.rs + zip.rs + ModrinthExportTarget`(Modrinth 反查最干净,sha512 单算法)跑通 resolved-vs-override。
2. **`setExcludeFiles` 等价不能漏**——resolved 文件只进索引、不进 overrides。
3. CurseForge 目标随 [content-providers.md](./content-providers.md) 的 CF provider + murmur2 一起落地。
4. 安装时记录来源元数据(配合 [modpack-import.md](./modpack-import.md) 的 import 一并写),解锁免费 resolve。
5. 把 modlist 当作一个 `ExportTarget`,不要特判——证明抽象到位。
