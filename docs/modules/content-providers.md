# 模块 · 内容平台 Provider 抽象(Modrinth / CurseForge 等)

> 「下载不同来源的内容」的统一抽象——搜索/详情/版本/**按哈希反查**/批量取文件,各平台一个可插拔实现。
> 这是导入、导出、浏览**三者共用**的底座(与 [modpack-import.md](./modpack-import.md) 的 importer、
> [modpack-export.md](./modpack-export.md) 的 exporter 同构,正是「相似的地方做相似设计」)。
>
> 本文取代/扩展旧 [mod-platform.md](./mod-platform.md) 的「平台抽象」一节,聚焦可落地的 Rust trait。

## 0. 参考实现

- **Prism `ResourceAPI`**(`modplatform/ResourceAPI.h`):抽象基类,`ModrinthAPI`/`FlameAPI` 子类,枚举 `ResourceProvider{MODRINTH,FLAME}` 选取。
  - 纯虚:`getSortingMethods` / URL 构造器(`getSearchURL/getInfoURL/getVersionsURL/getDependencyURL`)/ `getProjects(ids)` / JSON 规范化(`documentToArray`、`loadIndexedPack`、`loadIndexedPackVersion`)。
  - 基类共享:`searchProjects/getProjectInfo/getProjectVersions`(构参 → 调虚 URL 构造 → 跑通用 Net 任务 → 喂虚 `documentToArray/loadIndexed*` → 回调)。
  - **反查按能力声明**:`ProviderCapabilities::hashType(provider)` → Modrinth `[sha512,sha1]`,Flame `[sha1,md5,murmur2]`;具体端点是子类专属(Modrinth `POST /version_files`,Flame `POST /fingerprints` + `POST /mods/files`)。
- **PCL-CE**:`ResourceProject/{Curseforge,Modrinth}` 两套模型 + `ModDependencyResolver`(provider 无关,注入 `Func<source,projectId,Project?>` 闭包,按 `(source, projectId)` 键 + visited-set + MaxDepth=32 + 最佳文件挑选)。

## 1. `trait ResourceProvider`

泛化 Prism `ResourceAPI` + PCL-CE `ResourceProject`。方法是今天 `ModrinthApi`(search/get_versions/get_project)的**超集**,加上 exporter/importer 需要的反查与批量取文件。

```rust
// crates/mc-core/src/modplatform/provider.rs
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderId { Modrinth, CurseForge }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HashAlgo { Sha1, Sha512, Md5, Murmur2 }  // 按偏好序;= Prism ProviderCapabilities::hashType

pub struct ProviderCaps {
    pub id: ProviderId,
    pub readable_name: &'static str,
    pub hash_algos: &'static [HashAlgo],  // Modrinth:[Sha512,Sha1]  CurseForge:[Sha1,Md5,Murmur2]
    pub needs_api_key: bool,              // CurseForge 是,Modrinth 否
}

/// 跨平台统一身份 = (provider, project_id, version_id)(Prism (provider,addonId,fileId);PCL-CE (Source,ProjectId))。
pub struct ResolvedFile {
    pub provider: ProviderId,
    pub project_id: String,
    pub version_id: String,
    pub file: VersionFile,        // 复用现有 modplatform::VersionFile{url,filename,sha1,size,primary}
    pub project_name: Option<String>,
    pub project_slug: Option<String>,
    pub authors: Vec<String>,
}

#[async_trait::async_trait]
pub trait ResourceProvider: Send + Sync {
    fn caps(&self) -> &ProviderCaps;
    fn id(&self) -> ProviderId { self.caps().id }

    async fn search(&self, q: &SearchQuery<'_>) -> Result<Vec<SearchHit>>;
    async fn get_project(&self, project_id: &str) -> Result<SearchHit>;
    async fn get_projects(&self, project_ids: &[&str]) -> Result<Vec<SearchHit>>;          // 批量(Prism getProjects)
    async fn list_versions(&self, project_id: &str, game_version: Option<&str>, loader: Option<&str>) -> Result<Vec<ProjectVersion>>;

    // ---- 反查 / 批量取文件:exporter 与 import 去重的命脉 ----
    async fn resolve_by_hash(&self, algo: HashAlgo, hash: &str) -> Result<Option<ResolvedFile>>;
    /// 批量哈希→文件(下标对齐);Modrinth=POST /version_files,CurseForge=POST /fingerprints(murmur2)。
    async fn resolve_by_hashes(&self, algo: HashAlgo, hashes: &[&str]) -> Result<Vec<Option<ResolvedFile>>>;
    /// 批量按 (project,version) id 取文件——IMPORT 把 CF manifest 的 id 变 URL(POST /mods/files)。
    async fn get_files_bulk(&self, refs: &[(&str, &str)]) -> Result<Vec<ResolvedFile>>;
}
```

> **新依赖提醒**:`modplatform/mod.rs` 当初为避开 `async_trait` 才不定义统一 trait。要 `Arc<dyn ResourceProvider>`
> 就得加 `async-trait`(便宜、通用,推荐)或手写 `Pin<Box<dyn Future>>`。这是个需主人拍板的新依赖。

## 2. `ProviderRegistry`(双键选取)

```rust
pub struct ProviderRegistry { by_id: HashMap<ProviderId, Arc<dyn ResourceProvider>> }
impl ProviderRegistry {
    pub fn get(&self, id: ProviderId) -> Option<Arc<dyn ResourceProvider>>;
    pub fn for_host(&self, host: &str) -> Option<Arc<dyn ResourceProvider>>; // cdn.modrinth.com→Modrinth, *.forgecdn.net→CurseForge
    pub fn all(&self) -> impl Iterator<Item = &Arc<dyn ResourceProvider>>;
}
```

- **按 id**:调用方已知来源(CF 导出 → CF provider;`SearchHit` 自带来源)。
- **按 host**:把一个下载 URL 映射回所属 provider(`cdn.modrinth.com`→Modrinth、`*.forgecdn.net`→CurseForge)。这是导出「免费 resolve」与导入去重判断「本地文件的 URL 是否可远程引用」(Prism `MODRINTH_MRPACK_HOSTS` 白名单检查)的依据。

**同一个 registry 被导入(id→URL via `get_files_bulk`)和导出(hash→ref via `resolve_by_hashes`)共用**——这正是「让导入导出走同一套 provider」的具体收益。

## 3. 依赖解析(可移植 PCL-CE `ModDependencyResolver`)

provider 无关的纯算法,导入的依赖补全与「装 mod 时拉必需依赖」共用:

```
resolve(target, mc, loader, registry):
  visited = {}  (key = (provider, project_id))
  queue = [target];  depth guard MaxDepth = 32
  while queue:
    proj = registry.get(ref.provider).list_versions(ref.project_id, mc, loader)
    pick = 最佳文件(精确 MC 匹配 > loader 匹配 > release 类型 > 日期最新)
    for dep in pick.dependencies where required: 未 visited 则入队、递归
  → { to_install[], satisfied[], unresolved[], incompatible[] }
```

mc-core 落到 `modplatform/dependency.rs`,对 `&ProviderRegistry` 操作。**给用户可见预览**(将装哪些/冲突/没找到),别静默装(对齐旧 [mod-platform.md](./mod-platform.md) §4)。

## 4. 共享 vs 每 provider

**共享(写一次)**:`search/get_project/list_versions` 编排调 trait、依赖解析、registry 双键查找、api-key/header 注入策略、MC 版本归一。

**每 provider 模块**:URL/查询方言、JSON 字段 → 统一 `SearchHit/ProjectVersion/ResolvedFile` 映射、反查端点 + 其算法(Modrinth sha512 `/version_files`;CurseForge murmur2 `/fingerprints` 再 `/mods/files`)、批量 `/mods` 补全、`caps()`/api-key。

| Provider | hash_algos | api key | 反查 | 搜索方言 |
|----------|-----------|---------|------|----------|
| **Modrinth** | [Sha512, Sha1] | 否 | `POST /version_files` | json-array `facets` |
| **CurseForge** | [Sha1, Md5, Murmur2] | **是**(`x-api-key`) | `POST /fingerprints` → `POST /mods/files` | `classId` + 数字 `modLoaderTypes` |

## 5. 建议模块布局

```
crates/mc-core/src/modplatform/
  mod.rs          保留 ResourceKind/SearchHit/ProjectVersion/VersionFile/Dependency;新增 ProviderId/HashAlgo/ProviderCaps/ResolvedFile/SearchQuery/SortMethod
  provider.rs     trait ResourceProvider + ProviderRegistry + host→provider 路由
  modrinth.rs     现有 ModrinthApi 包成 impl ResourceProvider for ModrinthProvider(补 resolve_by_hash(es)/get_files_bulk/get_projects)
  curseforge.rs   新建 CurseForgeProvider(fingerprints/murmur2、POST /mods、POST /mods/files、x-api-key)
  dependency.rs   移植 PCL-CE ModDependencyResolver(provider 无关、visited-set、MaxDepth、最佳文件挑选)
```

## 6. 当前 mc-core 差距

- `modrinth.rs` 已有 search/get_versions/get_project 且模型一致 → 天然第一个 `ResourceProvider` 实现;但**无反查**(`resolve_by_hash(es)`)、**无批量** `get_files_bulk`/`get_projects`。
- **无 CurseForge backend**(无 Flame API、无 `x-api-key`、无 murmur2)。
- **无 trait/registry**(`mod.rs` 故意没定义 trait 避 `async_trait`)→ 需引 `async-trait`。
- **无依赖解析器**。
- 反查需要 sha512 + murmur2,而 `checksum.rs` 仅 sha1(见 [download.md](./download.md) §5)。

## 7. 自研要点

1. 先加 `provider.rs` 的 trait + registry,把现有 `ModrinthApi` 适配成第一个实现(基本零成本,主要是补反查/批量方法)。
2. **反查 `resolve_by_hashes` 是导入去重 + 导出的命脉**,优先于花哨搜索。
3. CurseForge provider 与 [modpack-import.md](./modpack-import.md) 的 CF importer、[modpack-export.md](./modpack-export.md) 的 CF target 一起落地(三者共用它)。
4. 用 `(provider, project_id, version_id)` 作依赖解析与导入/导出的去重键,避免同一 mod 多路径重复拉取。
5. 镜像:provider 的 API 与文件下载都应过 McIM 国内镜像(见 [download.md](./download.md) §4.3)。
